//! The connected calendars of a bot: the primary calendar of each Google and Microsoft OAuth
//! connector and the chosen calendar of each CalDAV connector, read and written through a
//! [`Transport`] with the host allowlist, call budget, audit and credential handling of
//! `connector_request`, or through the host's [`super::CalendarJobs`] where it runs them
//! elsewhere.

use async_trait::async_trait;
use chrono::{DateTime, Duration, NaiveDate, NaiveDateTime, SecondsFormat, TimeZone, Utc};
use chrono_tz::Tz;
use flow_like_types::bot_calendar::{
    CalendarEntry, CalendarEntrySource, CalendarSource, ExternalEventRef, MAX_EVENT_DAYS,
    MAX_EVENT_LINK_CHARS, MAX_EVENT_LOCATION_CHARS, TaskEvent, WakeSubject,
};
use flow_like_types::bot_connector::{CALDAV_PROVIDER, CallPolicy, ConnectorKind};
use flow_like_types::bot_task::MAX_TITLE_CHARS;
use flow_like_types::reqwest::Url;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use super::caldav::{
    DavAccount, DavCalendar, DavClient, DavRequest, DavResponse, DavTransport, HttpDavTransport,
    Scope,
};
use super::jobs::Delegate;
use super::{google, microsoft};
use crate::env::Environment;
use crate::error::{ToolError, ToolResult, code};
use crate::services::rest::{RestResponse, request_url, send};
use crate::services::{CallAudit, ConnectorEntry, ConnectorSource, Credential, take_call};

/// The calendar id an [`ExternalEventRef`] carries: v1 reads each account's main calendar.
pub const PRIMARY: &str = "primary";
/// The longest notes an entry carries.
pub const MAX_NOTES_CHARS: usize = 1_000;
/// Pages one listing follows before it stops and reports itself incomplete.
const MAX_PAGES: usize = 10;
const ERROR_TEXT_CHARS: usize = 300;
const UNTITLED: &str = "(no title)";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Provider {
    Google,
    Microsoft,
    Caldav,
}

impl Provider {
    pub fn parse(id: &str) -> Option<Self> {
        match id.trim().to_ascii_lowercase().as_str() {
            "google" => Some(Self::Google),
            "microsoft" => Some(Self::Microsoft),
            CALDAV_PROVIDER => Some(Self::Caldav),
            _ => None,
        }
    }

    pub fn id(self) -> &'static str {
        match self {
            Self::Google => "google",
            Self::Microsoft => "microsoft",
            Self::Caldav => CALDAV_PROVIDER,
        }
    }
}

/// The providers reached through a JSON REST API.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Rest {
    Google,
    Microsoft,
}

impl Rest {
    fn of(provider: Provider) -> Option<Self> {
        match provider {
            Provider::Google => Some(Self::Google),
            Provider::Microsoft => Some(Self::Microsoft),
            Provider::Caldav => None,
        }
    }
}

/// One connected calendar: a Google or Microsoft account connector, or a CalDAV connector with a
/// chosen calendar, whose owner lets the bot read.
#[derive(Debug, Clone)]
pub struct Calendar {
    pub entry: ConnectorEntry,
    pub provider: Provider,
}

impl Calendar {
    pub fn id(&self) -> &str {
        &self.entry.rule.id
    }

    pub fn label(&self) -> &str {
        &self.entry.rule.label
    }

    /// The provider's id of the calendar: [`PRIMARY`] for an account, the collection URL for
    /// CalDAV.
    pub fn calendar_id(&self) -> &str {
        match (self.provider, &self.entry.calendar) {
            (Provider::Caldav, Some(calendar)) => &calendar.url,
            _ => PRIMARY,
        }
    }

    pub fn source(&self, error: Option<String>) -> CalendarSource {
        CalendarSource {
            connector_id: self.id().to_string(),
            label: self.label().to_string(),
            provider: self.provider.id().to_string(),
            error,
        }
    }

