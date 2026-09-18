use flow_like::a2ui::micro_widget::{MICRO_WIDGET_COMPONENT_TYPE, encode_package_widget_ref};
use flow_like_types::{
    Value,
    json::{Map, json},
};
use std::collections::{BTreeMap, BTreeSet};

/// Sources are merged by agreement. Different contracts, or an untyped route,
/// require an open payload so the output never promises fields a source omits.
#[derive(Default)]
pub(super) struct EventInference {
    pub action_ids: BTreeSet<String>,
    schemas: Vec<Option<Value>>,
    incomplete: bool,
}

impl EventInference {
    pub fn add(&mut self, name: &str, schema: Option<&Value>) {
        self.action_ids.insert(name.to_string());
        // `meta::is_valid` panics on an unknown `$schema`; the contract is third-party input.
        self.schemas.push(
            schema
                .filter(|value| jsonschema::meta::try_is_valid(value).unwrap_or(false))
                .cloned(),
        );
    }

    pub fn untyped_source(&mut self) {
        self.schemas.push(None);
    }

    pub fn unknown_source(&mut self) {
        self.untyped_source();
        self.incomplete = true;
    }

    pub fn is_complete(&self) -> bool {
        !self.incomplete
    }

    pub fn payload_schema(&self) -> Option<&Value> {
        let first = self.schemas.first()?.as_ref()?;
        self.schemas
            .iter()
            .all(|schema| schema.as_ref() == Some(first))
            .then_some(first)
    }

    pub fn instantiated_events(
        &mut self,
        events: &BTreeMap<String, Option<Value>>,
        selected: &str,
    ) {
        self.action_ids.extend(events.keys().cloned());
        for (name, schema) in events {
            if selected.is_empty() || name == selected {
                self.add(name, schema.as_ref());
            }
        }
    }
}

const ROUTE_CONTEXT_KEYS: [&str; 6] = [
    "nodeId", "node_id", "appId", "app_id", "boardId", "board_id",
];

/// The pin receives the renderer's action context, not the raw payload: an
/// object payload is spread beside `actionId` and the untouched `payload`, and
/// route context fields are merged in, so the root must stay open.
pub(super) fn action_context_schema(payload: &Value) -> Value {
    let mut root = Map::new();
    let mut nested = payload.clone();
    if let Some(schema) = nested.as_object_mut() {
        for keyword in ["$schema", "$defs", "definitions"] {
            if let Some(value) = schema.remove(keyword) {
                root.insert(keyword.to_string(), value);
            }
        }
        // An embedded `$id` would resolve `#/$defs/...` against the payload, not the root.
        if root.contains_key("$defs") || root.contains_key("definitions") {
            schema.remove("$id");
        }
    }
    let mut properties = payload
        .get("properties")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    // The host validator skips a root `$ref` and accepts a missing payload unless
    // `type` rejects it; compaction then drops the undefined `payload` key.
    let enforced_type = payload
        .get("$ref")
        .is_none()
        .then(|| payload.get("type"))
        .flatten();
    let mut required: Vec<Value> = payload
        .get("required")
        .and_then(Value::as_array)
        .filter(|_| enforced_type.and_then(Value::as_str) == Some("object"))
        .into_iter()
        .flatten()
        .filter(|name| !matches!(name.as_str(), Some("actionId" | "payload")))
        .cloned()
        .collect();
    // An omitted optional field keeps the route's value for the same key.
    properties.retain(|name, _| {
        !ROUTE_CONTEXT_KEYS.contains(&name.as_str())
            || required
                .iter()
                .any(|field| field.as_str() == Some(name.as_str()))
    });
    required.push(json!("actionId"));
    if enforced_type.is_some() {
        required.push(json!("payload"));
    }
    properties.insert("actionId".to_string(), json!({"type": "string"}));
    properties.insert("payload".to_string(), nested);
    root.insert("type".to_string(), json!("object"));
    root.insert("properties".to_string(), Value::Object(properties));
    root.insert("required".to_string(), Value::Array(required));
    Value::Object(root)
}

