use crate::data::linkedin::provider::{LINKEDIN_PROVIDER_ID, LinkedInProvider};
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic, NodeScores},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_types::{JsonSchema, Value, async_trait, json::json, reqwest};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct LinkedInShareResponse {
    pub id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct UgcPostRequest {
    author: String,
    lifecycle_state: String,
    specific_content: SpecificContent,
    visibility: Visibility,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SpecificContent {
    #[serde(rename = "com.linkedin.ugc.ShareContent")]
    share_content: ShareContent,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ShareContent {
    share_commentary: ShareCommentary,
    share_media_category: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ShareCommentary {
    text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Visibility {
    #[serde(rename = "com.linkedin.ugc.MemberNetworkVisibility")]
    member_network_visibility: String,
}

#[crate::register_node]
#[derive(Default)]
pub struct ShareTextPostNode {}

impl ShareTextPostNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for ShareTextPostNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_linkedin_share_text",
            "Share Text Post",
            "Share a text post on LinkedIn",
            "Data/LinkedIn",
        );
        node.set_flowscript_name("linkedin", "shareText");
        node.add_icon("/flow/icons/linkedin.svg");

        node.add_input_pin(
            "exec_in",
            "Exec In",
            "Execution input",
            VariableType::Execution,
        );
        node.add_output_pin(
            "exec_success",
            "Success",
            "Executed when post is shared successfully",
            VariableType::Execution,
        );
        node.add_output_pin(
            "exec_error",
            "Error",
            "Executed when post sharing fails",
            VariableType::Execution,
        );

        node.add_input_pin(
            "provider",
            "Provider",
            "LinkedIn provider",
            VariableType::Struct,
        )
        .set_schema::<LinkedInProvider>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_input_pin(
            "author_id",
            "Author ID",
            "LinkedIn user ID (sub from Get Me node). Format: urn:li:person:{sub}",
            VariableType::String,
        );

        node.add_input_pin(
            "text",
            "Text",
            "The text content of your post",
            VariableType::String,
        );

        node.add_input_pin(
            "visibility",
            "Visibility",
            "Who can see this post: PUBLIC, CONNECTIONS",
            VariableType::String,
        )
        .set_default_value(Some(json!("PUBLIC")))
        .set_options(
            PinOptions::new()
                .set_valid_values(vec!["PUBLIC".to_string(), "CONNECTIONS".to_string()])
                .build(),
        );

        node.add_output_pin(
            "post_id",
            "Post ID",
            "The ID of the created post",
            VariableType::String,
        );

        node.add_output_pin(
            "error_message",
            "Error Message",
            "Error message if post sharing fails",
            VariableType::String,
        );

        node.add_required_oauth_scopes(LINKEDIN_PROVIDER_ID, vec!["w_member_social"]);
        node.set_scores(
            NodeScores::new()
                .set_privacy(5)
                .set_security(8)
                .set_performance(8)
                .set_governance(6)
                .set_reliability(8)
                .set_cost(10)
                .build(),
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_success").await?;
        context.deactivate_exec_pin("exec_error").await?;

        let provider: LinkedInProvider = context.evaluate_pin("provider").await?;
        let author_id: String = context.evaluate_pin("author_id").await?;
        let text: String = context.evaluate_pin("text").await?;
        let visibility: String = context.evaluate_pin("visibility").await?;

        if text.is_empty() {
            context
                .set_pin_value("error_message", json!("Post text cannot be empty"))
                .await?;
            context.activate_exec_pin("exec_error").await?;
            return Ok(());
        }

        let author_urn = if author_id.starts_with("urn:li:person:") {
            author_id
        } else {
            format!("urn:li:person:{}", author_id)
        };

        let request = UgcPostRequest {
            author: author_urn,
            lifecycle_state: "PUBLISHED".to_string(),
            specific_content: SpecificContent {
                share_content: ShareContent {
                    share_commentary: ShareCommentary { text },
                    share_media_category: "NONE".to_string(),
                },
            },
            visibility: Visibility {
                member_network_visibility: visibility,
            },
        };

        let client = reqwest::Client::new();
        let url = "https://api.linkedin.com/v2/ugcPosts";

        let response = client
            .post(url)
            .header("Authorization", provider.auth_header())
            .header("Content-Type", "application/json")
            .header("X-Restli-Protocol-Version", "2.0.0")
            .json(&request)
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status();
            let error_text = response.text().await.unwrap_or_default();
            context
                .set_pin_value(
                    "error_message",
                    json!(format!("{} - {}", status, error_text)),
                )
                .await?;
            context.activate_exec_pin("exec_error").await?;
            return Ok(());
        }

        let data: Value = response.json().await?;
        let post_id = data
            .get("id")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        context.set_pin_value("post_id", json!(post_id)).await?;
        context.set_pin_value("error_message", json!("")).await?;
        context.activate_exec_pin("exec_success").await?;

        Ok(())
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct ShareArticleNode {}

impl ShareArticleNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for ShareArticleNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_linkedin_share_article",
            "Share Article",
            "Share an article/link on LinkedIn with optional title and description",
            "Data/LinkedIn",
        );
        node.set_flowscript_name("linkedin", "shareArticle");
        node.add_icon("/flow/icons/linkedin.svg");

        node.add_input_pin(
            "exec_in",
            "Exec In",
            "Execution input",
            VariableType::Execution,
        );
        node.add_output_pin(
            "exec_success",
            "Success",
            "Executed when article is shared successfully",
            VariableType::Execution,
        );
        node.add_output_pin(
            "exec_error",
            "Error",
            "Executed when article sharing fails",
            VariableType::Execution,
        );

        node.add_input_pin(
            "provider",
            "Provider",
            "LinkedIn provider",
            VariableType::Struct,
        )
        .set_schema::<LinkedInProvider>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_input_pin(
            "author_id",
            "Author ID",
            "LinkedIn user ID (sub from Get Me node). Format: urn:li:person:{sub}",
            VariableType::String,
        );

        node.add_input_pin(
            "text",
            "Text",
            "Commentary text for your article share",
            VariableType::String,
        );

        node.add_input_pin(
            "url",
            "URL",
            "The URL of the article to share",
            VariableType::String,
        );

        node.add_input_pin(
            "title",
            "Title",
            "Optional title for the article",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_input_pin(
            "description",
            "Description",
            "Optional description for the article",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_input_pin(
            "visibility",
            "Visibility",
            "Who can see this post: PUBLIC, CONNECTIONS",
            VariableType::String,
        )
        .set_default_value(Some(json!("PUBLIC")))
        .set_options(
            PinOptions::new()
                .set_valid_values(vec!["PUBLIC".to_string(), "CONNECTIONS".to_string()])
                .build(),
        );

        node.add_output_pin(
            "post_id",
            "Post ID",
            "The ID of the created post",
            VariableType::String,
        );

        node.add_output_pin(
            "error_message",
            "Error Message",
            "Error message if article sharing fails",
            VariableType::String,
        );

        node.add_required_oauth_scopes(LINKEDIN_PROVIDER_ID, vec!["w_member_social"]);
        node.set_scores(
            NodeScores::new()
                .set_privacy(5)
                .set_security(8)
                .set_performance(8)
                .set_governance(6)
                .set_reliability(8)
                .set_cost(10)
                .build(),
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_success").await?;
        context.deactivate_exec_pin("exec_error").await?;

        let provider: LinkedInProvider = context.evaluate_pin("provider").await?;
        let author_id: String = context.evaluate_pin("author_id").await?;
        let text: String = context.evaluate_pin("text").await?;
        let url: String = context.evaluate_pin("url").await?;
        let title: String = context.evaluate_pin("title").await?;
        let description: String = context.evaluate_pin("description").await?;
        let visibility: String = context.evaluate_pin("visibility").await?;

        if url.is_empty() {
            context
                .set_pin_value("error_message", json!("Article URL cannot be empty"))
                .await?;
            context.activate_exec_pin("exec_error").await?;
            return Ok(());
        }

        let author_urn = if author_id.starts_with("urn:li:person:") {
            author_id
        } else {
            format!("urn:li:person:{}", author_id)
        };

        let mut media = json!({
            "status": "READY",
            "originalUrl": url,
        });

        if !title.is_empty() {
            media["title"] = json!({"text": title});
        }
        if !description.is_empty() {
            media["description"] = json!({"text": description});
        }

        let request = json!({
            "author": author_urn,
            "lifecycleState": "PUBLISHED",
            "specificContent": {
                "com.linkedin.ugc.ShareContent": {
                    "shareCommentary": {
                        "text": text
                    },
                    "shareMediaCategory": "ARTICLE",
                    "media": [media]
                }
            },
            "visibility": {
                "com.linkedin.ugc.MemberNetworkVisibility": visibility
            }
        });

        let client = reqwest::Client::new();
        let response = client
            .post("https://api.linkedin.com/v2/ugcPosts")
            .header("Authorization", provider.auth_header())
            .header("Content-Type", "application/json")
            .header("X-Restli-Protocol-Version", "2.0.0")
            .json(&request)
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status();
            let error_text = response.text().await.unwrap_or_default();
            context
                .set_pin_value(
                    "error_message",
                    json!(format!("{} - {}", status, error_text)),
                )
                .await?;
            context.activate_exec_pin("exec_error").await?;
            return Ok(());
        }

        let data: Value = response.json().await?;
        let post_id = data
            .get("id")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        context.set_pin_value("post_id", json!(post_id)).await?;
        context.set_pin_value("error_message", json!("")).await?;
        context.activate_exec_pin("exec_success").await?;

        Ok(())
    }
}
