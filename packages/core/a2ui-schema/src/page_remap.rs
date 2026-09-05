//! Moving a page payload into a different id space.
//!
//! A `proto::Page` is mostly opaque: components, widget definitions, exposed
//! props and customization values are JSON blobs, and the ids that make the
//! page *work* — the node an `on_click` starts, the board it runs on, the page
//! a navigation targets — live inside them. Two operations move a page across
//! an id boundary, forking an app and instantiating a board from a template,
//! and both have to rewrite exactly the same set of references. They share this
//! walker so a reference discovered by one is covered for the other; the pair
//! previously drifted, which is how forked pages ended up firing the source
//! app's nodes.
//!
//! Callers own the *policy* — which id maps to what, and what the page's own id
//! and board become. This module owns the *coverage*.

use flow_like_types_proto::proto;
use serde_json::Value;

use super::id_refs::{
    IdRef, rewrite_json_ids, rewrite_prop_value_containers, rewrite_unkeyed_json_ids,
};

/// How a caller resolves a reference it recognizes.
///
/// `by_field` answers "this id sits under a field named `nodeId`/`boardId`/…,
/// what does it become?". `by_literal` answers the harder case: a payload that
/// decodes to a bare literal — an exposed prop's default, a customization value
/// — carries no field name at all, so the caller is asked whether the string
/// *is* one of the ids it just remapped. Returning `None` from either leaves
/// the value exactly as it was.
pub struct IdTranslators<'a> {
    pub by_field: &'a mut dyn FnMut(IdRef, &str) -> Option<String>,
    pub by_literal: &'a mut dyn FnMut(&str) -> Option<String>,
}

impl IdTranslators<'_> {
    fn field(&mut self, kind: IdRef, id: &str) -> Option<String> {
        (self.by_field)(kind, id)
    }

    fn apply_field(&mut self, kind: IdRef, id: &mut String) {
        if let Some(translated) = self.field(kind, id.as_str()) {
            *id = translated;
        }
    }
}

/// Rewrite every id reference inside a page payload.
///
/// Covers the page's board and its behaviour hooks (`on_load_event_id` and
/// friends are **node** ids despite the name), every component blob, every
/// widget instance placed on the page, and the widget definitions snapshotted
/// into `widget_refs`. The page's own `id` is the caller's to set: a fork
/// carries it through a map, an instantiation mints a fresh one.
///
/// Returns one entry per payload that could not be rewritten, formatted
/// `"<owner>: <reason>"`. A page that ships with an un-rewritten payload keeps
/// pointing at the source, so callers are expected to report these rather than
/// drop them.
pub fn remap_page_refs(page: &mut proto::Page, translators: &mut IdTranslators) -> Vec<String> {
    let mut unrewritten = Vec::new();

    if let Some(board_id) = page.board_id.as_mut() {
        translators.apply_field(IdRef::Board, board_id);
    }
    for hook in [
        page.on_load_event_id.as_mut(),
        page.on_unload_event_id.as_mut(),
        page.on_interval_event_id.as_mut(),
    ]
    .into_iter()
    .flatten()
    {
        translators.apply_field(IdRef::Node, hook);
    }

    for content in page.content.iter_mut() {
        match content.content_type.as_mut() {
            Some(proto::page_content::ContentType::Widget(instance)) => {
                remap_widget_instance(instance, translators, &mut unrewritten);
            }
            Some(proto::page_content::ContentType::Component(component)) => {
                remap_component(component, translators, &mut unrewritten);
            }
            _ => {}
        }
    }

    for component in page.components.iter_mut() {
        remap_component(component, translators, &mut unrewritten);
    }

    // Keys are widget *instance* ids — they address a slot on this page, not a
    // widget in the destination's id space, so they are left alone.
    for widget in page.widget_refs.values_mut() {
        remap_widget_def(widget, translators, &mut unrewritten);
    }

    unrewritten
}

