//! Calendar provider calls a host runs in another process. The server keeps fetching and parsing
//! third-party calendars out of its API process: its [`ConnectorSource`] answers
//! [`ConnectorSource::calendar_jobs`], and each call to connected calendars becomes one internal
//! [`CALENDAR_JOB`] tool job on its executor ([`calendar_job`]). The job calls the providers with
//! the credentials the API resolved for that one call, parses their answers and returns typed,
//! bounded results. The API side ([`Delegate`]) takes the call budget, resolves the credentials,
//! checks every answer again and records the job's calls in its own audit; it never sees a
//! provider's bytes.

use std::sync::Mutex;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use chrono_tz::Tz;
use flow_like_types::Result;
use flow_like_types::bot_calendar::MAX_EXTERNAL_ID_CHARS;
use flow_like_types::bot_connector::{
    CALDAV_PROVIDER, CalDavCalendarInfo, ConnectorKind, ConnectorRule, MAX_CALDAV_CALENDARS,
    MAX_URL_CHARS,
};
use flow_like_types::reqwest::Url;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::HOST_ENVIRONMENT;
use super::caldav::{account_hosts, discover};
use super::provider::{
    Calendar, Client, EventDraft, ExternalEvent, HttpTransport, Listed, Transport, calendars,
    clamp, credential_for,
};
use crate::env::ToolEnv;
use crate::error::{ToolError, ToolResult, code};
use crate::outcome::ToolOutput;
use crate::services::{
    CallAudit, CallBudget, ConnectorEntry, ConnectorSource, Credential, take_call,
};

/// The internal tool a calendar job runs on the executor. It is never offered to a model.
pub const CALENDAR_JOB: &str = "calendar_job";
/// The largest output a calendar job returns; its listings are cut to fit.
pub const MAX_REPLY_BYTES: usize = 4 * 1024 * 1024;
/// The most events one calendar's listing carries back.
pub const MAX_JOB_EVENTS: usize = 1_000;
const REPLY_FIELD: &str = "calendar";
const ENVELOPE_BYTES: usize = 64 * 1024;
const MAX_AUDITS: usize = 64;
const MAX_ERROR_CHARS: usize = 1_000;
const MAX_OPERATION_CHARS: usize = 300;
const MAX_CALENDAR_NAME_CHARS: usize = 200;
const MAX_COLOR_CHARS: usize = 32;
const DISCOVERY_ID: &str = "caldav-discovery";
const KNOWN_CODES: [&str; 9] = [
    code::INVALID_ARGS,
    code::SCOPE_REFUSED,
    code::NOT_FOUND,
    code::TOO_LARGE,
    code::NOT_YET,
    code::RATE_LIMITED,
    code::NOT_SUPPORTED,
    code::CANCELLED,
    code::FAILED,
];

/// One provider call of a calendar job, against the connectors the job carries.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum CalendarCall {
    /// The events of every calendar of the job overlapping `[from, to)`.
    List {
        from: DateTime<Utc>,
        to: DateTime<Utc>,
    },
    Get {
        event_id: String,
    },
    Create {
        draft: EventDraft,
    },
    Patch {
        event_id: String,
        draft: EventDraft,
    },
    Delete {
        event_id: String,
    },
    /// The calendars behind the job's one CalDAV account.
    Discover,
}

impl CalendarCall {
    pub fn name(&self) -> &'static str {
        match self {
            Self::List { .. } => "list",
            Self::Get { .. } => "get",
            Self::Create { .. } => "create",
            Self::Patch { .. } => "patch",
            Self::Delete { .. } => "delete",
            Self::Discover => "discover",
        }
    }
}

/// One connector of a calendar job and the sign-in the host resolved for this one call.
#[derive(Debug, Clone)]
pub struct JobCalendar {
    pub entry: ConnectorEntry,
    pub credential: Credential,
}

/// Runs calendar calls in another process. A host answers it from
/// [`ConnectorSource::calendar_jobs`].
#[async_trait]
pub trait CalendarJobs: Send + Sync {
    /// Refuses up front, before any credential is resolved, when no call can run.
    fn ready(&self) -> ToolResult<()> {
        Ok(())
    }

