// Derived from agent-browser cli/src/native/cdp/client.rs @d01253d, Copyright 2025 Vercel Inc., Apache-2.0; modified by Rheosoph GmbH. See NOTICE.
use std::collections::{HashMap, HashSet, VecDeque};
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, Weak};
use std::task::{Context, Poll};
use std::time::Duration;

use serde::Deserialize as _;
use serde_json::Value;
use tokio::sync::{mpsc, oneshot, watch};
use tokio::task::AbortHandle;
use tokio::time::{Instant, Sleep};

use crate::error::BrowserError;
use crate::event_log::{Event, EventCursor, EventLog, EventLogLimits};
use crate::protocol::{self, InboundMessage, ProtocolErrorBody};
use crate::transport::{
    Inbound, MAX_OUTBOUND_BYTES, Outbound, Transport, TransportControl, WriteTicket,
};
use crate::types::{SessionId, TargetId, TargetInfo, TargetType};

const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);
const LIFTED_OP_DEADLINE: Duration = Duration::from_secs(24 * 60 * 60);
const PARSE_ERROR: i64 = -32700;
const DEAD_SESSION_MEMORY: usize = 4096;

const ALLOWED_AFTER_CRASH: &[&str] = &[
    "Page.reload",
    "Page.navigate",
    "Page.getNavigationHistory",
    "Page.navigateToHistoryEntry",
    "Page.stopLoading",
    "Page.handleJavaScriptDialog",
    "Runtime.runIfWaitingForDebugger",
    "Inspector.enable",
];

const LOGGED_PREFIXES: &[&str] = &[
    "Page.",
    "Target.",
    "Browser.",
    "Inspector.",
    "Runtime.executionContext",
];

tokio::task_local! {
    static OP_DEADLINE: Instant;
}

#[derive(Clone, Debug)]
pub struct ConnectionOptions {
    pub default_timeout: std::time::Duration,
    pub event_log: crate::event_log::EventLogLimits,
}

impl Default for ConnectionOptions {
    fn default() -> Self {
        Self {
            default_timeout: DEFAULT_TIMEOUT,
            event_log: EventLogLimits::default(),
        }
    }
}

#[derive(Clone)]
pub struct Connection {
    inner: std::sync::Arc<ConnectionInner>,
}

struct ConnectionInner {
    options: ConnectionOptions,
    outbound: mpsc::UnboundedSender<Outbound>,
    transport: TransportControl,
    next_id: AtomicU64,
    pending: Mutex<HashMap<u64, Pending>>,
    registry: Mutex<Registry>,
    hooks: Mutex<Arc<[Arc<dyn EventHook>]>>,
    routes: Mutex<Vec<Route>>,
    next_route: AtomicU64,
    events: EventLog,
    closed: watch::Sender<Option<String>>,
    reader: Mutex<Option<AbortHandle>>,
}

pub struct Request {
    pub method: std::borrow::Cow<'static, str>,
    pub params: serde_json::Value,
    pub session: Option<SessionId>,
    pub timeout: Option<std::time::Duration>,
}

#[derive(Debug)]
pub struct Reply {
    pub result: serde_json::Value,
    pub cursor: crate::event_log::EventCursor,
}

pub struct PendingReply {
    id: u64,
    method: Arc<str>,
    timeout: Duration,
    ticket: WriteTicket,
    cursor: EventCursor,
    receiver: oneshot::Receiver<crate::Result<Value>>,
    sleep: Pin<Box<Sleep>>,
    connection: Weak<ConnectionInner>,
}

impl PendingReply {
    pub fn ticket(&self) -> &WriteTicket {
        &self.ticket
    }

    pub fn cursor(&self) -> crate::event_log::EventCursor {
        self.cursor
    }

    fn abandon(&self) {
        if let Some(inner) = self.connection.upgrade() {
            inner.pending().remove(&self.id);
        }
    }
}

impl std::future::Future for PendingReply {
    type Output = crate::Result<Reply>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();
        match Pin::new(&mut this.receiver).poll(cx) {
            Poll::Ready(Ok(result)) => {
                let cursor = this.cursor;
                return Poll::Ready(result.map(|result| Reply { result, cursor }));
            }
            Poll::Ready(Err(_)) => {
                let reason = this
                    .connection
                    .upgrade()
                    .and_then(|inner| inner.closed.borrow().clone())
                    .unwrap_or_else(|| format!("the reply channel of {} closed", this.method));
                return Poll::Ready(Err(BrowserError::Disconnected { reason }));
            }
            Poll::Pending => {}
        }
        if this.sleep.as_mut().poll(cx).is_ready() {
            this.abandon();
            return Poll::Ready(Err(BrowserError::Timeout {
                method: this.method.to_string(),
                timeout_ms: u64::try_from(this.timeout.as_millis()).unwrap_or(u64::MAX),
            }));
        }
        Poll::Pending
    }
}

impl Drop for PendingReply {
    fn drop(&mut self) {
        self.abandon();
    }
}

pub trait EventHook: Send + Sync {
    fn on_event(&self, connection: &Connection, event: &crate::event_log::Event);
}

