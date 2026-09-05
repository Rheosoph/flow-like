use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic, NodeScores},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_types::{JsonSchema, Value, async_trait, json::json};
use serde::{Deserialize, Serialize};

pub const DATABRICKS_PROVIDER_ID: &str = "databricks";

/// Databricks provider - works with PAT, OAuth M2M (Service Principal), or access tokens
#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone)]
pub struct DatabricksProvider {
    pub provider_id: String,
    pub access_token: String,
    pub workspace_url: String,
}

impl DatabricksProvider {
    pub fn api_url(&self, path: &str) -> String {
        let base = self.workspace_url.trim_end_matches('/');
        if path.starts_with('/') {
            format!("{}/api/2.0{}", base, path)
        } else {
            format!("{}/api/2.0/{}", base, path)
        }
    }

    pub fn api_url_v21(&self, path: &str) -> String {
        let base = self.workspace_url.trim_end_matches('/');
        if path.starts_with('/') {
            format!("{}/api/2.1{}", base, path)
        } else {
            format!("{}/api/2.1/{}", base, path)
        }
    }
}

// =============================================================================
// Personal Access Token Provider
// =============================================================================

#[crate::register_node]
#[derive(Default)]
pub struct DatabricksPatProviderNode {}

impl DatabricksPatProviderNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for DatabricksPatProviderNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_databricks_provider_pat",
            "Databricks (PAT)",
            "Connect to Databricks using a Personal Access Token. Generate one in your Databricks workspace under User Settings > Developer > Access tokens.",
            "Data/Databricks",
        );
        node.set_flowscript_name("databricks", "providerPat");
        node.add_icon("/flow/icons/databricks.svg");

        node.add_input_pin(
            "token",
            "Personal Access Token",
            "Your Databricks Personal Access Token",
            VariableType::String,
        )
        .set_options(PinOptions::new().set_sensitive(true).build());

        node.add_input_pin(
            "workspace_url",
            "Workspace URL",
            "Your Databricks workspace URL (e.g., https://adb-1234567890123456.7.azuredatabricks.net or https://dbc-a1b2c3d4-e5f6.cloud.databricks.com)",
            VariableType::String,
        );

        node.add_output_pin(
            "provider",
            "Provider",
            "Databricks provider with authentication",
            VariableType::Struct,
        )
        .set_schema::<DatabricksProvider>()
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
        let workspace_url: String = context.evaluate_pin("workspace_url").await?;

        if token.is_empty() {
            return Err(flow_like_types::anyhow!(
                "Personal Access Token is required. Generate one in your Databricks workspace under User Settings > Developer > Access tokens."
            ));
        }

        if workspace_url.is_empty() {
            return Err(flow_like_types::anyhow!(
                "Workspace URL is required. Find it in your browser address bar when accessing your Databricks workspace."
            ));
        }

        let provider = DatabricksProvider {
            provider_id: DATABRICKS_PROVIDER_ID.to_string(),
            access_token: token,
            workspace_url,
        };

        context.set_pin_value("provider", json!(provider)).await?;

        Ok(())
    }
}

// =============================================================================
// Access Token Provider (for manual/external token management)
// =============================================================================

#[crate::register_node]
#[derive(Default)]
pub struct DatabricksTokenProviderNode {}

impl DatabricksTokenProviderNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for DatabricksTokenProviderNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_databricks_provider_token",
            "Databricks (Token)",
            "Connect to Databricks using an externally managed access token. Use this for tokens obtained from OAuth flows or service principals.",
            "Data/Databricks",
        );
        node.set_flowscript_name("databricks", "providerToken");
        node.add_icon("/flow/icons/databricks.svg");

        node.add_input_pin(
            "token",
            "Access Token",
            "Databricks access token (OAuth or PAT)",
            VariableType::String,
        )
        .set_options(PinOptions::new().set_sensitive(true).build());

        node.add_input_pin(
            "workspace_url",
            "Workspace URL",
            "Your Databricks workspace URL",
            VariableType::String,
        );

        node.add_output_pin(
            "provider",
            "Provider",
            "Databricks provider with authentication",
            VariableType::Struct,
        )
        .set_schema::<DatabricksProvider>()
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
        let workspace_url: String = context.evaluate_pin("workspace_url").await?;

        if token.is_empty() {
            return Err(flow_like_types::anyhow!("Access token is required"));
        }

        if workspace_url.is_empty() {
            return Err(flow_like_types::anyhow!("Workspace URL is required"));
        }

        let provider = DatabricksProvider {
            provider_id: DATABRICKS_PROVIDER_ID.to_string(),
            access_token: token,
            workspace_url,
        };

        context.set_pin_value("provider", json!(provider)).await?;

        Ok(())
    }
}