    /// Runs `call` against `connectors` in the other process and returns the job's output: the
    /// outcome envelope of [`CALENDAR_JOB`]. Errors never carry a credential.
    async fn run(&self, call: &CalendarCall, connectors: Vec<JobCalendar>) -> ToolResult<String>;
}

/// Why a provider call of a job failed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JobError {
    pub code: String,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub http_status: Option<u16>,
}

fn known_code(text: &str) -> &'static str {
    KNOWN_CODES
        .into_iter()
        .find(|known| *known == text)
        .unwrap_or(code::FAILED)
}

impl JobError {
    fn of(error: &ToolError) -> Self {
        Self {
            code: error.code.to_string(),
            message: clamp(&error.message, MAX_ERROR_CHARS),
            http_status: error
                .details
                .as_ref()
                .and_then(|details| details.get("http_status"))
                .and_then(Value::as_u64)
                .and_then(|status| u16::try_from(status).ok()),
        }
    }

    fn tool_error(&self) -> ToolError {
        let error = ToolError::new(
            known_code(&self.code),
            clamp(&self.message, MAX_ERROR_CHARS),
        );
        match self.http_status {
            Some(status) => error.with_details(json!({ "http_status": status })),
            None => error,
        }
    }
}

/// One calendar's part of a listing job.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JobListing {
    pub connector_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub listed: Option<Listed>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<JobError>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum JobAnswer {
    Listed { listings: Vec<JobListing> },
    Event { event: ExternalEvent },
    Deleted,
    Calendars { calendars: Vec<CalDavCalendarInfo> },
    Failed { error: JobError },
}

impl JobAnswer {
    fn kind(&self) -> &'static str {
        match self {
            Self::Listed { .. } => "listed",
            Self::Event { .. } => "event",
            Self::Deleted => "deleted",
            Self::Calendars { .. } => "calendars",
            Self::Failed { .. } => "failed",
        }
    }
}

/// One provider request a job made, for the host's audit: no arguments, no results.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JobAudit {
    pub connector_id: String,
    /// `METHOD host`.
    pub operation: String,
    pub write: bool,
    pub ok: bool,
}

/// What a calendar job returns under `calendar` in its outcome envelope.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JobReply {
    pub answer: JobAnswer,
    #[serde(default)]
    pub audits: Vec<JobAudit>,
}

/// The connector a CalDAV discovery runs against, before the connector itself exists.
pub fn account_entry(label: &str, url: &str) -> ConnectorEntry {
    ConnectorEntry {
        rule: ConnectorRule {
            id: DISCOVERY_ID.to_string(),
            label: label.to_string(),
            kind: Some(ConnectorKind::Caldav),
            ..ConnectorRule::default()
        },
        kind: ConnectorKind::Caldav,
        url: Some(url.to_string()),
        provider_id: Some(CALDAV_PROVIDER.to_string()),
        api_hosts: account_hosts(url),
        calendar: None,
    }
}

/// The job's connectors with each request they make kept for the reply. It never hands calls
/// on: the job calls the providers itself.
struct Recorder<'a> {
    inner: &'a dyn ConnectorSource,
    audits: Mutex<Vec<JobAudit>>,
}

impl<'a> Recorder<'a> {
    fn new(inner: &'a dyn ConnectorSource) -> Self {
        Self {
            inner,
            audits: Mutex::new(Vec::new()),
        }
    }

    fn take(&self) -> Vec<JobAudit> {
        std::mem::take(
            &mut *self
                .audits
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()),
        )
    }
}

#[async_trait]
impl ConnectorSource for Recorder<'_> {
    async fn entries(&self) -> Result<Vec<ConnectorEntry>> {
        self.inner.entries().await
    }

    async fn credential(&self, connector_id: &str) -> Result<Credential> {
        self.inner.credential(connector_id).await
    }

    fn budget(&self) -> &CallBudget {
        self.inner.budget()
    }

    fn audit(&self, entry: &CallAudit) {
        let mut audits = self
            .audits
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if audits.len() < MAX_AUDITS {
            audits.push(JobAudit {
                connector_id: entry.connector_id.clone(),
                operation: clamp(&entry.operation, MAX_OPERATION_CHARS),
                write: entry.effect != "read",
                ok: entry.ok,
            });
        }
    }
}

