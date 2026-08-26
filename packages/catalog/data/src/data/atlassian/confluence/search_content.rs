use super::{ConfluenceContent, parse_confluence_content};
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
pub struct SearchConfluenceContentNode {}

impl SearchConfluenceContentNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for SearchConfluenceContentNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_atlassian_confluence_search",
            "Search Confluence",
            "Search Confluence content using CQL (Confluence Query Language) or text search",
            "Data/Atlassian/Confluence",
        );
        node.set_flowscript_name("confluence", "search");
        node.add_icon("/flow/icons/confluence.svg");

        node.add_input_pin(
            "exec_in",
            "Input",
            "Trigger the search",
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
            "cql",
            "CQL Query",
            "CQL query string (e.g., 'space = TEAM AND type = page AND text ~ \"search term\"')",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_input_pin(
            "text",
            "Text Search",
            "Simple text search (alternative to CQL)",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_input_pin(
            "space_key",
            "Space Key",
            "Limit search to a specific space (optional)",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_input_pin(
            "content_type",
            "Content Type",
            "Filter by content type",
            VariableType::String,
        )
        .set_default_value(Some(json!("all")))
        .set_options(
            PinOptions::new()
                .set_valid_values(vec![
                    "all".to_string(),
                    "page".to_string(),
                    "blogpost".to_string(),
                    "comment".to_string(),
                    "attachment".to_string(),
                ])
                .build(),
        );

        node.add_input_pin(
            "limit",
            "Limit",
            "Maximum number of results to return (1-100)",
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
            "Triggered when search completes successfully",
            VariableType::Execution,
        );

        node.add_output_pin(
            "error",
            "Error",
            "Triggered when an error occurs",
            VariableType::Execution,
        );

        node.add_output_pin(
            "results",
            "Results",
            "Array of search results",
            VariableType::Struct,
        )
        .set_value_type(ValueType::Array)
        .set_schema::<ConfluenceContent>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_output_pin(
            "total",
            "Total",
            "Total number of matching results",
            VariableType::Integer,
        );

        node.add_output_pin(
            "has_more",
            "Has More",
            "Whether there are more results available",
            VariableType::Boolean,
        );

        node.add_required_oauth_scopes(ATLASSIAN_PROVIDER_ID, vec!["search:confluence"]);
        node.set_scores(
            NodeScores::new()
                .set_privacy(6)
                .set_security(8)
                .set_performance(7)
                .set_governance(7)
                .set_reliability(8)
                .set_cost(7)
                .build(),
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        context.deactivate_exec_pin("error").await?;

        let provider: AtlassianProvider = context.evaluate_pin("provider").await?;
        let cql: String = context.evaluate_pin("cql").await?;
        let text: String = context.evaluate_pin("text").await?;
        let space_key: String = context.evaluate_pin("space_key").await?;
        let content_type: String = context.evaluate_pin("content_type").await?;
        let limit: i64 = context.evaluate_pin("limit").await?;
        let start: i64 = context.evaluate_pin("start").await?;

        // Build CQL query
        let final_cql = if !cql.is_empty() {
            cql
        } else {
            let mut parts = Vec::new();

            if !text.is_empty() {
                parts.push(format!("text ~ \"{}\"", text.replace('"', "\\\"")));
            }

            if !space_key.is_empty() {
                parts.push(format!("space = \"{}\"", space_key));
            }

            if content_type != "all" {
                parts.push(format!("type = \"{}\"", content_type));
            }

            if parts.is_empty() {
                "type = page ORDER BY lastmodified DESC".to_string()
            } else {
                parts.join(" AND ")
            }
        };

        let client = reqwest::Client::new();

        // Use the provider's search URL method which handles OAuth vs API token correctly
        let url = format!(
            "{}?cql={}&limit={}&start={}",
            provider.confluence_search_url(),
            urlencoding::encode(&final_cql),
            limit.clamp(1, 100),
            start.max(0)
        );

        context.log_message(&format!("Confluence Search URL: {}", url), LogLevel::Debug);
        context.log_message(
            &format!("Auth type: {}", provider.auth_type),
            LogLevel::Debug,
        );
        context.log_message(
            &format!("Searching Confluence with CQL: {}", final_cql),
            LogLevel::Debug,
        );

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

        context.log_message(
            &format!("Response status: {}", response.status()),
            LogLevel::Debug,
        );

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

        let total = data
            .get("totalSize")
            .or_else(|| data.get("size"))
            .and_then(|v| v.as_i64())
            .unwrap_or(0);

        let results_data = data.get("results").and_then(|v| v.as_array());

        let results: Vec<ConfluenceContent> = results_data
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| parse_confluence_content(v, &provider.base_url))
                    .collect()
            })
            .unwrap_or_default();

        let returned_count = results.len() as i64;
        let has_more = start + returned_count < total;

        context.log_message(
            &format!("Found {} results (total: {})", returned_count, total),
            LogLevel::Debug,
        );

        context.set_pin_value("results", json!(results)).await?;
        context.set_pin_value("total", json!(total)).await?;
        context.set_pin_value("has_more", json!(has_more)).await?;
        context.activate_exec_pin("exec_out").await?;

        Ok(())
    }
}
