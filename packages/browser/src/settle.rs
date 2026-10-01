// Derived from Chromium chrome/test/chromedriver @154.0.8037.92, Copyright The Chromium Authors, BSD-3-Clause; modified by Rheosoph GmbH. See NOTICE.
use std::future::Future;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use serde_json::json;
use tokio::time::Instant;

use crate::connection::{with_op_deadline, without_op_deadline};
use crate::dialogs::Dialog;
use crate::error::{BrowserError, ErrorClass};
use crate::frames::{FrameLookup, FrameTree};
use crate::page::{Page, PageState};
use crate::protocol::decode;
use crate::script::ObjectGroup;
use crate::types::{FrameId, LoaderId, NavigateResult, NavigationHistory, SessionId};

const MAX_ATTEMPTS: u8 = 3;
const TOP_FRAME_ATTEMPT: u8 = 2;
const PENDING_POLL: Duration = Duration::from_millis(500);
const UNREPORTED_DOCUMENT_WAIT: Duration = Duration::from_secs(1);
const STOP_LOADING_WAIT: Duration = Duration::from_secs(10);
const SUCCESSOR_WAIT: Duration = Duration::from_secs(1);
const NET_ERROR_PREFIX: &str = "net::ERR_";
const JAVASCRIPT_SCHEME: &str = "javascript:";
const CLOSED_PAGE: &str = "The current browser tab was closed; use Select Tab or New Page";

