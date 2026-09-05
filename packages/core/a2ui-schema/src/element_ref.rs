use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// The handle every a2ui node passes around to say "this element, on this
/// surface". Producers fill in what they know: a widget instantiation carries the
/// widget and surface it belongs to, while a plain element only carries its id.
/// Consumers read `instance_id` and fall back to `id`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ElementRef {
    /// Identifier of the element or widget instance.
    pub id: String,
    /// Instance the element lives in. Absent for elements addressed directly.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instance_id: Option<String>,
    /// Widget this instance was created from.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub widget_id: Option<String>,
    /// Surface the element is rendered on.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub surface_id: Option<String>,
    /// The component itself, whose props are defined by the component type.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub component: Option<Value>,
}