pub(super) struct PageTarget<'a> {
    pub app_id: &'a str,
    pub board_id: &'a str,
    pub node_id: &'a str,
}

fn literal(value: &Value) -> Option<&str> {
    value.as_str().or_else(|| {
        if value.get("path").is_some() {
            None
        } else {
            value.get("literalString")?.as_str()
        }
    })
}

fn matches_scope(context: &Value, target: &PageTarget<'_>) -> bool {
    [
        ("appId", "app_id", target.app_id),
        ("boardId", "board_id", target.board_id),
    ]
    .iter()
    .all(|(camel, snake, expected)| {
        context
            .get(camel)
            .or_else(|| context.get(snake))
            .is_none_or(|value| {
                literal(value).is_some_and(|value| value.is_empty() || value == *expected)
            })
    })
}

fn action_targets(action: &Value, target: &PageTarget<'_>) -> bool {
    action["name"] == "workflow_event"
        && literal(&action["context"]["nodeId"]).or_else(|| literal(&action["context"]["node_id"]))
            == Some(target.node_id)
        && matches_scope(&action["context"], target)
}

fn binding_targets(binding: &Value, target: &PageTarget<'_>) -> bool {
    let workflow = &binding["workflow"];
    if workflow["flowId"].as_str() == Some(target.node_id) && matches_scope(workflow, target) {
        return true;
    }
    // PageContent::Widget uses the typed ActionBinding representation.
    let workflow = &binding["workflowEvent"];
    literal(&workflow["event_id"]).or_else(|| literal(&workflow["eventId"])) == Some(target.node_id)
        && matches_scope(
            workflow
                .get("context_mapping")
                .or_else(|| workflow.get("contextMapping"))
                .unwrap_or(&Value::Null),
            target,
        )
}

/// Inspect component locations only. A widget's props and schema examples are
/// data, even when they happen to contain objects that look like components.
pub(super) fn infer_page(
    page: &Value,
    target: &PageTarget<'_>,
    definitions: &BTreeMap<String, Value>,
    package_contracts: &BTreeMap<String, Value>,
    inference: &mut EventInference,
) {
    if ["onLoadEventId", "onUnloadEventId", "onIntervalEventId"]
        .into_iter()
        .any(|hook| page[hook].as_str() == Some(target.node_id))
    {
        inference.untyped_source();
    }
    let mut walker = PageWalker {
        page,
        target,
        definitions,
        package_contracts,
        inference,
        remaining: 10_000,
    };
    for component in page["components"].as_array().into_iter().flatten() {
        walker.component(component.get("component").unwrap_or(component), 0);
    }
    for content in page["content"].as_array().into_iter().flatten() {
        if let Some(component) = content.get("component") {
            walker.component(component.get("component").unwrap_or(component), 0);
        } else if let Some(instance) = content.get("widget") {
            walker.widget_instance(instance, 0);
        }
    }
}

struct PageWalker<'a, 'b> {
    page: &'a Value,
    target: &'a PageTarget<'a>,
    definitions: &'a BTreeMap<String, Value>,
    package_contracts: &'a BTreeMap<String, Value>,
    inference: &'b mut EventInference,
    remaining: usize,
}

impl PageWalker<'_, '_> {
    fn component(&mut self, component: &Value, depth: usize) {
        if depth >= 32 || self.remaining == 0 {
            self.inference.unknown_source();
            return;
        }
        self.remaining -= 1;
        match component["type"].as_str() {
            Some(MICRO_WIDGET_COMPONENT_TYPE) => {
                let key = encode_package_widget_ref(
                    component["packageId"].as_str().unwrap_or_default(),
                    component["widgetId"].as_str().unwrap_or_default(),
                );
                let contract = component
                    .get("contract")
                    .filter(|value| value.is_object())
                    .or_else(|| self.package_contracts.get(&key));
                let before = self.inference.schemas.len();
                self.routes(
                    component,
                    contract.and_then(|contract| contract.get("events")),
                    contract.is_some(),
                );
                if contract.is_none() && self.inference.schemas.len() > before {
                    self.inference.unknown_source();
                }
            }
            Some("widgetInstance") => self.widget_instance(component, depth + 1),
            // Ordinary components send their route context without a widget payload.
            _ if self.routes_directly(component) => self.inference.untyped_source(),
            _ => {}
        }
        // Flat components normally reference child ids; tolerate embedded children too.
        if let Some(children) = component.get("children").and_then(Value::as_array) {
            for child in children.iter().filter(|value| value.is_object()) {
                self.component(child.get("component").unwrap_or(child), depth + 1);
            }
        }
    }