#[derive(Clone, Debug)]
pub enum SessionScope {
    Exactly(SessionId),
    WithDescendants(SessionId),
}

#[derive(Clone, Copy, Debug)]
pub enum MethodMatch {
    Exact(&'static str),
    Prefix(&'static str),
}

impl MethodMatch {
    pub(crate) fn matches(self, method: &str) -> bool {
        match self {
            Self::Exact(exact) => method == exact,
            Self::Prefix(prefix) => method.starts_with(prefix),
        }
    }
}

#[derive(Clone, Debug)]
pub struct RouteFilter {
    pub sessions: SessionScope,
    pub methods: Vec<MethodMatch>,
}

impl RouteFilter {
    fn accepts(&self, event: &Event, lineage: &[SessionId]) -> bool {
        let in_scope = match &self.sessions {
            SessionScope::Exactly(id) => event.session.as_ref() == Some(id),
            SessionScope::WithDescendants(id) => lineage.contains(id),
        };
        in_scope
            && self
                .methods
                .iter()
                .any(|method| method.matches(&event.method))
    }
}

struct Route {
    id: u64,
    filter: RouteFilter,
    sender: mpsc::UnboundedSender<Event>,
}

pub struct RouteReceiver {
    receiver: mpsc::UnboundedReceiver<Event>,
    id: u64,
    connection: Weak<ConnectionInner>,
}

impl RouteReceiver {
    pub async fn recv(&mut self) -> Option<crate::event_log::Event> {
        self.receiver.recv().await
    }
}

impl Drop for RouteReceiver {
    fn drop(&mut self) {
        if let Some(inner) = self.connection.upgrade() {
            inner.routes().retain(|route| route.id != self.id);
        }
    }
}

#[derive(Clone, Debug)]
pub struct SessionInfo {
    pub id: SessionId,
    pub target: crate::types::TargetInfo,
    pub parent: Option<SessionId>,
    pub crashed: bool,
    pub detached: bool,
}

struct Pending {
    method: Arc<str>,
    session: Option<SessionId>,
    responder: oneshot::Sender<crate::Result<Value>>,
}

impl Pending {
    fn respond(self, result: crate::Result<Value>) {
        let _ = self.responder.send(result);
    }
}

enum Failure {
    Closed,
    Crashed(String),
}

#[derive(Default)]
struct RegistryChange {
    fail: Vec<(Vec<SessionId>, Failure)>,
    remove: Vec<SessionId>,
}

#[derive(Default)]
struct Registry {
    sessions: HashMap<SessionId, SessionInfo>,
    dead: DeadSessions,
}

/// Remembers the newest detached ids so commands racing a detach fail fast; an id
/// evicted from here is sent, and Chrome answers -32001 (`ErrorClass::SessionGone`).
#[derive(Default)]
struct DeadSessions {
    ids: HashSet<SessionId>,
    order: VecDeque<SessionId>,
}

impl DeadSessions {
    fn contains(&self, id: &SessionId) -> bool {
        self.ids.contains(id)
    }

    fn insert(&mut self, id: &SessionId) {
        if !self.ids.insert(id.clone()) {
            return;
        }
        self.order.push_back(id.clone());
        if self.order.len() > DEAD_SESSION_MEMORY
            && let Some(oldest) = self.order.pop_front()
        {
            self.ids.remove(&oldest);
        }
    }

    fn remove(&mut self, id: &SessionId) {
        if self.ids.remove(id) {
            self.order.retain(|dead| dead != id);
        }
    }
}

impl Registry {
    fn descendants(&self, id: &SessionId) -> Vec<SessionId> {
        let mut found: Vec<SessionId> = Vec::new();
        let mut frontier = vec![id.clone()];
        while let Some(parent) = frontier.pop() {
            for info in self.sessions.values() {
                if info.parent.as_ref() == Some(&parent)
                    && info.id != *id
                    && !found.contains(&info.id)
                {
                    found.push(info.id.clone());
                    frontier.push(info.id.clone());
                }
            }
        }
        found
    }

    fn subtree(&self, id: &SessionId) -> Vec<SessionId> {
        let mut sessions = vec![id.clone()];
        sessions.extend(self.descendants(id));
        sessions
    }

    fn lineage(&self, session: Option<&SessionId>) -> Vec<SessionId> {
        let mut lineage = Vec::new();
        let mut current = session.cloned();
        while let Some(id) = current {
            if lineage.contains(&id) {
                break;
            }
            current = self.sessions.get(&id).and_then(|info| info.parent.clone());
            lineage.push(id);
        }
        lineage
    }

    fn apply(
        &mut self,
        method: &str,
        session: Option<&SessionId>,
        params: &Value,
    ) -> RegistryChange {
        let mut change = RegistryChange::default();
        match method {
            "Target.attachedToTarget" => self.attach(session, params),
            "Target.detachedFromTarget" => {
                if let Some(id) = session_param(params) {
                    self.detach(&id, &mut change);
                }
            }
            "Inspector.detached" => {
                if let Some(id) = session {
                    self.detach(id, &mut change);
                }
            }
            "Inspector.targetCrashed" => {
                if let Some(id) = session {
                    self.crash(id, &mut change);
                }
            }
            "Target.targetCrashed" if session.is_none() => {
                let target = params
                    .get("targetId")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                let crashed: Vec<SessionId> = self
                    .sessions
                    .values()
                    .filter(|info| info.target.target_id.as_str() == target)
                    .map(|info| info.id.clone())
                    .collect();
                for id in crashed {
                    self.crash(&id, &mut change);
                }
            }
            "Inspector.targetReloadedAfterCrash" => {
                if let Some(info) = session.and_then(|id| self.sessions.get_mut(id)) {
                    info.crashed = false;
                }
            }
            _ => {}
        }
        change
    }

    fn attach(&mut self, parent: Option<&SessionId>, params: &Value) {
        let Some(id) = session_param(params) else {
            tracing::debug!("Target.attachedToTarget without a sessionId");
            return;
        };
        let target = attached_target(params);
        self.dead.remove(&id);
        self.sessions.insert(
            id.clone(),
            SessionInfo {
                id,
                target,
                parent: parent.cloned(),
                crashed: false,
                detached: false,
            },
        );
    }

    fn detach(&mut self, id: &SessionId, change: &mut RegistryChange) {
        let sessions = self.subtree(id);
        for session in &sessions {
            if let Some(info) = self.sessions.get_mut(session) {
                info.detached = true;
            }
            self.dead.insert(session);
        }
        change.remove.extend(sessions.iter().cloned());
        change.fail.push((sessions, Failure::Closed));
    }

    fn crash(&mut self, id: &SessionId, change: &mut RegistryChange) {
        let info = self
            .sessions
            .entry(id.clone())
            .or_insert_with(|| SessionInfo {
                id: id.clone(),
                target: TargetInfo::default(),
                parent: None,
                crashed: false,
                detached: false,
            });
        info.crashed = true;
        let target = info.target.target_id.to_string();
        change
            .fail
            .push((self.subtree(id), Failure::Crashed(target)));
    }
}

fn session_param(params: &Value) -> Option<SessionId> {
    params
        .get("sessionId")
        .and_then(Value::as_str)
        .map(SessionId::from)
}

fn attached_target(params: &Value) -> TargetInfo {
    let raw = params.get("targetInfo").unwrap_or(&Value::Null);
    TargetInfo::deserialize(raw).unwrap_or_else(|error| {
        tracing::debug!(
            method = "Target.attachedToTarget",
            %error,
            "typed decode failed; keeping only targetId and type"
        );
        TargetInfo {
            target_id: raw
                .get("targetId")
                .and_then(Value::as_str)
                .map(TargetId::from)
                .unwrap_or_default(),
            type_: raw
                .get("type")
                .and_then(|kind| TargetType::deserialize(kind).ok())
                .unwrap_or_default(),
            ..TargetInfo::default()
        }
    })
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

struct Submitted {
    id: u64,
    method: Arc<str>,
    timeout: Duration,
    deadline: Instant,
    ticket: WriteTicket,
    cursor: EventCursor,
    receiver: Option<oneshot::Receiver<crate::Result<Value>>>,
}

impl ConnectionInner {
    fn pending(&self) -> MutexGuard<'_, HashMap<u64, Pending>> {
        lock(&self.pending)
    }

    fn registry(&self) -> MutexGuard<'_, Registry> {
        lock(&self.registry)
    }

    fn routes(&self) -> MutexGuard<'_, Vec<Route>> {
        lock(&self.routes)
    }

