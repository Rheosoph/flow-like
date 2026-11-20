use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

use async_trait::async_trait;
use chrono::{DateTime, Duration, Utc};
use flow_like_types::Result;
use flow_like_types::bot_calendar::{
    CalendarEntrySource, CalendarWake, EventSchedule, EventWake, ExternalEventRef, TaskEvent,
    UpdateCalendarWake,
};
use flow_like_types::bot_connector::{ConnectorKind, ConnectorPolicy, ConnectorRule};
use flow_like_types::bot_task::{BotTask, CreateBotTask, TaskCreator, TaskKind, UpdateBotTask};
use flow_like_types::reqwest::Url;
use serde_json::{Value, json};

use super::provider::{Client, ExternalEvent, Next, Transport};
use super::tools::{
    CalendarCreateArgs, CalendarDeleteArgs, CalendarFreeSlotsArgs, CalendarUpdateArgs, create_with,
    delete_with, free_slots_with, update_with,
};
use super::*;
use crate::error::code;
use crate::services::rest::RestResponse;
use crate::services::{CallAudit, CallBudget, ConnectorEntry, Credential, POLICY_FIELD};
use crate::testing::FakeEnv;

const TOKEN: &str = "tok-calendar-123";

fn utc(text: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(text)
        .unwrap()
        .with_timezone(&Utc)
}

#[derive(Default)]
struct FakeScheduler {
    tasks: Mutex<Vec<BotTask>>,
    created: Mutex<Vec<CreateBotTask>>,
    updated: Mutex<Vec<(String, UpdateBotTask)>>,
    cancelled: Mutex<Vec<String>>,
    wake: Mutex<CalendarWake>,
    elsewhere: AtomicBool,
}

impl FakeScheduler {
    fn with_wake(wake: CalendarWake) -> Self {
        Self {
            wake: Mutex::new(wake),
            ..Self::default()
        }
    }

    fn add(&self, task: BotTask) {
        self.tasks.lock().unwrap().push(task);
    }

    /// What a host does on every write: works out the wake-up offsets and the next wake-up.
    fn resolve(&self, task: &mut BotTask) {
        let wake = self.wake.lock().unwrap().clone();
        if let Some(event) = task.event.as_mut() {
            event.wake_offsets = wake.resolve(&task.title, event);
            task.next_run_at = EventSchedule::of(event, task.cron.as_deref(), task.tz.as_deref())
                .ok()
                .and_then(|schedule| schedule.next_wake(Utc::now(), None))
                .map(provider::rfc3339);
            task.enabled = task.next_run_at.is_some();
        }
    }
}

fn task(id: &str, title: &str, event: TaskEvent) -> BotTask {
    BotTask {
        id: id.into(),
        bot_id: "bot".into(),
        owner_id: "owner".into(),
        kind: TaskKind::Event,
        at: None,
        cron: None,
        tz: None,
        instruction: String::new(),
        title: title.into(),
        next_run_at: None,
        last_run_at: None,
        enabled: true,
        monitoring: true,
        created_by: TaskCreator::Bot,
        created_at: "2026-11-01T00:00:00Z".into(),
        conversation: None,
        place: None,
        event: Some(event),
    }
}

fn event(starts_at: &str, ends_at: Option<&str>) -> TaskEvent {
    TaskEvent {
        starts_at: starts_at.into(),
        ends_at: ends_at.map(str::to_string),
        all_day: false,
        location: None,
        wake: None,
        wake_offsets: Vec::new(),
        external: None,
        link: None,
    }
}

fn reference(connector: &str, provider: &str, event_id: &str) -> ExternalEventRef {
    ExternalEventRef {
        connector_id: connector.into(),
        provider: provider.into(),
        calendar_id: "primary".into(),
        event_id: event_id.into(),
    }
}

fn row(id: &str, title: &str, at: &str, reference: ExternalEventRef) -> BotTask {
    task(
        id,
        title,
        TaskEvent {
            external: Some(reference),
            wake_offsets: vec![15],
            ..event(at, None)
        },
    )
}

#[async_trait]
impl crate::schedule::Scheduler for FakeScheduler {
    async fn create(&self, request: CreateBotTask) -> Result<BotTask> {
        self.created.lock().unwrap().push(request.clone());
        let external = request
            .event
            .as_ref()
            .and_then(|event| event.external.clone());
        let mut tasks = self.tasks.lock().unwrap();
        if let Some(existing) = tasks.iter_mut().find(|task| {
            external.is_some()
                && task
                    .event
                    .as_ref()
                    .and_then(|event| event.external.as_ref())
                    == external.as_ref()
        }) {
            existing.title = request.title.clone().unwrap_or_default();
            existing.event = request.event.clone();
            self.resolve(existing);
            return Ok(existing.clone());
        }
        let mut created = task(
            &format!("t{}", tasks.len() + 1),
            request.title.as_deref().unwrap_or_default(),
            request
                .event
                .clone()
                .unwrap_or_else(|| event("2026-01-01T00:00:00Z", None)),
        );
        created.kind = request.kind.unwrap_or(TaskKind::Event);
        created.instruction = request.instruction.clone();
        created.cron = request.cron.clone();
        created.tz = request.tz.clone();
        self.resolve(&mut created);
        tasks.push(created.clone());
        Ok(created)
    }

    async fn list(&self) -> Result<Vec<BotTask>> {
        Ok(self.tasks.lock().unwrap().clone())
    }

    async fn cancel(&self, task_id: &str) -> Result<bool> {
        self.cancelled.lock().unwrap().push(task_id.into());
        let mut tasks = self.tasks.lock().unwrap();
        let before = tasks.len();
        tasks.retain(|task| task.id != task_id);
        Ok(tasks.len() < before)
    }

    async fn update(&self, task_id: &str, request: UpdateBotTask) -> Result<Option<BotTask>> {
        self.updated
            .lock()
            .unwrap()
            .push((task_id.into(), request.clone()));
        let mut tasks = self.tasks.lock().unwrap();
        let Some(task) = tasks.iter_mut().find(|task| task.id == task_id) else {
            return Ok(None);
        };
        if let Some(title) = request.title {
            task.title = title;
        }
        if let Some(instruction) = request.instruction {
            task.instruction = instruction;
        }
        if let Some(event) = request.event {
            task.event = Some(event);
        }
        self.resolve(task);
        Ok(Some(task.clone()))
    }

    async fn in_owner_chat(&self) -> Result<bool> {
        Ok(!self.elsewhere.load(Ordering::SeqCst))
    }

    async fn calendar_wake(&self) -> Result<CalendarWake> {
        Ok(self.wake.lock().unwrap().clone())
    }

    async fn set_calendar_wake(&self, update: UpdateCalendarWake) -> Result<CalendarWake> {
        let current = self.wake.lock().unwrap().clone();
        let next = update
            .apply(current)
            .map_err(|error| flow_like_types::anyhow!(error))?;
        *self.wake.lock().unwrap() = next.clone();
        Ok(next)
    }
}

#[derive(Default)]
struct FakeSource {
    entries: Vec<ConnectorEntry>,
    budget: CallBudget,
    audits: Mutex<Vec<CallAudit>>,
    jobs: Option<std::sync::Arc<dyn CalendarJobs>>,
    credentials: std::sync::atomic::AtomicUsize,
}

#[async_trait]
impl crate::services::ConnectorSource for FakeSource {
    async fn entries(&self) -> Result<Vec<ConnectorEntry>> {
        Ok(self.entries.clone())
    }

    async fn credential(&self, _connector_id: &str) -> Result<Credential> {
        self.credentials.fetch_add(1, Ordering::SeqCst);
        Ok(Credential::from_secret(TOKEN))
    }

    fn budget(&self) -> &CallBudget {
        &self.budget
    }

    fn audit(&self, entry: &CallAudit) {
        self.audits.lock().unwrap().push(entry.clone());
    }

    fn calendar_jobs(&self) -> Option<&dyn CalendarJobs> {
        self.jobs.as_deref()
    }
}

fn account(id: &str, label: &str, provider: &str, host: &str) -> ConnectorEntry {
    ConnectorEntry {
        rule: ConnectorRule {
            id: id.into(),
            label: label.into(),
            kind: Some(ConnectorKind::Oauth),
            ..ConnectorRule::default()
        },
        kind: ConnectorKind::Oauth,
        url: None,
        provider_id: Some(provider.into()),
        api_hosts: vec![host.into()],
        calendar: None,
    }
}

fn google() -> ConnectorEntry {
    account("c-google", "Work", "google", "www.googleapis.com")
}

fn microsoft() -> ConnectorEntry {
    account("c-ms", "Outlook", "microsoft", "graph.microsoft.com")
}

fn source(entries: Vec<ConnectorEntry>) -> FakeSource {
    FakeSource {
        entries,
        ..FakeSource::default()
    }
}

struct Route {
    method: &'static str,
    path: String,
    query: Option<&'static str>,
    status: u16,
    body: Value,
}

#[derive(Default)]
struct FakeTransport {
    routes: Vec<Route>,
    seen: Mutex<Vec<(String, Url, Option<Value>)>>,
}

impl FakeTransport {
    fn route(mut self, method: &'static str, path: &str, status: u16, body: Value) -> Self {
        self.routes.push(Route {
            method,
            path: path.into(),
            query: None,
            status,
            body,
        });
        self
    }

