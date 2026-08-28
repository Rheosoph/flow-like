//! Waiter side: one MQTT-over-WebSocket connection per channel, driven by a spawned event loop
//! that feeds every incoming PUBLISH through the [`PushRouter`].

use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

use flow_like_types::async_trait;
use flow_like_types::channel::{
    Channel, ChannelExecutorGrant, ChannelGrant, ChannelHandle, ChannelOutcome, ChannelTicket,
    clamp_ttl, new_request_id, now_unix,
};
use flow_like_types::tokio::sync::oneshot;
use flow_like_types::tokio::task::JoinHandle;
use flow_like_types::tokio_util::sync::CancellationToken;
use flow_like_types::{Result, Value, anyhow, bail};
use rumqttc::{
    AsyncClient, Event, EventLoop, MqttOptions, Outgoing, Packet, QoS, SubscribeReasonCode,
    TlsConfiguration, Transport,
};

use super::policy::validate_topic;
use super::presign::{Presigner, mqtt_wss_url};
use super::router::{PushRouter, lock};

const KEEP_ALIVE: Duration = Duration::from_secs(30);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(20);
const CLOSE_GRACE: Duration = Duration::from_secs(2);
const INITIAL_BACKOFF: Duration = Duration::from_millis(250);
const MAX_BACKOFF: Duration = Duration::from_secs(5);
/// AWS IoT caps MQTT payloads at 128 KiB; leave headroom for the packet envelope.
const MAX_PACKET_SIZE: usize = 256 * 1024;
const REQUEST_CHANNEL_CAPACITY: usize = 16;

pub struct AwsIotChannel {
    channel_id: String,
    handle: ChannelHandle,
    router: Arc<PushRouter>,
    client: AsyncClient,
    task: Mutex<Option<JoinHandle<()>>>,
    closed: CancellationToken,
}

impl AwsIotChannel {
    /// Connects, subscribes to the inbox topic and only then returns; a failed handshake,
    /// rejected credentials or a silent broker surface as an error.
    pub async fn connect(grant: &ChannelGrant) -> Result<Arc<dyn Channel>> {
        let ChannelExecutorGrant::AwsMqtt {
            endpoint,
            region,
            client_id,
            inbox_topic,
            credentials,
        } = &grant.executor
        else {
            bail!(
                "channel '{}' grant is for transport '{}', not aws_mqtt",
                grant.channel_id,
                grant.transport()
            );
        };
        validate_topic(inbox_topic)?;
        if client_id.is_empty() {
            bail!(
                "channel '{}' grant has an empty IoT client id",
                grant.channel_id
            );
        }
        if credentials.expiration <= now_unix() {
            bail!(
                "channel '{}' IoT credentials expired at {}",
                grant.channel_id,
                credentials.expiration
            );
        }

        let presigner = Presigner::new(endpoint, region, credentials);
        presigner.presign(SystemTime::now())?;

        let mut options = MqttOptions::new(client_id.clone(), mqtt_wss_url(endpoint), 443);
        options
            .set_transport(Transport::wss_with_config(TlsConfiguration::Rustls(
                Arc::new(tls_config()?),
            )))
            .set_keep_alive(KEEP_ALIVE)
            .set_clean_session(true)
            .set_max_packet_size(MAX_PACKET_SIZE, MAX_PACKET_SIZE);
        let signer = presigner.clone();
        options.set_request_modifier(move |request| std::future::ready(signer.apply(request)));

        let (client, event_loop) = AsyncClient::new(options, REQUEST_CHANNEL_CAPACITY);
        let router = Arc::new(PushRouter::new(&grant.channel_id));
        let closed = CancellationToken::new();
        let (ready_tx, ready_rx) = oneshot::channel();
        let task = flow_like_types::tokio::spawn(run_event_loop(EventLoopContext {
            event_loop,
            client: client.clone(),
            router: router.clone(),
            inbox_topic: inbox_topic.clone(),
            closed: closed.clone(),
            ready: Some(ready_tx),
            channel_id: grant.channel_id.clone(),
        }));

        let channel = Arc::new(Self {
            channel_id: grant.channel_id.clone(),
            handle: grant.client.clone(),
            router,
            client,
            task: Mutex::new(Some(task)),
            closed,
        });

        let outcome = flow_like_types::tokio::time::timeout(CONNECT_TIMEOUT, ready_rx).await;
        match outcome {
            Ok(Ok(Ok(()))) => Ok(channel),
            Ok(Ok(Err(message))) => {
                channel.abort_task();
                Err(anyhow!("channel '{}': {message}", grant.channel_id))
            }
            Ok(Err(_)) => {
                channel.abort_task();
                Err(anyhow!(
                    "channel '{}': AWS IoT event loop stopped before the connection was ready",
                    grant.channel_id
                ))
            }
            Err(_) => {
                channel.abort_task();
                Err(anyhow!(
                    "channel '{}': AWS IoT connection to '{endpoint}' as '{client_id}' timed out after {}s",
                    grant.channel_id,
                    CONNECT_TIMEOUT.as_secs()
                ))
            }
        }
    }