    pub fn reference(&self, event_id: &str) -> ExternalEventRef {
        ExternalEventRef {
            connector_id: self.id().to_string(),
            provider: self.provider.id().to_string(),
            calendar_id: self.calendar_id().to_string(),
            event_id: event_id.to_string(),
        }
    }

    /// Whether the owner lets the bot use `method` on this account at all.
    pub fn allows(&self, method: &str) -> bool {
        self.entry.rule.request_call_policy(method) != CallPolicy::Never
    }
}

impl Calendar {
    /// The calendar `entry` connects: an OAuth account of Google or Microsoft, or a CalDAV
    /// connector with a chosen calendar, whose reads the owner did not switch off.
    fn of(entry: ConnectorEntry) -> Option<Self> {
        let provider = match entry.kind {
            ConnectorKind::Oauth => Provider::parse(entry.provider_id.as_deref()?)
                .filter(|provider| *provider != Provider::Caldav)?,
            ConnectorKind::Caldav if entry.calendar.is_some() => Provider::Caldav,
            _ => return None,
        };
        Some(Self { entry, provider }).filter(|calendar| calendar.allows("GET"))
    }
}

/// The provider id of the calendar `entry` connects, when [`calendars`] counts it as one.
pub fn calendar_provider(entry: ConnectorEntry) -> Option<&'static str> {
    Calendar::of(entry).map(|calendar| calendar.provider.id())
}

/// The calendars of the bot's connectors, as [`Calendar::of`] picks them.
pub async fn calendars(source: &dyn ConnectorSource) -> flow_like_types::Result<Vec<Calendar>> {
    Ok(source
        .entries()
        .await?
        .into_iter()
        .filter_map(Calendar::of)
        .collect())
}

/// The calendar a tool names by connector id or label.
pub fn find<'a>(calendars: &'a [Calendar], wanted: &str) -> ToolResult<&'a Calendar> {
    let wanted = wanted.trim();
    calendars
        .iter()
        .find(|calendar| calendar.id() == wanted)
        .or_else(|| {
            calendars
                .iter()
                .find(|calendar| calendar.label().eq_ignore_ascii_case(wanted))
        })
        .ok_or_else(|| {
            let known: Vec<&str> = calendars.iter().map(Calendar::label).collect();
            ToolError::not_found(format!(
                "No connected calendar '{wanted}'. Connected: {}. Omit `calendar` for your own calendar",
                if known.is_empty() {
                    "none".to_string()
                } else {
                    known.join(", ")
                }
            ))
        })
}

/// One event of a connected calendar, as both providers describe it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExternalEvent {
    pub event_id: String,
    pub title: String,
    pub starts_at: DateTime<Utc>,
    pub ends_at: Option<DateTime<Utc>>,
    pub all_day: bool,
    pub location: Option<String>,
    pub notes: Option<String>,
    pub link: Option<String>,
    pub recurring: bool,
    /// Blocks time: false for an event the provider marks free.
    pub busy: bool,
}

pub fn rfc3339(time: DateTime<Utc>) -> String {
    time.to_rfc3339_opts(SecondsFormat::Secs, true)
}

pub fn clamp(text: &str, limit: usize) -> String {
    text.trim().chars().take(limit).collect()
}

fn plain(text: Option<&str>, limit: usize) -> Option<String> {
    text.map(|text| clamp(text, limit))
        .filter(|text| !text.is_empty())
}

