//! The playground over a pipe: [`serve`] runs a [`Playground`] for the other end, and
//! [`RemotePlayground`] is a [`Playground`] whose work happens there.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use flow_like_bot_tools::browser::{
    BrowserBackend, BrowserScreen, BrowserStep, Download, PageState, SnapshotRequest, SnapshotView,
};
use flow_like_bot_tools::computer::{
    ComputerAction, ComputerBackend, Screen, WindowBounds, WindowOp, WindowSummary,
};
use flow_like_types::{Bytes, Result, anyhow};
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::sync::{mpsc, oneshot};

use crate::frames::{FrameKind, Shot};
use crate::input::InputEvent;
use crate::playground::Playground;
use crate::wire::{Request, Response, WireAction, WireError, WirePage, WireStep, decode, encode};

const CALL_TIMEOUT: Duration = Duration::from_secs(240);
const WORKER_GONE: &str = "The bot's playground worker stopped; the next call starts a new one";

fn params<T: DeserializeOwned>(method: &str, value: Value) -> Result<T> {
    serde_json::from_value(value)
        .map_err(|e| anyhow!("Bad parameters for playground call {method}: {e}"))
}

#[derive(Deserialize)]
struct ConvParams {
    #[serde(default)]
    conv: Option<String>,
}

#[derive(Deserialize)]
struct OpenParams {
    #[serde(default)]
    conv: Option<String>,
    url: String,
}

#[derive(Deserialize)]
struct SnapshotParams {
    #[serde(default)]
    conv: Option<String>,
    max_chars: usize,
    interactive_only: bool,
}

#[derive(Deserialize)]
struct StepParams {
    #[serde(default)]
    conv: Option<String>,
    step: WireStep,
}

#[derive(Deserialize)]
struct WindowShotParams {
    #[serde(default)]
    window: Option<String>,
}

#[derive(Deserialize)]
struct WindowParams {
    window: String,
    op: WindowOp,
    bounds: WindowBounds,
}

#[derive(Deserialize)]
struct LaunchParams {
    program: String,
    args: Vec<String>,
}

#[derive(Deserialize)]
struct InjectParams {
    event: InputEvent,
    width: u32,
    height: u32,
}

fn screen_json(screen: Screen) -> Value {
    json!({ "png": encode(&screen.png), "width": screen.width, "height": screen.height,
        "note": screen.note, "observation": screen.observation })
}

fn page_json(page: PageState) -> Value {
    json!(WirePage::from(page))
}

