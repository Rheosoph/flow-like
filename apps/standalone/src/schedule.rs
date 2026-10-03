//! Schedules (`cron` events without a default Page), repeating or one-time, run inside the
//! placement's own process. A scheduled time runs at most once: its watermark or its
//! one-time record is durable before the run starts, nothing runs at start, and nothing is
//! armed before the service is ready. The same pass claims the process's bots at the hub.

use anyhow::{Context, Result, ensure};
use async_trait::async_trait;
use chrono::{DateTime, NaiveDate, NaiveDateTime, NaiveTime, TimeZone, Utc};
use flow_like_runtime::{
    app::AppVisibility, flow::event::Event, profile::Profile, state::FlowLikeState,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::{BTreeMap, HashMap},
    io::Write,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};
use tokio::{
    sync::{oneshot, watch},
    task::{JoinError, JoinSet},
    time::Instant,
};
use tokio_util::sync::CancellationToken;

use crate::hosting::PreparedInvocation;

const STATE_DIRECTORY: &str = ".standalone-schedule";
const STATE_FILE: &str = "state.json";
const STATE_VERSION: u64 = 1;
const MAX_STATE_BYTES: usize = 64 * 1024;
const MAX_EVENTS: usize = 64;
const MAX_EXPRESSION: usize = 128;
const MIN_POLL: Duration = Duration::from_millis(250);
const MAX_POLL: Duration = Duration::from_secs(10);
/// A scheduled time found later than this is missed, not run late.
const LATE_LIMIT_SECS: i64 = 60;
const RUN_LIMIT: Duration = Duration::from_secs(24 * 60 * 60);
/// A run's own time limit lies this far behind the scheduler's, which records the time-out.
const RUN_BACKSTOP: Duration = Duration::from_secs(5 * 60);
const MAX_CONCURRENT_RUNS: usize = 8;
const STOP_GRACE: Duration = Duration::from_secs(12);
/// A cancelled run that does not end by itself is aborted after this long.
const CANCEL_GRACE: Duration = Duration::from_secs(2);
/// A scheduled time waits while the parent or the hub is asked, so both answers are bounded
/// well below the late limit.
const PARENT_TIMEOUT: Duration = Duration::from_secs(10);
const HUB_TIMEOUT: Duration = Duration::from_secs(20);
const PARENT_RETRY: Duration = Duration::from_secs(5);
const UNREACHABLE_RETRY: Duration = Duration::from_secs(60);
const HELD_RETRY: Duration = Duration::from_secs(300);
const HUB_TOO_OLD_RETRY: Duration = Duration::from_secs(600);
const CONFIRMATION: Duration = Duration::from_secs(1800);
/// A watermark this far ahead of the clock was written under a wrong clock.
const WATERMARK_AHEAD_LIMIT_SECS: i64 = 24 * 60 * 60;
/// One match per minute at most, so this covers every repeated hour of a time zone.
const REPEATED_HOUR_STEPS: usize = 1500;
/// A one-time schedule this service had armed before its time still runs this late.
const ONCE_LATE_LIMIT_SECS: i64 = 900;
/// 2000-01-01 and 2100-01-01, UTC: the instants a one-time schedule may name.
const ONCE_EARLIEST: i64 = 946_684_800;
const ONCE_LATEST: i64 = 4_102_444_800;
/// Finished one-time records of events that left the config, oldest dropped first.
const MAX_ONCE_DONE: usize = 64;

const EXPRESSION_KEYS: [&str; 5] = [
    "expression",
    "cron_expression",
    "cronExpression",
    "cron",
    "schedule",
];
const TIMEZONE_KEYS: [&str; 4] = ["timezone", "tz", "cron_timezone", "cronTimezone"];
const ONE_TIME_KEYS: [&str; 2] = ["scheduled_for", "scheduledFor"];
const FIELDS: [&str; 6] = [
    "seconds",
    "minute",
    "hour",
    "day-of-month",
    "month",
    "weekday",
];
const MONTHS: [&str; 12] = [
    "JAN", "FEB", "MAR", "APR", "MAY", "JUN", "JUL", "AUG", "SEP", "OCT", "NOV", "DEC",
];
const WEEKDAYS: [&str; 7] = ["SUN", "MON", "TUE", "WED", "THU", "FRI", "SAT"];

/// Why a device does not run a `cron` event's schedule. The codes are shared with clients.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ScheduleProblem {
    Missing,
    Once,
    Invalid(String),
    TooOften(String),
}

impl ScheduleProblem {
    pub(crate) fn code(&self) -> &'static str {
        match self {
            Self::Missing => "schedule_missing",
            Self::Once => "schedule_once",
            Self::Invalid(_) => "schedule_invalid",
            Self::TooOften(_) => "schedule_too_often",
        }
    }
}

impl std::fmt::Display for ScheduleProblem {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Missing => formatter.write_str("The schedule has no cron expression"),
            Self::Once => formatter.write_str(
                "The schedule runs once at a set date and time; a device runs repeating schedules only",
            ),
            Self::Invalid(sentence) | Self::TooOften(sentence) => formatter.write_str(sentence),
        }
    }
}

impl std::error::Error for ScheduleProblem {}

fn shortened(text: &str) -> &str {
    let mut end = text.len().min(64);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}

/// The text under the first of `keys` that holds any, trimmed. Blank text is not a value.
fn first_text<'a>(config: &'a Value, keys: &[&str]) -> Option<&'a str> {
    keys.iter()
        .find_map(|key| {
            config
                .get(*key)
                .and_then(Value::as_str)
                .filter(|value| !value.is_empty())
        })
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

/// What a schedule saves: a cron expression, or a one-time date and time.
enum Saved<'a> {
    Repeating(&'a str),
    Once { date: &'a str, time: &'a str },
}

/// The one schedule a config saves; both kinds at once is invalid.
fn saved_schedule(config: &Value) -> Result<Saved<'_>, ScheduleProblem> {
    let one_time = ONE_TIME_KEYS
        .iter()
        .find_map(|key| config.get(*key).filter(|value| !value.is_null()))
        .and_then(|value| Some((value.get("date")?.as_str()?, value.get("time")?.as_str()?)));
    match (first_text(config, &EXPRESSION_KEYS), one_time) {
        (Some(_), Some(_)) => Err(ScheduleProblem::Invalid(
            "The schedule sets a cron expression and a one-time date; it needs exactly one".into(),
        )),
        (None, Some((date, time))) => Ok(Saved::Once { date, time }),
        (None, None) => Err(ScheduleProblem::Missing),
        (Some(expression), None) => Ok(Saved::Repeating(expression)),
    }
}

/// The time zone a schedule saves, UTC when it saves none.
fn saved_zone(config: &Value) -> Result<chrono_tz::Tz, ScheduleProblem> {
    let Some(name) = first_text(config, &TIMEZONE_KEYS) else {
        return Ok(chrono_tz::UTC);
    };
    name.parse().map_err(|_| {
        ScheduleProblem::Invalid(format!(
            "The time zone {:?} is not a known IANA name such as Europe/Berlin",
            shortened(name)
        ))
    })
}

/// `text` is `pattern` with `9` standing for one ASCII digit and every other byte for itself.
fn shaped(text: &str, pattern: &str) -> bool {
    text.len() == pattern.len()
        && text
            .bytes()
            .zip(pattern.bytes())
            .all(|(byte, expected)| match expected {
                b'9' => byte.is_ascii_digit(),
                _ => byte == expected,
            })
}

/// A one-time date and time as the editor writes them, checked for their shape before
/// they are read: a lenient reader takes `9:30` or rolls `2026-02-30` over.
fn once_parts(date: &str, time: &str) -> Result<(NaiveDate, NaiveTime), ScheduleProblem> {
    let day = shaped(date, "9999-99-99")
        .then(|| NaiveDate::parse_from_str(date, "%Y-%m-%d").ok())
        .flatten()
        .ok_or_else(|| {
            ScheduleProblem::Invalid(format!(
                "The one-time date {:?} is not a calendar date written as YYYY-MM-DD",
                shortened(date)
            ))
        })?;
    let clock = shaped(time, "99:99")
        .then(|| NaiveTime::parse_from_str(time, "%H:%M").ok())
        .flatten()
        .ok_or_else(|| {
            ScheduleProblem::Invalid(format!(
                "The one-time time {:?} is not a time of day written as HH:MM, from 00:00 to 23:59",
                shortened(time)
            ))
        })?;
    Ok((day, clock))
}

/// The instant of a local date and time in `zone`: the earlier one when clocks go back, none
/// when clocks go forward over it. It is never compared with a clock.
fn once_instant(
    day: NaiveDate,
    clock: NaiveTime,
    zone: chrono_tz::Tz,
) -> Result<i64, ScheduleProblem> {
    let local = NaiveDateTime::new(day, clock);
    let at = zone
        .from_local_datetime(&local)
        .earliest()
        .ok_or_else(|| {
            ScheduleProblem::Invalid(format!(
                "The one-time time {} on {} does not exist in {}: the clocks are put forward over it",
                clock.format("%H:%M"),
                day,
                zone.name()
            ))
        })?
        .timestamp();
    if !(ONCE_EARLIEST..=ONCE_LATEST).contains(&at) {
        return Err(ScheduleProblem::Invalid(format!(
            "The one-time date {day} lies outside the years 2000 to 2100"
        )));
    }
    Ok(at)
}

/// Whether a device reads `field`: numbers, `*`, lists, ranges and steps, and `names`.
fn readable_field(field: &str, names: &[&str]) -> bool {
    field.split([',', '-', '/']).all(|part| {
        part == "*"
            || (!part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit()))
            || names.iter().any(|name| name.eq_ignore_ascii_case(part))
    })
}

/// `expression` single-spaced with its names in upper case, when it only uses syntax every
/// reader of the documented dialect reads the same way and cannot run more often than once
/// a minute.
fn readable_expression(expression: &str) -> Result<String, ScheduleProblem> {
    let invalid = |sentence: String| Err(ScheduleProblem::Invalid(sentence));
    let length = expression.chars().count();
    if length > MAX_EXPRESSION {
        return invalid(format!(
            "The cron expression is {length} characters long; a device reads at most {MAX_EXPRESSION}"
        ));
    }
    let quoted = format!("{:?}", shortened(expression));
    let fields: Vec<&str> = expression.split_whitespace().collect();
    if !(5..=6).contains(&fields.len()) {
        return invalid(format!(
            "The cron expression {quoted} has {} fields; a device reads 5, or 6 with leading seconds",
            fields.len()
        ));
    }
    let names = FIELDS[FIELDS.len() - fields.len()..].iter();
    for (field, name) in fields.iter().zip(names) {
        let words: &[&str] = match *name {
            "month" => &MONTHS,
            "weekday" => &WEEKDAYS,
            _ => &[],
        };
        if !readable_field(field, words) {
            return invalid(format!(
                "The {name} field {:?} of the cron expression {quoted} is not readable on a device: a field is built from numbers, *, lists, ranges and steps, with three-letter names only for months and weekdays",
                shortened(field)
            ));
        }
    }
    if fields.len() == 6 && !fields[0].bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(ScheduleProblem::TooOften(format!(
            "The cron expression {quoted} runs more often than once a minute: its seconds field {:?} must be one number from 0 to 59",
            shortened(fields[0])
        )));
    }
    Ok(fields.join(" ").to_ascii_uppercase())
}

#[derive(Clone, Debug)]
enum Timing {
    Repeating {
        expression: String,
        cron: croner::Cron,
    },
    Once {
        date: String,
        time: String,
        at: i64,
    },
}

/// When a one-time schedule runs, as discovery reports it:
/// `{"date":"2026-09-24","time":"09:00","at":1790233200,"timezone":"Europe/Berlin"}`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub(crate) struct OnceAt {
    pub(crate) date: String,
    pub(crate) time: String,
    pub(crate) at: i64,
    pub(crate) timezone: &'static str,
}

/// A schedule a device can run, read in the event's time zone (UTC when it saves none):
/// a repeating one in the documented cron dialect, never more often than once a minute,
/// or a one-time local date and time.
#[derive(Clone, Debug)]
pub(crate) struct ScheduleSpec {
    timing: Timing,
    timezone: chrono_tz::Tz,
    payload: Option<Value>,
    /// The pinned event version the schedule was read from.
    event_version: (u32, u32, u32),
}

impl ScheduleSpec {
    pub(crate) fn from_event(event: &Event) -> Result<Self, ScheduleProblem> {
        let spec = Self::parse(&event.config, Utc::now())?;
        Ok(Self {
            event_version: event.event_version,
            ..spec
        })
    }

    /// A build without the one-time part answers a one-time schedule with `schedule_once`.
    fn parse(config: &[u8], now: DateTime<Utc>) -> Result<Self, ScheduleProblem> {
        Self::parse_with(config, now, crate::event_kind::SCHEDULED_ONCE)
    }

    fn parse_with(config: &[u8], now: DateTime<Utc>, once: bool) -> Result<Self, ScheduleProblem> {
        let config: Value = serde_json::from_slice(config).unwrap_or(Value::Null);
        let timing = match saved_schedule(&config)? {
            Saved::Repeating(expression) => Self::repeating(expression, now)?,
            Saved::Once { .. } if !once => return Err(ScheduleProblem::Once),
            Saved::Once { date, time } => Self::once_at(date, time, &config)?,
        };
        Ok(Self {
            timing,
            timezone: saved_zone(&config)?,
            payload: config
                .get("payload")
                .filter(|payload| payload.is_object())
                .cloned(),
            event_version: (0, 0, 0),
        })
    }

    /// The shape of date and time first, then the zone, then the instant.
    fn once_at(date: &str, time: &str, config: &Value) -> Result<Timing, ScheduleProblem> {
        let (day, clock) = once_parts(date, time)?;
        Ok(Timing::Once {
            date: date.to_owned(),
            time: time.to_owned(),
            at: once_instant(day, clock, saved_zone(config)?)?,
        })
    }

    /// A readable expression that matches a date; the zone is checked after it.
    fn repeating(expression: &str, now: DateTime<Utc>) -> Result<Timing, ScheduleProblem> {
        let invalid = ScheduleProblem::Invalid;
        let expression = readable_expression(expression)?;
        let quoted = format!("{:?}", shortened(&expression));
        let cron = croner::Cron::new(&expression)
            .with_seconds_optional()
            .parse()
            .map_err(|error| {
                invalid(format!(
                    "The cron expression {quoted} cannot be read: {error}"
                ))
            })?;
        let utc = Self {
            timing: Timing::Repeating { expression, cron },
            timezone: chrono_tz::UTC,
            payload: None,
            event_version: (0, 0, 0),
        };
        if utc.next_after(now).is_none() {
            return Err(invalid(format!(
                "The cron expression {quoted} never matches a date"
            )));
        }
        Ok(utc.timing)
    }

    /// Single-spaced, names in upper case; empty for a one-time schedule.
    pub(crate) fn expression(&self) -> &str {
        match &self.timing {
            Timing::Repeating { expression, .. } => expression,
            Timing::Once { .. } => "",
        }
    }

    pub(crate) fn timezone(&self) -> &'static str {
        self.timezone.name()
    }

    /// The date, time, instant and zone of a one-time schedule; `None` for a repeating one.
    pub(crate) fn once(&self) -> Option<OnceAt> {
        match &self.timing {
            Timing::Once { date, time, at } => Some(OnceAt {
                date: date.clone(),
                time: time.clone(),
                at: *at,
                timezone: self.timezone(),
            }),
            Timing::Repeating { .. } => None,
        }
    }

    /// The first scheduled time strictly after `instant`, to the second.
    pub(crate) fn next_after(&self, instant: DateTime<Utc>) -> Option<DateTime<Utc>> {
        let instant = DateTime::from_timestamp(instant.timestamp(), 0)?;
        let cron = match &self.timing {
            Timing::Repeating { cron, .. } => cron,
            Timing::Once { at, .. } => {
                return DateTime::from_timestamp(*at, 0).filter(|once| *once > instant);
            }
        };
        let mut from = instant.with_timezone(&self.timezone);
        // In the hour that repeats when clocks go back, the parser answers with the first
        // pass of a local time, which can lie before `instant`. Asking again from its
        // answer moves one match on, until the repeated hour is over.
        for _ in 0..REPEATED_HOUR_STEPS {
            let next = cron.find_next_occurrence(&from, false).ok()?;
            if next.timestamp() > instant.timestamp() {
                return Some(next.with_timezone(&Utc));
            }
            from = next;
        }
        None
    }

    fn next_secs(&self, after: i64) -> Option<i64> {
        Some(
            self.next_after(DateTime::from_timestamp(after, 0)?)?
                .timestamp(),
        )
    }
}

/// Why a schedule of a running service does not fire.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum HoldReason {
    OtherService,
    NotReleased,
    RunsElsewhere,
    HubUnreachable,
    HubTooOld,
}

