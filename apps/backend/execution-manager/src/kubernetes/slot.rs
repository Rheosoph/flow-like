//! One HTTP assignment for one preinitialized executor process.
//!
//! This adapter shares the tenant sandbox. The external manager and gateway
//! enforce cancellation, lifetime and egress even if the adapter is compromised.
use super::read_line;
use crate::{Dispatch, Error, MAX_EVENT, MAX_INPUT, MAX_OUTPUT, Mode, Result, config::positive};
use axum::{
    Router,
    body::{Body, Bytes, to_bytes},
    extract::{Request, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{get, post},
};
use hyper_util::{
    rt::{TokioIo, TokioTimer},
    service::TowerToHyperService,
};
use serde::Deserialize;
use serde_json::json;
use std::{
    convert::Infallible,
    env,
    process::Stdio,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use tokio::{
    io::{AsyncWriteExt, BufReader},
    net::{TcpListener, TcpStream},
    process::{Child, Command},
    sync::{Mutex, Semaphore, mpsc},
};
use tokio_stream::wrappers::ReceiverStream;
use tokio_util::sync::CancellationToken;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Envelope {
    mode: Mode,
    payload: Dispatch,
}

struct Slot {
    child: Mutex<Option<Child>>,
    token: String,
    used: AtomicBool,
    finished: CancellationToken,
    maximum_duration: u64,
}

impl Slot {
    fn claim(&self) -> bool {
        self.used
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
    }
}

async fn ready(State(slot): State<Arc<Slot>>) -> StatusCode {
    if slot.used.load(Ordering::Acquire) {
        return StatusCode::SERVICE_UNAVAILABLE;
    }
    let mut child = slot.child.lock().await;
    if child
        .as_mut()
        .is_some_and(|child| matches!(child.try_wait(), Ok(None)))
    {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    }
}

async fn execute(State(slot): State<Arc<Slot>>, request: Request) -> Response {
    let headers = request.headers();
    let authenticated = headers.get("X-Slot-Token").is_some_and(|value| {
        constant_time_eq::constant_time_eq(value.as_bytes(), slot.token.as_bytes())
    });
    if !authenticated {
        return StatusCode::FORBIDDEN.into_response();
    }
    if headers.contains_key("transfer-encoding")
        || headers.get_all("content-length").iter().count() != 1
    {
        return StatusCode::BAD_REQUEST.into_response();
    }
    let Some(size) = headers
        .get("content-length")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<usize>().ok())
        .filter(|size| *size > 0 && *size < MAX_INPUT)
    else {
        return StatusCode::BAD_REQUEST.into_response();
    };
    let raw = match tokio::time::timeout(
        Duration::from_secs(10),
        to_bytes(request.into_body(), MAX_INPUT - 1),
    )
    .await
    {
        Ok(Ok(raw)) if raw.len() == size => raw,
        _ => return StatusCode::BAD_REQUEST.into_response(),
    };
    let envelope: Envelope = match serde_json::from_slice(&raw) {
        Ok(value) => value,
        Err(_) => return StatusCode::BAD_REQUEST.into_response(),
    };
    if !slot.claim() {
        return StatusCode::CONFLICT.into_response();
    }
    let Some(child) = slot.child.lock().await.take() else {
        return StatusCode::CONFLICT.into_response();
    };
    let (sender, receiver) = mpsc::channel::<std::result::Result<Bytes, Infallible>>(32);
    let state = slot.clone();
    tokio::spawn(async move {
        let _ = run_child(child, envelope, sender, state.maximum_duration).await;
        state.finished.cancel();
    });
    (
        [
            ("Content-Type", "application/x-ndjson"),
            ("Connection", "close"),
        ],
        Body::from_stream(ReceiverStream::new(receiver)),
    )
        .into_response()
}

async fn send(
    sender: &mut Option<mpsc::Sender<std::result::Result<Bytes, Infallible>>>,
    bytes: Bytes,
) {
    if let Some(channel) = sender.as_ref()
        && !matches!(
            tokio::time::timeout(Duration::from_secs(1), channel.send(Ok(bytes))).await,
            Ok(Ok(()))
        )
    {
        *sender = None;
    }
}

async fn run_child(
    mut child: Child,
    envelope: Envelope,
    sender: mpsc::Sender<std::result::Result<Bytes, Infallible>>,
    maximum_duration: u64,
) -> Result<()> {
    let result = tokio::time::timeout(Duration::from_secs(maximum_duration), async {
        let mut input = child
            .stdin
            .take()
            .ok_or_else(|| Error::internal("Runtime input unavailable"))?;
        let mut body =
            serde_json::to_vec(&json!({"mode":envelope.mode,"payload":envelope.payload}))?;
        body.push(b'\n');
        input.write_all(&body).await?;
        input.shutdown().await?;
        drop(input);
        let output = child
            .stdout
            .take()
            .ok_or_else(|| Error::internal("Runtime output unavailable"))?;
        let mut reader = BufReader::new(output);
        let mut sender = Some(sender);
        let mut total = 0usize;
        loop {
            let line = read_line(&mut reader, MAX_EVENT).await?;
            if line.is_empty() {
                break;
            }
            total += line.len();
            if total > MAX_OUTPUT {
                return Err(Error::internal("Execution output exceeds limit"));
            }
            send(&mut sender, line.into()).await;
        }
        let status = tokio::time::timeout(Duration::from_secs(10), child.wait()).await??;
        send(
            &mut sender,
            format!(
                "{}\n",
                json!({"slot_exit_code":status.code().unwrap_or(-1)})
            )
            .into(),
        )
        .await;
        Ok(())
    })
    .await
    .unwrap_or_else(|_| Err(Error::internal("Execution exceeded its time budget")));
    if child.try_wait()?.is_none() {
        let _ = child.kill().await;
    }
    let _ = tokio::time::timeout(Duration::from_secs(10), child.wait()).await;
    result
}

async fn probes() -> Result<()> {
    let endpoint = |name: &str| -> Result<(String, u16)> {
        serde_json::from_str(
            &env::var(name).map_err(|_| Error::invalid(format!("{name} is required")))?,
        )
        .map_err(|_| Error::invalid(format!("Invalid {name}")))
    };
    let allowed = endpoint("SLOT_GATEWAY_ENDPOINT")?;
    tokio::time::timeout(
        Duration::from_secs(3),
        TcpStream::connect((allowed.0.as_str(), allowed.1)),
    )
    .await
    .map_err(|_| Error::internal("Own execution gateway is unreachable"))?
    .map_err(|_| Error::internal("Own execution gateway is unreachable"))?;
    let mut denied = vec![
        endpoint("SLOT_DENIED_ENDPOINT")?,
        ("169.254.169.254".into(), 80),
    ];
    if let Ok(node) = env::var("SLOT_NODE_IP")
        && !node.is_empty()
    {
        denied.push((node, 10250));
    }
    for result in
        futures_util::future::join_all(denied.into_iter().map(|(host, port)| async move {
            if matches!(
                tokio::time::timeout(
                    Duration::from_millis(500),
                    TcpStream::connect((host.as_str(), port))
                )
                .await,
                Ok(Ok(_))
            ) {
                Err(Error::internal(
                    "Network policy did not deny a protected endpoint",
                ))
            } else {
                Ok(())
            }
        }))
        .await
    {
        result?;
    }
    Ok(())
}

pub async fn main() -> Result<()> {
    let token = env::var("SLOT_TOKEN").map_err(|_| Error::invalid("SLOT_TOKEN is required"))?;
    if token.len() < 32 {
        return Err(Error::invalid(
            "SLOT_TOKEN must contain at least 32 characters",
        ));
    }
    probes().await?;
    let mut command = Command::new("/app/executor");
    command
        .args(["--once", "warm"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    // Do not mutate process-wide environment after starting Tokio threads.
    // These capabilities belong to this transport adapter alone.
    for key in [
        "SLOT_TOKEN",
        "SLOT_DENIED_ENDPOINT",
        "SLOT_GATEWAY_ENDPOINT",
        "SLOT_NODE_IP",
        "POD_IP",
    ] {
        command.env_remove(key);
    }
    let mut child = command.spawn()?;
    let mut output = BufReader::new(
        child
            .stdout
            .take()
            .ok_or_else(|| Error::internal("Runtime output unavailable"))?,
    );
    let initialized =
        tokio::time::timeout(Duration::from_secs(180), read_line(&mut output, 64)).await;
    if !matches!(&initialized,Ok(Ok(value)) if value==b"ready\n") {
        let _ = child.kill().await;
        return Err(Error::internal("Runtime initialization failed"));
    }
    // The executor blocks for one assignment after the readiness line, so this
    // buffer must be empty before transferring stdout to the execution task.
    if !output.buffer().is_empty() {
        let _ = child.kill().await;
        return Err(Error::internal("Runtime emitted output before assignment"));
    }
    child.stdout = Some(output.into_inner());
    let idle = positive("SLOT_MAX_AGE_SECONDS", 600, 3600)?;
    let slot = Arc::new(Slot {
        child: Mutex::new(Some(child)),
        token,
        used: AtomicBool::new(false),
        finished: CancellationToken::new(),
        maximum_duration: positive("EXECUTION_TIMEOUT_SECONDS", 3600, 86400)? + 300,
    });
    let address = if env::var("POD_IP").is_ok_and(|ip| ip.contains(':')) {
        "[::]:8080"
    } else {
        "0.0.0.0:8080"
    };
    let listener = TcpListener::bind(address).await?;
    let app = Router::new()
        .route("/ready", get(ready))
        .route("/execute", post(execute))
        .with_state(slot.clone());
    let idle_slot = slot.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_secs(idle)).await;
        if !idle_slot.used.load(Ordering::Acquire) {
            idle_slot.finished.cancel();
        }
    });
    let capacity = Arc::new(Semaphore::new(8));
    let mut tasks = tokio::task::JoinSet::new();
    let stopped = shutdown_signal();
    tokio::pin!(stopped);
    loop {
        tokio::select! {
            _=slot.finished.cancelled()=>break,
            _=&mut stopped=>break,
            connection=listener.accept()=>{
                let (socket,_)=connection?;
                let Ok(permit)=capacity.clone().try_acquire_owned() else {drop(socket);continue;};
                let service=TowerToHyperService::new(app.clone());
                tasks.spawn(async move {
                    let _permit=permit;
                    let mut builder=hyper::server::conn::http1::Builder::new();
                    builder.timer(TokioTimer::new()).header_read_timeout(Duration::from_secs(10)).keep_alive(false).max_buf_size(32*1024);
                    let _=builder.serve_connection(TokioIo::new(socket),service).await;
                });
            }
            Some(_)=tasks.join_next()=>{}
        }
    }
    // Give the final NDJSON record time to reach the manager, then close idle
    // connections. The manager independently confirms Pod termination.
    let _ = tokio::time::timeout(Duration::from_secs(2), async {
        while tasks.join_next().await.is_some() {}
    })
    .await;
    tasks.abort_all();
    if let Some(mut child) = slot.child.lock().await.take() {
        let _ = child.kill().await;
        let _ = child.wait().await;
    }
    Ok(())
}

