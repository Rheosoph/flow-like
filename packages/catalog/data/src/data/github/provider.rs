use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic, NodeScores},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_types::{JsonSchema, async_trait, json::json};
use serde::{Deserialize, Serialize};

pub const GITHUB_PROVIDER_ID: &str = "github";
pub const GITHUB_API_VERSION: &str = "2026-03-10";

/// GitHub provider - works with OAuth, PAT, or GitHub App tokens
#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone)]
pub struct GitHubProvider {
    pub provider_id: String,
    pub access_token: String,
    pub base_url: String,
}

impl GitHubProvider {
    pub fn api_url(&self, path: &str) -> String {
        let base = self.base_url.trim_end_matches('/');
        if path.starts_with('/') {
            format!("{}{}", base, path)
        } else {
            format!("{}/{}", base, path)
        }
    }

    pub fn upload_api_url(&self, path: &str) -> String {
        let base = self.base_url.trim_end_matches('/');
        let upload_base = if base == "https://api.github.com" {
            "https://uploads.github.com"
        } else {
            base
        };

        if path.starts_with('/') {
            format!("{}{}", upload_base, path)
        } else {
            format!("{}/{}", upload_base, path)
        }
    }

    pub fn auth_header(&self) -> String {
        format!("Bearer {}", self.access_token)
    }

    pub fn clone_url(&self, owner: &str, repo: &str) -> String {
        let base = self.base_url.trim_end_matches('/');
        let web_base = if base == "https://api.github.com" {
            "https://github.com"
        } else {
            base.strip_suffix("/api/v3")
                .or_else(|| base.strip_suffix("/api"))
                .unwrap_or(base)
        };

        let authed_base = if let Some(rest) = web_base.strip_prefix("https://") {
            format!("https://{}@{}", self.access_token, rest)
        } else if let Some(rest) = web_base.strip_prefix("http://") {
            format!("http://{}@{}", self.access_token, rest)
        } else {
            format!("{}@", self.access_token)
        };

        format!(
            "{}/{}/{}.git",
            authed_base.trim_end_matches('/'),
            owner,
            repo
        )
    }
}

// =============================================================================
// Personal Access Token Provider
// =============================================================================

#[crate::register_node]
#[derive(Default)]
pub struct GitHubPatProviderNode {}

impl GitHubPatProviderNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for GitHubPatProviderNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_github_provider_pat",
            "GitHub (PAT)",
            "Connect to GitHub using a Personal Access Token. Generate one at github.com/settings/tokens",
            "Data/GitHub",
        );
        node.set_flowscript_name("github", "providerPat");
        node.add_icon("/flow/icons/github.svg");

        node.add_input_pin(
            "token",
            "Personal Access Token",
            "Your GitHub Personal Access Token (classic or fine-grained)",
            VariableType::String,
        )
        .set_options(PinOptions::new().set_sensitive(true).build());

        node.add_input_pin(
            "base_url",
            "API Base URL",
            "GitHub API base URL. Use 'https://api.github.com' for github.com or 'https://your-enterprise.com/api/v3' for Enterprise",
            VariableType::String,
        )
        .set_default_value(Some(json!("https://api.github.com")));

        node.add_output_pin(
            "provider",
            "Provider",
            "GitHub provider with authentication",
            VariableType::Struct,
        )
        .set_schema::<GitHubProvider>()
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
            .unwrap_or_else(|_| "https://api.github.com".to_string());

        if token.is_empty() {
            return Err(flow_like_types::anyhow!(
                "Personal Access Token is required. Generate one at github.com/settings/tokens"
            ));
        }

        let provider = GitHubProvider {
            provider_id: GITHUB_PROVIDER_ID.to_string(),
            access_token: token,
            base_url,
        };

        context.set_pin_value("provider", json!(provider)).await?;

        Ok(())
    }
}

// =============================================================================
// OAuth Provider (Device Flow)
// =============================================================================

#[crate::register_node]
#[derive(Default)]
pub struct GitHubOAuthProviderNode {}

impl GitHubOAuthProviderNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for GitHubOAuthProviderNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_github_provider_oauth",
            "GitHub (OAuth)",
            "Connect to GitHub using OAuth Device Flow.",
            "Data/GitHub",
        );
        node.set_flowscript_name("github", "providerOauth");
        node.add_icon("/flow/icons/github.svg");

        node.add_input_pin(
            "base_url",
            "API Base URL",
            "GitHub API base URL. Use 'https://api.github.com' for github.com or 'https://your-enterprise.com/api/v3' for Enterprise",
            VariableType::String,
        )
        .set_default_value(Some(json!("https://api.github.com")));

        node.add_output_pin(
            "provider",
            "Provider",
            "GitHub provider with authentication",
            VariableType::Struct,
        )
        .set_schema::<GitHubProvider>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        // Add OAuth provider reference - full config comes from Hub
        node.add_oauth_provider(GITHUB_PROVIDER_ID);
        node.add_required_oauth_scopes(GITHUB_PROVIDER_ID, vec!["repo", "read:user", "read:org"]);

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
        let base_url: String = context
            .evaluate_pin("base_url")
            .await
            .unwrap_or_else(|_| "https://api.github.com".to_string());

        let token = context
            .get_oauth_token(GITHUB_PROVIDER_ID)
            .ok_or_else(|| {
                flow_like_types::anyhow!(
                    "GitHub not authenticated. Please authorize access when prompted."
                )
            })?
            .clone();

        let provider = GitHubProvider {
            provider_id: GITHUB_PROVIDER_ID.to_string(),
            access_token: token.access_token,
            base_url,
        };

        context.set_pin_value("provider", json!(provider)).await?;

        Ok(())
    }
}

// =============================================================================
// GitHub App Installation Token Provider
// =============================================================================

#[crate::register_node]
#[derive(Default)]
pub struct GitHubAppProviderNode {}

impl GitHubAppProviderNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for GitHubAppProviderNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_github_provider_app",
            "GitHub (App Token)",
            "Connect to GitHub using a GitHub App installation token. Use this for server-to-server authentication.",
            "Data/GitHub",
        );
        node.set_flowscript_name("github", "providerApp");
        node.add_icon("/flow/icons/github.svg");

        node.add_input_pin(
            "installation_token",
            "Installation Token",
            "GitHub App installation access token",
            VariableType::String,
        )
        .set_options(PinOptions::new().set_sensitive(true).build());

        node.add_input_pin(
            "base_url",
            "API Base URL",
            "GitHub API base URL. Use 'https://api.github.com' for github.com or 'https://your-enterprise.com/api/v3' for Enterprise",
            VariableType::String,
        )
        .set_default_value(Some(json!("https://api.github.com")));

        node.add_output_pin(
            "provider",
            "Provider",
            "GitHub provider with authentication",
            VariableType::Struct,
        )
        .set_schema::<GitHubProvider>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.set_scores(
            NodeScores::new()
                .set_privacy(6)
                .set_security(9)
                .set_performance(9)
                .set_governance(8)
                .set_reliability(9)
                .set_cost(10)
                .build(),
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let token: String = context.evaluate_pin("installation_token").await?;
        let base_url: String = context
            .evaluate_pin("base_url")
            .await
            .unwrap_or_else(|_| "https://api.github.com".to_string());

        if token.is_empty() {
            return Err(flow_like_types::anyhow!("Installation token is required"));
        }

        let provider = GitHubProvider {
            provider_id: GITHUB_PROVIDER_ID.to_string(),
            access_token: token,
            base_url,
        };

        context.set_pin_value("provider", json!(provider)).await?;

        Ok(())
    }
}