    fn page(mut self, path: &str, query: &'static str, body: Value) -> Self {
        self.routes.push(Route {
            method: "GET",
            path: path.into(),
            query: Some(query),
            status: 200,
            body,
        });
        self
    }

    fn methods(&self) -> Vec<String> {
        self.seen
            .lock()
            .unwrap()
            .iter()
            .map(|(method, url, _)| format!("{method} {}", url.path()))
            .collect()
    }

    fn body_of(&self, method: &str) -> Option<Value> {
        self.seen
            .lock()
            .unwrap()
            .iter()
            .find(|(seen, ..)| seen == method)
            .and_then(|(_, _, body)| body.clone())
    }
}

#[async_trait]
impl Transport for FakeTransport {
    async fn send(
        &self,
        method: &str,
        url: Url,
        credential: &Credential,
        body: Option<Value>,
    ) -> std::result::Result<RestResponse, String> {
        assert_eq!(
            credential.header(),
            Some(format!("Bearer {TOKEN}").as_str())
        );
        self.seen
            .lock()
            .unwrap()
            .push((method.to_string(), url.clone(), body));
        let query = url.query().unwrap_or_default();
        let route = self.routes.iter().find(|route| {
            route.method == method
                && route.path == url.path()
                && route.query.is_none_or(|wanted| query.contains(wanted))
        });
        Ok(match route {
            Some(route) => RestResponse {
                status: route.status,
                content_type: Some("application/json".into()),
                text: if route.body.is_null() {
                    String::new()
                } else {
                    route.body.to_string()
                },
            },
            None => RestResponse {
                status: 404,
                content_type: None,
                text:
                    json!({ "error": { "message": format!("no route {method} {}", url.path()) } })
                        .to_string(),
            },
        })
    }
}

const GOOGLE_EVENTS: &str = "/calendar/v3/calendars/primary/events";
const GRAPH_VIEW: &str = "/v1.0/me/calendarView";

fn google_event(id: &str, title: &str, start: &str, end: &str) -> Value {
    json!({
        "id": id,
        "status": "confirmed",
        "summary": title,
        "htmlLink": format!("https://calendar.google.com/event?eid={id}"),
        "start": { "dateTime": start },
        "end": { "dateTime": end },
    })
}

#[test]
fn google_events_become_entries_and_unshown_ones_are_dropped() {
    let body = json!({
        "timeZone": "Europe/Berlin",
        "nextPageToken": "p2",
        "items": [
            {
                "id": "abc_20261121T090000Z",
                "status": "confirmed",
                "summary": "  Standup  ",
                "description": "x".repeat(2_000),
                "location": "Room 4",
                "htmlLink": "https://calendar.google.com/event?eid=abc",
                "start": { "dateTime": "2026-11-21T10:00:00+01:00", "timeZone": "Europe/Berlin" },
                "end": { "dateTime": "2026-11-21T10:15:00+01:00" },
                "recurringEventId": "abc",
            },
            { "id": "day", "summary": "Holiday", "start": { "date": "2026-11-22" }, "end": { "date": "2026-11-23" }, "transparency": "transparent" },
            { "id": "gone", "status": "cancelled" },
            { "id": "office", "eventType": "workingLocation", "start": { "date": "2026-11-22" } },
            { "id": "broken", "start": { "dateTime": "tomorrow" } },
            { "id": "untitled", "start": { "dateTime": "2026-11-23T08:00:00Z" } },
        ]
    });
    let page = google::parse_page(&body);
    assert_eq!(page.next, Some(Next::Token("p2".into())));
    assert_eq!(page.unreadable, 1);
    assert_eq!(page.events.len(), 3);
    let standup = &page.events[0];
    assert_eq!(standup.title, "Standup");
    assert_eq!(standup.starts_at, utc("2026-11-21T09:00:00Z"));
    assert_eq!(standup.ends_at, Some(utc("2026-11-21T09:15:00Z")));
    assert!(standup.recurring && standup.busy && !standup.all_day);
    assert_eq!(
        standup.notes.as_deref().map(str::len),
        Some(provider::MAX_NOTES_CHARS)
    );
    let holiday = &page.events[1];
    assert!(holiday.all_day && !holiday.busy);
    assert_eq!(holiday.starts_at, utc("2026-11-21T23:00:00Z"));
    assert_eq!(holiday.ends_at, Some(utc("2026-11-22T23:00:00Z")));
    assert_eq!(page.events[2].title, "(no title)");
}

#[test]
fn graph_events_read_their_zone_and_skip_unknown_ones() {
    let body = json!({
        "@odata.nextLink": "https://graph.microsoft.com/v1.0/me/calendarView?$skip=100",
        "value": [
            {
                "id": "AAMk-1=",
                "subject": "Review",
                "start": { "dateTime": "2026-11-21T09:00:00.0000000", "timeZone": "UTC" },
                "end": { "dateTime": "2026-11-21T10:00:00.0000000", "timeZone": "UTC" },
                "location": { "displayName": "Teams" },
                "bodyPreview": "Agenda",
                "webLink": "https://outlook.office365.com/owa/?itemid=1",
                "type": "occurrence",
                "showAs": "busy",
            },
            {
                "id": "berlin",
                "subject": "Lunch",
                "start": { "dateTime": "2026-11-21T12:00:00", "timeZone": "Europe/Berlin" },
                "end": { "dateTime": "2026-11-21T13:00:00", "timeZone": "Europe/Berlin" },
                "showAs": "free",
            },
            { "id": "windows", "start": { "dateTime": "2026-11-21T12:00:00", "timeZone": "W. Europe Standard Time" } },
            { "id": "off", "isCancelled": true },
            { "id": "day", "subject": "Off", "isAllDay": true, "start": { "dateTime": "2026-11-24T00:00:00.0000000", "timeZone": "UTC" }, "end": { "dateTime": "2026-11-25T00:00:00.0000000", "timeZone": "UTC" } },
        ]
    });
    let page = microsoft::parse_page(&body);
    assert!(matches!(page.next, Some(Next::Url(ref link)) if link.contains("$skip=100")));
    assert_eq!(page.unreadable, 1);
    assert_eq!(page.events.len(), 3);
    let review = &page.events[0];
    assert_eq!(review.starts_at, utc("2026-11-21T09:00:00Z"));
    assert_eq!(review.location.as_deref(), Some("Teams"));
    assert_eq!(review.notes.as_deref(), Some("Agenda"));
    assert!(review.recurring && review.busy);
    let lunch = &page.events[1];
    assert_eq!(lunch.starts_at, utc("2026-11-21T11:00:00Z"));
    assert!(!lunch.busy);
    assert!(page.events[2].all_day);
}

#[test]
fn write_bodies_carry_only_what_changes() {
    let berlin: chrono_tz::Tz = "Europe/Berlin".parse().unwrap();
    let whole_day = provider::EventDraft {
        title: Some("Holiday".into()),
        starts_at: Some(utc("2026-11-21T23:00:00Z")),
        ends_at: Some(utc("2026-11-23T23:00:00Z")),
        all_day: Some(true),
        tz: Some(berlin),
        ..provider::EventDraft::default()
    };
    assert_eq!(
        google::body(&whole_day),
        json!({ "summary": "Holiday", "start": { "date": "2026-11-22" }, "end": { "date": "2026-11-24" } })
    );
    let graph = microsoft::body(&whole_day);
    assert_eq!(graph["isAllDay"], true);
    assert_eq!(graph["start"]["dateTime"], "2026-11-22T00:00:00");
    assert_eq!(graph["start"]["timeZone"], "Europe/Berlin");
    let renamed = provider::EventDraft {
        location: Some(String::new()),
        ..provider::EventDraft::default()
    };
    assert_eq!(google::body(&renamed), json!({ "location": "" }));
    assert_eq!(
        microsoft::body(&renamed),
        json!({ "location": { "displayName": "" } })
    );
    let timed = provider::EventDraft {
        starts_at: Some(utc("2026-11-21T09:00:00Z")),
        ends_at: Some(utc("2026-11-21T10:00:00Z")),
        all_day: Some(false),
        ..provider::EventDraft::default()
    };
    assert_eq!(
        microsoft::body(&timed)["start"],
        json!({ "dateTime": "2026-11-21T09:00:00", "timeZone": "UTC" })
    );
    assert_eq!(
        google::body(&timed)["end"],
        json!({ "dateTime": "2026-11-21T10:00:00Z" })
    );
    assert_eq!(
        google::event_url("primary", "a/b c"),
        "https://www.googleapis.com/calendar/v3/calendars/primary/events/a%2Fb%20c"
    );
}

fn standup() -> BotTask {
    let mut standup = task(
        "t-own",
        "Standup",
        TaskEvent {
            wake_offsets: vec![15],
            ..event("2026-11-23T09:00:00Z", Some("2026-11-23T09:15:00Z"))
        },
    );
    standup.cron = Some("0 9 * * MON".into());
    standup.tz = Some("UTC".into());
    standup.instruction = "Collect the blockers".into();
    standup.next_run_at = Some("2026-11-23T08:45:00Z".into());
    standup
}

fn enabled_wake() -> CalendarWake {
    CalendarWake {
        enabled: true,
        skip_matching: vec!["lunch".into()],
        ..CalendarWake::default()
    }
}

