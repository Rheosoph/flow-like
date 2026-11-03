use super::router::Router;
use super::{
    CONNECT_TIMEOUT, Envelope, MAX_PAYLOAD_BYTES, MAX_RECONNECTS, connect_with, subject_for,
    valid_delivery_id,
};
use flow_like_types::channel::{
    Channel, ChannelExecutorGrant, ChannelGrant, ChannelHandle, ChannelOutcome, ChannelTicket,
    new_request_id, now_unix, ticket_deadline,
};
use flow_like_types::tokio_util::sync::CancellationToken;
use flow_like_types::{Result, Value, anyhow, async_trait, bail};
use futures_util::StreamExt;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;
use tokio::task::JoinHandle;

/// async-nats's longest reconnect backoff, kept after a client has given up.
const RESUBSCRIBE_DELAY: Duration = Duration::from_secs(4);

pub struct NatsChannel {
    channel_id: String,
    handle: ChannelHandle,
    router: Arc<Router>,
    /// `receive` replaces a client that stopped reconnecting.
    client: Arc<Mutex<async_nats::Client>>,
    task: Mutex<Option<JoinHandle<()>>>,
    proxy: Mutex<Option<super::proxy::ProxyGuard>>,
}

/// Each client stops after `max_reconnects` attempts, so one with an expired credential or a
/// dropped channel cannot retry forever. Until the grant expires, the inbox subscribes again
/// with a new client, and pending requests keep waiting through a long broker outage.
struct Inbox {
    url: String,
    subject: String,
    token: String,
    max_reconnects: usize,
    resubscribe_delay: Duration,
}

impl Inbox {
    async fn subscribe(&self) -> Result<(async_nats::Client, async_nats::Subscriber)> {
        let token = self.token.clone();
        let client = connect_with(&self.url, self.max_reconnects, CONNECT_TIMEOUT, move || {
            Ok(token.clone())
        })
        .await?;
        let subscriber = client
            .subscribe(self.subject.clone())
            .await
            .map_err(|_| anyhow!("NATS inbox subscription failed"))?;
        tokio::time::timeout(CONNECT_TIMEOUT, client.flush())
            .await
            .map_err(|_| anyhow!("NATS subscription readiness timed out"))?
            .map_err(|_| anyhow!("NATS subscription readiness failed"))?;
        Ok((client, subscriber))
    }

    /// Never gives up; `receive` bounds it by the grant's expiry and the channel's close.
    async fn resubscribe(&self) -> (async_nats::Client, async_nats::Subscriber) {
        loop {
            tokio::time::sleep(self.resubscribe_delay).await;
            if let Ok(connection) = self.subscribe().await {
                return connection;
            }
        }
    }
}

impl NatsChannel {
    pub async fn connect(grant: &ChannelGrant) -> Result<Arc<dyn Channel>> {
        Self::connect_inbox(grant, MAX_RECONNECTS, RESUBSCRIBE_DELAY).await
    }

    pub(super) async fn connect_inbox(
        grant: &ChannelGrant,
        max_reconnects: usize,
        resubscribe_delay: Duration,
    ) -> Result<Arc<dyn Channel>> {
        let ChannelExecutorGrant::Nats {
            url,
            subject,
            token,
        } = &grant.executor
        else {
            bail!("executor grant is not a NATS channel grant");
        };
        if subject != &subject_for(&grant.channel_id)? {
            bail!("NATS subject does not match channel");
        }
        if grant.expires_at <= now_unix() {
            bail!("NATS channel grant has expired");
        }
        let (connect_url, proxy) = super::proxy::prepare(url).await?;
        let inbox = Inbox {
            url: connect_url,
            subject: subject.clone(),
            token: token.clone(),
            max_reconnects,
            resubscribe_delay,
        };
        let (client, subscriber) = inbox.subscribe().await?;
        let router = Arc::new(Router::new(&grant.channel_id));
        let client = Arc::new(Mutex::new(client));
        let task = tokio::spawn(receive(
            inbox,
            client.clone(),
            subscriber,
            router.clone(),
            grant.expires_at,
            proxy.as_ref().map(super::proxy::ProxyGuard::shutdown_token),
        ));
        Ok(Arc::new(Self {
            channel_id: grant.channel_id.clone(),
            handle: grant.client.clone(),
            router,
            client,
            task: Mutex::new(Some(task)),
            proxy: Mutex::new(proxy),
        }))
    }
}

impl Drop for NatsChannel {
    fn drop(&mut self) {
        self.router.close();
        if let Some(task) = self
            .task
            .get_mut()
            .unwrap_or_else(|e| e.into_inner())
            .take()
        {
            task.abort();
        }
    }
}

