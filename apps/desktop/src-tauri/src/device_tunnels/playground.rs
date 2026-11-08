//! The owner's live view of a device bot's screen: one tunnel stream per (device, bot) that keeps
//! the latest frame and state, polled by the bot panel. The stream closes when the panel stops
//! polling. Frames travel device to desktop inside the E2E tunnel and never reach the hub.

use std::{
    collections::HashMap,
    future::Future,
    pin::Pin,
    sync::{Arc, LazyLock, Mutex, MutexGuard, PoisonError, Weak},
    time::{Duration, Instant},
};

use flow_like_device_client::{DeviceSession, TunnelMode, TunnelOpen, TunnelTarget};
use flow_like_device_protocol::{
    ManagementCommand, ManagementRequest, ManagementResponse, PlaygroundControl, PlaygroundDecoder,
    PlaygroundDisplay, PlaygroundEvent, PlaygroundFrame, PlaygroundFrameKind, PlaygroundState,
    TunnelDataOpen,
};
use flow_like_types::base64::{Engine as _, engine::general_purpose::STANDARD};
use flow_like_types::interaction_observation::InteractionObservation;
use serde::Serialize;
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt},
    sync::{mpsc, watch},
};

use crate::functions::ai::bot_tools::view::{ViewKind, ViewState};

/// The panel polls about once a second; a view it stopped polling closes after this long.
const LEASE: Duration = Duration::from_secs(30);
const LEASE_CHECK: Duration = Duration::from_secs(1);
const ACTIVE_FOR: Duration = Duration::from_secs(180);
const MAX_VIEWS: usize = 6;
const MAX_INSPECTION_BYTES: u64 = 1 << 20;

pub(super) static SESSIONS: LazyLock<Arc<Sessions<DeviceConnector>>> =
    LazyLock::new(|| Sessions::new(DeviceConnector));

pub(super) trait Duplex: AsyncRead + AsyncWrite + Unpin + Send {}
impl<T: AsyncRead + AsyncWrite + Unpin + Send> Duplex for T {}

type Connecting = Pin<Box<dyn Future<Output = Result<Box<dyn Duplex>, String>> + Send>>;

pub(super) trait Connector: Send + Sync + 'static {
    fn connect(&self, device_id: &str, bot_id: &str) -> Connecting;
}

/// What the bot panel shows; the first seven fields are those of `bot_computer_view`.
#[derive(Serialize, Debug, PartialEq)]
pub(crate) struct DeviceBotView {
    state: ViewState,
    kind: Option<ViewKind>,
    caption: Option<String>,
    image: Option<String>,
    age_ms: Option<u64>,
    observation: Option<InteractionObservation>,
    taken_over: bool,
    /// The owner can drive this picture: a computer screen on a device that has a display.
    can_control: bool,
    width: Option<u32>,
    height: Option<u32>,
    /// A refusal the device sent since the last poll, such as input it could not apply.
    notice: Option<String>,
}

#[derive(Default)]
struct Buffer {
    state: Option<PlaygroundState>,
    frame: Option<(PlaygroundFrame, Instant)>,
    notice: Option<String>,
    ended: Option<String>,
    polled: Option<Instant>,
}

pub(super) struct Live {
    buffer: Mutex<Buffer>,
    controls: mpsc::UnboundedSender<PlaygroundControl>,
    closed: watch::Sender<bool>,
    opened: Instant,
}

fn guard<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

impl Live {
    fn new() -> (Arc<Self>, mpsc::UnboundedReceiver<PlaygroundControl>) {
        let (controls, receiver) = mpsc::unbounded_channel();
        let (closed, _) = watch::channel(false);
        (
            Arc::new(Self {
                buffer: Mutex::default(),
                controls,
                closed,
                opened: Instant::now(),
            }),
            receiver,
        )
    }

    fn apply(&self, event: PlaygroundEvent) {
        let mut buffer = guard(&self.buffer);
        match event {
            PlaygroundEvent::State(state) => buffer.state = Some(state),
            PlaygroundEvent::Frame(frame) => buffer.frame = Some((frame, Instant::now())),
            PlaygroundEvent::Error { code, message } => {
                buffer.notice = Some(format!("{code}: {message}"));
            }
        }
    }

    fn end(&self, reason: impl Into<String>) {
        let mut buffer = guard(&self.buffer);
        buffer.ended.get_or_insert_with(|| reason.into());
        drop(buffer);
        self.closed.send_replace(true);
    }

    fn ended(&self) -> bool {
        guard(&self.buffer).ended.is_some()
    }

    fn expired(&self) -> bool {
        self.expired_at(Instant::now())
    }

    fn expired_at(&self, now: Instant) -> bool {
        let polled = guard(&self.buffer).polled.unwrap_or(self.opened);
        now.saturating_duration_since(polled) >= LEASE
    }

    fn send(&self, control: PlaygroundControl) -> Result<(), String> {
        if let Some(reason) = guard(&self.buffer).ended.clone() {
            return Err(format!("The live view of this bot ended: {reason}"));
        }
        self.controls
            .send(control)
            .map_err(|_| "The live view of this bot ended".to_owned())
    }