    fn hooks(&self) -> MutexGuard<'_, Arc<[Arc<dyn EventHook>]>> {
        lock(&self.hooks)
    }

    fn disconnected(&self) -> BrowserError {
        BrowserError::Disconnected {
            reason: self
                .closed
                .borrow()
                .clone()
                .unwrap_or_else(|| "the transport stopped accepting commands".to_owned()),
        }
    }

    fn check_sendable(&self, method: &str, session: Option<&SessionId>) -> crate::Result<()> {
        if let Some(reason) = self.closed.borrow().clone() {
            return Err(BrowserError::Disconnected { reason });
        }
        let Some(session) = session else {
            return Ok(());
        };
        let registry = self.registry();
        if registry.dead.contains(session) {
            return Err(BrowserError::TargetClosed {
                method: method.to_owned(),
            });
        }
        let allowed = method.starts_with("Target.") || ALLOWED_AFTER_CRASH.contains(&method);
        match registry.sessions.get(session) {
            Some(info) if info.crashed && !allowed => Err(BrowserError::TargetCrashed {
                target_id: info.target.target_id.to_string(),
            }),
            _ => Ok(()),
        }
    }

    fn submit(&self, request: Request, respond: bool) -> crate::Result<Submitted> {
        let method: Arc<str> = Arc::from(request.method.as_ref());
        self.check_sendable(&method, request.session.as_ref())?;
        let timeout =
            bounded_by_op_deadline(request.timeout.unwrap_or(self.options.default_timeout));
        if timeout.is_zero() {
            return Err(BrowserError::Timeout {
                method: method.to_string(),
                timeout_ms: 0,
            });
        }
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let text = protocol::encode_command(id, &method, &request.params, request.session.as_ref());
        if text.len() > MAX_OUTBOUND_BYTES {
            return Err(BrowserError::InvalidArgument {
                message: format!(
                    "{method} message is {} bytes; Chrome resets the connection above ~100 MiB",
                    text.len()
                ),
            });
        }
        let deadline = Instant::now() + timeout;
        let cursor = self.events.cursor();
        let ticket = WriteTicket::default();
        let receiver = respond
            .then(|| self.register_sendable(id, &method, request.session))
            .transpose()?;
        let outbound = Outbound {
            text,
            method: method.clone(),
            deadline: Some(deadline),
            ticket: ticket.clone(),
        };
        if self.outbound.send(outbound).is_err() || self.closed.borrow().is_some() {
            self.pending().remove(&id);
            return Err(self.disconnected());
        }
        Ok(Submitted {
            id,
            method,
            timeout,
            deadline,
            ticket,
            cursor,
            receiver,
        })
    }

    fn register(
        &self,
        id: u64,
        method: &Arc<str>,
        session: Option<SessionId>,
    ) -> oneshot::Receiver<crate::Result<Value>> {
        let (responder, receiver) = oneshot::channel();
        let pending = Pending {
            method: method.clone(),
            session,
            responder,
        };
        self.pending().insert(id, pending);
        receiver
    }

    /// The reader marks a crash or detach in the registry before it scans `pending`, so checking
    /// again after registering means that scan either finds this entry or the check sees the mark.
    fn register_sendable(
        &self,
        id: u64,
        method: &Arc<str>,
        session: Option<SessionId>,
    ) -> crate::Result<oneshot::Receiver<crate::Result<Value>>> {
        let receiver = self.register(id, method, session.clone());
        if let Err(error) = self.check_sendable(method, session.as_ref()) {
            self.pending().remove(&id);
            return Err(error);
        }
        Ok(receiver)
    }

    fn handle(self: &Arc<Self>, message: InboundMessage) {
        match message {
            InboundMessage::Response { id, result, .. } => self.resolve(id, result),
            InboundMessage::Malformed {
                id: Some(id),
                detail,
            } => self.fail_malformed(id, detail),
            InboundMessage::Malformed { id: None, detail } => {
                tracing::warn!(%detail, "dropped a malformed Chrome DevTools message");
            }
            InboundMessage::Event {
                method,
                session,
                params,
            } => self.dispatch(method, session, params),
        }
    }

    fn resolve(&self, id: u64, result: Result<Value, ProtocolErrorBody>) {
        let Some(pending) = self.pending().remove(&id) else {
            match result {
                Ok(_) => tracing::debug!(id, "reply for a command nobody waits for"),
                Err(body) => tracing::debug!(
                    id,
                    code = body.code,
                    message = %body.message,
                    "error reply for a command nobody waits for"
                ),
            }
            return;
        };
        let result = result.map_err(|body| BrowserError::Protocol {
            method: pending.method.to_string(),
            code: body.code,
            message: match body.data {
                Some(data) if !data.is_empty() => format!("{}: {data}", body.message),
                _ => body.message,
            },
        });
        pending.respond(result);
    }

    fn fail_malformed(&self, id: u64, detail: String) {
        let Some(pending) = self.pending().remove(&id) else {
            tracing::debug!(id, %detail, "malformed reply for a command nobody waits for");
            return;
        };
        let error = BrowserError::protocol(&pending.method, PARSE_ERROR, detail);
        pending.respond(Err(error));
    }

    fn fail_sessions(&self, sessions: &[SessionId], failure: &Failure) {
        let failed: Vec<Pending> = {
            let mut pending = self.pending();
            let ids: Vec<u64> = pending
                .iter()
                .filter(|(_, entry)| {
                    entry
                        .session
                        .as_ref()
                        .is_some_and(|session| sessions.contains(session))
                })
                .map(|(id, _)| *id)
                .collect();
            ids.iter().filter_map(|id| pending.remove(id)).collect()
        };
        for entry in failed {
            let error = match failure {
                Failure::Closed => BrowserError::TargetClosed {
                    method: entry.method.to_string(),
                },
                Failure::Crashed(target_id) => BrowserError::TargetCrashed {
                    target_id: target_id.clone(),
                },
            };
            entry.respond(Err(error));
        }
    }

    fn dispatch(self: &Arc<Self>, method: Arc<str>, session: Option<SessionId>, params: Value) {
        let change = self.registry().apply(&method, session.as_ref(), &params);
        for (sessions, failure) in &change.fail {
            self.fail_sessions(sessions, failure);
        }
        let event = Event {
            seq: self.events.next_seq(),
            method,
            session,
            params: Arc::new(params),
            received: Instant::now(),
        };
        let hooks = self.hooks().clone();
        if !hooks.is_empty() {
            let connection = Connection {
                inner: self.clone(),
            };
            for hook in hooks.iter() {
                hook.on_event(&connection, &event);
            }
        }
        self.deliver_to_routes(&event);
        if LOGGED_PREFIXES
            .iter()
            .any(|prefix| event.method.starts_with(prefix))
        {
            self.events.push(event);
        }
        if !change.remove.is_empty() {
            let mut registry = self.registry();
            for id in &change.remove {
                registry.sessions.remove(id);
            }
        }
    }

    fn deliver_to_routes(&self, event: &Event) {
        if self.routes().is_empty() {
            return;
        }
        let lineage = self.registry().lineage(event.session.as_ref());
        self.routes().retain(|route| {
            !route.filter.accepts(event, &lineage) || route.sender.send(event.clone()).is_ok()
        });
    }

    fn shutdown(&self, reason: &str) {
        let first = self.closed.send_if_modified(|closed| {
            if closed.is_some() {
                return false;
            }
            *closed = Some(reason.to_owned());
            true
        });
        if !first {
            return;
        }
        let pending: Vec<Pending> = self.pending().drain().map(|(_, entry)| entry).collect();
        for entry in pending {
            entry.respond(Err(BrowserError::Disconnected {
                reason: reason.to_owned(),
            }));
        }
        let routes = std::mem::take(&mut *self.routes());
        let hooks = std::mem::replace(&mut *self.hooks(), Arc::from(Vec::new()));
        drop((routes, hooks));
        self.events.close(reason);
    }

    fn stop_reader(&self) {
        if let Some(reader) = lock(&self.reader).take() {
            reader.abort();
        }
    }
}