async fn shutdown_signal() {
    #[cfg(unix)]
    {
        if let Ok(mut terminate) =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        {
            tokio::select! {_=terminate.recv()=>{},_=tokio::signal::ctrl_c()=>{}}
            return;
        }
    }
    let _ = tokio::signal::ctrl_c().await;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn single_use_claim_is_atomic_under_contention() {
        let slot = Arc::new(Slot {
            child: Mutex::new(None),
            token: "t".repeat(64),
            used: AtomicBool::new(false),
            finished: CancellationToken::new(),
            maximum_duration: 10,
        });
        let mut tasks = tokio::task::JoinSet::new();
        for _ in 0..32 {
            let slot = slot.clone();
            tasks.spawn(async move { slot.claim() });
        }
        let mut winners = 0;
        while let Some(value) = tasks.join_next().await {
            winners += usize::from(value.unwrap());
        }
        assert_eq!(winners, 1);
    }

    #[tokio::test]
    async fn framing_is_bounded_even_without_newlines() {
        let mut bytes = BufReader::new(&b"123456789"[..]);
        assert!(read_line(&mut bytes, 8).await.is_err());
        let mut bytes = BufReader::new(&b"first\nsecond\n"[..]);
        assert_eq!(read_line(&mut bytes, 8).await.unwrap(), b"first\n");
        assert_eq!(read_line(&mut bytes, 8).await.unwrap(), b"second\n");
    }

    fn test_payload() -> serde_json::Value {
        json!({"job_id":"job","run_id":"run","app_id":"app","board_id":"board","node_id":"node","user_id":"user","credentials":{},"executor_jwt":"signed-capability","callback_url":"http://api:8080"})
    }

    fn test_slot() -> Arc<Slot> {
        let child=Command::new("/bin/sh").args(["-c","cat >/dev/null; printf '%s\\n' '{\"event_type\":\"completed\",\"payload\":{\"answer\":42}}'"])
            .stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null()).kill_on_drop(true).spawn().unwrap();
        Arc::new(Slot {
            child: Mutex::new(Some(child)),
            token: "t".repeat(64),
            used: AtomicBool::new(false),
            finished: CancellationToken::new(),
            maximum_duration: 10,
        })
    }

    fn request(token: &str) -> Request {
        let body = json!({"mode":"callback-queued","payload":test_payload()}).to_string();
        Request::builder()
            .method("POST")
            .uri("/execute")
            .header("X-Slot-Token", token)
            .header("Content-Length", body.len())
            .body(Body::from(body))
            .unwrap()
    }

    #[tokio::test]
    async fn authenticated_transport_delivers_one_child_result_and_exit_acknowledgement() {
        let slot = test_slot();
        let denied = execute(State(slot.clone()), request("wrong")).await;
        assert_eq!(denied.status(), StatusCode::FORBIDDEN);
        assert!(!slot.used.load(Ordering::Acquire));
        let response = execute(State(slot.clone()), request(&slot.token)).await;
        assert_eq!(response.status(), StatusCode::OK);
        let repeated = execute(State(slot.clone()), request(&slot.token)).await;
        assert_eq!(repeated.status(), StatusCode::CONFLICT);
        let body = to_bytes(response.into_body(), MAX_OUTPUT).await.unwrap();
        let lines = std::str::from_utf8(&body)
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap())
            .collect::<Vec<_>>();
        assert_eq!(lines[0]["payload"]["answer"], 42);
        assert_eq!(lines[1]["slot_exit_code"], 0);
        tokio::time::timeout(Duration::from_secs(2), slot.finished.cancelled())
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn disconnected_manager_does_not_leave_the_child_running() {
        let slot = test_slot();
        let response = execute(State(slot.clone()), request(&slot.token)).await;
        drop(response);
        tokio::time::timeout(Duration::from_secs(2), slot.finished.cancelled())
            .await
            .unwrap();
        assert!(slot.child.lock().await.is_none());
    }
}
