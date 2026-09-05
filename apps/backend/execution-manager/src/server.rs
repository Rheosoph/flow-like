use crate::{
    Backend, CommonConfig, Dispatch, Error, EventSink, MAX_INPUT, Mode, Result, config::safe_id,
};
use axum::{
    Router,
    body::{Body, to_bytes},
    extract::{Path, Request, State},
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
    routing::{delete, get, post},
};
use bytes::Bytes;
use futures_util::{FutureExt, StreamExt};
use hyper_util::{
    rt::{TokioIo, TokioTimer},
    service::TowerToHyperService,
};
use serde_json::{Value, json};
use std::{
    convert::Infallible,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tokio::{
    net::TcpListener,
    sync::{Semaphore, mpsc, oneshot},
    task::JoinSet,
};
use tokio_stream::wrappers::ReceiverStream;
use tokio_util::sync::CancellationToken;

#[derive(Clone)]
pub struct ServerState {
    backend: Arc<dyn Backend>,
    config: Arc<CommonConfig>,
    capacity: Arc<Semaphore>,
    draining: Arc<AtomicBool>,
    completed: Arc<AtomicU64>,
    failed: Arc<AtomicU64>,
    rejected: Arc<AtomicU64>,
}

impl ServerState {
    pub fn new(backend: Arc<dyn Backend>, config: Arc<CommonConfig>) -> Self {
        Self {
            backend,
            capacity: Arc::new(Semaphore::new(config.capacity)),
            config,
            draining: Arc::new(AtomicBool::new(false)),
            completed: Arc::new(AtomicU64::new(0)),
            failed: Arc::new(AtomicU64::new(0)),
            rejected: Arc::new(AtomicU64::new(0)),
        }
    }

    fn ready(&self) -> bool {
        !self.draining.load(Ordering::Acquire) && self.backend.ready()
    }

    fn authenticated(&self, headers: &HeaderMap) -> bool {
        let mut values = headers.get_all("x-execution-manager-token").iter();
        let Some(value) = values.next() else {
            return false;
        };
        values.next().is_none()
            && constant_time_eq::constant_time_eq(value.as_bytes(), self.config.token.as_bytes())
    }
}

pub fn router(state: ServerState) -> Router {
    Router::new()
        .route(
            "/health",
            get(|| async { axum::Json(json!({"status":"healthy"})) }),
        )
        .route("/ready", get(readiness))
        .route("/metrics", get(metrics))
        .route("/execute", post(execute))
        .route("/execute/sse", post(execute))
        .route("/execute/stream", post(execute))
        .route("/execute/{run_id}", delete(cancel))
        .with_state(state)
}

async fn readiness(State(state): State<ServerState>) -> Response {
    let ready = state.ready();
    (
        if ready {
            StatusCode::OK
        } else {
            StatusCode::SERVICE_UNAVAILABLE
        },
        axum::Json(json!({"ready":ready})),
    )
        .into_response()
}

async fn metrics(State(state): State<ServerState>) -> Response {
    let body = format!(
        "executor_active_jobs {}\nexecutor_capacity {}\nflow_executions_total{{status=\"completed\"}} {}\nflow_executions_total{{status=\"failed\"}} {}\nflow_executions_total{{status=\"rejected\"}} {}\n{}",
        state
            .config
            .capacity
            .saturating_sub(state.capacity.available_permits()),
        state.config.capacity,
        state.completed.load(Ordering::Relaxed),
        state.failed.load(Ordering::Relaxed),
        state.rejected.load(Ordering::Relaxed),
        state.backend.metrics()
    );
    ([(header::CONTENT_TYPE, "text/plain; version=0.0.4")], body).into_response()
}

fn error_response(status: StatusCode, message: &str, non_admitted: bool) -> Response {
    let mut response = (status, axum::Json(json!({"error":message}))).into_response();
    if non_admitted {
        response
            .headers_mut()
            .insert("x-execution-admitted", "false".parse().unwrap());
    }
    response
}

pub fn validate_payload(raw: &[u8], config: &CommonConfig) -> Result<Dispatch> {
    const FIELDS: &[&str] = &[
        "job_id",
        "run_id",
        "app_id",
        "board_id",
        "board_version",
        "board_etag",
        "node_id",
        "event_json",
        "payload",
        "user_id",
        "credentials",
        "executor_jwt",
        "callback_url",
        "token",
        "oauth_tokens",
        "stream_state",
        "execution_mode",
        "runtime_variables",
        "user_context",
        "profile",
        "wasm_packages",
        "channel",
        "shadow",
        "artifact",
    ];
    if raw.is_empty() || raw.len() > MAX_INPUT {
        return Err(Error::invalid("Dispatch exceeds its input budget"));
    }
    let value: Value =
        serde_json::from_slice(raw).map_err(|_| Error::invalid("Invalid dispatch JSON"))?;
    let object = value
        .as_object()
        .ok_or_else(|| Error::invalid("Expected a dispatch object"))?;
    if object.keys().any(|key| !FIELDS.contains(&key.as_str())) {
        return Err(Error::invalid(
            "Launcher options and unknown dispatch fields are forbidden",
        ));
    }
    let payload: Dispatch = serde_json::from_value(value)
        .map_err(|_| Error::invalid("Invalid DispatchPayload contract"))?;
    for identifier in [
        &payload.job_id,
        &payload.run_id,
        &payload.app_id,
        &payload.board_id,
        &payload.node_id,
    ] {
        if !safe_id(identifier) {
            return Err(Error::invalid("Invalid execution identifier"));
        }
    }
    if payload.user_id.is_empty()
        || payload.user_id.len() > 1024
        || payload.executor_jwt.is_empty()
        || payload.executor_jwt.len() > 32768
        || payload.callback_url != config.callback_url
        || !payload.credentials.is_object()
        || payload.artifact.is_none()
        || payload
            .token
            .as_ref()
            .is_some_and(|token| !token.is_empty())
    {
        return Err(Error::invalid(
            "Dispatch does not match the approved execution capability",
        ));
    }
    // Use the same wire type as the API and runtime. The isolated runtime still
    // verifies the JWT and complete payload binding before loading tenant code.
    if serde_json::to_vec(&json!({"mode":Mode::CallbackQueued,"payload":&payload}))?.len() + 1
        > MAX_INPUT
    {
        return Err(Error::invalid("Wrapped dispatch exceeds its input budget"));
    }
    Ok(payload)
}

pub fn event(kind: &str, payload: Value) -> Value {
    json!({"event_id":uuid::Uuid::new_v4().simple().to_string(), "timestamp":{"secs_since_epoch":SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs(), "nanos_since_epoch":0}, "event_type":kind, "payload":payload})
}

async fn execute(State(state): State<ServerState>, request: Request) -> Response {
    if !state.authenticated(request.headers()) {
        return error_response(
            StatusCode::UNAUTHORIZED,
            "Unauthenticated manager request",
            false,
        );
    }
    let permit = if state.ready() {
        state.capacity.clone().try_acquire_owned().ok()
    } else {
        None
    };
    let Some(permit) = permit else {
        state.rejected.fetch_add(1, Ordering::Relaxed);
        return error_response(
            if state.ready() {
                StatusCode::TOO_MANY_REQUESTS
            } else {
                StatusCode::SERVICE_UNAVAILABLE
            },
            "No execution capacity",
            true,
        );
    };
    let streaming = request.uri().path() != "/execute";
    let sse = request.uri().path().ends_with("/sse");
    let queued = request
        .headers()
        .get("x-execution-queued")
        .is_some_and(|v| v == "true");
    let lengths: Vec<_> = request
        .headers()
        .get_all(header::CONTENT_LENGTH)
        .iter()
        .collect();
    let length = lengths
        .first()
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<usize>().ok());
    if request.headers().contains_key(header::TRANSFER_ENCODING)
        || lengths.len() != 1
        || !length.is_some_and(|n| (1..=MAX_INPUT).contains(&n))
    {
        return error_response(
            StatusCode::BAD_REQUEST,
            "A single bounded Content-Length is required",
            false,
        );
    }
    let body = match tokio::time::timeout(
        Duration::from_secs(10),
        to_bytes(request.into_body(), MAX_INPUT),
    )
    .await
    {
        Ok(Ok(body)) if Some(body.len()) == length => body,
        _ => {
            return error_response(
                StatusCode::BAD_REQUEST,
                "Incomplete or oversized dispatch body",
                false,
            );
        }
    };
    let payload = match validate_payload(&body, &state.config) {
        Ok(payload) => payload,
        Err(_) => {
            return error_response(StatusCode::BAD_REQUEST, "Invalid execution dispatch", false);
        }
    };
    let mode = if streaming {
        Mode::Stream
    } else if queued {
        Mode::CallbackQueued
    } else {
        Mode::Callback
    };
    let (sender, receiver) = mpsc::channel(2);
    let events = if streaming {
        EventSink::new(sender)
    } else {
        drop(sender);
        EventSink::default()
    };
    let (settled, settlement) = oneshot::channel();
    let (admitted, admission) = oneshot::channel();
    // This task owns admission and cleanup even if the client disconnects while
    // a durable claim is still being written. Never cancel an ambiguous claim.
    tokio::spawn(async move {
        let operation = async {
            let reservation = match state.backend.reserve(&payload).await {
                Ok(reservation) => reservation,
                Err(error) => {
                    if matches!(error, Error::NoCapacity) {
                        state.rejected.fetch_add(1, Ordering::Relaxed);
                    } else {
                        state.failed.fetch_add(1, Ordering::Relaxed);
                    }
                    let _ = admitted.send(Err(error));
                    return;
                }
            };
            let _ = admitted.send(Ok(()));
            let result = reservation.execute(payload, mode, events.clone()).await;
            if result.is_ok() {
                state.completed.fetch_add(1, Ordering::Relaxed);
            } else {
                state.failed.fetch_add(1, Ordering::Relaxed);
                events
                    .send(event(
                        "error",
                        json!({"message":"Execution interrupted; inspect run status"}),
                    ))
                    .await;
            }
            let _ = settled.send(result);
        };
        if std::panic::AssertUnwindSafe(operation)
            .catch_unwind()
            .await
            .is_err()
        {
            state.draining.store(true, Ordering::Release);
            state.failed.fetch_add(1, Ordering::Relaxed);
        }
        drop(permit);
    });
    // Reserve and fence the run before committing streaming success headers.
    match admission.await {
        Ok(Ok(())) => {}
        Ok(Err(Error::NoCapacity)) => {
            return error_response(
                StatusCode::TOO_MANY_REQUESTS,
                "No ready execution capacity",
                true,
            );
        }
        _ => {
            return error_response(
                StatusCode::BAD_GATEWAY,
                "Execution admission could not be confirmed; inspect run status",
                false,
            );
        }
    }
    if streaming {
        let stream = ReceiverStream::new(receiver).map(move |value| {
            let encoded = serde_json::to_string(&value).expect("JSON value serialization");
            Ok::<_, Infallible>(Bytes::from(if sse {
                format!("data: {encoded}\n\n")
            } else {
                encoded + "\n"
            }))
        });
        Response::builder()
            .header(
                header::CONTENT_TYPE,
                if sse {
                    "text/event-stream"
                } else {
                    "application/x-ndjson"
                },
            )
            .header(header::CACHE_CONTROL, "no-cache")
            .header("x-accel-buffering", "no")
            .body(Body::from_stream(stream))
            .unwrap()
    } else {
        drop(receiver);
        match settlement.await {
            Ok(Ok(value)) => axum::Json(value).into_response(),
            _ => error_response(
                StatusCode::BAD_GATEWAY,
                "Execution interrupted; inspect run status",
                false,
            ),
        }
    }
}

