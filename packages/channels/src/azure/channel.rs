//! Waiter side: one `json.webpubsub.azure.v1` connection per channel, joined to the run's
//! group before [`AzureWebPubSubChannel::connect`] returns (the service does not persist
//! messages), kept alive with periodic pings and rejoined with backoff after a socket loss.

use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use flow_like_types::channel::{
    Channel, ChannelExecutorGrant, ChannelGrant, ChannelHandle, ChannelOutcome, ChannelTicket,
    new_request_id, now_unix, ticket_deadline,
};
use flow_like_types::tokio_util::sync::CancellationToken;
use flow_like_types::{Value, async_trait};
use futures_util::{SinkExt, StreamExt};
use reqwest::Url;
use tokio::net::TcpStream;
use tokio::task::JoinHandle;
use tokio::time::{Instant, MissedTickBehavior};
use tokio_tungstenite::tungstenite::http::Uri;
use tokio_tungstenite::tungstenite::http::header::{AUTHORIZATION, SEC_WEBSOCKET_PROTOCOL};
use tokio_tungstenite::tungstenite::{ClientRequestBuilder, Message};
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream, connect_async};
use tracing::{debug, info, warn};

use super::protocol::{ClientFrame, SUBPROTOCOL};
use super::router::{FrameAction, Shared, handle_text};

const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
const JOIN_TIMEOUT: Duration = Duration::from_secs(10);
const PING_INTERVAL: Duration = Duration::from_secs(20);
const RECONNECT_MIN: Duration = Duration::from_millis(250);
const RECONNECT_MAX: Duration = Duration::from_secs(5);
const CLOSE_GRACE: Duration = Duration::from_secs(2);
const ACCESS_TOKEN_PARAM: &str = "access_token";

type Socket = WebSocketStream<MaybeTlsStream<TcpStream>>;

pub struct AzureWebPubSubChannel {
    channel_id: String,
    client: ChannelHandle,
    shared: Arc<Shared>,
    shutdown: CancellationToken,
    socket_task: Mutex<Option<JoinHandle<()>>>,
}

impl AzureWebPubSubChannel {
    /// Connects, joins the grant's group and waits for the join ack before returning.
    pub async fn connect(grant: &ChannelGrant) -> flow_like_types::Result<Arc<dyn Channel>> {
        let ChannelExecutorGrant::AzureWebPubSub { url, group } = &grant.executor else {
            flow_like_types::bail!(
                "channel {}: executor grant is not an Azure Web PubSub grant",
                grant.channel_id
            );
        };
        let connector = Connector::new(url, group)?;
        let shared = Arc::new(Shared::new(grant.channel_id.clone()));
        let session = tokio::time::timeout(CONNECT_TIMEOUT, Session::open(&connector, &shared))
            .await
            .map_err(|_| {
                flow_like_types::anyhow!(
                    "channel {}: connecting to Azure Web PubSub timed out after {CONNECT_TIMEOUT:?}",
                    grant.channel_id
                )
            })??;
        let shutdown = CancellationToken::new();
        let task = tokio::spawn(run_socket(
            session,
            connector,
            shared.clone(),
            shutdown.clone(),
        ));
        info!(channel_id = %grant.channel_id, group = %group, "joined Azure Web PubSub group");
        Ok(Arc::new(Self {
            channel_id: grant.channel_id.clone(),
            client: grant.client.clone(),
            shared,
            shutdown,
            socket_task: Mutex::new(Some(task)),
        }))
    }

    #[cfg(test)]
    pub(crate) fn detached(channel_id: &str, client: ChannelHandle) -> Self {
        Self {
            channel_id: channel_id.to_string(),
            client,
            shared: Arc::new(Shared::new(channel_id)),
            shutdown: CancellationToken::new(),
            socket_task: Mutex::new(None),
        }
    }

    #[cfg(test)]
    pub(crate) fn shared(&self) -> &Shared {
        &self.shared
    }
}

impl Drop for AzureWebPubSubChannel {
    fn drop(&mut self) {
        self.shutdown.cancel();
    }
}

#[async_trait]
impl Channel for AzureWebPubSubChannel {
    fn channel_id(&self) -> &str {
        &self.channel_id
    }

    fn handle(&self) -> ChannelHandle {
        self.client.clone()
    }

    async fn open(&self, ttl: Duration) -> flow_like_types::Result<ChannelTicket> {
        let request_id = new_request_id();
        let expires_at = ticket_deadline(&self.client, ttl);
        self.shared.register(&request_id);
        Ok(ChannelTicket {
            handle: self.client.for_request(&request_id, expires_at),
            request_id,
            expires_at,
        })
    }

