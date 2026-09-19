//! HTTP serving tuned for an ALB or Service Connect in front of the task.
//!
//! ECS deregisters a task from its target groups before it sends SIGTERM, so
//! the listener closes on the signal and in-flight requests drain for at most
//! the configured shutdown timeout. Long-lived streams are cut at that bound.

use crate::config::Config;
use crate::hardening::shutdown;
use crate::health::{self, Draining};
use axum::{
    Router,
    extract::{ConnectInfo, Request},
    http::{HeaderValue, StatusCode, header::RETRY_AFTER},
    middleware::Next,
    response::{IntoResponse, Response},
};
use hyper::body::Incoming;
use hyper_util::rt::{TokioExecutor, TokioIo, TokioTimer};
use hyper_util::server::conn::auto;
use hyper_util::server::graceful::GracefulShutdown;
use std::net::Ipv4Addr;
use std::sync::Arc;
use std::time::Duration;
use tokio::net::TcpListener;
use tokio::sync::Semaphore;
use tower::ServiceExt;

/// Back off when accept fails (typically EMFILE) instead of spinning on it.
const ACCEPT_BACKOFF: Duration = Duration::from_millis(50);

pub async fn serve(config: &Config, api: Router) -> Result<(), String> {
    let draining = Draining::default();
    let app =
        health::routes(draining.clone()).merge(shed_load(api, config.max_concurrent_requests));
    let listener = TcpListener::bind((Ipv4Addr::UNSPECIFIED, config.port))
        .await
        .map_err(|error| format!("failed to bind 0.0.0.0:{}: {error}", config.port))?;
    let http = http_builder(config);
    let graceful = GracefulShutdown::new();
    tracing::info!(port = config.port, "API listening");

    let signal = shutdown();
    tokio::pin!(signal);
    loop {
        let (stream, remote) = tokio::select! {
            accepted = listener.accept() => match accepted {
                Ok(accepted) => accepted,
                Err(error) => {
                    tracing::warn!(%error, "failed to accept a connection");
                    tokio::time::sleep(ACCEPT_BACKOFF).await;
                    continue;
                }
            },
            () = &mut signal => break,
        };
        // Responses are streamed in small frames; Nagle would hold them back.
        let _ = stream.set_nodelay(true);
        let app = app.clone();
        let service = hyper::service::service_fn(move |mut request: Request<Incoming>| {
            request.extensions_mut().insert(ConnectInfo(remote));
            app.clone().oneshot(request)
        });
        let connection = graceful.watch(
            http.serve_connection_with_upgrades(TokioIo::new(stream), service)
                .into_owned(),
        );
        tokio::spawn(async move {
            if let Err(error) = connection.await {
                tracing::debug!(%error, "connection closed with an error");
            }
        });
    }

    draining.start();
    drop(listener);
    tracing::info!(
        timeout_secs = config.shutdown_timeout.as_secs(),
        "shutdown signal received; draining connections"
    );
    tokio::select! {
        () = graceful.shutdown() => {}
        () = tokio::time::sleep(config.shutdown_timeout) => {
            tracing::warn!("connections were still open at the shutdown timeout; closing them");
        }
    }
    Ok(())
}

fn http_builder(config: &Config) -> auto::Builder<TokioExecutor> {
    let mut builder = auto::Builder::new(TokioExecutor::new());
    // hyper arms this timer while an idle keep-alive connection waits for its
    // next request, so it is the connection idle timeout as well.
    builder
        .http1()
        .timer(TokioTimer::new())
        .keep_alive(true)
        .header_read_timeout(config.idle_timeout);
    builder
        .http2()
        .timer(TokioTimer::new())
        .adaptive_window(true);
    builder
}

/// Past the limit a request is answered 503 at once rather than queued, so an
/// overloaded task keeps its latency while the service scales out. A permit is
/// held until the response head is ready; streamed bodies do not keep it.
fn shed_load(api: Router, limit: Option<usize>) -> Router {
    let Some(limit) = limit else {
        return api;
    };
    let permits = Arc::new(Semaphore::new(limit));
    api.layer(axum::middleware::from_fn(
        move |request: Request, next: Next| {
            let permit = permits.clone().try_acquire_owned();
            async move {
                match permit {
                    Ok(_permit) => next.run(request).await,
                    Err(_) => overloaded(),
                }
            }
        },
    ))
}

fn overloaded() -> Response {
    let mut response = (StatusCode::SERVICE_UNAVAILABLE, "overloaded").into_response();
    response
        .headers_mut()
        .insert(RETRY_AFTER, HeaderValue::from_static("1"));
    response
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::routing::get;
    use tokio::sync::oneshot;

    fn request(path: &str) -> Request {
        axum::http::Request::get(path)
            .body(axum::body::Body::empty())
            .unwrap()
    }

    #[tokio::test]
    async fn sheds_requests_beyond_the_limit_and_recovers() {
        let (release, released) = oneshot::channel::<()>();
        let released = Arc::new(tokio::sync::Mutex::new(Some(released)));
        let api = Router::new().route(
            "/slow",
            get(move || {
                let released = released.clone();
                async move {
                    if let Some(released) = released.lock().await.take() {
                        let _ = released.await;
                    }
                    "done"
                }
            }),
        );
        let app = shed_load(api, Some(1));
        let first = tokio::spawn(app.clone().oneshot(request("/slow")));
        tokio::task::yield_now().await;
        tokio::time::sleep(Duration::from_millis(20)).await;

        let shed = app.clone().oneshot(request("/slow")).await.unwrap();
        assert_eq!(shed.status(), StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(shed.headers()[RETRY_AFTER], "1");

        release.send(()).unwrap();
        assert_eq!(first.await.unwrap().unwrap().status(), StatusCode::OK);
        assert_eq!(
            app.oneshot(request("/slow")).await.unwrap().status(),
            StatusCode::OK
        );
    }

    #[tokio::test]
    async fn no_limit_leaves_the_router_untouched() {
        let app = shed_load(Router::new().route("/", get(|| async { "ok" })), None);
        assert_eq!(
            app.oneshot(request("/")).await.unwrap().status(),
            StatusCode::OK
        );
    }
}