    fn view(&self) -> DeviceBotView {
        let mut buffer = guard(&self.buffer);
        buffer.polled = Some(Instant::now());
        let notice = buffer.notice.take();
        let state = buffer.state.clone();
        let taken_over = state.as_ref().is_some_and(|state| state.taken_over);
        let display = state.as_ref().map(|state| state.display);
        let fresh = buffer
            .frame
            .as_ref()
            .filter(|(frame, received)| {
                !state.as_ref().is_some_and(|state| state.asleep)
                    && Duration::from_millis(frame.meta.age_ms) + received.elapsed() <= ACTIVE_FOR
            })
            .map(|(frame, received)| {
                (
                    frame,
                    Duration::from_millis(frame.meta.age_ms) + received.elapsed(),
                )
            });
        let Some((frame, age)) = fresh else {
            return DeviceBotView {
                state: ViewState::Asleep,
                kind: None,
                caption: None,
                image: None,
                age_ms: None,
                observation: None,
                taken_over,
                can_control: false,
                width: None,
                height: None,
                notice,
            };
        };
        let computer = frame.meta.kind == PlaygroundFrameKind::Computer;
        DeviceBotView {
            state: ViewState::Active,
            kind: Some(if computer {
                ViewKind::Computer
            } else {
                ViewKind::Browser
            }),
            caption: Some(frame.meta.caption.clone()),
            image: Some(format!(
                "data:image/jpeg;base64,{}",
                STANDARD.encode(&frame.jpeg)
            )),
            age_ms: Some(age.as_millis() as u64),
            observation: frame.meta.observation.clone(),
            taken_over,
            can_control: computer
                && display.is_some_and(|display| display != PlaygroundDisplay::None),
            width: Some(frame.meta.width),
            height: Some(frame.meta.height),
            notice,
        }
    }
}

async fn drive(
    live: Arc<Live>,
    io: Box<dyn Duplex>,
    mut controls: mpsc::UnboundedReceiver<PlaygroundControl>,
) {
    let mut closed = live.closed.subscribe();
    let (mut read, mut write) = tokio::io::split(io);
    let mut decoder = PlaygroundDecoder::default();
    let mut chunk = vec![0u8; 16 * 1024];
    let reason = loop {
        tokio::select! {
            _ = closed.changed() => break "closed".to_owned(),
            control = controls.recv() => {
                let Some(control) = control else { break "closed".to_owned() };
                let bytes = match control.encode() {
                    Ok(bytes) => bytes,
                    Err(error) => {
                        live.apply(PlaygroundEvent::Error {
                            code: "invalid_input".into(),
                            message: error.to_string(),
                        });
                        continue;
                    }
                };
                if let Err(error) = write.write_all(&bytes).await {
                    break format!("sending to the device failed: {error}");
                }
            }
            read = read.read(&mut chunk) => {
                match read {
                    Ok(0) => break "the device closed the stream".to_owned(),
                    Err(error) => break format!("reading from the device failed: {error}"),
                    Ok(count) => decoder.push(&chunk[..count]),
                }
                loop {
                    match decoder.next_event() {
                        Ok(Some(event)) => live.apply(event),
                        Ok(None) => break,
                        Err(error) => {
                            live.end(format!("the device sent a malformed message: {error}"));
                            let _ = write.shutdown().await;
                            return;
                        }
                    }
                }
            }
        }
    };
    live.end(reason);
    let _ = write.shutdown().await;
}

pub(super) struct Sessions<C> {
    connector: C,
    entries: Mutex<HashMap<(String, String), Arc<Live>>>,
}

impl<C: Connector> Sessions<C> {
    pub(super) fn new(connector: C) -> Arc<Self> {
        Arc::new(Self {
            connector,
            entries: Mutex::default(),
        })
    }

    fn remove(&self, key: &(String, String), live: &Arc<Live>) {
        let mut entries = guard(&self.entries);
        if entries
            .get(key)
            .is_some_and(|current| Arc::ptr_eq(current, live))
        {
            entries.remove(key);
        }
        drop(entries);
        live.end("closed");
    }

    async fn live(self: &Arc<Self>, device_id: &str, bot_id: &str) -> Result<Arc<Live>, String> {
        let key = (device_id.to_owned(), bot_id.to_owned());
        {
            let mut entries = guard(&self.entries);
            match entries.get(&key) {
                Some(live) if !live.ended() => return Ok(live.clone()),
                Some(_) => {
                    entries.remove(&key);
                }
                None if entries.len() >= MAX_VIEWS => {
                    return Err(format!(
                        "At most {MAX_VIEWS} bot screens can be watched at once; close one first."
                    ));
                }
                None => {}
            }
        }
        let io = self.connector.connect(device_id, bot_id).await?;
        let (live, controls) = Live::new();
        {
            let mut entries = guard(&self.entries);
            if let Some(existing) = entries.get(&key).filter(|live| !live.ended()) {
                return Ok(existing.clone());
            }
            entries.insert(key.clone(), live.clone());
        }
        tokio::spawn(drive(live.clone(), io, controls));
        tokio::spawn(watch_lease(Arc::downgrade(self), key, live.clone()));
        Ok(live)
    }