#[async_trait]
impl Channel for NatsChannel {
    fn channel_id(&self) -> &str {
        &self.channel_id
    }
    fn handle(&self) -> ChannelHandle {
        self.handle.clone()
    }
    async fn open(&self, ttl: Duration) -> Result<ChannelTicket> {
        let client = self
            .client
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        // A flush also waits for reconnect before the request is exposed to its client.
        tokio::time::timeout(CONNECT_TIMEOUT, client.flush())
            .await
            .map_err(|_| anyhow!("NATS channel is not ready"))?
            .map_err(|_| anyhow!("NATS channel is disconnected"))?;
        let request_id = new_request_id();
        let expires_at = ticket_deadline(&self.handle, ttl);
        if expires_at <= now_unix() {
            bail!("NATS channel grant has expired");
        }
        self.router.register(&request_id, expires_at)?;
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
        let Some(mut receiver) = self.router.take_receiver(&ticket.request_id) else {
            return Ok(ChannelOutcome::Closed);
        };
        let remaining = Duration::from_secs((ticket.expires_at - now_unix()).max(0) as u64);
        let cancel = cancel.unwrap_or_default();
        let result = tokio::select! {
            biased;
            value = &mut receiver => match value { Ok(value) => ChannelOutcome::Responded(value), Err(_) => ChannelOutcome::Closed },
            _ = self.router.cancelled.cancelled() => ChannelOutcome::Cancelled,
            _ = self.router.closed.cancelled() => ChannelOutcome::Closed,
            _ = cancel.cancelled() => ChannelOutcome::Cancelled,
            _ = tokio::time::sleep(remaining) => ChannelOutcome::Expired,
        };
        self.router.remove(&ticket.request_id);
        Ok(result)
    }
    async fn abandon(&self, ticket: &ChannelTicket) {
        self.router.remove(&ticket.request_id);
    }
    async fn drain_inbound(&self) -> Vec<Value> {
        self.router.drain()
    }
    async fn is_cancelled(&self) -> bool {
        self.router.cancelled.is_cancelled()
    }
    async fn close(&self) {
        self.router.close();
        let task = self.task.lock().unwrap_or_else(|e| e.into_inner()).take();
        if let Some(mut task) = task
            && tokio::time::timeout(Duration::from_secs(2), &mut task)
                .await
                .is_err()
        {
            task.abort();
        }
        drop(self.proxy.lock().unwrap_or_else(|e| e.into_inner()).take());
    }
}

async fn receive(
    inbox: Inbox,
    current: Arc<Mutex<async_nats::Client>>,
    mut subscriber: async_nats::Subscriber,
    router: Arc<Router>,
    expires_at: i64,
    proxy_shutdown: Option<CancellationToken>,
) {
    let mut client = current
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone();
    let expiry = tokio::time::sleep(Duration::from_secs((expires_at - now_unix()).max(0) as u64));
    tokio::pin!(expiry);
    loop {
        let message = tokio::select! {
            biased;
            _ = router.closed.cancelled() => break,
            _ = &mut expiry => break,
            message = subscriber.next() => message,
        };
        // The subscription ends once its client stops reconnecting.
        let Some(message) = message else {
            (client, subscriber) = tokio::select! {
                biased;
                _ = router.closed.cancelled() => break,
                _ = &mut expiry => break,
                connection = inbox.resubscribe() => connection,
            };
            *current.lock().unwrap_or_else(PoisonError::into_inner) = client.clone();
            continue;
        };
        let Some((envelope, reply)) = decode(&message, &inbox.subject) else {
            continue;
        };
        let ack = router.route(envelope);
        // A failed acknowledgement means the client is gone; its subscription ends next.
        if let Ok(payload) = serde_json::to_vec(&ack) {
            let _ = client.publish(reply, payload.into()).await;
        }
    }
    router.close();
    let _ = tokio::time::timeout(Duration::from_secs(1), client.drain()).await;
    if let Some(shutdown) = proxy_shutdown {
        shutdown.cancel();
    }
}

/// A well-formed push and the subject that acknowledges it.
fn decode(message: &async_nats::Message, subject: &str) -> Option<(Envelope, String)> {
    if message.payload.len() > MAX_PAYLOAD_BYTES {
        return None;
    }
    let envelope = serde_json::from_slice::<Envelope>(&message.payload).ok()?;
    if !valid_delivery_id(&envelope.id) {
        return None;
    }
    let reply = format!("{subject}.ack.{}", envelope.id);
    // Never let an untrusted reply field turn this executor into a publisher elsewhere.
    let expected = message.reply.as_ref().map(|value| value.as_str()) == Some(reply.as_str());
    expected.then_some((envelope, reply))
}
