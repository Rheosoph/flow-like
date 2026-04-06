use std::time::SystemTime;

use flow_like_types::{Timestamp, json, proto};

use crate::a2ui::widget::{
    ActionBinding, CanvasSettings, CustomizationOption, CustomizationType, ExposedProp,
    ExposedPropType, Page, PageContent, PageLayoutType, PageMeta, ValidationRule, Widget,
    WidgetAction, WidgetActionContextField, WidgetInstance, WidgetRef,
};

// ============================================================================
// Widget Conversions
// ============================================================================

impl From<Widget> for proto::Widget {
    fn from(value: Widget) -> Self {
        proto::Widget {
            id: value.id,
            name: value.name,
            description: value.description,
            root_component_id: value.root_component_id,
            components: value.components.into_iter().map(Into::into).collect(),
            data_model: value
                .data_model
                .into_iter()
                .map(|d| proto::DataEntry {
                    key: d.key,
                    value: json::to_vec(&d.value).unwrap_or_default(),
                })
                .collect(),
            customization_options: value
                .customization_options
                .into_iter()
                .map(Into::into)
                .collect(),
            exposed_props: value.exposed_props.into_iter().map(Into::into).collect(),
            catalog_id: value.catalog_id,
            thumbnail: value.thumbnail,
            tags: value.tags,
            version: value.version.map(|v| proto::WidgetVersion {
                major: v.0,
                minor: v.1,
                patch: v.2,
            }),
            created_at: Some(Timestamp::from(value.created_at)),
            updated_at: Some(Timestamp::from(value.updated_at)),
            actions: value.actions.into_iter().map(Into::into).collect(),
        }
    }
}

impl From<proto::Widget> for Widget {
    fn from(proto: proto::Widget) -> Self {
        Widget {
            id: proto.id,
            name: proto.name,
            description: proto.description,
            root_component_id: proto.root_component_id,
            components: proto.components.into_iter().map(Into::into).collect(),
            data_model: proto
                .data_model
                .into_iter()
                .map(|d| crate::a2ui::DataEntry {
                    key: d.key,
                    value: json::from_slice(&d.value).unwrap_or_default(),
                })
                .collect(),
            customization_options: proto
                .customization_options
                .into_iter()
                .map(Into::into)
                .collect(),
            exposed_props: proto.exposed_props.into_iter().map(Into::into).collect(),
            catalog_id: proto.catalog_id,
            thumbnail: proto.thumbnail,
            tags: proto.tags,
            version: proto.version.map(|v| (v.major, v.minor, v.patch)),
            created_at: proto
                .created_at
                .and_then(|t| SystemTime::try_from(t).ok())
                .unwrap_or_else(SystemTime::now),
            updated_at: proto
                .updated_at
                .and_then(|t| SystemTime::try_from(t).ok())
                .unwrap_or_else(SystemTime::now),
            actions: proto.actions.into_iter().map(Into::into).collect(),
        }
    }
}

// ============================================================================
// CustomizationOption Conversions
// ============================================================================

impl From<CustomizationOption> for proto::CustomizationOption {
    fn from(value: CustomizationOption) -> Self {
        proto::CustomizationOption {
            id: value.id,
            label: value.label,
            description: value.description,
            customization_type: proto::CustomizationType::from(value.customization_type) as i32,
            default_value: value.default_value,
            validations: value.validations.into_iter().map(Into::into).collect(),
            group: value.group,
        }
    }
}

impl From<proto::CustomizationOption> for CustomizationOption {
    fn from(proto: proto::CustomizationOption) -> Self {
        CustomizationOption {
            id: proto.id,
            label: proto.label,
            description: proto.description,
            customization_type: proto::CustomizationType::try_from(proto.customization_type)
                .unwrap_or(proto::CustomizationType::CustomizationString)
                .into(),
            default_value: proto.default_value,
            validations: proto.validations.into_iter().map(Into::into).collect(),
            group: proto.group,
        }
    }
}

impl From<CustomizationType> for proto::CustomizationType {
    fn from(value: CustomizationType) -> Self {
        match value {
            CustomizationType::String => proto::CustomizationType::CustomizationString,
            CustomizationType::Number => proto::CustomizationType::CustomizationNumber,
            CustomizationType::Boolean => proto::CustomizationType::CustomizationBoolean,
            CustomizationType::Color => proto::CustomizationType::CustomizationColor,
            CustomizationType::ImageUrl => proto::CustomizationType::CustomizationImageUrl,
            CustomizationType::Icon => proto::CustomizationType::CustomizationIcon,
            CustomizationType::Enum => proto::CustomizationType::CustomizationEnum,
            CustomizationType::Json => proto::CustomizationType::CustomizationJson,
        }
    }
}

