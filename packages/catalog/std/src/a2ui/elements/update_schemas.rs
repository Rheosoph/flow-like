//! Input schemas for unified A2UI update nodes
//!
//! These structs provide typed inputs for the various property types
//! that can be updated on A2UI elements.

use flow_like_types::json::{Deserialize, Serialize};
use schemars::JsonSchema;

// =============================================================================
// GeoMap Input Schemas
// =============================================================================

/// Coordinate on the map
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, Default)]
#[serde(rename_all = "camelCase")]
pub struct GeoCoordinate {
    pub latitude: f64,
    pub longitude: f64,
}

/// A single marker on the map
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct GeoMapMarker {
    /// Unique identifier for this marker
    pub id: String,
    /// Marker position
    pub coordinate: GeoCoordinate,
    /// Marker color (red, blue, green, yellow, orange, purple, pink, gray)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    /// Label displayed near the marker
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    /// Popup text shown on marker click
    #[serde(skip_serializing_if = "Option::is_none")]
    pub popup: Option<String>,
    /// Whether the marker can be dragged
    #[serde(skip_serializing_if = "Option::is_none")]
    pub draggable: Option<bool>,
}

/// A route/polyline on the map
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct GeoMapRoute {
    /// Unique identifier for this route
    pub id: String,
    /// Array of coordinates forming the route
    pub coordinates: Vec<GeoCoordinate>,
    /// Route line color
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    /// Route line width
    #[serde(skip_serializing_if = "Option::is_none")]
    pub width: Option<f64>,
}

/// Map viewport configuration
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, Default)]
#[serde(rename_all = "camelCase")]
pub struct GeoMapViewport {
    pub latitude: f64,
    pub longitude: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub zoom: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bearing: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pitch: Option<f64>,
}

// =============================================================================
// Model3D Input Schemas
// =============================================================================

/// 3D position vector
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, Default)]
pub struct Vec3 {
    pub x: f64,
    pub y: f64,
    pub z: f64,
}

/// 3D model transform configuration
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, Default)]
#[serde(rename_all = "camelCase")]
pub struct Model3dTransform {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub position: Option<Vec3>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rotation: Option<Vec3>,
    /// Uniform scale (single number) or per-axis scale (Vec3)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scale: Option<f64>,
}

/// 3D model animation configuration
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, Default)]
#[serde(rename_all = "camelCase")]
pub struct Model3dAnimation {
    /// Animation clip name
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Whether the animation is playing
    #[serde(skip_serializing_if = "Option::is_none")]
    pub playing: Option<bool>,
    /// Whether to loop the animation
    #[serde(rename = "loop", skip_serializing_if = "Option::is_none")]
    pub loop_anim: Option<bool>,
    /// Playback speed multiplier
    #[serde(skip_serializing_if = "Option::is_none")]
    pub speed: Option<f64>,
}

// =============================================================================
// Scene3D Input Schemas
// =============================================================================

/// 3D camera configuration
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, Default)]
#[serde(rename_all = "camelCase")]
pub struct Scene3dCamera {
    /// Camera type: "perspective" or "orthographic"
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub camera_type: Option<String>,
    /// Camera position in 3D space
    #[serde(skip_serializing_if = "Option::is_none")]
    pub position: Option<Vec3>,
    /// Point the camera looks at
    #[serde(skip_serializing_if = "Option::is_none")]
    pub look_at: Option<Vec3>,
}

/// 3D scene background configuration
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, Default)]
#[serde(rename_all = "camelCase")]
pub struct Scene3dBackground {
    /// Background color (hex string)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    /// Environment preset: "sunset", "dawn", "night", "warehouse", "forest", etc.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub environment: Option<String>,
}

/// 3D scene lighting configuration
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, Default)]
#[serde(rename_all = "camelCase")]
pub struct Scene3dLighting {
    /// Ambient light intensity (0.0 - 1.0)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ambient_intensity: Option<f64>,
    /// Directional light intensity
    #[serde(skip_serializing_if = "Option::is_none")]
    pub directional_intensity: Option<f64>,
    /// Directional light position
    #[serde(skip_serializing_if = "Option::is_none")]
    pub directional_position: Option<Vec3>,
}

/// 3D scene controls configuration
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, Default)]
#[serde(rename_all = "camelCase")]
pub struct Scene3dControls {
    /// Whether controls are enabled
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    /// Enable auto-rotation
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auto_rotate: Option<bool>,
    /// Enable zoom controls
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enable_zoom: Option<bool>,
    /// Enable pan controls
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enable_pan: Option<bool>,
}

// =============================================================================
// Sprite Input Schemas
// =============================================================================

/// 2D position
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, Default)]
pub struct Vec2 {
    pub x: f64,
    pub y: f64,
}

/// Sprite transform configuration
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, Default)]
#[serde(rename_all = "camelCase")]
pub struct SpriteTransform {
    /// Scale multiplier
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scale: Option<f64>,
    /// Rotation in degrees
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rotation: Option<f64>,
    /// Opacity (0.0 - 1.0)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub opacity: Option<f64>,
}

// =============================================================================
// Chart Style Input Schemas
// =============================================================================