/// Runs one calendar job where it was sent (the executor): the provider calls with the server's
/// egress rules (public https hosts only, addresses pinned), whatever the environment says.
pub async fn calendar_job(env: &dyn ToolEnv, call: CalendarCall) -> ToolResult<ToolOutput> {
    run_job(env, &HttpTransport(HOST_ENVIRONMENT), call).await
}

pub(crate) async fn run_job(
    env: &dyn ToolEnv,
    transport: &dyn Transport,
    call: CalendarCall,
) -> ToolResult<ToolOutput> {
    let source = env
        .connectors()
        .ok_or_else(|| ToolError::invalid("A calendar job carries the connectors it calls"))?;
    let recorder = Recorder::new(source.as_ref());
    let answer = answer(&recorder, transport, call)
        .await
        .unwrap_or_else(|error| JobAnswer::Failed {
            error: JobError::of(&error),
        });
    let mut reply = JobReply {
        answer,
        audits: recorder.take(),
    };
    fit(&mut reply, MAX_REPLY_BYTES - ENVELOPE_BYTES);
    Ok(ToolOutput::new(json!({ REPLY_FIELD: reply })))
}

async fn answer(
    source: &Recorder<'_>,
    transport: &dyn Transport,
    call: CalendarCall,
) -> ToolResult<JobAnswer> {
    let calendars = calendars(source).await?;
    let client = Client::new(source, transport);
    let only = || match calendars.as_slice() {
        [calendar] => Ok(calendar),
        other => Err(ToolError::invalid(format!(
            "A calendar job's {} call needs exactly one readable calendar; it carries {}",
            call.name(),
            other.len()
        ))),
    };
    let event = |event: ExternalEvent| JobAnswer::Event { event };
    match &call {
        CalendarCall::List { from, to } => {
            let results = client.list_all(&calendars, *from, *to).await;
            Ok(JobAnswer::Listed {
                listings: calendars
                    .iter()
                    .zip(results)
                    .map(|(calendar, result)| {
                        let (listed, error) = match result {
                            Ok(listed) => (Some(listed), None),
                            Err(error) => (None, Some(JobError::of(&error))),
                        };
                        JobListing {
                            connector_id: calendar.id().to_string(),
                            listed,
                            error,
                        }
                    })
                    .collect(),
            })
        }
        CalendarCall::Get { event_id } => client.get(only()?, event_id).await.map(event),
        CalendarCall::Create { draft } => client.create(only()?, draft).await.map(event),
        CalendarCall::Patch { event_id, draft } => {
            client.patch(only()?, event_id, draft).await.map(event)
        }
        CalendarCall::Delete { event_id } => client
            .delete(only()?, event_id)
            .await
            .map(|()| JobAnswer::Deleted),
        CalendarCall::Discover => discovered(source, transport).await,
    }
}

async fn discovered(source: &Recorder<'_>, transport: &dyn Transport) -> ToolResult<JobAnswer> {
    let entries = source.entries().await?;
    let [entry] = entries.as_slice() else {
        return Err(ToolError::invalid(format!(
            "A discovery job carries exactly one CalDAV account; it carries {}",
            entries.len()
        )));
    };
    let url = entry
        .url
        .as_deref()
        .filter(|_| entry.kind == ConnectorKind::Caldav)
        .ok_or_else(|| ToolError::invalid("A discovery job carries a CalDAV account's address"))?;
    let credential = source.credential(&entry.rule.id).await?;
    let calendars = discover(&entry.rule.label, url, &credential, transport.dav())
        .await
        .map_err(|message| ToolError::new(code::FAILED, message))?;
    Ok(JobAnswer::Calendars { calendars })
}

fn json_bytes(value: &impl Serialize) -> usize {
    serde_json::to_vec(value).map_or(0, |bytes| bytes.len())
}

