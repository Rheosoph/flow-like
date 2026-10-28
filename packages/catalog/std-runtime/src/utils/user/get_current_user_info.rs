use super::{project_users::api_base_url, run_user_route};
use flow_like::flow::{
    execution::{LogLevel, context::ExecutionContext},
    node::{Node, NodeLogic, NodeScores},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_types::{JsonSchema, Value, async_trait, json::json, reqwest};
use serde::{Deserialize, Serialize};

/// The profile fields of a user. Account, billing and permission data never
/// reach a board, whichever hub route answered.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct CurrentUserInfo {
    pub id: String,
    pub email: Option<String>,
    pub username: Option<String>,
    pub preferred_username: Option<String>,
    pub name: Option<String>,
    pub description: Option<String>,
    #[serde(alias = "avatar_url")]
    pub avatar: Option<String>,
    pub additional_information: Option<Value>,
    pub created_at: Option<String>,
}

/// Fetch the profile of the user a run acts as from the FlowLike hub.
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
            "Gets the profile of the user this run acts as. Server runs see only the fields the hub shares in user lookups.",
            "Utils/User",
        );
        node.set_flowscript_name("user", "getCurrentInfo");
        node.add_icon("/flow/icons/user.svg");
        node.set_version(2);

        node.add_output_pin(
            "user_info",
            "User Info",
            "Profile of the user this run acts as",
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
            None => return set_failure(context, 0, "No execution token available").await,
        };

        let route = run_user_route(context);
        let Some(url) = user_url(&context.profile.hub, context.profile.secure, &route) else {
            return set_failure(context, 0, "No hub URL configured on the execution profile").await;
        };

        match fetch_profile(&url, token, &route).await {
            Ok((user_info, status_code)) => set_profile(context, &user_info, status_code).await,
            Err((status_code, message)) => {
                context.log_message(&message, LogLevel::Warn);
                set_failure(context, status_code, &message).await
            }
        }
    }
}

async fn set_profile(
    context: &mut ExecutionContext,
    user_info: &CurrentUserInfo,
    status_code: i64,
) -> flow_like_types::Result<()> {
    context.set_pin_value("user_info", json!(user_info)).await?;
    context.set_pin_value("success", json!(true)).await?;
    context
        .set_pin_value("status_code", json!(status_code))
        .await?;
    context.set_pin_value("error", json!("")).await?;
    Ok(())
}

/// The profile `url` answers with and its status code, or the status code and
/// message of the failure.
async fn fetch_profile(
    url: &str,
    token: &str,
    route: &str,
) -> Result<(CurrentUserInfo, i64), (i64, String)> {
    let response = reqwest::Client::new()
        .get(url)
        .bearer_auth(token)
        .send()
        .await
        .map_err(|err| (0, format!("Failed to call /api/v1/{route}: {err}")))?;

    let status = response.status();
    let status_code = status.as_u16() as i64;
    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        let message = format!(
            "/api/v1/{route} returned {status}: {}",
            truncate_error_body(&body)
        );
        return Err((status_code, message));
    }

    response
        .json::<CurrentUserInfo>()
        .await
        .map(|user_info| (user_info, status_code))
        .map_err(|err| {
            (
                status_code,
                format!("Failed to parse /api/v1/{route} response: {err}"),
            )
        })
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

fn user_url(hub: &str, secure: bool, route: &str) -> Option<String> {
    api_base_url(hub, secure).map(|base| format!("{base}/{route}"))
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
    use super::{CurrentUserInfo, truncate_error_body, user_url};
    use flow_like_types::json::{self, json};

    fn reserialized(row: flow_like_types::Value) -> json::Map<String, flow_like_types::Value> {
        let info: CurrentUserInfo = json::from_value(row).unwrap();
        match json::to_value(info).unwrap() {
            flow_like_types::Value::Object(fields) => fields,
            other => panic!("a profile is an object, got {other}"),
        }
    }

    #[test]
    fn account_rows_keep_only_profile_fields() {
        let profile = reserialized(json!({
            "id": "user-1",
            "stripe_id": "cus_1",
            "subscription_id": "sub_1",
            "tracking_id": "tracking-1",
            "email": "person@example.com",
            "username": "person",
            "preferred_username": "person-handle",
            "name": "Person",
            "description": "About",
            "avatar": "https://signed.example/avatar",
            "additional_information": { "team": "ops" },
            "permission": 7,
            "accepted_terms_version": "v1",
            "tutorial_completed": true,
            "status": "ACTIVE",
            "tier": "PRO",
            "total_size": 1,
            "total_llm_price": 2,
            "total_embedding_price": 3,
            "llm_price_tracking_month": "2026-10",
            "dev_mode": true,
            "created_at": "2026-10-10T12:00:00+00:00",
            "updated_at": "2026-10-11T12:00:00+00:00"
        }));

        let mut keys = profile.keys().map(String::as_str).collect::<Vec<_>>();
        keys.sort_unstable();
        assert_eq!(
            keys,
            [
                "additional_information",
                "avatar",
                "created_at",
                "description",
                "email",
                "id",
                "name",
                "preferred_username",
                "username"
            ]
        );
        assert_eq!(profile["email"], "person@example.com");
        assert_eq!(profile["avatar"], "https://signed.example/avatar");
        assert_eq!(profile["created_at"], "2026-10-10T12:00:00+00:00");
    }

    #[test]
    fn directory_rows_map_avatar_url() {
        let profile = reserialized(json!({
            "id": "user-1",
            "email": null,
            "username": null,
            "preferred_username": "person-handle",
            "name": "Person",
            "avatar_url": "https://signed.example/avatar",
            "additional_information": null,
            "description": null,
            "created_at": null
        }));

        assert_eq!(profile["avatar"], "https://signed.example/avatar");
        assert_eq!(profile["created_at"], flow_like_types::Value::Null);
        assert_eq!(profile["email"], flow_like_types::Value::Null);
        assert!(!profile.contains_key("avatar_url"));
    }

    #[test]
    fn a_row_without_an_id_is_not_a_profile() {
        assert!(
            json::from_value::<CurrentUserInfo>(json!({ "name": "Person", "email": null }))
                .is_err()
        );
    }

    #[test]
    fn builds_the_run_user_url() {
        for (hub, secure, route, expected) in [
            (
                "https://hub.flow-like.com/",
                true,
                "execution/apps/app-1/user",
                "https://hub.flow-like.com/api/v1/execution/apps/app-1/user",
            ),
            (
                "https://hub.flow-like.com/api/v1",
                true,
                "user/info",
                "https://hub.flow-like.com/api/v1/user/info",
            ),
            (
                "localhost:8080",
                false,
                "user/info",
                "http://localhost:8080/api/v1/user/info",
            ),
        ] {
            assert_eq!(user_url(hub, secure, route).as_deref(), Some(expected));
        }
        assert_eq!(user_url("  ", true, "user/info"), None);
    }

    #[test]
    fn truncates_long_error_bodies() {
        let long = "a".repeat(600);
        let truncated = truncate_error_body(&long);
        assert_eq!(truncated.len(), 503);
        assert!(truncated.ends_with("..."));
    }
}
