use super::micro_widget_utils::{
    DYN_INPUT_PREFIX, add_contract_input_pins, clear_widget_ref_metadata,
    expected_contract_input_pin_names, set_widget_ref_metadata,
};
use flow_like::a2ui::micro_widget::{
    self, MICRO_WIDGET_COMPONENT_TYPE, ResolvedWidget, WidgetProvider,
};
use flow_like::a2ui::widget::{CustomizationType, ExposedPropType, Widget};
use flow_like::app::App;
use flow_like::flow::{
    board::Board,
    execution::context::ExecutionContext,
    node::{Node, NodeLogic, NodeScores, pin_is_wired, remove_unwired_pins},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_types::{Value, async_trait, json::json};
use std::collections::{BTreeMap, BTreeSet};

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

fn find_widget_by_selector<'a>(widgets: &'a [Widget], selector: &str) -> Option<&'a Widget> {
    widgets
        .iter()
        .find(|w| w.id == selector)
        .or_else(|| widgets.iter().find(|w| w.name == selector))
}

/// Type evidence gathered for one bound data path.
///
/// A builder-authored widget keeps its `data_model` empty, so the literal the binding was
/// converted from — `{"path": "/inputs/hidden", "defaultValue": false}` — is the only record of
/// what type the property had before it became a binding. Two components binding the same path
/// with differently typed defaults mark the path `conflicting`, which resolves to `Generic`
/// instead of silently picking one.
#[derive(Default)]
struct BoundPath {
    default: Option<Value>,
    conflicting: bool,
}

impl BoundPath {
    fn observe(&mut self, default: Option<Value>) {
        let Some(default) = default else { return };
        match &self.default {
            None => self.default = Some(default),
            Some(existing) => {
                if std::mem::discriminant(existing) != std::mem::discriminant(&default) {
                    self.conflicting = true;
                }
            }
        }
    }

    fn variable_type(&self) -> Option<VariableType> {
        if self.conflicting {
            return Some(VariableType::Generic);
        }
        self.default.as_ref().map(infer_variable_type)
    }
}

/// Extract the data paths referenced by component BoundValues (e.g. {"path": "/inputs/title"})
/// together with the literal default each binding carries.
fn collect_bound_paths(widget: &Widget) -> BTreeMap<String, BoundPath> {
    let mut paths = BTreeMap::new();
    for comp in &widget.components {
        visit_value_for_paths(&comp.component, &mut paths);
    }
    paths
}

