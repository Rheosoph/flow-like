use std::{collections::HashMap, time::SystemTime};

use flow_like_types::Value;
use flow_like_types::json::{Deserialize, Serialize};
use schemars::JsonSchema;

use super::{DataEntry, SurfaceComponent};

/// Version tuple for widgets and pages (major, minor, patch)
pub type Version = (u32, u32, u32);

/// Widget definition - a reusable UI component that can be instantiated in pages
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Widget {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub root_component_id: String,
    pub components: Vec<SurfaceComponent>,
    pub data_model: Vec<DataEntry>,
    pub customization_options: Vec<CustomizationOption>,
    /// Props exposed from widget components that can be customized when used in a page
    #[serde(default)]
    pub exposed_props: Vec<ExposedProp>,
    pub catalog_id: Option<String>,
    pub thumbnail: Option<String>,
    pub tags: Vec<String>,
    /// Semantic versioning
    pub version: Option<Version>,
    #[serde(
        serialize_with = "crate::utils::serde_helpers::serialize_systemtime",
        deserialize_with = "crate::utils::serde_helpers::deserialize_systemtime"
    )]
    pub created_at: SystemTime,
    #[serde(
        serialize_with = "crate::utils::serde_helpers::serialize_systemtime",
        deserialize_with = "crate::utils::serde_helpers::deserialize_systemtime"
    )]
    pub updated_at: SystemTime,
    /// Widget actions that can be triggered by elements and bound to workflows
    #[serde(default)]
    pub actions: Vec<WidgetAction>,
}

impl Widget {
    pub fn new(
        id: impl Into<String>,
        name: impl Into<String>,
        root_component_id: impl Into<String>,
    ) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            description: None,
            root_component_id: root_component_id.into(),
            components: Vec::new(),
            data_model: Vec::new(),
            customization_options: Vec::new(),
            exposed_props: Vec::new(),
            catalog_id: None,
            thumbnail: None,
            tags: Vec::new(),
            version: Some((0, 0, 1)),
            created_at: SystemTime::now(),
            updated_at: SystemTime::now(),
            actions: Vec::new(),
        }
    }

    pub fn with_description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }

    pub fn with_component(mut self, component: SurfaceComponent) -> Self {
        self.components.push(component);
        self
    }

    pub fn with_data(mut self, entry: DataEntry) -> Self {
        self.data_model.push(entry);
        self
    }

    pub fn with_customization(mut self, option: CustomizationOption) -> Self {
        self.customization_options.push(option);
        self
    }

    pub fn with_exposed_prop(mut self, prop: ExposedProp) -> Self {
        self.exposed_props.push(prop);
        self
    }

    pub fn with_action(mut self, action: WidgetAction) -> Self {
        self.actions.push(action);
        self
    }

    pub fn with_tag(mut self, tag: impl Into<String>) -> Self {
        self.tags.push(tag.into());
        self
    }

    pub fn bump_version(&mut self, version_type: VersionType) {
        let current = self.version.unwrap_or((0, 0, 0));
        self.version = Some(match version_type {
            VersionType::Major => (current.0 + 1, 0, 0),
            VersionType::Minor => (current.0, current.1 + 1, 0),
            VersionType::Patch => (current.0, current.1, current.2 + 1),
        });
        self.updated_at = SystemTime::now();
    }
}

/// Customization option for a widget
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct CustomizationOption {
    pub id: String,
    pub label: String,
    pub description: Option<String>,
    pub customization_type: CustomizationType,
    pub default_value: Option<Vec<u8>>,
    pub validations: Vec<ValidationRule>,
    pub group: Option<String>,
}

impl CustomizationOption {
    pub fn new(
        id: impl Into<String>,
        label: impl Into<String>,
        customization_type: CustomizationType,
    ) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            description: None,
            customization_type,
            default_value: None,
            validations: Vec::new(),
            group: None,
        }
    }
}

