//! A2UI Component Type Definitions
//!
//! This module provides strongly-typed Rust representations of all A2UI component types.
//! These schemas enable better tooling support including autocomplete and node recommendations.

use flow_like_types::{
    Value,
    json::{Deserialize, Serialize},
};
use schemars::JsonSchema;
use std::collections::HashMap;

use super::{Action, BoundValue, Children, Style};

/// All possible A2UI component types
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum A2UIComponentType {
    // Layout components
    Row(RowProps),
    Column(ColumnProps),
    Stack(StackProps),
    Grid(GridProps),
    ScrollArea(ScrollAreaProps),
    AspectRatio(AspectRatioProps),
    Overlay(OverlayProps),
    Absolute(AbsoluteProps),
    Box(BoxProps),
    Center(CenterProps),
    Spacer(SpacerProps),
    WidgetInstance(WidgetInstanceProps),

    // Display components
    Text(TextProps),
    Image(ImageProps),
    Icon(IconProps),
    Video(VideoProps),
    Lottie(LottieProps),
    Markdown(MarkdownProps),
    Divider(DividerProps),
    Badge(BadgeProps),
    Avatar(AvatarProps),
    UserProfile(UserProfileProps),
    Progress(ProgressProps),
    Spinner(SpinnerProps),
    Skeleton(SkeletonProps),
    Table(TableProps),
    TableRow(TableRowProps),
    TableCell(TableCellProps),
    FilePreview(FilePreviewProps),
    DiffView(DiffViewProps),
    BoundingBoxOverlay(BoundingBoxOverlayProps),

    // Interactive components
    Button(ButtonProps),
    Feedback(FeedbackProps),
    AppLink(AppLinkProps),
    TextField(TextFieldProps),
    Select(SelectProps),
    Slider(SliderProps),
    Checkbox(CheckboxProps),
    Switch(SwitchProps),
    RadioGroup(RadioGroupProps),
    DateTimeInput(DateTimeInputProps),
    FileInput(FileInputProps),
    ImageInput(ImageInputProps),
    VoiceInput(VoiceInputProps),
    Link(LinkProps),
    ImageLabeler(ImageLabelerProps),
    ImageHotspot(ImageHotspotProps),

    // Container components
    Card(CardProps),
    Modal(ModalProps),
    Tabs(TabsProps),
    Accordion(AccordionProps),
    Drawer(DrawerProps),
    Tooltip(TooltipProps),
    Popover(PopoverProps),

    // Game components
    Canvas2d(Canvas2dProps),
    Sprite(SpriteProps),
    Shape(ShapeProps),
    Scene3d(Scene3dProps),
    Model3d(Model3dProps),
    Dialogue(DialogueProps),
    CharacterPortrait(CharacterPortraitProps),
    ChoiceMenu(ChoiceMenuProps),
    InventoryGrid(InventoryGridProps),
    HealthBar(HealthBarProps),
    MiniMap(MiniMapProps),
    GeoMap(GeoMapProps),

    // Embeds & Charts
    Iframe(IframeProps),
    PlotlyChart(PlotlyChartProps),
    NivoChart(NivoChartProps),
    Graph(GraphProps),
    OntologyGraph(OntologyGraphProps),

    // Planning components
    Calendar(CalendarProps),
    Gantt(GanttProps),
}

/// A complete A2UI element with its component data, style, and metadata
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct A2UIElement {
    /// Unique identifier for this element
    pub id: String,
    /// Optional style configuration
    #[serde(skip_serializing_if = "Option::is_none")]
    pub style: Option<Style>,
    /// The component type and its properties
    #[serde(flatten)]
    pub component: A2UIComponentType,
    /// Child component references
    #[serde(skip_serializing_if = "Option::is_none")]
    pub children: Option<Children>,
    /// Actions that can be triggered on this component
    #[serde(skip_serializing_if = "Option::is_none")]
    pub actions: Option<Vec<Action>>,
    /// Ordered actions bound to named component events. Legacy `actions` remains the fallback.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub event_handlers: Option<HashMap<String, Vec<Action>>>,
    /// Internal element ID for workflow operations (added at runtime)
    #[serde(rename = "__element_id", skip_serializing_if = "Option::is_none")]
    pub element_id: Option<String>,
}

// =============================================================================
// Layout Components
// =============================================================================

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, Default)]
#[serde(rename_all = "camelCase")]
pub struct RowProps {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gap: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub align: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub justify: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub wrap: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reverse: Option<BoundValue>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, Default)]
#[serde(rename_all = "camelCase")]
pub struct ColumnProps {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gap: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub align: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub justify: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reverse: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub wrap: Option<BoundValue>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, Default)]
#[serde(rename_all = "camelCase")]
pub struct StackProps {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub align: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub width: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub height: Option<BoundValue>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, Default)]
#[serde(rename_all = "camelCase")]
pub struct BoxProps {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub r#as: Option<BoundValue>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, Default)]
#[serde(rename_all = "camelCase")]
pub struct CenterProps {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub inline: Option<BoundValue>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, Default)]
#[serde(rename_all = "camelCase")]
pub struct SpacerProps {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub flex: Option<BoundValue>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct WidgetInstanceProps {
    pub instance_id: String,
    pub widget_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub app_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exposed_prop_values: Option<HashMap<String, Value>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub action_bindings: Option<HashMap<String, Value>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub style_override: Option<Style>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, Default)]
