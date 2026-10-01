// Derived from rustwright src/lib.rs @fca1438, Copyright (c) 2026 Ikonomos Inc (dba Skyvern), MIT; modified by Rheosoph GmbH. See NOTICE.
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;
use futures_util::future::BoxFuture;
use serde_json::{Value, json};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};
use tokio::sync::watch;
use tokio::task::{AbortHandle, JoinHandle};

use super::{
    Inbound, Outbound, Transport, TransportChannels, TransportControl, TransportShutdown,
    WriteStatus,
};

const BINARY_FRAME: &str = "\u{0}binary:";
const CLOSE_FRAME: &str = "\u{0}close:";
const COMMAND_WAIT: Duration = Duration::from_secs(5);
const CLOSE_FLUSH: Duration = Duration::from_secs(1);

type CloseFuture = BoxFuture<'static, ()>;

pub struct ScriptedLink {
    pub to_peer: UnboundedSender<String>,
    pub from_peer: UnboundedReceiver<String>,
}

pub(crate) enum PeerFrame {
    Text(String),
    Binary(Vec<u8>),
    Close(String),
}

impl PeerFrame {
    pub(crate) fn decode(frame: String) -> PeerFrame {
        if let Some(encoded) = frame.strip_prefix(BINARY_FRAME) {
            return PeerFrame::Binary(BASE64.decode(encoded).unwrap_or_default());
        }
        if let Some(reason) = frame.strip_prefix(CLOSE_FRAME) {
            return PeerFrame::Close(reason.to_owned());
        }
        PeerFrame::Text(frame)
    }
}

#[derive(Clone)]
pub(crate) struct PeerSender(UnboundedSender<String>);

impl PeerSender {
    fn send(&self, frame: String) {
        if self.0.send(frame).is_err() {
            tracing::debug!("scripted peer frame dropped: the client transport is gone");
        }
    }

    pub(crate) fn reply(&self, command: &SentCommand, result: Value) {
        self.send(envelope(
            json!({"id": command.id, "result": result}),
            command.session.as_deref(),
        ));
    }

    pub(crate) fn reply_error(&self, command: &SentCommand, code: i64, message: &str) {
        let body = json!({"id": command.id, "error": {"code": code, "message": message}});
        self.send(envelope(body, command.session.as_deref()));
    }

    pub(crate) fn emit(&self, method: &str, session: Option<&str>, params: Value) {
        self.send(envelope(
            json!({"method": method, "params": params}),
            session,
        ));
    }

    pub(crate) fn send_text(&self, raw: &str) {
        self.send(raw.to_owned());
    }

    pub(crate) fn send_invalid_utf8(&self, raw: &[u8]) {
        self.send(format!("{BINARY_FRAME}{}", BASE64.encode(raw)));
    }

    pub(crate) fn close(&self, reason: &str) {
        self.send(format!("{CLOSE_FRAME}{reason}"));
    }
}

fn envelope(mut body: Value, session: Option<&str>) -> String {
    if let (Some(session), Some(object)) = (session, body.as_object_mut()) {
        object.insert("sessionId".into(), Value::from(session));
    }
    body.to_string()
}

#[derive(Clone, Debug)]
pub struct SentCommand {
    pub id: u64,
    pub method: String,
    pub session: Option<String>,
    pub params: serde_json::Value,
}

impl SentCommand {
    fn parse(text: &str) -> SentCommand {
        let Ok(value) = serde_json::from_str::<Value>(text) else {
            return SentCommand {
                id: 0,
                method: String::new(),
                session: None,
                params: Value::String(text.to_owned()),
            };
        };
        SentCommand {
            id: value.get("id").and_then(Value::as_u64).unwrap_or_default(),
            method: value
                .get("method")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned(),
            session: value
                .get("sessionId")
                .and_then(Value::as_str)
                .map(str::to_owned),
            params: value.get("params").cloned().unwrap_or(Value::Null),
        }
    }
}

type AutoReply = Arc<dyn Fn(&SentCommand) -> Option<Value> + Send + Sync>;
pub(crate) type ReplyAnnouncer = Arc<dyn Fn(&SentCommand, &Value) + Send + Sync>;