#[tokio::test]
async fn a_listing_joins_own_events_connected_events_and_their_wake_ups() {
    let scheduler = FakeScheduler::with_wake(enabled_wake());
    scheduler.add(standup());
    let mut kept = row(
        "t-row",
        "Board meeting",
        "2026-11-24T10:00:00Z",
        reference("c-google", "google", "g1"),
    );
    if let Some(event) = kept.event.as_mut() {
        event.wake = Some(EventWake {
            enabled: true,
            offsets_minutes: vec![30],
        });
        event.wake_offsets = vec![30];
    }
    kept.next_run_at = Some("2026-11-24T09:30:00Z".into());
    scheduler.add(kept);
    let transport = FakeTransport::default()
        .route(
            "GET",
            GOOGLE_EVENTS,
            200,
            json!({ "items": [
                google_event("g2", "Dentist", "2026-11-25T12:00:00Z", "2026-11-25T13:00:00Z"),
                google_event("g1", "Board meeting", "2026-11-24T10:00:00Z", "2026-11-24T11:00:00Z"),
                google_event("g3", "Lunch with Anna", "2026-11-26T12:00:00Z", "2026-11-26T13:00:00Z"),
            ]}),
        )
        .route(
            "GET",
            GRAPH_VIEW,
            403,
            json!({ "error": { "code": "ErrorAccessDenied", "message": "Access is denied." } }),
        );
    let source = source(vec![
        google(),
        microsoft(),
        account("c-gh", "GitHub", "github", "api.github.com"),
    ]);
    let client = Client::new(&source, &transport);
    let range = listing::collect(
        Some(&client),
        &scheduler,
        utc("2026-11-22T00:00:00Z"),
        utc("2026-12-01T00:00:00Z"),
        utc("2026-11-22T00:00:00Z"),
    )
    .await
    .unwrap()
    .range(utc("2026-11-22T00:00:00Z"), utc("2026-12-01T00:00:00Z"));
    let titles: Vec<&str> = range
        .entries
        .iter()
        .map(|entry| entry.title.as_str())
        .collect();
    assert_eq!(
        titles,
        [
            "Standup",
            "Board meeting",
            "Dentist",
            "Lunch with Anna",
            "Standup"
        ]
    );
    let first = &range.entries[0];
    assert_eq!(first.source, CalendarEntrySource::Bot);
    assert_eq!(first.notes.as_deref(), Some("Collect the blockers"));
    assert_eq!(first.ends_at.as_deref(), Some("2026-11-23T09:15:00Z"));
    assert!(first.recurring);
    assert_eq!(
        first.wake.as_ref().unwrap().next_at.as_deref(),
        Some("2026-11-23T08:45:00Z")
    );
    assert_eq!(range.entries[4].wake.as_ref().unwrap().next_at, None);
    let board = &range.entries[1];
    assert_eq!(board.task_id.as_deref(), Some("t-row"));
    assert_eq!(board.id, reference("c-google", "google", "g1").encode());
    assert_eq!(board.calendar.as_deref(), Some("Work"));
    let wake = board.wake.as_ref().unwrap();
    assert!(wake.own);
    assert_eq!(wake.offsets_minutes, vec![30]);
    let dentist = &range.entries[2];
    assert_eq!(dentist.task_id, None);
    assert_eq!(dentist.wake.as_ref().unwrap().offsets_minutes, vec![15]);
    assert!(!dentist.wake.as_ref().unwrap().own);
    assert_eq!(range.entries[3].wake, None);
    assert_eq!(range.sources.len(), 2);
    assert_eq!(range.sources[0].error, None);
    let denied = range.sources[1].error.as_deref().unwrap();
    assert!(
        denied.contains("reconnect") && denied.contains("Access is denied"),
        "{denied}"
    );
    let audits = source.audits.lock().unwrap();
    assert_eq!(audits.len(), 2);
    assert_eq!(audits[0].operation, "GET www.googleapis.com");
    assert!(audits[0].ok && !audits[1].ok);
}

#[tokio::test]
async fn a_listing_follows_pages_and_stops_at_the_cap() {
    let transport = FakeTransport::default()
        .page(
            GOOGLE_EVENTS,
            "pageToken=p2",
            json!({ "items": [google_event("b", "Second", "2026-11-23T10:00:00Z", "2026-11-23T11:00:00Z")] }),
        )
        .page(
            GOOGLE_EVENTS,
            "singleEvents=true",
            json!({ "nextPageToken": "p2", "items": [google_event("a", "First", "2026-11-22T10:00:00Z", "2026-11-22T11:00:00Z")] }),
        );
    let source = source(vec![google()]);
    let client = Client::new(&source, &transport);
    let calendars = provider::calendars(&source).await.unwrap();
    let listed = client
        .list(
            &calendars[0],
            utc("2026-11-22T00:00:00Z"),
            utc("2026-11-30T00:00:00Z"),
        )
        .await
        .unwrap();
    assert!(listed.complete);
    assert_eq!(listed.events.len(), 2);
    let first_url = transport.seen.lock().unwrap()[0].1.clone();
    let query: Vec<(String, String)> = first_url.query_pairs().into_owned().collect();
    assert!(query.contains(&("timeMin".into(), "2026-11-22T00:00:00Z".into())));
    assert!(query.contains(&("orderBy".into(), "startTime".into())));

    let endless = FakeTransport::default().page(
        GOOGLE_EVENTS,
        "singleEvents=true",
        json!({ "nextPageToken": "more", "items": [google_event("x", "Again", "2026-11-22T10:00:00Z", "2026-11-22T11:00:00Z")] }),
    );
    let client = Client::new(&source, &endless);
    let listed = client
        .list(
            &calendars[0],
            utc("2026-11-22T00:00:00Z"),
            utc("2026-11-30T00:00:00Z"),
        )
        .await
        .unwrap();
    assert!(!listed.complete);
    assert!(listed.note().unwrap().contains("shorter span"));
}

#[tokio::test]
async fn a_sync_plans_moves_and_removes_wake_ups() {
    let scheduler = FakeScheduler::with_wake(enabled_wake());
    scheduler.add(row(
        "r-moved",
        "Board meeting",
        "2026-11-21T10:00:00Z",
        reference("c-google", "google", "g1"),
    ));
    scheduler.add(row(
        "r-gone",
        "Cancelled call",
        "2026-11-21T15:00:00Z",
        reference("c-google", "google", "g9"),
    ));
    scheduler.add(row(
        "r-ended",
        "Old",
        "2026-11-19T09:00:00Z",
        reference("c-ms", "microsoft", "m1"),
    ));
    scheduler.add(row(
        "r-kept",
        "Later",
        "2026-11-22T09:00:00Z",
        reference("c-ms", "microsoft", "m2"),
    ));
    scheduler.add(row(
        "r-detached",
        "Detached",
        "2026-11-22T09:00:00Z",
        reference("c-gone", "google", "x"),
    ));
    scheduler.add(row(
        "r-lunch",
        "Meeting",
        "2026-11-21T12:00:00Z",
        reference("c-google", "google", "g3"),
    ));
    scheduler.add(standup());
    let mut holiday = google_event("g4", "Holiday", "", "");
    holiday["start"] = json!({ "date": "2026-11-22" });
    holiday["end"] = json!({ "date": "2026-11-23" });
    let transport = FakeTransport::default()
        .route(
            "GET",
            GOOGLE_EVENTS,
            200,
            json!({ "items": [
                google_event("g1", "Board meeting", "2026-11-21T11:00:00Z", "2026-11-21T12:00:00Z"),
                google_event("g2", "Dentist", "2026-11-21T14:00:00Z", "2026-11-21T15:00:00Z"),
                google_event("g3", "Lunch", "2026-11-21T12:00:00Z", "2026-11-21T13:00:00Z"),
                holiday,
            ]}),
        )
        .route(
            "GET",
            GRAPH_VIEW,
            500,
            json!({ "error": { "message": "busy" } }),
        );
    let source = source(vec![google(), microsoft()]);
    let client = Client::new(&source, &transport);
    let now = utc("2026-11-20T12:00:00Z");
    let report = sync::run(&client, &scheduler, now).await.unwrap();
    assert_eq!((report.created, report.moved, report.removed), (1, 1, 4));
    assert_eq!(report.errors.len(), 1);
    assert!(
        report.errors[0].starts_with("Outlook:"),
        "{:?}",
        report.errors
    );
    let mut cancelled = scheduler.cancelled.lock().unwrap().clone();
    cancelled.sort();
    assert_eq!(cancelled, ["r-detached", "r-ended", "r-gone", "r-lunch"]);
    let created = scheduler.created.lock().unwrap().clone();
    assert_eq!(created.len(), 1);
    assert_eq!(created[0].kind, Some(TaskKind::Event));
    assert_eq!(created[0].title.as_deref(), Some("Dentist"));
    assert_eq!(created[0].instruction, "");
    let planned = created[0].event.as_ref().unwrap();
    assert_eq!(
        planned.external,
        Some(reference("c-google", "google", "g2"))
    );
    assert_eq!(planned.starts_at, "2026-11-21T14:00:00Z");
    let updated = scheduler.updated.lock().unwrap().clone();
    assert_eq!(updated.len(), 1);
    assert_eq!(updated[0].0, "r-moved");
    let moved = updated[0].1.event.as_ref().unwrap();
    assert_eq!(moved.starts_at, "2026-11-21T11:00:00Z");
    assert_eq!(moved.ends_at.as_deref(), Some("2026-11-21T12:00:00Z"));
    let window = transport.seen.lock().unwrap()[0].1.clone();
    let time_max = window
        .query_pairs()
        .find(|(key, _)| key == "timeMax")
        .unwrap()
        .1
        .into_owned();
    assert_eq!(time_max, "2026-11-22T00:15:00Z");

    let again = sync::run(&client, &scheduler, now).await.unwrap();
    assert_eq!((again.created, again.moved, again.removed), (0, 0, 0));
}

