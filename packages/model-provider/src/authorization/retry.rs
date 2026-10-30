use std::{error::Error, future::Future, io::ErrorKind, time::Duration};

use bytes::Bytes;
use http::Request;
use rig::http_client;

const MAX_ATTEMPTS: u32 = 5;

/// Retry only while opening the HTTP response. Once its body or stream is handed to the
/// caller, replaying the request could duplicate output or tool calls.
pub(super) async fn send<T, F, Fut>(
    request: &Request<Bytes>,
    mut operation: F,
) -> http_client::Result<T>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = http_client::Result<T>>,
{
    for attempt in 1..=MAX_ATTEMPTS {
        let error = match operation().await {
            Ok(response) => return Ok(response),
            Err(error) => error,
        };
        let Some(reason) = retry_reason(&error) else {
            return Err(error);
        };
        let status = match &error {
            http_client::Error::InvalidStatusCode(status)
            | http_client::Error::InvalidStatusCodeWithMessage(status, _) => Some(status.as_u16()),
            _ => None,
        };
        // Log the host and failure category only. URLs, headers and provider error bodies
        // can contain credentials or prompt content.
        if attempt == MAX_ATTEMPTS {
            tracing::warn!(
                host = request.uri().host().unwrap_or_default(),
                attempts = attempt,
                ?status,
                reason,
                "Model request still unavailable; retry limit reached"
            );
            return Err(error);
        }
        let delay = Duration::from_secs(1 << (attempt - 1))
            + Duration::from_millis(rand::random_range(0..=250));
        tracing::warn!(
            host = request.uri().host().unwrap_or_default(),
            attempt,
            max_attempts = MAX_ATTEMPTS,
            retry_in_ms = delay.as_millis() as u64,
            ?status,
            reason,
            "Model request temporarily unavailable; retrying"
        );
        tokio::time::sleep(delay).await;
    }
    unreachable!("the last attempt returns its response or error")
}

fn retry_reason(error: &http_client::Error) -> Option<&'static str> {
    match error {
        http_client::Error::InvalidStatusCode(status)
        | http_client::Error::InvalidStatusCodeWithMessage(status, _)
            if matches!(status.as_u16(), 408 | 429 | 500 | 502 | 503 | 504) =>
        {
            Some("temporary HTTP failure")
        }
        http_client::Error::Instance(error) => transport_failure(error.as_ref()),
        _ => None,
    }
}

fn transport_failure(error: &(dyn Error + 'static)) -> Option<&'static str> {
    let mut source = Some(error);
    let mut reason = None;
    while let Some(error) = source {
        if error.is::<crate::egress::EgressRefused>()
            || error.is::<flow_like_types_contracts::authorization::AuthorizationError>()
        {
            return None;
        }
        if let Some(error) = error.downcast_ref::<reqwest_rig::Error>() {
            if error.is_builder() || error.is_redirect() || error.is_decode() {
                return None;
            }
            if error.is_timeout() {
                reason = Some("timeout");
            } else if error.is_connect() || error.is_request() {
                // A peer closing before its response headers is a request error whose
                // hyper source may have no io::Error (IncompleteMessage).
                reason = Some("connection failure");
            }
        }
        if let Some(error) = error.downcast_ref::<std::io::Error>() {
            match error.kind() {
                ErrorKind::ConnectionRefused
                | ErrorKind::ConnectionReset
                | ErrorKind::ConnectionAborted
                | ErrorKind::NotConnected
                | ErrorKind::BrokenPipe
                | ErrorKind::TimedOut
                | ErrorKind::Interrupted
                | ErrorKind::UnexpectedEof => reason = Some("connection failure"),
                ErrorKind::InvalidInput | ErrorKind::InvalidData | ErrorKind::PermissionDenied => {
                    return None;
                }
                _ => {}
            }
        }
        source = error.source();
    }
    reason
}