async fn dispatch(playground: &dyn Playground, method: &str, value: Value) -> Result<Value> {
    let browser = |conv: Option<String>| playground.browser(conv.as_deref());
    Ok(match method {
        "ping" => json!("pong"),
        "browser.open" => {
            let p: OpenParams = params(method, value)?;
            page_json(browser(p.conv).open(&p.url).await?)
        }
        "browser.snapshot" => {
            let p: SnapshotParams = params(method, value)?;
            let view = browser(p.conv)
                .snapshot(SnapshotRequest {
                    max_chars: p.max_chars,
                    interactive_only: p.interactive_only,
                })
                .await?;
            json!({ "page": WirePage::from(view.page), "text": view.text, "truncated": view.truncated })
        }
        "browser.act" => {
            let p: StepParams = params(method, value)?;
            let step: BrowserStep = p.step.into();
            page_json(browser(p.conv).act(&step).await?)
        }
        "browser.screenshot" => {
            let p: ConvParams = params(method, value)?;
            let capture = browser(p.conv).capture().await?;
            json!({ "png": encode(&capture.png), "observation": capture.observation })
        }
        "browser.observation" => {
            let p: ConvParams = params(method, value)?;
            json!(browser(p.conv).observation().await?)
        }
        "browser.current" => {
            let p: ConvParams = params(method, value)?;
            json!(browser(p.conv).current().await?.map(WirePage::from))
        }
        "browser.leave" => {
            let p: ConvParams = params(method, value)?;
            browser(p.conv).leave().await?;
            Value::Null
        }
        "browser.downloads" => {
            let p: ConvParams = params(method, value)?;
            let files = browser(p.conv).take_downloads().await?;
            json!(
                files
                    .iter()
                    .map(|file| json!({ "name": file.name, "data": encode(&file.bytes) }))
                    .collect::<Vec<_>>()
            )
        }
        "browser.close" => {
            let p: ConvParams = params(method, value)?;
            browser(p.conv).close().await?;
            Value::Null
        }
        "computer.screenshot" => {
            let p: WindowShotParams = params(method, value)?;
            let computer = playground.computer();
            screen_json(match p.window {
                Some(window) => computer.screenshot_window(&window).await?,
                None => computer.screenshot().await?,
            })
        }
        "computer.act" => {
            let action: WireAction = params(method, value)?;
            json!(playground.computer().act(&action.into()).await?)
        }
        "computer.windows" => json!(playground.computer().windows().await?),
        "computer.window" => {
            let p: WindowParams = params(method, value)?;
            json!(
                playground
                    .computer()
                    .window(&p.window, p.op, p.bounds)
                    .await?
            )
        }
        "computer.launch" => {
            let p: LaunchParams = params(method, value)?;
            json!(playground.computer().launch(&p.program, &p.args).await?)
        }
        "capture" => match playground.capture().await? {
            Some(shot) => {
                json!({ "png": encode(&shot.png), "kind": shot.kind, "caption": shot.caption,
                    "observation": shot.observation })
            }
            None => Value::Null,
        },
        "inject" => {
            let p: InjectParams = params(method, value)?;
            playground.inject(&p.event, p.width, p.height).await?;
            Value::Null
        }
        "stop" => {
            playground.stop().await;
            Value::Null
        }
        other => return Err(anyhow!("Unknown playground call '{other}'")),
    })
}

/// Answers requests from `reader` until it ends, then stops the playground.
pub async fn serve<R, W>(reader: R, writer: W, playground: Arc<dyn Playground>)
where
    R: AsyncRead + Send + Unpin + 'static,
    W: AsyncWrite + Send + Unpin + 'static,
{
    let (out, mut lines) = mpsc::unbounded_channel::<String>();
    let writer_task = tokio::spawn(async move {
        let mut writer = writer;
        while let Some(line) = lines.recv().await {
            if writer.write_all(line.as_bytes()).await.is_err()
                || writer.write_all(b"\n").await.is_err()
                || writer.flush().await.is_err()
            {
                break;
            }
        }
    });
    let mut input = BufReader::new(reader).lines();
    while let Ok(Some(line)) = input.next_line().await {
        let Ok(request) = serde_json::from_str::<Request>(&line) else {
            continue;
        };
        let (playground, out) = (playground.clone(), out.clone());
        tokio::spawn(async move {
            let result = dispatch(playground.as_ref(), &request.method, request.params).await;
            let response = match result {
                Ok(ok) => Response {
                    id: request.id,
                    ok: Some(ok),
                    err: None,
                },
                Err(error) => Response {
                    id: request.id,
                    ok: None,
                    err: Some(WireError::of(&error)),
                },
            };
            if let Ok(line) = serde_json::to_string(&response) {
                let _ = out.send(line);
            }
        });
    }
    playground.stop().await;
    drop(out);
    let _ = writer_task.await;
}

type Pending = Arc<Mutex<HashMap<u64, oneshot::Sender<std::result::Result<Value, WireError>>>>>;

/// The agent's end of one worker pipe. Dropping it drops the guard, which ends the worker.
pub struct Connection {
    out: mpsc::UnboundedSender<String>,
    pending: Pending,
    next: AtomicU64,
    alive: Arc<AtomicBool>,
    _guard: Box<dyn Send + Sync>,
}

