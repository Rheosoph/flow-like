//! Plain-value authoring form for model-written A2UI.
//!
//! Models write `{"id":"t","type":"text","content":"Hi"}`; the canonical wire shape is
//! `{"id":"t","component":{"type":"text","content":{"literalString":"Hi"}}}`. The component
//! manifest — generated from the typed `*Props` structs and checked in as
//! `component-manifest.json` so the frontend lowers with the same rules — says which props are
//! bound values, which are plain fields and which are nested structures.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::{HashMap, HashSet};
use std::sync::LazyLock;

use crate::A2UIComponentType;

pub const MANIFEST_VERSION: u32 = 1;

const BOUND_VALUE_KEYS: [&str; 6] = [
    "literalString",
    "literalNumber",
    "literalBool",
    "literalJson",
    "literalOptions",
    "path",
];
const SURFACE_KEYS: [&str; 3] = ["id", "style", "eventRelevant"];
const MAX_DESCRIPTION_CHARS: usize = 240;
const MAX_FIELD_DEPTH: usize = 3;
/// Inline widget definitions nest their own component trees; the bound keeps a self-similar
/// payload from recursing without limit.
const MAX_INLINE_WIDGET_DEPTH: usize = 4;

/// Literal values the renderers understand for string-typed bound props. Mirrors the
/// `// "a" | "b"` unions in `packages/ui/components/a2ui/types.ts` (a frontend test keeps the two
/// in sync).
const PROP_VALUES: &[(&str, &str, &[&str])] = &[
    (
        "row",
        "align",
        &["start", "center", "end", "stretch", "baseline"],
    ),
    (
        "row",
        "justify",
        &["start", "center", "end", "between", "around", "evenly"],
    ),
    (
        "column",
        "align",
        &["start", "center", "end", "stretch", "baseline"],
    ),
    (
        "column",
        "justify",
        &["start", "center", "end", "between", "around", "evenly"],
    ),
    ("stack", "align", &["start", "center", "end", "stretch"]),
    (
        "grid",
        "autoFlow",
        &["row", "column", "dense", "rowDense", "columnDense"],
    ),
    (
        "scrollArea",
        "direction",
        &["vertical", "horizontal", "both"],
    ),
    (
        "text",
        "variant",
        &["body", "heading", "label", "caption", "code"],
    ),
    (
        "text",
        "size",
        &["xs", "sm", "md", "lg", "xl", "2xl", "3xl", "4xl"],
    ),
    (
        "text",
        "weight",
        &["light", "normal", "medium", "semibold", "bold"],
    ),
    ("text", "align", &["left", "center", "right", "justify"]),
    (
        "image",
        "fit",
        &["contain", "cover", "fill", "none", "scaleDown"],
    ),
    ("image", "loading", &["lazy", "eager"]),
    ("divider", "orientation", &["horizontal", "vertical"]),
    (
        "badge",
        "variant",
        &["default", "secondary", "destructive", "outline"],
    ),
    ("avatar", "size", &["sm", "md", "lg", "xl"]),
    (
        "progress",
        "variant",
        &["default", "success", "warning", "error"],
    ),
    ("spinner", "size", &["sm", "md", "lg"]),
    (
        "button",
        "variant",
        &[
            "default",
            "secondary",
            "outline",
            "ghost",
            "destructive",
            "link",
        ],
    ),
    ("button", "size", &["sm", "md", "lg", "icon"]),
    ("button", "iconPosition", &["left", "right"]),
    (
        "feedback",
        "mode",
        &["icon", "compact", "segmented", "rating", "extended"],
    ),
    ("feedback", "size", &["sm", "md", "lg"]),
    ("feedback", "commentMode", &["none", "inline", "modal"]),
    ("feedback", "pageContextMode", &["none", "path", "query"]),
    ("appLink", "target", &["config", "settings", "overview"]),
    (
        "appLink",
        "variant",
        &[
            "default",
            "secondary",
            "outline",
            "ghost",
            "destructive",
            "link",
        ],
    ),
    ("appLink", "size", &["sm", "md", "lg", "icon"]),
    ("appLink", "iconPosition", &["left", "right"]),
    (
        "textField",
        "inputType",
        &[
            "text", "email", "password", "number", "tel", "url", "search",
        ],
    ),
    ("radioGroup", "orientation", &["horizontal", "vertical"]),
    ("dateTimeInput", "mode", &["date", "time", "datetime"]),
    ("link", "target", &["_blank", "_self", "_parent", "_top"]),
    (
        "link",
        "variant",
        &["default", "muted", "primary", "destructive"],
    ),
    ("link", "underline", &["always", "hover", "none"]),
    ("card", "variant", &["default", "bordered", "elevated"]),
    ("modal", "size", &["sm", "md", "lg", "xl", "full"]),
    ("tabs", "orientation", &["horizontal", "vertical"]),
    ("tabs", "variant", &["default", "pills", "underline"]),
    ("drawer", "side", &["left", "right", "top", "bottom"]),
    ("tooltip", "side", &["top", "right", "bottom", "left"]),
    ("popover", "side", &["top", "right", "bottom", "left"]),
    ("popover", "trigger", &["click", "hover"]),
    (
        "shape",
        "shapeType",
        &["rectangle", "circle", "ellipse", "polygon", "line", "path"],
    ),
    ("scene3d", "cameraType", &["perspective", "orthographic"]),
    (
        "characterPortrait",
        "position",
        &["left", "right", "center"],
    ),
    ("characterPortrait", "size", &["small", "medium", "large"]),
    ("choiceMenu", "layout", &["vertical", "horizontal", "grid"]),
    ("healthBar", "variant", &["bar", "segmented", "circular"]),
    ("iframe", "loading", &["lazy", "eager"]),
    (
        "plotlyChart",
        "legendPosition",
        &["top", "bottom", "left", "right"],
    ),
    ("forecastChart", "view", &["forecast", "evaluation"]),
    (
        "filePreview",
        "fileType",
        &["pdf", "image", "video", "audio", "code", "text", "document"],
    ),
    (
        "filePreview",
        "fit",
        &["contain", "cover", "fill", "none", "scaleDown"],
    ),
    ("filePreview", "loading", &["lazy", "eager"]),
    ("diffView", "mode", &["split", "unified", "inline"]),
    (
        "diffView",
        "kind",
        &["auto", "text", "code", "markdown", "json", "document"],
    ),
    ("diffView", "markdownMode", &["source", "rendered"]),
    (
        "nivoChart",
        "legendPosition",
        &["top", "bottom", "left", "right"],
    ),
    (
        "nivoChart",
        "chartType",
        &[
            "bar",
            "line",
            "pie",
            "radar",
            "scatter",
            "heatmap",
            "funnel",
            "treemap",
            "sunburst",
            "waffle",
            "bump",
            "areaBump",
            "stream",
            "radialBar",
            "bullet",
            "boxplot",
            "swarmplot",
            "calendar",
            "chord",
            "sankey",
            "circlePacking",
            "network",
            "voronoi",
            "marimekko",
            "parallelCoordinates",
        ],
    ),
    ("boundingBoxOverlay", "fit", &["contain", "cover", "fill"]),
    (
        "imageHotspot",
        "markerStyle",
        &["pulse", "dot", "ring", "square", "diamond", "none"],
    ),
    ("imageHotspot", "fit", &["contain", "cover", "fill"]),
];

