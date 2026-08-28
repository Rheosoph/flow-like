//! Mints channel credentials: the HTTP responder token every transport falls back to and, for
//! the configured cloud transport, the maximally downscoped client / executor grants.

use std::sync::Arc;

use flow_like_channels::ChannelForwarder;
use flow_like_types::channel::{
    ChannelClientDescriptor, ChannelExecutorGrant, ChannelGrant, ChannelHandle, clamp_ttl, now_unix,
};
use flow_like_types::{Result, anyhow};
use std::time::Duration;

use crate::execution::{ChannelJwtParams, sign_channel_responder};

use super::{ChannelBackend, api_base_url, push_url};

/// Everything the API needs to hand a run and its client a channel. Held on `State.channels`
/// and by the dispatcher.
pub struct ChannelIssuer {
    api_base_url: String,
    backend: ChannelBackend,
    forwarder: Option<Arc<dyn ChannelForwarder>>,
    #[cfg(feature = "channel-aws")]
    aws: Option<super::aws::AwsChannelRuntime>,
    #[cfg(feature = "channel-azure")]
    azure: Option<super::azure::AzureChannelRuntime>,
    #[cfg(feature = "channel-gcp")]
    gcp: Option<super::gcp::GcpChannelRuntime>,
}

/// One side of a cloud grant: the descriptor plus when its credential stops working.
pub struct MintedDescriptor {
    pub descriptor: ChannelClientDescriptor,
    pub expires_at: i64,
}

pub struct MintedExecutor {
    pub grant: ChannelExecutorGrant,
    pub expires_at: i64,
}

impl ChannelIssuer {
    /// HTTP-only issuer: what every deployment gets without `CHANNEL_TRANSPORT`.
    pub fn http_only(api_base_url: impl Into<String>) -> Self {
        Self {
            api_base_url: api_base_url.into(),
            backend: ChannelBackend::Http,
            forwarder: None,
            #[cfg(feature = "channel-aws")]
            aws: None,
            #[cfg(feature = "channel-azure")]
            azure: None,
            #[cfg(feature = "channel-gcp")]
            gcp: None,
        }
    }

    /// Build from `CHANNEL_TRANSPORT` and the transport's `CHANNEL_*` variables / secrets. A
    /// transport that fails to configure logs the reason and degrades to HTTP; cloud binaries
    /// validate the same variables at boot so this only happens on hosts that opted out of
    /// that validation.
    #[allow(unused_variables)]
    pub async fn from_env(
        secrets: &flow_like_secrets::SecretStore,
        #[cfg(feature = "aws")] aws_config: Arc<aws_config::SdkConfig>,
    ) -> Self {
        let mut issuer = Self::http_only(api_base_url());
        let backend = ChannelBackend::from_env();
        match backend {
            ChannelBackend::Http => {}
            #[cfg(feature = "channel-aws")]
            ChannelBackend::AwsMqtt => {
                match super::aws::AwsChannelRuntime::from_env(aws_config).await {
                    Ok(runtime) => {
                        issuer.forwarder = Some(runtime.forwarder());
                        issuer.aws = Some(runtime);
                        issuer.backend = backend;
                    }
                    Err(error) => {
                        tracing::error!(%error, "aws_mqtt channel transport unavailable; using http")
                    }
                }
            }
            #[cfg(feature = "channel-azure")]
            ChannelBackend::AzureWebPubSub => {
                match super::azure::AzureChannelRuntime::from_env(secrets).await {
                    Ok(runtime) => {
                        issuer.forwarder = Some(runtime.forwarder());
                        issuer.azure = Some(runtime);
                        issuer.backend = backend;
                    }
                    Err(error) => {
                        tracing::error!(%error, "azure_web_pubsub channel transport unavailable; using http")
                    }
                }
            }
            #[cfg(feature = "channel-gcp")]
            ChannelBackend::GcpFirebaseRtdb => {
                match super::gcp::GcpChannelRuntime::from_env(secrets).await {
                    Ok(runtime) => {
                        issuer.forwarder = Some(runtime.forwarder());
                        issuer.gcp = Some(runtime);
                        issuer.backend = backend;
                    }
                    Err(error) => {
                        tracing::error!(%error, "gcp_firebase_rtdb channel transport unavailable; using http")
                    }
                }
            }
        }
        tracing::info!(transport = issuer.backend.transport(), "Channel transport");
        issuer
    }

    pub fn backend(&self) -> &ChannelBackend {
        &self.backend
    }

