use flow_like::flow::{
    board::Board,
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    pin::ValueType,
    variable::VariableType,
};
use flow_like_types::{Value, async_trait};

#[crate::register_node]
#[derive(Default)]
pub struct RerouteNode {}

impl RerouteNode {
    pub fn new() -> Self {
        RerouteNode {}
    }
}

#[async_trait]
impl NodeLogic for RerouteNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new("reroute", "Reroute", "Control Flow Node", "Control");
        node.set_flowscript_name("control", "reroute");
        node.add_input_pin("route_in", "In", "", VariableType::Generic);
        node.add_output_pin("route_out", "Out", "", VariableType::Generic);

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let input: Value = context.evaluate_pin("route_in").await?;
        context.set_pin_value("route_out", input).await?;
        return Ok(());
    }

    async fn on_update(&self, node: &mut Node, board: &Board) {
        let found_input_type = node
            .match_type("route_in", board, None, None)
            .unwrap_or(VariableType::Generic);

        let found_output_type = node
            .match_type("route_out", board, None, None)
            .unwrap_or(VariableType::Generic);

        let input = match node.get_pin_by_name("route_in") {
            Some(pin) => pin,
            None => {
                println!("Input Reroute pin not found");
                return;
            }
        }
        .clone();

        let output = match node.get_pin_by_name("route_out") {
            Some(pin) => pin,
            None => {
                println!("Output Reroute pin not found");
                return;
            }
        }
        .clone();

        if !input.depends_on.is_empty() && found_input_type != VariableType::Generic {
            let output = node.get_pin_mut_by_name("route_out").unwrap();
            output.value_type = input.value_type.clone();
            output.data_type = found_input_type.clone();
            output.schema = input.schema.clone();
            output.options = input.options.clone();
            return;
        }

        if !output.connected_to.is_empty() && found_output_type != VariableType::Generic {
            let input = node.get_pin_mut_by_name("route_in").unwrap();
            input.value_type = output.value_type.clone();
            input.data_type = found_output_type.clone();
            input.schema = output.schema.clone();
            input.options = output.options.clone();
            return;
        }

        let input = node.get_pin_mut_by_name("route_in").unwrap();
        input.value_type = ValueType::Normal;
        input.data_type = VariableType::Generic;
        input.schema = None;
        input.options = None;
        let output = node.get_pin_mut_by_name("route_out").unwrap();
        output.value_type = ValueType::Normal;
        output.data_type = VariableType::Generic;
        output.schema = None;
        output.options = None;
    }
}
