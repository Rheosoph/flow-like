//! Calls to one CalDAV account. Every URL is checked against the account's hosts before the
//! sign-in goes out, answers become tool errors that name the operation, the host and the
//! status, and writes carry ETag preconditions with one retry after a 412.

use chrono::{DateTime, Utc};
use chrono_tz::Tz;
use flow_like_types::base64::Engine;
use flow_like_types::base64::engine::general_purpose::STANDARD;
use flow_like_types::reqwest::Url;
use serde_json::{Value, json};

use super::events::{self, EventId};
use super::ical::{self, Component};
use super::transport::{
    DavBody, DavMethod, DavRequest, DavResponse, DavTransport, Depth, Precondition,
};
use super::xml::{self, CALDAV};
use crate::calendar::provider::{EventDraft, ExternalEvent, Listed, clamp};
use crate::error::{ToolError, ToolResult, code};
use crate::services::{Credential, host_allowed};

/// Events one listing returns before it reports itself incomplete.
pub const MAX_LISTED: usize = 1_000;
const DETAIL_CHARS: usize = 200;
/// Statuses after which a listing asks again without `expand` and expands here.
const EXPAND_REFUSED: [u16; 6] = [400, 403, 415, 422, 500, 501];
const ICLOUD: &str = "icloud.com";

pub(super) const DISCOVER: &str = "find the calendars";
const LIST: &str = "list events";
const READ: &str = "read the event";
const CREATE: &str = "create the event";
const CHANGE: &str = "change the event";
const DELETE: &str = "delete the event";

/// The `Authorization` value of a user name and password (an app-specific password for iCloud).
pub fn basic_credential(user: &str, password: &str) -> Credential {
    Credential::authorization(format!(
        "Basic {}",
        STANDARD.encode(format!("{user}:{password}"))
    ))
}

/// A CalDAV account: where discovery starts and the only hosts its sign-in goes to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DavAccount {
    pub label: String,
    pub url: Url,
    /// Exact host names or `*.suffix`.
    pub hosts: Vec<String>,
}

impl DavAccount {
    /// An account at `url`: a server root, `/.well-known/caldav`, a principal, a calendar home or
    /// one calendar. The sign-in goes to the URL's host, to `extra_hosts`, and for iCloud to its
    /// partition hosts (`p52-caldav.icloud.com`).
    pub fn new(label: &str, url: &str, extra_hosts: &[String]) -> Result<Self, String> {
        let mut parsed = Url::parse(url.trim())
            .map_err(|error| format!("The calendar address is not a URL: {error}"))?;
        if !matches!(parsed.scheme(), "http" | "https") {
            return Err(
                "The calendar address must start with https:// (or http:// on your own network)"
                    .into(),
            );
        }
        if !parsed.username().is_empty() || parsed.password().is_some() {
            return Err(
                "Put the user name and password in their own fields, not in the address".into(),
            );
        }
        let host = parsed
            .host_str()
            .ok_or("The calendar address has no host")?
            .trim_end_matches('.')
            .to_ascii_lowercase();
        parsed.set_fragment(None);
        let mut hosts = vec![host.clone()];
        if host == ICLOUD || host.ends_with(&format!(".{ICLOUD}")) {
            hosts.push(format!("*.{ICLOUD}"));
        }
        hosts.extend(
            extra_hosts
                .iter()
                .map(|extra| extra.trim().to_ascii_lowercase()),
        );
        hosts.dedup();
        Ok(Self {
            label: label.trim().to_string(),
            url: parsed,
            hosts,
        })
    }

