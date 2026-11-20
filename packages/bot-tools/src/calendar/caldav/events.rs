//! Calendar object resources as the calendar module's events, and the iCalendar a write sends.
//! A listing gives one [`ExternalEvent`] per occurrence: its id is the resource's href, and for
//! an occurrence of a repeating event `href#RECURRENCE-ID` ([`EventId`]), the same whether the
//! server or this client expanded the series.

use std::collections::HashSet;

use chrono::{DateTime, Duration, Utc};
use chrono_tz::Tz;

use super::ical::{self, Component, Line, When, date_text, local_text, utc_text};
use super::recur;
use super::zones::{Zone, Zones};
use crate::calendar::provider::{EventDraft, ExternalEvent};

/// Occurrences one series gives in a listing before it is cut short.
pub const MAX_OCCURRENCES: u16 = 500;
const PRODID: &str = "-//Flow-Like//Bot Calendar//EN";

/// The id of an event of a CalDAV calendar: the resource's href, and for one occurrence of a
/// repeating event its RECURRENCE-ID (`YYYYMMDD`, or UTC `YYYYMMDDTHHMMSSZ`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EventId {
    pub href: String,
    pub occurrence: Option<String>,
}

impl EventId {
    pub fn parse(id: &str) -> Option<Self> {
        let id = id.trim();
        let (href, occurrence) = match id.split_once('#') {
            Some((href, occurrence)) => (
                href,
                Some(occurrence.trim().to_string()).filter(|text| !text.is_empty()),
            ),
            None => (id, None),
        };
        (!href.is_empty()).then(|| Self {
            href: href.to_string(),
            occurrence,
        })
    }