    pub(super) async fn view(
        self: &Arc<Self>,
        device_id: &str,
        bot_id: &str,
    ) -> Result<DeviceBotView, String> {
        Ok(self.live(device_id, bot_id).await?.view())
    }

    pub(super) async fn control(
        self: &Arc<Self>,
        device_id: &str,
        bot_id: &str,
        control: PlaygroundControl,
    ) -> Result<(), String> {
        self.live(device_id, bot_id).await?.send(control)
    }

    pub(super) fn close_all(&self) {
        let all: Vec<_> = guard(&self.entries).drain().map(|(_, live)| live).collect();
        for live in all {
            live.end("closed");
        }
    }

    pub(super) fn close_device(&self, device_id: &str) {
        let mut gone = Vec::new();
        guard(&self.entries).retain(|(device, _), live| {
            let keep = device != device_id;
            if !keep {
                gone.push(live.clone());
            }
            keep
        });
        for live in gone {
            live.end("closed");
        }
    }
}

async fn watch_lease<C: Connector>(
    sessions: Weak<Sessions<C>>,
    key: (String, String),
    live: Arc<Live>,
) {
    loop {
        tokio::time::sleep(LEASE_CHECK).await;
        let Some(sessions) = sessions.upgrade() else {
            return;
        };
        if live.ended() {
            sessions.remove(&key, &live);
            return;
        }
        if live.expired() {
            sessions.remove(&key, &live);
            return;
        }
    }
}

pub(super) struct DeviceConnector;

fn open_of(bot_id: &str) -> TunnelOpen {
    TunnelOpen {
        placement_id: String::new(),
        service_id: String::new(),
        mode: TunnelMode::Tcp,
        target: TunnelTarget::BotPlayground {
            bot_id: bot_id.to_owned(),
            observations: true,
        },
    }
}

impl Connector for DeviceConnector {
    fn connect(&self, device_id: &str, bot_id: &str) -> Connecting {
        let (device_id, bot_id) = (device_id.to_owned(), bot_id.to_owned());
        Box::pin(async move {
            let session = crate::device_models::session(&device_id).await?;
            if agent_offers_playground(&session).await? == Some(false) {
                return Err(format!(
                    "The agent on device {device_id} cannot show bot screens; update the agent"
                ));
            }
            let stream = session
                .open_stream(open_of(&bot_id))
                .await
                .map_err(|error| {
                    format!("Device {device_id} refused the live view of bot {bot_id}: {error}")
                })?;
            Ok(Box::new(stream) as Box<dyn Duplex>)
        })
    }
}

/// An agent without the `bot_playground` flag cannot decode the open and drops the whole tunnel,
/// so the flag is read first. `None` when this grant may not inspect the device.
async fn agent_offers_playground(session: &DeviceSession) -> Result<Option<bool>, String> {
    let issued_at = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs() as i64);
    let request = ManagementRequest {
        operation_id: format!("bot-playground-{}", uuid::Uuid::new_v4()),
        device_id: session.device_id().to_owned(),
        issued_at,
        expires_at: issued_at + 60,
        command: ManagementCommand::Inspect,
    };
    let mut stream = session
        .open_data(TunnelDataOpen::Request { request })
        .await
        .map_err(|error| format!("Could not inspect device {}: {error}", session.device_id()))?;
    let mut reply = Vec::new();
    stream
        .shutdown()
        .await
        .map_err(|error| format!("Could not inspect device {}: {error}", session.device_id()))?;
    (&mut stream)
        .take(MAX_INSPECTION_BYTES)
        .read_to_end(&mut reply)
        .await
        .map_err(|error| {
            format!(
                "Reading the inspection of {} failed: {error}",
                session.device_id()
            )
        })?;
    let response = serde_json::from_slice::<ManagementResponse>(&reply).map_err(|error| {
        format!(
            "The inspection of {} is malformed: {error}",
            session.device_id()
        )
    })?;
    inspected_flag(&response)
}

fn inspected_flag(response: &ManagementResponse) -> Result<Option<bool>, String> {
    match response.state.as_str() {
        "completed" => Ok(Some(
            response.result["features"]["bot_playground"].as_u64() == Some(1),
        )),
        "rejected" if response.result["code"] == "unauthorized" => Ok(None),
        state => Err(format!(
            "The device answered its inspection with {state}: {}",
            response.result["error"]
        )),
    }
}

pub(super) async fn view(device_id: &str, bot_id: &str) -> Result<DeviceBotView, String> {
    SESSIONS.view(device_id, bot_id).await
}

pub(super) async fn control(
    device_id: &str,
    bot_id: &str,
    control: PlaygroundControl,
) -> Result<(), String> {
    SESSIONS.control(device_id, bot_id, control).await
}

#[cfg(test)]
mod tests;
