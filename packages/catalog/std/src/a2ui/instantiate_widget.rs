use flow_like::app::App;
use flow_like::a2ui::widget::{CustomizationType, ExposedPropType, Widget};
use flow_like::flow::{
    board::Board,
    execution::context::ExecutionContext,
    node::{Node, NodeLogic, remove_pin},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_types::{Value, async_trait, json::json};
use std::collections::BTreeSet;
use std::sync::Arc;

#[crate::register_node]
#[derive(Default)]
pub struct InstantiateWidget;

impl InstantiateWidget {
    pub fn new() -> Self {
        Self
    }
}

const DYNAMIC_PIN_PREFIX: &str = "dyn_";

fn exposed_prop_type_to_variable_type(prop_type: &ExposedPropType) -> VariableType {
    match prop_type {
        ExposedPropType::String
        | ExposedPropType::Color
        | ExposedPropType::ImageUrl
        | ExposedPropType::Icon
        | ExposedPropType::TailwindClass => VariableType::String,
        ExposedPropType::Number => VariableType::Float,
        ExposedPropType::Boolean => VariableType::Boolean,
        ExposedPropType::Enum { .. } => VariableType::String,
        ExposedPropType::Json
        | ExposedPropType::StyleObject
        | ExposedPropType::BoundValue => VariableType::Generic,
    }
}

fn customization_type_to_variable_type(ct: &CustomizationType) -> VariableType {
    match ct {
        CustomizationType::String
        | CustomizationType::Color
        | CustomizationType::ImageUrl
        | CustomizationType::Icon => VariableType::String,
        CustomizationType::Number => VariableType::Float,
        CustomizationType::Boolean => VariableType::Boolean,
        CustomizationType::Enum => VariableType::String,
        CustomizationType::Json => VariableType::Generic,
    }
}

fn infer_variable_type(value: &Value) -> VariableType {
    match value {
        Value::String(_) => VariableType::String,
        Value::Number(_) => VariableType::Float,
        Value::Bool(_) => VariableType::Boolean,
        _ => VariableType::Generic,
    }
}

async fn load_app_widgets(board: &Board) -> Vec<Widget> {
    let app_state = match &board.app_state {
        Some(s) => s.clone(),
        None => return Vec::new(),
    };
    let app_id = match board.board_dir.filename() {
        Some(id) if !id.is_empty() => id.to_string(),
        _ => return Vec::new(),
    };
    let app = match App::load(app_id, app_state).await {
        Ok(a) => a,
        Err(_) => return Vec::new(),
    };
    app.get_widgets().await.unwrap_or_default()
}

/// Extract unique data paths referenced by component BoundValues (e.g. {"path": "/inputs/title"})
fn collect_bound_paths(widget: &Widget) -> BTreeSet<String> {
    let mut paths = BTreeSet::new();
    for comp in &widget.components {
        visit_value_for_paths(&comp.component, &mut paths);
    }
    paths
}

fn visit_value_for_paths(value: &Value, paths: &mut BTreeSet<String>) {
    match value {
        Value::Object(map) => {
            if let Some(Value::String(path)) = map.get("path") {
                if !path.is_empty() {
                    paths.insert(path.clone());
                }
                return;
            }
            for v in map.values() {
                visit_value_for_paths(v, paths);
            }
        }
        Value::Array(arr) => {
            for v in arr {
                visit_value_for_paths(v, paths);
            }
        }
        _ => {}
    }
}

/// Derive a short user-friendly label from a data path like "/inputs/title" -> "title"
fn label_from_path(path: &str) -> String {
    path.rsplit('/').next().unwrap_or(path).to_string()
}

fn add_dynamic_pins_for_widget(node: &mut Node, widget: &Widget) {
    // Collect all bound paths referenced in components
    let bound_paths = collect_bound_paths(widget);

    // Create a pin for each unique bound path
    for path in &bound_paths {
        let safe_key = path.replace('/', "_");
        let pin_name = format!("{DYNAMIC_PIN_PREFIX}path_{safe_key}");
        let label = label_from_path(path);

        // Check if the data model has a default for this path
        let data_entry = widget.data_model.iter().find(|e| &e.key == path || format!("/{}", e.key) == *path);
        let var_type = data_entry.map(|e| infer_variable_type(&e.value)).unwrap_or(VariableType::String);

        let pin = node.add_input_pin(&pin_name, &label, &format!("Bound: {path}"), var_type);
        if let Some(entry) = data_entry {
            if let Some(d) = flow_like_types::json::to_vec(&entry.value).ok() {
                pin.default_value = Some(d);
            }
        }
    }

    // Exposed props -> typed pins
    for prop in &widget.exposed_props {
        let pin_name = format!("{DYNAMIC_PIN_PREFIX}prop_{}", prop.id);
        let var_type = exposed_prop_type_to_variable_type(&prop.prop_type);
        let pin = node.add_input_pin(
            &pin_name,
            &prop.label,
            prop.description.as_deref().unwrap_or(&prop.label),
            var_type,
        );
        if let ExposedPropType::Enum { choices } = &prop.prop_type {
            pin.set_options(
                PinOptions::new()
                    .set_valid_values(choices.clone())
                    .build(),
            );
        }
        if let Some(default) = &prop.default_value {
            pin.default_value = Some(default.clone());
        }
    }

    // Customization options -> typed pins
    for opt in &widget.customization_options {
        let pin_name = format!("{DYNAMIC_PIN_PREFIX}cust_{}", opt.id);
        let var_type = customization_type_to_variable_type(&opt.customization_type);
        let pin = node.add_input_pin(
            &pin_name,
            &opt.label,
            opt.description.as_deref().unwrap_or(&opt.label),
            var_type,
        );
        if let Some(default) = &opt.default_value {
            pin.default_value = Some(default.clone());
        }
    }
}

fn remove_dynamic_pins(node: &mut Node) {
    let dynamic_pins: Vec<_> = node
        .pins
        .values()
        .filter(|p| p.name.starts_with(DYNAMIC_PIN_PREFIX))
        .cloned()
        .collect();
    for pin in dynamic_pins {
        remove_pin(node, Some(pin));
    }
}

#[async_trait]
impl NodeLogic for InstantiateWidget {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "a2ui_instantiate_widget",
            "Instantiate Widget",
            "Creates a new widget instance for dynamic insertion into containers. Select a widget from the dropdown to auto-generate input pins for its exposed props and customizations.",
            "UI/Container",
        );
        node.add_icon("/flow/icons/a2ui.svg");

        node.add_input_pin("exec_in", "▶", "Execution input", VariableType::Execution);

        node.add_input_pin(
            "widget_selector",
            "Widget",
            "Select a widget from the project",
            VariableType::String,
        );

        node.add_input_pin(
            "instance_id",
            "Instance ID",
            "Unique ID for this widget instance",
            VariableType::String,
        );

        node.add_output_pin("exec_out", "▶", "Execution output", VariableType::Execution);

        node.add_output_pin(
            "element_ref",
            "Element Ref",
            "Element reference for the instantiated widget (connect to Push To Container)",
            VariableType::Struct,
        );

        node.set_can_reference_fns(true);
        node.set_long_running(true);
        node.set_version(3);

        node
    }

    async fn on_update(&self, node: &mut Node, board: Arc<Board>) {
        node.error = None;
        let widgets = load_app_widgets(&board).await;

        let widget_names: Vec<String> = widgets.iter().map(|w| w.name.clone()).collect();

        if let Some(selector_pin) = node.get_pin_mut_by_name("widget_selector") {
            selector_pin.set_options(
                PinOptions::new()
                    .set_valid_values(widget_names)
                    .build(),
            );
        }

        let selected = node
            .get_pin_by_name("widget_selector")
            .and_then(|pin| pin.default_value.clone())
            .and_then(|bytes| flow_like_types::json::from_slice::<String>(&bytes).ok());

        remove_dynamic_pins(node);

        if let Some(ref name) = selected {
            if let Some(widget) = widgets.iter().find(|w| &w.name == name) {
                node.friendly_name = format!("Instantiate {}", widget.name);
                add_dynamic_pins_for_widget(node, widget);
            }
        }

        // Validate fn_ref connections (action event bindings)
        if let Some(fn_refs) = &node.fn_refs {
            let mut seen_actions = BTreeSet::new();
            let mut duplicates = Vec::new();

            for ref_id in &fn_refs.fn_refs {
                let ref_node = match board.nodes.get(ref_id) {
                    Some(n) => n,
                    None => continue,
                };

                if ref_node.name != "events_widget_action" {
                    node.error = Some(format!(
                        "Referenced node '{}' is not a Widget Action Event",
                        ref_node.friendly_name
                    ));
                    return;
                }

                let action_id = ref_node
                    .get_pin_by_name("action_id")
                    .and_then(|p| p.default_value.as_ref())
                    .and_then(|v| flow_like_types::json::from_slice::<String>(v).ok())
                    .unwrap_or_default();

                if action_id.is_empty() {
                    node.error = Some(format!(
                        "Widget Action Event '{}' has no Action ID set",
                        ref_node.friendly_name
                    ));
                    return;
                }

                if !seen_actions.insert(action_id.clone()) && !duplicates.contains(&action_id) {
                    duplicates.push(action_id);
                }
            }

            if !duplicates.is_empty() {
                node.error = Some(format!(
                    "Duplicate action handlers: [{}]. Each action must have exactly one event.",
                    duplicates.join(", ")
                ));
            }
        }
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let widget_name: String = context.evaluate_pin("widget_selector").await?;
        let instance_id: String = context.evaluate_pin("instance_id").await?;

        let app_id = context
            .execution_cache
            .as_ref()
            .map(|c| c.app_id.clone())
            .ok_or_else(|| flow_like_types::anyhow!("Execution cache not available"))?;

        let app = App::load(app_id.clone(), context.app_state.clone()).await?;
        let widgets = app.get_widgets().await.unwrap_or_default();
        let widget = widgets
            .iter()
            .find(|w| w.name == widget_name)
            .ok_or_else(|| flow_like_types::anyhow!("Widget '{}' not found", widget_name))?;
        let widget_id = widget.id.clone();

        // Collect bound path values from component path bindings
        let bound_paths = collect_bound_paths(widget);
        let mut data_values = flow_like_types::json::Map::new();
        for path in &bound_paths {
            let safe_key = path.replace('/', "_");
            let pin_name = format!("{DYNAMIC_PIN_PREFIX}path_{safe_key}");
            if let Ok(val) = context.evaluate_pin::<Value>(&pin_name).await {
                if !val.is_null() {
                    data_values.insert(path.clone(), val);
                }
            }
        }

        // Collect exposed prop values
        let mut exposed_prop_values = flow_like_types::json::Map::new();
        for prop in &widget.exposed_props {
            let pin_name = format!("{DYNAMIC_PIN_PREFIX}prop_{}", prop.id);
            if let Ok(val) = context.evaluate_pin::<Value>(&pin_name).await {
                if !val.is_null() {
                    exposed_prop_values.insert(prop.id.clone(), val);
                }
            }
        }

        // Collect customization values
        let mut customization_values = flow_like_types::json::Map::new();
        for opt in &widget.customization_options {
            let pin_name = format!("{DYNAMIC_PIN_PREFIX}cust_{}", opt.id);
            if let Ok(val) = context.evaluate_pin::<Value>(&pin_name).await {
                if !val.is_null() {
                    customization_values.insert(opt.id.clone(), val);
                }
            }
        }

        // Collect action bindings from referenced event functions
        let mut action_bindings = flow_like_types::json::Map::new();
        if let Ok(referenced_fns) = context.get_referenced_functions().await {
            for referenced_node in &referenced_fns {
                let node_guard = referenced_node.node.lock().await;
                let node_id = node_guard.id.clone();
                // Read action_id from the referenced event node's input pin
                let action_id = node_guard
                    .get_pin_by_name("action_id")
                    .and_then(|p| p.default_value.as_ref())
                    .and_then(|v| flow_like_types::json::from_slice::<String>(v).ok())
                    .unwrap_or_default();
                drop(node_guard);
                if !action_id.is_empty() {
                    action_bindings.insert(action_id, json!(node_id));
                }
            }
        }

        let widget_instance = json!({
            "id": instance_id,
            "widgetId": widget_id,
            "instanceId": instance_id,
            "dataValues": data_values,
            "customizationValues": customization_values,
            "exposedPropValues": exposed_prop_values,
            "actionBindings": action_bindings,
            "widgetRef": {
                "appId": app_id,
                "widgetId": widget_id
            }
        });

        // Register the widget instance in the frontend via upsert_element
        context
            .upsert_element(&instance_id, widget_instance.clone())
            .await?;

        // Output the full element struct for downstream nodes
        context
            .get_pin_by_name("element_ref")
            .await?
            .set_value(widget_instance)
            .await;

        context.activate_exec_pin("exec_out").await?;

        Ok(())
    }
}
