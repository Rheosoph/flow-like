//! API side: writes pushes received on the HTTP fallback into the database with a
//! service-account OAuth token (`?access_token=`, bypasses the rules), plus channel lifecycle
//! writes the API owns (`meta` on create, delete on sweep).

use std::sync::Arc;

use flow_like_types::channel::{CHANNEL_TRANSPORT_GCP_FIREBASE_RTDB, ChannelPush, ChannelPushKind};
use flow_like_types::realtime::{MAX_REALTIME_WAKEUP_BYTES, RealtimeReplyState};
use flow_like_types::{anyhow, async_trait};
use futures_util::future::BoxFuture;
use reqwest::header::IF_MATCH;
use reqwest::{Method, StatusCode, Url};
use serde_json::{Value, json};

use super::{
    REALTIME_REPLY_NODE, REALTIME_WAKE_NODE, database_root, json_url, validate_channel_id,
    validate_key,
};
use crate::ChannelForwarder;

/// Yields a Google OAuth access token for the service account (scopes
/// `https://www.googleapis.com/auth/firebase.database` and
/// `https://www.googleapis.com/auth/userinfo.email`). Called per request; cache inside.
pub type AccessTokenProvider =
    Arc<dyn Fn() -> BoxFuture<'static, flow_like_types::Result<String>> + Send + Sync>;

/// ETag of a location without data; `if-match` on it makes a `PUT` create-only.
const NULL_ETAG: &str = "null_etag";

pub struct FirebaseRtdbForwarder {
    client: reqwest::Client,
    root: Url,
    access_token: AccessTokenProvider,
}

impl FirebaseRtdbForwarder {
    pub fn new(
        database_url: &str,
        access_token: AccessTokenProvider,
    ) -> flow_like_types::Result<Self> {
        let root = database_root(database_url)?;
        let client = reqwest::Client::builder()
            .redirect(super::redirect_policy(root.host_str().unwrap_or_default()))
            .build()
            .map_err(|err| flow_like_types::anyhow!("firebase http client: {err}"))?;
        Ok(Self {
            client,
            root,
            access_token,
        })
    }

    /// `PUT /channels/{channel_id}/meta` with the server timestamp the sweeper keys on.
    pub async fn create_channel_meta(
        &self,
        channel_id: &str,
        owner: &str,
    ) -> flow_like_types::Result<()> {
        validate_channel_id(channel_id)?;
        let body = json!({ "owner": owner, "created_at": { ".sv": "timestamp" } });
        self.write(
            Method::PUT,
            &["channels", channel_id, "meta"],
            Some(&body),
            None,
        )
        .await
        .map(drop)
    }

    /// `DELETE /channels/{channel_id}`: the whole run node, including anything the client left.
    pub async fn delete_channel(&self, channel_id: &str) -> flow_like_types::Result<()> {
        validate_channel_id(channel_id)?;
        self.write(Method::DELETE, &["channels", channel_id], None, None)
            .await
            .map(drop)
    }

    /// Overwrite one wakeup marker, so an account never accumulates a second event ledger.
    pub async fn publish_realtime(
        &self,
        scope: &str,
        payload: &[u8],
    ) -> flow_like_types::Result<()> {
        self.put_realtime(scope, REALTIME_WAKE_NODE, payload)
            .await
            .map(drop)
    }

    /// Overwrite the account's reply node, never its wakeup node. Each message carries the
    /// reply's whole text so far, so an overwritten one loses nothing. A final one is removed
    /// once written: the stored message takes over and the database keeps no conversation text.
    pub async fn publish_realtime_reply(
        &self,
        scope: &str,
        payload: &[u8],
    ) -> flow_like_types::Result<()> {
        let body = self
            .put_realtime(scope, REALTIME_REPLY_NODE, payload)
            .await?;
        let state = serde_json::from_value::<RealtimeReplyState>(body["state"].clone());
        if !state.is_ok_and(RealtimeReplyState::is_final) {
            return Ok(());
        }
        self.write(
            Method::DELETE,
            &["realtime", scope, REALTIME_REPLY_NODE],
            None,
            None,
        )
        .await
        .map(drop)
    }

