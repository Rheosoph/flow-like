use super::element_utils::extract_element_id_from_pin;
use super::schema_utils::{set_component_schema_by_type, set_generic_element_schema};
use crate::a2ui::micro_widget_utils::{
    clear_widget_ref_metadata, set_widget_ref_metadata, widget_contract_from_component,
};
use flow_like::a2ui::{A2UIElement, SurfaceComponent};
use flow_like::flow::{
    board::Board,
    execution::{LogLevel, context::ExecutionContext},
    node::{Node, NodeLogic},
    pin::{Pin, PinOptions, ValueType},
    variable::VariableType,
};
use flow_like_types::{Value, async_trait};
use std::collections::HashSet;

fn literal_selector(pin: &Pin) -> Option<String> {
    if !pin.depends_on.is_empty() {
        return None;
    }
    flow_like_types::json::from_slice::<String>(pin.default_value.as_deref()?).ok()
}

/// Resolve the saved selector without evaluating workflow code. Top-level FlowScript constants
/// are board variables, so their getter connection must retain the same contract as a literal.
fn element_selector(board: &Board, input: &Pin) -> Option<String> {
    let mut pin = input;
    let mut visited = HashSet::new();
    loop {
        if pin.depends_on.is_empty() {
            return literal_selector(pin);
        }
        if pin.depends_on.len() != 1 {
            return None;
        }
        let source_id = pin.depends_on.iter().next()?;
        if !visited.insert(source_id) || visited.len() > 128 {
            return None;
        }
        let source = board.get_pin_by_id(source_id)?;
        let producer = board
            .nodes
            .values()
            .chain(board.layers.values().flat_map(|layer| layer.nodes.values()))
            .find(|node| node.pins.contains_key(source_id));
        match producer {
            Some(node) if node.name == "reroute" && source.name == "route_out" => {
                pin = node.get_pin_by_name("route_in")?;
            }
            Some(node) if node.name == "variable_get" && source.name == "value_ref" => {
                let variable_id = literal_selector(node.get_pin_by_name("var_ref")?)?;
                let variable = board.get_any_variable(&variable_id)?;
                if variable.data_type != VariableType::String
                    || variable.value_type != ValueType::Normal
                    || variable.exposed
                    || variable.secret
                    || variable.runtime_configured
                {
                    return None;
                }
                // A default cannot describe the widget contract if workflow code can replace it.
                let written = board
                    .nodes
                    .values()
                    .chain(board.layers.values().flat_map(|layer| layer.nodes.values()))
                    .filter(|node| node.name == "variable_set")
                    .any(|node| {
                        node.get_pin_by_name("var_ref")
                            .and_then(literal_selector)
                            .is_none_or(|target| target == variable_id)
                    });
                if written {
                    return None;
                }
                return flow_like_types::json::from_slice::<String>(
                    variable.default_value.as_deref()?,
                )
                .ok();
            }
            // Layer boundary pins relay their upstream source. An unconnected boundary is a
            // runtime parameter, even when it has a default.
            None if !source.depends_on.is_empty() => pin = source,
            _ => return None,
        }
    }
}

/// Gets an element's data from the workflow payload.
///
/// The element data should be included in the workflow payload when triggered from the UI.
/// The payload structure expected is:
/// ```json
/// {
///   "_elements": {
///     "element_id": { /* full component data */ }
///   }
/// }
/// ```
#[crate::register_node]
#[derive(Default)]
pub struct GetElement;

impl GetElement {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for GetElement {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "a2ui_get_element",
            "Get Element",
            "Gets an element's data from the page",
            "UI/Elements",
        );
        node.set_flowscript_name("ui", "getElement");
        node.add_icon("/flow/icons/a2ui.svg");

        node.add_input_pin(
            "element_ref",
            "Element",
            "Reference to the page element",
            VariableType::String,
        );

        node.add_output_pin(
            "element",
            "Element",
            "The element data",
            VariableType::Struct,
        )
        .set_schema::<A2UIElement>()
        .set_options(PinOptions::new().set_enforce_schema(false).build());

