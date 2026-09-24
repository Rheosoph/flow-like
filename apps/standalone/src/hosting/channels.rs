use super::{BODY_LIMIT, HostState, read_token};
use anyhow::{Result, ensure};
use axum::{
    body::to_bytes,
    extract::Request,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use flow_like_types::{
    channel::{ChannelPush, InProcessChannel, InProcessPushResult},
    intercom::InterComCallback,
    utils::constant_time_eq,
};
use rand_core::RngCore;
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    path::Path,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use tokio_util::sync::CancellationToken;

#[derive(Default)]
pub(super) struct Channels {
    grants: Mutex<HashMap<String, Arc<Grant>>>,
    incarnation: Option<String>,
}

pub(super) struct Grant {
    id: String,
    push_url: String,
    token: zeroize::Zeroizing<String>,
    service_fingerprint: blake3::Hash,
    deadline: Instant,
    expires_at: i64,
    cancel: CancellationToken,
}
pub(super) struct Registration<'a> {
    pub grant: Arc<Grant>,
    registry: &'a Channels,
}
impl Drop for Registration<'_> {
    fn drop(&mut self) {
        if let Ok(mut registry) = self.registry.grants.lock() {
            registry.remove(&self.grant.id);
        }
    }
}
impl Channels {
    pub fn new(incarnation: Option<String>) -> Self {
        Self {
            grants: Mutex::default(),
            incarnation,
        }
    }
    pub fn register(
        &self,
        id: String,
        secret: &Path,
        timeout: Duration,
        cancel: CancellationToken,
        service_fingerprint: blake3::Hash,
    ) -> Result<Registration<'_>> {
        ensure!(
            blake3::hash(&read_token(secret)?) == service_fingerprint,
            "Service access was rotated before dispatch"
        );
        let mut random = [0_u8; 32];
        rand_core::OsRng.fill_bytes(&mut random);
        let grant = Arc::new(Grant {
            id: id.clone(),
            push_url: match &self.incarnation {
                Some(owner) => format!("/channels/{owner}/{id}"),
                None => format!("/channels/{id}"),
            },
            token: zeroize::Zeroizing::new(URL_SAFE_NO_PAD.encode(random)),
            service_fingerprint,
            deadline: Instant::now() + timeout,
            expires_at: flow_like_types::channel::now_unix() + timeout.as_secs() as i64,
            cancel,
        });
        self.grants
            .lock()
            .map_err(|_| anyhow::anyhow!("Run reply registry unavailable"))?
            .insert(id, grant.clone());
        Ok(Registration {
            grant,
            registry: self,
        })
    }
}
impl Grant {
    fn validate(&self, token: &str, secret: &Path) -> Result<()> {
        ensure!(
            Instant::now() < self.deadline && !self.cancel.is_cancelled(),
            "Run has ended"
        );
        ensure!(
            constant_time_eq(token.as_bytes(), self.token.as_bytes()),
            "Invalid run reply token"
        );
        ensure!(
            blake3::hash(&read_token(secret)?) == self.service_fingerprint,
            "Service access was rotated"
        );
        Ok(())
    }
    pub fn rewrite(&self, value: &mut Value) {
        match value {
            Value::Object(map) => {
                if map.get("channel_id").and_then(Value::as_str) == Some(&self.id)
                    && map
                        .get("transport")
                        .and_then(|v| v.get("type"))
                        .and_then(Value::as_str)
                        == Some("in_process")
                {
                    map.insert("transport".into(), json!({"type":"http", "push_url":self.push_url, "token":self.token.as_str()}));
                    map.remove("fallback");
                    let expires = map
                        .get("expires_at")
                        .and_then(Value::as_i64)
                        .unwrap_or(self.expires_at)
                        .min(self.expires_at);
                    map.insert("expires_at".into(), expires.into());
                }
                for child in map.values_mut() {
                    self.rewrite(child);
                }
            }
            Value::Array(array) => {
                for child in array {
                    self.rewrite(child);
                }
            }
            _ => {}
        }
    }
}

