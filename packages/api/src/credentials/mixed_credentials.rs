use super::{CredentialsAccess, RuntimeCredentials, RuntimeCredentialsTrait};
use crate::state::{AppState, State};
use flow_like::credentials::mixed_credentials::MixedSharedCredentials;
use flow_like::credentials::SharedCredentials;
use flow_like::state::FlowLikeState;
use flow_like_storage::lancedb::connection::ConnectBuilder;
use flow_like_types::{Result, anyhow, async_trait};
use serde::{Deserialize, Serialize};

/// Runtime credentials that allow a different cloud provider per bucket type.
///
/// Created automatically when `META_BUCKET_PROVIDER`, `CONTENT_BUCKET_PROVIDER`,
/// or `LOGS_BUCKET_PROVIDER` environment variables specify differing providers.
/// Each inner [`RuntimeCredentials`] is scoped independently.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MixedRuntimeCredentials {
    pub meta: Box<RuntimeCredentials>,
    pub content: Box<RuntimeCredentials>,
    pub logs: Box<RuntimeCredentials>,
}

impl MixedRuntimeCredentials {
    /// Detect mixed provider configuration from environment variables.
    ///
    /// Returns `Some` only when per-bucket providers actually differ.
    /// If all providers are the same (or no overrides are set), returns `None`
    /// so the legacy single-provider codepath is used — preserving backwards
    /// compatibility for old clients.
    pub fn detect_from_env() -> Option<Self> {
        let meta_provider = std::env::var("META_BUCKET_PROVIDER").ok();
        let content_provider = std::env::var("CONTENT_BUCKET_PROVIDER").ok();
        let logs_provider = std::env::var("LOGS_BUCKET_PROVIDER").ok();

        if meta_provider.is_none() && content_provider.is_none() && logs_provider.is_none() {
            return None;
        }

        let default = default_provider_name();
        let meta = meta_provider.as_deref().unwrap_or(default);
        let content = content_provider.as_deref().unwrap_or(default);
        let logs = logs_provider.as_deref().unwrap_or(default);

        if meta == content && content == logs {
            return None;
        }

        let meta_creds = provider_to_runtime_credentials(meta).ok()?;
        let content_creds = provider_to_runtime_credentials(content).ok()?;
        let logs_creds = provider_to_runtime_credentials(logs).ok()?;

        tracing::info!(
            meta_provider = meta,
            content_provider = content,
            logs_provider = logs,
            "Mixed credential mode detected"
        );

        Some(Self {
            meta: Box::new(meta_creds),
            content: Box::new(content_creds),
            logs: Box::new(logs_creds),
        })
    }

    pub async fn master_credentials(&self) -> Result<Self> {
        let meta = master_inner(&self.meta).await?;
        let content = master_inner(&self.content).await?;
        let logs = master_inner(&self.logs).await?;

        Ok(Self {
            meta: Box::new(meta),
            content: Box::new(content),
            logs: Box::new(logs),
        })
    }

    pub async fn scoped_credentials(
        &self,
        sub: &str,
        app_id: &str,
        state: &State,
        mode: CredentialsAccess,
    ) -> Result<Self> {
        use flow_like_types::tokio;

        let (meta, content, logs) = tokio::join!(
            scope_inner(&self.meta, sub, app_id, state, mode.clone()),
            scope_inner(&self.content, sub, app_id, state, mode.clone()),
            scope_inner(&self.logs, sub, app_id, state, mode),
        );

        Ok(Self {
            meta: Box::new(meta?),
            content: Box::new(content?),
            logs: Box::new(logs?),
        })
    }
}

#[async_trait]
impl RuntimeCredentialsTrait for MixedRuntimeCredentials {
    fn into_shared_credentials(&self) -> SharedCredentials {
        SharedCredentials::Mixed(MixedSharedCredentials {
            meta: Box::new(self.meta.into_shared_credentials()),
            content: Box::new(self.content.into_shared_credentials()),
            logs: Box::new(self.logs.into_shared_credentials()),
        })
    }

    async fn to_db(&self, app_id: &str) -> Result<ConnectBuilder> {
        self.content.to_db(app_id).await
    }

    async fn to_db_scoped(&self, sub: &str, app_id: &str) -> Result<ConnectBuilder> {
        self.content.to_db_scoped(sub, app_id).await
    }