    pub fn encode(&self) -> String {
        match &self.occurrence {
            Some(occurrence) => format!("{}#{occurrence}", self.href),
            None => self.href.clone(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Length {
    Days(i64),
    Exact(Duration),
}

/// One VEVENT read: its start, its length and what kind of entry it is.
struct Fields<'a> {
    event: &'a Component,
    start: When,
    length: Length,
    /// The RECURRENCE-ID key of an occurrence the server sent on its own.
    recurrence: Option<String>,
    cancelled: bool,
}

fn unreadable_time(line: &str) -> String {
    format!("the event's {line} does not exist in its time zone")
}

impl<'a> Fields<'a> {
    fn of(event: &'a Component, zones: &Zones, zone: Tz) -> Result<Self, String> {
        let start = ical::when_of(
            event
                .property("DTSTART")
                .ok_or("the event has no DTSTART")?,
            zones,
        )?;
        let end = event.property("DTEND");
        let duration = event.property("DURATION");
        let length = match start {
            When::Date(date) => {
                let days = match (end, duration) {
                    (Some(end), _) => match ical::when_of(end, zones)? {
                        When::Date(end) => (end - date).num_days(),
                        _ => 1,
                    },
                    (None, Some(duration)) => ical::duration(&duration.value)
                        .ok_or_else(|| format!("DURATION '{}' cannot be read", duration.value))?
                        .num_days(),
                    (None, None) => 1,
                };
                Length::Days(days.max(1))
            }
            _ => {
                let exact = match (end, duration) {
                    (Some(end), _) => {
                        let starts = start
                            .instant(zone)
                            .ok_or_else(|| unreadable_time("DTSTART"))?;
                        let ends = ical::when_of(end, zones)?
                            .instant(zone)
                            .ok_or_else(|| unreadable_time("DTEND"))?;
                        ends - starts
                    }
                    (None, Some(duration)) => ical::duration(&duration.value)
                        .ok_or_else(|| format!("DURATION '{}' cannot be read", duration.value))?,
                    (None, None) => Duration::zero(),
                };
                Length::Exact(exact.max(Duration::zero()))
            }
        };
        let recurrence = event
            .property("RECURRENCE-ID")
            .map(|line| {
                ical::when_of(line, zones)?
                    .key(zone)
                    .ok_or_else(|| unreadable_time("RECURRENCE-ID"))
            })
            .transpose()?;
        let cancelled = event
            .property("STATUS")
            .is_some_and(|line| line.value.trim().eq_ignore_ascii_case("CANCELLED"));
        Ok(Self {
            event,
            start,
            length,
            recurrence,
            cancelled,
        })
    }

    fn repeats(&self) -> bool {
        self.event.property("RRULE").is_some() || self.event.property("RDATE").is_some()
    }

    fn span(&self) -> Duration {
        match self.length {
            Length::Days(days) => Duration::try_days(days).unwrap_or(Duration::MAX),
            Length::Exact(exact) => exact,
        }
    }

    fn materialize(
        &self,
        id: String,
        start: When,
        recurring: bool,
        zone: Tz,
    ) -> Option<ExternalEvent> {
        let starts_at = start.instant(zone)?;
        let ends_at = match (self.length, start) {
            (Length::Days(days), When::Date(date)) => date
                .checked_add_signed(Duration::try_days(days)?)
                .and_then(|end| When::Date(end).instant(zone)),
            (Length::Days(days), _) => starts_at.checked_add_signed(Duration::try_days(days)?),
            (Length::Exact(exact), _) => starts_at.checked_add_signed(exact),
        };
        let busy = !self
            .event
            .property("TRANSP")
            .is_some_and(|line| line.value.trim().eq_ignore_ascii_case("TRANSPARENT"));
        Some(ExternalEvent::new(
            &id,
            self.event.text("SUMMARY").as_deref(),
            starts_at,
            ends_at,
            start.is_date(),
            self.event.text("LOCATION").as_deref(),
            self.event.text("DESCRIPTION").as_deref(),
            self.event.property("URL").map(|line| line.value.trim()),
            recurring,
            busy,
        ))
    }

    /// The occurrence starts of a series beginning in `[after, before]`, without the excluded
    /// and the separately sent ones.
    fn starts(
        &self,
        zones: &Zones,
        zone: Tz,
        after: DateTime<Utc>,
        before: DateTime<Utc>,
        overridden: &HashSet<String>,
    ) -> (Vec<When>, bool) {
        let within = |when: &When| {
            when.instant(zone)
                .is_some_and(|time| time >= after && time <= before)
        };
        let ruled = self.event.property("RRULE").map(|rule| {
            recur::rule_starts(
                &rule.value,
                self.start,
                zone,
                after,
                before,
                MAX_OCCURRENCES,
            )
        });
        let (mut starts, limited) = match ruled {
            Some(Ok(ruled)) => (ruled.starts, ruled.limited),
            Some(Err(_)) | None => (Some(self.start).filter(within).into_iter().collect(), false),
        };
        for line in self.event.properties("RDATE") {
            starts.extend(
                ical::whens_of(line, zones)
                    .unwrap_or_default()
                    .into_iter()
                    .filter(within),
            );
        }
        let mut skipped: HashSet<String> = self
            .event
            .properties("EXDATE")
            .flat_map(|line| ical::whens_of(line, zones).unwrap_or_default())
            .filter_map(|when| when.key(zone))
            .collect();
        skipped.extend(overridden.iter().cloned());
        starts.retain(|when| when.key(zone).is_some_and(|key| skipped.insert(key)));
        starts.sort_by_key(|when| when.instant(zone));
        (starts, limited)
    }
}

/// The events of one calendar object resource.
#[derive(Debug, Default)]
pub struct Read {
    pub events: Vec<ExternalEvent>,
    /// A series gave more occurrences than [`MAX_OCCURRENCES`].
    pub limited: bool,
}

fn overlaps(event: &ExternalEvent, from: DateTime<Utc>, to: DateTime<Utc>) -> bool {
    let end = event.ends_at.unwrap_or(event.starts_at);
    event.starts_at < to && (end > from || event.starts_at >= from)
}

fn occurrence_keys(calendar: &Component, zones: &Zones, zone: Tz) -> HashSet<String> {
    calendar
        .children_named("VEVENT")
        .filter_map(|event| event.property("RECURRENCE-ID"))
        .filter_map(|line| ical::when_of(line, zones).ok()?.key(zone))
        .collect()
}

fn id_of(href: &str, occurrence: Option<String>) -> String {
    EventId {
        href: href.to_string(),
        occurrence,
    }
    .encode()
}

/// The events of the resource at `href` overlapping `[from, to)`, earliest first: cancelled
/// ones left out, repeating ones as single occurrences. Whole days and floating times are read
/// in `zone`.
pub fn read(
    href: &str,
    data: &str,
    zone: Tz,
    from: DateTime<Utc>,
    to: DateTime<Utc>,
) -> Result<Read, String> {
    let calendar = ical::parse(data)?;
    let zones = Zones::of(&calendar);
    let overridden = occurrence_keys(&calendar, &zones, zone);
    let mut read = Read::default();
    for event in calendar.children_named("VEVENT") {
        let fields = Fields::of(event, &zones, zone)?;
        if fields.cancelled {
            continue;
        }
        if let Some(key) = &fields.recurrence {
            read.events.extend(fields.materialize(
                id_of(href, Some(key.clone())),
                fields.start,
                true,
                zone,
            ));
            continue;
        }
        if !fields.repeats() {
            read.events
                .extend(fields.materialize(id_of(href, None), fields.start, false, zone));
            continue;
        }
        let after = from
            .checked_sub_signed(fields.span())
            .and_then(|time| time.checked_sub_signed(Duration::days(1)))
            .unwrap_or(from);
        let before = to + Duration::days(1);
        let (starts, limited) = fields.starts(&zones, zone, after, before, &overridden);
        read.limited |= limited;
        for start in starts {
            let key = start.key(zone);
            read.events
                .extend(fields.materialize(id_of(href, key), start, true, zone));
        }
    }
    read.events.retain(|event| overlaps(event, from, to));
    read.events.sort_by_key(|event| event.starts_at);
    Ok(read)
}

/// The series' main VEVENT: the one without a RECURRENCE-ID.
fn master(calendar: &Component) -> Option<&Component> {
    calendar
        .children_named("VEVENT")
        .find(|event| event.property("RECURRENCE-ID").is_none())
}

/// Whether the resource holds a repeating event (a rule, extra dates, or moved occurrences).
pub fn is_series(calendar: &Component) -> bool {
    calendar.children_named("VEVENT").any(|event| {
        event.property("RRULE").is_some()
            || event.property("RDATE").is_some()
            || event.property("RECURRENCE-ID").is_some()
    })
}

/// One event of a resource: the main event for `None`, else the occurrence with that
/// RECURRENCE-ID key. `Ok(None)` when it is cancelled or the series has no such occurrence.
pub fn find(
    href: &str,
    calendar: &Component,
    zone: Tz,
    occurrence: Option<&str>,
) -> Result<Option<ExternalEvent>, String> {
    let zones = Zones::of(calendar);
    let Some(key) = occurrence else {
        let Some(main) = master(calendar) else {
            return Ok(None);
        };
        let fields = Fields::of(main, &zones, zone)?;
        if fields.cancelled {
            return Ok(None);
        }
        return Ok(fields.materialize(id_of(href, None), fields.start, fields.repeats(), zone));
    };
    for event in calendar.children_named("VEVENT") {
        let fields = Fields::of(event, &zones, zone)?;
        if fields.recurrence.as_deref() == Some(key) {
            if fields.cancelled {
                return Ok(None);
            }
            return Ok(fields.materialize(
                id_of(href, Some(key.to_string())),
                fields.start,
                true,
                zone,
            ));
        }
    }
    let Some(main) = master(calendar) else {
        return Ok(None);
    };
    let fields = Fields::of(main, &zones, zone)?;
    let wanted = ical::when_value(key, None, &zones)?;
    let Some(at) = wanted.instant(zone) else {
        return Ok(None);
    };
    if fields.cancelled || !fields.repeats() {
        return Ok(None);
    }
    let (starts, _) = fields.starts(
        &zones,
        zone,
        at - Duration::days(1),
        at + Duration::days(1),
        &HashSet::new(),
    );
    Ok(starts
        .into_iter()
        .find(|start| start.key(zone).as_deref() == Some(key))
        .and_then(|start| {
            fields.materialize(id_of(href, Some(key.to_string())), start, true, zone)
        }))
}

/// DTSTART and DTEND for a write. Whole days are dates in `tz`; a timed event keeps the zone
/// its old start named (`keep`), else goes as UTC.
fn time_lines(
    starts_at: DateTime<Utc>,
    ends_at: Option<DateTime<Utc>>,
    all_day: bool,
    tz: Option<Tz>,
    keep: Option<&(String, Tz)>,
) -> (Line, Option<Line>) {
    if all_day {
        let zone = tz.unwrap_or(chrono_tz::UTC);
        let first = starts_at.with_timezone(&zone).date_naive();
        let last = ends_at
            .map(|end| end.with_timezone(&zone).date_naive())
            .filter(|end| *end > first)
            .or_else(|| first.succ_opt())
            .unwrap_or(first);
        return (
            Line::new("DTSTART", date_text(first)).with_param("VALUE", "DATE"),
            Some(Line::new("DTEND", date_text(last)).with_param("VALUE", "DATE")),
        );
    }
    let line = |name: &str, time: DateTime<Utc>| match keep {
        Some((tzid, zone)) => Line::new(name, local_text(time.with_timezone(zone).naive_local()))
            .with_param("TZID", tzid),
        None => Line::new(name, utc_text(time)),
    };
    (
        line("DTSTART", starts_at),
        ends_at
            .filter(|end| *end >= starts_at)
            .map(|end| line("DTEND", end)),
    )
}

/// The calendar object of a new event; `draft` carries a title and the times.
pub fn new_object(uid: &str, draft: &EventDraft, now: DateTime<Utc>) -> Result<Component, String> {
    let starts_at = draft.starts_at.ok_or("a new event needs a start")?;
    let stamp = utc_text(now);
    let mut event = Component::new("VEVENT");
    event.push(Line::text("UID", uid));
    event.push(Line::new("DTSTAMP", stamp.clone()));
    event.push(Line::new("CREATED", stamp.clone()));
    event.push(Line::new("LAST-MODIFIED", stamp));
    let (start, end) = time_lines(
        starts_at,
        draft.ends_at,
        draft.all_day.unwrap_or(false),
        draft.tz,
        None,
    );
    event.push(start);
    if let Some(end) = end {
        event.push(end);
    }
    let title = draft.title.as_deref().map(str::trim).unwrap_or_default();
    event.push(Line::text("SUMMARY", title));
    for (name, value) in [("LOCATION", &draft.location), ("DESCRIPTION", &draft.notes)] {
        if let Some(text) = value
            .as_deref()
            .map(str::trim)
            .filter(|text| !text.is_empty())
        {
            event.push(Line::text(name, text));
        }
    }
    let mut calendar = Component::new("VCALENDAR");
    calendar.push(Line::new("VERSION", "2.0"));
    calendar.push(Line::new("PRODID", PRODID));
    calendar.push(Line::new("CALSCALE", "GREGORIAN"));
    calendar.push_child(event);
    Ok(calendar)
}

fn set_text(event: &mut Component, name: &str, text: &str, stale: &str) {
    event.remove(stale);
    match text.trim() {
        "" => event.remove(name),
        text => event.set(Line::text(name, text)),
    }
}

/// Changes what `draft` sets on the main VEVENT of `calendar`; every other line, attendee,
/// alarm and component stays as it was.
pub fn apply(
    calendar: &mut Component,
    draft: &EventDraft,
    now: DateTime<Utc>,
) -> Result<(), String> {
    let keep = master(calendar)
        .and_then(|main| main.property("DTSTART"))
        .and_then(|line| {
            let tzid = line.param("TZID")?.to_string();
            match Zones::of(calendar).resolve(&tzid)? {
                Zone::Iana(zone) => Some((tzid, zone)),
                Zone::Fixed(_) => None,
            }
        });
    let main = calendar
        .children_mut()
        .find(|event| event.name == "VEVENT" && event.property("RECURRENCE-ID").is_none())
        .ok_or("the resource has no main VEVENT")?;
    if let Some(title) = &draft.title {
        main.set(Line::text("SUMMARY", title.trim()));
    }
    if let Some(location) = &draft.location {
        set_text(main, "LOCATION", location, "X-APPLE-STRUCTURED-LOCATION");
    }
    if let Some(notes) = &draft.notes {
        set_text(main, "DESCRIPTION", notes, "X-ALT-DESC");
    }
    if let Some(starts_at) = draft.starts_at {
        let (start, end) = time_lines(
            starts_at,
            draft.ends_at,
            draft.all_day.unwrap_or(false),
            draft.tz,
            keep.as_ref(),
        );
        main.set(start);
        main.remove("DURATION");
        match end {
            Some(end) => main.set(end),
            None => main.remove("DTEND"),
        }
        let sequence = main
            .property("SEQUENCE")
            .and_then(|line| line.value.trim().parse::<u32>().ok())
            .map_or(1, |sequence| sequence.saturating_add(1));
        main.set(Line::new("SEQUENCE", sequence.to_string()));
    }
    let stamp = utc_text(now);
    main.set(Line::new("DTSTAMP", stamp.clone()));
    main.set(Line::new("LAST-MODIFIED", stamp));
    Ok(())
}
