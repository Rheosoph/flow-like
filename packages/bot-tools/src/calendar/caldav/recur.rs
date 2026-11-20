//! Occurrences of a repeating VEVENT, for servers that answer the series instead of expanding
//! it. RRULE goes through the `rrule` crate in the event's own zone (so a weekly 09:00 stays at
//! 09:00 across DST); whole days repeat in UTC dates. `UNTIL` is applied here, in UTC, because
//! calendars write it in every form.

use chrono::{DateTime, NaiveDate, NaiveDateTime, TimeZone, Utc};
use chrono_tz::Tz;
use rrule::{RRule, Tz as RuleZone, Unvalidated};

use super::ical::When;
use super::zones::Zone;
use crate::calendar::provider::local_time;

/// Occurrence starts of one series inside a window, at most `limit`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Starts {
    pub starts: Vec<When>,
    /// More occurrences fell in the window than `limit`.
    pub limited: bool,
}

/// The rule without its UNTIL part, and the UNTIL text.
fn split_until(rule: &str) -> (String, Option<String>) {
    let mut until = None;
    let parts: Vec<&str> = rule
        .trim()
        .split(';')
        .filter(|part| match part.split_once('=') {
            Some((key, value)) if key.trim().eq_ignore_ascii_case("UNTIL") => {
                until = Some(value.trim().to_string());
                false
            }
            _ => !part.trim().is_empty(),
        })
        .collect();
    (parts.join(";"), until)
}

/// The last instant an UNTIL allows: a date counts whole, a floating time is read in `zone`.
fn until_instant(text: &str, zone: Tz, date_series: bool) -> Option<DateTime<Utc>> {
    let text = text.trim();
    if text.len() == 8 {
        let date = NaiveDate::parse_from_str(text, "%Y%m%d").ok()?;
        if date_series {
            return Some(date.and_hms_opt(0, 0, 0)?.and_utc());
        }
        return local_time(date.and_hms_opt(23, 59, 59)?, zone);
    }
    if let Some(utc) = text.strip_suffix(['Z', 'z']) {
        return NaiveDateTime::parse_from_str(utc, "%Y%m%dT%H%M%S")
            .ok()
            .map(|time| time.and_utc());
    }
    local_time(
        NaiveDateTime::parse_from_str(text, "%Y%m%dT%H%M%S").ok()?,
        zone,
    )
}

/// The first occurrence as the rule library wants it, and how its results read back.
fn rule_start(start: When, zone: Tz) -> Option<(DateTime<RuleZone>, bool)> {
    let utc = RuleZone::UTC;
    Some(match start {
        When::Date(date) => (
            date.and_hms_opt(0, 0, 0)?.and_utc().with_timezone(&utc),
            true,
        ),
        When::Utc(time) => (time.with_timezone(&utc), false),
        When::Local(time, Some(Zone::Iana(tz))) => (
            tz.from_local_datetime(&time)
                .earliest()?
                .with_timezone(&RuleZone::Tz(tz)),
            false,
        ),
        When::Local(_, Some(Zone::Fixed(_))) => (start.instant(zone)?.with_timezone(&utc), false),
        When::Local(time, None) => (
            zone.from_local_datetime(&time)
                .earliest()?
                .with_timezone(&RuleZone::Tz(zone)),
            false,
        ),
    })
}

/// The starts of the occurrences `rule` (an RRULE value) gives a series starting at `start`,
/// beginning within `[after, before]`. The series' own start always counts (RFC 5545 §3.8.5.3).
pub fn rule_starts(
    rule: &str,
    start: When,
    zone: Tz,
    after: DateTime<Utc>,
    before: DateTime<Utc>,
    limit: u16,
) -> Result<Starts, String> {
    let (first, dates) =
        rule_start(start, zone).ok_or("the series start does not exist in its time zone")?;
    let (rule_text, until) = split_until(rule);
    let mut parsed: RRule<Unvalidated> = rule_text
        .parse()
        .map_err(|error| format!("RRULE '{rule_text}' cannot be read: {error}"))?;
    if let Some(text) = until {
        let until = until_instant(&text, zone, dates)
            .ok_or_else(|| format!("RRULE UNTIL '{text}' cannot be read"))?;
        if until < first.with_timezone(&Utc) {
            let only = start
                .instant(zone)
                .filter(|time| *time >= after && *time <= before);
            return Ok(Starts {
                starts: only.map(|_| start).into_iter().collect(),
                limited: false,
            });
        }
        parsed = parsed.until(until.with_timezone(&RuleZone::UTC));
    }
    let set = parsed
        .build(first)
        .map_err(|error| format!("RRULE '{rule}' is not valid: {error}"))?
        .after(after.with_timezone(&RuleZone::UTC))
        .before(before.with_timezone(&RuleZone::UTC));
    let result = set.all(limit);
    let mut starts: Vec<When> = result
        .dates
        .iter()
        .map(|time| {
            if dates {
                When::Date(time.with_timezone(&Utc).date_naive())
            } else {
                When::Utc(time.with_timezone(&Utc))
            }
        })
        .collect();
    let first_utc = first.with_timezone(&Utc);
    if first_utc >= after && first_utc <= before {
        let own = if dates { start } else { When::Utc(first_utc) };
        if !starts.contains(&own) {
            starts.insert(0, own);
        }
    }
    Ok(Starts {
        starts,
        limited: result.limited,
    })
}
