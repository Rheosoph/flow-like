use std::collections::VecDeque;
use std::sync::{MutexGuard, PoisonError};
use std::time::Duration;

use serde_json::{Value, json};

use crate::connection::Connection;
use crate::element::javascript_error;
use crate::error::BrowserError;
use crate::event_log::Event;
use crate::frames::{FrameLookup, FrameTree};
use crate::protocol::decode;
use crate::settle::OpSpec;
use crate::types::{DescribedNode, FrameId, FrameNavigated, LoaderId, SessionId, TargetId};

const CONSOLE_MAX_EVENTS: usize = 1_000;
const CONSOLE_MAX_BYTES: usize = 4 * 1024 * 1024;
const CHILD_FRAME_WAIT: Duration = Duration::from_millis(500);
const REPLAY_TIMEOUT: Duration = Duration::from_secs(2);

#[derive(Clone)]
pub struct Page {
    pub(crate) inner: std::sync::Arc<PageInner>,
}

pub(crate) struct PageInner {
    pub(crate) connection: Connection,
    pub(crate) browser: std::sync::Weak<crate::browser::BrowserInner>,
    pub(crate) settings: std::sync::Arc<crate::browser::BrowserSettings>,
    pub(crate) target_id: TargetId,
    pub(crate) session: SessionId,
    pub(crate) state: std::sync::Mutex<PageState>,
    pub(crate) version: tokio::sync::watch::Sender<u64>,
    pub(crate) dialogs: tokio::sync::watch::Sender<u64>,
}

pub(crate) struct PageState {
    pub(crate) frames: crate::frames::FrameTree,
    pub(crate) contexts: crate::frames::ExecutionContexts,
    pub(crate) navigation: crate::navigation::NavigationTracker,
    pub(crate) dialog: crate::dialogs::DialogState,
    pub(crate) emulation: crate::emulation::EmulationState,
    pub(crate) input: crate::input::InputState,
    pub(crate) fetch: crate::fetch::FetchState,
    pub(crate) console: ConsoleBacklog,
    pub(crate) sessions: Vec<SessionId>,
    pub(crate) crashed: bool,
    pub(crate) closed: bool,
}

pub(crate) enum PageEffect {
    SendNowait {
        session: SessionId,
        method: &'static str,
        params: serde_json::Value,
    },
}

struct ConsoleEntry {
    event: Event,
    bytes: usize,
}

pub(crate) struct ConsoleBacklog {
    page_session: SessionId,
    main_frame: FrameId,
    entries: VecDeque<ConsoleEntry>,
    bytes: usize,
}

impl ConsoleBacklog {
    pub(crate) fn new(page_session: SessionId, main_frame: FrameId) -> Self {
        Self {
            page_session,
            main_frame,
            entries: VecDeque::new(),
            bytes: 0,
        }
    }

    pub(crate) fn on_event(&mut self, event: &Event) {
        if event.session.as_ref() != Some(&self.page_session) {
            return;
        }
        match &*event.method {
            "Runtime.consoleAPICalled" | "Runtime.exceptionThrown" => self.record(event),
            "Page.frameNavigated" => {
                let main_committed = event
                    .decode::<FrameNavigated>()
                    .is_some_and(|navigated| navigated.frame.id == self.main_frame);
                if main_committed {
                    self.entries.clear();
                    self.bytes = 0;
                }
            }
            _ => {}
        }
    }

    fn record(&mut self, event: &Event) {
        let bytes = event.params.to_string().len();
        self.entries.push_back(ConsoleEntry {
            event: event.clone(),
            bytes,
        });
        self.bytes += bytes;
        while self.entries.len() > CONSOLE_MAX_EVENTS || self.bytes > CONSOLE_MAX_BYTES {
            let Some(evicted) = self.entries.pop_front() else {
                break;
            };
            self.bytes -= evicted.bytes;
        }
    }

    pub(crate) fn snapshot(&self) -> Vec<Event> {
        self.entries
            .iter()
            .map(|entry| entry.event.clone())
            .collect()
    }
}

