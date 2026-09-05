use flow_like_types::async_trait;
use schemars::JsonSchema;
use std::sync::Arc;

use crate::{
    flow::board::{Board, Comment, commands::Command},
    state::FlowLikeState,
};
use serde::{Deserialize, Serialize};

#[derive(Clone, Serialize, Deserialize, JsonSchema)]
pub struct RemoveCommentCommand {
    pub comment: Comment,
}

impl RemoveCommentCommand {
    pub fn new(comment: Comment) -> Self {
        RemoveCommentCommand { comment }
    }
}

#[async_trait]
impl Command for RemoveCommentCommand {
    async fn execute(
        &mut self,
        board: &mut Board,
        _: Arc<FlowLikeState>,
    ) -> flow_like_types::Result<()> {
        board.comments.remove(&self.comment.id);
        Ok(())
    }

    async fn undo(
        &mut self,
        board: &mut Board,
        _: Arc<FlowLikeState>,
    ) -> flow_like_types::Result<()> {
        board
            .comments
            .insert(self.comment.id.clone(), self.comment.clone());
        Ok(())
    }
}
