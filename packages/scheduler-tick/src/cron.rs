//! Cron occurrence computation for one tick window.
//!
//! The `cron` crate wants seven fields (`sec min hour dom mon dow year`) while
//! users write the Unix five-field form; the normalisation is the same one
//! `flow-like-sinks` applies so a schedule fires at the same instants whether
//! it is dispatched by the docker-compose sink service or by this tick job.
//! Expressions are evaluated in UTC: `CronScheduleInfo` carries no timezone,
//! and the reference cron consumer runs the schedules in UTC too.

use chrono::{DateTime, Utc};
use std::str::FromStr;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum CronError {
    #[error("cron expression '{expression}' could not be parsed: {reason}")]
    Parse { expression: String, reason: String },
    /// The window would fire more than `cap` times. Returned instead of a
    /// truncated list so the caller can decide to skip the schedule outright: an
    /// every-second expression must never turn one tick into hundreds of
    /// trigger POSTs, and a truncated prefix would fire stale occurrences while
    /// silently dropping the newest ones.
    #[error(
        "cron expression '{expression}' fires more than {cap} times between {after} and {until}"
    )]
    TooManyOccurrences {
        expression: String,
        cap: usize,
        after: DateTime<Utc>,
        until: DateTime<Utc>,
    },
}

/// Normalize a Unix-style cron expression (5 or 6 fields) into the 7-field
/// form the `cron` crate requires (`sec min hour dom mon dow year`).
///
/// - 5-field (`min hour dom mon dow`)   -> prepend `0 ` for seconds, append ` *` for year.
/// - 6-field (`sec min hour dom mon dow`) -> append ` *` for year.
/// - anything else is passed through unchanged and left to the parser.
pub fn normalize_cron_for_crate(expr: &str) -> String {
    let parts: Vec<&str> = expr.split_whitespace().collect();
    match parts.len() {
        5 => format!("0 {} *", parts.join(" ")),
        6 => format!("{} *", parts.join(" ")),
        _ => expr.to_string(),
    }
}

/// Every firing instant of `expr` strictly after `after_exclusive` and up to and
/// including `until_inclusive`, ascending. An empty or inverted window yields an
/// empty list without touching the parser's iterator.
pub fn occurrences_in(
    expr: &str,
    after_exclusive: DateTime<Utc>,
    until_inclusive: DateTime<Utc>,
    cap: usize,
) -> Result<Vec<DateTime<Utc>>, CronError> {
    let normalized = normalize_cron_for_crate(expr);
    let schedule = cron::Schedule::from_str(&normalized).map_err(|error| CronError::Parse {
        expression: expr.to_string(),
        reason: error.to_string(),
    })?;

    if after_exclusive >= until_inclusive {
        return Ok(Vec::new());
    }

    // `Schedule::after` is already exclusive of its argument, but only at
    // whole-second granularity; the explicit bound check keeps the contract
    // exact for sub-second `after` values as well.
    let occurrences: Vec<DateTime<Utc>> = schedule
        .after(&after_exclusive)
        .take_while(|instant| *instant <= until_inclusive)
        .filter(|instant| *instant > after_exclusive)
        .take(cap.saturating_add(1))
        .collect();

    if occurrences.len() > cap {
        return Err(CronError::TooManyOccurrences {
            expression: expr.to_string(),
            cap,
            after: after_exclusive,
            until: until_inclusive,
        });
    }
    Ok(occurrences)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn at(h: u32, m: u32, s: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 8, 15, h, m, s).unwrap()
    }

    #[test]
    fn normalizes_five_and_six_field_expressions() {
        assert_eq!(normalize_cron_for_crate("*/5 * * * *"), "0 */5 * * * * *");
        assert_eq!(normalize_cron_for_crate("30 * * * * *"), "30 * * * * * *");
        assert_eq!(
            normalize_cron_for_crate("0 0 12 * * * 2026"),
            "0 0 12 * * * 2026"
        );
        assert_eq!(normalize_cron_for_crate("  0   0 * * *  "), "0 0 0 * * * *");
    }

    #[test]
    fn five_field_every_minute_yields_the_minute_boundaries_in_the_window() {
        let occurrences = occurrences_in("* * * * *", at(12, 4, 30), at(12, 6, 30), 100).unwrap();
        assert_eq!(occurrences, vec![at(12, 5, 0), at(12, 6, 0)]);
    }

    #[test]
    fn six_field_expressions_fire_on_seconds() {
        let occurrences =
            occurrences_in("*/20 * * * * *", at(12, 0, 0), at(12, 1, 0), 100).unwrap();
        assert_eq!(
            occurrences,
            vec![at(12, 0, 20), at(12, 0, 40), at(12, 1, 0)]
        );
    }

    #[test]
    fn seven_field_expressions_pass_through() {
        let occurrences =
            occurrences_in("0 30 12 * * * 2026", at(12, 0, 0), at(13, 0, 0), 100).unwrap();
        assert_eq!(occurrences, vec![at(12, 30, 0)]);
        assert!(
            occurrences_in("0 30 12 * * * 2025", at(12, 0, 0), at(13, 0, 0), 100)
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn window_start_is_exclusive_and_end_is_inclusive() {
        let occurrences = occurrences_in("* * * * *", at(12, 5, 0), at(12, 6, 0), 100).unwrap();
        assert_eq!(occurrences, vec![at(12, 6, 0)]);

        let sub_second_start = at(12, 5, 0) + chrono::Duration::milliseconds(500);
        let occurrences = occurrences_in("* * * * *", sub_second_start, at(12, 6, 0), 100).unwrap();
        assert_eq!(occurrences, vec![at(12, 6, 0)]);
    }

    #[test]
    fn empty_or_inverted_windows_yield_nothing() {
        assert!(
            occurrences_in("* * * * *", at(12, 5, 0), at(12, 5, 0), 100)
                .unwrap()
                .is_empty()
        );
        assert!(
            occurrences_in("* * * * *", at(12, 6, 0), at(12, 5, 0), 100)
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn cap_rejects_high_frequency_expressions() {
        let error = occurrences_in("* * * * * *", at(12, 0, 0), at(12, 10, 0), 30).unwrap_err();
        assert!(matches!(
            error,
            CronError::TooManyOccurrences { cap: 30, .. }
        ));
        // Exactly `cap` occurrences is still accepted.
        assert_eq!(
            occurrences_in("* * * * *", at(12, 0, 0), at(12, 10, 0), 10)
                .unwrap()
                .len(),
            10
        );
    }

    #[test]
    fn unparseable_expressions_are_reported() {
        let error = occurrences_in("not a cron", at(12, 0, 0), at(12, 1, 0), 10).unwrap_err();
        assert!(matches!(error, CronError::Parse { .. }));
        assert!(matches!(
            occurrences_in("99 * * * *", at(12, 0, 0), at(12, 1, 0), 10),
            Err(CronError::Parse { .. })
        ));
    }
}
