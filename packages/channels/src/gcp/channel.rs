//! Waiter side: the executor's [`Channel`] over two Firebase REST SSE streams.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use flow_like_types::channel::{
    Channel, ChannelExecutorGrant, ChannelGrant, ChannelHandle, ChannelOutcome, ChannelTicket,
    new_request_id, now_unix, ticket_deadline,
};
use flow_like_types::tokio_util::sync::CancellationToken;
use flow_like_types::{Value, anyhow, async_trait};
use reqwest::Url;
use tokio::sync::watch;
use tokio::task::JoinHandle;

use super::auth::FirebaseAuth;
use super::router::{Flags, Router, StreamKind, Subscription};
use super::stream::{self, StreamConfig};
use super::{database_root, json_url, path_segments};

/// The sign-in that gates the connection: the streams behind it are long-lived, so the ceiling
/// sits on this call rather than on the shared client.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);

pub struct FirebaseRtdbChannel {
    channel_id: String,
    handle: ChannelHandle,
    router: Arc<Router>,
    auth: Arc<FirebaseAuth>,
    client: reqwest::Client,
    root: Url,
    stop: CancellationToken,
    tasks: Mutex<Vec<JoinHandle<()>>>,
}

impl FirebaseRtdbChannel {
    /// Sign in with the grant's custom token and start streaming both collections. Fails fast
    /// when the credentials or the database url are unusable.
    pub async fn connect(grant: &ChannelGrant) -> flow_like_types::Result<Arc<dyn Channel>> {
        let ChannelExecutorGrant::GcpFirebaseRtdb {
            database_url,
            api_key,
            custom_token,
            inbox_path,
            inbound_path,
        } = &grant.executor
        else {
            return Err(anyhow!(
                "channel {}: grant is not a firebase realtime database grant",
                grant.channel_id
            ));
        };
        let root = database_root(database_url)?;
        let inbox_url = json_url(&root, &path_segments(inbox_path))?;
        let inbound_url = json_url(&root, &path_segments(inbound_path))?;
        let client = reqwest::Client::builder()
            .build()
            .map_err(|err| anyhow!("channel {}: http client: {err}", grant.channel_id))?;
        let auth = Arc::new(FirebaseAuth::new(
            client.clone(),
            api_key.clone(),
            custom_token.clone(),
        ));
        flow_like_types::tokio::time::timeout(CONNECT_TIMEOUT, auth.id_token())
            .await
            .map_err(|_| {
                anyhow!(
                    "channel {}: firebase sign-in timed out after {CONNECT_TIMEOUT:?}",
                    grant.channel_id
                )
            })?
            .map_err(|err| {
                anyhow!(
                    "channel {}: firebase sign-in failed: {err}",
                    grant.channel_id
                )
            })?;

        let router = Arc::new(Router::new(&grant.channel_id));
        let stop = CancellationToken::new();
        let tasks = [
            (StreamKind::Inbox, inbox_url),
            (StreamKind::Inbound, inbound_url),
        ]
        .into_iter()
        .map(|(kind, url)| {
            tokio::spawn(stream::run(StreamConfig {
                client: client.clone(),
                url,
                kind,
                auth: auth.clone(),
                router: router.clone(),
                stop: stop.clone(),
            }))
        })
        .collect();
        tracing::debug!(channel = %grant.channel_id, "firebase channel connected");
        Ok(Arc::new(Self {
            channel_id: grant.channel_id.clone(),
            handle: grant.client.clone(),
            router,
            auth,
            client,
            root,
            stop,
            tasks: Mutex::new(tasks),
        }))
    }

    async fn await_reply(
        &self,
        ticket: &ChannelTicket,
        cancel: Option<CancellationToken>,
    ) -> ChannelOutcome {
        let receiver = match self.router.subscribe(&ticket.request_id) {
            Subscription::Ready(value) => return ChannelOutcome::Responded(value),
            Subscription::Pending(receiver) => receiver,
            Subscription::Unknown => return ChannelOutcome::Closed,
        };
        let remaining = (ticket.expires_at - now_unix()).max(0) as u64;
        let expiry = tokio::time::sleep(Duration::from_secs(remaining));
        tokio::pin!(expiry);
        let cancel = cancel.unwrap_or_default();
        let mut flags = self.router.flags();
        tokio::select! {
            reply = receiver => match reply {
                Ok(value) => ChannelOutcome::Responded(value),
                Err(_) => ChannelOutcome::Closed,
            },
            _ = &mut expiry => ChannelOutcome::Expired,
            _ = cancel.cancelled() => ChannelOutcome::Cancelled,
            flags = wait_flags(&mut flags) => if flags.closed {
                ChannelOutcome::Closed
            } else {
                ChannelOutcome::Cancelled
            },
        }
    }

