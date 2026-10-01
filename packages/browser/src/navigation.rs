// Derived from Chromium chrome/test/chromedriver @154.0.8037.92, Copyright The Chromium Authors, BSD-3-Clause; modified by Rheosoph GmbH. See NOTICE.
use std::collections::{HashMap, HashSet};

use tokio::time::Instant;

use crate::event_log::{Event, EventCursor};
use crate::frames::{FrameTree, known_loader};
use crate::types::{
    DialogClosed, FrameId, FrameIdParams, FrameRequestedNavigation, FrameStartedNavigating,
    LifecycleEvent, LoaderId, NavigationType, SessionId,
};

pub(crate) const REQUESTED_EXPIRY: std::time::Duration = std::time::Duration::from_millis(1000);

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum LoadState {
    Idle,
    Requested { at: tokio::time::Instant },
    Loading { pending_loader: Option<LoaderId> },
}

struct FrameLoad {
    state: LoadState,
    settled_seq: u64,
    lifecycle: HashMap<LoaderId, HashSet<String>>,
    parked: Option<LoaderId>,
}

impl FrameLoad {
    fn new() -> Self {
        Self {
            state: LoadState::Idle,
            settled_seq: 0,
            lifecycle: HashMap::new(),
            parked: None,
        }
    }

    fn pending_loader(&self) -> Option<&LoaderId> {
        match &self.state {
            LoadState::Loading { pending_loader } => pending_loader.as_ref(),
            _ => None,
        }
    }

    fn is_pending(&self, now: Instant) -> bool {
        match self.state {
            LoadState::Idle => false,
            LoadState::Requested { at } => now.saturating_duration_since(at) < REQUESTED_EXPIRY,
            LoadState::Loading { .. } => true,
        }
    }

    fn enter(&mut self, state: LoadState) -> bool {
        if self.state == state {
            return false;
        }
        self.state = state;
        true
    }

    fn settle(&mut self, seq: u64, state: LoadState) -> bool {
        self.settled_seq = self.settled_seq.max(seq);
        self.parked = None;
        self.enter(state)
    }

    fn reset(&mut self) -> bool {
        self.parked = None;
        self.enter(LoadState::Idle)
    }

    // A remote-to-local swap back detaches the child session before the parent session attaches
    // the frame and reports the rest of the load (Chrome 154, cdp_semantics e13).
    fn park(&mut self) -> bool {
        self.parked = self.pending_loader().cloned();
        self.enter(LoadState::Idle)
    }

    fn resume(&mut self) -> bool {
        self.parked.take().is_some_and(|loader| {
            self.enter(LoadState::Loading {
                pending_loader: Some(loader),
            })
        })
    }

    fn cancel_request(&mut self) -> bool {
        matches!(self.state, LoadState::Requested { .. }) && self.enter(LoadState::Idle)
    }

    fn keep_lifecycle_of(&mut self, committed: Option<&LoaderId>) {
        let pending = self.pending_loader().cloned();
        self.lifecycle
            .retain(|loader, _| Some(loader) == pending.as_ref() || Some(loader) == committed);
    }
}

pub(crate) struct NavigationTracker {
    frames: HashMap<FrameId, FrameLoad>,
    gone: bool,
}

fn page_session(frames: &FrameTree) -> Option<&SessionId> {
    frames
        .get(frames.main_id())
        .and_then(|main| main.session.as_ref())
}

impl NavigationTracker {
    pub(crate) fn new() -> Self {
        Self {
            frames: HashMap::new(),
            gone: false,
        }
    }

    pub(crate) fn on_event(&mut self, event: &Event, frames: &FrameTree, now: Instant) -> bool {
        match event.method.strip_prefix("Page.") {
            Some(name) => self.on_page_event(name, event, frames, now),
            None => self.on_session_event(event, frames),
        }
    }

    fn on_page_event(
        &mut self,
        name: &str,
        event: &Event,
        frames: &FrameTree,
        now: Instant,
    ) -> bool {
        match name {
            "frameRequestedNavigation" => self.requested(event, now),
            "frameStartedNavigating" => self.started_navigating(event, frames),
            "frameStartedLoading" => self.started_loading(event),
            "frameStoppedLoading" => event
                .decode::<FrameIdParams>()
                .is_some_and(|stopped| self.stop(&stopped.frame_id, event.seq, frames)),
            "loadEventFired" => self.load_event_fired(event, frames),
            "frameClearedScheduledNavigation" => self.cleared_scheduled(event),
            "javascriptDialogClosed" => self.dialog_closed(event),
            "lifecycleEvent" => self.lifecycle_event(event, frames),
            "frameAttached" | "frameDetached" | "frameNavigated" => {
                self.follow_frames(name, event, frames)
            }
            _ => false,
        }
    }

