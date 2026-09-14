//! Estimated infrastructure cost of running flows.
//!
//! Execution rows carry wall clock duration, never money. The platform bills
//! serverless compute, so the cost of a run is its duration multiplied by the
//! memory footprint of the functions that served it. The reference deployment
//! serves a run with an x86_64 function holding 2 GB for the whole run plus an
//! arm64 function holding 1.25 GB while a realtime execution waits. Async
//! dispatch has a short API leg; this legacy aggregate is a realtime estimate.
//!
//! Deployments that size their functions differently scale the whole estimate
//! with `COMPUTE_COST_MULTIPLIER`; an unset or unusable value keeps the
//! reference sizing at `1.0`. The variable is read once and cached, so changing
//! it requires a restart.

use serde::Serialize;
use std::sync::LazyLock;
use utoipa::ToSchema;

/// Scales the whole estimate for deployments that do not match the reference
/// sizing. Unset means the reference sizing is accurate.
pub const COMPUTE_COST_MULTIPLIER_ENV: &str = "COMPUTE_COST_MULTIPLIER";

const MICROS_PER_SECOND: f64 = 1_000_000.0;

/// One serverless function serving part of an execution.
#[derive(Clone, Copy, Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ComputeLeg {
    /// Processor architecture the function runs on.
    pub architecture: &'static str,
    /// Memory the function reserves, in gigabytes.
    pub memory_gb: f64,
    /// Share of the run's wall clock this function is billed for.
    pub duration_share: f64,
    /// Price of one gigabyte-second, in micro-dollars.
    pub micro_dollars_per_gb_second: f64,
}

impl ComputeLeg {
    fn micro_dollars_per_second(&self) -> f64 {
        self.memory_gb * self.duration_share * self.micro_dollars_per_gb_second
    }
}

/// AWS Lambda list prices: $0.0000166667 per x86 GB-second and $0.0000133334
/// per Arm GB-second.
const LEGS: [ComputeLeg; 2] = [
    ComputeLeg {
        architecture: "x86_64",
        memory_gb: 2.0,
        duration_share: 1.0,
        micro_dollars_per_gb_second: 16.6667,
    },
    ComputeLeg {
        architecture: "arm64",
        memory_gb: 1.25,
        duration_share: 1.0,
        micro_dollars_per_gb_second: 13.3334,
    },
];

/// $0.20 per million invocations, charged once per leg.
const REQUEST_MICRO_DOLLARS: f64 = 0.2;

static MULTIPLIER: LazyLock<f64> = LazyLock::new(read_multiplier);

fn read_multiplier() -> f64 {
    let Some(raw) = crate::storage_config::non_empty_env(COMPUTE_COST_MULTIPLIER_ENV) else {
        return 1.0;
    };

    match raw.parse::<f64>() {
        Ok(value) if value.is_finite() && value >= 0.0 => value,
        _ => {
            tracing::warn!(
                "{COMPUTE_COST_MULTIPLIER_ENV}={raw} is not a non-negative number, estimating compute cost at 1.0x"
            );
            1.0
        }
    }
}

/// The rate card behind every estimate, so clients can explain the number they
/// render instead of presenting it as an opaque total.
#[derive(Clone, Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ComputeCostModel {
    /// Functions billed for a single execution.
    pub legs: Vec<ComputeLeg>,
    /// Price of one invocation of one function, in micro-dollars.
    pub request_micro_dollars: f64,
    /// Deployment factor applied on top of the legs.
    pub multiplier: f64,
    /// Cost of one second of execution across all legs, in micro-dollars,
    /// multiplier included. Excludes the per-invocation price.
    pub micro_dollars_per_second: f64,
    /// Cost of starting one execution across all legs, in micro-dollars,
    /// multiplier included.
    pub micro_dollars_per_execution: f64,
}

/// Deployment factor applied to every estimate.
pub fn compute_cost_multiplier() -> f64 {
    *MULTIPLIER
}

pub fn compute_cost_model() -> ComputeCostModel {
    let multiplier = compute_cost_multiplier();
    ComputeCostModel {
        legs: LEGS.to_vec(),
        request_micro_dollars: REQUEST_MICRO_DOLLARS,
        multiplier,
        micro_dollars_per_second: micro_dollars_per_second() * multiplier,
        micro_dollars_per_execution: REQUEST_MICRO_DOLLARS * LEGS.len() as f64 * multiplier,
    }
}

fn micro_dollars_per_second() -> f64 {
    LEGS.iter().map(ComputeLeg::micro_dollars_per_second).sum()
}

fn estimate(total_duration_micros: f64, executions: i64, multiplier: f64) -> i64 {
    if executions <= 0 || !total_duration_micros.is_finite() || total_duration_micros <= 0.0 {
        return 0;
    }

    let seconds = total_duration_micros / MICROS_PER_SECOND;
    let duration_cost = seconds * micro_dollars_per_second();
    let request_cost = executions as f64 * LEGS.len() as f64 * REQUEST_MICRO_DOLLARS;

    ((duration_cost + request_cost) * multiplier).round() as i64
}

/// Estimated cost in micro-dollars of `executions` runs that together took
/// `total_duration_micros` of wall clock.
pub fn compute_cost_micro_dollars(total_duration_micros: i64, executions: i64) -> i64 {
    estimate(
        total_duration_micros as f64,
        executions,
        compute_cost_multiplier(),
    )
}

/// Estimated cost in micro-dollars derived from an aggregate: the average run
/// duration times the number of runs is the wall clock the deployment paid for.
pub fn compute_cost_from_avg_latency(avg_latency_ms: Option<f64>, executions: i64) -> i64 {
    let Some(avg_latency_ms) = avg_latency_ms else {
        return 0;
    };

    estimate(
        avg_latency_ms * 1_000.0 * executions as f64,
        executions,
        compute_cost_multiplier(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Realtime holds both the 2 GB executor and 1.25 GB API for the full second.
    const ONE_SECOND_MICRO_DOLLARS: f64 = 2.0 * 16.6667 + 1.25 * 13.3334;

    #[test]
    fn one_second_run_costs_both_legs_plus_two_invocations() {
        let expected = (ONE_SECOND_MICRO_DOLLARS + 2.0 * REQUEST_MICRO_DOLLARS).round() as i64;
        assert_eq!(estimate(1_000_000.0, 1, 1.0), expected);
    }

    #[test]
    fn multiplier_scales_the_whole_estimate() {
        let single = estimate(1_000_000.0, 1, 1.0) as f64;
        let doubled = estimate(1_000_000.0, 1, 2.0) as f64;
        assert!((doubled - single * 2.0).abs() <= 1.0);
    }

    #[test]
    fn average_latency_matches_the_summed_duration() {
        let from_average = compute_cost_from_avg_latency(Some(250.0), 40);
        let from_total = compute_cost_micro_dollars(10_000_000, 40);
        assert_eq!(from_average, from_total);
    }

    #[test]
    fn missing_or_empty_measurements_cost_nothing() {
        assert_eq!(compute_cost_from_avg_latency(None, 100), 0);
        assert_eq!(compute_cost_from_avg_latency(Some(120.0), 0), 0);
        assert_eq!(compute_cost_micro_dollars(-5, 10), 0);
    }
}
