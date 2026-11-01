//! Mints channel credentials: clients push through the API using a responder token, while
//! executors receive scoped grants for the configured provider.

use std::sync::Arc;

use flow_like_channels::ChannelForwarder;
use flow_like_types::channel::{
    ChannelClientDescriptor, ChannelExecutorGrant, ChannelGrant, ChannelHandle, clamp_ttl, now_unix,
};
use flow_like_types::tokio::{spawn, time::timeout};
use flow_like_types::{Result, anyhow};
use futures_util::future::BoxFuture;
use std::time::{Duration, Instant};

use crate::execution::{ChannelJwtParams, sign_channel_responder};

use super::{ChannelBackend, api_base_url, push_url, redact_provider_error};

/// Per account, intermediate run wakeups and failure logs each go out at most once per window.
const REALTIME_WINDOW: Duration = Duration::from_millis(500);
/// How long a request waits for its wakeup; a slower publication finishes detached.
const REALTIME_PUBLISH_WAIT: Duration = Duration::from_millis(250);
const REALTIME_PUBLISH_TIMEOUT: Duration = Duration::from_secs(2);

#[derive(Default)]
struct RealtimeAccount {
    published_at: Option<Instant>,
    failure_logged_at: Option<Instant>,
}

type RealtimeAccountState = Arc<parking_lot::Mutex<RealtimeAccount>>;

