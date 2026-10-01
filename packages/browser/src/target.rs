// Derived from agent-browser cli/src/native/browser.rs @d01253d, Copyright 2025 Vercel Inc., Apache-2.0; modified by Rheosoph GmbH. See NOTICE.
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, Weak};
use std::time::Duration;

use serde_json::{Value, json};
use tokio::sync::watch;
use tokio::time::Instant;

use crate::browser::{BrowserInner, BrowserSettings, ConnectionKind, PageInfo};
use crate::connection::{Connection, EventHook, PendingReply, Reply};
use crate::error::{BrowserError, ErrorClass};
use crate::event_log::Event;
use crate::frames::{DetachResolution, UTIL_WORLD};
use crate::page::{Page, PageInner};
use crate::session::Session;
use crate::types::{
    AttachedToTarget, FrameId, FrameTreeNode, SessionId, TargetId, TargetInfo, TargetType,
};

const SUCCESSOR_WINDOW: Duration = Duration::from_secs(1);
const PROBE_TIMEOUT: Duration = Duration::from_secs(2);
const REVIVE_BUDGET: Duration = Duration::from_secs(10);
const ISOLATED_WORLD_TIMEOUT: Duration = Duration::from_secs(2);
const CLOSE_CONFIRM_TIMEOUT: Duration = Duration::from_secs(5);
const SERVER_ERROR: i64 = -32000;
const INVALID_PARAMS: i64 = -32602;
const PARSE_ERROR: i64 = -32700;
const PRERENDER: &str = "prerender";
// Chrome's subtype of a page target whose frame tree node is gone, e.g. the page a prerender
// activation replaced (content/browser/devtools/render_frame_devtools_agent_host.cc GetSubtype).
const DISCONNECTED: &str = "disconnected";
const LOAD_STATE_PROBE: &str = "({s: document.readyState, u: document.URL, b: document.baseURI})";

type AttachSlot = Arc<tokio::sync::Mutex<Option<Page>>>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SessionRole {
    Page,
    Iframe,
    Worker,
}

struct Tracked {
    info: TargetInfo,
    attach: AttachSlot,
}

struct Vanished {
    target: TargetId,
    position: usize,
    at: Instant,
}

struct Activation {
    target: TargetId,
    at: Instant,
}

#[derive(Default)]
struct Registry {
    order: Vec<TargetId>,
    targets: HashMap<TargetId, Tracked>,
    attached: HashMap<TargetId, Page>,
    owners: HashMap<SessionId, Page>,
    closing: HashSet<TargetId>,
    successors: HashMap<TargetId, TargetId>,
    vanished: Vec<Vanished>,
    activated: Vec<Activation>,
    replaced: HashSet<TargetId>,
}

fn is_tab(info: &TargetInfo) -> bool {
    info.type_ == TargetType::Page
        && info.subtype.as_deref().unwrap_or_default().is_empty()
        && !info.url.starts_with("devtools://")
}

fn is_prerender(info: &TargetInfo) -> bool {
    info.type_ == TargetType::Page && info.subtype.as_deref() == Some(PRERENDER)
}

fn is_disconnected(info: &TargetInfo) -> bool {
    info.type_ == TargetType::Page && info.subtype.as_deref() == Some(DISCONNECTED)
}

fn replaced_by_prerender() -> BrowserError {
    BrowserError::NoSuchPage {
        message: "The current tab was replaced by a prerendered page; use Select Tab".to_owned(),
    }
}

fn within_window(first: Instant, second: Instant) -> bool {
    first.max(second) - first.min(second) <= SUCCESSOR_WINDOW
}

fn closed_tab() -> BrowserError {
    BrowserError::NoSuchPage {
        message: "The current browser tab was closed; use Select Tab or New Page".to_owned(),
    }
}

fn same_page(first: &Page, second: &Page) -> bool {
    Arc::ptr_eq(&first.inner, &second.inner)
}

impl Registry {
    fn track(&mut self, info: TargetInfo) -> bool {
        if info.type_ != TargetType::Page || self.targets.contains_key(&info.target_id) {
            return false;
        }
        if is_tab(&info) {
            self.order.push(info.target_id.clone());
        }
        let tracked = Tracked {
            info,
            attach: AttachSlot::default(),
        };
        self.targets.insert(tracked.info.target_id.clone(), tracked);
        true
    }

    fn update(&mut self, info: TargetInfo, now: Instant) -> bool {
        let Some(tracked) = self.targets.get_mut(&info.target_id) else {
            return self.track(info);
        };
        let was_prerender = is_prerender(&tracked.info);
        let was_tab = is_tab(&tracked.info);
        let now_tab = is_tab(&info);
        let disconnected = is_disconnected(&info);
        let id = info.target_id.clone();
        tracked.info = info;
        let position = self.position(&id);
        self.retab(&id, was_tab, now_tab);
        // A prerender activation never detaches a tab we do not hold; its page turns
        // disconnected instead, and a held page is never prerendered (see §2.16).
        if was_tab && disconnected {
            self.vanish(&id, position, now);
        }
        if was_prerender && now_tab {
            self.activated.push(Activation {
                target: id,
                at: now,
            });
            self.bind_successor(now);
        }
        true
    }

    fn retab(&mut self, id: &TargetId, was_tab: bool, now_tab: bool) {
        if was_tab && !now_tab {
            self.order.retain(|target| target != id);
        } else if now_tab && !was_tab {
            self.order.push(id.clone());
        }
    }

    fn destroyed(&mut self, target: &TargetId, now: Instant) -> Option<Page> {
        let position = self.position(target);
        self.targets.remove(target);
        self.order.retain(|id| id != target);
        self.activated
            .retain(|activation| activation.target != *target);
        let page = self.attached.remove(target)?;
        self.forget_sessions(&page);
        self.vanish(target, position, now);
        Some(page)
    }