    /// Whether the sign-in may go to `url`: one of the account's hosts, never from https down
    /// to http, never with credentials in the URL.
    pub fn permits(&self, url: &Url) -> Result<(), String> {
        let host = url.host_str().unwrap_or_default();
        if !matches!(url.scheme(), "http" | "https")
            || (self.url.scheme() == "https" && url.scheme() != "https")
        {
            return Err(format!(
                "'{host}' is not reached over {}",
                self.url.scheme()
            ));
        }
        if !url.username().is_empty() || url.password().is_some() {
            return Err(format!("the address for '{host}' carries credentials"));
        }
        if !host_allowed(host, &self.hosts) {
            return Err(format!(
                "'{host}' is not a host of this calendar account (allowed: {})",
                self.hosts.join(", ")
            ));
        }
        Ok(())
    }
}

/// One calendar collection of an account.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DavCalendar {
    /// The collection URL, ending in `/`.
    pub url: Url,
    pub name: String,
    pub color: Option<String>,
    /// `getctag`: changes whenever an event changes.
    pub ctag: Option<String>,
    pub sync_token: Option<String>,
    /// False when the server says the account may only read it.
    pub writable: bool,
    /// The calendar's own zone (`calendar-timezone`), for whole days and floating times.
    pub zone: Option<Tz>,
}

/// What a change or a deletion covers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    /// One event; refused for an occurrence or a whole repeating series.
    Event,
    /// The whole series the event belongs to (its title, place and notes, or all of it).
    Series,
}

pub(super) fn collection(mut url: Url) -> Url {
    if !url.path().ends_with('/') {
        let path = format!("{}/", url.path());
        url.set_path(&path);
    }
    url.set_query(None);
    url.set_fragment(None);
    url
}

pub(super) fn host_of(url: &Url) -> String {
    match url.port() {
        Some(port) => format!("{}:{port}", url.host_str().unwrap_or_default()),
        None => url.host_str().unwrap_or_default().to_string(),
    }
}

