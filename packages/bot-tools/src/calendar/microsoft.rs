//! Microsoft Graph v1.0: the signed-in user's default calendar. Graph answers times as a local
//! `dateTime` with a `timeZone`, UTC unless asked otherwise; a zone that is not UTC must be an
//! IANA name, else the event is left out as unreadable.

use chrono::{DateTime, NaiveDateTime, Utc};
use chrono_tz::Tz;
use serde_json::{Map, Value, json};

use super::provider::{EventDraft, ExternalEvent, Next, Page, local_time, rfc3339, segment};

const BASE: &str = "https://graph.microsoft.com/v1.0/me";
const PAGE_SIZE: u32 = 100;
const SELECT: &str = "id,subject,start,end,isAllDay,isCancelled,location,bodyPreview,webLink,type,seriesMasterId,showAs";
const LOCAL_FORMAT: &str = "%Y-%m-%dT%H:%M:%S";

pub fn view_url() -> String {
    format!("{BASE}/calendarView")
}

pub fn events_url() -> String {
    format!("{BASE}/events")
}

pub fn event_url(event_id: &str) -> String {
    format!("{}/{}", events_url(), segment(event_id))
}

pub fn list_query(from: DateTime<Utc>, to: DateTime<Utc>) -> Map<String, Value> {
    let mut query = Map::new();
    query.insert("startDateTime".into(), json!(rfc3339(from)));
    query.insert("endDateTime".into(), json!(rfc3339(to)));
    query.insert("$top".into(), json!(PAGE_SIZE));
    query.insert("$select".into(), json!(SELECT));
    query.insert("$orderby".into(), json!("start/dateTime"));
    query
}

fn zone_of(name: &str) -> Result<Tz, String> {
    let name = name.trim();
    if name.is_empty() || name.eq_ignore_ascii_case("UTC") || name == "Etc/UTC" {
        return Ok(chrono_tz::UTC);
    }
    name.parse::<Tz>()
        .map_err(|_| format!("time zone '{name}' is not an IANA zone"))
}

/// A `dateTimeTimeZone` object.
fn time_of(value: &Value) -> Result<DateTime<Utc>, String> {
    let text = value
        .get("dateTime")
        .and_then(Value::as_str)
        .ok_or("the event time has no dateTime")?;
    let zone = zone_of(
        value
            .get("timeZone")
            .and_then(Value::as_str)
            .unwrap_or("UTC"),
    )?;
    let local = NaiveDateTime::parse_from_str(text.trim(), "%Y-%m-%dT%H:%M:%S%.f")
        .map_err(|_| format!("'{text}' is not a local date and time"))?;
    local_time(local, zone).ok_or_else(|| format!("'{text}' does not exist in {}", zone.name()))
}

/// One event of a listing or a write's answer. `Ok(None)` for a cancelled event.
pub fn parse_event(item: &Value) -> Result<Option<ExternalEvent>, String> {
    let text = |key: &str| item.get(key).and_then(Value::as_str);
    let id = text("id")
        .filter(|id| !id.trim().is_empty())
        .ok_or("the event has no id")?;
    if item.get("isCancelled").and_then(Value::as_bool) == Some(true) {
        return Ok(None);
    }
    let starts_at = time_of(item.get("start").ok_or("the event has no start")?)?;
    let ends_at = item.get("end").map(time_of).transpose()?;
    let recurring = text("seriesMasterId").is_some()
        || matches!(
            text("type"),
            Some("occurrence" | "exception" | "seriesMaster")
        );
    Ok(Some(ExternalEvent::new(
        id,
        text("subject"),
        starts_at,
        ends_at,
        item.get("isAllDay").and_then(Value::as_bool) == Some(true),
        item.pointer("/location/displayName")
            .and_then(Value::as_str),
        text("bodyPreview"),
        text("webLink"),
        recurring,
        !matches!(text("showAs"), Some("free" | "workingElsewhere")),
    )))
}

/// The events of one `calendarView` page and the link to the next.
pub fn parse_page(body: &Value) -> Page {
    let mut page = Page::default();
    for item in body
        .get("value")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        match parse_event(item) {
            Ok(Some(event)) => page.events.push(event),
            Ok(None) => {}
            Err(_) => page.unreadable += 1,
        }
    }
    page.next = body
        .get("@odata.nextLink")
        .and_then(Value::as_str)
        .filter(|link| !link.is_empty())
        .map(|link| Next::Url(link.to_string()));
    page
}

/// A whole day is midnight of its date in the event's zone; a timed event goes as UTC.
fn time_json(time: DateTime<Utc>, all_day: bool, tz: Option<Tz>) -> Value {
    match (all_day, tz) {
        (true, Some(zone)) => json!({
            "dateTime": time.with_timezone(&zone).date_naive().and_hms_opt(0, 0, 0).map(|day| day.format(LOCAL_FORMAT).to_string()),
            "timeZone": zone.name(),
        }),
        (true, None) => json!({
            "dateTime": time.date_naive().and_hms_opt(0, 0, 0).map(|day| day.format(LOCAL_FORMAT).to_string()),
            "timeZone": "UTC",
        }),
        (false, _) => json!({
            "dateTime": time.naive_utc().format(LOCAL_FORMAT).to_string(),
            "timeZone": "UTC",
        }),
    }
}

/// The request body of a create or a patch: only what `draft` sets.
pub fn body(draft: &EventDraft) -> Value {
    let mut body = Map::new();
    if let Some(title) = &draft.title {
        body.insert("subject".into(), json!(title));
    }
    if let Some(notes) = &draft.notes {
        body.insert(
            "body".into(),
            json!({ "contentType": "text", "content": notes }),
        );
    }
    if let Some(location) = &draft.location {
        body.insert("location".into(), json!({ "displayName": location }));
    }
    if let Some(starts_at) = draft.starts_at {
        let all_day = draft.all_day.unwrap_or(false);
        let ends_at = draft.ends_at.unwrap_or(starts_at);
        body.insert("isAllDay".into(), json!(all_day));
        body.insert("start".into(), time_json(starts_at, all_day, draft.tz));
        body.insert("end".into(), time_json(ends_at, all_day, draft.tz));
    }
    Value::Object(body)
}
