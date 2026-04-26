//! Translate standard Unix cron expressions into AWS EventBridge Scheduler cron.
//!
//! Differences documented at:
//! <https://docs.aws.amazon.com/scheduler/latest/UserGuide/schedule-types.html>
//!
//! - AWS cron has **six** required fields: `minutes hours day-of-month month day-of-week year`.
//! - AWS day-of-week is `1–7` with `1 = Sunday`; standard Unix cron is `0–6` with `0 = Sunday`.
//! - Exactly one of day-of-month / day-of-week must be `?` when the other is set (AWS
//!   rejects `*` in both).
//! - AWS does not support sub-minute precision; seconds in a 6-field standard cron
//!   are dropped with a warning.

use super::SchedulerError;

/// Convert a standard (5- or 6-field with seconds) Unix cron expression into AWS cron syntax.
///
/// Returns the fully wrapped expression, e.g. `cron(0 9 ? * MON-FRI *)`.
pub fn standard_to_aws_cron(expr: &str) -> Result<String, SchedulerError> {
    let trimmed = expr.trim();
    if trimmed.is_empty() {
        return Err(SchedulerError::InvalidCronExpression(
            "empty cron expression".into(),
        ));
    }

    let parts: Vec<&str> = trimmed.split_whitespace().collect();

    let (minute, hour, dom, month, dow) = match parts.len() {
        5 => (parts[0], parts[1], parts[2], parts[3], parts[4]),
        6 => {
            // Standard 6-field cron has seconds as the first field. AWS has no
            // sub-minute precision, so anything other than `0` or `*` (which
            // collapse to "at :00" / "any second" respectively and round-trip
            // cleanly to minute resolution) would silently degrade — reject
            // instead of pretending to honor it.
            let sec = parts[0];
            if sec != "0" && sec != "*" {
                return Err(SchedulerError::InvalidCronExpression(format!(
                    "AWS EventBridge Scheduler has no sub-minute precision; \
                     seconds field must be `0` or `*`, got `{}` in `{}`",
                    sec, trimmed
                )));
            }
            (parts[1], parts[2], parts[3], parts[4], parts[5])
        }
        n => {
            return Err(SchedulerError::InvalidCronExpression(format!(
                "expected 5 or 6 fields, got {}: {}",
                n, trimmed
            )));
        }
    };

    let dow_aws = translate_day_of_week(dow)?;
    let (dom_aws, dow_aws) = apply_question_mark(dom, &dow_aws);

    Ok(format!(
        "cron({} {} {} {} {} *)",
        minute, hour, dom_aws, month, dow_aws
    ))
}

/// Shift numeric day-of-week values from Unix (0–6, Sun=0) to AWS (1–7, Sun=1).
/// Names (SUN-SAT), ranges, lists, steps, and wildcards are preserved.
fn translate_day_of_week(dow: &str) -> Result<String, SchedulerError> {
    if dow == "*" || dow == "?" {
        return Ok(dow.to_string());
    }

    // Strip an optional step suffix like "*/2" or "1-5/2" — shift the base only.
    let (base, step) = match dow.split_once('/') {
        Some((b, s)) => (b, Some(s)),
        None => (dow, None),
    };

    let shifted = base
        .split(',')
        .map(shift_dow_token)
        .collect::<Result<Vec<_>, _>>()?
        .join(",");

    Ok(match step {
        Some(s) => format!("{}/{}", shifted, s),
        None => shifted,
    })
}

fn shift_dow_token(token: &str) -> Result<String, SchedulerError> {
    let t = token.trim();
    if t.is_empty() {
        return Err(SchedulerError::InvalidCronExpression(
            "empty day-of-week token".into(),
        ));
    }

    // Range like "1-5"
    if let Some((a, b)) = t.split_once('-') {
        return Ok(format!("{}-{}", shift_single(a)?, shift_single(b)?));
    }

    shift_single(t)
}

