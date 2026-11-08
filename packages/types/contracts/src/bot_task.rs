//! Bot tasks: work a bot schedules for itself, once or on a cron schedule in a time zone. A
//! host fires a due task by writing a `trigger` entry (source `schedule`) into the bot's
//! conversation with its owner and moving `next_run_at` on; a task that was missed while its host
//! was away fires once when the host is back, never once per missed run.

use chrono::{DateTime, Duration, TimeZone, Utc};
use chrono_tz::Tz;
use cron::Schedule;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::{collections::BTreeSet, str::FromStr};

/// Tasks one bot keeps, enabled or not.
pub const MAX_TASKS_PER_BOT: usize = 50;
/// The shortest gap between two runs of a cron task.
pub const MIN_CRON_INTERVAL_SECS: i64 = 300;
/// Occurrences checked against [`MIN_CRON_INTERVAL_SECS`] when a cron task is saved.
const INTERVAL_PROBES: usize = 24;
pub const MAX_TITLE_CHARS: usize = 200;
pub const MAX_INSTRUCTION_CHARS: usize = 4_000;
pub const STORED_ONCE: &str = "ONCE";
pub const STORED_CRON: &str = "CRON";
pub const STORED_BOT: &str = "BOT";
pub const STORED_OWNER: &str = "OWNER";
pub const DEFAULT_TIMEZONE: &str = "UTC";

macro_rules! task_enum {
    ($(#[$meta:meta])* $name:ident { $($variant:ident => $id:literal),+ $(,)? }) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
        #[serde(rename_all = "snake_case")]
        pub enum $name {
            $($variant),+
        }

        impl $name {
            pub fn as_str(self) -> &'static str {
                match self {
                    $(Self::$variant => $id),+
                }
            }

            pub fn parse(value: &str) -> Option<Self> {
                let value = value.trim();
                [$(Self::$variant),+]
                    .into_iter()
                    .find(|candidate| candidate.as_str().eq_ignore_ascii_case(value))
            }

            pub fn stored(self) -> String {
                self.as_str().to_ascii_uppercase()
            }
        }
    };
}

task_enum!(
    /// When a task runs.
    TaskKind {
        Once => "once",
        Cron => "cron",
    }
);

task_enum!(
    /// Who made the task.
    TaskCreator {
        Bot => "bot",
        Owner => "owner",
    }
);

/// A scheduled task as the API and the local store show it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct BotTask {
    pub id: String,
    pub bot_id: String,
    /// The bot's owner; the API leaves it out of its task views.
    #[serde(default)]
    pub owner_id: String,
    pub kind: TaskKind,
    /// RFC 3339 time of a once task.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub at: Option<String>,
    /// Five-field cron expression of a cron task.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cron: Option<String>,
    /// IANA time zone the cron expression runs in.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tz: Option<String>,
    pub instruction: String,
    pub title: String,
    /// When the task fires next; none once a once task ran or while it is paused.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_run_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_run_at: Option<String>,
    pub enabled: bool,
    /// Check for useful changes; routine successful runs may stay quiet.
    #[serde(default)]
    pub monitoring: bool,
    pub created_by: TaskCreator,
    pub created_at: String,
}

/// Creates a task. A once task needs `at`, a cron task `cron` (and optionally `tz`, UTC when
/// missing).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct CreateBotTask {
    pub kind: Option<TaskKind>,
    #[serde(default)]
    pub at: Option<String>,
    #[serde(default)]
    pub cron: Option<String>,
    #[serde(default)]
    pub tz: Option<String>,
    #[serde(default)]
    pub instruction: String,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub enabled: Option<bool>,
    #[serde(default)]
    pub monitoring: bool,
}

/// Changes a task; what is missing stays. Moving the schedule re-arms the task.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct UpdateBotTask {
    #[serde(default)]
    pub at: Option<String>,
    #[serde(default)]
    pub cron: Option<String>,
    #[serde(default)]
    pub tz: Option<String>,
    #[serde(default)]
    pub instruction: Option<String>,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub enabled: Option<bool>,
    #[serde(default)]
    pub monitoring: Option<bool>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct BotTaskList {
    pub tasks: Vec<BotTask>,
}

/// The id of the `trigger` entry a task's run writes: the same task and run time always give
/// the same id, so a run that is written twice lands once.
pub fn fire_message_id(task_id: &str, run_at: DateTime<Utc>) -> String {
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"flow-like.bot-task.fire/v1\0");
    hasher.update(&(task_id.len() as u64).to_be_bytes());
    hasher.update(task_id.as_bytes());
    hasher.update(&run_at.timestamp_micros().to_be_bytes());
    let hex = hasher.finalize().to_hex();
    format!("tk-{}", &hex[..32])
}