impl HoldReason {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::OtherService => "other_service",
            Self::NotReleased => "not_released",
            Self::RunsElsewhere => "runs_elsewhere",
            Self::HubUnreachable => "hub_unreachable",
            Self::HubTooOld => "hub_too_old",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Outcome {
    Succeeded,
    Failed,
    Cancelled,
    TimedOut,
}

impl Outcome {
    fn as_str(self) -> &'static str {
        match self {
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
            Self::TimedOut => "timed_out",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum SkipReason {
    Overlap,
    Missed,
    Busy,
}

impl SkipReason {
    fn as_str(self) -> &'static str {
        match self {
            Self::Overlap => "overlap",
            Self::Missed => "missed",
            Self::Busy => "busy",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct LastRun {
    at: i64,
    finished_at: i64,
    outcome: Outcome,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct LastSkip {
    at: i64,
    reason: SkipReason,
}

/// One schedule's entry of the state file. Times are Unix seconds; no error text, payload
/// or variable value is ever written.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
struct EventState {
    expression: String,
    timezone: String,
    armed_at: Option<i64>,
    /// The scheduled time last started or skipped. The device never runs it again.
    watermark: Option<i64>,
    next_at: Option<i64>,
    hold: Option<HoldReason>,
    /// The hub confirmed the claim for the config's approval, and since when the approval has it.
    confirmed: bool,
    since: Option<i64>,
    running_since: Option<i64>,
    last: Option<LastRun>,
    runs: u64,
    failed: u64,
    skipped_overlap: u64,
    skipped_missed: u64,
    skipped_busy: u64,
    last_skip: Option<LastSkip>,
}

impl EventState {
    /// No scheduled time at or before this one runs: not what the clock has passed, not
    /// what this device ran, not what the hub may have run before it handed over.
    fn floor(&self, now: i64) -> i64 {
        now.max(self.watermark.unwrap_or(i64::MIN))
            .max(self.since.unwrap_or(i64::MIN))
    }

    /// The scheduled time that has arrived, already recorded as the watermark, and whether
    /// the entry changed. `clock_went_back` asks for the next time again: without a
    /// watermark it lies earlier now.
    fn arrived(
        &mut self,
        id: &str,
        spec: &ScheduleSpec,
        now: i64,
        clock_went_back: bool,
    ) -> (Option<i64>, bool) {
        let ahead = now.saturating_add(WATERMARK_AHEAD_LIMIT_SECS);
        let dropped = self.watermark.is_some_and(|watermark| watermark > ahead);
        if dropped {
            tracing::warn!(
                event_id = %id,
                "The last scheduled time this device ran lies more than 24 hours ahead of its clock, so the earlier clock was wrong; the schedule runs from the current clock again"
            );
            self.watermark = None;
        }
        if self.hold.is_some() {
            return (None, dropped);
        }
        match self.next_at {
            Some(next) if next <= now => {
                self.watermark = Some(self.watermark.map_or(next, |known| known.max(next)));
                self.next_at = spec.next_secs(self.floor(now));
                (Some(next), true)
            }
            Some(_) if !(dropped || clock_went_back) => (None, false),
            previous => {
                self.next_at = spec.next_secs(self.floor(now));
                (None, dropped || self.next_at != previous)
            }
        }
    }

    fn skipped(&mut self, id: &str, scheduled_at: i64, reason: SkipReason) {
        let counter = match reason {
            SkipReason::Overlap => &mut self.skipped_overlap,
            SkipReason::Missed => &mut self.skipped_missed,
            SkipReason::Busy => &mut self.skipped_busy,
        };
        *counter = counter.saturating_add(1);
        self.last_skip = Some(LastSkip {
            at: scheduled_at,
            reason,
        });
        tracing::info!(
            event_id = %id,
            scheduled_at,
            reason = reason.as_str(),
            "Scheduled run skipped"
        );
    }

    /// `claimed`: the hub just confirmed the schedule, with the time this approval got it.
    fn arm(&mut self, id: &str, spec: &ScheduleSpec, now: i64, armed: bool, claimed: Option<i64>) {
        if let Some(since) = claimed {
            self.confirmed = true;
            self.since = Some(since);
        }
        let behind = self
            .next_at
            .is_none_or(|next| self.since.is_some_and(|since| next <= since));
        self.hold = None;
        if !armed {
            self.armed_at.get_or_insert(now);
            tracing::info!(event_id = %id, "Schedule is armed");
        }
        if !armed || behind {
            self.next_at = spec.next_secs(self.floor(now));
        }
    }

    /// `answered`: the hub answered, so what it confirmed before no longer counts.
    fn held(&mut self, id: &str, reason: HoldReason, announce: bool, answered: bool) {
        if announce || self.hold != Some(reason) {
            tracing::info!(
                event_id = %id,
                reason = reason.as_str(),
                "Schedule is held and does not run"
            );
        }
        if answered {
            self.confirmed = false;
            self.since = None;
        }
        self.hold = Some(reason);
        self.next_at = None;
    }
}

/// What the hub's answer means for an event the parent lets this service run. `armed`: the
/// event runs now. Only an armed event survives a hub that cannot be reached; one that is
/// not armed yet runs without the hub only when the hub confirmed it for this approval.
fn decide(
    id: &str,
    hold: Option<HoldReason>,
    confirmed: bool,
    armed: bool,
    hub: &Option<Result<ClaimOutcome, ClaimError>>,
) -> Decision {
    match hub {
        None => Decision::Arm(None),
        Some(Ok(outcome)) => outcome.decision(id),
        Some(Err(ClaimError::Unreachable)) if armed => Decision::Keep,
        Some(Err(ClaimError::Unreachable)) if confirmed => Decision::Arm(None),
        Some(Err(ClaimError::Unreachable)) => Decision::Hold(match hold {
            Some(reason) if reason != HoldReason::OtherService => reason,
            _ => HoldReason::HubUnreachable,
        }),
        Some(Err(ClaimError::HubTooOld)) => Decision::Hold(HoldReason::HubTooOld),
        Some(Err(ClaimError::Denied)) => Decision::Hold(HoldReason::NotReleased),
    }
}

/// One pass's answers of the parent and the hub.
struct Answers<'a> {
    /// What another running service of this device runs.
    refused: &'a HashMap<String, String>,
    hub: &'a Option<Result<ClaimOutcome, ClaimError>>,
    /// The first pass after Ready: nothing was armed before it.
    first: bool,
    /// The hub answered, so what it confirmed before no longer counts.
    answered: bool,
    now: i64,
}

impl Answers<'_> {
    /// Whether an event ran before this pass, and what the answers mean for it.
    fn decision(&self, id: &str, hold: Option<HoldReason>, confirmed: bool) -> (bool, Decision) {
        let armed = !self.first && hold.is_none();
        let decision = if self.refused.contains_key(id) {
            Decision::Hold(HoldReason::OtherService)
        } else {
            decide(id, hold, confirmed, armed, self.hub)
        };
        (armed, decision)
    }

    fn apply_repeating(&self, id: &str, spec: &ScheduleSpec, entry: &mut EventState) {
        match self.decision(id, entry.hold, entry.confirmed) {
            (_, Decision::Keep) => {}
            (armed, Decision::Arm(claimed)) => entry.arm(id, spec, self.now, armed, claimed),
            (_, Decision::Hold(reason)) => entry.held(id, reason, self.first, self.answered),
        }
    }

    fn apply_once(&self, id: &str, entry: &mut OnceEntry) {
        match self.decision(id, entry.claim.hold, entry.claim.confirmed) {
            (_, Decision::Keep) => {}
            (armed, Decision::Arm(claimed)) => entry.arm(id, self.now, armed, claimed),
            (_, Decision::Hold(reason)) => entry.held(id, reason, self.first, self.answered),
        }
    }

    fn apply_gate(&self, id: &str, entry: &mut ClaimEntry) {
        match self.decision(id, entry.hold, entry.confirmed) {
            (_, Decision::Keep) => {}
            (armed, Decision::Arm(claimed)) => {
                if !armed {
                    tracing::info!(event_id = %id, "Claimed event may run");
                }
                entry.armed(claimed);
            }
            (_, Decision::Hold(reason)) => {
                if entry.held(reason, self.answered) || self.first {
                    tracing::info!(
                        event_id = %id,
                        reason = reason.as_str(),
                        "Claimed event is held and does not run"
                    );
                }
            }
        }
    }
}

/// Where a self-firing event that is not a schedule (a bot) may run: what the parent and the
/// hub answered in the scheduler's last pass.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Gate {
    /// Before Ready, and while the parent has not answered once.
    Undecided,
    /// `since`: the hub's clock when this approval got the event; `None` without a hub.
    Armed {
        since: Option<i64>,
    },
    Held(HoldReason),
}

/// The gates of a placement process's self-firing events that are not schedules. The
/// scheduler makes the one parent call and the one claim call of the process for them too,
/// so neither side hands back what the other runs.
#[derive(Clone, Default)]
pub(crate) struct ClaimGates {
    gates: Arc<BTreeMap<String, watch::Sender<Gate>>>,
}

impl ClaimGates {
    pub(crate) fn new(extra_ids: Vec<String>) -> Self {
        Self {
            gates: Arc::new(
                extra_ids
                    .into_iter()
                    .map(|id| (id, watch::channel(Gate::Undecided).0))
                    .collect(),
            ),
        }
    }

    /// The gate of `event_id`, or `None` when the scheduler does not decide it.
    #[cfg_attr(not(feature = "bots"), allow(dead_code))]
    pub(crate) fn watch(&self, event_id: &str) -> Option<watch::Receiver<Gate>> {
        self.gates.get(event_id).map(watch::Sender::subscribe)
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.gates.is_empty()
    }

    fn len(&self) -> usize {
        self.gates.len()
    }

    fn ids(&self) -> impl Iterator<Item = &String> {
        self.gates.keys()
    }

    fn contains(&self, event_id: &str) -> bool {
        self.gates.contains_key(event_id)
    }

    /// Receivers wake only when the gate changes.
    fn set(&self, event_id: &str, gate: Gate) {
        if let Some(sender) = self.gates.get(event_id) {
            sender.send_if_modified(|current| {
                let changed = *current != gate;
                *current = gate;
                changed
            });
        }
    }
}

/// The hub's answer for a gated event, kept with a schedule's rules: `confirmed` and `since`
/// survive a restart for the same approval, so a start without the hub opens only what the
/// hub confirmed before.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
struct ClaimEntry {
    hold: Option<HoldReason>,
    confirmed: bool,
    since: Option<i64>,
}

impl ClaimEntry {
    fn carried(stored: &Value, same_grant: bool) -> Self {
        let confirmed = same_grant && stored["confirmed"] == true;
        Self {
            hold: None,
            confirmed,
            since: stored["since"].as_i64().filter(|_| confirmed),
        }
    }

    /// `claimed`: the hub just confirmed the event, with the time this approval got it.
    fn armed(&mut self, claimed: Option<i64>) {
        if let Some(since) = claimed {
            self.confirmed = true;
            self.since = Some(since);
        }
        self.hold = None;
    }

    /// `answered`: the hub answered, so what it confirmed before no longer counts. True when
    /// the hold is new.
    fn held(&mut self, reason: HoldReason, answered: bool) -> bool {
        if answered {
            self.confirmed = false;
            self.since = None;
        }
        self.hold.replace(reason) != Some(reason)
    }

    fn gate(&self) -> Gate {
        match self.hold {
            None => Gate::Armed { since: self.since },
            Some(reason) => Gate::Held(reason),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum OnceState {
    /// This service has it; its time has not been handled.
    #[default]
    Pending,
    /// A run was started; durable before the run starts.
    Started,
    Ran,
    /// Armed before its time, and nothing could start it within the late limit.
    Missed,
    /// Its time was over when this service could first run it, or it was held at its time.
    Passed,
}

impl OnceState {
    fn finished(self) -> bool {
        matches!(self, Self::Ran | Self::Missed | Self::Passed)
    }
}

/// A one-time schedule's record. Its instant runs at most once: the record outlives
/// restarts, updates, Start, rollbacks and clock changes, and `started` is durable before
/// the run starts.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
struct OnceEntry {
    once_at: i64,
    timezone: String,
    event_version: Option<(u32, u32, u32)>,
    once_state: OnceState,
    /// When this service first had it armed, kept for the life of the record.
    once_armed_at: Option<i64>,
    #[serde(flatten)]
    claim: ClaimEntry,
    running_since: Option<i64>,
    last: Option<LastRun>,
    runs: u64,
    failed: u64,
}

impl OnceEntry {
    /// What a later process takes over. `same_approval`: the earlier file had this config's
    /// approval, or neither had one. A run the earlier process started never ends otherwise
    /// than cut off: it is not started again.
    fn carried(
        id: &str,
        stored: &Value,
        same_grant: bool,
        same_approval: bool,
        now: i64,
    ) -> Option<Self> {
        let once_at = stored["once_at"].as_i64()?;
        let mut entry = serde_json::from_value::<Self>(stored.clone()).unwrap_or_else(|_| Self {
            once_at,
            once_armed_at: stored["once_armed_at"].as_i64(),
            ..Default::default()
        });
        // A state this agent cannot read may have run: it never runs again.
        entry.once_state =
            serde_json::from_value(stored["once_state"].clone()).unwrap_or(OnceState::Passed);
        entry.claim = ClaimEntry::carried(stored, same_grant);
        entry.running_since = None;
        if !same_approval {
            entry.once_armed_at = None;
        }
        if entry.once_state == OnceState::Started {
            tracing::info!(
                event_id = %id,
                "One-time scheduled run was cut off by the end of its process and is not started again"
            );
            entry.ended(Outcome::Cancelled, now);
        }
        Some(entry)
    }

    fn from_done(record: &DoneRecord) -> Self {
        let ran = record.state == OnceState::Ran;
        let outcome = record.outcome.filter(|_| ran).unwrap_or(Outcome::Cancelled);
        Self {
            once_at: record.once_at,
            once_state: record.state,
            last: ran.then_some(LastRun {
                at: record.once_at,
                finished_at: record.at,
                outcome,
            }),
            runs: u64::from(ran),
            failed: u64::from(ran && matches!(outcome, Outcome::Failed | Outcome::TimedOut)),
            ..Default::default()
        }
    }

    /// The record of a finished entry that leaves `events`.
    fn done(&self, event_id: &str, now: i64) -> DoneRecord {
        let last = self
            .last
            .as_ref()
            .filter(|_| self.once_state == OnceState::Ran);
        DoneRecord {
            event_id: event_id.to_owned(),
            once_at: self.once_at,
            state: self.once_state,
            outcome: last.map(|last| last.outcome),
            at: last.map_or(now, |last| last.finished_at),
        }
    }

    fn ended(&mut self, outcome: Outcome, now: i64) {
        self.once_state = OnceState::Ran;
        self.running_since = None;
        self.last = Some(LastRun {
            at: self.once_at,
            finished_at: now,
            outcome,
        });
        if matches!(outcome, Outcome::Failed | Outcome::TimedOut) {
            self.failed = self.failed.saturating_add(1);
        }
    }

    fn finish(&mut self, id: &str, state: OnceState, now: i64) {
        self.once_state = state;
        let reason = match state {
            OnceState::Missed => "it was armed and nothing could start it within 15 minutes",
            _ => "its time was over when this service could run it",
        };
        tracing::info!(
            event_id = %id,
            once_at = self.once_at,
            late = now - self.once_at,
            reason,
            "One-time schedule does not run"
        );
    }

    /// The parent and the hub let this service run it. `armed`: it was armed before this
    /// answer.
    fn arm(&mut self, id: &str, now: i64, armed: bool, claimed: Option<i64>) {
        self.claim.armed(claimed);
        if armed || self.once_state != OnceState::Pending {
            return;
        }
        match self.armable(now) {
            Ok(()) => {
                self.once_armed_at.get_or_insert(now);
                tracing::info!(event_id = %id, once_at = self.once_at, "One-time schedule is armed");
            }
            Err(state) => self.finish(id, state, now),
        }
    }

    /// Whether it may be armed now, else the state it ends in. Armed for the first time only
    /// before its time, armed again also up to the late limit, and never at or before the
    /// time from which the hub may have run it.
    fn armable(&self, now: i64) -> Result<(), OnceState> {
        let after_floor = self.claim.since.is_none_or(|floor| floor < self.once_at);
        match self.once_armed_at {
            None if self.once_at > now && after_floor => Ok(()),
            None => Err(OnceState::Passed),
            Some(_) if now < self.once_at => Ok(()),
            Some(_) if now - self.once_at <= ONCE_LATE_LIMIT_SECS && after_floor => Ok(()),
            Some(_) => Err(OnceState::Missed),
        }
    }

    fn held(&mut self, id: &str, reason: HoldReason, announce: bool, answered: bool) {
        if self.claim.held(reason, answered) || announce {
            tracing::info!(
                event_id = %id,
                reason = reason.as_str(),
                "Schedule is held and does not run"
            );
        }
    }

    /// Whether its time has come and it may start now. A time it may no longer run at ends it.
    fn arrived(&mut self, id: &str, now: i64) -> bool {
        if self.once_state != OnceState::Pending || now < self.once_at {
            return false;
        }
        let state = if self.claim.hold.is_some() || self.once_armed_at.is_none() {
            OnceState::Passed
        } else if now - self.once_at > ONCE_LATE_LIMIT_SECS
            || self.claim.since.is_some_and(|floor| floor >= self.once_at)
        {
            OnceState::Missed
        } else {
            return true;
        };
        self.finish(id, state, now);
        false
    }

    /// Armed and its time is still ahead.
    fn waiting(&self) -> Option<i64> {
        (self.once_state == OnceState::Pending
            && self.claim.hold.is_none()
            && self.once_armed_at.is_some())
        .then_some(self.once_at)
    }
}

/// A finished one-time record of an event that left the config or got a new time, so the
/// event that comes back with the same time does not run it again.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct DoneRecord {
    event_id: String,
    once_at: i64,
    state: OnceState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    outcome: Option<Outcome>,
    at: i64,
}

impl DoneRecord {
    /// A record whose state this agent cannot read counts as passed: it never runs again.
    fn carried(stored: &Value) -> Option<Self> {
        let event_id = stored["event_id"].as_str()?;
        crate::config::validate_id("event", event_id).ok()?;
        let state = serde_json::from_value(stored["state"].clone())
            .ok()
            .filter(|state: &OnceState| state.finished())
            .unwrap_or(OnceState::Passed);
        Some(Self {
            event_id: event_id.to_owned(),
            once_at: stored["once_at"].as_i64()?,
            state,
            outcome: serde_json::from_value(stored["outcome"].clone()).ok(),
            at: stored["at"].as_i64().unwrap_or_default(),
        })
    }
}

/// The finished records an earlier process left, in the order they were kept.
fn carried_done(previous: &Value) -> Vec<DoneRecord> {
    let mut done: Vec<DoneRecord> = previous["once_done"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(DoneRecord::carried)
        .collect();
    let excess = done.len().saturating_sub(MAX_ONCE_DONE);
    done.drain(..excess);
    done
}

/// Keeps a finished record that leaves `events`; an unfinished one is dropped with it.
fn retire(done: &mut Vec<DoneRecord>, event_id: &str, entry: &OnceEntry, now: i64) {
    if !entry.once_state.finished() {
        return;
    }
    done.retain(|kept| !(kept.event_id == event_id && kept.once_at == entry.once_at));
    done.push(entry.done(event_id, now));
    let excess = done.len().saturating_sub(MAX_ONCE_DONE);
    done.drain(..excess);
}

/// Takes the finished record of `event_id` at `once_at` out of `done`, when there is one.
fn take_done(done: &mut Vec<DoneRecord>, event_id: &str, once_at: i64) -> Option<DoneRecord> {
    let index = done
        .iter()
        .position(|kept| kept.event_id == event_id && kept.once_at == once_at)?;
    Some(done.remove(index))
}

#[derive(Serialize)]
struct StateFile<'a> {
    version: u64,
    config_revision: u64,
    intent_revision: u64,
    decided: bool,
    grant_id: Option<&'a str>,
    events: Entries<'a>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    once_done: &'a Vec<DoneRecord>,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    claims: &'a BTreeMap<String, ClaimEntry>,
}

/// Repeating and one-time entries share the file's `events`.
struct Entries<'a> {
    repeating: &'a BTreeMap<String, EventState>,
    once: &'a BTreeMap<String, OnceEntry>,
}

#[derive(Serialize)]
#[serde(untagged)]
enum Entry<'a> {
    Repeating(&'a EventState),
    Once(&'a OnceEntry),
}

impl Serialize for Entries<'_> {
    fn serialize<S: serde::Serializer>(
        &self,
        serializer: S,
    ) -> std::result::Result<S::Ok, S::Error> {
        serializer.collect_map(
            self.repeating
                .iter()
                .map(|(id, entry)| (id, Entry::Repeating(entry)))
                .chain(self.once.iter().map(|(id, entry)| (id, Entry::Once(entry)))),
        )
    }
}

/// The device's wall clock, in Unix milliseconds.
pub(crate) trait Clock: Send + Sync {
    fn now_ms(&self) -> i64;
}

struct SystemClock;

impl Clock for SystemClock {
    fn now_ms(&self) -> i64 {
        Utc::now().timestamp_millis()
    }
}

/// The agent's parent process: which schedules another running service of this device
/// already runs, as `(event id, service id)`.
#[async_trait]
pub(crate) trait ScheduleArbiter: Send + Sync {
    async fn hold(&self, ids: &[String]) -> Result<Vec<(String, String)>>;
}

pub(crate) struct ClaimOutcome {
    /// Each claimed schedule with the hub's clock when this approval got it.
    pub(crate) claimed: Vec<(String, i64)>,
    pub(crate) held: Vec<(String, HoldReason)>,
}

impl ClaimOutcome {
    /// A schedule the hub does not name is never armed.
    fn decision(&self, id: &str) -> Decision {
        let claimed = self.claimed.iter().find(|(claimed, _)| claimed == id);
        let held = self.held.iter().find(|(held, _)| held == id);
        match (claimed, held) {
            (Some((_, since)), _) => Decision::Arm(Some(*since)),
            (None, Some((_, reason))) => Decision::Hold(*reason),
            (None, None) => Decision::Hold(HoldReason::HubUnreachable),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ClaimError {
    Unreachable,
    HubTooOld,
    Denied,
}

/// The hub: `ids` is the complete set of schedules this service runs now.
#[async_trait]
pub(crate) trait ScheduleClaims: Send + Sync {
    async fn claim(&self, ids: &[String]) -> Result<ClaimOutcome, ClaimError>;
}

/// One scheduled run. A failure is the caller's to count; its text stays in the run's own log.
#[async_trait]
pub(crate) trait ScheduleRunner: Send + Sync {
    async fn run(&self, event_id: &str, run_id: &str, cancel: CancellationToken) -> bool;
}

/// What the scheduler of one supervised placement process needs besides its events.
pub struct ScheduleContext {
    pub(crate) state_dir: PathBuf,
    pub(crate) placement_id: String,
    pub(crate) project_id: String,
    pub(crate) config_revision: u64,
    pub(crate) intent_revision: u64,
    /// The approval an online placement claims its schedules with.
    pub(crate) grant_id: Option<String>,
    pub(crate) arbiter: Option<Arc<dyn ScheduleArbiter>>,
    pub(crate) claims: Option<Arc<dyn ScheduleClaims>>,
    pub(crate) clock: Arc<dyn Clock>,
}

impl ScheduleContext {
    /// The context of a placement process the agent supervises. The hub side is added by
    /// the online runtime.
    #[cfg(unix)]
    pub fn supervised(
        data_root: &Path,
        config: &crate::config::PlacementConfig,
        config_revision: u64,
        intent_revision: u64,
        broker: Arc<crate::ipc::ChildBroker>,
    ) -> Self {
        Self {
            state_dir: data_root.join(STATE_DIRECTORY).join(&config.id),
            placement_id: config.id.clone(),
            project_id: config.project_id.clone(),
            config_revision,
            intent_revision,
            grant_id: config
                .resource_grant
                .as_ref()
                .filter(|_| config.source == crate::config::ProjectSource::Online)
                .map(|grant| grant.grant_id.clone()),
            arbiter: Some(broker),
            claims: None,
            clock: Arc::new(SystemClock),
        }
    }
}

/// `<data root>/.standalone-schedule/<placement>`, private to the agent's account.
fn create_state_directory(directory: &Path) -> Result<()> {
    directory
        .parent()
        .context("Schedule state directory has no parent")
        .and_then(crate::runtime::private_runtime_directory)
        .and_then(|()| crate::runtime::private_runtime_directory(directory))
        .with_context(|| {
            format!(
                "Create the schedule state directory {}",
                directory.display()
            )
        })
}

/// Replaces the state file as a whole. The file and its name are durable on return: a
/// watermark lost to a power failure could let a scheduled time run again.
fn write_state(directory: &Path, bytes: &[u8]) -> Result<()> {
    ensure!(
        bytes.len() <= MAX_STATE_BYTES,
        "Schedule state of {} bytes exceeds its {MAX_STATE_BYTES} byte limit",
        bytes.len()
    );
    let temporary = directory.join(format!(".{STATE_FILE}.tmp"));
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
    }
    options
        .open(&temporary)
        .and_then(|mut file| {
            file.write_all(bytes)?;
            file.sync_all()
        })
        .and_then(|()| std::fs::rename(&temporary, directory.join(STATE_FILE)))
        .and_then(|()| std::fs::File::open(directory)?.sync_all())
        .with_context(|| format!("Write schedule state in {}", directory.display()))
}

/// The file an earlier process left, or `None` when there is none a device can use.
fn read_state(directory: &Path) -> Result<Option<Value>> {
    let path = directory.join(STATE_FILE);
    let Some(bytes) = crate::diagnostics::small_file(&path, MAX_STATE_BYTES)
        .with_context(|| format!("Read schedule state {}", path.display()))?
    else {
        return Ok(None);
    };
    let usable = serde_json::from_slice::<Value>(&bytes)
        .ok()
        .filter(|_| bytes.len() <= MAX_STATE_BYTES)
        .filter(|state| state["version"].as_u64() == Some(STATE_VERSION));
    if usable.is_none() {
        tracing::warn!(
            "Schedule state {} is not readable by this agent; schedules start without their earlier watermarks",
            path.display()
        );
    }
    Ok(usable)
}

/// What survives a restart. The watermark always does; counters only within one intent
/// revision; the hub's confirmation only for the same approval.
fn carried(previous: &Value, context: &ScheduleContext) -> BTreeMap<String, EventState> {
    let same_intent = previous["intent_revision"].as_u64() == Some(context.intent_revision);
    let same_grant =
        context.grant_id.is_some() && previous["grant_id"].as_str() == context.grant_id.as_deref();
    let mut events = BTreeMap::new();
    for (id, entry) in previous["events"].as_object().into_iter().flatten() {
        if crate::config::validate_id("event", id).is_err() || is_once(entry) {
            continue;
        }
        let stored: EventState =
            serde_json::from_value(entry.clone()).unwrap_or_else(|_| EventState {
                watermark: entry["watermark"].as_i64(),
                confirmed: entry["confirmed"] == true,
                since: entry["since"].as_i64(),
                ..Default::default()
            });
        let mut kept = EventState {
            watermark: stored.watermark,
            ..Default::default()
        };
        if same_grant {
            kept.confirmed = stored.confirmed;
            kept.since = stored.since.filter(|_| stored.confirmed);
        }
        if same_intent {
            kept.armed_at = stored.armed_at;
            kept.last = stored.last;
            kept.runs = stored.runs;
            kept.failed = stored.failed;
            kept.skipped_overlap = stored.skipped_overlap;
            kept.skipped_missed = stored.skipped_missed;
            kept.skipped_busy = stored.skipped_busy;
            kept.last_skip = stored.last_skip;
        }
        events.insert(id.clone(), kept);
    }
    events
}

/// A one-time entry of the state file has an instant instead of an expression.
fn is_once(entry: &Value) -> bool {
    entry.get("once_at").is_some_and(Value::is_i64)
}

/// What the hub confirmed for each gate, for the same approval only.
fn carried_claims(
    previous: Option<&Value>,
    context: &ScheduleContext,
    gates: &ClaimGates,
) -> BTreeMap<String, ClaimEntry> {
    let same_grant = context.grant_id.is_some()
        && previous.and_then(|previous| previous["grant_id"].as_str())
            == context.grant_id.as_deref();
    gates
        .ids()
        .map(|id| {
            let stored = previous.map_or(&Value::Null, |previous| &previous["claims"][id]);
            (id.clone(), ClaimEntry::carried(stored, same_grant))
        })
        .collect()
}

/// The finished one-time records of events `specs` no longer lists are kept apart; the
/// unfinished ones go with their events.
fn retire_unlisted(
    onces: &mut BTreeMap<String, OnceEntry>,
    done: &mut Vec<DoneRecord>,
    specs: &BTreeMap<String, ScheduleSpec>,
    now: i64,
) {
    for (id, entry) in onces.extract_if(.., |id, _| !specs.contains_key(id)) {
        retire(done, &id, &entry, now);
    }
}

/// The entries an earlier process left, until this process's specs have taken theirs.
#[derive(Default)]
struct Carried {
    events: BTreeMap<String, EventState>,
    onces: BTreeMap<String, OnceEntry>,
    once_done: Vec<DoneRecord>,
}

impl Carried {
    fn read(previous: Option<&Value>, context: &ScheduleContext, now: i64) -> Self {
        previous.map_or_else(Self::default, |previous| Self {
            events: carried(previous, context),
            onces: carried_onces(previous, context, now),
            once_done: carried_done(previous),
        })
    }

    /// Gives every spec its entry. Entries of events the specs no longer list stay until
    /// the process arms, unless there are too many.
    fn place(&mut self, specs: &BTreeMap<String, ScheduleSpec>, now: i64) {
        for (id, spec) in specs {
            let earlier = self.onces.remove(id);
            match spec.once() {
                Some(once) => self.place_once(id, spec, &once, earlier, now),
                None => {
                    if let Some(earlier) = earlier {
                        retire(&mut self.once_done, id, &earlier, now);
                    }
                    let entry = self.events.entry(id.clone()).or_default();
                    entry.expression = spec.expression().to_owned();
                    entry.timezone = spec.timezone().to_owned();
                }
            }
        }
        if self.events.len() > MAX_EVENTS {
            self.events.retain(|id, _| specs.contains_key(id));
        }
        if self.onces.len() > MAX_EVENTS {
            retire_unlisted(&mut self.onces, &mut self.once_done, specs, now);
        }
    }

    /// The record of this event and instant, wherever it was kept. A new instant starts a
    /// new record and keeps the earlier one apart when it finished.
    fn place_once(
        &mut self,
        id: &str,
        spec: &ScheduleSpec,
        once: &OnceAt,
        earlier: Option<OnceEntry>,
        now: i64,
    ) {
        self.events.remove(id);
        let mut record = match earlier {
            Some(earlier) if earlier.once_at == once.at => earlier,
            earlier => {
                let record = take_done(&mut self.once_done, id, once.at)
                    .map_or_else(OnceEntry::default, |done| OnceEntry::from_done(&done));
                if let Some(earlier) = earlier {
                    retire(&mut self.once_done, id, &earlier, now);
                }
                record
            }
        };
        record.once_at = once.at;
        record.timezone = once.timezone.to_owned();
        record.event_version = Some(spec.event_version);
        self.onces.insert(id.to_owned(), record);
    }
}

/// The one-time records an earlier process left. Unlike a watermark, a record survives a
/// new intent revision and is never dropped for lying ahead of the clock.
fn carried_onces(
    previous: &Value,
    context: &ScheduleContext,
    now: i64,
) -> BTreeMap<String, OnceEntry> {
    let same_grant =
        context.grant_id.is_some() && previous["grant_id"].as_str() == context.grant_id.as_deref();
    let same_approval = previous["grant_id"].as_str() == context.grant_id.as_deref();
    previous["events"]
        .as_object()
        .into_iter()
        .flatten()
        .filter(|(id, entry)| crate::config::validate_id("event", id).is_ok() && is_once(entry))
        .filter_map(|(id, entry)| {
            OnceEntry::carried(id, entry, same_grant, same_approval, now)
                .map(|entry| (id.clone(), entry))
        })
        .collect()
}

struct Running {
    run_id: String,
    scheduled_at: i64,
    started: Instant,
    cancel: CancellationToken,
    abort: tokio::task::AbortHandle,
    /// Set once the scheduler ended the run itself: what to record, and when it asked.
    ended: Option<(Outcome, Instant)>,
}

enum Decision {
    Keep,
    Arm(Option<i64>),
    Hold(HoldReason),
}

/// The schedules of one placement process, between `prepare` and the end of `run`.
pub(crate) struct Scheduler {
    context: ScheduleContext,
    specs: BTreeMap<String, ScheduleSpec>,
    /// Repeating schedules.
    events: BTreeMap<String, EventState>,
    onces: BTreeMap<String, OnceEntry>,
    once_done: Vec<DoneRecord>,
    /// One entry per gate.
    claims: BTreeMap<String, ClaimEntry>,
    gates: ClaimGates,
    decided: bool,
    dirty: bool,
    /// Schedules another running service of this device runs, with that service.
    refused: HashMap<String, String>,
    next_ask: Option<Instant>,
    last_now: i64,
    running: HashMap<String, Running>,
    /// Each run's task ends with whether the run succeeded.
    tasks: JoinSet<bool>,
    task_events: HashMap<tokio::task::Id, String>,
}

/// Before Ready: creates the state directory, takes over what an earlier process left and
/// rewrites the file for this one. A failure fails the start, because without a durable
/// watermark a restart could repeat a scheduled time.
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn prepare(
    context: ScheduleContext,
    specs: Vec<(String, ScheduleSpec)>,
) -> Result<Scheduler> {
    prepare_with_gates(context, specs, ClaimGates::default())
}

/// `prepare` for a process whose self-firing events are schedules, gated events or both.
/// Without a schedule it only claims and keeps the gates.
pub(crate) fn prepare_with_gates(
    context: ScheduleContext,
    specs: Vec<(String, ScheduleSpec)>,
    gates: ClaimGates,
) -> Result<Scheduler> {
    let claimed = specs.len() + gates.len();
    ensure!(
        (1..=MAX_EVENTS).contains(&claimed),
        "A placement runs 1 to {MAX_EVENTS} schedules and bots together, got {claimed}"
    );
    ensure!(
        specs.iter().all(|(id, _)| !gates.contains(id)),
        "An event of the placement is both a schedule and a bot"
    );
    create_state_directory(&context.state_dir)?;
    let previous = read_state(&context.state_dir)?;
    let now = context.clock.now_ms().div_euclid(1000);
    let claims = carried_claims(previous.as_ref(), &context, &gates);
    let specs: BTreeMap<String, ScheduleSpec> = specs.into_iter().collect();
    let mut carried = Carried::read(previous.as_ref(), &context, now);
    carried.place(&specs, now);
    let Carried {
        events,
        onces,
        once_done,
    } = carried;
    let scheduler = Scheduler {
        context,
        specs,
        events,
        onces,
        once_done,
        claims,
        gates,
        decided: false,
        dirty: false,
        refused: HashMap::new(),
        next_ask: None,
        last_now: i64::MIN,
        running: HashMap::new(),
        tasks: JoinSet::new(),
        task_events: HashMap::new(),
    };
    write_state(&scheduler.context.state_dir, &scheduler.encoded()?)?;
    Ok(scheduler)
}

/// Runs file work in place: on a multi-threaded runtime the other tasks move to another
/// worker meanwhile.
fn in_place<T>(work: impl FnOnce() -> T) -> T {
    match tokio::runtime::Handle::current().runtime_flavor() {
        tokio::runtime::RuntimeFlavor::MultiThread => tokio::task::block_in_place(work),
        _ => work(),
    }
}

async fn ready_or_stopped(ready: oneshot::Receiver<()>, stop: &CancellationToken) -> bool {
    tokio::select! {
        biased;
        _ = stop.cancelled() => false,
        signal = ready => {
            if signal.is_err() {
                stop.cancelled().await;
            }
            signal.is_ok()
        }
    }
}

impl Scheduler {
    fn encoded(&self) -> Result<Vec<u8>> {
        Ok(serde_json::to_vec(&StateFile {
            version: STATE_VERSION,
            config_revision: self.context.config_revision,
            intent_revision: self.context.intent_revision,
            decided: self.decided,
            grant_id: self.context.grant_id.as_deref(),
            events: Entries {
                repeating: &self.events,
                once: &self.onces,
            },
            once_done: &self.once_done,
            claims: &self.claims,
        })?)
    }

    /// Writes the state file before it returns. A write is never left behind half done or
    /// overtaken by a later one, so it does not yield: the other tasks of a multi-threaded
    /// runtime move to another worker meanwhile.
    fn persist(&mut self) -> Result<()> {
        let bytes = self.encoded()?;
        in_place(|| write_state(&self.context.state_dir, &bytes))?;
        self.dirty = false;
        Ok(())
    }

    fn now(&self) -> i64 {
        self.context.clock.now_ms().div_euclid(1000)
    }

    /// Runs the placement's schedules until `stop`. `ready` resolves once the supervisor
    /// accepted the service as ready; nothing is asked, claimed or armed before that.
    pub(crate) async fn run(
        mut self,
        runner: Arc<dyn ScheduleRunner>,
        ready: oneshot::Receiver<()>,
        stop: CancellationToken,
    ) -> Result<()> {
        if !ready_or_stopped(ready, &stop).await {
            return Ok(());
        }
        loop {
            let decided = tokio::select! {
                biased;
                _ = stop.cancelled() => return Ok(()),
                decided = self.ask() => decided,
            };
            if decided {
                break;
            }
            tokio::select! {
                biased;
                _ = stop.cancelled() => return Ok(()),
                _ = tokio::time::sleep(PARENT_RETRY) => {}
            }
        }
        loop {
            tokio::select! {
                biased;
                _ = stop.cancelled() => break,
                _ = self.pass(&runner) => {}
            }
            let wait = self.wait();
            tokio::select! {
                biased;
                _ = stop.cancelled() => break,
                Some(finished) = self.tasks.join_next_with_id(), if !self.tasks.is_empty() => {
                    self.finished(finished);
                }
                _ = tokio::time::sleep(wait) => {}
            }
        }
        self.drain().await;
        Ok(())
    }

    /// Until the earliest scheduled time or the next question to the parent and the hub,
    /// but never long enough to miss a suspend or a clock change.
    fn wait(&self) -> Duration {
        let now_ms = self.context.clock.now_ms();
        // A one-time schedule whose time has come waits for a free run slot: a finished run
        // wakes the loop.
        let due = self
            .events
            .values()
            .filter(|entry| entry.hold.is_none())
            .filter_map(|entry| entry.next_at)
            .chain(
                self.onces
                    .values()
                    .filter_map(OnceEntry::waiting)
                    .filter(|once| once.saturating_mul(1000) > now_ms),
            )
            .min()
            .map_or(MAX_POLL, |next| {
                Duration::from_millis(
                    next.saturating_mul(1000)
                        .saturating_sub(now_ms)
                        .clamp(0, MAX_POLL.as_millis() as i64) as u64,
                )
            });
        let ask = self
            .next_ask
            .map_or(MAX_POLL, |at| at.saturating_duration_since(Instant::now()));
        due.min(ask).clamp(MIN_POLL, MAX_POLL)
    }

    async fn pass(&mut self, runner: &Arc<dyn ScheduleRunner>) {
        while let Some(finished) = self.tasks.try_join_next_with_id() {
            self.finished(finished);
        }
        self.end_overlong_runs();
        self.start_due(runner);
        if self.next_ask.is_some_and(|at| Instant::now() >= at) {
            self.ask().await;
        }
        if self.dirty
            && let Err(error) = self.persist()
        {
            tracing::warn!("Schedule state could not be written: {error:#}");
        }
    }

    fn end_overlong_runs(&mut self) {
        for (event_id, running) in &mut self.running {
            match running.ended {
                None if running.started.elapsed() >= RUN_LIMIT => {
                    tracing::info!(
                        event_id = %event_id,
                        run_id = %running.run_id,
                        "Scheduled run reached its 24 hour limit and is cancelled"
                    );
                    running.cancel.cancel();
                    running.ended = Some((Outcome::TimedOut, Instant::now()));
                }
                Some((_, asked)) if asked.elapsed() >= CANCEL_GRACE => running.abort.abort(),
                _ => {}
            }
        }
    }

    /// Skips or starts every scheduled time that has arrived. The watermarks and one-time
    /// records of the runs to start are durable before the first of them starts. A one-time
    /// schedule takes a free run slot first and waits for one inside its late limit.
    fn start_due(&mut self, runner: &Arc<dyn ScheduleRunner>) {
        let now = self.now();
        let clock_went_back = now < self.last_now;
        self.last_now = now;
        let mut starts = self.once_starts(now);
        for (id, spec) in &self.specs {
            let Some(entry) = self.events.get_mut(id) else {
                continue;
            };
            let (arrived, changed) = entry.arrived(id, spec, now, clock_went_back);
            self.dirty |= changed;
            let Some(scheduled_at) = arrived else {
                continue;
            };
            let skip = if now - scheduled_at > LATE_LIMIT_SECS {
                Some(SkipReason::Missed)
            } else if self.running.contains_key(id) {
                Some(SkipReason::Overlap)
            } else if self.running.len() + starts.len() >= MAX_CONCURRENT_RUNS {
                Some(SkipReason::Busy)
            } else {
                None
            };
            match skip {
                Some(reason) => entry.skipped(id, scheduled_at, reason),
                None => starts.push((id.clone(), scheduled_at)),
            }
        }
        if starts.is_empty() {
            return;
        }
        match self.persist() {
            Ok(()) => starts
                .into_iter()
                .for_each(|(id, scheduled_at)| self.start(id, scheduled_at, now, runner)),
            Err(error) => self.not_started(starts, now, &error),
        }
    }

    /// The one-time schedules to start now, marked `started`; one whose time it may no longer
    /// run at ends. One without a free run slot waits.
    fn once_starts(&mut self, now: i64) -> Vec<(String, i64)> {
        let mut starts = Vec::new();
        for (id, entry) in &mut self.onces {
            if !self.specs.contains_key(id) {
                continue;
            }
            let before = entry.once_state;
            let due = entry.arrived(id, now);
            self.dirty |= entry.once_state != before;
            let free = self.running.len() + starts.len() < MAX_CONCURRENT_RUNS;
            if due && free && !self.running.contains_key(id) {
                entry.once_state = OnceState::Started;
                starts.push((id.clone(), entry.once_at));
            }
        }
        starts
    }

    /// Without a durable watermark a restart could run these times again, so they do not
    /// run and count as failed. A one-time schedule is tried again inside its late limit.
    fn not_started(&mut self, starts: Vec<(String, i64)>, now: i64, error: &anyhow::Error) {
        tracing::warn!(
            "Scheduled runs were not started because their times could not be recorded first: {error:#}"
        );
        for (id, scheduled_at) in starts {
            if let Some(entry) = self.onces.get_mut(&id) {
                entry.once_state = OnceState::Pending;
            } else if let Some(entry) = self.events.get_mut(&id) {
                entry.runs = entry.runs.saturating_add(1);
                entry.failed = entry.failed.saturating_add(1);
                entry.last = Some(LastRun {
                    at: scheduled_at,
                    finished_at: now,
                    outcome: Outcome::Failed,
                });
            }
        }
        self.dirty = true;
    }

    fn start(
        &mut self,
        event_id: String,
        scheduled_at: i64,
        now: i64,
        runner: &Arc<dyn ScheduleRunner>,
    ) {
        let run_id = uuid::Uuid::new_v4().to_string();
        let cancel = CancellationToken::new();
        if let Some(entry) = self.events.get_mut(&event_id) {
            entry.running_since = Some(now);
            entry.runs = entry.runs.saturating_add(1);
        } else if let Some(entry) = self.onces.get_mut(&event_id) {
            entry.running_since = Some(now);
            entry.runs = entry.runs.saturating_add(1);
        }
        self.dirty = true;
        tracing::info!(
            event_id = %event_id,
            run_id = %run_id,
            scheduled_at,
            "Scheduled run started"
        );
        let abort = self.tasks.spawn({
            let (runner, event_id, run_id, cancel) = (
                runner.clone(),
                event_id.clone(),
                run_id.clone(),
                cancel.clone(),
            );
            async move { runner.run(&event_id, &run_id, cancel).await }
        });
        self.task_events.insert(abort.id(), event_id.clone());
        self.running.insert(
            event_id,
            Running {
                run_id,
                scheduled_at,
                started: Instant::now(),
                cancel,
                abort,
                ended: None,
            },
        );
    }

    /// The runner counts the run's usage; this records its outcome.
    fn finished(&mut self, result: Result<(tokio::task::Id, bool), JoinError>) {
        let (task, succeeded) = match result {
            Ok((task, succeeded)) => (task, succeeded),
            Err(error) => (error.id(), false),
        };
        let Some(event_id) = self.task_events.remove(&task) else {
            return;
        };
        let Some(running) = self.running.remove(&event_id) else {
            return;
        };
        let outcome = match running.ended {
            Some((outcome, _)) => outcome,
            None if succeeded => Outcome::Succeeded,
            None => Outcome::Failed,
        };
        self.record(&event_id, &running, outcome);
    }

    fn record(&mut self, event_id: &str, running: &Running, outcome: Outcome) {
        let now = self.now();
        if let Some(entry) = self.events.get_mut(event_id) {
            entry.running_since = None;
            entry.last = Some(LastRun {
                at: running.scheduled_at,
                finished_at: now,
                outcome,
            });
            if matches!(outcome, Outcome::Failed | Outcome::TimedOut) {
                entry.failed = entry.failed.saturating_add(1);
            }
        } else if let Some(entry) = self.onces.get_mut(event_id) {
            entry.ended(outcome, now);
        }
        self.dirty = true;
        tracing::info!(
            event_id = %event_id,
            run_id = %running.run_id,
            outcome = outcome.as_str(),
            duration_ms = running.started.elapsed().as_millis() as u64,
            "Scheduled run finished"
        );
    }

    /// Asks the parent and, for an online app, the hub which schedules and gated events this
    /// service runs, in one call each. False while the parent has not answered for the first
    /// time: nothing is armed then.
    async fn ask(&mut self) -> bool {
        let ids: Vec<String> = self.specs.keys().chain(self.gates.ids()).cloned().collect();
        if !self.ask_parent(&ids).await {
            return false;
        }
        let hub = self.ask_hub(&ids).await;
        let answers = Answers {
            refused: &self.refused,
            hub: &hub,
            first: !self.decided,
            // Only an unanswered question keeps what the hub confirmed before.
            answered: !matches!(hub, Some(Err(ClaimError::Unreachable))),
            now: self.context.clock.now_ms().div_euclid(1000),
        };
        for (id, spec) in &self.specs {
            if let Some(entry) = self.onces.get_mut(id) {
                answers.apply_once(id, entry);
            } else if let Some(entry) = self.events.get_mut(id) {
                answers.apply_repeating(id, spec, entry);
            }
        }
        for (id, entry) in &mut self.claims {
            answers.apply_gate(id, entry);
            self.gates.set(id, entry.gate());
        }
        self.settle(&hub);
        true
    }

    /// The hold of every schedule and gated event. A finished one-time schedule keeps its
    /// claim and so the confirmations: the hub would read a missing one as a hand-back.
    fn holds(&self) -> impl Iterator<Item = Option<HoldReason>> + '_ {
        self.events
            .values()
            .map(|entry| entry.hold)
            .chain(self.onces.values().map(|entry| entry.claim.hold))
            .chain(self.claims.values().map(|entry| entry.hold))
    }

    /// Takes what the parent refuses. False while it has not answered once: nothing is
    /// decided then. A parent that is busy later changes nothing it said before.
    async fn ask_parent(&mut self, ids: &[String]) -> bool {
        let Some(arbiter) = self.context.arbiter.clone() else {
            return true;
        };
        match tokio::time::timeout(PARENT_TIMEOUT, arbiter.hold(ids)).await {
            Ok(Ok(held)) => self.refused = held.into_iter().collect(),
            _ => return self.decided,
        }
        true
    }

    /// The hub's answer for every schedule the parent lets this service run, also when
    /// that is none: the hub then learns what this service no longer runs.
    async fn ask_hub(&self, ids: &[String]) -> Option<Result<ClaimOutcome, ClaimError>> {
        let claims = self.context.claims.clone()?;
        let candidates: Vec<String> = ids
            .iter()
            .filter(|id| !self.refused.contains_key(*id))
            .cloned()
            .collect();
        let answer = tokio::time::timeout(HUB_TIMEOUT, claims.claim(&candidates)).await;
        Some(answer.unwrap_or(Err(ClaimError::Unreachable)))
    }

    /// Every schedule has its state now: events the config no longer lists go, and the
    /// next question is set.
    fn settle(&mut self, hub: &Option<Result<ClaimOutcome, ClaimError>>) {
        self.events.retain(|id, _| self.specs.contains_key(id));
        let now = self.now();
        retire_unlisted(&mut self.onces, &mut self.once_done, &self.specs, now);
        if !self.decided {
            let (total, held) = self.holds().fold((0, 0), |(total, held), hold| {
                (total + 1, held + usize::from(hold.is_some()))
            });
            tracing::info!(
                placement_id = %self.context.placement_id,
                project_id = %self.context.project_id,
                armed = total - held,
                held,
                "Schedules are decided"
            );
        }
        self.decided = true;
        self.dirty = true;
        self.next_ask = self.ask_again(hub).map(|delay| Instant::now() + delay);
    }

    /// A confirmation is due while one schedule or gated event runs through the hub.
    fn ask_again(&self, hub: &Option<Result<ClaimOutcome, ClaimError>>) -> Option<Duration> {
        let holds = |reasons: &[HoldReason]| {
            self.holds()
                .any(|hold| hold.is_some_and(|hold| reasons.contains(&hold)))
        };
        let online = self.context.claims.is_some();
        [
            (matches!(hub, Some(Err(ClaimError::Unreachable)))
                || holds(&[HoldReason::HubUnreachable]))
            .then_some(UNREACHABLE_RETRY),
            holds(&[
                HoldReason::NotReleased,
                HoldReason::RunsElsewhere,
                HoldReason::OtherService,
            ])
            .then_some(HELD_RETRY),
            holds(&[HoldReason::HubTooOld]).then_some(HUB_TOO_OLD_RETRY),
            (online && self.holds().any(|hold| hold.is_none())).then_some(CONFIRMATION),
        ]
        .into_iter()
        .flatten()
        .min()
    }

    /// Stop: no new runs. Runs in progress get the service's drain time, then are cancelled.
    async fn drain(&mut self) {
        let deadline = Instant::now() + STOP_GRACE;
        while !self.tasks.is_empty() {
            tokio::select! {
                Some(finished) = self.tasks.join_next_with_id() => self.finished(finished),
                _ = tokio::time::sleep_until(deadline) => break,
            }
        }
        for running in self.running.values_mut() {
            if running.ended.is_none() {
                running.ended = Some((Outcome::Cancelled, Instant::now()));
            }
            running.cancel.cancel();
        }
        let deadline = Instant::now() + CANCEL_GRACE;
        while !self.tasks.is_empty() {
            tokio::select! {
                Some(finished) = self.tasks.join_next_with_id() => self.finished(finished),
                _ = tokio::time::sleep_until(deadline) => break,
            }
        }
        self.tasks.abort_all();
        for (event_id, running) in std::mem::take(&mut self.running) {
            let outcome = running
                .ended
                .map_or(Outcome::Cancelled, |(outcome, _)| outcome);
            self.record(&event_id, &running, outcome);
        }
        if let Err(error) = self.persist() {
            tracing::warn!("Schedule state could not be written at stop: {error:#}");
        }
    }
}

/// Forgets the state an earlier config left, except the records of one-time schedules that
/// finished: a rollback that brings one back must not run it again. The file goes when there
/// are none. A directory that is a link is left alone.
fn forget_state(context: &ScheduleContext) -> Result<()> {
    let directory = &context.state_dir;
    let real = |path: &Path| std::fs::symlink_metadata(path).is_ok_and(|found| found.is_dir());
    if !(directory.parent().is_some_and(real) && real(directory)) {
        return Ok(());
    }
    let now = context.clock.now_ms().div_euclid(1000);
    let mut carried = Carried::read(read_state(directory).ok().flatten().as_ref(), context, now);
    retire_unlisted(
        &mut carried.onces,
        &mut carried.once_done,
        &BTreeMap::new(),
        now,
    );
    let done = carried.once_done;
    if done.is_empty() {
        return match std::fs::remove_file(directory.join(STATE_FILE)) {
            Err(error) if error.kind() != std::io::ErrorKind::NotFound => Err(error.into()),
            _ => Ok(()),
        };
    }
    let bytes = serde_json::to_vec(&StateFile {
        version: STATE_VERSION,
        config_revision: context.config_revision,
        intent_revision: context.intent_revision,
        decided: false,
        grant_id: context.grant_id.as_deref(),
        events: Entries {
            repeating: &BTreeMap::new(),
            once: &BTreeMap::new(),
        },
        once_done: &done,
        claims: &BTreeMap::new(),
    })?;
    write_state(directory, &bytes)
}

/// A placement without a schedule or a bot, once it is ready: it forgets the state an
/// earlier config left, and an online one tells the hub that it runs none, until one call
/// was answered.
pub(crate) async fn hand_back_all(
    context: ScheduleContext,
    ready: oneshot::Receiver<()>,
    stop: CancellationToken,
) {
    if !ready_or_stopped(ready, &stop).await {
        return;
    }
    if in_place(|| forget_state(&context)).is_err() {
        tracing::warn!(
            placement_id = %context.placement_id,
            "The schedule state of an earlier configuration could not be removed"
        );
    }
    let Some(claims) = context.claims else {
        return;
    };
    loop {
        let answer = tokio::select! {
            biased;
            _ = stop.cancelled() => return,
            answer = claims.claim(&[]) => answer,
        };
        if !matches!(answer, Err(ClaimError::Unreachable)) {
            return;
        }
        tokio::select! {
            biased;
            _ = stop.cancelled() => return,
            _ = tokio::time::sleep(UNREACHABLE_RETRY) => {}
        }
    }
}

/// What a scheduled run shares with every other run of the placement process.
pub(crate) struct RunEnvironment {
    pub(crate) project_id: String,
    pub(crate) state: Arc<FlowLikeState>,
    pub(crate) profile: Profile,
    pub(crate) visibility: AppVisibility,
    pub(crate) execution_sub: Option<String>,
}

struct FlowRunner {
    context: crate::run_once::RunContext,
    events: HashMap<String, (PreparedInvocation, Option<Value>)>,
}

/// Runs each schedule's pinned event as the placement's other events run: the same
/// template, identity, usage attribution and log policy, without a request.
pub(crate) fn flow_runner(
    environment: RunEnvironment,
    scheduled: Vec<(ScheduleSpec, PreparedInvocation)>,
) -> Arc<dyn ScheduleRunner> {
    let RunEnvironment {
        project_id,
        state,
        profile,
        visibility,
        execution_sub,
    } = environment;
    Arc::new(FlowRunner {
        context: crate::run_once::RunContext {
            project_id,
            state,
            profile,
            visibility,
            execution_sub,
        },
        events: scheduled
            .into_iter()
            .map(|(spec, invocation)| (invocation.event.id.clone(), (invocation, spec.payload)))
            .collect(),
    })
}

#[async_trait]
impl ScheduleRunner for FlowRunner {
    async fn run(&self, event_id: &str, run_id: &str, cancel: CancellationToken) -> bool {
        let Some((prepared, payload)) = self.events.get(event_id) else {
            tracing::warn!(
                event_id,
                run_id,
                "Scheduled run is not part of this placement"
            );
            return false;
        };
        let request = crate::run_once::RunRequest {
            payload: payload.clone(),
            callback: None,
            cancel,
            run_id: Some(run_id.to_owned()),
            time_limit: RUN_LIMIT + RUN_BACKSTOP,
            request_bytes: 0,
        };
        crate::run_once::run_event_once(&self.context, prepared, request).await
            == crate::run_once::RunEnd::Succeeded
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::sync::{
        Mutex,
        atomic::{AtomicI64, AtomicUsize, Ordering},
    };

    fn at(text: &str) -> DateTime<Utc> {
        text.parse().unwrap()
    }

    fn secs(text: &str) -> i64 {
        at(text).timestamp()
    }

    fn parsed(config: Value) -> Result<ScheduleSpec, ScheduleProblem> {
        ScheduleSpec::parse(
            &serde_json::to_vec(&config).unwrap(),
            at("2026-10-03T00:00:00Z"),
        )
    }

    fn spec(expression: &str, timezone: Option<&str>) -> ScheduleSpec {
        parsed(json!({"expression": expression, "timezone": timezone})).unwrap()
    }

    fn code(config: Value) -> Option<&'static str> {
        parsed(config).err().map(|problem| problem.code())
    }

    /// The next `count` times after `from`, as weekday, day and time in the schedule's zone.
    fn times(spec: &ScheduleSpec, from: &str, count: usize) -> Vec<String> {
        let mut instant = at(from);
        (0..count)
            .map(|_| {
                instant = spec.next_after(instant).unwrap();
                instant
                    .with_timezone(&spec.timezone)
                    .format("%a %d %H:%M")
                    .to_string()
            })
            .collect()
    }

    fn utc_times(spec: &ScheduleSpec, from: &str, count: usize) -> Vec<String> {
        let mut instant = at(from);
        (0..count)
            .map(|_| {
                instant = spec.next_after(instant).unwrap();
                instant.format("%d %H:%M").to_string()
            })
            .collect()
    }

    #[test]
    fn the_documented_dialect_reads_weekdays_sundays_and_either_day_field() {
        // 2026-10-03 is a Saturday.
        for expression in ["0 0 9 * * 1-5", "0 9 * * 1-5", "0 9 * * mon-fri"] {
            assert_eq!(
                times(
                    &spec(expression, Some("Europe/Berlin")),
                    "2026-10-03T00:00:00Z",
                    6
                ),
                [
                    "Mon 05 09:00",
                    "Tue 06 09:00",
                    "Wed 07 09:00",
                    "Thu 08 09:00",
                    "Fri 09 09:00",
                    "Mon 12 09:00"
                ],
                "{expression}"
            );
        }
        for expression in ["0 0 18 * * 0", "0 0 18 * * 7", "0 18 * * Sun"] {
            assert_eq!(
                times(&spec(expression, None), "2026-10-03T00:00:00Z", 2),
                ["Sun 04 18:00", "Sun 11 18:00"],
                "{expression}"
            );
        }
        // Day of month and weekday both set: either matches.
        assert_eq!(
            times(&spec("0 0 9 1 * 1", None), "2026-10-20T00:00:00Z", 3),
            ["Mon 26 09:00", "Sun 01 09:00", "Mon 02 09:00"]
        );
        // A fixed second runs once in its minute.
        assert_eq!(
            spec("30 0 9 * * *", None).next_after(at("2026-10-03T08:59:59Z")),
            Some(at("2026-10-03T09:00:30Z"))
        );
    }

    #[test]
    fn keys_have_aliases_and_a_missing_zone_is_utc() {
        assert_eq!(spec("0 9 * * *", None).timezone(), "UTC");
        for blank in [json!(""), json!("  "), json!(5)] {
            let schedule = parsed(json!({"expression": "0 9 * * *", "timezone": blank})).unwrap();
            assert_eq!(schedule.timezone(), "UTC");
        }
        for key in TIMEZONE_KEYS {
            let schedule =
                parsed(json!({"expression": "0 9 * * *", key: "Europe/Berlin"})).unwrap();
            assert_eq!(schedule.timezone(), "Europe/Berlin", "{key}");
        }
        for key in EXPRESSION_KEYS {
            let schedule = parsed(json!({key: " 0  9 * *  mon "})).unwrap();
            assert_eq!(schedule.expression(), "0 9 * * MON", "{key}");
        }
        let first = |config: Value| parsed(config).unwrap().expression().to_owned();
        assert_eq!(
            first(json!({"schedule": "0 1 * * *", "cron": "0 2 * * *"})),
            "0 2 * * *"
        );
        assert_eq!(
            first(json!({"cron_expression": "0 4 * * *", "expression": "0 3 * * *"})),
            "0 3 * * *"
        );
        assert_eq!(
            first(json!({"expression": "", "cron": "0 5 * * *", "scheduled_for": null})),
            "0 5 * * *"
        );
    }

    #[test]
    fn syntax_that_readers_of_the_dialect_read_differently_is_invalid() {
        for invalid in [
            "0 9 * *",
            "0 0 9 * * * 2027",
            "0 0 9 ? * *",
            "0 0 9 L * *",
            "0 0 9 * * 1#2",
            "0 0 9 15W * *",
            "@daily",
            "0 MON * * *",
            "0 0 9 JAN * *",
            "0 0 9 * MON JAN",
            "0 0 9 * * 1,,2",
            "61 * * * *",
            "0 0 0 30 2 *",
            "0 0 9 * * MONDAY",
        ] {
            assert_eq!(
                code(json!({"expression": invalid})),
                Some("schedule_invalid"),
                "{invalid}"
            );
        }
        assert_eq!(
            code(json!({"expression": "0 9 * * *", "timezone": "Mars/Olympus"})),
            Some("schedule_invalid")
        );
        assert_eq!(
            code(json!({"expression": format!("0 9 * * {}", "1,".repeat(64))})),
            Some("schedule_invalid")
        );
        // The sentence names the value that was unexpected.
        let sentence = parsed(json!({"expression": "0 0 9 ? * *"}))
            .unwrap_err()
            .to_string();
        assert!(
            sentence.contains("day-of-month") && sentence.contains("\"?\""),
            "{sentence}"
        );
        let zone = parsed(json!({"expression": "0 9 * * *", "tz": "Mars/Olympus"}))
            .unwrap_err()
            .to_string();
        assert!(zone.contains("Mars/Olympus"), "{zone}");
    }

    #[test]
    fn frequent_one_time_and_missing_schedules_have_their_own_codes() {
        for often in [
            "*/30 * * * * *",
            "* * * * * *",
            "0,30 * * * * *",
            "0/1 * * * * *",
            "0-5 * * * * *",
        ] {
            assert_eq!(
                code(json!({"expression": often})),
                Some("schedule_too_often"),
                "{often}"
            );
        }
        let one_time = json!({"date": "2026-12-24", "time": "18:00"});
        // Only a build without the one-time part refuses it for being one-time.
        let once = (!crate::event_kind::SCHEDULED_ONCE).then_some("schedule_once");
        for key in ONE_TIME_KEYS {
            assert_eq!(code(json!({key: one_time})), once, "{key}");
            assert_eq!(
                code(json!({key: one_time, "expression": "0 9 * * *"})),
                Some("schedule_invalid"),
                "{key}"
            );
        }
        assert_eq!(
            code(json!({"scheduled_for": {"date": "2026-12-24"}, "expression": "0 9 * * *"})),
            None
        );
        for missing in [
            json!({}),
            json!({"expression": ""}),
            json!({"expression": "   "}),
            json!({"expression": null}),
            json!({"expression": 5}),
            json!([]),
            json!("0 9 * * *"),
        ] {
            assert_eq!(code(missing.clone()), Some("schedule_missing"), "{missing}");
        }
        assert_eq!(
            ScheduleSpec::parse(b"", at("2026-10-03T00:00:00Z"))
                .unwrap_err()
                .code(),
            "schedule_missing"
        );
    }

    #[test]
    fn only_an_object_payload_reaches_a_run() {
        let payload = |payload: Value| {
            parsed(json!({"expression": "0 9 * * *", "payload": payload}))
                .unwrap()
                .payload
        };
        assert_eq!(
            payload(json!({"region": "eu"})),
            Some(json!({"region": "eu"}))
        );
        for none in [json!("text"), json!(null), json!([1]), json!(7)] {
            assert_eq!(payload(none), None);
        }
        assert_eq!(spec("0 9 * * *", None).payload, None);
    }

    /// A one-time spec as a build with the one-time part reads it.
    fn once_config(config: Value) -> Result<ScheduleSpec, ScheduleProblem> {
        ScheduleSpec::parse_with(
            &serde_json::to_vec(&config).unwrap(),
            at("2026-10-03T00:00:00Z"),
            true,
        )
    }

    fn once_spec(
        date: &str,
        time: &str,
        zone: Option<&str>,
    ) -> Result<ScheduleSpec, ScheduleProblem> {
        once_config(json!({"scheduled_for": {"date": date, "time": time}, "timezone": zone}))
    }

    /// The shared table of the one-time rule; the client test carries it literally. An empty
    /// zone saves none. A time that has passed is valid: whether it passed is state.
    const ONCE_TABLE: [(&str, &str, &str, &str); 12] = [
        (
            "2026-08-15",
            "09:30",
            "Europe/Berlin",
            "2026-08-15T07:30:00Z",
        ),
        (
            "2026-10-25",
            "02:30",
            "Europe/Berlin",
            "2026-10-25T00:30:00Z",
        ),
        ("2027-03-28", "02:30", "Europe/Berlin", "schedule_invalid"),
        (
            "2027-03-28",
            "03:00",
            "Europe/Berlin",
            "2027-03-28T01:00:00Z",
        ),
        (
            "2026-10-04",
            "02:15",
            "Australia/Lord_Howe",
            "schedule_invalid",
        ),
        ("2026-08-15", "09:30", "", "2026-08-15T09:30:00Z"),
        ("2026-02-30", "09:30", "", "schedule_invalid"),
        ("2026-08-15", "9:30", "", "schedule_invalid"),
        ("2026-08-15", "09:30:00", "", "schedule_invalid"),
        ("2026-08-15", "24:00", "", "schedule_invalid"),
        ("1999-12-31", "23:59", "", "schedule_invalid"),
        ("2001-01-01", "00:00", "", "2001-01-01T00:00:00Z"),
    ];

    #[test]
    fn a_one_time_schedule_runs_at_its_local_time_and_a_time_that_does_not_exist_is_invalid() {
        for (date, time, zone, expected) in ONCE_TABLE {
            let parsed = once_spec(date, time, Some(zone).filter(|zone| !zone.is_empty()));
            let row = format!("{date} {time} {zone}");
            if expected == "schedule_invalid" {
                assert_eq!(parsed.unwrap_err().code(), expected, "{row}");
                continue;
            }
            let spec = parsed.unwrap();
            let timezone = if zone.is_empty() { "UTC" } else { zone };
            let once = OnceAt {
                date: date.into(),
                time: time.into(),
                at: secs(expected),
                timezone,
            };
            assert_eq!(spec.once(), Some(once), "{row}");
            assert_eq!(spec.expression(), "");
        }
        assert!(spec("0 9 * * *", None).once().is_none());
    }

    #[test]
    fn a_one_time_problem_names_the_value_that_was_unexpected() {
        let gap = once_spec("2027-03-28", "02:30", Some("Europe/Berlin"))
            .unwrap_err()
            .to_string();
        for named in ["02:30", "2027-03-28", "Europe/Berlin"] {
            assert!(gap.contains(named), "{gap}");
        }
        let zone = once_spec("2026-08-15", "09:30", Some("Mars/Olympus"))
            .unwrap_err()
            .to_string();
        assert!(zone.contains("Mars/Olympus"), "{zone}");
        for (date, time, named) in [
            ("2026-02-30", "09:30", "\"2026-02-30\""),
            ("2026-08-15", "9:30", "\"9:30\""),
        ] {
            let sentence = once_spec(date, time, None).unwrap_err().to_string();
            assert!(sentence.contains(named), "{sentence}");
        }
    }

    #[test]
    fn a_one_time_schedule_is_read_under_either_key_and_found_by_discovery() {
        let one_time = json!({"date": "2026-09-24", "time": "09:00"});
        for key in ONE_TIME_KEYS {
            let spec = once_config(json!({key: one_time, "tz": "Europe/Berlin",
                "payload": {"batch": 1}}))
            .unwrap();
            // The discovery row's `once`, as the client parses it.
            assert_eq!(
                json!(spec.once().unwrap()),
                json!({"date":"2026-09-24","time":"09:00","at":1790233200,"timezone":"Europe/Berlin"}),
                "{key}"
            );
            assert_eq!(spec.payload, Some(json!({"batch": 1})));
            assert_eq!(
                spec.next_after(at("2026-09-24T06:59:59Z")),
                Some(at("2026-09-24T07:00:00Z"))
            );
            assert_eq!(spec.next_after(at("2026-09-24T07:00:00Z")), None);
        }
        assert_eq!(
            once_config(json!({"scheduled_for": one_time, "cron": "0 9 * * *"}))
                .unwrap_err()
                .code(),
            "schedule_invalid"
        );
        assert_eq!(
            once_config(json!({"scheduled_for": {"date": "2026-09-24", "time": 900}}))
                .unwrap_err()
                .code(),
            "schedule_missing"
        );
    }

    #[test]
    fn daylight_saving_runs_a_repeated_time_once_and_a_missing_time_after_the_gap() {
        let nightly = spec("0 30 2 * * *", Some("Europe/Berlin"));
        // Clocks go back on 2026-10-25: 02:30 occurs twice and runs at its first pass.
        assert_eq!(
            utc_times(&nightly, "2026-10-24T12:00:00Z", 2),
            ["25 00:30", "26 01:30"]
        );
        // Asked during the second pass, the answer is never a time that has passed.
        assert_eq!(
            nightly.next_after(at("2026-10-25T01:15:00Z")),
            Some(at("2026-10-26T01:30:00Z"))
        );
        // Clocks go forward on 2027-03-28: 02:30 does not exist and runs at 03:00 local.
        assert_eq!(
            utc_times(&nightly, "2027-03-27T12:00:00Z", 2),
            ["28 01:00", "29 00:30"]
        );
        assert_eq!(times(&nightly, "2027-03-27T12:00:00Z", 1), ["Sun 28 03:00"]);
        // An "every N minutes" schedule does not run during the repeated hour.
        let half_hourly = spec("*/30 * * * *", Some("Europe/Berlin"));
        assert_eq!(
            utc_times(&half_hourly, "2026-10-24T23:10:00Z", 4),
            ["24 23:30", "25 00:00", "25 00:30", "25 02:00"]
        );
        let every_minute = spec("* * * * *", Some("Europe/Berlin"));
        assert_eq!(
            every_minute.next_after(at("2026-10-25T01:15:30Z")),
            Some(at("2026-10-25T02:00:00Z"))
        );
    }

    struct ManualClock(AtomicI64);

    impl ManualClock {
        fn set(&self, text: &str) {
            self.0.store(secs(text) * 1000, Ordering::SeqCst);
        }
    }

    impl Clock for ManualClock {
        fn now_ms(&self) -> i64 {
            self.0.load(Ordering::SeqCst)
        }
    }

    /// A wall clock that moves with the test runtime's paused time.
    struct TickingClock {
        base_ms: i64,
        started: Instant,
    }

    impl Clock for TickingClock {
        fn now_ms(&self) -> i64 {
            self.base_ms + self.started.elapsed().as_millis() as i64
        }
    }

    #[derive(Clone, Copy)]
    enum Mode {
        Succeed,
        Fail,
        UntilCancelled,
        Gated,
    }

    struct Runner {
        mode: Mutex<Mode>,
        starts: Mutex<Vec<String>>,
        gate: tokio::sync::Semaphore,
        state_dir: PathBuf,
        /// What the state file held for the event when its run started.
        watermarks: Mutex<Vec<Option<i64>>>,
        once_states: Mutex<Vec<Value>>,
    }

    impl Runner {
        fn starts(&self) -> Vec<String> {
            self.starts.lock().unwrap().clone()
        }

        fn set(&self, mode: Mode) {
            *self.mode.lock().unwrap() = mode;
        }
    }

    #[async_trait]
    impl ScheduleRunner for Runner {
        async fn run(&self, event_id: &str, _: &str, cancel: CancellationToken) -> bool {
            self.starts.lock().unwrap().push(event_id.to_owned());
            let file = stored(&self.state_dir);
            self.watermarks
                .lock()
                .unwrap()
                .push(file["events"][event_id]["watermark"].as_i64());
            self.once_states
                .lock()
                .unwrap()
                .push(file["events"][event_id]["once_state"].clone());
            let mode = *self.mode.lock().unwrap();
            match mode {
                Mode::Succeed => true,
                Mode::Fail => false,
                Mode::UntilCancelled => {
                    cancel.cancelled().await;
                    false
                }
                Mode::Gated => {
                    self.gate.acquire().await.unwrap().forget();
                    true
                }
            }
        }
    }

    /// `None` stands for a parent that answers `unavailable`.
    struct Parent {
        answer: Mutex<Option<Vec<(String, String)>>>,
        asked: AtomicUsize,
        named: Mutex<Vec<Vec<String>>>,
    }

    #[async_trait]
    impl ScheduleArbiter for Parent {
        async fn hold(&self, ids: &[String]) -> Result<Vec<(String, String)>> {
            self.asked.fetch_add(1, Ordering::SeqCst);
            self.named.lock().unwrap().push(ids.to_vec());
            self.answer.lock().unwrap().clone().context("unavailable")
        }
    }

    enum HubAnswer {
        /// Every schedule asked for is claimed, since this time.
        Claimed(i64),
        Mixed(Vec<(&'static str, i64)>, Vec<(&'static str, HoldReason)>),
        Fails(ClaimError),
    }

    struct Hub {
        answer: Mutex<HubAnswer>,
        calls: Mutex<Vec<Vec<String>>>,
    }

    impl Hub {
        fn answers(&self, answer: HubAnswer) {
            *self.answer.lock().unwrap() = answer;
        }

        fn calls(&self) -> Vec<Vec<String>> {
            self.calls.lock().unwrap().clone()
        }
    }

    #[async_trait]
    impl ScheduleClaims for Hub {
        async fn claim(&self, ids: &[String]) -> Result<ClaimOutcome, ClaimError> {
            self.calls.lock().unwrap().push(ids.to_vec());
            match &*self.answer.lock().unwrap() {
                HubAnswer::Claimed(since) => Ok(ClaimOutcome {
                    claimed: ids.iter().map(|id| (id.clone(), *since)).collect(),
                    held: Vec::new(),
                }),
                HubAnswer::Mixed(claimed, held) => Ok(ClaimOutcome {
                    claimed: claimed
                        .iter()
                        .map(|(id, since)| ((*id).to_owned(), *since))
                        .collect(),
                    held: held
                        .iter()
                        .map(|(id, reason)| ((*id).to_owned(), *reason))
                        .collect(),
                }),
                HubAnswer::Fails(error) => Err(*error),
            }
        }
    }

    fn stored(state_dir: &Path) -> Value {
        std::fs::read(state_dir.join(STATE_FILE))
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default()
    }

    async fn settle() {
        for _ in 0..50 {
            tokio::task::yield_now().await;
        }
    }

    struct Fixture {
        directory: tempfile::TempDir,
        clock: Arc<ManualClock>,
        runner: Arc<Runner>,
        parent: Option<Arc<Parent>>,
        hub: Option<Arc<Hub>>,
        grant: &'static str,
    }

    impl Fixture {
        fn new(now: &str) -> Self {
            let directory = tempfile::tempdir().unwrap();
            let state_dir = directory.path().join(STATE_DIRECTORY).join("placement");
            Self::in_directory(now, directory, state_dir)
        }

        fn in_directory(now: &str, directory: tempfile::TempDir, state_dir: PathBuf) -> Self {
            let clock = Arc::new(ManualClock(AtomicI64::new(0)));
            clock.set(now);
            Self {
                directory,
                clock,
                runner: Arc::new(Runner {
                    mode: Mutex::new(Mode::Succeed),
                    starts: Mutex::default(),
                    gate: tokio::sync::Semaphore::new(0),
                    state_dir,
                    watermarks: Mutex::default(),
                    once_states: Mutex::default(),
                }),
                parent: None,
                hub: None,
                grant: "grant",
            }
        }

        fn online(mut self, answer: HubAnswer) -> Self {
            self.hub = Some(Arc::new(Hub {
                answer: Mutex::new(answer),
                calls: Mutex::default(),
            }));
            self
        }

        fn with_parent(mut self, answer: Option<Vec<(&str, &str)>>) -> Self {
            self.parent = Some(Arc::new(Parent {
                answer: Mutex::new(None),
                asked: AtomicUsize::new(0),
                named: Mutex::default(),
            }));
            self.parent_answers(answer);
            self
        }

        fn parent_answers(&self, answer: Option<Vec<(&str, &str)>>) {
            *self.parent.as_ref().unwrap().answer.lock().unwrap() = answer.map(|held| {
                held.into_iter()
                    .map(|(event, placement)| (event.to_owned(), placement.to_owned()))
                    .collect()
            });
        }

        fn hub(&self) -> &Hub {
            self.hub.as_ref().unwrap()
        }

        fn state_dir(&self) -> PathBuf {
            self.runner.state_dir.clone()
        }

        fn context(&self, intent_revision: u64, clock: Arc<dyn Clock>) -> ScheduleContext {
            ScheduleContext {
                state_dir: self.state_dir(),
                placement_id: "placement".into(),
                project_id: "project".into(),
                config_revision: 3,
                intent_revision,
                grant_id: self.hub.as_ref().map(|_| self.grant.to_owned()),
                arbiter: self
                    .parent
                    .clone()
                    .map(|parent| parent as Arc<dyn ScheduleArbiter>),
                claims: self.hub.clone().map(|hub| hub as Arc<dyn ScheduleClaims>),
                clock,
            }
        }

        fn scheduler(&self, intent_revision: u64, events: &[(&str, &str)]) -> Scheduler {
            prepare(
                self.context(intent_revision, self.clock.clone()),
                events
                    .iter()
                    .map(|(id, expression)| ((*id).to_owned(), spec(expression, None)))
                    .collect(),
            )
            .unwrap()
        }

        fn with_specs(&self, intent_revision: u64, specs: Vec<(&str, ScheduleSpec)>) -> Scheduler {
            prepare(
                self.context(intent_revision, self.clock.clone()),
                specs
                    .into_iter()
                    .map(|(id, spec)| (id.to_owned(), spec))
                    .collect(),
            )
            .unwrap()
        }

        /// A scheduler for `events` and the gated events `gated`, with their gates.
        fn gated(
            &self,
            intent_revision: u64,
            events: &[(&str, &str)],
            gated: &[&str],
        ) -> (Scheduler, ClaimGates) {
            let gates = ClaimGates::new(gated.iter().map(|id| (*id).to_owned()).collect());
            let scheduler = prepare_with_gates(
                self.context(intent_revision, self.clock.clone()),
                events
                    .iter()
                    .map(|(id, expression)| ((*id).to_owned(), spec(expression, None)))
                    .collect(),
                gates.clone(),
            )
            .unwrap();
            (scheduler, gates)
        }

        fn runner(&self) -> Arc<dyn ScheduleRunner> {
            self.runner.clone()
        }

        /// One pass of the scheduler loop at `now`, and time for started runs to get going.
        async fn pass(&self, scheduler: &mut Scheduler, now: &str) {
            self.clock.set(now);
            scheduler.pass(&self.runner()).await;
            settle().await;
        }
    }

    const EVERY_MINUTE: &str = "* * * * *";

    #[tokio::test(start_paused = true)]
    async fn nothing_runs_at_start_and_a_time_is_recorded_before_it_runs() {
        let fixture = Fixture::new("2026-10-05T10:00:05Z");
        let mut scheduler = fixture.scheduler(1, &[("report", EVERY_MINUTE)]);
        assert_eq!(stored(&fixture.state_dir())["decided"], false);
        assert!(scheduler.ask().await);
        let first = secs("2026-10-05T10:01:00Z");
        assert_eq!(scheduler.events["report"].next_at, Some(first));
        fixture.pass(&mut scheduler, "2026-10-05T10:00:59Z").await;
        assert!(fixture.runner.starts().is_empty());
        let file = stored(&fixture.state_dir());
        assert_eq!(file["decided"], true);
        assert_eq!(file["events"]["report"]["next_at"], first);

        fixture.pass(&mut scheduler, "2026-10-05T10:01:00Z").await;
        assert_eq!(fixture.runner.starts(), ["report"]);
        assert_eq!(*fixture.runner.watermarks.lock().unwrap(), [Some(first)]);
        // The same pass again, and the finished run, start nothing new.
        fixture.pass(&mut scheduler, "2026-10-05T10:01:00Z").await;
        fixture.pass(&mut scheduler, "2026-10-05T10:01:30Z").await;
        assert_eq!(fixture.runner.starts().len(), 1);
        let entry = &scheduler.events["report"];
        assert_eq!((entry.runs, entry.failed), (1, 0));
        assert_eq!(entry.running_since, None);
        assert_eq!(
            entry.last,
            Some(LastRun {
                at: first,
                finished_at: secs("2026-10-05T10:01:00Z"),
                outcome: Outcome::Succeeded
            })
        );
        assert_eq!(entry.next_at, Some(first + 60));
    }

    #[tokio::test(start_paused = true)]
    async fn a_time_found_too_late_is_missed_and_one_found_in_time_runs() {
        let fixture = Fixture::new("2026-10-05T10:00:05Z");
        let mut scheduler = fixture.scheduler(1, &[("report", EVERY_MINUTE)]);
        scheduler.ask().await;
        // 61 seconds late, for example after the device was asleep.
        fixture.pass(&mut scheduler, "2026-10-05T10:02:01Z").await;
        assert!(fixture.runner.starts().is_empty());
        let entry = &scheduler.events["report"];
        assert_eq!(entry.skipped_missed, 1);
        assert_eq!(
            entry.last_skip,
            Some(LastSkip {
                at: secs("2026-10-05T10:01:00Z"),
                reason: SkipReason::Missed
            })
        );
        assert_eq!(entry.next_at, Some(secs("2026-10-05T10:03:00Z")));
        // 59 seconds late still runs.
        fixture.pass(&mut scheduler, "2026-10-05T10:03:59Z").await;
        assert_eq!(fixture.runner.starts().len(), 1);
        // A clock set forward skips what lies in between, counted once.
        fixture.pass(&mut scheduler, "2026-10-05T14:30:10Z").await;
        assert_eq!(fixture.runner.starts().len(), 1);
        assert_eq!(scheduler.events["report"].skipped_missed, 2);
        assert_eq!(
            scheduler.events["report"].next_at,
            Some(secs("2026-10-05T14:31:00Z"))
        );
    }

    #[tokio::test(start_paused = true)]
    async fn overlapping_and_surplus_runs_are_skipped_and_counted() {
        let fixture = Fixture::new("2026-10-05T10:00:05Z");
        fixture.runner.set(Mode::Gated);
        let ids: Vec<String> = (1..=9).map(|index| format!("event-{index}")).collect();
        let events: Vec<(&str, &str)> = ids.iter().map(|id| (id.as_str(), EVERY_MINUTE)).collect();
        let mut scheduler = fixture.scheduler(1, &events);
        scheduler.ask().await;
        fixture.pass(&mut scheduler, "2026-10-05T10:01:00Z").await;
        assert_eq!(fixture.runner.starts().len(), MAX_CONCURRENT_RUNS);
        assert_eq!(scheduler.events["event-9"].skipped_busy, 1);
        assert_eq!(scheduler.events["event-9"].runs, 0);
        // The previous runs are still going at the next time.
        fixture.pass(&mut scheduler, "2026-10-05T10:02:00Z").await;
        assert_eq!(fixture.runner.starts().len(), MAX_CONCURRENT_RUNS);
        assert_eq!(scheduler.events["event-1"].skipped_overlap, 1);
        assert_eq!(
            scheduler.events["event-1"].running_since,
            Some(secs("2026-10-05T10:01:00Z"))
        );
        assert_eq!(scheduler.events["event-9"].skipped_busy, 2);
        fixture.runner.gate.add_permits(MAX_CONCURRENT_RUNS);
        settle().await;
        fixture.pass(&mut scheduler, "2026-10-05T10:03:00Z").await;
        assert_eq!(fixture.runner.starts().len(), 2 * MAX_CONCURRENT_RUNS);
        assert_eq!(scheduler.events["event-1"].runs, 2);
    }

    #[tokio::test(start_paused = true)]
    async fn a_time_that_cannot_be_recorded_does_not_run_and_counts_as_failed() {
        let fixture = Fixture::new("2026-10-05T10:00:05Z");
        let mut scheduler = fixture.scheduler(1, &[("report", EVERY_MINUTE)]);
        scheduler.ask().await;
        fixture.pass(&mut scheduler, "2026-10-05T10:00:10Z").await;
        std::fs::remove_dir_all(fixture.state_dir()).unwrap();
        std::fs::write(fixture.state_dir(), b"").unwrap();
        fixture.pass(&mut scheduler, "2026-10-05T10:01:00Z").await;
        assert!(fixture.runner.starts().is_empty());
        let entry = &scheduler.events["report"];
        assert_eq!((entry.runs, entry.failed), (1, 1));
        assert_eq!(entry.last.as_ref().unwrap().outcome, Outcome::Failed);
        assert_eq!(entry.next_at, Some(secs("2026-10-05T10:02:00Z")));
        // The next time is tried normally once the directory works again.
        std::fs::remove_file(fixture.state_dir()).unwrap();
        std::fs::create_dir(fixture.state_dir()).unwrap();
        fixture.pass(&mut scheduler, "2026-10-05T10:02:00Z").await;
        assert_eq!(fixture.runner.starts(), ["report"]);
    }

    #[tokio::test(start_paused = true)]
    async fn a_failing_run_is_counted_and_the_schedule_goes_on() {
        let fixture = Fixture::new("2026-10-05T10:00:05Z");
        fixture.runner.set(Mode::Fail);
        let mut scheduler = fixture.scheduler(1, &[("report", EVERY_MINUTE)]);
        scheduler.ask().await;
        fixture.pass(&mut scheduler, "2026-10-05T10:01:00Z").await;
        fixture.pass(&mut scheduler, "2026-10-05T10:02:00Z").await;
        fixture.pass(&mut scheduler, "2026-10-05T10:02:30Z").await;
        assert_eq!(fixture.runner.starts().len(), 2);
        let entry = &scheduler.events["report"];
        assert_eq!((entry.runs, entry.failed), (2, 2));
        assert_eq!(entry.last.as_ref().unwrap().outcome, Outcome::Failed);
        assert_eq!(entry.hold, None);
    }

    #[tokio::test(start_paused = true)]
    async fn a_run_over_the_limit_is_cancelled_as_timed_out() {
        let fixture = Fixture::new("2026-10-05T10:00:05Z");
        fixture.runner.set(Mode::UntilCancelled);
        let mut scheduler = fixture.scheduler(1, &[("report", "0 10 * * *")]);
        scheduler.ask().await;
        fixture.pass(&mut scheduler, "2026-10-06T10:00:00Z").await;
        assert_eq!(fixture.runner.starts().len(), 1);
        tokio::time::advance(RUN_LIMIT - Duration::from_secs(1)).await;
        fixture.pass(&mut scheduler, "2026-10-07T09:59:59Z").await;
        assert!(scheduler.running.contains_key("report"));
        tokio::time::advance(Duration::from_secs(1)).await;
        // The next day's time arrives while the run is cancelled: it is an overlap.
        fixture.pass(&mut scheduler, "2026-10-07T10:00:00Z").await;
        fixture.pass(&mut scheduler, "2026-10-07T10:00:01Z").await;
        let entry = &scheduler.events["report"];
        assert_eq!(entry.last.as_ref().unwrap().outcome, Outcome::TimedOut);
        assert_eq!((entry.runs, entry.failed, entry.skipped_overlap), (1, 1, 1));
        assert!(scheduler.running.is_empty());
    }

    #[tokio::test(start_paused = true)]
    async fn a_restart_never_repeats_a_time_also_after_the_clock_was_set_back() {
        let fixture = Fixture::new("2026-10-05T10:00:05Z");
        let events = [("report", EVERY_MINUTE)];
        let mut scheduler = fixture.scheduler(7, &events);
        scheduler.ask().await;
        fixture.pass(&mut scheduler, "2026-10-05T10:01:00Z").await;
        fixture.pass(&mut scheduler, "2026-10-05T10:01:05Z").await;
        drop(scheduler);
        let ran = secs("2026-10-05T10:01:00Z");

        // The same intent revision: counters go on, the time that ran does not run again.
        fixture.clock.set("2026-10-05T10:00:30Z");
        let mut restarted = fixture.scheduler(7, &events);
        restarted.ask().await;
        assert_eq!(restarted.events["report"].runs, 1);
        assert_eq!(restarted.events["report"].watermark, Some(ran));
        assert_eq!(restarted.events["report"].next_at, Some(ran + 60));
        drop(restarted);

        // The clock is set back ten minutes, then the service is started again: a new
        // intent revision resets the counters and keeps the watermark.
        fixture.clock.set("2026-10-05T09:51:30Z");
        let mut started = fixture.scheduler(8, &events);
        started.ask().await;
        let entry = &started.events["report"];
        assert_eq!((entry.runs, entry.last.clone()), (0, None));
        assert_eq!(entry.watermark, Some(ran));
        assert_eq!(entry.next_at, Some(ran + 60));
        for now in ["2026-10-05T09:52:00Z", "2026-10-05T10:01:00Z"] {
            fixture.pass(&mut started, now).await;
        }
        assert_eq!(fixture.runner.starts().len(), 1);
        fixture.pass(&mut started, "2026-10-05T10:02:00Z").await;
        assert_eq!(fixture.runner.starts().len(), 2);
    }

    #[tokio::test(start_paused = true)]
    async fn a_clock_set_back_waits_for_the_watermark_and_a_watermark_far_ahead_is_dropped() {
        let fixture = Fixture::new("2026-10-05T10:00:05Z");
        let mut scheduler = fixture.scheduler(1, &[("report", EVERY_MINUTE)]);
        scheduler.ask().await;
        fixture.pass(&mut scheduler, "2026-10-05T10:01:00Z").await;
        let ran = secs("2026-10-05T10:01:00Z");
        // Set back by an hour: nothing runs until the clock passed the watermark.
        fixture.pass(&mut scheduler, "2026-10-05T09:01:00Z").await;
        fixture.pass(&mut scheduler, "2026-10-05T09:30:00Z").await;
        assert_eq!(fixture.runner.starts().len(), 1);
        assert_eq!(scheduler.events["report"].watermark, Some(ran));
        assert_eq!(scheduler.events["report"].next_at, Some(ran + 60));
        // Set back by two days: the earlier clock was wrong, and the watermark with it.
        fixture.pass(&mut scheduler, "2026-10-03T10:00:30Z").await;
        assert_eq!(scheduler.events["report"].watermark, None);
        assert_eq!(
            scheduler.events["report"].next_at,
            Some(secs("2026-10-03T10:01:00Z"))
        );
        fixture.pass(&mut scheduler, "2026-10-03T10:01:00Z").await;
        assert_eq!(fixture.runner.starts().len(), 2);
    }

    #[tokio::test(start_paused = true)]
    async fn an_unwritable_state_directory_fails_before_ready() {
        let fixture = Fixture::new("2026-10-05T10:00:05Z");
        std::fs::write(fixture.directory.path().join(STATE_DIRECTORY), b"").unwrap();
        let error = prepare(
            fixture.context(1, fixture.clock.clone()),
            vec![("report".into(), spec(EVERY_MINUTE, None))],
        )
        .err()
        .unwrap();
        assert!(format!("{error:#}").contains("schedule state directory"));
    }

    #[tokio::test(start_paused = true)]
    async fn an_unreadable_state_file_is_replaced_and_a_foreign_one_is_carried_carefully() {
        let fixture = Fixture::new("2026-10-05T10:00:05Z");
        drop(fixture.scheduler(1, &[("report", EVERY_MINUTE)]));
        std::fs::write(fixture.state_dir().join(STATE_FILE), b"{ torn").unwrap();
        drop(fixture.scheduler(1, &[("report", EVERY_MINUTE)]));
        assert_eq!(stored(&fixture.state_dir())["version"], 1);
        // Unknown values, other events and invalid ids of a file this agent did not write.
        let foreign = json!({"version": 1, "intent_revision": 1, "grant_id": null, "events": {
            "report": {"watermark": 1791194460, "hold": "a_newer_reason", "runs": 4, "expression": "x".repeat(4000)},
            "removed": {"watermark": 5, "runs": 9},
            "../escape": {"watermark": 6}
        }});
        std::fs::write(
            fixture.state_dir().join(STATE_FILE),
            serde_json::to_vec(&foreign).unwrap(),
        )
        .unwrap();
        let mut scheduler = fixture.scheduler(1, &[("report", EVERY_MINUTE)]);
        assert_eq!(scheduler.events["report"].watermark, Some(1791194460));
        assert_eq!(scheduler.events["report"].expression, EVERY_MINUTE);
        assert_eq!(scheduler.events["removed"].watermark, Some(5));
        assert!(!scheduler.events.contains_key("../escape"));
        // Events the config no longer lists go when the process arms.
        scheduler.ask().await;
        assert_eq!(scheduler.events.keys().collect::<Vec<_>>(), ["report"]);
        let newer = json!({"version": 2, "events": {"report": {"watermark": 7}}});
        std::fs::write(
            fixture.state_dir().join(STATE_FILE),
            serde_json::to_vec(&newer).unwrap(),
        )
        .unwrap();
        assert_eq!(
            fixture.scheduler(1, &[("report", EVERY_MINUTE)]).events["report"].watermark,
            None
        );
    }

    #[tokio::test(start_paused = true)]
    async fn the_state_file_has_the_shape_the_agent_parent_reads() {
        // The literal file of the contract between the placement process and the parent.
        let literal = json!({"version":1,"config_revision":12,"intent_revision":7,"decided":true,"grant_id":"g-1",
            "events":{"evt_report":{"expression":"0 0 2 * * *","timezone":"Europe/Berlin","armed_at":1790000000,
              "watermark":1790035200,"next_at":1790121600,"hold":null,"confirmed":true,"since":1789990000,
              "running_since":null,"last":{"at":1790035200,"finished_at":1790035212,"outcome":"succeeded"},
              "runs":12,"failed":1,"skipped_overlap":0,"skipped_missed":2,"skipped_busy":0,
              "last_skip":{"at":1789948800,"reason":"missed"}}}});
        let fixture = Fixture::new("2026-10-05T10:00:05Z").online(HubAnswer::Claimed(1789990000));
        let mut scheduler = fixture.scheduler(7, &[("evt_report", EVERY_MINUTE)]);
        scheduler.context.config_revision = 12;
        scheduler.context.grant_id = Some("g-1".into());
        scheduler.decided = true;
        let entry: EventState =
            serde_json::from_value(literal["events"]["evt_report"].clone()).unwrap();
        assert_eq!(entry.last.as_ref().unwrap().outcome, Outcome::Succeeded);
        assert_eq!(entry.last_skip.as_ref().unwrap().reason, SkipReason::Missed);
        assert_eq!((entry.watermark, entry.runs), (Some(1790035200), 12));
        scheduler.events.insert("evt_report".into(), entry);
        scheduler.persist().unwrap();
        assert_eq!(stored(&fixture.state_dir()), literal);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(fixture.state_dir().join(STATE_FILE))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o600);
        }
    }

    #[test]
    fn fixed_words_of_the_state_file_are_the_ones_clients_read() {
        let words = [
            (json!(HoldReason::OtherService), "other_service"),
            (json!(HoldReason::NotReleased), "not_released"),
            (json!(HoldReason::RunsElsewhere), "runs_elsewhere"),
            (json!(HoldReason::HubUnreachable), "hub_unreachable"),
            (json!(HoldReason::HubTooOld), "hub_too_old"),
            (json!(Outcome::Succeeded), "succeeded"),
            (json!(Outcome::Failed), "failed"),
            (json!(Outcome::Cancelled), "cancelled"),
            (json!(Outcome::TimedOut), "timed_out"),
            (json!(SkipReason::Overlap), "overlap"),
            (json!(SkipReason::Missed), "missed"),
            (json!(SkipReason::Busy), "busy"),
        ];
        for (value, word) in words {
            assert_eq!(value, word);
        }
        for reason in [
            HoldReason::OtherService,
            HoldReason::NotReleased,
            HoldReason::RunsElsewhere,
            HoldReason::HubUnreachable,
            HoldReason::HubTooOld,
        ] {
            assert_eq!(json!(reason), reason.as_str());
        }
    }

    #[tokio::test(start_paused = true)]
    async fn the_agent_parent_reports_what_the_running_process_wrote() {
        use crate::diagnostics::{Detail, Diagnostics, Rows, test_support};
        let root = tempfile::tempdir().unwrap();
        let mut record = test_support::running_record(&["report"]);
        record.config_revision = 3;
        record.replicas[0].config_revision = 3;
        let state_dir = test_support::schedule_state_dir(root.path(), &record.id);
        let data_root = root.path().join("placement-data/api/current/store");
        assert_eq!(state_dir, data_root.join(STATE_DIRECTORY).join("api"));
        std::fs::create_dir_all(&data_root).unwrap();
        let reported = |root: &Path| {
            Rows::new(&Diagnostics::default(), root, false)
                .placement(&record, false, Detail::Full)
                .get("schedules")
                .cloned()
        };
        let path = root.path().to_path_buf();
        let fixture = Fixture::in_directory("2026-09-01T10:00:05Z", root, state_dir);
        let mut scheduler = fixture.scheduler(1, &[("report", EVERY_MINUTE)]);
        // Prepared, but not decided: the parent reports nothing yet.
        assert_eq!(reported(&path), None);
        scheduler.ask().await;
        fixture.pass(&mut scheduler, "2026-09-01T10:01:00Z").await;
        fixture.pass(&mut scheduler, "2026-09-01T10:01:02Z").await;
        let ran = secs("2026-09-01T10:01:00Z");
        assert_eq!(
            reported(&path),
            Some(
                json!([{"event_id":"report","expression":"* * * * *","timezone":"UTC","hold":null,
                "last_outcome":"succeeded","next_at":ran + 60,"running":false,"last_at":ran,
                "runs":1,"failed":0,"skipped":0,"last_skip":null,"clock_behind":false}])
            )
        );
    }

    #[tokio::test(start_paused = true)]
    async fn the_agent_parent_reports_a_one_time_schedule_and_its_end_after_a_stop() {
        use crate::diagnostics::{Detail, Diagnostics, Rows, test_support};
        let root = tempfile::tempdir().unwrap();
        let mut record = test_support::running_record(&["once"]);
        record.config_revision = 3;
        record.replicas[0].config_revision = 3;
        let state_dir = test_support::schedule_state_dir(root.path(), &record.id);
        std::fs::create_dir_all(root.path().join("placement-data/api/current/store")).unwrap();
        let reported = |root: &Path, record: &crate::state::PlacementRecord| {
            Rows::new(&Diagnostics::default(), root, false)
                .placement(record, false, Detail::Full)
                .get("schedules")
                .cloned()
        };
        let path = root.path().to_path_buf();
        let fixture = Fixture::in_directory("2026-10-05T10:00:05Z", root, state_dir);
        // The test record pins its events at 1.0.0.
        let pinned = ScheduleSpec {
            event_version: (1, 0, 0),
            ..once("10:01")
        };
        let mut scheduler = fixture.with_specs(1, vec![("once", pinned)]);
        scheduler.ask().await;
        fixture.pass(&mut scheduler, "2026-10-05T10:00:10Z").await;
        let at = secs("2026-10-05T10:01:00Z");
        assert_eq!(
            reported(&path, &record),
            Some(
                json!([{"event_id":"once","once_at":at,"timezone":"UTC","once_state":"pending",
                "hold":null,"next_at":at,"running":false,"last_at":null,"last_outcome":null}])
            )
        );
        fixture.pass(&mut scheduler, "2026-10-05T10:01:00Z").await;
        fixture.pass(&mut scheduler, "2026-10-05T10:01:04Z").await;
        let ran = json!([{"event_id":"once","once_at":at,"timezone":"UTC","once_state":"ran",
            "hold":null,"next_at":null,"running":false,"last_at":at,"last_outcome":"succeeded"}]);
        assert_eq!(reported(&path, &record), Some(ran.clone()));
        scheduler.drain().await;
        // A finished one-time schedule is a fact about the past: also reported when stopped.
        let mut stopped = record.clone();
        stopped.replicas[0].observed_state = crate::state::ObservedState::Stopped;
        assert_eq!(reported(&path, &stopped), Some(ran));
    }

    #[tokio::test(start_paused = true)]
    async fn the_task_waits_for_ready_and_stop_cuts_a_run_off_after_its_grace() {
        let fixture = Fixture::new("2026-10-05T10:00:30Z");
        fixture.runner.set(Mode::UntilCancelled);
        let clock = Arc::new(TickingClock {
            base_ms: secs("2026-10-05T10:00:30Z") * 1000,
            started: Instant::now(),
        });
        let scheduler = prepare(
            fixture.context(1, clock),
            vec![("report".into(), spec(EVERY_MINUTE, None))],
        )
        .unwrap();
        let (ready, receiver) = oneshot::channel();
        let stop = CancellationToken::new();
        let task = tokio::spawn(scheduler.run(fixture.runner(), receiver, stop.clone()));
        // 10:01:00 and 10:02:00 pass before the service is ready.
        tokio::time::sleep(Duration::from_secs(95)).await;
        assert!(fixture.runner.starts().is_empty());
        assert_eq!(stored(&fixture.state_dir())["decided"], false);
        // Ready at 10:02:05: the first time after that is 10:03:00.
        ready.send(()).unwrap();
        tokio::time::sleep(Duration::from_secs(50)).await;
        assert!(fixture.runner.starts().is_empty());
        tokio::time::sleep(Duration::from_secs(6)).await;
        assert_eq!(fixture.runner.starts(), ["report"]);
        let file = stored(&fixture.state_dir());
        assert_eq!(
            file["events"]["report"]["running_since"],
            secs("2026-10-05T10:03:00Z")
        );
        let stopping = Instant::now();
        stop.cancel();
        task.await.unwrap().unwrap();
        assert!(stopping.elapsed() >= STOP_GRACE);
        assert!(stopping.elapsed() < STOP_GRACE + CANCEL_GRACE);
        let file = stored(&fixture.state_dir());
        assert_eq!(file["events"]["report"]["last"]["outcome"], "cancelled");
        assert_eq!(file["events"]["report"]["running_since"], Value::Null);
        assert_eq!(fixture.runner.starts().len(), 1);
    }

    #[tokio::test(start_paused = true)]
    async fn a_parent_that_does_not_answer_arms_nothing_and_is_asked_again() {
        let fixture = Fixture::new("2026-10-05T10:00:30Z").with_parent(None);
        let clock = Arc::new(TickingClock {
            base_ms: secs("2026-10-05T10:00:30Z") * 1000,
            started: Instant::now(),
        });
        let scheduler = prepare(
            fixture.context(1, clock),
            vec![("report".into(), spec(EVERY_MINUTE, None))],
        )
        .unwrap();
        let (ready, receiver) = oneshot::channel();
        let stop = CancellationToken::new();
        let task = tokio::spawn(scheduler.run(fixture.runner(), receiver, stop.clone()));
        ready.send(()).unwrap();
        tokio::time::sleep(Duration::from_secs(62)).await;
        let asked = fixture
            .parent
            .as_ref()
            .unwrap()
            .asked
            .load(Ordering::SeqCst);
        assert_eq!(asked, 13, "once at ready, then every five seconds");
        assert!(fixture.runner.starts().is_empty());
        assert_eq!(stored(&fixture.state_dir())["decided"], false);
        fixture.parent_answers(Some(Vec::new()));
        tokio::time::sleep(Duration::from_secs(4)).await;
        assert_eq!(stored(&fixture.state_dir())["decided"], true);
        tokio::time::sleep(Duration::from_secs(60)).await;
        assert_eq!(fixture.runner.starts(), ["report"]);
        stop.cancel();
        task.await.unwrap().unwrap();
    }

    #[tokio::test(start_paused = true)]
    async fn the_parents_refusal_wins_before_the_hub_is_asked() {
        let now = secs("2026-10-05T10:00:05Z");
        let fixture = Fixture::new("2026-10-05T10:00:05Z")
            .online(HubAnswer::Claimed(now))
            .with_parent(Some(vec![("mail", "other-service")]));
        let mut scheduler =
            fixture.scheduler(1, &[("mail", EVERY_MINUTE), ("report", EVERY_MINUTE)]);
        assert!(scheduler.ask().await);
        assert_eq!(fixture.hub().calls(), [["report"]]);
        assert_eq!(
            scheduler.events["mail"].hold,
            Some(HoldReason::OtherService)
        );
        assert_eq!(scheduler.events["mail"].next_at, None);
        assert_eq!(scheduler.events["report"].hold, None);
        fixture.pass(&mut scheduler, "2026-10-05T10:01:00Z").await;
        assert_eq!(fixture.runner.starts(), ["report"]);
        // The other service is gone: the next question frees the schedule.
        assert_eq!(scheduler.next_ask, Some(Instant::now() + HELD_RETRY));
        fixture.parent_answers(Some(Vec::new()));
        tokio::time::advance(HELD_RETRY).await;
        fixture.pass(&mut scheduler, "2026-10-05T10:01:30Z").await;
        assert_eq!(fixture.hub().calls()[1], ["mail", "report"]);
        assert_eq!(scheduler.events["mail"].hold, None);
        // A parent that is busy later changes nothing.
        fixture.parent_answers(None);
        tokio::time::advance(CONFIRMATION).await;
        fixture.pass(&mut scheduler, "2026-10-05T10:01:40Z").await;
        assert_eq!(scheduler.events["mail"].hold, None);
        assert_eq!(fixture.hub().calls().len(), 3);
    }

    #[tokio::test(start_paused = true)]
    async fn nothing_is_armed_before_the_hub_said_claimed() {
        let fixture =
            Fixture::new("2026-10-05T10:00:05Z").online(HubAnswer::Fails(ClaimError::Unreachable));
        let mut scheduler = fixture.scheduler(1, &[("report", EVERY_MINUTE)]);
        assert!(scheduler.ask().await);
        assert_eq!(
            scheduler.events["report"].hold,
            Some(HoldReason::HubUnreachable)
        );
        assert_eq!(scheduler.next_ask, Some(Instant::now() + UNREACHABLE_RETRY));
        fixture.pass(&mut scheduler, "2026-10-05T10:01:00Z").await;
        assert!(fixture.runner.starts().is_empty());
        assert_eq!(
            stored(&fixture.state_dir())["events"]["report"]["hold"],
            "hub_unreachable"
        );
        // The retry arms it, from the time the hub handed it over.
        fixture
            .hub()
            .answers(HubAnswer::Claimed(secs("2026-10-05T10:01:45Z")));
        tokio::time::advance(UNREACHABLE_RETRY).await;
        fixture.pass(&mut scheduler, "2026-10-05T10:01:10Z").await;
        let entry = &scheduler.events["report"];
        assert_eq!((entry.hold, entry.confirmed), (None, true));
        assert_eq!(scheduler.next_ask, Some(Instant::now() + CONFIRMATION));
        // The hub's clock is ahead: 10:01:45 there is 10:01:10 here. The hub may have run
        // every time up to its 10:01:45, so this device starts with the time after it.
        assert_eq!(entry.next_at, Some(secs("2026-10-05T10:02:00Z")));
        assert_eq!(fixture.hub().calls(), [["report"], ["report"]]);
    }

    #[tokio::test(start_paused = true)]
    async fn the_hand_over_time_is_a_floor_only_for_a_fresh_claim() {
        // The device clock is 30 seconds behind the hub's, and the claim is fresh.
        let fixture = Fixture::new("2026-10-05T10:00:45Z")
            .online(HubAnswer::Claimed(secs("2026-10-05T10:01:15Z")));
        let mut scheduler = fixture.scheduler(1, &[("report", EVERY_MINUTE)]);
        scheduler.ask().await;
        fixture.pass(&mut scheduler, "2026-10-05T10:01:00Z").await;
        assert!(fixture.runner.starts().is_empty());
        fixture.pass(&mut scheduler, "2026-10-05T10:02:00Z").await;
        assert_eq!(fixture.runner.starts().len(), 1);
        drop(scheduler);
        // A restart: the approval has had the schedule since last week, and the time that
        // is ten seconds away runs.
        let fixture = Fixture::new("2026-10-05T10:00:50Z")
            .online(HubAnswer::Claimed(secs("2026-09-28T08:00:00Z")));
        let mut scheduler = fixture.scheduler(1, &[("report", EVERY_MINUTE)]);
        scheduler.ask().await;
        fixture.pass(&mut scheduler, "2026-10-05T10:01:00Z").await;
        assert_eq!(fixture.runner.starts().len(), 1);
    }

    #[tokio::test(start_paused = true)]
    async fn a_start_without_the_hub_arms_only_what_it_confirmed_for_the_same_approval() {
        let since = secs("2026-10-01T08:00:00Z");
        let mut fixture = Fixture::new("2026-10-05T10:00:05Z").online(HubAnswer::Claimed(since));
        let events = [("mail", EVERY_MINUTE), ("report", EVERY_MINUTE)];
        let mut scheduler = fixture.scheduler(1, &events);
        scheduler.ask().await;
        fixture.pass(&mut scheduler, "2026-10-05T10:00:10Z").await;
        drop(scheduler);
        fixture
            .hub()
            .answers(HubAnswer::Fails(ClaimError::Unreachable));

        let mut offline = fixture.scheduler(2, &events);
        offline.ask().await;
        let entry = &offline.events["report"];
        assert_eq!(
            (entry.hold, entry.confirmed, entry.since),
            (None, true, Some(since))
        );
        fixture.pass(&mut offline, "2026-10-05T10:01:00Z").await;
        assert_eq!(fixture.runner.starts().len(), 2);
        drop(offline);

        // A config that dropped `mail` forgets its confirmation; added again it is new.
        let mut smaller = fixture.scheduler(3, &[("report", EVERY_MINUTE)]);
        smaller.ask().await;
        fixture.pass(&mut smaller, "2026-10-05T10:01:10Z").await;
        drop(smaller);
        let mut again = fixture.scheduler(4, &events);
        again.ask().await;
        assert_eq!(again.events["report"].hold, None);
        assert_eq!(again.events["mail"].hold, Some(HoldReason::HubUnreachable));
        assert!(!again.events["mail"].confirmed);
        fixture.pass(&mut again, "2026-10-05T10:01:20Z").await;
        drop(again);

        // Another approval: what the hub confirmed for the earlier one does not count.
        fixture.grant = "renewed-grant";
        let mut renewed = fixture.scheduler(5, &events);
        renewed.ask().await;
        for id in ["mail", "report"] {
            assert_eq!(
                renewed.events[id].hold,
                Some(HoldReason::HubUnreachable),
                "{id}"
            );
        }
    }

    #[tokio::test(start_paused = true)]
    async fn hub_refusals_hold_only_what_they_name_and_set_the_next_question() {
        let since = secs("2026-10-05T10:00:00Z");
        let held = || {
            let (released, elsewhere) = (HoldReason::NotReleased, HoldReason::RunsElsewhere);
            vec![("mail", released), ("sync", elsewhere)]
        };
        let fixture = Fixture::new("2026-10-05T10:00:05Z")
            .online(HubAnswer::Mixed(vec![("report", since)], held()));
        let events = ["mail", "report", "sync", "unnamed"].map(|id| (id, EVERY_MINUTE));
        let mut scheduler = fixture.scheduler(1, &events);
        scheduler.ask().await;
        let holds = |scheduler: &Scheduler| -> Vec<Option<HoldReason>> {
            scheduler.events.values().map(|entry| entry.hold).collect()
        };
        assert_eq!(
            holds(&scheduler),
            [
                Some(HoldReason::NotReleased),
                None,
                Some(HoldReason::RunsElsewhere),
                // A schedule the answer does not name is never armed.
                Some(HoldReason::HubUnreachable),
            ]
        );
        assert_eq!(scheduler.next_ask, Some(Instant::now() + UNREACHABLE_RETRY));
        fixture.pass(&mut scheduler, "2026-10-05T10:01:00Z").await;
        assert_eq!(fixture.runner.starts(), ["report"]);

        let claimed = vec![("report", since), ("unnamed", since)];
        fixture.hub().answers(HubAnswer::Mixed(claimed, held()));
        tokio::time::advance(UNREACHABLE_RETRY).await;
        fixture.pass(&mut scheduler, "2026-10-05T10:01:10Z").await;
        assert_eq!(scheduler.events["unnamed"].hold, None);
        assert_eq!(scheduler.next_ask, Some(Instant::now() + HELD_RETRY));

        fixture
            .hub()
            .answers(HubAnswer::Fails(ClaimError::HubTooOld));
        tokio::time::advance(HELD_RETRY).await;
        fixture.pass(&mut scheduler, "2026-10-05T10:01:20Z").await;
        assert_eq!(holds(&scheduler), [Some(HoldReason::HubTooOld); 4]);
        assert!(scheduler.events.values().all(|entry| !entry.confirmed));
        assert_eq!(scheduler.next_ask, Some(Instant::now() + HUB_TOO_OLD_RETRY));
    }

    #[tokio::test(start_paused = true)]
    async fn a_confirmation_holds_what_the_hub_took_back_and_an_unreachable_hub_changes_nothing() {
        let since = secs("2026-10-05T10:00:00Z");
        let fixture = Fixture::new("2026-10-05T10:00:05Z").online(HubAnswer::Claimed(since));
        let events = [("mail", EVERY_MINUTE), ("report", EVERY_MINUTE)];
        let mut scheduler = fixture.scheduler(1, &events);
        scheduler.ask().await;
        assert_eq!(scheduler.next_ask, Some(Instant::now() + CONFIRMATION));
        let next = scheduler.events["report"].next_at;

        // A confirmation the hub cannot be reached for changes nothing, and is repeated soon.
        fixture
            .hub()
            .answers(HubAnswer::Fails(ClaimError::Unreachable));
        tokio::time::advance(CONFIRMATION).await;
        fixture.pass(&mut scheduler, "2026-10-05T10:00:20Z").await;
        for id in ["mail", "report"] {
            let entry = &scheduler.events[id];
            assert_eq!(
                (entry.hold, entry.confirmed, entry.next_at),
                (None, true, next)
            );
        }
        assert_eq!(scheduler.next_ask, Some(Instant::now() + UNREACHABLE_RETRY));

        // Someone gave `mail` back to the hub: it is held at once.
        fixture.hub().answers(HubAnswer::Mixed(
            vec![("report", since)],
            vec![("mail", HoldReason::NotReleased)],
        ));
        tokio::time::advance(UNREACHABLE_RETRY).await;
        fixture.pass(&mut scheduler, "2026-10-05T10:00:30Z").await;
        let mail = &scheduler.events["mail"];
        assert_eq!(
            (mail.hold, mail.confirmed, mail.since, mail.next_at),
            (Some(HoldReason::NotReleased), false, None, None)
        );
        assert_eq!(scheduler.events["report"].next_at, next);
        fixture.pass(&mut scheduler, "2026-10-05T10:01:00Z").await;
        assert_eq!(fixture.runner.starts(), ["report"]);

        // A refused call holds everything.
        fixture.hub().answers(HubAnswer::Fails(ClaimError::Denied));
        tokio::time::advance(HELD_RETRY).await;
        fixture.pass(&mut scheduler, "2026-10-05T10:01:10Z").await;
        for id in ["mail", "report"] {
            let entry = &scheduler.events[id];
            assert_eq!(
                (entry.hold, entry.confirmed),
                (Some(HoldReason::NotReleased), false),
                "{id}"
            );
        }
        fixture.pass(&mut scheduler, "2026-10-05T10:02:00Z").await;
        assert_eq!(fixture.runner.starts().len(), 1);
    }

    #[tokio::test(start_paused = true)]
    async fn an_offline_placement_asks_no_hub_and_arms_at_once() {
        let fixture = Fixture::new("2026-10-05T10:00:05Z");
        let mut scheduler = fixture.scheduler(1, &[("report", EVERY_MINUTE)]);
        assert!(scheduler.ask().await);
        let entry = &scheduler.events["report"];
        assert_eq!(
            (entry.hold, entry.confirmed, entry.since),
            (None, false, None)
        );
        assert_eq!(entry.armed_at, Some(secs("2026-10-05T10:00:05Z")));
        assert_eq!(scheduler.next_ask, None);
        assert_eq!(stored(&fixture.state_dir())["grant_id"], Value::Null);
    }

    #[tokio::test(start_paused = true)]
    async fn a_placement_without_schedules_hands_everything_back_until_the_hub_answered() {
        let since = secs("2026-10-05T10:00:00Z");
        let fixture = Fixture::new("2026-10-05T10:00:05Z").online(HubAnswer::Claimed(since));
        let mut scheduler = fixture.scheduler(1, &[("report", EVERY_MINUTE)]);
        scheduler.ask().await;
        fixture.pass(&mut scheduler, "2026-10-05T10:00:10Z").await;
        drop(scheduler);
        assert!(fixture.state_dir().join(STATE_FILE).exists());

        fixture
            .hub()
            .answers(HubAnswer::Fails(ClaimError::Unreachable));
        let (ready, receiver) = oneshot::channel();
        let stop = CancellationToken::new();
        let task = tokio::spawn(hand_back_all(
            fixture.context(2, fixture.clock.clone()),
            receiver,
            stop.clone(),
        ));
        tokio::time::sleep(Duration::from_secs(30)).await;
        assert_eq!(fixture.hub().calls().len(), 1, "nothing before ready");
        ready.send(()).unwrap();
        tokio::time::sleep(Duration::from_secs(150)).await;
        assert_eq!(fixture.hub().calls().len(), 4);
        assert!(fixture.hub().calls()[1..].iter().all(Vec::is_empty));
        assert!(!fixture.state_dir().join(STATE_FILE).exists());
        fixture.hub().answers(HubAnswer::Claimed(since));
        tokio::time::timeout(Duration::from_secs(61), task)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(fixture.hub().calls().len(), 5);

        // A schedule that comes back after that starts unconfirmed.
        fixture
            .hub()
            .answers(HubAnswer::Fails(ClaimError::Unreachable));
        let mut again = fixture.scheduler(3, &[("report", EVERY_MINUTE)]);
        again.ask().await;
        assert_eq!(
            again.events["report"].hold,
            Some(HoldReason::HubUnreachable)
        );
    }

    #[tokio::test(start_paused = true)]
    async fn a_hub_that_is_too_old_or_refuses_is_told_once() {
        for refusal in [ClaimError::HubTooOld, ClaimError::Denied] {
            let fixture = Fixture::new("2026-10-05T10:00:05Z").online(HubAnswer::Fails(refusal));
            let (ready, receiver) = oneshot::channel();
            ready.send(()).unwrap();
            hand_back_all(
                fixture.context(1, fixture.clock.clone()),
                receiver,
                CancellationToken::new(),
            )
            .await;
            assert_eq!(fixture.hub().calls(), [Vec::<String>::new()]);
        }
    }

    fn gate(gates: &ClaimGates, id: &str) -> Gate {
        *gates.watch(id).unwrap().borrow()
    }

    #[tokio::test(start_paused = true)]
    async fn schedules_and_bots_are_named_in_one_call_and_neither_hands_the_other_back() {
        let since = secs("2026-10-05T10:00:00Z");
        let fixture = Fixture::new("2026-10-05T10:00:05Z")
            .online(HubAnswer::Claimed(since))
            .with_parent(Some(Vec::new()));
        let (mut scheduler, gates) =
            fixture.gated(1, &[("report", EVERY_MINUTE)], &["helper", "assistant"]);
        let mut helper = gates.watch("helper").unwrap();
        assert_eq!(*helper.borrow_and_update(), Gate::Undecided);
        assert!(gates.watch("report").is_none());
        assert!(scheduler.ask().await);
        let union = ["report", "assistant", "helper"];
        assert_eq!(fixture.hub().calls(), [union]);
        assert_eq!(
            *fixture.parent.as_ref().unwrap().named.lock().unwrap(),
            [union]
        );
        assert!(helper.has_changed().unwrap());
        assert_eq!(
            *helper.borrow_and_update(),
            Gate::Armed { since: Some(since) }
        );
        assert_eq!(scheduler.events["report"].hold, None);
        assert_eq!(scheduler.next_ask, Some(Instant::now() + CONFIRMATION));
        fixture.pass(&mut scheduler, "2026-10-05T10:00:10Z").await;
        assert_eq!(
            stored(&fixture.state_dir())["claims"],
            json!({"assistant": {"hold": null, "confirmed": true, "since": since},
                   "helper": {"hold": null, "confirmed": true, "since": since}})
        );
        // Each confirmation names both again, and an unchanged answer does not wake a gate.
        tokio::time::advance(CONFIRMATION).await;
        fixture.pass(&mut scheduler, "2026-10-05T10:30:10Z").await;
        assert_eq!(fixture.hub().calls(), [union, union]);
        assert!(!helper.has_changed().unwrap());
        fixture.pass(&mut scheduler, "2026-10-05T10:31:00Z").await;
        assert_eq!(fixture.runner.starts(), ["report"]);
    }

    #[tokio::test(start_paused = true)]
    async fn a_placement_with_only_bots_claims_them_and_keeps_confirming() {
        let since = secs("2026-10-05T10:00:00Z");
        let fixture = Fixture::new("2026-10-05T10:00:05Z").online(HubAnswer::Claimed(since));
        let (mut scheduler, gates) = fixture.gated(1, &[], &["helper"]);
        assert!(scheduler.ask().await);
        assert_eq!(fixture.hub().calls(), [["helper"]]);
        assert_eq!(gate(&gates, "helper"), Gate::Armed { since: Some(since) });
        assert_eq!(scheduler.next_ask, Some(Instant::now() + CONFIRMATION));
        fixture.pass(&mut scheduler, "2026-10-05T10:00:10Z").await;
        let file = stored(&fixture.state_dir());
        assert_eq!(
            (file["decided"].clone(), file["events"].clone()),
            (json!(true), json!({}))
        );
        tokio::time::advance(CONFIRMATION).await;
        fixture.pass(&mut scheduler, "2026-10-05T10:30:10Z").await;
        assert_eq!(fixture.hub().calls(), [["helper"], ["helper"]]);
        assert!(fixture.runner.starts().is_empty());
        // A local-only placement opens the gate without a hub.
        let local = Fixture::new("2026-10-05T10:00:05Z");
        let (mut scheduler, gates) = local.gated(1, &[], &["helper"]);
        assert!(scheduler.ask().await);
        assert_eq!(gate(&gates, "helper"), Gate::Armed { since: None });
        assert_eq!(scheduler.next_ask, None);
    }

    #[tokio::test(start_paused = true)]
    async fn a_bot_the_hub_does_not_run_here_closes_only_its_gate() {
        let since = secs("2026-10-05T10:00:00Z");
        let fixture = Fixture::new("2026-10-05T10:00:05Z").online(HubAnswer::Mixed(
            vec![("report", since), ("helper", since)],
            vec![("assistant", HoldReason::NotReleased)],
        ));
        let (mut scheduler, gates) =
            fixture.gated(1, &[("report", EVERY_MINUTE)], &["helper", "assistant"]);
        scheduler.ask().await;
        assert_eq!(
            gate(&gates, "assistant"),
            Gate::Held(HoldReason::NotReleased)
        );
        assert_eq!(gate(&gates, "helper"), Gate::Armed { since: Some(since) });
        assert_eq!(scheduler.events["report"].hold, None);
        assert_eq!(scheduler.next_ask, Some(Instant::now() + HELD_RETRY));
        // Released later: the next question opens it.
        fixture.hub().answers(HubAnswer::Claimed(since));
        tokio::time::advance(HELD_RETRY).await;
        fixture.pass(&mut scheduler, "2026-10-05T10:05:10Z").await;
        assert_eq!(
            gate(&gates, "assistant"),
            Gate::Armed { since: Some(since) }
        );
        // Given back to the hub while it runs: held at the next confirmation.
        fixture.hub().answers(HubAnswer::Mixed(
            vec![("report", since), ("assistant", since)],
            vec![("helper", HoldReason::RunsElsewhere)],
        ));
        tokio::time::advance(CONFIRMATION).await;
        fixture.pass(&mut scheduler, "2026-10-05T10:35:10Z").await;
        assert_eq!(
            gate(&gates, "helper"),
            Gate::Held(HoldReason::RunsElsewhere)
        );
        assert_eq!(
            stored(&fixture.state_dir())["claims"]["helper"],
            json!({"hold": "runs_elsewhere", "confirmed": false, "since": null})
        );
        // A refused call closes every gate.
        fixture.hub().answers(HubAnswer::Fails(ClaimError::Denied));
        tokio::time::advance(HELD_RETRY).await;
        fixture.pass(&mut scheduler, "2026-10-05T10:40:10Z").await;
        for id in ["helper", "assistant"] {
            assert_eq!(
                gate(&gates, id),
                Gate::Held(HoldReason::NotReleased),
                "{id}"
            );
        }
    }

    #[tokio::test(start_paused = true)]
    async fn the_parents_refusal_closes_the_gate_before_the_hub_is_asked() {
        let since = secs("2026-10-05T10:00:00Z");
        let fixture = Fixture::new("2026-10-05T10:00:05Z")
            .online(HubAnswer::Claimed(since))
            .with_parent(None);
        let (mut scheduler, gates) = fixture.gated(1, &[("report", EVERY_MINUTE)], &["helper"]);
        // A parent that has not answered decides nothing.
        assert!(!scheduler.ask().await);
        assert_eq!(gate(&gates, "helper"), Gate::Undecided);
        assert!(fixture.hub().calls().is_empty());
        fixture.parent_answers(Some(vec![("helper", "other-service")]));
        assert!(scheduler.ask().await);
        assert_eq!(gate(&gates, "helper"), Gate::Held(HoldReason::OtherService));
        assert_eq!(fixture.hub().calls(), [["report"]]);
        fixture.parent_answers(Some(Vec::new()));
        tokio::time::advance(HELD_RETRY).await;
        fixture.pass(&mut scheduler, "2026-10-05T10:05:10Z").await;
        assert_eq!(fixture.hub().calls()[1], ["report", "helper"]);
        assert_eq!(gate(&gates, "helper"), Gate::Armed { since: Some(since) });
    }

    #[tokio::test(start_paused = true)]
    async fn a_start_without_the_hub_opens_only_gates_it_confirmed_for_the_same_approval() {
        let since = secs("2026-10-01T08:00:00Z");
        let mut fixture = Fixture::new("2026-10-05T10:00:05Z").online(HubAnswer::Claimed(since));
        let (mut scheduler, _) = fixture.gated(1, &[("report", EVERY_MINUTE)], &["helper"]);
        scheduler.ask().await;
        fixture.pass(&mut scheduler, "2026-10-05T10:00:10Z").await;
        drop(scheduler);
        fixture
            .hub()
            .answers(HubAnswer::Fails(ClaimError::Unreachable));

        let (mut offline, gates) =
            fixture.gated(2, &[("report", EVERY_MINUTE)], &["helper", "assistant"]);
        offline.ask().await;
        assert_eq!(gate(&gates, "helper"), Gate::Armed { since: Some(since) });
        assert_eq!(
            gate(&gates, "assistant"),
            Gate::Held(HoldReason::HubUnreachable)
        );
        assert_eq!(offline.next_ask, Some(Instant::now() + UNREACHABLE_RETRY));
        // A confirmation the hub cannot be reached for keeps an open gate open.
        tokio::time::advance(UNREACHABLE_RETRY).await;
        fixture.pass(&mut offline, "2026-10-05T10:01:10Z").await;
        assert_eq!(gate(&gates, "helper"), Gate::Armed { since: Some(since) });
        drop(offline);

        fixture.grant = "renewed-grant";
        let (mut renewed, gates) = fixture.gated(3, &[], &["helper"]);
        renewed.ask().await;
        assert_eq!(
            gate(&gates, "helper"),
            Gate::Held(HoldReason::HubUnreachable)
        );
    }

    #[tokio::test(start_paused = true)]
    async fn a_config_that_dropped_the_bot_calls_the_hub_without_it() {
        let since = secs("2026-10-05T10:00:00Z");
        let fixture = Fixture::new("2026-10-05T10:00:05Z").online(HubAnswer::Claimed(since));
        let (mut scheduler, _) = fixture.gated(1, &[("report", EVERY_MINUTE)], &["helper"]);
        scheduler.ask().await;
        fixture.pass(&mut scheduler, "2026-10-05T10:00:10Z").await;
        drop(scheduler);
        let mut smaller = fixture.scheduler(2, &[("report", EVERY_MINUTE)]);
        smaller.ask().await;
        fixture.pass(&mut smaller, "2026-10-05T10:00:20Z").await;
        assert_eq!(
            fixture.hub().calls(),
            [vec!["report", "helper"], vec!["report"]]
        );
        assert!(stored(&fixture.state_dir()).get("claims").is_none());
        drop(smaller);
        // Added back, the bot is confirmed anew: nothing of the earlier claim is kept.
        fixture
            .hub()
            .answers(HubAnswer::Fails(ClaimError::Unreachable));
        let (mut again, gates) = fixture.gated(3, &[("report", EVERY_MINUTE)], &["helper"]);
        again.ask().await;
        assert_eq!(
            gate(&gates, "helper"),
            Gate::Held(HoldReason::HubUnreachable)
        );
    }

    #[tokio::test(start_paused = true)]
    async fn the_claim_set_is_bounded_and_never_names_an_event_twice() {
        let fixture = Fixture::new("2026-10-05T10:00:05Z");
        let prepared = |specs: Vec<(String, ScheduleSpec)>, gated: Vec<String>| {
            prepare_with_gates(
                fixture.context(1, fixture.clock.clone()),
                specs,
                ClaimGates::new(gated),
            )
            .err()
            .map(|error| error.to_string())
        };
        let schedules = |count: usize| {
            (0..count)
                .map(|index| (format!("schedule-{index}"), spec(EVERY_MINUTE, None)))
                .collect::<Vec<_>>()
        };
        let bots = |count: usize| {
            (0..count)
                .map(|index| format!("bot-{index}"))
                .collect::<Vec<_>>()
        };
        assert_eq!(prepared(schedules(0), bots(1)), None);
        assert_eq!(prepared(schedules(32), bots(32)), None);
        for (specs, gated) in [(0, 0), (33, 32), (0, 65)] {
            let error = prepared(schedules(specs), bots(gated)).unwrap();
            assert!(error.contains("schedules and bots together"), "{error}");
        }
        let error = prepared(schedules(1), vec!["schedule-0".into()]).unwrap();
        assert!(error.contains("both a schedule and a bot"), "{error}");
        assert!(prepare(fixture.context(1, fixture.clock.clone()), Vec::new()).is_err());
    }

    /// A one-time schedule at `time` UTC on 2026-10-05, pinned at event version 0.0.4.
    fn once(time: &str) -> ScheduleSpec {
        ScheduleSpec {
            event_version: (0, 0, 4),
            ..once_spec("2026-10-05", time, None).unwrap()
        }
    }

    fn once_state(scheduler: &Scheduler, id: &str) -> OnceState {
        scheduler.onces[id].once_state
    }

    /// Runs the one-time schedule `once` at 10:30 in a process that armed it at 10:00:05.
    async fn ran_once(fixture: &Fixture) -> Scheduler {
        let mut scheduler = fixture.with_specs(1, vec![("once", once("10:30"))]);
        assert!(scheduler.ask().await);
        fixture.pass(&mut scheduler, "2026-10-05T10:29:59Z").await;
        assert!(fixture.runner.starts().is_empty());
        fixture.pass(&mut scheduler, "2026-10-05T10:30:00Z").await;
        fixture.pass(&mut scheduler, "2026-10-05T10:30:02Z").await;
        scheduler
    }

    const RAN: LastRun = LastRun {
        at: 1_791_196_200,
        finished_at: 1_791_196_202,
        outcome: Outcome::Succeeded,
    };

    #[tokio::test(start_paused = true)]
    async fn a_one_time_schedule_runs_once_at_its_time() {
        let fixture = Fixture::new("2026-10-05T10:00:05Z");
        let mut scheduler = ran_once(&fixture).await;
        assert_eq!(RAN.at, secs("2026-10-05T10:30:00Z"));
        assert_eq!(
            scheduler.onces["once"].once_armed_at,
            Some(secs("2026-10-05T10:00:05Z"))
        );
        assert!(scheduler.events.is_empty());
        assert_eq!(fixture.runner.starts(), ["once"]);
        // `started` was on disk before the run started.
        assert_eq!(
            *fixture.runner.once_states.lock().unwrap(),
            [json!("started")]
        );
        fixture.pass(&mut scheduler, "2026-10-05T10:31:00Z").await;
        let entry = &scheduler.onces["once"];
        assert_eq!(
            (entry.once_state, entry.last.clone(), entry.runs),
            (OnceState::Ran, Some(RAN), 1)
        );
        assert_eq!(fixture.runner.starts().len(), 1);
    }

    #[tokio::test(start_paused = true)]
    async fn a_restart_or_a_clock_set_back_never_runs_a_one_time_schedule_again() {
        let fixture = Fixture::new("2026-10-05T10:00:05Z");
        drop(ran_once(&fixture).await);
        let events = || vec![("once", once("10:30"))];
        // A restart of the same intent revision with the clock before its time.
        fixture.clock.set("2026-10-05T10:20:00Z");
        let mut restarted = fixture.with_specs(1, events());
        restarted.ask().await;
        fixture.pass(&mut restarted, "2026-10-05T10:30:00Z").await;
        drop(restarted);
        // Start, with the clock set back two days: the 24-hour rule never touches the record.
        fixture.clock.set("2026-10-03T10:00:00Z");
        let mut started = fixture.with_specs(2, events());
        started.ask().await;
        for now in [
            "2026-10-03T10:30:00Z",
            "2026-10-05T10:30:00Z",
            "2026-10-05T10:30:30Z",
        ] {
            fixture.pass(&mut started, now).await;
        }
        assert_eq!(fixture.runner.starts(), ["once"]);
        let entry = &started.onces["once"];
        assert_eq!(
            (entry.once_state, entry.last.clone(), entry.runs),
            (OnceState::Ran, Some(RAN), 1)
        );
    }

    /// Arms a one-time schedule at 10:30 in one process, ends that process before its time and
    /// starts the next one at `back`, when the hub answers `then`.
    async fn back_at(fixture: &Fixture, back: &str, then: Option<HubAnswer>) -> Scheduler {
        let mut first = fixture.with_specs(1, vec![("once", once("10:30"))]);
        first.ask().await;
        fixture.pass(&mut first, "2026-10-05T10:00:10Z").await;
        drop(first);
        if let Some(answer) = then {
            fixture.hub().answers(answer);
        }
        fixture.clock.set(back);
        let mut next = fixture.with_specs(2, vec![("once", once("10:30"))]);
        next.ask().await;
        next
    }

    #[tokio::test(start_paused = true)]
    async fn a_service_that_had_it_armed_runs_it_up_to_fifteen_minutes_late() {
        for (back, runs) in [
            ("2026-10-05T10:40:00Z", true),
            ("2026-10-05T10:45:00Z", true),
            ("2026-10-05T10:46:00Z", false),
        ] {
            let fixture = Fixture::new("2026-10-05T10:00:05Z");
            let mut next = back_at(&fixture, back, None).await;
            fixture.pass(&mut next, back).await;
            fixture.pass(&mut next, back).await;
            let expected = if runs {
                OnceState::Ran
            } else {
                OnceState::Missed
            };
            assert_eq!(once_state(&next, "once"), expected, "{back}");
            assert_eq!(fixture.runner.starts().len(), usize::from(runs), "{back}");
        }
        // Online: the approval held the claim over its time, so the hub skipped it.
        let since = secs("2026-10-05T10:00:00Z");
        let fixture = Fixture::new("2026-10-05T10:00:05Z").online(HubAnswer::Claimed(since));
        let mut next = back_at(&fixture, "2026-10-05T10:40:00Z", None).await;
        fixture.pass(&mut next, "2026-10-05T10:40:00Z").await;
        assert_eq!(fixture.runner.starts(), ["once"]);
        // A claim taken anew after its time: the hub may have run it.
        let fixture = Fixture::new("2026-10-05T10:00:05Z").online(HubAnswer::Claimed(since));
        let anew = HubAnswer::Claimed(secs("2026-10-05T10:40:00Z"));
        let mut next = back_at(&fixture, "2026-10-05T10:40:00Z", Some(anew)).await;
        fixture.pass(&mut next, "2026-10-05T10:40:00Z").await;
        assert_eq!(once_state(&next, "once"), OnceState::Missed);
        assert!(fixture.runner.starts().is_empty());
    }

    #[tokio::test(start_paused = true)]
    async fn a_time_this_service_could_not_take_in_time_has_passed() {
        // Deployed after its time: valid, ready, and never run.
        let fixture = Fixture::new("2026-10-05T10:31:00Z");
        let mut late = fixture.with_specs(1, vec![("once", once("10:30"))]);
        late.ask().await;
        fixture.pass(&mut late, "2026-10-05T10:31:05Z").await;
        assert_eq!(once_state(&late, "once"), OnceState::Passed);
        assert_eq!(late.onces["once"].once_armed_at, None);
        assert!(fixture.runner.starts().is_empty());

        // Held at its time: it passes, and a later claim does not bring it back.
        let fixture = Fixture::new("2026-10-05T10:00:05Z").online(HubAnswer::Mixed(
            Vec::new(),
            vec![("once", HoldReason::RunsElsewhere)],
        ));
        let mut held = fixture.with_specs(1, vec![("once", once("10:30"))]);
        held.ask().await;
        assert_eq!(once_state(&held, "once"), OnceState::Pending);
        fixture.pass(&mut held, "2026-10-05T10:30:00Z").await;
        assert_eq!(once_state(&held, "once"), OnceState::Passed);
        let since = secs("2026-10-05T10:00:00Z");
        fixture.hub().answers(HubAnswer::Claimed(since));
        tokio::time::advance(HELD_RETRY).await;
        fixture.pass(&mut held, "2026-10-05T10:30:10Z").await;
        assert_eq!(held.onces["once"].claim.hold, None);
        assert_eq!(once_state(&held, "once"), OnceState::Passed);
        assert!(fixture.runner.starts().is_empty());

        // Handed over by the hub at its time, by a hub clock that is ahead.
        let fixture = Fixture::new("2026-10-05T10:29:00Z")
            .online(HubAnswer::Claimed(secs("2026-10-05T10:30:00Z")));
        let mut handed = fixture.with_specs(1, vec![("once", once("10:30"))]);
        handed.ask().await;
        assert_eq!(once_state(&handed, "once"), OnceState::Passed);
    }

    #[tokio::test(start_paused = true)]
    async fn another_approval_arms_it_again_before_its_time_and_after_it_it_has_passed() {
        let since = secs("2026-10-05T10:00:00Z");
        let mut fixture = Fixture::new("2026-10-05T10:00:05Z").online(HubAnswer::Claimed(since));
        let mut first = fixture.with_specs(1, vec![("once", once("10:30"))]);
        first.ask().await;
        fixture.pass(&mut first, "2026-10-05T10:00:10Z").await;
        drop(first);
        fixture.grant = "renewed-grant";
        fixture.clock.set("2026-10-05T10:10:00Z");
        fixture
            .hub()
            .answers(HubAnswer::Claimed(secs("2026-10-05T10:10:00Z")));
        let mut renewed = fixture.with_specs(2, vec![("once", once("10:30"))]);
        renewed.ask().await;
        assert_eq!(
            renewed.onces["once"].once_armed_at,
            Some(secs("2026-10-05T10:10:00Z"))
        );
        drop(renewed);
        fixture.grant = "third-grant";
        fixture.clock.set("2026-10-05T10:40:00Z");
        fixture
            .hub()
            .answers(HubAnswer::Claimed(secs("2026-10-05T10:40:00Z")));
        let mut third = fixture.with_specs(3, vec![("once", once("10:30"))]);
        third.ask().await;
        fixture.pass(&mut third, "2026-10-05T10:40:05Z").await;
        assert_eq!(once_state(&third, "once"), OnceState::Passed);
        assert!(fixture.runner.starts().is_empty());
    }

    #[tokio::test(start_paused = true)]
    async fn a_one_time_record_whose_state_cannot_be_read_never_runs() {
        let fixture = Fixture::new("2026-10-05T10:00:05Z");
        let specs = || {
            ["10:30", "10:40", "10:50"]
                .into_iter()
                .zip(["bare", "unknown", "readable"])
                .map(|(time, id)| (id, once(time)))
                .collect::<Vec<_>>()
        };
        drop(fixture.with_specs(1, specs()));
        let at = |time: &str| secs(&format!("2026-10-05T{time}:00Z"));
        let armed = secs("2026-10-05T10:00:00Z");
        let foreign = json!({"version": 1, "intent_revision": 1, "grant_id": null, "events": {
            "bare": {"once_at": at("10:30")},
            "unknown": {"once_at": at("10:40"), "once_state": "rescheduled", "runs": "many"},
            "readable": {"once_at": at("10:50"), "once_state": "pending", "once_armed_at": armed,
                "last": {"outcome": "exploded"}}},
            "once_done": [{"event_id": "gone", "once_at": at("09:00"), "state": "later"},
                {"event_id": "../escape", "once_at": 1, "state": "ran"}]});
        let file = fixture.state_dir().join(STATE_FILE);
        std::fs::write(file, serde_json::to_vec(&foreign).unwrap()).unwrap();
        let mut scheduler = fixture.with_specs(2, specs());
        for (id, state) in [
            ("bare", OnceState::Passed),
            ("unknown", OnceState::Passed),
            ("readable", OnceState::Pending),
        ] {
            assert_eq!(once_state(&scheduler, id), state, "{id}");
        }
        assert_eq!(scheduler.onces["readable"].once_armed_at, Some(armed));
        let kept: Vec<_> = scheduler
            .once_done
            .iter()
            .map(|done| (done.event_id.as_str(), done.state))
            .collect();
        assert_eq!(kept, [("gone", OnceState::Passed)]);
        scheduler.ask().await;
        for now in ["10:30", "10:40", "10:50"] {
            fixture
                .pass(&mut scheduler, &format!("2026-10-05T{now}:00Z"))
                .await;
        }
        assert_eq!(fixture.runner.starts(), ["readable"]);
    }

    #[tokio::test(start_paused = true)]
    async fn a_one_time_schedule_waits_for_a_free_run_slot_inside_its_limit() {
        for (freed, expected) in [
            ("2026-10-05T10:16:00Z", OnceState::Ran),
            ("2026-10-05T10:16:01Z", OnceState::Missed),
        ] {
            let fixture = Fixture::new("2026-10-05T09:59:30Z");
            fixture.runner.set(Mode::Gated);
            let ids: Vec<String> = (1..=MAX_CONCURRENT_RUNS)
                .map(|index| format!("event-{index}"))
                .collect();
            let mut specs: Vec<(&str, ScheduleSpec)> = ids
                .iter()
                .map(|id| (id.as_str(), spec("0 10 * * *", None)))
                .collect();
            specs.push(("once", once("10:01")));
            let mut scheduler = fixture.with_specs(1, specs);
            scheduler.ask().await;
            fixture.pass(&mut scheduler, "2026-10-05T10:00:00Z").await;
            assert_eq!(fixture.runner.starts().len(), MAX_CONCURRENT_RUNS);
            for now in ["2026-10-05T10:01:00Z", "2026-10-05T10:10:00Z"] {
                fixture.pass(&mut scheduler, now).await;
            }
            assert_eq!(once_state(&scheduler, "once"), OnceState::Pending);
            assert_eq!(scheduler.onces["once"].runs, 0);
            fixture.runner.gate.add_permits(MAX_CONCURRENT_RUNS + 1);
            settle().await;
            fixture.pass(&mut scheduler, freed).await;
            fixture.pass(&mut scheduler, freed).await;
            assert_eq!(once_state(&scheduler, "once"), expected, "{freed}");
            let started = fixture.runner.starts().iter().any(|id| id == "once");
            assert_eq!(started, expected == OnceState::Ran, "{freed}");
        }
    }

    #[tokio::test(start_paused = true)]
    async fn a_one_time_time_that_cannot_be_recorded_is_tried_again_inside_its_limit() {
        let unwritable = |fixture: &Fixture| {
            std::fs::remove_dir_all(fixture.state_dir()).unwrap();
            std::fs::write(fixture.state_dir(), b"").unwrap();
        };
        let fixture = Fixture::new("2026-10-05T10:00:05Z");
        let mut scheduler = fixture.with_specs(1, vec![("once", once("10:01"))]);
        scheduler.ask().await;
        fixture.pass(&mut scheduler, "2026-10-05T10:00:10Z").await;
        unwritable(&fixture);
        for now in ["2026-10-05T10:01:00Z", "2026-10-05T10:05:00Z"] {
            fixture.pass(&mut scheduler, now).await;
        }
        assert!(fixture.runner.starts().is_empty());
        let entry = &scheduler.onces["once"];
        assert_eq!(
            (entry.once_state, entry.runs, entry.failed),
            (OnceState::Pending, 0, 0)
        );
        std::fs::remove_file(fixture.state_dir()).unwrap();
        std::fs::create_dir(fixture.state_dir()).unwrap();
        fixture.pass(&mut scheduler, "2026-10-05T10:10:00Z").await;
        assert_eq!(fixture.runner.starts(), ["once"]);
        assert_eq!(
            *fixture.runner.once_states.lock().unwrap(),
            [json!("started")]
        );

        // A file that stays unwritable over the limit: missed.
        let fixture = Fixture::new("2026-10-05T10:00:05Z");
        let mut scheduler = fixture.with_specs(1, vec![("once", once("10:01"))]);
        scheduler.ask().await;
        unwritable(&fixture);
        for now in ["2026-10-05T10:01:00Z", "2026-10-05T10:16:01Z"] {
            fixture.pass(&mut scheduler, now).await;
        }
        assert_eq!(once_state(&scheduler, "once"), OnceState::Missed);
        assert!(fixture.runner.starts().is_empty());
    }

    #[tokio::test(start_paused = true)]
    async fn a_run_cut_off_by_a_crash_or_a_stop_is_never_started_again() {
        let fixture = Fixture::new("2026-10-05T10:00:05Z");
        fixture.runner.set(Mode::UntilCancelled);
        let mut scheduler = fixture.with_specs(1, vec![("once", once("10:01"))]);
        scheduler.ask().await;
        fixture.pass(&mut scheduler, "2026-10-05T10:01:00Z").await;
        assert_eq!(
            stored(&fixture.state_dir())["events"]["once"]["once_state"],
            "started"
        );
        // Power is lost during the run: the process ends without its stop.
        drop(scheduler);
        fixture.clock.set("2026-10-05T10:03:00Z");
        let mut next = fixture.with_specs(2, vec![("once", once("10:01"))]);
        let entry = &next.onces["once"];
        assert_eq!(entry.once_state, OnceState::Ran);
        assert_eq!(entry.last.as_ref().unwrap().outcome, Outcome::Cancelled);
        next.ask().await;
        fixture.pass(&mut next, "2026-10-05T10:03:05Z").await;
        assert_eq!(fixture.runner.starts().len(), 1);

        // A stop during the run cuts it off the same way.
        let fixture = Fixture::new("2026-10-05T10:00:05Z");
        fixture.runner.set(Mode::UntilCancelled);
        let mut scheduler = fixture.with_specs(1, vec![("once", once("10:01"))]);
        scheduler.ask().await;
        fixture.pass(&mut scheduler, "2026-10-05T10:01:00Z").await;
        scheduler.drain().await;
        let entry = &stored(&fixture.state_dir())["events"]["once"];
        assert_eq!(
            (
                entry["once_state"].clone(),
                entry["last"]["outcome"].clone()
            ),
            (json!("ran"), json!("cancelled"))
        );
    }

    fn yearly() -> ScheduleSpec {
        spec("0 0 1 1 *", None)
    }

    #[tokio::test(start_paused = true)]
    async fn a_finished_record_outlives_its_event_and_comes_back_with_it() {
        let fixture = Fixture::new("2026-10-05T10:00:05Z");
        drop(ran_once(&fixture).await);
        // An update drops the event: its finished record is kept apart.
        let mut without = fixture.with_specs(2, vec![("report", yearly())]);
        without.ask().await;
        fixture.pass(&mut without, "2026-10-05T10:31:00Z").await;
        let file = stored(&fixture.state_dir());
        assert!(file["events"].get("once").is_none());
        assert_eq!(
            file["once_done"],
            json!([{"event_id": "once", "once_at": RAN.at, "state": "ran",
                    "outcome": "succeeded", "at": RAN.finished_at}])
        );
        drop(without);
        // A rollback brings it back with the same time, the clock set back: it does not run.
        fixture.clock.set("2026-10-05T09:00:00Z");
        let mut back = fixture.with_specs(3, vec![("once", once("10:30"))]);
        assert_eq!(once_state(&back, "once"), OnceState::Ran);
        back.ask().await;
        fixture.pass(&mut back, "2026-10-05T10:30:00Z").await;
        assert_eq!(fixture.runner.starts(), ["once"]);
        assert!(stored(&fixture.state_dir()).get("once_done").is_none());
    }

    #[tokio::test(start_paused = true)]
    async fn a_new_time_starts_a_new_record_and_keeps_the_finished_one_apart() {
        let fixture = Fixture::new("2026-10-05T10:00:05Z");
        drop(ran_once(&fixture).await);
        fixture.clock.set("2026-10-05T10:35:00Z");
        let mut moved = fixture.with_specs(2, vec![("once", once("11:00"))]);
        moved.ask().await;
        fixture.pass(&mut moved, "2026-10-05T10:35:05Z").await;
        assert_eq!(once_state(&moved, "once"), OnceState::Pending);
        let done =
            |index: usize| stored(&fixture.state_dir())["once_done"][index]["once_at"].clone();
        assert_eq!(done(0), RAN.at);
        for now in ["2026-10-05T11:00:00Z", "2026-10-05T11:00:05Z"] {
            fixture.pass(&mut moved, now).await;
        }
        assert_eq!(fixture.runner.starts(), ["once", "once"]);
        drop(moved);
        // A rollback to the first time finds it finished; the second one waits apart.
        let rolled_back = fixture.with_specs(3, vec![("once", once("10:30"))]);
        assert_eq!(once_state(&rolled_back, "once"), OnceState::Ran);
        assert_eq!(done(0), secs("2026-10-05T11:00:00Z"));
    }

    #[tokio::test(start_paused = true)]
    async fn an_unfinished_record_goes_with_its_event() {
        let fixture = Fixture::new("2026-10-05T10:00:05Z");
        let mut armed = fixture.with_specs(1, vec![("once", once("10:30"))]);
        armed.ask().await;
        fixture.pass(&mut armed, "2026-10-05T10:00:10Z").await;
        drop(armed);
        let mut dropped = fixture.with_specs(2, vec![("report", yearly())]);
        dropped.ask().await;
        fixture.pass(&mut dropped, "2026-10-05T10:20:00Z").await;
        assert!(stored(&fixture.state_dir()).get("once_done").is_none());
        drop(dropped);
        // Back after its time: it has passed.
        fixture.clock.set("2026-10-05T10:35:00Z");
        let mut again = fixture.with_specs(3, vec![("once", once("10:30"))]);
        again.ask().await;
        assert_eq!(once_state(&again, "once"), OnceState::Passed);
    }

    #[tokio::test(start_paused = true)]
    async fn a_config_without_schedules_keeps_the_records_of_finished_one_time_schedules() {
        let fixture = Fixture::new("2026-10-05T10:00:05Z");
        let mut scheduler =
            fixture.with_specs(1, vec![("once", once("10:01")), ("later", once("11:00"))]);
        scheduler.ask().await;
        fixture.pass(&mut scheduler, "2026-10-05T10:01:00Z").await;
        fixture.pass(&mut scheduler, "2026-10-05T10:01:05Z").await;
        drop(scheduler);
        let (ready, receiver) = oneshot::channel();
        ready.send(()).unwrap();
        hand_back_all(
            fixture.context(2, fixture.clock.clone()),
            receiver,
            CancellationToken::new(),
        )
        .await;
        let file = stored(&fixture.state_dir());
        assert_eq!(
            (file["decided"].clone(), file["events"].clone()),
            (json!(false), json!({}))
        );
        let kept: Vec<&str> = file["once_done"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|record| record["event_id"].as_str())
            .collect();
        assert_eq!(kept, ["once"]);
        // A rollback that brings it back, with the clock set back, does not run it again.
        fixture.clock.set("2026-10-05T09:00:00Z");
        let mut back = fixture.with_specs(3, vec![("once", once("10:01"))]);
        back.ask().await;
        assert_eq!(once_state(&back, "once"), OnceState::Ran);
        fixture.pass(&mut back, "2026-10-05T10:01:00Z").await;
        assert_eq!(fixture.runner.starts(), ["once"]);
    }

    #[tokio::test(start_paused = true)]
    async fn a_finished_one_time_schedule_stays_claimed_and_confirmed() {
        let since = secs("2026-10-05T10:00:00Z");
        let fixture = Fixture::new("2026-10-05T10:00:05Z").online(HubAnswer::Claimed(since));
        let mut scheduler = fixture.with_specs(1, vec![("once", once("10:01"))]);
        scheduler.ask().await;
        fixture.pass(&mut scheduler, "2026-10-05T10:01:00Z").await;
        fixture.pass(&mut scheduler, "2026-10-05T10:01:05Z").await;
        assert_eq!(once_state(&scheduler, "once"), OnceState::Ran);
        tokio::time::advance(CONFIRMATION).await;
        fixture.pass(&mut scheduler, "2026-10-05T10:31:05Z").await;
        assert_eq!(fixture.hub().calls(), [["once"], ["once"]]);
        assert_eq!(scheduler.onces["once"].claim.hold, None);
        assert_eq!(scheduler.next_ask, Some(Instant::now() + CONFIRMATION));
        drop(scheduler);
        let mut restarted = fixture.with_specs(2, vec![("once", once("10:01"))]);
        restarted.ask().await;
        assert_eq!(fixture.hub().calls()[2], ["once"]);
        assert_eq!(restarted.next_ask, Some(Instant::now() + CONFIRMATION));
        assert_eq!(fixture.runner.starts().len(), 1);
    }

    #[tokio::test(start_paused = true)]
    async fn the_one_time_state_has_the_shape_the_agent_parent_reads() {
        // The literal file of §1.6a of the contract between the placement process and the parent.
        let literal = json!({"version":1,"config_revision":12,"intent_revision":7,"decided":true,"grant_id":"g-1",
            "events":{"evt_once":{"once_at":1790233200,"timezone":"Europe/Berlin","event_version":[0,0,4],
              "once_state":"pending","once_armed_at":1790000000,"hold":null,"confirmed":true,"since":1789990000,
              "running_since":null,"last":null,"runs":0,"failed":0}},
            "once_done":[{"event_id":"evt_old","once_at":1789023600,"state":"ran","outcome":"succeeded","at":1789023604}],
            "claims":{"evt_helper":{"hold":null,"confirmed":true,"since":1789990000}}});
        let fixture = Fixture::new("2026-09-22T10:00:00Z").online(HubAnswer::Claimed(1789990000));
        let once = ScheduleSpec {
            event_version: (0, 0, 4),
            ..once_spec("2026-09-24", "09:00", Some("Europe/Berlin")).unwrap()
        };
        let mut scheduler = prepare_with_gates(
            fixture.context(7, fixture.clock.clone()),
            vec![("evt_once".into(), once)],
            ClaimGates::new(vec!["evt_helper".into()]),
        )
        .unwrap();
        scheduler.context.config_revision = 12;
        scheduler.context.grant_id = Some("g-1".into());
        scheduler.decided = true;
        let entry: OnceEntry =
            serde_json::from_value(literal["events"]["evt_once"].clone()).unwrap();
        assert_eq!(
            (entry.once_state, entry.claim.since, entry.event_version),
            (OnceState::Pending, Some(1789990000), Some((0, 0, 4)))
        );
        scheduler.onces.insert("evt_once".into(), entry);
        scheduler.once_done = carried_done(&literal);
        scheduler.claims.insert(
            "evt_helper".into(),
            serde_json::from_value(literal["claims"]["evt_helper"].clone()).unwrap(),
        );
        scheduler.persist().unwrap();
        assert_eq!(stored(&fixture.state_dir()), literal);
        // The next process of the same approval takes record, finished records and claims over.
        drop(scheduler);
        let mut fixture = fixture;
        fixture.grant = "g-1";
        let (next, gates) = fixture.gated(8, &[], &["evt_helper"]);
        assert_eq!(next.claims["evt_helper"].since, Some(1789990000));
        assert_eq!(next.once_done.len(), 1);
        assert!(next.onces["evt_once"].claim.confirmed);
        drop(gates);
    }
}
