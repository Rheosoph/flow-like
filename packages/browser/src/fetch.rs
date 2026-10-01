use std::collections::HashSet;
use std::sync::{Arc, Weak};

use serde_json::{Value, json};

use crate::connection::{MethodMatch, PendingReply, RouteFilter, RouteReceiver, SessionScope};
use crate::event_log::Event;
use crate::page::{Page, PageInner};
use crate::session::Session;
use crate::types::SessionId;

pub type AuthDecider =
    std::sync::Arc<dyn Fn(&serde_json::Value, bool) -> serde_json::Value + Send + Sync>;

pub(crate) struct FetchState {
    auth: Option<BasicAuth>,
    route: Option<tokio::task::AbortHandle>,
    updates: Arc<tokio::sync::Mutex<()>>,
}

struct BasicAuth {
    url_pattern: String,
    decider: AuthDecider,
}

impl FetchState {
    pub(crate) fn new() -> Self {
        Self {
            auth: None,
            route: None,
            updates: Arc::new(tokio::sync::Mutex::new(())),
        }
    }

    fn decider(&self) -> Option<AuthDecider> {
        self.auth.as_ref().map(|auth| auth.decider.clone())
    }
}

impl Drop for FetchState {
    fn drop(&mut self) {
        if let Some(route) = self.route.take() {
            route.abort();
        }
    }
}

pub struct Observation {
    pub backlog: Vec<crate::event_log::Event>,
    pub route: crate::connection::RouteReceiver,
}

impl Page {
    pub async fn enable_basic_auth(&self, origin: &str, decider: AuthDecider) -> crate::Result<()> {
        let updates = self.inner.lock_state().fetch.updates.clone();
        let _serialized = updates.lock().await;
        self.serve_fetch_route();
        let url_pattern = format!("{origin}/*");
        let params = enable_params(&url_pattern);
        let previous = self.inner.lock_state().fetch.auth.replace(BasicAuth {
            url_pattern,
            decider,
        });
        let enabled = self.send_fetch_enable(&params).await;
        if enabled.is_err() {
            self.inner.lock_state().fetch.auth = previous;
        }
        enabled
    }

    async fn send_fetch_enable(&self, params: &Value) -> crate::Result<()> {
        let children: Vec<SessionId> = self
            .inner
            .lock_state()
            .sessions
            .iter()
            .filter(|session| **session != self.inner.session)
            .cloned()
            .collect();
        let confirmation = self
            .session()
            .enqueue("Fetch.enable", params.clone(), None)?;
        let child_replies: Vec<PendingReply> = children
            .into_iter()
            .filter_map(|child| {
                let session = self.inner.connection.session(Some(child.clone()));
                session
                    .enqueue("Fetch.enable", params.clone(), None)
                    .inspect_err(|error| {
                        tracing::debug!(session = %child, %error, "Fetch.enable was not sent to a child session");
                    })
                    .ok()
            })
            .collect();
        if !child_replies.is_empty() {
            tokio::spawn(log_child_failures(child_replies));
        }
        confirmation.await.map(drop)
    }

    pub fn observe(&self, methods: Vec<MethodMatch>) -> Observation {
        let route = self.inner.connection.route(RouteFilter {
            sessions: SessionScope::Exactly(self.inner.session.clone()),
            methods: methods.clone(),
        });
        let backlog = self.inner.lock_state().console.snapshot();
        let backlog = backlog
            .into_iter()
            .filter(|event| methods.iter().any(|pattern| pattern.matches(&event.method)))
            .collect();
        Observation { backlog, route }
    }

    fn serve_fetch_route(&self) {
        if self.inner.lock_state().fetch.route.is_some() {
            return;
        }
        let route = self.inner.connection.route(RouteFilter {
            sessions: SessionScope::WithDescendants(self.inner.session.clone()),
            methods: vec![MethodMatch::Prefix("Fetch.")],
        });
        let task = tokio::spawn(answer_paused_requests(route, Arc::downgrade(&self.inner)));
        self.inner.lock_state().fetch.route = Some(task.abort_handle());
    }
}

pub(crate) fn session_commands(page: &Page) -> Vec<(String, serde_json::Value)> {
    let state = page.inner.lock_state();
    state
        .fetch
        .auth
        .as_ref()
        .map(|auth| ("Fetch.enable".to_owned(), enable_params(&auth.url_pattern)))
        .into_iter()
        .collect()
}

fn enable_params(url_pattern: &str) -> Value {
    json!({"handleAuthRequests": true, "patterns": [{"urlPattern": url_pattern}]})
}

async fn log_child_failures(replies: Vec<PendingReply>) {
    for reply in replies {
        if let Err(error) = reply.await {
            tracing::debug!(%error, "Fetch.enable failed on a child session");
        }
    }
}

async fn answer_paused_requests(mut route: RouteReceiver, page: Weak<PageInner>) {
    let mut attempted = HashSet::new();
    while let Some(event) = route.recv().await {
        let Some(inner) = page.upgrade() else {
            return;
        };
        let Some(session) = event.session.clone() else {
            continue;
        };
        let Some(answer) = answer(&inner, &event, &mut attempted) else {
            continue;
        };
        let session = inner.connection.session(Some(session));
        drop(inner);
        tokio::spawn(send_answer(session, answer));
    }
}

enum Answer {
    Continue(Value),
    Authenticate(Value),
}

