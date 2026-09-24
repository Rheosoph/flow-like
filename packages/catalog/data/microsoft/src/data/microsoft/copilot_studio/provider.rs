use super::{CATEGORY, FLOWSCRIPT_NAMESPACE, INVOKE_SCOPE, POWER_PLATFORM_PROVIDER_ID, scores};
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_types::{JsonSchema, async_trait, json::json};
use serde::{Deserialize, Serialize};

/// Delegated Power Platform API credentials (audience `https://api.powerplatform.com`). Not
/// interchangeable with a Microsoft Graph provider.
#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone)]
pub struct CopilotStudioProvider {
    pub provider_id: String,
    pub access_token: String,
}

pub(crate) fn add_provider_input_pin(node: &mut Node) {
    node.add_input_pin(
        "provider",
        "Provider",
        "Copilot Studio provider with a delegated Power Platform token",
        VariableType::Struct,
    )
    .set_schema::<CopilotStudioProvider>()
    .set_options(PinOptions::new().set_enforce_schema(true).build());
}

fn add_provider_output_pin(node: &mut Node) {
    node.add_output_pin(
        "provider",
        "Provider",
        "Copilot Studio provider with a delegated Power Platform token",
        VariableType::Struct,
    )
    .set_schema::<CopilotStudioProvider>()
    .set_options(PinOptions::new().set_enforce_schema(true).build());
}

#[crate::register_node]
#[derive(Default)]
pub struct CopilotStudioOAuthProviderNode {}

#[async_trait]
impl NodeLogic for CopilotStudioOAuthProviderNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_microsoft_copilot_studio_provider_oauth",
            "Copilot Studio (OAuth)",
            "Sign in to Microsoft Copilot Studio with delegated OAuth (Power Platform API). The account must be in the same Entra tenant as the agent, and the agent must be published and shared with it. Agent usage is billed to that tenant in Copilot Credits.",
            CATEGORY,
        );
        node.set_flowscript_name(FLOWSCRIPT_NAMESPACE, "providerOauth");
        node.add_icon("/flow/icons/microsoft.svg");

        add_provider_output_pin(&mut node);

        node.add_oauth_provider(POWER_PLATFORM_PROVIDER_ID);
        node.add_required_oauth_scopes(POWER_PLATFORM_PROVIDER_ID, vec![INVOKE_SCOPE]);
        node.set_scores(scores(6, 8, 7));
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let token = context
            .get_oauth_token(POWER_PLATFORM_PROVIDER_ID)
            .ok_or_else(|| {
                flow_like_types::anyhow!(
                    "Copilot Studio is not authenticated or its Power Platform token expired. Authorize access when prompted."
                )
            })?
            .access_token
            .clone();

        let provider = CopilotStudioProvider {
            provider_id: POWER_PLATFORM_PROVIDER_ID.to_string(),
            access_token: token,
        };
        context.set_pin_value("provider", json!(provider)).await?;
        Ok(())
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct CopilotStudioTokenProviderNode {}

#[async_trait]
impl NodeLogic for CopilotStudioTokenProviderNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_microsoft_copilot_studio_provider_token",
            "Copilot Studio (Token)",
            "Use a delegated Power Platform access token (audience https://api.powerplatform.com, scope CopilotStudio.Copilots.Invoke), for example from your own identity broker. App-only tokens are not supported by Copilot Studio.",
            CATEGORY,
        );
        node.set_flowscript_name(FLOWSCRIPT_NAMESPACE, "providerToken");
        node.add_icon("/flow/icons/microsoft.svg");

        node.add_input_pin(
            "token",
            "Access Token",
            "Delegated Power Platform access token (audience https://api.powerplatform.com)",
            VariableType::String,
        )
        .set_options(PinOptions::new().set_sensitive(true).build());

        add_provider_output_pin(&mut node);

        node.set_scores(scores(6, 9, 7));
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let token: String = context.evaluate_pin("token").await?;
        let token = token.trim();
        if token.is_empty() {
            return Err(flow_like_types::anyhow!(
                "Copilot Studio access token is empty; provide a delegated token for https://api.powerplatform.com"
            ));
        }

        let provider = CopilotStudioProvider {
            provider_id: POWER_PLATFORM_PROVIDER_ID.to_string(),
            access_token: token.to_string(),
        };
        context.set_pin_value("provider", json!(provider)).await?;
        Ok(())
    }
}