/// JSON shapes of bound props whose literal is structured, shown in prompt references.
const PROP_SHAPES: &[(&str, &str, &str)] = &[
    ("table", "columns", "[{id,header,accessor}]"),
    ("table", "data", "[{accessor: value}]"),
    ("nivoChart", "data", "[{indexBy field, one number per key}]"),
    ("nivoChart", "keys", "[series field]"),
    ("select", "options", "[{value,label}]"),
    ("radioGroup", "options", "[{value,label}]"),
];

/// Props the renderers ignore today. They stay valid on the wire but are left out of prompts.
const UNRENDERED_PROPS: &[(&str, &str)] = &[
    ("card", "headerImage"),
    ("card", "headerIcon"),
    ("image", "aspectRatio"),
    ("markdown", "allowHtml"),
    ("select", "multiple"),
    ("select", "searchable"),
    ("checkbox", "indeterminate"),
    ("geoMap", "clusterMarkers"),
    ("geoMap", "clusterRadius"),
    ("geoMap", "clusterMaxZoom"),
    ("widgetInstance", "styleOverride"),
    ("microWidgetInstance", "styleOverride"),
];

/// Components a model may write into an `a2ui` chat fence. Everything here renders from its own
/// props alone: no app context, no uploads, no embedded documents, nothing that covers the window.
pub const CHAT_UI_COMPONENTS: &[&str] = &[
    "column",
    "row",
    "grid",
    "stack",
    "box",
    "center",
    "spacer",
    "scrollArea",
    "aspectRatio",
    "text",
    "markdown",
    "image",
    "icon",
    "badge",
    "avatar",
    "progress",
    "divider",
    "skeleton",
    "card",
    "tabs",
    "accordion",
    "tooltip",
    "button",
    "link",
    "textField",
    "select",
    "slider",
    "checkbox",
    "switch",
    "radioGroup",
    "dateTimeInput",
    "table",
    "nivoChart",
];

/// The core of [`CHAT_UI_COMPONENTS`] the chat prompt teaches. The rest still render when a model
/// writes them, but they rarely help a reply and only make the prompt longer.
pub const CHAT_PROMPT_COMPONENTS: &[&str] = &[
    "column",
    "row",
    "grid",
    "box",
    "text",
    "markdown",
    "image",
    "icon",
    "badge",
    "avatar",
    "progress",
    "divider",
    "card",
    "tabs",
    "accordion",
    "tooltip",
    "button",
    "link",
    "textField",
    "select",
    "slider",
    "checkbox",
    "switch",
    "radioGroup",
    "dateTimeInput",
    "table",
    "nivoChart",
];

/// Actions a chat fence may carry. `send_message` posts `context.message` as the user's next chat
/// message; `external_link` opens `context.url`. Every other action is stripped before rendering.
pub const CHAT_UI_ACTIONS: &[&str] = &["send_message", "external_link"];