    fn page_session_gone(&mut self, session: &SessionId, now: Instant) -> Option<Page> {
        let page = self
            .owners
            .get(session)
            .filter(|page| page.inner.session == *session)
            .cloned()?;
        let target = page.target_id().clone();
        if self
            .attached
            .get(&target)
            .is_some_and(|held| same_page(held, &page))
        {
            self.attached.remove(&target);
        }
        self.forget_sessions(&page);
        let position = self.position(&target);
        self.vanish(&target, position, now);
        Some(page)
    }

    fn forget(&mut self, target: &TargetId) -> Option<Page> {
        self.targets.remove(target);
        self.order.retain(|id| id != target);
        let page = self.attached.remove(target)?;
        self.forget_sessions(&page);
        Some(page)
    }

    fn forget_sessions(&mut self, page: &Page) {
        self.owners.retain(|_, owner| !same_page(owner, page));
    }

    fn position(&self, target: &TargetId) -> Option<usize> {
        self.order.iter().position(|id| id == target)
    }

    fn vanish(&mut self, target: &TargetId, position: Option<usize>, now: Instant) {
        let known = self.successors.contains_key(target)
            || self
                .vanished
                .iter()
                .any(|vanished| vanished.target == *target);
        if self.closing.contains(target) || known {
            return;
        }
        self.vanished.push(Vanished {
            target: target.clone(),
            position: position.unwrap_or(self.order.len()),
            at: now,
        });
        self.bind_successor(now);
    }

    fn expire(&mut self, now: Instant) {
        let (expired, pending): (Vec<Vanished>, Vec<Vanished>) = std::mem::take(&mut self.vanished)
            .into_iter()
            .partition(|vanished| now >= vanished.at + SUCCESSOR_WINDOW);
        self.vanished = pending;
        for vanished in expired {
            let contested = self
                .activated
                .iter()
                .any(|activation| within_window(activation.at, vanished.at));
            if contested {
                self.order.retain(|id| *id != vanished.target);
                self.replaced.insert(vanished.target);
            }
        }
        self.activated
            .retain(|activation| now < activation.at + SUCCESSOR_WINDOW);
    }

    fn bind_successor(&mut self, now: Instant) {
        self.expire(now);
        let [vanished] = self.vanished.as_slice() else {
            return;
        };
        let [activation] = self.activated.as_slice() else {
            return;
        };
        if !within_window(vanished.at, activation.at) {
            return;
        }
        let vanished = self.vanished.remove(0);
        let activation = self.activated.remove(0);
        self.order
            .retain(|id| *id != vanished.target && *id != activation.target);
        let position = vanished.position.min(self.order.len());
        self.order.insert(position, activation.target.clone());
        self.successors.insert(vanished.target, activation.target);
    }

    fn follow(&self, target: &TargetId) -> TargetId {
        let mut current = target.clone();
        for _ in 0..=self.successors.len() {
            match self.successors.get(&current) {
                Some(next) => current = next.clone(),
                None => break,
            }
        }
        current
    }

    fn binding_deadline(&self, target: &TargetId) -> Option<Instant> {
        let vanished = self
            .vanished
            .iter()
            .find(|vanished| vanished.target == *target)?;
        let candidates = !self.activated.is_empty()
            || self
                .targets
                .values()
                .any(|tracked| is_prerender(&tracked.info));
        candidates.then_some(vanished.at + SUCCESSOR_WINDOW)
    }

    fn slot(&self, target: &TargetId) -> crate::Result<AttachSlot> {
        if self.replaced.contains(target) {
            return Err(replaced_by_prerender());
        }
        match self.targets.get(target) {
            Some(tracked) if !is_disconnected(&tracked.info) => Ok(tracked.attach.clone()),
            _ => Err(closed_tab()),
        }
    }

    fn tabs(&self) -> Vec<PageInfo> {
        self.order
            .iter()
            .filter_map(|id| self.targets.get(id))
            .map(|tracked| PageInfo {
                target_id: tracked.info.target_id.clone(),
                url: tracked.info.url.clone(),
                title: tracked.info.title.clone(),
                opener_id: tracked.info.opener_id.clone(),
            })
            .collect()
    }
}

pub(crate) struct TargetManager {
    connection: Connection,
    settings: Arc<BrowserSettings>,
    browser: Weak<BrowserInner>,
    registry: Mutex<Registry>,
    changed: watch::Sender<u64>,
}

fn target_info(event: &Event) -> Option<TargetInfo> {
    serde_json::from_value(event.params.get("targetInfo")?.clone()).ok()
}

fn session_param(params: &Value) -> Option<SessionId> {
    params["sessionId"].as_str().map(SessionId::from)
}

fn bump(page: &Page) {
    page.inner.version.send_modify(|version| *version += 1);
}