impl From<proto::CustomizationType> for CustomizationType {
    fn from(proto: proto::CustomizationType) -> Self {
        match proto {
            proto::CustomizationType::CustomizationString => CustomizationType::String,
            proto::CustomizationType::CustomizationNumber => CustomizationType::Number,
            proto::CustomizationType::CustomizationBoolean => CustomizationType::Boolean,
            proto::CustomizationType::CustomizationColor => CustomizationType::Color,
            proto::CustomizationType::CustomizationImageUrl => CustomizationType::ImageUrl,
            proto::CustomizationType::CustomizationIcon => CustomizationType::Icon,
            proto::CustomizationType::CustomizationEnum => CustomizationType::Enum,
            proto::CustomizationType::CustomizationJson => CustomizationType::Json,
        }
    }
}

impl From<ValidationRule> for proto::ValidationRule {
    fn from(value: ValidationRule) -> Self {
        proto::ValidationRule {
            rule_type: value.rule_type,
            value: value.value,
            message: value.message,
        }
    }
}

impl From<proto::ValidationRule> for ValidationRule {
    fn from(proto: proto::ValidationRule) -> Self {
        ValidationRule {
            rule_type: proto.rule_type,
            value: proto.value,
            message: proto.message,
        }
    }
}

// ============================================================================
// ExposedProp Conversions
// ============================================================================

impl From<ExposedProp> for proto::ExposedProp {
    fn from(value: ExposedProp) -> Self {
        let (prop_type, enum_choices) = match value.prop_type {
            ExposedPropType::Enum { choices } => {
                (proto::ExposedPropType::ExposedPropEnum as i32, choices)
            }
            other => (proto::ExposedPropType::from(other) as i32, Vec::new()),
        };
        proto::ExposedProp {
            id: value.id,
            label: value.label,
            description: value.description,
            target_component_id: value.target_component_id,
            property_path: value.property_path,
            prop_type,
            default_value: value.default_value,
            group: value.group,
            enum_choices,
        }
    }
}

impl From<proto::ExposedProp> for ExposedProp {
    fn from(proto: proto::ExposedProp) -> Self {
        let prop_type = match proto::ExposedPropType::try_from(proto.prop_type)
            .unwrap_or(proto::ExposedPropType::ExposedPropString)
        {
            proto::ExposedPropType::ExposedPropEnum => ExposedPropType::Enum {
                choices: proto.enum_choices,
            },
            other => ExposedPropType::from(other),
        };
        ExposedProp {
            id: proto.id,
            label: proto.label,
            description: proto.description,
            target_component_id: proto.target_component_id,
            property_path: proto.property_path,
            prop_type,
            default_value: proto.default_value,
            group: proto.group,
        }
    }
}

impl From<ExposedPropType> for proto::ExposedPropType {
    fn from(value: ExposedPropType) -> Self {
        match value {
            ExposedPropType::String => proto::ExposedPropType::ExposedPropString,
            ExposedPropType::Number => proto::ExposedPropType::ExposedPropNumber,
            ExposedPropType::Boolean => proto::ExposedPropType::ExposedPropBoolean,
            ExposedPropType::Color => proto::ExposedPropType::ExposedPropColor,
            ExposedPropType::ImageUrl => proto::ExposedPropType::ExposedPropImageUrl,
            ExposedPropType::Icon => proto::ExposedPropType::ExposedPropIcon,
            ExposedPropType::Enum { .. } => proto::ExposedPropType::ExposedPropEnum,
            ExposedPropType::Json => proto::ExposedPropType::ExposedPropJson,
            ExposedPropType::TailwindClass => proto::ExposedPropType::ExposedPropTailwindClass,
            ExposedPropType::StyleObject => proto::ExposedPropType::ExposedPropStyleObject,
            ExposedPropType::BoundValue => proto::ExposedPropType::ExposedPropBoundValue,
        }
    }
}