// =============================================================================
// OAuth M2M (Service Principal) Provider
// =============================================================================

#[crate::register_node]
#[derive(Default)]
pub struct DatabricksServicePrincipalProviderNode {}

impl DatabricksServicePrincipalProviderNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for DatabricksServicePrincipalProviderNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_databricks_provider_service_principal",
            "Databricks (Service Principal)",
            "Connect to Databricks using OAuth M2M (Machine-to-Machine) authentication with a service principal. Ideal for automated workflows and CI/CD pipelines.",
            "Data/Databricks",
        );
        node.set_flowscript_name("databricks", "providerServicePrincipal");
        node.add_icon("/flow/icons/databricks.svg");

        node.add_input_pin("exec_in", "Input", "Trigger", VariableType::Execution);

        node.add_input_pin(
            "client_id",
            "Client ID",
            "The service principal's client ID (application ID)",
            VariableType::String,
        );

        node.add_input_pin(
            "client_secret",
            "Client Secret",
            "The service principal's OAuth secret",
            VariableType::String,
        )
        .set_options(PinOptions::new().set_sensitive(true).build());

        node.add_input_pin(
            "workspace_url",
            "Workspace URL",
            "Your Databricks workspace URL for workspace-level operations",
            VariableType::String,
        );

        node.add_input_pin(
            "account_id",
            "Account ID",
            "Optional: Databricks account ID for account-level operations. Leave empty for workspace-level only.",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_output_pin(
            "exec_out",
            "Success",
            "Triggered on successful authentication",
            VariableType::Execution,
        );

        node.add_output_pin(
            "error",
            "Error",
            "Triggered on authentication failure",
            VariableType::Execution,
        );

        node.add_output_pin(
            "provider",
            "Provider",
            "Databricks provider with authentication",
            VariableType::Struct,
        )
        .set_schema::<DatabricksProvider>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_output_pin(
            "error_message",
            "Error Message",
            "Error message if authentication fails",
            VariableType::String,
        );

        node.set_scores(
            NodeScores::new()
                .set_privacy(7)
                .set_security(9)
                .set_performance(8)
                .set_governance(8)
                .set_reliability(9)
                .set_cost(9)
                .build(),
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        context.deactivate_exec_pin("error").await?;

        let client_id: String = context.evaluate_pin("client_id").await?;
        let client_secret: String = context.evaluate_pin("client_secret").await?;
        let workspace_url: String = context.evaluate_pin("workspace_url").await?;
        let account_id: String = context.evaluate_pin("account_id").await.unwrap_or_default();

        if client_id.is_empty() {
            context
                .set_pin_value("error_message", json!("Client ID is required"))
                .await?;
            context.activate_exec_pin("error").await?;
            return Ok(());
        }

        if client_secret.is_empty() {
            context
                .set_pin_value("error_message", json!("Client secret is required"))
                .await?;
            context.activate_exec_pin("error").await?;
            return Ok(());
        }

        if workspace_url.is_empty() {
            context
                .set_pin_value("error_message", json!("Workspace URL is required"))
                .await?;
            context.activate_exec_pin("error").await?;
            return Ok(());
        }

        // Construct the token endpoint URL
        // For workspace-level: https://<workspace-url>/oidc/v1/token
        // For account-level: https://accounts.cloud.databricks.com/oidc/accounts/<account-id>/v1/token
        let token_url = if account_id.is_empty() {
            format!("{}/oidc/v1/token", workspace_url.trim_end_matches('/'))
        } else {
            format!(
                "https://accounts.cloud.databricks.com/oidc/accounts/{}/v1/token",
                account_id
            )
        };

        let client = flow_like_types::reqwest::Client::new();
        let response = client
            .post(&token_url)
            .basic_auth(&client_id, Some(&client_secret))
            .form(&[("grant_type", "client_credentials"), ("scope", "all-apis")])
            .send()
            .await;

        match response {
            Ok(resp) => {
                if resp.status().is_success() {
                    let token_response: Value = resp.json().await.map_err(|e| {
                        flow_like_types::anyhow!("Failed to parse token response: {}", e)
                    })?;

                    let access_token = token_response["access_token"]
                        .as_str()
                        .ok_or_else(|| flow_like_types::anyhow!("No access_token in response"))?;

                    let provider = DatabricksProvider {
                        provider_id: DATABRICKS_PROVIDER_ID.to_string(),
                        access_token: access_token.to_string(),
                        workspace_url,
                    };

                    context.set_pin_value("provider", json!(provider)).await?;
                    context.set_pin_value("error_message", json!("")).await?;
                    context.activate_exec_pin("exec_out").await?;
                } else {
                    let status = resp.status();
                    let error_text = resp
                        .text()
                        .await
                        .unwrap_or_else(|_| "Unknown error".to_string());
                    context
                        .set_pin_value(
                            "error_message",
                            json!(format!(
                                "Authentication failed ({}): {}",
                                status, error_text
                            )),
                        )
                        .await?;
                    context.activate_exec_pin("error").await?;
                }
            }
            Err(e) => {
                context
                    .set_pin_value("error_message", json!(format!("Request failed: {}", e)))
                    .await?;
                context.activate_exec_pin("error").await?;
            }
        }

        Ok(())
    }
}