const CONNECTION_ERRORS: [(i32, &str); 77] = [
    (-100, "CONNECTION_CLOSED"),
    (-101, "CONNECTION_RESET"),
    (-102, "CONNECTION_REFUSED"),
    (-103, "CONNECTION_ABORTED"),
    (-104, "CONNECTION_FAILED"),
    (-105, "NAME_NOT_RESOLVED"),
    (-106, "INTERNET_DISCONNECTED"),
    (-107, "SSL_PROTOCOL_ERROR"),
    (-108, "ADDRESS_INVALID"),
    (-109, "ADDRESS_UNREACHABLE"),
    (-110, "SSL_CLIENT_AUTH_CERT_NEEDED"),
    (-111, "TUNNEL_CONNECTION_FAILED"),
    (-112, "NO_SSL_VERSIONS_ENABLED"),
    (-113, "SSL_VERSION_OR_CIPHER_MISMATCH"),
    (-114, "SSL_RENEGOTIATION_REQUESTED"),
    (-115, "PROXY_AUTH_UNSUPPORTED"),
    (-117, "BAD_SSL_CLIENT_AUTH_CERT"),
    (-118, "CONNECTION_TIMED_OUT"),
    (-119, "HOST_RESOLVER_QUEUE_TOO_LARGE"),
    (-120, "SOCKS_CONNECTION_FAILED"),
    (-121, "SOCKS_CONNECTION_HOST_UNREACHABLE"),
    (-122, "ALPN_NEGOTIATION_FAILED"),
    (-123, "SSL_NO_RENEGOTIATION"),
    (-124, "WINSOCK_UNEXPECTED_WRITTEN_BYTES"),
    (-125, "SSL_DECOMPRESSION_FAILURE_ALERT"),
    (-126, "SSL_BAD_RECORD_MAC_ALERT"),
    (-127, "PROXY_AUTH_REQUESTED"),
    (-130, "PROXY_CONNECTION_FAILED"),
    (-131, "MANDATORY_PROXY_CONFIGURATION_FAILED"),
    (-133, "PRECONNECT_MAX_SOCKET_LIMIT"),
    (-134, "SSL_CLIENT_AUTH_PRIVATE_KEY_ACCESS_DENIED"),
    (-135, "SSL_CLIENT_AUTH_CERT_NO_PRIVATE_KEY"),
    (-136, "PROXY_CERTIFICATE_INVALID"),
    (-137, "NAME_RESOLUTION_FAILED"),
    (-138, "NETWORK_ACCESS_DENIED"),
    (-139, "TEMPORARILY_THROTTLED"),
    (-140, "HTTPS_PROXY_TUNNEL_RESPONSE_REDIRECT"),
    (-141, "SSL_CLIENT_AUTH_SIGNATURE_FAILED"),
    (-142, "MSG_TOO_BIG"),
    (-145, "WS_PROTOCOL_ERROR"),
    (-147, "ADDRESS_IN_USE"),
    (-148, "SSL_HANDSHAKE_NOT_COMPLETED"),
    (-149, "SSL_BAD_PEER_PUBLIC_KEY"),
    (-150, "SSL_PINNED_KEY_NOT_IN_CERT_CHAIN"),
    (-151, "CLIENT_AUTH_CERT_TYPE_UNSUPPORTED"),
    (-153, "SSL_DECRYPT_ERROR_ALERT"),
    (-154, "WS_THROTTLE_QUEUE_TOO_LARGE"),
    (-156, "SSL_SERVER_CERT_CHANGED"),
    (-159, "SSL_UNRECOGNIZED_NAME_ALERT"),
    (-160, "SOCKET_SET_RECEIVE_BUFFER_SIZE_ERROR"),
    (-161, "SOCKET_SET_SEND_BUFFER_SIZE_ERROR"),
    (-162, "SOCKET_RECEIVE_BUFFER_SIZE_UNCHANGEABLE"),
    (-163, "SOCKET_SEND_BUFFER_SIZE_UNCHANGEABLE"),
    (-164, "SSL_CLIENT_AUTH_CERT_BAD_FORMAT"),
    (-166, "ICANN_NAME_COLLISION"),
    (-167, "SSL_SERVER_CERT_BAD_FORMAT"),
    (-168, "CT_STH_PARSING_FAILED"),
    (-169, "CT_STH_INCOMPLETE"),
    (-170, "UNABLE_TO_REUSE_CONNECTION_FOR_PROXY_AUTH"),
    (-171, "CT_CONSISTENCY_PROOF_PARSING_FAILED"),
    (-172, "SSL_OBSOLETE_CIPHER"),
    (-173, "WS_UPGRADE"),
    (-174, "READ_IF_READY_NOT_IMPLEMENTED"),
    (-176, "NO_BUFFER_SPACE"),
    (-177, "SSL_CLIENT_AUTH_NO_COMMON_ALGORITHMS"),
    (-178, "EARLY_DATA_REJECTED"),
    (-179, "WRONG_VERSION_ON_EARLY_DATA"),
    (-180, "TLS13_DOWNGRADE_DETECTED"),
    (-181, "SSL_KEY_USAGE_INCOMPATIBLE"),
    (-182, "INVALID_ECH_CONFIG_LIST"),
    (-183, "ECH_NOT_NEGOTIATED"),
    (-184, "ECH_FALLBACK_CERTIFICATE_INVALID"),
    (-186, "PROXY_UNABLE_TO_CONNECT_TO_DESTINATION"),
    (-187, "PROXY_DELEGATE_CANCELED_CONNECT_REQUEST"),
    (-188, "PROXY_DELEGATE_CANCELED_CONNECT_RESPONSE"),
    (-189, "CONTROL_MSG_TOO_BIG"),
    (-190, "MULTICAST_NOT_ALLOWED"),
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum OpKind {
    Read,
    Script,
    Input,
    Navigation,
    Output,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct OpSpec {
    pub kind: OpKind,
    pub frame_scoped: bool,
}

impl OpSpec {
    pub(crate) const READ: OpSpec = OpSpec {
        kind: OpKind::Read,
        frame_scoped: false,
    };
    pub(crate) const FRAME_READ: OpSpec = OpSpec {
        kind: OpKind::Read,
        frame_scoped: true,
    };
    pub(crate) const SCRIPT: OpSpec = OpSpec {
        kind: OpKind::Script,
        frame_scoped: true,
    };
    pub(crate) const INPUT: OpSpec = OpSpec {
        kind: OpKind::Input,
        frame_scoped: false,
    };
    pub(crate) const NAVIGATION: OpSpec = OpSpec {
        kind: OpKind::Navigation,
        frame_scoped: false,
    };
    pub(crate) const OUTPUT: OpSpec = OpSpec {
        kind: OpKind::Output,
        frame_scoped: false,
    };
}

#[derive(Clone, Debug)]
pub(crate) struct OpAttempt {
    pub index: u8,
    pub frame: FrameId,
    pub deadline: tokio::time::Instant,
    /// Shared by every attempt of one op; released when the op ends.
    pub objects: Arc<ObjectGroup>,
    committed: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

impl OpAttempt {
    fn new(index: u8, frame: FrameId, deadline: Instant, objects: &Arc<ObjectGroup>) -> Self {
        Self {
            index,
            frame,
            deadline,
            objects: objects.clone(),
            committed: Arc::new(AtomicBool::new(false)),
        }
    }

    pub(crate) fn commit(&self) {
        self.committed.store(true, Ordering::Release);
    }

    pub(crate) fn is_committed(&self) -> bool {
        self.committed.load(Ordering::Acquire)
    }

    fn remaining(&self) -> Duration {
        self.deadline.saturating_duration_since(Instant::now())
    }
}

pub(crate) enum OpOutcome<T> {
    Done(T),
    DialogOpened(crate::dialogs::Dialog),
}

impl<T> OpOutcome<T> {
    pub(crate) fn read(self) -> crate::Result<T> {
        match self {
            Self::Done(value) => Ok(value),
            Self::DialogOpened(dialog) => Err(dialog.open_error()),
        }
    }

    pub(crate) fn or_else(self, on_dialog: impl FnOnce(crate::dialogs::Dialog) -> T) -> T {
        match self {
            Self::Done(value) => value,
            Self::DialogOpened(dialog) => on_dialog(dialog),
        }
    }
}

pub(crate) const SCRIPT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

#[derive(Clone, Debug, PartialEq)]
pub enum NavigationOutcome {
    Loaded { url: String },
    SameDocument { url: String },
    Download { url: String },
    DialogOpened,
}

struct Retry {
    error: BrowserError,
    navigation: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Flush {
    Done,
    Pending,
    DeadlineReached,
}

enum Navigated {
    Loaded,
    SameDocument,
    Download,
}

enum NavigateReply {
    Done(Navigated),
    AwaitLoad(LoaderId),
}

enum LoadEnd {
    Loaded,
    Closed,
    Crashed,
}

impl Page {
    pub(crate) async fn run_op<T, F, Fut>(
        &self,
        frame: &FrameId,
        spec: OpSpec,
        mut op: F,
    ) -> crate::Result<OpOutcome<T>>
    where
        F: FnMut(OpAttempt) -> Fut,
        Fut: std::future::Future<Output = crate::Result<T>>,
    {
        let deadline = Instant::now() + self.page_load_timeout();
        self.check_op_start(spec.kind)?;
        let main = self.main_frame_id();
        let objects = Arc::new(ObjectGroup::new("op"));
        let mut current = frame.clone();
        let mut exhausted = None;
        for index in 0..MAX_ATTEMPTS {
            current = attempt_frame(index, spec, current, &main);
            let dialogs = self.dialog_count();
            self.pre_wait(index, spec.kind, &current, deadline).await?;
            let started = Instant::now();
            let attempt = OpAttempt::new(index, current.clone(), deadline, &objects);
            match self
                .race_dialog(&attempt, dialogs, op(attempt.clone()))
                .await
            {
                Ok(outcome) => {
                    return self
                        .settle_after(outcome, spec.kind, &current, started, deadline)
                        .await;
                }
                Err(error) => exhausted = Some(self.after_failure(spec, &attempt, error)?),
            }
        }
        Err(self.exhausted_error(exhausted))
    }

    fn exhausted_error(&self, last: Option<Retry>) -> BrowserError {
        match last {
            Some(Retry {
                error,
                navigation: false,
            }) => error,
            _ => self.renderer_timeout(),
        }
    }

    pub(crate) async fn wait_for_pending_navigations(
        &self,
        frame: &FrameId,
        since: tokio::time::Instant,
        grace: std::time::Duration,
        deadline: tokio::time::Instant,
    ) -> crate::Result<()> {
        let settle_at = since + grace;
        let unreported_until = since + UNREPORTED_DOCUMENT_WAIT;
        loop {
            let mut changes = self.inner.version.subscribe();
            let flush = self.checked_flush(frame, deadline).await?;
            let now = Instant::now();
            let unreported = now < unreported_until;
            let tracked = pending_in(&self.inner.lock_state(), frame, unreported);
            let pending = flush != Flush::Done || tracked;
            if !pending && now >= settle_at {
                return Ok(());
            }
            if now >= deadline {
                return Err(self.stop_loading_after_deadline(frame).await);
            }
            let wake = if pending {
                now + PENDING_POLL
            } else {
                settle_at
            };
            if flush == Flush::Pending {
                let _ = tokio::time::timeout_at(wake.min(deadline), changes.changed()).await;
            } else {
                self.wait_for_settling_change(frame, unreported, tracked, wake.min(deadline))
                    .await;
            }
        }
    }

    pub async fn goto(&self, url: &str) -> crate::Result<NavigationOutcome> {
        if is_javascript_url(url) {
            return Err(BrowserError::InvalidArgument {
                message: "javascript: URLs cannot be opened with Go To URL; use Execute JavaScript"
                    .to_owned(),
            });
        }
        let main = self.main_frame_id();
        let sent = AtomicBool::new(false);
        let result = self
            .run_op(&main, OpSpec::NAVIGATION, |attempt| {
                self.navigate_in(attempt, url, &sent)
            })
            .await;
        match result {
            Ok(OpOutcome::Done(navigated)) => Ok(self.navigation_outcome(navigated, url).await),
            Ok(OpOutcome::DialogOpened(_)) => Ok(NavigationOutcome::DialogOpened),
            Err(error) if sent.load(Ordering::Acquire) && self.is_closed() => {
                match self.successor_url().await {
                    Some(url) => Ok(NavigationOutcome::Loaded { url }),
                    None => Err(error),
                }
            }
            Err(error) => Err(error),
        }
    }

    pub async fn back(&self) -> crate::Result<()> {
        self.traverse_history(-1).await
    }

    pub async fn forward(&self) -> crate::Result<()> {
        self.traverse_history(1).await
    }

    pub async fn reload(&self) -> crate::Result<()> {
        let main = self.main_frame_id();
        self.run_op(&main, OpSpec::NAVIGATION, |attempt| async move {
            self.session()
                .request("Page.reload", json!({}), Some(attempt.remaining()))
                .await
                .map(drop)
        })
        .await
        .map(drop)
    }

    fn check_op_start(&self, kind: OpKind) -> crate::Result<()> {
        let state = self.inner.lock_state();
        if state.closed {
            return Err(BrowserError::NoSuchPage {
                message: CLOSED_PAGE.to_owned(),
            });
        }
        if let Some(dialog) = state.dialog.open() {
            return Err(dialog.open_error());
        }
        if state.crashed && kind != OpKind::Navigation {
            return Err(self.crashed_error());
        }
        Ok(())
    }

    async fn pre_wait(
        &self,
        index: u8,
        kind: OpKind,
        frame: &FrameId,
        deadline: Instant,
    ) -> crate::Result<()> {
        let reloads_a_crash =
            index == 0 && kind == OpKind::Navigation && self.inner.lock_state().crashed;
        if reloads_a_crash {
            return Ok(());
        }
        self.wait_for_pending_navigations(frame, Instant::now(), Duration::ZERO, deadline)
            .await
    }

    async fn checked_flush(&self, frame: &FrameId, deadline: Instant) -> crate::Result<Flush> {
        self.check_settle_blockers()?;
        let flush = self.flush(frame, deadline).await?;
        self.check_settle_blockers()?;
        Ok(flush)
    }

    async fn wait_for_settling_change(
        &self,
        frame: &FrameId,
        unreported: bool,
        pending_now: bool,
        wake: Instant,
    ) {
        self.wait_for_state(wake, |state| {
            (pending_in(state, frame, unreported) != pending_now).then_some(())
        })
        .await;
    }

    async fn race_dialog<T>(
        &self,
        attempt: &OpAttempt,
        dialogs: u64,
        op: impl Future<Output = crate::Result<T>>,
    ) -> crate::Result<OpOutcome<T>> {
        tokio::select! {
            biased;
            result = with_op_deadline(attempt.deadline, op) => result.map(OpOutcome::Done),
            dialog = self.dialog_before_commit(dialogs, attempt) => Ok(OpOutcome::DialogOpened(dialog)),
        }
    }

    async fn dialog_before_commit(&self, dialogs: u64, attempt: &OpAttempt) -> Dialog {
        let dialog = self.dialog_opened_after(dialogs).await;
        if attempt.is_committed() {
            std::future::pending::<()>().await;
        }
        dialog
    }

    fn after_failure(
        &self,
        spec: OpSpec,
        attempt: &OpAttempt,
        error: BrowserError,
    ) -> Result<Retry, BrowserError> {
        let committed_input = spec.kind == OpKind::Input && attempt.is_committed();
        match error.class() {
            ErrorClass::NoSuchExecutionContext
            | ErrorClass::AbortedByNavigation
            | ErrorClass::FrameInTransit => navigation_failure(spec.kind, committed_input, error),
            ErrorClass::Timeout if committed_input || spec.kind == OpKind::Output => Err(error),
            ErrorClass::Timeout => Ok(Retry {
                error,
                navigation: false,
            }),
            ErrorClass::Fatal => Err(error),
            ErrorClass::SessionGone if self.page_session_gone() => Err(error),
            ErrorClass::SessionGone => {
                let detaching = self.subtree_session_detaching(&attempt.frame);
                self.navigation_hint(spec.kind, committed_input, &attempt.frame, error, detaching)
            }
            ErrorClass::NodeGone => Err(BrowserError::StaleElement),
            _ => self.navigation_hint(spec.kind, committed_input, &attempt.frame, error, false),
        }
    }

    fn navigation_hint(
        &self,
        kind: OpKind,
        committed_input: bool,
        frame: &FrameId,
        error: BrowserError,
        detaching: bool,
    ) -> Result<Retry, BrowserError> {
        let retry = kind != OpKind::Script
            && !committed_input
            && !ends_the_op(&error)
            && (detaching || self.navigation_in_flight(frame));
        if retry {
            Ok(Retry {
                error,
                navigation: true,
            })
        } else {
            Err(error)
        }
    }

    async fn settle_after<T>(
        &self,
        outcome: OpOutcome<T>,
        kind: OpKind,
        frame: &FrameId,
        started: Instant,
        deadline: Instant,
    ) -> crate::Result<OpOutcome<T>> {
        if self.is_closed() {
            return Ok(outcome);
        }
        let settled = async {
            self.wait_for_crash_recovery(kind, deadline).await?;
            self.wait_for_pending_navigations(frame, started, Duration::ZERO, deadline)
                .await
        }
        .await;
        match settled {
            Err(error)
                if matches!(outcome, OpOutcome::Done(_))
                    && !matches!(error, BrowserError::DialogOpen { .. })
                    && !self.is_closed() =>
            {
                Err(error)
            }
            _ => Ok(outcome),
        }
    }

    async fn wait_for_crash_recovery(&self, kind: OpKind, deadline: Instant) -> crate::Result<()> {
        if kind != OpKind::Navigation {
            return Ok(());
        }
        let recovered = self.wait_for_state(deadline, |state| {
            (!state.crashed || state.closed).then_some(())
        });
        self.unless_disconnected(recovered).await.map(drop)
    }

    /// A lost connection leaves the page state as it was, so page-state waits race it.
    async fn unless_disconnected<T>(&self, wait: impl Future<Output = T>) -> crate::Result<T> {
        let connection = &self.inner.connection;
        tokio::select! {
            biased;
            found = wait => Ok(found),
            () = connection.closed() => Err(BrowserError::Disconnected {
                reason: connection.closed_reason().unwrap_or_default(),
            }),
        }
    }

    async fn flush(&self, frame: &FrameId, deadline: Instant) -> crate::Result<Flush> {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Ok(Flush::DeadlineReached);
        }
        let (session, on_page) = self.flush_session(frame);
        let sent = self.inner.connection.session(Some(session)).enqueue(
            "Runtime.evaluate",
            json!({"expression": "1"}),
            Some(remaining),
        );
        let result = match sent {
            Ok(reply) => tokio::select! {
                result = reply => result.map(drop),
                () = self.settle_blocked(deadline) => return Ok(Flush::Pending),
            },
            Err(error) => Err(error),
        };
        flush_outcome(result, on_page, deadline)
    }

    fn flush_session(&self, frame: &FrameId) -> (SessionId, bool) {
        let page = &self.inner.session;
        match self.inner.lock_state().frames.session_for_frame(frame) {
            Ok(session) => {
                let on_page = session == *page;
                (session, on_page)
            }
            Err(_) => (page.clone(), true),
        }
    }

    async fn settle_blocked(&self, deadline: Instant) {
        let blocked = self
            .wait_for_state(deadline, |state| blocks_settling(state).then_some(()))
            .await;
        if blocked.is_none() {
            std::future::pending::<()>().await;
        }
    }

    fn check_settle_blockers(&self) -> crate::Result<()> {
        let state = self.inner.lock_state();
        if let Some(dialog) = state.dialog.open() {
            return Err(dialog.open_error());
        }
        if state.closed {
            return Err(BrowserError::TargetClosed {
                method: "Runtime.evaluate".to_owned(),
            });
        }
        if state.crashed {
            return Err(self.crashed_error());
        }
        Ok(())
    }

    async fn stop_loading_after_deadline(&self, frame: &FrameId) -> BrowserError {
        let page = self.session();
        match without_op_deadline(|| page.send_nowait("Page.stopLoading", json!({}))) {
            Ok(_) => {}
            Err(error @ BrowserError::Disconnected { .. }) => return error,
            Err(error) => {
                tracing::debug!(target_id = %self.target_id(), %error, "Page.stopLoading after the page load timeout was not sent");
            }
        }
        let stopped_by = Instant::now() + STOP_LOADING_WAIT;
        let stopped = self
            .wait_until(stopped_by, |state| {
                (blocks_settling(state) || !settling(state, frame, Instant::now())).then_some(())
            })
            .await;
        match stopped {
            Err(error) => error,
            Ok(_) => self.renderer_timeout(),
        }
    }

    async fn wait_until<T>(
        &self,
        deadline: Instant,
        lookup: impl Fn(&PageState) -> Option<T>,
    ) -> crate::Result<Option<T>> {
        let polled = async {
            loop {
                let slice = (Instant::now() + PENDING_POLL).min(deadline);
                let found = self.wait_for_state(slice, &lookup).await;
                if found.is_some() || Instant::now() >= deadline {
                    return found;
                }
            }
        };
        self.unless_disconnected(polled).await
    }

    fn navigation_in_flight(&self, frame: &FrameId) -> bool {
        pending_in(&self.inner.lock_state(), frame, true)
    }

    fn page_session_gone(&self) -> bool {
        self.is_closed()
            || self
                .inner
                .connection
                .session_info(&self.inner.session)
                .is_some_and(|info| info.detached)
    }

    /// The connection fails the commands of a detached child session before the target hook
    /// parks its frames, so a session the registry marks detached or no longer knows is in transit.
    fn subtree_session_detaching(&self, frame: &FrameId) -> bool {
        let sessions = subtree_sessions(&self.inner.lock_state().frames, frame);
        match sessions {
            Ok(sessions) => sessions.iter().any(|session| {
                self.inner
                    .connection
                    .session_info(session)
                    .is_none_or(|info| info.detached)
            }),
            Err(_) => true,
        }
    }

    fn crashed_error(&self) -> BrowserError {
        BrowserError::TargetCrashed {
            target_id: self.target_id().to_string(),
        }
    }

    fn renderer_timeout(&self) -> BrowserError {
        BrowserError::RendererTimeout {
            seconds: self.page_load_timeout().as_secs_f64(),
        }
    }

    async fn navigate_in(
        &self,
        attempt: OpAttempt,
        url: &str,
        sent: &AtomicBool,
    ) -> crate::Result<Navigated> {
        let (previous, crashed) = {
            let state = self.inner.lock_state();
            (state.frames.committed_loader(&attempt.frame), state.crashed)
        };
        let reply = self.session().enqueue(
            "Page.navigate",
            json!({"url": url}),
            Some(attempt.remaining()),
        )?;
        sent.store(true, Ordering::Release);
        let navigated: NavigateResult = decode("Page.navigate", reply.await?.result)?;
        match classify_navigate_reply(navigated, url, crashed)? {
            NavigateReply::Done(navigated) => Ok(navigated),
            NavigateReply::AwaitLoad(loader) => self
                .wait_for_load(&attempt, &loader, previous)
                .await
                .map(|()| Navigated::Loaded),
        }
    }

    async fn wait_for_load(
        &self,
        attempt: &OpAttempt,
        loader: &LoaderId,
        previous: Option<LoaderId>,
    ) -> crate::Result<()> {
        let main = &attempt.frame;
        let end = self
            .wait_until(attempt.deadline, |state| {
                load_end(state, main, loader, previous.as_ref())
            })
            .await?;
        match end {
            Some(LoadEnd::Loaded) => Ok(()),
            Some(LoadEnd::Closed) => Err(BrowserError::TargetClosed {
                method: "Page.navigate".to_owned(),
            }),
            Some(LoadEnd::Crashed) => Err(self.crashed_error()),
            None => Err(self.stop_loading_after_deadline(main).await),
        }
    }

    async fn navigation_outcome(&self, navigated: Navigated, url: &str) -> NavigationOutcome {
        match navigated {
            Navigated::Download => NavigationOutcome::Download {
                url: url.to_owned(),
            },
            Navigated::SameDocument => NavigationOutcome::SameDocument {
                url: self.main_url(),
            },
            Navigated::Loaded => {
                let successor = if self.is_closed() {
                    self.successor_url().await
                } else {
                    None
                };
                NavigationOutcome::Loaded {
                    url: successor.unwrap_or_else(|| self.main_url()),
                }
            }
        }
    }

    async fn successor_url(&self) -> Option<String> {
        let browser = self.browser()?;
        let successor = tokio::time::timeout(SUCCESSOR_WAIT, browser.page(self.target_id()))
            .await
            .ok()?
            .ok()?;
        (successor.target_id() != self.target_id()).then(|| successor.main_url())
    }

    async fn traverse_history(&self, step: i64) -> crate::Result<()> {
        let main = self.main_frame_id();
        self.run_op(&main, OpSpec::NAVIGATION, |attempt| {
            self.history_step_in(attempt, step)
        })
        .await
        .map(drop)
    }

    async fn history_step_in(&self, attempt: OpAttempt, step: i64) -> crate::Result<()> {
        let session = self.session();
        let history: NavigationHistory = decode(
            "Page.getNavigationHistory",
            session.send("Page.getNavigationHistory", json!({})).await?,
        )?;
        let entry = history
            .current_index
            .checked_add(step)
            .and_then(|index| usize::try_from(index).ok())
            .and_then(|index| history.entries.get(index));
        let Some(entry) = entry else {
            return if self.inner.lock_state().crashed {
                Err(self.crashed_error())
            } else {
                Ok(())
            };
        };
        session
            .request(
                "Page.navigateToHistoryEntry",
                json!({"entryId": entry.id}),
                Some(attempt.remaining()),
            )
            .await
            .map(drop)
    }
}

fn attempt_frame(index: u8, spec: OpSpec, current: FrameId, main: &FrameId) -> FrameId {
    if index == TOP_FRAME_ATTEMPT && spec.frame_scoped {
        main.clone()
    } else {
        current
    }
}

fn navigation_failure(
    kind: OpKind,
    committed_input: bool,
    error: BrowserError,
) -> Result<Retry, BrowserError> {
    if kind == OpKind::Script {
        Err(BrowserError::NavigationInterrupted)
    } else if committed_input {
        Err(error)
    } else {
        Ok(Retry {
            error,
            navigation: true,
        })
    }
}

fn classify_navigate_reply(
    reply: NavigateResult,
    url: &str,
    crashed: bool,
) -> crate::Result<NavigateReply> {
    let error_text = reply.error_text.filter(|text| !text.is_empty());
    if let Some(error) = error_text.as_ref().filter(|text| is_connection_error(text)) {
        return Err(BrowserError::NavigationFailed {
            url: url.to_owned(),
            error: error.clone(),
        });
    }
    let loader = reply.loader_id.filter(|loader| !loader.as_str().is_empty());
    Ok(match loader {
        _ if reply.is_download => NavigateReply::Done(Navigated::Download),
        _ if error_text.is_some() => NavigateReply::Done(Navigated::Loaded),
        None => NavigateReply::Done(Navigated::SameDocument),
        Some(_) if crashed => NavigateReply::Done(Navigated::Loaded),
        Some(loader) => NavigateReply::AwaitLoad(loader),
    })
}

fn ends_the_op(error: &BrowserError) -> bool {
    matches!(
        error,
        BrowserError::RendererTimeout { .. }
            | BrowserError::NavigationFailed { .. }
            | BrowserError::DialogOpen { .. }
            | BrowserError::NoSuchPage { .. }
    )
}

fn flush_outcome(
    result: crate::Result<()>,
    on_page: bool,
    deadline: Instant,
) -> crate::Result<Flush> {
    let Err(error) = result else {
        return Ok(Flush::Done);
    };
    match error.class() {
        ErrorClass::NoSuchExecutionContext
        | ErrorClass::AbortedByNavigation
        | ErrorClass::FrameInTransit => Ok(Flush::Pending),
        ErrorClass::SessionGone if !on_page => Ok(Flush::Pending),
        ErrorClass::Timeout if Instant::now() >= deadline => Ok(Flush::DeadlineReached),
        ErrorClass::Timeout => Ok(Flush::Pending),
        _ => Err(error),
    }
}

fn subtree_sessions(frames: &FrameTree, frame: &FrameId) -> Result<Vec<SessionId>, FrameLookup> {
    let mut sessions: Vec<SessionId> = Vec::new();
    for id in std::iter::once(frame.clone()).chain(frames.descendants_preorder(frame)) {
        match frames.session_for_frame(&id) {
            Ok(session) if !sessions.contains(&session) => sessions.push(session),
            Err(FrameLookup::InTransit) => return Err(FrameLookup::InTransit),
            Ok(_) | Err(FrameLookup::Missing) => {}
        }
    }
    Ok(sessions)
}

fn blocks_settling(state: &PageState) -> bool {
    state.dialog.open().is_some() || state.closed || state.crashed
}

fn settling(state: &PageState, frame: &FrameId, now: Instant) -> bool {
    let frames = &state.frames;
    state.navigation.is_pending(frame, frames.main_id(), now) || frames.in_transit(frame)
}

/// Every op on a frame fails with `FrameInTransit` while its document is unreported (loader
/// unknown after an OOPIF attach or swap), so waits may count that as pending too. They do so only
/// for a short grace: a frame that never reports a document must not hold ops until the timeout.
fn pending_in(state: &PageState, frame: &FrameId, unreported_counts: bool) -> bool {
    settling(state, frame, Instant::now())
        || (unreported_counts
            && state
                .frames
                .get(frame)
                .is_some_and(|node| node.loader_id.is_none()))
}

fn load_end(
    state: &PageState,
    main: &FrameId,
    loader: &LoaderId,
    previous: Option<&LoaderId>,
) -> Option<LoadEnd> {
    if state.closed {
        return Some(LoadEnd::Closed);
    }
    if state.crashed {
        return Some(LoadEnd::Crashed);
    }
    let navigation = &state.navigation;
    let committed = state.frames.committed_loader(main);
    let idle_after_commit = committed.is_some()
        && committed.as_ref() != previous
        && !navigation.is_pending(main, main, Instant::now());
    (navigation.lifecycle_reached(main, loader, "load") || idle_after_commit)
        .then_some(LoadEnd::Loaded)
}

/// URL parsing (WHATWG and GURL) drops leading C0 controls and spaces and removes every tab and
/// newline before it reads the scheme.
fn is_javascript_url(url: &str) -> bool {
    let scheme: String = url
        .trim_start_matches(|c: char| c <= ' ')
        .chars()
        .filter(|c| !matches!(c, '\t' | '\n' | '\r'))
        .take(JAVASCRIPT_SCHEME.len())
        .collect();
    scheme.eq_ignore_ascii_case(JAVASCRIPT_SCHEME)
}

pub(crate) fn is_connection_error(error_text: &str) -> bool {
    error_text
        .strip_prefix(NET_ERROR_PREFIX)
        .is_some_and(|name| CONNECTION_ERRORS.iter().any(|(_, known)| *known == name))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn connection_errors_are_the_net_100_to_199_range() {
        assert!(
            CONNECTION_ERRORS
                .iter()
                .all(|(code, _)| (-199..=-100).contains(code))
        );
        let mut codes: Vec<i32> = CONNECTION_ERRORS.iter().map(|(code, _)| *code).collect();
        codes.dedup();
        assert_eq!(codes.len(), CONNECTION_ERRORS.len());
        for fails in [
            "net::ERR_NAME_NOT_RESOLVED",
            "net::ERR_CONNECTION_REFUSED",
            "net::ERR_INTERNET_DISCONNECTED",
            "net::ERR_SSL_PROTOCOL_ERROR",
            "net::ERR_MULTICAST_NOT_ALLOWED",
        ] {
            assert!(is_connection_error(fails), "{fails}");
        }
        for lands in [
            "net::ERR_ABORTED",
            "net::ERR_TIMED_OUT",
            "net::ERR_BLOCKED_BY_CLIENT",
            "net::ERR_CERT_AUTHORITY_INVALID",
            "net::ERR_HTTP_RESPONSE_CODE_FAILURE",
            "net::ERR_DNS_TIMED_OUT",
            "net::ERR_UNSAFE_PORT",
            "NAME_NOT_RESOLVED",
            "net::ERR_name_not_resolved",
            "",
        ] {
            assert!(!is_connection_error(lands), "{lands}");
        }
    }

    #[test]
    fn subtree_sessions_cover_child_local_roots_and_report_transit() {
        let (main, page, child) = (
            FrameId::from("T1"),
            SessionId::from("S1"),
            SessionId::from("S2"),
        );
        let mut frames = FrameTree::new(main.clone(), page.clone());
        let info = crate::types::TargetInfo {
            target_id: "F2".into(),
            type_: crate::types::TargetType::Iframe,
            parent_frame_id: Some(main.clone()),
            attached: true,
            ..crate::types::TargetInfo::default()
        };
        frames.on_child_session_attached(&page, &child, &info, 1);
        assert_eq!(
            subtree_sessions(&frames, &main),
            Ok(vec![page.clone(), child.clone()])
        );
        assert_eq!(
            subtree_sessions(&frames, &FrameId::from("F2")),
            Ok(vec![child.clone()])
        );
        assert_eq!(
            subtree_sessions(&frames, &FrameId::from("gone")),
            Ok(Vec::new())
        );
        frames.on_child_session_detached(&child);
        assert_eq!(
            subtree_sessions(&frames, &main),
            Err(FrameLookup::InTransit)
        );
    }

    #[test]
    fn javascript_urls_are_recognised_case_insensitively() {
        assert!(is_javascript_url("javascript:void(0)"));
        assert!(is_javascript_url("  JavaScript:alert(1)"));
        assert!(is_javascript_url("java\tscript:alert(1)"));
        assert!(is_javascript_url("jav\nascript:alert(1)"));
        assert!(is_javascript_url("javascript\r:alert(1)"));
        assert!(is_javascript_url("\u{1}\u{1f} javascript:alert(1)"));
        assert!(is_javascript_url("javascript://[x%0aalert(1)"));
        assert!(!is_javascript_url("https://example.com/javascript:"));
        assert!(!is_javascript_url("java"));
        assert!(!is_javascript_url("java script:alert(1)"));
    }
}