/// Rewrite an embedded widget definition. A definition snapshotted into a page
/// is a typed proto whose blobs are opaque `bytes`, so each one has to be
/// handed to the walker by hand — see [`remap_widget_json`] for the same job on
/// the standalone `.widget` file, where those blobs take a different shape
/// again.
pub fn remap_widget_def(
    widget: &mut proto::Widget,
    translators: &mut IdTranslators,
    unrewritten: &mut Vec<String>,
) {
    translators.apply_field(IdRef::Widget, &mut widget.id);

    for component in widget.components.iter_mut() {
        remap_component(component, translators, unrewritten);
    }
    for prop in widget.exposed_props.iter_mut() {
        if let Some(default_value) = prop.default_value.as_mut() {
            record(
                unrewritten,
                &format!("{}.exposed_props[{}]", widget.id, prop.id),
                remap_value_blob(default_value, translators),
            );
        }
    }
    for option in widget.customization_options.iter_mut() {
        if let Some(default_value) = option.default_value.as_mut() {
            record(
                unrewritten,
                &format!("{}.customization_options[{}]", widget.id, option.id),
                remap_value_blob(default_value, translators),
            );
        }
    }
    // `data_model` is the widget's initial *state*, not binding configuration.
    // It gets the field-name pass only, matching how a standalone `.widget`
    // file is treated: a string in app data must not be rewritten just because
    // it happens to equal an id.
    for entry in widget.data_model.iter_mut() {
        record(
            unrewritten,
            &format!("{}.data_model[{}]", widget.id, entry.key),
            remap_keyed_blob(&mut entry.value, translators),
        );
    }
}

/// Rewrite a widget stored as **plain JSON** — the standalone `{id}.widget`
/// file, which `App::save_widget` writes by serializing the native `Widget`.
///
/// A field-name walk alone is not enough there. `ExposedProp::default_value`
/// and `CustomizationOption::default_value` are `Option<Vec<u8>>` with no
/// `serde_bytes`, so serde emits them as **arrays of integers**: an id inside a
/// widget's exposed-prop default is a list of numbers to any walker that reads
/// strings, and it would ship into the fork unchanged. Those arrays are
/// decoded, remapped and re-encoded here. Nothing else is: the native
/// `DataEntry::value` is a plain `Value`, and reading an app's `[3, 7, 12]` as
/// bytes would corrupt it.
pub fn remap_widget_json(value: &mut Value, translators: &mut IdTranslators) -> Vec<String> {
    let mut unrewritten = Vec::new();
    rewrite_json_ids(value, translators.by_field);
    remap_json_byte_blobs(value, translators, &mut unrewritten);
    unrewritten
}

/// The blob fields of a JSON-serialized widget, and the entry lists that hold
/// them. Deliberately only these two: everything else in the document, the
/// `data_model` included, is plain JSON the field-name walk already reaches, and
/// an app's own integer array must never be mistaken for a byte blob.
const BYTE_BLOB_CONTAINERS: [&str; 4] = [
    "exposedProps",
    "exposed_props",
    "customizationOptions",
    "customization_options",
];
const BYTE_BLOB_FIELDS: [&str; 2] = ["defaultValue", "default_value"];

fn remap_json_byte_blobs(
    value: &mut Value,
    translators: &mut IdTranslators,
    unrewritten: &mut Vec<String>,
) {
    let Value::Object(map) = value else {
        if let Value::Array(items) = value {
            for item in items.iter_mut() {
                remap_json_byte_blobs(item, translators, unrewritten);
            }
        }
        return;
    };
    for (key, child) in map.iter_mut() {
        if BYTE_BLOB_CONTAINERS.contains(&key.as_str())
            && let Value::Array(entries) = child
        {
            for entry in entries.iter_mut() {
                let Value::Object(entry) = entry else {
                    continue;
                };
                let owner = entry
                    .get("id")
                    .or_else(|| entry.get("key"))
                    .and_then(Value::as_str)
                    .unwrap_or(key)
                    .to_string();
                for field in BYTE_BLOB_FIELDS {
                    if let Some(blob) = entry.get_mut(field) {
                        record(unrewritten, &owner, remap_byte_array(blob, translators));
                    }
                }
            }
            continue;
        }
        remap_json_byte_blobs(child, translators, unrewritten);
    }
}