    fn abort_task(&self) {
        if let Some(task) = lock(&self.task).take() {
            task.abort();
        }
    }

    #[cfg(test)]
    fn detached(channel_id: &str, handle: ChannelHandle) -> Arc<Self> {
        let (client, _event_loop) =
            AsyncClient::new(MqttOptions::new("test", "wss://localhost/mqtt", 443), 4);
        Arc::new(Self {
            channel_id: channel_id.to_string(),
            handle,
            router: Arc::new(PushRouter::new(channel_id)),
            client,
            task: Mutex::new(None),
            closed: CancellationToken::new(),
        })
    }
}

impl Drop for AwsIotChannel {
    fn drop(&mut self) {
        if let Some(task) = self
            .task
            .get_mut()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take()
        {
            task.abort();
        }
    }
}

/// Explicit provider and roots: the workspace links both `ring` and `aws-lc-rs` into rustls, so
/// `ClientConfig::builder()` (what `Transport::wss_with_default_config` uses) would panic unless
/// the binary installed a process default first.
fn tls_config() -> Result<rustls::ClientConfig> {
    let provider = rustls::crypto::CryptoProvider::get_default()
        .cloned()
        .unwrap_or_else(|| Arc::new(rustls::crypto::aws_lc_rs::default_provider()));
    let mut roots = rustls::RootCertStore::empty();
    roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    Ok(rustls::ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .map_err(|error| anyhow!("rustls provider supports no usable TLS version: {error}"))?
        .with_root_certificates(roots)
        .with_no_client_auth())
}

struct EventLoopContext {
    event_loop: EventLoop,
    client: AsyncClient,
    router: Arc<PushRouter>,
    inbox_topic: String,
    closed: CancellationToken,
    ready: Option<oneshot::Sender<std::result::Result<(), String>>>,
    channel_id: String,
}

impl EventLoopContext {
    fn signal_ready(&mut self, result: std::result::Result<(), String>) {
        if let Some(ready) = self.ready.take() {
            let _ = ready.send(result);
        }
    }
}

