use flow_like_types::async_trait;
use schemars::JsonSchema;
use std::sync::Arc;

use crate::{
    flow::{
        board::{Board, commands::Command},
        variable::Variable,
    },
    state::FlowLikeState,
};
use serde::{Deserialize, Serialize};

#[derive(Clone, Serialize, Deserialize, JsonSchema)]
pub struct RemoveVariableCommand {
    pub variable: Variable,
    /// When set, operate on a layer's variables instead of board-level variables
    #[serde(default)]
    pub layer_id: Option<String>,
}

impl RemoveVariableCommand {
    pub fn new(variable: Variable) -> Self {
        RemoveVariableCommand {
            variable,
            layer_id: None,
        }
    }
}

#[async_trait]
impl Command for RemoveVariableCommand {
    async fn execute(
        &mut self,
        board: &mut Board,
        _: Arc<FlowLikeState>,
    ) -> flow_like_types::Result<()> {
        let variables = if let Some(ref layer_id) = self.layer_id {
            &mut board
                .layers
                .get_mut(layer_id)
                .ok_or_else(|| flow_like_types::anyhow!("Layer not found"))?
                .variables
        } else {
            &mut board.variables
        };

        let old_variable = variables.remove(&self.variable.id);

        if let Some(old_variable) = old_variable {
            if !old_variable.editable {
                variables.insert(old_variable.id.clone(), old_variable);
                return Err(flow_like_types::anyhow!("Variable is not editable"));
            }

            self.variable = old_variable;
        }

        Ok(())
    }

    async fn undo(
        &mut self,
        board: &mut Board,
        _: Arc<FlowLikeState>,
    ) -> flow_like_types::Result<()> {
        let variables = if let Some(ref layer_id) = self.layer_id {
            &mut board
                .layers
                .get_mut(layer_id)
                .ok_or_else(|| flow_like_types::anyhow!("Layer not found"))?
                .variables
        } else {
            &mut board.variables
        };

        variables.insert(self.variable.id.clone(), self.variable.clone());
        Ok(())
    }
}