/// Exposed prop from a widget component that can be customized when used in a page
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ExposedProp {
    /// Unique identifier for this exposed prop
    pub id: String,
    /// Display label in the UI
    pub label: String,
    /// Description of what this prop does
    pub description: Option<String>,
    /// The component ID within the widget that this prop targets
    pub target_component_id: String,
    /// The property path on the component (e.g., "content", "style.className", "data")
    pub property_path: String,
    /// The type of value this prop accepts
    pub prop_type: ExposedPropType,
    /// Default value (serialized)
    pub default_value: Option<Vec<u8>>,
    /// Group for organizing props in the UI (e.g., "Content", "Style", "Data")
    pub group: Option<String>,
}

impl ExposedProp {
    pub fn new(
        id: impl Into<String>,
        label: impl Into<String>,
        target_component_id: impl Into<String>,
        property_path: impl Into<String>,
        prop_type: ExposedPropType,
    ) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            description: None,
            target_component_id: target_component_id.into(),
            property_path: property_path.into(),
            prop_type,
            default_value: None,
            group: None,
        }
    }

    pub fn with_description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }

    pub fn with_group(mut self, group: impl Into<String>) -> Self {
        self.group = Some(group.into());
        self
    }

    pub fn with_default<T: Serialize>(mut self, value: T) -> Self {
        self.default_value = flow_like_types::json::to_vec(&value).ok();
        self
    }
}

/// Type of exposed property value
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum ExposedPropType {
    /// Plain string value
    String,
    /// Numeric value
    Number,
    /// Boolean value
    Boolean,
    /// Color value (hex, rgb, etc.)
    Color,
    /// Image URL
    ImageUrl,
    /// Icon name
    Icon,
    /// Enum with specific choices
    Enum { choices: Vec<String> },
    /// JSON data (for complex values like chart data)
    Json,
    /// Tailwind CSS class string
    TailwindClass,
    /// Style object (for style overrides)
    StyleObject,
    /// BoundValue - can be literal or data binding
    BoundValue,
}

/// Type of customization value
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum CustomizationType {
    String,
    Number,
    Boolean,
    Color,
    ImageUrl,
    Icon,
    Enum,
    Json,
}

/// Validation rule for customization
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ValidationRule {
    pub rule_type: String,
    pub value: Option<Vec<u8>>,
    pub message: Option<String>,
}

/// Widget action definition - an action that can be triggered by elements and bound to workflows
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct WidgetAction {
    /// Unique identifier for this action (e.g., "clicked_delete", "clicked_open")
    pub id: String,
    /// Display label for the UI
    pub label: String,
    /// Description of what this action does
    pub description: Option<String>,
    /// Icon name for the UI (e.g., "trash", "external-link")
    pub icon: Option<String>,
    /// Context schema - defines what data is passed with the action
    #[serde(default)]
    pub context_schema: Vec<WidgetActionContextField>,
}

impl WidgetAction {
    pub fn new(id: impl Into<String>, label: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            description: None,
            icon: None,
            context_schema: Vec::new(),
        }
    }

    pub fn with_description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }

    pub fn with_icon(mut self, icon: impl Into<String>) -> Self {
        self.icon = Some(icon.into());
        self
    }

    pub fn with_context_field(mut self, field: WidgetActionContextField) -> Self {
        self.context_schema.push(field);
        self
    }
}

/// Field definition for widget action context
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct WidgetActionContextField {
    /// Field name in context
    pub name: String,
    /// Display label
    pub label: String,
    /// Type of the field value
    pub field_type: ExposedPropType,
    /// Description of the field
    pub description: Option<String>,
    /// Default data path binding
    pub default_path: Option<String>,
}

impl WidgetActionContextField {
    pub fn new(
        name: impl Into<String>,
        label: impl Into<String>,
        field_type: ExposedPropType,
    ) -> Self {
        Self {
            name: name.into(),
            label: label.into(),
            field_type,
            description: None,
            default_path: None,
        }
    }

    pub fn with_description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }

    pub fn with_default_path(mut self, path: impl Into<String>) -> Self {
        self.default_path = Some(path.into());
        self
    }
}

/// Payload containing action context fields from a widget action trigger.
/// Keys are field names defined in the action's context_schema.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, Default)]
pub struct ActionContextPayload(pub HashMap<String, Value>);

