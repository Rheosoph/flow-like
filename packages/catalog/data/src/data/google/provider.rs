use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic, NodeScores},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_types::{JsonSchema, async_trait, json::json};
use serde::{Deserialize, Serialize};

pub const GOOGLE_PROVIDER_ID: &str = "google";

/// Google provider output - contains authentication token for all Google services
/// (Drive, Sheets, Docs, Slides, Gmail, YouTube, Meet, Calendar, etc.)
#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone)]
pub struct GoogleProvider {
    /// The provider ID
    pub provider_id: String,
    /// The OAuth access token
    pub access_token: String,
    /// Optional refresh token
    pub refresh_token: Option<String>,
    /// Token expiration timestamp (unix seconds)
    pub expires_at: Option<u64>,
}

#[crate::register_node]
#[derive(Default)]
pub struct GoogleProviderNode {}

impl GoogleProviderNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for GoogleProviderNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_google_provider",
            "Google",
            "Authenticate with Google to access Drive, Sheets, Docs, Gmail, YouTube, Calendar and more.",
            "Data/Google",
        );
        node.set_flowscript_name("google", "provider");
        node.add_icon("/flow/icons/google.svg");

        node.add_output_pin(
            "provider",
            "Provider",
            "Google provider with authentication token - works with all Google services",
            VariableType::Struct,
        )
        .set_schema::<GoogleProvider>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        // Add OAuth provider reference - full config comes from Hub
        // Base scopes (openid, email, profile) are in the Hub config
        // Individual service nodes add their required scopes via add_required_oauth_scopes
        node.add_oauth_provider(GOOGLE_PROVIDER_ID);

        node.set_scores(
            NodeScores::new()
                .set_privacy(5)
                .set_security(8)
                .set_performance(7)
                .set_governance(6)
                .set_reliability(9)
                .set_cost(5)
                .build(),
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let token = context
            .get_oauth_token(GOOGLE_PROVIDER_ID)
            .ok_or_else(|| {
                flow_like_types::anyhow!(
                    "Google not authenticated. Please authorize access when prompted."
                )
            })?
            .clone();

        let provider = GoogleProvider {
            provider_id: GOOGLE_PROVIDER_ID.to_string(),
            access_token: token.access_token,
            refresh_token: token.refresh_token,
            expires_at: token.expires_at,
        };

        context.set_pin_value("provider", json!(provider)).await?;

        Ok(())
    }
}