impl ExternalEvent {
    /// Builds an event from provider fields, trimmed to what an event task can hold.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        event_id: &str,
        title: Option<&str>,
        starts_at: DateTime<Utc>,
        ends_at: Option<DateTime<Utc>>,
        all_day: bool,
        location: Option<&str>,
        notes: Option<&str>,
        link: Option<&str>,
        recurring: bool,
        busy: bool,
    ) -> Self {
        let title = plain(title, MAX_TITLE_CHARS).unwrap_or_else(|| UNTITLED.to_string());
        let link = link
            .map(str::trim)
            .filter(|link| {
                (link.starts_with("https://") || link.starts_with("http://"))
                    && link.chars().count() <= MAX_EVENT_LINK_CHARS
            })
            .map(str::to_string);
        Self {
            event_id: event_id.to_string(),
            title,
            starts_at,
            ends_at: ends_at.filter(|end| *end >= starts_at),
            all_day,
            location: plain(location, MAX_EVENT_LOCATION_CHARS),
            notes: plain(notes, MAX_NOTES_CHARS),
            link,
            recurring,
            busy,
        }
    }

    pub fn subject(&self) -> WakeSubject<'_> {
        WakeSubject {
            title: &self.title,
            location: self.location.as_deref(),
            all_day: self.all_day,
            external: true,
        }
    }

    /// The listing entry of this event in `calendar`.
    pub fn entry(&self, calendar: &Calendar) -> CalendarEntry {
        CalendarEntry {
            id: calendar.reference(&self.event_id).encode(),
            source: CalendarEntrySource::External,
            title: self.title.clone(),
            starts_at: rfc3339(self.starts_at),
            ends_at: self.ends_at.map(rfc3339),
            all_day: self.all_day,
            location: self.location.clone(),
            notes: self.notes.clone(),
            connector_id: Some(calendar.id().to_string()),
            calendar: Some(calendar.label().to_string()),
            link: self.link.clone(),
            recurring: self.recurring,
            task_id: None,
            wake: None,
        }
    }

    /// The event of a wake-up row for this event: an end at most [`MAX_EVENT_DAYS`] after the
    /// start, the row's own wake-up kept.
    pub fn task_event(
        &self,
        reference: ExternalEventRef,
        wake: Option<flow_like_types::bot_calendar::EventWake>,
    ) -> TaskEvent {
        let longest = self.starts_at + Duration::days(MAX_EVENT_DAYS);
        TaskEvent {
            starts_at: rfc3339(self.starts_at),
            ends_at: self.ends_at.map(|end| rfc3339(end.min(longest))),
            all_day: self.all_day,
            location: self.location.clone(),
            wake,
            wake_offsets: Vec::new(),
            external: Some(reference),
            link: self.link.clone(),
        }
    }
}

/// What a write sends to a provider; a `None` field stays as it is.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct EventDraft {
    pub title: Option<String>,
    /// Set together with `ends_at` and `all_day` whenever the time changes.
    pub starts_at: Option<DateTime<Utc>>,
    pub ends_at: Option<DateTime<Utc>>,
    pub all_day: Option<bool>,
    pub location: Option<String>,
    pub notes: Option<String>,
    /// The zone the event's times are meant in: whole days are dates there.
    #[serde(default, with = "zone_name")]
    pub tz: Option<Tz>,
}

/// A zone as its IANA name.
mod zone_name {
    use chrono_tz::Tz;
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(zone: &Option<Tz>, serializer: S) -> Result<S::Ok, S::Error> {
        match zone {
            Some(zone) => serializer.serialize_some(zone.name()),
            None => serializer.serialize_none(),
        }
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Option<Tz>, D::Error> {
        Option::<String>::deserialize(deserializer)?
            .map(|name| {
                name.parse::<Tz>().map_err(|_| {
                    serde::de::Error::custom(format!("'{name}' is not an IANA time zone"))
                })
            })
            .transpose()
    }
}

/// Where the next page of a listing is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Next {
    /// Google: a `pageToken` for the same request.
    Token(String),
    /// Microsoft: the full URL of the next page.
    Url(String),
}

/// One page of a provider's event listing.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Page {
    pub events: Vec<ExternalEvent>,
    /// Events left out because their times could not be read.
    pub unreadable: usize,
    pub next: Option<Next>,
}