    fn routes_directly(&self, component: &Value) -> bool {
        component["actions"]
            .as_array()
            .and_then(|actions| actions.first())
            .into_iter()
            .chain(
                component["eventHandlers"]
                    .as_object()
                    .into_iter()
                    .flat_map(|handlers| handlers.values())
                    .filter_map(Value::as_array)
                    .flatten(),
            )
            .any(|action| action_targets(action, self.target))
    }

    fn routes(&mut self, instance: &Value, events: Option<&Value>, declared_only: bool) {
        let mut names = BTreeSet::new();
        for map in [
            events,
            (!declared_only)
                .then(|| instance.get("eventHandlers"))
                .flatten(),
            (!declared_only)
                .then(|| instance.get("actionBindings"))
                .flatten(),
        ]
        .into_iter()
        .flatten()
        .filter_map(Value::as_object)
        {
            names.extend(map.keys().cloned());
        }
        names.remove("*");
        if !declared_only
            && names.is_empty()
            && instance["eventHandlers"]["*"]
                .as_array()
                .is_some_and(|actions| {
                    actions
                        .iter()
                        .any(|action| action_targets(action, self.target))
                })
        {
            self.inference.unknown_source();
        }
        for name in names {
            // Exact handlers, including an empty list, win over a wildcard;
            // either named form takes precedence over legacy bindings.
            let targets = if let Some(actions) = instance["eventHandlers"]
                .get(&name)
                .or_else(|| instance["eventHandlers"].get("*"))
            {
                actions.as_array().is_some_and(|actions| {
                    actions
                        .iter()
                        .any(|action| action_targets(action, self.target))
                })
            } else if let Some(binding) = instance["actionBindings"].get(&name) {
                binding_targets(binding, self.target)
            } else {
                instance["actions"]
                    .as_array()
                    .and_then(|actions| actions.first())
                    .is_some_and(|action| action_targets(action, self.target))
            };
            if targets {
                self.inference.add(
                    &name,
                    events
                        .and_then(|events| events.get(&name))
                        .and_then(|event| event.get("payloadSchema")),
                );
            }
        }
    }