/// Everything the API needs to hand a run and its client a channel. Held on `State.channels`
/// and by the dispatcher.
pub struct ChannelIssuer {
    api_base_url: String,
    backend: ChannelBackend,
    forwarder: Option<Arc<dyn ChannelForwarder>>,
    realtime_accounts: moka::sync::Cache<String, RealtimeAccountState>,
    #[cfg(feature = "channel-aws")]
    aws: Option<Arc<super::aws::AwsChannelRuntime>>,
    #[cfg(feature = "channel-azure")]
    azure: Option<Arc<super::azure::AzureChannelRuntime>>,
    #[cfg(feature = "channel-gcp")]
    gcp: Option<Arc<super::gcp::GcpChannelRuntime>>,
    #[cfg(feature = "channel-nats")]
    nats: Option<Arc<super::nats::NatsChannelRuntime>>,
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
            realtime_accounts: moka::sync::Cache::builder()
                .max_capacity(10_000)
                .time_to_idle(Duration::from_secs(60))
                .build(),
            #[cfg(feature = "channel-aws")]
            aws: None,
            #[cfg(feature = "channel-azure")]
            azure: None,
            #[cfg(feature = "channel-gcp")]
            gcp: None,
            #[cfg(feature = "channel-nats")]
            nats: None,
        }
    }

    /// Build from `CHANNEL_TRANSPORT` and the transport's `CHANNEL_*` variables / secrets. A
    /// transport that fails to configure logs the reason and degrades to HTTP; cloud binaries
    /// validate the same variables at boot so this only happens on hosts that opted out of
    /// that validation.
    #[allow(unused_variables, unused_mut)]
    pub async fn from_env(
        secrets: &flow_like_secrets::SecretStore,
        #[cfg(feature = "aws")] aws_config: Arc<aws_config::SdkConfig>,
    ) -> Self {
        let mut issuer = Self::http_only(api_base_url());
        let backend = ChannelBackend::from_env();
        match backend {
            ChannelBackend::Http => {}
            #[cfg(feature = "channel-nats")]
            ChannelBackend::Nats => {
                match super::nats::NatsChannelRuntime::from_env(secrets).await {
                    Ok(runtime) => {
                        issuer.forwarder = Some(runtime.forwarder());
                        issuer.nats = Some(Arc::new(runtime));
                        issuer.backend = backend;
                    }
                    Err(error) => {
                        tracing::error!(%error, "nats channel transport unavailable; using http")
                    }
                }
            }
            #[cfg(feature = "channel-aws")]
            ChannelBackend::AwsMqtt => {
                match super::aws::AwsChannelRuntime::from_env(aws_config).await {
                    Ok(runtime) => {
                        issuer.forwarder = Some(runtime.forwarder());
                        issuer.aws = Some(Arc::new(runtime));
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
                        issuer.azure = Some(Arc::new(runtime));
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
                        issuer.gcp = Some(Arc::new(runtime));
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

    /// Mint reception rights only. The authenticated account supplies the scope.
    #[allow(unused_variables)]
    pub async fn realtime_connect(
        &self,
        sub: &str,
        prefer_http: bool,
    ) -> Result<flow_like_types::realtime::RealtimeConnectResponse> {
        use flow_like_types::realtime::{
            REALTIME_PROTOCOL_VERSION, RealtimeConnectResponse, RealtimeTransport,
        };
        let ttl = 15 * 60;
        let expires_at = now_unix() + ttl;
        let scope = realtime_scope(sub);
        let transport = if prefer_http {
            RealtimeTransport::Http {}
        } else {
            match self.backend {
                ChannelBackend::Http => RealtimeTransport::Http {},
                #[cfg(feature = "channel-nats")]
                ChannelBackend::Nats => self
                    .nats
                    .as_ref()
                    .ok_or_else(|| anyhow!("NATS unavailable"))?
                    .realtime(&scope, ttl)?,
                #[cfg(feature = "channel-azure")]
                ChannelBackend::AzureWebPubSub => self
                    .azure
                    .as_ref()
                    .ok_or_else(|| anyhow!("Azure Web PubSub unavailable"))?
                    .realtime(&scope, ttl)?,
                #[cfg(feature = "channel-gcp")]
                ChannelBackend::GcpFirebaseRtdb => self
                    .gcp
                    .as_ref()
                    .ok_or_else(|| anyhow!("Firebase unavailable"))?
                    .realtime(&scope, ttl)?,
                #[cfg(feature = "channel-aws")]
                ChannelBackend::AwsMqtt => {
                    self.aws
                        .as_ref()
                        .ok_or_else(|| anyhow!("AWS IoT unavailable"))?
                        .realtime(&scope, ttl)
                        .await?
                }
            }
        };
        let expires_at = match &transport {
            RealtimeTransport::AwsMqtt { credentials, .. } => {
                credentials.expiration.min(expires_at)
            }
            _ => expires_at,
        };
        Ok(RealtimeConnectResponse {
            protocol_version: REALTIME_PROTOCOL_VERSION,
            expires_at,
            expires_in: None,
            max_poll_wait_ms: flow_like_types::realtime::MAX_REALTIME_TIMEOUT_MS,
            transport,
        })
    }

    /// Wakeups contain no event data and never determine whether a durable write succeeds.
    pub async fn publish_realtime_invalidation(&self, sub: &str) {
        self.publish_realtime_for_user(sub, false).await;
    }

    /// Intermediate run batches share a short throttle window. Terminal state bypasses it.
    pub async fn publish_realtime_run_invalidation(&self, sub: &str) {
        self.publish_realtime_for_user(sub, true).await;
    }

    async fn publish_realtime_for_user(&self, sub: &str, throttle: bool) {
        use flow_like_types::realtime::{REALTIME_PROTOCOL_VERSION, RealtimeWakeup};
        if matches!(self.backend, ChannelBackend::Http) || sub.is_empty() {
            return;
        }
        let scope = realtime_scope(sub);
        let account = self
            .realtime_accounts
            .get_with(scope.clone(), Default::default);
        if !admit(&account, throttle, Instant::now()) {
            return;
        }
        let wakeup = RealtimeWakeup {
            protocol_version: REALTIME_PROTOCOL_VERSION,
            kind: "invalidate".into(),
            nonce: flow_like_types::channel::new_request_id(),
        };
        let Ok(payload) = serde_json::to_vec(&wakeup) else {
            return;
        };
        send_wakeup(
            self.realtime_publication(scope, payload),
            account,
            self.transport(),
        )
        .await;
    }

    /// The provider call owns its inputs so it can outlive the request that started it.
    #[allow(unused_variables)]
    fn realtime_publication(
        &self,
        scope: String,
        payload: Vec<u8>,
    ) -> BoxFuture<'static, Result<()>> {
        match self.backend {
            ChannelBackend::Http => Box::pin(async { Ok(()) }),
            #[cfg(feature = "channel-nats")]
            ChannelBackend::Nats => {
                let nats = self.nats.clone();
                Box::pin(async move {
                    nats.ok_or_else(|| anyhow!("NATS unavailable"))?
                        .publish_realtime(&scope, &payload)
                        .await
                })
            }
            #[cfg(feature = "channel-azure")]
            ChannelBackend::AzureWebPubSub => {
                let azure = self.azure.clone();
                Box::pin(async move {
                    azure
                        .ok_or_else(|| anyhow!("Azure Web PubSub unavailable"))?
                        .publish_realtime(&scope, &payload)
                        .await
                })
            }
            #[cfg(feature = "channel-gcp")]
            ChannelBackend::GcpFirebaseRtdb => {
                let gcp = self.gcp.clone();
                Box::pin(async move {
                    gcp.ok_or_else(|| anyhow!("Firebase unavailable"))?
                        .publish_realtime(&scope, &payload)
                        .await
                })
            }
            #[cfg(feature = "channel-aws")]
            ChannelBackend::AwsMqtt => {
                let aws = self.aws.clone();
                Box::pin(async move {
                    aws.ok_or_else(|| anyhow!("AWS IoT unavailable"))?
                        .publish_realtime(&scope, &payload)
                        .await
                })
            }
        }
    }

    /// Refresh the API endpoint while retaining the responder token and its original deadline.
    pub async fn refresh_client_handle(
        &self,
        claims: &crate::execution::ChannelClaims,
        responder_token: &str,
    ) -> Result<ChannelHandle> {
        if claims.exp <= now_unix() {
            return Err(anyhow!("channel has expired"));
        }
        if claims.transport != ChannelBackend::Http.transport()
            && claims.transport != self.transport()
        {
            return Err(anyhow!("channel transport no longer matches this API"));
        }
        Ok(ChannelHandle {
            channel_id: claims.channel_id.clone(),
            request_id: None,
            expires_at: claims.exp,
            transport: ChannelClientDescriptor::Http {
                push_url: push_url(&self.api_base_url, &claims.channel_id),
                token: responder_token.to_owned(),
            },
            fallback: None,
        })
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

    #[allow(unused_variables)]
    async fn mint_executor(
        &self,
        channel_id: &str,
        sub: &str,
        ttl_secs: i64,
    ) -> Result<Option<MintedExecutor>> {
        match &self.backend {
            ChannelBackend::Http => Ok(None),
            #[cfg(feature = "channel-nats")]
            ChannelBackend::Nats => self
                .nats
                .as_ref()
                .ok_or_else(|| anyhow!("nats channel transport is selected but not configured"))?
                .executor(channel_id, ttl_secs)
                .map(Some),
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

    /// Every remote client pushes through the API. The signed transport claim routes the
    /// push to the configured waiter without exposing provider write credentials.
    pub async fn client_handle(
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
            transport: self.http_descriptor(channel_id, sub, app_id, self.transport(), ttl_secs)?,
            fallback: None,
        })
    }

    /// Keep the executor on its configured provider and give the client an API capability.
    /// The handle and grant share the earliest deadline, including executor credential expiry.
    pub async fn grant(
        &self,
        channel_id: &str,
        sub: &str,
        app_id: Option<&str>,
        ttl_secs: i64,
    ) -> Result<ChannelGrant> {
        let ttl_secs = Self::ttl(ttl_secs);
        let (executor, client) = flow_like_types::tokio::join!(
            self.mint_executor(channel_id, sub, ttl_secs),
            self.client_handle(channel_id, sub, app_id, ttl_secs),
        );
        let Some(executor) = executor? else {
            return self.http_grant(channel_id, sub, app_id, ttl_secs);
        };
        let client = client?;
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

fn realtime_scope(sub: &str) -> String {
    let mut hash = blake3::Hasher::new_derive_key("flow-like realtime account scope v1");
    hash.update(sub.as_bytes());
    hash.finalize().to_hex().to_string()
}

/// Intermediate run batches share the account window and are dropped inside it; terminal and
/// notification wakeups always go out.
fn admit(account: &RealtimeAccountState, throttle: bool, now: Instant) -> bool {
    !throttle || claim_window(&mut account.lock().published_at, now)
}

fn claim_window(slot: &mut Option<Instant>, now: Instant) -> bool {
    if slot.is_some_and(|at| now.saturating_duration_since(at) < REALTIME_WINDOW) {
        return false;
    }
    *slot = Some(now);
    true
}

/// Lambda freezes detached work with its invocation, so the request gives the publication a
/// short head start. A slower publication finishes in the background within its own timeout;
/// clients recover dropped wakeups through HTTP replay.
async fn send_wakeup(
    publication: BoxFuture<'static, Result<()>>,
    account: RealtimeAccountState,
    transport: &'static str,
) {
    let task = spawn(async move {
        let error = match timeout(REALTIME_PUBLISH_TIMEOUT, publication).await {
            Ok(Ok(())) => return,
            Ok(Err(error)) => redact_provider_error(&error),
            Err(_) => format!("no answer within {REALTIME_PUBLISH_TIMEOUT:?}"),
        };
        if claim_window(&mut account.lock().failure_logged_at, Instant::now()) {
            tracing::warn!(
                transport,
                %error,
                "Realtime wakeup failed; clients recover through HTTP replay"
            );
        }
    });
    let _ = timeout(REALTIME_PUBLISH_WAIT, task).await;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::execution::verify_channel_responder;
    use flow_like_types::channel::CHANNEL_TRANSPORT_HTTP;

    #[test]
    fn realtime_topics_cannot_be_selected_through_account_identifiers() {
        let normal = realtime_scope("user-a");
        assert_eq!(normal, realtime_scope("user-a"));
        assert_ne!(normal, realtime_scope("user-b"));
        for sub in ["user-a", "other.*.>", "../other/#", "üser@example.test"] {
            let scope = realtime_scope(sub);
            assert_eq!(scope.len(), 64);
            assert!(scope.bytes().all(|byte| byte.is_ascii_hexdigit()));
        }
    }

    #[tokio::test]
    async fn http_realtime_grant_has_no_publish_credentials() {
        let issuer = ChannelIssuer::http_only("https://api.example.test");
        let grant = issuer.realtime_connect("user-a", false).await.unwrap();
        assert_eq!(grant.protocol_version, 1);
        assert!(matches!(
            grant.transport,
            flow_like_types::realtime::RealtimeTransport::Http {}
        ));
        assert!(grant.expires_at > now_unix());
        issuer.publish_realtime_invalidation("user-a").await;
        assert_eq!(issuer.realtime_accounts.entry_count(), 0);
    }

    #[test]
    fn intermediate_wakeups_share_a_window_and_terminal_ones_never_wait() {
        let account = RealtimeAccountState::default();
        let start = Instant::now();
        assert!(admit(&account, true, start));
        assert!(!admit(&account, true, start + Duration::from_millis(100)));
        assert!(admit(&account, false, start + Duration::from_millis(100)));
        assert!(admit(&account, false, start + Duration::from_millis(101)));
        assert!(!admit(&account, true, start + Duration::from_millis(499)));
        assert!(admit(&account, true, start + REALTIME_WINDOW));
        assert!(admit(&RealtimeAccountState::default(), true, start));
    }

    #[tokio::test]
    async fn a_hanging_provider_holds_the_request_only_for_the_wait_bound() {
        let started = Instant::now();
        send_wakeup(
            Box::pin(std::future::pending()),
            RealtimeAccountState::default(),
            "test",
        )
        .await;
        let waited = started.elapsed();
        assert!(
            waited >= REALTIME_PUBLISH_WAIT && waited < REALTIME_PUBLISH_TIMEOUT / 2,
            "{waited:?}"
        );
    }

    #[tokio::test]
    async fn failures_are_logged_once_per_window_per_account() {
        let failing = || Box::pin(async { Err::<(), _>(anyhow!("provider unavailable")) });
        let account = RealtimeAccountState::default();
        send_wakeup(Box::pin(async { Ok(()) }), account.clone(), "test").await;
        assert!(account.lock().failure_logged_at.is_none());

        send_wakeup(failing(), account.clone(), "test").await;
        let logged = account.lock().failure_logged_at;
        assert!(logged.is_some());
        send_wakeup(failing(), account.clone(), "test").await;
        assert_eq!(account.lock().failure_logged_at, logged);

        let other = RealtimeAccountState::default();
        send_wakeup(failing(), other.clone(), "test").await;
        assert!(other.lock().failure_logged_at.is_some());
    }

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

    #[tokio::test]
    async fn refresh_keeps_original_responder_token_and_deadline() {
        crate::backend_jwt::init_for_tests();
        let issuer = ChannelIssuer::http_only("https://api.test");
        let original = issuer.http_handle("run-1", "user-1", None, 60).unwrap();
        let ChannelClientDescriptor::Http { token, .. } = &original.transport else {
            unreachable!()
        };
        let claims = verify_channel_responder(token).unwrap();
        let refreshed = issuer.refresh_client_handle(&claims, token).await.unwrap();
        assert_eq!(refreshed.expires_at, claims.exp);
        assert_eq!(refreshed.transport, original.transport);
        let mut expired = claims;
        expired.exp = now_unix() - 1;
        assert!(issuer.refresh_client_handle(&expired, token).await.is_err());
    }

    #[tokio::test]
    async fn every_remote_client_uses_api_push_without_provider_credentials() {
        crate::backend_jwt::init_for_tests();
        for transport in [
            "http",
            "aws_mqtt",
            "azure_web_pubsub",
            "gcp_firebase_rtdb",
            "nats",
        ] {
            let Ok(backend) = ChannelBackend::parse(Some(transport)) else {
                continue;
            };
            // Provider runtimes deliberately remain absent. Client issuance must not need
            // STS, a Firebase key, a Web PubSub key, or a NATS account signer.
            let mut issuer = ChannelIssuer::http_only("https://api.test");
            issuer.backend = backend;
            let client = issuer
                .client_handle("run-1", "user-1", Some("app-1"), 120)
                .await
                .unwrap();
            let ChannelClientDescriptor::Http { push_url, token } = &client.transport else {
                panic!("remote clients must use the API");
            };
            assert_eq!(push_url, "https://api.test/api/v1/channels/run-1/push");
            assert!(client.fallback.is_none());
            let claims = verify_channel_responder(token).unwrap();
            assert_eq!(claims.transport, transport);
            assert_eq!(claims.channel_id, "run-1");
            assert_eq!(claims.sub, "user-1");
            assert_eq!(claims.app_id.as_deref(), Some("app-1"));
            assert!(client.expires_at <= claims.exp);

            let deadline = now_unix() + 30;
            let request = client.for_request("request-1", deadline);
            assert_eq!(request.request_id.as_deref(), Some("request-1"));
            assert_eq!(request.expires_at, deadline);
            assert_eq!(request.transport, client.transport);

            // A legacy provider handle's HTTP fallback carries this same capability.
            // Refreshing it retains identity and expiry, without renewing provider grants.
            let refreshed = issuer.refresh_client_handle(&claims, token).await.unwrap();
            assert_eq!(refreshed.transport, client.transport);
            assert_eq!(refreshed.channel_id, client.channel_id);
            assert_eq!(refreshed.expires_at, claims.exp);
            assert!(refreshed.fallback.is_none());
            assert!(refreshed.request_id.is_none());
        }
    }

    #[tokio::test]
    async fn refresh_cannot_redirect_a_provider_capability_to_an_http_waiter() {
        crate::backend_jwt::init_for_tests();
        let issuer = ChannelIssuer::http_only("https://api.test");
        let token = issuer
            .responder_token("run-1", "user-1", Some("app-1"), "nats", 60)
            .unwrap();
        let claims = verify_channel_responder(&token).unwrap();
        assert!(issuer.refresh_client_handle(&claims, &token).await.is_err());
    }
}
