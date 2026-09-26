use std::{collections::BTreeMap, sync::Arc};

use super::{
    GenericCommand,
    nodes::{add_node::AddNodeCommand, move_node::MoveNodeCommand, remove_node::RemoveNodeCommand},
    pins::{connect_pins::ConnectPinsCommand, disconnect_pins::DisconnectPinsCommand},
};
use crate::{
    flow::{
        board::{Board, Layer, LayerType},
        node::Node,
        pin::PinType,
        variable::VariableType,
    },
    state::{FlowLikeConfig, FlowLikeState},
    utils::http::HTTPClient,
};
use flow_like_storage::Path;

fn node(id: &str, reroute: bool, layer: Option<&str>) -> Node {
    let mut node = Node::new(if reroute { "reroute" } else { id }, id, "", "Test");
    node.id = id.to_string();
    node.coordinates = Some((0.0, 0.0, 0.0));
    node.layer = layer.map(str::to_string);
    if id != "source" {
        node.add_input_pin("route_in", "In", "", VariableType::String);
    }
    if id == "source" || reroute {
        node.add_output_pin("route_out", "Out", "", VariableType::String);
    }
    node.auto_reroute = reroute.then_some(true);
    node
}

fn connect(from: &Node, to: &Node) -> GenericCommand {
    GenericCommand::ConnectPin(ConnectPinsCommand::new(
        from.id.clone(),
        to.id.clone(),
        from.pins
            .values()
            .find(|pin| pin.pin_type == PinType::Output)
            .unwrap()
            .id
            .clone(),
        to.pins
            .values()
            .find(|pin| pin.pin_type == PinType::Input)
            .unwrap()
            .id
            .clone(),
    ))
}

fn disconnect(from: &Node, to: &Node) -> GenericCommand {
    let GenericCommand::ConnectPin(wire) = connect(from, to) else {
        unreachable!()
    };
    GenericCommand::DisconnectPin(DisconnectPinsCommand::new(
        wire.from_node,
        wire.to_node,
        wire.from_pin,
        wire.to_pin,
    ))
}

fn remove_disconnected(node: &Node) -> GenericCommand {
    let mut snapshot = node.clone();
    for pin in snapshot.pins.values_mut() {
        pin.connected_to.clear();
        pin.depends_on.clear();
    }
    GenericCommand::RemoveNode(RemoveNodeCommand::new(snapshot))
}

fn topology(board: &Board) -> flow_like_types::Value {
    let nodes: BTreeMap<_, _> = board
        .nodes
        .iter()
        .map(|(id, node)| {
            let pins: BTreeMap<_, _> = node
                .pins
                .iter()
                .map(|(id, pin)| (id, (&pin.depends_on, &pin.connected_to)))
                .collect();
            (id, (&node.layer, node.auto_reroute, node.coordinates, pins))
        })
        .collect();
    flow_like_types::json::to_value(nodes).unwrap()
}

// Deserialize each batch as the backend does for an editor command payload.
fn through_wire(commands: Vec<GenericCommand>) -> Vec<GenericCommand> {
    flow_like_types::json::from_slice(&flow_like_types::json::to_vec(&commands).unwrap()).unwrap()
}

async fn insert_and_remove_chain(layer_id: Option<&str>) {
    let state = Arc::new(FlowLikeState::new(
        FlowLikeConfig::new(),
        HTTPClient::new_without_refetch(),
    ));
    let mut board = Board::new_detached(Some("routing".into()), Path::default());
    if let Some(id) = layer_id {
        board.layers.insert(
            id.into(),
            Layer::new(id.into(), "Group".into(), LayerType::Collapsed),
        );
    }
    let source = node("source", false, layer_id);
    let mut target = node("target", false, layer_id);
    target.coordinates = Some((400.0, 0.0, 0.0));
    let fanout = node("fanout", false, layer_id);
    for node in [&source, &target, &fanout] {
        board.nodes.insert(node.id.clone(), node.clone());
    }
    board
        .execute_commands(
            through_wire(vec![connect(&source, &target), connect(&source, &fanout)]),
            state.clone(),
        )
        .await
        .expect("initial fanout");
    let direct = topology(&board);

    let mut first = node("first", true, layer_id);
    first.coordinates = Some((100.0, 60.0, 0.0));
    let mut second = node("second", true, layer_id);
    second.coordinates = Some((300.0, 60.0, 0.0));
    let inserted = board
        .execute_commands(
            through_wire(vec![
                GenericCommand::MoveNode(MoveNodeCommand::new(
                    target.id.clone(),
                    (500.0, 0.0, 0.0),
                    layer_id.map(str::to_string),
                )),
                GenericCommand::AddNode(AddNodeCommand {
                    node: first.clone(),
                    current_layer: layer_id.map(str::to_string),
                }),
                GenericCommand::AddNode(AddNodeCommand {
                    node: second.clone(),
                    current_layer: layer_id.map(str::to_string),
                }),
                disconnect(&source, &target),
                connect(&source, &first),
                connect(&first, &second),
                connect(&second, &target),
            ]),
            state.clone(),
        )
        .await
        .expect("route through two nodes");
    let routed = topology(&board);
    assert_eq!(board.nodes.len(), 5);
    let output = board.nodes[&source.id]
        .get_pin_by_name("route_out")
        .unwrap();
    assert_eq!(output.connected_to.len(), 2);
    assert!(
        output
            .connected_to
            .contains(&fanout.get_pin_by_name("route_in").unwrap().id)
    );
    for id in [&first.id, &second.id] {
        assert_eq!(board.nodes[id].layer.as_deref(), layer_id);
        assert_eq!(board.nodes[id].auto_reroute, Some(true));
    }

    board
        .undo(inserted.clone(), state.clone())
        .await
        .expect("undo routed layout once");
    assert_eq!(topology(&board), direct);
    board
        .redo(inserted, state.clone())
        .await
        .expect("redo routed layout once");
    assert_eq!(topology(&board), routed);

    let removed = board
        .execute_commands(
            through_wire(vec![
                GenericCommand::MoveNode(MoveNodeCommand::new(
                    target.id.clone(),
                    (400.0, 0.0, 0.0),
                    layer_id.map(str::to_string),
                )),
                disconnect(&source, &first),
                disconnect(&first, &second),
                disconnect(&second, &target),
                remove_disconnected(&board.nodes[&first.id]),
                remove_disconnected(&board.nodes[&second.id]),
                connect(&source, &target),
            ]),
            state.clone(),
        )
        .await
        .expect("replace chain with direct wire");
    assert_eq!(topology(&board), direct);
    board
        .undo(removed.clone(), state.clone())
        .await
        .expect("undo chain removal once");
    assert_eq!(topology(&board), routed);
    board
        .redo(removed, state)
        .await
        .expect("redo chain removal once");
    assert_eq!(topology(&board), direct);
}

#[flow_like_types::tokio::test]
async fn auto_reroute_batch_preserves_fanout_and_undo_redo() {
    insert_and_remove_chain(None).await;
}

#[flow_like_types::tokio::test]
async fn auto_reroute_batch_preserves_layer_and_undo_redo() {
    insert_and_remove_chain(Some("group")).await;
}