#[tokio::test]
async fn with_wake_ups_off_a_sync_removes_rows_without_reading_calendars() {
    let scheduler = FakeScheduler::default();
    scheduler.add(row(
        "r1",
        "Call",
        "2026-11-21T10:00:00Z",
        reference("c-google", "google", "g1"),
    ));
    let transport = FakeTransport::default();
    let source = source(vec![google()]);
    let report = sync::run(
        &Client::new(&source, &transport),
        &scheduler,
        utc("2026-11-20T12:00:00Z"),
    )
    .await
    .unwrap();
    assert_eq!(report.removed, 1);
    assert!(transport.seen.lock().unwrap().is_empty());
}

async fn env_with(
    scheduler: std::sync::Arc<FakeScheduler>,
    entries: Vec<ConnectorEntry>,
) -> FakeEnv {
    let env = FakeEnv::new().await;
    *env.scheduler.lock().unwrap() = Some(scheduler);
    *env.connectors.lock().unwrap() = Some(std::sync::Arc::new(source(entries)));
    env
}

fn create_args(value: Value) -> CalendarCreateArgs {
    serde_json::from_value(value).unwrap()
}

#[tokio::test]
async fn the_calendar_is_private_to_the_owners_chat_except_free_slots() {
    let scheduler = std::sync::Arc::new(FakeScheduler::default());
    scheduler.elsewhere.store(true, Ordering::SeqCst);
    let env = env_with(scheduler.clone(), vec![]).await;
    for (tool, args) in [
        (CALENDAR_LIST, json!({})),
        (
            CALENDAR_CREATE,
            json!({ "title": "x", "starts_at": "2030-01-01T10:00:00Z" }),
        ),
        (CALENDAR_UPDATE, json!({ "id": "t1", "title": "y" })),
        (CALENDAR_DELETE, json!({ "id": "t1" })),
        (CALENDAR_WAKE_SETTINGS, json!({})),
    ] {
        let outcome = crate::run(tool, args, &env).await;
        assert_eq!(
            outcome.error.expect(tool).code,
            code::SCOPE_REFUSED,
            "{tool}"
        );
    }
    assert!(scheduler.created.lock().unwrap().is_empty());
    let slots = crate::run(
        CALENDAR_FREE_SLOTS,
        json!({ "from": "2030-01-07T00:00:00Z", "to": "2030-01-08T00:00:00Z", "duration_minutes": 60 }),
        &env,
    )
    .await;
    assert!(slots.is_ok(), "{:?}", slots.error);
    assert_eq!(slots.result["count"], 1);
}

#[tokio::test]
async fn an_own_event_is_an_event_task_with_the_notes_as_instruction() {
    let scheduler = std::sync::Arc::new(FakeScheduler::with_wake(enabled_wake()));
    let env = env_with(scheduler.clone(), vec![]).await;
    let outcome = crate::run(
        CALENDAR_CREATE,
        json!({
            "title": "Dentist",
            "starts_at": "2030-03-04T10:00",
            "tz": "Europe/Berlin",
            "notes": "Bring the insurance card",
            "location": " Praxis ",
            "wake": { "enabled": true, "offsets_minutes": [60] },
        }),
        &env,
    )
    .await;
    assert!(outcome.is_ok(), "{:?}", outcome.error);
    let created = scheduler.created.lock().unwrap()[0].clone();
    assert_eq!(created.kind, Some(TaskKind::Event));
    assert_eq!(created.instruction, "Bring the insurance card");
    assert_eq!(created.tz.as_deref(), Some("Europe/Berlin"));
    let event = created.event.as_ref().unwrap();
    assert_eq!(event.starts_at, "2030-03-04T09:00:00Z");
    assert_eq!(event.ends_at, None);
    assert_eq!(event.location.as_deref(), Some("Praxis"));
    assert_eq!(event.wake.as_ref().unwrap().offsets_minutes, vec![60]);
    assert_eq!(
        outcome.result["event"]["wake"]["offsets_minutes"],
        json!([60])
    );
    assert_eq!(outcome.result["calendar"], "own");

    let repeating = crate::run(
        CALENDAR_CREATE,
        json!({ "title": "Weekly", "starts_at": "2030-03-04T09:00:00Z", "repeat": { "cron": "0 9 * * MON" } }),
        &env,
    )
    .await;
    assert!(repeating.is_ok(), "{:?}", repeating.error);
    assert_eq!(created_cron(&scheduler), Some("0 9 * * MON".into()));

    for args in [
        json!({ "title": "x", "starts_at": "2030-03-04T10:00" }),
        json!({ "title": "x", "starts_at": "2030-03-04" }),
        json!({ "title": " ", "starts_at": "2030-03-04T10:00:00Z" }),
        json!({ "title": "x", "starts_at": "2030-03-04T10:00:00Z", "ends_at": "2030-03-04T09:00:00Z" }),
        json!({ "title": "x", "starts_at": "2030-03-04T10:00:00Z", "repeat": { "cron": "* * * * *" } }),
    ] {
        let outcome = crate::run(CALENDAR_CREATE, args.clone(), &env).await;
        assert_eq!(
            outcome.error.expect("refused").code,
            code::INVALID_ARGS,
            "{args}"
        );
    }
}

fn created_cron(scheduler: &FakeScheduler) -> Option<String> {
    scheduler
        .created
        .lock()
        .unwrap()
        .last()
        .and_then(|request| request.cron.clone())
}

#[tokio::test]
async fn an_all_day_event_runs_from_midnight_to_after_its_last_day() {
    let scheduler = std::sync::Arc::new(FakeScheduler::default());
    let env = env_with(scheduler.clone(), vec![]).await;
    let outcome = crate::run(
        CALENDAR_CREATE,
        json!({ "title": "Trip", "starts_at": "2030-05-01", "ends_at": "2030-05-03", "all_day": true, "tz": "Europe/Berlin" }),
        &env,
    )
    .await;
    assert!(outcome.is_ok(), "{:?}", outcome.error);
    let created = scheduler.created.lock().unwrap();
    let event = created[0].event.as_ref().unwrap();
    assert!(event.all_day);
    assert_eq!(event.starts_at, "2030-04-30T22:00:00Z");
    assert_eq!(event.ends_at.as_deref(), Some("2030-05-03T22:00:00Z"));
}

#[tokio::test]
async fn adding_to_a_connected_calendar_writes_at_the_provider_and_keeps_its_wake_up() {
    let scheduler = std::sync::Arc::new(FakeScheduler::default());
    let env = env_with(scheduler.clone(), vec![google()]).await;
    let transport = FakeTransport::default().route(
        "POST",
        GOOGLE_EVENTS,
        200,
        google_event(
            "new1",
            "Dentist",
            "2030-03-04T09:00:00Z",
            "2030-03-04T10:00:00Z",
        ),
    );
    let output = create_with(
        &env,
        &transport,
        create_args(json!({
            "title": "Dentist",
            "starts_at": "2030-03-04T10:00:00+01:00",
            "calendar": "c-google",
            "notes": "Bring the card",
            "wake": { "enabled": true, "offsets_minutes": [30] },
            "policy": "ask",
        })),
    )
    .await
    .unwrap();
    assert_eq!(transport.methods(), [format!("POST {GOOGLE_EVENTS}")]);
    let body = transport.body_of("POST").unwrap();
    assert_eq!(body["summary"], "Dentist");
    assert_eq!(body["description"], "Bring the card");
    assert_eq!(body["start"]["dateTime"], "2030-03-04T09:00:00Z");
    assert_eq!(body["end"]["dateTime"], "2030-03-04T10:00:00Z");
    let id = reference("c-google", "google", "new1").encode();
    assert_eq!(output.result["event"]["id"], id);
    assert_eq!(output.result["untrusted"], true);
    assert_eq!(output.result["event"]["wake"]["own"], true);
    let created = scheduler.created.lock().unwrap()[0].clone();
    assert_eq!(
        created.event.unwrap().external,
        Some(reference("c-google", "google", "new1"))
    );
    let refused = create_with(
        &env,
        &transport,
        create_args(json!({ "title": "x", "starts_at": "2030-03-04T10:00:00Z", "calendar": "Work", "repeat": { "cron": "0 9 * * MON" } })),
    )
    .await
    .unwrap_err();
    assert_eq!(refused.code, code::INVALID_ARGS);
}

