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
// Graph Input Schemas
// =============================================================================

/// Visual style applied to every node or edge carrying a label
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct GraphLabelStyle {
    /// The node or edge label this style applies to
    pub label: String,
    /// Hex color, e.g. "#6366f1"
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    /// Icon key rendered inside the node
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    /// Node radius in pixels
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size: Option<f64>,
}

/// A single graph node
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct GraphNodeInput {
    /// Unique identifier, referenced by edge source/target
    pub id: String,
    /// Type of the node — drives colour, icon and the legend
    pub label: String,
    /// Text shown on and next to the node
    #[serde(skip_serializing_if = "Option::is_none")]
    pub caption: Option<String>,
    /// Arbitrary properties listed in the node inspector
    #[serde(skip_serializing_if = "Option::is_none")]
    pub props: Option<flow_like_types::Value>,
}

/// A single graph edge
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct GraphEdgeInput {
    /// Unique identifier for this edge
    pub id: String,
    /// Id of the node the edge starts at
    pub source: String,
    /// Id of the node the edge points to
    pub target: String,
    /// Type of the relation — drives colour and the legend
    pub label: String,
    /// Arbitrary properties listed in the edge inspector
    #[serde(skip_serializing_if = "Option::is_none")]
    pub props: Option<flow_like_types::Value>,
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
    /// Stable unique id used to move/update/remove the item. Auto-generated when omitted.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    /// Event title shown on the calendar.
    pub title: String,
    /// Start timestamp, ISO 8601 (e.g. "2026-07-01T09:00:00Z" or "2026-07-01").
    #[schemars(extend("format" = "date-time"))]
    pub start: String,
    /// End timestamp, ISO 8601. Defaults to a short block when omitted.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schemars(extend("format" = "date-time"))]
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
    /// Optional link opened from the item's detail dialog. Relative paths
    /// (e.g. "/orders/42") navigate inside the app; absolute URLs (https://…)
    /// open externally.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub link: Option<String>,
    /// Key-value metadata echoed back on action events and shown in the
    /// item's detail dialog (e.g. ticket number, external ids).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<std::collections::HashMap<String, flow_like_types::Value>>,
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
    #[schemars(extend("format" = "date-time"))]
    pub start: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schemars(extend("format" = "date-time"))]
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
    /// Optional link opened from the item's detail dialog. Relative paths
    /// (e.g. "/orders/42") navigate inside the app; absolute URLs (https://…)
    /// open externally.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub link: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<std::collections::HashMap<String, flow_like_types::Value>>,
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
    #[schemars(extend("format" = "date-time"))]
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
    /// Header title shown above the calendar.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// Visual density: "compact" | "default" | "comfortable".
    #[serde(skip_serializing_if = "Option::is_none")]
    pub density: Option<String>,
    /// Show the month/week/day/agenda view switcher.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub show_view_switcher: Option<bool>,
    /// Element height (CSS value, e.g. "600px").
    #[serde(skip_serializing_if = "Option::is_none")]
    pub height: Option<String>,
    /// Auto-switch to agenda view on narrow widths.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub responsive: Option<bool>,
    /// Width in px below which the responsive compact mode kicks in.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub compact_breakpoint: Option<f64>,
}

// =============================================================================
// Gantt Input Schemas
// =============================================================================

/// A single task on the Gantt timeline.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct GanttTask {
    /// Stable unique id used to move/update/remove the item. Auto-generated when omitted.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    /// Task name shown in the left panel and on the bar.
    pub name: String,
    /// Start date/time, ISO 8601.
    #[schemars(extend("format" = "date-time"))]
    pub start: String,
    /// End date/time, ISO 8601.
    #[schemars(extend("format" = "date-time"))]
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
    /// Optional assignee: a free-text display name, or a team member's user
    /// sub — subs resolve to the member's profile (avatar + name) in the UI.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub assignee: Option<String>,
    /// Render as a milestone diamond instead of a bar.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub milestone: Option<bool>,
    /// Whether this task's children are collapsed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub collapsed: Option<bool>,
    /// Optional link opened from the item's detail dialog. Relative paths
    /// (e.g. "/orders/42") navigate inside the app; absolute URLs (https://…)
    /// open externally.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub link: Option<String>,
    /// Key-value metadata echoed back on action events and shown in the
    /// item's detail dialog (e.g. ticket number, external ids).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<std::collections::HashMap<String, flow_like_types::Value>>,
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
    #[schemars(extend("format" = "date-time"))]
    pub start: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schemars(extend("format" = "date-time"))]
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
    /// Optional link opened from the item's detail dialog. Relative paths
    /// (e.g. "/orders/42") navigate inside the app; absolute URLs (https://…)
    /// open externally.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub link: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<std::collections::HashMap<String, flow_like_types::Value>>,
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
    /// Header title shown above the timeline.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// Visual density: "compact" | "default" | "comfortable".
    #[serde(skip_serializing_if = "Option::is_none")]
    pub density: Option<String>,
    /// Show the day/week/month/quarter/compact view switcher.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub show_view_switcher: Option<bool>,
    /// Show the left task-list panel.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub show_task_list: Option<bool>,
    /// Width of the left task-list panel in px.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub task_list_width: Option<f64>,
    /// Shade weekend columns on the timeline.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shade_weekends: Option<bool>,
    /// Extra left-panel columns (e.g. ["assignee", "progress"]).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub columns: Option<Vec<String>>,
    /// Element height (CSS value, e.g. "600px").
    #[serde(skip_serializing_if = "Option::is_none")]
    pub height: Option<String>,
    /// Auto-switch to compact view on narrow widths.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub responsive: Option<bool>,
    /// Width in px below which the responsive compact mode kicks in.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub compact_breakpoint: Option<f64>,
}