#[derive(Default)]
struct ControlShared {
    seen: Mutex<Vec<SentCommand>>,
    auto_reply: Mutex<Option<AutoReply>>,
    announcer: Mutex<Option<ReplyAnnouncer>>,
}

impl ControlShared {
    fn record(&self, command: SentCommand) {
        self.seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(command);
    }

    fn seen(&self) -> Vec<SentCommand> {
        self.seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn auto_reply_for(&self, command: &SentCommand) -> Option<Value> {
        let reply = self
            .auto_reply
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()?;
        reply(command)
    }

    fn announce(&self, command: &SentCommand, result: &Value) {
        let announcer = self
            .announcer
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        if let Some(announcer) = announcer {
            announcer(command, result);
        }
    }
}

pub struct InMemoryTransport {
    link: ScriptedLink,
}

pub struct InMemoryControl {
    pub(crate) peer: PeerSender,
    commands: UnboundedReceiver<SentCommand>,
    shared: Arc<ControlShared>,
}

pub fn scripted_pair() -> (ScriptedLink, InMemoryControl) {
    let (to_peer, from_client) = unbounded_channel();
    let (to_client, from_peer) = unbounded_channel();
    let (queue, commands) = unbounded_channel();
    let shared = Arc::new(ControlShared::default());
    let peer = PeerSender(to_client);
    tokio::spawn(pump(from_client, peer.clone(), shared.clone(), queue));
    let control = InMemoryControl {
        peer,
        commands,
        shared,
    };
    (ScriptedLink { to_peer, from_peer }, control)
}

async fn pump(
    mut from_client: UnboundedReceiver<String>,
    peer: PeerSender,
    shared: Arc<ControlShared>,
    queue: UnboundedSender<SentCommand>,
) {
    while let Some(text) = from_client.recv().await {
        let command = SentCommand::parse(&text);
        shared.record(command.clone());
        match shared.auto_reply_for(&command) {
            Some(result) => {
                shared.announce(&command, &result);
                peer.reply(&command, result);
            }
            None => {
                if queue.send(command).is_err() {
                    tracing::debug!("scripted peer command dropped: the control is gone");
                }
            }
        }
    }
}

impl InMemoryTransport {
    pub fn new() -> (InMemoryTransport, InMemoryControl) {
        let (link, control) = scripted_pair();
        (InMemoryTransport { link }, control)
    }
}

impl Transport for InMemoryTransport {
    fn start(self: Box<Self>) -> TransportChannels {
        let ScriptedLink { to_peer, from_peer } = self.link;
        let (inbound_tx, inbound) = unbounded_channel();
        let (outbound, outbound_rx) = unbounded_channel();
        let (closing, closing_rx) = watch::channel(false);
        let inlet = Arc::new(Inlet(Mutex::new(Some(inbound_tx))));
        let reader = tokio::spawn(read_peer(from_peer, inlet.clone())).abort_handle();
        let writer = tokio::spawn(write_peer(outbound_rx, to_peer, closing_rx));
        let shutdown = MemoryShutdown {
            inlet,
            reader,
            writer: Mutex::new(Some(writer)),
            closing,
        };
        TransportChannels {
            inbound,
            outbound,
            control: TransportControl::new(Arc::new(shutdown)),
        }
    }
}

struct Inlet(Mutex<Option<UnboundedSender<Inbound>>>);

impl Inlet {
    fn deliver(&self, inbound: Inbound) -> bool {
        let guard = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        guard
            .as_ref()
            .is_some_and(|sender| sender.send(inbound).is_ok())
    }

