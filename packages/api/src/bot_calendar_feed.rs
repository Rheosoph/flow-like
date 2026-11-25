//! The iCalendar a bot's subscription link serves: its own events, scheduled tasks and, when the
//! owner opts in, dated memories, in a window around now. Pure: the route loads the rows.

use crate::entity::bot_task;
use chrono::{DateTime, Duration, NaiveDate, Utc};
use chrono_tz::Tz;
use flow_like_bot_memory::host::FactView;
use flow_like_types::{
    bot_calendar::{EventSchedule, TaskEvent},
    bot_task::{DEFAULT_TIMEZONE, STORED_CRON, STORED_EVENT, STORED_ONCE, TaskKind, TaskSchedule},
};
use icalendar::{Calendar, Component, Event, EventLike, Parameter, Property};

pub const WINDOW_PAST_DAYS: i64 = 30;
pub const WINDOW_FUTURE_DAYS: i64 = 180;
pub const MAX_ITEMS: usize = 2_000;
pub const MAX_FACTS: usize = 5_000;
const TASK_MINUTES: i64 = 15;
const MEMORY_SPAN_DAYS: i64 = 31;
const SUMMARY_MAX_CHARS: usize = 120;
const REFRESH: &str = "PT15M";
const PRODID: &str = "-//Flow-Like//Bot Calendar//EN";
const CAPABILITY_EVENTS: &str = "calendar";
const CAPABILITY_TASKS: &str = "scheduling";
const CAPABILITY_MEMORIES: &str = "memory";

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct FeedLayers {
    pub events: bool,
    pub tasks: bool,
    pub memories: bool,
    pub notes: bool,
}

impl FeedLayers {
    /// The layers the bot's capabilities leave switched on; notes only ever add detail.
    pub fn within(self, capabilities: &[String]) -> Self {
        let holds = |id: &str| capabilities.iter().any(|capability| capability == id);
        Self {
            events: self.events && holds(CAPABILITY_EVENTS),
            tasks: self.tasks && holds(CAPABILITY_TASKS),
            memories: self.memories && holds(CAPABILITY_MEMORIES),
            notes: self.notes,
        }
    }
}

pub struct FeedSource<'a> {
    pub bot_name: &'a str,
    pub layers: FeedLayers,
    pub tasks: &'a [bot_task::Model],
    pub facts: &'a [FactView],
    pub now: DateTime<Utc>,
}

struct Window {
    from: DateTime<Utc>,
    to: DateTime<Utc>,
}

enum When {
    Timed {
        start: DateTime<Utc>,
        end: Option<DateTime<Utc>>,
    },
    Days {
        start: NaiveDate,
        end: Option<NaiveDate>,
    },
}

impl When {
    fn start(&self) -> DateTime<Utc> {
        match self {
            Self::Timed { start, .. } => *start,
            Self::Days { start, .. } => start.and_time(chrono::NaiveTime::MIN).and_utc(),
        }
    }
}

struct Item {
    start: DateTime<Utc>,
    uid: String,
    event: Event,
}

struct Draft<'a> {
    uid: String,
    summary: &'a str,
    stamp: DateTime<Utc>,
    when: When,
    location: Option<&'a str>,
    description: Option<&'a str>,
    category: Option<&'a str>,
    opaque: bool,
}

impl Draft<'_> {
    fn build(self) -> Item {
        let start = self.when.start();
        let mut event = Event::new();
        event
            .uid(&self.uid)
            .summary(self.summary)
            .timestamp(self.stamp);
        match self.when {
            When::Timed { start, end } => {
                event.starts(start);
                if let Some(end) = end {
                    event.ends(end);
                }
            }
            When::Days { start, end } => {
                event.starts(start);
                if let Some(end) = end {
                    event.ends(end);
                }
            }
        }
        if let Some(location) = self.location.map(str::trim).filter(|text| !text.is_empty()) {
            event.location(location);
        }
        if let Some(description) = self
            .description
            .map(str::trim)
            .filter(|text| !text.is_empty())
        {
            event.description(description);
        }
        if let Some(category) = self.category {
            event.add_property("CATEGORIES", category);
        }
        event.add_property("TRANSP", if self.opaque { "OPAQUE" } else { "TRANSPARENT" });
        Item {
            start,
            uid: self.uid,
            event: event.done(),
        }
    }
}

