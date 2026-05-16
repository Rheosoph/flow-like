use flow_like::flow::{
    execution::{LogLevel, context::ExecutionContext},
    node::{Node, NodeLogic, NodeScores},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_types::{JsonSchema, Value, async_trait, json::json, reqwest};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct CurrentUserInfo {
    pub id: String,
    pub stripe_id: Option<String>,
    pub tracking_id: Option<String>,
    pub email: Option<String>,
    pub username: Option<String>,
    pub preferred_username: Option<String>,
    pub name: Option<String>,
    pub description: Option<String>,
    pub avatar: Option<String>,
    pub additional_information: Option<Value>,
    pub permission: i64,
    pub accepted_terms_version: Option<String>,
    pub tutorial_completed: bool,
    pub status: Value,
    pub tier: Value,
    pub total_size: i64,
    pub total_llm_price: i64,
    pub total_embedding_price: i64,
    pub llm_price_tracking_month: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

/// Fetch the current user's persisted user record from the FlowLike hub.
#[crate::register_node]
#[derive(Default)]
pub struct GetCurrentUserInfoNode {}

impl GetCurrentUserInfoNode {
    pub fn new() -> Self {
        GetCurrentUserInfoNode {}
    }
}

#[async_trait]
impl NodeLogic for GetCurrentUserInfoNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "utils_user_get_current_user_info",
            "Get Current User Info",
            "Fetches the current user's persisted user information from the configured FlowLike hub's /api/v1/user/info endpoint when an execution token is available.",
            "Utils/User",
        );
        node.add_icon("/flow/icons/user.svg");

        node.add_output_pin(
            "user_info",
            "User Info",
            "The user record returned by /api/v1/user/info",
            VariableType::Struct,
        )
        .set_schema::<CurrentUserInfo>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_output_pin(
            "success",
            "Success",
            "True when user info was fetched successfully",
            VariableType::Boolean,
        );

        node.add_output_pin(
            "status_code",
            "Status Code",
            "HTTP status code returned by the hub, or 0 if no request was made",
            VariableType::Integer,
        );

        node.add_output_pin(
            "error",
            "Error",
            "Error message when user info could not be fetched",
            VariableType::String,
        );

        node.set_scores(
            NodeScores::new()
                .set_privacy(7)
                .set_security(8)
                .set_performance(6)
                .set_governance(8)
                .set_reliability(8)
                .set_cost(9)
                .build(),
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let token = match context
            .token
            .as_deref()
            .map(str::trim)
            .filter(|token| !token.is_empty())
        {
            Some(token) => token,
            None => {
                set_failure(context, 0, "No execution token available").await?;
                return Ok(());
            }
        };

        let url = match user_info_url(&context.profile.hub, context.profile.secure) {
            Some(url) => url,
            None => {
                set_failure(context, 0, "No hub URL configured on the execution profile").await?;
                return Ok(());
            }
        };

        let response = match reqwest::Client::new()
            .get(&url)
            .bearer_auth(token)
            .send()
            .await
        {
            Ok(response) => response,
            Err(err) => {
                let message = format!("Failed to call /api/v1/user/info: {err}");
                context.log_message(&message, LogLevel::Warn);
                set_failure(context, 0, &message).await?;
                return Ok(());
            }
        };

        let status = response.status();
        let status_code = status.as_u16() as i64;

        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            let message = format!(
                "/api/v1/user/info returned {}: {}",
                status,
                truncate_error_body(&body)
            );
            context.log_message(&message, LogLevel::Warn);
            set_failure(context, status_code, &message).await?;
            return Ok(());
        }

        let user_info = match response.json::<Value>().await {
            Ok(user_info) => user_info,
            Err(err) => {
                let message = format!("Failed to parse /api/v1/user/info response: {err}");
                context.log_message(&message, LogLevel::Warn);
                set_failure(context, status_code, &message).await?;
                return Ok(());
            }
        };

        context.set_pin_value("user_info", user_info).await?;
        context.set_pin_value("success", json!(true)).await?;
        context
            .set_pin_value("status_code", json!(status_code))
            .await?;
        context.set_pin_value("error", json!("")).await?;

        Ok(())
    }
}

async fn set_failure(
    context: &mut ExecutionContext,
    status_code: i64,
    error: &str,
) -> flow_like_types::Result<()> {
    context.set_pin_value("user_info", Value::Null).await?;
    context.set_pin_value("success", json!(false)).await?;
    context
        .set_pin_value("status_code", json!(status_code))
        .await?;
    context.set_pin_value("error", json!(error)).await?;
    Ok(())
}

fn user_info_url(hub: &str, secure: bool) -> Option<String> {
    let hub = hub.trim().trim_end_matches('/');
    if hub.is_empty() {
        return None;
    }

    let origin = if hub.starts_with("http://") || hub.starts_with("https://") {
        hub.to_string()
    } else {
        let protocol = if secure { "https" } else { "http" };
        format!("{protocol}://{hub}")
    };

    if origin.ends_with("/api/v1") {
        return Some(format!("{origin}/user/info"));
    }

    Some(format!("{origin}/api/v1/user/info"))
}

fn truncate_error_body(body: &str) -> String {
    const MAX_CHARS: usize = 500;
    let body = body.trim();
    let mut chars = body.chars();
    let truncated: String = chars.by_ref().take(MAX_CHARS).collect();
    if chars.next().is_none() {
        return body.to_string();
    }

    format!("{truncated}...")
}

#[cfg(test)]
mod tests {
    use super::{truncate_error_body, user_info_url};

    #[test]
    fn builds_user_info_url() {
        assert_eq!(
            user_info_url("https://hub.flow-like.com/", true).as_deref(),
            Some("https://hub.flow-like.com/api/v1/user/info")
        );
        assert_eq!(
            user_info_url("https://hub.flow-like.com/api/v1", true).as_deref(),
            Some("https://hub.flow-like.com/api/v1/user/info")
        );
        assert_eq!(
            user_info_url("localhost:8080", false).as_deref(),
            Some("http://localhost:8080/api/v1/user/info")
        );
        assert_eq!(user_info_url("  ", true).as_deref(), None);
    }

    #[test]
    fn truncates_long_error_bodies() {
        let long = "a".repeat(600);
        let truncated = truncate_error_body(&long);
        assert_eq!(truncated.len(), 503);
        assert!(truncated.ends_with("..."));
    }
}