        node.add_output_pin(
            "exists",
            "Exists",
            "Whether the element exists",
            VariableType::Boolean,
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let element_value: Value = context.evaluate_pin("element_ref").await?;
        let element_id = extract_element_id_from_pin(element_value)
            .ok_or_else(|| flow_like_types::anyhow!("Invalid element reference"))?;

        context.log_message(
            &format!("[GetElement] Looking for element_id: {}", element_id),
            LogLevel::Debug,
        );

        let element = context.read_element(&element_id).await?;

        if let Some((found_id, element_value)) = element {
            // Create element with __element_id for use with setter nodes
            let mut enriched_element = element_value.clone();
            if let Some(obj) = enriched_element.as_object_mut() {
                obj.insert("__element_id".to_string(), Value::String(found_id.clone()));
            }

            let element_pin = context.get_pin_by_name("element").await?;
            element_pin.set_value(enriched_element).await;

            let exists_pin = context.get_pin_by_name("exists").await?;
            exists_pin.set_value(Value::Bool(true)).await;

            context.log_message(
                &format!("Got element: {} (found as {})", element_id, found_id),
                LogLevel::Debug,
            );
        } else {
            let element_pin = context.get_pin_by_name("element").await?;
            element_pin.set_value(Value::Null).await;

            let exists_pin = context.get_pin_by_name("exists").await?;
            exists_pin.set_value(Value::Bool(false)).await;

            context.log_message(
                &format!("Element not found: {}", element_id),
                LogLevel::Debug,
            );
        }

        Ok(())
    }

    async fn on_update(&self, node: &mut Node, board: &Board) {
        node.error = None;

        let read_only_node = node.clone();
        let element_ref = match read_only_node.get_pin_by_name("element_ref") {
            Some(pin) => pin,
            None => {
                node.error = Some("Element reference pin not found!".to_string());
                return;
            }
        };

        let element_id = element_selector(board, element_ref);

        if let Some(id) = &element_id {
            let short_name = id.rsplit('/').next().unwrap_or(id);
            let component = find_component_in_board(board, id).await;

            if let Some(component) = component {
                let comp_type = component.get_component_type_name();
                node.friendly_name = format!("Get {} ({})", short_name, comp_type);

                if let Some(element_pin) = node.get_pin_mut_by_name("element") {
                    set_component_schema_by_type(element_pin, &comp_type);
                    if let Some(metadata) = widget_contract_from_component(&component.component)
                        && let Ok(contract) = flow_like_types::json::to_value(&metadata.contract)
                    {
                        set_widget_ref_metadata(
                            element_pin,
                            &metadata.selector,
                            &metadata.widget_name,
                            &contract,
                        );
                    }
                }
            } else {
                node.friendly_name = format!("Get {}", short_name);
                if let Some(element_pin) = node.get_pin_mut_by_name("element") {
                    set_generic_element_schema(element_pin);
                }
            }
        } else {
            node.friendly_name = "Get Element".to_string();
            if let Some(element_pin) = node.get_pin_mut_by_name("element") {
                set_generic_element_schema(element_pin);
                clear_widget_ref_metadata(element_pin);
            }
        }
    }
}