    fn on_session_event(&mut self, event: &Event, frames: &FrameTree) -> bool {
        let page = page_session(frames);
        let on_page = event.session.is_some() && event.session.as_ref() == page;
        match &*event.method {
            "Inspector.targetCrashed" | "Inspector.detached" if on_page => self.mark_gone(),
            "Inspector.targetCrashed" | "Inspector.detached" => {
                event.session.as_ref().is_some_and(|session| {
                    self.apply_to(&frames.frames_of_session(session), FrameLoad::reset)
                })
            }
            "Inspector.targetReloadedAfterCrash" if on_page => {
                std::mem::replace(&mut self.gone, false)
            }
            "Target.detachedFromTarget"
                if page.is_some_and(|page| event.params["sessionId"] == page.as_str()) =>
            {
                self.mark_gone()
            }
            _ => false,
        }
    }

    fn load_mut(&mut self, frame: &FrameId) -> &mut FrameLoad {
        self.frames
            .entry(frame.clone())
            .or_insert_with(FrameLoad::new)
    }

    fn requested(&mut self, event: &Event, now: Instant) -> bool {
        let Some(requested) = event.decode::<FrameRequestedNavigation>() else {
            return false;
        };
        if requested.disposition != "currentTab" {
            return false;
        }
        let load = self.load_mut(&requested.frame_id);
        !matches!(load.state, LoadState::Loading { .. })
            && load.enter(LoadState::Requested { at: now })
    }

    fn started_navigating(&mut self, event: &Event, frames: &FrameTree) -> bool {
        let Some(started) = event.decode::<FrameStartedNavigating>() else {
            return false;
        };
        if matches!(
            started.navigation_type,
            NavigationType::SameDocument | NavigationType::HistorySameDocument
        ) {
            return false;
        }
        let pending_loader = known_loader(&started.loader_id);
        let changed = self
            .load_mut(&started.frame_id)
            .settle(event.seq, LoadState::Loading { pending_loader });
        self.keep_lifecycle(&started.frame_id, frames);
        changed
    }

    fn started_loading(&mut self, event: &Event) -> bool {
        let Some(started) = event.decode::<FrameIdParams>() else {
            return false;
        };
        let load = self.load_mut(&started.frame_id);
        let next = match &load.state {
            LoadState::Loading { .. } => load.state.clone(),
            _ => LoadState::Loading {
                pending_loader: None,
            },
        };
        load.settle(event.seq, next)
    }

    fn stop(&mut self, frame: &FrameId, seq: u64, frames: &FrameTree) -> bool {
        let changed = self.load_mut(frame).settle(seq, LoadState::Idle);
        self.keep_lifecycle(frame, frames);
        changed
    }

    fn load_event_fired(&mut self, event: &Event, frames: &FrameTree) -> bool {
        let root = event
            .session
            .as_ref()
            .and_then(|session| frames.frames_of_session(session).into_iter().next());
        root.is_some_and(|root| self.stop(&root, event.seq, frames))
    }

    fn cleared_scheduled(&mut self, event: &Event) -> bool {
        event
            .decode::<FrameIdParams>()
            .and_then(|cleared| self.frames.get_mut(&cleared.frame_id))
            .is_some_and(FrameLoad::cancel_request)
    }

    fn dialog_closed(&mut self, event: &Event) -> bool {
        let dismissed = event
            .decode::<DialogClosed>()
            .is_some_and(|closed| !closed.result);
        if !dismissed {
            return false;
        }
        self.frames
            .values_mut()
            .fold(false, |changed, load| load.cancel_request() | changed)
    }

    fn lifecycle_event(&mut self, event: &Event, frames: &FrameTree) -> bool {
        let Some(lifecycle) = event.decode::<LifecycleEvent>() else {
            return false;
        };
        let committed = frames.committed_loader(&lifecycle.frame_id);
        let loader = &lifecycle.loader_id;
        let pending = self
            .frames
            .get(&lifecycle.frame_id)
            .and_then(FrameLoad::pending_loader);
        if pending != Some(loader) && committed.as_ref() != Some(loader) {
            return false;
        }
        let load = self.load_mut(&lifecycle.frame_id);
        load.keep_lifecycle_of(committed.as_ref());
        let names = load.lifecycle.entry(lifecycle.loader_id).or_default();
        if lifecycle.name == "init" {
            names.clear();
        }
        names.insert(lifecycle.name)
    }