/// The events of one calendar over a span.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Listed {
    pub events: Vec<ExternalEvent>,
    /// Every page was read; false when the page cap stopped the listing.
    pub complete: bool,
    pub unreadable: usize,
}

impl Listed {
    /// A note for the source when the listing left events out.
    pub fn note(&self) -> Option<String> {
        let mut notes = Vec::new();
        if !self.complete {
            notes.push(format!(
                "only the first {} events were read; ask for a shorter span",
                self.events.len()
            ));
        }
        if self.unreadable > 0 {
            notes.push(format!(
                "{} events with times that could not be read were left out",
                self.unreadable
            ));
        }
        (!notes.is_empty()).then(|| notes.join("; "))
    }
}

/// Midnight of `date` in `tz`, or the first moment of that day where midnight does not exist.
pub fn local_midnight(date: NaiveDate, tz: Tz) -> Option<DateTime<Utc>> {
    (0..3).find_map(|hour| {
        tz.from_local_datetime(&date.and_hms_opt(hour, 0, 0)?)
            .earliest()
            .map(|time| time.with_timezone(&Utc))
    })
}

/// A local wall-clock time in `tz` as UTC; the earlier one when the clock repeats it.
pub fn local_time(time: NaiveDateTime, tz: Tz) -> Option<DateTime<Utc>> {
    tz.from_local_datetime(&time)
        .earliest()
        .map(|time| time.with_timezone(&Utc))
}

/// Characters of a URL path segment as they must travel.
pub fn segment(text: &str) -> String {
    text.bytes()
        .map(|byte| {
            if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
                (byte as char).to_string()
            } else {
                format!("%{byte:02X}")
            }
        })
        .collect()
}

/// How a calendar request leaves this host; a fake in tests.
#[async_trait]
pub trait Transport: Send + Sync {
    async fn send(
        &self,
        method: &str,
        url: Url,
        credential: &Credential,
        body: Option<Value>,
    ) -> Result<RestResponse, String>;

    /// Where CalDAV requests go; a transport without one reaches no CalDAV calendar.
    fn dav(&self) -> &dyn DavTransport {
        &NO_DAV
    }
}

struct NoDav;

static NO_DAV: NoDav = NoDav;

#[async_trait]
impl DavTransport for NoDav {
    async fn send(
        &self,
        request: &DavRequest,
        _credential: &Credential,
    ) -> Result<DavResponse, String> {
        Err(format!(
            "This host cannot reach CalDAV calendars ({})",
            request.url.host_str().unwrap_or_default()
        ))
    }
}

/// HTTPS with the bounds and egress checks of `connector_request`; CalDAV with the same egress.
pub struct HttpTransport(pub Environment);

#[async_trait]
impl Transport for HttpTransport {
    async fn send(
        &self,
        method: &str,
        url: Url,
        credential: &Credential,
        body: Option<Value>,
    ) -> Result<RestResponse, String> {
        send(self.0, method, url, credential, body).await
    }

    fn dav(&self) -> &dyn DavTransport {
        self
    }
}

#[async_trait]
impl DavTransport for HttpTransport {
    async fn send(
        &self,
        request: &DavRequest,
        credential: &Credential,
    ) -> Result<DavResponse, String> {
        HttpDavTransport(self.0).send(request, credential).await
    }
}

/// The CalDAV requests of one calendar, each audited like a REST call.
struct Audited<'a> {
    inner: &'a dyn DavTransport,
    source: &'a dyn ConnectorSource,
    calendar: &'a Calendar,
}

#[async_trait]
impl DavTransport for Audited<'_> {
    async fn send(
        &self,
        request: &DavRequest,
        credential: &Credential,
    ) -> Result<DavResponse, String> {
        let sent = self.inner.send(request, credential).await;
        self.source.audit(&CallAudit {
            connector_id: self.calendar.id().to_string(),
            label: self.calendar.label().to_string(),
            kind: ConnectorKind::Caldav,
            operation: format!(
                "{} {}",
                request.method.name(),
                request.url.host_str().unwrap_or_default()
            ),
            effect: if request.method.reads() {
                "read"
            } else {
                "write"
            },
            ok: sent
                .as_ref()
                .is_ok_and(|response| (200..300).contains(&response.status)),
        });
        sent
    }
}