fn remap_byte_array(value: &mut Value, translators: &mut IdTranslators) -> Result<(), String> {
    let Value::Array(items) = value else {
        return Ok(());
    };
    let mut bytes = Vec::with_capacity(items.len());
    for item in items.iter() {
        let Some(byte) = item.as_u64().filter(|byte| *byte <= u8::MAX as u64) else {
            return Ok(());
        };
        bytes.push(byte as u8);
    }
    if bytes.is_empty() {
        return Ok(());
    }
    remap_value_blob(&mut bytes, translators)?;
    *value = Value::Array(bytes.into_iter().map(|byte| Value::from(byte)).collect());
    Ok(())
}

/// Translate every reference on a widget instance: the widget it renders, the
/// cross-app reference behind it, each action binding (target *and* the context
/// it passes along), and the value blobs the page customized it with.
fn remap_widget_instance(
    instance: &mut proto::WidgetInstance,
    translators: &mut IdTranslators,
    unrewritten: &mut Vec<String>,
) {
    translators.apply_field(IdRef::Widget, &mut instance.widget_id);

    // A `widget_ref` names an app *and* a widget, and the two only make sense
    // together. Translating the widget id of a genuinely third-party reference
    // would address a widget id minted for this id space inside an app that
    // knows nothing about it, so an app the caller does not recognize keeps
    // both halves. An empty app id is the same-app shorthand.
    if let Some(widget_ref) = instance.widget_ref.as_mut() {
        let translated_app = translators.field(IdRef::App, widget_ref.app_id.as_str());
        if translated_app.is_some() || widget_ref.app_id.is_empty() {
            if let Some(app_id) = translated_app {
                widget_ref.app_id = app_id;
            }
            translators.apply_field(IdRef::Widget, &mut widget_ref.widget_id);
        }
    }

    for binding in instance.action_bindings.values_mut() {
        match binding.binding_type.as_mut() {
            Some(proto::action_binding::BindingType::WorkflowEventId(node_id)) => {
                translators.apply_field(IdRef::Node, node_id);
            }
            Some(proto::action_binding::BindingType::PageId(page_id)) => {
                translators.apply_field(IdRef::Page, page_id);
            }
            _ => {}
        }
        for (field, bound) in binding.context_mapping.iter_mut() {
            remap_bound_value(field, bound, translators);
        }
    }

    for (prop_id, blob) in instance.customization_values.iter_mut() {
        record(
            unrewritten,
            &format!("{}.customization[{}]", instance.instance_id, prop_id),
            remap_value_blob(blob, translators),
        );
    }
    for (prop_id, blob) in instance.exposed_prop_values.iter_mut() {
        record(
            unrewritten,
            &format!("{}.exposed_prop[{}]", instance.instance_id, prop_id),
            remap_value_blob(blob, translators),
        );
    }
}

/// All real component data lives in `component_json`: `From<SurfaceComponent>`
/// serializes the component into those bytes and leaves the typed oneof unset.
fn remap_component(
    component: &mut proto::Component,
    translators: &mut IdTranslators,
    unrewritten: &mut Vec<String>,
) {
    let Some(bytes) = component.component_json.as_mut() else {
        return;
    };
    let outcome = remap_keyed_blob(bytes, translators);
    record(unrewritten, &component.id, outcome);
}

/// A context-mapping entry is named after the runtime context field it fills
/// (`nodeId`, `pageId`, …), so the field name decides what its literal means —
/// the same rule the JSON walker applies, on a typed `BoundValue` instead of a
/// JSON object. `literalJson` is re-parsed because the renderer re-parses it
/// too, and a `path` binding's `default_value` stands in for the same reference
/// whenever the path does not resolve.
fn remap_bound_value(field: &str, bound: &mut proto::BoundValue, translators: &mut IdTranslators) {
    if let Some(default_value) = bound.default_value.as_mut()
        && let Err(reason) = remap_value_blob(default_value, translators)
    {
        tracing::warn!("skip bound-value default for {field}: {reason}");
    }

    let Some(value) = bound.value.as_mut() else {
        return;
    };
    match value {
        proto::bound_value::Value::LiteralString(literal) => {
            let mut wrapped = serde_json::Map::new();
            wrapped.insert(field.to_string(), Value::String(literal.clone()));
            let mut wrapped = Value::Object(wrapped);
            rewrite_json_ids(&mut wrapped, translators.by_field);
            if let Some(translated) = wrapped.get(field).and_then(|value| value.as_str()) {
                *literal = translated.to_string();
            }
        }
        proto::bound_value::Value::LiteralJson(raw) => {
            let Ok(mut parsed) = serde_json::from_str::<Value>(raw) else {
                return;
            };
            rewrite_json_ids(&mut parsed, translators.by_field);
            if let Ok(encoded) = serde_json::to_string(&parsed) {
                *raw = encoded;
            }
        }
        _ => {}
    }
}