#[derive(Clone)]
pub struct Frame {
    pub(crate) page: Page,
    pub(crate) id: FrameId,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct FrameStamp {
    pub loader: LoaderId,
    pub local_root: TargetId,
    pub session: SessionId,
}

impl PageInner {
    pub(crate) fn new(
        connection: Connection,
        browser: std::sync::Weak<crate::browser::BrowserInner>,
        settings: std::sync::Arc<crate::browser::BrowserSettings>,
        target_id: TargetId,
        session: SessionId,
        main_frame: FrameId,
    ) -> Self {
        let state = PageState {
            frames: crate::frames::FrameTree::new(main_frame.clone(), session.clone()),
            contexts: crate::frames::ExecutionContexts::new(),
            navigation: crate::navigation::NavigationTracker::new(),
            dialog: crate::dialogs::DialogState::new(),
            emulation: crate::emulation::EmulationState::new(),
            input: crate::input::InputState::new(),
            fetch: crate::fetch::FetchState::new(),
            console: ConsoleBacklog::new(session.clone(), main_frame),
            sessions: vec![session.clone()],
            crashed: false,
            closed: false,
        };
        Self {
            connection,
            browser,
            settings,
            target_id,
            session,
            state: std::sync::Mutex::new(state),
            version: tokio::sync::watch::Sender::new(0),
            dialogs: tokio::sync::watch::Sender::new(0),
        }
    }

    pub(crate) fn lock_state(&self) -> MutexGuard<'_, PageState> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub(crate) fn bump_version(&self) {
        self.version.send_modify(|version| *version += 1);
    }
}

struct Applied {
    changed: bool,
    dialog: Option<u64>,
    effects: Vec<PageEffect>,
}

impl Page {
    pub fn target_id(&self) -> &TargetId {
        &self.inner.target_id
    }

    pub fn session(&self) -> crate::session::Session {
        self.inner
            .connection
            .session(Some(self.inner.session.clone()))
    }

    pub fn browser(&self) -> Option<crate::browser::Browser> {
        self.inner
            .browser
            .upgrade()
            .map(|inner| crate::browser::Browser { inner })
    }

    pub fn main_frame(&self) -> Frame {
        let id = self.inner.lock_state().frames.main_id().clone();
        Frame {
            page: self.clone(),
            id,
        }
    }

    pub fn frame(&self, id: &FrameId) -> Option<Frame> {
        let known = self.inner.lock_state().frames.get(id).is_some();
        known.then(|| Frame {
            page: self.clone(),
            id: id.clone(),
        })
    }

    pub fn is_closed(&self) -> bool {
        self.inner.lock_state().closed
    }

    pub fn pending_dialog(&self) -> Option<crate::dialogs::Dialog> {
        self.inner.lock_state().dialog.open().cloned()
    }

    pub async fn url(&self) -> crate::Result<String> {
        let main = self.main_frame_id();
        self.run_op(&main, OpSpec::READ, |_| self.url_in())
            .await?
            .read()
    }

    /// Chrome shows network, certificate and blocked-URL failures on an internal error page;
    /// like chromedriver, such a page reports the URL its history entry was opened for.
    async fn url_in(&self) -> crate::Result<String> {
        let url = self.main_url();
        if !is_error_page(&url) {
            return Ok(url);
        }
        const METHOD: &str = "Page.getNavigationHistory";
        let history = self.session().send(METHOD, json!({})).await?;
        current_history_url(&history)
            .ok_or_else(|| crate::input::unexpected_reply(METHOD, "current entry url", &history))
    }

    pub async fn title(&self) -> crate::Result<String> {
        let main = self.main_frame_id();
        self.run_op(&main, OpSpec::READ, |_| self.title_in())
            .await?
            .read()
    }

    pub async fn cdp(
        &self,
        method: &str,
        params: serde_json::Value,
    ) -> crate::Result<serde_json::Value> {
        let main = self.main_frame_id();
        self.run_op(&main, OpSpec::READ, |_| self.cdp_in(method, &params))
            .await?
            .read()
    }

