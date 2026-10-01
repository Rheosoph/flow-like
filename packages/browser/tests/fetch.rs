use std::sync::{Arc, Mutex};
use std::time::Duration;

use flow_like_browser::connection::{EventHook, MethodMatch, RouteReceiver};
use flow_like_browser::event_log::Event;
use flow_like_browser::fetch::{AuthDecider, Observation};
use flow_like_browser::testing::{PageHarness, default_auto_reply};
use flow_like_browser::transport::memory::SentCommand;
use flow_like_browser::{BrowserError, Connection, Page};
use serde_json::{Value, json};

const ORIGIN: &str = "https://allowed.test";
const OTHER_ORIGIN: &str = "https://other.test";
const PASSWORD: &str = "secret-value";
const WAIT: Duration = Duration::from_secs(5);
const QUIET: Duration = Duration::from_millis(150);

fn normalized_origin(value: &str) -> Option<String> {
    let url = url::Url::parse(value).ok()?;
    Some(url.origin().ascii_serialization())
}

fn auth_response(origin: &str, challenge: &Value, first_attempt: bool) -> Value {
    let matches_origin = challenge["origin"]
        .as_str()
        .and_then(normalized_origin)
        .as_deref()
        == Some(origin);
    let basic = challenge["scheme"]
        .as_str()
        .is_some_and(|scheme| scheme.eq_ignore_ascii_case("basic"));
    if first_attempt && matches_origin && challenge["source"] == "Server" && basic {
        json!({"response": "ProvideCredentials", "username": "test-user", "password": PASSWORD})
    } else {
        json!({"response": "CancelAuth"})
    }
}

fn decider_for(origin: &'static str) -> AuthDecider {
    Arc::new(move |challenge, first| auth_response(origin, challenge, first))
}

fn auth_required(request: &str, origin: &str, source: &str) -> Value {
    json!({"requestId": request, "authChallenge": {"origin": origin, "source": source, "scheme": "basic", "realm": "lab"}})
}

async fn scripted_page() -> PageHarness {
    let harness = PageHarness::new().await;
    harness.control.set_auto_reply(|command| {
        (!command.method.starts_with("Fetch."))
            .then(|| default_auto_reply(command).unwrap_or_else(|| json!({})))
    });
    harness
}

async fn enable(
    harness: &mut PageHarness,
    origin: &'static str,
    sessions: &[&str],
) -> Vec<SentCommand> {
    let page = harness.page.clone();
    let enabling =
        tokio::spawn(async move { page.enable_basic_auth(origin, decider_for(origin)).await });
    let mut enables = Vec::new();
    for _ in sessions {
        let command = harness.control.next_command().await;
        assert_eq!(command.method, "Fetch.enable");
        harness.control.reply(&command, json!({}));
        enables.push(command);
    }
    tokio::time::timeout(WAIT, enabling)
        .await
        .expect("enable_basic_auth never returned")
        .unwrap()
        .unwrap();
    let mut enabled: Vec<_> = enables
        .iter()
        .filter_map(|command| command.session.clone())
        .collect();
    enabled.sort();
    assert_eq!(enabled, sessions);
    enables
}

async fn answers(harness: &mut PageHarness, count: usize) -> Vec<SentCommand> {
    let mut commands = Vec::new();
    while commands.len() < count {
        let command = harness.control.next_command().await;
        harness.control.reply(&command, json!({}));
        commands.push(command);
    }
    tokio::time::sleep(QUIET).await;
    if let Some(extra) = harness.control.try_next_command() {
        panic!("unexpected extra command {} {}", extra.method, extra.params);
    }
    commands
}

fn responses_for<'a>(commands: &'a [SentCommand], request: &str) -> Vec<&'a Value> {
    commands
        .iter()
        .filter(|command| {
            command.method == "Fetch.continueWithAuth" && command.params["requestId"] == request
        })
        .map(|command| &command.params["authChallengeResponse"])
        .collect()
}

fn continued(commands: &[SentCommand], request: &str) -> Vec<Option<String>> {
    commands
        .iter()
        .filter(|command| {
            command.method == "Fetch.continueRequest" && command.params["requestId"] == request
        })
        .map(|command| command.session.clone())
        .collect()
}