impl From<proto::ExposedPropType> for ExposedPropType {
    fn from(proto: proto::ExposedPropType) -> Self {
        match proto {
            proto::ExposedPropType::ExposedPropString => ExposedPropType::String,
            proto::ExposedPropType::ExposedPropNumber => ExposedPropType::Number,
            proto::ExposedPropType::ExposedPropBoolean => ExposedPropType::Boolean,
            proto::ExposedPropType::ExposedPropColor => ExposedPropType::Color,
            proto::ExposedPropType::ExposedPropImageUrl => ExposedPropType::ImageUrl,
            proto::ExposedPropType::ExposedPropIcon => ExposedPropType::Icon,
            proto::ExposedPropType::ExposedPropEnum => ExposedPropType::Enum {
                choices: Vec::new(),
            },
            proto::ExposedPropType::ExposedPropJson => ExposedPropType::Json,
            proto::ExposedPropType::ExposedPropTailwindClass => ExposedPropType::TailwindClass,
            proto::ExposedPropType::ExposedPropStyleObject => ExposedPropType::StyleObject,
            proto::ExposedPropType::ExposedPropBoundValue => ExposedPropType::BoundValue,
        }
    }
}

// ============================================================================
// Page Conversions
// ============================================================================

impl From<Page> for proto::Page {
    fn from(value: Page) -> Self {
        proto::Page {
            id: value.id,
            name: value.name,
            route: value.route,
            title: value.title,
            canvas_settings: value.canvas_settings.map(Into::into),
            content: value.content.into_iter().map(Into::into).collect(),
            layout_type: proto::PageLayoutType::from(value.layout_type) as i32,
            attached_element_id: value.attached_element_id,
            meta: value.meta.map(Into::into),
            components: value.components.into_iter().map(Into::into).collect(),
            version: value.version.map(|v| proto::PageVersion {
                major: v.0,
                minor: v.1,
                patch: v.2,
            }),
            created_at: Some(Timestamp::from(value.created_at)),
            updated_at: Some(Timestamp::from(value.updated_at)),
            board_id: value.board_id,
            on_load_event_id: value.on_load_event_id,
            on_unload_event_id: value.on_unload_event_id,
            on_interval_event_id: value.on_interval_event_id,
            on_interval_seconds: value.on_interval_seconds,
            widget_refs: value
                .widget_refs
                .into_iter()
                .map(|(k, v)| (k, v.into()))
                .collect(),
            cache: value.cache,
        }
    }
}

impl From<proto::Page> for Page {
    fn from(proto: proto::Page) -> Self {
        Page {
            id: proto.id,
            name: proto.name,
            route: proto.route,
            title: proto.title,
            canvas_settings: proto.canvas_settings.map(Into::into),
            content: proto.content.into_iter().map(Into::into).collect(),
            layout_type: proto::PageLayoutType::try_from(proto.layout_type)
                .unwrap_or(proto::PageLayoutType::PageLayoutFreeform)
                .into(),
            attached_element_id: proto.attached_element_id,
            meta: proto.meta.map(Into::into),
            components: proto.components.into_iter().map(Into::into).collect(),
            version: proto.version.map(|v| (v.major, v.minor, v.patch)),
            created_at: proto
                .created_at
                .and_then(|t| SystemTime::try_from(t).ok())
                .unwrap_or_else(SystemTime::now),
            updated_at: proto
                .updated_at
                .and_then(|t| SystemTime::try_from(t).ok())
                .unwrap_or_else(SystemTime::now),
            board_id: proto.board_id,
            on_load_event_id: proto.on_load_event_id,
            on_unload_event_id: proto.on_unload_event_id,
            on_interval_event_id: proto.on_interval_event_id,
            on_interval_seconds: proto.on_interval_seconds,
            widget_refs: proto
                .widget_refs
                .into_iter()
                .map(|(k, v)| (k, v.into()))
                .collect(),
            cache: proto.cache,
        }
    }
}

impl From<CanvasSettings> for proto::CanvasSettings {
    fn from(value: CanvasSettings) -> Self {
        proto::CanvasSettings {
            background_color: value.background_color,
            background_image: value.background_image,
            padding: value.padding,
            custom_css: value.custom_css,
        }
    }
}

impl From<proto::CanvasSettings> for CanvasSettings {
    fn from(proto: proto::CanvasSettings) -> Self {
        CanvasSettings {
            background_color: proto.background_color,
            background_image: proto.background_image,
            padding: proto.padding,
            custom_css: proto.custom_css,
        }
    }
}