/// Field-name pass only, for payloads whose strings are user data rather than
/// binding configuration.
fn remap_keyed_blob(bytes: &mut Vec<u8>, translators: &mut IdTranslators) -> Result<(), String> {
    remap_blob(bytes, |value| {
        rewrite_json_ids(value, translators.by_field);
        // A widget instance written as a component keeps its exposed-prop and
        // customization values here, keyed by prop id — the same maps the typed
        // `WidgetInstance` holds as `bytes`. Same data, same coverage.
        rewrite_prop_value_containers(value, translators.by_literal);
    })
}

/// Field-name pass plus whole-string matching, for payloads keyed by prop id or
/// carrying a bare literal — there is no field name to key off, so the caller
/// decides per string whether it is one of the ids being remapped.
fn remap_value_blob(bytes: &mut Vec<u8>, translators: &mut IdTranslators) -> Result<(), String> {
    remap_blob(bytes, |value| {
        rewrite_unkeyed_json_ids(value, translators.by_field, translators.by_literal)
    })
}

fn remap_blob(bytes: &mut Vec<u8>, rewrite: impl FnOnce(&mut Value)) -> Result<(), String> {
    if bytes.is_empty() {
        return Ok(());
    }
    let mut value: Value =
        serde_json::from_slice(bytes).map_err(|err| format!("parse failed: {err}"))?;
    rewrite(&mut value);
    let encoded = serde_json::to_vec(&value).map_err(|err| format!("re-encode failed: {err}"))?;
    *bytes = encoded;
    Ok(())
}