fn bounded_by_op_deadline(timeout: Duration) -> Duration {
    OP_DEADLINE
        .try_with(|deadline| deadline.saturating_duration_since(Instant::now()))
        .map_or(timeout, |remaining| timeout.min(remaining))
}

struct ReaderExit(Arc<ConnectionInner>);

impl Drop for ReaderExit {
    fn drop(&mut self) {
        let reason = if std::thread::panicking() {
            "the CDP reader panicked while handling a message"
        } else {
            "the CDP reader task was cancelled"
        };
        self.0.shutdown(reason);
    }
}

async fn read_loop(inner: Arc<ConnectionInner>, mut inbound: mpsc::UnboundedReceiver<Inbound>) {
    let exit = ReaderExit(inner);
    let reason = loop {
        let Some(message) = inbound.recv().await else {
            break "the transport ended without a close reason".to_owned();
        };
        let parsed = match message {
            Inbound::Text(text) => protocol::parse_inbound(&text),
            Inbound::InvalidUtf8(bytes) => protocol::parse_invalid_utf8(&bytes),
            Inbound::Closed { reason } => break reason,
        };
        exit.0.handle(parsed);
    };
    exit.0.shutdown(&reason);
}

impl Connection {
    pub fn start(transport: Box<dyn Transport>, options: ConnectionOptions) -> Connection {
        let channels = transport.start();
        let inner = Arc::new(ConnectionInner {
            events: EventLog::new(options.event_log),
            options,
            outbound: channels.outbound,
            transport: channels.control,
            next_id: AtomicU64::new(1),
            pending: Mutex::new(HashMap::new()),
            registry: Mutex::new(Registry::default()),
            hooks: Mutex::new(Arc::from(Vec::new())),
            routes: Mutex::new(Vec::new()),
            next_route: AtomicU64::new(1),
            closed: watch::Sender::new(None),
            reader: Mutex::new(None),
        });
        let reader = tokio::spawn(read_loop(inner.clone(), channels.inbound));
        *lock(&inner.reader) = Some(reader.abort_handle());
        Connection { inner }
    }