    fn finish(&self, reason: String) {
        let sender = self.0.lock().unwrap_or_else(PoisonError::into_inner).take();
        if let Some(sender) = sender {
            let _ = sender.send(Inbound::Closed { reason });
        }
    }
}

async fn read_peer(mut from_peer: UnboundedReceiver<String>, inlet: Arc<Inlet>) {
    while let Some(frame) = from_peer.recv().await {
        let inbound = match PeerFrame::decode(frame) {
            PeerFrame::Text(text) => Inbound::Text(text),
            PeerFrame::Binary(bytes) => match String::from_utf8(bytes) {
                Ok(text) => Inbound::Text(text),
                Err(error) => Inbound::InvalidUtf8(error.into_bytes()),
            },
            PeerFrame::Close(reason) => {
                inlet.finish(reason);
                return;
            }
        };
        if !inlet.deliver(inbound) {
            return;
        }
    }
    inlet.finish("the scripted peer went away".to_owned());
}

async fn write_peer(
    mut outbound: UnboundedReceiver<Outbound>,
    to_peer: UnboundedSender<String>,
    mut closing: watch::Receiver<bool>,
) {
    loop {
        tokio::select! {
            message = outbound.recv() => {
                let Some(message) = message else {
                    return;
                };
                if !forward(message, &to_peer) {
                    return;
                }
            }
            _ = closing.wait_for(|closing| *closing) => {
                while let Ok(message) = outbound.try_recv() {
                    if !forward(message, &to_peer) {
                        return;
                    }
                }
                return;
            }
        }
    }
}

fn forward(message: Outbound, to_peer: &UnboundedSender<String>) -> bool {
    if message
        .deadline
        .is_some_and(|deadline| deadline <= tokio::time::Instant::now())
    {
        message.ticket.set(WriteStatus::NotWritten);
        return true;
    }
    message.ticket.set(WriteStatus::Indeterminate);
    if to_peer.send(message.text).is_err() {
        return false;
    }
    message.ticket.set(WriteStatus::Written);
    true
}

struct MemoryShutdown {
    inlet: Arc<Inlet>,
    reader: AbortHandle,
    writer: Mutex<Option<JoinHandle<()>>>,
    closing: watch::Sender<bool>,
}

impl MemoryShutdown {
    fn begin_close(&self, reason: &str) {
        self.inlet.finish(reason.to_owned());
        self.reader.abort();
        let _ = self.closing.send(true);
    }
}

impl TransportShutdown for MemoryShutdown {
    fn abort(&self) {
        self.begin_close("aborted by the client");
    }

    fn close(&self) -> CloseFuture {
        self.begin_close("closed by the client");
        let writer = self
            .writer
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        Box::pin(flush_writer(writer))
    }
}

async fn flush_writer(writer: Option<JoinHandle<()>>) {
    if let Some(writer) = writer {
        let _ = tokio::time::timeout(CLOSE_FLUSH, writer).await;
    }
}

impl InMemoryControl {
    pub async fn next_command(&mut self) -> SentCommand {
        match tokio::time::timeout(COMMAND_WAIT, self.commands.recv()).await {
            Ok(Some(command)) => command,
            Ok(None) => panic!(
                "the scripted peer stopped receiving commands; commands seen: {}",
                self.describe_seen()
            ),
            Err(_) => panic!(
                "no command arrived within {} s; commands seen: {}",
                COMMAND_WAIT.as_secs(),
                self.describe_seen()
            ),
        }
    }

    pub async fn expect(&mut self, method: &str) -> SentCommand {
        let command = self.next_command().await;
        assert_eq!(
            command.method,
            method,
            "expected {method} next; commands seen: {}",
            self.describe_seen()
        );
        command
    }

    pub async fn wait_for(&mut self, method: &str, session: Option<&str>) -> SentCommand {
        loop {
            let command = self.next_command().await;
            if command.method == method && command.session.as_deref() == session {
                return command;
            }
            self.peer.reply(&command, json!({}));
        }
    }

    pub fn try_next_command(&mut self) -> Option<SentCommand> {
        self.commands.try_recv().ok()
    }

    pub fn reply(&self, command: &SentCommand, result: serde_json::Value) {
        self.peer.reply(command, result);
    }

    pub fn reply_error(&self, command: &SentCommand, code: i64, message: &str) {
        self.peer.reply_error(command, code, message);
    }

    pub fn emit(&self, method: &str, session: Option<&str>, params: serde_json::Value) {
        self.peer.emit(method, session, params);
    }

    pub fn send_text(&self, raw: &str) {
        self.peer.send_text(raw);
    }

    pub fn send_invalid_utf8(&self, raw: &[u8]) {
        self.peer.send_invalid_utf8(raw);
    }

    pub fn close(&self, reason: &str) {
        self.peer.close(reason);
    }