/// An event as a listing may carry it: its id within the reference limit, every text clamped as
/// the providers' parsers clamp it.
fn bounded(event: ExternalEvent) -> Option<ExternalEvent> {
    let id = event.event_id.as_str();
    if id.trim().is_empty() || id.chars().count() > MAX_EXTERNAL_ID_CHARS {
        return None;
    }
    Some(ExternalEvent::new(
        id,
        Some(&event.title),
        event.starts_at,
        event.ends_at,
        event.all_day,
        event.location.as_deref(),
        event.notes.as_deref(),
        event.link.as_deref(),
        event.recurring,
        event.busy,
    ))
}

/// The listing as the host keeps it: at most [`MAX_JOB_EVENTS`] bounded events, earliest
/// first; events that are not bounded count as unreadable.
fn bounded_listing(listed: Listed) -> Listed {
    let mut kept = Listed {
        events: Vec::new(),
        complete: listed.complete && listed.events.len() <= MAX_JOB_EVENTS,
        unreadable: listed.unreadable,
    };
    for event in listed.events.into_iter().take(MAX_JOB_EVENTS) {
        match bounded(event) {
            Some(event) => kept.events.push(event),
            None => kept.unreadable = kept.unreadable.saturating_add(1),
        }
    }
    kept.events.sort_by_key(|event| event.starts_at);
    kept
}

/// Cuts the reply to `budget` bytes: listings lose their latest events first and say they are
/// incomplete.
pub(crate) fn fit(reply: &mut JobReply, budget: usize) {
    if matches!(&reply.answer, JobAnswer::Event { event } if bounded(event.clone()).is_none()) {
        reply.answer = JobAnswer::Failed {
            error: JobError::of(&ToolError::new(
                code::FAILED,
                "The provider answered with an event id that is too long",
            )),
        };
    }
    if let JobAnswer::Listed { listings } = &mut reply.answer {
        for listed in listings.iter_mut().filter_map(|part| part.listed.as_mut()) {
            *listed = bounded_listing(std::mem::take(listed));
        }
    }
    let mut total = json_bytes(reply);
    let JobAnswer::Listed { listings } = &mut reply.answer else {
        return;
    };
    while total > budget {
        let Some(listed) = listings
            .iter_mut()
            .filter_map(|part| part.listed.as_mut())
            .filter(|listed| !listed.events.is_empty())
            .max_by_key(|listed| listed.events.len())
        else {
            break;
        };
        if let Some(event) = listed.events.pop() {
            total = total.saturating_sub(json_bytes(&event) + 1);
        }
        listed.complete = false;
    }
}

fn unusable(message: impl Into<String>) -> ToolError {
    ToolError::new(code::FAILED, message)
}

/// The typed reply inside a calendar job's output, refused when it is too large, failed, cut
/// short or not a reply at all.
fn reply_of(output: &str) -> ToolResult<JobReply> {
    if output.len() > MAX_REPLY_BYTES {
        return Err(ToolError::new(
            code::TOO_LARGE,
            format!(
                "The calendar job answered {} bytes, more than the {MAX_REPLY_BYTES} a calendar call may return",
                output.len()
            ),
        ));
    }
    let mut envelope: Value = serde_json::from_str(output).map_err(|error| {
        unusable(format!(
            "The calendar job answered something unreadable: {error}"
        ))
    })?;
    if envelope["status"] != "ok" {
        let error = &envelope["error"];
        return Err(ToolError::new(
            known_code(error["code"].as_str().unwrap_or_default()),
            clamp(
                error["message"]
                    .as_str()
                    .unwrap_or("The calendar job failed"),
                MAX_ERROR_CHARS,
            ),
        ));
    }
    if envelope["truncated"] == true {
        return Err(unusable("The calendar job's answer was cut short"));
    }
    serde_json::from_value(envelope[REPLY_FIELD].take()).map_err(|error| {
        unusable(format!(
            "The calendar job answered without a calendar reply: {error}"
        ))
    })
}

fn unexpected(answer: &JobAnswer, call: &str) -> ToolError {
    unusable(format!(
        "The calendar job answered '{}' to a {call} call",
        answer.kind()
    ))
}

