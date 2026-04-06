use flow_like_types::async_trait;
use schemars::JsonSchema;
use std::{collections::BTreeSet, sync::Arc};

use crate::{
    flow::{
        board::{Board, Layer, commands::Command},
        node::Node,
        pin::PinType,
        variable::VariableType,
    },
    state::FlowLikeState,
};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug)]
enum NodeOrLayer {
    Node(Node),
    Layer(Layer),
}

impl NodeOrLayer {
    fn is_layer(&self) -> bool {
        match self {
            NodeOrLayer::Node(_) => false,
            NodeOrLayer::Layer(_) => true,
        }
    }
}

#[derive(Clone, Serialize, Deserialize, JsonSchema)]
pub struct ConnectPinsCommand {
    pub from_pin: String,
    pub to_pin: String,
    pub from_node: String,
    pub to_node: String,
}

impl ConnectPinsCommand {
    pub fn new(from_node: String, to_node: String, from_pin: String, to_pin: String) -> Self {
        ConnectPinsCommand {
            from_pin,
            to_pin,
            from_node,
            to_node,
        }
    }
}

fn find_node_or_layer(board: &Board, id: &str) -> flow_like_types::Result<NodeOrLayer> {
    if let Some(node) = board.nodes.get(id) {
        return Ok(NodeOrLayer::Node(node.clone()));
    }
    if let Some(layer) = board.layers.get(id) {
        return Ok(NodeOrLayer::Layer(layer.clone()));
    }
    Err(flow_like_types::anyhow!("Entity ({}) not found", id))
}

fn upsert_node_or_layer(board: &mut Board, entity: NodeOrLayer) {
    match entity {
        NodeOrLayer::Node(node) => {
            board.nodes.insert(node.id.clone(), node);
        }
        NodeOrLayer::Layer(layer) => {
            board.layers.insert(layer.id.clone(), layer);
        }
    }
}

#[async_trait]
impl Command for ConnectPinsCommand {
    async fn execute(
        &mut self,
        board: &mut Board,
        _state: Arc<FlowLikeState>,
    ) -> flow_like_types::Result<()> {
        connect_pins(board, &self.from_node, &self.from_pin, &self.to_node, &self.to_pin)
    }

    async fn undo(
        &mut self,
        board: &mut Board,
        _state: Arc<FlowLikeState>,
    ) -> flow_like_types::Result<()> {
        disconnect_pins(board, &self.from_node, &self.from_pin, &self.to_node, &self.to_pin)
    }
}

pub fn connect_pins(
    board: &mut Board,
    from_node: &str,
    from_pin: &str,
    to_node: &str,
    to_pin: &str,
) -> flow_like_types::Result<()> {
    if from_node == to_node {
        return Err(flow_like_types::anyhow!(
            "Cannot connect a node to itself".to_string()
        ));
    }

    if from_pin == to_pin {
        return Err(flow_like_types::anyhow!(
            "Cannot connect a pin to itself".to_string()
        ));
    }

    let mut from_entity = find_node_or_layer(board, from_node)?;
    let from_is_layer = from_entity.is_layer();
    let mut to_entity = find_node_or_layer(board, to_node)?;
    let to_is_layer = to_entity.is_layer();

    let from_pin_ref = match &mut from_entity {
        NodeOrLayer::Node(node) => node.pins.get_mut(from_pin),
        NodeOrLayer::Layer(layer) => layer.pins.get_mut(from_pin),
    }
    .ok_or_else(|| flow_like_types::anyhow!("From Pin ({}) not found in container", from_pin))?;

    let to_pin_ref = match &mut to_entity {
        NodeOrLayer::Node(node) => node.pins.get_mut(to_pin),
        NodeOrLayer::Layer(layer) => layer.pins.get_mut(to_pin),
    }
    .ok_or_else(|| flow_like_types::anyhow!("To Pin ({}) not found in container", to_pin))?;

    if from_pin_ref.pin_type == PinType::Input && !from_is_layer {
        return Err(flow_like_types::anyhow!(
            "Cannot connect an input pin".to_string()
        ));
    }

    if to_pin_ref.pin_type == PinType::Output && !to_is_layer {
        return Err(flow_like_types::anyhow!(
            "Cannot connect an output pin".to_string()
        ));
    }

    if from_pin_ref.data_type == VariableType::Execution {
        let mut old_connect_to = from_pin_ref.connected_to.clone();
        from_pin_ref.connected_to = BTreeSet::from([to_pin_ref.id.clone()]);
        old_connect_to.remove(&to_pin_ref.id);

        board.nodes.iter_mut().for_each(|(_, node)| {
            node.pins.iter_mut().for_each(|(_, pin)| {
                pin.depends_on.remove(&from_pin_ref.id);
            });
        });
        board.layers.iter_mut().for_each(|(_, layer)| {
            layer.pins.iter_mut().for_each(|(_, pin)| {
                pin.depends_on.remove(&from_pin_ref.id);
            });
        });

        to_pin_ref.depends_on.insert(from_pin_ref.id.clone());
    }

    if from_pin_ref.data_type != VariableType::Execution {
        to_pin_ref.depends_on = BTreeSet::from([from_pin_ref.id.clone()]);
    }

    from_pin_ref.connected_to.insert(to_pin_ref.id.clone());

    upsert_node_or_layer(board, from_entity);
    upsert_node_or_layer(board, to_entity);

    Ok(())
}

pub fn disconnect_pins(
    board: &mut Board,
    from_node: &str,
    from_pin: &str,
    to_node: &str,
    to_pin: &str,
) -> flow_like_types::Result<()> {
    let mut from_entity = find_node_or_layer(board, from_node)?;
    let mut to_entity = find_node_or_layer(board, to_node)?;

    let from_pin_ref = match &mut from_entity {
        NodeOrLayer::Node(node) => node.pins.get_mut(from_pin),
        NodeOrLayer::Layer(layer) => layer.pins.get_mut(from_pin),
    }
    .ok_or_else(|| flow_like_types::anyhow!("From Pin ({}) not found in container", from_pin))?;

    let to_pin_ref = match &mut to_entity {
        NodeOrLayer::Node(node) => node.pins.get_mut(to_pin),
        NodeOrLayer::Layer(layer) => layer.pins.get_mut(to_pin),
    }
    .ok_or_else(|| flow_like_types::anyhow!("To Pin ({}) not found in container", to_pin))?;

    to_pin_ref.depends_on.remove(&from_pin_ref.id);
    from_pin_ref.connected_to.remove(&to_pin_ref.id);

    upsert_node_or_layer(board, from_entity);
    upsert_node_or_layer(board, to_entity);

    Ok(())
}
