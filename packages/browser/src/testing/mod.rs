// Derived from rustwright src/lib.rs @fca1438, Copyright (c) 2026 Ikonomos Inc (dba Skyvern), MIT; modified by Rheosoph GmbH. See NOTICE.
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Weak};
use std::time::Duration;

use serde_json::{Value, json};

use crate::browser::{BrowserSettings, ConnectionKind};
use crate::connection::{Connection, ConnectionOptions, EventHook};
use crate::dialogs::Dialog;
use crate::element::Element;
use crate::event_log::Event;
use crate::frames::{DetachResolution, FrameTree, UTIL_WORLD};
use crate::page::{Frame, Page, PageInner};
use crate::protocol::{self, InboundMessage};
use crate::settle::{OpOutcome, OpSpec};
use crate::transport::memory::{InMemoryControl, InMemoryTransport, PeerSender, SentCommand};
use crate::types::{
    FrameId, FrameInfo, FrameTreeNode, SessionId, TargetId, TargetInfo, TargetType,
};

pub mod mock_server;

const TARGET: &str = "T1";
const SESSION: &str = "S1";
const LOADER: &str = "L1";
const URL: &str = "http://127.0.0.1/";
const ORIGIN: &str = "http://127.0.0.1";
const PAGE_LOAD_TIMEOUT: Duration = Duration::from_secs(30);
const BOOT_WAIT: Duration = Duration::from_secs(5);
const ISOLATED_WORLD_CONTEXT: i64 = 99;

pub struct PageHarness {
    pub page: Page,
    pub control: InMemoryControl,
    pub connection: Connection,
}

/// The op kinds of `Page::run_op` for scripted tests of the settle rules through `run_command`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OpClass {
    Read,
    FrameRead,
    Script,
    Input,
    Navigation,
    Output,
}

impl OpClass {
    fn spec(self) -> OpSpec {
        match self {
            Self::Read => OpSpec::READ,
            Self::FrameRead => OpSpec::FRAME_READ,
            Self::Script => OpSpec::SCRIPT,
            Self::Input => OpSpec::INPUT,
            Self::Navigation => OpSpec::NAVIGATION,
            Self::Output => OpSpec::OUTPUT,
        }
    }
}

#[derive(Debug)]
pub enum CommandOutcome {
    Done(Value),
    DialogOpened(Dialog),
}

/// Sends one command on the owning session of the attempt frame inside `Page::run_op`, so tests
/// see the pre-wait, retries, frame fallback, dialog race and post-wait of a real op. `Input`
/// commits the attempt once the command is written, as the input dispatch does.
pub async fn run_command(
    page: &Page,
    frame: &str,
    class: OpClass,
    method: &str,
    params: Value,
) -> crate::Result<CommandOutcome> {
    let outcome = page
        .run_op(&FrameId::from(frame), class.spec(), |attempt| {
            let params = params.clone();
            async move {
                let frame = Frame {
                    page: page.clone(),
                    id: attempt.frame.clone(),
                };
                let reply = frame.session()?.enqueue(method, params, None)?;
                if class == OpClass::Input {
                    attempt.commit();
                }
                Ok(reply.await?.result)
            }
        })
        .await?;
    Ok(match outcome {
        OpOutcome::Done(value) => CommandOutcome::Done(value),
        OpOutcome::DialogOpened(dialog) => CommandOutcome::DialogOpened(dialog),
    })
}

/// An element of the current document of the frame, as a find in that frame would mint it.
pub fn element(page: &Page, frame: &str, backend_node_id: i64) -> crate::Result<Element> {
    let frame = Frame {
        page: page.clone(),
        id: FrameId::from(frame),
    };
    let stamp = frame.stamp()?;
    Ok(Element::new(frame, &stamp, backend_node_id, None))
}

struct PageForwarder(Weak<PageInner>);

impl PageForwarder {
    fn page_session_gone(page: &Page, event: &Event) -> bool {
        let page_session = page.inner.session.as_str();
        match &*event.method {
            "Target.detachedFromTarget" => {
                event.session.is_none() && event.params["sessionId"] == page_session
            }
            "Inspector.detached" => event.session.as_ref() == Some(&page.inner.session),
            _ => false,
        }
    }
}

