use flow_like::a2ui::widget::{CustomizationType, ExposedPropType, Widget};
use flow_like::app::App;
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
        ExposedPropType::Json | ExposedPropType::StyleObject | ExposedPropType::BoundValue => {
            VariableType::Generic
        }
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

/// Collect the expected dynamic pin names for a widget.
fn expected_dynamic_pin_names(widget: &Widget) -> BTreeSet<String> {
    let mut names = BTreeSet::new();
    for path in collect_bound_paths(widget) {
        let safe_key = path.replace('/', "_");
        names.insert(format!("{DYNAMIC_PIN_PREFIX}path_{safe_key}"));
    }
    for prop in &widget.exposed_props {
        names.insert(format!("{DYNAMIC_PIN_PREFIX}prop_{}", prop.id));
    }
    for opt in &widget.customization_options {
        names.insert(format!("{DYNAMIC_PIN_PREFIX}cust_{}", opt.id));
    }
    names
}

fn add_dynamic_pins_for_widget(node: &mut Node, widget: &Widget) {
    let bound_paths = collect_bound_paths(widget);

    for path in &bound_paths {
        let safe_key = path.replace('/', "_");
        let pin_name = format!("{DYNAMIC_PIN_PREFIX}path_{safe_key}");

        // Skip if pin already exists (preserves user-set default value)
        if node.get_pin_by_name(&pin_name).is_some() {
            continue;
        }

        let label = label_from_path(path);
        let data_entry = widget
            .data_model
            .iter()
            .find(|e| &e.key == path || format!("/{}", e.key) == *path);
        let var_type = data_entry
            .map(|e| infer_variable_type(&e.value))
            .unwrap_or(VariableType::String);

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
        if node.get_pin_by_name(&pin_name).is_some() {
            continue;
        }
        let var_type = exposed_prop_type_to_variable_type(&prop.prop_type);
        let pin = node.add_input_pin(
            &pin_name,
            &prop.label,
            prop.description.as_deref().unwrap_or(&prop.label),
            var_type,
        );
        if let ExposedPropType::Enum { choices } = &prop.prop_type {
            pin.set_options(PinOptions::new().set_valid_values(choices.clone()).build());
        }
        if let Some(default) = &prop.default_value {
            pin.default_value = Some(default.clone());
        }
    }

    // Customization options -> typed pins
    for opt in &widget.customization_options {
        let pin_name = format!("{DYNAMIC_PIN_PREFIX}cust_{}", opt.id);
        if node.get_pin_by_name(&pin_name).is_some() {
            continue;
        }
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

fn remove_stale_dynamic_pins(node: &mut Node, keep: &BTreeSet<String>) {
    let stale: Vec<_> = node
        .pins
        .values()
        .filter(|p| p.name.starts_with(DYNAMIC_PIN_PREFIX) && !keep.contains(&p.name))
        .cloned()
        .collect();
    for pin in stale {
        remove_pin(node, Some(pin));
    }
}

/// Replace BoundValue path references with literal values from data_values
fn apply_data_bindings(value: &mut Value, data_values: &flow_like_types::json::Map<String, Value>) {
    if let Value::Object(map) = value {
        let replacement = map
            .get("path")
            .and_then(|v| v.as_str())
            .and_then(|path| data_values.get(path))
            .map(value_to_bound_value);
        if let Some(replacement) = replacement {
            *value = replacement;
            return;
        }
        for v in map.values_mut() {
            apply_data_bindings(v, data_values);
        }
    } else if let Value::Array(arr) = value {
        for v in arr.iter_mut() {
            apply_data_bindings(v, data_values);
        }
    }
}

fn value_to_bound_value(value: &Value) -> Value {
    match value {
        Value::String(s) => json!({"literalString": s}),
        Value::Number(n) => json!({"literalNumber": n}),
        Value::Bool(b) => json!({"literalBool": b}),
        other => json!({"literalJson": other.to_string()}),
    }
}

fn set_nested_property(value: &mut Value, path: &str, new_val: Value) {
    let parts: Vec<&str> = path.split('.').collect();
    let mut current = value;
    for (i, part) in parts.iter().enumerate() {
        if i == parts.len() - 1 {
            if let Value::Object(map) = current {
                map.insert((*part).to_string(), new_val);
            }
            return;
        }
        current = match current {
            Value::Object(map) => map
                .entry((*part).to_string())
                .or_insert(Value::Object(Default::default())),
            _ => return,
        };
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
            selector_pin.set_options(PinOptions::new().set_valid_values(widget_names).build());
        }

        let selected = node
            .get_pin_by_name("widget_selector")
            .and_then(|pin| pin.default_value.clone())
            .and_then(|bytes| flow_like_types::json::from_slice::<String>(&bytes).ok());

        if let Some(ref name) = selected {
            if let Some(widget) = widgets.iter().find(|w| &w.name == name) {
                let expected = expected_dynamic_pin_names(widget);
                remove_stale_dynamic_pins(node, &expected);
                node.friendly_name = format!("Instantiate {}", widget.name);
                add_dynamic_pins_for_widget(node, widget);
            } else {
                remove_stale_dynamic_pins(node, &BTreeSet::new());
            }
        } else {
            remove_stale_dynamic_pins(node, &BTreeSet::new());
        }

        // Validate fn_ref connections (action event bindings)
        if let Some(fn_refs) = &node.fn_refs {
            // Collect valid action IDs from the selected widget
            let valid_actions: BTreeSet<String> = selected
                .as_ref()
                .and_then(|name| widgets.iter().find(|w| &w.name == name))
                .map(|w| w.actions.iter().map(|a| a.id.clone()).collect())
                .unwrap_or_default();

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

                // Empty action_id = catch-all handler, skip validation
                if action_id.is_empty() {
                    continue;
                }

                if !valid_actions.is_empty() && !valid_actions.contains(&action_id) {
                    node.error = Some(format!(
                        "Action '{}' is not defined on the selected widget. Available: [{}]",
                        action_id,
                        valid_actions.iter().cloned().collect::<Vec<_>>().join(", ")
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
            let mut catch_all_nodes = Vec::new();
            for referenced_node in &referenced_fns {
                let node_guard = referenced_node.node.lock().await;
                let node_id = node_guard.id.clone();
                let action_id = node_guard
                    .get_pin_by_name("action_id")
                    .and_then(|p| p.default_value.as_ref())
                    .and_then(|v| flow_like_types::json::from_slice::<String>(v).ok())
                    .unwrap_or_default();
                drop(node_guard);
                if !action_id.is_empty() {
                    action_bindings.insert(action_id, json!(node_id));
                } else {
                    catch_all_nodes.push(node_id);
                }
            }
            // Catch-all: nodes without action_id bind to all unbound widget actions
            for node_id in catch_all_nodes {
                for action in &widget.actions {
                    if !action_bindings.contains_key(&action.id) {
                        action_bindings.insert(action.id.clone(), json!(&node_id));
                    }
                }
            }
        }

        // The widget's root_component_id and target_component_id fields may include
        // the widget ID as a prefix (e.g. "{widget_id}-root") while actual component IDs
        // in the components array don't have this prefix. Build a helper to strip it.
        let component_ids: Vec<&str> = widget.components.iter().map(|c| c.id.as_str()).collect();
        let strip_widget_prefix = |id: &str| -> String {
            if component_ids.contains(&id) {
                return id.to_string();
            }
            let prefix = format!("{}-", widget_id);
            if let Some(stripped) = id.strip_prefix(&prefix) {
                if component_ids.contains(&stripped) {
                    return stripped.to_string();
                }
            }
            id.to_string()
        };
        let effective_root_id = strip_widget_prefix(&widget.root_component_id);

        // Build inline widget definition with data bindings applied
        let mut inline_components = Vec::new();
        for comp in &widget.components {
            let mut component_data = comp.component.clone();

            if !data_values.is_empty() {
                apply_data_bindings(&mut component_data, &data_values);
            }

            let style_value = comp
                .style
                .as_ref()
                .and_then(|s| flow_like_types::json::to_value(s).ok());

            inline_components.push(json!({
                "id": comp.id,
                "component": component_data,
                "style": style_value
            }));
        }

        // Apply exposed prop values to target components
        for prop in &widget.exposed_props {
            if let Some(val) = exposed_prop_values.get(&prop.id) {
                let target_id = strip_widget_prefix(&prop.target_component_id);
                if let Some(comp) = inline_components
                    .iter_mut()
                    .find(|c| c.get("id").and_then(|id| id.as_str()) == Some(target_id.as_str()))
                {
                    if let Some(comp_data) = comp.get_mut("component") {
                        set_nested_property(
                            comp_data,
                            &prop.property_path,
                            value_to_bound_value(val),
                        );
                    }
                }
            }
        }

        // Apply customization values to the root component
        for opt in &widget.customization_options {
            if let Some(val) = customization_values.get(&opt.id) {
                if let Some(comp) = inline_components.iter_mut().find(|c| {
                    c.get("id").and_then(|id| id.as_str()) == Some(effective_root_id.as_str())
                }) {
                    if let Some(comp_data) = comp.get_mut("component") {
                        set_nested_property(comp_data, &opt.id, val.clone());
                    }
                }
            }
        }

        // Create a single widgetInstance component with inline definition
        let widget_instance_component = json!({
            "type": "widgetInstance",
            "instanceId": instance_id,
            "widgetId": widget_id,
            "inlineWidgetDef": {
                "name": widget.name,
                "rootComponentId": effective_root_id,
                "components": inline_components
            },
            "actionBindings": action_bindings,
            "exposedPropValues": exposed_prop_values,
            "customizationValues": customization_values,
        });

        // Register the widgetInstance in the frontend surface via upsert_element
        context
            .upsert_element(
                &instance_id,
                json!({
                    "type": "createComponent",
                    "component": widget_instance_component
                }),
            )
            .await?;

        // Output element ref for PushToContainer
        let element_ref = json!({
            "id": instance_id,
            "instanceId": instance_id,
            "widgetId": widget_id,
        });

        context
            .get_pin_by_name("element_ref")
            .await?
            .set_value(element_ref)
            .await;

        context.activate_exec_pin("exec_out").await?;

        Ok(())
    }
}