    /// Replaces the previous auto reply. `None` leaves the command to `next_command`/`wait_for`,
    /// so a test withholds or error-replies a command by returning `None` for it and composes
    /// `testing::default_auto_reply` for everything else.
    pub fn set_auto_reply(
        &self,
        reply: impl Fn(&SentCommand) -> Option<serde_json::Value> + Send + Sync + 'static,
    ) {
        *self
            .shared
            .auto_reply
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Some(Arc::new(reply));
    }

    pub(crate) fn set_reply_announcer(&self, announcer: ReplyAnnouncer) {
        *self
            .shared
            .announcer
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Some(announcer);
    }

    pub fn commands_seen(&self) -> Vec<SentCommand> {
        self.shared.seen()
    }

    fn describe_seen(&self) -> String {
        let seen = self.shared.seen();
        if seen.is_empty() {
            return "none".to_owned();
        }
        seen.iter()
            .map(|command| match &command.session {
                Some(session) => format!("{}@{session}", command.method),
                None => command.method.clone(),
            })
            .collect::<Vec<_>>()
            .join(", ")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::connection::{
        Connection, ConnectionOptions, MethodMatch, RouteFilter, SessionScope,
    };
    use crate::error::BrowserError;
    use crate::transport::WriteTicket;
    use crate::types::SessionId;

    fn connect() -> (Connection, InMemoryControl) {
        let (transport, control) = InMemoryTransport::new();
        let connection = Connection::start(Box::new(transport), ConnectionOptions::default());
        (connection, control)
    }

    #[tokio::test]
    async fn scripted_peer_replies_to_the_matching_command() {
        let (connection, mut control) = connect();
        let call = tokio::spawn({
            let connection = connection.clone();
            async move {
                connection
                    .send_raw("Browser.getVersion", json!({}), None)
                    .await
            }
        });
        let command = control.expect("Browser.getVersion").await;
        assert_eq!(command.session, None);
        control.reply(&command, json!({"product": "Chrome/154"}));
        let result = call.await.unwrap().unwrap();
        assert_eq!(result["product"], "Chrome/154");
    }

    #[tokio::test]
    async fn scripted_peer_errors_become_protocol_errors() {
        let (connection, mut control) = connect();
        let session = SessionId::from("S1");
        let call = tokio::spawn({
            let connection = connection.clone();
            async move {
                connection
                    .send_raw(
                        "DOM.resolveNode",
                        json!({"backendNodeId": 4}),
                        Some(&session),
                    )
                    .await
            }
        });
        let command = control.wait_for("DOM.resolveNode", Some("S1")).await;
        assert_eq!(command.params["backendNodeId"], 4);
        control.reply_error(&command, -32000, "No node with given id found");
        let error = call.await.unwrap().unwrap_err();
        assert!(matches!(
            error,
            BrowserError::Protocol { ref method, code: -32000, .. } if method == "DOM.resolveNode"
        ));
    }

    #[tokio::test]
    async fn scripted_peer_emits_events_to_routes() {
        let (connection, control) = connect();
        let mut route = connection.route(RouteFilter {
            sessions: SessionScope::Exactly(SessionId::from("S1")),
            methods: vec![MethodMatch::Prefix("Page.")],
        });
        control.emit("Page.loadEventFired", Some("S2"), json!({}));
        control.emit("Runtime.consoleAPICalled", Some("S1"), json!({}));
        control.emit(
            "Page.frameStoppedLoading",
            Some("S1"),
            json!({"frameId": "F1"}),
        );
        let event = route.recv().await.unwrap();
        assert_eq!(&*event.method, "Page.frameStoppedLoading");
        assert_eq!(event.params["frameId"], "F1");
    }

    #[tokio::test]
    async fn auto_replied_commands_are_seen_but_not_queued() {
        let (connection, mut control) = connect();
        control.set_auto_reply(|command| (command.method == "Page.enable").then(|| json!({})));
        let session = SessionId::from("S1");
        connection
            .send_raw("Page.enable", json!({}), Some(&session))
            .await
            .unwrap();
        assert!(control.try_next_command().is_none());
        let seen: Vec<_> = control
            .commands_seen()
            .into_iter()
            .map(|command| command.method)
            .collect();
        assert_eq!(seen, ["Page.enable"]);
    }

    #[tokio::test]
    async fn the_latest_auto_reply_replaces_the_previous_one() {
        let (connection, mut control) = connect();
        control.set_auto_reply(|command| (command.method == "Page.enable").then(|| json!({})));
        control.set_auto_reply(|command| {
            (command.method == "Runtime.evaluate").then(|| json!({"result": {"value": 3}}))
        });
        let session = SessionId::from("S1");
        let evaluated = connection
            .send_raw(
                "Runtime.evaluate",
                json!({"expression": "1"}),
                Some(&session),
            )
            .await
            .unwrap();
        assert_eq!(evaluated["result"]["value"], 3);
        let enable = connection
            .session(Some(session))
            .enqueue("Page.enable", json!({}), None)
            .unwrap();
        let withheld = control.expect("Page.enable").await;
        control.reply_error(&withheld, -32000, "Page.enable held back");
        assert!(matches!(
            enable.await,
            Err(BrowserError::Protocol { code: -32000, .. })
        ));
    }

    #[tokio::test]
    async fn wait_for_answers_the_commands_it_skips() {
        let (connection, mut control) = connect();
        let session = SessionId::from("S1");
        let first = connection
            .session(Some(session.clone()))
            .enqueue("Page.enable", json!({}), None)
            .unwrap();
        let second = connection
            .session(Some(session.clone()))
            .enqueue("Page.getFrameTree", json!({}), None)
            .unwrap();
        let wanted = control.wait_for("Page.getFrameTree", Some("S1")).await;
        control.reply(&wanted, json!({"frameTree": {}}));
        assert_eq!(first.await.unwrap().result, json!({}));
        assert_eq!(second.await.unwrap().result, json!({"frameTree": {}}));
    }

    #[tokio::test]
    async fn binary_and_close_frames_reach_the_client() {
        let (transport, control) = InMemoryTransport::new();
        let mut channels = Box::new(transport).start();
        control.send_invalid_utf8(&[b'{', 0xff, b'}']);
        control.send_text("{\"method\":\"Page.loadEventFired\"}");
        control.close("peer hung up");
        assert!(matches!(
            channels.inbound.recv().await,
            Some(Inbound::InvalidUtf8(bytes)) if bytes == [b'{', 0xff, b'}']
        ));
        assert!(matches!(
            channels.inbound.recv().await,
            Some(Inbound::Text(_))
        ));
        assert!(matches!(
            channels.inbound.recv().await,
            Some(Inbound::Closed { reason }) if reason == "peer hung up"
        ));
        assert!(channels.inbound.recv().await.is_none());
    }

    #[tokio::test]
    async fn writer_skips_commands_past_their_deadline() {
        let (transport, mut control) = InMemoryTransport::new();
        let channels = Box::new(transport).start();
        let expired = WriteTicket::default();
        let written = WriteTicket::default();
        let now = tokio::time::Instant::now();
        for (method, deadline, ticket) in [
            ("Page.reload", now - Duration::from_millis(1), &expired),
            ("Page.enable", now + Duration::from_secs(5), &written),
        ] {
            let text = json!({"id": 1, "method": method, "params": {}}).to_string();
            let outbound = Outbound {
                text,
                method: method.into(),
                deadline: Some(deadline),
                ticket: ticket.clone(),
            };
            assert!(channels.outbound.send(outbound).is_ok());
        }
        assert_eq!(control.next_command().await.method, "Page.enable");
        assert_eq!(expired.status(), WriteStatus::NotWritten);
        assert_eq!(written.status(), WriteStatus::Written);
        assert!(control.try_next_command().is_none());
    }

    #[tokio::test]
    async fn closing_the_peer_disconnects_pending_commands() {
        let (connection, mut control) = connect();
        let pending = connection
            .session(None)
            .enqueue("Browser.getVersion", json!({}), None)
            .unwrap();
        control.expect("Browser.getVersion").await;
        control.close("scripted crash");
        assert!(matches!(
            pending.await,
            Err(BrowserError::Disconnected { reason }) if reason == "scripted crash"
        ));
        connection.closed().await;
        assert_eq!(
            connection.closed_reason().as_deref(),
            Some("scripted crash")
        );
    }
}