/// When a task runs, checked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TaskSchedule {
    Once { at: DateTime<Utc> },
    Cron { expr: String, tz: String },
}

const WEEKDAYS: [&str; 7] = ["SUN", "MON", "TUE", "WED", "THU", "FRI", "SAT"];

fn weekday_number(token: &str) -> Result<u8, String> {
    let token = token.trim();
    if let Some(index) = WEEKDAYS
        .iter()
        .position(|name| name.eq_ignore_ascii_case(token))
    {
        return Ok(index as u8);
    }
    match token.parse::<u8>() {
        Ok(7) => Ok(0),
        Ok(day) if day <= 6 => Ok(day),
        _ => Err(format!(
            "'{token}' is not a day of the week; use 0-6 (0 = Sunday) or SUN-SAT"
        )),
    }
}

fn weekdays_of(token: &str) -> Result<BTreeSet<u8>, String> {
    let (base, step) = match token.split_once('/') {
        Some((base, step)) => {
            let step: u8 = step
                .parse()
                .ok()
                .filter(|step| *step > 0)
                .ok_or_else(|| format!("'{token}' has no valid step"))?;
            (base, step)
        }
        None => (token, 1),
    };
    let (first, last) = if base == "*" {
        (0, 6)
    } else if let Some((first, last)) = base.split_once('-') {
        let (first, last) = (weekday_number(first)?, weekday_number(last)?);
        let last = if last == 0 && first > 0 { 7 } else { last };
        (first, last)
    } else if token.contains('/') {
        (weekday_number(base)?, 6)
    } else {
        let day = weekday_number(base)?;
        return Ok(BTreeSet::from([day]));
    };
    if first > last {
        return Err(format!("'{token}' runs backwards"));
    }
    Ok((first..=last)
        .step_by(step as usize)
        .map(|day| day % 7)
        .collect())
}

/// The day-of-week field with the numbers of standard crontab (0 and 7 are Sunday) as day
/// names, which the cron library reads the same everywhere.
fn day_names(field: &str) -> Result<String, String> {
    if matches!(field, "*" | "?") {
        return Ok("*".to_string());
    }
    let mut days = BTreeSet::new();
    for token in field.split(',') {
        days.extend(weekdays_of(token)?);
    }
    Ok(days
        .into_iter()
        .map(|day| WEEKDAYS[day as usize])
        .collect::<Vec<_>>()
        .join(","))
}

fn parse_schedule(expr: &str) -> Result<Schedule, String> {
    let fields: Vec<&str> = expr.split_whitespace().collect();
    let [minute, hour, day, month, weekday] = fields[..] else {
        return Err(format!(
            "cron '{}' needs 5 fields: minute hour day-of-month month day-of-week",
            expr.trim()
        ));
    };
    let normalized = format!("0 {minute} {hour} {day} {month} {}", day_names(weekday)?);
    Schedule::from_str(&normalized).map_err(|error| format!("cron '{}': {error}", expr.trim()))
}

fn parse_tz(tz: &str) -> Result<Tz, String> {
    tz.trim()
        .parse::<Tz>()
        .map_err(|_| format!("'{tz}' is not an IANA time zone such as Europe/Berlin"))
}

fn next_cron(expr: &str, tz: &str, after: DateTime<Utc>) -> Result<Option<DateTime<Utc>>, String> {
    let (schedule, tz) = (parse_schedule(expr)?, parse_tz(tz)?);
    Ok(schedule
        .after(&tz.from_utc_datetime(&after.naive_utc()))
        .next()
        .map(|next| next.with_timezone(&Utc)))
}

fn check_gaps(expr: &str, tz: &str, from: DateTime<Utc>) -> Result<(), String> {
    let mut previous = from;
    for probe in 0..INTERVAL_PROBES {
        let Some(next) = next_cron(expr, tz, previous)? else {
            return if probe == 0 {
                Err(format!("cron '{}' never runs", expr.trim()))
            } else {
                Ok(())
            };
        };
        if probe > 0 && next - previous < Duration::seconds(MIN_CRON_INTERVAL_SECS) {
            return Err(format!(
                "cron '{}' runs more often than every {} minutes",
                expr.trim(),
                MIN_CRON_INTERVAL_SECS / 60
            ));
        }
        previous = next;
    }
    Ok(())
}