#[tokio::test]
async fn a_calendar_whose_writes_are_off_is_never_written() {
    let scheduler = std::sync::Arc::new(FakeScheduler::default());
    let mut locked = google();
    locked
        .rule
        .tool_policies
        .insert("write".into(), ConnectorPolicy::Never);
    let env = env_with(scheduler.clone(), vec![locked]).await;
    let transport = FakeTransport::default();
    let refused = create_with(
        &env,
        &transport,
        create_args(
            json!({ "title": "x", "starts_at": "2030-03-04T10:00:00Z", "calendar": "Work" }),
        ),
    )
    .await
    .unwrap_err();
    assert_eq!(refused.code, code::SCOPE_REFUSED);
    let deleted = delete_with(
        &env,
        &transport,
        CalendarDeleteArgs {
            id: reference("c-google", "google", "g1").encode(),
            calendar: None,
        },
    )
    .await
    .unwrap_err();
    assert_eq!(deleted.code, code::SCOPE_REFUSED);
    assert!(transport.seen.lock().unwrap().is_empty());
}

fn update_args(value: Value) -> CalendarUpdateArgs {
    serde_json::from_value(value).unwrap()
}

#[tokio::test]
async fn a_wake_change_on_a_connected_event_never_writes_to_the_provider() {
    let scheduler = std::sync::Arc::new(FakeScheduler::with_wake(enabled_wake()));
    scheduler.add(row(
        "r1",
        "Board meeting",
        "2030-03-04T10:00:00Z",
        reference("c-google", "google", "g1"),
    ));
    let env = env_with(scheduler.clone(), vec![google()]).await;
    let event_path = format!("{GOOGLE_EVENTS}/g1");
    let transport = FakeTransport::default().route(
        "GET",
        &event_path,
        200,
        google_event(
            "g1",
            "Board meeting",
            "2030-03-04T10:00:00Z",
            "2030-03-04T11:00:00Z",
        ),
    );
    let id = reference("c-google", "google", "g1").encode();
    let output = update_with(
        &env,
        &transport,
        update_args(json!({ "id": id, "wake": { "enabled": false } })),
    )
    .await
    .unwrap();
    assert_eq!(transport.methods(), [format!("GET {event_path}")]);
    assert_eq!(output.result["updated"], true);
    let kept = scheduler.tasks.lock().unwrap()[0].clone();
    assert_eq!(kept.id, "r1");
    let event = kept.event.unwrap();
    assert_eq!(event.wake.map(|wake| wake.enabled), Some(false));
    assert!(event.wake_offsets.is_empty());

    let output = update_with(
        &env,
        &transport,
        update_args(json!({ "id": id, "wake": "default" })),
    )
    .await
    .unwrap();
    assert_eq!(output.result["updated"], true);
    let back = scheduler.tasks.lock().unwrap()[0].clone();
    assert_eq!(back.event.as_ref().unwrap().wake, None);
    assert_eq!(back.event.unwrap().wake_offsets, vec![15]);
}

#[tokio::test]
async fn moving_a_connected_event_patches_it_and_moves_its_wake_up() {
    let scheduler = std::sync::Arc::new(FakeScheduler::with_wake(enabled_wake()));
    scheduler.add(row(
        "r1",
        "Board meeting",
        "2030-03-04T10:00:00Z",
        reference("c-ms", "microsoft", "AAMk-1="),
    ));
    let env = env_with(scheduler.clone(), vec![microsoft()]).await;
    let event_path = "/v1.0/me/events/AAMk-1%3D";
    let graph = |start: &str, end: &str| {
        json!({
            "id": "AAMk-1=",
            "subject": "Board meeting",
            "start": { "dateTime": start, "timeZone": "UTC" },
            "end": { "dateTime": end, "timeZone": "UTC" },
        })
    };
    let transport = FakeTransport::default()
        .route(
            "GET",
            event_path,
            200,
            graph("2030-03-04T10:00:00", "2030-03-04T11:30:00"),
        )
        .route(
            "PATCH",
            event_path,
            200,
            graph("2030-03-05T08:00:00", "2030-03-05T09:30:00"),
        );
    let id = reference("c-ms", "microsoft", "AAMk-1=").encode();
    let output = update_with(
        &env,
        &transport,
        update_args(json!({ "id": id, "starts_at": "2030-03-05T08:00:00Z" })),
    )
    .await
    .unwrap();
    assert_eq!(
        transport.methods(),
        [format!("GET {event_path}"), format!("PATCH {event_path}")]
    );
    let body = transport.body_of("PATCH").unwrap();
    assert_eq!(body["start"]["dateTime"], "2030-03-05T08:00:00");
    assert_eq!(body["end"]["dateTime"], "2030-03-05T09:30:00");
    assert!(body.get("subject").is_none());
    assert_eq!(output.result["event"]["starts_at"], "2030-03-05T08:00:00Z");
    let moved = scheduler.tasks.lock().unwrap()[0].clone();
    assert_eq!(moved.id, "r1");
    assert_eq!(moved.event.unwrap().starts_at, "2030-03-05T08:00:00Z");
}

#[tokio::test]
async fn an_own_event_moves_with_its_length_and_deletes_by_id() {
    let scheduler = std::sync::Arc::new(FakeScheduler::default());
    scheduler.add(task(
        "t1",
        "Review",
        event("2030-03-04T10:00:00Z", Some("2030-03-04T11:30:00Z")),
    ));
    let env = env_with(scheduler.clone(), vec![]).await;
    let moved = crate::run(
        CALENDAR_UPDATE,
        json!({ "id": "t1", "starts_at": "2030-03-06T08:00:00Z", "title": "Design review" }),
        &env,
    )
    .await;
    assert!(moved.is_ok(), "{:?}", moved.error);
    let (id, request) = scheduler.updated.lock().unwrap()[0].clone();
    assert_eq!(id, "t1");
    assert_eq!(request.title.as_deref(), Some("Design review"));
    assert_eq!(request.instruction, None);
    let event = request.event.unwrap();
    assert_eq!(event.starts_at, "2030-03-06T08:00:00Z");
    assert_eq!(event.ends_at.as_deref(), Some("2030-03-06T09:30:00Z"));
    let nothing = crate::run(CALENDAR_UPDATE, json!({ "id": "t1" }), &env).await;
    assert_eq!(nothing.error.unwrap().code, code::INVALID_ARGS);
    let missing = crate::run(CALENDAR_DELETE, json!({ "id": "nope" }), &env).await;
    assert_eq!(missing.error.unwrap().code, code::NOT_FOUND);
    let deleted = crate::run(CALENDAR_DELETE, json!({ "id": "t1" }), &env).await;
    assert!(deleted.is_ok(), "{:?}", deleted.error);
    assert_eq!(*scheduler.cancelled.lock().unwrap(), ["t1"]);
}

#[tokio::test]
async fn deleting_a_connected_event_removes_its_wake_up_too() {
    let scheduler = std::sync::Arc::new(FakeScheduler::default());
    scheduler.add(row(
        "r1",
        "Board meeting",
        "2030-03-04T10:00:00Z",
        reference("c-google", "google", "g1"),
    ));
    let env = env_with(scheduler.clone(), vec![google()]).await;
    let event_path = format!("{GOOGLE_EVENTS}/g1");
    let transport = FakeTransport::default().route("DELETE", &event_path, 204, Value::Null);
    let output = delete_with(
        &env,
        &transport,
        CalendarDeleteArgs {
            id: reference("c-google", "google", "g1").encode(),
            calendar: Some("Work".into()),
        },
    )
    .await
    .unwrap();
    assert_eq!(output.result["deleted"], true);
    assert_eq!(*scheduler.cancelled.lock().unwrap(), ["r1"]);
    let mismatch = delete_with(
        &env,
        &transport,
        CalendarDeleteArgs {
            id: reference("c-google", "google", "g1").encode(),
            calendar: Some("Home".into()),
        },
    )
    .await
    .unwrap_err();
    assert_eq!(mismatch.code, code::INVALID_ARGS);
}

#[tokio::test]
async fn free_slots_merge_every_calendar_and_reveal_nothing_else() {
    let scheduler = std::sync::Arc::new(FakeScheduler::default());
    scheduler.add(task(
        "t1",
        "Private thing",
        event("2030-01-07T10:00:00Z", Some("2030-01-07T11:00:00Z")),
    ));
    scheduler.add(task("t2", "Reminder", event("2030-01-07T15:00:00Z", None)));
    scheduler.elsewhere.store(true, Ordering::SeqCst);
    let env = env_with(scheduler.clone(), vec![google(), microsoft()]).await;
    let transport = FakeTransport::default()
        .route(
            "GET",
            GOOGLE_EVENTS,
            200,
            json!({ "items": [google_event("g1", "Secret", "2030-01-07T13:00:00Z", "2030-01-07T14:00:00Z")] }),
        )
        .route("GET", GRAPH_VIEW, 401, json!({}));
    let output = free_slots_with(
        &env,
        &transport,
        serde_json::from_value::<CalendarFreeSlotsArgs>(json!({
            "from": "2030-01-07T00:00:00Z",
            "to": "2030-01-08T00:00:00Z",
            "duration_minutes": 60,
        }))
        .unwrap(),
    )
    .await
    .unwrap();
    let slots: Vec<(String, String)> = output.result["slots"]
        .as_array()
        .unwrap()
        .iter()
        .map(|slot| {
            (
                slot["starts_at"].as_str().unwrap().to_string(),
                slot["ends_at"].as_str().unwrap().to_string(),
            )
        })
        .collect();
    assert_eq!(
        slots,
        [
            (
                "2030-01-07T09:00:00+00:00".to_string(),
                "2030-01-07T10:00:00+00:00".to_string()
            ),
            (
                "2030-01-07T11:00:00+00:00".to_string(),
                "2030-01-07T13:00:00+00:00".to_string()
            ),
            (
                "2030-01-07T14:00:00+00:00".to_string(),
                "2030-01-07T15:00:00+00:00".to_string()
            ),
            (
                "2030-01-07T15:30:00+00:00".to_string(),
                "2030-01-07T18:00:00+00:00".to_string()
            ),
        ]
    );
    assert_eq!(output.result["unread_calendars"], 1);
    let text = output.result.to_string();
    assert!(!text.contains("Secret") && !text.contains("Private") && !text.contains("Outlook"));
}