/// Props listed in the chat prompt for components whose full prop list would dominate it or
/// needs an app (routes, page state).
const CHAT_PROMPT_PROPS: &[(&str, &[&str])] = &[
    (
        "nivoChart",
        &[
            "chartType",
            "data",
            "indexBy",
            "keys",
            "title",
            "height",
            "colors",
            "showLegend",
        ],
    ),
    (
        "table",
        &[
            "columns", "data", "caption", "striped", "compact", "sortable",
        ],
    ),
    ("link", &["href", "label", "variant", "underline"]),
    (
        "card",
        &["title", "description", "footer", "variant", "padding"],
    ),
    (
        "button",
        &[
            "label",
            "variant",
            "size",
            "icon",
            "iconPosition",
            "disabled",
        ],
    ),
    (
        "textField",
        &[
            "value",
            "label",
            "placeholder",
            "helperText",
            "inputType",
            "multiline",
        ],
    ),
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PropKind {
    /// A `BoundValue`: plain JSON values are wrapped as literals, `{"path"}` objects stay bindings.
    Bound,
    /// A plain field (string, number, bool or enum) that is never wrapped.
    Plain,
    /// An array of structures; `fields` describes one item.
    List,
    /// A nested structure; `fields` describes it.
    Object,
    Style,
    /// Kept verbatim (maps, free-form JSON).
    Opaque,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PropSpec {
    pub name: String,
    pub kind: PropKind,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub required: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub values: Vec<String>,
    /// JSON shape of a structured literal, e.g. `[{value,label}]`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shape: Option<String>,
    /// Valid on the wire but ignored by the renderer; never advertised to models.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub unrendered: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fields: Vec<PropSpec>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ComponentSpec {
    #[serde(rename = "type")]
    pub component_type: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub props: Vec<PropSpec>,
}

impl ComponentSpec {
    pub fn prop(&self, name: &str) -> Option<&PropSpec> {
        self.props.iter().find(|prop| prop.name == name)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatUiPolicy {
    pub components: Vec<String>,
    pub actions: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ComponentManifest {
    pub version: u32,
    pub components: Vec<ComponentSpec>,
    pub chat: ChatUiPolicy,
}

impl ComponentManifest {
    pub fn component(&self, component_type: &str) -> Option<&ComponentSpec> {
        MANIFEST_INDEX
            .get(component_type)
            .and_then(|index| self.components.get(*index))
            .filter(|spec| spec.component_type == component_type)
            .or_else(|| {
                self.components
                    .iter()
                    .find(|spec| spec.component_type == component_type)
            })
    }
}

static MANIFEST: LazyLock<ComponentManifest> = LazyLock::new(|| {
    serde_json::from_str(include_str!("component-manifest.json"))
        .expect("the bundled A2UI component manifest must be valid JSON")
});

static MANIFEST_INDEX: LazyLock<HashMap<String, usize>> = LazyLock::new(|| {
    MANIFEST
        .components
        .iter()
        .enumerate()
        .map(|(index, spec)| (spec.component_type.clone(), index))
        .collect()
});

/// The checked-in manifest (`component-manifest.json`).
pub fn component_manifest() -> &'static ComponentManifest {
    &MANIFEST
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthoringError {
    pub index: usize,
    pub id: Option<String>,
    pub message: String,
}

impl std::fmt::Display for AuthoringError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.id {
            Some(id) => write!(f, "component '{id}': {}", self.message),
            None => write!(f, "component #{}: {}", self.index, self.message),
        }
    }
}

/// Lower one authored component (flat or nested, plain or wrapped values) to the canonical
/// `SurfaceComponent` JSON shape, including the trees of `widgetInstance.inlineWidgetDef`.
/// Unknown types and props are kept for the validators to judge.
pub fn lower_component(value: Value) -> Result<Value, String> {
    lower_component_at(value, 0)
}

fn lower_component_at(value: Value, depth: usize) -> Result<Value, String> {
    let Value::Object(mut object) = value else {
        return Err("a component must be a JSON object".into());
    };
    let mut component = match object.remove("component") {
        Some(Value::Object(component)) => component,
        Some(_) => return Err("`component` must be an object with a `type`".into()),
        None => {
            let keys: Vec<String> = object
                .keys()
                .filter(|key| !SURFACE_KEYS.contains(&key.as_str()))
                .cloned()
                .collect();
            let mut component = Map::new();
            for key in keys {
                if let Some(value) = object.remove(&key) {
                    component.insert(key, value);
                }
            }
            component
        }
    };
    let Some(component_type) = component
        .get("type")
        .and_then(Value::as_str)
        .map(str::to_owned)
    else {
        return Err("missing component `type`".into());
    };
    lower_component_props(&component_type, &mut component);
    if depth < MAX_INLINE_WIDGET_DEPTH {
        lower_inline_widget(&mut component, depth)?;
    }
    object.insert("component".into(), Value::Object(component));
    Ok(Value::Object(object))
}

fn lower_inline_widget(component: &mut Map<String, Value>, depth: usize) -> Result<(), String> {
    let Some(Value::Array(children)) = component
        .get_mut("inlineWidgetDef")
        .and_then(|definition| definition.get_mut("components"))
    else {
        return Ok(());
    };
    let mut lowered = Vec::with_capacity(children.len());
    for (index, child) in std::mem::take(children).into_iter().enumerate() {
        let child = lower_component_at(child, depth + 1)
            .map_err(|message| format!("inlineWidgetDef.components[{index}]: {message}"))?;
        lowered.push(child);
    }
    *children = lowered;
    Ok(())
}

/// Lower every component, collecting errors for the ones that cannot be lowered.
pub fn lower_components(values: Vec<Value>) -> (Vec<Value>, Vec<AuthoringError>) {
    let mut lowered = Vec::with_capacity(values.len());
    let mut errors = Vec::new();
    for (index, value) in values.into_iter().enumerate() {
        let id = value.get("id").and_then(Value::as_str).map(str::to_owned);
        match lower_component(value) {
            Ok(component) => lowered.push(component),
            Err(message) => errors.push(AuthoringError { index, id, message }),
        }
    }
    (lowered, errors)
}

/// Keys lowering leaves where they are: the discriminator, base identity, and the `action`
/// shorthand (folded into `actions` afterwards).
const UNLOWERED_KEYS: [&str; 4] = ["type", "id", "style", "action"];

fn lower_component_props(component_type: &str, component: &mut Map<String, Value>) {
    let spec = component_manifest().component(component_type);
    let keys: Vec<String> = component
        .keys()
        .filter(|key| !UNLOWERED_KEYS.contains(&key.as_str()))
        .cloned()
        .collect();
    for key in keys {
        let Some(value) = component.remove(&key) else {
            continue;
        };
        if let Some(lowered) = lower_prop(spec, &key, value) {
            component.insert(key, lowered);
        }
    }
    fold_action_shorthand(component);
}

/// One prop of a component; `None` removes it.
fn lower_prop(spec: Option<&ComponentSpec>, key: &str, value: Value) -> Option<Value> {
    match key {
        "actions" => Some(lower_actions(value)),
        "eventHandlers" => Some(lower_event_handlers(value)),
        "children" => lower_children(value),
        "hidden" => lower_bound("hidden", value),
        _ => match spec.and_then(|spec| spec.prop(key)) {
            Some(prop) => lower_by_spec(prop, value, 0),
            None => Some(value),
        },
    }
}

fn lower_event_handlers(handlers: Value) -> Value {
    match handlers {
        Value::Object(handlers) => Value::Object(
            handlers
                .into_iter()
                .map(|(event, actions)| (event, lower_actions(actions)))
                .collect(),
        ),
        other => other,
    }
}

fn fold_action_shorthand(component: &mut Map<String, Value>) {
    if let Some(action) = component.remove("action") {
        let actions = component
            .entry("actions")
            .or_insert_with(|| Value::Array(Vec::new()));
        if let (Value::Array(list), Value::Array(more)) = (actions, lower_actions(action)) {
            list.extend(more);
        }
    }
}

/// Normalize an action list: a single action becomes a one-item list, a bare name becomes
/// `{"name": …}`, and every action carries a `context` object. Entries that are not actions drop.
fn lower_actions(value: Value) -> Value {
    let items = match value {
        Value::Array(items) => items,
        other => vec![other],
    };
    Value::Array(items.into_iter().filter_map(lower_action).collect())
}

fn lower_action(value: Value) -> Option<Value> {
    let mut action = match value {
        Value::String(name) if !name.trim().is_empty() => {
            let mut action = Map::new();
            action.insert("name".into(), Value::String(name));
            action
        }
        Value::Object(action) if action.get("name").is_some_and(Value::is_string) => action,
        _ => return None,
    };
    action
        .entry("context")
        .or_insert_with(|| Value::Object(Map::new()));
    Some(Value::Object(action))
}

fn lower_by_spec(spec: &PropSpec, value: Value, depth: usize) -> Option<Value> {
    match spec.kind {
        PropKind::Bound => lower_bound(&spec.name, value),
        PropKind::Plain => Some(unwrap_literal(value)),
        PropKind::List => Some(match value {
            Value::Array(items) => Value::Array(
                items
                    .into_iter()
                    .map(|item| lower_fields(&spec.fields, item, depth + 1))
                    .collect(),
            ),
            other => other,
        }),
        PropKind::Object => Some(lower_fields(&spec.fields, value, depth + 1)),
        PropKind::Style | PropKind::Opaque => Some(value),
    }
}

fn lower_fields(fields: &[PropSpec], value: Value, depth: usize) -> Value {
    let Value::Object(mut object) = value else {
        return value;
    };
    if depth > MAX_FIELD_DEPTH || fields.is_empty() {
        return Value::Object(object);
    }
    for field in fields {
        if let Some(value) = object.remove(&field.name)
            && let Some(lowered) = lower_by_spec(field, value, depth)
        {
            object.insert(field.name.clone(), lowered);
        }
    }
    Value::Object(object)
}

/// Wrap a plain JSON value as a `BoundValue`; existing bound values pass through and `null`
/// removes the prop.
pub fn lower_bound(prop_name: &str, value: Value) -> Option<Value> {
    let wrapped = match value {
        Value::Null => return None,
        Value::String(text) => literal("literalString", Value::String(text)),
        Value::Number(number) => literal("literalNumber", Value::Number(number)),
        Value::Bool(flag) => literal("literalBool", Value::Bool(flag)),
        Value::Object(object) if is_bound_value(&object) => Value::Object(object),
        Value::Array(items) if prop_name == "options" => match select_options(&items) {
            Some(options) => literal("literalOptions", Value::Array(options)),
            None => literal(
                "literalJson",
                Value::String(Value::Array(items).to_string()),
            ),
        },
        other => literal("literalJson", Value::String(other.to_string())),
    };
    Some(wrapped)
}

/// Normalize children: an id array (or a single id) becomes `{"explicitList": [...]}`. Numeric
/// ids become strings; anything else in the list is not an id and drops.
pub fn lower_children(value: Value) -> Option<Value> {
    match value {
        Value::Null => None,
        Value::String(id) => Some(explicit_list(vec![Value::String(id)])),
        Value::Array(ids) => Some(explicit_list(
            ids.into_iter()
                .filter_map(|id| match id {
                    Value::String(_) => Some(id),
                    Value::Number(number) => Some(Value::String(number.to_string())),
                    _ => None,
                })
                .collect(),
        )),
        other => Some(other),
    }
}

fn explicit_list(ids: Vec<Value>) -> Value {
    let mut object = Map::new();
    object.insert("explicitList".into(), Value::Array(ids));
    Value::Object(object)
}

fn literal(key: &str, value: Value) -> Value {
    let mut object = Map::new();
    object.insert(key.into(), value);
    Value::Object(object)
}

fn is_bound_value(object: &Map<String, Value>) -> bool {
    BOUND_VALUE_KEYS.iter().any(|key| object.contains_key(*key))
}

fn unwrap_literal(value: Value) -> Value {
    match value {
        Value::Object(object) if object.len() == 1 => {
            let (key, inner) = object.iter().next().expect("one entry");
            if key.starts_with("literal") && key != "literalJson" && key != "literalOptions" {
                inner.clone()
            } else {
                Value::Object(object)
            }
        }
        other => other,
    }
}

fn select_options(items: &[Value]) -> Option<Vec<Value>> {
    items
        .iter()
        .map(|item| match item {
            Value::String(text) => Some(option(text.clone(), text.clone())),
            Value::Number(number) => Some(option(number.to_string(), number.to_string())),
            Value::Object(object) => {
                let value = scalar_text(object.get("value")?)?;
                let label = object
                    .get("label")
                    .and_then(scalar_text)
                    .unwrap_or_else(|| value.clone());
                Some(option(value, label))
            }
            _ => None,
        })
        .collect()
}

fn scalar_text(value: &Value) -> Option<String> {
    match value {
        Value::String(text) => Some(text.clone()),
        Value::Number(number) => Some(number.to_string()),
        Value::Bool(flag) => Some(flag.to_string()),
        _ => None,
    }
}

fn option(value: String, label: String) -> Value {
    let mut object = Map::new();
    object.insert("value".into(), Value::String(value));
    object.insert("label".into(), Value::String(label));
    Value::Object(object)
}

/// One compact signature line per component type: `- text: content*, variant(body|heading)`.
/// `*` marks required props, `(a|b)` the renderer's literal values, `=[...]` the JSON shape of a
/// structured literal, `[...]` the fields of list items and `{...}` nested objects. Unknown types
/// and unrendered props are skipped.
pub fn component_reference(types: &[&str]) -> String {
    reference_lines(types, false)
}

/// [`component_reference`] for [`CHAT_UI_COMPONENTS`], trimmed for chat: style-typed props and
/// props that need an app are left out.
pub fn chat_component_reference() -> String {
    reference_lines(CHAT_PROMPT_COMPONENTS, true)
}

fn reference_lines(types: &[&str], chat: bool) -> String {
    types
        .iter()
        .filter_map(|component_type| {
            let spec = component_manifest().component(component_type)?;
            let limit = chat
                .then(|| {
                    CHAT_PROMPT_PROPS
                        .iter()
                        .find(|(name, _)| name == component_type)
                        .map(|(_, props)| *props)
                })
                .flatten();
            Some(signature_line(spec, limit, chat))
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn signature_line(spec: &ComponentSpec, limit: Option<&[&str]>, chat: bool) -> String {
    let advertised = |prop: &&PropSpec| !prop.unrendered && !(chat && prop.kind == PropKind::Style);
    let props: Vec<&PropSpec> = match limit {
        Some(names) => names
            .iter()
            .filter_map(|name| spec.prop(name))
            .filter(advertised)
            .collect(),
        None => spec.props.iter().filter(advertised).collect(),
    };
    if props.is_empty() {
        return format!("- {}", spec.component_type);
    }
    format!(
        "- {}: {}",
        spec.component_type,
        props
            .iter()
            .map(|prop| prop_signature(prop))
            .collect::<Vec<_>>()
            .join(", ")
    )
}

fn prop_signature(prop: &PropSpec) -> String {
    let mut signature = prop.name.clone();
    if prop.required {
        signature.push('*');
    }
    if !prop.values.is_empty() {
        signature.push('(');
        signature.push_str(&prop.values.join("|"));
        signature.push(')');
    }
    if let Some(shape) = &prop.shape {
        signature.push('=');
        signature.push_str(shape);
    }
    let fields = || {
        prop.fields
            .iter()
            .filter(|field| !field.unrendered)
            .map(prop_signature)
            .collect::<Vec<_>>()
            .join(", ")
    };
    match prop.kind {
        PropKind::List if !prop.fields.is_empty() => {
            signature.push('[');
            signature.push_str(&fields());
            signature.push(']');
        }
        PropKind::Object if !prop.fields.is_empty() => {
            signature.push('{');
            signature.push_str(&fields());
            signature.push('}');
        }
        _ => {}
    }
    signature
}

const CHAT_UI_PROMPT_RULES: &str = r#"### Interactive UI (`a2ui` fence)
Reach for an `a2ui` fence whenever the reader will scan rather than read: key figures (weather, prices, KPIs, status), options to compare or pick from, a plan or timeline, a small dashboard, or a short form whose values feed your next turn. Lead with one sentence of prose, then the fence; keep explanations and single facts in Markdown. Tag the fence `a2ui`, never `json`.

One JSON object per line, root first: `{"id":"…","type":"…",…props}`. Values are plain JSON; `children` lists child ids; `style.className` takes Tailwind theme classes (bg-card, bg-muted, text-muted-foreground, border, rounded-xl, p-4, gap-3, w-full). Cards do not space their children: give a card one `column` child with a `gap`. Finish each line before the next and close the fence before prose; lines render as they arrive.
```a2ui
{"id":"root","type":"column","gap":"12px","children":["title","cards"]}
{"id":"title","type":"text","content":"Recipe Remix","variant":"heading","size":"xl"}
{"id":"cards","type":"grid","columns":2,"gap":"12px","children":["c1"]}
{"id":"c1","type":"card","title":"Bean & Rice Skillet","description":"25 min · serves 2","children":["c1-body"]}
{"id":"c1-body","type":"column","gap":"8px","children":["c1-tag","c1-go"]}
{"id":"c1-tag","type":"badge","content":"Vegetarian","variant":"secondary"}
{"id":"c1-go","type":"button","label":"Remix this","action":{"name":"send_message","context":{"message":"Remix the bean skillet"}}}
```
Actions: `send_message` posts `context.message` as the user's next message (`{{id}}` inserts that input's value); `external_link` opens `context.url`. Nothing else runs. Use only verified https image/link URLs.
Components (`*` required, `(a|b)` values, `[…]` item fields):"#;

/// The prompt section that teaches a model the `a2ui` chat fence: rules, one example and the
/// generated reference for [`CHAT_UI_COMPONENTS`].
pub fn chat_ui_prompt_section() -> String {
    format!("{CHAT_UI_PROMPT_RULES}\n{}", chat_component_reference())
}

/// Build the manifest from the typed component schemas. The checked-in
/// `component-manifest.json` must equal this (see the `manifest_is_current` test).
pub fn generate_component_manifest() -> ComponentManifest {
    let schema = serde_json::to_value(schemars::schema_for!(A2UIComponentType))
        .expect("the A2UI component schema serializes");
    let defs = schema
        .get("$defs")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    let variants = schema
        .get("oneOf")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let values: HashMap<(&str, &str), &[&str]> = PROP_VALUES
        .iter()
        .map(|(component, prop, values)| ((*component, *prop), *values))
        .collect();
    let components = variants
        .iter()
        .filter_map(|variant| component_spec(variant, &defs, &values))
        .collect();
    ComponentManifest {
        version: MANIFEST_VERSION,
        components,
        chat: ChatUiPolicy {
            components: CHAT_UI_COMPONENTS
                .iter()
                .map(|name| (*name).into())
                .collect(),
            actions: CHAT_UI_ACTIONS.iter().map(|name| (*name).into()).collect(),
        },
    }
}

fn component_spec(
    variant: &Value,
    defs: &Map<String, Value>,
    values: &HashMap<(&str, &str), &[&str]>,
) -> Option<ComponentSpec> {
    let component_type = variant
        .pointer("/properties/type/const")
        .or_else(|| variant.pointer("/properties/type/enum/0"))
        .and_then(Value::as_str)?
        .to_owned();
    let body = object_schema(variant, defs);
    let mut required: HashSet<String> = required_names(variant);
    required.extend(required_names(&body));
    let description = body
        .get("description")
        .or_else(|| variant.get("description"))
        .and_then(Value::as_str)
        .map(clean_description);
    let mut visited = HashSet::new();
    let props = body
        .get("properties")
        .and_then(Value::as_object)
        .map(|properties| {
            properties
                .iter()
                .filter(|(name, _)| name.as_str() != "type")
                .map(|(name, schema)| {
                    let mut prop =
                        prop_spec(name, schema, required.contains(name), defs, 0, &mut visited);
                    let key = (component_type.as_str(), name.as_str());
                    if let Some(known) = values.get(&key) {
                        prop.values = known.iter().map(|value| (*value).into()).collect();
                    }
                    prop.shape = PROP_SHAPES
                        .iter()
                        .find(|(component, prop, _)| (*component, *prop) == key)
                        .map(|(_, _, shape)| (*shape).into());
                    prop.unrendered = UNRENDERED_PROPS.contains(&key);
                    prop
                })
                .collect()
        })
        .unwrap_or_default();
    Some(ComponentSpec {
        component_type,
        description,
        props,
    })
}

/// Resolve a schema that may be a `$ref`, an `allOf` of refs or an inline object into one object
/// schema with merged `properties` and `required`.
fn object_schema(schema: &Value, defs: &Map<String, Value>) -> Value {
    let mut merged = Map::new();
    let mut properties = Map::new();
    let mut required: Vec<Value> = Vec::new();
    for part in schema_parts(schema, defs) {
        if let Some(own) = part.get("properties").and_then(Value::as_object) {
            properties.extend(
                own.iter()
                    .map(|(name, value)| (name.clone(), value.clone())),
            );
        }
        if let Some(own) = part.get("required").and_then(Value::as_array) {
            required.extend(own.iter().cloned());
        }
        if let Some(description) = part.get("description") {
            merged
                .entry("description")
                .or_insert_with(|| description.clone());
        }
    }
    merged.insert("properties".into(), Value::Object(properties));
    merged.insert("required".into(), Value::Array(required));
    Value::Object(merged)
}

/// The schema followed by every part it pulls in through `$ref` and `allOf`, each definition once.
fn schema_parts(schema: &Value, defs: &Map<String, Value>) -> Vec<Value> {
    let mut parts = Vec::new();
    let mut stack = vec![schema.clone()];
    let mut seen = HashSet::new();
    while let Some(current) = stack.pop() {
        if let Some(name) = ref_name(&current)
            && seen.insert(name.to_owned())
            && let Some(def) = defs.get(name)
        {
            stack.push(def.clone());
        }
        if let Some(all_of) = current.get("allOf").and_then(Value::as_array) {
            stack.extend(all_of.iter().cloned());
        }
        parts.push(current);
    }
    parts
}

fn required_names(schema: &Value) -> HashSet<String> {
    schema
        .get("required")
        .and_then(Value::as_array)
        .map(|names| {
            names
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

fn prop_spec(
    name: &str,
    schema: &Value,
    required: bool,
    defs: &Map<String, Value>,
    depth: usize,
    visited: &mut HashSet<String>,
) -> PropSpec {
    let description = schema
        .get("description")
        .and_then(Value::as_str)
        .map(clean_description);
    let target = non_null(schema);
    let (kind, values, fields) = classify(target, defs, depth, visited);
    PropSpec {
        name: name.to_owned(),
        kind,
        required,
        description,
        values,
        shape: None,
        unrendered: false,
        fields,
    }
}

/// A prop's kind, its literal values and, for structures, its fields.
type Classified = (PropKind, Vec<String>, Vec<PropSpec>);

fn kind_only(kind: PropKind) -> Classified {
    (kind, Vec::new(), Vec::new())
}

fn classify(
    schema: &Value,
    defs: &Map<String, Value>,
    depth: usize,
    visited: &mut HashSet<String>,
) -> Classified {
    if let Some(name) = ref_name(schema) {
        return classify_ref(name, defs, depth, visited);
    }
    if let Some(values) = enum_values(schema) {
        return (PropKind::Plain, values, Vec::new());
    }
    match primary_type(schema) {
        Some("array") => classify_array(schema, defs, depth, visited),
        Some("string" | "number" | "integer" | "boolean") => kind_only(PropKind::Plain),
        _ => kind_only(PropKind::Opaque),
    }
}

fn classify_ref(
    name: &str,
    defs: &Map<String, Value>,
    depth: usize,
    visited: &mut HashSet<String>,
) -> Classified {
    match name {
        "BoundValue" => kind_only(PropKind::Bound),
        "Style" => kind_only(PropKind::Style),
        _ => match defs.get(name) {
            Some(def) => classify_def(name, def, defs, depth, visited),
            None => kind_only(PropKind::Opaque),
        },
    }
}

/// Arrays of structures are lists with item fields; arrays of scalars are plain values.
fn classify_array(
    schema: &Value,
    defs: &Map<String, Value>,
    depth: usize,
    visited: &mut HashSet<String>,
) -> Classified {
    let item = schema.get("items").map(non_null).and_then(ref_name);
    match item {
        None => kind_only(PropKind::Plain),
        Some("BoundValue" | "Style") => kind_only(PropKind::Opaque),
        Some(item) => match defs.get(item) {
            Some(def) if def.get("properties").is_some() => (
                PropKind::List,
                Vec::new(),
                struct_fields(item, def, defs, depth, visited),
            ),
            _ => kind_only(PropKind::Plain),
        },
    }
}

fn classify_def(
    name: &str,
    def: &Value,
    defs: &Map<String, Value>,
    depth: usize,
    visited: &mut HashSet<String>,
) -> (PropKind, Vec<String>, Vec<PropSpec>) {
    if let Some(values) = enum_values(def) {
        return (PropKind::Plain, values, Vec::new());
    }
    if def.get("properties").is_some() {
        let fields = struct_fields(name, def, defs, depth, visited);
        return (PropKind::Object, Vec::new(), fields);
    }
    match primary_type(def) {
        Some("string" | "number" | "integer" | "boolean") => {
            (PropKind::Plain, Vec::new(), Vec::new())
        }
        _ => (PropKind::Opaque, Vec::new(), Vec::new()),
    }
}

fn struct_fields(
    name: &str,
    def: &Value,
    defs: &Map<String, Value>,
    depth: usize,
    visited: &mut HashSet<String>,
) -> Vec<PropSpec> {
    if depth >= MAX_FIELD_DEPTH || !visited.insert(name.to_owned()) {
        return Vec::new();
    }
    let body = object_schema(def, defs);
    let required = required_names(&body);
    let fields = body
        .get("properties")
        .and_then(Value::as_object)
        .map(|properties| {
            properties
                .iter()
                .map(|(field, schema)| {
                    prop_spec(
                        field,
                        schema,
                        required.contains(field),
                        defs,
                        depth + 1,
                        visited,
                    )
                })
                .collect()
        })
        .unwrap_or_default();
    visited.remove(name);
    fields
}

/// Strip the `null` alternative schemars adds for `Option<T>`.
fn non_null(schema: &Value) -> &Value {
    for key in ["anyOf", "oneOf"] {
        if let Some(options) = schema.get(key).and_then(Value::as_array) {
            let mut rest = options.iter().filter(|option| !is_null_schema(option));
            if let (Some(only), None) = (rest.next(), rest.next()) {
                return only;
            }
        }
    }
    schema
}

fn is_null_schema(schema: &Value) -> bool {
    schema.get("type").and_then(Value::as_str) == Some("null")
        || schema.get("const").is_some_and(Value::is_null)
}

fn primary_type(schema: &Value) -> Option<&str> {
    match schema.get("type")? {
        Value::String(kind) => Some(kind.as_str()),
        Value::Array(kinds) => kinds
            .iter()
            .filter_map(Value::as_str)
            .find(|kind| *kind != "null"),
        _ => None,
    }
}

fn ref_name(schema: &Value) -> Option<&str> {
    schema
        .get("$ref")
        .and_then(Value::as_str)
        .and_then(|reference| reference.rsplit('/').next())
}

fn enum_values(schema: &Value) -> Option<Vec<String>> {
    if let Some(values) = schema.get("enum").and_then(Value::as_array) {
        let names: Vec<String> = values
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_owned)
            .collect();
        return (!names.is_empty()).then_some(names);
    }
    let options = schema.get("oneOf").and_then(Value::as_array)?;
    let names: Vec<String> = options
        .iter()
        .filter_map(|option| option.get("const").and_then(Value::as_str))
        .map(str::to_owned)
        .collect();
    (names.len() == options.len() && !names.is_empty()).then_some(names)
}

fn clean_description(text: &str) -> String {
    let paragraph = text.split("\n\n").next().unwrap_or(text);
    let collapsed = paragraph.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.chars().count() <= MAX_DESCRIPTION_CHARS {
        return collapsed;
    }
    let mut cut: String = collapsed.chars().take(MAX_DESCRIPTION_CHARS - 1).collect();
    cut.push('…');
    cut
}

/// Pretty JSON for the checked-in manifest file (tab-free, trailing newline).
pub fn manifest_json(manifest: &ComponentManifest) -> String {
    let mut text = serde_json::to_string_pretty(manifest).expect("the manifest serializes");
    text.push('\n');
    text
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn manifest_is_current() {
        let generated = manifest_json(&generate_component_manifest());
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/src/component-manifest.json");
        if std::env::var_os("A2UI_WRITE_MANIFEST").is_some() {
            std::fs::write(path, &generated).expect("write the component manifest");
            return;
        }
        assert!(
            include_str!("component-manifest.json") == generated,
            "component-manifest.json is stale; regenerate with \
             `A2UI_WRITE_MANIFEST=1 cargo test -p flow-like-a2ui-schema manifest_is_current`"
        );
    }

    #[test]
    fn manifest_covers_every_component_variant() {
        let manifest = component_manifest();
        for component_type in [
            "text",
            "button",
            "card",
            "tabs",
            "table",
            "nivoChart",
            "widgetInstance",
        ] {
            assert!(
                manifest.component(component_type).is_some(),
                "manifest is missing {component_type}"
            );
        }
        let text = manifest.component("text").unwrap();
        let content = text.prop("content").unwrap();
        assert_eq!(content.kind, PropKind::Bound);
        assert!(content.required);
        assert_eq!(
            text.prop("variant").unwrap().values,
            vec!["body", "heading", "label", "caption", "code"]
        );
        let overlay = manifest.component("overlay").unwrap();
        assert_eq!(
            overlay.prop("baseComponentId").unwrap().kind,
            PropKind::Plain
        );
        let overlays = overlay.prop("overlays").unwrap();
        assert_eq!(overlays.kind, PropKind::List);
        assert_eq!(
            overlays
                .fields
                .iter()
                .find(|field| field.name == "anchor")
                .map(|field| field.kind),
            Some(PropKind::Bound)
        );
        for component_type in CHAT_UI_COMPONENTS {
            assert!(
                manifest.component(component_type).is_some(),
                "chat component {component_type} is not a known component type"
            );
        }
    }

    #[test]
    fn flat_plain_component_lowers_to_the_canonical_shape() {
        let lowered = lower_component(json!({
            "id": "title",
            "type": "text",
            "content": "Recipe Remix",
            "size": "xl",
            "maxLines": 2,
            "truncate": true,
            "style": {"className": "mb-2"}
        }))
        .unwrap();
        assert_eq!(
            lowered,
            json!({
                "id": "title",
                "style": {"className": "mb-2"},
                "component": {
                    "type": "text",
                    "content": {"literalString": "Recipe Remix"},
                    "size": {"literalString": "xl"},
                    "maxLines": {"literalNumber": 2},
                    "truncate": {"literalBool": true}
                }
            })
        );
    }

    #[test]
    fn canonical_components_pass_through_unchanged() {
        let canonical = json!({
            "id": "root",
            "component": {
                "type": "column",
                "gap": {"literalString": "8px"},
                "children": {"explicitList": ["a", "b"]}
            }
        });
        assert_eq!(lower_component(canonical.clone()).unwrap(), canonical);
    }

    #[test]
    fn children_bindings_actions_and_options_lower() {
        let lowered = lower_component(json!({
            "id": "pick",
            "type": "select",
            "value": {"path": "$.choice", "defaultValue": "a"},
            "options": ["Small", {"value": 2, "label": "Two"}],
            "hidden": false,
            "children": ["x"],
            "action": {"name": "send_message", "context": {"message": "go"}}
        }))
        .unwrap();
        let component = &lowered["component"];
        assert_eq!(
            component["value"],
            json!({"path": "$.choice", "defaultValue": "a"})
        );
        assert_eq!(
            component["options"],
            json!({"literalOptions": [
                {"value": "Small", "label": "Small"},
                {"value": "2", "label": "Two"}
            ]})
        );
        assert_eq!(component["hidden"], json!({"literalBool": false}));
        assert_eq!(component["children"], json!({"explicitList": ["x"]}));
        assert_eq!(
            component["actions"],
            json!([{"name": "send_message", "context": {"message": "go"}}])
        );
        assert!(component.get("action").is_none());
    }

    #[test]
    fn action_names_numeric_child_ids_and_handler_lists_normalize() {
        let lowered = lower_component(json!({
            "id": "go",
            "type": "button",
            "label": "Go",
            "action": "send_message",
            "eventHandlers": {"click": {"name": "external_link", "context": {"url": "https://a.b"}}},
            "children": [1, "two", null]
        }))
        .unwrap();
        let component = &lowered["component"];
        assert_eq!(
            component["actions"],
            json!([{"name": "send_message", "context": {}}])
        );
        assert_eq!(
            component["eventHandlers"],
            json!({"click": [{"name": "external_link", "context": {"url": "https://a.b"}}]})
        );
        assert_eq!(component["children"], json!({"explicitList": ["1", "two"]}));
    }

    #[test]
    fn inline_widget_trees_lower_and_report_their_failing_child() {
        let lowered = lower_component(json!({
            "id": "w",
            "type": "widgetInstance",
            "instanceId": "w",
            "widgetId": "card",
            "inlineWidgetDef": {
                "rootComponentId": "t",
                "components": [{"id": "t", "type": "text", "content": "Inside"}]
            }
        }))
        .unwrap();
        assert_eq!(
            lowered["component"]["inlineWidgetDef"]["components"][0]["component"]["content"],
            json!({"literalString": "Inside"})
        );
        let error = lower_component(json!({
            "id": "w",
            "type": "widgetInstance",
            "inlineWidgetDef": {"components": [{"id": "bad"}]}
        }))
        .unwrap_err();
        assert!(
            error.starts_with("inlineWidgetDef.components[0]: "),
            "{error}"
        );
    }

    #[test]
    fn structured_props_keep_plain_fields_and_wrap_nested_bound_values() {
        let lowered = lower_component(json!({
            "id": "o",
            "type": "overlay",
            "baseComponentId": {"literalString": "base"},
            "overlays": [{"componentId": "badge", "anchor": "topRight"}]
        }))
        .unwrap();
        let component = &lowered["component"];
        assert_eq!(component["baseComponentId"], json!("base"));
        assert_eq!(
            component["overlays"],
            json!([{"componentId": "badge", "anchor": {"literalString": "topRight"}}])
        );
    }

    #[test]
    fn json_valued_bound_props_become_literal_json() {
        let lowered = lower_component(json!({
            "id": "t",
            "type": "table",
            "data": [{"name": "A", "value": 1}]
        }))
        .unwrap();
        let data = &lowered["component"]["data"];
        let text = data["literalJson"].as_str().expect("literalJson string");
        assert_eq!(
            serde_json::from_str::<Value>(text).unwrap(),
            json!([{"name": "A", "value": 1}])
        );
    }

    #[test]
    fn invalid_components_report_errors_without_dropping_the_rest() {
        let (lowered, errors) = lower_components(vec![
            json!({"id": "ok", "type": "divider"}),
            json!({"id": "bad", "content": "no type"}),
            json!("not an object"),
        ]);
        assert_eq!(lowered.len(), 1);
        assert_eq!(errors.len(), 2);
        assert_eq!(errors[0].id.as_deref(), Some("bad"));
        assert_eq!(errors[1].index, 2);
    }

    #[test]
    fn reference_lines_mark_required_props_and_values() {
        let reference = component_reference(&["text", "button", "notAComponent"]);
        let lines: Vec<&str> = reference.lines().collect();
        assert_eq!(lines.len(), 2);
        assert!(lines[0].starts_with("- text: content*, variant(body|heading|label|caption|code)"));
        assert!(lines[1].starts_with("- button: label*"));
    }

    #[test]
    fn chat_prompt_section_is_compact_and_lists_every_chat_component() {
        let section = chat_ui_prompt_section();
        for component_type in CHAT_PROMPT_COMPONENTS {
            assert!(
                CHAT_UI_COMPONENTS.contains(component_type),
                "the prompt teaches {component_type}, which chat replies do not render"
            );
            assert!(
                section.contains(&format!("\n- {component_type}")),
                "chat prompt does not list {component_type}"
            );
        }
        assert_eq!(section.matches("```a2ui").count(), 1);
        assert!(section.contains("columns*=[{id,header,accessor}]"));
        for hidden in [
            "headerImage",
            "allowHtml",
            "searchable",
            "listStyle",
            "route",
        ] {
            assert!(!section.contains(hidden), "chat prompt advertises {hidden}");
        }
        assert!(
            section.len() <= 5_000,
            "chat UI prompt section grew to {} bytes",
            section.len()
        );
        if let Some(path) = std::env::var_os("A2UI_PROMPT_DUMP") {
            std::fs::write(path, &section).expect("write the prompt dump");
        }
    }

    #[test]
    fn chat_prompt_example_lowers_cleanly() {
        let example: Vec<Value> = CHAT_UI_PROMPT_RULES
            .split("```a2ui\n")
            .nth(1)
            .and_then(|rest| rest.split("\n```").next())
            .expect("the prompt carries an example fence")
            .lines()
            .map(|line| serde_json::from_str(line).expect("each example line is JSON"))
            .collect();
        let (lowered, errors) = lower_components(example);
        assert!(errors.is_empty(), "{errors:?}");
        for component in &lowered {
            let component_type = component["component"]["type"].as_str().unwrap();
            assert!(CHAT_UI_COMPONENTS.contains(&component_type));
        }
    }
}