    async fn delete_remote(&self) {
        let token = match self.auth.id_token().await {
            Ok(token) => token,
            Err(err) => {
                tracing::warn!(channel = %self.channel_id, error = %err, "firebase channel cleanup skipped: no id token");
                return;
            }
        };
        let mut url = match json_url(&self.root, &["channels", &self.channel_id]) {
            Ok(url) => url,
            Err(err) => {
                tracing::warn!(channel = %self.channel_id, error = %err, "firebase channel cleanup skipped");
                return;
            }
        };
        url.query_pairs_mut()
            .append_pair("auth", &token)
            .append_pair("print", "silent");
        match self.client.delete(url).send().await {
            Ok(response) if response.status().is_success() => {
                tracing::debug!(channel = %self.channel_id, "firebase channel deleted");
            }
            Ok(response) => {
                tracing::warn!(channel = %self.channel_id, status = %response.status(), "firebase channel delete rejected");
            }
            Err(err) => {
                tracing::warn!(channel = %self.channel_id, error = %err, "firebase channel delete failed");
            }
        }
    }

    #[cfg(test)]
    fn for_tests(channel_id: &str) -> Self {
        let client = reqwest::Client::new();
        Self {
            channel_id: channel_id.to_string(),
            handle: ChannelHandle {
                channel_id: channel_id.to_string(),
                request_id: None,
                expires_at: now_unix() + 3600,
                transport: flow_like_types::channel::ChannelClientDescriptor::GcpFirebaseRtdb {
                    database_url: "https://demo.firebaseio.com".into(),
                    api_key: "key".into(),
                    project_id: "demo".into(),
                    custom_token: "token".into(),
                    inbox_path: format!("/channels/{channel_id}/inbox"),
                    inbound_path: format!("/channels/{channel_id}/inbound"),
                    expires_at: now_unix() + 3600,
                },
                fallback: None,
            },
            router: Arc::new(Router::new(channel_id)),
            auth: Arc::new(FirebaseAuth::new(
                client.clone(),
                "key".into(),
                "token".into(),
            )),
            client,
            root: database_root("https://demo.firebaseio.com").unwrap(),
            stop: CancellationToken::new(),
            tasks: Mutex::new(Vec::new()),
        }
    }
}

async fn wait_flags(receiver: &mut watch::Receiver<Flags>) -> Flags {
    match receiver
        .wait_for(|flags| flags.cancelled || flags.closed)
        .await
    {
        Ok(flags) => *flags,
        Err(_) => Flags {
            cancelled: false,
            closed: true,
        },
    }
}

#[async_trait]
impl Channel for FirebaseRtdbChannel {
    fn channel_id(&self) -> &str {
        &self.channel_id
    }

    fn handle(&self) -> ChannelHandle {
        self.handle.clone()
    }

    async fn open(&self, ttl: Duration) -> flow_like_types::Result<ChannelTicket> {
        let request_id = new_request_id();
        let expires_at = ticket_deadline(&self.handle, ttl);
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
    ) -> flow_like_types::Result<ChannelOutcome> {
        let outcome = self.await_reply(ticket, cancel).await;
        self.router.remove(&ticket.request_id);
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
        self.stop.cancel();
        let tasks: Vec<JoinHandle<()>> = self
            .tasks
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .drain(..)
            .collect();
        for task in tasks {
            task.abort();
        }
        self.router.close();
        self.delete_remote().await;
    }
}