/// The calendars a discovery found, as a connector may keep them.
fn bounded_calendars(calendars: Vec<CalDavCalendarInfo>) -> Vec<CalDavCalendarInfo> {
    calendars
        .into_iter()
        .filter(|calendar| {
            calendar.url.chars().count() <= MAX_URL_CHARS
                && Url::parse(&calendar.url)
                    .is_ok_and(|url| matches!(url.scheme(), "https" | "http"))
        })
        .take(MAX_CALDAV_CALENDARS)
        .map(|calendar| CalDavCalendarInfo {
            name: clamp(&calendar.name, MAX_CALENDAR_NAME_CHARS),
            color: calendar
                .color
                .map(|color| clamp(&color, MAX_COLOR_CHARS))
                .filter(|color| !color.is_empty()),
            zone: calendar.zone.filter(|zone| zone.parse::<Tz>().is_ok()),
            url: calendar.url,
            writable: calendar.writable,
        })
        .collect()
}

/// The calendars behind a CalDAV account, found by a job in the other process: what
/// [`super::caldav::discover`] answers on a host that reaches the account itself. The message
/// of an error is for the owner and never carries the password.
pub async fn discover_with(
    jobs: &dyn CalendarJobs,
    label: &str,
    url: &str,
    credential: Credential,
) -> std::result::Result<Vec<CalDavCalendarInfo>, String> {
    jobs.ready().map_err(|error| error.message)?;
    let connector = JobCalendar {
        entry: account_entry(label, url),
        credential,
    };
    let output = jobs
        .run(&CalendarCall::Discover, vec![connector])
        .await
        .map_err(|error| error.message)?;
    match reply_of(&output).map_err(|error| error.message)?.answer {
        JobAnswer::Calendars { calendars } => Ok(bounded_calendars(calendars)),
        JobAnswer::Failed { error } => Err(clamp(&error.message, MAX_ERROR_CHARS)),
        other => Err(unexpected(&other, "discover").message),
    }
}

/// The API side of calendar jobs: the call budget, the credentials and the answer checks stay in
/// this process; the provider calls run as jobs.
pub(crate) struct Delegate<'a> {
    source: &'a dyn ConnectorSource,
    jobs: &'a dyn CalendarJobs,
}

impl<'a> Delegate<'a> {
    pub(crate) fn new(source: &'a dyn ConnectorSource, jobs: &'a dyn CalendarJobs) -> Self {
        Self { source, jobs }
    }

    async fn connector(&self, calendar: &Calendar) -> ToolResult<JobCalendar> {
        Ok(JobCalendar {
            entry: calendar.entry.clone(),
            credential: credential_for(self.source, calendar).await?,
        })
    }

    /// Records the job's requests in this host's audit; requests for connectors the job was not
    /// sent are dropped.
    fn record(&self, audits: &[JobAudit], calendars: &[&Calendar]) {
        for audit in audits.iter().take(MAX_AUDITS) {
            let Some(calendar) = calendars
                .iter()
                .find(|calendar| calendar.id() == audit.connector_id)
            else {
                continue;
            };
            self.source.audit(&CallAudit {
                connector_id: calendar.id().to_string(),
                label: calendar.label().to_string(),
                kind: calendar.entry.kind,
                operation: clamp(&audit.operation, MAX_OPERATION_CHARS),
                effect: if audit.write { "write" } else { "read" },
                ok: audit.ok,
            });
        }
    }

    async fn run(
        &self,
        call: &CalendarCall,
        calendars: &[&Calendar],
        connectors: Vec<JobCalendar>,
    ) -> ToolResult<JobAnswer> {
        let output = self.jobs.run(call, connectors).await?;
        let reply = reply_of(&output)?;
        self.record(&reply.audits, calendars);
        Ok(reply.answer)
    }