fn stamp_of(uid_time: DateTime<Utc>) -> String {
    uid_time.format("%Y%m%dT%H%M%SZ").to_string()
}

fn zone_of(name: Option<&str>) -> Tz {
    name.map(str::trim)
        .filter(|name| !name.is_empty())
        .unwrap_or(DEFAULT_TIMEZONE)
        .parse()
        .unwrap_or(Tz::UTC)
}

fn local_date(time: DateTime<Utc>, zone: Tz) -> NaiveDate {
    time.with_timezone(&zone).date_naive()
}

fn all_day(start: DateTime<Utc>, length: Option<Duration>, zone: Tz) -> When {
    let first = local_date(start, zone);
    let end = length
        .filter(|length| *length > Duration::zero())
        .map(|length| {
            let finish = (start + length).with_timezone(&zone);
            let day = finish.date_naive();
            let at_midnight = finish.time() == chrono::NaiveTime::MIN;
            if at_midnight && day > first {
                day
            } else {
                day.max(first) + Duration::days(1)
            }
        });
    When::Days { start: first, end }
}

fn event_items(source: &FeedSource<'_>, window: &Window, out: &mut Vec<Item>) {
    for row in source.tasks.iter().filter(|row| row.kind == STORED_EVENT) {
        let Some(event) = row
            .event
            .clone()
            .and_then(|value| serde_json::from_value::<TaskEvent>(value).ok())
            .filter(|event| event.external.is_none())
        else {
            continue;
        };
        let Ok(schedule) = EventSchedule::of(&event, row.cron.as_deref(), row.timezone.as_deref())
        else {
            continue;
        };
        let repeating = schedule.recurrence.is_some();
        let zone = zone_of(row.timezone.as_deref());
        for start in schedule.occurrences(window.from, window.to, MAX_ITEMS) {
            let uid = if repeating {
                format!("{}-{}@flow-like", row.id, stamp_of(start))
            } else {
                format!("{}@flow-like", row.id)
            };
            let when = if event.all_day {
                all_day(start, schedule.duration, zone)
            } else {
                When::Timed {
                    start,
                    end: schedule
                        .duration
                        .filter(|length| *length > Duration::zero())
                        .map(|length| start + length),
                }
            };
            out.push(
                Draft {
                    uid,
                    summary: &row.title,
                    stamp: row.updated_at.to_utc(),
                    when,
                    location: event.location.as_deref(),
                    description: source.layers.notes.then_some(row.instruction.as_str()),
                    category: None,
                    opaque: true,
                }
                .build(),
            );
        }
    }
}

fn task_times(row: &bot_task::Model, window: &Window, now: DateTime<Utc>) -> Vec<DateTime<Utc>> {
    if row.kind == STORED_ONCE {
        return row
            .run_at
            .map(|at| at.to_utc())
            .filter(|at| *at >= window.from && *at < window.to)
            .into_iter()
            .collect();
    }
    let Some(schedule) = row.cron.as_deref().and_then(|cron| {
        TaskSchedule::parse(
            TaskKind::Cron,
            None,
            Some(cron),
            row.timezone.as_deref(),
            now,
        )
        .ok()
    }) else {
        return Vec::new();
    };
    let mut times = Vec::new();
    let mut cursor = schedule.first_run(window.from - Duration::seconds(1));
    while let Some(at) = cursor {
        if at >= window.to || times.len() >= MAX_ITEMS {
            break;
        }
        times.push(at);
        cursor = schedule.after_fire(at);
    }
    times
}