impl EventHook for PageForwarder {
    fn on_event(&self, _connection: &Connection, event: &Event) {
        let Some(inner) = self.0.upgrade() else {
            return;
        };
        let page = Page { inner };
        let gone = Self::page_session_gone(&page, event);
        let owned = event
            .session
            .as_ref()
            .is_some_and(|session| page.owns_session(session));
        if owned || gone {
            page.on_event(event);
        }
        if gone {
            page.mark_closed();
        }
    }
}

fn context_params(frame: &str, id: i64, unique_id: &str, util: bool) -> Value {
    json!({"context": {
        "id": id,
        "origin": ORIGIN,
        "name": if util { UTIL_WORLD } else { "" },
        "uniqueId": unique_id,
        "auxData": {"isDefault": !util, "type": if util { "isolated" } else { "default" }, "frameId": frame},
    }})
}

fn frame_json(frames: &FrameTree, id: &FrameId) -> Option<Value> {
    let node = frames.get(id)?;
    let children: Vec<Value> = node
        .children
        .iter()
        .filter(|child| {
            frames
                .get(child)
                .is_some_and(|child| child.session.is_none())
        })
        .filter_map(|child| frame_json(frames, child))
        .collect();
    Some(json!({
        "frame": {
            "id": node.id,
            "parentId": node.parent,
            "loaderId": node.loader_id.clone().unwrap_or_default(),
            "url": node.url,
            "urlFragment": node.url_fragment,
            "name": node.name,
        },
        "childFrames": children,
    }))
}

fn frame_tree_of(page: &Page, session: &SessionId) -> Option<Value> {
    let state = page.inner.lock_state();
    let frames = &state.frames;
    let main = frames.main_id().clone();
    let root = std::iter::once(main.clone())
        .chain(frames.descendants_preorder(&main))
        .find(|id| {
            frames
                .get(id)
                .is_some_and(|node| node.session.as_ref() == Some(session))
        })?;
    frame_json(frames, &root)
}

fn harness_reply(
    page: Weak<PageInner>,
) -> impl Fn(&SentCommand) -> Option<Value> + Send + Sync + 'static {
    move |command| {
        if command.method != "Page.getFrameTree" {
            return default_auto_reply(command);
        }
        let page = Page {
            inner: page.upgrade()?,
        };
        let session = SessionId::from(command.session.as_deref()?);
        Some(json!({"frameTree": frame_tree_of(&page, &session)?}))
    }
}

fn isolated_world_announcer(
    peer: PeerSender,
) -> impl Fn(&SentCommand, &Value) + Send + Sync + 'static {
    let worlds = AtomicU64::new(0);
    move |command, reply| {
        if command.method != "Page.createIsolatedWorld" {
            return;
        }
        let (Some(frame), Some(session), Some(id)) = (
            command.params["frameId"].as_str(),
            command.session.as_deref(),
            reply["executionContextId"].as_i64(),
        ) else {
            return;
        };
        let world = worlds.fetch_add(1, Ordering::Relaxed) + 1;
        let unique_id = format!("{session}-isolated-{world}");
        let params = context_params(frame, id, &unique_id, true);
        peer.emit("Runtime.executionContextCreated", Some(session), params);
    }
}

fn harness_page(connection: &Connection) -> Page {
    let settings = Arc::new(BrowserSettings::new(
        ConnectionKind::Direct,
        true,
        PAGE_LOAD_TIMEOUT,
    ));
    let inner = PageInner::new(
        connection.clone(),
        Weak::new(),
        settings,
        TargetId::from(TARGET),
        SessionId::from(SESSION),
        FrameId::from(TARGET),
    );
    let page = Page {
        inner: Arc::new(inner),
    };
    let main = FrameTreeNode {
        frame: FrameInfo {
            id: TARGET.into(),
            loader_id: LOADER.into(),
            url: URL.into(),
            ..FrameInfo::default()
        },
        child_frames: Vec::new(),
    };
    let cursor = connection.events().cursor();
    page.inner
        .lock_state()
        .frames
        .seed(&SessionId::from(SESSION), &main, cursor);
    page
}

