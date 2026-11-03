use super::{
    ACK_TIMEOUT, API_MAX_RECONNECTS, Acknowledgement, CONNECT_TIMEOUT, Envelope, MAX_PAYLOAD_BYTES,
    Role, connect_with, mint_realtime_publisher_token, mint_user_token, subject_for,
    valid_delivery_id, validate_account_seed, validate_url,
};
use crate::{ChannelForwardError, ChannelForwarder};
use flow_like_types::channel::{ChannelPush, new_request_id};
use flow_like_types::{Result, anyhow, async_trait, bail};
use futures_util::StreamExt;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};
use tokio::sync::OwnedMutexGuard;

const REALTIME_TOKEN_TTL_SECS: i64 = 900;
/// Connecting and publishing stay well inside the API's two-second wakeup budget.
const REALTIME_CONNECT_TIMEOUT: Duration = Duration::from_secs(1);
const REALTIME_PUBLISH_TIMEOUT: Duration = Duration::from_millis(500);
/// After a failure, wakeups fail fast instead of each waiting on a degraded broker.
const REALTIME_RETRY_DELAY: Duration = Duration::from_secs(5);

#[derive(Default)]
struct RealtimePublisher {
    client: Option<Arc<async_nats::Client>>,
    retry_at: Option<Instant>,
}

pub struct NatsForwarder {
    url: String,
    account_seed: String,
    realtime: Arc<Mutex<RealtimePublisher>>,
    connecting: Arc<tokio::sync::Mutex<()>>,
}

impl NatsForwarder {
    pub fn new(url: String, account_seed: String) -> Result<Self> {
        validate_url(&url)?;
        validate_account_seed(&account_seed)?;
        Ok(Self {
            url,
            account_seed,
            realtime: Arc::default(),
            connecting: Arc::default(),
        })
    }

    /// Best-effort wakeups share one API connection, without channel reply acknowledgements.
    pub async fn publish_realtime(&self, scope: &str, payload: &[u8]) -> Result<()> {
        let subject = super::realtime_subject(scope)?;
        if payload.len() > flow_like_types::realtime::MAX_REALTIME_WAKEUP_BYTES {
            bail!("realtime wakeup exceeds the size limit");
        }
        let client = self.realtime_client().await?;
        let published = tokio::time::timeout(REALTIME_PUBLISH_TIMEOUT, async {
            client
                .publish(subject, payload.to_vec().into())
                .await
                .ok()?;
            client.flush().await.ok()
        })
        .await;
        let error = match published {
            Ok(Some(())) => return Ok(()),
            Ok(None) => anyhow!("NATS realtime publication failed"),
            Err(_) => anyhow!("NATS realtime publication timed out"),
        };
        self.discard_realtime(&client);
        Err(error)
    }

    /// One background connect at a time; callers wait for it only as long as it may take.
    async fn realtime_client(&self) -> Result<Arc<async_nats::Client>> {
        if let Some(client) = self.ready_realtime()? {
            return Ok(client);
        }
        if let Ok(gate) = self.connecting.clone().try_lock_owned() {
            if let Some(client) = self.ready_realtime()? {
                return Ok(client);
            }
            tokio::spawn(connect_realtime(
                self.url.clone(),
                self.account_seed.clone(),
                self.realtime.clone(),
                gate,
            ));
        }
        let _ = tokio::time::timeout(REALTIME_CONNECT_TIMEOUT, self.connecting.lock()).await;
        self.ready_realtime()?
            .ok_or_else(|| anyhow!("NATS realtime publisher is not connected"))
    }

    fn ready_realtime(&self) -> Result<Option<Arc<async_nats::Client>>> {
        let state = lock(&self.realtime);
        if let Some(client) = &state.client {
            return Ok(Some(client.clone()));
        }
        if state.retry_at.is_some_and(|at| Instant::now() < at) {
            bail!("NATS realtime publisher is unavailable after a recent failure");
        }
        Ok(None)
    }

    fn discard_realtime(&self, client: &Arc<async_nats::Client>) {
        let mut state = lock(&self.realtime);
        if !state
            .client
            .as_ref()
            .is_some_and(|current| Arc::ptr_eq(current, client))
        {
            return;
        }
        state.client = None;
        state.retry_at = Some(Instant::now() + REALTIME_RETRY_DELAY);
        drop(state);
        let client = client.clone();
        tokio::spawn(async move {
            let _ = tokio::time::timeout(Duration::from_secs(1), client.drain()).await;
        });
    }
}