fn attach_error(error: BrowserError) -> BrowserError {
    match error {
        BrowserError::Protocol { code, message, .. }
            if code == SERVER_ERROR && message.contains("Not allowed") =>
        {
            BrowserError::NoSuchPage {
                message: "This tab is protected by browser policy; choose another tab".to_owned(),
            }
        }
        BrowserError::Protocol { code, .. } if code == INVALID_PARAMS => closed_tab(),
        other => other,
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Liveness {
    Responsive,
    Silent,
}

async fn probe(session: &Session) -> crate::Result<Liveness> {
    match session
        .send_with_timeout(
            "Runtime.evaluate",
            json!({"expression": "1"}),
            PROBE_TIMEOUT,
        )
        .await
    {
        Ok(_) => Ok(Liveness::Responsive),
        Err(BrowserError::Timeout { .. }) => Ok(Liveness::Silent),
        Err(error) if error.class() == ErrorClass::SessionGone => Err(error),
        Err(BrowserError::Protocol { .. }) => Ok(Liveness::Responsive),
        Err(error) => Err(error),
    }
}

impl TargetManager {
    pub(crate) fn new(
        connection: Connection,
        settings: std::sync::Arc<crate::browser::BrowserSettings>,
        browser: std::sync::Weak<crate::browser::BrowserInner>,
    ) -> std::sync::Arc<Self> {
        Arc::new(Self {
            connection,
            settings,
            browser,
            registry: Mutex::new(Registry::default()),
            changed: watch::Sender::new(0),
        })
    }

    fn registry(&self) -> MutexGuard<'_, Registry> {
        self.registry.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn notify(&self) {
        self.changed.send_modify(|version| *version += 1);
    }

    fn root(&self) -> Session {
        self.connection.session(None)
    }

    pub(crate) async fn start(self: &std::sync::Arc<Self>) -> crate::Result<()> {
        let root = self.root();
        root.send("Target.setDiscoverTargets", json!({"discover": true}))
            .await?;
        let listed = root.send("Target.getTargets", json!({})).await?;
        let infos: Vec<TargetInfo> = match listed.get("targetInfos") {
            Some(infos) => crate::protocol::decode("Target.getTargets", infos.clone())?,
            None => Vec::new(),
        };
        let mut changed = false;
        {
            let mut registry = self.registry();
            for info in infos {
                changed |= registry.track(info);
            }
        }
        if changed {
            self.notify();
        }
        Ok(())
    }

    pub(crate) fn pages(&self) -> Vec<PageInfo> {
        self.registry().tabs()
    }

    pub(crate) fn page_for_session(&self, session: &SessionId) -> Option<Page> {
        self.registry().owners.get(session).cloned()
    }

    pub(crate) fn attached_page(&self, target: &TargetId) -> Option<Page> {
        let page = self.registry().attached.get(target).cloned()?;
        (!page.is_closed()).then_some(page)
    }

    pub(crate) async fn page(
        self: &std::sync::Arc<Self>,
        target: &TargetId,
    ) -> crate::Result<Page> {
        let (id, slot) = self.resolve(target).await?;
        let held = slot.lock_owned().await;
        if let Some(page) = held.as_ref().filter(|page| !page.is_closed()) {
            return Ok(page.clone());
        }
        let manager = Arc::clone(self);
        let attaching = tokio::spawn(async move { manager.attach_into(held, &id).await });
        match attaching.await {
            Ok(attached) => attached,
            Err(error) if error.is_panic() => std::panic::resume_unwind(error.into_panic()),
            Err(_) => Err(BrowserError::Disconnected {
                reason: format!("attaching target {target} was cancelled by the runtime"),
            }),
        }
    }

    async fn attach_into(
        &self,
        mut held: tokio::sync::OwnedMutexGuard<Option<Page>>,
        target: &TargetId,
    ) -> crate::Result<Page> {
        *held = None;
        let page = self.attach(target).await?;
        *held = Some(page.clone());
        Ok(page)
    }

    async fn resolve(&self, target: &TargetId) -> crate::Result<(TargetId, AttachSlot)> {
        let mut changes = self.changed.subscribe();
        loop {
            let wait = {
                let mut registry = self.registry();
                registry.expire(Instant::now());
                let id = registry.follow(target);
                match registry.binding_deadline(&id) {
                    Some(deadline) => deadline,
                    None => return registry.slot(&id).map(|slot| (id, slot)),
                }
            };
            let _ = tokio::time::timeout_at(wait, changes.changed()).await;
        }
    }

    async fn attach(&self, target: &TargetId) -> crate::Result<Page> {
        let session = self.attach_session(target).await?;
        if self.settings.kind() != ConnectionKind::Launched {
            self.ensure_responsive(target, &session).await?;
        }
        let page = self.adopt(target, session);
        let prepared = match prepare(&page).await {
            Ok(()) if page.is_closed() => Err(closed_tab()),
            prepared => prepared,
        };
        if let Err(error) = prepared {
            self.release(&page);
            return Err(error);
        }
        Ok(page)
    }

    async fn attach_session(&self, target: &TargetId) -> crate::Result<SessionId> {
        let reply = self
            .root()
            .send(
                "Target.attachToTarget",
                json!({"targetId": target, "flatten": true}),
            )
            .await
            .map_err(attach_error)?;
        session_param(&reply).ok_or_else(|| {
            BrowserError::protocol(
                "Target.attachToTarget",
                PARSE_ERROR,
                format!("the reply for target {target} carries no sessionId"),
            )
        })
    }

    async fn ensure_responsive(&self, target: &TargetId, session: &SessionId) -> crate::Result<()> {
        let revived = self.revive(target, session).await;
        if !matches!(revived, Ok(Liveness::Responsive)) {
            self.detach(session);
        }
        match revived? {
            Liveness::Responsive => Ok(()),
            Liveness::Silent => Err(BrowserError::NoSuchPage {
                message: "The tab is not responding (it may be discarded, frozen, or blocked by a dialog)"
                    .to_owned(),
            }),
        }
    }

    async fn revive(&self, target: &TargetId, session: &SessionId) -> crate::Result<Liveness> {
        let tab = self.connection.session(Some(session.clone()));
        tab.send_nowait("Runtime.runIfWaitingForDebugger", json!({}))?;
        if probe(&tab).await? == Liveness::Responsive {
            return Ok(Liveness::Responsive);
        }
        let deadline = Instant::now() + REVIVE_BUDGET;
        if let Err(error) = self
            .root()
            .send_with_timeout(
                "Target.activateTarget",
                json!({"targetId": target}),
                PROBE_TIMEOUT,
            )
            .await
        {
            tracing::debug!(%target, %error, "activating a silent tab failed");
        }
        while Instant::now() < deadline {
            if probe(&tab).await? == Liveness::Responsive {
                return Ok(Liveness::Responsive);
            }
        }
        Ok(Liveness::Silent)
    }

    fn detach(&self, session: &SessionId) {
        if let Err(error) = self
            .root()
            .send_nowait("Target.detachFromTarget", json!({"sessionId": session}))
        {
            tracing::debug!(%session, %error, "detaching an unusable page session failed");
        }
    }

    fn adopt(&self, target: &TargetId, session: SessionId) -> Page {
        let inner = PageInner::new(
            self.connection.clone(),
            self.browser.clone(),
            self.settings.clone(),
            target.clone(),
            session.clone(),
            FrameId::new(target.as_str()),
        );
        let page = Page {
            inner: Arc::new(inner),
        };
        page.add_session(session.clone());
        let mut registry = self.registry();
        registry.owners.insert(session, page.clone());
        registry.attached.insert(target.clone(), page.clone());
        page
    }

    fn release(&self, page: &Page) {
        {
            let mut registry = self.registry();
            registry.forget_sessions(page);
            let target = page.target_id();
            if registry
                .attached
                .get(target)
                .is_some_and(|held| same_page(held, page))
            {
                registry.attached.remove(target);
            }
        }
        page.mark_closed();
        self.detach(&page.inner.session);
    }

    pub(crate) async fn create_page(self: &std::sync::Arc<Self>) -> crate::Result<Page> {
        let created = self
            .root()
            .send("Target.createTarget", json!({"url": "about:blank"}))
            .await?;
        let target = created["targetId"]
            .as_str()
            .map(TargetId::from)
            .ok_or_else(|| {
                BrowserError::protocol(
                    "Target.createTarget",
                    PARSE_ERROR,
                    "the reply carries no targetId",
                )
            })?;
        let info = TargetInfo {
            target_id: target.clone(),
            type_: TargetType::Page,
            url: "about:blank".to_owned(),
            ..TargetInfo::default()
        };
        if self.registry().track(info) {
            self.notify();
        }
        self.page(&target).await
    }

    pub(crate) async fn close_page(&self, target: &TargetId) -> crate::Result<()> {
        let target = {
            let mut registry = self.registry();
            let id = registry.follow(target);
            registry.slot(&id)?;
            registry.closing.insert(id.clone());
            id
        };
        let closed = self.close_target(&target).await;
        let forgotten = {
            let mut registry = self.registry();
            registry.closing.remove(&target);
            closed.map(|()| registry.forget(&target))?
        };
        self.notify();
        if let Some(page) = forgotten {
            page.mark_closed();
        }
        Ok(())
    }

    async fn close_target(&self, target: &TargetId) -> crate::Result<()> {
        let cursor = self.connection.events().cursor();
        match self
            .root()
            .send("Target.closeTarget", json!({"targetId": target}))
            .await
        {
            Ok(_) => {}
            Err(BrowserError::Protocol { code, .. }) if code == INVALID_PARAMS => return Ok(()),
            Err(BrowserError::Disconnected { .. }) => return Ok(()),
            Err(error) => return Err(error),
        }
        let deadline = Instant::now() + CLOSE_CONFIRM_TIMEOUT;
        let destroyed = self
            .connection
            .events()
            .wait_for(cursor, deadline, |event| {
                &*event.method == "Target.targetDestroyed"
                    && event.params["targetId"] == target.as_str()
            })
            .await;
        match destroyed {
            Ok(Some(_)) | Err(BrowserError::Disconnected { .. }) => {}
            Ok(None) => tracing::warn!(%target, "the closed tab was not destroyed within 5 s"),
            Err(error) => tracing::debug!(%target, %error, "could not confirm that the tab closed"),
        }
        Ok(())
    }

    pub(crate) async fn wait_first_page(
        &self,
        deadline: tokio::time::Instant,
    ) -> crate::Result<TargetId> {
        let mut changes = self.changed.subscribe();
        loop {
            if let Some(first) = self.registry().order.first().cloned() {
                return Ok(first);
            }
            let changed = tokio::select! {
                changed = tokio::time::timeout_at(deadline, changes.changed()) => changed,
                () = self.connection.closed() => {
                    return Err(BrowserError::Disconnected {
                        reason: self.connection.closed_reason().unwrap_or_default(),
                    });
                }
            };
            if !matches!(changed, Ok(Ok(()))) {
                return Err(BrowserError::Launch {
                    message: "The browser started but opened no page before the launch timeout"
                        .to_owned(),
                });
            }
        }
    }

    fn on_root_event(&self, event: &Event) {
        let now = Instant::now();
        let (changed, gone) = {
            let mut registry = self.registry();
            match &*event.method {
                "Target.targetCreated" => (
                    target_info(event).is_some_and(|info| registry.track(info)),
                    None,
                ),
                "Target.targetInfoChanged" => (
                    target_info(event).is_some_and(|info| registry.update(info, now)),
                    None,
                ),
                "Target.targetDestroyed" => {
                    let target = event.params["targetId"].as_str().map(TargetId::from);
                    let gone = target.and_then(|target| registry.destroyed(&target, now));
                    (true, gone)
                }
                "Target.detachedFromTarget" => {
                    let gone = session_param(&event.params)
                        .and_then(|session| registry.page_session_gone(&session, now));
                    (gone.is_some(), gone)
                }
                _ => return,
            }
        };
        if changed {
            self.notify();
        }
        if let Some(page) = gone {
            page.on_event(event);
            page.mark_closed();
        }
    }

    fn on_session_event(&self, session: &SessionId, event: &Event) {
        let Some(page) = self.page_for_session(session) else {
            return;
        };
        page.on_event(event);
        match &*event.method {
            "Target.attachedToTarget" => self.on_child_attached(&page, session, event),
            "Target.detachedFromTarget" => self.on_child_detached(&page, session, event),
            "Inspector.detached" if *session == page.inner.session => {
                self.on_page_session_lost(session);
            }
            _ => {}
        }
    }

    fn on_page_session_lost(&self, session: &SessionId) {
        let gone = self.registry().page_session_gone(session, Instant::now());
        if let Some(page) = gone {
            self.notify();
            page.mark_closed();
        }
    }

    fn on_child_attached(&self, page: &Page, parent: &SessionId, event: &Event) {
        let Some(attached) = event.decode::<AttachedToTarget>() else {
            return;
        };
        let child = attached.session_id;
        let role = match attached.target_info.type_ {
            TargetType::Iframe => SessionRole::Iframe,
            TargetType::Worker => SessionRole::Worker,
            _ => return self.dismiss(parent, &child),
        };
        if role == SessionRole::Iframe {
            register_child_frame(page, parent, &child, &attached.target_info, event.seq);
        }
        page.add_session(child.clone());
        self.registry().owners.insert(child.clone(), page.clone());
        let page = page.clone();
        tokio::spawn(async move {
            if let Err(error) = setup_session(&page, &child, role).await {
                tracing::debug!(
                    session = %child,
                    target = %page.target_id(),
                    %error,
                    "child session setup failed"
                );
            }
        });
    }

    fn dismiss(&self, parent: &SessionId, child: &SessionId) {
        let resumed =
            self.connection
                .send_nowait("Runtime.runIfWaitingForDebugger", json!({}), Some(child));
        let detached = self.connection.send_nowait(
            "Target.detachFromTarget",
            json!({"sessionId": child}),
            Some(parent),
        );
        if let Err(error) = resumed.and(detached) {
            tracing::debug!(session = %child, %error, "releasing an unused child session failed");
        }
    }

    fn on_child_detached(&self, page: &Page, parent: &SessionId, event: &Event) {
        let Some(child) = session_param(&event.params) else {
            return;
        };
        if self.registry().owners.remove(&child).is_none() {
            return;
        }
        let resolution = {
            let mut state = page.inner.lock_state();
            let resolution = state.frames.on_child_session_detached(&child);
            if resolution == DetachResolution::NeedsRoundTrip {
                let frames = state.frames.frames_of_session(&child);
                state.navigation.reset_frames(&frames);
            }
            resolution
        };
        bump(page);
        if resolution == DetachResolution::NeedsRoundTrip {
            tokio::spawn(resolve_detach(page.clone(), parent.clone(), child));
        }
    }
}

impl EventHook for TargetManager {
    fn on_event(&self, _connection: &Connection, event: &Event) {
        match event.session.as_ref() {
            None => self.on_root_event(event),
            Some(session) => self.on_session_event(session, event),
        }
    }
}

fn register_child_frame(
    page: &Page,
    parent: &SessionId,
    child: &SessionId,
    info: &TargetInfo,
    seq: u64,
) {
    let frame = FrameId::new(info.target_id.as_str());
    let changed = {
        let mut state = page.inner.lock_state();
        let changed = state
            .frames
            .on_child_session_attached(parent, child, info, seq);
        state.contexts.forget_frame(&frame, child);
        changed
    };
    if changed {
        bump(page);
    }
}

async fn resolve_detach(page: Page, parent: SessionId, child: SessionId) {
    let parent_session = page.inner.connection.session(Some(parent));
    if let Err(error) = parent_session.send("Page.enable", json!({})).await {
        tracing::debug!(session = %child, %error, "round trip after a child detach failed");
    }
    page.inner
        .lock_state()
        .frames
        .resolve_pending_detach(&child);
    bump(&page);
}

async fn prepare(page: &Page) -> crate::Result<()> {
    let dialogs = page.dialog_count();
    let session = page.inner.session.clone();
    setup_session(page, &session, SessionRole::Page).await?;
    let tab = page.session();
    let probe = tab.request(
        "Runtime.evaluate",
        json!({"expression": LOAD_STATE_PROBE, "returnByValue": true}),
        None,
    );
    let reply = tokio::select! {
        reply = probe => reply,
        _ = page.dialog_opened_after(dialogs) => return Ok(()),
    };
    match reply {
        Ok(reply) => seed_loading(page, &reply),
        Err(error) => {
            tracing::debug!(target = %page.target_id(), %error, "readyState probe failed")
        }
    }
    Ok(())
}

fn seed_loading(page: &Page, reply: &Reply) {
    let value = &reply.result["result"]["value"];
    let (Some(state), Some(url), Some(base)) = (
        value["s"].as_str(),
        value["u"].as_str(),
        value["b"].as_str(),
    ) else {
        return;
    };
    let pending = !url.starts_with("about:blank") && (state != "complete" || base == "about:blank");
    if !pending {
        return;
    }
    let changed = {
        let mut state = page.inner.lock_state();
        let main = state.frames.main_id().clone();
        state.navigation.seed_loading(&main, reply.cursor)
    };
    if changed {
        bump(page);
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Step {
    Essential,
    FrameTree,
    Optional,
    Replay,
}

struct SetupCommand {
    method: String,
    params: Value,
    step: Step,
}

struct Queued {
    method: String,
    step: Step,
    reply: crate::Result<PendingReply>,
}

fn command(method: &str, params: Value, step: Step) -> SetupCommand {
    SetupCommand {
        method: method.to_owned(),
        params,
        step,
    }
}

fn extra_commands(
    commands: Vec<(String, Value)>,
    step: Step,
) -> impl Iterator<Item = SetupCommand> {
    commands
        .into_iter()
        .map(move |(method, params)| SetupCommand {
            method,
            params,
            step,
        })
}

fn frame_session_commands(page: &Page, role: SessionRole) -> Vec<SetupCommand> {
    let mut commands = vec![
        command("Page.enable", json!({}), Step::Essential),
        command("Page.getFrameTree", json!({}), Step::FrameTree),
        command(
            "Page.setLifecycleEventsEnabled",
            json!({"enabled": true}),
            Step::Optional,
        ),
        command("Runtime.enable", json!({}), Step::Essential),
        command(
            "Page.addScriptToEvaluateOnNewDocument",
            json!({"source": "", "worldName": UTIL_WORLD}),
            Step::Optional,
        ),
    ];
    if role == SessionRole::Page {
        commands.push(command(
            "Emulation.setFocusEmulationEnabled",
            json!({"enabled": true}),
            Step::Optional,
        ));
    }
    commands.extend(extra_commands(
        crate::fetch::session_commands(page),
        Step::Optional,
    ));
    if role == SessionRole::Iframe {
        commands.extend(extra_commands(
            crate::emulation::replay_commands(page),
            Step::Replay,
        ));
    }
    commands.push(command(
        "Target.setAutoAttach",
        json!({"autoAttach": true, "waitForDebuggerOnStart": true, "flatten": true}),
        Step::Optional,
    ));
    commands
}

pub(crate) async fn setup_session(
    page: &Page,
    session: &SessionId,
    role: SessionRole,
) -> crate::Result<()> {
    match role {
        SessionRole::Worker => setup_worker(page, session).await,
        SessionRole::Page | SessionRole::Iframe => setup_frame_session(page, session, role).await,
    }
}

async fn setup_frame_session(
    page: &Page,
    session: &SessionId,
    role: SessionRole,
) -> crate::Result<()> {
    let dialogs = page.dialog_count();
    let target = page.inner.connection.session(Some(session.clone()));
    let queued: Vec<Queued> = frame_session_commands(page, role)
        .into_iter()
        .map(|setup| Queued {
            reply: target.enqueue(&setup.method, setup.params, None),
            method: setup.method,
            step: setup.step,
        })
        .collect();
    if let Err(error) = target.send_nowait("Runtime.runIfWaitingForDebugger", json!({})) {
        tracing::debug!(%session, %error, "resuming a new session failed");
    }
    let mut finish = Box::pin(finish_setup(page.clone(), session.clone(), role, queued));
    tokio::select! {
        finished = &mut finish => finished,
        _ = page.dialog_opened_after(dialogs) => {
            tokio::spawn(async move {
                if let Err(error) = finish.await {
                    tracing::debug!(%error, "setup replies drained after a dialog failed");
                }
            });
            Ok(())
        }
    }
}

async fn finish_setup(
    page: Page,
    session: SessionId,
    role: SessionRole,
    queued: Vec<Queued>,
) -> crate::Result<()> {
    let mut frame_tree = None;
    let mut failure = None;
    for Queued {
        method,
        step,
        reply,
    } in queued
    {
        let result = match reply {
            Ok(pending) => pending.await,
            Err(error) => Err(error),
        };
        match result {
            Ok(reply) if step == Step::FrameTree => frame_tree = Some(reply),
            Ok(_) => {}
            Err(error) => {
                if let Err(fatal) = judge_setup_failure(role, step, &method, &session, error) {
                    failure.get_or_insert(fatal);
                }
            }
        }
    }
    if let Some(reply) = frame_tree {
        seed_frames(&page, &session, &reply);
    }
    create_isolated_worlds(&page, &session).await;
    failure.map_or(Ok(()), Err)
}

fn judge_setup_failure(
    role: SessionRole,
    step: Step,
    method: &str,
    session: &SessionId,
    error: BrowserError,
) -> crate::Result<()> {
    let fatal = role == SessionRole::Page && step == Step::Essential;
    if error.class() == ErrorClass::SessionGone {
        return if fatal { Err(closed_tab()) } else { Ok(()) };
    }
    if fatal {
        return Err(error);
    }
    tracing::debug!(%session, method, %error, "session setup command failed");
    Ok(())
}

fn seed_frames(page: &Page, session: &SessionId, reply: &Reply) {
    let Some(tree) = reply.result.get("frameTree") else {
        return;
    };
    let Ok(tree) = crate::protocol::decode::<FrameTreeNode>("Page.getFrameTree", tree.clone())
    else {
        return;
    };
    page.inner
        .lock_state()
        .frames
        .seed(session, &tree, reply.cursor);
    bump(page);
}

async fn create_isolated_worlds(page: &Page, session: &SessionId) {
    let frames: Vec<FrameId> = {
        let state = page.inner.lock_state();
        state
            .frames
            .frames_of_session(session)
            .into_iter()
            .filter(|frame| state.contexts.util(frame, session).is_none())
            .collect()
    };
    let target = page.inner.connection.session(Some(session.clone()));
    let pending: Vec<_> = frames
        .iter()
        .map(|frame| {
            target.enqueue(
                "Page.createIsolatedWorld",
                json!({"frameId": frame, "worldName": UTIL_WORLD, "grantUniveralAccess": true}),
                Some(ISOLATED_WORLD_TIMEOUT),
            )
        })
        .collect();
    for (frame, reply) in frames.iter().zip(pending) {
        let created = match reply {
            Ok(pending) => pending.await.map(drop),
            Err(error) => Err(error),
        };
        if let Err(error) = created {
            tracing::debug!(%frame, %session, %error, "creating the utility world failed");
        }
    }
}

async fn setup_worker(page: &Page, session: &SessionId) -> crate::Result<()> {
    let connection = &page.inner.connection;
    let worker = connection.session(Some(session.clone()));
    let pending: Vec<(String, crate::Result<PendingReply>)> = crate::fetch::session_commands(page)
        .into_iter()
        .map(|(method, params)| {
            let reply = worker.enqueue(&method, params, None);
            (method, reply)
        })
        .collect();
    worker.send_nowait("Runtime.runIfWaitingForDebugger", json!({}))?;
    if pending.is_empty() {
        let parent = connection
            .session_info(session)
            .and_then(|info| info.parent);
        connection.send_nowait(
            "Target.detachFromTarget",
            json!({"sessionId": session}),
            parent.as_ref(),
        )?;
        return Ok(());
    }
    for (method, reply) in pending {
        let result = match reply {
            Ok(pending) => pending.await.map(drop),
            Err(error) => Err(error),
        };
        if let Err(error) = result {
            judge_setup_failure(SessionRole::Worker, Step::Optional, &method, session, error)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::browser::Browser;
    use crate::navigation::LoadState;
    use crate::test_hooks::TestSetup;
    use crate::testing::default_auto_reply;
    use crate::transport::memory::{InMemoryControl, InMemoryTransport, SentCommand};

    const URL: &str = "http://127.0.0.1/slow";

    fn page_info(id: &str, subtype: Option<&str>) -> TargetInfo {
        TargetInfo {
            target_id: TargetId::from(id),
            type_: TargetType::Page,
            subtype: subtype.map(str::to_owned),
            url: format!("http://127.0.0.1/{id}"),
            ..TargetInfo::default()
        }
    }

    fn registry(ids: &[&str]) -> Registry {
        let mut registry = Registry::default();
        for id in ids {
            registry.track(page_info(id, None));
        }
        registry
    }

    fn order(registry: &Registry) -> Vec<&str> {
        registry.order.iter().map(TargetId::as_str).collect()
    }

    fn activate(registry: &mut Registry, id: &str, at: Instant) {
        registry.update(page_info(id, None), at);
    }

    #[tokio::test]
    async fn tabs_exclude_prerender_devtools_and_other_types() {
        let mut registry = registry(&["T1"]);
        registry.track(page_info("R1", Some(PRERENDER)));
        registry.track(TargetInfo {
            url: "devtools://devtools/bundled/inspector.html".to_owned(),
            ..page_info("D1", None)
        });
        registry.track(TargetInfo {
            type_: TargetType::Iframe,
            ..page_info("F1", None)
        });
        assert!(!registry.track(page_info("T1", None)), "deduplicated");
        assert_eq!(order(&registry), ["T1"]);
        assert!(registry.targets.contains_key(&TargetId::from("R1")));
        assert!(!registry.targets.contains_key(&TargetId::from("F1")));
    }

    #[tokio::test]
    async fn a_vanish_then_an_activation_binds_the_successor_in_place() {
        let now = Instant::now();
        let mut registry = registry(&["T1", "T2"]);
        registry.track(page_info("R1", Some(PRERENDER)));
        registry.vanish(&TargetId::from("T1"), Some(0), now);
        assert_eq!(
            registry.binding_deadline(&TargetId::from("T1")),
            Some(now + SUCCESSOR_WINDOW)
        );
        activate(&mut registry, "R1", now + Duration::from_millis(400));
        assert_eq!(registry.follow(&TargetId::from("T1")).as_str(), "R1");
        assert_eq!(order(&registry), ["R1", "T2"]);
        assert!(registry.binding_deadline(&TargetId::from("T1")).is_none());
    }

    #[tokio::test]
    async fn an_activation_then_a_vanish_binds_too() {
        let now = Instant::now();
        let mut registry = registry(&["T1", "T2"]);
        registry.track(page_info("R1", Some(PRERENDER)));
        activate(&mut registry, "R1", now);
        assert_eq!(order(&registry), ["T1", "T2", "R1"]);
        registry.order.retain(|id| id.as_str() != "T2");
        registry.vanish(
            &TargetId::from("T2"),
            Some(1),
            now + Duration::from_millis(900),
        );
        assert_eq!(registry.follow(&TargetId::from("T2")).as_str(), "R1");
        assert_eq!(order(&registry), ["T1", "R1"]);
    }

    #[tokio::test]
    async fn events_further_apart_than_the_window_never_bind() {
        let now = Instant::now();
        let mut registry = registry(&["T1"]);
        registry.track(page_info("R1", Some(PRERENDER)));
        registry.vanish(&TargetId::from("T1"), Some(0), now);
        activate(&mut registry, "R1", now + Duration::from_millis(1_500));
        assert_eq!(registry.follow(&TargetId::from("T1")).as_str(), "T1");
        assert!(
            registry.slot(&TargetId::from("T1")).is_ok(),
            "an unbound page that still exists is attached again"
        );
    }

    #[tokio::test]
    async fn two_candidates_mark_the_page_replaced() {
        let now = Instant::now();
        let mut registry = registry(&["T1"]);
        registry.track(page_info("R1", Some(PRERENDER)));
        registry.track(page_info("R2", Some(PRERENDER)));
        activate(&mut registry, "R1", now);
        activate(&mut registry, "R2", now);
        registry.vanish(&TargetId::from("T1"), Some(0), now);
        assert_eq!(registry.follow(&TargetId::from("T1")).as_str(), "T1");
        registry.expire(now + SUCCESSOR_WINDOW);
        let error = registry
            .slot(&TargetId::from("T1"))
            .map(drop)
            .expect_err("T1 was replaced");
        assert_eq!(
            error.to_string(),
            "The current tab was replaced by a prerendered page; use Select Tab"
        );
        assert_eq!(order(&registry), ["R1", "R2"]);
    }

    fn disconnect(registry: &mut Registry, id: &str, at: Instant) {
        let info = TargetInfo {
            subtype: Some(DISCONNECTED.to_owned()),
            ..page_info(id, None)
        };
        registry.update(info, at);
    }

    /// The order Chrome 154 reports an activation in a tab held only through its tab target.
    #[tokio::test]
    async fn an_unheld_tab_turning_disconnected_hands_over_to_the_activated_prerender() {
        let now = Instant::now();
        let mut registry = registry(&["T1", "T2", "T3"]);
        registry.track(page_info("R1", Some(PRERENDER)));
        disconnect(&mut registry, "T2", now);
        assert_eq!(order(&registry), ["T1", "T3"]);
        assert_eq!(
            registry.binding_deadline(&TargetId::from("T2")),
            Some(now + SUCCESSOR_WINDOW)
        );
        activate(&mut registry, "R1", now + Duration::from_millis(5));
        disconnect(&mut registry, "T2", now + Duration::from_millis(10));
        assert_eq!(registry.follow(&TargetId::from("T2")).as_str(), "R1");
        assert_eq!(order(&registry), ["T1", "R1", "T3"]);
        assert!(registry.vanished.is_empty());
        registry.destroyed(&TargetId::from("T2"), now + Duration::from_millis(20));
        assert_eq!(registry.follow(&TargetId::from("T2")).as_str(), "R1");
    }

    #[tokio::test]
    async fn a_disconnected_page_without_a_successor_is_never_attached() {
        let now = Instant::now();
        let mut registry = registry(&["T1"]);
        disconnect(&mut registry, "T1", now);
        registry.expire(now + SUCCESSOR_WINDOW);
        let error = registry
            .slot(&TargetId::from("T1"))
            .map(drop)
            .expect_err("a disconnected page has no frame to attach to");
        assert_eq!(
            error.to_string(),
            "The current browser tab was closed; use Select Tab or New Page"
        );
        assert!(order(&registry).is_empty());
    }

    #[tokio::test]
    async fn without_prerender_pages_nothing_waits_for_a_binding() {
        let now = Instant::now();
        let mut registry = registry(&["T1"]);
        registry.vanish(&TargetId::from("T1"), Some(0), now);
        assert!(registry.binding_deadline(&TargetId::from("T1")).is_none());
        registry.closing.insert(TargetId::from("T1"));
        registry.vanished.clear();
        registry.vanish(&TargetId::from("T1"), Some(0), now);
        assert!(registry.vanished.is_empty(), "our own closes never vanish");
    }

    struct Loading {
        browser: Browser,
        control: InMemoryControl,
    }

    fn load_state_reply(command: &SentCommand, value: &Value) -> Option<Value> {
        let expression = command.params["expression"].as_str().unwrap_or_default();
        match command.method.as_str() {
            "Runtime.evaluate" if expression == LOAD_STATE_PROBE => {
                (!value.is_null()).then(|| json!({"result": {"type": "object", "value": value}}))
            }
            "Browser.getVersion" => Some(json!({"product": "HeadlessChrome/154.0.8037.92"})),
            "Target.setDiscoverTargets" => Some(json!({})),
            "Target.getTargets" => Some(json!({"targetInfos": [
                {"targetId": "T1", "type": "page", "url": URL, "openerId": "T0"},
            ]})),
            "Target.attachToTarget" => Some(json!({"sessionId": "S1"})),
            "Page.getFrameTree" => Some(json!({"frameTree": {
                "frame": {"id": "T1", "loaderId": "L1", "url": URL},
                "childFrames": [],
            }})),
            _ => default_auto_reply(command),
        }
    }

    async fn loading(value: Value) -> Loading {
        let (transport, control) = InMemoryTransport::new();
        control.set_auto_reply(move |command| load_state_reply(command, &value));
        let setup = TestSetup {
            kind: ConnectionKind::Launched,
            headless: true,
            page_load_timeout: Duration::from_secs(30),
            process: None,
            staging: None,
            run_owned_setup: false,
        };
        let browser = Browser::connect_transport(Box::new(transport), setup)
            .await
            .expect("construction");
        Loading { browser, control }
    }

    fn main_state(page: &Page) -> LoadState {
        let state = page.inner.lock_state();
        state.navigation.state(state.frames.main_id())
    }

    async fn seeded(value: Value) -> LoadState {
        let loading = loading(value).await;
        let page = loading
            .browser
            .page(&TargetId::from("T1"))
            .await
            .expect("attach");
        main_state(&page)
    }

    #[tokio::test]
    async fn a_still_loading_popup_is_pending_after_attach() {
        let state = seeded(json!({"s": "loading", "u": URL, "b": URL})).await;
        assert_eq!(
            state,
            LoadState::Loading {
                pending_loader: None
            }
        );
    }

    #[tokio::test]
    async fn loaded_and_initial_blank_documents_are_idle() {
        let complete = seeded(json!({"s": "complete", "u": URL, "b": URL})).await;
        assert_eq!(complete, LoadState::Idle);
        let blank = seeded(json!({"s": "loading", "u": "about:blank", "b": "about:blank"})).await;
        assert_eq!(blank, LoadState::Idle);
    }

    #[tokio::test]
    async fn a_complete_document_on_an_about_blank_base_is_still_pending() {
        let state = seeded(json!({"s": "complete", "u": URL, "b": "about:blank"})).await;
        assert_eq!(
            state,
            LoadState::Loading {
                pending_loader: None
            }
        );
    }

    #[tokio::test]
    async fn a_stop_event_newer_than_the_probe_wins_over_the_seed() {
        let mut loading = loading(Value::Null).await;
        let browser = loading.browser.clone();
        let attaching = tokio::spawn(async move { browser.page(&TargetId::from("T1")).await });
        let probe = loading
            .control
            .wait_for("Runtime.evaluate", Some("S1"))
            .await;
        assert_eq!(probe.params["expression"], LOAD_STATE_PROBE);
        loading.control.emit(
            "Page.frameStoppedLoading",
            Some("S1"),
            json!({"frameId": "T1"}),
        );
        loading.control.reply(
            &probe,
            json!({"result": {"type": "object", "value": {"s": "loading", "u": URL, "b": URL}}}),
        );
        let page = attaching.await.expect("join").expect("attach");
        assert_eq!(main_state(&page), LoadState::Idle);
    }
}