    async fn wait(
        &self,
        ticket: &ChannelTicket,
        cancel: Option<CancellationToken>,
    ) -> flow_like_types::Result<ChannelOutcome> {
        let Some(mut receiver) = self.shared.take_receiver(&ticket.request_id) else {
            return Ok(ChannelOutcome::Closed);
        };
        let remaining = Duration::from_secs((ticket.expires_at - now_unix()).max(0) as u64);
        let cancel = cancel.unwrap_or_default();
        let outcome = tokio::select! {
            biased;
            result = &mut receiver => match result {
                Ok(value) => ChannelOutcome::Responded(value),
                Err(_) => ChannelOutcome::Closed,
            },
            _ = self.shared.cancelled.cancelled() => ChannelOutcome::Cancelled,
            _ = cancel.cancelled() => ChannelOutcome::Cancelled,
            _ = tokio::time::sleep(remaining) => ChannelOutcome::Expired,
        };
        self.abandon(ticket).await;
        Ok(outcome)
    }

    async fn abandon(&self, ticket: &ChannelTicket) {
        self.shared.forget(&ticket.request_id);
    }

    async fn drain_inbound(&self) -> Vec<Value> {
        self.shared.drain_inbound()
    }

    async fn is_cancelled(&self) -> bool {
        self.shared.cancelled.is_cancelled()
    }

    async fn close(&self) {
        self.shutdown.cancel();
        self.shared.clear();
        let task = self
            .socket_task
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        if let Some(task) = task
            && tokio::time::timeout(CLOSE_GRACE * 2, task).await.is_err()
        {
            debug!(channel_id = %self.channel_id, "Azure Web PubSub socket task did not finish in time");
        }
    }
}

/// Handshake request template: subprotocol set, token moved from the query string into the
/// `Authorization` header when the grant url carried it there.
struct Connector {
    request: ClientRequestBuilder,
    group: String,
}

impl Connector {
    fn new(url: &str, group: &str) -> flow_like_types::Result<Self> {
        let mut parsed = Url::parse(url)
            .map_err(|e| flow_like_types::anyhow!("invalid Azure Web PubSub client url: {e}"))?;
        let (token, rest): (Option<String>, Vec<(String, String)>) = {
            let mut token = None;
            let mut rest = Vec::new();
            for (key, value) in parsed.query_pairs() {
                if key == ACCESS_TOKEN_PARAM {
                    token = Some(value.into_owned());
                } else {
                    rest.push((key.into_owned(), value.into_owned()));
                }
            }
            (token, rest)
        };
        if token.is_some() {
            parsed.set_query(None);
            if !rest.is_empty() {
                parsed.query_pairs_mut().extend_pairs(rest);
            }
        }
        let uri: Uri = parsed
            .as_str()
            .parse()
            .map_err(|e| flow_like_types::anyhow!("invalid Azure Web PubSub client url: {e}"))?;
        let mut request = ClientRequestBuilder::new(uri).with_sub_protocol(SUBPROTOCOL);
        if let Some(token) = token {
            request = request.with_header(AUTHORIZATION.as_str(), format!("Bearer {token}"));
        }
        Ok(Self {
            request,
            group: group.to_string(),
        })
    }
}

enum Exit {
    Lost,
    Shutdown,
}

struct Session {
    socket: Socket,
    next_ack: u64,
}

impl Session {
    async fn open(connector: &Connector, shared: &Shared) -> flow_like_types::Result<Self> {
        let (socket, response) = connect_async(connector.request.clone())
            .await
            .map_err(|e| {
                flow_like_types::anyhow!(
                    "channel {}: Azure Web PubSub handshake failed: {e}",
                    shared.channel_id
                )
            })?;
        let negotiated = response
            .headers()
            .get(SEC_WEBSOCKET_PROTOCOL)
            .and_then(|value| value.to_str().ok());
        if negotiated != Some(SUBPROTOCOL) {
            flow_like_types::bail!(
                "channel {}: Azure Web PubSub did not negotiate the {SUBPROTOCOL} subprotocol (got {negotiated:?})",
                shared.channel_id
            );
        }
        let mut session = Self {
            socket,
            next_ack: 1,
        };
        session.join(connector, shared).await?;
        Ok(session)
    }

    fn next_ack(&mut self) -> u64 {
        let id = self.next_ack;
        self.next_ack += 1;
        id
    }

