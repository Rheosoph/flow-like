//! Mints channel credentials: each side of a run gets a grant scoped to that run on the
//! configured provider, and the client also gets a responder token for the API push endpoint,
//! which is its fallback.

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

/// Per account, intermediate run wakeups go out at most once per window; one arriving inside it
/// is published when the window ends.
const REALTIME_WINDOW: Duration = Duration::from_millis(250);
/// How long a request waits for its wakeup; a slower publication finishes detached.
const REALTIME_PUBLISH_WAIT: Duration = Duration::from_millis(250);
const REALTIME_PUBLISH_TIMEOUT: Duration = Duration::from_secs(2);
/// A provider outage fails every account's wakeups at once; the transport warns once per window.
const FAILURE_LOG_WINDOW: Duration = Duration::from_secs(1);

#[derive(Default)]
struct RealtimeAccount {
    published_at: Option<Instant>,
    trailing_until: Option<Instant>,
}

type RealtimeAccountState = Arc<parking_lot::Mutex<RealtimeAccount>>;

#[derive(Default)]
struct FailureWindow {
    warned_at: Option<Instant>,
    suppressed: u64,
}

/// Wakeup failure warnings for the issuer's transport, shared by all accounts.
#[derive(Default)]
struct FailureLog(parking_lot::Mutex<FailureWindow>);

impl FailureLog {
    /// When this failure may warn, the number of failures suppressed since the last warning.
    fn claim(&self, now: Instant) -> Option<u64> {
        let mut window = self.0.lock();
        if !claim_window(&mut window.warned_at, now, FAILURE_LOG_WINDOW) {
            window.suppressed += 1;
            return None;
        }
        Some(std::mem::take(&mut window.suppressed))
    }
}

/// Everything the API needs to hand a run and its client a channel. Held on `State.channels`
/// and by the dispatcher.
pub struct ChannelIssuer {
    api_base_url: String,
    backend: ChannelBackend,
    forwarder: Option<Arc<dyn ChannelForwarder>>,
    realtime_accounts: moka::sync::Cache<String, RealtimeAccountState>,
    realtime_failures: Arc<FailureLog>,
    on_lambda: bool,
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
            realtime_failures: Arc::default(),
            on_lambda: std::env::var_os("AWS_LAMBDA_FUNCTION_NAME").is_some(),
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

    /// Whether the transport can reach an account's devices with a reply. Firebase overwrites
    /// one node per message, so replies get a node of their own and never hide a wakeup.
    pub fn carries_realtime_replies(&self) -> bool {
        !matches!(self.backend, ChannelBackend::Http)
    }

