//! API side: writes pushes received on the HTTP fallback into the database with a
//! service-account OAuth token (`?access_token=`, bypasses the rules), plus channel lifecycle
//! writes the API owns (`meta` on create, delete on sweep).

use std::sync::Arc;

use flow_like_types::channel::{CHANNEL_TRANSPORT_GCP_FIREBASE_RTDB, ChannelPush, ChannelPushKind};
use flow_like_types::{anyhow, async_trait};
use futures_util::future::BoxFuture;
use reqwest::header::IF_MATCH;
use reqwest::{Method, StatusCode, Url};
use serde_json::{Value, json};

use super::{database_root, json_url, validate_channel_id, validate_key};
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
        Ok(Self {
            client: reqwest::Client::new(),
            root: database_root(database_url)?,
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
            .map_err(|err| anyhow!("firebase {method} {path} failed: {err}"))?;
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
}