// =============================================================================
// Shared Item Helpers (Calendar / Gantt)
// =============================================================================

/// Fill missing/empty `id` fields on an item or an array of items.
pub fn ensure_item_ids(value: &mut flow_like_types::Value) {
    if let Some(arr) = value.as_array_mut() {
        for item in arr {
            ensure_item_id(item);
        }
    } else {
        ensure_item_id(value);
    }
}

/// Fill a missing/empty `id` field on a single item with a generated id.
pub fn ensure_item_id(item: &mut flow_like_types::Value) {
    if let Some(obj) = item.as_object_mut() {
        let missing = obj
            .get("id")
            .and_then(|v| v.as_str())
            .map(|s| s.is_empty())
            .unwrap_or(true);
        if missing {
            obj.insert(
                "id".into(),
                flow_like_types::Value::String(flow_like_types::create_id()),
            );
        }
    }
}

/// Numeric-coercion-safe deep equality: numbers compare via `as_f64()`,
/// objects compare key-order independent.
pub fn values_equal(a: &flow_like_types::Value, b: &flow_like_types::Value) -> bool {
    use flow_like_types::Value;
    match (a, b) {
        (Value::Number(x), Value::Number(y)) => match (x.as_f64(), y.as_f64()) {
            (Some(x), Some(y)) => x == y,
            _ => x == y,
        },
        (Value::Array(x), Value::Array(y)) => {
            x.len() == y.len() && x.iter().zip(y).all(|(a, b)| values_equal(a, b))
        }
        (Value::Object(x), Value::Object(y)) => {
            x.len() == y.len()
                && x.iter().all(|(key, value)| {
                    y.get(key).map(|other| values_equal(value, other)) == Some(true)
                })
        }
        _ => a == b,
    }
}

/// Diff two item arrays by string `id`. Returns `(created, updated, deleted)`;
/// `updated` contains the current version of changed items. Items without an
/// id cannot be paired: current ones count as created, previous ones as deleted.
pub fn diff_items(
    previous: &flow_like_types::Value,
    current: &flow_like_types::Value,
) -> (
    Vec<flow_like_types::Value>,
    Vec<flow_like_types::Value>,
    Vec<flow_like_types::Value>,
) {
    let empty = Vec::new();
    let prev_items = previous.as_array().unwrap_or(&empty);
    let curr_items = current.as_array().unwrap_or(&empty);

    let item_id = |item: &flow_like_types::Value| -> Option<String> {
        item.get("id")?.as_str().map(String::from)
    };

    let prev_by_id: std::collections::HashMap<String, &flow_like_types::Value> = prev_items
        .iter()
        .filter_map(|item| item_id(item).map(|id| (id, item)))
        .collect();
    let curr_ids: std::collections::HashSet<String> =
        curr_items.iter().filter_map(&item_id).collect();

    let mut created = Vec::new();
    let mut updated = Vec::new();
    let mut deleted = Vec::new();

    for item in curr_items {
        match item_id(item) {
            Some(id) => match prev_by_id.get(&id) {
                Some(prev) => {
                    if !values_equal(prev, item) {
                        updated.push(item.clone());
                    }
                }
                None => created.push(item.clone()),
            },
            None => created.push(item.clone()),
        }
    }

    for item in prev_items {
        match item_id(item) {
            Some(id) => {
                if !curr_ids.contains(&id) {
                    deleted.push(item.clone());
                }
            }
            None => deleted.push(item.clone()),
        }
    }

    (created, updated, deleted)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gantt_task_schema_types_dates_and_dependency_items() {
        let schema = flow_like_types::json::to_value(schemars::schema_for!(GanttTask)).unwrap();
        assert_eq!(schema["properties"]["start"]["format"], "date-time");
        assert_eq!(schema["properties"]["end"]["format"], "date-time");
        // Nullable arrays must keep `items` on the same schema so pin
        // inference can see the element type.
        assert_eq!(
            schema["properties"]["dependencies"]["items"]["type"],
            "string"
        );
        // Metadata is a typed key-value object, not `any`.
        let meta_type = &schema["properties"]["metadata"]["type"];
        assert!(
            meta_type == "object"
                || meta_type
                    .as_array()
                    .is_some_and(|t| t.contains(&flow_like_types::json::json!("object"))),
            "metadata should be object-typed, got {meta_type}"
        );
    }

    #[test]
    fn calendar_event_schema_types_dates() {
        let schema = flow_like_types::json::to_value(schemars::schema_for!(CalendarEvent)).unwrap();
        assert_eq!(schema["properties"]["start"]["format"], "date-time");
        assert_eq!(schema["properties"]["end"]["format"], "date-time");
    }
}