impl From<PageContent> for proto::PageContent {
    fn from(value: PageContent) -> Self {
        proto::PageContent {
            content_type: Some(match value {
                PageContent::Widget(w) => proto::page_content::ContentType::Widget(w.into()),
                PageContent::Component(c) => proto::page_content::ContentType::Component(c.into()),
                PageContent::ComponentRef(id) => proto::page_content::ContentType::ComponentId(id),
            }),
            grid_placement: None,
            region: None,
        }
    }
}

impl From<proto::PageContent> for PageContent {
    fn from(proto: proto::PageContent) -> Self {
        match proto.content_type {
            Some(proto::page_content::ContentType::Widget(w)) => PageContent::Widget(w.into()),
            Some(proto::page_content::ContentType::Component(c)) => {
                PageContent::Component(c.into())
            }
            Some(proto::page_content::ContentType::ComponentId(id)) => {
                PageContent::ComponentRef(id)
            }
            None => PageContent::ComponentRef(String::new()),
        }
    }
}

impl From<PageLayoutType> for proto::PageLayoutType {
    fn from(value: PageLayoutType) -> Self {
        match value {
            PageLayoutType::Freeform => proto::PageLayoutType::PageLayoutFreeform,
            PageLayoutType::Stack => proto::PageLayoutType::PageLayoutStack,
            PageLayoutType::Grid => proto::PageLayoutType::PageLayoutGrid,
            PageLayoutType::Sidebar => proto::PageLayoutType::PageLayoutSidebar,
            PageLayoutType::HolyGrail => proto::PageLayoutType::PageLayoutHolyGrail,
        }
    }
}

impl From<proto::PageLayoutType> for PageLayoutType {
    fn from(proto: proto::PageLayoutType) -> Self {
        match proto {
            proto::PageLayoutType::PageLayoutFreeform => PageLayoutType::Freeform,
            proto::PageLayoutType::PageLayoutStack => PageLayoutType::Stack,
            proto::PageLayoutType::PageLayoutGrid => PageLayoutType::Grid,
            proto::PageLayoutType::PageLayoutSidebar => PageLayoutType::Sidebar,
            proto::PageLayoutType::PageLayoutHolyGrail => PageLayoutType::HolyGrail,
        }
    }
}

impl From<PageMeta> for proto::PageMeta {
    fn from(value: PageMeta) -> Self {
        proto::PageMeta {
            description: value.description,
            og_image: value.og_image,
            keywords: value.keywords,
            favicon: value.favicon,
            theme_color: value.theme_color,
        }
    }
}

impl From<proto::PageMeta> for PageMeta {
    fn from(proto: proto::PageMeta) -> Self {
        PageMeta {
            description: proto.description,
            og_image: proto.og_image,
            keywords: proto.keywords,
            favicon: proto.favicon,
            theme_color: proto.theme_color,
        }
    }
}

impl From<WidgetInstance> for proto::WidgetInstance {
    fn from(value: WidgetInstance) -> Self {
        proto::WidgetInstance {
            widget_id: value.widget_id,
            instance_id: value.instance_id,
            position: value.position.map(Into::into),
            customization_values: value.customization_values,
            exposed_prop_values: value.exposed_prop_values,
            style_override: value.style_override.map(Into::into),
            action_bindings: value
                .action_bindings
                .into_iter()
                .map(|(k, v)| (k, v.into()))
                .collect(),
            widget_ref: value.widget_ref.map(Into::into),
        }
    }
}

impl From<proto::WidgetInstance> for WidgetInstance {
    fn from(proto: proto::WidgetInstance) -> Self {
        WidgetInstance {
            widget_id: proto.widget_id,
            instance_id: proto.instance_id,
            position: proto.position.map(Into::into),
            customization_values: proto.customization_values,
            exposed_prop_values: proto.exposed_prop_values,
            style_override: proto.style_override.map(Into::into),
            action_bindings: proto
                .action_bindings
                .into_iter()
                .map(|(k, v)| (k, v.into()))
                .collect(),
            widget_ref: proto.widget_ref.map(Into::into),
        }
    }
}

// ============================================================================
// Widget Action Conversions
// ============================================================================

impl From<WidgetAction> for proto::WidgetAction {
    fn from(value: WidgetAction) -> Self {
        proto::WidgetAction {
            id: value.id,
            label: value.label,
            description: value.description,
            icon: value.icon,
            context_schema: value.context_schema.into_iter().map(Into::into).collect(),
        }
    }
}

