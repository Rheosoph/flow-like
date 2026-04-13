use std::sync::Arc;

use flow_like_types::async_trait;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{
    flow::node::{Node, NodeLogic},
    state::FlowLikeState,
};

use super::Board;

pub mod comments;
pub mod layer;
pub mod nodes;
pub mod pins;
pub mod variables;

macro_rules! impl_command_methods {
    ($($variant:ident),*) => {
        impl GenericCommand {
            pub fn to_dyn(&self) -> Arc<flow_like_types::sync::Mutex<dyn Command>> {
                match self {
                    $(GenericCommand::$variant(cmd) => Arc::new(flow_like_types::sync::Mutex::new(cmd.clone())),)*
                }
            }

            pub async fn execute(
                &mut self,
                board: &mut Board,
                state: Arc<FlowLikeState>,
            ) -> flow_like_types::Result<()> {
                match self {
                    $(GenericCommand::$variant(cmd) => cmd.execute(board, state).await,)*
                }
            }

            pub async fn validate(
                &self,
                board: &Board,
                state: Arc<FlowLikeState>,
            ) -> flow_like_types::Result<()> {
                match self {
                    $(GenericCommand::$variant(cmd) => cmd.validate(board, state).await,)*
                }
            }

            pub async fn undo(
                &mut self,
                board: &mut Board,
                state: Arc<FlowLikeState>,
            ) -> flow_like_types::Result<()> {
                match self {
                    $(GenericCommand::$variant(cmd) => cmd.undo(board, state).await,)*
                }
            }
        }
    };
}

impl_command_methods!(
    RemoveComment,
    UpsertComment,
    AddNode,
    CopyPaste,
    MoveNode,
    RemoveNode,
    UpdateNode,
    DisconnectPin,
    ConnectPin,
    UpsertPin,
    RemoveVariable,
    UpsertVariable,
    UpsertLayer,
    RemoveLayer
);

#[async_trait]
pub trait Command: Send + Sync {
    async fn validate(
        &self,
        _board: &Board,
        _state: Arc<FlowLikeState>,
    ) -> flow_like_types::Result<()> {
        Ok(())
    }

    async fn execute(
        &mut self,
        board: &mut Board,
        state: Arc<FlowLikeState>,
    ) -> flow_like_types::Result<()>;
    async fn undo(
        &mut self,
        board: &mut Board,
        state: Arc<FlowLikeState>,
    ) -> flow_like_types::Result<()>;

    async fn node_to_logic(
        &self,
        node: &Node,
        state: Arc<FlowLikeState>,
    ) -> flow_like_types::Result<Arc<dyn NodeLogic>> {
        let node_registry = state.node_registry().clone();

        let registry_guard = node_registry.read().await;

        registry_guard.instantiate(node)
    }
}

#[derive(Clone, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "command_type")]
pub enum GenericCommand {
    RemoveComment(comments::remove_comment::RemoveCommentCommand),
    UpsertComment(comments::upsert_comment::UpsertCommentCommand),
    AddNode(nodes::add_node::AddNodeCommand),
    CopyPaste(nodes::copy_paste::CopyPasteCommand),
    MoveNode(nodes::move_node::MoveNodeCommand),
    RemoveNode(nodes::remove_node::RemoveNodeCommand),
    UpdateNode(nodes::update_node::UpdateNodeCommand),
    DisconnectPin(pins::disconnect_pins::DisconnectPinsCommand),
    ConnectPin(pins::connect_pins::ConnectPinsCommand),
    UpsertPin(pins::upsert_pin::UpsertPinCommand),
    RemoveVariable(variables::remove_variable::RemoveVariableCommand),
    UpsertVariable(variables::upsert_variable::UpsertVariableCommand),
    UpsertLayer(layer::upsert_layer::UpsertLayerCommand),
    RemoveLayer(layer::remove_layer::RemoveLayerCommand),
}
