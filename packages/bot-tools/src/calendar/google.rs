//! Google Calendar v3: the events of a calendar as single occurrences, and writes to them.

use chrono::{DateTime, NaiveDate, Utc};
use chrono_tz::Tz;
use serde_json::{Map, Value, json};

use super::provider::{EventDraft, ExternalEvent, Next, Page, local_midnight, rfc3339, segment};

const BASE: &str = "https://www.googleapis.com/calendar/v3/calendars";
const PAGE_SIZE: u32 = 100;
const FIELDS: &str = "nextPageToken,timeZone,items(id,status,htmlLink,summary,description,location,start,end,recurringEventId,transparency,eventType)";

pub fn events_url(calendar_id: &str) -> String {
    format!("{BASE}/{}/events", segment(calendar_id))
}

pub fn event_url(calendar_id: &str, event_id: &str) -> String {
    format!("{}/{}", events_url(calendar_id), segment(event_id))
}

pub fn list_query(from: DateTime<Utc>, to: DateTime<Utc>) -> Map<String, Value> {
    let mut query = Map::new();
    query.insert("timeMin".into(), json!(rfc3339(from)));
    query.insert("timeMax".into(), json!(rfc3339(to)));
    query.insert("singleEvents".into(), json!(true));
    query.insert("orderBy".into(), json!("startTime"));
    query.insert("maxResults".into(), json!(PAGE_SIZE));
    query.insert("fields".into(), json!(FIELDS));
    query
}

fn zone_of(text: Option<&str>) -> Option<Tz> {
    text.and_then(|zone| zone.trim().parse::<Tz>().ok())
}

/// A `start` or `end` object: `dateTime` for a timed event, `date` for a whole day, which starts
/// at midnight in `zone`.
fn time_of(value: &Value, zone: Tz) -> Result<(DateTime<Utc>, bool), String> {
    if let Some(text) = value.get("dateTime").and_then(Value::as_str) {
        return DateTime::parse_from_rfc3339(text)
            .map(|time| (time.with_timezone(&Utc), false))
            .map_err(|_| format!("'{text}' is not an RFC 3339 time"));
    }
    let text = value
        .get("date")
        .and_then(Value::as_str)
        .ok_or("the event has neither dateTime nor date")?;
    let date = NaiveDate::parse_from_str(text, "%Y-%m-%d")
        .map_err(|_| format!("'{text}' is not a date"))?;
    local_midnight(date, zone)
        .map(|time| (time, true))
        .ok_or_else(|| format!("midnight of {text} does not exist in {}", zone.name()))
}

/// One item of an events listing. `Ok(None)` for an event that is not shown: cancelled, or a
/// working-location marker. `calendar_zone` is the listing's `timeZone`, where whole days start.
pub fn parse_event(
    item: &Value,
    calendar_zone: Option<Tz>,
) -> Result<Option<ExternalEvent>, String> {
    let text = |key: &str| item.get(key).and_then(Value::as_str);
    let id = text("id")
        .filter(|id| !id.trim().is_empty())
        .ok_or("the event has no id")?;
    if text("status") == Some("cancelled") || text("eventType") == Some("workingLocation") {
        return Ok(None);
    }
    let start = item.get("start").ok_or("the event has no start")?;
    let zone = zone_of(start.get("timeZone").and_then(Value::as_str))
        .or(calendar_zone)
        .unwrap_or(chrono_tz::UTC);
    let (starts_at, all_day) = time_of(start, zone)?;
    let ends_at = item
        .get("end")
        .map(|end| time_of(end, zone).map(|(time, _)| time))
        .transpose()?;
    Ok(Some(ExternalEvent::new(
        id,
        text("summary"),
        starts_at,
        ends_at,
        all_day,
        text("location"),
        text("description"),
        text("htmlLink"),
        text("recurringEventId").is_some(),
        text("transparency") != Some("transparent"),
    )))
}

/// The events of one listing page and where the next page is.
pub fn parse_page(body: &Value) -> Page {
    let zone = zone_of(body.get("timeZone").and_then(Value::as_str));
    let mut page = Page::default();
    for item in body
        .get("items")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        match parse_event(item, zone) {
            Ok(Some(event)) => page.events.push(event),
            Ok(None) => {}
            Err(_) => page.unreadable += 1,
        }
    }
    page.next = body
        .get("nextPageToken")
        .and_then(Value::as_str)
        .filter(|token| !token.is_empty())
        .map(|token| Next::Token(token.to_string()));
    page
}

fn time_json(time: DateTime<Utc>, all_day: bool, tz: Option<Tz>) -> Value {
    if all_day {
        let zone = tz.unwrap_or(chrono_tz::UTC);
        return json!({ "date": time.with_timezone(&zone).date_naive().to_string() });
    }
    match tz {
        Some(zone) => json!({ "dateTime": rfc3339(time), "timeZone": zone.name() }),
        None => json!({ "dateTime": rfc3339(time) }),
    }
}

/// The request body of an insert or a patch: only what `draft` sets.
pub fn body(draft: &EventDraft) -> Value {
    let mut body = Map::new();
    if let Some(title) = &draft.title {
        body.insert("summary".into(), json!(title));
    }
    if let Some(notes) = &draft.notes {
        body.insert("description".into(), json!(notes));
    }
    if let Some(location) = &draft.location {
        body.insert("location".into(), json!(location));
    }
    if let Some(starts_at) = draft.starts_at {
        let all_day = draft.all_day.unwrap_or(false);
        let ends_at = draft.ends_at.unwrap_or(starts_at);
        body.insert("start".into(), time_json(starts_at, all_day, draft.tz));
        body.insert("end".into(), time_json(ends_at, all_day, draft.tz));
    }
    Value::Object(body)
}
