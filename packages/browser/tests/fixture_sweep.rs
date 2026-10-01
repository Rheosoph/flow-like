use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use flow_like_browser::BrowserError;
use flow_like_browser::connection::{
    Connection, ConnectionOptions, EventHook, PendingReply, Reply, Request,
};
use flow_like_browser::event_log::Event;
use flow_like_browser::launch::cft::CFT_PINNED;
use flow_like_browser::testing::{RecordedFrame, parse_recorded_frame};
use flow_like_browser::transport::memory::{InMemoryControl, InMemoryTransport};
use flow_like_browser::types::{SessionId, typed_event_check, typed_response_check};
use serde_json::{Value, json};

const RECORDED: [&str; 7] = [
    "oopif_swap",
    "dialogs",
    "downloads",
    "navigation",
    "crash",
    "ax_tree",
    "prerender",
];
const RECORD_HINT: &str = "record it against Chrome with FLOW_LIKE_CDP_RECORD=1 cargo test -p flow-like-browser --test e2e_core -- --include-ignored record_ prerender_";
const REPLY_WAIT: Duration = Duration::from_secs(5);
const PARSE_ERROR: i64 = -32700;

type SeenEvent = (String, Option<String>);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Direction {
    Send,
    Recv,
}

struct Line {
    number: usize,
    direction: Direction,
    text: String,
    frame: Value,
}

struct Fixture {
    name: String,
    header: Option<Value>,
    lines: Vec<Line>,
}

impl Fixture {
    fn received(&self) -> impl Iterator<Item = &Line> {
        self.lines
            .iter()
            .filter(|line| line.direction == Direction::Recv)
    }

    fn sent(&self) -> impl Iterator<Item = &Line> {
        self.lines
            .iter()
            .filter(|line| line.direction == Direction::Send)
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

fn fixtures_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/cdp")
}

fn load_fixtures() -> Vec<Fixture> {
    let dir = fixtures_dir();
    let mut paths: Vec<PathBuf> = std::fs::read_dir(&dir)
        .unwrap_or_else(|error| panic!("listing {}: {error}", dir.display()))
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == "jsonl")
        })
        .collect();
    paths.sort();
    assert!(!paths.is_empty(), "no CDP fixtures in {}", dir.display());
    paths.iter().map(|path| load_fixture(path)).collect()
}

fn load_fixture(path: &Path) -> Fixture {
    let text = std::fs::read_to_string(path)
        .unwrap_or_else(|error| panic!("reading {}: {error}", path.display()));
    let name = path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or_default()
        .to_owned();
    let mut fixture = Fixture {
        name,
        header: None,
        lines: Vec::new(),
    };
    for (index, raw) in text.lines().enumerate() {
        if raw.trim().is_empty() {
            continue;
        }
        let entry: Value = serde_json::from_str(raw)
            .unwrap_or_else(|error| panic!("{}:{} is not JSON: {error}", fixture.name, index + 1));
        match parse_line(&fixture.name, index + 1, &entry) {
            Some(line) => fixture.lines.push(line),
            None => fixture.header = Some(entry),
        }
    }
    fixture
}

fn parse_line(fixture: &str, number: usize, entry: &Value) -> Option<Line> {
    let direction = match entry["dir"].as_str() {
        Some("send") => Direction::Send,
        Some("recv") => Direction::Recv,
        None if entry.get("chrome").is_some() => return None,
        other => panic!("{fixture}:{number} has no send/recv direction ({other:?})"),
    };
    let (text, frame) = match &entry["frame"] {
        Value::String(raw) => (
            raw.clone(),
            serde_json::from_str(raw).unwrap_or(Value::Null),
        ),
        frame => (frame.to_string(), frame.clone()),
    };
    Some(Line {
        number,
        direction,
        text,
        frame,
    })
}