#[tokio::test]
async fn wake_settings_read_without_arguments_and_change_with_them() {
    let scheduler = std::sync::Arc::new(FakeScheduler::default());
    let env = env_with(scheduler.clone(), vec![]).await;
    let read = crate::run(CALENDAR_WAKE_SETTINGS, json!({}), &env).await;
    assert_eq!(read.result["changed"], false);
    assert_eq!(read.result["settings"]["enabled"], false);
    let changed = crate::run(
        CALENDAR_WAKE_SETTINGS,
        json!({ "enabled": true, "skip_matching": ["Focus"] }),
        &env,
    )
    .await;
    assert_eq!(changed.result["changed"], true);
    assert_eq!(
        changed.result["settings"]["skip_matching"],
        json!(["Focus"])
    );
    assert!(scheduler.wake.lock().unwrap().enabled);
    let bad = crate::run(
        CALENDAR_WAKE_SETTINGS,
        json!({ "offsets_minutes": [1, 2, 3, 4, 5] }),
        &env,
    )
    .await;
    assert_eq!(bad.error.unwrap().code, code::INVALID_ARGS);
}

#[test]
fn only_writes_to_a_connected_calendar_ask_and_they_name_it() {
    let external = reference("c-google", "google", "g1").encode();
    assert_eq!(
        connector_write(CALENDAR_CREATE, &json!({ "calendar": " Work " })),
        Some(("Work".to_string(), "POST"))
    );
    assert_eq!(
        connector_write(CALENDAR_CREATE, &json!({ "calendar": "" })),
        None
    );
    assert_eq!(
        connector_write(
            CALENDAR_UPDATE,
            &json!({ "id": external, "wake": { "enabled": false } })
        ),
        None
    );
    assert_eq!(
        connector_write(
            CALENDAR_UPDATE,
            &json!({ "id": external, "all_day": false })
        ),
        Some(("c-google".to_string(), "PATCH"))
    );
    assert_eq!(
        connector_write(CALENDAR_DELETE, &json!({ "id": external })),
        Some(("c-google".to_string(), "DELETE"))
    );
    assert_eq!(
        connector_write(CALENDAR_DELETE, &json!({ "id": "t1" })),
        None
    );
    assert_eq!(
        connector_write(CALENDAR_LIST, &json!({ "calendar": "Work" })),
        None
    );
    assert_eq!(
        crate::services::grant_scope(
            CALENDAR_DELETE,
            &json!({ "calendar": "c-google", "id": external })
        ),
        Some("connector:c-google:calendar:DELETE".to_string())
    );
    assert_eq!(
        grant_scope(CALENDAR_DELETE, &json!({ "id": external })),
        None
    );
    assert_eq!(
        ask_message(
            CALENDAR_UPDATE,
            &json!({ "id": external, "title": "Board", "notes": "long text", "connector_label": "Work" })
        )
        .as_deref(),
        Some("The bot wants to change an event in your calendar 'Work': title to \"Board\", notes.")
    );
    assert_eq!(ask_message(CALENDAR_LIST, &json!({})), None);
}

#[test]
fn the_specs_belong_to_the_calendar_capability() {
    let specs = specs();
    assert_eq!(specs.len(), 6);
    for spec in &specs {
        assert_eq!(spec.capability, Some("calendar"), "{}", spec.name);
        assert_eq!(spec.module, "calendar");
        assert_eq!(
            crate::spec(spec.name).map(|found| found.name),
            Some(spec.name)
        );
        let asks_by_policy = spec.approval_args == [POLICY_FIELD];
        assert_eq!(
            asks_by_policy,
            write_method(spec.name).is_some(),
            "{}",
            spec.name
        );
        assert_eq!(
            crate::services::POLICY_TOOLS.contains(&spec.name),
            asks_by_policy
        );
    }
    assert!(!crate::spec(CALENDAR_LIST).unwrap().mutating);
}

#[test]
fn an_external_event_becomes_a_checked_wake_up_row() {
    let long = ExternalEvent::new(
        "e1",
        Some(&"t".repeat(300)),
        utc("2026-11-21T10:00:00Z"),
        Some(utc("2027-02-01T10:00:00Z")),
        false,
        Some(&"l".repeat(400)),
        None,
        Some("javascript:alert(1)"),
        false,
        true,
    );
    assert_eq!(long.title.chars().count(), 200);
    assert_eq!(long.link, None);
    let row = long.task_event(reference("c-google", "google", "e1"), None);
    let checked = row.check().unwrap();
    assert_eq!(checked.ends_at.as_deref(), Some("2026-12-22T10:00:00Z"));
    assert_eq!(
        Duration::days(31),
        utc("2026-12-22T10:00:00Z") - utc("2026-11-21T10:00:00Z")
    );
}

/// One job as it was sent: the call and each connector id with its `Authorization`.
type SentJob = (Value, Vec<(String, Option<String>)>);

/// What a server's executor does with a calendar job, run here: the job's connectors as a
/// `StaticSource` and the providers answering from `transport`.
struct InProcessJobs<T> {
    transport: T,
    refused: Option<crate::error::ToolError>,
    calls: Mutex<Vec<SentJob>>,
}

impl<T> InProcessJobs<T> {
    fn new(transport: T) -> Self {
        Self {
            transport,
            refused: None,
            calls: Mutex::default(),
        }
    }
}

#[async_trait]
impl<T: Transport> CalendarJobs for InProcessJobs<T> {
    fn ready(&self) -> crate::error::ToolResult<()> {
        self.refused.clone().map_or(Ok(()), Err)
    }

    async fn run(
        &self,
        call: &CalendarCall,
        connectors: Vec<JobCalendar>,
    ) -> crate::error::ToolResult<String> {
        let wire = serde_json::to_value(call).unwrap();
        let sent = connectors
            .iter()
            .map(|connector| {
                (
                    connector.entry.rule.id.clone(),
                    connector.credential.header().map(str::to_string),
                )
            })
            .collect();
        self.calls.lock().unwrap().push((wire.clone(), sent));
        let env = FakeEnv::new().await;
        *env.connectors.lock().unwrap() =
            Some(std::sync::Arc::new(crate::services::StaticSource::new(
                connectors
                    .into_iter()
                    .map(|connector| (connector.entry, connector.credential))
                    .collect(),
            )));
        let call: CalendarCall = serde_json::from_value(wire).unwrap();
        let outcome = match jobs::run_job(&env, &self.transport, call).await {
            Ok(output) => crate::ToolOutcome::from_output(CALENDAR_JOB, output, MAX_REPLY_BYTES),
            Err(error) => crate::ToolOutcome::from_error(CALENDAR_JOB, error),
        };
        Ok(outcome.envelope().to_string())
    }
}

/// A job whose output is fixed: what a misbehaving executor could send.
struct CannedJobs(String);

#[async_trait]
impl CalendarJobs for CannedJobs {
    async fn run(
        &self,
        _call: &CalendarCall,
        _connectors: Vec<JobCalendar>,
    ) -> crate::error::ToolResult<String> {
        Ok(self.0.clone())
    }
}

fn canned(reply: &JobReply) -> String {
    let output = crate::ToolOutput::new(json!({ "calendar": reply }));
    crate::ToolOutcome::from_output(CALENDAR_JOB, output, MAX_REPLY_BYTES)
        .envelope()
        .to_string()
}

fn delegated(entries: Vec<ConnectorEntry>, jobs: std::sync::Arc<dyn CalendarJobs>) -> FakeSource {
    FakeSource {
        entries,
        jobs: Some(jobs),
        ..FakeSource::default()
    }
}

fn job_event(id: &str, title: &str) -> ExternalEvent {
    ExternalEvent {
        event_id: id.into(),
        title: title.into(),
        starts_at: utc("2030-01-07T09:00:00Z"),
        ends_at: None,
        all_day: false,
        location: None,
        notes: None,
        link: None,
        recurring: false,
        busy: true,
    }
}