impl PageHarness {
    pub async fn new() -> PageHarness {
        let (transport, control) = InMemoryTransport::new();
        let connection = Connection::start(Box::new(transport), ConnectionOptions::default());
        let page = harness_page(&connection);
        connection.add_hook(Arc::new(PageForwarder(Arc::downgrade(&page.inner))));
        control.set_auto_reply(harness_reply(Arc::downgrade(&page.inner)));
        control.set_reply_announcer(Arc::new(isolated_world_announcer(control.peer.clone())));
        let harness = PageHarness {
            page,
            control,
            connection,
        };
        harness.announce_page();
        harness.wait_until_booted().await;
        harness
    }

    fn announce_page(&self) {
        let page_target = json!({"targetId": TARGET, "type": "page", "url": URL, "attached": true});
        self.control.emit(
            "Target.attachedToTarget",
            None,
            json!({"sessionId": SESSION, "targetInfo": page_target, "waitingForDebugger": false}),
        );
        self.context_created(SESSION, TARGET, 1, "S1-main", false);
        self.context_created(SESSION, TARGET, 2, "S1-util", true);
    }

    async fn wait_until_booted(&self) {
        let deadline = tokio::time::Instant::now() + BOOT_WAIT;
        let frame = FrameId::from(TARGET);
        let session = SessionId::from(SESSION);
        let mut version = self.page.inner.version.subscribe();
        loop {
            let booted = {
                let state = self.page.inner.lock_state();
                state.contexts.main(&frame, &session).is_some()
                    && state.contexts.util(&frame, &session).is_some()
            };
            if booted && self.connection.session_info(&session).is_some() {
                return;
            }
            let woke = tokio::time::timeout_at(deadline, version.changed()).await;
            assert!(
                matches!(woke, Ok(Ok(()))),
                "PageHarness: page {TARGET} never saw its default and util contexts on {SESSION}"
            );
        }
    }

    pub fn emit(&self, method: &str, params: serde_json::Value) {
        self.control.emit(method, Some(SESSION), params);
    }

    pub fn emit_on(&self, session: &str, method: &str, params: serde_json::Value) {
        self.control.emit(method, Some(session), params);
    }