    fn follow_frames(&mut self, name: &str, event: &Event, frames: &FrameTree) -> bool {
        let reported = match name {
            "frameAttached" => event.params["frameId"].as_str(),
            "frameNavigated" => event.params["frame"]["id"].as_str(),
            _ => None,
        };
        let mut changed = reported.is_some_and(|frame| self.resume(&FrameId::from(frame), frames));
        if let Some(navigated) = event.params["frame"]["id"].as_str() {
            self.keep_lifecycle(&FrameId::from(navigated), frames);
        }
        self.frames.retain(|id, load| {
            let known = frames.get(id).is_some();
            changed |= !known && load.state != LoadState::Idle;
            known
        });
        changed
    }

    fn resume(&mut self, frame: &FrameId, frames: &FrameTree) -> bool {
        let owned_by_live_session = frames.get(frame).is_some() && !frames.in_transit(frame);
        owned_by_live_session && self.frames.get_mut(frame).is_some_and(FrameLoad::resume)
    }

    fn keep_lifecycle(&mut self, frame: &FrameId, frames: &FrameTree) {
        let committed = frames.committed_loader(frame);
        if let Some(load) = self.frames.get_mut(frame) {
            load.keep_lifecycle_of(committed.as_ref());
        }
    }

    fn mark_gone(&mut self) -> bool {
        let mut changed = !std::mem::replace(&mut self.gone, true);
        for load in self.frames.values_mut() {
            changed |= load.reset();
        }
        changed
    }

    pub(crate) fn seed_loading(&mut self, frame: &FrameId, cursor: EventCursor) -> bool {
        if self.gone {
            return false;
        }
        let load = self.load_mut(frame);
        if load.settled_seq >= cursor.0 || matches!(load.state, LoadState::Loading { .. }) {
            return false;
        }
        load.enter(LoadState::Loading {
            pending_loader: None,
        })
    }

    pub(crate) fn reset_frames(&mut self, frames: &[FrameId]) -> bool {
        self.apply_to(frames, FrameLoad::park)
    }

    fn apply_to(&mut self, frames: &[FrameId], apply: fn(&mut FrameLoad) -> bool) -> bool {
        frames.iter().fold(false, |changed, frame| {
            self.frames.get_mut(frame).is_some_and(apply) | changed
        })
    }

    pub(crate) fn is_pending(&self, frame: &FrameId, main: &FrameId, now: Instant) -> bool {
        [frame, main]
            .into_iter()
            .any(|id| self.frames.get(id).is_some_and(|load| load.is_pending(now)))
    }

    pub(crate) fn lifecycle_reached(&self, frame: &FrameId, loader: &LoaderId, name: &str) -> bool {
        self.frames
            .get(frame)
            .and_then(|load| load.lifecycle.get(loader))
            .is_some_and(|names| names.contains(name))
    }