impl Connection {
    pub fn start<R, W>(reader: R, writer: W, guard: Box<dyn Send + Sync>) -> Arc<Self>
    where
        R: AsyncRead + Send + Unpin + 'static,
        W: AsyncWrite + Send + Unpin + 'static,
    {
        let pending: Pending = Arc::default();
        let alive = Arc::new(AtomicBool::new(true));
        let (out, mut lines) = mpsc::unbounded_channel::<String>();
        tokio::spawn(async move {
            let mut writer = writer;
            while let Some(line) = lines.recv().await {
                if writer.write_all(line.as_bytes()).await.is_err()
                    || writer.write_all(b"\n").await.is_err()
                    || writer.flush().await.is_err()
                {
                    break;
                }
            }
        });
        let (reading, alive_flag) = (pending.clone(), alive.clone());
        tokio::spawn(async move {
            let mut input = BufReader::new(reader).lines();
            while let Ok(Some(line)) = input.next_line().await {
                let Ok(response) = serde_json::from_str::<Response>(&line) else {
                    tracing::debug!("Ignoring a non-protocol line from a playground worker");
                    continue;
                };
                let waiter = reading
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .remove(&response.id);
                if let Some(waiter) = waiter {
                    let _ = waiter.send(match response.err {
                        Some(error) => Err(error),
                        None => Ok(response.ok.unwrap_or(Value::Null)),
                    });
                }
            }
            alive_flag.store(false, Ordering::Release);
            let waiting: Vec<_> = reading
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .drain()
                .collect();
            for (_, waiter) in waiting {
                let _ = waiter.send(Err(WireError {
                    code: None,
                    message: WORKER_GONE.into(),
                }));
            }
        });
        Arc::new(Self {
            out,
            pending,
            next: AtomicU64::new(1),
            alive,
            _guard: guard,
        })
    }

    pub fn is_alive(&self) -> bool {
        self.alive.load(Ordering::Acquire)
    }

    pub async fn call(&self, method: &str, value: Value) -> Result<Value> {
        if !self.is_alive() {
            return Err(anyhow!(WORKER_GONE));
        }
        let id = self.next.fetch_add(1, Ordering::Relaxed);
        let (waiter, answer) = oneshot::channel();
        self.pending
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(id, waiter);
        let line = serde_json::to_string(&Request {
            id,
            method: method.into(),
            params: value,
        })?;
        if self.out.send(line).is_err() {
            self.pending
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .remove(&id);
            return Err(anyhow!(WORKER_GONE));
        }
        match tokio::time::timeout(CALL_TIMEOUT, answer).await {
            Ok(Ok(Ok(ok))) => Ok(ok),
            Ok(Ok(Err(error))) => Err(error.into_error()),
            Ok(Err(_)) => Err(anyhow!(WORKER_GONE)),
            Err(_) => {
                self.pending
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .remove(&id);
                Err(anyhow!(
                    "The playground worker did not answer {method} within {} s",
                    CALL_TIMEOUT.as_secs()
                ))
            }
        }
    }
}

pub struct Transport {
    pub reader: Box<dyn AsyncRead + Send + Unpin>,
    pub writer: Box<dyn AsyncWrite + Send + Unpin>,
    pub guard: Box<dyn Send + Sync>,
}

/// Starts the worker process (or, in tests, an in-process stand-in) of one bot.
#[async_trait]
pub trait Spawner: Send + Sync {
    async fn spawn(&self) -> Result<Transport>;
}

/// A bot's playground in its worker process, started on the first call and again after it ends.
pub struct RemotePlayground {
    spawner: Arc<dyn Spawner>,
    connection: tokio::sync::Mutex<Option<Arc<Connection>>>,
    last_used: Mutex<Instant>,
}

impl RemotePlayground {
    pub fn new(spawner: Arc<dyn Spawner>) -> Arc<Self> {
        Arc::new(Self {
            spawner,
            connection: tokio::sync::Mutex::new(None),
            last_used: Mutex::new(Instant::now()),
        })
    }

    fn touch(&self) {
        *self.last_used.lock().unwrap_or_else(|e| e.into_inner()) = Instant::now();
    }

    fn idle_for(&self) -> Duration {
        self.last_used
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .elapsed()
    }