    async fn send(&mut self, frame: &ClientFrame<'_>) -> flow_like_types::Result<()> {
        let text = serde_json::to_string(frame)?;
        self.socket
            .send(Message::Text(text.into()))
            .await
            .map_err(|e| flow_like_types::anyhow!("sending {} frame: {e}", frame.kind()))
    }

    async fn join(
        &mut self,
        connector: &Connector,
        shared: &Shared,
    ) -> flow_like_types::Result<()> {
        let ack_id = self.next_ack();
        self.send(&ClientFrame::JoinGroup {
            group: &connector.group,
            ack_id,
        })
        .await?;
        tokio::time::timeout(JOIN_TIMEOUT, self.await_ack(ack_id, shared))
            .await
            .map_err(|_| {
                flow_like_types::anyhow!(
                    "channel {}: joinGroup {} was not acknowledged within {JOIN_TIMEOUT:?}",
                    shared.channel_id,
                    connector.group
                )
            })?
    }

    /// Routes everything that arrives while the ack is outstanding, so a push streamed
    /// between the initial-group join and the explicit ack is not lost.
    async fn await_ack(&mut self, ack_id: u64, shared: &Shared) -> flow_like_types::Result<()> {
        loop {
            let Some(next) = self.socket.next().await else {
                flow_like_types::bail!(
                    "channel {}: socket closed before joinGroup was acknowledged",
                    shared.channel_id
                );
            };
            let message = next.map_err(|e| {
                flow_like_types::anyhow!(
                    "channel {}: socket error before joinGroup was acknowledged: {e}",
                    shared.channel_id
                )
            })?;
            match message {
                Message::Text(text) => match handle_text(shared, text.as_str()) {
                    FrameAction::Ack {
                        ack_id: id,
                        success,
                        error,
                    } if id == ack_id => {
                        if success {
                            return Ok(());
                        }
                        flow_like_types::bail!(
                            "channel {}: joinGroup rejected: {}",
                            shared.channel_id,
                            error.unwrap_or_else(|| "unknown error".to_string())
                        );
                    }
                    FrameAction::Disconnected => flow_like_types::bail!(
                        "channel {}: disconnected by Azure Web PubSub while joining the group",
                        shared.channel_id
                    ),
                    _ => {}
                },
                Message::Ping(_) => self.socket.flush().await?,
                Message::Close(frame) => flow_like_types::bail!(
                    "channel {}: socket closed while joining the group: {frame:?}",
                    shared.channel_id
                ),
                _ => {}
            }
        }
    }

    async fn serve(&mut self, shared: &Shared, shutdown: &CancellationToken) -> Exit {
        let mut ping = tokio::time::interval_at(Instant::now() + PING_INTERVAL, PING_INTERVAL);
        ping.set_missed_tick_behavior(MissedTickBehavior::Delay);
        loop {
            tokio::select! {
                _ = shutdown.cancelled() => return Exit::Shutdown,
                _ = ping.tick() => {
                    if let Err(error) = self.send(&ClientFrame::Ping).await {
                        warn!(channel_id = %shared.channel_id, error = %error, "Azure Web PubSub ping failed");
                        return Exit::Lost;
                    }
                }
                next = self.socket.next() => match next {
                    None => {
                        warn!(channel_id = %shared.channel_id, "Azure Web PubSub socket ended");
                        return Exit::Lost;
                    }
                    Some(Err(error)) => {
                        warn!(channel_id = %shared.channel_id, error = %error, "Azure Web PubSub socket error");
                        return Exit::Lost;
                    }
                    Some(Ok(Message::Text(text))) => match handle_text(shared, text.as_str()) {
                        FrameAction::Disconnected => return Exit::Lost,
                        FrameAction::Ack { ack_id, success, error } => {
                            debug!(channel_id = %shared.channel_id, ack_id, success, error = error.as_deref().unwrap_or_default(), "Azure Web PubSub ack");
                        }
                        FrameAction::Continue => {}
                    },
                    Some(Ok(Message::Ping(_))) => {
                        if let Err(error) = self.socket.flush().await {
                            warn!(channel_id = %shared.channel_id, error = %error, "Azure Web PubSub pong failed");
                            return Exit::Lost;
                        }
                    }
                    Some(Ok(Message::Close(frame))) => {
                        debug!(channel_id = %shared.channel_id, ?frame, "Azure Web PubSub close frame");
                        return Exit::Lost;
                    }
                    Some(Ok(_)) => {}
                }
            }
        }
    }