fn shift_single(v: &str) -> Result<String, SchedulerError> {
    // Names pass through; AWS accepts SUN-SAT.
    if v.chars().any(|c| c.is_ascii_alphabetic()) {
        return Ok(v.to_ascii_uppercase());
    }

    let n: u32 = v.parse().map_err(|_| {
        SchedulerError::InvalidCronExpression(format!("invalid day-of-week value: {}", v))
    })?;

    // Unix treats 7 as Sunday too; AWS uses 1=Sun..7=Sat.
    let shifted = match n {
        0 => 1,
        1..=6 => n + 1,
        7 => 1,
        _ => {
            return Err(SchedulerError::InvalidCronExpression(format!(
                "day-of-week out of range (0-7): {}",
                n
            )));
        }
    };
    Ok(shifted.to_string())
}

/// AWS requires `?` in exactly one of day-of-month / day-of-week. If standard cron
/// had `*` in both (or restricts via both), we substitute `?` into the one that
/// has no constraint.
fn apply_question_mark(dom: &str, dow: &str) -> (String, String) {
    let dom_star = dom == "*";
    let dow_star = dow == "*";
    let dom_q = dom == "?";
    let dow_q = dow == "?";

    match (dom_star || dom_q, dow_star || dow_q) {
        // Both unconstrained — prefer ? in DOW to keep the usual "every day" reading.
        (true, true) => (dom.to_string(), "?".to_string()),
        // DOM constrained, DOW unconstrained — DOW must be ?.
        (false, true) => (dom.to_string(), "?".to_string()),
        // DOW constrained, DOM unconstrained — DOM must be ?.
        (true, false) => ("?".to_string(), dow.to_string()),
        // Both constrained — AWS rejects this. Pass through and let AWS surface the error.
        (false, false) => (dom.to_string(), dow.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn five_field_daily() {
        assert_eq!(
            standard_to_aws_cron("0 9 * * *").unwrap(),
            "cron(0 9 * * ? *)"
        );
    }

    #[test]
    fn six_field_drops_seconds() {
        // 0 9 every day with seconds=0
        assert_eq!(
            standard_to_aws_cron("0 0 9 * * *").unwrap(),
            "cron(0 9 * * ? *)"
        );
    }

    #[test]
    fn weekdays_range_shifts_dow() {
        // Unix 1-5 (Mon-Fri) => AWS 2-6
        assert_eq!(
            standard_to_aws_cron("0 9 * * 1-5").unwrap(),
            "cron(0 9 ? * 2-6 *)"
        );
    }

    #[test]
    fn sunday_wraps() {
        // Unix 0 or 7 => AWS 1 (Sunday)
        assert_eq!(
            standard_to_aws_cron("0 18 * * 0").unwrap(),
            "cron(0 18 ? * 1 *)"
        );
        assert_eq!(
            standard_to_aws_cron("0 18 * * 7").unwrap(),
            "cron(0 18 ? * 1 *)"
        );
    }

    #[test]
    fn dom_specified_dow_gets_q() {
        assert_eq!(
            standard_to_aws_cron("0 9 15 * *").unwrap(),
            "cron(0 9 15 * ? *)"
        );
    }

    #[test]
    fn names_preserved() {
        assert_eq!(
            standard_to_aws_cron("0 9 * * MON-FRI").unwrap(),
            "cron(0 9 ? * MON-FRI *)"
        );
    }

    #[test]
    fn list_of_days() {
        // Unix 0,6 (Sun+Sat) => AWS 1,7
        assert_eq!(
            standard_to_aws_cron("0 9 * * 0,6").unwrap(),
            "cron(0 9 ? * 1,7 *)"
        );
    }

    #[test]
    fn invalid_field_count() {
        assert!(standard_to_aws_cron("0 9 *").is_err());
    }

    #[test]
    fn invalid_dow() {
        assert!(standard_to_aws_cron("0 9 * * 8").is_err());
    }

    #[test]
    fn sub_minute_rejected() {
        // Seconds=*/30 would silently become "every minute" on AWS — surface
        // the incompatibility loudly instead.
        assert!(standard_to_aws_cron("*/30 * * * * *").is_err());
        assert!(standard_to_aws_cron("15 * * * * *").is_err());
        // `0` and `*` are accepted because they don't alter minute resolution.
        assert!(standard_to_aws_cron("0 * * * * *").is_ok());
        assert!(standard_to_aws_cron("* * * * * *").is_ok());
    }
}