#[tokio::test]
async fn basic_auth_never_sends_credentials_to_other_origins_or_proxies() {
    let mut harness = scripted_page().await;
    let page = harness.page.clone();
    let enabling =
        tokio::spawn(async move { page.enable_basic_auth(ORIGIN, decider_for(ORIGIN)).await });
    let enable = harness.control.wait_for("Fetch.enable", Some("S1")).await;
    assert_eq!(
        enable.params,
        json!({"handleAuthRequests": true, "patterns": [{"urlPattern": "https://allowed.test/*"}]})
    );
    assert!(!enable.params.to_string().contains(PASSWORD));
    harness.emit("Fetch.requestPaused", json!({"requestId": "one"}));
    harness.control.reply(&enable, json!({}));
    tokio::time::timeout(WAIT, enabling)
        .await
        .expect("enable_basic_auth never returned")
        .unwrap()
        .unwrap();
    for (request, origin, source) in [
        ("one", ORIGIN, "Server"),
        ("two", "https://allowed.test.evil.test", "Server"),
        ("three", ORIGIN, "Proxy"),
        ("one", ORIGIN, "Server"),
    ] {
        harness.emit("Fetch.authRequired", auth_required(request, origin, source));
    }
    let commands = answers(&mut harness, 5).await;
    assert!(
        commands
            .iter()
            .all(|command| command.session.as_deref() == Some("S1"))
    );
    assert_eq!(continued(&commands, "one"), vec![Some("S1".to_owned())]);
    assert_credentials_only_on_first_attempt(&commands);
}

fn assert_credentials_only_on_first_attempt(commands: &[SentCommand]) {
    let first = responses_for(commands, "one");
    let provided: Vec<_> = first
        .iter()
        .filter(|response| response["response"] == "ProvideCredentials")
        .collect();
    assert_eq!(
        provided.len(),
        1,
        "only the first attempt gets credentials: {first:?}"
    );
    assert_eq!(provided[0]["username"], "test-user");
    assert_eq!(provided[0]["password"], PASSWORD);
    assert!(first.contains(&&json!({"response": "CancelAuth"})));
    for request in ["two", "three"] {
        assert_eq!(
            responses_for(commands, request),
            vec![&json!({"response": "CancelAuth"})]
        );
    }
    let leaked = commands
        .iter()
        .filter(|command| command.params.to_string().contains(PASSWORD))
        .count();
    assert_eq!(leaked, 1);
}

#[tokio::test]
async fn child_sessions_are_enabled_and_answered_on_their_own_session() {
    let mut harness = scripted_page().await;
    harness.attach_child("S2", "F2", "T1", "https://allowed.test/early", "L2");
    enable(&mut harness, ORIGIN, &["S1", "S2"]).await;
    harness.attach_child("S3", "F3", "T1", "https://allowed.test/late", "L3");
    harness.emit_on("S3", "Fetch.requestPaused", json!({"requestId": "late"}));
    harness.emit_on(
        "S3",
        "Fetch.authRequired",
        auth_required("late", ORIGIN, "Server"),
    );
    harness.emit_on(
        "S2",
        "Fetch.authRequired",
        auth_required("early", ORIGIN, "Server"),
    );
    let commands = answers(&mut harness, 3).await;
    assert_eq!(continued(&commands, "late"), vec![Some("S3".to_owned())]);
    for (request, session) in [("late", "S3"), ("early", "S2")] {
        let answer = commands
            .iter()
            .find(|command| {
                command.method == "Fetch.continueWithAuth" && command.params["requestId"] == request
            })
            .unwrap_or_else(|| panic!("{request} was not answered"));
        assert_eq!(answer.session.as_deref(), Some(session));
        assert_eq!(
            answer.params["authChallengeResponse"]["response"],
            "ProvideCredentials"
        );
    }
}