impl TaskSchedule {
    /// The schedule of a task as saved: a once task needs a future `at`, a cron task a valid
    /// expression no tighter than [`MIN_CRON_INTERVAL_SECS`] in a known time zone.
    pub fn parse(
        kind: TaskKind,
        at: Option<&str>,
        cron: Option<&str>,
        tz: Option<&str>,
        now: DateTime<Utc>,
    ) -> Result<Self, String> {
        match kind {
            TaskKind::Once => {
                let text = at.ok_or("a once task needs 'at', an RFC 3339 time")?;
                let at = DateTime::parse_from_rfc3339(text.trim())
                    .map_err(|_| format!("'at' value '{text}' is not an RFC 3339 time"))?
                    .with_timezone(&Utc);
                if at <= now {
                    return Err(format!("'at' value '{text}' is not in the future"));
                }
                Ok(Self::Once { at })
            }
            TaskKind::Cron => {
                let expr = cron.ok_or("a cron task needs 'cron', a 5-field expression")?;
                let tz = tz
                    .map(str::trim)
                    .filter(|tz| !tz.is_empty())
                    .unwrap_or(DEFAULT_TIMEZONE);
                parse_tz(tz)?;
                check_gaps(expr, tz, now)?;
                Ok(Self::Cron {
                    expr: expr.split_whitespace().collect::<Vec<_>>().join(" "),
                    tz: tz.to_string(),
                })
            }
        }
    }

    pub fn kind(&self) -> TaskKind {
        match self {
            Self::Once { .. } => TaskKind::Once,
            Self::Cron { .. } => TaskKind::Cron,
        }
    }

    /// When a task with this schedule runs first.
    pub fn first_run(&self, now: DateTime<Utc>) -> Option<DateTime<Utc>> {
        match self {
            Self::Once { at } => Some(*at),
            Self::Cron { expr, tz } => next_cron(expr, tz, now).ok().flatten(),
        }
    }

    /// When the task runs after it fired at `now`: the first cron time after `now`, so any
    /// number of missed runs collapse into the one that fired; none for a once task.
    pub fn after_fire(&self, now: DateTime<Utc>) -> Option<DateTime<Utc>> {
        match self {
            Self::Once { .. } => None,
            Self::Cron { expr, tz } => next_cron(expr, tz, now).ok().flatten(),
        }
    }
}