    async fn title_in(&self) -> crate::Result<String> {
        let reply = self
            .session()
            .send(
                "Runtime.evaluate",
                json!({"expression": "document.title", "returnByValue": true}),
            )
            .await?;
        if let Some(details) = reply.get("exceptionDetails") {
            return Err(javascript_error(details));
        }
        Ok(reply["result"]["value"]
            .as_str()
            .unwrap_or_default()
            .to_owned())
    }

    async fn cdp_in(&self, method: &str, params: &Value) -> crate::Result<Value> {
        if method == "Page.captureScreenshot" {
            self.activate_for_capture().await?;
        }
        let result = self.session().send(method, params.clone()).await?;
        if crate::emulation::reaches_iframes(method) {
            self.replay_on_iframes(method, params).await;
        }
        Ok(result)
    }

    async fn replay_on_iframes(&self, method: &str, params: &Value) {
        let sessions = {
            let mut state = self.inner.lock_state();
            state.emulation.record(method, params);
            iframe_sessions(&state.frames, &self.inner.session)
        };
        let sends = sessions.into_iter().map(|session| async move {
            let result = self
                .inner
                .connection
                .session(Some(session.clone()))
                .send_with_timeout(method, params.clone(), REPLAY_TIMEOUT)
                .await;
            (session, result)
        });
        for (session, result) in futures_util::future::join_all(sends).await {
            if let Err(error) = result {
                tracing::debug!(method, %session, %error, "emulation replay on an iframe session failed");
            }
        }
    }

    pub(crate) fn main_frame_id(&self) -> FrameId {
        self.inner.lock_state().frames.main_id().clone()
    }

    pub(crate) fn main_url(&self) -> String {
        self.inner.lock_state().frames.main_url()
    }

    pub(crate) fn page_load_timeout(&self) -> std::time::Duration {
        self.inner.settings.page_load_timeout()
    }

    pub(crate) fn has_oopifs(&self) -> bool {
        !iframe_sessions(&self.inner.lock_state().frames, &self.inner.session).is_empty()
    }

    #[cfg(any(test, feature = "test-support"))]
    pub(crate) fn owns_session(&self, session: &SessionId) -> bool {
        self.inner.lock_state().sessions.contains(session)
    }

    pub(crate) fn add_session(&self, session: SessionId) {
        let mut state = self.inner.lock_state();
        if !state.sessions.contains(&session) {
            state.sessions.push(session);
        }
    }

    pub(crate) fn mark_closed(&self) {
        let changed = {
            let mut state = self.inner.lock_state();
            !std::mem::replace(&mut state.closed, true)
        };
        if changed {
            self.inner.bump_version();
        }
    }

    pub(crate) fn on_event(&self, event: &crate::event_log::Event) {
        let applied = self.apply(event);
        if applied.changed {
            self.inner.bump_version();
        }
        if let Some(ordinal) = applied.dialog {
            self.inner.dialogs.send_if_modified(|count| {
                let raised = *count < ordinal;
                if raised {
                    *count = ordinal;
                }
                raised
            });
        }
        for effect in applied.effects {
            let PageEffect::SendNowait {
                session,
                method,
                params,
            } = effect;
            if let Err(error) = self
                .inner
                .connection
                .send_nowait(method, params, Some(&session))
            {
                tracing::debug!(method, %error, "page effect could not be sent");
            }
        }
    }

    fn apply(&self, event: &Event) -> Applied {
        let mut state = self.inner.lock_state();
        let state = &mut *state;
        let now = tokio::time::Instant::now();
        let swap_back = swap_back_candidate(&state.frames, event);
        let mut changed = state.frames.on_event(event);
        if let (Some(frame), Some(session)) = (swap_back, event.session.as_ref())
            && state
                .frames
                .get(&frame)
                .is_some_and(|node| node.session.is_none())
        {
            state.contexts.forget_frame(&frame, session);
        }
        changed |= state.contexts.on_event(event);
        changed |= state.navigation.on_event(event, &state.frames, now);
        let dialog_changed = state.dialog.on_event(event);
        changed |= dialog_changed;
        let dialog = (dialog_changed && &*event.method == "Page.javascriptDialogOpening")
            .then(|| state.dialog.open().map(|dialog| dialog.seq))
            .flatten();
        state.console.on_event(event);
        let effects = if &*event.method == "Page.javascriptDialogClosed" {
            state.input.on_dialog_closed()
        } else {
            Vec::new()
        };
        changed |= self.apply_session_changes(state, event);
        Applied {
            changed,
            dialog,
            effects,
        }
    }

