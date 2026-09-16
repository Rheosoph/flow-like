//! Load balancer and container health, served outside the API middleware so
//! probes create no traces, request metrics or database load. Neither check
//! consults a dependency: failing every task during a database outage would
//! only make ECS replace healthy tasks in a loop.

use axum::{Router, extract::State, http::StatusCode, routing::get};
use std::io::{Read, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpStream};
use std::process::ExitCode;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

pub const LIVE_PATH: &str = "/health/live";
pub const READY_PATH: &str = "/health/ready";
const PROBE_TIMEOUT: Duration = Duration::from_secs(2);

/// Set once the process stops accepting connections.
#[derive(Clone, Debug, Default)]
pub struct Draining(Arc<AtomicBool>);

impl Draining {
    pub fn start(&self) {
        self.0.store(true, Ordering::Relaxed);
    }

    fn active(&self) -> bool {
        self.0.load(Ordering::Relaxed)
    }
}

pub fn routes(draining: Draining) -> Router {
    Router::new()
        .route(LIVE_PATH, get(|| async { (StatusCode::OK, "ok") }))
        .route(READY_PATH, get(ready))
        .with_state(draining)
}

async fn ready(State(draining): State<Draining>) -> (StatusCode, &'static str) {
    if draining.active() {
        (StatusCode::SERVICE_UNAVAILABLE, "draining")
    } else {
        (StatusCode::OK, "ready")
    }
}

/// `api healthcheck`: the container health command, answered without a shell.
pub fn probe(port: u16) -> ExitCode {
    match live(port) {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => {
            eprintln!("{LIVE_PATH} on port {port} did not answer 200");
            ExitCode::FAILURE
        }
        Err(error) => {
            eprintln!("{LIVE_PATH} on port {port} is unreachable: {error}");
            ExitCode::FAILURE
        }
    }
}

fn live(port: u16) -> std::io::Result<bool> {
    let address = SocketAddr::from((Ipv4Addr::LOCALHOST, port));
    let mut stream = TcpStream::connect_timeout(&address, PROBE_TIMEOUT)?;
    stream.set_read_timeout(Some(PROBE_TIMEOUT))?;
    stream.set_write_timeout(Some(PROBE_TIMEOUT))?;
    write!(
        stream,
        "GET {LIVE_PATH} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n"
    )?;
    let mut status = [0u8; 12];
    stream.read_exact(&mut status)?;
    Ok(&status == b"HTTP/1.1 200")
}

#[cfg(test)]
mod tests {
    use super::*;
    use tower::ServiceExt;

    async fn status(router: &Router, path: &str) -> StatusCode {
        router
            .clone()
            .oneshot(
                axum::http::Request::get(path)
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap()
            .status()
    }

    #[tokio::test]
    async fn readiness_fails_while_liveness_holds_during_drain() {
        let draining = Draining::default();
        let router = routes(draining.clone());
        assert_eq!(status(&router, READY_PATH).await, StatusCode::OK);
        draining.start();
        assert_eq!(
            status(&router, READY_PATH).await,
            StatusCode::SERVICE_UNAVAILABLE
        );
        assert_eq!(status(&router, LIVE_PATH).await, StatusCode::OK);
    }

    #[tokio::test]
    async fn probe_reads_the_status_line_of_a_live_server() {
        let listener = tokio::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
            .await
            .unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(axum::serve(listener, routes(Draining::default())).into_future());
        let result = tokio::task::spawn_blocking(move || live(port))
            .await
            .unwrap();
        assert!(result.unwrap());
    }
}