fn record(unrewritten: &mut Vec<String>, owner: &str, outcome: Result<(), String>) {
    if let Err(reason) = outcome {
        unrewritten.push(format!("{owner}: {reason}"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::collections::HashMap;

    struct Fixture {
        nodes: HashMap<String, String>,
        pages: HashMap<String, String>,
        widgets: HashMap<String, String>,
        source_app: String,
        app: String,
    }

    impl Fixture {
        fn new() -> Self {
            Self {
                nodes: HashMap::from([("src_node".to_string(), "dst_node".to_string())]),
                pages: HashMap::from([("src_page".to_string(), "dst_page".to_string())]),
                widgets: HashMap::from([("src_widget".to_string(), "dst_widget".to_string())]),
                source_app: "src_app".to_string(),
                app: "dst_app".to_string(),
            }
        }

        fn remap(&self, page: &mut proto::Page) -> Vec<String> {
            let mut by_field = |kind: IdRef, id: &str| match kind {
                IdRef::Node => self.nodes.get(id).cloned(),
                IdRef::Page => self.pages.get(id).cloned(),
                IdRef::Widget => self.widgets.get(id).cloned(),
                IdRef::App => (id == self.source_app).then(|| self.app.clone()),
                _ => None,
            };
            let mut by_literal = |id: &str| {
                self.nodes
                    .get(id)
                    .or_else(|| self.pages.get(id))
                    .or_else(|| self.widgets.get(id))
                    .cloned()
            };
            let mut translators = IdTranslators {
                by_field: &mut by_field,
                by_literal: &mut by_literal,
            };
            remap_page_refs(page, &mut translators)
        }
    }

    fn blob(value: Value) -> Vec<u8> {
        serde_json::to_vec(&value).expect("serialize fixture")
    }

    fn decode(bytes: &[u8]) -> Value {
        serde_json::from_slice(bytes).expect("decode fixture")
    }

    fn component(id: &str, value: Value) -> proto::Component {
        proto::Component {
            id: id.to_string(),
            component_json: Some(blob(value)),
            ..Default::default()
        }
    }

    #[test]
    fn action_contexts_follow_the_new_id_space() {
        let mut page = proto::Page {
            on_load_event_id: Some("src_node".to_string()),
            ..Default::default()
        };
        page.components.push(component(
            "cta",
            json!({
                "eventHandlers": {
                    "click": [{
                        "name": "workflow_event",
                        "context": { "nodeId": "src_node", "appId": "src_app" }
                    }]
                }
            }),
        ));

        let unrewritten = Fixture::new().remap(&mut page);

        assert!(unrewritten.is_empty());
        assert_eq!(page.on_load_event_id.as_deref(), Some("dst_node"));
        let context = &decode(page.components[0].component_json.as_ref().unwrap())["eventHandlers"]
            ["click"][0]["context"];
        assert_eq!(context["nodeId"], "dst_node");
        assert_eq!(context["appId"], "dst_app");
    }

    #[test]
    fn a_third_party_widget_reference_keeps_both_halves() {
        let mut page = proto::Page::default();
        page.content.push(proto::PageContent {
            content_type: Some(proto::page_content::ContentType::Widget(
                proto::WidgetInstance {
                    widget_id: "src_widget".to_string(),
                    instance_id: "slot".to_string(),
                    widget_ref: Some(proto::WidgetRef {
                        app_id: "unrelated_app".to_string(),
                        widget_id: "src_widget".to_string(),
                        version: None,
                    }),
                    ..Default::default()
                },
            )),
            ..Default::default()
        });

        Fixture::new().remap(&mut page);

        let Some(proto::page_content::ContentType::Widget(instance)) =
            page.content[0].content_type.as_ref()
        else {
            panic!("widget content survives");
        };
        assert_eq!(instance.widget_id, "dst_widget");
        let widget_ref = instance.widget_ref.as_ref().expect("ref kept");
        assert_eq!(widget_ref.app_id, "unrelated_app");
        assert_eq!(widget_ref.widget_id, "src_widget");
    }

    #[test]
    fn same_app_widget_references_are_translated_as_a_pair() {
        let mut page = proto::Page::default();
        page.content.push(proto::PageContent {
            content_type: Some(proto::page_content::ContentType::Widget(
                proto::WidgetInstance {
                    widget_id: "src_widget".to_string(),
                    instance_id: "slot".to_string(),
                    widget_ref: Some(proto::WidgetRef {
                        app_id: "src_app".to_string(),
                        widget_id: "src_widget".to_string(),
                        version: None,
                    }),
                    ..Default::default()
                },
            )),
            ..Default::default()
        });

        Fixture::new().remap(&mut page);

        let Some(proto::page_content::ContentType::Widget(instance)) =
            page.content[0].content_type.as_ref()
        else {
            panic!("widget content survives");
        };
        let widget_ref = instance.widget_ref.as_ref().expect("ref kept");
        assert_eq!(widget_ref.app_id, "dst_app");
        assert_eq!(widget_ref.widget_id, "dst_widget");
    }

    #[test]
    fn value_blobs_keyed_by_prop_id_are_matched_whole() {
        let mut instance = proto::WidgetInstance {
            widget_id: "src_widget".to_string(),
            instance_id: "slot".to_string(),
            ..Default::default()
        };
        instance.exposed_prop_values.insert(
            "target".to_string(),
            blob(json!({ "literalString": "src_page" })),
        );
        instance
            .customization_values
            .insert("label".to_string(), blob(json!("Ingest files")));

        let mut page = proto::Page::default();
        page.content.push(proto::PageContent {
            content_type: Some(proto::page_content::ContentType::Widget(instance)),
            ..Default::default()
        });

        Fixture::new().remap(&mut page);

        let Some(proto::page_content::ContentType::Widget(instance)) =
            page.content[0].content_type.as_ref()
        else {
            panic!("widget content survives");
        };
        assert_eq!(
            decode(&instance.exposed_prop_values["target"])["literalString"],
            "dst_page"
        );
        assert_eq!(
            decode(&instance.customization_values["label"]),
            Value::String("Ingest files".to_string())
        );
    }

    #[test]
    fn an_unreadable_payload_is_reported() {
        let mut page = proto::Page::default();
        page.components.push(proto::Component {
            id: "broken".to_string(),
            component_json: Some(b"{not json".to_vec()),
            ..Default::default()
        });

        let unrewritten = Fixture::new().remap(&mut page);

        assert_eq!(unrewritten.len(), 1);
        assert!(unrewritten[0].starts_with("broken: "), "{unrewritten:?}");
    }
}
