use std::{collections::BTreeMap, sync::Arc};

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
    RemoveLayer,
    MoveToLayer
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
    MoveToLayer(layer::move_to_layer::MoveToLayerCommand),
}

impl GenericCommand {
    /// Record what this command wrote directly, so `node_updates` can re-evaluate only what the
    /// edit could have reached instead of the whole board.
    ///
    /// Only direct writes belong here — propagation along wires, references and variables is the
    /// sweep's job. The match is exhaustive on purpose: a new command variant must be classified
    /// here rather than silently defaulting to "changed nothing", which would leave the nodes it
    /// touched holding stale derivations.
    pub fn touched(&self, touched: &mut super::dirty::Touched) {
        match self {
            // Comments carry no derivation, so nothing re-evaluates on their account.
            GenericCommand::RemoveComment(_) | GenericCommand::UpsertComment(_) => {}
            GenericCommand::AddNode(command) => {
                touched.nodes.insert(command.node.id.clone());
            }
            GenericCommand::UpdateNode(command) => {
                touched.nodes.insert(command.node.id.clone());
            }
            // Coordinates feed no derivation, but a move also reparents between layers.
            GenericCommand::MoveNode(command) => {
                touched.nodes.insert(command.node_id.clone());
            }
            GenericCommand::RemoveNode(command) => {
                touched.nodes.insert(command.node.id.clone());
                touched
                    .nodes
                    .extend(command.connected_nodes.iter().map(|node| node.id.clone()));
            }
            GenericCommand::CopyPaste(command) => {
                touched
                    .nodes
                    .extend(command.new_nodes.iter().map(|node| node.id.clone()));
                touched
                    .layers
                    .extend(command.new_layers.iter().map(|layer| layer.id.clone()));
            }
            GenericCommand::ConnectPin(command) => {
                touched.nodes.insert(command.from_node.clone());
                touched.nodes.insert(command.to_node.clone());
            }
            GenericCommand::DisconnectPin(command) => {
                touched.nodes.insert(command.from_node.clone());
                touched.nodes.insert(command.to_node.clone());
            }
            GenericCommand::UpsertPin(command) => {
                touched.nodes.insert(command.node_id.clone());
            }
            GenericCommand::UpsertVariable(command) => {
                touched.variables.insert(command.variable.id.clone());
            }
            GenericCommand::RemoveVariable(command) => {
                touched.variables.insert(command.variable.id.clone());
            }
            GenericCommand::UpsertLayer(command) => {
                touched.layers.insert(command.layer.id.clone());
                touched.nodes.extend(command.node_ids.iter().cloned());
                touched.nodes.extend(command.layer.nodes.keys().cloned());
            }
            GenericCommand::RemoveLayer(command) => {
                touched.layers.insert(command.layer.id.clone());
                touched.layers.extend(command.child_layers.iter().cloned());
                touched.nodes.extend(command.layer_nodes.iter().cloned());
                touched
                    .nodes
                    .extend(command.nodes.iter().map(|node| node.id.clone()));
            }
            // `previous` mixes node, comment and layer ids with no per-entry type tag, so a
            // moved id is recorded in both sets — `Touched::seed` only acts on the set that
            // matches what the id actually names and no-ops on the other.
            GenericCommand::MoveToLayer(command) => {
                touched.nodes.extend(command.previous.keys().cloned());
                touched.layers.extend(command.previous.keys().cloned());
                if let Some(target) = &command.target {
                    touched.layers.insert(target.clone());
                }
            }
        }
    }
}

/// Stable digest for a typed command batch. Commands contain nested `HashMap`s, so serializing the
/// structs directly would make retry identities depend on randomized map iteration order. Routing
/// every backend through this helper keeps idempotency semantics provider-neutral.
pub fn canonical_commands_digest(commands: &[GenericCommand]) -> flow_like_types::Result<String> {
    let value = canonicalize_json(flow_like_types::json::to_value(commands)?);
    let encoded = canonical_json::ser::to_string(&value)?;
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"flow-like.command-batch/v1\0");
    hasher.update(encoded.as_bytes());
    Ok(hasher.finalize().to_hex().to_string())
}

pub(crate) fn canonicalize_json(value: flow_like_types::Value) -> flow_like_types::Value {
    use flow_like_types::Value;

    match value {
        Value::Array(values) => Value::Array(values.into_iter().map(canonicalize_json).collect()),
        Value::Object(values) => {
            let sorted = values
                .into_iter()
                .map(|(key, value)| (key, canonicalize_json(value)))
                .collect::<BTreeMap<_, _>>();
            Value::Object(sorted.into_iter().collect())
        }
        value => value,
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;
    use crate::flow::board::commands::nodes::copy_paste::CopyPasteCommand;

    fn copy_paste_with_refs(entries: &[(&str, &str)]) -> GenericCommand {
        let mut command =
            CopyPasteCommand::new(Vec::new(), Vec::new(), Vec::new(), (0.0, 0.0, 0.0));
        command.original_refs = entries
            .iter()
            .map(|(key, value)| ((*key).to_string(), (*value).to_string()))
            .collect::<HashMap<_, _>>();
        GenericCommand::CopyPaste(command)
    }

    #[test]
    fn canonical_command_digest_ignores_hash_map_iteration_order() {
        let left = vec![copy_paste_with_refs(&[("a", "1"), ("b", "2"), ("c", "3")])];
        let right = vec![copy_paste_with_refs(&[("c", "3"), ("b", "2"), ("a", "1")])];

        assert_eq!(
            canonical_commands_digest(&left).expect("left digest"),
            canonical_commands_digest(&right).expect("right digest")
        );
    }

    #[test]
    fn canonical_command_digest_changes_with_payload() {
        let left = vec![copy_paste_with_refs(&[("a", "1")])];
        let right = vec![copy_paste_with_refs(&[("a", "2")])];

        assert_ne!(
            canonical_commands_digest(&left).expect("left digest"),
            canonical_commands_digest(&right).expect("right digest")
        );
    }
}