pub(super) fn wrap(callback: InterComCallback, grant: Arc<Grant>) -> InterComCallback {
    callback.map(|callback| {
        Arc::new(move |mut event: flow_like_types::intercom::InterComEvent| {
            grant.rewrite(&mut event.payload);
            callback(event)
        }) as _
    })
}

pub(super) async fn push(host: &HostState, request: Request, id: &str) -> Response {
    if request.method() != axum::http::Method::POST || request.uri().query().is_some() {
        return StatusCode::NOT_FOUND.into_response();
    }
    if request.headers().get_all("authorization").iter().count() != 1 {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let token = request
        .headers()
        .get("authorization")
        .and_then(|h| h.to_str().ok())
        .and_then(|h| h.strip_prefix("Bearer "))
        .unwrap_or_default()
        .to_owned();
    if token.len() != 43
        || !token
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))
    {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let id = if let Some((owner, id)) = id.split_once('/') {
        #[cfg(unix)]
        {
            let Some(route) = &host.reply_route else {
                return StatusCode::NOT_FOUND.into_response();
            };
            if !super::forward::valid_incarnation(owner) || !valid_id(id) {
                return StatusCode::NOT_FOUND.into_response();
            }
            if owner != route.incarnation {
                return route
                    .forward(request, owner, id, &token)
                    .await
                    .into_response();
            }
            id
        }
        #[cfg(not(unix))]
        {
            let _ = (owner, id);
            return StatusCode::NOT_FOUND.into_response();
        }
    } else {
        id
    };
    if !valid_id(id) {
        return StatusCode::NOT_FOUND.into_response();
    }
    let grant = match authorize(host, id, &token) {
        Ok(grant) => grant,
        Err(status) => return status.into_response(),
    };
    let result = tokio::time::timeout(Duration::from_secs(10), async {
        let body = to_bytes(request.into_body(), BODY_LIMIT).await?;
        deliver(host, &grant, &token, &body).await
    })
    .await;
    match result {
        Ok(Ok(InProcessPushResult::Delivered | InProcessPushResult::Duplicate)) => {
            StatusCode::NO_CONTENT
        }
        Ok(Ok(InProcessPushResult::Full)) => StatusCode::TOO_MANY_REQUESTS,
        Ok(Ok(_)) => StatusCode::GONE,
        Ok(Err(_)) => StatusCode::BAD_REQUEST,
        Err(_) => StatusCode::REQUEST_TIMEOUT,
    }
    .into_response()
}

pub(super) fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 128
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))
}

pub(super) fn authorize(
    host: &HostState,
    id: &str,
    token: &str,
) -> std::result::Result<Arc<Grant>, StatusCode> {
    let grant = host
        .channels
        .grants
        .lock()
        .ok()
        .and_then(|registry| registry.get(id).cloned())
        .ok_or(StatusCode::NOT_FOUND)?;
    grant
        .validate(token, &host.secret)
        .map_err(|_| StatusCode::UNAUTHORIZED)?;
    Ok(grant)
}

pub(super) async fn deliver(
    host: &HostState,
    grant: &Grant,
    token: &str,
    body: &[u8],
) -> Result<InProcessPushResult> {
    let push: ChannelPush = serde_json::from_slice(body)?;
    ensure!(push.channel_id == grant.id, "Reply is for another run");
    grant.validate(token, &host.secret)?;
    // The registry can disappear while the request body is arriving.
    ensure!(
        host.channels
            .grants
            .lock()
            .map_err(|_| anyhow::anyhow!("Run registry unavailable"))?
            .contains_key(&grant.id),
        "Run ended"
    );
    if matches!(push.kind, flow_like_types::channel::ChannelPushKind::Cancel) {
        grant.cancel.cancel();
    }
    Ok(InProcessChannel::deliver(push).await)
}