fn visit_value_for_paths(value: &Value, paths: &mut BTreeMap<String, BoundPath>) {
    match value {
        Value::Object(map) => {
            if let Some(Value::String(path)) = map.get("path") {
                if !path.is_empty() {
                    let default = map.get("defaultValue").filter(|v| !v.is_null()).cloned();
                    paths.entry(path.clone()).or_default().observe(default);
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
    for path in collect_bound_paths(widget).keys() {
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
    let mut stale_types: Vec<String> = Vec::new();

    for (path, bound) in &bound_paths {
        let safe_key = path.replace('/', "_");
        let pin_name = format!("{DYNAMIC_PIN_PREFIX}path_{safe_key}");

        let data_entry = widget
            .data_model
            .iter()
            .find(|e| &e.key == path || format!("/{}", e.key) == *path);
        let seed = data_entry
            .map(|e| &e.value)
            .or(bound.default.as_ref())
            .cloned();
        let var_type = data_entry
            .map(|e| infer_variable_type(&e.value))
            .or_else(|| bound.variable_type())
            .unwrap_or(VariableType::String);

        // An existing pin keeps its user-set literal, but a pin minted before the widget
        // declared a type — or before its binding changed type — is re-typed so the node heals
        // on reload instead of demanding a rebuild. A wired pin is left alone: re-typing it
        // leaves an edge the editor would refuse to draw, with nothing said on the producer.
        // It gets named on `node.error` instead, since staying silently mistyped is what makes
        // a bound boolean look like a text pin forever.
        if let Some(existing) = node.get_pin_mut_by_name(&pin_name) {
            if existing.data_type != var_type {
                if pin_is_wired(existing) {
                    stale_types.push(format!("{} (should be {:?})", existing.name, var_type));
                } else {
                    existing.data_type = var_type;
                    existing.default_value =
                        seed.and_then(|v| flow_like_types::json::to_vec(&v).ok());
                }
            }
            continue;
        }

        let label = label_from_path(path);
        let pin = node.add_input_pin(&pin_name, &label, &format!("Bound: {path}"), var_type);
        if let Some(d) = seed.and_then(|v| flow_like_types::json::to_vec(&v).ok()) {
            pin.default_value = Some(d);
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

    if !stale_types.is_empty() {
        stale_types.sort();
        let message = format!(
            "Connected bound inputs still carry their old type: {}. Disconnect them to pick up the widget's type.",
            stale_types.join(", ")
        );
        node.error = Some(match node.error.take() {
            Some(existing) if !existing.is_empty() => format!("{existing} {message}"),
            _ => message,
        });
    }
}

/// Drop the `dyn_*` pins the selected widget no longer declares. A pin that still carries a wire
/// is kept and reported on `node.error`: an unset or unresolvable selector is not evidence that
/// the binding was wrong, and removing the pin silently deletes the edge on both ends.
fn remove_stale_dynamic_pins(node: &mut Node, keep: &BTreeSet<String>) {
    let stale: Vec<String> = node
        .pins
        .values()
        .filter(|p| p.name.starts_with(DYNAMIC_PIN_PREFIX) && !keep.contains(&p.name))
        .map(|p| p.id.clone())
        .collect();
    remove_unwired_pins(node, &stale);
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

/// Build the frozen `microWidgetInstance` component for a package widget
/// instance. The contract JSON is embedded verbatim from the manifest.
fn build_micro_widget_component(
    entry: &micro_widget::PackageWidgetRef,
    instance_id: &str,
    props: flow_like_types::json::Map<String, Value>,
    action_bindings: flow_like_types::json::Map<String, Value>,
) -> Value {
    json!({
        "type": MICRO_WIDGET_COMPONENT_TYPE,
        "instanceId": instance_id,
        "packageId": entry.package_id,
        "widgetId": entry.widget_id,
        "packageVersion": entry.package_version,
        "bundleHash": entry.bundle_hash,
        "contract": entry.contract,
        "props": props,
        "actionBindings": action_bindings,
        "preview": false,
    })
}

async fn instantiate_package_widget(
    context: &mut ExecutionContext,
    widget_selector: &str,
    instance_id: &str,
    app_id: &str,
) -> flow_like_types::Result<()> {
    let (package_id, widget_id) = micro_widget::decode_package_widget_ref(widget_selector)
        .ok_or_else(|| {
            flow_like_types::anyhow!("Invalid package widget reference '{}'", widget_selector)
        })?;

    let provider = WidgetProvider::load(app_id, context.app_state.clone()).await?;
    let entry = provider
        .resolve_package(package_id, widget_id)
        .ok_or_else(|| {
            flow_like_types::anyhow!(
                "Package widget '{}' not found — is package '{}' added to the app?",
                widget_selector,
                package_id
            )
        })?;
    let contract = entry.parsed_contract()?;

    // One props entry per contract input from the dyn_in_* pins; unset
    // optional inputs are omitted.
    let mut props = flow_like_types::json::Map::new();
    for key in contract.inputs.keys() {
        let pin_name = format!("{DYN_INPUT_PREFIX}{key}");
        if let Ok(val) = context.evaluate_pin::<Value>(&pin_name).await
            && !val.is_null()
        {
            props.insert(key.clone(), val);
        }
    }

    // Preserve the existing Instantiate Widget fn-ref contract for package
    // widgets too. Each referenced Widget Action Event binds its action_id to
    // the workflow node; an empty action_id remains a catch-all for currently
    // unbound events declared by the package contract.
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

            if action_id.is_empty() {
                catch_all_nodes.push(node_id);
            } else {
                action_bindings.insert(
                    action_id,
                    json!({ "workflow": { "flowId": node_id, "inputMappings": {} } }),
                );
            }
        }

        for node_id in catch_all_nodes {
            for event_id in contract.events.keys() {
                if !action_bindings.contains_key(event_id) {
                    action_bindings.insert(
                        event_id.clone(),
                        json!({ "workflow": { "flowId": &node_id, "inputMappings": {} } }),
                    );
                }
            }
        }
    }

    let component = build_micro_widget_component(entry, instance_id, props, action_bindings);

    context
        .upsert_element(
            instance_id,
            json!({
                "type": "createComponent",
                "component": component.clone()
            }),
        )
        .await?;

    let element_ref = json!({
        "id": instance_id,
        "instanceId": instance_id,
        "widgetId": entry.widget_id,
        "surfaceId": instance_id,
        "component": component,
    });

    context
        .get_pin_by_name("element_ref")
        .await?
        .set_value(element_ref)
        .await;

    context.activate_exec_pin("exec_out").await?;

    Ok(())
}

#[async_trait]
impl NodeLogic for InstantiateWidget {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "a2ui_instantiate_widget",
            "Instantiate Widget",
            "Creates a new widget instance for dynamic insertion into containers. The dropdown lists project widgets and widgets from packages added to the project; selecting one auto-generates typed input pins (exposed props and customizations for project widgets, contract inputs for package widgets).",
            "UI/Container",
        );
        node.set_flowscript_name("ui", "instantiateWidget");
        node.add_icon("/flow/icons/a2ui.svg");
        node.set_scores(
            NodeScores::new()
                .set_privacy(8)
                .set_security(8)
                .set_performance(8)
                .set_governance(7)
                .set_reliability(8)
                .set_cost(9)
                .build(),
        );

        node.add_input_pin("exec_in", "▶", "Execution input", VariableType::Execution);

        node.add_input_pin(
            "widget_selector",
            "Widget",
            "Select a widget from the project or from packages added to the project",
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
        )
        .set_schema::<flow_like::a2ui::ElementRef>();

        node.set_can_reference_fns(true);
        node.set_long_running(true);
        node.set_version(6);

        node
    }

    async fn on_update(&self, node: &mut Node, board: &Board) {
        node.error = None;
        let provider = WidgetProvider::from_board(board).await;

        if let Some(selector_pin) = node.get_pin_mut_by_name("widget_selector") {
            selector_pin.set_options(
                PinOptions::new()
                    .set_valid_values(provider.selector_values())
                    .build(),
            );
        }

        let selected = node
            .get_pin_by_name("widget_selector")
            .and_then(|pin| pin.default_value.clone())
            .and_then(|bytes| flow_like_types::json::from_slice::<String>(&bytes).ok());

        match selected.as_ref().and_then(|s| provider.resolve(s)) {
            Some(ResolvedWidget::Declarative(widget)) => {
                let expected = expected_dynamic_pin_names(widget);
                remove_stale_dynamic_pins(node, &expected);
                node.friendly_name = format!("Instantiate {}", widget.name);
                add_dynamic_pins_for_widget(node, widget);
                if let Some(element_ref) = node.get_pin_mut_by_name("element_ref") {
                    clear_widget_ref_metadata(element_ref);
                }
            }
            Some(ResolvedWidget::Package(entry)) => match entry.parsed_contract() {
                Ok(contract) => {
                    let expected = expected_contract_input_pin_names(&contract);
                    remove_stale_dynamic_pins(node, &expected);
                    node.friendly_name = format!("Instantiate {}", entry.name);
                    add_contract_input_pins(node, &contract, true);
                    if let Some(element_ref) = node.get_pin_mut_by_name("element_ref") {
                        set_widget_ref_metadata(
                            element_ref,
                            &entry.selector(),
                            &entry.name,
                            &entry.contract,
                        );
                    }
                }
                Err(e) => {
                    remove_stale_dynamic_pins(node, &BTreeSet::new());
                    if let Some(element_ref) = node.get_pin_mut_by_name("element_ref") {
                        clear_widget_ref_metadata(element_ref);
                    }
                    node.error = Some(e.to_string());
                }
            },
            None => {
                if selected.is_none() {
                    remove_stale_dynamic_pins(node, &BTreeSet::new());
                    if let Some(element_ref) = node.get_pin_mut_by_name("element_ref") {
                        clear_widget_ref_metadata(element_ref);
                    }
                } else {
                    // A registry source can be briefly unavailable while a host
                    // starts. Preserve the last good contract-shaped pins and
                    // metadata instead of destructively collapsing the node.
                    node.error = Some(
                        "The selected widget contract is temporarily unavailable; refresh after the package registry is ready."
                            .to_string(),
                    );
                }
            }
        }

        // Validate fn_ref connections (action / contract-event bindings)
        if let Some(fn_refs) = &node.fn_refs {
            // Collect valid action IDs from the selected widget: declarative
            // widget actions or package widget contract event names.
            let valid_actions: BTreeSet<String> = selected
                .as_ref()
                .and_then(|selector| provider.resolve(selector))
                .map(|resolved| match resolved {
                    ResolvedWidget::Declarative(w) => {
                        w.actions.iter().map(|a| a.id.clone()).collect()
                    }
                    ResolvedWidget::Package(entry) => entry
                        .parsed_contract()
                        .map(|c| c.events.keys().cloned().collect())
                        .unwrap_or_default(),
                })
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

        let widget_selector: String = context.evaluate_pin("widget_selector").await?;
        let instance_id: String = context.evaluate_pin("instance_id").await?;

        let app_id = context
            .execution_cache
            .as_ref()
            .map(|c| c.app_id.clone())
            .ok_or_else(|| flow_like_types::anyhow!("Execution cache not available"))?;

        if micro_widget::decode_package_widget_ref(&widget_selector).is_some() {
            return instantiate_package_widget(context, &widget_selector, &instance_id, &app_id)
                .await;
        }

        let app = App::load(app_id.clone(), context.app_state.clone()).await?;
        let widgets = app.get_widgets().await.unwrap_or_default();
        let widget = find_widget_by_selector(&widgets, &widget_selector)
            .ok_or_else(|| flow_like_types::anyhow!("Widget '{}' not found", widget_selector))?;
        let widget_id = widget.id.clone();

        // Collect bound path values from component path bindings
        let bound_paths = collect_bound_paths(widget);
        let mut data_values = flow_like_types::json::Map::new();
        for path in bound_paths.keys() {
            let safe_key = path.replace('/', "_");
            let pin_name = format!("{DYNAMIC_PIN_PREFIX}path_{safe_key}");
            if let Ok(val) = context.evaluate_pin::<Value>(&pin_name).await
                && !val.is_null()
            {
                data_values.insert(path.clone(), val);
            }
        }

        // Collect exposed prop values
        let mut exposed_prop_values = flow_like_types::json::Map::new();
        for prop in &widget.exposed_props {
            let pin_name = format!("{DYNAMIC_PIN_PREFIX}prop_{}", prop.id);
            if let Ok(val) = context.evaluate_pin::<Value>(&pin_name).await
                && !val.is_null()
            {
                exposed_prop_values.insert(prop.id.clone(), val);
            }
        }

        // Collect customization values
        let mut customization_values = flow_like_types::json::Map::new();
        for opt in &widget.customization_options {
            let pin_name = format!("{DYNAMIC_PIN_PREFIX}cust_{}", opt.id);
            if let Ok(val) = context.evaluate_pin::<Value>(&pin_name).await
                && !val.is_null()
            {
                customization_values.insert(opt.id.clone(), val);
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
                    action_bindings.insert(
                        action_id,
                        json!({ "workflow": { "flowId": node_id, "inputMappings": {} } }),
                    );
                } else {
                    catch_all_nodes.push(node_id);
                }
            }
            // Catch-all: nodes without action_id bind to all unbound widget actions
            for node_id in catch_all_nodes {
                for action in &widget.actions {
                    if !action_bindings.contains_key(&action.id) {
                        action_bindings.insert(
                            action.id.clone(),
                            json!({ "workflow": { "flowId": &node_id, "inputMappings": {} } }),
                        );
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
            if let Some(stripped) = id.strip_prefix(&prefix)
                && component_ids.contains(&stripped)
            {
                return stripped.to_string();
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
                    && let Some(comp_data) = comp.get_mut("component")
                {
                    set_nested_property(comp_data, &prop.property_path, value_to_bound_value(val));
                }
            }
        }

        // Apply customization values to the root component
        for opt in &widget.customization_options {
            if let Some(val) = customization_values.get(&opt.id)
                && let Some(comp) = inline_components.iter_mut().find(|c| {
                    c.get("id").and_then(|id| id.as_str()) == Some(effective_root_id.as_str())
                })
                && let Some(comp_data) = comp.get_mut("component")
            {
                set_nested_property(comp_data, &opt.id, val.clone());
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
                    "component": widget_instance_component.clone()
                }),
            )
            .await?;

        // Output element ref for PushToContainer / Push Widget. The full inline
        // component is embedded so chat push nodes can forward a self-contained
        // widgetInstance without re-reading the surface.
        let element_ref = json!({
            "id": instance_id,
            "instanceId": instance_id,
            "widgetId": widget_id,
            "surfaceId": instance_id,
            "component": widget_instance_component,
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

#[cfg(test)]
mod tests {
    use super::*;
    use flow_like::a2ui::micro_widget::PackageWidgetRef;
    use flow_like::a2ui::{DataEntry, SurfaceComponent};
    use flow_like::flow::pin::PinType;

    fn widget_with_components(components: Vec<(&str, Value)>) -> Widget {
        let mut widget = Widget::new("w1", "Widget", "root");
        widget.components = components
            .into_iter()
            .map(|(id, component)| SurfaceComponent::new(id, component))
            .collect();
        widget
    }

    fn bound_pin_type(node: &Node, path: &str) -> VariableType {
        let safe_key = path.replace('/', "_");
        node.get_pin_by_name(&format!("{DYNAMIC_PIN_PREFIX}path_{safe_key}"))
            .unwrap_or_else(|| panic!("missing pin for {path}"))
            .data_type
            .clone()
    }

    #[test]
    fn bound_path_pin_takes_the_type_of_the_binding_default() {
        let widget = widget_with_components(vec![(
            "text",
            json!({
                "type": "text",
                "hidden": { "path": "/inputs/hidden", "defaultValue": false },
                "content": { "path": "/inputs/title", "defaultValue": "Hello" },
                "size": { "path": "/inputs/size", "defaultValue": 12 },
            }),
        )]);

        let mut node = InstantiateWidget.get_node();
        add_dynamic_pins_for_widget(&mut node, &widget);

        assert_eq!(
            bound_pin_type(&node, "/inputs/hidden"),
            VariableType::Boolean
        );
        assert_eq!(bound_pin_type(&node, "/inputs/title"), VariableType::String);
        assert_eq!(bound_pin_type(&node, "/inputs/size"), VariableType::Float);
    }

    #[test]
    fn bound_path_without_type_evidence_stays_string() {
        let widget = widget_with_components(vec![(
            "text",
            json!({ "type": "text", "content": { "path": "/inputs/title" } }),
        )]);

        let mut node = InstantiateWidget.get_node();
        add_dynamic_pins_for_widget(&mut node, &widget);

        assert_eq!(bound_pin_type(&node, "/inputs/title"), VariableType::String);
    }

    #[test]
    fn data_model_entry_outranks_the_binding_default() {
        let mut widget = widget_with_components(vec![(
            "text",
            json!({
                "type": "text",
                "hidden": { "path": "/inputs/hidden", "defaultValue": "false" },
            }),
        )]);
        widget
            .data_model
            .push(DataEntry::new("/inputs/hidden", json!(true)));

        let mut node = InstantiateWidget.get_node();
        add_dynamic_pins_for_widget(&mut node, &widget);

        assert_eq!(
            bound_pin_type(&node, "/inputs/hidden"),
            VariableType::Boolean
        );
    }

    #[test]
    fn conflicting_binding_defaults_resolve_to_generic() {
        let widget = widget_with_components(vec![
            (
                "a",
                json!({ "type": "text", "hidden": { "path": "/inputs/x", "defaultValue": false } }),
            ),
            (
                "b",
                json!({ "type": "text", "content": { "path": "/inputs/x", "defaultValue": "no" } }),
            ),
        ]);

        let mut node = InstantiateWidget.get_node();
        add_dynamic_pins_for_widget(&mut node, &widget);

        assert_eq!(bound_pin_type(&node, "/inputs/x"), VariableType::Generic);
    }

    #[test]
    fn an_unwired_pin_is_retyped_but_a_wired_one_is_left_alone() {
        let untyped = widget_with_components(vec![(
            "text",
            json!({
                "type": "text",
                "hidden": { "path": "/inputs/hidden" },
                "content": { "path": "/inputs/title" },
            }),
        )]);
        let mut node = InstantiateWidget.get_node();
        add_dynamic_pins_for_widget(&mut node, &untyped);
        assert_eq!(
            bound_pin_type(&node, "/inputs/hidden"),
            VariableType::String
        );

        let wired_id = node
            .get_pin_by_name(&format!("{DYNAMIC_PIN_PREFIX}path__inputs_title"))
            .unwrap()
            .id
            .clone();
        node.pins
            .get_mut(&wired_id)
            .unwrap()
            .depends_on
            .insert("producer-pin".to_string());

        let typed = widget_with_components(vec![(
            "text",
            json!({
                "type": "text",
                "hidden": { "path": "/inputs/hidden", "defaultValue": false },
                "content": { "path": "/inputs/title", "defaultValue": 3 },
            }),
        )]);
        add_dynamic_pins_for_widget(&mut node, &typed);

        assert_eq!(
            bound_pin_type(&node, "/inputs/hidden"),
            VariableType::Boolean
        );
        assert_eq!(bound_pin_type(&node, "/inputs/title"), VariableType::String);

        let error = node.error.expect("wired mistyped pin must be reported");
        assert!(
            error.contains("dyn_path__inputs_title") && error.contains("Float"),
            "unexpected diagnostic: {error}"
        );
        assert!(
            !error.contains("dyn_path__inputs_hidden"),
            "a re-typed pin must not be reported: {error}"
        );
    }

    #[test]
    fn bound_pins_are_inputs_and_seeded_with_the_binding_default() {
        let widget = widget_with_components(vec![(
            "text",
            json!({
                "type": "text",
                "hidden": { "path": "/inputs/hidden", "defaultValue": true },
            }),
        )]);

        let mut node = InstantiateWidget.get_node();
        add_dynamic_pins_for_widget(&mut node, &widget);

        let pin = node
            .get_pin_by_name(&format!("{DYNAMIC_PIN_PREFIX}path__inputs_hidden"))
            .unwrap();
        assert_eq!(pin.pin_type, PinType::Input);
        let default: Value =
            flow_like_types::json::from_slice(pin.default_value.as_ref().unwrap()).unwrap();
        assert_eq!(default, json!(true));
    }

    #[test]
    fn test_build_micro_widget_component_frozen_shape() {
        let contract = json!({
            "contractVersion": 1,
            "id": "sales-chart",
            "inputs": {
                "title": { "type": "string", "default": "Sales" }
            }
        });
        let entry = PackageWidgetRef {
            package_id: "com.example.sales".to_string(),
            package_version: "1.2.0".to_string(),
            widget_id: "sales-chart".to_string(),
            name: "Sales Chart".to_string(),
            description: String::new(),
            bundle_hash: Some("deadbeef".to_string()),
            contract: contract.clone(),
        };

        let mut props = flow_like_types::json::Map::new();
        props.insert("title".to_string(), json!("Q3 Sales"));

        let component = build_micro_widget_component(
            &entry,
            "instance-1",
            props,
            flow_like_types::json::Map::new(),
        );

        assert_eq!(
            component,
            json!({
                "type": "microWidgetInstance",
                "instanceId": "instance-1",
                "packageId": "com.example.sales",
                "widgetId": "sales-chart",
                "packageVersion": "1.2.0",
                "bundleHash": "deadbeef",
                "contract": contract,
                "props": { "title": "Q3 Sales" },
                "actionBindings": {},
                "preview": false,
            })
        );
    }

    #[test]
    fn test_build_micro_widget_component_null_bundle_hash() {
        let entry = PackageWidgetRef {
            package_id: "com.example.sales".to_string(),
            package_version: "1.0.0".to_string(),
            widget_id: "kpi-card".to_string(),
            name: "KPI Card".to_string(),
            description: String::new(),
            bundle_hash: None,
            contract: json!({ "contractVersion": 1, "id": "kpi-card" }),
        };

        let component = build_micro_widget_component(
            &entry,
            "i-2",
            flow_like_types::json::Map::new(),
            flow_like_types::json::Map::new(),
        );
        assert_eq!(component.get("bundleHash"), Some(&Value::Null));
        assert_eq!(component.get("props"), Some(&json!({})));
        assert_eq!(component.get("actionBindings"), Some(&json!({})));
        assert_eq!(component.get("preview"), Some(&json!(false)));
    }

    #[test]
    fn test_build_micro_widget_component_preserves_action_bindings() {
        let entry = PackageWidgetRef {
            package_id: "com.example.sales".to_string(),
            package_version: "1.0.0".to_string(),
            widget_id: "sales-chart".to_string(),
            name: "Sales Chart".to_string(),
            description: String::new(),
            bundle_hash: Some("deadbeef".to_string()),
            contract: json!({
                "contractVersion": 1,
                "id": "sales-chart",
                "events": {
                    "pointSelected": { "description": "A point was selected" },
                    "refreshRequested": { "description": "Refresh was requested" }
                }
            }),
        };
        let mut action_bindings = flow_like_types::json::Map::new();
        action_bindings.insert(
            "pointSelected".to_string(),
            json!({
                "workflow": {
                    "flowId": "handle-point-selected",
                    "inputMappings": {}
                }
            }),
        );
        action_bindings.insert(
            "refreshRequested".to_string(),
            json!({
                "workflow": {
                    "flowId": "handle-refresh",
                    "inputMappings": {}
                }
            }),
        );

        let component = build_micro_widget_component(
            &entry,
            "instance-with-bindings",
            flow_like_types::json::Map::new(),
            action_bindings.clone(),
        );

        assert_eq!(
            component.get("actionBindings"),
            Some(&Value::Object(action_bindings))
        );
    }
}