#[tokio::test]
async fn a_second_basic_auth_replaces_the_first_on_the_same_route() {
    let mut harness = scripted_page().await;
    enable(&mut harness, ORIGIN, &["S1"]).await;
    let replaced = enable(&mut harness, OTHER_ORIGIN, &["S1"]).await;
    assert_eq!(
        replaced[0].params["patterns"][0]["urlPattern"],
        "https://other.test/*"
    );
    harness.emit("Fetch.requestPaused", json!({"requestId": "x"}));
    harness.emit(
        "Fetch.authRequired",
        auth_required("y", OTHER_ORIGIN, "Server"),
    );
    harness.emit("Fetch.authRequired", auth_required("z", ORIGIN, "Server"));
    let commands = answers(&mut harness, 3).await;
    assert_eq!(continued(&commands, "x").len(), 1);
    assert_eq!(
        responses_for(&commands, "y")[0]["response"],
        "ProvideCredentials"
    );
    assert_eq!(
        responses_for(&commands, "z"),
        vec![&json!({"response": "CancelAuth"})]
    );
}

#[tokio::test]
async fn paused_requests_with_unexpected_field_types_are_still_answered() {
    let mut harness = scripted_page().await;
    enable(&mut harness, ORIGIN, &["S1"]).await;
    let odd_fields = json!({"requestId": "odd", "frameId": 7, "responseStatusCode": "200", "resourceType": null});
    harness.emit("Fetch.requestPaused", odd_fields);
    let mut challenge = auth_required("auth", ORIGIN, "Server");
    challenge["frameId"] = json!(["not", "a", "string"]);
    harness.emit("Fetch.authRequired", challenge);
    harness.emit("Fetch.requestPaused", json!({"frameId": "F1"}));
    let commands = answers(&mut harness, 2).await;
    assert_eq!(continued(&commands, "odd"), vec![Some("S1".to_owned())]);
    assert_eq!(
        responses_for(&commands, "auth")[0]["response"],
        "ProvideCredentials"
    );
}

#[tokio::test]
async fn a_rejected_fetch_enable_fails_the_call() {
    let mut harness = scripted_page().await;
    let page = harness.page.clone();
    let enabling =
        tokio::spawn(async move { page.enable_basic_auth(ORIGIN, decider_for(ORIGIN)).await });
    let enable = harness.control.wait_for("Fetch.enable", Some("S1")).await;
    harness.control.reply_error(
        &enable,
        -32602,
        "Can't specify empty patterns with handleAuth set",
    );
    let outcome = tokio::time::timeout(WAIT, enabling).await.unwrap().unwrap();
    assert!(
        matches!(outcome, Err(BrowserError::Protocol { ref method, code: -32602, .. }) if method == "Fetch.enable"),
        "{outcome:?}"
    );
}