async fn send_answer(session: Session, answer: Answer) {
    let (method, params) = match answer {
        Answer::Continue(params) => ("Fetch.continueRequest", params),
        Answer::Authenticate(params) => ("Fetch.continueWithAuth", params),
    };
    if let Err(error) = session.send(method, params).await {
        tracing::debug!(method, %error, "a paused request could not be answered");
    }
}

fn answer(inner: &PageInner, event: &Event, attempted: &mut HashSet<String>) -> Option<Answer> {
    let method = &*event.method;
    if !matches!(method, "Fetch.requestPaused" | "Fetch.authRequired") {
        return None;
    }
    let Some(request_id) = event.params["requestId"]
        .as_str()
        .filter(|request_id| !request_id.is_empty())
    else {
        tracing::debug!(
            method,
            "a paused request without a requestId cannot be answered"
        );
        return None;
    };
    if method == "Fetch.requestPaused" {
        return Some(Answer::Continue(json!({"requestId": request_id})));
    }
    let first = attempted.insert(request_id.to_owned());
    let decider = inner.lock_state().fetch.decider();
    let response = match decider {
        Some(decider) => decider(&event.params["authChallenge"], first),
        None => json!({"response": "CancelAuth"}),
    };
    Some(Answer::Authenticate(
        json!({"requestId": request_id, "authChallengeResponse": response}),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{PageHarness, default_auto_reply};
    use std::time::Duration;

    fn allow_all() -> AuthDecider {
        Arc::new(|_, _| json!({"response": "Default"}))
    }

    fn pattern_commands(url_pattern: &str) -> Vec<(String, Value)> {
        vec![("Fetch.enable".to_owned(), enable_params(url_pattern))]
    }

    fn withhold_fetch_enable(harness: &PageHarness) {
        harness.control.set_auto_reply(|command| {
            (command.method != "Fetch.enable")
                .then(|| default_auto_reply(command))
                .flatten()
        });
    }

    fn start_enable(
        harness: &PageHarness,
        origin: &'static str,
    ) -> tokio::task::JoinHandle<crate::Result<()>> {
        let page = harness.page.clone();
        tokio::spawn(async move { page.enable_basic_auth(origin, allow_all()).await })
    }

    async fn answered_enable(
        harness: &mut PageHarness,
        origin: &'static str,
        accept: bool,
    ) -> crate::Result<()> {
        let enabling = start_enable(harness, origin);
        let enable = harness.control.wait_for("Fetch.enable", Some("S1")).await;
        if accept {
            harness.control.reply(&enable, json!({}));
        } else {
            harness.control.reply_error(&enable, -32602, "rejected");
        }
        enabling.await.unwrap()
    }

    #[tokio::test]
    async fn a_failed_fetch_enable_keeps_the_previous_auth() {
        let mut harness = PageHarness::new().await;
        withhold_fetch_enable(&harness);
        let rejected = answered_enable(&mut harness, "https://one.test", false).await;
        assert!(rejected.is_err(), "{rejected:?}");
        assert!(session_commands(&harness.page).is_empty());
        answered_enable(&mut harness, "https://zero.test", true)
            .await
            .unwrap();
        let rejected = answered_enable(&mut harness, "https://two.test", false).await;
        assert!(rejected.is_err(), "{rejected:?}");
        assert_eq!(
            session_commands(&harness.page),
            pattern_commands("https://zero.test/*")
        );
    }

    #[tokio::test]
    async fn overlapping_enables_reach_chrome_in_call_order() {
        let mut harness = PageHarness::new().await;
        withhold_fetch_enable(&harness);
        let first = start_enable(&harness, "https://one.test");
        let first_enable = harness.control.wait_for("Fetch.enable", Some("S1")).await;
        let second = start_enable(&harness, "https://two.test");
        tokio::time::sleep(Duration::from_millis(150)).await;
        assert!(
            harness.control.try_next_command().is_none(),
            "the second Fetch.enable was sent before the first was confirmed"
        );
        harness.control.reply(&first_enable, json!({}));
        first.await.unwrap().unwrap();
        let second_enable = harness.control.wait_for("Fetch.enable", Some("S1")).await;
        assert_eq!(second_enable.params, enable_params("https://two.test/*"));
        harness.control.reply(&second_enable, json!({}));
        second.await.unwrap().unwrap();
        assert_eq!(
            session_commands(&harness.page),
            pattern_commands("https://two.test/*")
        );
    }

    #[tokio::test]
    async fn session_commands_carry_the_current_auth_pattern() {
        let harness = PageHarness::new().await;
        assert!(session_commands(&harness.page).is_empty());
        harness
            .page
            .enable_basic_auth("https://one.test", allow_all())
            .await
            .unwrap();
        harness
            .page
            .enable_basic_auth("https://two.test", allow_all())
            .await
            .unwrap();
        let commands = session_commands(&harness.page);
        assert_eq!(
            commands,
            vec![(
                "Fetch.enable".to_owned(),
                json!({"handleAuthRequests": true, "patterns": [{"urlPattern": "https://two.test/*"}]})
            )]
        );
    }

    #[tokio::test]
    async fn dropping_the_page_state_stops_the_route_task() {
        let harness = PageHarness::new().await;
        harness
            .page
            .enable_basic_auth("https://one.test", allow_all())
            .await
            .unwrap();
        let route = harness.page.inner.lock_state().fetch.route.clone().unwrap();
        assert!(!route.is_finished());
        drop(std::mem::replace(
            &mut harness.page.inner.lock_state().fetch,
            FetchState::new(),
        ));
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            while !route.is_finished() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("the route task kept running after its page state was dropped");
    }
}