fn task_items(source: &FeedSource<'_>, window: &Window, out: &mut Vec<Item>) {
    for row in source
        .tasks
        .iter()
        .filter(|row| row.enabled && (row.kind == STORED_ONCE || row.kind == STORED_CRON))
    {
        for at in task_times(row, window, source.now) {
            out.push(
                Draft {
                    uid: format!("{}-{}@flow-like", row.id, stamp_of(at)),
                    summary: &row.title,
                    stamp: row.updated_at.to_utc(),
                    when: When::Timed {
                        start: at,
                        end: Some(at + Duration::minutes(TASK_MINUTES)),
                    },
                    location: None,
                    description: source.layers.notes.then_some(row.instruction.as_str()),
                    category: Some("Task"),
                    opaque: false,
                }
                .build(),
            );
        }
    }
}

fn instant(text: Option<&str>) -> Option<DateTime<Utc>> {
    text.and_then(|text| DateTime::parse_from_rfc3339(text.trim()).ok())
        .map(|time| time.to_utc())
}

fn is_whole_day(time: DateTime<Utc>) -> bool {
    time.time() == chrono::NaiveTime::MIN
}

fn one_line(text: &str, max: usize) -> String {
    let flat = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.chars().count() <= max {
        return flat;
    }
    let mut cut: String = flat.chars().take(max.saturating_sub(1)).collect();
    cut.push('…');
    cut
}

fn memory_when(fact: &FactView, window: &Window) -> Option<When> {
    if let Some(time) = instant(fact.time.as_deref()) {
        if time < window.from || time >= window.to {
            return None;
        }
        return Some(if is_whole_day(time) {
            When::Days {
                start: time.date_naive(),
                end: None,
            }
        } else {
            When::Timed {
                start: time,
                end: None,
            }
        });
    }
    let from = instant(fact.valid_from.as_deref())?;
    let to = instant(fact.valid_to.as_deref())?;
    if to < from || to - from > Duration::days(MEMORY_SPAN_DAYS) {
        return None;
    }
    if to < window.from || from >= window.to {
        return None;
    }
    Some(if is_whole_day(from) && is_whole_day(to) {
        When::Days {
            start: from.date_naive(),
            end: Some(to.date_naive() + Duration::days(1)),
        }
    } else {
        When::Timed {
            start: from,
            end: (to > from).then_some(to),
        }
    })
}

fn memory_items(source: &FeedSource<'_>, window: &Window, out: &mut Vec<Item>) {
    for fact in source.facts.iter().take(MAX_FACTS) {
        if fact.superseded_by.is_some() || fact.conversation_scope.is_some() {
            continue;
        }
        if instant(fact.expires_at.as_deref()).is_some_and(|expires| expires <= source.now) {
            continue;
        }
        let Some(when) = memory_when(fact, window) else {
            continue;
        };
        out.push(
            Draft {
                uid: format!("{}@flow-like", fact.id),
                summary: &one_line(&fact.text, SUMMARY_MAX_CHARS),
                stamp: instant(Some(&fact.recorded_at)).unwrap_or(source.now),
                when,
                location: None,
                description: None,
                category: Some("Memory"),
                opaque: false,
            }
            .build(),
        );
    }
}

pub fn calendar_name(bot_name: &str) -> String {
    let name = one_line(bot_name, 80);
    let name = if name.is_empty() { "Bot" } else { &name };
    format!("{name} · Flow-Like")
}

/// The feed document: events first by start, at most [`MAX_ITEMS`] of them.
pub fn render(source: &FeedSource<'_>) -> String {
    let window = Window {
        from: source.now - Duration::days(WINDOW_PAST_DAYS),
        to: source.now + Duration::days(WINDOW_FUTURE_DAYS),
    };
    let mut items = Vec::new();
    if source.layers.events {
        event_items(source, &window, &mut items);
    }
    if source.layers.tasks {
        task_items(source, &window, &mut items);
    }
    if source.layers.memories {
        memory_items(source, &window, &mut items);
    }
    items.sort_by(|a, b| a.start.cmp(&b.start).then_with(|| a.uid.cmp(&b.uid)));
    items.truncate(MAX_ITEMS);

    let mut calendar = Calendar::empty();
    calendar
        .append_property(Property::new("VERSION", "2.0"))
        .append_property(Property::new("PRODID", PRODID))
        .append_property(Property::new("CALSCALE", "GREGORIAN"))
        .append_property(Property::new("METHOD", "PUBLISH"))
        .name(&calendar_name(source.bot_name));
    calendar
        .append_property(
            Property::new("REFRESH-INTERVAL", REFRESH)
                .append_parameter(Parameter::new("VALUE", "DURATION"))
                .done(),
        )
        .append_property(Property::new("X-PUBLISHED-TTL", REFRESH));
    for item in items {
        calendar.push(item.event);
    }
    calendar.to_string()
}