/// Payload containing current values of event-relevant widget components.
/// Keys are component IDs, values are the components' current state values.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, Default)]
pub struct InputValuesPayload(pub HashMap<String, Value>);

/// Reference to a widget in another app
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct WidgetRef {
    /// The app ID where the widget is defined
    pub app_id: String,
    /// The widget ID
    pub widget_id: String,
    /// Optional version pinning (latest if None)
    pub version: Option<Version>,
}

impl WidgetRef {
    pub fn new(app_id: impl Into<String>, widget_id: impl Into<String>) -> Self {
        Self {
            app_id: app_id.into(),
            widget_id: widget_id.into(),
            version: None,
        }
    }

    pub fn with_version(mut self, major: u32, minor: u32, patch: u32) -> Self {
        self.version = Some((major, minor, patch));
        self
    }
}

/// Binding configuration for a widget action
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum ActionBinding {
    /// Trigger a workflow event (events_simple node ID)
    WorkflowEvent {
        event_id: String,
        #[serde(default)]
        context_mapping: std::collections::HashMap<String, super::BoundValue>,
    },
    /// Navigate to a page
    PageNavigation {
        page_id: String,
        #[serde(default)]
        context_mapping: std::collections::HashMap<String, super::BoundValue>,
    },
    /// Open an external URL
    ExternalUrl {
        url: String,
        #[serde(default)]
        new_tab: bool,
    },
    /// Emit a custom action for handling
    CustomAction {
        action_name: String,
        #[serde(default)]
        context_mapping: std::collections::HashMap<String, super::BoundValue>,
    },
}

/// Page definition - a top-level UI container with routing
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Page {
    pub id: String,
    pub name: String,
    pub route: String,
    pub title: Option<String>,
    /// Canvas settings for page styling (background, padding, custom CSS)
    #[serde(default)]
    pub canvas_settings: Option<CanvasSettings>,
    pub content: Vec<PageContent>,
    pub layout_type: PageLayoutType,
    pub attached_element_id: Option<String>,
    pub meta: Option<PageMeta>,
    pub components: Vec<SurfaceComponent>,
    /// Semantic versioning
    pub version: Option<Version>,
    #[serde(
        serialize_with = "crate::utils::serde_helpers::serialize_systemtime",
        deserialize_with = "crate::utils::serde_helpers::deserialize_systemtime"
    )]
    pub created_at: SystemTime,
    #[serde(
        serialize_with = "crate::utils::serde_helpers::serialize_systemtime",
        deserialize_with = "crate::utils::serde_helpers::deserialize_systemtime"
    )]
    pub updated_at: SystemTime,
    /// Reference to parent board
    pub board_id: Option<String>,
    /// Node ID (from events_simple) to execute when page loads
    pub on_load_event_id: Option<String>,
    /// Node ID to execute when page unloads/user navigates away
    pub on_unload_event_id: Option<String>,
    /// Node ID to execute on a timed interval
    pub on_interval_event_id: Option<String>,
    /// Interval time in seconds (must be > 0)
    pub on_interval_seconds: Option<u32>,
    /// Widget definitions referenced by widget instances on this page
    /// Key is the instance ID, value is the widget definition
    #[serde(default)]
    pub widget_refs: HashMap<String, Widget>,
    /// When true, the frontend caches the last rendered state and shows it
    /// instantly while the onLoad event runs in the background.
    #[serde(default)]
    pub cache: bool,
}