async fn run_event_loop(mut ctx: EventLoopContext) {
    let mut backoff = INITIAL_BACKOFF;
    loop {
        let event = flow_like_types::tokio::select! {
            biased;
            event = ctx.event_loop.poll() => event,
            _ = ctx.closed.cancelled() => break,
        };
        match event {
            Ok(Event::Incoming(Packet::ConnAck(ack))) => {
                backoff = INITIAL_BACKOFF;
                tracing::info!(
                    channel_id = %ctx.channel_id,
                    session_present = ack.session_present,
                    "AWS IoT channel connected"
                );
                if let Err(error) = ctx.client.try_subscribe(&ctx.inbox_topic, QoS::AtLeastOnce) {
                    tracing::error!(%error, channel_id = %ctx.channel_id, topic = %ctx.inbox_topic, "AWS IoT subscribe could not be queued");
                    ctx.signal_ready(Err(format!(
                        "AWS IoT subscribe could not be queued: {error}"
                    )));
                }
            }
            Ok(Event::Incoming(Packet::SubAck(ack))) => {
                if ack
                    .return_codes
                    .iter()
                    .any(|code| matches!(code, SubscribeReasonCode::Failure))
                {
                    tracing::warn!(
                        channel_id = %ctx.channel_id,
                        topic = %ctx.inbox_topic,
                        "AWS IoT rejected the inbox subscription; relying on direct messages only"
                    );
                }
                ctx.signal_ready(Ok(()));
            }
            Ok(Event::Incoming(Packet::Publish(publish))) => {
                let result = ctx.router.route_payload(&publish.payload);
                tracing::debug!(
                    channel_id = %ctx.channel_id,
                    topic = %publish.topic,
                    bytes = publish.payload.len(),
                    ?result,
                    "AWS IoT channel push routed"
                );
            }
            Ok(Event::Outgoing(Outgoing::Disconnect)) => break,
            Ok(_) => {}
            Err(error) => {
                if ctx.closed.is_cancelled() {
                    break;
                }
                if ctx.ready.is_some() {
                    ctx.signal_ready(Err(format!("AWS IoT connection failed: {error}")));
                    break;
                }
                tracing::warn!(
                    %error,
                    channel_id = %ctx.channel_id,
                    retry_in_ms = backoff.as_millis() as u64,
                    "AWS IoT channel connection lost; reconnecting"
                );
                flow_like_types::tokio::select! {
                    _ = ctx.closed.cancelled() => break,
                    _ = flow_like_types::tokio::time::sleep(backoff) => {}
                }
                backoff = (backoff * 2).min(MAX_BACKOFF);
            }
        }
    }
    tracing::debug!(channel_id = %ctx.channel_id, "AWS IoT channel event loop stopped");
}

#[async_trait]
impl Channel for AwsIotChannel {
    fn channel_id(&self) -> &str {
        &self.channel_id
    }

    fn handle(&self) -> ChannelHandle {
        self.handle.clone()
    }

    async fn open(&self, ttl: Duration) -> Result<ChannelTicket> {
        if self.closed.is_cancelled() {
            bail!("channel '{}' is closed", self.channel_id);
        }
        let request_id = new_request_id();
        let expires_at = now_unix() + clamp_ttl(ttl).as_secs() as i64;
        self.router.register(&request_id);
        Ok(ChannelTicket {
            handle: self.handle.for_request(&request_id, expires_at),
            request_id,
            expires_at,
        })
    }

    async fn wait(
        &self,
        ticket: &ChannelTicket,
        cancel: Option<CancellationToken>,
    ) -> Result<ChannelOutcome> {
        let Some(receiver) = self.router.take_receiver(&ticket.request_id) else {
            return Ok(ChannelOutcome::Closed);
        };
        let remaining = Duration::from_secs((ticket.expires_at - now_unix()).max(0) as u64);
        let cancel = cancel.unwrap_or_default();
        let outcome = flow_like_types::tokio::select! {
            biased;
            result = receiver => match result {
                Ok(value) => ChannelOutcome::Responded(value),
                Err(_) => ChannelOutcome::Closed,
            },
            _ = self.router.cancelled().cancelled() => ChannelOutcome::Cancelled,
            _ = cancel.cancelled() => ChannelOutcome::Cancelled,
            _ = self.closed.cancelled() => ChannelOutcome::Closed,
            _ = flow_like_types::tokio::time::sleep(remaining) => ChannelOutcome::Expired,
        };
        self.abandon(ticket).await;
        Ok(outcome)
    }

    async fn abandon(&self, ticket: &ChannelTicket) {
        self.router.remove(&ticket.request_id);
    }

    async fn drain_inbound(&self) -> Vec<Value> {
        self.router.drain_inbound()
    }