/// Bar chart style configuration
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, Default)]
#[serde(rename_all = "camelCase")]
pub struct BarChartStyle {
    /// Layout: "horizontal" or "vertical"
    #[serde(skip_serializing_if = "Option::is_none")]
    pub layout: Option<String>,
    /// Group mode: "grouped" or "stacked"
    #[serde(skip_serializing_if = "Option::is_none")]
    pub group_mode: Option<String>,
    /// Padding between bars (0.0 - 1.0)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub padding: Option<f64>,
    /// Inner padding for grouped bars
    #[serde(skip_serializing_if = "Option::is_none")]
    pub inner_padding: Option<f64>,
    /// Border radius for bars
    #[serde(skip_serializing_if = "Option::is_none")]
    pub border_radius: Option<f64>,
    /// Show bar labels
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enable_label: Option<bool>,
    /// Show X-axis grid lines
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enable_grid_x: Option<bool>,
    /// Show Y-axis grid lines
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enable_grid_y: Option<bool>,
}

/// Line chart style configuration
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, Default)]
#[serde(rename_all = "camelCase")]
pub struct LineChartStyle {
    /// Curve type: "linear", "natural", "step", "basis", "cardinal", etc.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub curve: Option<String>,
    /// Line stroke width
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line_width: Option<f64>,
    /// Enable area fill under the line
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enable_area: Option<bool>,
    /// Area fill opacity
    #[serde(skip_serializing_if = "Option::is_none")]
    pub area_opacity: Option<f64>,
    /// Show data points
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enable_points: Option<bool>,
    /// Data point size
    #[serde(skip_serializing_if = "Option::is_none")]
    pub point_size: Option<f64>,
    /// Enable crosshair slices
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enable_slices: Option<String>,
}

/// Pie/donut chart style configuration
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, Default)]
#[serde(rename_all = "camelCase")]
pub struct PieChartStyle {
    /// Inner radius for donut effect (0-1)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub inner_radius: Option<f64>,
    /// Padding angle between slices
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pad_angle: Option<f64>,
    /// Corner radius of slices
    #[serde(skip_serializing_if = "Option::is_none")]
    pub corner_radius: Option<f64>,
    /// Start angle in degrees
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start_angle: Option<f64>,
    /// End angle in degrees
    #[serde(skip_serializing_if = "Option::is_none")]
    pub end_angle: Option<f64>,
    /// Sort slices by value
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sort_by_value: Option<bool>,
    /// Show arc labels
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enable_arc_labels: Option<bool>,
}

/// Radar chart style configuration
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, Default)]
#[serde(rename_all = "camelCase")]
pub struct RadarChartStyle {
    /// Grid shape: "circular" or "linear"
    #[serde(skip_serializing_if = "Option::is_none")]
    pub grid_shape: Option<String>,
    /// Number of grid levels
    #[serde(skip_serializing_if = "Option::is_none")]
    pub grid_levels: Option<i32>,
    /// Fill opacity (0-1)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fill_opacity: Option<f64>,
    /// Border stroke width
    #[serde(skip_serializing_if = "Option::is_none")]
    pub border_width: Option<f64>,
    /// Show dots at data points
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enable_dots: Option<bool>,
    /// Dot size
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dot_size: Option<f64>,
}

/// Generic chart style for types not covered above
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, Default)]
#[serde(rename_all = "camelCase")]
pub struct GenericChartStyle {
    /// Enable labels
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enable_labels: Option<bool>,
    /// Enable grid
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enable_grid: Option<bool>,
    /// Any additional properties (passed through as-is)
    #[serde(flatten)]
    pub extra: std::collections::HashMap<String, flow_like_types::Value>,
}

// =============================================================================
// Labeler Box Input Schemas
// =============================================================================

/// A bounding box for image labeling
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct LabelerBox {
    /// Unique identifier for this box
    pub id: String,
    /// X coordinate (pixels or normalized 0-1)
    pub x: f64,
    /// Y coordinate (pixels or normalized 0-1)
    pub y: f64,
    /// Box width
    pub width: f64,
    /// Box height
    pub height: f64,
    /// Label/class name for the box
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

// =============================================================================
// Hotspot Input Schemas
// =============================================================================

/// A hotspot point on an image
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Hotspot {
    /// Unique identifier for this hotspot
    pub id: String,
    /// X coordinate (pixels or normalized 0-1)
    pub x: f64,
    /// Y coordinate (pixels or normalized 0-1)
    pub y: f64,
    /// Hotspot size in pixels
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size: Option<f64>,
    /// Label text shown on hover
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    /// Description shown in tooltip
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Hotspot color (e.g., '#3b82f6')
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    /// Action ID to trigger when clicked
    #[serde(skip_serializing_if = "Option::is_none")]
    pub action: Option<String>,
}

// =============================================================================
// Table Input Schemas
// =============================================================================

/// A table column definition
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct TableColumn {
    /// Column accessor key (matches row data keys)
    pub accessor: String,
    /// Column header display text
    pub header: String,
    /// Column width (e.g., "100px", "auto")
    #[serde(skip_serializing_if = "Option::is_none")]
    pub width: Option<String>,
    /// Whether column is sortable
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sortable: Option<bool>,
}

/// Parameters for updating a specific table cell
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct TableCellUpdate {
    /// Row index (0-based)
    pub row_index: i32,
    /// Column accessor
    pub column: String,
    /// New cell value
    pub value: flow_like_types::Value,
}

// =============================================================================
// Media Source Schemas
// =============================================================================

/// Image source with alt text
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, Default)]
#[serde(rename_all = "camelCase")]
pub struct ImageSource {
    /// Image URL
    pub src: String,
    /// Alternative text
    #[serde(skip_serializing_if = "Option::is_none")]
    pub alt: Option<String>,
}