    #[cfg(test)]
    pub(crate) fn state(&self, frame: &FrameId) -> LoadState {
        self.frames
            .get(frame)
            .map_or(LoadState::Idle, |load| load.state.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frames::DetachResolution;
    use crate::types::TargetInfo;
    use serde_json::{Value, json};
    use std::sync::Arc;
    use std::time::Duration;

    const MAIN: &str = "T1";
    const PAGE: &str = "S1";

    fn id(value: &str) -> FrameId {
        FrameId::from(value)
    }

    fn loader(value: &str) -> LoaderId {
        LoaderId::from(value)
    }

    fn loading(value: Option<&str>) -> LoadState {
        LoadState::Loading {
            pending_loader: value.map(loader),
        }
    }

    struct Fixture {
        frames: FrameTree,
        tracker: NavigationTracker,
        seq: u64,
    }

    impl Fixture {
        fn new() -> Self {
            let mut fixture = Self {
                frames: FrameTree::new(id(MAIN), SessionId::from(PAGE)),
                tracker: NavigationTracker::new(),
                seq: 0,
            };
            fixture.navigated(PAGE, MAIN, "L1");
            fixture
        }

        fn apply(&mut self, session: &str, method: &str, params: Value) -> bool {
            self.seq += 1;
            let event = Event {
                seq: self.seq,
                method: method.into(),
                session: Some(SessionId::from(session)),
                params: Arc::new(params),
                received: Instant::now(),
            };
            self.frames.on_event(&event);
            self.tracker.on_event(&event, &self.frames, Instant::now())
        }

        fn on_frame(&mut self, session: &str, method: &str, frame: &str) -> bool {
            self.apply(session, method, json!({"frameId": frame}))
        }

        fn requested(&mut self, frame: &str, disposition: &str) -> bool {
            self.apply(
                PAGE,
                "Page.frameRequestedNavigation",
                json!({"frameId": frame, "reason": "formSubmissionGet", "url": "http://a/next", "disposition": disposition}),
            )
        }

        fn started(&mut self, session: &str, frame: &str, loader: &str, kind: &str) -> bool {
            self.apply(
                session,
                "Page.frameStartedNavigating",
                json!({"frameId": frame, "url": "http://a/next", "loaderId": loader, "navigationType": kind}),
            )
        }

        fn navigated(&mut self, session: &str, frame: &str, loader: &str) -> bool {
            let parent = (frame != MAIN).then_some(MAIN);
            self.apply(
                session,
                "Page.frameNavigated",
                json!({"frame": {"id": frame, "parentId": parent, "loaderId": loader, "url": "http://a/doc"}, "type": "Navigation"}),
            )
        }

        fn lifecycle(&mut self, session: &str, frame: &str, loader: &str, name: &str) -> bool {
            self.apply(
                session,
                "Page.lifecycleEvent",
                json!({"frameId": frame, "loaderId": loader, "name": name, "timestamp": 1.0}),
            )
        }

        fn dialog_closed(&mut self, result: bool) -> bool {
            self.apply(
                PAGE,
                "Page.javascriptDialogClosed",
                json!({"result": result, "userInput": ""}),
            )
        }

        fn local_child(&mut self, frame: &str) {
            self.apply(
                PAGE,
                "Page.frameAttached",
                json!({"frameId": frame, "parentFrameId": MAIN}),
            );
            self.navigated(PAGE, frame, "L-local");
        }

        fn remote_child(&mut self, session: &str, frame: &str) {
            self.seq += 1;
            let info = TargetInfo {
                target_id: frame.into(),
                parent_frame_id: Some(MAIN.into()),
                ..TargetInfo::default()
            };
            self.frames.on_child_session_attached(
                &SessionId::from(PAGE),
                &SessionId::from(session),
                &info,
                self.seq,
            );
            self.navigated(session, frame, "L-remote");
        }

        fn state(&self, frame: &str) -> LoadState {
            self.tracker.state(&id(frame))
        }

        fn pending(&self, frame: &str) -> bool {
            self.tracker
                .is_pending(&id(frame), &id(MAIN), Instant::now())
        }

        fn reached(&self, frame: &str, loader_id: &str, name: &str) -> bool {
            self.tracker
                .lifecycle_reached(&id(frame), &loader(loader_id), name)
        }
    }

    #[tokio::test(start_paused = true)]
    async fn a_request_that_never_starts_expires() {
        let mut fixture = Fixture::new();
        assert!(!fixture.requested(MAIN, "newTab"));
        assert_eq!(fixture.state(MAIN), LoadState::Idle);
        assert!(fixture.requested(MAIN, "currentTab"));
        assert!(matches!(fixture.state(MAIN), LoadState::Requested { .. }));
        assert!(fixture.pending(MAIN));
        tokio::time::advance(REQUESTED_EXPIRY - Duration::from_millis(1)).await;
        assert!(fixture.pending(MAIN));
        tokio::time::advance(Duration::from_millis(1)).await;
        assert!(!fixture.pending(MAIN), "an expired request counts as Idle");
        assert!(matches!(fixture.state(MAIN), LoadState::Requested { .. }));
        assert!(fixture.requested(MAIN, "currentTab"));
        assert!(fixture.pending(MAIN), "a new request restarts the expiry");
    }

    #[test]
    fn a_form_submission_goes_from_requested_to_loading_to_idle() {
        let mut fixture = Fixture::new();
        assert!(fixture.requested(MAIN, "currentTab"));
        assert!(fixture.started(PAGE, MAIN, "L2", "differentDocument"));
        assert_eq!(fixture.state(MAIN), loading(Some("L2")));
        assert!(!fixture.on_frame(PAGE, "Page.frameStartedLoading", MAIN));
        assert!(
            !fixture.requested(MAIN, "currentTab"),
            "a load is never downgraded"
        );
        assert_eq!(fixture.state(MAIN), loading(Some("L2")));
        fixture.navigated(PAGE, MAIN, "L2");
        assert_eq!(fixture.state(MAIN), loading(Some("L2")));
        assert!(fixture.pending(MAIN));
        assert!(fixture.on_frame(PAGE, "Page.frameStoppedLoading", MAIN));
        assert_eq!(fixture.state(MAIN), LoadState::Idle);
        assert!(!fixture.pending(MAIN));
    }

    #[test]
    fn same_document_navigations_start_and_stop() {
        let mut fixture = Fixture::new();
        assert!(!fixture.started(PAGE, MAIN, "L1", "sameDocument"));
        assert!(!fixture.started(PAGE, MAIN, "L1", "historySameDocument"));
        assert_eq!(fixture.state(MAIN), LoadState::Idle);
        assert!(fixture.on_frame(PAGE, "Page.frameStartedLoading", MAIN));
        assert_eq!(fixture.state(MAIN), loading(None));
        fixture.apply(
            PAGE,
            "Page.navigatedWithinDocument",
            json!({"frameId": MAIN, "url": "http://a/doc#x", "navigationType": "fragment"}),
        );
        assert!(fixture.pending(MAIN));
        assert!(fixture.on_frame(PAGE, "Page.frameStoppedLoading", MAIN));
        assert_eq!(fixture.state(MAIN), LoadState::Idle);
        assert_eq!(
            fixture.frames.committed_loader(&id(MAIN)),
            Some(loader("L1"))
        );
    }

    #[test]
    fn lifecycle_events_of_other_loaders_are_ignored() {
        let mut fixture = Fixture::new();
        assert!(fixture.lifecycle(PAGE, MAIN, "L1", "load"));
        assert!(fixture.reached(MAIN, "L1", "load"));
        fixture.started(PAGE, MAIN, "L2", "differentDocument");
        assert!(!fixture.lifecycle(PAGE, MAIN, "L0", "networkIdle"));
        assert!(fixture.lifecycle(PAGE, MAIN, "L1", "networkIdle"));
        assert!(fixture.lifecycle(PAGE, MAIN, "L2", "DOMContentLoaded"));
        assert!(fixture.lifecycle(PAGE, MAIN, "L2", "init"));
        assert!(
            !fixture.reached(MAIN, "L2", "DOMContentLoaded"),
            "init starts the set of a loader again"
        );
        fixture.navigated(PAGE, MAIN, "L2");
        assert!(!fixture.lifecycle(PAGE, MAIN, "L1", "networkAlmostIdle"));
        assert!(!fixture.reached(MAIN, "L1", "load"));
        assert!(!fixture.reached(MAIN, "L1", "networkIdle"));
        assert!(fixture.lifecycle(PAGE, MAIN, "L2", "DOMContentLoaded"));
        assert!(fixture.lifecycle(PAGE, MAIN, "L2", "load"));
        assert!(!fixture.lifecycle(PAGE, MAIN, "L2", "load"));
        assert!(fixture.reached(MAIN, "L2", "load"));
        assert!(fixture.reached(MAIN, "L2", "init"));
        fixture.on_frame(PAGE, "Page.frameStoppedLoading", MAIN);
        assert!(fixture.reached(MAIN, "L2", "load"));
        assert!(!fixture.lifecycle(PAGE, MAIN, "L3", "init"));
        assert!(!fixture.reached("F9", "L2", "load"));
    }

    #[test]
    fn an_aborted_navigation_drops_its_lifecycle() {
        let mut fixture = Fixture::new();
        fixture.started(PAGE, MAIN, "L2", "differentDocument");
        assert!(fixture.lifecycle(PAGE, MAIN, "L2", "init"));
        fixture.on_frame(PAGE, "Page.frameStoppedLoading", MAIN);
        assert!(!fixture.reached(MAIN, "L2", "init"));
        assert!(!fixture.lifecycle(PAGE, MAIN, "L2", "load"));
    }

    #[test]
    fn a_dismissed_dialog_clears_a_requested_navigation() {
        let mut fixture = Fixture::new();
        fixture.local_child("F3");
        fixture.requested(MAIN, "currentTab");
        fixture.on_frame(PAGE, "Page.frameStartedLoading", "F3");
        assert!(!fixture.dialog_closed(true));
        assert!(matches!(fixture.state(MAIN), LoadState::Requested { .. }));
        assert!(fixture.dialog_closed(false));
        assert_eq!(fixture.state(MAIN), LoadState::Idle);
        assert_eq!(
            fixture.state("F3"),
            loading(None),
            "a dismissal never ends a load"
        );
        assert!(!fixture.dialog_closed(false));
    }

    #[test]
    fn a_cleared_scheduled_navigation_only_clears_requested() {
        let mut fixture = Fixture::new();
        fixture.requested(MAIN, "currentTab");
        assert!(fixture.on_frame(PAGE, "Page.frameClearedScheduledNavigation", MAIN));
        assert_eq!(fixture.state(MAIN), LoadState::Idle);
        fixture.on_frame(PAGE, "Page.frameStartedLoading", MAIN);
        assert!(!fixture.on_frame(PAGE, "Page.frameClearedScheduledNavigation", MAIN));
        assert_eq!(fixture.state(MAIN), loading(None));
    }

    #[test]
    fn a_page_crash_makes_the_tracker_gone() {
        let mut fixture = Fixture::new();
        fixture.remote_child("S2", "F2");
        fixture.on_frame(PAGE, "Page.frameStartedLoading", MAIN);
        fixture.on_frame("S2", "Page.frameStartedLoading", "F2");
        assert!(fixture.apply(PAGE, "Inspector.targetCrashed", json!({})));
        assert!(fixture.tracker.gone);
        assert_eq!(fixture.state(MAIN), LoadState::Idle);
        assert_eq!(fixture.state("F2"), LoadState::Idle);
        let cursor = EventCursor(fixture.seq + 1);
        assert!(!fixture.tracker.seed_loading(&id(MAIN), cursor));
        assert!(fixture.apply(PAGE, "Inspector.targetReloadedAfterCrash", json!({})));
        assert!(!fixture.tracker.gone);
        assert!(fixture.apply(
            PAGE,
            "Inspector.detached",
            json!({"reason": "target_closed"})
        ));
        assert!(fixture.tracker.gone);
    }

    #[test]
    fn a_detached_page_session_makes_the_tracker_gone() {
        let mut fixture = Fixture::new();
        assert!(!fixture.apply(
            PAGE,
            "Target.detachedFromTarget",
            json!({"sessionId": "S2", "targetId": "F2"}),
        ));
        assert!(!fixture.tracker.gone);
        assert!(fixture.apply(
            PAGE,
            "Target.detachedFromTarget",
            json!({"sessionId": PAGE, "targetId": MAIN}),
        ));
        assert!(fixture.tracker.gone);
    }

    #[test]
    fn a_child_crash_resets_only_its_frames() {
        let mut fixture = Fixture::new();
        fixture.remote_child("S2", "F2");
        fixture.on_frame(PAGE, "Page.frameStartedLoading", MAIN);
        fixture.on_frame("S2", "Page.frameStartedLoading", "F2");
        assert!(fixture.apply("S2", "Inspector.targetCrashed", json!({})));
        assert!(!fixture.tracker.gone);
        assert_eq!(fixture.state("F2"), LoadState::Idle);
        assert_eq!(fixture.state(MAIN), loading(None));
    }

    #[test]
    fn seed_loading_yields_to_newer_events() {
        let mut fixture = Fixture::new();
        fixture.local_child("F3");
        let cursor = EventCursor(fixture.seq + 1);
        fixture.on_frame(PAGE, "Page.frameStoppedLoading", MAIN);
        assert!(!fixture.tracker.seed_loading(&id(MAIN), cursor));
        assert_eq!(fixture.state(MAIN), LoadState::Idle);
        assert!(fixture.tracker.seed_loading(&id("F3"), cursor));
        assert_eq!(fixture.state("F3"), loading(None));
        assert!(!fixture.tracker.seed_loading(&id("F3"), cursor));
        let later = EventCursor(fixture.seq + 1);
        assert!(fixture.tracker.seed_loading(&id(MAIN), later));
        assert_eq!(fixture.state(MAIN), loading(None));
        fixture.started(PAGE, MAIN, "L2", "differentDocument");
        assert!(
            !fixture
                .tracker
                .seed_loading(&id(MAIN), EventCursor(fixture.seq + 1))
        );
        assert_eq!(fixture.state(MAIN), loading(Some("L2")));
    }

    #[test]
    fn requests_newer_than_the_cursor_do_not_block_the_seed() {
        let mut fixture = Fixture::new();
        let cursor = EventCursor(fixture.seq + 1);
        fixture.requested(MAIN, "currentTab");
        fixture.on_frame(PAGE, "Page.frameClearedScheduledNavigation", MAIN);
        fixture.dialog_closed(false);
        assert_eq!(fixture.state(MAIN), LoadState::Idle);
        assert!(
            fixture.tracker.seed_loading(&id(MAIN), cursor),
            "none of these events says whether the document still loads"
        );
        assert_eq!(fixture.state(MAIN), loading(None));
        assert!(!fixture.on_frame(PAGE, "Page.frameClearedScheduledNavigation", MAIN));
        assert!(fixture.on_frame(PAGE, "Page.frameStoppedLoading", MAIN));
        assert!(!fixture.pending(MAIN));
    }

    #[test]
    fn a_child_detach_resets_the_frames_it_owned() {
        let mut fixture = Fixture::new();
        fixture.remote_child("S2", "F2");
        fixture.apply(
            "S2",
            "Page.frameAttached",
            json!({"frameId": "F4", "parentFrameId": "F2"}),
        );
        fixture.on_frame("S2", "Page.frameStartedLoading", "F2");
        fixture.on_frame("S2", "Page.frameStartedLoading", "F4");
        fixture.on_frame(PAGE, "Page.frameStartedLoading", MAIN);
        let child = SessionId::from("S2");
        assert_eq!(
            fixture.frames.on_child_session_detached(&child),
            DetachResolution::NeedsRoundTrip
        );
        let owned = fixture.frames.frames_of_session(&child);
        assert_eq!(owned, vec![id("F2"), id("F4")]);
        assert!(fixture.tracker.reset_frames(&owned));
        assert_eq!(fixture.state("F2"), LoadState::Idle);
        assert_eq!(fixture.state("F4"), LoadState::Idle);
        assert_eq!(fixture.state(MAIN), loading(None));
        assert!(!fixture.tracker.reset_frames(&owned));
    }

    fn detach_child(fixture: &mut Fixture, session: &str) {
        let child = SessionId::from(session);
        assert_eq!(
            fixture.frames.on_child_session_detached(&child),
            DetachResolution::NeedsRoundTrip
        );
        let owned = fixture.frames.frames_of_session(&child);
        fixture.tracker.reset_frames(&owned);
    }

    #[test]
    fn a_recorded_remote_to_local_swap_back_stays_pending_until_it_stops() {
        let mut fixture = Fixture::new();
        fixture.remote_child("S2", "F2");
        let back = "94C6";
        fixture.requested("F2", "currentTab");
        fixture.started("S2", "F2", back, "differentDocument");
        fixture.on_frame("S2", "Page.frameStartedLoading", "F2");
        detach_child(&mut fixture, "S2");
        assert_eq!(fixture.state("F2"), LoadState::Idle);
        assert!(fixture.frames.in_transit(&id("F2")));
        assert!(fixture.apply(
            PAGE,
            "Page.frameAttached",
            json!({"frameId": "F2", "parentFrameId": MAIN}),
        ));
        assert_eq!(fixture.state("F2"), loading(Some(back)));
        assert!(fixture.lifecycle(PAGE, "F2", back, "init"));
        fixture.navigated(PAGE, "F2", back);
        assert_eq!(
            fixture
                .frames
                .resolve_pending_detach(&SessionId::from("S2")),
            DetachResolution::SwappedBack
        );
        assert!(fixture.lifecycle(PAGE, "F2", back, "load"));
        assert!(fixture.pending("F2"), "the new document is still loading");
        assert!(fixture.on_frame(PAGE, "Page.frameStoppedLoading", "F2"));
        assert!(!fixture.pending("F2"));
        assert!(fixture.reached("F2", back, "init"));
    }

    #[test]
    fn a_removed_or_crashed_child_does_not_resume_its_load() {
        let mut fixture = Fixture::new();
        fixture.remote_child("S2", "F2");
        fixture.started("S2", "F2", "L9", "differentDocument");
        detach_child(&mut fixture, "S2");
        fixture.navigated("S2", "F2", "L9");
        assert_eq!(fixture.state("F2"), LoadState::Idle, "still in transit");
        assert_eq!(
            fixture
                .frames
                .resolve_pending_detach(&SessionId::from("S2")),
            DetachResolution::Removed
        );
        fixture.local_child("F3");
        assert!(!fixture.tracker.frames.contains_key(&id("F2")));

        fixture.remote_child("S4", "F4");
        fixture.started("S4", "F4", "L10", "differentDocument");
        assert!(fixture.apply("S4", "Inspector.targetCrashed", json!({})));
        fixture.navigated("S4", "F4", "L10");
        assert_eq!(fixture.state("F4"), LoadState::Idle);
    }

    #[test]
    fn pending_covers_the_frame_and_the_main_frame() {
        let mut fixture = Fixture::new();
        fixture.local_child("F3");
        fixture.on_frame(PAGE, "Page.frameStartedLoading", "F3");
        assert!(fixture.pending("F3"));
        assert!(
            !fixture.pending(MAIN),
            "child loads are not waited for from the main frame"
        );
        fixture.on_frame(PAGE, "Page.frameStoppedLoading", "F3");
        fixture.on_frame(PAGE, "Page.frameStartedLoading", MAIN);
        assert!(fixture.pending("F3"));
        assert_eq!(fixture.state("F9"), LoadState::Idle);
        assert!(fixture.pending("F9"));
    }

    #[test]
    fn load_event_fired_settles_the_root_of_its_session() {
        let mut fixture = Fixture::new();
        fixture.remote_child("S2", "F2");
        fixture.on_frame(PAGE, "Page.frameStartedLoading", MAIN);
        fixture.on_frame("S2", "Page.frameStartedLoading", "F2");
        assert!(fixture.apply("S2", "Page.loadEventFired", json!({"timestamp": 1.0})));
        assert_eq!(fixture.state("F2"), LoadState::Idle);
        assert_eq!(fixture.state(MAIN), loading(None));
        assert!(fixture.apply(PAGE, "Page.loadEventFired", json!({"timestamp": 2.0})));
        assert_eq!(fixture.state(MAIN), LoadState::Idle);
        assert!(!fixture.apply("S9", "Page.loadEventFired", json!({"timestamp": 3.0})));
    }

    #[test]
    fn removed_frames_are_forgotten_and_swapped_frames_kept() {
        let mut fixture = Fixture::new();
        fixture.local_child("F3");
        fixture.on_frame(PAGE, "Page.frameStartedLoading", MAIN);
        fixture.on_frame(PAGE, "Page.frameStartedLoading", "F3");
        assert!(!fixture.apply(
            PAGE,
            "Page.frameDetached",
            json!({"frameId": "F3", "reason": "swap"}),
        ));
        assert_eq!(fixture.state("F3"), loading(None));
        assert!(fixture.apply(
            PAGE,
            "Page.frameDetached",
            json!({"frameId": "F3", "reason": "remove"}),
        ));
        assert_eq!(fixture.state("F3"), LoadState::Idle);
        assert!(!fixture.tracker.frames.contains_key(&id("F3")));
        assert_eq!(fixture.state(MAIN), loading(None));
    }

    #[test]
    fn a_recorded_local_to_remote_swap_settles_on_the_child_session() {
        let mut fixture = Fixture::new();
        fixture.local_child("F2");
        let child_loader = "5E89";
        fixture.requested("F2", "currentTab");
        fixture.started(PAGE, "F2", child_loader, "differentDocument");
        fixture.on_frame(PAGE, "Page.frameStartedLoading", "F2");
        fixture.on_frame(PAGE, "Page.frameClearedScheduledNavigation", "F2");
        assert_eq!(fixture.state("F2"), loading(Some(child_loader)));
        fixture.seq += 1;
        let info = TargetInfo {
            target_id: "F2".into(),
            parent_frame_id: Some(MAIN.into()),
            ..TargetInfo::default()
        };
        fixture.frames.on_child_session_attached(
            &SessionId::from(PAGE),
            &SessionId::from("S2"),
            &info,
            fixture.seq,
        );
        fixture.navigated("S2", "F2", child_loader);
        fixture.apply(
            PAGE,
            "Page.frameDetached",
            json!({"frameId": "F2", "reason": "swap"}),
        );
        assert!(fixture.pending("F2"));
        assert!(fixture.lifecycle("S2", "F2", child_loader, "load"));
        fixture.apply("S2", "Page.loadEventFired", json!({"timestamp": 1.0}));
        assert_eq!(fixture.state("F2"), LoadState::Idle);
        assert!(!fixture.on_frame("S2", "Page.frameStoppedLoading", "F2"));
        assert!(fixture.reached("F2", child_loader, "load"));
        assert_eq!(
            fixture.frames.session_for_frame(&id("F2")),
            Ok(SessionId::from("S2"))
        );
    }
}