/// A percent-encoded path as text, for comparing hrefs that servers encode differently.
pub(super) fn decoded(path: &str) -> String {
    let bytes = path.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        let hex = |at: usize| bytes.get(at).and_then(|byte| (*byte as char).to_digit(16));
        match (bytes[index], hex(index + 1), hex(index + 2)) {
            (b'%', Some(high), Some(low)) => {
                out.push((high * 16 + low) as u8);
                index += 3;
            }
            (byte, _, _) => {
                out.push(byte);
                index += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// The HTTP status a tool error carries in its details.
pub fn http_status(error: &ToolError) -> Option<u64> {
    error
        .details
        .as_ref()
        .and_then(|details| details.get("http_status"))
        .and_then(Value::as_u64)
}

/// Calls to one account for one turn or one host pass.
pub struct DavClient<'a> {
    pub(super) account: &'a DavAccount,
    credential: &'a Credential,
    transport: &'a dyn DavTransport,
    /// Where whole days and floating times are read when a calendar names no zone.
    zone: Tz,
}

impl<'a> DavClient<'a> {
    pub fn new(
        account: &'a DavAccount,
        credential: &'a Credential,
        transport: &'a dyn DavTransport,
        zone: Tz,
    ) -> Self {
        Self {
            account,
            credential,
            transport,
            zone,
        }
    }

    pub(super) fn label(&self) -> &str {
        &self.account.label
    }

    fn scrubbed(&self, text: &str) -> String {
        let mut text = text.to_string();
        self.credential.scrub(&mut text);
        text
    }

    pub(super) async fn send(
        &self,
        operation: &str,
        request: DavRequest,
    ) -> ToolResult<DavResponse> {
        let label = self.label();
        self.account.permits(&request.url).map_err(|error| {
            ToolError::new(
                code::FAILED,
                format!("'{label}' cannot {operation}: {error}"),
            )
        })?;
        let host = host_of(&request.url);
        let mut response = self
            .transport
            .send(&request, self.credential)
            .await
            .map_err(|message| {
                ToolError::new(
                    code::FAILED,
                    format!(
                        "'{label}' could not {operation} at {host}: {}",
                        self.scrubbed(&message)
                    ),
                )
            })?;
        response.text = self.scrubbed(&response.text);
        Ok(response)
    }

    /// The error for an answer that is not the one `operation` needs.
    pub(super) fn refusal(&self, operation: &str, url: &Url, response: &DavResponse) -> ToolError {
        let label = self.label();
        let host = host_of(url);
        let status = response.status;
        let detail = xml::error_message(&response.text)
            .or_else(|| {
                response
                    .text
                    .lines()
                    .map(str::trim)
                    .find(|line| !line.is_empty() && !line.starts_with('<'))
                    .map(str::to_string)
            })
            .map(|detail| format!(": {}", clamp(&detail, DETAIL_CHARS)))
            .unwrap_or_default();
        let error = match status {
            401 => ToolError::new(
                code::FAILED,
                format!(
                    "'{label}' refused the sign-in to {operation} at {host} (HTTP 401{detail}): the user name or the password is wrong (iCloud and Fastmail need an app-specific password)"
                ),
            ),
            403 => ToolError::new(
                code::FAILED,
                format!(
                    "'{label}' does not allow you to {operation} at {host} (HTTP 403{detail}); the calendar may be read-only or not shared with this account"
                ),
            ),
            404 | 410 => ToolError::not_found(format!(
                "'{label}' has no such event or calendar at {host} (HTTP {status}); it may have been deleted"
            )),
            412 => ToolError::new(
                code::FAILED,
                format!(
                    "'{label}': the event changed at {host} while you were changing it (HTTP 412); read it again and retry"
                ),
            ),
            429 | 503 => ToolError::new(
                code::RATE_LIMITED,
                format!("'{label}' asks to slow down (HTTP {status}); try again later"),
            ),
            _ => ToolError::new(
                code::FAILED,
                format!("'{label}' answered HTTP {status} to {operation} at {host}{detail}"),
            ),
        };
        error.with_details(json!({ "http_status": status }))
    }

    fn unreadable(&self, operation: &str, url: &Url, error: &str) -> ToolError {
        ToolError::new(
            code::FAILED,
            format!(
                "'{}' answered {operation} at {} with something that could not be read: {error}",
                self.label(),
                host_of(url)
            ),
        )
    }

    fn zone_of(&self, calendar: &DavCalendar) -> Tz {
        calendar.zone.unwrap_or(self.zone)
    }

    /// The URL of the resource `href` names; only resources inside `calendar` are reachable.
    fn resource_url(&self, calendar: &DavCalendar, href: &str) -> ToolResult<Url> {
        let home = collection(calendar.url.clone());
        let url = home.join(href.trim()).map_err(|error| {
            ToolError::invalid(format!("Event href '{href}' is invalid: {error}"))
        })?;
        let base = decoded(home.path());
        let inside = url.host_str() == home.host_str()
            && url.port_or_known_default() == home.port_or_known_default()
            && decoded(url.path()).starts_with(&base)
            && decoded(url.path()).len() > base.len()
            && url.query().is_none();
        if !inside {
            return Err(ToolError::invalid(format!(
                "'{href}' is not an event of the calendar '{}'",
                calendar.name
            )));
        }
        Ok(url)
    }

    fn event_id(event_id: &str) -> ToolResult<EventId> {
        EventId::parse(event_id)
            .ok_or_else(|| ToolError::invalid(format!("'{event_id}' is not a CalDAV event id")))
    }

    /// The events of `calendar` overlapping `[from, to)`, earliest first. The server is asked
    /// to expand repeating events; one that refuses, or answers the series as it is stored, has
    /// them expanded here.
    pub async fn list(
        &self,
        calendar: &DavCalendar,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
    ) -> ToolResult<Listed> {
        let report = |expand: bool| {
            DavRequest::new(DavMethod::Report, calendar.url.clone())
                .depth(Depth::One)
                .body(DavBody::Xml(xml::events_query(from, to, expand)))
        };
        let mut response = self.send(LIST, report(true)).await?;
        if EXPAND_REFUSED.contains(&response.status) {
            response = self.send(LIST, report(false)).await?;
        }
        if !matches!(response.status, 200 | 207) {
            return Err(self.refusal(LIST, &calendar.url, &response));
        }
        let entries = xml::multistatus(&response.text)
            .map_err(|error| self.unreadable(LIST, &calendar.url, &error))?;
        let zone = self.zone_of(calendar);
        let mut listed = Listed {
            complete: true,
            ..Listed::default()
        };
        for entry in entries {
            let Some(data) = entry
                .prop(CALDAV, "calendar-data")
                .and_then(|prop| prop.trimmed())
            else {
                continue;
            };
            let Ok(url) = self.resource_url(calendar, &entry.href) else {
                listed.unreadable += 1;
                continue;
            };
            match events::read(url.path(), data, zone, from, to) {
                Ok(read) => {
                    listed.complete &= !read.limited;
                    listed.events.extend(read.events);
                }
                Err(_) => listed.unreadable += 1,
            }
        }
        listed.events.sort_by_key(|event| event.starts_at);
        if listed.events.len() > MAX_LISTED {
            listed.events.truncate(MAX_LISTED);
            listed.complete = false;
        }
        Ok(listed)
    }

    /// The stored calendar object at `url` and its ETag.
    async fn fetch(&self, operation: &str, url: &Url) -> ToolResult<(Component, Option<String>)> {
        let response = self
            .send(operation, DavRequest::new(DavMethod::Get, url.clone()))
            .await?;
        if response.status != 200 {
            return Err(self.refusal(operation, url, &response));
        }
        let object =
            ical::parse(&response.text).map_err(|error| self.unreadable(operation, url, &error))?;
        Ok((object, response.etag))
    }

    fn found(
        &self,
        operation: &str,
        url: &Url,
        object: &Component,
        zone: Tz,
        occurrence: Option<&str>,
    ) -> ToolResult<ExternalEvent> {
        events::find(url.path(), object, zone, occurrence)
            .map_err(|error| self.unreadable(operation, url, &error))?
            .ok_or_else(|| {
                ToolError::not_found(format!(
                    "'{}' reports the event as cancelled, or the series has no such occurrence",
                    self.label()
                ))
            })
    }

    /// One event of `calendar` as it is now.
    pub async fn get(&self, calendar: &DavCalendar, event_id: &str) -> ToolResult<ExternalEvent> {
        let id = Self::event_id(event_id)?;
        let url = self.resource_url(calendar, &id.href)?;
        let (object, _) = self.fetch(READ, &url).await?;
        self.found(
            READ,
            &url,
            &object,
            self.zone_of(calendar),
            id.occurrence.as_deref(),
        )
    }

    fn writable(&self, calendar: &DavCalendar) -> ToolResult<()> {
        if calendar.writable {
            return Ok(());
        }
        Err(ToolError::refused(format!(
            "The calendar '{}' of '{}' is read-only; nothing was changed",
            calendar.name,
            self.label()
        )))
    }

    /// Adds an event to `calendar` as a new resource; `draft` carries a title and the times.
    pub async fn create(
        &self,
        calendar: &DavCalendar,
        draft: &EventDraft,
    ) -> ToolResult<ExternalEvent> {
        self.writable(calendar)?;
        let name = flow_like_types::create_id();
        let object = events::new_object(&format!("{name}@flow-like"), draft, Utc::now())
            .map_err(ToolError::invalid)?;
        let url = collection(calendar.url.clone())
            .join(&format!("{name}.ics"))
            .map_err(|error| {
                ToolError::new(code::FAILED, format!("The event URL is invalid: {error}"))
            })?;
        let request = DavRequest::new(DavMethod::Put, url.clone())
            .precondition(Precondition::IfAbsent)
            .body(DavBody::Calendar(object.render()));
        let response = self.send(CREATE, request).await?;
        if !matches!(response.status, 200 | 201 | 204) {
            return Err(self.refusal(CREATE, &url, &response));
        }
        self.found(CREATE, &url, &object, self.zone_of(calendar), None)
    }

    fn refuse_occurrence(&self, operation: &str) -> ToolError {
        ToolError::new(
            code::NOT_SUPPORTED,
            format!(
                "This is one occurrence of a repeating event in '{}'. You cannot {operation} for a single occurrence of a CalDAV event yet; only the whole series",
                self.label()
            ),
        )
    }

    /// Changes what `draft` sets. [`Scope::Series`] changes the series an occurrence belongs to;
    /// a series keeps its times (moving one is refused).
    pub async fn update(
        &self,
        calendar: &DavCalendar,
        event_id: &str,
        draft: &EventDraft,
        scope: Scope,
    ) -> ToolResult<ExternalEvent> {
        self.writable(calendar)?;
        let id = Self::event_id(event_id)?;
        if id.occurrence.is_some() && scope == Scope::Event {
            return Err(self.refuse_occurrence(CHANGE));
        }
        let url = self.resource_url(calendar, &id.href)?;
        let moves = draft.starts_at.is_some() || draft.ends_at.is_some() || draft.all_day.is_some();
        let zone = self.zone_of(calendar);
        let mut last = None;
        for _ in 0..2 {
            let (mut object, etag) = self.fetch(CHANGE, &url).await?;
            if events::is_series(&object) {
                if scope == Scope::Event {
                    return Err(self.refuse_occurrence(CHANGE));
                }
                if moves {
                    return Err(ToolError::new(
                        code::NOT_SUPPORTED,
                        format!(
                            "Moving a repeating event of '{}' is not supported; change its title, place or notes, or delete the series",
                            self.label()
                        ),
                    ));
                }
            }
            events::apply(&mut object, draft, Utc::now())
                .map_err(|error| self.unreadable(CHANGE, &url, &error))?;
            let request = DavRequest::new(DavMethod::Put, url.clone())
                .precondition(etag.map_or(Precondition::None, Precondition::IfMatch))
                .body(DavBody::Calendar(object.render()));
            let response = self.send(CHANGE, request).await?;
            match response.status {
                200 | 201 | 204 => {
                    return self
                        .found(CHANGE, &url, &object, zone, id.occurrence.as_deref())
                        .or_else(|_| self.found(CHANGE, &url, &object, zone, None));
                }
                412 => last = Some(response),
                _ => return Err(self.refusal(CHANGE, &url, &response)),
            }
        }
        let response = last.unwrap_or_default();
        Err(self.refusal(CHANGE, &url, &response))
    }

    /// Deletes an event, or with [`Scope::Series`] the series it belongs to. One that is
    /// already gone counts as deleted.
    pub async fn delete(
        &self,
        calendar: &DavCalendar,
        event_id: &str,
        scope: Scope,
    ) -> ToolResult<()> {
        self.writable(calendar)?;
        let id = Self::event_id(event_id)?;
        if id.occurrence.is_some() && scope == Scope::Event {
            return Err(self.refuse_occurrence(DELETE));
        }
        let url = self.resource_url(calendar, &id.href)?;
        let mut last = None;
        for _ in 0..2 {
            let (object, etag) = match self.fetch(DELETE, &url).await {
                Ok(fetched) => fetched,
                Err(error) if matches!(http_status(&error), Some(404 | 410)) => return Ok(()),
                Err(error) => return Err(error),
            };
            if scope == Scope::Event && events::is_series(&object) {
                return Err(self.refuse_occurrence(DELETE));
            }
            let request = DavRequest::new(DavMethod::Delete, url.clone())
                .precondition(etag.map_or(Precondition::None, Precondition::IfMatch));
            let response = self.send(DELETE, request).await?;
            match response.status {
                200 | 202 | 204 | 404 | 410 => return Ok(()),
                412 => last = Some(response),
                _ => return Err(self.refusal(DELETE, &url, &response)),
            }
        }
        let response = last.unwrap_or_default();
        Err(self.refusal(DELETE, &url, &response))
    }
}