#[tokio::test]
async fn a_server_listing_reads_every_calendar_in_one_job_and_never_calls_a_provider_itself() {
    let scheduler = FakeScheduler::with_wake(enabled_wake());
    scheduler.add(standup());
    let provider = FakeTransport::default()
        .route(
            "GET",
            GOOGLE_EVENTS,
            200,
            json!({ "items": [google_event("g2", "Dentist", "2026-11-25T12:00:00Z", "2026-11-25T13:00:00Z")] }),
        )
        .route(
            "GET",
            GRAPH_VIEW,
            403,
            json!({ "error": { "message": "Access is denied." } }),
        );
    let jobs = std::sync::Arc::new(InProcessJobs::new(provider));
    let source = delegated(vec![google(), microsoft()], jobs.clone());
    let here = FakeTransport::default();
    let client = Client::new(&source, &here);
    assert!(client.delegates());
    let (from, to) = (utc("2026-11-22T00:00:00Z"), utc("2026-12-01T00:00:00Z"));
    let range = listing::collect(Some(&client), &scheduler, from, to, from)
        .await
        .unwrap()
        .range(from, to);
    assert!(here.seen.lock().unwrap().is_empty());
    {
        let calls = jobs.calls.lock().unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(
            calls[0].0,
            json!({ "op": "list", "from": "2026-11-22T00:00:00Z", "to": "2026-12-01T00:00:00Z" })
        );
        let bearer = format!("Bearer {TOKEN}");
        let sent: Vec<(&str, Option<&str>)> = calls[0]
            .1
            .iter()
            .map(|(id, header)| (id.as_str(), header.as_deref()))
            .collect();
        assert_eq!(
            sent,
            [
                ("c-google", Some(bearer.as_str())),
                ("c-ms", Some(bearer.as_str()))
            ]
        );
    }
    let dentist = range
        .entries
        .iter()
        .find(|entry| entry.title == "Dentist")
        .unwrap();
    assert_eq!(dentist.source, CalendarEntrySource::External);
    assert_eq!(dentist.calendar.as_deref(), Some("Work"));
    assert_eq!(dentist.wake.as_ref().unwrap().offsets_minutes, vec![15]);
    assert_eq!(range.sources[0].error, None);
    let denied = range.sources[1].error.as_deref().unwrap();
    assert!(denied.contains("Access is denied"), "{denied}");
    let audits = source.audits.lock().unwrap();
    let recorded: Vec<(&str, &str, bool)> = audits
        .iter()
        .map(|audit| (audit.label.as_str(), audit.operation.as_str(), audit.ok))
        .collect();
    assert_eq!(
        recorded,
        [
            ("Work", "GET www.googleapis.com", true),
            ("Outlook", "GET graph.microsoft.com", false)
        ]
    );
}

#[tokio::test]
async fn a_server_sync_plans_wake_ups_from_one_job() {
    let scheduler = FakeScheduler::with_wake(enabled_wake());
    let soon = Utc::now() + Duration::hours(3);
    let provider = FakeTransport::default().route(
        "GET",
        GOOGLE_EVENTS,
        200,
        json!({ "items": [google_event(
            "g1",
            "Review",
            &provider::rfc3339(soon),
            &provider::rfc3339(soon + Duration::hours(1)),
        )] }),
    );
    let jobs = std::sync::Arc::new(InProcessJobs::new(provider));
    let source = delegated(vec![google()], jobs.clone());
    let here = FakeTransport::default();
    let report = sync::run(&Client::new(&source, &here), &scheduler, Utc::now())
        .await
        .unwrap();
    assert_eq!(report.created, 1, "{report:?}");
    assert!(here.seen.lock().unwrap().is_empty());
    assert_eq!(jobs.calls.lock().unwrap().len(), 1);
    let created = scheduler.created.lock().unwrap()[0].clone();
    assert_eq!(
        created.event.unwrap().external,
        Some(reference("c-google", "google", "g1"))
    );
}

#[tokio::test]
async fn server_writes_send_typed_calls_and_keep_the_wake_up_here() {
    let provider = FakeTransport::default()
        .route(
            "POST",
            GOOGLE_EVENTS,
            200,
            google_event(
                "new1",
                "Dentist",
                "2030-01-07T09:00:00Z",
                "2030-01-07T10:00:00Z",
            ),
        )
        .route("DELETE", &format!("{GOOGLE_EVENTS}/new1"), 410, Value::Null);
    let jobs = std::sync::Arc::new(InProcessJobs::new(provider));
    let scheduler = std::sync::Arc::new(FakeScheduler::default());
    let env = FakeEnv::new().await;
    *env.scheduler.lock().unwrap() = Some(scheduler.clone());
    *env.connectors.lock().unwrap() =
        Some(std::sync::Arc::new(delegated(vec![google()], jobs.clone())));
    let here = FakeTransport::default();
    let output = create_with(
        &env,
        &here,
        create_args(json!({
            "title": "Dentist",
            "starts_at": "2030-01-07T10:00",
            "tz": "Europe/Berlin",
            "calendar": "Work",
            "wake": { "enabled": true, "offsets_minutes": [30] },
        })),
    )
    .await
    .unwrap();
    assert_eq!(output.result["event"]["title"], "Dentist");
    assert_eq!(output.result["event"]["wake"]["own"], true);
    let create = jobs.calls.lock().unwrap()[0].0.clone();
    assert_eq!(create["op"], "create");
    assert_eq!(create["draft"]["title"], "Dentist");
    assert_eq!(create["draft"]["tz"], "Europe/Berlin");
    assert_eq!(create["draft"]["starts_at"], "2030-01-07T09:00:00Z");
    assert_eq!(scheduler.created.lock().unwrap().len(), 1);
    let deleted = delete_with(
        &env,
        &here,
        CalendarDeleteArgs {
            id: reference("c-google", "google", "new1").encode(),
            calendar: None,
        },
    )
    .await
    .unwrap();
    assert_eq!(deleted.result["deleted"], true);
    assert_eq!(
        jobs.calls.lock().unwrap()[1].0,
        json!({ "op": "delete", "event_id": "new1" })
    );
    assert!(here.seen.lock().unwrap().is_empty());
}

#[tokio::test]
async fn without_an_executor_connected_calendars_are_refused_before_any_sign_in() {
    let mut jobs = InProcessJobs::new(FakeTransport::default());
    jobs.refused = Some(crate::error::ToolError::new(
        code::NOT_YET,
        "This server has no executor for calendar calls",
    ));
    let jobs = std::sync::Arc::new(jobs);
    let scheduler = FakeScheduler::with_wake(enabled_wake());
    scheduler.add(standup());
    let source = delegated(vec![google()], jobs.clone());
    let here = FakeTransport::default();
    let client = Client::new(&source, &here);
    let (from, to) = (utc("2026-11-22T00:00:00Z"), utc("2026-12-01T00:00:00Z"));
    let range = listing::collect(Some(&client), &scheduler, from, to, from)
        .await
        .unwrap()
        .range(from, to);
    assert!(!range.entries.is_empty());
    assert!(
        range
            .entries
            .iter()
            .all(|entry| entry.source == CalendarEntrySource::Bot)
    );
    assert!(
        range.sources[0]
            .error
            .as_deref()
            .unwrap()
            .contains("no executor")
    );
    let calendars = provider::calendars(&source).await.unwrap();
    let refused = client.get(&calendars[0], "g1").await.unwrap_err();
    assert_eq!(refused.code, code::NOT_YET);
    assert_eq!(source.credentials.load(Ordering::SeqCst), 0);
    assert!(jobs.calls.lock().unwrap().is_empty());
}

#[tokio::test]
async fn answers_of_a_job_are_bounded_before_the_host_keeps_them() {
    let mut events: Vec<ExternalEvent> = (0..1_200)
        .map(|n| job_event(&format!("e{n}"), "Slot"))
        .collect();
    events[0] = ExternalEvent {
        title: "t".repeat(5_000),
        notes: Some("n".repeat(9_000)),
        link: Some("javascript:alert(1)".into()),
        ..job_event("e0", "")
    };
    events[1].event_id = "x".repeat(flow_like_types::bot_calendar::MAX_EXTERNAL_ID_CHARS + 1);
    let reply = JobReply {
        answer: JobAnswer::Listed {
            listings: vec![
                JobListing {
                    connector_id: "c-google".into(),
                    listed: Some(provider::Listed {
                        events,
                        complete: true,
                        unreadable: 0,
                    }),
                    error: None,
                },
                JobListing {
                    connector_id: "c-stranger".into(),
                    listed: Some(provider::Listed::default()),
                    error: None,
                },
            ],
        },
        audits: (0..100)
            .map(|n| JobAudit {
                connector_id: if n % 2 == 0 { "c-google" } else { "c-stranger" }.into(),
                operation: "GET www.googleapis.com".into(),
                write: false,
                ok: true,
            })
            .collect(),
    };
    let source = delegated(
        vec![google(), microsoft()],
        std::sync::Arc::new(CannedJobs(canned(&reply))),
    );
    let here = FakeTransport::default();
    let client = Client::new(&source, &here);
    let calendars = provider::calendars(&source).await.unwrap();
    let results = client
        .list_all(
            &calendars,
            utc("2030-01-01T00:00:00Z"),
            utc("2030-02-01T00:00:00Z"),
        )
        .await;
    let listed = results[0].as_ref().unwrap();
    assert_eq!(listed.events.len(), MAX_JOB_EVENTS - 1);
    assert_eq!(listed.unreadable, 1);
    assert!(!listed.complete);
    let first = listed
        .events
        .iter()
        .find(|event| event.event_id == "e0")
        .unwrap();
    assert_eq!(first.title.chars().count(), 200);
    assert_eq!(
        first.notes.as_deref().map(|notes| notes.chars().count()),
        Some(provider::MAX_NOTES_CHARS)
    );
    assert_eq!(first.link, None);
    let missing = results[1].as_ref().unwrap_err();
    assert!(missing.message.contains("'Outlook'"), "{}", missing.message);
    let audits = source.audits.lock().unwrap();
    assert_eq!(audits.len(), 32);
    assert!(audits.iter().all(|audit| audit.connector_id == "c-google"));
}

