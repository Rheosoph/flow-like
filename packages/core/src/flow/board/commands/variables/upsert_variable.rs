use canonical_json::ser::to_string as canonical_json_string;
use flow_like_types::async_trait;
use schemars::JsonSchema;
use std::sync::Arc;

use crate::{
    flow::{
        board::{Board, commands::Command},
        variable::{Variable, VariableType, infer_schema_from_json},
    },
    state::FlowLikeState,
};
use serde::{Deserialize, Serialize};

/// Normalizes a JSON string to canonical format (sorted keys, no extra whitespace)
fn normalize_schema(schema: &str) -> Option<String> {
    serde_json::from_str::<serde_json::Value>(schema)
        .ok()
        .and_then(|v| canonical_json_string(&v).ok())
}

#[derive(Clone, Serialize, Deserialize, JsonSchema)]
pub struct UpsertVariableCommand {
    pub variable: Variable,
    pub old_variable: Option<Variable>,
    /// When set, operate on a layer's variables instead of board-level variables
    #[serde(default)]
    pub layer_id: Option<String>,
}

impl UpsertVariableCommand {
    pub fn new(variable: Variable) -> Self {
        UpsertVariableCommand {
            variable,
            old_variable: None,
            layer_id: None,
        }
    }
}

#[async_trait]
impl Command for UpsertVariableCommand {
    async fn execute(
        &mut self,
        board: &mut Board,
        _: Arc<FlowLikeState>,
    ) -> flow_like_types::Result<()> {
        // If the variable is a Struct type and has a schema that looks like example JSON,
        // infer the proper JSON Schema from it. For other types, preserve the schema as-is.
        if self.variable.data_type == VariableType::Struct
            && let Some(ref schema_str) = self.variable.schema
            && !schema_str.trim().is_empty()
            && let Ok(inferred) = infer_schema_from_json(schema_str)
        {
            self.variable.schema = Some(inferred);
        }
        // If inference fails, keep the original schema
        // For non-Struct types, keep schema as-is (don't set to None)

        // Normalize schema to canonical JSON format for consistent hashing
        if let Some(ref schema_str) = self.variable.schema
            && let Some(normalized) = normalize_schema(schema_str)
        {
            self.variable.schema = Some(normalized);
        }

        let variables = if let Some(ref layer_id) = self.layer_id {
            &mut board
                .layers
                .get_mut(layer_id)
                .ok_or_else(|| flow_like_types::anyhow!("Layer not found"))?
                .variables
        } else {
            &mut board.variables
        };

        if let Some(old_variable) = variables.insert(self.variable.id.clone(), self.variable.clone())
        {
            if !old_variable.editable {
                variables.insert(old_variable.id.clone(), old_variable);
                return Err(flow_like_types::anyhow!("Variable is not editable"));
            }

            self.old_variable = Some(old_variable);
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

        variables.remove(&self.variable.id);
        if let Some(old_variable) = self.old_variable.take() {
            variables.insert(old_variable.id.clone(), old_variable);
        }
        Ok(())
    }
}