/// Avatar source with fallback
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, Default)]
#[serde(rename_all = "camelCase")]
pub struct AvatarSource {
    /// Avatar image URL
    pub src: String,
    /// Fallback text (initials) when image fails
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fallback: Option<String>,
}

/// Video source with poster
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, Default)]
#[serde(rename_all = "camelCase")]
pub struct VideoSource {
    /// Video URL
    pub src: String,
    /// Poster image URL
    #[serde(skip_serializing_if = "Option::is_none")]
    pub poster: Option<String>,
}

// =============================================================================
// Calendar Input Schemas
// =============================================================================

/// A single event/appointment shown on the interactive calendar.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct CalendarEvent {
    /// Stable unique id used to move/update/remove the event.
    pub id: String,
    /// Event title shown on the calendar.
    pub title: String,
    /// Start timestamp, ISO 8601 (e.g. "2026-07-01T09:00:00Z" or "2026-07-01").
    pub start: String,
    /// End timestamp, ISO 8601. Defaults to a short block when omitted.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub end: Option<String>,
    /// Render as an all-day event (no time-of-day).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub all_day: Option<bool>,
    /// Accent color (CSS color or design token).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    /// Optional longer description.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Optional location string.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub location: Option<String>,
    /// Grouping key (e.g. source calendar id).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub calendar_id: Option<String>,
    /// Whether this event can be moved/resized in the UI.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub editable: Option<bool>,
    /// Arbitrary passthrough payload echoed back on action events.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<flow_like_types::Value>,
}

/// Patch for a single calendar event. Only the `id` is required; provided
/// fields overwrite, omitted fields are left unchanged.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct CalendarEventUpdate {
    /// Id of the event to patch.
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub end: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub all_day: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub location: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub calendar_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub editable: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<flow_like_types::Value>,
}

/// View/behavior configuration for the calendar element.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, Default)]
#[serde(rename_all = "camelCase")]
pub struct CalendarConfig {
    /// "month" | "week" | "day" | "agenda"
    #[serde(skip_serializing_if = "Option::is_none")]
    pub view: Option<String>,
    /// Focused date (ISO 8601).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub date: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub editable: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub selectable: Option<bool>,
    /// First day of week (0 = Sunday).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub first_day_of_week: Option<i32>,
    /// Earliest time shown (e.g. "06:00").
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min_time: Option<String>,
    /// Latest time shown (e.g. "22:00").
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_time: Option<String>,
    /// Slot granularity in minutes.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub slot_duration: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub show_weekends: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub show_now_indicator: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub show_all_day: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub locale: Option<String>,
}

// =============================================================================
// Gantt Input Schemas
// =============================================================================

/// A single task on the Gantt timeline.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct GanttTask {
    /// Stable unique id used to move/update/remove the task.
    pub id: String,
    /// Task name shown in the left panel and on the bar.
    pub name: String,
    /// Start date/time, ISO 8601.
    pub start: String,
    /// End date/time, ISO 8601.
    pub end: String,
    /// Completion percentage (0-100).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub progress: Option<f64>,
    /// Ids of predecessor tasks this one depends on.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dependencies: Option<Vec<String>>,
    /// Parent task id for grouping / sub-tasks.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent: Option<String>,
    /// Bar accent color (CSS color or design token).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    /// Optional assignee (user id or display name).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub assignee: Option<String>,
    /// Render as a milestone diamond instead of a bar.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub milestone: Option<bool>,
    /// Whether this task's children are collapsed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub collapsed: Option<bool>,
    /// Arbitrary passthrough payload echoed back on action events.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<flow_like_types::Value>,
}

/// Patch for a single Gantt task. Only the `id` is required.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct GanttTaskUpdate {
    /// Id of the task to patch.
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub end: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub progress: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dependencies: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub assignee: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub milestone: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub collapsed: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<flow_like_types::Value>,
}

/// A dependency edge between two Gantt tasks (predecessor -> successor).
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct GanttDependencyUpdate {
    /// Predecessor task id.
    pub from_id: String,
    /// Successor task id (the one that gains the dependency).
    pub to_id: String,
}

/// View/behavior configuration for the Gantt element.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, Default)]
#[serde(rename_all = "camelCase")]
pub struct GanttConfig {
    /// "day" | "week" | "month" | "quarter" | "compact"
    #[serde(skip_serializing_if = "Option::is_none")]
    pub view: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub editable: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub draggable: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resizable: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub show_dependencies: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub show_progress: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub show_today: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub row_height: Option<f64>,
}