    pub fn transport(&self) -> &'static str {
        self.backend.transport()
    }

    pub fn forwarder(&self) -> Option<&Arc<dyn ChannelForwarder>> {
        self.forwarder.as_ref()
    }

    fn ttl(ttl_secs: i64) -> i64 {
        clamp_ttl(Duration::from_secs(ttl_secs.max(0) as u64)).as_secs() as i64
    }

    /// Responder token bound to `(channel_id, sub)` whose `transport` claim names where the
    /// waiter listens, so the push endpoint knows whether to flip a row or forward.
    fn responder_token(
        &self,
        channel_id: &str,
        sub: &str,
        app_id: Option<&str>,
        transport: &str,
        ttl_secs: i64,
    ) -> Result<String> {
        sign_channel_responder(ChannelJwtParams {
            sub: sub.to_string(),
            channel_id: channel_id.to_string(),
            app_id: app_id.map(str::to_string),
            transport: transport.to_string(),
            ttl_seconds: Some(ttl_secs),
        })
        .map_err(|e| anyhow!("failed to sign channel responder token: {e}"))
    }

    fn http_descriptor(
        &self,
        channel_id: &str,
        sub: &str,
        app_id: Option<&str>,
        transport: &str,
        ttl_secs: i64,
    ) -> Result<ChannelClientDescriptor> {
        Ok(ChannelClientDescriptor::Http {
            push_url: push_url(&self.api_base_url, channel_id),
            token: self.responder_token(channel_id, sub, app_id, transport, ttl_secs)?,
        })
    }

    /// The HTTP handle: replies flip a `Channel` row the waiter polls.
    pub fn http_handle(
        &self,
        channel_id: &str,
        sub: &str,
        app_id: Option<&str>,
        ttl_secs: i64,
    ) -> Result<ChannelHandle> {
        let ttl_secs = Self::ttl(ttl_secs);
        Ok(ChannelHandle {
            channel_id: channel_id.to_string(),
            request_id: None,
            expires_at: now_unix() + ttl_secs,
            transport: self.http_descriptor(
                channel_id,
                sub,
                app_id,
                ChannelBackend::Http.transport(),
                ttl_secs,
            )?,
            fallback: None,
        })
    }

    /// Grant whose waiter polls rows through the API; valid on every deployment.
    pub fn http_grant(
        &self,
        channel_id: &str,
        sub: &str,
        app_id: Option<&str>,
        ttl_secs: i64,
    ) -> Result<ChannelGrant> {
        let client = self.http_handle(channel_id, sub, app_id, ttl_secs)?;
        Ok(ChannelGrant {
            channel_id: channel_id.to_string(),
            expires_at: client.expires_at,
            executor: ChannelExecutorGrant::Http {},
            client,
        })
    }

    /// The push endpoint as fallback for a cloud handle: its token names the cloud transport
    /// so a push landing there is forwarded to the waiter's connection.
    fn fallback_descriptor(
        &self,
        channel_id: &str,
        sub: &str,
        app_id: Option<&str>,
        ttl_secs: i64,
    ) -> Result<ChannelClientDescriptor> {
        self.http_descriptor(channel_id, sub, app_id, self.transport(), ttl_secs)
    }

    #[allow(unused_variables)]
    async fn mint_client(
        &self,
        channel_id: &str,
        sub: &str,
        ttl_secs: i64,
    ) -> Result<Option<MintedDescriptor>> {
        match &self.backend {
            ChannelBackend::Http => Ok(None),
            #[cfg(feature = "channel-aws")]
            ChannelBackend::AwsMqtt => self
                .aws
                .as_ref()
                .ok_or_else(|| {
                    anyhow!("aws_mqtt channel transport is selected but not configured")
                })?
                .client(channel_id, sub, ttl_secs)
                .await
                .map(Some),
            #[cfg(feature = "channel-azure")]
            ChannelBackend::AzureWebPubSub => self
                .azure
                .as_ref()
                .ok_or_else(|| {
                    anyhow!("azure_web_pubsub channel transport is selected but not configured")
                })?
                .client(channel_id, sub, ttl_secs)
                .map(Some),
            #[cfg(feature = "channel-gcp")]
            ChannelBackend::GcpFirebaseRtdb => self
                .gcp
                .as_ref()
                .ok_or_else(|| {
                    anyhow!("gcp_firebase_rtdb channel transport is selected but not configured")
                })?
                .client(channel_id, sub, ttl_secs)
                .map(Some),
        }
    }

    #[allow(unused_variables)]
    async fn mint_executor(
        &self,
        channel_id: &str,
        sub: &str,
        ttl_secs: i64,
    ) -> Result<Option<MintedExecutor>> {
        match &self.backend {
            ChannelBackend::Http => Ok(None),
            #[cfg(feature = "channel-aws")]
            ChannelBackend::AwsMqtt => self
                .aws
                .as_ref()
                .ok_or_else(|| {
                    anyhow!("aws_mqtt channel transport is selected but not configured")
                })?
                .executor(channel_id, ttl_secs)
                .await
                .map(Some),
            #[cfg(feature = "channel-azure")]
            ChannelBackend::AzureWebPubSub => self
                .azure
                .as_ref()
                .ok_or_else(|| {
                    anyhow!("azure_web_pubsub channel transport is selected but not configured")
                })?
                .executor(channel_id, ttl_secs)
                .map(Some),
            #[cfg(feature = "channel-gcp")]
            ChannelBackend::GcpFirebaseRtdb => self
                .gcp
                .as_ref()
                .ok_or_else(|| {
                    anyhow!("gcp_firebase_rtdb channel transport is selected but not configured")
                })?
                .executor(channel_id, sub, ttl_secs)
                .await
                .map(Some),
        }
    }

    /// Client handle for the configured transport (HTTP handle when that is the transport),
    /// with the push endpoint as fallback.
    pub async fn client_handle(
        &self,
        channel_id: &str,
        sub: &str,
        app_id: Option<&str>,
        ttl_secs: i64,
    ) -> Result<ChannelHandle> {
        let ttl_secs = Self::ttl(ttl_secs);
        let Some(minted) = self.mint_client(channel_id, sub, ttl_secs).await? else {
            return self.http_handle(channel_id, sub, app_id, ttl_secs);
        };
        let fallback = self.fallback_descriptor(channel_id, sub, app_id, ttl_secs)?;
        Ok(ChannelHandle {
            channel_id: channel_id.to_string(),
            request_id: None,
            expires_at: minted.expires_at.min(now_unix() + ttl_secs),
            transport: minted.descriptor,
            fallback: Some(fallback),
        })
    }

    /// Full grant for the configured transport: executor credentials plus the client handle
    /// forwarded inside every request. `expires_at` is the earliest credential expiry.
    pub async fn grant(
        &self,
        channel_id: &str,
        sub: &str,
        app_id: Option<&str>,
        ttl_secs: i64,
    ) -> Result<ChannelGrant> {
        let ttl_secs = Self::ttl(ttl_secs);
        let Some(executor) = self.mint_executor(channel_id, sub, ttl_secs).await? else {
            return self.http_grant(channel_id, sub, app_id, ttl_secs);
        };
        let client = self
            .client_handle(channel_id, sub, app_id, ttl_secs)
            .await?;
        let expires_at = executor.expires_at.min(client.expires_at);
        Ok(ChannelGrant {
            channel_id: channel_id.to_string(),
            expires_at,
            executor: executor.grant,
            client: ChannelHandle {
                expires_at,
                ..client
            },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::execution::verify_channel_responder;
    use flow_like_types::channel::CHANNEL_TRANSPORT_HTTP;

    #[tokio::test]
    async fn http_grant_carries_a_bound_responder_token() {
        crate::backend_jwt::init_for_tests();
        let issuer = ChannelIssuer::http_only("https://api.test");
        assert_eq!(issuer.transport(), CHANNEL_TRANSPORT_HTTP);
        assert!(issuer.forwarder().is_none());

        let grant = issuer
            .grant("run-1", "user-1", Some("app-1"), 300)
            .await
            .unwrap();
        assert_eq!(grant.channel_id, "run-1");
        assert_eq!(grant.executor, ChannelExecutorGrant::Http {});
        assert!(grant.client.fallback.is_none());
        assert!(grant.client.request_id.is_none());
        assert!((grant.expires_at - now_unix() - 300).abs() <= 2);

        let ChannelClientDescriptor::Http { push_url, token } = &grant.client.transport else {
            panic!("http grant must carry an http descriptor");
        };
        assert_eq!(push_url, "https://api.test/api/v1/channels/run-1/push");
        let claims = verify_channel_responder(token).unwrap();
        assert_eq!(claims.channel_id, "run-1");
        assert_eq!(claims.sub, "user-1");
        assert_eq!(claims.app_id.as_deref(), Some("app-1"));
        assert_eq!(claims.transport, CHANNEL_TRANSPORT_HTTP);
        assert!((claims.exp - now_unix() - 300).abs() <= 2);
    }

    #[tokio::test]
    async fn ttl_is_clamped() {
        crate::backend_jwt::init_for_tests();
        let issuer = ChannelIssuer::http_only("https://api.test");
        let handle = issuer.http_handle("run-1", "user-1", None, 0).unwrap();
        assert!(handle.expires_at > now_unix());
        let handle = issuer
            .http_handle("run-1", "user-1", None, 100 * 60 * 60)
            .unwrap();
        assert!(handle.expires_at <= now_unix() + 9 * 60 * 60 + 1);
        let client = issuer
            .client_handle("run-1", "user-1", None, 60)
            .await
            .unwrap();
        assert!(matches!(
            client.transport,
            ChannelClientDescriptor::Http { .. }
        ));
        assert!(client.fallback.is_none());
    }
}
