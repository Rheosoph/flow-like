//! Bounded asynchronous Docker Engine transport over one local Unix socket.
//! Mutation failures are never retried because their admission is ambiguous.
use std::{path::PathBuf, sync::Arc, time::Duration};

use bytes::{Bytes, BytesMut};
use http_body_util::{BodyExt, Full};
use hyper::{Request, client::conn::http1::SendRequest};
use hyper_util::rt::TokioIo;
use serde_json::{Value, json};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{
        UnixStream,
        unix::{OwnedReadHalf, OwnedWriteHalf},
    },
    sync::{Mutex, Semaphore},
    time::{Instant, timeout},
};

const MAX_RESPONSE: usize = 32 * 1024 * 1024;
const MAX_FRAME: usize = 16 * 1024 * 1024;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Debug, thiserror::Error)]
pub enum EngineError {
    #[error("Execution engine resource is missing")]
    Missing,
    #[error("Execution container removal is in progress")]
    Removing,
    #[error("Execution engine rejected {0} ({1})")]
    Rejected(String, u16),
    #[error("Execution engine transport failed")]
    Transport,
    #[error("Execution engine response exceeded its budget")]
    Budget,
    #[error("Execution engine operation timed out")]
    Timeout,
    #[error("Execution output protocol is invalid")]
    Protocol,
}
pub type Result<T> = std::result::Result<T, EngineError>;

pub struct Engine {
    path: PathBuf,
    idle: Mutex<Vec<SendRequest<Full<Bytes>>>>,
    permits: Semaphore,
}

impl Engine {
    pub fn new(path: PathBuf) -> Arc<Self> {
        Arc::new(Self {
            path,
            idle: Mutex::new(Vec::new()),
            permits: Semaphore::new(32),
        })
    }

    pub async fn request(&self, method: &str, path: &str, body: Option<Value>) -> Result<Value> {
        timeout(REQUEST_TIMEOUT, self.request_inner(method, path, body))
            .await
            .map_err(|_| EngineError::Timeout)?
    }

    async fn request_inner(&self, method: &str, path: &str, body: Option<Value>) -> Result<Value> {
        let _permit = self
            .permits
            .acquire()
            .await
            .map_err(|_| EngineError::Transport)?;
        let existing = {
            let mut idle = self.idle.lock().await;
            loop {
                match idle.pop() {
                    Some(connection) if !connection.is_closed() => break Some(connection),
                    Some(_) => continue,
                    None => break None,
                }
            }
        };
        let mut connection = match existing {
            Some(connection) => connection,
            None => {
                let socket = UnixStream::connect(&self.path)
                    .await
                    .map_err(|_| EngineError::Transport)?;
                let (sender, connection) =
                    hyper::client::conn::http1::handshake(TokioIo::new(socket))
                        .await
                        .map_err(|_| EngineError::Transport)?;
                tokio::spawn(async move {
                    let _ = connection.await;
                });
                sender
            }
        };
        let bytes = body
            .map(|value| serde_json::to_vec(&value))
            .transpose()
            .map_err(|_| EngineError::Protocol)?
            .unwrap_or_default();
        let request = Request::builder()
            .method(method)
            .uri(format!("/v1.47{path}"))
            .header("Host", "localhost")
            .header("Content-Type", "application/json")
            .body(Full::new(Bytes::from(bytes)))
            .map_err(|_| EngineError::Protocol)?;
        let response = connection
            .send_request(request)
            .await
            .map_err(|_| EngineError::Transport)?;
        let status = response.status().as_u16();
        let mut body = response.into_body();
        let mut bytes = Vec::new();
        while let Some(frame) = body.frame().await {
            if let Ok(data) = frame.map_err(|_| EngineError::Transport)?.into_data() {
                if bytes.len() + data.len() > MAX_RESPONSE {
                    return Err(EngineError::Budget);
                }
                bytes.extend_from_slice(&data);
            }
        }
        if !connection.is_closed() {
            self.idle.lock().await.push(connection);
        }
        match status {
            404 => Err(EngineError::Missing),
            409 if method == "DELETE" && path.starts_with("/containers/") => {
                Err(EngineError::Removing)
            }
            200..=299 if bytes.is_empty() => Ok(Value::Null),
            200..=299 => serde_json::from_slice(&bytes).map_err(|_| EngineError::Protocol),
            _ => Err(EngineError::Rejected(method.to_owned(), status)),
        }
    }