    fn apply_session_changes(&self, state: &mut PageState, event: &Event) -> bool {
        let on_page_session = event.session.as_ref() == Some(&self.inner.session);
        match &*event.method {
            "Target.detachedFromTarget" => {
                let Some(detached) = event.params["sessionId"].as_str() else {
                    return false;
                };
                let before = state.sessions.len();
                state.sessions.retain(|session| {
                    session.as_str() != detached || *session == self.inner.session
                });
                before != state.sessions.len()
            }
            "Inspector.targetCrashed" if on_page_session => {
                !std::mem::replace(&mut state.crashed, true)
            }
            "Inspector.targetReloadedAfterCrash" if on_page_session => {
                std::mem::replace(&mut state.crashed, false)
            }
            _ => false,
        }
    }

    pub(crate) fn dialog_count(&self) -> u64 {
        *self.inner.dialogs.borrow()
    }

    pub(crate) async fn dialog_opened_after(&self, count: u64) -> crate::dialogs::Dialog {
        let mut opened = self.inner.dialogs.subscribe();
        loop {
            let open = self
                .inner
                .lock_state()
                .dialog
                .open()
                .filter(|dialog| dialog.seq > count)
                .cloned();
            if let Some(dialog) = open {
                return dialog;
            }
            if opened.changed().await.is_err() {
                std::future::pending::<()>().await;
            }
        }
    }

    pub(crate) async fn wait_for_state<T>(
        &self,
        deadline: tokio::time::Instant,
        lookup: impl Fn(&PageState) -> Option<T>,
    ) -> Option<T> {
        let mut version = self.inner.version.subscribe();
        loop {
            let found = lookup(&self.inner.lock_state());
            if found.is_some() {
                return found;
            }
            let woke = tokio::time::timeout_at(deadline, version.changed()).await;
            if !matches!(woke, Ok(Ok(()))) {
                return lookup(&self.inner.lock_state());
            }
        }
    }

    async fn loaded_child_frame(&self, id: FrameId) -> crate::Result<Frame> {
        let deadline = tokio::time::Instant::now() + CHILD_FRAME_WAIT;
        let settled = self
            .wait_for_state(deadline, |state| {
                let frames = &state.frames;
                (frames.get(&id).is_some() && !frames.in_transit(&id)).then_some(())
            })
            .await;
        if settled.is_some() {
            return Ok(Frame {
                page: self.clone(),
                id,
            });
        }
        if self.inner.lock_state().frames.get(&id).is_some() {
            return Err(BrowserError::FrameInTransit {
                frame: id.to_string(),
            });
        }
        Err(BrowserError::NoSuchFrame {
            message: "The frame has not loaded yet".to_owned(),
        })
    }
}

fn swap_back_candidate(frames: &crate::frames::FrameTree, event: &Event) -> Option<FrameId> {
    if &*event.method != "Page.frameAttached" {
        return None;
    }
    let frame = FrameId::from(event.params["frameId"].as_str()?);
    let owner = frames.get(&frame)?.session.as_ref()?;
    (Some(owner) != event.session.as_ref()).then_some(frame)
}

fn iframe_sessions(frames: &FrameTree, page_session: &SessionId) -> Vec<SessionId> {
    let mut sessions: Vec<SessionId> = Vec::new();
    for id in frames.descendants_preorder(frames.main_id()) {
        let local_root = frames.get(&id).and_then(|node| node.session.as_ref());
        if let Some(session) = local_root
            && session != page_session
            && !sessions.contains(session)
        {
            sessions.push(session.clone());
        }
    }
    sessions
}