    pub(crate) async fn list(
        &self,
        calendars: &[Calendar],
        from: DateTime<Utc>,
        to: DateTime<Utc>,
    ) -> Vec<ToolResult<Listed>> {
        if let Err(error) = self.jobs.ready() {
            return calendars.iter().map(|_| Err(error.clone())).collect();
        }
        let mut results: Vec<Option<ToolResult<Listed>>> = vec![None; calendars.len()];
        let mut sent: Vec<(usize, &Calendar)> = Vec::new();
        let mut connectors = Vec::new();
        for (index, calendar) in calendars.iter().enumerate() {
            let connector = match take_call(self.source) {
                Ok(()) => self.connector(calendar).await,
                Err(error) => Err(error),
            };
            match connector {
                Ok(connector) => {
                    sent.push((index, calendar));
                    connectors.push(connector);
                }
                Err(error) => results[index] = Some(Err(error)),
            }
        }
        if !sent.is_empty() {
            let refs: Vec<&Calendar> = sent.iter().map(|(_, calendar)| *calendar).collect();
            let call = CalendarCall::List { from, to };
            let mut listings = match self.run(&call, &refs, connectors).await {
                Ok(JobAnswer::Listed { listings }) => Ok(listings),
                Ok(JobAnswer::Failed { error }) => Err(error.tool_error()),
                Ok(other) => Err(unexpected(&other, call.name())),
                Err(error) => Err(error),
            };
            for (index, calendar) in sent {
                results[index] = Some(match &mut listings {
                    Ok(listings) => {
                        let part = listings
                            .iter()
                            .position(|part| part.connector_id == calendar.id())
                            .map(|position| listings.swap_remove(position));
                        match part {
                            Some(JobListing {
                                listed: Some(listed),
                                ..
                            }) => Ok(bounded_listing(listed)),
                            Some(JobListing {
                                error: Some(error), ..
                            }) => Err(error.tool_error()),
                            _ => Err(unusable(format!(
                                "The calendar job did not read '{}'",
                                calendar.label()
                            ))),
                        }
                    }
                    Err(error) => Err(error.clone()),
                });
            }
        }
        results
            .into_iter()
            .map(|result| result.unwrap_or_else(|| Err(unusable("The calendar was not read"))))
            .collect()
    }

    async fn one(&self, calendar: &Calendar, call: CalendarCall) -> ToolResult<JobAnswer> {
        self.jobs.ready()?;
        let connector = self.connector(calendar).await?;
        match self.run(&call, &[calendar], vec![connector]).await? {
            JobAnswer::Failed { error } => Err(error.tool_error()),
            answer => Ok(answer),
        }
    }

    async fn event(&self, calendar: &Calendar, call: CalendarCall) -> ToolResult<ExternalEvent> {
        let name = call.name();
        match self.one(calendar, call).await? {
            JobAnswer::Event { event } => bounded(event).ok_or_else(|| {
                unusable(format!(
                    "'{}' answered with an event that could not be kept",
                    calendar.label()
                ))
            }),
            other => Err(unexpected(&other, name)),
        }
    }

    pub(crate) async fn get(
        &self,
        calendar: &Calendar,
        event_id: &str,
    ) -> ToolResult<ExternalEvent> {
        let event_id = event_id.to_string();
        self.event(calendar, CalendarCall::Get { event_id }).await
    }

    pub(crate) async fn create(
        &self,
        calendar: &Calendar,
        draft: &EventDraft,
    ) -> ToolResult<ExternalEvent> {
        let draft = draft.clone();
        self.event(calendar, CalendarCall::Create { draft }).await
    }

    pub(crate) async fn patch(
        &self,
        calendar: &Calendar,
        event_id: &str,
        draft: &EventDraft,
    ) -> ToolResult<ExternalEvent> {
        let (event_id, draft) = (event_id.to_string(), draft.clone());
        self.event(calendar, CalendarCall::Patch { event_id, draft })
            .await
    }

    pub(crate) async fn delete(&self, calendar: &Calendar, event_id: &str) -> ToolResult<()> {
        let call = CalendarCall::Delete {
            event_id: event_id.to_string(),
        };
        match self.one(calendar, call).await? {
            JobAnswer::Deleted => Ok(()),
            other => Err(unexpected(&other, "delete")),
        }
    }
}
