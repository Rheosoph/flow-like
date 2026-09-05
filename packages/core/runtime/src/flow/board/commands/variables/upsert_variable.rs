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
    async fn validate(&self, board: &Board, _: Arc<FlowLikeState>) -> flow_like_types::Result<()> {
        let variables = if let Some(ref layer_id) = self.layer_id {
            &board
                .layers
                .get(layer_id)
                .ok_or_else(|| flow_like_types::anyhow!("Layer not found"))?
                .variables
        } else {
            &board.variables
        };

        if let Some(old_variable) = variables.get(&self.variable.id)
            && !old_variable.editable
        {
            return Err(flow_like_types::anyhow!("Variable is not editable"));
        }

        Ok(())
    }

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

        if let Some(old_variable) = variables.get(&self.variable.id)
            && !old_variable.editable
        {
            return Err(flow_like_types::anyhow!("Variable is not editable"));
        }

        // Board responses strip secret values, so a client that only edited the variable's
        // metadata sends `default_value: None`. That means "unchanged", not "clear"; clearing a
        // secret is an explicit empty value.
        if self.variable.secret
            && self.variable.default_value.is_none()
            && let Some(old_variable) = variables.get(&self.variable.id)
            && old_variable.secret
        {
            self.variable.default_value = old_variable.default_value.clone();
        }

        if let Some(old_variable) =
            variables.insert(self.variable.id.clone(), self.variable.clone())
        {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::flow::board::commands::Command;
    use crate::flow::pin::ValueType;
    use crate::flow::variable::{Variable, VariableType};
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
    async fn editing_a_secret_variables_metadata_keeps_its_stored_value() {
        let mut board = Board::new_detached(Some("b".into()), Path::default());
        let mut variable = Variable::new("token", VariableType::String, ValueType::Normal);
        variable.secret = true;
        variable.editable = true;
        variable.default_value = Some(flow_like_types::json::to_vec(&json!("s3cr3t")).unwrap());
        board
            .variables
            .insert(variable.id.clone(), variable.clone());

        // A web client holds the filtered variable (no value) and only edits its description.
        let mut incoming = variable.clone();
        incoming.default_value = None;
        incoming.description = Some("rotated monthly".into());
        UpsertVariableCommand::new(incoming)
            .execute(&mut board, state())
            .await
            .expect("upsert");

        let stored = &board.variables[&variable.id];
        assert_eq!(stored.description.as_deref(), Some("rotated monthly"));
        assert_eq!(stored.default_value, variable.default_value);
    }
}