/// `DOM.describeNode` of the owner bracketed by its frame stamp: after a same-process navigation
/// the call succeeds with a node of the new document, so a stale owner never reaches it.
async fn describe_owner(owner: &crate::element::Element) -> crate::Result<DescribedNode> {
    let frame = owner.frame();
    let before = frame.stamp()?;
    if owner.loader_id != before.loader || owner.local_root != before.local_root {
        return Err(BrowserError::StaleElement);
    }
    let mut reply = frame
        .page
        .inner
        .connection
        .session(Some(before.session.clone()))
        .send(
            "DOM.describeNode",
            json!({"backendNodeId": owner.backend_node_id}),
        )
        .await?;
    if frame.stamp()? != before {
        return Err(BrowserError::LoaderChanged {
            method: "DOM.describeNode".to_owned(),
        });
    }
    let node = reply
        .get_mut("node")
        .map(Value::take)
        .unwrap_or_else(|| json!({}));
    decode("DOM.describeNode", node)
}

fn lookup_error(frame: &FrameId, lookup: FrameLookup) -> BrowserError {
    match lookup {
        FrameLookup::Missing => BrowserError::NoSuchFrame {
            message: format!("The frame {frame} is no longer part of the page"),
        },
        FrameLookup::InTransit => BrowserError::FrameInTransit {
            frame: frame.to_string(),
        },
    }
}

impl Frame {
    pub fn id(&self) -> &FrameId {
        &self.id
    }

    pub fn page(&self) -> &Page {
        &self.page
    }

    pub fn is_main(&self) -> bool {
        *self.page.inner.lock_state().frames.main_id() == self.id
    }

    pub fn loader_id(&self) -> Option<LoaderId> {
        self.page
            .inner
            .lock_state()
            .frames
            .committed_loader(&self.id)
    }

    pub async fn child_frame(&self, owner: &crate::element::Element) -> crate::Result<Frame> {
        owner
            .run(OpSpec::READ, |_| self.child_frame_in(owner))
            .await?
            .read()
    }

    pub fn parent(&self) -> Option<Frame> {
        let state = self.page.inner.lock_state();
        let parent = state.frames.get(&self.id)?.parent.clone()?;
        state.frames.get(&parent)?;
        Some(Frame {
            page: self.page.clone(),
            id: parent,
        })
    }

    async fn child_frame_in(&self, owner: &crate::element::Element) -> crate::Result<Frame> {
        let node = describe_owner(owner).await?;
        match node.frame_id.filter(|id| !id.as_str().is_empty()) {
            Some(id) => self.page.loaded_child_frame(id).await,
            None => Err(BrowserError::NoSuchFrame {
                message: "The element is not an iframe or frame".to_owned(),
            }),
        }
    }

    pub(crate) fn stamp(&self) -> crate::Result<FrameStamp> {
        let state = self.page.inner.lock_state();
        let frames = &state.frames;
        if frames.get(&self.id).is_none() {
            return Err(lookup_error(&self.id, FrameLookup::Missing));
        }
        let session = frames
            .session_for_frame(&self.id)
            .map_err(|lookup| lookup_error(&self.id, lookup))?;
        let local_root = frames
            .local_root(&self.id)
            .map_err(|lookup| lookup_error(&self.id, lookup))?;
        let loader = frames
            .committed_loader(&self.id)
            .ok_or_else(|| lookup_error(&self.id, FrameLookup::InTransit))?;
        Ok(FrameStamp {
            loader,
            local_root: TargetId::new(local_root.as_str()),
            session,
        })
    }

    pub(crate) fn session(&self) -> crate::Result<crate::session::Session> {
        let session = self
            .page
            .inner
            .lock_state()
            .frames
            .session_for_frame(&self.id)
            .map_err(|lookup| lookup_error(&self.id, lookup))?;
        Ok(self.page.inner.connection.session(Some(session)))
    }
}