    pub async fn create(&self, name: &str, spec: Value) -> Result<()> {
        self.request(
            "POST",
            &format!("/containers/create?name={}", encode(name)),
            Some(spec),
        )
        .await?;
        Ok(())
    }

    pub async fn volume(&self, name: &str, labels: Value) -> Result<()> {
        self.request(
            "POST",
            "/volumes/create",
            Some(json!({"Name": name, "Labels": labels})),
        )
        .await?;
        Ok(())
    }

    pub async fn remove_volume(&self, name: &str) -> Result<()> {
        match self
            .request("DELETE", &format!("/volumes/{}", encode(name)), None)
            .await
        {
            Ok(_) | Err(EngineError::Missing) => Ok(()),
            Err(error) => Err(error),
        }
    }

    pub async fn remove(&self, name: &str) -> Result<()> {
        match self
            .request(
                "DELETE",
                &format!("/containers/{}?force=true", encode(name)),
                None,
            )
            .await
        {
            Ok(_) | Err(EngineError::Missing) => Ok(()),
            Err(EngineError::Removing) => timeout(Duration::from_secs(10), async {
                loop {
                    match self.inspect(name).await {
                        Err(EngineError::Missing) => return Ok(()),
                        Err(error) => return Err(error),
                        Ok(_) => tokio::time::sleep(Duration::from_millis(20)).await,
                    }
                }
            })
            .await
            .map_err(|_| EngineError::Timeout)?,
            Err(error) => Err(error),
        }
    }

    pub async fn inspect(&self, name: &str) -> Result<Value> {
        self.request("GET", &format!("/containers/{}/json", encode(name)), None)
            .await
    }

