use crate::data::atlassian::provider::{ATLASSIAN_PROVIDER_ID, AtlassianProvider};
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic, NodeScores},
    pin::{PinOptions, ValueType},
    variable::VariableType,
};
use flow_like_types::{JsonSchema, Value, async_trait, json::json, reqwest};
use serde::{Deserialize, Serialize};

/// Confluence label
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ConfluenceLabel {
    pub id: String,
    pub name: String,
    pub prefix: String,
}

fn parse_label(value: &Value) -> Option<ConfluenceLabel> {
    let obj = value.as_object()?;

    let id = if let Some(id_str) = obj.get("id").and_then(|i| i.as_str()) {
        id_str.to_string()
    } else if let Some(id_num) = obj.get("id").and_then(|i| i.as_i64()) {
        id_num.to_string()
    } else {
        String::new()
    };

    Some(ConfluenceLabel {
        id,
        name: obj.get("name")?.as_str()?.to_string(),
        prefix: obj
            .get("prefix")
            .and_then(|p| p.as_str())
            .unwrap_or("global")
            .to_string(),
    })
}

/// Get labels for a page
#[crate::register_node]
#[derive(Default)]
pub struct GetLabelsNode {}

impl GetLabelsNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for GetLabelsNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_atlassian_confluence_get_labels",
            "Get Labels",
            "Get all labels for a Confluence page",
            "Data/Atlassian/Confluence",
        );
        node.add_icon("/flow/icons/confluence.svg");
        node.set_version(1);

        node.add_input_pin(
            "exec_in",
            "Exec In",
            "Execution input",
            VariableType::Execution,
        );
        node.add_output_pin(
            "exec_out",
            "Exec Out",
            "Execution output",
            VariableType::Execution,
        );

        node.add_input_pin(
            "provider",
            "Provider",
            "Atlassian provider",
            VariableType::Struct,
        )
        .set_schema::<AtlassianProvider>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_input_pin(
            "page_id",
            "Page ID",
            "The ID of the page to get labels for",
            VariableType::String,
        );

        node.add_output_pin("labels", "Labels", "List of labels", VariableType::Struct)
            .set_value_type(ValueType::Array)
            .set_schema::<ConfluenceLabel>()
            .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_output_pin("count", "Count", "Number of labels", VariableType::Integer);

        node.add_required_oauth_scopes(ATLASSIAN_PROVIDER_ID, vec!["read:confluence-content.all"]);
        node.set_scores(
            NodeScores::new()
                .set_privacy(7)
                .set_security(8)
                .set_performance(8)
                .set_governance(7)
                .set_reliability(8)
                .set_cost(9)
                .build(),
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let provider: AtlassianProvider = context.evaluate_pin("provider").await?;
        let page_id: String = context.evaluate_pin("page_id").await?;

        if page_id.is_empty() {
            return Err(flow_like_types::anyhow!("Page ID is required"));
        }

        let client = reqwest::Client::new();

        let labels = if provider.is_cloud {
            // Cloud v2 API
            let url = provider.confluence_api_url(&format!("/pages/{}/labels", page_id));

            let response = client
                .get(&url)
                .header("Authorization", provider.auth_header())
                .send()
                .await?;

            if !response.status().is_success() {
                let status = response.status();
                let error_text = response.text().await.unwrap_or_default();
                return Err(flow_like_types::anyhow!(
                    "Failed to get labels: {} - {}",
                    status,
                    error_text
                ));
            }

            let data: Value = response.json().await?;
            data["results"]
                .as_array()
                .unwrap_or(&vec![])
                .iter()
                .filter_map(parse_label)
                .collect::<Vec<_>>()
        } else {
            // Server v1 API
            let url = provider.confluence_rest_api_url(&format!("/content/{}/label", page_id));

            let response = client
                .get(&url)
                .header("Authorization", provider.auth_header())
                .send()
                .await?;

            if !response.status().is_success() {
                let status = response.status();
                let error_text = response.text().await.unwrap_or_default();
                return Err(flow_like_types::anyhow!(
                    "Failed to get labels: {} - {}",
                    status,
                    error_text
                ));
            }

            let data: Value = response.json().await?;
            data["results"]
                .as_array()
                .unwrap_or(&vec![])
                .iter()
                .filter_map(parse_label)
                .collect::<Vec<_>>()
        };

        let count = labels.len() as i64;

        context.set_pin_value("labels", json!(labels)).await?;
        context.set_pin_value("count", json!(count)).await?;

        Ok(())
    }
}

/// Add a label to a page
#[crate::register_node]
#[derive(Default)]
pub struct AddLabelNode {}

