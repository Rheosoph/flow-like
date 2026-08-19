use flow_like_types::async_trait;
use schemars::JsonSchema;
use std::sync::Arc;

use crate::{
    flow::{
        board::{Board, commands::Command},
        node::Node,
    },
    state::FlowLikeState,
};
use serde::{Deserialize, Serialize};

#[derive(Clone, Serialize, Deserialize, JsonSchema)]
pub struct UpdateNodeCommand {
    pub old_node: Option<Node>,
    pub node: Node,
}

impl UpdateNodeCommand {
    pub fn new(node: Node) -> Self {
        UpdateNodeCommand {
            node,
            old_node: None,
        }
    }
}

#[async_trait]
impl Command for UpdateNodeCommand {
    async fn execute(
        &mut self,
        board: &mut Board,
        _state: Arc<FlowLikeState>,
    ) -> flow_like_types::Result<()> {
        // Validate and deduplicate fn_refs - never trust the frontend!
        if let Some(fn_refs) = &mut self.node.fn_refs {
            super::validate_and_deduplicate_fn_refs(fn_refs, board);
        }

        // Board responses strip sensitive literals, so a client round-tripping the node cannot
        // send them back. Keep what the board holds instead of nulling it.
        if let Some(existing) = board.nodes.get(&self.node.id) {
            for (id, pin) in self.node.pins.iter_mut() {
                pin.keep_sensitive_value_from(existing.pins.get(id));
            }
        }

        self.old_node = board.nodes.insert(self.node.id.clone(), self.node.clone());
        Ok(())
    }

    async fn undo(
        &mut self,
        board: &mut Board,
        _state: Arc<FlowLikeState>,
    ) -> flow_like_types::Result<()> {
        if let Some(old_node) = self.old_node.take() {
            board.nodes.insert(old_node.id.clone(), old_node.clone());
        } else {
            board.nodes.remove(&self.node.id);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::flow::board::commands::Command;
    use crate::flow::variable::VariableType;
    use crate::state::{FlowLikeConfig, FlowLikeState};
    use crate::utils::http::HTTPClient;
    use flow_like_storage::Path;
    use flow_like_types::json::json;

    fn state() -> Arc<FlowLikeState> {
        Arc::new(FlowLikeState::new(
            FlowLikeConfig::new(),
            HTTPClient::new_without_refetch(),
        ))
    }

    #[flow_like_types::tokio::test]
    async fn round_tripping_a_filtered_node_keeps_the_stored_sensitive_literal() {
        let mut board = Board::new_detached(Some("b".into()), Path::default());
        let mut node = crate::flow::node::Node::new("demo", "Demo", "", "Test");
        node.add_input_pin("api_key", "API key", "", VariableType::String)
            .set_options(
                crate::flow::pin::PinOptions::new()
                    .set_sensitive(true)
                    .build(),
            )
            .set_default_value(Some(json!("s3cr3t")));
        node.add_input_pin("plain", "Plain", "", VariableType::String)
            .set_default_value(Some(json!("visible")));
        board.nodes.insert(node.id.clone(), node.clone());

        // What a client holds after `filter_board_secrets`: the secret is gone, and it renames
        // the node, sending everything back.
        let mut incoming = node.clone();
        incoming.friendly_name = "Renamed".into();
        for pin in incoming.pins.values_mut() {
            if pin.is_sensitive() {
                pin.default_value = None;
            } else {
                pin.set_default_value(Some(json!("edited")));
            }
        }

        UpdateNodeCommand::new(incoming)
            .execute(&mut board, state())
            .await
            .expect("update");

        let stored = &board.nodes[&node.id];
        assert_eq!(stored.friendly_name, "Renamed");
        let api_key = stored.get_pin_by_name("api_key").unwrap();
        assert_eq!(
            api_key.default_value,
            Some(flow_like_types::json::to_vec(&json!("s3cr3t")).unwrap()),
            "None on a sensitive pin means unchanged"
        );
        let plain = stored.get_pin_by_name("plain").unwrap();
        assert_eq!(
            plain.default_value,
            Some(flow_like_types::json::to_vec(&json!("edited")).unwrap()),
            "ordinary pins are replaced as before"
        );
    }

    #[flow_like_types::tokio::test]
    async fn an_explicit_empty_value_still_clears_a_sensitive_literal() {
        let mut board = Board::new_detached(Some("b".into()), Path::default());
        let mut node = crate::flow::node::Node::new("demo", "Demo", "", "Test");
        node.add_input_pin("api_key", "API key", "", VariableType::String)
            .set_options(
                crate::flow::pin::PinOptions::new()
                    .set_sensitive(true)
                    .build(),
            )
            .set_default_value(Some(json!("s3cr3t")));
        board.nodes.insert(node.id.clone(), node.clone());

        let mut incoming = node.clone();
        for pin in incoming.pins.values_mut() {
            pin.set_default_value(Some(json!("")));
        }
        UpdateNodeCommand::new(incoming)
            .execute(&mut board, state())
            .await
            .expect("update");

        let api_key = board.nodes[&node.id].get_pin_by_name("api_key").unwrap();
        assert_eq!(
            api_key.default_value,
            Some(flow_like_types::json::to_vec(&json!("")).unwrap())
        );
    }
}