impl Page {
    pub fn new(id: impl Into<String>, name: impl Into<String>, route: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            route: route.into(),
            title: None,
            canvas_settings: None,
            content: Vec::new(),
            layout_type: PageLayoutType::Freeform,
            attached_element_id: None,
            meta: None,
            components: Vec::new(),
            version: Some((0, 0, 1)),
            created_at: SystemTime::now(),
            updated_at: SystemTime::now(),
            board_id: None,
            on_load_event_id: None,
            on_unload_event_id: None,
            on_interval_event_id: None,
            on_interval_seconds: None,
            widget_refs: HashMap::new(),
            cache: false,
        }
    }

    pub fn with_board_id(mut self, board_id: impl Into<String>) -> Self {
        self.board_id = Some(board_id.into());
        self
    }

    pub fn with_title(mut self, title: impl Into<String>) -> Self {
        self.title = Some(title.into());
        self
    }

    pub fn with_layout(mut self, layout_type: PageLayoutType) -> Self {
        self.layout_type = layout_type;
        self
    }

    pub fn with_content(mut self, content: PageContent) -> Self {
        self.content.push(content);
        self
    }

    pub fn with_component(mut self, component: SurfaceComponent) -> Self {
        self.components.push(component);
        self
    }

    pub fn bump_version(&mut self, version_type: VersionType) {
        let current = self.version.unwrap_or((0, 0, 0));
        self.version = Some(match version_type {
            VersionType::Major => (current.0 + 1, 0, 0),
            VersionType::Minor => (current.0, current.1 + 1, 0),
            VersionType::Patch => (current.0, current.1, current.2 + 1),
        });
        self.updated_at = SystemTime::now();
    }
}

/// Canvas settings for page styling used by the visual builder
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, Default)]
#[serde(rename_all = "camelCase")]
pub struct CanvasSettings {
    pub background_color: Option<String>,
    pub background_image: Option<String>,
    pub padding: Option<String>,
    /// Custom CSS to inject into the page (scoped to page container).
    pub custom_css: Option<String>,
}

/// Content that can be placed on a page
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum PageContent {
    Widget(WidgetInstance),
    Component(SurfaceComponent),
    ComponentRef(String),
}

/// An instance of a widget placed on a page
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct WidgetInstance {
    pub widget_id: String,
    pub instance_id: String,
    pub position: Option<super::Position>,
    pub customization_values: std::collections::HashMap<String, Vec<u8>>,
    /// Values for exposed props (key is the exposed prop id)
    #[serde(default)]
    pub exposed_prop_values: std::collections::HashMap<String, Vec<u8>>,
    pub style_override: Option<super::Style>,
    /// Action bindings - map action_id to binding configuration
    #[serde(default)]
    pub action_bindings: std::collections::HashMap<String, ActionBinding>,
    /// Widget reference for cross-app widgets (optional - if not same app)
    pub widget_ref: Option<WidgetRef>,
}

impl WidgetInstance {
    pub fn new(widget_id: impl Into<String>, instance_id: impl Into<String>) -> Self {
        Self {
            widget_id: widget_id.into(),
            instance_id: instance_id.into(),
            position: None,
            customization_values: std::collections::HashMap::new(),
            exposed_prop_values: std::collections::HashMap::new(),
            style_override: None,
            action_bindings: std::collections::HashMap::new(),
            widget_ref: None,
        }
    }

    pub fn with_widget_ref(mut self, widget_ref: WidgetRef) -> Self {
        self.widget_ref = Some(widget_ref);
        self
    }

    pub fn with_action_binding(
        mut self,
        action_id: impl Into<String>,
        binding: ActionBinding,
    ) -> Self {
        self.action_bindings.insert(action_id.into(), binding);
        self
    }

    pub fn with_exposed_prop_value<T: Serialize>(
        mut self,
        prop_id: impl Into<String>,
        value: T,
    ) -> Self {
        if let Ok(bytes) = flow_like_types::json::to_vec(&value) {
            self.exposed_prop_values.insert(prop_id.into(), bytes);
        }
        self
    }
}

/// Page layout type
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum PageLayoutType {
    #[default]
    Freeform,
    Stack,
    Grid,
    Sidebar,
    HolyGrail,
}

/// Page meta information (SEO, etc.)
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct PageMeta {
    pub description: Option<String>,
    pub og_image: Option<String>,
    pub keywords: Vec<String>,
    pub favicon: Option<String>,
    pub theme_color: Option<String>,
}

/// Version bump type
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub enum VersionType {
    Major,
    Minor,
    Patch,
}

#[cfg(test)]
mod tests {
    use super::*;
    use flow_like_types::Value;

