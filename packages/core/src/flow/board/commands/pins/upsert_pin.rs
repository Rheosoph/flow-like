use canonical_json::ser::to_string as canonical_json_string;
use flow_like_types::{async_trait, create_id};
use schemars::JsonSchema;
use std::sync::Arc;

use crate::{
    flow::{
        board::{Board, commands::Command},
        pin::Pin,
        variable::{VariableType, infer_schema_from_json},
    },
    state::FlowLikeState,
};
use serde::{Deserialize, Serialize};

fn normalize_schema(schema: &str) -> Option<String> {
    serde_json::from_str::<serde_json::Value>(schema)
        .ok()
        .and_then(|value| canonical_json_string(&value).ok())
}

#[derive(Clone, Serialize, Deserialize, JsonSchema)]
pub struct UpsertPinCommand {
    pub node_id: String,
    pub pin: Pin,
    pub old_pin: Option<Pin>,
}

impl UpsertPinCommand {
    pub fn new(node_id: String, pin: Pin) -> Self {
        UpsertPinCommand {
            node_id,
            pin,
            old_pin: None,
        }
    }
}

#[async_trait]
impl Command for UpsertPinCommand {
    async fn execute(
        &mut self,
        board: &mut Board,
        _: Arc<FlowLikeState>,
    ) -> flow_like_types::Result<()> {
        if self.pin.data_type == VariableType::Struct
            && let Some(ref schema_str) = self.pin.schema
            && !schema_str.trim().is_empty()
            && let Ok(inferred) = infer_schema_from_json(schema_str)
        {
            self.pin.schema = Some(inferred);
        }

        if let Some(ref schema_str) = self.pin.schema
            && let Some(normalized) = normalize_schema(schema_str)
        {
            self.pin.schema = Some(normalized);
        }

        let pin_exists_on_node = match board.nodes.get(&self.node_id) {
            Some(node) => node.pins.contains_key(&self.pin.id),
            None => return Err(flow_like_types::anyhow!("Node not found".to_string())),
        };

        let pin_id_conflicts = !pin_exists_on_node && board.get_pin_by_id(&self.pin.id).is_some();

        let node = match board.nodes.get_mut(&self.node_id) {
            Some(node) => node,
            None => return Err(flow_like_types::anyhow!("Node not found".to_string())),
        };

        if pin_exists_on_node {
            self.old_pin = node.pins.insert(self.pin.id.clone(), self.pin.clone());
            return Ok(());
        }

        let mut pin = self.pin.clone();
        if pin_id_conflicts {
            pin.id = create_id();
        }

        let num_pins = node
            .pins
            .iter()
            .filter(|(_, v)| v.pin_type == pin.pin_type)
            .count();

        pin.index = num_pins as u16 + 1;

        node.pins.insert(pin.id.clone(), pin.clone());
        self.pin = pin;

        Ok(())
    }

    async fn undo(
        &mut self,
        board: &mut Board,
        _: Arc<FlowLikeState>,
    ) -> flow_like_types::Result<()> {
        let node = match board.nodes.get_mut(&self.node_id) {
            Some(node) => node,
            None => return Err(flow_like_types::anyhow!("Node not found".to_string())),
        };

        if let Some(old_pin) = self.old_pin.take() {
            node.pins.insert(old_pin.id.clone(), old_pin);
        } else {
            node.pins.remove(&self.pin.id);
        }

        Ok(())
    }
}