/// What one CalDAV operation needs: the account, its chosen calendar and the sign-in.
struct DavCall<'a> {
    account: DavAccount,
    target: DavCalendar,
    credential: Credential,
    transport: Audited<'a>,
}

impl DavCall<'_> {
    fn client(&self) -> DavClient<'_> {
        DavClient::new(
            &self.account,
            &self.credential,
            &self.transport,
            chrono_tz::UTC,
        )
    }
}

fn dav_target(calendar: &Calendar) -> ToolResult<(DavAccount, DavCalendar)> {
    let label = calendar.label();
    let unusable = |why: String| {
        ToolError::new(
            code::FAILED,
            format!("'{label}' cannot be used: {why}; ask your owner to check it in its settings"),
        )
    };
    let chosen = calendar
        .entry
        .calendar
        .as_ref()
        .ok_or_else(|| unusable("no calendar is chosen".to_string()))?;
    let address = calendar.entry.url.as_deref().unwrap_or(&chosen.url);
    let account = DavAccount::new(label, address, &calendar.entry.api_hosts).map_err(unusable)?;
    let url = Url::parse(&chosen.url)
        .map_err(|error| unusable(format!("its calendar URL is invalid ({error})")))?;
    Ok((
        account,
        DavCalendar {
            url,
            name: chosen.name.clone(),
            color: chosen.color.clone(),
            ctag: None,
            sync_token: None,
            writable: chosen.writable,
            zone: chosen.zone.as_deref().and_then(|zone| zone.parse().ok()),
        },
    ))
}