#[serde(rename_all = "camelCase")]
pub struct GridProps {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub columns: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rows: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gap: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub column_gap: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub row_gap: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auto_flow: Option<BoundValue>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, Default)]
#[serde(rename_all = "camelCase")]
pub struct ScrollAreaProps {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub direction: Option<BoundValue>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct AspectRatioProps {
    pub ratio: BoundValue,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct OverlayItem {
    pub component_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub anchor: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub offset_x: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub offset_y: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub z_index: Option<BoundValue>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct OverlayProps {
    pub base_component_id: String,
    pub overlays: Vec<OverlayItem>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, Default)]
#[serde(rename_all = "camelCase")]
pub struct AbsoluteProps {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub width: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub height: Option<BoundValue>,
}

// =============================================================================
// Display Components
// =============================================================================

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct TextProps {
    pub content: BoundValue,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub variant: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub weight: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub align: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub truncate: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_lines: Option<BoundValue>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ImageProps {
    pub src: BoundValue,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub alt: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fit: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fallback: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub loading: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub aspect_ratio: Option<BoundValue>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct IconProps {
    pub name: BoundValue,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stroke_width: Option<BoundValue>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct VideoProps {
    pub src: BoundValue,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub poster: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub autoplay: Option<BoundValue>,
    #[serde(rename = "loop", skip_serializing_if = "Option::is_none")]
    pub loop_video: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub muted: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub controls: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub width: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub height: Option<BoundValue>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct LottieProps {
    pub src: BoundValue,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub autoplay: Option<BoundValue>,
    #[serde(rename = "loop", skip_serializing_if = "Option::is_none")]
    pub loop_animation: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub speed: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub width: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub height: Option<BoundValue>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct MarkdownProps {
    pub content: BoundValue,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub allow_html: Option<BoundValue>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, Default)]
#[serde(rename_all = "camelCase")]
pub struct DividerProps {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub orientation: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thickness: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color: Option<BoundValue>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct BadgeProps {
    pub content: BoundValue,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub variant: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color: Option<BoundValue>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, Default)]
#[serde(rename_all = "camelCase")]
pub struct AvatarProps {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub src: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fallback: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size: Option<BoundValue>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct UserProfileProps {
    /// User subject/sub ID. Compatible with Set Element Value via component.value.
    pub value: BoundValue,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub variant: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub avatar_size: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub show_hover: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub show_email: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub show_description: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub show_user_id: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub show_profile_link: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fallback_label: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub muted: Option<BoundValue>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ProgressProps {
    pub value: BoundValue,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub show_label: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub variant: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color: Option<BoundValue>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, Default)]
#[serde(rename_all = "camelCase")]
pub struct SpinnerProps {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color: Option<BoundValue>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, Default)]
#[serde(rename_all = "camelCase")]
pub struct SkeletonProps {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub width: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub height: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rounded: Option<BoundValue>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct TableColumnDef {
    pub id: String,
    pub header: BoundValue,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub accessor: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub width: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub align: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sortable: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hidden: Option<BoundValue>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct TableProps {
    pub columns: BoundValue,
    pub data: BoundValue,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub caption: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub striped: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bordered: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hoverable: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub compact: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sticky_header: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sortable: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub searchable: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub paginated: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub page_size: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub selectable: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub on_row_click: Option<BoundValue>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct TableRowProps {
    pub cells: BoundValue,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub selected: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub disabled: Option<BoundValue>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct TableCellProps {
    pub content: BoundValue,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_header: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub col_span: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub row_span: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub align: Option<BoundValue>,
}

// =============================================================================
// Interactive Components
// =============================================================================

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ButtonProps {
    pub label: BoundValue,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub variant: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub disabled: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub loading: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icon: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icon_position: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tooltip: Option<BoundValue>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct FeedbackProps {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mode: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub positive_label: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub negative_label: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub positive_rating: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub negative_rating: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub show_comment: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub comment_mode: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub comment_label: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub comment_placeholder: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub comment_title: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub comment_description: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub comment_submit_label: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub comment_cancel_label: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub feedback_id: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub include_state: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub page_context_mode: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub page_context_query_param_allowlist: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub page_context_query_param_denylist: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub include_page_hash: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub success_message: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub disabled: Option<BoundValue>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct AppLinkProps {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub variant: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icon: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icon_position: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub app_id: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub event_id: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub disabled: Option<BoundValue>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct TextFieldProps {
    pub value: BoundValue,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub placeholder: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub helper_text: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub disabled: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_type: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub multiline: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rows: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_length: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub required: Option<BoundValue>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SelectProps {
    pub value: BoundValue,
    pub options: BoundValue,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub placeholder: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub disabled: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub multiple: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub searchable: Option<BoundValue>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SliderProps {
    pub value: BoundValue,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub step: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub disabled: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub show_value: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<BoundValue>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct CheckboxProps {
    pub checked: BoundValue,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub disabled: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub indeterminate: Option<BoundValue>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SwitchProps {
    pub checked: BoundValue,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub disabled: Option<BoundValue>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct RadioGroupProps {
    pub value: BoundValue,
    pub options: BoundValue,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub disabled: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub orientation: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<BoundValue>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct DateTimeInputProps {
    pub value: BoundValue,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mode: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub disabled: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<BoundValue>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct FileInputProps {
    pub value: BoundValue,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub helper_text: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub accept: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub multiple: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_size: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_files: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub disabled: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<BoundValue>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ImageInputProps {
    pub value: BoundValue,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub helper_text: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub accept: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub multiple: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_size: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_files: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub disabled: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub aspect_ratio: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub show_preview: Option<BoundValue>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct VoiceInputProps {
    pub value: BoundValue,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub helper_text: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_duration: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auto_stop: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub silence_threshold: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub silence_duration: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub disabled: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<BoundValue>,
    /// Deprecated alias for `variant` ("waveform" | "bars").
    #[serde(skip_serializing_if = "Option::is_none")]
    pub visualizer: Option<BoundValue>,
    /// Visual style: "conservative" | "waveform" | "orb" | "vortex" | "shader" | "aurora" | "pulse".
    #[serde(skip_serializing_if = "Option::is_none")]
    pub variant: Option<BoundValue>,
    /// Element size: "sm" | "md" | "lg".
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size: Option<BoundValue>,
    /// Capture mode: "record" (send audio) | "stt" (send transcript text).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mode: Option<BoundValue>,
    /// Invoke mode: "manual" (click) | "hold" (press-and-hold) | "auto" (pause detection).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub invoke: Option<BoundValue>,
    /// Base accent color (CSS color string).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color: Option<BoundValue>,
    /// Accent color while recording (CSS color string).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recording_color: Option<BoundValue>,
    /// Post-input look: "player" (animated playback) | "autoplay" (player that plays the result immediately, for conversations) | "summary" (compact info + delete). Default "player".
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result_mode: Option<BoundValue>,
    /// Backend-set response media URL, used by result modes such as "autoplay".
    #[serde(skip_serializing_if = "Option::is_none")]
    pub src: Option<BoundValue>,
    /// Alias for `src`; media-source update nodes write both fields.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<BoundValue>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct LinkProps {
    pub href: BoundValue,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub route: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub query_params: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub external: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub variant: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub underline: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub disabled: Option<BoundValue>,
}

// =============================================================================
// Container Components
// =============================================================================

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, Default)]
#[serde(rename_all = "camelCase")]
pub struct CardProps {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub footer: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hoverable: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub clickable: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub variant: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub padding: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub header_image: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub header_icon: Option<BoundValue>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ModalProps {
    pub open: BoundValue,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub close_on_overlay: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub close_on_escape: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub show_close_button: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub centered: Option<BoundValue>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct TabDefinition {
    pub id: String,
    pub label: BoundValue,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icon: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub disabled: Option<BoundValue>,
    pub content_component_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct TabsProps {
    pub value: BoundValue,
    pub tabs: Vec<TabDefinition>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub orientation: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub variant: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub list_style: Option<Style>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trigger_style: Option<Style>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content_style: Option<Style>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct AccordionItem {
    pub id: String,
    pub title: BoundValue,
    pub content_component_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct AccordionProps {
    pub items: Vec<AccordionItem>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub multiple: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_expanded: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub collapsible: Option<BoundValue>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct DrawerProps {
    pub open: BoundValue,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub side: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub overlay: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub closable: Option<BoundValue>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct TooltipProps {
    pub content: BoundValue,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub side: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub delay_ms: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_width: Option<BoundValue>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct PopoverProps {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub open: Option<BoundValue>,
    pub content_component_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub side: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trigger: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub close_on_click_outside: Option<BoundValue>,
}

// =============================================================================
// Game Components
// =============================================================================

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Canvas2dProps {
    pub width: BoundValue,
    pub height: BoundValue,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub background_color: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pixel_perfect: Option<BoundValue>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SpriteProps {
    pub src: BoundValue,
    pub x: BoundValue,
    pub y: BoundValue,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub width: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub height: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rotation: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scale: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub opacity: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub flip_x: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub flip_y: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub z_index: Option<BoundValue>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ShapeProps {
    pub shape_type: BoundValue,
    pub x: BoundValue,
    pub y: BoundValue,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub width: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub height: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub radius: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub points: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fill: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stroke: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stroke_width: Option<BoundValue>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Scene3dProps {
    pub width: BoundValue,
    pub height: BoundValue,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub camera_type: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub camera_position: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub background_color: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub control_mode: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fixed_view: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auto_rotate_speed: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enable_controls: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enable_zoom: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enable_pan: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fov: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub near: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub far: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ambient_light: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub directional_light: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub show_grid: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub show_axes: Option<BoundValue>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Model3dProps {
    pub src: BoundValue,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub position: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rotation: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scale: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cast_shadow: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub receive_shadow: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub animation: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auto_rotate: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rotate_speed: Option<BoundValue>,
    /// Height of the standalone model viewer.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub viewer_height: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub background_color: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub camera_distance: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fov: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub camera_angle: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub camera_position: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub camera_target: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enable_controls: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enable_zoom: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enable_pan: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auto_rotate_camera: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub camera_rotate_speed: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ambient_light: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub directional_light: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fill_light: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rim_light: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub light_color: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lighting_preset: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub show_ground: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ground_color: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enable_reflections: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub environment: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub environment_source: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub use_hdr_background: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub polyhaven_hdri: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub polyhaven_resolution: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hdri_url: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ground_size: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ground_offset_y: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ground_follow_camera: Option<BoundValue>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct DialogueProps {
    pub text: BoundValue,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub speaker_name: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub speaker_portrait_id: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub typewriter: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub typewriter_speed: Option<BoundValue>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct CharacterPortraitProps {
    pub image: BoundValue,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expression: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub position: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dimmed: Option<BoundValue>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ChoiceMenuProps {
    pub choices: BoundValue,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub layout: Option<BoundValue>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct InventoryGridProps {
    pub items: BoundValue,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub columns: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rows: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cell_size: Option<BoundValue>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct HealthBarProps {
    pub value: BoundValue,
    pub max_value: BoundValue,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub show_value: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fill_color: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub background_color: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub variant: Option<BoundValue>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct MiniMapProps {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub map_image: Option<BoundValue>,
    pub width: BoundValue,
    pub height: BoundValue,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub markers: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub player_x: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub player_y: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub player_rotation: Option<BoundValue>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct GeoMapProps {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub viewport: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub markers: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub routes: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub show_controls: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub show_zoom: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub show_compass: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub show_locate: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub show_fullscreen: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub interactive: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub control_position: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cluster_markers: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cluster_radius: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cluster_max_zoom: Option<BoundValue>,
}

// =============================================================================
// Embeds
// =============================================================================

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct IframeProps {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub src: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub srcdoc: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub width: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub height: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sandbox: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub allow: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub loading: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub referrer_policy: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub border: Option<BoundValue>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ChartAxis {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub axis_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub show_grid: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tick_format: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ChartSeries {
    pub name: String,
    #[serde(rename = "type")]
    pub series_type: String,
    pub data_source: HashMap<String, String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mode: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, Default)]
#[serde(rename_all = "camelCase")]
pub struct PlotlyChartProps {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub chart_type: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub series: Option<Vec<ChartSeries>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub x_axis: Option<ChartAxis>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub y_axis: Option<ChartAxis>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub layout: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub config: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub width: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub height: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub responsive: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub show_legend: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub legend_position: Option<BoundValue>,
}

// =============================================================================
// Nivo Charts
// =============================================================================

/// Nivo Chart component props - supports 25+ chart types from the Nivo library
/// Chart types: bar, line, pie, radar, heatmap, scatter, funnel, treemap, sunburst,
/// calendar, bump, areaBump, circlePacking, network, sankey, stream, swarmplot,
/// voronoi, waffle, marimekko, parallelCoordinates, radialBar, boxplot, bullet, chord
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct NivoChartProps {
    /// The chart type (bar, line, pie, radar, etc.)
    pub chart_type: BoundValue,
    /// Chart title displayed above the chart
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<BoundValue>,
    /// Chart data in Nivo format (varies by chart type)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<BoundValue>,
    /// Chart height (e.g., "400px")
    #[serde(skip_serializing_if = "Option::is_none")]
    pub height: Option<BoundValue>,
    /// Color scheme name (e.g., "nivo", "paired") or array of colors
    #[serde(skip_serializing_if = "Option::is_none")]
    pub colors: Option<BoundValue>,
    /// Enable animations
    #[serde(skip_serializing_if = "Option::is_none")]
    pub animate: Option<BoundValue>,
    /// Show legend
    #[serde(skip_serializing_if = "Option::is_none")]
    pub show_legend: Option<BoundValue>,
    /// Legend position: "top", "bottom", "left", "right"
    #[serde(skip_serializing_if = "Option::is_none")]
    pub legend_position: Option<BoundValue>,
    /// Key for indexing data (bar, radar charts)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub index_by: Option<BoundValue>,
    /// Data keys to display (bar, radar, stream charts)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub keys: Option<BoundValue>,
    /// Chart margins { top, right, bottom, left }
    #[serde(skip_serializing_if = "Option::is_none")]
    pub margin: Option<BoundValue>,
    /// Bottom axis configuration
    #[serde(skip_serializing_if = "Option::is_none")]
    pub axis_bottom: Option<BoundValue>,
    /// Left axis configuration
    #[serde(skip_serializing_if = "Option::is_none")]
    pub axis_left: Option<BoundValue>,
    /// Top axis configuration
    #[serde(skip_serializing_if = "Option::is_none")]
    pub axis_top: Option<BoundValue>,
    /// Right axis configuration
    #[serde(skip_serializing_if = "Option::is_none")]
    pub axis_right: Option<BoundValue>,
    /// Bar chart specific style options
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bar_style: Option<BoundValue>,
    /// Line chart specific style options
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line_style: Option<BoundValue>,
    /// Pie/donut chart specific style options
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pie_style: Option<BoundValue>,
    /// Radar chart specific style options
    #[serde(skip_serializing_if = "Option::is_none")]
    pub radar_style: Option<BoundValue>,
    /// Heatmap chart specific style options
    #[serde(skip_serializing_if = "Option::is_none")]
    pub heatmap_style: Option<BoundValue>,
    /// Scatter chart specific style options
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scatter_style: Option<BoundValue>,
    /// Funnel chart specific style options
    #[serde(skip_serializing_if = "Option::is_none")]
    pub funnel_style: Option<BoundValue>,
    /// Treemap chart specific style options
    #[serde(skip_serializing_if = "Option::is_none")]
    pub treemap_style: Option<BoundValue>,
    /// Sankey diagram specific style options
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sankey_style: Option<BoundValue>,
    /// Calendar chart specific style options
    #[serde(skip_serializing_if = "Option::is_none")]
    pub calendar_style: Option<BoundValue>,
    /// Chord diagram specific style options
    #[serde(skip_serializing_if = "Option::is_none")]
    pub chord_style: Option<BoundValue>,
    /// Full Nivo config override for advanced customization
    #[serde(skip_serializing_if = "Option::is_none")]
    pub config: Option<BoundValue>,
}

// =============================================================================
// Graphs
// =============================================================================

/// Node/edge network graph rendered with the WebGL canvas used by the ontology
/// explorer. Nodes and edges use the subgraph wire shape, so the output of a
/// graph or ontology query can be bound straight to this component.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct GraphProps {
    /// Nodes: `[{ id, label, caption?, props? }]`
    pub nodes: BoundValue,
    /// Edges: `[{ id, source, target, label, props? }]`
    #[serde(skip_serializing_if = "Option::is_none")]
    pub edges: Option<BoundValue>,
    /// Per-label style overrides: `{ "<label>": { color, icon, size } }`
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label_styles: Option<BoundValue>,
    /// Toolbar with node/edge counts and the search box (default true)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub show_toolbar: Option<BoundValue>,
    /// Search box over the loaded nodes (default true)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub show_search: Option<BoundValue>,
    /// Floating label legend (default true)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub show_legend: Option<BoundValue>,
    /// Node/edge detail drawer on selection (default true)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub show_inspector: Option<BoundValue>,
    /// Component height, e.g. "480px" (default "480px")
    #[serde(skip_serializing_if = "Option::is_none")]
    pub height: Option<BoundValue>,
}

/// Embeds an existing project ontology exactly as the Data Studio shows it:
/// live data, neighbour expansion, search, path finding and governed actions.
/// Access is enforced by the same permissions as the Data Studio itself.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct OntologyGraphProps {
    /// Id of the ontology (graph overlay) to display
    pub ontology_id: BoundValue,
    /// Project the ontology belongs to (defaults to the running project)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub app_id: Option<BoundValue>,
    /// Node budget for the initial load (defaults to the ontology's own limit)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<BoundValue>,
    /// Neighbour and child expansion (default true)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub allow_expand: Option<BoundValue>,
    /// Search across the loaded graph and the full ontology (default true)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub allow_search: Option<BoundValue>,
    /// Shortest-path finding between two nodes (default true)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub allow_paths: Option<BoundValue>,
    /// Running governed ontology actions on a node (default true)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub allow_actions: Option<BoundValue>,
    /// Cypher query panel (default false)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub allow_cypher: Option<BoundValue>,
    /// Legend style edits, persisted onto the shared ontology (default false)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub allow_style_edit: Option<BoundValue>,
    /// Node-limit selector in the toolbar (default true)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub allow_limit_change: Option<BoundValue>,
    /// Toolbar with counts, search and limit selector (default true)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub show_toolbar: Option<BoundValue>,
    /// Floating label legend (default true)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub show_legend: Option<BoundValue>,
    /// Component height, e.g. "480px" (default "480px")
    #[serde(skip_serializing_if = "Option::is_none")]
    pub height: Option<BoundValue>,
}

// =============================================================================
// Image Labeler (Bounding Box Annotation)
// =============================================================================

/// A labeled bounding box on an image
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct LabelBox {
    /// Unique identifier for the box
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
    pub label: String,
    /// Optional confidence score (0-1)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub confidence: Option<f64>,
    /// Optional custom color
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
}

/// Image Labeler component for drawing and managing bounding boxes
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ImageLabelerProps {
    /// Image source URL
    pub src: BoundValue,
    /// Alternative text for accessibility
    #[serde(skip_serializing_if = "Option::is_none")]
    pub alt: Option<BoundValue>,
    /// Initial bounding boxes
    #[serde(skip_serializing_if = "Option::is_none")]
    pub boxes: Option<BoundValue>,
    /// Available labels to choose from
    pub labels: BoundValue,
    /// Disable editing
    #[serde(skip_serializing_if = "Option::is_none")]
    pub disabled: Option<BoundValue>,
    /// Show labels on boxes
    #[serde(skip_serializing_if = "Option::is_none")]
    pub show_labels: Option<BoundValue>,
    /// Minimum box size in pixels
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min_box_size: Option<BoundValue>,
}

// =============================================================================
// Image Hotspot (Point and Click Interactive Image)
// =============================================================================

/// A clickable hotspot on an image
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Hotspot {
    /// Unique identifier
    pub id: String,
    /// X coordinate (pixels or normalized 0-1)
    pub x: f64,
    /// Y coordinate (pixels or normalized 0-1)
    pub y: f64,
    /// Hotspot size in pixels
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size: Option<f64>,
    /// Hotspot color
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    /// Icon to display (emoji or icon name)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    /// Label text
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    /// Description shown in tooltip
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Action name to trigger on click
    #[serde(skip_serializing_if = "Option::is_none")]
    pub action: Option<String>,
    /// Whether this hotspot is disabled
    #[serde(skip_serializing_if = "Option::is_none")]
    pub disabled: Option<bool>,
}

/// Image Hotspot component for interactive point-and-click images
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ImageHotspotProps {
    /// Image source URL
    pub src: BoundValue,
    /// Alternative text for accessibility
    #[serde(skip_serializing_if = "Option::is_none")]
    pub alt: Option<BoundValue>,
    /// Array of hotspots
    pub hotspots: BoundValue,
    /// Show marker indicators
    #[serde(skip_serializing_if = "Option::is_none")]
    pub show_markers: Option<BoundValue>,
    /// Marker style: "pulse", "dot", "ring", "square", "diamond", "none"
    #[serde(skip_serializing_if = "Option::is_none")]
    pub marker_style: Option<BoundValue>,
    /// Image fit: "contain", "cover", "fill"
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fit: Option<BoundValue>,
    /// Use normalized coordinates (0-1) instead of pixels
    #[serde(skip_serializing_if = "Option::is_none")]
    pub normalized: Option<BoundValue>,
    /// Show tooltips on hover
    #[serde(skip_serializing_if = "Option::is_none")]
    pub show_tooltips: Option<BoundValue>,
}

// =============================================================================
// Bounding Box Overlay (Display Only)
// =============================================================================

/// A bounding box for display overlay
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct BoundingBox {
    /// Optional unique identifier
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    /// X coordinate
    pub x: f64,
    /// Y coordinate
    pub y: f64,
    /// Box width
    pub width: f64,
    /// Box height
    pub height: f64,
    /// Optional label text
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    /// Optional confidence score (0-1)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub confidence: Option<f64>,
    /// Optional custom color
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
}

/// Bounding Box Overlay component for displaying detection results
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct BoundingBoxOverlayProps {
    /// Image source URL
    pub src: BoundValue,
    /// Alternative text for accessibility
    #[serde(skip_serializing_if = "Option::is_none")]
    pub alt: Option<BoundValue>,
    /// Array of bounding boxes to display
    pub boxes: BoundValue,
    /// Show labels on boxes
    #[serde(skip_serializing_if = "Option::is_none")]
    pub show_labels: Option<BoundValue>,
    /// Show confidence scores
    #[serde(skip_serializing_if = "Option::is_none")]
    pub show_confidence: Option<BoundValue>,
    /// Stroke width for boxes
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stroke_width: Option<BoundValue>,
    /// Font size for labels
    #[serde(skip_serializing_if = "Option::is_none")]
    pub font_size: Option<BoundValue>,
    /// Image fit: "contain", "cover", "fill"
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fit: Option<BoundValue>,
    /// Use normalized coordinates (0-1)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub normalized: Option<BoundValue>,
    /// Enable click events on boxes
    #[serde(skip_serializing_if = "Option::is_none")]
    pub interactive: Option<BoundValue>,
}

// =============================================================================
// File Preview
// =============================================================================

/// File Preview component for displaying various file types
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct FilePreviewProps {
    /// File URL or data URI
    #[serde(skip_serializing_if = "Option::is_none")]
    pub src: Option<BoundValue>,
    /// Alias for `src`; media-source update nodes write both fields.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<BoundValue>,
    /// File name (used for type detection if not specified)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub filename: Option<BoundValue>,
    /// MIME type used for preview selection when available.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mime_type: Option<BoundValue>,
    /// File type override: "pdf", "image", "video", "audio", "code", "text"
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file_type: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub show_controls: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fit: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fallback_text: Option<BoundValue>,
    /// Height of the preview area
    #[serde(skip_serializing_if = "Option::is_none")]
    pub height: Option<BoundValue>,
    /// Show download button
    #[serde(skip_serializing_if = "Option::is_none")]
    pub show_download: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub loading: Option<BoundValue>,
    /// Audio only: animated visualizer style ("conservative" | "waveform" | "orb" | "vortex" | "shader" | "aurora" | "pulse"). When set, renders an animated player instead of the default controls.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub variant: Option<BoundValue>,
    /// Audio (animated `variant`) only: auto-play when the source is set, e.g. a conversation reply. Default false.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auto_play: Option<BoundValue>,
}

/// Diff viewer component for text, code, markdown and documents
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct DiffViewProps {
    /// Left / old content, or a document URL
    pub original: BoundValue,
    /// Right / new content, or a document URL
    pub modified: BoundValue,
    /// Layout: "split" | "unified" | "inline"
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mode: Option<BoundValue>,
    /// Content kind: "auto" | "text" | "code" | "markdown" | "json" | "document"
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kind: Option<BoundValue>,
    /// Syntax language for code/json (e.g. "typescript", "rust")
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<BoundValue>,
    /// Markdown rendering: "source" | "rendered"
    #[serde(skip_serializing_if = "Option::is_none")]
    pub markdown_mode: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub show_line_numbers: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub word_wrap: Option<BoundValue>,
    /// Intra-line word-level highlighting
    #[serde(skip_serializing_if = "Option::is_none")]
    pub word_level: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub collapse_unchanged: Option<BoundValue>,
    /// Context lines kept around changes when collapsing
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context_lines: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub show_stats: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub original_label: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modified_label: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ignore_whitespace: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ignore_case: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trim_trailing_whitespace: Option<BoundValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub swap_sides: Option<BoundValue>,
}

// =============================================================================
// Planning Components (Calendar & Gantt)
// =============================================================================

/// Interactive calendar for viewing and scheduling events. `events` binds an
/// array of `CalendarEvent` objects; interactions (create/update/move/resize/
/// open/delete) fire the element's bound `workflow_event` action. Clicking an
/// event opens a detail dialog; right-click offers edit/duplicate/delete.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct CalendarProps {
    /// Array of calendar events (see `CalendarEvent`).
    pub events: BoundValue,
    /// Active view: "month" | "week" | "day" | "agenda".
    #[serde(skip_serializing_if = "Option::is_none")]
    pub view: Option<BoundValue>,
    /// Focused date (ISO 8601) the view centers on.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub date: Option<BoundValue>,
    /// Optional header title shown next to the navigation.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<BoundValue>,
    /// Visual density: "compact" | "default" | "comfortable".
    #[serde(skip_serializing_if = "Option::is_none")]
    pub density: Option<BoundValue>,
    /// Allow moving/resizing events by dragging.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub editable: Option<BoundValue>,
    /// Allow selecting empty slots to create events.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub selectable: Option<BoundValue>,
    /// First day of week (0 = Sunday, 1 = Monday).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub first_day_of_week: Option<BoundValue>,
    /// Earliest time shown in week/day views (e.g. "06:00").
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min_time: Option<BoundValue>,
    /// Latest time shown in week/day views (e.g. "22:00").
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_time: Option<BoundValue>,
    /// Time-slot granularity in minutes (week/day views).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub slot_duration: Option<BoundValue>,
    /// Show Saturday/Sunday columns.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub show_weekends: Option<BoundValue>,
    /// Show the current-time indicator line.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub show_now_indicator: Option<BoundValue>,
    /// Show the all-day row above the time grid.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub show_all_day: Option<BoundValue>,
    /// Show the month/week/day view switcher in the header.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub show_view_switcher: Option<BoundValue>,
    /// BCP-47 locale for date formatting (e.g. "en-US", "de-DE").
    #[serde(skip_serializing_if = "Option::is_none")]
    pub locale: Option<BoundValue>,
    /// Explicit height (e.g. "640px").
    #[serde(skip_serializing_if = "Option::is_none")]
    pub height: Option<BoundValue>,
    /// Enable automatic compact/agenda fallback on narrow containers.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub responsive: Option<BoundValue>,
    /// Width in px below which the calendar collapses to the agenda view.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub compact_breakpoint: Option<BoundValue>,
}

/// Interactive Gantt timeline for planning tasks. `tasks` binds an array of
/// `GanttTask` objects; interactions (create/update/move/resize/open/delete/
/// link/reorder) fire the element's bound `workflow_event` action. Clicking a
/// bar opens a detail dialog; right-click offers edit/duplicate/progress/
/// delete; the task list supports drag-and-drop reordering.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct GanttProps {
    /// Array of tasks (see `GanttTask`).
    pub tasks: BoundValue,
    /// Timeline zoom: "day" | "week" | "month" | "quarter" | "compact".
    #[serde(skip_serializing_if = "Option::is_none")]
    pub view: Option<BoundValue>,
    /// Header title (default "Timeline").
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<BoundValue>,
    /// Visual density: "compact" | "default" | "comfortable".
    #[serde(skip_serializing_if = "Option::is_none")]
    pub density: Option<BoundValue>,
    /// Master switch for editing (drag/resize/link). Falls back to `draggable`
    /// / `resizable` when unset.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub editable: Option<BoundValue>,
    /// Allow moving task bars horizontally.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub draggable: Option<BoundValue>,
    /// Allow resizing task bars by their edges.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resizable: Option<BoundValue>,
    /// Draw dependency arrows between tasks.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub show_dependencies: Option<BoundValue>,
    /// Render the progress fill inside each bar.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub show_progress: Option<BoundValue>,
    /// Draw the "today" marker line.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub show_today: Option<BoundValue>,
    /// Show the day/week/month/quarter view switcher in the header.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub show_view_switcher: Option<BoundValue>,
    /// Show the left task-list panel.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub show_task_list: Option<BoundValue>,
    /// Width in px of the left task-list panel.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub task_list_width: Option<BoundValue>,
    /// Shade Saturday/Sunday columns in day/week views.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shade_weekends: Option<BoundValue>,
    /// Row height in px.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub row_height: Option<BoundValue>,
    /// Extra left-panel columns (e.g. ["assignee", "progress"]).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub columns: Option<BoundValue>,
    /// Explicit height (e.g. "640px").
    #[serde(skip_serializing_if = "Option::is_none")]
    pub height: Option<BoundValue>,
    /// Enable automatic compact fallback on narrow containers.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub responsive: Option<BoundValue>,
    /// Width in px below which the timeline collapses to the compact view.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub compact_breakpoint: Option<BoundValue>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn frontend_component_variants_deserialize_on_the_rust_boundary() {
        let components = [
            json!({ "type": "box", "as": { "literalString": "section" } }),
            json!({ "type": "center", "inline": { "literalBool": true } }),
            json!({ "type": "spacer", "size": { "literalString": "1rem" } }),
            json!({
                "type": "widgetInstance",
                "instanceId": "instance-1",
                "widgetId": "widget-1"
            }),
        ];

        for component in components {
            serde_json::from_value::<A2UIComponentType>(component)
                .expect("frontend component should deserialize");
        }
    }

    #[test]
    fn extended_frontend_props_use_the_expected_camel_case_names() {
        let component: A2UIComponentType = serde_json::from_value(json!({
            "type": "model3d",
            "src": { "literalString": "/models/example.glb" },
            "viewerHeight": { "literalString": "320px" },
            "environmentSource": { "literalString": "polyhaven" },
            "groundFollowCamera": { "literalBool": true }
        }))
        .expect("extended model3d props should deserialize");

        let serialized = serde_json::to_value(component).expect("component should serialize");
        assert!(serialized.get("viewerHeight").is_some());
        assert!(serialized.get("environmentSource").is_some());
        assert!(serialized.get("groundFollowCamera").is_some());
    }

    #[test]
    fn file_preview_sources_are_optional_but_nivo_chart_type_is_required() {
        serde_json::from_value::<A2UIComponentType>(json!({
            "type": "filePreview",
            "url": { "literalString": "/files/example.pdf" },
            "mimeType": { "literalString": "application/pdf" }
        }))
        .expect("filePreview should accept url without src");

        assert!(
            serde_json::from_value::<A2UIComponentType>(json!({ "type": "nivoChart" })).is_err()
        );
        serde_json::from_value::<A2UIComponentType>(json!({
            "type": "nivoChart",
            "chartType": { "literalString": "bar" }
        }))
        .expect("nivoChart should accept a required chartType");
    }

    #[test]
    fn graph_components_require_their_data_source() {
        assert!(serde_json::from_value::<A2UIComponentType>(json!({ "type": "graph" })).is_err());
        serde_json::from_value::<A2UIComponentType>(json!({
            "type": "graph",
            "nodes": { "path": "graph.nodes" },
            "edges": { "path": "graph.edges" },
            "showLegend": { "literalBool": false }
        }))
        .expect("graph should accept bound nodes and edges");

        assert!(
            serde_json::from_value::<A2UIComponentType>(json!({ "type": "ontologyGraph" }))
                .is_err()
        );
        let component: A2UIComponentType = serde_json::from_value(json!({
            "type": "ontologyGraph",
            "ontologyId": { "literalString": "ontology-1" },
            "allowCypher": { "literalBool": true },
            "allowStyleEdit": { "literalBool": false }
        }))
        .expect("ontologyGraph should accept a required ontologyId");

        let serialized = serde_json::to_value(component).expect("component should serialize");
        assert_eq!(serialized["type"], "ontologyGraph");
        assert!(serialized.get("ontologyId").is_some());
        assert!(serialized.get("allowStyleEdit").is_some());
    }

    #[test]
    fn generated_prop_schemas_match_the_frontend_wire_contract() {
        let file_preview = serde_json::to_value(schemars::schema_for!(FilePreviewProps))
            .expect("filePreview schema should serialize");
        let file_properties = file_preview["properties"]
            .as_object()
            .expect("filePreview schema should expose properties");
        for property in ["src", "url", "mimeType", "showControls", "fallbackText"] {
            assert!(file_properties.contains_key(property));
        }
        assert!(
            file_preview["required"]
                .as_array()
                .is_none_or(|required| required.iter().all(|field| field.as_str() != Some("src")))
        );

        let nivo = serde_json::to_value(schemars::schema_for!(NivoChartProps))
            .expect("nivoChart schema should serialize");
        assert!(nivo["properties"]["chartType"].is_object());
        assert!(nivo["required"].as_array().is_some_and(|required| {
            required
                .iter()
                .any(|field| field.as_str() == Some("chartType"))
        }));

        let model = serde_json::to_value(schemars::schema_for!(Model3dProps))
            .expect("model3d schema should serialize");
        let model_properties = model["properties"]
            .as_object()
            .expect("model3d schema should expose properties");
        for property in [
            "viewerHeight",
            "cameraTarget",
            "lightingPreset",
            "environmentSource",
            "groundFollowCamera",
        ] {
            assert!(model_properties.contains_key(property));
        }
    }
}
