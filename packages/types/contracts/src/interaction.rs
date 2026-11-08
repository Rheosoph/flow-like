//! Action feedback attached to a captured browser or desktop image.

use serde::{Deserialize, Serialize};

/// Positions and rectangles are fractions of the associated image, independent of resizing.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct InteractionPoint {
    pub x: f64,
    pub y: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct InteractionRect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PointerSource {
    Observed,
    Dispatched,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InteractionPointer {
    pub position: InteractionPoint,
    pub source: PointerSource,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InteractionFocus {
    pub label: String,
    pub bounds: Option<InteractionRect>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InteractionPhase {
    Running,
    Completed,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InteractionAction {
    /// An action name only. Typed text, key values, and secrets must never be included.
    pub kind: String,
    pub phase: InteractionPhase,
    pub at_ms: u64,
    /// The intended location, which is not evidence of where an event landed.
    pub target: Option<InteractionPoint>,
    pub target_bounds: Option<InteractionRect>,
}

/// The desktop input rectangle corresponding to the captured image.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InteractionMapping {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InteractionObservation {
    pub captured_at_ms: u64,
    pub width: u32,
    pub height: u32,
    pub pointer: Option<InteractionPointer>,
    pub focus: Option<InteractionFocus>,
    pub action: Option<InteractionAction>,
    pub mapping: Option<InteractionMapping>,
}