    async fn put_realtime(
        &self,
        scope: &str,
        node: &str,
        payload: &[u8],
    ) -> flow_like_types::Result<Value> {
        if scope.len() != 64 || !scope.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            flow_like_types::bail!("invalid realtime scope");
        }
        if payload.len() > MAX_REALTIME_WAKEUP_BYTES {
            flow_like_types::bail!(
                "realtime {node} message is {} bytes, the limit is {MAX_REALTIME_WAKEUP_BYTES}",
                payload.len()
            );
        }
        let body = serde_json::from_slice(payload)?;
        self.write(Method::PUT, &["realtime", scope, node], Some(&body), None)
            .await?;
        Ok(body)
    }

    async fn write(
        &self,
        method: Method,
        segments: &[&str],
        body: Option<&Value>,
        if_match: Option<&str>,
    ) -> flow_like_types::Result<StatusCode> {
        let path = segments.join("/");
        let token = (self.access_token)()
            .await
            .map_err(|err| anyhow!("firebase {method} {path}: access token: {err}"))?;
        let mut url = json_url(&self.root, segments)?;
        url.query_pairs_mut()
            .append_pair("access_token", &token)
            .append_pair("print", "silent");
        let mut request = self.client.request(method.clone(), url);
        if let Some(body) = body {
            request = request.json(body);
        }
        if let Some(etag) = if_match {
            request = request.header(IF_MATCH, etag);
        }
        let response = request
            .send()
            .await
            .map_err(|err| anyhow!("firebase {method} {path} failed: {}", err.without_url()))?;
        let status = response.status();
        if status.is_success() || status == StatusCode::PRECONDITION_FAILED {
            return Ok(status);
        }
        let text = response.text().await.unwrap_or_default();
        Err(anyhow!(
            "firebase {method} {path} returned {status}: {}",
            text.chars().take(300).collect::<String>()
        ))
    }
}

