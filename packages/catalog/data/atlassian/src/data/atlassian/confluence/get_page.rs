use super::{ConfluencePage, parse_confluence_page};
use crate::data::atlassian::provider::{ATLASSIAN_PROVIDER_ID, AtlassianProvider};
use flow_like::flow::{
    execution::{LogLevel, context::ExecutionContext},
    node::{Node, NodeLogic, NodeScores},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_types::{Value, async_trait, json::json, reqwest};

#[crate::register_node]
#[derive(Default)]
pub struct GetConfluencePageNode {}

impl GetConfluencePageNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for GetConfluencePageNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_atlassian_confluence_get_page",
            "Get Confluence Page",
            "Get a Confluence page by its ID",
            "Data/Atlassian/Confluence",
        );
        node.set_flowscript_name("confluence", "getPage");
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
            "page_id",
            "Page ID",
            "The page ID to retrieve",
            VariableType::String,
        );

        node.add_input_pin(
            "include_body",
            "Include Body",
            "Whether to include the page body content",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(true)));

        node.add_input_pin(
            "body_format",
            "Body Format",
            "Format for the body content",
            VariableType::String,
        )
        .set_default_value(Some(json!("storage")))
        .set_options(
            PinOptions::new()
                .set_valid_values(vec![
                    "storage".to_string(),
                    "view".to_string(),
                    "export_view".to_string(),
                ])
                .build(),
        );

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

        node.add_output_pin("page", "Page", "The Confluence page", VariableType::Struct)
            .set_schema::<ConfluencePage>()
            .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_output_pin(
            "body_content",
            "Body Content",
            "The page body as plain text/HTML",
            VariableType::String,
        );

        node.add_required_oauth_scopes(ATLASSIAN_PROVIDER_ID, vec!["read:confluence-content.all"]);
        node.set_scores(
            NodeScores::new()
                .set_privacy(6)
                .set_security(8)
                .set_performance(8)
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
        let page_id: String = context.evaluate_pin("page_id").await?;
        let include_body: bool = context.evaluate_pin("include_body").await?;
        let body_format: String = context.evaluate_pin("body_format").await?;

        if page_id.is_empty() {
            context.log_message("Page ID is required", LogLevel::Error);
            context.activate_exec_pin("error").await?;
            return Ok(());
        }

        let client = reqwest::Client::new();

        // Build URL with expansions
        let body_expand = format!("body.{}", body_format);
        let mut expand_parts = vec![
            "version".to_string(),
            "space".to_string(),
            "history".to_string(),
            "ancestors".to_string(),
        ];
        if include_body {
            expand_parts.push(body_expand.clone());
        }

        let url = provider.confluence_rest_api_url(&format!(
            "/content/{}?expand={}",
            page_id,
            expand_parts.join(",")
        ));

        context.log_message(
            &format!("Fetching Confluence page: {}", page_id),
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

        let page = match parse_confluence_page(&data, &provider.base_url) {
            Some(p) => p,
            None => {
                context.log_message("Failed to parse page from response", LogLevel::Error);
                context.activate_exec_pin("error").await?;
                return Ok(());
            }
        };

        // Extract body content
        let body_content = data
            .get("body")
            .and_then(|b| b.get(&body_format))
            .and_then(|f| f.get("value"))
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string();

        context.set_pin_value("page", json!(page)).await?;
        context
            .set_pin_value("body_content", json!(body_content))
            .await?;
        context.activate_exec_pin("exec_out").await?;

        Ok(())
    }
}