// =============================================================================
// OAuth Provider (User-facing OAuth flow)
// =============================================================================

#[crate::register_node]
#[derive(Default)]
pub struct DatabricksOAuthProviderNode {}

impl DatabricksOAuthProviderNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for DatabricksOAuthProviderNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_databricks_provider_oauth",
            "Databricks (OAuth)",
            "Connect to Databricks using OAuth. The workspace URL determines the OAuth endpoints.",
            "Data/Databricks",
        );
        node.set_flowscript_name("databricks", "providerOauth");
        node.add_icon("/flow/icons/databricks.svg");

        node.add_input_pin(
            "workspace_url",
            "Workspace URL",
            "Your Databricks workspace URL (e.g., https://adb-1234567890123456.7.azuredatabricks.net)",
            VariableType::String,
        );

        node.add_output_pin(
            "provider",
            "Provider",
            "Databricks provider with authentication",
            VariableType::Struct,
        )
        .set_schema::<DatabricksProvider>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        // Add OAuth provider reference - full config comes from Hub
        node.add_oauth_provider(DATABRICKS_PROVIDER_ID);
        node.add_required_oauth_scopes(DATABRICKS_PROVIDER_ID, vec!["all-apis", "offline_access"]);

        node.set_scores(
            NodeScores::new()
                .set_privacy(6)
                .set_security(8)
                .set_performance(8)
                .set_governance(7)
                .set_reliability(9)
                .set_cost(7)
                .build(),
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let workspace_url: String = context.evaluate_pin("workspace_url").await?;

        if workspace_url.is_empty() {
            return Err(flow_like_types::anyhow!("Workspace URL is required"));
        }

        let token = context
            .get_oauth_token(DATABRICKS_PROVIDER_ID)
            .ok_or_else(|| {
                flow_like_types::anyhow!(
                    "Databricks not authenticated. Please authorize access when prompted."
                )
            })?
            .clone();

        let provider = DatabricksProvider {
            provider_id: DATABRICKS_PROVIDER_ID.to_string(),
            access_token: token.access_token,
            workspace_url,
        };

        context.set_pin_value("provider", json!(provider)).await?;

        Ok(())
    }
}
