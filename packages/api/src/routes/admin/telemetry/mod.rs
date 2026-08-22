//! Admin dashboards over anonymous product telemetry: usage overview, timeseries,
//! event list, engagement, FlowPilot funnel, crash issues, release health,
//! distributed traces and performance percentiles, plus the one user-attributed surface here:
//! captured FlowScript apply failures (see `flowscript_failures`).

pub mod alerts;
pub mod dashboards;
pub mod engagement;
pub mod events;
pub mod flowpilot;
pub mod flowscript_failures;
pub mod issues;
pub mod llm;
pub mod overview;
pub mod performance;
pub mod prompt_feedback;
pub mod query;
pub mod release_health;
pub mod rollup;
pub mod saved_queries;
pub mod sourcemaps;
pub mod span_stats;
pub mod sweep;
pub mod symbolicate;
pub mod timeseries;
pub mod traces;

pub use engagement::*;
pub use events::*;
pub use flowpilot::*;
pub use overview::*;
pub use timeseries::*;

use chrono::{Duration, DurationRound, NaiveDateTime};

const TOP_LIST_LIMIT: u64 = 10;

fn bucket_for(hours: i64, requested: Option<&str>) -> &'static str {
    if let Some(r) = requested {
        match r {
            "minute" => return "minute",
            "hour" => return "hour",
            "day" => return "day",
            _ => {}
        }
    }
    if hours <= 6 {
        "minute"
    } else if hours <= 24 * 7 {
        "hour"
    } else {
        "day"
    }
}

fn bucket_step(bucket: &str) -> Duration {
    match bucket {
        "minute" => Duration::minutes(1),
        "hour" => Duration::hours(1),
        _ => Duration::days(1),
    }
}

fn trunc_to_bucket(ts: NaiveDateTime, bucket: &str) -> NaiveDateTime {
    ts.duration_trunc(bucket_step(bucket)).unwrap_or(ts)
}

/// Ordered, gap-free bucket starts covering `cutoff..=now`, used to zero-fill charts.
fn bucket_slots(cutoff: NaiveDateTime, now: NaiveDateTime, bucket: &str) -> Vec<NaiveDateTime> {
    let step = bucket_step(bucket);
    let end = trunc_to_bucket(now, bucket);
    let mut slot = trunc_to_bucket(cutoff, bucket);
    let mut slots = Vec::new();
    while slot <= end {
        slots.push(slot);
        slot += step;
    }
    slots
}