    async fn connected(&self) -> Result<Arc<Connection>> {
        let mut slot = self.connection.lock().await;
        if let Some(connection) = slot.clone().filter(|c| c.is_alive()) {
            return Ok(connection);
        }
        let transport = self.spawner.spawn().await?;
        let connection = Connection::start(transport.reader, transport.writer, transport.guard);
        *slot = Some(connection.clone());
        Ok(connection)
    }

    async fn running(&self) -> Option<Arc<Connection>> {
        self.connection
            .lock()
            .await
            .clone()
            .filter(|c| c.is_alive())
    }

    async fn call(&self, method: &str, value: Value) -> Result<Value> {
        self.touch();
        let result = self.connected().await?.call(method, value).await;
        self.touch();
        result
    }

    async fn call_if_running(&self, method: &str, value: Value) -> Result<Option<Value>> {
        match self.running().await {
            Some(connection) => Ok(Some(connection.call(method, value).await?)),
            None => Ok(None),
        }
    }

    async fn end(&self) {
        if let Some(connection) = self.connection.lock().await.take() {
            let _ =
                tokio::time::timeout(Duration::from_secs(3), connection.call("stop", Value::Null))
                    .await;
        }
    }
}

fn field<T: DeserializeOwned>(value: Value, name: &str) -> Result<T> {
    serde_json::from_value(value.get(name).cloned().unwrap_or(Value::Null))
        .map_err(|e| anyhow!("Playground answered with a bad '{name}': {e}"))
}

fn screen_of(value: Value) -> Result<Screen> {
    let png: String = field(value.clone(), "png")?;
    let mut screen = Screen::new(
        decode(&png)?,
        field(value.clone(), "width")?,
        field(value.clone(), "height")?,
    );
    screen.note = field(value.clone(), "note")?;
    screen.observation = field(value, "observation")?;
    Ok(screen)
}

fn page_of(value: Value) -> Result<PageState> {
    Ok(serde_json::from_value::<WirePage>(value)
        .map_err(|e| anyhow!("Playground answered with a bad page: {e}"))?
        .into())
}

#[async_trait]
impl Playground for Arc<RemotePlayground> {
    fn browser(&self, conversation: Option<&str>) -> Arc<dyn BrowserBackend> {
        Arc::new(RemoteBrowser {
            remote: self.clone(),
            conv: conversation.map(str::to_string),
        })
    }

    fn computer(&self) -> Arc<dyn ComputerBackend> {
        Arc::new(RemoteComputer {
            remote: self.clone(),
        })
    }

    async fn capture(&self) -> Result<Option<Shot>> {
        let Some(value) = self.call_if_running("capture", Value::Null).await? else {
            return Ok(None);
        };
        if value.is_null() {
            return Ok(None);
        }
        let png: String = field(value.clone(), "png")?;
        let kind: FrameKind = field(value.clone(), "kind")?;
        Ok(Some(Shot {
            png: decode(&png)?,
            kind,
            caption: field(value.clone(), "caption")?,
            observation: field(value, "observation")?,
        }))
    }

    async fn inject(&self, event: &InputEvent, width: u32, height: u32) -> Result<()> {
        self.call(
            "inject",
            json!({ "event": event, "width": width, "height": height }),
        )
        .await?;
        Ok(())
    }

    async fn stop(&self) {
        self.end().await;
    }

    async fn sweep(&self, idle: Duration) {
        if self.idle_for() > idle {
            self.end().await;
        }
    }
}

struct RemoteBrowser {
    remote: Arc<RemotePlayground>,
    conv: Option<String>,
}

impl RemoteBrowser {
    async fn call(&self, method: &str, mut extra: Value) -> Result<Value> {
        extra["conv"] = json!(self.conv);
        self.remote.call(method, extra).await
    }
}

#[async_trait]
impl BrowserBackend for RemoteBrowser {
    async fn open(&self, url: &str) -> Result<PageState> {
        page_of(self.call("browser.open", json!({ "url": url })).await?)
    }

    async fn snapshot(&self, request: SnapshotRequest) -> Result<SnapshotView> {
        let value = self
            .call(
                "browser.snapshot",
                json!({ "max_chars": request.max_chars, "interactive_only": request.interactive_only }),
            )
            .await?;
        Ok(SnapshotView {
            page: page_of(value.get("page").cloned().unwrap_or(Value::Null))?,
            text: field(value.clone(), "text")?,
            truncated: field(value, "truncated")?,
        })
    }