fn provider_message(text: &str) -> String {
    let message = serde_json::from_str::<Value>(text)
        .ok()
        .and_then(|body| {
            body.pointer("/error/message")
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .unwrap_or_else(|| text.to_string());
    let line = message.lines().map(str::trim).find(|line| !line.is_empty());
    clamp(line.unwrap_or_default(), ERROR_TEXT_CHARS)
}

/// The sign-in of `calendar`, resolved now by the host.
pub(crate) async fn credential_for(
    source: &dyn ConnectorSource,
    calendar: &Calendar,
) -> ToolResult<Credential> {
    let credential = source.credential(calendar.id()).await.map_err(|error| {
        ToolError::new(
            code::FAILED,
            format!("Signing in to '{}' failed: {error}", calendar.label()),
        )
    })?;
    if credential.header().is_none() {
        return Err(ToolError::new(
            code::FAILED,
            format!(
                "'{}' has no sign-in; the owner must reconnect it",
                calendar.label()
            ),
        ));
    }
    Ok(credential)
}

/// Provider calls made from this process.
struct Direct<'a> {
    source: &'a dyn ConnectorSource,
    transport: &'a dyn Transport,
}

impl Direct<'_> {
    async fn request(
        &self,
        calendar: &Calendar,
        credential: &Credential,
        method: &str,
        url: &str,
        query: &Map<String, Value>,
        body: Option<Value>,
    ) -> ToolResult<Value> {
        let label = calendar.label();
        let url = request_url(&calendar.entry.api_hosts, url, query).map_err(|error| {
            ToolError::new(
                code::FAILED,
                format!("'{label}' cannot reach its calendar: {error}"),
            )
        })?;
        let host = url.host_str().unwrap_or_default().to_string();
        let sent = self.transport.send(method, url, credential, body).await;
        let ok = sent
            .as_ref()
            .is_ok_and(|response| (200..300).contains(&response.status));
        self.source.audit(&CallAudit {
            connector_id: calendar.id().to_string(),
            label: label.to_string(),
            kind: ConnectorKind::Oauth,
            operation: format!("{method} {host}"),
            effect: if method == "GET" { "read" } else { "write" },
            ok,
        });
        let response = sent.map_err(|mut message| {
            credential.scrub(&mut message);
            ToolError::new(
                code::FAILED,
                format!("The calendar request to '{label}' failed: {message}"),
            )
        })?;
        let mut text = response.text;
        credential.scrub(&mut text);
        let status = response.status;
        match status {
            200..=299 if text.trim().is_empty() => Ok(Value::Null),
            200..=299 => serde_json::from_str(&text).map_err(|error| {
                ToolError::new(
                    code::FAILED,
                    format!("'{label}' answered with something that is not JSON: {error}"),
                )
            }),
            401 | 403 => Err(ToolError::new(
                code::FAILED,
                format!(
                    "'{label}' refused the calendar request (HTTP {status}: {}); ask your owner to reconnect it with calendar permission",
                    provider_message(&text)
                ),
            )),
            404 | 410 => Err(ToolError::not_found(format!(
                "'{label}' has no such event (HTTP {status})"
            ))
            .with_details(serde_json::json!({ "http_status": status }))),
            429 => Err(ToolError::new(
                code::RATE_LIMITED,
                format!("'{label}' asks to slow down (HTTP 429); try again later"),
            )),
            _ => Err(ToolError::new(
                code::FAILED,
                format!(
                    "'{label}' answered HTTP {status}: {}",
                    provider_message(&text)
                ),
            )),
        }
    }

    async fn dav_call<'b>(&'b self, calendar: &'b Calendar) -> ToolResult<DavCall<'b>> {
        let credential = credential_for(self.source, calendar).await?;
        let (account, target) = dav_target(calendar)?;
        Ok(DavCall {
            account,
            target,
            credential,
            transport: Audited {
                inner: self.transport.dav(),
                source: self.source,
                calendar,
            },
        })
    }

    /// The events of `calendar` overlapping `[from, to)`, earliest first.
    async fn list(
        &self,
        calendar: &Calendar,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
    ) -> ToolResult<Listed> {
        let Some(rest) = Rest::of(calendar.provider) else {
            let call = self.dav_call(calendar).await?;
            return call.client().list(&call.target, from, to).await;
        };
        let credential = credential_for(self.source, calendar).await?;
        let (mut url, mut query) = match rest {
            Rest::Google => (google::events_url(PRIMARY), google::list_query(from, to)),
            Rest::Microsoft => (microsoft::view_url(), microsoft::list_query(from, to)),
        };
        let mut listed = Listed::default();
        for _ in 0..MAX_PAGES {
            let body = self
                .request(calendar, &credential, "GET", &url, &query, None)
                .await?;
            let page = match rest {
                Rest::Google => google::parse_page(&body),
                Rest::Microsoft => microsoft::parse_page(&body),
            };
            listed.events.extend(page.events);
            listed.unreadable += page.unreadable;
            match page.next {
                None => {
                    listed.complete = true;
                    break;
                }
                Some(Next::Token(token)) => {
                    query.insert("pageToken".into(), Value::String(token));
                }
                Some(Next::Url(next)) => {
                    url = next;
                    query = Map::new();
                }
            }
        }
        listed.events.sort_by_key(|event| event.starts_at);
        Ok(listed)
    }

    fn parse_one(calendar: &Calendar, rest: Rest, body: &Value) -> ToolResult<ExternalEvent> {
        let parsed = match rest {
            Rest::Google => google::parse_event(body, None),
            Rest::Microsoft => microsoft::parse_event(body),
        };
        match parsed {
            Ok(Some(event)) => Ok(event),
            Ok(None) => Err(ToolError::not_found(format!(
                "'{}' reports the event as cancelled",
                calendar.label()
            ))),
            Err(error) => Err(ToolError::new(
                code::FAILED,
                format!(
                    "'{}' answered with an event that could not be read: {error}",
                    calendar.label()
                ),
            )),
        }
    }

    fn event_url(rest: Rest, event_id: &str) -> String {
        match rest {
            Rest::Google => google::event_url(PRIMARY, event_id),
            Rest::Microsoft => microsoft::event_url(event_id),
        }
    }

    /// One event of `calendar` as it is now.
    async fn get(&self, calendar: &Calendar, event_id: &str) -> ToolResult<ExternalEvent> {
        let Some(rest) = Rest::of(calendar.provider) else {
            let call = self.dav_call(calendar).await?;
            return call.client().get(&call.target, event_id).await;
        };
        let credential = credential_for(self.source, calendar).await?;
        let url = Self::event_url(rest, event_id);
        let body = self
            .request(calendar, &credential, "GET", &url, &Map::new(), None)
            .await?;
        Self::parse_one(calendar, rest, &body)
    }

    fn body(rest: Rest, draft: &EventDraft) -> Value {
        match rest {
            Rest::Google => google::body(draft),
            Rest::Microsoft => microsoft::body(draft),
        }
    }

    /// Adds an event to `calendar`; `draft` carries a title and the times.
    async fn create(&self, calendar: &Calendar, draft: &EventDraft) -> ToolResult<ExternalEvent> {
        let Some(rest) = Rest::of(calendar.provider) else {
            let call = self.dav_call(calendar).await?;
            return call.client().create(&call.target, draft).await;
        };
        let credential = credential_for(self.source, calendar).await?;
        let url = match rest {
            Rest::Google => google::events_url(PRIMARY),
            Rest::Microsoft => microsoft::events_url(),
        };
        let body = self
            .request(
                calendar,
                &credential,
                "POST",
                &url,
                &Map::new(),
                Some(Self::body(rest, draft)),
            )
            .await?;
        Self::parse_one(calendar, rest, &body)
    }

    /// Changes what `draft` sets on one event of `calendar`. A CalDAV occurrence of a repeating
    /// event is refused: only whole CalDAV series change.
    async fn patch(
        &self,
        calendar: &Calendar,
        event_id: &str,
        draft: &EventDraft,
    ) -> ToolResult<ExternalEvent> {
        let Some(rest) = Rest::of(calendar.provider) else {
            let call = self.dav_call(calendar).await?;
            return call
                .client()
                .update(&call.target, event_id, draft, Scope::Event)
                .await;
        };
        let credential = credential_for(self.source, calendar).await?;
        let url = Self::event_url(rest, event_id);
        let body = self
            .request(
                calendar,
                &credential,
                "PATCH",
                &url,
                &Map::new(),
                Some(Self::body(rest, draft)),
            )
            .await?;
        Self::parse_one(calendar, rest, &body)
    }

    /// Deletes one event of `calendar`; one the provider reports as already deleted (HTTP 410)
    /// counts as deleted.
    async fn delete(&self, calendar: &Calendar, event_id: &str) -> ToolResult<()> {
        let Some(rest) = Rest::of(calendar.provider) else {
            let call = self.dav_call(calendar).await?;
            return call
                .client()
                .delete(&call.target, event_id, Scope::Event)
                .await;
        };
        let credential = credential_for(self.source, calendar).await?;
        let url = Self::event_url(rest, event_id);
        match self
            .request(calendar, &credential, "DELETE", &url, &Map::new(), None)
            .await
        {
            Ok(_) => Ok(()),
            Err(error)
                if error
                    .details
                    .as_ref()
                    .and_then(|details| details.get("http_status"))
                    .and_then(Value::as_u64)
                    == Some(410) =>
            {
                Ok(())
            }
            Err(error) => Err(error),
        }
    }
}