    pub async fn wait(&self, name: &str) -> Result<i64> {
        timeout(Duration::from_secs(10), async {
            loop {
                let value = self.inspect(name).await?;
                if value["State"]["Running"].as_bool() == Some(false) {
                    return value["State"]["ExitCode"]
                        .as_i64()
                        .ok_or(EngineError::Protocol);
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .map_err(|_| EngineError::Timeout)?
    }

    pub async fn attach(&self, name: &str) -> Result<Attached> {
        timeout(REQUEST_TIMEOUT, async {
            let mut socket = UnixStream::connect(&self.path).await.map_err(|_| EngineError::Transport)?;
            let request = format!("POST /v1.47/containers/{}/attach?stream=1&stdin=1&stdout=1&stderr=0 HTTP/1.1\r\nHost: localhost\r\nConnection: Upgrade\r\nUpgrade: tcp\r\nContent-Length: 0\r\n\r\n", encode(name));
            socket.write_all(request.as_bytes()).await.map_err(|_| EngineError::Transport)?;
            let mut header = Vec::new();
            while !header.ends_with(b"\r\n\r\n") {
                if header.len() >= 65536 { return Err(EngineError::Budget); }
                header.push(socket.read_u8().await.map_err(|_| EngineError::Transport)?);
            }
            let first_line = header.split(|b| *b == b'\n').next().ok_or(EngineError::Protocol)?;
            let status = first_line.split(|b| *b == b' ').nth(1).ok_or(EngineError::Protocol)?;
            if status != b"101" && status != b"200" { return Err(EngineError::Protocol); }
            self.request("POST", &format!("/containers/{}/start", encode(name)), None).await?;
            let (read, write) = socket.into_split();
            Ok(Attached { input: write, output: Output::new(read) })
        }).await.map_err(|_| EngineError::Timeout)?
    }
}

pub fn encode(value: &str) -> String {
    percent_encoding::utf8_percent_encode(value, percent_encoding::NON_ALPHANUMERIC).to_string()
}

pub struct Attached {
    pub input: OwnedWriteHalf,
    pub output: Output,
}
impl Attached {
    pub async fn ready(&mut self, expected: &[u8], until: Instant) -> Result<()> {
        let line = tokio::time::timeout_at(until, self.output.line(32))
            .await
            .map_err(|_| EngineError::Timeout)??;
        if line != expected {
            return Err(EngineError::Protocol);
        }
        Ok(())
    }
}

pub struct Output {
    socket: OwnedReadHalf,
    pending: BytesMut,
    frame_remaining: usize,
    frame_stream: u8,
}
impl Output {
    fn new(socket: OwnedReadHalf) -> Self {
        Self {
            socket,
            pending: BytesMut::new(),
            frame_remaining: 0,
            frame_stream: 0,
        }
    }

    /// Callers retain this future until completion. Cancelling it mid-frame is
    /// permitted only when the entire attached container is being discarded.
    pub async fn line(&mut self, maximum: usize) -> Result<Vec<u8>> {
        let mut line = Vec::new();
        loop {
            if !self.pending.is_empty() {
                let newline = self.pending.iter().position(|byte| *byte == b'\n');
                let take = newline.map_or(self.pending.len(), |index| index + 1);
                if line.len() + take > maximum {
                    return Err(EngineError::Budget);
                }
                line.extend_from_slice(&self.pending.split_to(take));
                if newline.is_some() {
                    return Ok(line);
                }
            }
            if self.frame_remaining == 0 {
                let mut header = [0_u8; 8];
                // Distinguish a clean EOF from a truncated frame.
                let first = self
                    .socket
                    .read(&mut header[..1])
                    .await
                    .map_err(|_| EngineError::Transport)?;
                if first == 0 {
                    return Ok(line);
                }
                self.socket
                    .read_exact(&mut header[1..])
                    .await
                    .map_err(|_| EngineError::Protocol)?;
                let size = u32::from_be_bytes(header[4..8].try_into().unwrap()) as usize;
                if header[1..4] != [0, 0, 0] || !matches!(header[0], 1 | 2) || size > MAX_FRAME {
                    return Err(EngineError::Protocol);
                }
                self.frame_remaining = size;
                self.frame_stream = header[0];
            }
            // A valid frame may contain many lines. Incremental reads keep its
            // advertised 16 MiB size from becoming a per-connection allocation.
            let size = self.frame_remaining.min(16 * 1024);
            let mut chunk = vec![0; size];
            self.socket
                .read_exact(&mut chunk)
                .await
                .map_err(|_| EngineError::Protocol)?;
            self.frame_remaining -= size;
            if self.frame_stream == 1 {
                self.pending.extend_from_slice(&chunk);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tokio::net::UnixListener;

    async fn mock_engine(
        responses: Vec<(u16, &'static str)>,
    ) -> (
        Arc<Engine>,
        Arc<AtomicUsize>,
        Arc<AtomicUsize>,
        tokio::task::JoinHandle<()>,
        tempfile::TempDir,
    ) {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("docker.sock");
        let listener = UnixListener::bind(&path).unwrap();
        let connections = Arc::new(AtomicUsize::new(0));
        let requests = Arc::new(AtomicUsize::new(0));
        let replies = Arc::new(Mutex::new(VecDeque::from(responses)));
        let count = connections.clone();
        let request_count = requests.clone();
        let server = tokio::spawn(async move {
            loop {
                let (socket, _) = listener.accept().await.unwrap();
                count.fetch_add(1, Ordering::Relaxed);
                let replies = replies.clone();
                let request_count = request_count.clone();
                tokio::spawn(async move {
                    let service = hyper::service::service_fn(move |_request| {
                        let replies = replies.clone();
                        request_count.fetch_add(1, Ordering::Relaxed);
                        async move {
                            let (status, body) = replies
                                .lock()
                                .await
                                .pop_front()
                                .expect("Unexpected retry/request");
                            if status == 0 {
                                return Err(std::io::Error::other(
                                    "Dropped response after mutation",
                                ));
                            }
                            Ok(hyper::Response::builder()
                                .status(status)
                                .body(Full::new(Bytes::from_static(body.as_bytes())))
                                .unwrap())
                        }
                    });
                    let _ = hyper::server::conn::http1::Builder::new()
                        .serve_connection(TokioIo::new(socket), service)
                        .await;
                });
            }
        });
        (Engine::new(path), connections, requests, server, directory)
    }

    #[tokio::test]
    async fn engine_reuses_http_connections_and_confirms_concurrent_removal() {
        let (engine, connections, requests, server, _directory) = mock_engine(vec![
            (200, "{}"),
            (200, "{}"),
            (409, "{}"),
            (200, "{}"),
            (404, "{}"),
        ])
        .await;
        engine.request("GET", "/info", None).await.unwrap();
        engine.request("GET", "/info", None).await.unwrap();
        assert_eq!(connections.load(Ordering::Relaxed), 1);
        engine.remove("slot").await.unwrap();
        assert_eq!(requests.load(Ordering::Relaxed), 5);
        assert_eq!(connections.load(Ordering::Relaxed), 1);
        server.abort();
    }

    #[tokio::test]
    async fn ambiguous_mutation_is_never_retried() {
        let (engine, _connections, requests, server, _directory) = mock_engine(vec![(0, "")]).await;
        assert!(matches!(
            engine
                .request("POST", "/containers/create?name=slot", Some(json!({})))
                .await,
            Err(EngineError::Transport)
        ));
        assert_eq!(requests.load(Ordering::Relaxed), 1);
        server.abort();
    }

    #[tokio::test]
    async fn attach_upgrades_before_start_and_half_closes_stdin() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("attach.sock");
        let listener = UnixListener::bind(&path).unwrap();
        async fn header(socket: &mut UnixStream) -> String {
            let mut bytes = Vec::new();
            while !bytes.ends_with(b"\r\n\r\n") {
                bytes.push(socket.read_u8().await.unwrap());
            }
            String::from_utf8(bytes).unwrap()
        }
        let daemon = tokio::spawn(async move {
            let (mut attached, _) = listener.accept().await.unwrap();
            let request = header(&mut attached).await;
            assert!(request.starts_with("POST /v1.47/containers/slot/attach?"));
            assert!(request.contains("stderr=0"));
            attached
                .write_all(b"HTTP/1.1 101 UPGRADED\r\nConnection: Upgrade\r\nUpgrade: tcp\r\n\r\n")
                .await
                .unwrap();
            let (mut start, _) = listener.accept().await.unwrap();
            assert!(
                header(&mut start)
                    .await
                    .starts_with("POST /v1.47/containers/slot/start ")
            );
            start
                .write_all(b"HTTP/1.1 204 No Content\r\n\r\n")
                .await
                .unwrap();
            attached
                .write_all(&[1, 0, 0, 0, 0, 0, 0, 6, b'r', b'e', b'a', b'd', b'y', b'\n'])
                .await
                .unwrap();
            let mut input = Vec::new();
            attached.read_to_end(&mut input).await.unwrap();
            assert_eq!(input, b"dispatch\n");
            attached
                .write_all(&[1, 0, 0, 0, 0, 0, 0, 5, b'd', b'o', b'n', b'e', b'\n'])
                .await
                .unwrap();
        });
        let engine = Engine::new(path);
        let mut attached = engine.attach("slot").await.unwrap();
        attached
            .ready(b"ready\n", Instant::now() + Duration::from_secs(1))
            .await
            .unwrap();
        attached.input.write_all(b"dispatch\n").await.unwrap();
        attached.input.shutdown().await.unwrap();
        assert_eq!(attached.output.line(32).await.unwrap(), b"done\n");
        daemon.await.unwrap();
    }

    #[tokio::test]
    async fn multiplexed_stdout_excludes_stderr_and_preserves_lines() {
        let (read, mut write) = UnixStream::pair().unwrap();
        let mut output = Output::new(read.into_split().0);
        for (stream, data) in [
            (2, b"private stderr".as_slice()),
            (1, b"ready\nnext\n".as_slice()),
        ] {
            let mut frame = vec![stream, 0, 0, 0];
            frame.extend_from_slice(&(data.len() as u32).to_be_bytes());
            frame.extend_from_slice(data);
            write.write_all(&frame).await.unwrap();
        }
        assert_eq!(output.line(32).await.unwrap(), b"ready\n");
        assert_eq!(output.line(32).await.unwrap(), b"next\n");
    }

    #[tokio::test]
    async fn readiness_deadline_covers_partial_frames() {
        let (read, mut write) = UnixStream::pair().unwrap();
        let (reader, writer) = read.into_split();
        let mut attached = Attached {
            input: writer,
            output: Output::new(reader),
        };
        write
            .write_all(&[1, 0, 0, 0, 0, 0, 0, 6, b'r'])
            .await
            .unwrap();
        assert!(matches!(
            attached
                .ready(b"ready\n", Instant::now() + Duration::from_millis(20))
                .await,
            Err(EngineError::Timeout)
        ));
    }

    #[tokio::test]
    async fn rejects_oversized_frames_and_lines() {
        let (read, mut write) = UnixStream::pair().unwrap();
        let mut output = Output::new(read.into_split().0);
        write
            .write_all(&[1, 0, 0, 0, 255, 255, 255, 255])
            .await
            .unwrap();
        assert!(matches!(output.line(32).await, Err(EngineError::Protocol)));
        let (read, mut write) = UnixStream::pair().unwrap();
        let mut output = Output::new(read.into_split().0);
        write
            .write_all(&[1, 0, 0, 0, 0, 0, 0, 3, b'a', b'b', b'\n'])
            .await
            .unwrap();
        assert!(matches!(output.line(2).await, Err(EngineError::Budget)));
    }
}