async fn flush(harness: &PageHarness, marker: &str) {
    let cursor = harness.connection.events().cursor();
    harness.control.emit(
        "Target.targetInfoChanged",
        None,
        json!({"targetInfo": {"targetId": marker}}),
    );
    let deadline = tokio::time::Instant::now() + WAIT;
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

fn console(text: &str) -> Value {
    json!({"type": "log", "args": [{"type": "string", "value": text}], "executionContextId": 1})
}

fn label(event: &Event) -> String {
    let text = event.params["args"][0]["value"]
        .as_str()
        .or_else(|| event.params["exceptionDetails"]["text"].as_str())
        .or_else(|| event.params["requestId"].as_str())
        .unwrap_or_default();
    format!("{} {text}", event.method)
}

struct Live {
    events: Vec<Event>,
    skipped: usize,
}

async fn live_events(route: &mut RouteReceiver, after: u64, count: usize) -> Live {
    let mut live = Live {
        events: Vec::new(),
        skipped: 0,
    };
    let mut highest = after;
    while live.events.len() < count {
        let event = tokio::time::timeout(WAIT, route.recv())
            .await
            .expect("the observer route went quiet")
            .expect("the observer route closed");
        if event.seq <= highest {
            live.skipped += 1;
            continue;
        }
        highest = event.seq;
        live.events.push(event);
    }
    live
}

#[tokio::test]
async fn observe_replays_the_console_backlog_then_live_events_without_duplicates() {
    let harness = PageHarness::new().await;
    harness.emit("Runtime.consoleAPICalled", console("before"));
    harness.emit(
        "Runtime.exceptionThrown",
        json!({"exceptionDetails": {"text": "boom", "lineNumber": 1, "columnNumber": 2}}),
    );
    harness.emit_on("S9", "Runtime.consoleAPICalled", console("elsewhere"));
    flush(&harness, "backlog").await;
    let mut observation = harness.page.observe(vec![
        MethodMatch::Exact("Runtime.consoleAPICalled"),
        MethodMatch::Exact("Runtime.exceptionThrown"),
        MethodMatch::Prefix("Network."),
    ]);
    let backlog: Vec<String> = observation.backlog.iter().map(label).collect();
    assert_eq!(
        backlog,
        [
            "Runtime.consoleAPICalled before",
            "Runtime.exceptionThrown boom"
        ]
    );
    let seen = observation
        .backlog
        .iter()
        .map(|event| event.seq)
        .max()
        .unwrap();
    harness.emit("Runtime.consoleAPICalled", console("after"));
    harness.emit_on("S9", "Runtime.consoleAPICalled", console("elsewhere"));
    harness.emit("Page.loadEventFired", json!({"timestamp": 1.0}));
    harness.emit("Network.requestWillBeSent", json!({"requestId": "r1"}));
    let live = live_events(&mut observation.route, seen, 2).await.events;
    let live_labels: Vec<String> = live.iter().map(label).collect();
    assert_eq!(
        live_labels,
        [
            "Runtime.consoleAPICalled after",
            "Network.requestWillBeSent r1"
        ]
    );
    let mut sequence: Vec<u64> = observation.backlog.iter().map(|event| event.seq).collect();
    sequence.extend(live.iter().map(|event| event.seq));
    assert!(sequence.windows(2).all(|pair| pair[0] < pair[1]));
}

struct ObserveOnMarker {
    page: Page,
    marker: &'static str,
    observation: Mutex<Option<Observation>>,
}

impl EventHook for ObserveOnMarker {
    fn on_event(&self, _connection: &Connection, event: &Event) {
        let mut observation = self.observation.lock().unwrap();
        if observation.is_none() && event.params["args"][0]["value"] == self.marker {
            *observation = Some(
                self.page
                    .observe(vec![MethodMatch::Exact("Runtime.consoleAPICalled")]),
            );
        }
    }
}

#[tokio::test]
async fn an_event_in_both_the_backlog_and_the_route_is_delivered_once_by_seq() {
    let harness = PageHarness::new().await;
    let hook = Arc::new(ObserveOnMarker {
        page: harness.page.clone(),
        marker: "overlap",
        observation: Mutex::new(None),
    });
    harness.connection.add_hook(hook.clone());
    harness.emit("Runtime.consoleAPICalled", console("overlap"));
    harness.emit("Runtime.consoleAPICalled", console("after"));
    flush(&harness, "overlap").await;
    let mut observation = hook
        .observation
        .lock()
        .unwrap()
        .take()
        .expect("the hook never started the observer");
    let backlog: Vec<String> = observation.backlog.iter().map(label).collect();
    assert_eq!(backlog, ["Runtime.consoleAPICalled overlap"]);
    let seen = observation.backlog[0].seq;
    let live = live_events(&mut observation.route, seen, 1).await;
    assert_eq!(live.skipped, 1, "the backlog event also reached the route");
    let delivered: Vec<&Event> = observation.backlog.iter().chain(&live.events).collect();
    let labels: Vec<String> = delivered.iter().map(|event| label(event)).collect();
    assert_eq!(
        labels,
        [
            "Runtime.consoleAPICalled overlap",
            "Runtime.consoleAPICalled after"
        ]
    );
    assert!(delivered.windows(2).all(|pair| pair[0].seq < pair[1].seq));
}

#[tokio::test]
async fn observe_only_replays_backlog_entries_it_was_asked_for() {
    let harness = PageHarness::new().await;
    harness.emit("Runtime.consoleAPICalled", console("before"));
    flush(&harness, "backlog").await;
    let observation = harness.page.observe(vec![MethodMatch::Prefix("Network.")]);
    assert!(observation.backlog.is_empty());
    let console_only = harness
        .page
        .observe(vec![MethodMatch::Exact("Runtime.consoleAPICalled")]);
    assert_eq!(console_only.backlog.len(), 1);
}