    async fn act(&self, step: &BrowserStep) -> Result<PageState> {
        page_of(
            self.call("browser.act", json!({ "step": WireStep::from(step) }))
                .await?,
        )
    }

    async fn screenshot(&self) -> Result<Vec<u8>> {
        Ok(self.capture().await?.png)
    }

    async fn capture(&self) -> Result<BrowserScreen> {
        let value = self.call("browser.screenshot", json!({})).await?;
        Ok(BrowserScreen {
            png: decode(&field::<String>(value.clone(), "png")?)?,
            observation: field(value, "observation")?,
        })
    }

    async fn observation(
        &self,
    ) -> Result<Option<flow_like_types::interaction_observation::InteractionObservation>> {
        let value = self.call("browser.observation", json!({})).await?;
        serde_json::from_value(value).map_err(|e| anyhow!("Bad browser observation: {e}"))
    }

    async fn current(&self) -> Result<Option<PageState>> {
        let value = self
            .remote
            .call_if_running("browser.current", json!({ "conv": self.conv }))
            .await?;
        match value {
            Some(page) if !page.is_null() => Ok(Some(page_of(page)?)),
            _ => Ok(None),
        }
    }

    async fn leave(&self) -> Result<()> {
        self.call("browser.leave", json!({})).await?;
        Ok(())
    }

    async fn take_downloads(&self) -> Result<Vec<Download>> {
        let Some(value) = self
            .remote
            .call_if_running("browser.downloads", json!({ "conv": self.conv }))
            .await?
        else {
            return Ok(Vec::new());
        };
        let files: Vec<Value> =
            serde_json::from_value(value).map_err(|e| anyhow!("Bad download list: {e}"))?;
        files
            .into_iter()
            .map(|file| {
                let data: String = field(file.clone(), "data")?;
                Ok(Download {
                    name: field(file, "name")?,
                    bytes: Bytes::from(decode(&data)?),
                })
            })
            .collect()
    }

    async fn close(&self) -> Result<()> {
        self.remote
            .call_if_running("browser.close", json!({ "conv": self.conv }))
            .await?;
        Ok(())
    }
}

struct RemoteComputer {
    remote: Arc<RemotePlayground>,
}

#[async_trait]
impl ComputerBackend for RemoteComputer {
    async fn screenshot(&self) -> Result<Screen> {
        screen_of(self.remote.call("computer.screenshot", json!({})).await?)
    }

    async fn act(&self, action: &ComputerAction) -> Result<String> {
        let value = self
            .remote
            .call("computer.act", json!(WireAction::from(action)))
            .await?;
        serde_json::from_value(value).map_err(|e| anyhow!("Bad answer to computer.act: {e}"))
    }

    async fn screenshot_window(&self, window: &str) -> Result<Screen> {
        screen_of(
            self.remote
                .call("computer.screenshot", json!({ "window": window }))
                .await?,
        )
    }

    async fn windows(&self) -> Result<Vec<WindowSummary>> {
        let value = self.remote.call("computer.windows", json!({})).await?;
        serde_json::from_value(value).map_err(|e| anyhow!("Bad window list: {e}"))
    }

    async fn window(&self, window: &str, op: WindowOp, bounds: WindowBounds) -> Result<String> {
        let value = self
            .remote
            .call(
                "computer.window",
                json!({ "window": window, "op": op, "bounds": bounds }),
            )
            .await?;
        serde_json::from_value(value).map_err(|e| anyhow!("Bad answer to computer.window: {e}"))
    }

    async fn launch(&self, program: &str, args: &[String]) -> Result<String> {
        let value = self
            .remote
            .call(
                "computer.launch",
                json!({ "program": program, "args": args }),
            )
            .await?;
        serde_json::from_value(value).map_err(|e| anyhow!("Bad answer to computer.launch: {e}"))
    }
}
