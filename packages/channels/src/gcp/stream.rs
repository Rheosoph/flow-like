//! One REST SSE stream (`inbox` or `inbound`) kept open for the channel's lifetime with idle
//! detection, backoff and token renewal.

use std::sync::Arc;
use std::time::Duration;

use eventsource_stream::Eventsource;
use flow_like_types::tokio_util::sync::CancellationToken;
use futures_util::StreamExt;
use reqwest::header::ACCEPT;
use reqwest::{StatusCode, Url};

use super::auth::FirebaseAuth;
use super::router::{Router, StreamAction, StreamKind};

/// Firebase sends `keep-alive` every ~30 s; silence beyond this means the connection is dead.
pub(crate) const IDLE_TIMEOUT: Duration = Duration::from_secs(90);
const BACKOFF_MIN: Duration = Duration::from_millis(250);
const BACKOFF_MAX: Duration = Duration::from_secs(5);

pub(crate) struct StreamConfig {
    pub client: reqwest::Client,
    pub url: Url,
    pub kind: StreamKind,
    pub auth: Arc<FirebaseAuth>,
    pub router: Arc<Router>,
    pub stop: CancellationToken,
}

enum SessionEnd {
    Stopped,
    Reconnect,
    Reauthenticate,
}

pub(crate) async fn run(config: StreamConfig) {
    let mut backoff = BACKOFF_MIN;
    loop {
        if config.stop.is_cancelled() {
            return;
        }
        match session(&config, &mut backoff).await {
            SessionEnd::Stopped => return,
            SessionEnd::Reauthenticate => config.auth.invalidate().await,
            SessionEnd::Reconnect => {}
        }
        tokio::select! {
            _ = config.stop.cancelled() => return,
            _ = tokio::time::sleep(backoff) => {}
        }
        backoff = (backoff * 2).min(BACKOFF_MAX);
    }
}

async fn session(config: &StreamConfig, backoff: &mut Duration) -> SessionEnd {
    let stream = config.kind.name();
    let token = match config.auth.id_token().await {
        Ok(token) => token,
        Err(_) => {
            tracing::warn!(stream, "firebase stream cannot obtain an id token");
            return SessionEnd::Reconnect;
        }
    };
    let mut url = config.url.clone();
    url.query_pairs_mut().append_pair("auth", &token);
    let request = config
        .client
        .get(url)
        .header(ACCEPT, "text/event-stream")
        .send();
    let response = tokio::select! {
        _ = config.stop.cancelled() => return SessionEnd::Stopped,
        response = request => response,
    };
    let response = match response {
        Ok(response) => response,
        Err(err) => {
            let err = err.without_url();
            tracing::warn!(stream, error = %err, "firebase stream request failed");
            return SessionEnd::Reconnect;
        }
    };
    let status = response.status();
    if status == StatusCode::UNAUTHORIZED || status == StatusCode::FORBIDDEN {
        tracing::warn!(stream, %status, "firebase stream rejected the id token");
        return SessionEnd::Reauthenticate;
    }
    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        tracing::warn!(
            stream,
            %status,
            body = %body.chars().take(300).collect::<String>(),
            "firebase stream returned an error"
        );
        return SessionEnd::Reconnect;
    }
    tracing::debug!(stream, "firebase stream connected");
    let events = response.bytes_stream().eventsource();
    tokio::pin!(events);
    loop {
        let next = tokio::select! {
            _ = config.stop.cancelled() => return SessionEnd::Stopped,
            next = tokio::time::timeout(IDLE_TIMEOUT, events.next()) => next,
        };
        match next {
            Err(_) => {
                tracing::warn!(
                    stream,
                    "firebase stream idle for {IDLE_TIMEOUT:?}; reconnecting"
                );
                return SessionEnd::Reconnect;
            }
            Ok(None) => {
                tracing::debug!(stream, "firebase stream ended; reconnecting");
                return SessionEnd::Reconnect;
            }
            Ok(Some(Err(_))) => {
                tracing::warn!(stream, "firebase stream broke; reconnecting");
                return SessionEnd::Reconnect;
            }
            Ok(Some(Ok(event))) => {
                *backoff = BACKOFF_MIN;
                match config
                    .router
                    .handle_event(config.kind, &event.event, &event.data)
                {
                    StreamAction::Continue => {}
                    StreamAction::Reauthenticate => {
                        tracing::info!(stream, "firebase revoked the id token; renewing");
                        return SessionEnd::Reauthenticate;
                    }
                    StreamAction::Reconnect => {
                        tracing::warn!(
                            stream,
                            "firebase cancelled the stream (rules denied the read)"
                        );
                        return SessionEnd::Reconnect;
                    }
                }
            }
        }
    }
}
