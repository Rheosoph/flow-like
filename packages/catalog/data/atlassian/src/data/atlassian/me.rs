use crate::data::atlassian::provider::{ATLASSIAN_PROVIDER_ID, AtlassianProvider};
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic, NodeScores},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_types::{JsonSchema, Value, async_trait, json::json, reqwest};
use serde::{Deserialize, Serialize};

/// Atlassian account information (cross-product identity)
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct AtlassianMe {
    /// Account ID (globally unique across all Atlassian products)
    pub account_id: String,
    /// Account type (atlassian, app, customer)
    pub account_type: String,
    /// Email address
    pub email: Option<String>,
    /// Display name
    pub name: String,
    /// Profile picture URL
    pub picture: Option<String>,
    /// Whether the account is verified
    pub email_verified: Option<bool>,
    /// Account status
    pub account_status: Option<String>,
    /// Nickname
    pub nickname: Option<String>,
    /// Timezone (IANA format)
    pub zoneinfo: Option<String>,
    /// Locale
    pub locale: Option<String>,
}

fn parse_atlassian_me(value: &Value) -> Option<AtlassianMe> {
    let obj = value.as_object()?;

    Some(AtlassianMe {
        account_id: obj.get("account_id")?.as_str()?.to_string(),
        account_type: obj
            .get("account_type")
            .and_then(|v| v.as_str())
            .unwrap_or("atlassian")
            .to_string(),
        email: obj.get("email").and_then(|v| v.as_str()).map(String::from),
        name: obj.get("name")?.as_str()?.to_string(),
        picture: obj
            .get("picture")
            .and_then(|v| v.as_str())
            .map(String::from),
        email_verified: obj.get("email_verified").and_then(|v| v.as_bool()),
        account_status: obj
            .get("account_status")
            .and_then(|v| v.as_str())
            .map(String::from),
        nickname: obj
            .get("nickname")
            .and_then(|v| v.as_str())
            .map(String::from),
        zoneinfo: obj
            .get("zoneinfo")
            .and_then(|v| v.as_str())
            .map(String::from),
        locale: obj.get("locale").and_then(|v| v.as_str()).map(String::from),
    })
}

/// Get the current authenticated user's Atlassian account information.
/// This uses the cross-product /me endpoint which works for any Atlassian OAuth token.
#[crate::register_node]
#[derive(Default)]
pub struct GetMeNode {}

impl GetMeNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for GetMeNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_atlassian_get_me",
            "Get Me",
            "Get the current authenticated user's Atlassian account information (cross-product)",
            "Data/Atlassian",
        );
        node.set_flowscript_name("atlassian", "getMe");
        node.add_icon("/flow/icons/atlassian.svg");

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

        node.add_output_pin(
            "me",
            "Me",
            "Current user's Atlassian account",
            VariableType::Struct,
        )
        .set_schema::<AtlassianMe>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_output_pin(
            "account_id",
            "Account ID",
            "The user's globally unique Atlassian account ID",
            VariableType::String,
        );

        node.add_output_pin(
            "email",
            "Email",
            "The user's email address",
            VariableType::String,
        );

        node.add_output_pin(
            "name",
            "Name",
            "The user's display name",
            VariableType::String,
        );

        // read:me is the only scope needed for this endpoint
        node.add_required_oauth_scopes(ATLASSIAN_PROVIDER_ID, vec!["read:me"]);
        node.set_scores(
            NodeScores::new()
                .set_privacy(7)
                .set_security(9)
                .set_performance(9)
                .set_governance(7)
                .set_reliability(9)
                .set_cost(10)
                .build(),
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let provider: AtlassianProvider = context.evaluate_pin("provider").await?;

        // The /me endpoint is at api.atlassian.com, not the instance URL
        // This works for OAuth tokens only
        if provider.auth_type != "oauth" {
            return Err(flow_like_types::anyhow!(
                "Get Me requires OAuth authentication. For API token or PAT, use the product-specific Get Current User nodes."
            ));
        }

        let client = reqwest::Client::new();
        let url = "https://api.atlassian.com/me";

        let response = client
            .get(url)
            .header("Authorization", provider.auth_header())
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status();
            let error_text = response.text().await.unwrap_or_default();
            return Err(flow_like_types::anyhow!(
                "Failed to get current user: {} - {}",
                status,
                error_text
            ));
        }

        let data: Value = response.json().await?;
        let me = parse_atlassian_me(&data).ok_or_else(|| {
            flow_like_types::anyhow!("Failed to parse user data from Atlassian API")
        })?;

        context.set_pin_value("me", json!(me.clone())).await?;
        context
            .set_pin_value("account_id", json!(me.account_id))
            .await?;
        context
            .set_pin_value("email", json!(me.email.unwrap_or_default()))
            .await?;
        context.set_pin_value("name", json!(me.name)).await?;

        context.activate_exec_pin("exec_out").await?;

        Ok(())
    }
}