#[tokio::test]
async fn a_job_answer_that_is_too_large_cut_failed_or_off_topic_is_refused() {
    let missing = JobReply {
        answer: JobAnswer::Failed {
            error: JobError {
                code: "not_found".into(),
                message: "'Work' has no such event (HTTP 404)".into(),
                http_status: Some(404),
            },
        },
        audits: Vec::new(),
    };
    let deleted = JobReply {
        answer: JobAnswer::Deleted,
        audits: Vec::new(),
    };
    let cases = [
        ("x".repeat(MAX_REPLY_BYTES + 1), code::TOO_LARGE, "more than"),
        (
            json!({ "status": "ok", "truncated": true, "calendar": {} }).to_string(),
            code::FAILED,
            "cut short",
        ),
        (
            json!({ "status": "error", "error": { "code": "made_up", "message": "boom" } })
                .to_string(),
            code::FAILED,
            "boom",
        ),
        (
            json!({ "status": "error", "error": { "code": "rate_limited", "message": "slow down" } })
                .to_string(),
            code::RATE_LIMITED,
            "slow down",
        ),
        ("not json".to_string(), code::FAILED, "unreadable"),
        (canned(&deleted), code::FAILED, "'deleted' to a get call"),
        (canned(&missing), code::NOT_FOUND, "no such event"),
    ];
    let here = FakeTransport::default();
    for (output, wanted, text) in cases {
        let source = delegated(vec![google()], std::sync::Arc::new(CannedJobs(output)));
        let client = Client::new(&source, &here);
        let calendars = provider::calendars(&source).await.unwrap();
        let error = client.get(&calendars[0], "g1").await.unwrap_err();
        assert_eq!(error.code, wanted, "{text}: {}", error.message);
        assert!(error.message.contains(text), "{}", error.message);
        if wanted == code::NOT_FOUND {
            assert_eq!(error.details, Some(json!({ "http_status": 404 })));
        }
    }
}

#[test]
fn a_job_cuts_its_listing_to_the_reply_limit() {
    let events: Vec<ExternalEvent> = (0..1_500)
        .map(|n| ExternalEvent {
            notes: Some("n".repeat(1_000)),
            ..job_event(&format!("e{n:04}"), "Slot")
        })
        .collect();
    let mut reply = JobReply {
        answer: JobAnswer::Listed {
            listings: vec![JobListing {
                connector_id: "c-google".into(),
                listed: Some(provider::Listed {
                    events,
                    complete: true,
                    unreadable: 0,
                }),
                error: None,
            }],
        },
        audits: Vec::new(),
    };
    jobs::fit(&mut reply, 200_000);
    assert!(serde_json::to_vec(&reply).unwrap().len() <= 200_000);
    let JobAnswer::Listed { listings } = &reply.answer else {
        panic!("{:?}", reply.answer);
    };
    let listed = listings[0].listed.as_ref().unwrap();
    assert!(!listed.complete);
    assert!(!listed.events.is_empty() && listed.events.len() < 200);
    assert_eq!(listed.events[0].event_id, "e0000");
}

#[tokio::test]
async fn a_calendar_job_runs_one_call_with_the_credential_it_carries() {
    let provider = FakeTransport::default().route(
        "GET",
        &format!("{GOOGLE_EVENTS}/g1"),
        200,
        google_event(
            "g1",
            "Review",
            "2030-01-07T09:00:00Z",
            "2030-01-07T10:00:00Z",
        ),
    );
    let env = FakeEnv::new().await;
    *env.connectors.lock().unwrap() = Some(std::sync::Arc::new(
        crate::services::StaticSource::new(vec![(google(), Credential::from_secret(TOKEN))]),
    ));
    let output = jobs::run_job(
        &env,
        &provider,
        CalendarCall::Get {
            event_id: "g1".into(),
        },
    )
    .await
    .unwrap();
    let reply: JobReply = serde_json::from_value(output.result["calendar"].clone()).unwrap();
    match &reply.answer {
        JobAnswer::Event { event } => assert_eq!(event.title, "Review"),
        other => panic!("{other:?}"),
    }
    assert_eq!(
        reply.audits,
        [JobAudit {
            connector_id: "c-google".into(),
            operation: "GET www.googleapis.com".into(),
            write: false,
            ok: true,
        }]
    );
    *env.connectors.lock().unwrap() = Some(std::sync::Arc::new(
        crate::services::StaticSource::new(vec![
            (google(), Credential::from_secret(TOKEN)),
            (microsoft(), Credential::from_secret(TOKEN)),
        ]),
    ));
    let two = jobs::run_job(
        &env,
        &provider,
        CalendarCall::Delete {
            event_id: "g1".into(),
        },
    )
    .await
    .unwrap();
    let reply: JobReply = serde_json::from_value(two.result["calendar"].clone()).unwrap();
    match &reply.answer {
        JobAnswer::Failed { error } => {
            assert_eq!(error.code, code::INVALID_ARGS);
            assert!(error.message.contains("exactly one"), "{}", error.message);
        }
        other => panic!("{other:?}"),
    }
    assert!(
        provider
            .methods()
            .iter()
            .all(|method| method.starts_with("GET"))
    );
    let bare = FakeEnv::new().await;
    let refused = crate::run(CALENDAR_JOB, json!({ "op": "discover" }), &bare).await;
    assert_eq!(refused.error.unwrap().code, code::INVALID_ARGS);
    assert!(crate::spec(CALENDAR_JOB).is_none());
}

/// A host whose CalDAV servers refuse every sign-in.
struct RefusingDav;

#[async_trait]
impl Transport for RefusingDav {
    async fn send(
        &self,
        _method: &str,
        _url: Url,
        _credential: &Credential,
        _body: Option<Value>,
    ) -> std::result::Result<RestResponse, String> {
        Err("no REST calls here".into())
    }

    fn dav(&self) -> &dyn caldav::DavTransport {
        self
    }
}

#[async_trait]
impl caldav::DavTransport for RefusingDav {
    async fn send(
        &self,
        _request: &caldav::DavRequest,
        credential: &Credential,
    ) -> std::result::Result<caldav::DavResponse, String> {
        assert!(credential.header().unwrap().starts_with("Basic "));
        Ok(caldav::DavResponse {
            status: 401,
            ..caldav::DavResponse::default()
        })
    }
}

#[tokio::test]
async fn a_discovery_job_reports_a_refused_sign_in_without_the_password() {
    let password = "s3cret-app-password";
    let jobs = InProcessJobs::new(RefusingDav);
    let error = discover_with(
        &jobs,
        "iCloud",
        "https://caldav.icloud.com/",
        caldav::basic_credential("me@icloud.com", password),
    )
    .await
    .unwrap_err();
    assert!(error.contains("401"), "{error}");
    assert!(!error.contains(password));
    let (call, sent) = jobs.calls.lock().unwrap()[0].clone();
    assert_eq!(call, json!({ "op": "discover" }));
    assert_eq!(sent[0].0, "caldav-discovery");

    let found = JobReply {
        answer: JobAnswer::Calendars {
            calendars: vec![
                flow_like_types::bot_connector::CalDavCalendarInfo {
                    url: "https://p52-caldav.icloud.com/1/calendars/home/".into(),
                    name: "h".repeat(500),
                    color: Some("#ff0000ff".repeat(10)),
                    writable: true,
                    zone: Some("Mars/Olympus".into()),
                },
                flow_like_types::bot_connector::CalDavCalendarInfo {
                    url: "javascript:alert(1)".into(),
                    name: "x".into(),
                    color: None,
                    writable: false,
                    zone: None,
                },
            ],
        },
        audits: Vec::new(),
    };
    let calendars = discover_with(
        &CannedJobs(canned(&found)),
        "iCloud",
        "https://caldav.icloud.com/",
        caldav::basic_credential("me@icloud.com", password),
    )
    .await
    .unwrap();
    assert_eq!(calendars.len(), 1);
    assert_eq!(calendars[0].name.chars().count(), 200);
    assert_eq!(calendars[0].color.as_deref().map(str::len), Some(32));
    assert_eq!(calendars[0].zone, None);
}

#[test]
fn a_calendar_call_travels_as_a_tagged_object() {
    let call = CalendarCall::Patch {
        event_id: "g1".into(),
        draft: provider::EventDraft {
            title: Some("Moved".into()),
            tz: Some("Europe/Berlin".parse().unwrap()),
            ..provider::EventDraft::default()
        },
    };
    let wire = serde_json::to_value(&call).unwrap();
    assert_eq!(wire["op"], "patch");
    assert_eq!(wire["draft"]["tz"], "Europe/Berlin");
    assert_eq!(serde_json::from_value::<CalendarCall>(wire).unwrap(), call);
    let unknown = json!({ "op": "patch", "event_id": "g1", "draft": { "tz": "Mars/Olympus" } });
    assert!(serde_json::from_value::<CalendarCall>(unknown).is_err());
}