fn is_error_page(url: &str) -> bool {
    matches!(
        url,
        "chrome-error://chromewebdata/" | "data:text/html,chromewebdata"
    )
}

fn current_history_url(history: &Value) -> Option<String> {
    let index = usize::try_from(history.get("currentIndex")?.as_u64()?).ok()?;
    history
        .get("entries")?
        .get(index)?
        .get("url")?
        .as_str()
        .map(str::to_owned)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::PageHarness;
    use serde_json::json;
    use std::time::Duration;

    async fn flush(harness: &PageHarness, marker: &str) {
        let cursor = harness.connection.events().cursor();
        harness.control.emit(
            "Target.targetInfoChanged",
            None,
            json!({"targetInfo": {"targetId": marker}}),
        );
        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        let seen = harness
            .connection
            .events()
            .wait_for(cursor, deadline, |event| {
                event.params["targetInfo"]["targetId"] == marker
            })
            .await
            .unwrap();
        assert!(seen.is_some(), "the reader never processed {marker}");
    }

    async fn wait_until(harness: &PageHarness, what: &str, done: impl Fn(&PageState) -> bool) {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        let reached = harness
            .page
            .wait_for_state(deadline, |state| done(state).then_some(()))
            .await;
        assert!(reached.is_some(), "{what} never applied");
    }

    #[tokio::test]
    async fn a_dialog_opening_during_a_wait_is_not_lost() {
        let harness = PageHarness::new().await;
        let count = harness.page.dialog_count();
        let waiter = tokio::spawn({
            let page = harness.page.clone();
            async move { page.dialog_opened_after(count).await }
        });
        harness.emit(
            "Page.javascriptDialogOpening",
            json!({"url": "http://127.0.0.1/", "message": "hi", "type": "alert", "defaultPrompt": ""}),
        );
        let dialog = tokio::time::timeout(Duration::from_secs(5), waiter)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(dialog.message, "hi");
        assert_eq!(dialog.seq, count + 1);
        assert_eq!(harness.page.dialog_count(), count + 1);
        assert!(harness.page.pending_dialog().is_some());
        harness.emit(
            "Page.javascriptDialogClosed",
            json!({"result": true, "userInput": ""}),
        );
        wait_until(&harness, "the dialog close", |state| {
            state.dialog.open().is_none()
        })
        .await;
        assert!(harness.page.pending_dialog().is_none());
    }

    #[tokio::test]
    async fn console_backlog_resets_when_the_main_frame_commits() {
        let harness = PageHarness::new().await;
        harness.emit(
            "Runtime.consoleAPICalled",
            json!({"type": "log", "args": []}),
        );
        harness.emit_on(
            "S9",
            "Runtime.consoleAPICalled",
            json!({"type": "log", "args": []}),
        );
        harness.emit(
            "Runtime.exceptionThrown",
            json!({"exceptionDetails": {"text": "boom"}}),
        );
        flush(&harness, "console").await;
        let backlog = harness.page.inner.lock_state().console.snapshot();
        assert_eq!(backlog.len(), 2);
        harness.emit(
            "Page.frameNavigated",
            json!({"frame": {"id": "T1", "loaderId": "L2", "url": "http://127.0.0.1/next"}, "type": "Navigation"}),
        );
        wait_until(&harness, "the navigation", |state| {
            state.frames.committed_loader(&FrameId::from("T1")) == Some(LoaderId::from("L2"))
        })
        .await;
        assert!(
            harness
                .page
                .inner
                .lock_state()
                .console
                .snapshot()
                .is_empty()
        );
    }

    #[tokio::test]
    async fn a_swap_back_to_the_parent_forgets_the_child_contexts() {
        let harness = PageHarness::new().await;
        harness.attach_child("S2", "F2", "T1", "http://127.0.0.1/child", "L2");
        let frame = harness.page.frame(&FrameId::from("F2")).unwrap();
        let child = SessionId::from("S2");
        assert!(
            harness
                .page
                .inner
                .lock_state()
                .contexts
                .util(frame.id(), &child)
                .is_some()
        );
        harness.emit(
            "Page.frameAttached",
            json!({"frameId": "F2", "parentFrameId": "T1"}),
        );
        wait_until(&harness, "the swap back", |state| {
            state.frames.session_for_frame(frame.id()).ok() == Some(SessionId::from("S1"))
        })
        .await;
        assert!(
            harness
                .page
                .inner
                .lock_state()
                .contexts
                .util(frame.id(), &child)
                .is_none()
        );
        assert!(matches!(
            frame.stamp(),
            Err(BrowserError::FrameInTransit { .. })
        ));
    }

    #[tokio::test]
    async fn crashes_and_closes_are_tracked_on_the_page_session() {
        let harness = PageHarness::new().await;
        harness.emit("Inspector.targetCrashed", json!({}));
        wait_until(&harness, "the crash", |state| state.crashed).await;
        let already = tokio::time::Instant::now() + Duration::from_millis(50);
        assert_eq!(
            harness
                .page
                .wait_for_state(already, |state| state.crashed.then_some(()))
                .await,
            Some(()),
            "a change applied before the wait is still seen"
        );
        harness.emit("Inspector.targetReloadedAfterCrash", json!({}));
        wait_until(&harness, "the reload", |state| !state.crashed).await;
        harness.page.mark_closed();
        assert!(harness.page.is_closed());
    }

    async fn committed(harness: &PageHarness, url: &str) {
        harness.emit(
            "Page.frameNavigated",
            json!({"frame": {"id": "T1", "loaderId": "L2", "url": url}, "type": "Navigation"}),
        );
        harness.emit("Page.frameStoppedLoading", json!({"frameId": "T1"}));
        wait_until(harness, url, |state| state.frames.main_url() == url).await;
    }

    #[test]
    fn error_pages_take_the_url_of_the_current_history_entry() {
        assert!(is_error_page("chrome-error://chromewebdata/"));
        assert!(is_error_page("data:text/html,chromewebdata"));
        assert!(!is_error_page("https://example.com/"));
        assert!(!is_error_page("about:blank"));
        let history = json!({
            "currentIndex": 1,
            "entries": [{"id": 1, "url": "https://a.example/"}, {"id": 2, "url": "https://expired.example/"}]
        });
        assert_eq!(
            current_history_url(&history).as_deref(),
            Some("https://expired.example/")
        );
        for broken in [
            json!({"currentIndex": 2, "entries": [{"url": "https://a.example/"}]}),
            json!({"currentIndex": -1, "entries": [{"url": "https://a.example/"}]}),
            json!({"currentIndex": 0, "entries": [{"title": "no url"}]}),
            json!({"entries": [{"url": "https://a.example/"}]}),
        ] {
            assert_eq!(current_history_url(&broken), None, "{broken}");
        }
    }

    #[tokio::test]
    async fn a_loaded_page_reports_its_own_url_without_reading_the_history() {
        let harness = PageHarness::new().await;
        committed(&harness, "https://example.com/done").await;
        assert_eq!(
            harness.page.url().await.unwrap(),
            "https://example.com/done"
        );
        assert!(
            !harness
                .control
                .commands_seen()
                .iter()
                .any(|command| command.method == "Page.getNavigationHistory")
        );
    }

    #[tokio::test]
    async fn an_error_page_reports_the_requested_url_like_chromedriver() {
        let mut harness = PageHarness::new().await;
        committed(&harness, "chrome-error://chromewebdata/").await;
        let page = harness.page.clone();
        let url = tokio::spawn(async move { page.url().await });
        let history = harness
            .control
            .wait_for("Page.getNavigationHistory", Some("S1"))
            .await;
        harness.control.reply(
            &history,
            json!({
                "currentIndex": 1,
                "entries": [
                    {"id": 1, "url": "http://127.0.0.1/"},
                    {"id": 2, "url": "https://expired.example/"}
                ]
            }),
        );
        assert_eq!(
            url.await
                .expect("the url task finishes")
                .expect("the history names the current entry"),
            "https://expired.example/"
        );
    }
}
