//! Schema utilities for A2UI element nodes
//!
//! Provides helpers to set dynamic schemas based on component types.

use flow_like::a2ui::A2UIElement;
use flow_like::a2ui::components::*;
use flow_like::flow::pin::Pin;

/// Sets a schema on a pin based on the component type name.
/// Returns true if a specific schema was set, false if generic was used.
pub fn set_component_schema_by_type(pin: &mut Pin, component_type: &str) -> bool {
    let type_lower = component_type.to_lowercase();

    match type_lower.as_str() {
        // Layout components
        "row" => {
            pin.set_schema::<RowProps>();
            true
        }
        "column" => {
            pin.set_schema::<ColumnProps>();
            true
        }
        "stack" => {
            pin.set_schema::<StackProps>();
            true
        }
        "grid" => {
            pin.set_schema::<GridProps>();
            true
        }
        "scrollarea" => {
            pin.set_schema::<ScrollAreaProps>();
            true
        }
        "aspectratio" => {
            pin.set_schema::<AspectRatioProps>();
            true
        }
        "overlay" => {
            pin.set_schema::<OverlayProps>();
            true
        }
        "absolute" => {
            pin.set_schema::<AbsoluteProps>();
            true
        }

        // Display components
        "text" => {
            pin.set_schema::<TextProps>();
            true
        }
        "image" => {
            pin.set_schema::<ImageProps>();
            true
        }
        "icon" => {
            pin.set_schema::<IconProps>();
            true
        }
        "video" => {
            pin.set_schema::<VideoProps>();
            true
        }
        "lottie" => {
            pin.set_schema::<LottieProps>();
            true
        }
        "markdown" => {
            pin.set_schema::<MarkdownProps>();
            true
        }
        "divider" => {
            pin.set_schema::<DividerProps>();
            true
        }
        "badge" => {
            pin.set_schema::<BadgeProps>();
            true
        }
        "avatar" => {
            pin.set_schema::<AvatarProps>();
            true
        }
        "userprofile" => {
            pin.set_schema::<UserProfileProps>();
            true
        }
        "progress" => {
            pin.set_schema::<ProgressProps>();
            true
        }
        "spinner" => {
            pin.set_schema::<SpinnerProps>();
            true
        }
        "skeleton" => {
            pin.set_schema::<SkeletonProps>();
            true
        }

        // Interactive components
        "button" => {
            pin.set_schema::<ButtonProps>();
            true
        }
        "feedback" => {
            pin.set_schema::<FeedbackProps>();
            true
        }
        "applink" => {
            pin.set_schema::<AppLinkProps>();
            true
        }
        "textfield" => {
            pin.set_schema::<TextFieldProps>();
            true
        }
        "select" => {
            pin.set_schema::<SelectProps>();
            true
        }
        "slider" => {
            pin.set_schema::<SliderProps>();
            true
        }
        "checkbox" => {
            pin.set_schema::<CheckboxProps>();
            true
        }
        "switch" => {
            pin.set_schema::<SwitchProps>();
            true
        }
        "radiogroup" => {
            pin.set_schema::<RadioGroupProps>();
            true
        }
        "datetimeinput" => {
            pin.set_schema::<DateTimeInputProps>();
            true
        }
        "fileinput" => {
            pin.set_schema::<FileInputProps>();
            true
        }
        "imageinput" => {
            pin.set_schema::<ImageInputProps>();
            true
        }
        "link" => {
            pin.set_schema::<LinkProps>();
            true
        }

        // Container components
        "card" => {
            pin.set_schema::<CardProps>();
            true
        }
        "modal" => {
            pin.set_schema::<ModalProps>();
            true
        }
        "tabs" => {
            pin.set_schema::<TabsProps>();
            true
        }
        "accordion" => {
            pin.set_schema::<AccordionProps>();
            true
        }
        "drawer" => {
            pin.set_schema::<DrawerProps>();
            true
        }
        "tooltip" => {
            pin.set_schema::<TooltipProps>();
            true
        }
        "popover" => {
            pin.set_schema::<PopoverProps>();
            true
        }

        // Game components
        "canvas2d" => {
            pin.set_schema::<Canvas2dProps>();
            true
        }
        "sprite" => {
            pin.set_schema::<SpriteProps>();
            true
        }
        "shape" => {
            pin.set_schema::<ShapeProps>();
            true
        }
        "scene3d" => {
            pin.set_schema::<Scene3dProps>();
            true
        }
        "model3d" => {
            pin.set_schema::<Model3dProps>();
            true
        }
        "dialogue" => {
            pin.set_schema::<DialogueProps>();
            true
        }
        "characterportrait" => {
            pin.set_schema::<CharacterPortraitProps>();
            true
        }
        "choicemenu" => {
            pin.set_schema::<ChoiceMenuProps>();
            true
        }
        "inventorygrid" => {
            pin.set_schema::<InventoryGridProps>();
            true
        }
        "healthbar" => {
            pin.set_schema::<HealthBarProps>();
            true
        }
        "minimap" => {
            pin.set_schema::<MiniMapProps>();
            true
        }
        "geomap" => {
            pin.set_schema::<GeoMapProps>();
            true
        }

        // Embeds
        "iframe" => {
            pin.set_schema::<IframeProps>();
            true
        }
        "plotlychart" => {
            pin.set_schema::<PlotlyChartProps>();
            true
        }

        // Unknown type - use generic element
        _ => {
            pin.set_schema::<A2UIElement>();
            false
        }
    }
}

/// Sets a schema on a pin to generic A2UIElement
pub fn set_generic_element_schema(pin: &mut Pin) {
    pin.set_schema::<A2UIElement>();
}

/// Returns a list of valid component type names for documentation/validation
pub fn valid_component_types() -> &'static [&'static str] {
    &[
        // Layout
        "row",
        "column",
        "stack",
        "grid",
        "scrollArea",
        "aspectRatio",
        "overlay",
        "absolute",
        // Display
        "text",
        "image",
        "icon",
        "video",
        "lottie",
        "markdown",
        "divider",
        "badge",
        "avatar",
        "userProfile",
        "progress",
        "spinner",
        "skeleton",
        // Interactive
        "button",
        "feedback",
        "appLink",
        "textField",
        "select",
        "slider",
        "checkbox",
        "switch",
        "radioGroup",
        "dateTimeInput",
        "fileInput",
        "imageInput",
        "link",
        // Container
        "card",
        "modal",
        "tabs",
        "accordion",
        "drawer",
        "tooltip",
        "popover",
        // Game
        "canvas2d",
        "sprite",
        "shape",
        "scene3d",
        "model3d",
        "dialogue",
        "characterPortrait",
        "choiceMenu",
        "inventoryGrid",
        "healthBar",
        "miniMap",
        "geoMap",
        // Embeds
        "iframe",
        "plotlyChart",
    ]
}
