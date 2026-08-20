use flow_like::flow::{
    node::{Node, NodeLogic, NodeScores},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_types::{JsonSchema, async_trait, json::json};
use serde::{Deserialize, Serialize};

pub const LINKEDIN_PROVIDER_ID: &str = "linkedin";

#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone)]
pub struct LinkedInProvider {
    pub provider_id: String,
    pub access_token: String,
}

impl LinkedInProvider {
    pub fn api_url(&self, path: &str) -> String {
        let path = path.strip_prefix('/').unwrap_or(path);
        format!("https://api.linkedin.com/v2/{}", path)
    }

    pub fn auth_header(&self) -> String {
        format!("Bearer {}", self.access_token)
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct LinkedInOAuthProviderNode {}

impl LinkedInOAuthProviderNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for LinkedInOAuthProviderNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_linkedin_provider_oauth",
            "LinkedIn (OAuth)",
            "Connect to LinkedIn using OAuth 2.0. Requires OAuth provider configuration in flow-like.config.json.",
            "Data/LinkedIn",
        );
        node.add_icon("/flow/icons/linkedin.svg");

        node.add_output_pin(
            "provider",
            "Provider",
            "LinkedIn provider for API access",
            VariableType::Struct,
        )
        .set_schema::<LinkedInProvider>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_oauth_provider(LINKEDIN_PROVIDER_ID);
        node.add_required_oauth_scopes(LINKEDIN_PROVIDER_ID, vec!["openid", "profile", "email"]);

        node.set_scores(
            NodeScores::new()
                .set_privacy(6)
                .set_security(8)
                .set_performance(9)
                .set_governance(7)
                .set_reliability(8)
                .set_cost(10)
                .build(),
        );

        node
    }

    async fn run(
        &self,
        context: &mut flow_like::flow::execution::context::ExecutionContext,
    ) -> flow_like_types::Result<()> {
        let token = context
            .get_oauth_token(LINKEDIN_PROVIDER_ID)
            .ok_or_else(|| {
                flow_like_types::anyhow!(
                    "LinkedIn not authenticated. Please authorize access when prompted."
                )
            })?
            .clone();

        let provider = LinkedInProvider {
            provider_id: LINKEDIN_PROVIDER_ID.to_string(),
            access_token: token.access_token,
        };

        context.set_pin_value("provider", json!(provider)).await?;

        Ok(())
    }
}
