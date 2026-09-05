use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic, NodeScores},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_types::{JsonSchema, async_trait, json::json};
use serde::{Deserialize, Serialize};

pub const MICROSOFT_PROVIDER_ID: &str = "microsoft";

/// Microsoft Graph provider - works with OAuth or access tokens
#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone)]
pub struct MicrosoftGraphProvider {
    pub provider_id: String,
    pub access_token: String,
    pub base_url: String,
}

impl MicrosoftGraphProvider {
    pub fn api_url(&self, path: &str) -> String {
        let base = self.base_url.trim_end_matches('/');
        if path.starts_with('/') {
            format!("{}{}", base, path)
        } else {
            format!("{}/{}", base, path)
        }
    }
}

// =============================================================================
// Access Token Provider (for server-to-server or manual token)
// =============================================================================

#[crate::register_node]
#[derive(Default)]
pub struct MicrosoftGraphTokenProviderNode {}

impl MicrosoftGraphTokenProviderNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for MicrosoftGraphTokenProviderNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_microsoft_provider_token",
            "Microsoft Graph (Token)",
            "Connect to Microsoft Graph API using an access token. Use for server-to-server auth or manual token management.",
            "Data/Microsoft",
        );
        node.set_flowscript_name("microsoft", "providerToken");
        node.add_icon("/flow/icons/microsoft.svg");

        node.add_input_pin(
            "token",
            "Access Token",
            "Microsoft Graph API access token",
            VariableType::String,
        )
        .set_options(PinOptions::new().set_sensitive(true).build());

        node.add_input_pin(
            "base_url",
            "API Base URL",
            "Microsoft Graph API base URL",
            VariableType::String,
        )
        .set_default_value(Some(json!("https://graph.microsoft.com/v1.0")));

        node.add_output_pin(
            "provider",
            "Provider",
            "Microsoft Graph provider with authentication",
            VariableType::Struct,
        )
        .set_schema::<MicrosoftGraphProvider>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.set_scores(
            NodeScores::new()
                .set_privacy(6)
                .set_security(7)
                .set_performance(9)
                .set_governance(6)
                .set_reliability(9)
                .set_cost(10)
                .build(),
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let token: String = context.evaluate_pin("token").await?;
        let base_url: String = context
            .evaluate_pin("base_url")
            .await
            .unwrap_or_else(|_| "https://graph.microsoft.com/v1.0".to_string());

        if token.is_empty() {
            return Err(flow_like_types::anyhow!("Access token is required"));
        }

        let provider = MicrosoftGraphProvider {
            provider_id: MICROSOFT_PROVIDER_ID.to_string(),
            access_token: token,
            base_url,
        };

        context.set_pin_value("provider", json!(provider)).await?;

        Ok(())
    }
}

// =============================================================================
// OAuth Provider (Authorization Code Flow with PKCE)
// =============================================================================

#[crate::register_node]
#[derive(Default)]
pub struct MicrosoftGraphOAuthProviderNode {}

impl MicrosoftGraphOAuthProviderNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for MicrosoftGraphOAuthProviderNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_microsoft_provider_oauth",
            "Microsoft Graph (OAuth)",
            "Connect to Microsoft Graph using OAuth Authorization Code Flow with PKCE.",
            "Data/Microsoft",
        );
        node.set_flowscript_name("microsoft", "providerOauth");
        node.add_icon("/flow/icons/microsoft.svg");

        node.add_input_pin(
            "base_url",
            "API Base URL",
            "Microsoft Graph API base URL",
            VariableType::String,
        )
        .set_default_value(Some(json!("https://graph.microsoft.com/v1.0")));

        node.add_output_pin(
            "provider",
            "Provider",
            "Microsoft Graph provider with authentication",
            VariableType::Struct,
        )
        .set_schema::<MicrosoftGraphProvider>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        // Add OAuth provider reference - full config comes from Hub
        node.add_oauth_provider(MICROSOFT_PROVIDER_ID);
        node.add_required_oauth_scopes(MICROSOFT_PROVIDER_ID, vec!["User.Read", "offline_access"]);

        node.set_scores(
            NodeScores::new()
                .set_privacy(6)
                .set_security(9)
                .set_performance(8)
                .set_governance(7)
                .set_reliability(9)
                .set_cost(7)
                .build(),
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let base_url: String = context
            .evaluate_pin("base_url")
            .await
            .unwrap_or_else(|_| "https://graph.microsoft.com/v1.0".to_string());

        let token = context
            .get_oauth_token(MICROSOFT_PROVIDER_ID)
            .ok_or_else(|| {
                flow_like_types::anyhow!(
                    "Microsoft not authenticated. Please authorize access when prompted."
                )
            })?
            .clone();

        let provider = MicrosoftGraphProvider {
            provider_id: MICROSOFT_PROVIDER_ID.to_string(),
            access_token: token.access_token,
            base_url,
        };

        context.set_pin_value("provider", json!(provider)).await?;

        Ok(())
    }
}