async fn cancel(
    State(state): State<ServerState>,
    Path(run_id): Path<String>,
    headers: HeaderMap,
) -> Response {
    if !state.authenticated(&headers) {
        return error_response(
            StatusCode::UNAUTHORIZED,
            "Unauthenticated manager request",
            false,
        );
    }
    if !safe_id(&run_id) {
        return error_response(StatusCode::BAD_REQUEST, "Invalid run_id", false);
    }
    tokio::spawn(async move {
        match state.backend.cancel(&run_id).await {
            Ok(value) => axum::Json(value).into_response(),
            Err(_) => {
                state.draining.store(true, Ordering::Release);
                error_response(
                    StatusCode::SERVICE_UNAVAILABLE,
                    "Execution termination could not be confirmed",
                    false,
                )
            }
        }
    })
    .await
    .unwrap_or_else(|_| {
        error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "Execution termination could not be confirmed",
            false,
        )
    })
}

pub async fn serve(
    listener: TcpListener,
    state: ServerState,
    stop: CancellationToken,
) -> Result<()> {
    serve_with_header_timeout(listener, state, stop, Duration::from_secs(10)).await
}

async fn serve_with_header_timeout(
    listener: TcpListener,
    state: ServerState,
    stop: CancellationToken,
    header_timeout: Duration,
) -> Result<()> {
    let app = router(state.clone());
    let connections = Arc::new(Semaphore::new(state.config.capacity + 32));
    let mut tasks = JoinSet::new();
    let mut supervision = tokio::time::interval(Duration::from_millis(250));
    loop {
        tokio::select! {
            _ = stop.cancelled() => break,
            _ = supervision.tick() => {
                if state.draining.load(Ordering::Acquire) || !state.backend.ready() { break; }
            },
            Some(_) = tasks.join_next(), if !tasks.is_empty() => {},
            accepted = listener.accept() => {
                let (mut socket, _) = accepted?;
                let Ok(permit) = connections.clone().try_acquire_owned() else {
                    use tokio::io::AsyncWriteExt;
                    let _ = tokio::time::timeout(Duration::from_millis(50), socket.write_all(b"HTTP/1.1 503 Service Unavailable\r\nContent-Length: 0\r\nX-Execution-Admitted: false\r\nConnection: close\r\n\r\n")).await;
                    continue;
                };
                let service = TowerToHyperService::new(app.clone());
                tasks.spawn(async move {
                    let _permit = permit;
                    let mut transport = tokio_io_timeout::TimeoutStream::new(socket);
                    transport.set_write_timeout(Some(Duration::from_secs(30)));
                    let _ = hyper::server::conn::http1::Builder::new().timer(TokioTimer::new()).half_close(true)
                        .header_read_timeout(header_timeout).max_headers(64)
                        .serve_connection(TokioIo::new(Box::pin(transport)), service).await;
                });
            }
        }
    }
    state.draining.store(true, Ordering::Release);
    state.capacity.close();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(state.config.budget() + 60);
    let drained = tokio::time::timeout_at(deadline, async {
        state.backend.shutdown().await?;
        while state.capacity.available_permits() < state.config.capacity {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        Ok::<_, Error>(())
    })
    .await;
    // Stop idle HTTP connections after accepted runs have finished cleanup.
    tasks.abort_all();
    while tasks.join_next().await.is_some() {}
    drained.map_err(|_| Error::internal("Execution drain deadline exceeded"))?
}

pub async fn shutdown_signal() {
    #[cfg(unix)]
    {
        let mut terminate =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
                .expect("SIGTERM handler");
        tokio::select! { _ = terminate.recv() => {}, _ = tokio::signal::ctrl_c() => {} }
    }
    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
    }
}

#[cfg(test)]
mod tests;