    #[test]
    fn test_widget_creation() {
        let widget = Widget::new("test-id", "Test Widget", "root-1");

        assert_eq!(widget.id, "test-id");
        assert_eq!(widget.name, "Test Widget");
        assert_eq!(widget.root_component_id, "root-1");
        assert!(widget.description.is_none());
        assert!(widget.components.is_empty());
        assert_eq!(widget.version, Some((0, 0, 1)));
    }

    #[test]
    fn test_widget_builder_pattern() {
        let widget = Widget::new("w1", "My Widget", "root")
            .with_description("A test widget")
            .with_tag("test")
            .with_tag("demo");

        assert_eq!(widget.description, Some("A test widget".to_string()));
        assert_eq!(widget.tags, vec!["test", "demo"]);
    }

    #[test]
    fn test_widget_version_bump() {
        let mut widget = Widget::new("v1", "Versioned", "root");
        assert_eq!(widget.version, Some((0, 0, 1)));

        widget.bump_version(VersionType::Patch);
        assert_eq!(widget.version, Some((0, 0, 2)));

        widget.bump_version(VersionType::Minor);
        assert_eq!(widget.version, Some((0, 1, 0)));

        widget.bump_version(VersionType::Major);
        assert_eq!(widget.version, Some((1, 0, 0)));
    }

    #[test]
    fn test_page_creation() {
        let page = Page::new("page-1", "Home", "/");

        assert_eq!(page.id, "page-1");
        assert_eq!(page.name, "Home");
        assert_eq!(page.route, "/");
        assert!(page.title.is_none());
        assert_eq!(page.version, Some((0, 0, 1)));
    }

    #[test]
    fn test_page_builder_pattern() {
        let page = Page::new("p1", "Dashboard", "/dashboard")
            .with_title("My Dashboard")
            .with_layout(PageLayoutType::Sidebar);

        assert_eq!(page.title, Some("My Dashboard".to_string()));
        assert!(matches!(page.layout_type, PageLayoutType::Sidebar));
    }

    #[test]
    fn test_page_version_bump() {
        let mut page = Page::new("p1", "Test", "/test");
        assert_eq!(page.version, Some((0, 0, 1)));

        page.bump_version(VersionType::Minor);
        assert_eq!(page.version, Some((0, 1, 0)));
    }

    #[test]
    fn test_customization_option() {
        let opt = CustomizationOption::new("color", "Primary Color", CustomizationType::Color);

        assert_eq!(opt.id, "color");
        assert_eq!(opt.label, "Primary Color");
        assert!(matches!(opt.customization_type, CustomizationType::Color));
    }

    #[test]
    fn test_widget_instance() {
        let instance = WidgetInstance::new("widget-1", "instance-1");

        assert_eq!(instance.widget_id, "widget-1");
        assert_eq!(instance.instance_id, "instance-1");
        assert!(instance.customization_values.is_empty());
    }

    #[test]
    fn test_page_layout_type_default() {
        let layout: PageLayoutType = Default::default();
        assert!(matches!(layout, PageLayoutType::Freeform));
    }

    #[test]
    fn test_page_layout_type_uses_camel_case_wire_values() {
        let cases = [
            (PageLayoutType::Freeform, "freeform"),
            (PageLayoutType::Stack, "stack"),
            (PageLayoutType::Grid, "grid"),
            (PageLayoutType::Sidebar, "sidebar"),
            (PageLayoutType::HolyGrail, "holyGrail"),
        ];

        for (layout, expected) in cases {
            assert_eq!(
                flow_like_types::json::to_value(layout).expect("layout should serialize"),
                Value::String(expected.to_string()),
            );
        }
    }

    #[test]
    fn test_page_content_variants() {
        let widget_content = PageContent::Widget(WidgetInstance::new("w1", "i1"));
        let ref_content = PageContent::ComponentRef("comp-1".to_string());

        assert!(matches!(widget_content, PageContent::Widget(_)));
        assert!(matches!(ref_content, PageContent::ComponentRef(_)));
    }
}