    async fn leave_and_close(mut self, group: &str) {
        let ack_id = self.next_ack();
        let _ = self.send(&ClientFrame::LeaveGroup { group, ack_id }).await;
        let _ = self.socket.close(None).await;
        let _ = tokio::time::timeout(CLOSE_GRACE, async {
            while let Some(Ok(_)) = self.socket.next().await {}
        })
        .await;
    }
}

async fn run_socket(
    mut session: Session,
    connector: Connector,
    shared: Arc<Shared>,
    shutdown: CancellationToken,
) {
    loop {
        if let Exit::Shutdown = session.serve(&shared, &shutdown).await {
            session.leave_and_close(&connector.group).await;
            debug!(channel_id = %shared.channel_id, "left Azure Web PubSub group");
            return;
        }
        let Some(next) = reconnect(&connector, &shared, &shutdown).await else {
            return;
        };
        session = next;
    }
}

async fn reconnect(
    connector: &Connector,
    shared: &Shared,
    shutdown: &CancellationToken,
) -> Option<Session> {
    let mut delay = RECONNECT_MIN;
    loop {
        tokio::select! {
            _ = shutdown.cancelled() => return None,
            _ = tokio::time::sleep(delay) => {}
        }
        match Session::open(connector, shared).await {
            Ok(session) => {
                info!(channel_id = %shared.channel_id, group = %connector.group, "rejoined Azure Web PubSub group");
                return Some(session);
            }
            Err(error) => {
                warn!(channel_id = %shared.channel_id, error = %error, retry_in = ?delay, "Azure Web PubSub reconnect failed");
                delay = (delay * 2).min(RECONNECT_MAX);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use flow_like_types::channel::{ChannelClientDescriptor, ChannelPush, ChannelPushKind};
    use serde_json::json;
    use tokio_tungstenite::tungstenite::client::IntoClientRequest;

    fn client_handle(channel_id: &str) -> ChannelHandle {
        ChannelHandle {
            channel_id: channel_id.to_string(),
            request_id: None,
            expires_at: now_unix() + 3600,
            transport: ChannelClientDescriptor::AzureWebPubSub {
                url: "wss://demo.webpubsub.azure.com/client/hubs/h?access_token=t".into(),
                group: format!("run:{channel_id}"),
                expires_at: now_unix() + 3600,
            },
            fallback: None,
        }
    }

    fn reply_frame(channel_id: &str, request_id: &str, value: Value) -> String {
        json!({
            "type": "message",
            "from": "group",
            "group": format!("run:{channel_id}"),
            "dataType": "json",
            "data": {
                "channel_id": channel_id,
                "request_id": request_id,
                "kind": "reply",
                "value": value
            },
            "fromUserId": "user"
        })
        .to_string()
    }

    #[tokio::test]
    async fn reply_before_wait_is_delivered() {
        let channel = AzureWebPubSubChannel::detached("chan", client_handle("chan"));
        let ticket = channel.open(Duration::from_secs(30)).await.unwrap();
        assert_eq!(
            ticket.handle.request_id.as_deref(),
            Some(&*ticket.request_id)
        );
        assert_eq!(ticket.handle.channel_id, "chan");
        handle_text(
            channel.shared(),
            &reply_frame("chan", &ticket.request_id, json!({"ok": true})),
        );
        assert_eq!(
            channel.wait(&ticket, None).await.unwrap(),
            ChannelOutcome::Responded(json!({"ok": true}))
        );
        assert_eq!(
            channel.wait(&ticket, None).await.unwrap(),
            ChannelOutcome::Closed
        );
    }

    #[tokio::test]
    async fn waiter_is_woken_by_reply() {
        let channel = Arc::new(AzureWebPubSubChannel::detached(
            "chan",
            client_handle("chan"),
        ));
        let ticket = channel.open(Duration::from_secs(30)).await.unwrap();
        let waiter = {
            let channel = channel.clone();
            let ticket = ticket.clone();
            tokio::spawn(async move { channel.wait(&ticket, None).await.unwrap() })
        };
        tokio::time::sleep(Duration::from_millis(10)).await;
        handle_text(
            channel.shared(),
            &reply_frame("chan", &ticket.request_id, json!(7)),
        );
        assert_eq!(waiter.await.unwrap(), ChannelOutcome::Responded(json!(7)));
    }

    #[tokio::test]
    async fn cancel_push_wakes_waiter() {
        let channel = Arc::new(AzureWebPubSubChannel::detached(
            "chan",
            client_handle("chan"),
        ));
        let ticket = channel.open(Duration::from_secs(30)).await.unwrap();
        let waiter = {
            let channel = channel.clone();
            let ticket = ticket.clone();
            tokio::spawn(async move { channel.wait(&ticket, None).await.unwrap() })
        };
        tokio::time::sleep(Duration::from_millis(10)).await;
        channel.shared().route(ChannelPush {
            channel_id: "chan".into(),
            request_id: None,
            kind: ChannelPushKind::Cancel,
            value: Value::Null,
        });
        assert_eq!(waiter.await.unwrap(), ChannelOutcome::Cancelled);
        assert!(channel.is_cancelled().await);
    }

    #[tokio::test]
    async fn cancel_token_wakes_waiter() {
        let channel = AzureWebPubSubChannel::detached("chan", client_handle("chan"));
        let ticket = channel.open(Duration::from_secs(30)).await.unwrap();
        let token = CancellationToken::new();
        token.cancel();
        assert_eq!(
            channel.wait(&ticket, Some(token)).await.unwrap(),
            ChannelOutcome::Cancelled
        );
        assert!(!channel.is_cancelled().await);
    }

    #[tokio::test(start_paused = true)]
    async fn ticket_expires() {
        let channel = AzureWebPubSubChannel::detached("chan", client_handle("chan"));
        let ticket = channel.open(Duration::from_secs(1)).await.unwrap();
        assert_eq!(
            channel.wait(&ticket, None).await.unwrap(),
            ChannelOutcome::Expired
        );
        let late = channel.shared().route(ChannelPush {
            channel_id: "chan".into(),
            request_id: Some(ticket.request_id.clone()),
            kind: ChannelPushKind::Reply,
            value: Value::Null,
        });
        assert_eq!(late, super::super::router::RouteResult::UnknownRequest);
    }

    #[tokio::test]
    async fn close_wakes_waiter_with_closed() {
        let channel = Arc::new(AzureWebPubSubChannel::detached(
            "chan",
            client_handle("chan"),
        ));
        let ticket = channel.open(Duration::from_secs(30)).await.unwrap();
        let waiter = {
            let channel = channel.clone();
            let ticket = ticket.clone();
            tokio::spawn(async move { channel.wait(&ticket, None).await.unwrap() })
        };
        tokio::time::sleep(Duration::from_millis(10)).await;
        channel.close().await;
        assert_eq!(waiter.await.unwrap(), ChannelOutcome::Closed);
    }

    #[tokio::test]
    async fn abandon_then_wait_is_closed() {
        let channel = AzureWebPubSubChannel::detached("chan", client_handle("chan"));
        let ticket = channel.open(Duration::from_secs(30)).await.unwrap();
        channel.abandon(&ticket).await;
        assert_eq!(
            channel.wait(&ticket, None).await.unwrap(),
            ChannelOutcome::Closed
        );
    }

    #[tokio::test]
    async fn inbound_drains_through_channel() {
        let channel = AzureWebPubSubChannel::detached("chan", client_handle("chan"));
        channel.shared().route(ChannelPush {
            channel_id: "chan".into(),
            request_id: None,
            kind: ChannelPushKind::Inbound,
            value: json!("steer"),
        });
        assert_eq!(channel.drain_inbound().await, vec![json!("steer")]);
        assert!(channel.drain_inbound().await.is_empty());
        assert_eq!(channel.handle(), client_handle("chan"));
    }

    #[test]
    fn connector_moves_token_into_authorization_header() {
        let connector = Connector::new(
            "wss://demo.webpubsub.azure.com/client/hubs/h?access_token=abc.def&x=1",
            "run:chan",
        )
        .unwrap();
        let request = connector.request.into_client_request().unwrap();
        assert_eq!(request.uri().query(), Some("x=1"));
        assert_eq!(request.uri().path(), "/client/hubs/h");
        assert_eq!(
            request.headers().get(AUTHORIZATION).unwrap(),
            "Bearer abc.def"
        );
        assert_eq!(
            request.headers().get(SEC_WEBSOCKET_PROTOCOL).unwrap(),
            SUBPROTOCOL
        );
        assert_eq!(connector.group, "run:chan");
    }

    #[test]
    fn connector_keeps_url_without_token() {
        let connector =
            Connector::new("wss://demo.webpubsub.azure.com/client/hubs/h", "run:chan").unwrap();
        let request = connector.request.into_client_request().unwrap();
        assert_eq!(request.uri().query(), None);
        assert!(request.headers().get(AUTHORIZATION).is_none());
        assert!(Connector::new("not a url", "run:chan").is_err());
    }
}
