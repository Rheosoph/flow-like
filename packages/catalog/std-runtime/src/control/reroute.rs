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
                tracing::warn!("Input Reroute pin not found");
                return;
            }
        }
        .clone();

        let output = match node.get_pin_by_name("route_out") {
            Some(pin) => pin,
            None => {
                tracing::warn!("Output Reroute pin not found");
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

#[cfg(test)]
mod tests {
    use super::*;
    use flow_like::{
        flow::{
            board::commands::{
                GenericCommand,
                nodes::add_node::AddNodeCommand,
                pins::{connect_pins::ConnectPinsCommand, disconnect_pins::DisconnectPinsCommand},
            },
            pin::{Pin, resolve_schema},
        },
        state::{FlowLikeConfig, FlowLikeState},
        utils::http::HTTPClient,
    };
    use flow_like_catalog_core::FlowPath;
    use std::sync::Arc;

    fn connect(from: &Node, to: &Node) -> GenericCommand {
        GenericCommand::ConnectPin(ConnectPinsCommand::new(
            from.id.clone(),
            to.id.clone(),
            from.get_pin_by_name("route_out").unwrap().id.clone(),
            to.get_pin_by_name("route_in").unwrap().id.clone(),
        ))
    }

    fn insert_reroute(board: &Board, from: &str, to: &str) -> (String, Vec<GenericCommand>) {
        let source = &board.nodes[from];
        let target = &board.nodes[to];
        let source_pin = source.get_pin_by_name("route_out").unwrap();
        let mut reroute = RerouteNode::new().get_node();
        // The editor copies the compact source schema before submitting the wire-split batch.
        for pin in reroute.pins.values_mut() {
            pin.data_type = source_pin.data_type.clone();
            pin.value_type = source_pin.value_type.clone();
            pin.schema = source_pin.schema.clone();
        }
        let id = reroute.id.clone();
        let commands = vec![
            GenericCommand::AddNode(AddNodeCommand {
                node: reroute.clone(),
                current_layer: None,
            }),
            GenericCommand::DisconnectPin(DisconnectPinsCommand::new(
                source.id.clone(),
                target.id.clone(),
                source_pin.id.clone(),
                target.get_pin_by_name("route_in").unwrap().id.clone(),
            )),
            connect(source, &reroute),
            connect(&reroute, target),
        ];
        (id, commands)
    }

    fn assert_reroute_schema(board: &Board, id: &str, value_type: &ValueType) {
        for pin in board.nodes[id].pins.values() {
            assert_eq!(pin.data_type, VariableType::Struct);
            assert_eq!(&pin.value_type, value_type);
            assert_eq!(
                resolve_schema(pin.schema.as_deref().unwrap(), &board.refs).unwrap(),
                Pin::schema_string_for::<FlowPath>().unwrap()
            );
        }
    }

    #[tokio::test]
    async fn wire_split_preserves_flow_path_schemas_through_chains_and_branches() {
        for value_type in [ValueType::Normal, ValueType::Array] {
            let state = Arc::new(FlowLikeState::new(
                FlowLikeConfig::new(),
                HTTPClient::new_without_refetch(),
            ));
            state
                .node_registry()
                .write()
                .await
                .push_node(Arc::new(RerouteNode::new()));
            let mut board = Board::new_detached(Some("reroute-test".into()), Default::default());
            board.refs.insert(
                "struct-schema".into(),
                Pin::schema_string_for::<FlowPath>().unwrap(),
            );
            let mut source = Node::new("source", "Source", "", "Test");
            let output = source.add_output_pin("route_out", "Out", "", VariableType::Struct);
            output.schema = Some("struct-schema".into());
            output.value_type = value_type.clone();
            let mut target = Node::new("target", "Target", "", "Test");
            let input = target.add_input_pin("route_in", "In", "", VariableType::Struct);
            input.schema = Some("struct-schema".into());
            input.value_type = value_type.clone();
            let mut branch = Node::new("branch", "Branch", "", "Test");
            let input = branch.add_input_pin("route_in", "In", "", VariableType::Struct);
            input.schema = Some("struct-schema".into());
            input.value_type = value_type.clone();
            for node in [&source, &target, &branch] {
                board.nodes.insert(node.id.clone(), node.clone());
            }
            board
                .execute_command(connect(&source, &target), state.clone())
                .await
                .unwrap();

            let (first, commands) = insert_reroute(&board, &source.id, &target.id);
            let inserted = board
                .execute_commands(commands, state.clone())
                .await
                .unwrap();
            assert_reroute_schema(&board, &first, &value_type);
            board.undo(inserted.clone(), state.clone()).await.unwrap();
            assert!(!board.nodes.contains_key(&first));
            board.redo(inserted, state.clone()).await.unwrap();
            assert_reroute_schema(&board, &first, &value_type);

            let (second, commands) = insert_reroute(&board, &first, &target.id);
            board
                .execute_commands(commands, state.clone())
                .await
                .unwrap();
            board
                .execute_command(connect(&board.nodes[&second], &branch), state.clone())
                .await
                .unwrap();
            for id in [&first, &second] {
                assert_reroute_schema(&board, id, &value_type);
            }
            let output = board.nodes[&second].get_pin_by_name("route_out").unwrap();
            assert_eq!(output.connected_to.len(), 2);
            for consumer in [&target, &branch] {
                let input = board.nodes[&consumer.id]
                    .get_pin_by_name("route_in")
                    .unwrap();
                assert!(output.connected_to.contains(&input.id));
                assert!(input.depends_on.contains(&output.id));
                assert!(flow_like::flow::pin::schemas_are_compatible(
                    Some(resolve_schema(output.schema.as_deref().unwrap(), &board.refs).unwrap()),
                    Some(resolve_schema(input.schema.as_deref().unwrap(), &board.refs).unwrap()),
                ));
            }
        }
    }
}