impl From<proto::WidgetAction> for WidgetAction {
    fn from(proto: proto::WidgetAction) -> Self {
        WidgetAction {
            id: proto.id,
            label: proto.label,
            description: proto.description,
            icon: proto.icon,
            context_schema: proto.context_schema.into_iter().map(Into::into).collect(),
        }
    }
}

impl From<WidgetActionContextField> for proto::WidgetActionContextField {
    fn from(value: WidgetActionContextField) -> Self {
        proto::WidgetActionContextField {
            name: value.name,
            label: value.label,
            field_type: proto::ExposedPropType::from(value.field_type) as i32,
            description: value.description,
            default_path: value.default_path,
        }
    }
}

impl From<proto::WidgetActionContextField> for WidgetActionContextField {
    fn from(proto: proto::WidgetActionContextField) -> Self {
        WidgetActionContextField {
            name: proto.name,
            label: proto.label,
            field_type: proto::ExposedPropType::try_from(proto.field_type)
                .unwrap_or(proto::ExposedPropType::ExposedPropString)
                .into(),
            description: proto.description,
            default_path: proto.default_path,
        }
    }
}

impl From<ActionBinding> for proto::ActionBinding {
    fn from(value: ActionBinding) -> Self {
        match value {
            ActionBinding::WorkflowEvent {
                event_id,
                context_mapping,
            } => proto::ActionBinding {
                binding_type: Some(proto::action_binding::BindingType::WorkflowEventId(
                    event_id,
                )),
                context_mapping: context_mapping
                    .into_iter()
                    .map(|(k, v)| (k, v.into()))
                    .collect(),
            },
            ActionBinding::PageNavigation {
                page_id,
                context_mapping,
            } => proto::ActionBinding {
                binding_type: Some(proto::action_binding::BindingType::PageId(page_id)),
                context_mapping: context_mapping
                    .into_iter()
                    .map(|(k, v)| (k, v.into()))
                    .collect(),
            },
            ActionBinding::ExternalUrl { url, .. } => proto::ActionBinding {
                binding_type: Some(proto::action_binding::BindingType::ExternalUrl(url)),
                context_mapping: std::collections::HashMap::new(),
            },
            ActionBinding::CustomAction {
                action_name,
                context_mapping,
            } => proto::ActionBinding {
                binding_type: Some(proto::action_binding::BindingType::CustomAction(
                    action_name,
                )),
                context_mapping: context_mapping
                    .into_iter()
                    .map(|(k, v)| (k, v.into()))
                    .collect(),
            },
        }
    }
}

impl From<proto::ActionBinding> for ActionBinding {
    fn from(proto: proto::ActionBinding) -> Self {
        let context_mapping: std::collections::HashMap<String, crate::a2ui::BoundValue> = proto
            .context_mapping
            .into_iter()
            .map(|(k, v)| (k, v.into()))
            .collect();

        match proto.binding_type {
            Some(proto::action_binding::BindingType::WorkflowEventId(event_id)) => {
                ActionBinding::WorkflowEvent {
                    event_id,
                    context_mapping,
                }
            }
            Some(proto::action_binding::BindingType::PageId(page_id)) => {
                ActionBinding::PageNavigation {
                    page_id,
                    context_mapping,
                }
            }
            Some(proto::action_binding::BindingType::ExternalUrl(url)) => {
                ActionBinding::ExternalUrl { url, new_tab: true }
            }
            Some(proto::action_binding::BindingType::CustomAction(action_name)) => {
                ActionBinding::CustomAction {
                    action_name,
                    context_mapping,
                }
            }
            None => ActionBinding::CustomAction {
                action_name: String::new(),
                context_mapping,
            },
        }
    }
}

impl From<WidgetRef> for proto::WidgetRef {
    fn from(value: WidgetRef) -> Self {
        proto::WidgetRef {
            app_id: value.app_id,
            widget_id: value.widget_id,
            version: value.version.map(|v| proto::WidgetVersion {
                major: v.0,
                minor: v.1,
                patch: v.2,
            }),
        }
    }
}

impl From<proto::WidgetRef> for WidgetRef {
    fn from(proto: proto::WidgetRef) -> Self {
        WidgetRef {
            app_id: proto.app_id,
            widget_id: proto.widget_id,
            version: proto.version.map(|v| (v.major, v.minor, v.patch)),
        }
    }
}
