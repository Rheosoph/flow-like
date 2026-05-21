pub mod api;
#[cfg(feature = "execute")]
pub(crate) mod auth;
pub mod camera;
pub(crate) mod http_runtime;
pub mod mcp;
pub mod message_handler;
pub mod mqtt;
pub mod rest;
pub mod scrape;
pub mod tcp;
#[cfg(all(test, feature = "execute"))]
pub(crate) mod test_support;
pub mod tls;
pub mod udp;
pub mod websocket;

#[cfg(feature = "execute")]
pub(crate) async fn wait_for_cancel(
    token: Option<flow_like_types::tokio_util::sync::CancellationToken>,
) {
    if let Some(token) = token {
        token.cancelled().await;
    } else {
        std::future::pending::<()>().await;
    }
}

#[cfg(all(test, feature = "execute"))]
mod tests {
    use super::*;
    use flow_like_types::tokio_util::sync::CancellationToken;
    use std::time::Duration;

    #[tokio::test]
    async fn wait_for_cancel_returns_after_token_cancelled() {
        let token = CancellationToken::new();
        token.cancel();

        tokio::time::timeout(Duration::from_secs(1), wait_for_cancel(Some(token)))
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn wait_for_cancel_without_token_stays_pending() {
        let result = tokio::time::timeout(Duration::from_millis(20), wait_for_cancel(None)).await;

        assert!(result.is_err());
    }
}