pub(crate) async fn find_component_in_board(
    board: &Board,
    element_id: &str,
) -> Option<SurfaceComponent> {
    let loaded = board.load_all_pages(None).await.ok()?;

    // A known Page can arrive before its payload. Do not borrow a different Page's
    // contract while the explicitly selected Page is unavailable.
    if let Some((page_id, _)) = element_id.split_once('/')
        && loaded.unreadable.iter().any(|page| page.page_id == page_id)
    {
        return None;
    }

    // The exactly referenced page wins so schema and display come from the user's actual
    // selection; any page's same-named component is only the fallback, matching the
    // runtime's page-retargeting of element refs.
    for page in &loaded.pages {
        for component in &page.components {
            if component.id == element_id || element_id == format!("{}/{}", page.id, component.id) {
                return Some(component.clone());
            }
        }
    }

    for page in &loaded.pages {
        for component in &page.components {
            if element_id.ends_with(&format!("/{}", component.id))
                || element_id == component.id.split('/').next_back().unwrap_or("")
            {
                return Some(component.clone());
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use flow_like::flow::{
        board::{Layer, LayerType},
        variable::Variable,
    };
    use flow_like_storage::object_store::path::Path;
    use flow_like_types::json::json;

    fn selector_graph() -> (Board, Pin) {
        let mut board = Board::new_detached(Some("selectors".into()), Path::default());
        let mut variable = Variable::new("selector", VariableType::String, ValueType::Normal);
        variable.id = "selector".into();
        variable.set_default_value(json!("page/chat"));
        board.variables.insert(variable.id.clone(), variable);

        let mut getter = Node::new("variable_get", "Get selector", "", "Variable");
        getter.id = "getter".into();
        getter
            .add_input_pin("var_ref", "Variable", "", VariableType::String)
            .set_default_value(Some(json!("selector")));
        let output = getter
            .add_output_pin("value_ref", "Value", "", VariableType::String)
            .id
            .clone();
        let mut input = GetElement::new()
            .get_node()
            .get_pin_by_name("element_ref")
            .unwrap()
            .clone();
        input.depends_on.insert(output);
        board.nodes.insert(getter.id.clone(), getter);
        (board, input)
    }

    fn setter(target: &str) -> Node {
        let mut setter = Node::new("variable_set", "Set selector", "", "Variable");
        setter
            .add_input_pin("var_ref", "Variable", "", VariableType::String)
            .set_default_value(Some(json!(target)));
        setter
    }

    fn reroute(source_id: &str) -> Node {
        let mut reroute = Node::new("reroute", "Reroute", "", "Utils");
        reroute
            .add_input_pin("route_in", "Input", "", VariableType::String)
            .depends_on
            .insert(source_id.to_string());
        reroute.add_output_pin("route_out", "Output", "", VariableType::String);
        reroute
    }

    #[test]
    fn saved_selector_defaults_refresh_without_replacing_the_connection() {
        let (mut board, mut input) = selector_graph();
        input.set_default_value(Some(json!("page/stale")));
        let original = flow_like_types::json::to_value(&input).unwrap();

        assert_eq!(
            element_selector(&board, &input).as_deref(),
            Some("page/chat")
        );
        board
            .variables
            .get_mut("selector")
            .unwrap()
            .set_default_value(json!("page/other"));
        assert_eq!(
            element_selector(&board, &input).as_deref(),
            Some("page/other")
        );
        assert_eq!(flow_like_types::json::to_value(&input).unwrap(), original);

        input.depends_on.clear();
        assert_eq!(
            element_selector(&board, &input).as_deref(),
            Some("page/stale")
        );
    }

    #[test]
    fn wired_defaults_cannot_resolve_a_dynamic_selector_or_variable_reference() {
        let (mut board, mut input) = selector_graph();
        input.set_default_value(Some(json!("page/stale")));
        board.nodes.get_mut("getter").unwrap().name = "dynamic_string".into();
        assert_eq!(element_selector(&board, &input), None);

        let getter = board.nodes.get_mut("getter").unwrap();
        getter.name = "variable_get".into();
        getter
            .get_pin_mut_by_name("var_ref")
            .unwrap()
            .depends_on
            .insert("runtime-variable-id".into());
        assert_eq!(element_selector(&board, &input), None);
    }

    #[test]
    fn overridable_or_non_string_variables_do_not_define_a_widget_contract() {
        for excluded in [
            "exposed", "secret", "runtime", "array", "integer", "missing",
        ] {
            let (mut board, input) = selector_graph();
            let variable = board.variables.get_mut("selector").unwrap();
            match excluded {
                "exposed" => variable.exposed = true,
                "secret" => variable.secret = true,
                "runtime" => variable.runtime_configured = true,
                "array" => variable.value_type = ValueType::Array,
                "integer" => variable.data_type = VariableType::Integer,
                "missing" => variable.default_value = None,
                _ => unreachable!(),
            }
            assert_eq!(element_selector(&board, &input), None, "{excluded}");
        }
    }

    #[test]
    fn matching_and_dynamic_variable_writers_invalidate_the_saved_selector() {
        let (mut board, input) = selector_graph();
        let mut writer = setter("unrelated");
        writer.id = "writer".into();
        board.nodes.insert(writer.id.clone(), writer);
        assert_eq!(
            element_selector(&board, &input).as_deref(),
            Some("page/chat")
        );

        board
            .nodes
            .get_mut("writer")
            .unwrap()
            .get_pin_mut_by_name("var_ref")
            .unwrap()
            .set_default_value(Some(json!("selector")));
        assert_eq!(element_selector(&board, &input), None);

        let reference = board
            .nodes
            .get_mut("writer")
            .unwrap()
            .get_pin_mut_by_name("var_ref")
            .unwrap();
        reference.set_default_value(Some(json!("unrelated")));
        reference.depends_on.insert("dynamic-target".into());
        assert_eq!(element_selector(&board, &input), None);

        board.nodes.remove("writer");
        assert_eq!(
            element_selector(&board, &input).as_deref(),
            Some("page/chat")
        );
    }

    #[test]
    fn selector_resolution_checks_variables_and_writers_stored_in_layers() {
        let (mut board, input) = selector_graph();
        let mut layer = Layer::new("module".into(), "Module".into(), LayerType::Module);
        let variable = board.variables.remove("selector").unwrap();
        layer.variables.insert(variable.id.clone(), variable);
        let getter = board.nodes.remove("getter").unwrap();
        layer.nodes.insert(getter.id.clone(), getter);
        board.layers.insert(layer.id.clone(), layer);
        assert_eq!(
            element_selector(&board, &input).as_deref(),
            Some("page/chat")
        );

        let writer = setter("selector");
        board
            .layers
            .get_mut("module")
            .unwrap()
            .nodes
            .insert(writer.id.clone(), writer);
        assert_eq!(element_selector(&board, &input), None);
    }

    #[test]
    fn selectors_cross_reroutes_and_connected_layer_boundaries() {
        let (mut board, mut input) = selector_graph();
        let source_id = input.depends_on.iter().next().unwrap().clone();
        let route = reroute(&source_id);
        let route_out = route.get_pin_by_name("route_out").unwrap().clone();
        board.nodes.insert(route.id.clone(), route);

        let mut boundary = route_out.clone();
        boundary.id = "boundary".into();
        boundary.depends_on.insert(route_out.id);
        boundary.set_default_value(Some(json!("page/stale-boundary")));
        let mut layer = Layer::new("layer".into(), "Layer".into(), LayerType::Collapsed);
        layer.pins.insert(boundary.id.clone(), boundary);
        board.layers.insert(layer.id.clone(), layer);
        input.depends_on = ["boundary".to_string()].into();
        assert_eq!(
            element_selector(&board, &input).as_deref(),
            Some("page/chat")
        );

        board
            .layers
            .get_mut("layer")
            .unwrap()
            .pins
            .get_mut("boundary")
            .unwrap()
            .depends_on
            .clear();
        assert_eq!(element_selector(&board, &input), None);
    }

    #[test]
    fn cycles_ambiguous_sources_and_missing_sources_do_not_use_defaults() {
        let (mut board, mut input) = selector_graph();
        input.set_default_value(Some(json!("page/stale")));
        input.depends_on.insert("second-source".into());
        assert_eq!(element_selector(&board, &input), None);
        input.depends_on = ["missing-source".to_string()].into();
        assert_eq!(element_selector(&board, &input), None);

        let mut route = reroute("temporary");
        let output = route.get_pin_by_name("route_out").unwrap().id.clone();
        route.get_pin_mut_by_name("route_in").unwrap().depends_on = [output.clone()].into();
        board.nodes.insert(route.id.clone(), route);
        input.depends_on = [output].into();
        assert_eq!(element_selector(&board, &input), None);
    }
}