    fn widget_instance(&mut self, instance: &Value, depth: usize) {
        if depth >= 32 || self.remaining == 0 {
            self.inference.unknown_source();
            return;
        }
        // Match the renderer's precedence: saved reference, inline definition,
        // then a same-app reusable widget. Never load an arbitrary foreign app.
        let instance_id = instance["instanceId"].as_str().unwrap_or_default();
        let widget_id = instance["widgetId"].as_str().unwrap_or_default();
        let definition = self.page["widgetRefs"]
            .get(instance_id)
            .or_else(|| instance.get("inlineWidgetDef"))
            .or_else(|| {
                instance
                    .get("appId")
                    .is_none_or(|app| literal(app) == Some(self.target.app_id))
                    .then(|| self.definitions.get(widget_id))
                    .flatten()
            });
        let actions = definition.map(|definition| {
            Value::Object(
                definition["actions"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|action| {
                        action["id"]
                            .as_str()
                            .map(|id| (id.to_string(), Value::Null))
                    })
                    .collect(),
            )
        });
        self.routes(instance, actions.as_ref(), false);
        if let Some(definition) = definition {
            for component in definition["components"].as_array().into_iter().flatten() {
                self.component(component.get("component").unwrap_or(component), depth + 1);
            }
        } else {
            self.inference.unknown_source();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn target() -> PageTarget<'static> {
        PageTarget {
            app_id: "app",
            board_id: "board",
            node_id: "handler",
        }
    }
    fn payload() -> Value {
        json!({"type":"object","properties":{"id":{"type":"string"}},"required":["id"]})
    }
    fn route() -> Value {
        json!({"name":"workflow_event","context":{"nodeId":{"literalString":"handler"}}})
    }
    fn micro() -> Value {
        json!({"type":"microWidgetInstance","packageId":"geo","widgetId":"map","instanceId":"map-1",
            "contract":{"events":{"selected":{"payloadSchema":payload()},"moved":{"payloadSchema":{"type":"object","properties":{"longitude":{"type":"number"}}}}}},
            "eventHandlers":{"selected":[route()]}})
    }
    fn page(component: Value) -> Value {
        json!({"components":[{"id":"map","component":component}]})
    }
    fn infer(page: &Value) -> EventInference {
        let mut result = EventInference::default();
        infer_page(
            page,
            &target(),
            &BTreeMap::new(),
            &BTreeMap::new(),
            &mut result,
        );
        result
    }
    fn with_component(component: Value) -> Value {
        let mut root = page(micro());
        root["components"]
            .as_array_mut()
            .unwrap()
            .push(json!({"component":component}));
        root
    }

    #[test]
    fn page_placed_package_event_infers_its_saved_payload_without_instantiation() {
        let result = infer(&page(micro()));
        assert_eq!(result.action_ids, BTreeSet::from(["selected".into()]));
        assert_eq!(result.payload_schema(), Some(&payload()));
    }

    #[test]
    fn all_reachable_payloads_must_agree_even_when_event_names_match() {
        let mut root = page(micro());
        let mut second = micro();
        root["components"]
            .as_array_mut()
            .unwrap()
            .push(json!({"component":second}));
        assert_eq!(infer(&root).payload_schema(), Some(&payload()));
        second["contract"]["events"]["selected"]["payloadSchema"] = json!({"type":"string"});
        root["components"][1]["component"] = second;
        assert!(infer(&root).payload_schema().is_none());
        root["components"].as_array_mut().unwrap().pop();
        root["components"][0]["component"]["eventHandlers"]["moved"] = json!([route()]);
        let result = infer(&root);
        assert_eq!(
            result.action_ids,
            BTreeSet::from(["moved".into(), "selected".into()])
        );
        assert!(result.payload_schema().is_none());
    }

    #[test]
    fn instantiated_selection_and_catch_all_keep_their_binding_meaning() {
        let events = BTreeMap::from([
            ("selected".into(), Some(payload())),
            ("moved".into(), Some(json!({"type":"string"}))),
        ]);
        let mut selected = EventInference::default();
        selected.instantiated_events(&events, "selected");
        assert_eq!(selected.action_ids.len(), 2);
        assert_eq!(selected.payload_schema(), Some(&payload()));
        let mut catch_all = EventInference::default();
        catch_all.instantiated_events(&events, "");
        assert!(catch_all.payload_schema().is_none());
        selected.add("untyped-page-route", None);
        assert!(selected.payload_schema().is_none());
    }

    #[test]
    fn binding_scope_must_resolve_statically_to_this_app_board_and_node() {
        for context in [
            json!({"nodeId":"handler","appId":"foreign"}),
            json!({"nodeId":"handler","boardId":{"literalString":"foreign"}}),
            json!({"nodeId":"other"}),
            json!({"nodeId":{"path":"/target","defaultValue":"handler"}}),
            json!({"nodeId":"handler","appId":{"path":"/app"}}),
        ] {
            let mut component = micro();
            component["eventHandlers"]["selected"][0]["context"] = context;
            assert!(infer(&page(component)).action_ids.is_empty());
        }
        let mut component = micro();
        component["eventHandlers"]["selected"][0]["context"] =
            json!({"node_id":"handler","app_id":{"literalString":"app"},"board_id":"board"});
        assert_eq!(infer(&page(component)).payload_schema(), Some(&payload()));
    }

    #[test]
    fn named_handlers_override_legacy_bindings_even_when_disabled() {
        let mut component = micro();
        component["actionBindings"] = json!({"selected":{"workflow":{"flowId":"handler"}}});
        component["eventHandlers"] = json!({"selected":[]});
        assert!(infer(&page(component.clone())).action_ids.is_empty());
        component["eventHandlers"] = json!({"selected":[{"name":"external_link","context":{}}]});
        assert!(infer(&page(component.clone())).action_ids.is_empty());
        component.as_object_mut().unwrap().remove("eventHandlers");
        assert_eq!(
            infer(&page(component.clone())).payload_schema(),
            Some(&payload())
        );
        component["actionBindings"]["selected"]["workflow"]["boardId"] = json!("foreign");
        assert!(infer(&page(component)).action_ids.is_empty());
    }

    #[test]
    fn legacy_fallback_routes_all_unbound_declared_events() {
        let mut component = micro();
        component.as_object_mut().unwrap().remove("eventHandlers");
        component["actions"] = json!([route()]);
        let result = infer(&page(component.clone()));
        assert_eq!(result.action_ids.len(), 2);
        assert!(result.payload_schema().is_none());
        component["actionBindings"] = json!({"moved":{"workflow":{"flowId":"other"}}});
        assert_eq!(infer(&page(component)).payload_schema(), Some(&payload()));
    }

    #[test]
    fn placed_nested_definitions_and_typed_page_content_are_discovered() {
        let definition = json!({"components":[{"component":micro()}]});
        for root in [
            json!({"content":[{"component":{"id":"placed","component":micro()}}]}),
            page(
                json!({"type":"widgetInstance","instanceId":"outer","widgetId":"wrapper","inlineWidgetDef":definition}),
            ),
            json!({"content":[{"widget":{"instanceId":"outer","widgetId":"wrapper"}}],"widgetRefs":{"outer":definition}}),
        ] {
            assert_eq!(infer(&root).payload_schema(), Some(&payload()));
        }
        let typed = json!({"content":[{"widget":{"instanceId":"outer","widgetId":"wrapper","actionBindings":{"clicked":{"workflowEvent":{"event_id":"handler","context_mapping":{}}}}}}]});
        let result = infer(&typed);
        assert_eq!(result.action_ids, BTreeSet::from(["clicked".into()]));
        assert!(result.payload_schema().is_none());
    }

    #[test]
    fn reusable_widget_and_package_contract_fallbacks_use_only_known_definitions() {
        let mut component = micro();
        let contract = component
            .as_object_mut()
            .unwrap()
            .remove("contract")
            .unwrap();
        let definitions = BTreeMap::from([(
            "wrapper".into(),
            json!({"components":[{"component":component}]}),
        )]);
        let contracts = BTreeMap::from([(encode_package_widget_ref("geo", "map"), contract)]);
        let mut root =
            page(json!({"type":"widgetInstance","instanceId":"outer","widgetId":"wrapper"}));
        let mut result = EventInference::default();
        infer_page(&root, &target(), &definitions, &contracts, &mut result);
        assert_eq!(result.payload_schema(), Some(&payload()));
        root["components"][0]["component"]["appId"] = json!("foreign");
        let mut result = EventInference::default();
        infer_page(&root, &target(), &definitions, &contracts, &mut result);
        assert!(result.action_ids.is_empty());
    }

    #[test]
    fn unrelated_data_and_unplaced_widget_definitions_do_not_create_routes() {
        let root = json!({"components":[{"component":{"type":"text","content":{"literalJson":micro().to_string()},"props":{"example":micro()}}}],"widgetRefs":{"unplaced":{"components":[{"component":micro()}]}}});
        assert!(infer(&root).action_ids.is_empty());
    }

    #[test]
    fn rebind_and_unbind_drop_the_previous_event_and_payload() {
        let mut component = micro();
        assert_eq!(
            infer(&page(component.clone())).payload_schema(),
            Some(&payload())
        );
        component["eventHandlers"] = json!({"moved":[route()]});
        let rebound = infer(&page(component.clone()));
        assert_eq!(rebound.action_ids, BTreeSet::from(["moved".into()]));
        assert_eq!(
            rebound.payload_schema(),
            Some(&component["contract"]["events"]["moved"]["payloadSchema"])
        );
        component["eventHandlers"] = json!({});
        let unbound = infer(&page(component));
        assert!(unbound.action_ids.is_empty() && unbound.payload_schema().is_none());
    }

    #[test]
    fn recursive_reusable_widgets_stop_at_a_bounded_depth() {
        let instance =
            json!({"type":"widgetInstance","instanceId":"recursive","widgetId":"wrapper"});
        let root = page(instance.clone());
        let definitions = BTreeMap::from([(
            "wrapper".into(),
            json!({"components":[{"component":instance}]}),
        )]);
        let mut result = EventInference::default();
        infer_page(
            &root,
            &target(),
            &definitions,
            &BTreeMap::new(),
            &mut result,
        );
        assert!(result.action_ids.is_empty() && result.payload_schema().is_none());
    }

    #[test]
    fn exact_empty_handlers_shadow_wildcards_and_wildcards_shadow_legacy_bindings() {
        let mut component = micro();
        component["eventHandlers"] = json!({"*":[route()],"moved":[]});
        component["actionBindings"] = json!({"selected":{"workflow":{"flowId":"other"}},"moved":{"workflow":{"flowId":"handler"}}});
        let result = infer(&page(component.clone()));
        assert_eq!(result.action_ids, BTreeSet::from(["selected".into()]));
        assert_eq!(result.payload_schema(), Some(&payload()));
        component["eventHandlers"] = json!({"*":[]});
        assert!(infer(&page(component)).action_ids.is_empty());
    }

    #[test]
    fn missing_or_invalid_payload_schemas_remain_generic() {
        for schema in [
            Value::Null,
            json!([]),
            json!("object"),
            json!({"type":123}),
            json!({"required":"id"}),
        ] {
            let mut component = micro();
            component["contract"]["events"]["selected"]["payloadSchema"] = schema;
            let result = infer(&page(component));
            assert_eq!(result.action_ids, BTreeSet::from(["selected".into()]));
            assert!(result.payload_schema().is_none());
        }
        let mut component = micro();
        component["contract"]["events"]["selected"] = json!({});
        assert!(infer(&page(component)).payload_schema().is_none());
    }

    #[test]
    fn package_contract_rejects_stale_routes_and_never_exposes_wildcard_as_an_event() {
        let mut component = micro();
        component["eventHandlers"] = json!({"undeclared":[route()],"*":[route()],"moved":[]});
        component["actionBindings"] = json!({"alsoUndeclared":{"workflow":{"flowId":"handler"}}});
        let result = infer(&page(component.clone()));
        assert_eq!(result.action_ids, BTreeSet::from(["selected".into()]));
        assert_eq!(result.payload_schema(), Some(&payload()));
        component.as_object_mut().unwrap().remove("contract");
        let result = infer(&page(component));
        assert!(result.action_ids.contains("undeclared") && result.payload_schema().is_none());
        assert!(!result.action_ids.contains("*"));
    }

    #[test]
    fn declarative_default_and_wildcard_routes_expand_declared_actions() {
        for wildcard in [false, true] {
            let mut instance = json!({"type":"widgetInstance","instanceId":"outer","widgetId":"wrapper","inlineWidgetDef":{"actions":[{"id":"save"},{"id":"cancel"}],"components":[]}});
            if wildcard {
                instance["eventHandlers"] = json!({"*":[route()],"cancel":[]});
            } else {
                instance["actions"] = json!([route()]);
            }
            let root = page(instance);
            let result = infer(&root);
            let expected = if wildcard {
                BTreeSet::from(["save".into()])
            } else {
                BTreeSet::from(["cancel".into(), "save".into()])
            };
            assert_eq!(result.action_ids, expected);
            assert!(result.payload_schema().is_none());
            let mut mixed = EventInference::default();
            mixed.add("selected", Some(&payload()));
            infer_page(
                &root,
                &target(),
                &BTreeMap::new(),
                &BTreeMap::new(),
                &mut mixed,
            );
            assert!(mixed.payload_schema().is_none());
        }
    }

    #[test]
    fn ordinary_component_routes_to_the_event_keep_the_context_generic() {
        for component in [
            json!({"type":"button","actions":[route()]}),
            json!({"type":"button","eventHandlers":{"click":[route()]}}),
            json!({"type":"widgetInstance","instanceId":"outer","widgetId":"wrapper",
                "inlineWidgetDef":{"components":[{"component":{"type":"button","actions":[route()]}}]}}),
        ] {
            let result = infer(&with_component(component));
            assert_eq!(result.action_ids, BTreeSet::from(["selected".into()]));
            assert!(result.is_complete() && result.payload_schema().is_none());
        }
        for component in [
            json!({"type":"button","actions":[{"name":"workflow_event","context":{"nodeId":"other"}},route()]}),
            json!({"type":"button","eventHandlers":{"click":[{"name":"navigate_page","context":{"nodeId":"handler"}}]}}),
            json!({"type":"button","eventHandlers":{"click":[{"name":"workflow_event","context":{"nodeId":"handler","appId":"foreign"}}]}}),
        ] {
            assert_eq!(
                infer(&with_component(component)).payload_schema(),
                Some(&payload())
            );
        }
    }

    #[test]
    fn page_lifecycle_hooks_on_the_event_keep_the_context_generic() {
        for hook in ["onLoadEventId", "onUnloadEventId", "onIntervalEventId"] {
            let mut root = page(micro());
            root[hook] = json!("other");
            assert_eq!(infer(&root).payload_schema(), Some(&payload()));
            root[hook] = json!("handler");
            let result = infer(&root);
            assert_eq!(result.action_ids, BTreeSet::from(["selected".into()]));
            assert!(result.is_complete() && result.payload_schema().is_none());
            let hook_only = infer(&json!({ hook: "handler", "components": [] }));
            assert!(hook_only.action_ids.is_empty() && hook_only.payload_schema().is_none());
        }
    }

    #[test]
    fn placed_copies_of_one_package_widget_share_its_installed_contract() {
        let mut component = micro();
        let contract = component
            .as_object_mut()
            .unwrap()
            .remove("contract")
            .unwrap();
        let contracts = BTreeMap::from([(encode_package_widget_ref("geo", "map"), contract)]);
        let mut second = component.clone();
        second["instanceId"] = json!("map-2");
        let mut root = json!({"components":[{"id":"map","component":component},{"id":"map-2","component":second}]});
        let infer_installed = |root: &Value| {
            let mut result = EventInference::default();
            infer_page(root, &target(), &BTreeMap::new(), &contracts, &mut result);
            result
        };
        let result = infer_installed(&root);
        assert!(result.is_complete());
        assert_eq!(result.payload_schema(), Some(&payload()));
        root["components"][1]["component"]["eventHandlers"]["moved"] = json!([route()]);
        let result = infer_installed(&root);
        assert_eq!(
            result.action_ids,
            BTreeSet::from(["moved".into(), "selected".into()])
        );
        assert!(result.payload_schema().is_none());
    }

    #[test]
    fn object_payloads_are_spread_beside_action_id_and_payload() {
        let payload = json!({"type":"object","properties":{"id":{"type":"string"},"actionId":{"type":"number"}},
            "required":["id","actionId"],"additionalProperties":false});
        let schema = action_context_schema(&payload);
        assert_eq!(
            schema,
            json!({"type":"object","properties":{"id":{"type":"string"},"actionId":{"type":"string"},"payload":payload},
                "required":["id","actionId","payload"]})
        );
        assert!(jsonschema::meta::is_valid(&schema));
        assert!(jsonschema::is_valid(
            &schema,
            &json!({"id":"a","actionId":"selected","payload":{"id":"a","actionId":1},"nodeId":"handler","appId":"app"})
        ));
        assert!(!jsonschema::is_valid(
            &schema,
            &json!({"actionId":"selected","payload":{"id":"a","actionId":1}})
        ));
    }

    #[test]
    fn non_object_payloads_are_typed_only_under_payload() {
        for (payload, value, typed) in [
            (json!({"type":"string"}), json!("text"), true),
            (
                json!({"type":"array","items":{"type":"number"}}),
                json!([1, 2]),
                true,
            ),
            (json!(true), Value::Null, false),
            (json!({}), json!({"id":1}), false),
            (
                json!({"properties":{"id":{"type":"string"}},"required":["id"]}),
                json!("text"),
                false,
            ),
        ] {
            let schema = action_context_schema(&payload);
            assert!(jsonschema::meta::is_valid(&schema));
            assert!(schema.get("additionalProperties").is_none());
            let required = if typed {
                json!(["actionId", "payload"])
            } else {
                json!(["actionId"])
            };
            assert_eq!(schema["required"], required);
            assert_eq!(schema["properties"]["payload"], payload);
            assert!(jsonschema::is_valid(
                &schema,
                &json!({"actionId":"selected","payload":value,"nodeId":"handler"})
            ));
            assert_eq!(
                jsonschema::is_valid(&schema, &json!({"actionId":"closed","nodeId":"handler"})),
                !typed
            );
        }
        let referenced = json!({"$ref":"#/$defs/entity","$defs":{"entity":{"type":"object"}},
            "type":"object","properties":{"id":{"type":"string"}},"required":["id"]});
        assert_eq!(
            action_context_schema(&referenced)["required"],
            json!(["actionId"])
        );
    }

    #[test]
    fn optional_payload_fields_named_like_route_keys_are_typed_only_under_payload() {
        let payload = json!({"type":"object","properties":{"nodeId":{"type":"number"},"board_id":{"type":"number"},
            "appId":{"type":"number"},"x":{"type":"number"}},"required":["appId"]});
        let schema = action_context_schema(&payload);
        assert_eq!(schema["properties"]["payload"], payload);
        assert_eq!(
            schema["properties"]
                .as_object()
                .unwrap()
                .keys()
                .map(String::as_str)
                .collect::<BTreeSet<_>>(),
            BTreeSet::from(["actionId", "appId", "payload", "x"])
        );
        let context = |app_id: Value| json!({"nodeId":"handler","board_id":"board","appId":app_id,"x":1,"actionId":"moved","payload":{"x":1,"appId":2}});
        assert!(jsonschema::is_valid(&schema, &context(json!(2))));
        assert!(!jsonschema::is_valid(&schema, &context(json!("app"))));
    }

    #[test]
    fn hoisted_definitions_resolve_for_spread_fields_and_the_nested_payload() {
        for keyword in ["$defs", "definitions"] {
            let mut payload = json!({"$schema":"https://json-schema.org/draft/2020-12/schema","$id":"urn:geo:selected",
                "type":"object","properties":{"id":{"$ref":format!("#/{keyword}/id")}},"required":["id"]});
            payload[keyword] = json!({"id":{"type":"string","minLength":1}});
            let schema = action_context_schema(&payload);
            assert!(jsonschema::meta::is_valid(&schema));
            assert_eq!(schema[keyword], payload[keyword]);
            assert_eq!(schema["$schema"], payload["$schema"]);
            let nested = schema["properties"]["payload"].as_object().unwrap();
            assert!(
                ["$schema", "$id", keyword]
                    .iter()
                    .all(|key| !nested.contains_key(*key))
            );
            let context = |spread: &str, nested: &str| json!({"id":spread,"actionId":"selected","payload":{"id":nested}});
            assert!(jsonschema::is_valid(&schema, &context("a", "a")));
            assert!(!jsonschema::is_valid(&schema, &context("", "a")));
            assert!(!jsonschema::is_valid(&schema, &context("a", "")));
        }
        let identified = json!({"$id":"urn:geo:moved","type":"string"});
        assert_eq!(
            action_context_schema(&identified)["properties"]["payload"],
            identified
        );
    }

    #[test]
    fn unknown_meta_schema_is_dropped_instead_of_panicking() {
        let mut inference = EventInference::default();
        inference.add(
            "selected",
            Some(&json!({"$schema":"https://example.com/not-a-draft","type":"object"})),
        );
        inference.add("moved", Some(&json!({"type":"object"})));
        assert_eq!(
            inference.schemas,
            vec![None, Some(json!({"type":"object"}))]
        );
    }
}