enum Route<'a> {
    Direct(Direct<'a>),
    Jobs(Delegate<'a>),
}

/// Calls to connected calendars for one turn or one host pass: made from this process through
/// the [`Transport`], or handed to the host's [`super::CalendarJobs`] when it answers
/// [`ConnectorSource::calendar_jobs`] (the server: its executor). Each listing and each write
/// takes one unit of the source's call budget.
pub struct Client<'a> {
    source: &'a dyn ConnectorSource,
    route: Route<'a>,
}

impl<'a> Client<'a> {
    pub fn new(source: &'a dyn ConnectorSource, transport: &'a dyn Transport) -> Self {
        let route = match source.calendar_jobs() {
            Some(jobs) => Route::Jobs(Delegate::new(source, jobs)),
            None => Route::Direct(Direct { source, transport }),
        };
        Self { source, route }
    }

    pub fn source(&self) -> &'a dyn ConnectorSource {
        self.source
    }

    /// Whether the calls leave through the host's [`super::CalendarJobs`].
    #[cfg(test)]
    pub fn delegates(&self) -> bool {
        matches!(self.route, Route::Jobs(_))
    }

    /// The events of `calendar` overlapping `[from, to)`, earliest first.
    #[cfg(test)]
    pub async fn list(
        &self,
        calendar: &Calendar,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
    ) -> ToolResult<Listed> {
        self.list_all(std::slice::from_ref(calendar), from, to)
            .await
            .pop()
            .unwrap_or_else(|| Ok(Listed::default()))
    }

    /// The events of each of `calendars` overlapping `[from, to)`, one result per calendar in
    /// their order; a host that runs the calls elsewhere reads them all in one job.
    pub async fn list_all(
        &self,
        calendars: &[Calendar],
        from: DateTime<Utc>,
        to: DateTime<Utc>,
    ) -> Vec<ToolResult<Listed>> {
        match &self.route {
            Route::Direct(direct) => {
                let mut results = Vec::with_capacity(calendars.len());
                for calendar in calendars {
                    results.push(match take_call(self.source) {
                        Ok(()) => direct.list(calendar, from, to).await,
                        Err(error) => Err(error),
                    });
                }
                results
            }
            Route::Jobs(jobs) => jobs.list(calendars, from, to).await,
        }
    }

    /// One event of `calendar` as it is now.
    pub async fn get(&self, calendar: &Calendar, event_id: &str) -> ToolResult<ExternalEvent> {
        take_call(self.source)?;
        match &self.route {
            Route::Direct(direct) => direct.get(calendar, event_id).await,
            Route::Jobs(jobs) => jobs.get(calendar, event_id).await,
        }
    }

    /// Adds an event to `calendar`; `draft` carries a title and the times.
    pub async fn create(
        &self,
        calendar: &Calendar,
        draft: &EventDraft,
    ) -> ToolResult<ExternalEvent> {
        take_call(self.source)?;
        match &self.route {
            Route::Direct(direct) => direct.create(calendar, draft).await,
            Route::Jobs(jobs) => jobs.create(calendar, draft).await,
        }
    }

    /// Changes what `draft` sets on one event of `calendar`. A CalDAV occurrence of a repeating
    /// event is refused: only whole CalDAV series change.
    pub async fn patch(
        &self,
        calendar: &Calendar,
        event_id: &str,
        draft: &EventDraft,
    ) -> ToolResult<ExternalEvent> {
        take_call(self.source)?;
        match &self.route {
            Route::Direct(direct) => direct.patch(calendar, event_id, draft).await,
            Route::Jobs(jobs) => jobs.patch(calendar, event_id, draft).await,
        }
    }

    /// Deletes one event of `calendar`; one the provider reports as already deleted (HTTP 410)
    /// counts as deleted.
    pub async fn delete(&self, calendar: &Calendar, event_id: &str) -> ToolResult<()> {
        take_call(self.source)?;
        match &self.route {
            Route::Direct(direct) => direct.delete(calendar, event_id).await,
            Route::Jobs(jobs) => jobs.delete(calendar, event_id).await,
        }
    }
}