/// Holds `_gate` until the outcome is recorded, so waiters observe it.
async fn connect_realtime(
    url: String,
    account_seed: String,
    state: Arc<Mutex<RealtimePublisher>>,
    _gate: OwnedMutexGuard<()>,
) {
    let connected = connect_with(
        &url,
        API_MAX_RECONNECTS,
        REALTIME_CONNECT_TIMEOUT,
        move || mint_realtime_publisher_token(&account_seed, REALTIME_TOKEN_TTL_SECS),
    )
    .await;
    let mut state = lock(&state);
    match connected {
        Ok(client) => {
            state.client = Some(Arc::new(client));
            state.retry_at = None;
        }
        Err(_) => state.retry_at = Some(Instant::now() + REALTIME_RETRY_DELAY),
    }
}

fn lock(state: &Mutex<RealtimePublisher>) -> MutexGuard<'_, RealtimePublisher> {
    state.lock().unwrap_or_else(PoisonError::into_inner)
}

#[async_trait]
impl ChannelForwarder for NatsForwarder {
    fn transport(&self) -> &'static str {
        "nats"
    }
    async fn forward(&self, push: &ChannelPush) -> Result<()> {
        self.forward_with_id(push, &new_request_id()).await
    }
    async fn forward_with_id(&self, push: &ChannelPush, id: &str) -> Result<()> {
        let subject = subject_for(&push.channel_id)?;
        let (account_seed, channel_id) = (self.account_seed.clone(), push.channel_id.clone());
        let client = connect_with(&self.url, API_MAX_RECONNECTS, CONNECT_TIMEOUT, move || {
            mint_user_token(&account_seed, &channel_id, Role::Client, 30)
        })
        .await?;
        let result = publish_confirmed(&client, &subject, push, id).await;
        let _ = tokio::time::timeout(Duration::from_secs(1), client.drain()).await;
        result
    }
}

/// Confirms receiver acceptance. Repeated attempts keep one delivery id so a lost ack cannot
/// enqueue the same steering message twice within the receiver's recent receipt window.
pub async fn publish_confirmed(
    client: &async_nats::Client,
    subject: &str,
    push: &ChannelPush,
    id: &str,
) -> Result<()> {
    if !valid_delivery_id(id) {
        bail!("invalid NATS delivery id");
    }
    if subject != subject_for(&push.channel_id)? {
        bail!("NATS subject does not match channel");
    }
    let payload = serde_json::to_vec(&Envelope {
        id: id.into(),
        push: push.clone(),
    })?;
    if payload.len() > MAX_PAYLOAD_BYTES {
        bail!("NATS channel message exceeds 1 MiB");
    }
    let reply = format!("{subject}.ack.{id}");
    let mut acknowledgements = client
        .subscribe(reply.clone())
        .await
        .map_err(|_| anyhow!("NATS acknowledgement subscription failed"))?;
    let result = async {
        tokio::time::timeout(super::CONNECT_TIMEOUT, client.flush())
            .await
            .map_err(|_| anyhow!("NATS acknowledgement subscription timed out"))?
            .map_err(|_| anyhow!("NATS acknowledgement subscription failed"))?;
        for _ in 0..3 {
            client
                .publish_with_reply(subject.to_owned(), reply.clone(), payload.clone().into())
                .await
                .map_err(|_| anyhow!("NATS channel publication failed"))?;
            match tokio::time::timeout(ACK_TIMEOUT, acknowledgements.next()).await {
                Ok(Some(message)) => {
                    if message.status == Some(async_nats::StatusCode::NO_RESPONDERS) {
                        return Err(ChannelForwardError::NoListener.into());
                    }
                    let ack: Acknowledgement = serde_json::from_slice(&message.payload)
                        .map_err(|_| anyhow!("invalid NATS delivery acknowledgement"))?;
                    if ack.accepted {
                        return Ok(());
                    }
                    return Err(ack.rejection().into());
                }
                Ok(None) => bail!("NATS acknowledgement subscription closed"),
                Err(_) => {}
            }
        }
        bail!("NATS waiter did not acknowledge the message")
    }
    .await;
    let _ = acknowledgements.unsubscribe().await;
    result
}