    pub fn enqueue(&self, request: Request) -> crate::Result<PendingReply> {
        let submitted = self.inner.submit(request, true)?;
        let Some(receiver) = submitted.receiver else {
            return Err(self.inner.disconnected());
        };
        Ok(PendingReply {
            id: submitted.id,
            method: submitted.method,
            timeout: submitted.timeout,
            ticket: submitted.ticket,
            cursor: submitted.cursor,
            receiver,
            sleep: Box::pin(tokio::time::sleep_until(submitted.deadline)),
            connection: Arc::downgrade(&self.inner),
        })
    }

    pub async fn request(&self, request: Request) -> crate::Result<Reply> {
        self.enqueue(request)?.await
    }

    pub async fn send_raw(
        &self,
        method: &str,
        params: serde_json::Value,
        session: Option<&SessionId>,
    ) -> crate::Result<serde_json::Value> {
        let request = Request {
            method: method.to_owned().into(),
            params,
            session: session.cloned(),
            timeout: None,
        };
        Ok(self.request(request).await?.result)
    }

    pub fn send_nowait(
        &self,
        method: &str,
        params: serde_json::Value,
        session: Option<&SessionId>,
    ) -> crate::Result<WriteTicket> {
        let request = Request {
            method: method.to_owned().into(),
            params,
            session: session.cloned(),
            timeout: None,
        };
        Ok(self.inner.submit(request, false)?.ticket)
    }

    pub fn session(&self, id: Option<SessionId>) -> crate::session::Session {
        crate::session::Session::new(self.clone(), id)
    }

    pub fn events(&self) -> &crate::event_log::EventLog {
        &self.inner.events
    }