/// A file name for the download header: ASCII letters, digits and hyphens of the bot's name.
pub fn file_slug(bot_name: &str) -> String {
    let mut slug = String::new();
    for char in bot_name.chars() {
        if char.is_ascii_alphanumeric() {
            slug.push(char.to_ascii_lowercase());
        } else if !slug.is_empty() && !slug.ends_with('-') {
            slug.push('-');
        }
    }
    let slug = slug.trim_matches('-');
    if slug.is_empty() {
        "bot-calendar".to_string()
    } else {
        format!("{}-calendar", slug.chars().take(48).collect::<String>())
    }
}

pub fn etag_of(body: &str) -> String {
    format!("\"{}\"", &blake3::hash(body.as_bytes()).to_hex()[..32])
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use serde_json::json;

    fn now() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 11, 20, 12, 0, 0).unwrap()
    }

    fn row(id: &str, kind: &str) -> bot_task::Model {
        let at = now().fixed_offset();
        bot_task::Model {
            id: id.into(),
            bot_id: "bot".into(),
            owner_id: "owner".into(),
            kind: kind.into(),
            run_at: None,
            cron: None,
            timezone: None,
            instruction: "Do the thing, then rest.".into(),
            title: "Plan".into(),
            next_run_at: None,
            last_run_at: None,
            enabled: true,
            monitoring: None,
            created_by: "OWNER".into(),
            created_at: at,
            updated_at: at,
            conversation_id: None,
            requested_by: None,
            owner_written: None,
            request_conversation_id: None,
            request_message_id: None,
            place_latitude: None,
            place_longitude: None,
            place_radius_m: None,
            place_on: None,
            place_label: None,
            place_repeat: None,
            place_until: None,
            event: None,
        }
    }

    fn event_row(id: &str, event: serde_json::Value) -> bot_task::Model {
        let mut task = row(id, STORED_EVENT);
        task.title = "Dentist, 2nd floor; bring card\nsecond line".into();
        task.event = Some(event);
        task
    }

    fn layers() -> FeedLayers {
        FeedLayers {
            events: true,
            tasks: true,
            memories: true,
            notes: false,
        }
    }

    fn render_of(tasks: &[bot_task::Model], facts: &[FactView], layers: FeedLayers) -> String {
        render(&FeedSource {
            bot_name: "Secretary",
            layers,
            tasks,
            facts,
            now: now(),
        })
    }

    fn fact(id: &str, text: &str) -> FactView {
        FactView {
            id: id.into(),
            text: text.into(),
            kind: "event".into(),
            subject: "user".into(),
            subject_id: None,
            object_id: None,
            relation: None,
            source: "stated".into(),
            confidence: 1.0,
            recorded_at: "2026-11-01T08:00:00Z".into(),
            valid_from: None,
            valid_to: None,
            geojson: None,
            time: None,
            expires_at: None,
            superseded_by: None,
            evidence: Vec::new(),
            conversation_scope: None,
        }
    }

    fn unfolded(body: &str) -> String {
        body.replace("\r\n ", "")
    }

    fn count(body: &str, needle: &str) -> usize {
        body.matches(needle).count()
    }

    #[test]
    fn header_names_the_calendar_and_asks_for_quarter_hour_refresh() {
        let body = render_of(&[], &[], layers());
        assert!(body.starts_with("BEGIN:VCALENDAR\r\n"));
        assert!(body.contains("PRODID:-//Flow-Like//Bot Calendar//EN"));
        assert!(body.contains("X-WR-CALNAME:Secretary · Flow-Like"));
        assert!(body.contains("REFRESH-INTERVAL;VALUE=DURATION:PT15M"));
        assert!(body.contains("X-PUBLISHED-TTL:PT15M"));
        assert!(body.trim_end().ends_with("END:VCALENDAR"));
        assert!(!body.contains("VALARM"));
        assert!(!body.contains("ATTENDEE"));
    }

    #[test]
    fn a_one_time_event_has_a_stable_uid_and_its_length() {
        let task = event_row(
            "evt1",
            json!({"starts_at": "2026-12-01T09:00:00Z", "ends_at": "2026-12-01T10:30:00Z", "location": "Main St 1"}),
        );
        let body = unfolded(&render_of(&[task], &[], layers()));
        assert_eq!(count(&body, "BEGIN:VEVENT"), 1);
        assert!(body.contains("UID:evt1@flow-like"));
        assert!(body.contains("DTSTART:20261201T090000Z"));
        assert!(body.contains("DTEND:20261201T103000Z"));
        assert!(body.contains("LOCATION:Main St 1"));
        assert!(body.contains("TRANSP:OPAQUE"));
        assert!(body.contains("DTSTAMP:20261120T120000Z"));
    }

    #[test]
    fn titles_keep_commas_semicolons_and_newlines() {
        let task = event_row("evt1", json!({"starts_at": "2026-12-01T09:00:00Z"}));
        let body = unfolded(&render_of(&[task], &[], layers()));
        assert!(
            body.contains("SUMMARY:Dentist\\, 2nd floor\\; bring card\\nsecond line"),
            "{body}"
        );
        assert!(!body.contains("DTEND"));
    }

    #[test]
    fn long_lines_are_folded_at_75_octets() {
        let mut task = event_row("evt1", json!({"starts_at": "2026-12-01T09:00:00Z"}));
        task.title = "x".repeat(300);
        let body = render_of(&[task], &[], layers());
        assert!(body.split("\r\n").all(|line| line.len() <= 75), "{body}");
        assert!(unfolded(&body).contains(&format!("SUMMARY:{}", "x".repeat(300))));
    }

    #[test]
    fn a_repeating_event_expands_with_a_uid_per_occurrence() {
        let mut task = event_row(
            "evt2",
            json!({"starts_at": "2026-11-23T08:00:00Z", "ends_at": "2026-11-23T08:30:00Z"}),
        );
        task.cron = Some("0 8 * * MON".into());
        task.timezone = Some("UTC".into());
        let body = unfolded(&render_of(&[task], &[], layers()));
        assert!(body.contains("UID:evt2-20261123T080000Z@flow-like"));
        assert!(body.contains("UID:evt2-20261130T080000Z@flow-like"));
        assert!(!body.contains("UID:evt2@flow-like"));
        let last = now() + Duration::days(WINDOW_FUTURE_DAYS);
        assert!(count(&body, "BEGIN:VEVENT") >= 25);
        assert!(!body.contains(&format!("{}", (last + Duration::days(8)).format("%Y%m%d"))));
    }

    #[test]
    fn an_all_day_event_uses_dates_in_the_tasks_zone() {
        let mut task = event_row(
            "evt3",
            json!({"starts_at": "2026-12-23T23:00:00Z", "ends_at": "2026-12-24T23:00:00Z", "all_day": true}),
        );
        task.timezone = Some("Europe/Berlin".into());
        let body = unfolded(&render_of(&[task], &[], layers()));
        assert!(body.contains("DTSTART;VALUE=DATE:20261224"), "{body}");
        assert!(body.contains("DTEND;VALUE=DATE:20261225"), "{body}");
    }

    #[test]
    fn external_events_and_place_tasks_stay_out() {
        let external = event_row(
            "ext",
            json!({"starts_at": "2026-12-01T09:00:00Z", "external": {"connector_id": "c", "provider": "google", "calendar_id": "primary", "event_id": "e1"}}),
        );
        let place = row("place", "PLACE");
        let body = render_of(&[external, place], &[], layers());
        assert_eq!(count(&body, "BEGIN:VEVENT"), 0);
    }

    #[test]
    fn a_once_task_is_a_quarter_hour_transparent_task() {
        let mut task = row("task1", STORED_ONCE);
        task.run_at = Some((now() + Duration::days(2)).fixed_offset());
        let body = unfolded(&render_of(&[task], &[], layers()));
        assert!(body.contains("CATEGORIES:Task"));
        assert!(body.contains("TRANSP:TRANSPARENT"));
        assert!(body.contains("DTSTART:20261122T120000Z"));
        assert!(body.contains("DTEND:20261122T121500Z"));
        assert!(body.contains("UID:task1-20261122T120000Z@flow-like"));
    }

    #[test]
    fn cron_tasks_list_their_runs_and_disabled_tasks_none() {
        let mut daily = row("daily", STORED_CRON);
        daily.cron = Some("0 7 * * *".into());
        let mut off = daily.clone();
        off.id = "off".into();
        off.enabled = false;
        let body = unfolded(&render_of(&[daily, off], &[], layers()));
        assert!(body.contains("UID:daily-20261121T070000Z@flow-like"));
        assert!(body.contains("UID:daily-20261120T070000Z@flow-like"));
        assert!(!body.contains("UID:off-"));
    }

    #[test]
    fn layers_and_capabilities_switch_content_off() {
        let mut task = row("task1", STORED_ONCE);
        task.run_at = Some((now() + Duration::days(2)).fixed_offset());
        let event = event_row("evt1", json!({"starts_at": "2026-12-01T09:00:00Z"}));
        let rows = [task, event];
        let only_events = FeedLayers {
            tasks: false,
            ..layers()
        };
        let body = render_of(&rows, &[], only_events);
        assert!(body.contains("UID:evt1@flow-like") && !body.contains("task1"));
        let held = layers().within(&["scheduling".to_string()]);
        assert_eq!(
            held,
            FeedLayers {
                events: false,
                tasks: true,
                memories: false,
                notes: false
            }
        );
        let body = render_of(&rows, &[], held);
        assert!(body.contains("task1") && !body.contains("evt1"));
    }

    #[test]
    fn notes_add_the_instruction_only_when_asked() {
        let mut task = row("task1", STORED_ONCE);
        task.run_at = Some((now() + Duration::days(2)).fixed_offset());
        let event = event_row("evt1", json!({"starts_at": "2026-12-01T09:00:00Z"}));
        let rows = [task, event];
        let without = render_of(&rows, &[], layers());
        assert!(!without.contains("DESCRIPTION"));
        let with = unfolded(&render_of(
            &rows,
            &[],
            FeedLayers {
                notes: true,
                ..layers()
            },
        ));
        assert_eq!(count(&with, "DESCRIPTION:Do the thing\\, then rest."), 2);
    }

    #[test]
    fn memories_with_a_day_a_short_span_or_neither() {
        let mut birthday = fact("f1", "Anna's birthday");
        birthday.time = Some("2026-12-24T00:00:00+00:00".into());
        let mut trip = fact("f2", "Trip to Lisbon");
        trip.valid_from = Some("2026-12-27T00:00:00+00:00".into());
        trip.valid_to = Some("2026-12-30T00:00:00+00:00".into());
        let mut meeting = fact("f3", "Call with Max");
        meeting.time = Some("2026-12-02T14:30:00+00:00".into());
        let mut year = fact("f4", "Lives in Berlin");
        year.valid_from = Some("2026-01-01T00:00:00+00:00".into());
        year.valid_to = Some("2026-12-31T00:00:00+00:00".into());
        let mut old = fact("f5", "Superseded");
        old.time = Some("2026-12-05T10:00:00+00:00".into());
        old.superseded_by = Some("f6".into());
        let mut expired = fact("f7", "Expired");
        expired.time = Some("2026-12-05T10:00:00+00:00".into());
        expired.expires_at = Some("2026-11-01T00:00:00+00:00".into());
        let mut room = fact("f8", "Room fact");
        room.time = Some("2026-12-05T10:00:00+00:00".into());
        room.conversation_scope = Some("group".into());
        let body = unfolded(&render_of(
            &[],
            &[birthday, trip, meeting, year, old, expired, room],
            layers(),
        ));
        assert!(body.contains("DTSTART;VALUE=DATE:20261224"));
        assert!(body.contains("DTSTART;VALUE=DATE:20261227"));
        assert!(body.contains("DTEND;VALUE=DATE:20261231"));
        assert!(body.contains("DTSTART:20261202T143000Z"));
        assert_eq!(count(&body, "CATEGORIES:Memory"), 3);
        assert!(body.contains("SUMMARY:Anna's birthday"));
        assert!(!body.contains("Berlin") && !body.contains("Superseded"));
        assert!(!body.contains("Expired") && !body.contains("Room fact"));
    }

    #[test]
    fn memories_are_left_out_unless_the_layer_is_on() {
        let mut birthday = fact("f1", "Anna's birthday");
        birthday.time = Some("2026-12-24T00:00:00+00:00".into());
        let body = render_of(
            &[],
            &[birthday],
            FeedLayers {
                memories: false,
                ..layers()
            },
        );
        assert_eq!(count(&body, "BEGIN:VEVENT"), 0);
    }

    #[test]
    fn a_long_memory_text_becomes_one_short_summary() {
        let mut long = fact("f1", &format!("line one\n{}", "word ".repeat(80)));
        long.time = Some("2026-12-24T00:00:00+00:00".into());
        let body = unfolded(&render_of(&[], &[long], layers()));
        let summary = body
            .split("\r\n")
            .find(|line| line.starts_with("SUMMARY:"))
            .unwrap();
        assert!(summary.chars().count() <= "SUMMARY:".len() + SUMMARY_MAX_CHARS);
        assert!(summary.ends_with('…') && !summary.contains("\\n"));
    }

    #[test]
    fn the_feed_stops_at_the_cap_keeping_the_earliest() {
        let mut busy = event_row(
            "busy",
            json!({"starts_at": "2026-11-20T00:00:00Z", "ends_at": "2026-11-20T00:05:00Z"}),
        );
        busy.cron = Some("*/5 * * * *".into());
        busy.timezone = Some("UTC".into());
        let body = unfolded(&render_of(&[busy], &[], layers()));
        assert_eq!(count(&body, "BEGIN:VEVENT"), MAX_ITEMS);
        assert!(body.contains("UID:busy-20261120T000000Z@flow-like"));
        assert!(!body.contains("UID:busy-20261201T000000Z@flow-like"));
    }

    #[test]
    fn events_come_out_earliest_first() {
        let late = event_row("late", json!({"starts_at": "2026-12-20T09:00:00Z"}));
        let early = event_row("early", json!({"starts_at": "2026-12-01T09:00:00Z"}));
        let body = render_of(&[late, early], &[], layers());
        assert!(body.find("UID:early@").unwrap() < body.find("UID:late@").unwrap());
    }

    #[test]
    fn output_is_stable_for_unchanged_rows() {
        let task = event_row("evt1", json!({"starts_at": "2026-12-01T09:00:00Z"}));
        let first = render_of(std::slice::from_ref(&task), &[], layers());
        let second = render_of(std::slice::from_ref(&task), &[], layers());
        assert_eq!(etag_of(&first), etag_of(&second));
    }

    #[test]
    fn slug_and_name_are_safe() {
        assert_eq!(file_slug("Secretary Ü!"), "secretary-calendar");
        assert_eq!(file_slug("***"), "bot-calendar");
        assert_eq!(calendar_name("  "), "Bot · Flow-Like");
        assert_eq!(calendar_name("A\nB"), "A B · Flow-Like");
    }
}