impl AddLabelNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for AddLabelNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_atlassian_confluence_add_label",
            "Add Label",
            "Add a label to a Confluence page",
            "Data/Atlassian/Confluence",
        );
        node.add_icon("/flow/icons/confluence.svg");
        node.set_version(1);

        node.add_input_pin(
            "exec_in",
            "Exec In",
            "Execution input",
            VariableType::Execution,
        );
        node.add_output_pin(
            "exec_out",
            "Exec Out",
            "Execution output",
            VariableType::Execution,
        );

        node.add_input_pin(
            "provider",
            "Provider",
            "Atlassian provider",
            VariableType::Struct,
        )
        .set_schema::<AtlassianProvider>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_input_pin(
            "page_id",
            "Page ID",
            "The ID of the page to add the label to",
            VariableType::String,
        );

        node.add_input_pin(
            "label",
            "Label",
            "The label name to add",
            VariableType::String,
        );

        node.add_output_pin(
            "success",
            "Success",
            "Whether the label was added successfully",
            VariableType::Boolean,
        );

        node.add_required_oauth_scopes(ATLASSIAN_PROVIDER_ID, vec!["write:confluence-content"]);
        node.set_scores(
            NodeScores::new()
                .set_privacy(6)
                .set_security(8)
                .set_performance(8)
                .set_governance(7)
                .set_reliability(8)
                .set_cost(9)
                .build(),
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let provider: AtlassianProvider = context.evaluate_pin("provider").await?;
        let page_id: String = context.evaluate_pin("page_id").await?;
        let label: String = context.evaluate_pin("label").await?;

        if page_id.is_empty() {
            return Err(flow_like_types::anyhow!("Page ID is required"));
        }
        if label.is_empty() {
            return Err(flow_like_types::anyhow!("Label is required"));
        }

        let client = reqwest::Client::new();

        let url = provider.confluence_rest_api_url(&format!("/content/{}/label", page_id));
        let body = json!([{
            "prefix": "global",
            "name": label
        }]);

        let response = client
            .post(&url)
            .header("Authorization", provider.auth_header())
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status();
            let error_text = response.text().await.unwrap_or_default();
            return Err(flow_like_types::anyhow!(
                "Failed to add label: {} - {}",
                status,
                error_text
            ));
        }

        context.set_pin_value("success", json!(true)).await?;

        Ok(())
    }
}

/// Remove a label from a page
#[crate::register_node]
#[derive(Default)]
pub struct RemoveLabelNode {}

impl RemoveLabelNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for RemoveLabelNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_atlassian_confluence_remove_label",
            "Remove Label",
            "Remove a label from a Confluence page",
            "Data/Atlassian/Confluence",
        );
        node.add_icon("/flow/icons/confluence.svg");
        node.set_version(1);

        node.add_input_pin(
            "exec_in",
            "Exec In",
            "Execution input",
            VariableType::Execution,
        );
        node.add_output_pin(
            "exec_out",
            "Exec Out",
            "Execution output",
            VariableType::Execution,
        );

        node.add_input_pin(
            "provider",
            "Provider",
            "Atlassian provider",
            VariableType::Struct,
        )
        .set_schema::<AtlassianProvider>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_input_pin(
            "page_id",
            "Page ID",
            "The ID of the page to remove the label from",
            VariableType::String,
        );

        node.add_input_pin(
            "label",
            "Label",
            "The label name to remove",
            VariableType::String,
        );

        node.add_output_pin(
            "success",
            "Success",
            "Whether the label was removed successfully",
            VariableType::Boolean,
        );

        node.add_required_oauth_scopes(ATLASSIAN_PROVIDER_ID, vec!["write:confluence-content"]);
        node.set_scores(
            NodeScores::new()
                .set_privacy(6)
                .set_security(8)
                .set_performance(8)
                .set_governance(7)
                .set_reliability(8)
                .set_cost(9)
                .build(),
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let provider: AtlassianProvider = context.evaluate_pin("provider").await?;
        let page_id: String = context.evaluate_pin("page_id").await?;
        let label: String = context.evaluate_pin("label").await?;

        if page_id.is_empty() {
            return Err(flow_like_types::anyhow!("Page ID is required"));
        }
        if label.is_empty() {
            return Err(flow_like_types::anyhow!("Label is required"));
        }

        let client = reqwest::Client::new();

        let url = provider.confluence_rest_api_url(&format!(
            "/content/{}/label/{}",
            page_id,
            urlencoding::encode(&label)
        ));

        let response = client
            .delete(&url)
            .header("Authorization", provider.auth_header())
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status();
            let error_text = response.text().await.unwrap_or_default();
            return Err(flow_like_types::anyhow!(
                "Failed to remove label: {} - {}",
                status,
                error_text
            ));
        }

        context.set_pin_value("success", json!(true)).await?;

        Ok(())
    }
}
