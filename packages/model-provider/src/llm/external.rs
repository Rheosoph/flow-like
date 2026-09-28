//! Model providers backed by personal agent accounts or a local coding CLI.

use std::{sync::Arc, time::Duration};

use anyhow::{Result, bail};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::provider::ModelProvider;

use super::{ModelLogic, claude_code::ClaudeCodeModel, subscription};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum ExternalProvider {
    ClaudeCode,
    Codex,
    GithubCopilot,
    MicrosoftCopilot,
}

impl ExternalProvider {
    pub fn from_provider_name(name: &str) -> Option<Self> {
        match name.trim().to_ascii_lowercase().as_str() {
            "custom:claude-code" => Some(Self::ClaudeCode),
            "custom:codex" => Some(Self::Codex),
            "custom:github-copilot" => Some(Self::GithubCopilot),
            "custom:microsoft-copilot" => Some(Self::MicrosoftCopilot),
            _ => None,
        }
    }

    pub fn provider_name(self) -> &'static str {
        match self {
            Self::ClaudeCode => "custom:claude-code",
            Self::Codex => "custom:codex",
            Self::GithubCopilot => "custom:github-copilot",
            Self::MicrosoftCopilot => "custom:microsoft-copilot",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::ClaudeCode => "Claude Code",
            Self::Codex => "Codex (ChatGPT)",
            Self::GithubCopilot => "GitHub Copilot",
            Self::MicrosoftCopilot => "Microsoft 365 Copilot",
        }
    }

    pub fn supports_tools(self) -> bool {
        !matches!(self, Self::MicrosoftCopilot)
    }

    pub fn supports_vision(self) -> bool {
        matches!(self, Self::Codex | Self::GithubCopilot)
    }

    /// Codex can use the executing user's CLI credentials. Other HTTP
    /// providers require credentials carried by the user's private Bit.
    pub fn requires_local_credentials(self, provider: &ModelProvider) -> bool {
        self == Self::ClaudeCode
            || (self == Self::Codex
                && !provider.params.as_ref().is_some_and(|params| {
                    params
                        .get("access_token")
                        .and_then(|value| value.as_str())
                        .is_some_and(|value| !value.trim().is_empty())
                }))
    }
}

/// A bounded, non-interactive readiness check. It does not generate tokens
/// and cannot guarantee that quota or network access will remain available.
pub async fn check_available(provider: &ModelProvider, allow_local: bool) -> Result<()> {
    let Some(kind) = ExternalProvider::from_provider_name(&provider.provider_name) else {
        return Ok(());
    };
    ensure_allowed(kind, provider, allow_local)?;
    tokio::time::timeout(Duration::from_secs(8), async {
        match kind {
            ExternalProvider::ClaudeCode => super::claude_code::check_available(provider).await,
            _ => subscription::check_available(provider).await,
        }
    })
    .await
    .map_err(|_| anyhow::anyhow!("{} availability check timed out", kind.label()))?
}

/// Called for each invocation so changed credentials and local availability
/// are not hidden by the global model cache.
pub async fn build(provider: &ModelProvider, allow_local: bool) -> Result<Arc<dyn ModelLogic>> {
    let kind = ExternalProvider::from_provider_name(&provider.provider_name)
        .ok_or_else(|| anyhow::anyhow!("Unknown external model provider"))?;
    ensure_allowed(kind, provider, allow_local)?;
    match kind {
        ExternalProvider::ClaudeCode => {
            check_available(provider, allow_local).await?;
            Ok(Arc::new(ClaudeCodeModel::from_provider(provider).await?))
        }
        _ => subscription::build(provider).await,
    }
}

fn ensure_allowed(
    kind: ExternalProvider,
    provider: &ModelProvider,
    allow_local: bool,
) -> Result<()> {
    if kind.requires_local_credentials(provider)
        && (!allow_local
            || cfg!(any(
                target_arch = "wasm32",
                target_os = "ios",
                target_os = "android"
            )))
    {
        bail!("{} requires a configured desktop runtime", kind.label());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn provider(kind: ExternalProvider) -> ModelProvider {
        ModelProvider {
            provider_name: kind.provider_name().into(),
            model_id: Some("model".into()),
            version: None,
            api_surface: None,
            params: None,
        }
    }

    #[test]
    fn local_auth_is_separate_from_remote_inference() {
        let mut codex = provider(ExternalProvider::Codex);
        assert!(ExternalProvider::Codex.requires_local_credentials(&codex));
        codex.params = Some(std::collections::HashMap::from([(
            "access_token".into(),
            serde_json::json!("token"),
        )]));
        assert!(!ExternalProvider::Codex.requires_local_credentials(&codex));
        assert!(
            ExternalProvider::ClaudeCode
                .requires_local_credentials(&provider(ExternalProvider::ClaudeCode))
        );
        assert!(!ExternalProvider::MicrosoftCopilot.supports_tools());
    }

    #[tokio::test]
    async fn remote_hosts_never_probe_local_credentials_or_executables() {
        for kind in [ExternalProvider::ClaudeCode, ExternalProvider::Codex] {
            let error = check_available(&provider(kind), false).await.unwrap_err();
            assert!(error.to_string().contains("desktop runtime"));
        }
    }
}