impl Drop for FirebaseRtdbChannel {
    fn drop(&mut self) {
        self.stop.cancel();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use flow_like_types::channel::{ChannelPush, ChannelPushKind};
    use serde_json::json;

    fn reply_frame(channel: &str, request_id: &str, value: Value) -> String {
        let push = ChannelPush {
            channel_id: channel.into(),
            request_id: Some(request_id.into()),
            kind: ChannelPushKind::Reply,
            value,
        };
        json!({ "path": format!("/{request_id}"), "data": { "payload": serde_json::to_string(&push).unwrap() } })
            .to_string()
    }

    #[tokio::test]
    async fn ticket_carries_a_request_handle() {
        let channel = FirebaseRtdbChannel::for_tests("run-h");
        let ticket = channel.open(Duration::from_secs(30)).await.unwrap();
        assert_eq!(
            ticket.handle.request_id.as_deref(),
            Some(ticket.request_id.as_str())
        );
        assert_eq!(ticket.handle.channel_id, "run-h");
        assert_eq!(ticket.handle.expires_at, ticket.expires_at);
        assert!(channel.handle().request_id.is_none());
        assert_eq!(channel.channel_id(), "run-h");
    }

    #[tokio::test]
    async fn reply_before_wait_is_returned_immediately() {
        let channel = FirebaseRtdbChannel::for_tests("run-a");
        let ticket = channel.open(Duration::from_secs(30)).await.unwrap();
        channel.router.handle_event(
            StreamKind::Inbox,
            "put",
            &reply_frame("run-a", &ticket.request_id, json!({ "ok": true })),
        );
        assert_eq!(
            channel.wait(&ticket, None).await.unwrap(),
            ChannelOutcome::Responded(json!({ "ok": true }))
        );
        assert_eq!(
            channel.wait(&ticket, None).await.unwrap(),
            ChannelOutcome::Closed
        );
    }

    #[tokio::test]
    async fn reply_during_wait_wakes_the_waiter() {
        let channel = Arc::new(FirebaseRtdbChannel::for_tests("run-b"));
        let ticket = channel.open(Duration::from_secs(30)).await.unwrap();
        let waiter = {
            let channel = channel.clone();
            let ticket = ticket.clone();
            tokio::spawn(async move { channel.wait(&ticket, None).await.unwrap() })
        };
        tokio::time::sleep(Duration::from_millis(20)).await;
        channel.router.handle_event(
            StreamKind::Inbox,
            "put",
            &reply_frame("run-b", &ticket.request_id, json!(42)),
        );
        assert_eq!(waiter.await.unwrap(), ChannelOutcome::Responded(json!(42)));
    }

    #[tokio::test(start_paused = true)]
    async fn expiry_cancel_token_and_cancel_push() {
        let channel = Arc::new(FirebaseRtdbChannel::for_tests("run-c"));
        let ticket = channel.open(Duration::from_secs(1)).await.unwrap();
        assert_eq!(
            channel.wait(&ticket, None).await.unwrap(),
            ChannelOutcome::Expired
        );

        let ticket = channel.open(Duration::from_secs(60)).await.unwrap();
        let token = CancellationToken::new();
        let waiter = {
            let channel = channel.clone();
            let ticket = ticket.clone();
            let token = token.clone();
            tokio::spawn(async move { channel.wait(&ticket, Some(token)).await.unwrap() })
        };
        tokio::time::sleep(Duration::from_millis(20)).await;
        token.cancel();
        assert_eq!(waiter.await.unwrap(), ChannelOutcome::Cancelled);

        let ticket = channel.open(Duration::from_secs(60)).await.unwrap();
        let waiter = {
            let channel = channel.clone();
            let ticket = ticket.clone();
            tokio::spawn(async move { channel.wait(&ticket, None).await.unwrap() })
        };
        tokio::time::sleep(Duration::from_millis(20)).await;
        channel.router.deliver(
            ChannelPush {
                channel_id: "run-c".into(),
                request_id: None,
                kind: ChannelPushKind::Cancel,
                value: Value::Null,
            },
            "cancel-1",
        );
        assert_eq!(waiter.await.unwrap(), ChannelOutcome::Cancelled);
        assert!(channel.is_cancelled().await);
        assert_eq!(
            channel
                .wait(&channel.open(Duration::from_secs(60)).await.unwrap(), None)
                .await
                .unwrap(),
            ChannelOutcome::Cancelled
        );
    }

    #[tokio::test]
    async fn abandon_and_close_release_registrations() {
        let channel = Arc::new(FirebaseRtdbChannel::for_tests("run-d"));
        let ticket = channel.open(Duration::from_secs(60)).await.unwrap();
        channel.abandon(&ticket).await;
        channel.router.handle_event(
            StreamKind::Inbox,
            "put",
            &reply_frame("run-d", &ticket.request_id, json!(1)),
        );
        assert_eq!(
            channel.wait(&ticket, None).await.unwrap(),
            ChannelOutcome::Closed
        );

        let ticket = channel.open(Duration::from_secs(60)).await.unwrap();
        let waiter = {
            let channel = channel.clone();
            let ticket = ticket.clone();
            tokio::spawn(async move { channel.wait(&ticket, None).await.unwrap() })
        };
        tokio::time::sleep(Duration::from_millis(20)).await;
        channel.router.close();
        assert_eq!(waiter.await.unwrap(), ChannelOutcome::Closed);

        channel.router.deliver(
            ChannelPush {
                channel_id: "run-d".into(),
                request_id: None,
                kind: ChannelPushKind::Inbound,
                value: json!("steer"),
            },
            "n1",
        );
        assert_eq!(channel.drain_inbound().await, vec![json!("steer")]);
        assert!(channel.drain_inbound().await.is_empty());
    }
}