#[test]
fn every_recorded_frame_parses_and_passes_typed_decoding() {
    let failures: Vec<String> = load_fixtures().iter().flat_map(check_fixture).collect();
    assert!(
        failures.is_empty(),
        "{} recorded frames failed:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

fn check_fixture(fixture: &Fixture) -> Vec<String> {
    let methods: HashMap<u64, String> = fixture
        .sent()
        .filter_map(|line| {
            let id = line.frame["id"].as_u64()?;
            Some((id, line.frame["method"].as_str()?.to_owned()))
        })
        .collect();
    fixture
        .received()
        .filter_map(|line| check_frame(fixture, line, &methods).err())
        .collect()
}

fn check_frame(
    fixture: &Fixture,
    line: &Line,
    methods: &HashMap<u64, String>,
) -> Result<(), String> {
    let at = format!("{}:{}", fixture.name, line.number);
    match parse_recorded_frame(&line.text) {
        RecordedFrame::Malformed { id } => {
            Err(format!("{at}: the frame does not parse (id {id:?})"))
        }
        RecordedFrame::Event { method, params, .. } => {
            typed(typed_event_check(&method, &params), &at, &method)
        }
        RecordedFrame::Response { id, result, .. } => {
            if line.frame.get("error").is_some() {
                return Ok(());
            }
            let method = methods
                .get(&id)
                .ok_or_else(|| format!("{at}: reply {id} has no recorded command"))?;
            typed(typed_response_check(method, &result), &at, method)
        }
    }
}

fn typed(check: Option<Result<(), String>>, at: &str, method: &str) -> Result<(), String> {
    match check {
        Some(Err(error)) => Err(format!("{at}: {method} failed typed decoding: {error}")),
        _ => Ok(()),
    }
}

#[tokio::test]
async fn the_connection_reader_delivers_every_recorded_frame() {
    for fixture in load_fixtures() {
        replay(&fixture).await;
    }
}

struct SeenEvents(Mutex<Vec<SeenEvent>>);

impl EventHook for SeenEvents {
    fn on_event(&self, _connection: &Connection, event: &Event) {
        let session = event.session.as_ref().map(ToString::to_string);
        lock(&self.0).push((event.method.to_string(), session));
    }
}

/// Replays the transcript in recorded order. Before each command the connection has processed
/// every event recorded before it, so its session state (crashes, detaches) matches the moment
/// the recorded client sent that command.
async fn replay(fixture: &Fixture) {
    let (transport, mut control) = InMemoryTransport::new();
    let connection = Connection::start(Box::new(transport), ConnectionOptions::default());
    let seen = Arc::new(SeenEvents(Mutex::new(Vec::new())));
    connection.add_hook(seen.clone());
    let mut replay = Replay::default();
    for (index, line) in fixture.lines.iter().enumerate() {
        match line.direction {
            Direction::Send => {
                wait_for_events(&seen, replay.events_sent).await;
                replay.send(&connection, &mut control, index, line).await;
            }
            Direction::Recv => replay.receive(&control, line),
        }
    }
    let expected = recorded_events(fixture);
    wait_for_events(&seen, expected.len()).await;
    assert!(
        replay.refused.is_empty(),
        "{}: the connection refused recorded commands: {:?}",
        fixture.name,
        replay.refused
    );
    assert_eq!(
        *lock(&seen.0),
        expected,
        "{}: the connection delivered a different event sequence",
        fixture.name
    );
    replay.check_replies(fixture).await;
    connection.abort();
}

fn recorded_events(fixture: &Fixture) -> Vec<SeenEvent> {
    fixture
        .received()
        .filter_map(|line| match parse_recorded_frame(&line.text) {
            RecordedFrame::Event {
                method, session, ..
            } => Some((method, session)),
            _ => None,
        })
        .collect()
}

async fn wait_for_events(seen: &SeenEvents, count: usize) {
    let deadline = tokio::time::Instant::now() + REPLY_WAIT;
    while lock(&seen.0).len() < count && tokio::time::Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

struct Waiting {
    recorded_id: u64,
    method: String,
    session: Option<String>,
    sent_at: usize,
    reply: PendingReply,
}

#[derive(Default)]
struct Replay {
    live_ids: HashMap<u64, u64>,
    waiting: Vec<Waiting>,
    refused: Vec<String>,
    events_sent: usize,
}

impl Replay {
    async fn send(
        &mut self,
        connection: &Connection,
        control: &mut InMemoryControl,
        index: usize,
        line: &Line,
    ) {
        let (Some(id), Some(method)) = (line.frame["id"].as_u64(), line.frame["method"].as_str())
        else {
            panic!("line {} is a command without an id or method", line.number);
        };
        let session = line.frame["sessionId"].as_str().map(str::to_owned);
        let request = Request {
            method: method.to_owned().into(),
            params: line
                .frame
                .get("params")
                .cloned()
                .unwrap_or_else(|| json!({})),
            session: session.as_deref().map(SessionId::from),
            timeout: Some(REPLY_WAIT * 2),
        };
        match connection.enqueue(request) {
            Ok(reply) => {
                let sent = control.next_command().await;
                self.live_ids.insert(id, sent.id);
                self.waiting.push(Waiting {
                    recorded_id: id,
                    method: method.to_owned(),
                    session,
                    sent_at: index,
                    reply,
                });
            }
            Err(error) => self
                .refused
                .push(format!("line {}: {method}: {error}", line.number)),
        }
    }

    /// Replies go out under the live id of their command; a reply whose command never went out
    /// is dropped rather than sent under a recorded id that may belong to another command.
    fn receive(&mut self, control: &InMemoryControl, line: &Line) {
        let Some(recorded) = line.frame.get("id").and_then(Value::as_u64) else {
            self.events_sent += 1;
            control.send_text(&line.text);
            return;
        };
        if let Some(live) = self.live_ids.get(&recorded) {
            let mut frame = line.frame.clone();
            frame["id"] = json!(live);
            control.send_text(&frame.to_string());
        }
    }

    async fn check_replies(self, fixture: &Fixture) {
        let replies: HashMap<u64, (usize, bool)> = fixture
            .lines
            .iter()
            .enumerate()
            .filter(|(_, line)| line.direction == Direction::Recv)
            .filter_map(|(index, line)| {
                let id = line.frame["id"].as_u64()?;
                Some((id, (index, line.frame.get("error").is_some())))
            })
            .collect();
        for waiting in self.waiting {
            let Some(&(reply_at, recorded_error)) = replies.get(&waiting.recorded_id) else {
                continue;
            };
            let outcome = tokio::time::timeout(REPLY_WAIT, waiting.reply)
                .await
                .unwrap_or_else(|_| {
                    panic!(
                        "{}: the recorded reply to {} (id {}) never reached its command",
                        fixture.name, waiting.method, waiting.recorded_id
                    )
                });
            let lost = waiting
                .session
                .as_deref()
                .is_some_and(|session| session_lost(fixture, waiting.sent_at, reply_at, session));
            check_outcome(
                &fixture.name,
                &waiting.method,
                recorded_error,
                lost,
                outcome,
            );
        }
    }
}

/// Whether the recording shows the session crash or detach between a command and its reply.
fn session_lost(fixture: &Fixture, sent_at: usize, reply_at: usize, session: &str) -> bool {
    fixture.lines[sent_at..reply_at].iter().any(|line| {
        let frame = &line.frame;
        let on_session = frame["sessionId"] == session;
        match frame["method"].as_str() {
            Some("Inspector.targetCrashed" | "Inspector.detached") => on_session,
            Some("Target.detachedFromTarget") => frame["params"]["sessionId"] == session,
            _ => false,
        }
    })
}

fn check_outcome(
    fixture: &str,
    method: &str,
    recorded_error: bool,
    lost: bool,
    outcome: flow_like_browser::Result<Reply>,
) {
    match outcome {
        Ok(_) if !recorded_error => {}
        Err(BrowserError::Protocol { code, .. }) if recorded_error && code != PARSE_ERROR => {}
        Err(BrowserError::TargetClosed { .. } | BrowserError::TargetCrashed { .. }) if lost => {}
        other => panic!(
            "{fixture}: the reply to {method} arrived as {other:?} (recorded as an error: {recorded_error}, session lost before the reply: {lost})"
        ),
    }
}

#[test]
fn every_recorded_scenario_has_a_normalised_fixture() {
    for name in RECORDED {
        let path = fixtures_dir().join(format!("{name}.jsonl"));
        assert!(
            path.is_file(),
            "{} is missing; {RECORD_HINT}",
            path.display()
        );
        let fixture = load_fixture(&path);
        check_header(&fixture);
        assert!(
            fixture.received().next().is_some(),
            "{name}.jsonl holds no received frames"
        );
        let raw = raw_devtools_ids(&fixture);
        assert!(
            raw.is_empty(),
            "{name}.jsonl still carries unnormalised DevTools ids: {raw:?}"
        );
        let local = local_paths(&fixture);
        assert!(
            local.is_empty(),
            "{name}.jsonl carries machine-local paths: {local:?}"
        );
        let missing = missing_markers(&fixture);
        assert!(
            missing.is_empty(),
            "{name}.jsonl does not record what the scenario exists for: {missing:?}; {RECORD_HINT}"
        );
    }
}

fn check_header(fixture: &Fixture) {
    let name = &fixture.name;
    let header = fixture.header.as_ref().unwrap_or_else(|| {
        panic!("{name}.jsonl has no chrome/recorded header line; {RECORD_HINT}")
    });
    let chrome = header["chrome"].as_str().unwrap_or_default();
    assert_eq!(chrome.split('.').count(), 4, "{name}.jsonl header {header}");
    assert!(
        header["recorded"]
            .as_str()
            .is_some_and(|date| date.len() == 10),
        "{name}.jsonl header {header}"
    );
    assert_eq!(
        major(chrome),
        major(CFT_PINNED),
        "{name}.jsonl was recorded with Chrome {chrome}, not the pinned {CFT_PINNED}; {RECORD_HINT}"
    );
}

fn major(version: &str) -> &str {
    version.split('.').next().unwrap_or_default()
}

/// A frame each scenario exists for: `method`, and `value` at `path` inside its params (or in
/// the frame itself for a command). A fixture without them would pass the sweep vacuously.
struct Marker {
    fixture: &'static str,
    what: &'static str,
    method: &'static str,
    path: &'static [&'static str],
    value: &'static str,
}

const MARKERS: [Marker; 14] = [
    Marker {
        fixture: "oopif_swap",
        what: "an OOPIF session detached (remote to local)",
        method: "Target.detachedFromTarget",
        path: &[],
        value: "",
    },
    Marker {
        fixture: "oopif_swap",
        what: "a frame swapped out of the parent (local to remote)",
        method: "Page.frameDetached",
        path: &["reason"],
        value: "swap",
    },
    Marker {
        fixture: "dialogs",
        what: "a prompt opened",
        method: "Page.javascriptDialogOpening",
        path: &["type"],
        value: "prompt",
    },
    Marker {
        fixture: "dialogs",
        what: "a dialog closed",
        method: "Page.javascriptDialogClosed",
        path: &[],
        value: "",
    },
    Marker {
        fixture: "downloads",
        what: "a GUID-named staging download (allowAndName)",
        method: "Browser.setDownloadBehavior",
        path: &["behavior"],
        value: "allowAndName",
    },
    Marker {
        fixture: "downloads",
        what: "a download completed",
        method: "Browser.downloadProgress",
        path: &["state"],
        value: "completed",
    },
    Marker {
        fixture: "navigation",
        what: "a same-document navigation",
        method: "Page.navigatedWithinDocument",
        path: &[],
        value: "",
    },
    Marker {
        fixture: "navigation",
        what: "a history navigation",
        method: "Page.navigateToHistoryEntry",
        path: &[],
        value: "",
    },
    Marker {
        fixture: "crash",
        what: "the renderer crashed",
        method: "Inspector.targetCrashed",
        path: &[],
        value: "",
    },
    Marker {
        fixture: "crash",
        what: "the page came back after its crash",
        method: "Inspector.targetReloadedAfterCrash",
        path: &[],
        value: "",
    },
    Marker {
        fixture: "ax_tree",
        what: "a full accessibility tree was read",
        method: "Accessibility.getFullAXTree",
        path: &[],
        value: "",
    },
    Marker {
        fixture: "prerender",
        what: "a prerender target",
        method: "Target.targetCreated",
        path: &["targetInfo", "subtype"],
        value: "prerender",
    },
    Marker {
        fixture: "prerender",
        what: "the replaced page turned disconnected",
        method: "Target.targetInfoChanged",
        path: &["targetInfo", "subtype"],
        value: "disconnected",
    },
    Marker {
        fixture: "prerender",
        what: "Chrome refused to prerender for a held page",
        method: "Preload.prerenderStatusUpdated",
        path: &["prerenderStatus"],
        value: "PrerenderingDisabledByDevTools",
    },
];

impl Marker {
    fn matches(&self, frame: &Value) -> bool {
        let value = self
            .path
            .iter()
            .fold(&frame["params"], |inner, key| &inner[*key]);
        frame["method"] == self.method && (self.path.is_empty() || *value == self.value)
    }
}

fn missing_markers(fixture: &Fixture) -> Vec<&'static str> {
    MARKERS
        .iter()
        .filter(|marker| marker.fixture == fixture.name)
        .filter(|marker| !fixture.lines.iter().any(|line| marker.matches(&line.frame)))
        .map(|marker| marker.what)
        .collect()
}

const LOCAL_PATH_MARKERS: [&str; 6] = [
    "/Users/",
    "/home/",
    "/private/",
    "/var/folders/",
    "/tmp/",
    ":\\",
];

fn local_paths(fixture: &Fixture) -> Vec<String> {
    let mut found = Vec::new();
    for line in &fixture.lines {
        collect_strings(&line.frame, &mut |text| {
            if LOCAL_PATH_MARKERS
                .iter()
                .any(|marker| text.contains(marker))
            {
                found.push(text.to_owned());
            }
        });
    }
    found
}

fn collect_strings(value: &Value, visit: &mut impl FnMut(&str)) {
    match value {
        Value::String(text) => visit(text),
        Value::Array(items) => items.iter().for_each(|item| collect_strings(item, visit)),
        Value::Object(map) => map.values().for_each(|item| collect_strings(item, visit)),
        _ => {}
    }
}

fn raw_devtools_ids(fixture: &Fixture) -> Vec<String> {
    let mut found = Vec::new();
    for line in &fixture.lines {
        collect_raw_ids(&line.frame, &mut found);
    }
    found.sort();
    found.dedup();
    found
}

fn collect_raw_ids(value: &Value, found: &mut Vec<String>) {
    match value {
        Value::String(text) if is_devtools_id(text) => found.push(text.clone()),
        Value::Array(items) => items.iter().for_each(|item| collect_raw_ids(item, found)),
        Value::Object(map) => map.values().for_each(|item| collect_raw_ids(item, found)),
        _ => {}
    }
}

fn is_devtools_id(text: &str) -> bool {
    text.len() == 32
        && text
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'A'..=b'F').contains(&byte))
}

#[test]
fn raw_devtools_ids_are_recognised() {
    assert!(is_devtools_id("18B43571B0EA45F69D5C3B3624A1FAFE"));
    assert!(!is_devtools_id("T1"));
    assert!(!is_devtools_id("18b43571b0ea45f69d5c3b3624a1fafe"));
    let seed = load_fixture(&fixtures_dir().join("oopif-normalized.jsonl"));
    assert!(
        !raw_devtools_ids(&seed).is_empty(),
        "the vendored seed transcript keeps its raw frame ids, so the check must see them"
    );
}