#[async_trait]
impl ChannelForwarder for FirebaseRtdbForwarder {
    fn transport(&self) -> &'static str {
        CHANNEL_TRANSPORT_GCP_FIREBASE_RTDB
    }

    /// Replies are `PUT` create-only at `inbox/{request_id}` (`if-match: null_etag`); a 412
    /// means the client already answered through the transport and is treated as delivered,
    /// matching the waiter's first-reply-wins. Inbound and cancel pushes are `POST`ed under
    /// `inbound` with a server-generated push id.
    async fn forward(&self, push: &ChannelPush) -> flow_like_types::Result<()> {
        validate_channel_id(&push.channel_id)?;
        let payload = json!({ "payload": serde_json::to_string(push)? });
        match push.kind {
            ChannelPushKind::Reply => {
                let request_id = push
                    .request_id
                    .as_deref()
                    .filter(|id| !id.is_empty())
                    .ok_or_else(|| {
                        anyhow!("channel {}: reply push has no request_id", push.channel_id)
                    })?;
                validate_key("request id", request_id)?;
                let status = self
                    .write(
                        Method::PUT,
                        &["channels", &push.channel_id, "inbox", request_id],
                        Some(&payload),
                        Some(NULL_ETAG),
                    )
                    .await?;
                if status == StatusCode::PRECONDITION_FAILED {
                    tracing::debug!(
                        channel = %push.channel_id,
                        request = request_id,
                        "reply already present in firebase; first reply wins"
                    );
                }
                Ok(())
            }
            ChannelPushKind::Inbound | ChannelPushKind::Cancel => self
                .write(
                    Method::POST,
                    &["channels", &push.channel_id, "inbound"],
                    Some(&payload),
                    None,
                )
                .await
                .map(drop),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn forwarder() -> FirebaseRtdbForwarder {
        FirebaseRtdbForwarder::new(
            "https://demo.europe-west1.firebasedatabase.app",
            Arc::new(|| Box::pin(async { Ok("token".to_string()) })),
        )
        .unwrap()
    }

    #[tokio::test]
    async fn rejects_malformed_pushes_before_any_request() {
        let forwarder = forwarder();
        assert_eq!(forwarder.transport(), "gcp_firebase_rtdb");
        let err = forwarder
            .forward(&ChannelPush {
                channel_id: "run-1".into(),
                request_id: None,
                kind: ChannelPushKind::Reply,
                value: Value::Null,
            })
            .await
            .unwrap_err();
        assert!(err.to_string().contains("request_id"), "{err}");
        let err = forwarder
            .forward(&ChannelPush {
                channel_id: "run/1".into(),
                request_id: Some("r".into()),
                kind: ChannelPushKind::Reply,
                value: Value::Null,
            })
            .await
            .unwrap_err();
        assert!(err.to_string().contains("channel id"), "{err}");
        let err = forwarder
            .forward(&ChannelPush {
                channel_id: "run-1".into(),
                request_id: Some("r.1".into()),
                kind: ChannelPushKind::Reply,
                value: Value::Null,
            })
            .await
            .unwrap_err();
        assert!(err.to_string().contains("request id"), "{err}");
        assert!(forwarder.delete_channel("").await.is_err());
        assert!(forwarder.create_channel_meta("a#b", "owner").await.is_err());
        assert!(
            FirebaseRtdbForwarder::new("nope", Arc::new(|| Box::pin(async { Ok(String::new()) })))
                .is_err()
        );
    }

    type Recorded = Arc<std::sync::Mutex<Vec<(String, String)>>>;

    /// A database that answers every request with an empty 200 and records its method, path
    /// and body.
    async fn recording_database() -> (String, Recorded) {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let recorded = Recorded::default();
        let requests = recorded.clone();
        tokio::spawn(async move {
            while let Ok((mut socket, _)) = listener.accept().await {
                let mut head = Vec::new();
                let mut byte = [0u8; 1];
                while !head.ends_with(b"\r\n\r\n") && socket.read(&mut byte).await.unwrap_or(0) == 1
                {
                    head.push(byte[0]);
                }
                let head = String::from_utf8_lossy(&head).into_owned();
                let length = head
                    .lines()
                    .find_map(|line| {
                        let line = line.to_ascii_lowercase();
                        line.strip_prefix("content-length:")
                            .and_then(|value| value.trim().parse::<usize>().ok())
                    })
                    .unwrap_or(0);
                let mut body = vec![0; length];
                let _ = socket.read_exact(&mut body).await;
                let mut request_line = head.split_whitespace();
                let method = request_line.next().unwrap_or_default();
                let path = request_line
                    .next()
                    .and_then(|target| target.split('?').next())
                    .unwrap_or_default();
                requests.lock().unwrap().push((
                    format!("{method} {path}"),
                    String::from_utf8_lossy(&body).into_owned(),
                ));
                let _ = socket
                    .write_all(b"HTTP/1.1 200 OK\r\ncontent-length: 0\r\nconnection: close\r\n\r\n")
                    .await;
            }
        });
        (url, recorded)
    }

    fn reply_message(state: &str) -> Vec<u8> {
        serde_json::to_vec(&json!({
            "protocol_version": 1, "kind": "reply", "conversation_id": "c-1",
            "message_id": "re-1", "reply_to": "m-1", "sender_id": "bot-1",
            "offset": 0, "text": "Hello", "state": state, "later_field": 3
        }))
        .unwrap()
    }

    async fn recorded_forwarder() -> (FirebaseRtdbForwarder, Recorded) {
        let (url, recorded) = recording_database().await;
        let forwarder =
            FirebaseRtdbForwarder::new(&url, Arc::new(|| Box::pin(async { Ok("t".into()) })))
                .unwrap();
        (forwarder, recorded)
    }

    #[tokio::test]
    async fn wakeups_and_replies_never_share_a_node() {
        let (forwarder, recorded) = recorded_forwarder().await;
        let scope = "a".repeat(64);
        let wakeup = br#"{"protocol_version":1,"kind":"invalidate","nonce":"n"}"#;
        forwarder.publish_realtime(&scope, wakeup).await.unwrap();
        for state in ["writing", "working", "done", "dropped"] {
            forwarder
                .publish_realtime_reply(&scope, &reply_message(state))
                .await
                .unwrap();
        }
        forwarder.publish_realtime(&scope, wakeup).await.unwrap();

        let wake = format!("/realtime/{scope}/wake.json");
        let reply = format!("/realtime/{scope}/reply.json");
        let requests = recorded.lock().unwrap().clone();
        let lines: Vec<_> = requests.iter().map(|(line, _)| line.as_str()).collect();
        assert_eq!(
            lines,
            [
                format!("PUT {wake}"),
                format!("PUT {reply}"),
                format!("PUT {reply}"),
                format!("PUT {reply}"),
                format!("DELETE {reply}"),
                format!("PUT {reply}"),
                format!("DELETE {reply}"),
                format!("PUT {wake}"),
            ]
        );
        for (line, body) in &requests {
            let body: Value = serde_json::from_str(body).unwrap_or(Value::Null);
            if line.starts_with("PUT") && line.ends_with("/wake.json") {
                assert_eq!(body["kind"], "invalidate", "{line}");
            } else if line.starts_with("PUT") {
                assert_eq!(body["kind"], "reply", "{line}");
                assert_eq!(body["later_field"], 3, "a reply passes through whole");
            }
        }
    }

    #[tokio::test]
    async fn a_reply_over_the_limit_or_for_a_foreign_path_sends_nothing() {
        let (forwarder, recorded) = recorded_forwarder().await;
        let oversized = vec![b' '; MAX_REALTIME_WAKEUP_BYTES + 1];
        let refused = [
            forwarder
                .publish_realtime_reply(&"a".repeat(64), &oversized)
                .await,
            forwarder
                .publish_realtime_reply("../wake", &reply_message("writing"))
                .await,
        ];
        assert!(refused.iter().all(Result::is_err));
        assert!(recorded.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn transport_failures_never_echo_the_access_token() {
        let forwarder = FirebaseRtdbForwarder::new(
            "http://127.0.0.1:0",
            Arc::new(|| Box::pin(async { Ok("secret-oauth-token".to_string()) })),
        )
        .unwrap();
        let push = ChannelPush {
            channel_id: "run-1".into(),
            request_id: None,
            kind: ChannelPushKind::Inbound,
            value: Value::Null,
        };
        let errors = [
            forwarder.forward(&push).await.unwrap_err(),
            forwarder
                .publish_realtime(&"a".repeat(64), b"{}")
                .await
                .unwrap_err(),
        ];
        for error in errors {
            assert!(error.to_string().contains("failed"), "{error}");
            for rendered in [
                error.to_string(),
                format!("{error:#}"),
                format!("{error:?}"),
            ] {
                assert!(!rendered.contains("secret-oauth-token"), "{rendered}");
                assert!(!rendered.contains("access_token="), "{rendered}");
            }
        }
    }
}