    async fn to_state(&self, state: AppState) -> Result<FlowLikeState> {
        // Delegate to the content credential for the full state setup (content store,
        // all DB builders, model provider, node registry). Then replace the meta
        // store with the one from the meta credential.
        let flow_like_state = self.content.to_state(state).await?;

        let meta_shared = self.meta.into_shared_credentials();
        let meta_store = meta_shared.to_store(true).await?;

        {
            let mut config = flow_like_state.config.write().await;
            config.register_app_meta_store(meta_store);
        }

        Ok(flow_like_state)
    }
}

async fn scope_inner(
    creds: &RuntimeCredentials,
    sub: &str,
    app_id: &str,
    state: &State,
    mode: CredentialsAccess,
) -> Result<RuntimeCredentials> {
    match creds {
        #[cfg(feature = "aws")]
        RuntimeCredentials::Aws(aws) => Ok(RuntimeCredentials::Aws(
            aws.scoped_credentials(sub, app_id, state, mode).await?,
        )),
        #[cfg(feature = "azure")]
        RuntimeCredentials::Azure(azure) => Ok(RuntimeCredentials::Azure(
            azure.scoped_credentials(sub, app_id, state, mode).await?,
        )),
        #[cfg(feature = "gcp")]
        RuntimeCredentials::Gcp(gcp) => Ok(RuntimeCredentials::Gcp(
            gcp.scoped_credentials(sub, app_id, state, mode).await?,
        )),
        #[cfg(feature = "r2")]
        RuntimeCredentials::R2(r2) => Ok(RuntimeCredentials::R2(
            r2.scoped_credentials(sub, app_id, state, mode).await?,
        )),
        RuntimeCredentials::Mixed(_) => {
            Err(anyhow!("Nested mixed credentials are not supported"))
        }
    }
}

async fn master_inner(creds: &RuntimeCredentials) -> Result<RuntimeCredentials> {
    match creds {
        #[cfg(feature = "aws")]
        RuntimeCredentials::Aws(aws) => {
            Ok(RuntimeCredentials::Aws(aws.master_credentials().await))
        }
        #[cfg(feature = "azure")]
        RuntimeCredentials::Azure(azure) => {
            Ok(RuntimeCredentials::Azure(azure.master_credentials().await))
        }
        #[cfg(feature = "gcp")]
        RuntimeCredentials::Gcp(gcp) => {
            Ok(RuntimeCredentials::Gcp(gcp.master_credentials().await))
        }
        #[cfg(feature = "r2")]
        RuntimeCredentials::R2(r2) => {
            Ok(RuntimeCredentials::R2(r2.master_credentials().await))
        }
        RuntimeCredentials::Mixed(_) => {
            Err(anyhow!("Nested mixed credentials are not supported"))
        }
    }
}

/// Default provider based on compile-time feature flags (same priority as
/// `RuntimeCredentials::master_credentials`).
fn default_provider_name() -> &'static str {
    #[cfg(feature = "r2")]
    return "r2";

    #[cfg(all(feature = "aws", not(feature = "r2")))]
    return "aws";

    #[cfg(all(feature = "azure", not(feature = "aws"), not(feature = "r2")))]
    return "azure";

    #[cfg(all(
        feature = "gcp",
        not(feature = "aws"),
        not(feature = "azure"),
        not(feature = "r2")
    ))]
    return "gcp";

    #[cfg(not(any(feature = "aws", feature = "azure", feature = "gcp", feature = "r2")))]
    return "none";
}

fn provider_to_runtime_credentials(provider: &str) -> Result<RuntimeCredentials> {
    match provider {
        #[cfg(feature = "r2")]
        "r2" => {
            use super::r2_credentials::R2RuntimeCredentials;
            Ok(RuntimeCredentials::R2(R2RuntimeCredentials::from_env()))
        }
        #[cfg(feature = "aws")]
        "aws" | "s3" => {
            use super::aws_credentials::AwsRuntimeCredentials;
            Ok(RuntimeCredentials::Aws(AwsRuntimeCredentials::from_env()))
        }
        #[cfg(feature = "azure")]
        "azure" => {
            use super::azure_credentials::AzureRuntimeCredentials;
            Ok(RuntimeCredentials::Azure(
                AzureRuntimeCredentials::from_env(),
            ))
        }
        #[cfg(feature = "gcp")]
        "gcp" => {
            use super::gcp_credentials::GcpRuntimeCredentials;
            Ok(RuntimeCredentials::Gcp(GcpRuntimeCredentials::from_env()))
        }
        other => Err(anyhow!(
            "Unknown or unsupported provider '{}' (check feature flags)",
            other
        )),
    }
}
