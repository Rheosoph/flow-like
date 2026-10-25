use flow_like::a2ui::components::{
    excalidraw_event_context_schema, file_preview_event_context_schema,
};
use flow_like::a2ui::micro_widget::{MICRO_WIDGET_COMPONENT_TYPE, encode_package_widget_ref};
use flow_like_catalog_core::UploadedFile;
use flow_like_types::{
    Value,
    json::{Map, json},
};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::LazyLock;

/// Final action contexts are merged by agreement. Different contracts, or an untyped route,
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
                .map(action_context_schema),
        );
    }

    pub fn add_context(&mut self, name: &str, schema: Value) {
        self.action_ids.insert(name.to_string());
        self.schemas.push(
            jsonschema::meta::try_is_valid(&schema)
                .unwrap_or(false)
                .then_some(schema),
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

    pub fn has_sources(&self) -> bool {
        !self.schemas.is_empty()
    }

    pub fn context_schema(&self) -> Option<&Value> {
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

    pub fn instantiated_contexts(
        &mut self,
        events: &BTreeMap<String, Option<Value>>,
        selected: &str,
    ) {
        self.action_ids.extend(events.keys().cloned());
        for (name, schema) in events {
            if selected.is_empty() || name == selected {
                match schema {
                    Some(schema) => self.add_context(name, schema.clone()),
                    None => self.add(name, None),
                }
            }
        }
    }
}

const ROUTE_CONTEXT_KEYS: [&str; 6] = [
    "nodeId", "node_id", "appId", "app_id", "boardId", "board_id",
];

/// A newly created Event can arrive before its Page binding is persisted. Only
/// the explicit, canonical builder seed may survive this temporary empty scan.
pub(super) fn is_pending_component_context(schema: &Value, selected: &str) -> bool {
    let marker = &schema["x-flow-like-pending-component-event"];
    if marker["eventName"] != selected {
        return false;
    }
    let expected = match marker["componentType"].as_str() {
        Some("filePreview" | "fileEditor") => file_preview_event_context_schema(selected),
        Some("excalidraw") => excalidraw_event_context_schema(selected),
        _ => None,
    };
    let Some(expected) = expected else {
        return false;
    };
    let mut seeded = schema.clone();
    if let Some(object) = seeded.as_object_mut() {
        object.remove("x-flow-like-pending-component-event");
    }
    seeded == expected
}

/// The pin receives the renderer's action context, not the raw payload: an
/// object payload is spread beside `actionId` and the untouched `payload`, and
/// route context fields are merged in, so the root must stay open.
pub(super) fn action_context_schema(payload: &Value) -> Value {
    let payload = &resolve_file_markers(payload);
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

const FILE_MARKER: &str = "file";
const SUBSCHEMA_MAP_KEYWORDS: [&str; 6] = [
    "properties",
    "patternProperties",
    "$defs",
    "definitions",
    "dependentSchemas",
    "dependencies",
];
const SUBSCHEMA_KEYWORDS: [&str; 15] = [
    "items",
    "prefixItems",
    "additionalItems",
    "unevaluatedItems",
    "contains",
    "additionalProperties",
    "unevaluatedProperties",
    "propertyNames",
    "allOf",
    "anyOf",
    "oneOf",
    "not",
    "if",
    "then",
    "else",
];

/// `UploadedFile` as schemars emits it, split into its body and its `$defs`.
static UPLOADED_FILE_SCHEMA: LazyLock<(Value, Map<String, Value>)> = LazyLock::new(|| {
    let mut body = flow_like_types::json::to_value(schemars::schema_for!(UploadedFile))
        .unwrap_or_else(|_| json!({"type": "object"}));
    let defs = body
        .as_object_mut()
        .and_then(|schema| {
            schema.remove("$schema");
            schema.remove("$defs")
        })
        .and_then(|defs| match defs {
            Value::Object(defs) => Some(defs),
            _ => None,
        })
        .unwrap_or_default();
    (body, defs)
});

fn is_file_marker(schema: &Value) -> bool {
    schema.get("x-flow-like-type").and_then(Value::as_str) == Some(FILE_MARKER)
}

/// The host swaps every widget file handle for its stored `UploadedFile` before the run,
/// so each `file` marker is typed as that shape. Its definitions join the payload's own
/// definitions (renamed on a clash), which `action_context_schema` hoists to the root.
fn resolve_file_markers(payload: &Value) -> Value {
    let (body, file_defs) = &*UPLOADED_FILE_SCHEMA;
    let key = if payload.get("definitions").is_some() {
        "definitions"
    } else {
        "$defs"
    };
    let mut taken: BTreeSet<String> = payload
        .get(key)
        .and_then(Value::as_object)
        .map(|defs| defs.keys().cloned().collect())
        .unwrap_or_default();
    let renames: BTreeMap<&str, String> = file_defs
        .keys()
        .map(|name| {
            let free = (1..)
                .map(|n| match n {
                    1 => name.clone(),
                    n => format!("{name}_{n}"),
                })
                .find(|candidate| !taken.contains(candidate))
                .unwrap_or_else(|| name.clone());
            taken.insert(free.clone());
            (name.as_str(), free)
        })
        .collect();
    let mut file = body.clone();
    retarget_refs(&mut file, key, &renames);

    let mut resolved = payload.clone();
    if !replace_file_markers(&mut resolved, &file) {
        return resolved;
    }
    let Some(schema) = resolved.as_object_mut() else {
        return resolved;
    };
    let defs = schema
        .entry(key)
        .or_insert_with(|| Value::Object(Map::new()));
    if !defs.is_object() {
        *defs = Value::Object(Map::new());
    }
    if let Some(defs) = defs.as_object_mut() {
        for (name, def) in file_defs {
            let mut def = def.clone();
            retarget_refs(&mut def, key, &renames);
            defs.insert(renames[name.as_str()].clone(), def);
        }
    }
    resolved
}

fn replace_file_markers(schema: &mut Value, file: &Value) -> bool {
    if is_file_marker(schema) {
        let mut replacement = file.clone();
        if let Some(description) = schema.get("description") {
            replacement["description"] = description.clone();
        }
        *schema = replacement;
        return true;
    }
    let Some(object) = schema.as_object_mut() else {
        return false;
    };
    let mut replaced = false;
    for (keyword, value) in object.iter_mut() {
        let keyword = keyword.as_str();
        if SUBSCHEMA_MAP_KEYWORDS.contains(&keyword) {
            for child in value.as_object_mut().into_iter().flat_map(Map::values_mut) {
                replaced |= replace_file_markers(child, file);
            }
        } else if SUBSCHEMA_KEYWORDS.contains(&keyword) {
            match value {
                Value::Array(children) => {
                    for child in children {
                        replaced |= replace_file_markers(child, file);
                    }
                }
                child => replaced |= replace_file_markers(child, file),
            }
        }
    }
    replaced
}

fn retarget_refs(value: &mut Value, key: &str, renames: &BTreeMap<&str, String>) {
    match value {
        Value::Object(object) => {
            for (keyword, child) in object.iter_mut() {
                if keyword == "$ref"
                    && let Some(name) = child.as_str().and_then(|r| r.strip_prefix("#/$defs/"))
                    && let Some(renamed) = renames.get(name)
                {
                    *child = Value::String(format!("#/{key}/{renamed}"));
                } else {
                    retarget_refs(child, key, renames);
                }
            }
        }
        Value::Array(items) => {
            for item in items {
                retarget_refs(item, key, renames);
            }
        }
        _ => {}
    }
}

type ContextSchema = fn(&str) -> Option<Value>;

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

/// An exposed declarative action keeps the inner component's context. Infer it
/// only when every known emitter agrees; package payload schemas use another path.
pub(super) fn declarative_action_contexts(
    definition: &Value,
    depth: usize,
    remaining: &mut usize,
) -> BTreeMap<String, Option<Value>> {
    let mut contexts: BTreeMap<String, EventInference> = definition["actions"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|action| {
            action["id"]
                .as_str()
                .map(|id| (id.into(), EventInference::default()))
        })
        .collect();
    if contexts.is_empty() {
        return BTreeMap::new();
    }
    for component in definition["components"].as_array().into_iter().flatten() {
        infer_exposed_component(
            component.get("component").unwrap_or(component),
            depth,
            remaining,
            &mut contexts,
        );
    }
    contexts
        .into_iter()
        .map(|(name, inference)| (name, inference.context_schema().cloned()))
        .collect()
}

fn infer_exposed_component(
    component: &Value,
    depth: usize,
    remaining: &mut usize,
    contexts: &mut BTreeMap<String, EventInference>,
) {
    // Nested instances introduce another action scope. Until that scope can be
    // proven, its emitters cannot establish an outer action's field contract.
    if depth >= 32
        || *remaining == 0
        || matches!(
            component["type"].as_str(),
            Some("widgetInstance" | MICRO_WIDGET_COMPONENT_TYPE)
        )
    {
        for inference in contexts.values_mut() {
            inference.unknown_source();
        }
        return;
    }
    *remaining -= 1;
    let mut add_route = |action: &Value, schema: Option<Value>| {
        if action["name"] != "widget_event" {
            return;
        }
        let Some(name) = action["context"]["actionId"].as_str() else {
            for inference in contexts.values_mut() {
                inference.unknown_source();
            }
            return;
        };
        if let Some(inference) = contexts.get_mut(name) {
            match schema {
                Some(schema) => inference.add_context(name, schema),
                None => inference.add(name, None),
            }
        }
    };
    let typed_events: Option<(&[&str], ContextSchema)> = match component["type"].as_str() {
        Some("filePreview" | "fileEditor") => Some((
            &["save", "saved", "dirtyChange", "error"],
            file_preview_event_context_schema,
        )),
        Some("excalidraw") => Some((
            &["load", "save", "autoSave", "dirtyChange", "error"],
            excalidraw_event_context_schema,
        )),
        _ => None,
    };
    if let Some((events, context_schema)) = typed_events {
        for &event in events {
            for action in component["eventHandlers"][event]
                .as_array()
                .into_iter()
                .flatten()
            {
                add_route(action, context_schema(event));
            }
        }
    } else {
        for action in component["actions"]
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
        {
            add_route(action, None);
        }
    }
    if let Some(children) = component["children"].as_array() {
        for child in children.iter().filter(|child| child.is_object()) {
            infer_exposed_component(
                child.get("component").unwrap_or(child),
                depth + 1,
                remaining,
                contexts,
            );
        }
    }
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
                    None,
                );
                if contract.is_none() && self.inference.schemas.len() > before {
                    self.inference.unknown_source();
                }
            }
            Some("widgetInstance") => self.widget_instance(component, depth + 1),
            Some("filePreview" | "fileEditor") => self.file_preview(component),
            Some("excalidraw") => self.excalidraw(component),
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

    fn file_preview(&mut self, component: &Value) {
        self.exact_component_events(
            component,
            &["save", "saved", "dirtyChange", "error"],
            file_preview_event_context_schema,
        );
    }

    fn excalidraw(&mut self, component: &Value) {
        self.exact_component_events(
            component,
            &["load", "save", "autoSave", "dirtyChange", "error"],
            excalidraw_event_context_schema,
        );
    }

    fn exact_component_events(
        &mut self,
        component: &Value,
        event_names: &[&str],
        context_schema: ContextSchema,
    ) {
        // These events require exact handlers. Neither wildcard nor historical
        // component actions can cause the renderer to dispatch them.
        for &name in event_names {
            let targets = component["eventHandlers"][name]
                .as_array()
                .is_some_and(|actions| {
                    actions
                        .iter()
                        .any(|action| action_targets(action, self.target))
                });
            if targets && let Some(schema) = context_schema(name) {
                self.inference.add_context(name, schema);
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

    fn routes(
        &mut self,
        instance: &Value,
        events: Option<&Value>,
        declared_only: bool,
        direct_contexts: Option<&BTreeMap<String, Option<Value>>>,
    ) {
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
                if let Some(contexts) = direct_contexts {
                    match contexts.get(&name).and_then(Option::as_ref) {
                        Some(schema) => self.inference.add_context(&name, schema.clone()),
                        None => self.inference.add(&name, None),
                    }
                    continue;
                }
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
        let contexts = definition
            .map(|definition| declarative_action_contexts(definition, depth, &mut self.remaining));
        self.routes(instance, actions.as_ref(), false, contexts.as_ref());
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

    fn file_preview(event: &str) -> Value {
        json!({"type":"filePreview","eventHandlers":{event:[route()]}})
    }

    fn excalidraw(event: &str) -> Value {
        json!({"type":"excalidraw","eventHandlers":{event:[route()]}})
    }

    #[test]
    fn excalidraw_events_infer_exact_typed_contexts() {
        for event in ["load", "save", "autoSave", "dirtyChange", "error"] {
            let result = infer(&page(excalidraw(event)));
            let expected = excalidraw_event_context_schema(event).unwrap();
            assert_eq!(result.action_ids, BTreeSet::from([event.to_string()]));
            assert_eq!(result.context_schema(), Some(&expected));
            assert!(jsonschema::meta::is_valid(&expected));
        }
        let mut component = excalidraw("save");
        component["eventHandlers"]["autoSave"] = json!([route()]);
        assert_eq!(
            infer(&page(component.clone())).context_schema(),
            excalidraw_event_context_schema("save").as_ref()
        );
        component["eventHandlers"]["load"] = json!([route()]);
        assert!(infer(&page(component)).context_schema().is_none());
        for component in [
            json!({"type":"excalidraw","actions":[route()]}),
            json!({"type":"excalidraw","eventHandlers":{"*":[route()],"save":[]}}),
            excalidraw("change"),
            excalidraw("saved"),
        ] {
            assert!(!infer(&page(component)).has_sources());
        }
    }

    #[test]
    fn excalidraw_scene_contexts_validate_nested_scene_and_operation_fields() {
        let scene = json!({"type":"excalidraw","version":2,"source":"flow-like",
            "elements":[{"id":"shape-1","type":"rectangle"}],"appState":{},
            "files":{"image-1":{"id":"image-1","dataURL":"data:image/png;base64,..."}}});
        for event in ["load", "save", "autoSave"] {
            let schema = excalidraw_event_context_schema(event).unwrap();
            let mut context =
                json!({"actionId":event,"value":scene,"scene":scene,"nodeId":"handler"});
            if event != "load" {
                assert!(!jsonschema::is_valid(&schema, &context));
                context["changeCount"] = json!(3);
            }
            assert!(jsonschema::is_valid(&schema, &context));
            for invalid in [
                json!({"type":"invalid"}),
                json!({"elements":["invalid"]}),
                json!({"appState":[]}),
                json!({"files":{"image-1":"invalid"}}),
            ] {
                let mut malformed = context.clone();
                malformed["scene"]
                    .as_object_mut()
                    .unwrap()
                    .extend(invalid.as_object().unwrap().clone());
                assert!(!jsonschema::is_valid(&schema, &malformed));
            }
        }
        let error = excalidraw_event_context_schema("error").unwrap();
        for operation in ["load", "save", "autoSave"] {
            assert!(jsonschema::is_valid(
                &error,
                &json!({"actionId":"error","operation":operation,"message":"Failed"})
            ));
        }
        assert!(!jsonschema::is_valid(
            &error,
            &json!({"actionId":"error","operation":"change","message":"Failed"})
        ));
        let dirty = excalidraw_event_context_schema("dirtyChange").unwrap();
        assert!(jsonschema::is_valid(
            &dirty,
            &json!({"actionId":"dirtyChange","dirty":true})
        ));
        assert!(!jsonschema::is_valid(
            &dirty,
            &json!({"actionId":"dirtyChange","dirty":"true"})
        ));
    }

    #[test]
    fn excalidraw_bindings_respect_widget_nesting_and_target_scope() {
        let definition = json!({"components":[{"component":excalidraw("save")}]});
        for root in [
            page(
                json!({"type":"widgetInstance","instanceId":"outer","widgetId":"wrapper","inlineWidgetDef":definition}),
            ),
            json!({"content":[{"widget":{"instanceId":"outer","widgetId":"wrapper"}}],"widgetRefs":{"outer":definition}}),
        ] {
            assert_eq!(
                infer(&root).context_schema(),
                excalidraw_event_context_schema("save").as_ref()
            );
        }
        let mut component = excalidraw("save");
        component["eventHandlers"]["save"][0]["context"]["appId"] = json!("foreign");
        assert!(!infer(&page(component)).has_sources());
        assert!(
            infer(&with_component(excalidraw("save")))
                .context_schema()
                .is_none()
        );
    }

    #[test]
    fn exposed_excalidraw_actions_preserve_the_scene_context() {
        let mut instance = json!({"type":"widgetInstance","instanceId":"outer","widgetId":"drawing",
        "eventHandlers":{"persistDrawing":[route()]},
        "inlineWidgetDef":{"actions":[{"id":"persistDrawing"}],"components":[{"component":{
            "type":"excalidraw","eventHandlers":{
                "save":[{"name":"widget_event","context":{"actionId":"persistDrawing"}}],
                "autoSave":[{"name":"widget_event","context":{"actionId":"persistDrawing"}}]
            }
        }}]}});
        let expected = excalidraw_event_context_schema("save").unwrap();
        let result = infer(&page(instance.clone()));
        assert_eq!(result.action_ids, BTreeSet::from(["persistDrawing".into()]));
        assert_eq!(result.context_schema(), Some(&expected));
        let contexts = declarative_action_contexts(&instance["inlineWidgetDef"], 0, &mut 10_000);
        let mut instantiated = EventInference::default();
        instantiated.instantiated_contexts(&contexts, "persistDrawing");
        assert_eq!(instantiated.context_schema(), Some(&expected));
        instance["inlineWidgetDef"]["components"][0]["component"]["eventHandlers"]["load"] =
            json!([{"name":"widget_event","context":{"actionId":"persistDrawing"}}]);
        assert!(infer(&page(instance)).context_schema().is_none());
    }

    #[test]
    fn excalidraw_pending_component_seed_requires_the_exact_contract() {
        let mut seed = excalidraw_event_context_schema("autoSave").unwrap();
        assert!(!is_pending_component_context(&seed, "autoSave"));
        seed["x-flow-like-pending-component-event"] =
            json!({"componentType":"excalidraw","eventName":"autoSave"});
        assert!(is_pending_component_context(&seed, "autoSave"));
        assert!(!is_pending_component_context(&seed, "save"));
        seed["x-flow-like-pending-component-event"]["componentType"] = json!("filePreview");
        assert!(!is_pending_component_context(&seed, "autoSave"));
        seed["x-flow-like-pending-component-event"]["componentType"] = json!("excalidraw");
        seed["properties"]["changeCount"] = json!({"type":"string"});
        assert!(!is_pending_component_context(&seed, "autoSave"));
    }

    #[test]
    fn file_preview_events_infer_direct_contexts_without_a_widget_payload_wrapper() {
        for component_type in ["filePreview", "fileEditor"] {
            for event in ["save", "saved", "dirtyChange", "error"] {
                let mut component = file_preview(event);
                component["type"] = json!(component_type);
                let result = infer(&page(component));
                assert_eq!(result.action_ids, BTreeSet::from([event.to_string()]));
                let expected = file_preview_event_context_schema(event).unwrap();
                assert_eq!(result.context_schema(), Some(&expected));
                assert!(expected["properties"].get("payload").is_none());
                assert!(jsonschema::meta::is_valid(&expected));
                let mut seed = expected;
                seed["x-flow-like-pending-component-event"] =
                    json!({"componentType":component_type,"eventName":event});
                assert!(is_pending_component_context(&seed, event));
            }
        }
    }

    #[test]
    fn file_preview_context_refs_validate_the_actual_export_and_notification_shapes() {
        let file = json!({"name":"deck.pptx","size":123,"type":"application/octet-stream",
            "url":"https://temporary.example/deck.pptx","backendUrl":"https://temporary.example/deck.pptx"});
        let mut context = json!({"actionId":"save","src":"files/original.pptx","filename":"deck.pptx",
            "url":"https://temporary.example/deck.pptx","signedUrls":"https://temporary.example/deck.pptx",
            "file":file,"value":file,"nodeId":"handler"});
        let save = file_preview_event_context_schema("save").unwrap();
        assert!(jsonschema::is_valid(&save, &context));
        context["signedUrls"] = json!(["https://temporary.example/deck.pptx"]);
        assert!(!jsonschema::is_valid(&save, &context));
        context["signedUrls"] = json!("https://temporary.example/deck.pptx");
        context["file"]["size"] = json!("123");
        assert!(!jsonschema::is_valid(&save, &context));
        for (event, value) in [
            (
                "dirtyChange",
                json!({"actionId":"dirtyChange","src":"files/a.pptx","filename":"a.pptx","dirty":true}),
            ),
            (
                "error",
                json!({"actionId":"error","src":"files/a.pptx","filename":"a.pptx","message":"Upload failed"}),
            ),
        ] {
            let schema = file_preview_event_context_schema(event).unwrap();
            assert!(jsonschema::is_valid(&schema, &value));
            let mut missing = value;
            missing.as_object_mut().unwrap().remove("actionId");
            assert!(!jsonschema::is_valid(&schema, &missing));
        }
    }

    #[test]
    fn file_preview_save_and_saved_agree_but_mixed_contexts_remain_generic() {
        let mut component = file_preview("save");
        component["eventHandlers"]["saved"] = json!([route()]);
        let result = infer(&page(component.clone()));
        assert_eq!(
            result.action_ids,
            BTreeSet::from(["save".into(), "saved".into()])
        );
        assert_eq!(
            result.context_schema(),
            file_preview_event_context_schema("save").as_ref()
        );
        component["eventHandlers"]["error"] = json!([route()]);
        assert!(infer(&page(component)).context_schema().is_none());
        assert!(
            infer(&with_component(file_preview("save")))
                .context_schema()
                .is_none()
        );
        let root = json!({"components":[{"component":file_preview("save")},
            {"component":{"type":"button","eventHandlers":{"click":[route()]}}}]});
        assert!(infer(&root).context_schema().is_none());
    }

    #[test]
    fn file_preview_only_uses_its_four_exact_event_handlers() {
        for component in [
            json!({"type":"filePreview","actions":[route()]}),
            json!({"type":"filePreview","eventHandlers":{"*":[route()]}}),
            json!({"type":"filePreview","eventHandlers":{"*":[route()],"save":[]}}),
            json!({"type":"filePreview","eventHandlers":{"ready":[route()],"load":[route()]}}),
        ] {
            let result = infer(&page(component.clone()));
            assert!(!result.has_sources() && result.action_ids.is_empty());
            assert_eq!(
                infer(&with_component(component)).context_schema(),
                Some(&action_context_schema(&payload()))
            );
        }
    }

    #[test]
    fn file_preview_respects_target_scope_and_component_locations() {
        let mut component = file_preview("save");
        component["eventHandlers"]["save"][0]["context"]["appId"] = json!("foreign");
        assert!(!infer(&page(component)).has_sources());
        let root = json!({"components":[{"component":{"type":"text","props":{"example":file_preview("save")}}}],
            "widgetRefs":{"unplaced":{"components":[{"component":file_preview("save")}]}}});
        assert!(!infer(&root).has_sources());
    }

    #[test]
    fn file_preview_bindings_are_found_in_saved_and_inline_widget_definitions() {
        let definition = json!({"components":[{"component":file_preview("save")}]});
        for root in [
            page(
                json!({"type":"widgetInstance","instanceId":"outer","widgetId":"wrapper","inlineWidgetDef":definition}),
            ),
            json!({"content":[{"widget":{"instanceId":"outer","widgetId":"wrapper"}}],"widgetRefs":{"outer":definition}}),
            json!({"components":[{"component":{"type":"column","children":[file_preview("save")]}}]}),
        ] {
            assert_eq!(
                infer(&root).context_schema(),
                file_preview_event_context_schema("save").as_ref()
            );
        }
        let unresolved = json!({"components":[{"component":file_preview("save")},
            {"component":{"type":"widgetInstance","instanceId":"missing","widgetId":"unavailable"}}]});
        let result = infer(&unresolved);
        assert!(!result.is_complete() && result.context_schema().is_none());
    }

    fn exposed_file_widget() -> Value {
        json!({"type":"widgetInstance","instanceId":"outer","widgetId":"document",
        "eventHandlers":{"persistDocument":[route()]},
        "inlineWidgetDef":{"actions":[{"id":"persistDocument"}],"components":[{"component":{
            "type":"filePreview","eventHandlers":{
                "save":[{"name":"widget_event","context":{"actionId":"persistDocument"}}],
                "saved":[{"name":"widget_event","context":{"actionId":"persistDocument"}}]
            }
        }}]}})
    }

    #[test]
    fn exposed_file_events_keep_direct_context_through_named_and_legacy_alias_bindings() {
        let expected = file_preview_event_context_schema("save").unwrap();
        let mut instance = exposed_file_widget();
        for legacy in [false, true] {
            if legacy {
                instance.as_object_mut().unwrap().remove("eventHandlers");
                instance["actionBindings"] =
                    json!({"persistDocument":{"workflow":{"flowId":"handler"}}});
            }
            let result = infer(&page(instance.clone()));
            assert_eq!(
                result.action_ids,
                BTreeSet::from(["persistDocument".into()])
            );
            assert_eq!(result.context_schema(), Some(&expected));
        }
        let definition = instance
            .as_object_mut()
            .unwrap()
            .remove("inlineWidgetDef")
            .unwrap();
        let saved = json!({"content":[{"widget":instance}],"widgetRefs":{"outer":definition}});
        assert_eq!(infer(&saved).context_schema(), Some(&expected));
    }

    #[test]
    fn exposed_file_events_do_not_overpromise_with_mixed_or_unknown_emitters() {
        for emitter in [
            json!({"type":"button","actions":[{"name":"widget_event","context":{"actionId":"persistDocument"}}]}),
            json!({"type":"filePreview","eventHandlers":{"error":[{"name":"widget_event","context":{"actionId":"persistDocument"}}]}}),
            json!({"type":"button","actions":[{"name":"widget_event","context":{"actionId":{"path":"/action"}}}]}),
            json!({"type":"widgetInstance","instanceId":"missing","widgetId":"missing"}),
            json!({"type":"microWidgetInstance","packageId":"geo","widgetId":"map",
                "contract":{"events":{"persistDocument":{"contextSchema":file_preview_event_context_schema("save").unwrap()}}}}),
        ] {
            let mut instance = exposed_file_widget();
            instance["inlineWidgetDef"]["components"]
                .as_array_mut()
                .unwrap()
                .push(json!({"component":emitter}));
            assert!(infer(&page(instance)).context_schema().is_none());
        }
    }

    #[test]
    fn exposed_file_events_require_declared_aliases_and_exact_inner_handlers() {
        let mut instance = exposed_file_widget();
        instance["inlineWidgetDef"]["components"][0]["component"]["eventHandlers"] = json!({
            "*":[{"name":"widget_event","context":{"actionId":"persistDocument"}}],
            "load":[{"name":"widget_event","context":{"actionId":"persistDocument"}}]
        });
        assert!(infer(&page(instance)).context_schema().is_none());
        let mut instance = exposed_file_widget();
        instance["inlineWidgetDef"]["actions"] = json!([]);
        assert!(infer(&page(instance)).context_schema().is_none());
        let mut instance = exposed_file_widget();
        instance["eventHandlers"]["persistDocument"] = json!([]);
        instance["actionBindings"] = json!({"persistDocument":{"workflow":{"flowId":"handler"}}});
        assert!(!infer(&page(instance)).has_sources());
    }

    #[test]
    fn instantiated_declarative_file_events_use_final_contexts_and_keep_selection() {
        let instance = exposed_file_widget();
        let mut definition = instance["inlineWidgetDef"].clone();
        definition["actions"]
            .as_array_mut()
            .unwrap()
            .push(json!({"id":"cancel"}));
        let contexts = declarative_action_contexts(&definition, 0, &mut 10_000);
        let mut selected = EventInference::default();
        selected.instantiated_contexts(&contexts, "persistDocument");
        assert_eq!(
            selected.action_ids,
            BTreeSet::from(["cancel".into(), "persistDocument".into()])
        );
        assert_eq!(
            selected.context_schema(),
            file_preview_event_context_schema("save").as_ref()
        );
        let mut all = EventInference::default();
        all.instantiated_contexts(&contexts, "");
        assert!(all.context_schema().is_none());
        for (depth, mut remaining) in [(32, 10_000), (0, 0)] {
            let contexts = declarative_action_contexts(&definition, depth, &mut remaining);
            assert!(contexts["persistDocument"].is_none());
        }
    }

    #[test]
    fn pending_component_seed_must_match_the_canonical_selected_context() {
        let mut seed = file_preview_event_context_schema("save").unwrap();
        assert!(!is_pending_component_context(&seed, "save"));
        seed["x-flow-like-pending-component-event"] =
            json!({"componentType":"filePreview","eventName":"save"});
        assert!(is_pending_component_context(&seed, "save"));
        assert!(!is_pending_component_context(&seed, "saved"));
        seed["required"] = json!([]);
        assert!(!is_pending_component_context(&seed, "save"));
    }

    #[test]
    fn page_placed_package_event_infers_its_saved_payload_without_instantiation() {
        let result = infer(&page(micro()));
        assert_eq!(result.action_ids, BTreeSet::from(["selected".into()]));
        assert_eq!(
            result.context_schema(),
            Some(&action_context_schema(&payload()))
        );
    }

    #[test]
    fn all_reachable_payloads_must_agree_even_when_event_names_match() {
        let mut root = page(micro());
        let mut second = micro();
        root["components"]
            .as_array_mut()
            .unwrap()
            .push(json!({"component":second}));
        assert_eq!(
            infer(&root).context_schema(),
            Some(&action_context_schema(&payload()))
        );
        second["contract"]["events"]["selected"]["payloadSchema"] = json!({"type":"string"});
        root["components"][1]["component"] = second;
        assert!(infer(&root).context_schema().is_none());
        root["components"].as_array_mut().unwrap().pop();
        root["components"][0]["component"]["eventHandlers"]["moved"] = json!([route()]);
        let result = infer(&root);
        assert_eq!(
            result.action_ids,
            BTreeSet::from(["moved".into(), "selected".into()])
        );
        assert!(result.context_schema().is_none());
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
        assert_eq!(
            selected.context_schema(),
            Some(&action_context_schema(&payload()))
        );
        let mut catch_all = EventInference::default();
        catch_all.instantiated_events(&events, "");
        assert!(catch_all.context_schema().is_none());
        selected.add("untyped-page-route", None);
        assert!(selected.context_schema().is_none());
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
        assert_eq!(
            infer(&page(component)).context_schema(),
            Some(&action_context_schema(&payload()))
        );
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
            infer(&page(component.clone())).context_schema(),
            Some(&action_context_schema(&payload()))
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
        assert!(result.context_schema().is_none());
        component["actionBindings"] = json!({"moved":{"workflow":{"flowId":"other"}}});
        assert_eq!(
            infer(&page(component)).context_schema(),
            Some(&action_context_schema(&payload()))
        );
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
            assert_eq!(
                infer(&root).context_schema(),
                Some(&action_context_schema(&payload()))
            );
        }
        let typed = json!({"content":[{"widget":{"instanceId":"outer","widgetId":"wrapper","actionBindings":{"clicked":{"workflowEvent":{"event_id":"handler","context_mapping":{}}}}}}]});
        let result = infer(&typed);
        assert_eq!(result.action_ids, BTreeSet::from(["clicked".into()]));
        assert!(result.context_schema().is_none());
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
        assert_eq!(
            result.context_schema(),
            Some(&action_context_schema(&payload()))
        );
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
            infer(&page(component.clone())).context_schema(),
            Some(&action_context_schema(&payload()))
        );
        component["eventHandlers"] = json!({"moved":[route()]});
        let rebound = infer(&page(component.clone()));
        assert_eq!(rebound.action_ids, BTreeSet::from(["moved".into()]));
        assert_eq!(
            rebound.context_schema(),
            Some(&action_context_schema(
                &component["contract"]["events"]["moved"]["payloadSchema"]
            ))
        );
        component["eventHandlers"] = json!({});
        let unbound = infer(&page(component));
        assert!(unbound.action_ids.is_empty() && unbound.context_schema().is_none());
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
        assert!(result.action_ids.is_empty() && result.context_schema().is_none());
    }

    #[test]
    fn exact_empty_handlers_shadow_wildcards_and_wildcards_shadow_legacy_bindings() {
        let mut component = micro();
        component["eventHandlers"] = json!({"*":[route()],"moved":[]});
        component["actionBindings"] = json!({"selected":{"workflow":{"flowId":"other"}},"moved":{"workflow":{"flowId":"handler"}}});
        let result = infer(&page(component.clone()));
        assert_eq!(result.action_ids, BTreeSet::from(["selected".into()]));
        assert_eq!(
            result.context_schema(),
            Some(&action_context_schema(&payload()))
        );
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
            assert!(result.context_schema().is_none());
        }
        let mut component = micro();
        component["contract"]["events"]["selected"] = json!({});
        assert!(infer(&page(component)).context_schema().is_none());
    }

    #[test]
    fn package_contract_rejects_stale_routes_and_never_exposes_wildcard_as_an_event() {
        let mut component = micro();
        component["eventHandlers"] = json!({"undeclared":[route()],"*":[route()],"moved":[]});
        component["actionBindings"] = json!({"alsoUndeclared":{"workflow":{"flowId":"handler"}}});
        let result = infer(&page(component.clone()));
        assert_eq!(result.action_ids, BTreeSet::from(["selected".into()]));
        assert_eq!(
            result.context_schema(),
            Some(&action_context_schema(&payload()))
        );
        component.as_object_mut().unwrap().remove("contract");
        let result = infer(&page(component));
        assert!(result.action_ids.contains("undeclared") && result.context_schema().is_none());
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
            assert!(result.context_schema().is_none());
            let mut mixed = EventInference::default();
            mixed.add("selected", Some(&payload()));
            infer_page(
                &root,
                &target(),
                &BTreeMap::new(),
                &BTreeMap::new(),
                &mut mixed,
            );
            assert!(mixed.context_schema().is_none());
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
            assert!(result.is_complete() && result.context_schema().is_none());
        }
        for component in [
            json!({"type":"button","actions":[{"name":"workflow_event","context":{"nodeId":"other"}},route()]}),
            json!({"type":"button","eventHandlers":{"click":[{"name":"navigate_page","context":{"nodeId":"handler"}}]}}),
            json!({"type":"button","eventHandlers":{"click":[{"name":"workflow_event","context":{"nodeId":"handler","appId":"foreign"}}]}}),
        ] {
            assert_eq!(
                infer(&with_component(component)).context_schema(),
                Some(&action_context_schema(&payload()))
            );
        }
    }

    #[test]
    fn page_lifecycle_hooks_on_the_event_keep_the_context_generic() {
        for hook in ["onLoadEventId", "onUnloadEventId", "onIntervalEventId"] {
            let mut root = page(micro());
            root[hook] = json!("other");
            assert_eq!(
                infer(&root).context_schema(),
                Some(&action_context_schema(&payload()))
            );
            root[hook] = json!("handler");
            let result = infer(&root);
            assert_eq!(result.action_ids, BTreeSet::from(["selected".into()]));
            assert!(result.is_complete() && result.context_schema().is_none());
            let hook_only = infer(&json!({ hook: "handler", "components": [] }));
            assert!(hook_only.action_ids.is_empty() && hook_only.context_schema().is_none());
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
        assert_eq!(
            result.context_schema(),
            Some(&action_context_schema(&payload()))
        );
        root["components"][1]["component"]["eventHandlers"]["moved"] = json!([route()]);
        let result = infer_installed(&root);
        assert_eq!(
            result.action_ids,
            BTreeSet::from(["moved".into(), "selected".into()])
        );
        assert!(result.context_schema().is_none());
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

    fn file_marker() -> Value {
        json!({"type":"object","x-flow-like-type":"file"})
    }

    fn stored_file() -> Value {
        json!(UploadedFile {
            name: Some("report.pdf".into()),
            size: Some(1024),
            mime_type: Some("application/pdf".into()),
            url: Some("https://files.example/report.pdf".into()),
            flow_path: Some(flow_like_catalog_core::FlowPath::new(
                "tmp/user/viewer/apps/app/uploads/report.pdf".into(),
                "request-files".into(),
                None,
            )),
            ..Default::default()
        })
    }

    #[test]
    fn file_markers_resolve_to_the_stored_file_in_spread_fields_and_payload() {
        let mut attachment = file_marker();
        attachment["description"] = json!("Picked attachment");
        let payload = json!({"type":"object","properties":{
                "note":{"type":"string"},
                "attachment":attachment,
                "gallery":{"type":"array","items":file_marker()},
                "sample":{"type":"object","examples":[file_marker()]}},
            "required":["attachment"]});
        let schema = action_context_schema(&payload);
        assert!(jsonschema::meta::is_valid(&schema));

        let (body, defs) = &*UPLOADED_FILE_SCHEMA;
        let mut described = body.clone();
        described["description"] = json!("Picked attachment");
        let nested = &schema["properties"]["payload"];
        for fields in [&schema["properties"], &nested["properties"]] {
            assert_eq!(fields["attachment"], described);
            assert_eq!(fields["gallery"]["items"], *body);
            assert_eq!(fields["sample"]["examples"][0], file_marker());
        }
        assert_eq!(schema["$defs"], Value::Object(defs.clone()));
        assert!(nested.get("$defs").is_none());
        assert_eq!(schema.to_string().matches("x-flow-like-type").count(), 2);

        let file_pin = flow_like::flow::pin::Pin::schema_string_for::<UploadedFile>().unwrap();
        for field in [
            &schema["properties"]["attachment"],
            &nested["properties"]["gallery"]["items"],
        ] {
            let mut broken_out = field.clone();
            broken_out["$defs"] = schema["$defs"].clone();
            assert!(flow_like::flow::pin::schema_covers(
                &broken_out.to_string(),
                &file_pin
            ));
        }

        let context = |file: Value| {
            let fields = json!({"note":"hi","attachment":file,"gallery":[file]});
            let mut context = fields.clone();
            context["actionId"] = json!("sent");
            context["payload"] = fields;
            context
        };
        assert!(jsonschema::is_valid(&schema, &context(stored_file())));
        let mut broken = stored_file();
        broken["flowPath"] = json!({"path":"tmp/user/viewer/apps/app/report.pdf"});
        assert!(!jsonschema::is_valid(&schema, &context(broken)));
        assert!(!jsonschema::is_valid(
            &schema,
            &context(json!({"size":"big"}))
        ));
    }

    #[test]
    fn file_markers_in_definitions_join_them_without_clobbering_payload_names() {
        let payload = json!({"type":"object",
            "properties":{"file":{"$ref":"#/definitions/upload"},"path":{"$ref":"#/definitions/FlowPath"}},
            "definitions":{"upload":file_marker(),"FlowPath":{"type":"string"}}});
        let schema = action_context_schema(&payload);
        assert!(jsonschema::meta::is_valid(&schema));
        assert!(schema.get("$defs").is_none());
        let definitions = &schema["definitions"];
        assert_eq!(definitions["FlowPath"], json!({"type":"string"}));
        assert_eq!(
            definitions["FlowPath_2"],
            UPLOADED_FILE_SCHEMA.1["FlowPath"]
        );
        assert_eq!(
            definitions["upload"]["properties"]["flowPath"]["anyOf"][0]["$ref"],
            "#/definitions/FlowPath_2"
        );
        let fields = json!({"file":stored_file(),"path":"tmp/a.txt"});
        let mut context = fields.clone();
        context["actionId"] = json!("sent");
        context["payload"] = fields;
        assert!(jsonschema::is_valid(&schema, &context));
        context["file"]["flowPath"] = json!("tmp/a.txt");
        assert!(!jsonschema::is_valid(&schema, &context));
    }

    #[test]
    fn a_file_payload_spreads_the_stored_file_fields() {
        let schema = action_context_schema(&file_marker());
        assert!(jsonschema::meta::is_valid(&schema));
        assert_eq!(schema["required"], json!(["actionId", "payload"]));
        assert_eq!(
            schema["properties"]["payload"]["title"],
            "A2UIFileInputFile"
        );
        for field in ["name", "size", "type", "url", "flowPath"] {
            assert!(schema["properties"].get(field).is_some(), "{field}");
        }
        let mut context = stored_file();
        context["actionId"] = json!("uploaded");
        context["payload"] = stored_file();
        assert!(jsonschema::is_valid(&schema, &context));
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
            vec![None, Some(action_context_schema(&json!({"type":"object"})))]
        );
    }
}
