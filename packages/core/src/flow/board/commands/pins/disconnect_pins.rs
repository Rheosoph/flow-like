use flow_like_types::async_trait;
use schemars::JsonSchema;
use std::sync::Arc;

use crate::{
    flow::board::{Board, commands::Command},
    state::FlowLikeState,
};
use serde::{Deserialize, Serialize};

use super::connect_pins::{connect_pins, disconnect_pins};

#[derive(Clone, Serialize, Deserialize, JsonSchema)]
pub struct DisconnectPinsCommand {
    pub from_pin: String,
    pub to_pin: String,
    pub from_node: String,
    pub to_node: String,
}

impl DisconnectPinsCommand {
    pub fn new(from_node: String, to_node: String, from_pin: String, to_pin: String) -> Self {
        DisconnectPinsCommand {
            from_pin,
            to_pin,
            from_node,
            to_node,
        }
    }
}

#[async_trait]
impl Command for DisconnectPinsCommand {
    async fn execute(
        &mut self,
        board: &mut Board,
        _state: Arc<FlowLikeState>,
    ) -> flow_like_types::Result<()> {
        disconnect_pins(
            board,
            &self.from_node,
            &self.from_pin,
            &self.to_node,
            &self.to_pin,
        )
    }

    async fn undo(
        &mut self,
        board: &mut Board,
        _state: Arc<FlowLikeState>,
    ) -> flow_like_types::Result<()> {
        connect_pins(
            board,
            &self.from_node,
            &self.from_pin,
            &self.to_node,
            &self.to_pin,
        )
    }
}