    async fn is_cancelled(&self) -> bool {
        self.router.is_cancelled()
    }

    async fn close(&self) {
        self.router.clear();
        if self.closed.is_cancelled() {
            return;
        }
        let _ = self.client.try_disconnect();
        self.closed.cancel();
        let task = lock(&self.task).take();
        if let Some(mut task) = task
            && flow_like_types::tokio::time::timeout(CLOSE_GRACE, &mut task)
                .await
                .is_err()
        {
            tracing::debug!(channel_id = %self.channel_id, "AWS IoT event loop did not stop in time; aborting");
            task.abort();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use flow_like_types::channel::{
        AwsTemporaryCredentials, ChannelClientDescriptor, ChannelPush, ChannelPushKind,
    };

    fn handle(channel_id: &str) -> ChannelHandle {
        ChannelHandle {
            channel_id: channel_id.into(),
            request_id: None,
            expires_at: now_unix() + 3600,
            transport: ChannelClientDescriptor::AwsMqtt {
                endpoint: "x-ats.iot.eu-central-1.amazonaws.com".into(),
                region: "eu-central-1".into(),
                target_client_id: format!("run-{channel_id}"),
                topic: format!("runs/{channel_id}/inbox"),
                credentials: AwsTemporaryCredentials {
                    access_key_id: "AKID".into(),
                    secret_access_key: "SECRET".into(),
                    session_token: "TOKEN".into(),
                    expiration: now_unix() + 3600,
                },
            },
            fallback: None,
        }
    }

    fn push(
        channel_id: &str,
        request_id: Option<&str>,
        kind: ChannelPushKind,
        value: Value,
    ) -> ChannelPush {
        ChannelPush {
            channel_id: channel_id.into(),
            request_id: request_id.map(str::to_string),
            kind,
            value,
        }
    }

    #[flow_like_types::tokio::test]
    async fn open_derives_request_handle_and_reply_before_wait_is_delivered() {
        let channel = AwsIotChannel::detached("run-a", handle("run-a"));
        let ticket = channel.open(Duration::from_secs(30)).await.unwrap();
        assert_eq!(
            ticket.handle.request_id.as_deref(),
            Some(ticket.request_id.as_str())
        );
        assert_eq!(ticket.handle.channel_id, "run-a");
        assert_eq!(ticket.handle.expires_at, ticket.expires_at);
        assert!(matches!(
            ticket.handle.transport,
            ChannelClientDescriptor::AwsMqtt { .. }
        ));

        channel.router.route(push(
            "run-a",
            Some(&ticket.request_id),
            ChannelPushKind::Reply,
            Value::from(7),
        ));
        assert_eq!(
            channel.wait(&ticket, None).await.unwrap(),
            ChannelOutcome::Responded(Value::from(7))
        );
        assert_eq!(
            channel.wait(&ticket, None).await.unwrap(),
            ChannelOutcome::Closed
        );
    }

    #[flow_like_types::tokio::test]
    async fn wait_resolves_while_blocked() {
        let channel = AwsIotChannel::detached("run-b", handle("run-b"));
        let ticket = channel.open(Duration::from_secs(30)).await.unwrap();
        let waiter = {
            let channel = channel.clone();
            let ticket = ticket.clone();
            flow_like_types::tokio::spawn(async move { channel.wait(&ticket, None).await.unwrap() })
        };
        flow_like_types::tokio::time::sleep(Duration::from_millis(20)).await;
        channel.router.route(push(
            "run-b",
            Some(&ticket.request_id),
            ChannelPushKind::Reply,
            Value::from("ok"),
        ));
        assert_eq!(
            waiter.await.unwrap(),
            ChannelOutcome::Responded(Value::from("ok"))
        );
    }

    #[flow_like_types::tokio::test(start_paused = true)]
    async fn wait_expires_at_ticket_deadline() {
        let channel = AwsIotChannel::detached("run-c", handle("run-c"));
        let ticket = channel.open(Duration::from_secs(1)).await.unwrap();
        assert_eq!(
            channel.wait(&ticket, None).await.unwrap(),
            ChannelOutcome::Expired
        );
        assert_eq!(
            channel.router.route(push(
                "run-c",
                Some(&ticket.request_id),
                ChannelPushKind::Reply,
                Value::Null
            )),
            super::super::router::RouteResult::UnknownRequest
        );
    }

    #[flow_like_types::tokio::test]
    async fn cancel_token_and_cancel_push_end_the_wait() {
        let channel = AwsIotChannel::detached("run-d", handle("run-d"));
        let ticket = channel.open(Duration::from_secs(30)).await.unwrap();
        let token = CancellationToken::new();
        token.cancel();
        assert_eq!(
            channel.wait(&ticket, Some(token)).await.unwrap(),
            ChannelOutcome::Cancelled
        );

        let ticket = channel.open(Duration::from_secs(30)).await.unwrap();
        let waiter = {
            let channel = channel.clone();
            let ticket = ticket.clone();
            flow_like_types::tokio::spawn(async move { channel.wait(&ticket, None).await.unwrap() })
        };
        flow_like_types::tokio::time::sleep(Duration::from_millis(20)).await;
        channel
            .router
            .route(push("run-d", None, ChannelPushKind::Cancel, Value::Null));
        assert_eq!(waiter.await.unwrap(), ChannelOutcome::Cancelled);
        assert!(channel.is_cancelled().await);
    }

    #[flow_like_types::tokio::test]
    async fn inbound_drains_and_close_wakes_waiters() {
        let channel = AwsIotChannel::detached("run-e", handle("run-e"));
        channel.router.route(push(
            "run-e",
            None,
            ChannelPushKind::Inbound,
            Value::from("steer"),
        ));
        assert_eq!(channel.drain_inbound().await, vec![Value::from("steer")]);
        assert!(channel.drain_inbound().await.is_empty());

        let ticket = channel.open(Duration::from_secs(30)).await.unwrap();
        let waiter = {
            let channel = channel.clone();
            let ticket = ticket.clone();
            flow_like_types::tokio::spawn(async move { channel.wait(&ticket, None).await.unwrap() })
        };
        flow_like_types::tokio::time::sleep(Duration::from_millis(20)).await;
        channel.close().await;
        assert_eq!(waiter.await.unwrap(), ChannelOutcome::Closed);
        assert!(channel.open(Duration::from_secs(1)).await.is_err());
    }

    #[flow_like_types::tokio::test]
    async fn connect_rejects_non_aws_grants() {
        let grant = ChannelGrant {
            channel_id: "run-f".into(),
            expires_at: now_unix() + 60,
            executor: ChannelExecutorGrant::Http {},
            client: handle("run-f"),
        };
        let error = AwsIotChannel::connect(&grant)
            .await
            .err()
            .unwrap()
            .to_string();
        assert!(error.contains("not aws_mqtt"), "{error}");
    }

    #[flow_like_types::tokio::test]
    async fn connect_rejects_expired_credentials_before_dialing() {
        let grant = ChannelGrant {
            channel_id: "run-g".into(),
            expires_at: now_unix() + 60,
            executor: ChannelExecutorGrant::AwsMqtt {
                endpoint: "x-ats.iot.eu-central-1.amazonaws.com".into(),
                region: "eu-central-1".into(),
                client_id: "run-g".into(),
                inbox_topic: "runs/run-g/inbox".into(),
                credentials: AwsTemporaryCredentials {
                    access_key_id: "AKID".into(),
                    secret_access_key: "SECRET".into(),
                    session_token: "TOKEN".into(),
                    expiration: now_unix() - 1,
                },
            },
            client: handle("run-g"),
        };
        let error = AwsIotChannel::connect(&grant)
            .await
            .err()
            .unwrap()
            .to_string();
        assert!(error.contains("expired"), "{error}");
    }
}
