//! Free time between busy intervals, within daily working hours in a time zone.

use chrono::{DateTime, Duration, NaiveTime, Utc};
use chrono_tz::Tz;

use super::provider::{local_midnight, local_time};

/// The hours of each day a free slot may fall into; `end` none runs to midnight.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Hours {
    pub start: NaiveTime,
    pub end: Option<NaiveTime>,
}

impl Hours {
    /// `HH:MM` to `HH:MM`, where the end may be `24:00`; the end must come after the start.
    pub fn parse(start: Option<&str>, end: Option<&str>) -> Result<Self, String> {
        let time = |field: &str, text: &str| {
            NaiveTime::parse_from_str(text.trim(), "%H:%M")
                .map_err(|_| format!("'{field}' value '{text}' is not a time like 09:00"))
        };
        let start = match start.map(str::trim).filter(|text| !text.is_empty()) {
            Some(text) => time("day_start", text)?,
            None => NaiveTime::from_hms_opt(9, 0, 0).unwrap_or_default(),
        };
        let end = match end.map(str::trim).filter(|text| !text.is_empty()) {
            Some("24:00") => None,
            Some(text) => Some(time("day_end", text)?),
            None => NaiveTime::from_hms_opt(18, 0, 0),
        };
        if end.is_some_and(|end| end <= start) {
            return Err("'day_end' must come after 'day_start'".into());
        }
        Ok(Self { start, end })
    }
}

/// Busy intervals sorted and joined where they overlap or touch.
fn merge(mut busy: Vec<(DateTime<Utc>, DateTime<Utc>)>) -> Vec<(DateTime<Utc>, DateTime<Utc>)> {
    busy.retain(|(start, end)| end > start);
    busy.sort();
    let mut merged: Vec<(DateTime<Utc>, DateTime<Utc>)> = Vec::new();
    for (start, end) in busy {
        match merged.last_mut() {
            Some(last) if start <= last.1 => last.1 = last.1.max(end),
            _ => merged.push((start, end)),
        }
    }
    merged
}

/// Free intervals of at least `minimum` inside `[from, to)` and inside `hours` of each day in
/// `tz`, earliest first, at most `limit`.
pub fn free_slots(
    busy: Vec<(DateTime<Utc>, DateTime<Utc>)>,
    from: DateTime<Utc>,
    to: DateTime<Utc>,
    minimum: Duration,
    hours: Hours,
    tz: Tz,
    limit: usize,
) -> Vec<(DateTime<Utc>, DateTime<Utc>)> {
    let busy = merge(busy);
    let mut slots = Vec::new();
    let mut day = from.with_timezone(&tz).date_naive();
    let last = to.with_timezone(&tz).date_naive();
    while day <= last && slots.len() < limit {
        let open = local_time(day.and_time(hours.start), tz);
        let close = match hours.end {
            Some(end) => local_time(day.and_time(end), tz),
            None => day.succ_opt().and_then(|next| local_midnight(next, tz)),
        };
        if let (Some(open), Some(close)) = (open, close) {
            let close = close.min(to);
            let mut cursor = open.max(from);
            for (start, end) in &busy {
                if *end <= cursor {
                    continue;
                }
                if *start >= close {
                    break;
                }
                if *start - cursor >= minimum {
                    slots.push((cursor, *start));
                }
                cursor = cursor.max(*end);
            }
            if close - cursor >= minimum {
                slots.push((cursor, close));
            }
        }
        let Some(next) = day.succ_opt() else { break };
        day = next;
    }
    slots.truncate(limit);
    slots
}

#[cfg(test)]
mod tests {
    use super::*;

    fn utc(text: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(text)
            .unwrap()
            .with_timezone(&Utc)
    }

    fn hours(start: &str, end: &str) -> Hours {
        Hours::parse(Some(start), Some(end)).unwrap()
    }

    #[test]
    fn busy_times_split_the_working_day() {
        let busy = vec![
            (utc("2026-11-23T10:00:00Z"), utc("2026-11-23T11:00:00Z")),
            (utc("2026-11-23T10:30:00Z"), utc("2026-11-23T12:00:00Z")),
            (utc("2026-11-23T13:00:00Z"), utc("2026-11-23T13:20:00Z")),
        ];
        let slots = free_slots(
            busy,
            utc("2026-11-23T00:00:00Z"),
            utc("2026-11-24T00:00:00Z"),
            Duration::minutes(45),
            hours("09:00", "17:00"),
            chrono_tz::UTC,
            10,
        );
        assert_eq!(
            slots,
            vec![
                (utc("2026-11-23T09:00:00Z"), utc("2026-11-23T10:00:00Z")),
                (utc("2026-11-23T12:00:00Z"), utc("2026-11-23T13:00:00Z")),
                (utc("2026-11-23T13:20:00Z"), utc("2026-11-23T17:00:00Z")),
            ]
        );
    }

    #[test]
    fn working_hours_follow_the_time_zone_and_the_span() {
        let slots = free_slots(
            Vec::new(),
            utc("2026-11-23T09:00:00Z"),
            utc("2026-11-24T12:00:00Z"),
            Duration::minutes(30),
            hours("09:00", "18:00"),
            "Europe/Berlin".parse().unwrap(),
            10,
        );
        assert_eq!(
            slots,
            vec![
                (utc("2026-11-23T09:00:00Z"), utc("2026-11-23T17:00:00Z")),
                (utc("2026-11-24T08:00:00Z"), utc("2026-11-24T12:00:00Z")),
            ]
        );
    }

    #[test]
    fn hours_are_checked() {
        assert!(Hours::parse(Some("18:00"), Some("09:00")).is_err());
        assert!(Hours::parse(Some("9am"), None).is_err());
        assert_eq!(Hours::parse(None, Some("24:00")).unwrap().end, None);
        let all_day = free_slots(
            vec![(utc("2026-11-23T00:00:00Z"), utc("2026-11-23T23:00:00Z"))],
            utc("2026-11-23T00:00:00Z"),
            utc("2026-11-24T00:00:00Z"),
            Duration::minutes(60),
            Hours::parse(Some("00:00"), Some("24:00")).unwrap(),
            chrono_tz::UTC,
            10,
        );
        assert_eq!(
            all_day,
            vec![(utc("2026-11-23T23:00:00Z"), utc("2026-11-24T00:00:00Z"))]
        );
    }
}