    /// The first command `wanted` accepts, auto-replied ones included, once it was sent.
    pub async fn sent(&self, what: &str, wanted: impl Fn(&SentCommand) -> bool) -> SentCommand {
        let deadline = tokio::time::Instant::now() + BOOT_WAIT;
        loop {
            let seen = self.control.commands_seen().into_iter().find(&wanted);
            if let Some(command) = seen {
                return command;
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "PageHarness: {what} was never sent"
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }

    pub fn context_created(
        &self,
        session: &str,
        frame: &str,
        id: i64,
        unique_id: &str,
        util: bool,
    ) {
        self.control.emit(
            "Runtime.executionContextCreated",
            Some(session),
            context_params(frame, id, unique_id, util),
        );
    }

    pub fn attach_child(
        &self,
        child_session: &str,
        frame: &str,
        parent_frame: &str,
        url: &str,
        loader: &str,
    ) {
        let parent_session = self
            .page
            .inner
            .lock_state()
            .frames
            .session_for_frame(&FrameId::from(parent_frame))
            .unwrap_or_else(|_| SessionId::from(SESSION));
        let target_info = json!({"targetId": frame, "type": "iframe", "url": url, "attached": true, "parentFrameId": parent_frame});
        self.control.emit(
            "Target.attachedToTarget",
            Some(parent_session.as_str()),
            json!({"sessionId": child_session, "targetInfo": target_info, "waitingForDebugger": true}),
        );
        let info = TargetInfo {
            target_id: frame.into(),
            type_: TargetType::Iframe,
            url: url.into(),
            attached: true,
            parent_frame_id: Some(parent_frame.into()),
            ..TargetInfo::default()
        };
        self.register_child(&parent_session, SessionId::from(child_session), &info);
        self.load_child_document(child_session, frame, parent_frame, url, loader);
    }

    pub fn set_page_load_timeout(&self, timeout: Duration) {
        self.page.inner.settings.set_page_load_timeout(timeout);
    }

    /// Emits `Target.detachedFromTarget` for the child on its parent session and applies the
    /// side effects of the target manager, leaving the frames of a detach-first swap in transit
    /// until the parent re-reports them (`Page.frameAttached`).
    pub fn detach_child(&self, child_session: &str) {
        self.emit_child_detached(child_session);
        self.park_child_frames(child_session);
    }

    /// Emits only `Target.detachedFromTarget` for the child: the connection fails its pending
    /// commands while the frames still name the session, as it does before the target manager
    /// hook runs. `park_child_frames` applies the rest.
    pub fn emit_child_detached(&self, child_session: &str) {
        let info = self
            .connection
            .session_info(&SessionId::from(child_session));
        let parent = info
            .as_ref()
            .and_then(|info| info.parent.clone())
            .unwrap_or_else(|| SessionId::from(SESSION));
        let target = info.map(|info| info.target.target_id.to_string());
        self.control.emit(
            "Target.detachedFromTarget",
            Some(parent.as_str()),
            json!({"sessionId": child_session, "targetId": target}),
        );
    }

    /// The target manager side effects of a child detach that needs a round trip: the frames of
    /// the session stay in transit until the parent re-reports them.
    pub fn park_child_frames(&self, child_session: &str) {
        let child = SessionId::from(child_session);
        {
            let mut state = self.page.inner.lock_state();
            if state.frames.on_child_session_detached(&child) == DetachResolution::NeedsRoundTrip {
                let rooted = state.frames.frames_of_session(&child);
                state.navigation.reset_frames(&rooted);
            }
        }
        self.page.inner.bump_version();
    }

    fn register_child(&self, parent_session: &SessionId, child: SessionId, info: &TargetInfo) {
        let seq = self.connection.events().next_seq();
        let frame = FrameId::new(info.target_id.as_str());
        {
            let mut state = self.page.inner.lock_state();
            state
                .frames
                .on_child_session_attached(parent_session, &child, info, seq);
            state.contexts.forget_frame(&frame, &child);
        }
        self.page.add_session(child);
    }

    fn load_child_document(
        &self,
        child_session: &str,
        frame: &str,
        parent_frame: &str,
        url: &str,
        loader: &str,
    ) {
        let navigated = json!({
            "frame": {"id": frame, "parentId": parent_frame, "loaderId": loader, "url": url},
            "type": "Navigation",
        });
        self.apply(child_session, "Page.frameNavigated", navigated);
        let main_context = context_params(frame, 1, &format!("{child_session}-main"), false);
        self.apply(
            child_session,
            "Runtime.executionContextCreated",
            main_context,
        );
        let util_context = context_params(frame, 2, &format!("{child_session}-util"), true);
        self.apply(
            child_session,
            "Runtime.executionContextCreated",
            util_context,
        );
    }

    fn apply(&self, session: &str, method: &str, params: Value) {
        let event = Event {
            seq: self.connection.events().next_seq(),
            method: method.into(),
            session: Some(SessionId::from(session)),
            params: Arc::new(params),
            received: tokio::time::Instant::now(),
        };
        self.page.on_event(&event);
    }
}

pub fn default_auto_reply(command: &SentCommand) -> Option<serde_json::Value> {
    let method = command.method.as_str();
    match method {
        "Runtime.evaluate" if command.params["expression"] == "1" => {
            Some(json!({"result": {"type": "number", "value": 1, "description": "1"}}))
        }
        "Page.addScriptToEvaluateOnNewDocument" => Some(json!({"identifier": "1"})),
        "Page.createIsolatedWorld" => Some(json!({"executionContextId": ISOLATED_WORLD_CONTEXT})),
        "Page.setLifecycleEventsEnabled"
        | "Target.setAutoAttach"
        | "Runtime.runIfWaitingForDebugger"
        | "Emulation.setFocusEmulationEnabled"
        | "Runtime.releaseObjectGroup" => Some(json!({})),
        _ if method.ends_with(".enable") => Some(json!({})),
        _ => None,
    }
}

pub enum RecordedFrame {
    Response {
        id: u64,
        session: Option<String>,
        result: serde_json::Value,
    },
    Event {
        method: String,
        session: Option<String>,
        params: serde_json::Value,
    },
    Malformed {
        id: Option<u64>,
    },
}

pub fn parse_recorded_frame(text: &str) -> RecordedFrame {
    match protocol::parse_inbound(text) {
        InboundMessage::Response {
            id,
            session,
            result,
        } => RecordedFrame::Response {
            id,
            session: session.map(|session| session.to_string()),
            result: result.unwrap_or_else(
                |error| json!({"error": {"code": error.code, "message": error.message}}),
            ),
        },
        InboundMessage::Event {
            method,
            session,
            params,
        } => RecordedFrame::Event {
            method: method.to_string(),
            session: session.map(|session| session.to_string()),
            params,
        },
        InboundMessage::Malformed { id, .. } => RecordedFrame::Malformed { id },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::BrowserError;
    use crate::types::LoaderId;

    async fn wait_for_session_info(harness: &PageHarness, session: &str) {
        let deadline = tokio::time::Instant::now() + BOOT_WAIT;
        let cursor = crate::event_log::EventCursor(1);
        let found = harness
            .connection
            .events()
            .wait_for(cursor, deadline, |event| {
                &*event.method == "Target.attachedToTarget" && event.params["sessionId"] == session
            })
            .await
            .unwrap();
        assert!(
            found.is_some(),
            "Target.attachedToTarget for {session} was never processed"
        );
    }

    #[tokio::test]
    async fn harness_boots_with_the_main_frame_and_both_worlds() {
        let harness = PageHarness::new().await;
        let frame = harness.page.main_frame();
        assert!(frame.is_main());
        let stamp = frame.stamp().unwrap();
        assert_eq!(stamp.loader, LoaderId::from(LOADER));
        assert_eq!(stamp.local_root, TargetId::from(TARGET));
        assert_eq!(stamp.session, SessionId::from(SESSION));
        assert_eq!(harness.page.session().id(), Some(&SessionId::from(SESSION)));
        assert_eq!(harness.page.main_loader(), Some(LoaderId::from(LOADER)));
        let state = harness.page.inner.lock_state();
        assert_eq!(state.frames.main_url(), URL);
        assert_eq!(
            state.contexts.main(frame.id(), &stamp.session).unwrap().id,
            1
        );
        assert_eq!(
            state.contexts.util(frame.id(), &stamp.session).unwrap().id,
            2
        );
    }

    #[tokio::test]
    async fn default_auto_reply_answers_setup_commands() {
        let harness = PageHarness::new().await;
        let session = harness.page.session();
        for method in [
            "Page.enable",
            "Runtime.enable",
            "Target.setAutoAttach",
            "Runtime.runIfWaitingForDebugger",
        ] {
            assert_eq!(
                session.send(method, json!({})).await.unwrap(),
                json!({}),
                "{method}"
            );
        }
        let flushed = session
            .send("Runtime.evaluate", json!({"expression": "1"}))
            .await
            .unwrap();
        assert_eq!(flushed["result"]["value"], 1);
        let script = session
            .send(
                "Page.addScriptToEvaluateOnNewDocument",
                json!({"source": ""}),
            )
            .await
            .unwrap();
        assert_eq!(script["identifier"], "1");
        assert!(
            default_auto_reply(&SentCommand {
                id: 1,
                method: "DOM.describeNode".into(),
                session: None,
                params: json!({}),
            })
            .is_none()
        );
    }

    #[tokio::test]
    async fn a_test_can_withhold_a_default_reply() {
        let mut harness = PageHarness::new().await;
        harness.control.set_auto_reply(|command| {
            let flush = command.method == "Runtime.evaluate" && command.params["expression"] == "1";
            if flush {
                None
            } else {
                default_auto_reply(command)
            }
        });
        let session = harness.page.session();
        assert_eq!(
            session.send("Page.enable", json!({})).await.unwrap(),
            json!({})
        );
        let flush = tokio::spawn({
            let session = session.clone();
            async move {
                session
                    .send("Runtime.evaluate", json!({"expression": "1"}))
                    .await
            }
        });
        let held = harness
            .control
            .wait_for("Runtime.evaluate", Some(SESSION))
            .await;
        harness
            .control
            .reply_error(&held, -32000, "Execution context was destroyed.");
        assert!(matches!(
            flush.await.unwrap(),
            Err(BrowserError::Protocol { code: -32000, ref method, .. }) if method == "Runtime.evaluate"
        ));
    }

    #[tokio::test]
    async fn attach_child_registers_the_child_session() {
        let harness = PageHarness::new().await;
        harness.attach_child("S2", "F2", TARGET, "http://127.0.0.1/child", "L2");
        let child = SessionId::from("S2");
        assert!(harness.page.owns_session(&child));
        let frame = harness
            .page
            .frame(&FrameId::from("F2"))
            .expect("the child frame is registered");
        let stamp = frame.stamp().unwrap();
        assert_eq!(stamp.session, child);
        assert_eq!(stamp.loader, LoaderId::from("L2"));
        assert_eq!(stamp.local_root, TargetId::from("F2"));
        assert_eq!(frame.session().unwrap().id(), Some(&child));
        {
            let state = harness.page.inner.lock_state();
            assert_eq!(
                &*state.contexts.util(frame.id(), &child).unwrap().unique_id,
                "S2-util"
            );
            assert!(
                state
                    .contexts
                    .main(frame.id(), &SessionId::from(SESSION))
                    .is_none()
            );
            assert!(
                state
                    .contexts
                    .main(&FrameId::from(TARGET), &SessionId::from(SESSION))
                    .is_some()
            );
        }
        wait_for_session_info(&harness, "S2").await;
        let info = harness.connection.session_info(&child).unwrap();
        assert_eq!(info.parent, Some(SessionId::from(SESSION)));
        assert_eq!(
            harness.connection.descendants(&SessionId::from(SESSION)),
            vec![child]
        );
    }

    #[tokio::test]
    async fn frame_tree_replies_stop_at_child_local_roots() {
        let harness = PageHarness::new().await;
        harness.attach_child("S2", "F2", TARGET, "http://127.0.0.1/child", "L2");
        let tree = harness
            .page
            .session()
            .send("Page.getFrameTree", json!({}))
            .await
            .unwrap();
        assert_eq!(tree["frameTree"]["frame"]["id"], TARGET);
        assert_eq!(tree["frameTree"]["childFrames"], json!([]));
        let child_tree = harness
            .connection
            .session(Some(SessionId::from("S2")))
            .send("Page.getFrameTree", json!({}))
            .await
            .unwrap();
        assert_eq!(child_tree["frameTree"]["frame"]["id"], "F2");
        assert_eq!(child_tree["frameTree"]["frame"]["loaderId"], "L2");
    }

    #[tokio::test]
    async fn isolated_worlds_announce_a_util_context() {
        let harness = PageHarness::new().await;
        harness.control.set_auto_reply(default_auto_reply);
        harness.attach_child("S2", "F2", TARGET, "http://127.0.0.1/child", "L2");
        let child = SessionId::from("S2");
        harness
            .page
            .inner
            .lock_state()
            .contexts
            .forget_frame(&FrameId::from("F2"), &SessionId::from("none"));
        let reply = harness
            .connection
            .session(Some(child.clone()))
            .send(
                "Page.createIsolatedWorld",
                json!({"frameId": "F2", "worldName": UTIL_WORLD}),
            )
            .await
            .unwrap();
        assert_eq!(reply["executionContextId"], ISOLATED_WORLD_CONTEXT);
        let deadline = tokio::time::Instant::now() + BOOT_WAIT;
        let frame = FrameId::from("F2");
        let util = harness
            .page
            .wait_for_state(deadline, |state| state.contexts.util(&frame, &child))
            .await
            .expect("the isolated world never arrived");
        assert_eq!(util.id, ISOLATED_WORLD_CONTEXT);
    }

    #[test]
    fn recorded_frames_parse_through_the_protocol_parser() {
        assert!(matches!(
            parse_recorded_frame(r#"{"id":3,"sessionId":"S1","result":{"a":1}}"#),
            RecordedFrame::Response { id: 3, session: Some(ref session), ref result } if session == "S1" && result["a"] == 1
        ));
        assert!(matches!(
            parse_recorded_frame(r#"{"method":"Page.loadEventFired","params":{}}"#),
            RecordedFrame::Event { ref method, session: None, .. } if method == "Page.loadEventFired"
        ));
        assert!(matches!(
            parse_recorded_frame(r#"{"id":4,"error":{"code":-32000,"message":"x"}}"#),
            RecordedFrame::Response { id: 4, ref result, .. } if result["error"]["code"] == -32000
        ));
        assert!(matches!(
            parse_recorded_frame("{\"id\":9,"),
            RecordedFrame::Malformed { id: Some(9) }
        ));
    }
}