    pub fn route(&self, filter: RouteFilter) -> RouteReceiver {
        let (sender, receiver) = mpsc::unbounded_channel();
        let id = self.inner.next_route.fetch_add(1, Ordering::Relaxed);
        let mut routes = self.inner.routes();
        if !self.is_closed() {
            routes.push(Route { id, filter, sender });
        }
        RouteReceiver {
            receiver,
            id,
            connection: Arc::downgrade(&self.inner),
        }
    }

    pub fn add_hook(&self, hook: std::sync::Arc<dyn EventHook>) {
        let mut hooks = self.inner.hooks();
        if self.is_closed() {
            return;
        }
        let mut list: Vec<Arc<dyn EventHook>> = hooks.iter().cloned().collect();
        list.push(hook);
        *hooks = Arc::from(list);
    }

    pub fn session_info(&self, id: &SessionId) -> Option<SessionInfo> {
        self.inner.registry().sessions.get(id).cloned()
    }

    pub fn descendants(&self, id: &SessionId) -> Vec<SessionId> {
        self.inner.registry().descendants(id)
    }

    pub(crate) fn default_timeout(&self) -> std::time::Duration {
        self.inner.options.default_timeout
    }

    pub fn is_closed(&self) -> bool {
        self.inner.closed.borrow().is_some()
    }

    pub fn closed_reason(&self) -> Option<String> {
        self.inner.closed.borrow().clone()
    }

    pub async fn closed(&self) {
        let mut closed = self.inner.closed.subscribe();
        let _ = closed.wait_for(Option::is_some).await;
    }

    pub async fn close(&self) {
        self.inner.transport.close().await;
        self.inner.shutdown("closed by the client");
        self.inner.stop_reader();
    }

    pub fn abort(&self) {
        self.inner.shutdown("aborted");
        self.inner.stop_reader();
        self.inner.transport.abort();
    }
}

pub(crate) async fn with_op_deadline<F: std::future::Future>(
    deadline: tokio::time::Instant,
    future: F,
) -> F::Output {
    let deadline = OP_DEADLINE
        .try_with(|outer| (*outer).min(deadline))
        .unwrap_or(deadline);
    OP_DEADLINE.scope(deadline, future).await
}