    /// Sends an encoded [`flow_like_types::realtime::RealtimeReply`] to one account's devices and
    /// waits for the provider, so a caller that publishes in sequence is paced by it. The message
    /// carries conversation content: `sub` must be someone who also receives the stored message.
    /// Best effort; the stored message and its wakeup do not depend on it.
    pub async fn publish_realtime_reply(&self, sub: &str, payload: Vec<u8>) {
        if !self.carries_realtime_replies() || sub.is_empty() {
            return;
        }
        let publication =
            self.realtime_publication(realtime_scope(sub), payload, RealtimeMessage::Reply);
        send_wakeup(
            publication,
            self.realtime_failures.clone(),
            self.transport(),
            None,
        )
        .await;
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
        let admission = admit(&account, throttle, Instant::now());
        if admission == Admission::Covered {
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
        let publication = self.realtime_publication(scope, payload, RealtimeMessage::Wakeup);
        let failures = self.realtime_failures.clone();
        let transport = self.transport();
        let Admission::After(delay) = admission else {
            send_wakeup(
                publication,
                failures,
                transport,
                publish_wait(throttle, self.on_lambda),
            )
            .await;
            return;
        };
        let window_end = async move {
            flow_like_types::tokio::time::sleep(delay).await;
            claim_trailing(&account, Instant::now());
        };
        if self.on_lambda {
            window_end.await;
            send_wakeup(publication, failures, transport, publish_wait(false, true)).await;
        } else {
            spawn(async move {
                window_end.await;
                send_wakeup(publication, failures, transport, publish_wait(true, false)).await;
            });
        }
    }

    /// The provider call owns its inputs so it can outlive the request that started it.
    #[allow(unused_variables)]
    fn realtime_publication(
        &self,
        scope: String,
        payload: Vec<u8>,
        message: RealtimeMessage,
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
                    let gcp = gcp.ok_or_else(|| anyhow!("Firebase unavailable"))?;
                    match message {
                        RealtimeMessage::Wakeup => gcp.publish_realtime(&scope, &payload).await,
                        RealtimeMessage::Reply => {
                            gcp.publish_realtime_reply(&scope, &payload).await
                        }
                    }
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

    /// Refresh only the client's provider credentials. The responder token is reused, so the
    /// channel keeps its original deadline.
    pub async fn refresh_client_handle(
        &self,
        claims: &crate::execution::ChannelClaims,
        responder_token: &str,
    ) -> Result<ChannelHandle> {
        if claims.exp <= now_unix() {
            return Err(anyhow!("channel has expired"));
        }
        let http = ChannelClientDescriptor::Http {
            push_url: push_url(&self.api_base_url, &claims.channel_id),
            token: responder_token.to_owned(),
        };
        if claims.transport == ChannelBackend::Http.transport() {
            return Ok(ChannelHandle {
                channel_id: claims.channel_id.clone(),
                request_id: None,
                expires_at: claims.exp,
                transport: http,
                fallback: None,
            });
        }
        let minted = self.remint_client(claims).await?;
        Ok(ChannelHandle {
            channel_id: claims.channel_id.clone(),
            request_id: None,
            expires_at: minted.expires_at.min(claims.exp),
            transport: minted.descriptor,
            fallback: Some(http),
        })
    }

    /// Provider credentials for the client that end at the channel's original deadline.
    async fn remint_client(
        &self,
        claims: &crate::execution::ChannelClaims,
    ) -> Result<MintedDescriptor> {
        if claims.transport != self.transport() {
            return Err(anyhow!("channel transport no longer matches this API"));
        }
        // A NATS token carries its own expiry, so it is minted to the exact deadline.
        #[cfg(feature = "channel-nats")]
        {
            if matches!(self.backend, ChannelBackend::Nats) {
                return self
                    .nats_runtime()?
                    .client_until(&claims.channel_id, claims.exp);
            }
        }
        self.mint_client(&claims.channel_id, &claims.sub, claims.exp - now_unix())
            .await?
            .ok_or_else(|| anyhow!("channel transport is not configured"))
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

    /// The push endpoint as fallback for a provider handle: its token names the provider, so a
    /// push that lands there is forwarded to the waiter's connection.
    fn fallback_descriptor(
        &self,
        channel_id: &str,
        sub: &str,
        app_id: Option<&str>,
        ttl_secs: i64,
    ) -> Result<ChannelClientDescriptor> {
        self.http_descriptor(channel_id, sub, app_id, self.transport(), ttl_secs)
    }

    #[cfg(feature = "channel-nats")]
    fn nats_runtime(&self) -> Result<&Arc<super::nats::NatsChannelRuntime>> {
        self.nats
            .as_ref()
            .ok_or_else(|| anyhow!("nats channel transport is selected but not configured"))
    }

    #[cfg(feature = "channel-aws")]
    fn aws_runtime(&self) -> Result<&Arc<super::aws::AwsChannelRuntime>> {
        self.aws
            .as_ref()
            .ok_or_else(|| anyhow!("aws_mqtt channel transport is selected but not configured"))
    }

    #[cfg(feature = "channel-azure")]
    fn azure_runtime(&self) -> Result<&Arc<super::azure::AzureChannelRuntime>> {
        self.azure.as_ref().ok_or_else(|| {
            anyhow!("azure_web_pubsub channel transport is selected but not configured")
        })
    }

    #[cfg(feature = "channel-gcp")]
    fn gcp_runtime(&self) -> Result<&Arc<super::gcp::GcpChannelRuntime>> {
        self.gcp.as_ref().ok_or_else(|| {
            anyhow!("gcp_firebase_rtdb channel transport is selected but not configured")
        })
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
            #[cfg(feature = "channel-nats")]
            ChannelBackend::Nats => self.nats_runtime()?.client(channel_id, ttl_secs).map(Some),
            #[cfg(feature = "channel-aws")]
            ChannelBackend::AwsMqtt => self
                .aws_runtime()?
                .client(channel_id, sub, ttl_secs)
                .await
                .map(Some),
            #[cfg(feature = "channel-azure")]
            ChannelBackend::AzureWebPubSub => self
                .azure_runtime()?
                .client(channel_id, sub, ttl_secs)
                .map(Some),
            #[cfg(feature = "channel-gcp")]
            ChannelBackend::GcpFirebaseRtdb => self
                .gcp_runtime()?
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
            #[cfg(feature = "channel-nats")]
            ChannelBackend::Nats => self
                .nats_runtime()?
                .executor(channel_id, ttl_secs)
                .map(Some),
            #[cfg(feature = "channel-aws")]
            ChannelBackend::AwsMqtt => self
                .aws_runtime()?
                .executor(channel_id, ttl_secs)
                .await
                .map(Some),
            #[cfg(feature = "channel-azure")]
            ChannelBackend::AzureWebPubSub => self
                .azure_runtime()?
                .executor(channel_id, ttl_secs)
                .map(Some),
            #[cfg(feature = "channel-gcp")]
            ChannelBackend::GcpFirebaseRtdb => self
                .gcp_runtime()?
                .executor(channel_id, sub, ttl_secs)
                .await
                .map(Some),
        }
    }

    /// Client handle for the configured transport: credentials that reach this run's waiter
    /// directly, with the push endpoint as fallback. The HTTP handle when that is the transport.
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
    /// forwarded inside every request. `expires_at` is the earliest credential expiry. Both
    /// sides are minted concurrently, because every attended dispatch pays this latency and on
    /// AWS each side is its own `sts:AssumeRole`.
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

/// What a realtime publication carries; Firebase writes each to its own node.
#[derive(Clone, Copy)]
enum RealtimeMessage {
    Wakeup,
    Reply,
}

#[derive(Debug, PartialEq, Eq)]
enum Admission {
    Now,
    After(Duration),
    Covered,
}

/// Intermediate run batches publish at once when the account window is open, otherwise once when
/// it ends; a wakeup arriving while that trailing publish is pending is covered by it. Terminal
/// and notification wakeups always go out at once.
fn admit(account: &RealtimeAccountState, throttle: bool, now: Instant) -> Admission {
    if !throttle {
        return Admission::Now;
    }
    let mut account = account.lock();
    if account.trailing_until.is_some_and(|until| now < until) {
        return Admission::Covered;
    }
    let Some(published_at) = account.published_at else {
        account.published_at = Some(now);
        return Admission::Now;
    };
    let window_end = published_at + REALTIME_WINDOW;
    if now >= window_end {
        account.published_at = Some(now);
        return Admission::Now;
    }
    account.trailing_until = Some(window_end);
    Admission::After(window_end - now)
}

fn claim_trailing(account: &RealtimeAccountState, now: Instant) {
    let mut account = account.lock();
    account.published_at = Some(now);
    account.trailing_until = None;
}

fn claim_window(slot: &mut Option<Instant>, now: Instant, window: Duration) -> bool {
    if slot.is_some_and(|at| now.saturating_duration_since(at) < window) {
        return false;
    }
    *slot = Some(now);
    true
}

/// How long a request holds for its wakeup; `None` until the publication finishes. Lambda
/// freezes detached work with the invocation, so there terminal and notification wakeups
/// finish inside the request. Intermediate ones, and every wakeup elsewhere, get a short head
/// start and finish detached; clients recover dropped wakeups through HTTP replay.
fn publish_wait(throttle: bool, on_lambda: bool) -> Option<Duration> {
    (throttle || !on_lambda).then_some(REALTIME_PUBLISH_WAIT)
}

async fn send_wakeup(
    publication: BoxFuture<'static, Result<()>>,
    failures: Arc<FailureLog>,
    transport: &'static str,
    wait: Option<Duration>,
) {
    let task = spawn(async move {
        let error = match timeout(REALTIME_PUBLISH_TIMEOUT, publication).await {
            Ok(Ok(())) => return,
            Ok(Err(error)) => redact_provider_error(&error),
            Err(_) => format!("no answer within {REALTIME_PUBLISH_TIMEOUT:?}"),
        };
        match failures.claim(Instant::now()) {
            Some(suppressed) => tracing::warn!(
                transport,
                %error,
                suppressed,
                "Realtime wakeup failed; clients recover through HTTP replay"
            ),
            None => tracing::debug!(transport, %error, "Realtime wakeup failed"),
        }
    });
    match wait {
        Some(wait) => {
            let _ = timeout(wait, task).await;
        }
        None => {
            let _ = task.await;
        }
    }
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
        assert!(!issuer.carries_realtime_replies());
        issuer
            .publish_realtime_reply("user-a", b"{}".to_vec())
            .await;
    }

    #[test]
    fn every_provider_transport_carries_replies() {
        for transport in ["aws_mqtt", "azure_web_pubsub", "gcp_firebase_rtdb", "nats"] {
            let Ok(backend) = ChannelBackend::parse(Some(transport)) else {
                continue;
            };
            let mut issuer = ChannelIssuer::http_only("https://api.test");
            issuer.backend = backend;
            assert!(issuer.carries_realtime_replies(), "{transport}");
        }
    }

    #[test]
    fn intermediate_wakeups_share_a_window_and_terminal_ones_never_wait() {
        let account = RealtimeAccountState::default();
        let start = Instant::now();
        let ms = Duration::from_millis;
        assert_eq!(admit(&account, true, start), Admission::Now);
        assert_eq!(
            admit(&account, true, start + ms(100)),
            Admission::After(ms(150))
        );
        assert_eq!(admit(&account, false, start + ms(100)), Admission::Now);
        assert_eq!(admit(&account, true, start + ms(200)), Admission::Covered);
        claim_trailing(&account, start + REALTIME_WINDOW);
        assert_eq!(
            admit(&account, true, start + REALTIME_WINDOW + ms(10)),
            Admission::After(ms(240))
        );
        assert_eq!(
            admit(&RealtimeAccountState::default(), true, start),
            Admission::Now
        );
    }

    #[test]
    fn a_wakeup_after_the_window_publishes_at_once_and_opens_a_new_one() {
        let account = RealtimeAccountState::default();
        let start = Instant::now();
        assert_eq!(admit(&account, true, start), Admission::Now);
        let later = start + REALTIME_WINDOW;
        assert_eq!(admit(&account, true, later), Admission::Now);
        assert_eq!(
            admit(&account, true, later + Duration::from_millis(1)),
            Admission::After(REALTIME_WINDOW - Duration::from_millis(1))
        );
    }

    #[tokio::test]
    async fn a_hanging_provider_holds_the_request_only_for_the_wait_bound() {
        let started = Instant::now();
        send_wakeup(
            Box::pin(std::future::pending()),
            Arc::default(),
            "test",
            publish_wait(false, false),
        )
        .await;
        let waited = started.elapsed();
        assert!(
            waited >= REALTIME_PUBLISH_WAIT && waited < REALTIME_PUBLISH_TIMEOUT / 2,
            "{waited:?}"
        );
    }

    #[test]
    fn lambda_holds_only_terminal_and_notification_wakeups_to_the_end() {
        assert_eq!(publish_wait(false, true), None);
        for (throttle, on_lambda) in [(true, true), (false, false), (true, false)] {
            assert_eq!(
                publish_wait(throttle, on_lambda),
                Some(REALTIME_PUBLISH_WAIT),
                "throttle {throttle}, lambda {on_lambda}"
            );
        }
    }

    #[tokio::test(start_paused = true)]
    async fn a_lambda_request_finishes_a_slow_wakeup_before_it_responds() {
        use std::sync::atomic::{AtomicBool, Ordering};
        let slow = |published: Arc<AtomicBool>| {
            Box::pin(async move {
                flow_like_types::tokio::time::sleep(Duration::from_millis(600)).await;
                published.store(true, Ordering::SeqCst);
                Ok::<(), flow_like_types::Error>(())
            })
        };
        for (on_lambda, finished) in [(true, true), (false, false)] {
            let published = Arc::new(AtomicBool::new(false));
            send_wakeup(
                slow(published.clone()),
                Arc::default(),
                "test",
                publish_wait(false, on_lambda),
            )
            .await;
            assert_eq!(published.load(Ordering::SeqCst), finished, "{on_lambda}");
        }

        let started = flow_like_types::tokio::time::Instant::now();
        send_wakeup(
            Box::pin(std::future::pending()),
            Arc::default(),
            "test",
            publish_wait(false, true),
        )
        .await;
        let waited = started.elapsed();
        assert!(
            waited >= REALTIME_PUBLISH_TIMEOUT
                && waited < REALTIME_PUBLISH_TIMEOUT + Duration::from_millis(10),
            "{waited:?}"
        );
    }

    #[test]
    fn a_warning_counts_the_failures_suppressed_since_the_last_one() {
        let failures = FailureLog::default();
        let start = Instant::now();
        assert_eq!(failures.claim(start), Some(0));
        assert_eq!(failures.claim(start + Duration::from_millis(10)), None);
        assert_eq!(failures.claim(start + Duration::from_millis(999)), None);
        assert_eq!(failures.claim(start + FAILURE_LOG_WINDOW), Some(2));
        assert_eq!(failures.claim(start + FAILURE_LOG_WINDOW * 3), Some(0));
    }

    #[tokio::test]
    async fn every_account_shares_one_failure_warning_window() {
        let failures = Arc::new(FailureLog::default());
        let wait = publish_wait(false, true);
        send_wakeup(Box::pin(async { Ok(()) }), failures.clone(), "test", wait).await;
        assert!(failures.0.lock().warned_at.is_none());
        for _account in 0..3 {
            let failing = Box::pin(async { Err::<(), _>(anyhow!("provider unavailable")) });
            send_wakeup(failing, failures.clone(), "test", wait).await;
        }
        let window = failures.0.lock();
        assert!(window.warned_at.is_some());
        assert_eq!(window.suppressed, 2);
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
    async fn a_provider_handle_needs_the_provider_runtime_and_falls_back_to_the_api() {
        crate::backend_jwt::init_for_tests();
        for transport in ["aws_mqtt", "azure_web_pubsub", "gcp_firebase_rtdb", "nats"] {
            let Ok(backend) = ChannelBackend::parse(Some(transport)) else {
                continue;
            };
            let mut issuer = ChannelIssuer::http_only("https://api.test");
            issuer.backend = backend;
            // The client's credentials come from the provider runtime. Without it nothing is
            // handed out, and dispatch degrades to the http grant.
            assert!(
                issuer
                    .client_handle("run-1", "user-1", Some("app-1"), 120)
                    .await
                    .is_err(),
                "{transport}"
            );
            assert!(
                issuer
                    .grant("run-1", "user-1", Some("app-1"), 120)
                    .await
                    .is_err(),
                "{transport}"
            );

            // The fallback names the provider, so the push endpoint forwards to the waiter
            // instead of flipping a row.
            let fallback = issuer
                .fallback_descriptor("run-1", "user-1", Some("app-1"), 120)
                .unwrap();
            let ChannelClientDescriptor::Http { push_url, token } = &fallback else {
                panic!("the fallback is the API push endpoint");
            };
            assert_eq!(push_url, "https://api.test/api/v1/channels/run-1/push");
            let claims = verify_channel_responder(token).unwrap();
            assert_eq!(claims.transport, transport);
            assert_eq!(claims.channel_id, "run-1");
            assert_eq!(claims.sub, "user-1");
            assert_eq!(claims.app_id.as_deref(), Some("app-1"));

            // A refresh mints the provider credentials again, so it needs the runtime too.
            assert!(issuer.refresh_client_handle(&claims, token).await.is_err());
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