/// The text fields of a task, trimmed and within their limits.
pub fn check_text(title: Option<&str>, instruction: &str) -> Result<(String, String), String> {
    let instruction = instruction.trim();
    if instruction.is_empty() {
        return Err("a task needs an instruction".to_string());
    }
    let chars = instruction.chars().count();
    if chars > MAX_INSTRUCTION_CHARS {
        return Err(format!(
            "an instruction has at most {MAX_INSTRUCTION_CHARS} characters; this one has {chars}"
        ));
    }
    let title = title
        .map(str::trim)
        .filter(|title| !title.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| instruction.chars().take(60).collect());
    let chars = title.chars().count();
    if chars > MAX_TITLE_CHARS {
        return Err(format!(
            "a title has at most {MAX_TITLE_CHARS} characters; this one has {chars}"
        ));
    }
    Ok((title, instruction.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn utc(text: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(text)
            .unwrap()
            .with_timezone(&Utc)
    }

    fn cron(expr: &str, tz: &str) -> TaskSchedule {
        TaskSchedule::parse(
            TaskKind::Cron,
            None,
            Some(expr),
            Some(tz),
            utc("2026-01-01T00:00:00Z"),
        )
        .unwrap()
    }

    #[test]
    fn numeric_weekdays_follow_standard_crontab() {
        let monday = cron("0 9 * * 1", "UTC");
        assert_eq!(
            monday.first_run(utc("2026-11-03T00:00:00Z")),
            Some(utc("2026-11-09T09:00:00Z"))
        );
        let sunday_zero = cron("0 9 * * 0", "UTC");
        let sunday_seven = cron("0 9 * * 7", "UTC");
        let from = utc("2026-11-03T00:00:00Z");
        assert_eq!(
            sunday_zero.first_run(from),
            Some(utc("2026-11-08T09:00:00Z"))
        );
        assert_eq!(sunday_seven.first_run(from), sunday_zero.first_run(from));
        let weekdays = cron("30 8 * * 1-5", "UTC");
        assert_eq!(
            weekdays.first_run(utc("2026-11-06T09:00:00Z")),
            Some(utc("2026-11-09T08:30:00Z"))
        );
        assert_eq!(
            cron("0 9 * * Mon,Wed", "UTC").first_run(from),
            Some(utc("2026-11-04T09:00:00Z"))
        );
        assert_eq!(
            cron("0 9 * * 5-7", "UTC").first_run(from),
            Some(utc("2026-11-06T09:00:00Z"))
        );
    }

    #[test]
    fn a_cron_runs_in_its_time_zone() {
        let berlin = cron("0 9 * * *", "Europe/Berlin");
        assert_eq!(
            berlin.first_run(utc("2026-01-10T00:00:00Z")),
            Some(utc("2026-01-10T08:00:00Z"))
        );
        assert_eq!(
            berlin.first_run(utc("2026-07-10T00:00:00Z")),
            Some(utc("2026-07-10T07:00:00Z"))
        );
    }

    #[test]
    fn dst_keeps_the_local_time_across_the_switch() {
        let ny = cron("0 9 * * *", "America/New_York");
        let before = ny.first_run(utc("2026-03-07T15:00:00Z")).unwrap();
        assert_eq!(before, utc("2026-03-08T13:00:00Z"));
        let after = ny.after_fire(before).unwrap();
        assert_eq!(after, utc("2026-03-09T13:00:00Z"));
        assert_eq!(
            before
                .signed_duration_since(utc("2026-03-07T14:00:00Z"))
                .num_hours(),
            23
        );
    }

    #[test]
    fn a_local_time_that_does_not_exist_is_skipped_for_that_day() {
        let ny = cron("30 2 * * *", "America/New_York");
        let first = ny.first_run(utc("2026-03-08T00:00:00Z")).unwrap();
        assert_eq!(first, utc("2026-03-09T06:30:00Z"));
    }

    #[test]
    fn a_local_time_that_happens_twice_runs_once() {
        let ny = cron("30 1 * * *", "America/New_York");
        let first = ny.first_run(utc("2026-11-01T00:00:00Z")).unwrap();
        let second = ny.after_fire(first).unwrap();
        assert_eq!(second.signed_duration_since(first).num_hours(), 25);
        assert_eq!(
            second.date_naive(),
            utc("2026-11-02T00:00:00Z").date_naive()
        );
    }

    #[test]
    fn missed_runs_collapse_into_the_next_future_one() {
        let hourly = cron("0 * * * *", "UTC");
        let next = hourly.after_fire(utc("2026-11-03T12:30:00Z"));
        assert_eq!(next, Some(utc("2026-11-03T13:00:00Z")));
        let once = TaskSchedule::parse(
            TaskKind::Once,
            Some("2026-11-04T09:00:00Z"),
            None,
            None,
            utc("2026-11-03T00:00:00Z"),
        )
        .unwrap();
        assert_eq!(
            once.first_run(utc("2026-11-03T00:00:00Z")),
            Some(utc("2026-11-04T09:00:00Z"))
        );
        assert_eq!(once.after_fire(utc("2026-11-05T00:00:00Z")), None);
    }

    #[test]
    fn schedules_are_checked_when_saved() {
        let now = utc("2026-11-03T00:00:00Z");
        let parse = |kind, at, cron, tz| TaskSchedule::parse(kind, at, cron, tz, now);
        assert!(parse(TaskKind::Once, Some("2026-11-02T09:00:00Z"), None, None).is_err());
        assert!(parse(TaskKind::Once, Some("tomorrow"), None, None).is_err());
        assert!(parse(TaskKind::Once, None, None, None).is_err());
        assert!(parse(TaskKind::Cron, None, Some("* * * * *"), None).is_err());
        assert!(parse(TaskKind::Cron, None, Some("*/4 * * * *"), None).is_err());
        assert!(parse(TaskKind::Cron, None, Some("0,2 9 * * *"), None).is_err());
        assert!(parse(TaskKind::Cron, None, Some("*/5 * * * *"), None).is_ok());
        assert!(parse(TaskKind::Cron, None, Some("0 9 * *"), None).is_err());
        assert!(parse(TaskKind::Cron, None, Some("0 9 * * *"), Some("Mars/Base")).is_err());
        assert!(parse(TaskKind::Cron, None, Some("0 9 31 2 *"), None).is_err());
        assert!(parse(TaskKind::Cron, None, Some("0 9 * * 9"), None).is_err());
    }

    #[test]
    fn the_fire_id_is_stable_and_short() {
        let at = utc("2026-11-03T09:00:00Z");
        let id = fire_message_id("task-1", at);
        assert_eq!(id, fire_message_id("task-1", at));
        assert_ne!(id, fire_message_id("task-2", at));
        assert_ne!(id, fire_message_id("task-1", utc("2026-11-03T09:05:00Z")));
        assert!(id.starts_with("tk-") && id.len() == 35);
    }

    #[test]
    fn text_is_trimmed_and_titled_from_the_instruction() {
        assert_eq!(
            check_text(None, "  Check the report ").unwrap(),
            (
                "Check the report".to_string(),
                "Check the report".to_string()
            )
        );
        assert!(check_text(None, "  ").is_err());
        assert!(check_text(Some("x".repeat(201).as_str()), "go").is_err());
    }
}
