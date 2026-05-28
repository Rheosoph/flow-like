use super::{ConfluenceSpace, parse_confluence_space};
use crate::data::atlassian::provider::{ATLASSIAN_PROVIDER_ID, AtlassianProvider};
use flow_like::flow::{
    execution::{LogLevel, context::ExecutionContext},
    node::{Node, NodeLogic, NodeScores},
    pin::{PinOptions, ValueType},
    variable::VariableType,
};
use flow_like_types::{Value, async_trait, json::json, reqwest};

#[crate::register_node]
#[derive(Default)]
pub struct ListConfluenceSpacesNode {}

impl ListConfluenceSpacesNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for ListConfluenceSpacesNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_atlassian_confluence_list_spaces",
            "List Confluence Spaces",
            "List all accessible Confluence spaces",
            "Data/Atlassian/Confluence",
        );
        node.add_icon("/flow/icons/confluence.svg");
        node.set_version(1);

        node.add_input_pin(
            "exec_in",
            "Input",
            "Trigger the request",
            VariableType::Execution,
        );

        node.add_input_pin(
            "provider",
            "Provider",
            "Atlassian provider (from Atlassian node)",
            VariableType::Struct,
        )
        .set_schema::<AtlassianProvider>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_input_pin(
            "space_type",
            "Space Type",
            "Filter by space type",
            VariableType::String,
        )
        .set_default_value(Some(json!("all")))
        .set_options(
            PinOptions::new()
                .set_valid_values(vec![
                    "all".to_string(),
                    "global".to_string(),
                    "personal".to_string(),
                ])
                .build(),
        );

        node.add_input_pin(
            "status",
            "Status",
            "Filter by space status",
            VariableType::String,
        )
        .set_default_value(Some(json!("current")))
        .set_options(
            PinOptions::new()
                .set_valid_values(vec!["current".to_string(), "archived".to_string()])
                .build(),
        );

        node.add_input_pin(
            "limit",
            "Limit",
            "Maximum number of spaces to return (1-100)",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(25)));

        node.add_input_pin(
            "start",
            "Start",
            "Index of the first result to return (for pagination)",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(0)));

        node.add_output_pin(
            "exec_out",
            "Success",
            "Triggered when request completes successfully",
            VariableType::Execution,
        );

        node.add_output_pin(
            "error",
            "Error",
            "Triggered when an error occurs",
            VariableType::Execution,
        );

        node.add_output_pin(
            "spaces",
            "Spaces",
            "Array of Confluence spaces",
            VariableType::Struct,
        )
        .set_value_type(ValueType::Array)
        .set_schema::<ConfluenceSpace>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_output_pin(
            "count",
            "Count",
            "Number of spaces returned",
            VariableType::Integer,
        );

        node.add_required_oauth_scopes(
            ATLASSIAN_PROVIDER_ID,
            vec!["read:confluence-space.summary"],
        );
        node.set_scores(
            NodeScores::new()
                .set_privacy(6)
                .set_security(8)
                .set_performance(7)
                .set_governance(7)
                .set_reliability(8)
                .set_cost(8)
                .build(),
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        context.deactivate_exec_pin("error").await?;

        let provider: AtlassianProvider = context.evaluate_pin("provider").await?;
        let space_type: String = context.evaluate_pin("space_type").await?;
        let status: String = context.evaluate_pin("status").await?;
        let limit: i64 = context.evaluate_pin("limit").await?;
        let start: i64 = context.evaluate_pin("start").await?;

        let client = reqwest::Client::new();

        // Build URL with query parameters
        let mut params = vec![
            format!("limit={}", limit.clamp(1, 100)),
            format!("start={}", start.max(0)),
            format!("status={}", status),
            "expand=description".to_string(),
        ];

        if space_type != "all" {
            params.push(format!("type={}", space_type));
        }

        let url = provider.confluence_rest_api_url(&format!("/space?{}", params.join("&")));

        context.log_message("Fetching Confluence spaces", LogLevel::Debug);

        let response = client
            .get(&url)
            .header("Authorization", provider.auth_header())
            .header("Accept", "application/json")
            .send()
            .await;

        let response = match response {
            Ok(r) => r,
            Err(e) => {
                context.log_message(&format!("Request failed: {}", e), LogLevel::Error);
                context.activate_exec_pin("error").await?;
                return Ok(());
            }
        };

        if !response.status().is_success() {
            let status = response.status();
            let error_text = response.text().await.unwrap_or_default();
            context.log_message(
                &format!("Confluence API error {}: {}", status, error_text),
                LogLevel::Error,
            );
            context.activate_exec_pin("error").await?;
            return Ok(());
        }

        let data: Value = match response.json().await {
            Ok(d) => d,
            Err(e) => {
                context.log_message(&format!("Failed to parse response: {}", e), LogLevel::Error);
                context.activate_exec_pin("error").await?;
                return Ok(());
            }
        };

        let spaces_data = data.get("results").and_then(|v| v.as_array());

        let spaces: Vec<ConfluenceSpace> = spaces_data
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| parse_confluence_space(v, &provider.base_url))
                    .collect()
            })
            .unwrap_or_default();

        let count = spaces.len() as i64;

        context.log_message(&format!("Found {} spaces", count), LogLevel::Debug);

        context.set_pin_value("spaces", json!(spaces)).await?;
        context.set_pin_value("count", json!(count)).await?;
        context.activate_exec_pin("exec_out").await?;

        Ok(())
    }
}