/// For cleanup sent from `Drop` (input releases, `IO.close`, `Page.stopLoading`), which tokio runs
/// inside the scope of the op even after its deadline passed.
pub(crate) fn without_op_deadline<R>(send: impl FnOnce() -> R) -> R {
    OP_DEADLINE.sync_scope(Instant::now() + LIFTED_OP_DEADLINE, send)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transport::memory::{InMemoryControl, InMemoryTransport};
    use serde_json::json;
    use std::sync::atomic::AtomicUsize;

    fn connect() -> (Connection, InMemoryControl) {
        let (transport, control) = InMemoryTransport::new();
        (
            Connection::start(Box::new(transport), ConnectionOptions::default()),
            control,
        )
    }

    struct Probe {
        seen: AtomicUsize,
        detached_info_visible: Mutex<Option<bool>>,
    }

    impl EventHook for Probe {
        fn on_event(&self, connection: &Connection, event: &Event) {
            self.seen.fetch_add(1, Ordering::SeqCst);
            if &*event.method == "Target.detachedFromTarget" {
                let visible = connection.session_info(&SessionId::from("S2")).is_some();
                *lock(&self.detached_info_visible) = Some(visible);
                let _ = connection.send_nowait("Runtime.runIfWaitingForDebugger", json!({}), None);
            }
        }
    }

    async fn attach_page_and_child(connection: &Connection, control: &InMemoryControl) {
        control.emit(
            "Target.attachedToTarget",
            None,
            json!({"sessionId": "S1", "targetInfo": {"targetId": "T1", "type": "page"}}),
        );
        control.emit(
            "Target.attachedToTarget",
            Some("S1"),
            json!({"sessionId": "S2", "targetInfo": {"targetId": "F2", "type": "iframe"}}),
        );
        let cursor = connection.events().cursor();
        let deadline = Instant::now() + Duration::from_secs(5);
        connection
            .events()
            .wait_for(cursor, deadline, |event| {
                event.session.as_ref().is_some_and(|s| s.as_str() == "S1")
            })
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn detach_fails_pending_of_the_session_and_its_children() {
        let (connection, mut control) = connect();
        let probe = Arc::new(Probe {
            seen: AtomicUsize::new(0),
            detached_info_visible: Mutex::new(None),
        });
        connection.add_hook(probe.clone());
        attach_page_and_child(&connection, &control).await;
        assert_eq!(
            connection.descendants(&SessionId::from("S1")),
            vec![SessionId::from("S2")]
        );
        let child = connection
            .session(Some(SessionId::from("S2")))
            .enqueue("DOM.describeNode", json!({}), None)
            .unwrap();
        control.expect("DOM.describeNode").await;
        control.emit(
            "Target.detachedFromTarget",
            None,
            json!({"sessionId": "S1"}),
        );
        assert!(matches!(
            child.await,
            Err(BrowserError::TargetClosed { method }) if method == "DOM.describeNode"
        ));
        control.expect("Runtime.runIfWaitingForDebugger").await;
        assert_eq!(*lock(&probe.detached_info_visible), Some(true));
        assert!(connection.session_info(&SessionId::from("S2")).is_none());
        assert!(matches!(
            connection
                .send_raw("Page.enable", json!({}), Some(&SessionId::from("S2")))
                .await,
            Err(BrowserError::TargetClosed { .. })
        ));
    }

    #[tokio::test]
    async fn crashed_sessions_only_accept_the_allowlist() {
        let (connection, mut control) = connect();
        control.emit(
            "Target.attachedToTarget",
            None,
            json!({"sessionId": "S1", "targetInfo": {"targetId": "T1", "type": "page"}}),
        );
        control.emit("Inspector.targetCrashed", Some("S1"), json!({}));
        let cursor = EventCursor(1);
        let deadline = Instant::now() + Duration::from_secs(5);
        connection
            .events()
            .wait_for(cursor, deadline, |event| {
                &*event.method == "Inspector.targetCrashed"
            })
            .await
            .unwrap();
        let session = SessionId::from("S1");
        assert!(matches!(
            connection.send_nowait("Runtime.evaluate", json!({}), Some(&session)),
            Err(BrowserError::TargetCrashed { target_id }) if target_id == "T1"
        ));
        connection
            .send_nowait("Page.reload", json!({}), Some(&session))
            .unwrap();
        assert_eq!(control.next_command().await.method, "Page.reload");
    }

    fn page_session(index: usize) -> Value {
        json!({
            "sessionId": format!("S{index}"),
            "targetInfo": {"targetId": format!("T{index}"), "type": "page"},
        })
    }

    async fn attach_and_detach_pages(
        connection: &Connection,
        control: &InMemoryControl,
        count: usize,
    ) -> SessionId {
        let cursor = connection.events().cursor();
        for index in 0..count {
            control.emit("Target.attachedToTarget", None, page_session(index));
            control.emit("Target.detachedFromTarget", None, page_session(index));
        }
        let newest = format!("S{}", count - 1);
        connection
            .events()
            .wait_for(cursor, Instant::now() + Duration::from_secs(5), |event| {
                &*event.method == "Target.detachedFromTarget"
                    && event.params["sessionId"] == newest.as_str()
            })
            .await
            .unwrap()
            .expect("the last detach was dispatched");
        SessionId::from(newest)
    }

    #[tokio::test]
    async fn sessions_detached_before_the_dead_memory_fall_back_to_the_wire() {
        let (transport, mut control) = InMemoryTransport::new();
        let options = ConnectionOptions {
            event_log: EventLogLimits {
                max_events: 4 * DEAD_SESSION_MEMORY,
                ..EventLogLimits::default()
            },
            ..ConnectionOptions::default()
        };
        let connection = Connection::start(Box::new(transport), options);
        let newest = attach_and_detach_pages(&connection, &control, DEAD_SESSION_MEMORY + 1).await;
        assert!(matches!(
            connection.send_nowait("Runtime.evaluate", json!({}), Some(&newest)),
            Err(BrowserError::TargetClosed { .. })
        ));
        let oldest = connection
            .session(Some(SessionId::from("S0")))
            .enqueue("Runtime.evaluate", json!({}), None)
            .expect("an id older than the dead-session memory is sent to Chrome");
        let command = control.expect("Runtime.evaluate").await;
        assert_eq!(command.session.as_deref(), Some("S0"));
        control.reply_error(&command, -32001, "Session with given id not found.");
        let error = oldest.await.unwrap_err();
        assert_eq!(error.class(), crate::error::ErrorClass::SessionGone);
    }

    #[test]
    fn the_dead_session_memory_keeps_only_the_newest_ids() {
        let mut registry = Registry::default();
        let detached = DEAD_SESSION_MEMORY + 10;
        for index in 0..detached {
            registry.apply("Target.attachedToTarget", None, &page_session(index));
            let change = registry.apply("Target.detachedFromTarget", None, &page_session(index));
            for id in &change.remove {
                registry.sessions.remove(id);
            }
        }
        assert_eq!(registry.dead.ids.len(), DEAD_SESSION_MEMORY);
        assert_eq!(registry.dead.order.len(), DEAD_SESSION_MEMORY);
        assert!(!registry.dead.contains(&SessionId::from("S9")));
        assert!(registry.dead.contains(&SessionId::from("S10")));
        let newest = detached - 1;
        registry.apply("Target.attachedToTarget", None, &page_session(newest));
        let reattached = SessionId::from(format!("S{newest}"));
        assert!(!registry.dead.contains(&reattached));
        assert_eq!(registry.dead.order.len(), DEAD_SESSION_MEMORY - 1);
    }

    #[tokio::test]
    async fn enqueue_order_is_wire_order_and_deadlines_apply() {
        let (connection, mut control) = connect();
        let root = connection.session(None);
        let first = root.enqueue("A.one", json!({}), None).unwrap();
        let second = root
            .enqueue("A.two", json!({}), Some(Duration::from_millis(20)))
            .unwrap();
        root.send_nowait("A.three", json!({})).unwrap();
        for method in ["A.one", "A.two", "A.three"] {
            control.expect(method).await;
        }
        assert!(matches!(
            second.await,
            Err(BrowserError::Timeout { method, timeout_ms: 20 }) if method == "A.two"
        ));
        drop(first);
        let bounded = with_op_deadline(Instant::now() + Duration::from_millis(10), async {
            root.send("A.four", json!({})).await
        })
        .await;
        assert!(
            matches!(bounded, Err(BrowserError::Timeout { timeout_ms, .. }) if timeout_ms <= 10)
        );
        assert!(connection.inner.pending().is_empty());
    }

    #[tokio::test]
    async fn an_expired_op_deadline_refuses_commands_unless_lifted() {
        let (connection, mut control) = connect();
        let root = connection.session(None);
        let (refused, lifted) = with_op_deadline(Instant::now(), async {
            let release = json!({"type": "mouseReleased", "x": 1, "y": 1, "button": "left"});
            let refused = root.send_nowait("Input.dispatchMouseEvent", release);
            let lifted = without_op_deadline(|| {
                root.send_nowait("Input.dispatchKeyEvent", json!({"type": "keyUp"}))
            });
            (refused, lifted)
        })
        .await;
        assert!(matches!(
            refused,
            Err(BrowserError::Timeout { timeout_ms: 0, ref method }) if method == "Input.dispatchMouseEvent"
        ));
        assert!(lifted.is_ok());
        assert_eq!(
            control.next_command().await.method,
            "Input.dispatchKeyEvent"
        );
        assert!(control.try_next_command().is_none());
        assert!(connection.inner.pending().is_empty());
    }

    #[tokio::test]
    async fn nested_op_deadlines_keep_the_earliest() {
        let (connection, mut control) = connect();
        let root = connection.session(None);
        let outer = Instant::now() + Duration::from_millis(30);
        let bounded = with_op_deadline(outer, async {
            let inner = Instant::now() + Duration::from_secs(60);
            with_op_deadline(inner, root.send("A.slow", json!({}))).await
        })
        .await;
        assert!(matches!(
            bounded,
            Err(BrowserError::Timeout { timeout_ms, ref method }) if (1..=30).contains(&timeout_ms) && method == "A.slow"
        ));
        assert_eq!(control.next_command().await.method, "A.slow");
        let lifted = with_op_deadline(Instant::now(), async {
            without_op_deadline(|| bounded_by_op_deadline(Duration::from_secs(5)))
        })
        .await;
        assert_eq!(lifted, Duration::from_secs(5));
        assert_eq!(
            bounded_by_op_deadline(Duration::from_secs(5)),
            Duration::from_secs(5)
        );
    }

    struct PanicsOn(&'static str);

    impl EventHook for PanicsOn {
        fn on_event(&self, _connection: &Connection, event: &Event) {
            if &*event.method == self.0 {
                panic!("scripted hook failure on {}", self.0);
            }
        }
    }

    #[tokio::test]
    async fn a_panicking_hook_closes_the_connection() {
        let (connection, mut control) = connect();
        let hook = Arc::new(PanicsOn("Test.explode"));
        connection.add_hook(hook.clone());
        let pending = connection
            .session(None)
            .enqueue("Browser.getVersion", json!({}), None)
            .unwrap();
        control.expect("Browser.getVersion").await;
        control.emit("Test.explode", None, json!({}));
        assert!(matches!(
            pending.await,
            Err(BrowserError::Disconnected { reason }) if reason.contains("panicked")
        ));
        assert!(connection.is_closed());
        assert_eq!(Arc::strong_count(&hook), 1);
        assert!(matches!(
            connection.send_nowait("Browser.getVersion", json!({}), None),
            Err(BrowserError::Disconnected { .. })
        ));
    }

    #[tokio::test]
    async fn aborting_fails_pending_commands_and_stops_the_reader() {
        let (connection, mut control) = connect();
        let pending = connection
            .session(None)
            .enqueue("Browser.getVersion", json!({}), None)
            .unwrap();
        control.expect("Browser.getVersion").await;
        connection
            .send_nowait("Browser.close", json!({}), None)
            .unwrap();
        connection.abort();
        assert!(matches!(
            pending.await,
            Err(BrowserError::Disconnected { reason }) if reason == "aborted"
        ));
        assert_eq!(control.next_command().await.method, "Browser.close");
        assert!(lock(&connection.inner.reader).is_none());
        assert!(connection.inner.pending().is_empty());
        assert_eq!(connection.closed_reason().as_deref(), Some("aborted"));
    }

    #[tokio::test]
    async fn invalid_utf8_fails_only_its_command() {
        let (connection, mut control) = connect();
        let pending = connection
            .session(None)
            .enqueue("Page.getFrameTree", json!({}), None)
            .unwrap();
        let command = control.expect("Page.getFrameTree").await;
        let mut frame = format!("{{\"id\":{},\"result\":\"", command.id).into_bytes();
        frame.extend_from_slice(&[0xff, b'"', b'}']);
        control.send_invalid_utf8(&frame);
        assert!(matches!(
            pending.await,
            Err(BrowserError::Protocol { code: -32700, method, .. }) if method == "Page.getFrameTree"
        ));
        assert!(!connection.is_closed());
    }
}
