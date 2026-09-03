//! Shared contract between the maintenance Lambda and the API.
//!
//! Keep this surface deliberately small: the Lambda may request one named,
//! allowlisted job per invocation, while the API owns all data access and job
//! implementation details.

use crate::cache::CacheCleanupResult;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MaintenanceJob {
    TelemetryAlerts,
    CacheCleanup,
    RunSweep,
    StateCleanup,
    RegressionSuites,
}

impl MaintenanceJob {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::TelemetryAlerts => "telemetry_alerts",
            Self::CacheCleanup => "cache_cleanup",
            Self::RunSweep => "run_sweep",
            Self::StateCleanup => "state_cleanup",
            Self::RegressionSuites => "regression_suites",
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(tag = "job", rename_all = "snake_case")]
pub enum MaintenanceRunRequest {
    TelemetryAlerts,
    CacheCleanup,
    RunSweep,
    StateCleanup,
    RegressionSuites,
}

impl MaintenanceRunRequest {
    pub const fn job(self) -> MaintenanceJob {
        match self {
            Self::TelemetryAlerts => MaintenanceJob::TelemetryAlerts,
            Self::CacheCleanup => MaintenanceJob::CacheCleanup,
            Self::RunSweep => MaintenanceJob::RunSweep,
            Self::StateCleanup => MaintenanceJob::StateCleanup,
            Self::RegressionSuites => MaintenanceJob::RegressionSuites,
        }
    }
}

impl From<MaintenanceJob> for MaintenanceRunRequest {
    fn from(job: MaintenanceJob) -> Self {
        match job {
            MaintenanceJob::TelemetryAlerts => Self::TelemetryAlerts,
            MaintenanceJob::CacheCleanup => Self::CacheCleanup,
            MaintenanceJob::RunSweep => Self::RunSweep,
            MaintenanceJob::StateCleanup => Self::StateCleanup,
            MaintenanceJob::RegressionSuites => Self::RegressionSuites,
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunSweepMaintenanceResult {
    pub swept: u64,
    pub grace_secs: u64,
    pub batch_size: u64,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TelemetryAlertsMaintenanceResult {
    pub evaluated: u64,
    pub triggered: u64,
    pub resolved: u64,
}

/// Expired execution runs/events removed from the selected live state store.
/// Backends with native TTL delete on their own and report zero here.
#[derive(Clone, Copy, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StateCleanupMaintenanceResult {
    pub deleted_runs: i64,
    pub deleted_events: i64,
}

/// Regression-suite maintenance: scheduled suite runs dispatched this tick,
/// stuck `running` suite runs flipped to `errored`, and `queued` suite runs
/// executed inline.
#[derive(Clone, Copy, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RegressionSuitesMaintenanceResult {
    pub dispatched: u64,
    pub swept: u64,
    /// Default tolerates responses from an API without queued-run support.
    #[serde(default)]
    pub executed: u64,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(tag = "job", content = "result", rename_all = "snake_case")]
pub enum MaintenanceRunResponse {
    TelemetryAlerts(TelemetryAlertsMaintenanceResult),
    CacheCleanup(CacheCleanupResult),
    RunSweep(RunSweepMaintenanceResult),
    StateCleanup(StateCleanupMaintenanceResult),
    RegressionSuites(RegressionSuitesMaintenanceResult),
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn request_contract_is_stable() {
        assert_eq!(
            serde_json::to_value(MaintenanceRunRequest::TelemetryAlerts).unwrap(),
            json!({ "job": "telemetry_alerts" })
        );
        assert_eq!(
            serde_json::from_value::<MaintenanceRunRequest>(json!({ "job": "telemetry_alerts" }))
                .unwrap(),
            MaintenanceRunRequest::TelemetryAlerts
        );
    }

    #[test]
    fn unknown_jobs_are_rejected() {
        assert!(
            serde_json::from_value::<MaintenanceRunRequest>(json!({ "job": "delete_everything" }))
                .is_err()
        );
    }

    #[test]
    fn cache_cleanup_round_trips() {
        assert_eq!(
            serde_json::to_value(MaintenanceRunRequest::CacheCleanup).unwrap(),
            json!({ "job": "cache_cleanup" })
        );
        assert_eq!(
            MaintenanceRunRequest::CacheCleanup.job().as_str(),
            "cache_cleanup"
        );
        assert_eq!(
            serde_json::to_value(MaintenanceRunResponse::CacheCleanup(CacheCleanupResult {
                deleted: 7
            }))
            .unwrap(),
            json!({ "job": "cache_cleanup", "result": { "deleted": 7 } })
        );
    }

    #[test]
    fn run_sweep_round_trips() {
        assert_eq!(
            serde_json::to_value(MaintenanceRunRequest::RunSweep).unwrap(),
            json!({ "job": "run_sweep" })
        );
        assert_eq!(MaintenanceRunRequest::RunSweep.job().as_str(), "run_sweep");
        assert_eq!(
            serde_json::to_value(MaintenanceRunResponse::RunSweep(
                RunSweepMaintenanceResult {
                    swept: 7,
                    grace_secs: 3_600,
                    batch_size: 500,
                }
            ))
            .unwrap(),
            json!({
                "job": "run_sweep",
                "result": {
                    "swept": 7,
                    "graceSecs": 3_600,
                    "batchSize": 500,
                }
            })
        );
    }

    #[test]
    fn state_cleanup_round_trips() {
        assert_eq!(
            serde_json::to_value(MaintenanceRunRequest::StateCleanup).unwrap(),
            json!({ "job": "state_cleanup" })
        );
        assert_eq!(
            MaintenanceRunRequest::StateCleanup.job().as_str(),
            "state_cleanup"
        );
        assert_eq!(
            serde_json::to_value(MaintenanceRunResponse::StateCleanup(
                StateCleanupMaintenanceResult {
                    deleted_runs: 3,
                    deleted_events: 41,
                }
            ))
            .unwrap(),
            json!({
                "job": "state_cleanup",
                "result": {
                    "deletedRuns": 3,
                    "deletedEvents": 41,
                }
            })
        );
    }

    #[test]
    fn regression_suites_round_trips() {
        assert_eq!(
            serde_json::to_value(MaintenanceRunRequest::RegressionSuites).unwrap(),
            json!({ "job": "regression_suites" })
        );
        assert_eq!(
            MaintenanceRunRequest::RegressionSuites.job().as_str(),
            "regression_suites"
        );
        assert_eq!(
            serde_json::to_value(MaintenanceRunResponse::RegressionSuites(
                RegressionSuitesMaintenanceResult {
                    dispatched: 2,
                    swept: 1,
                    executed: 3,
                }
            ))
            .unwrap(),
            json!({
                "job": "regression_suites",
                "result": {
                    "dispatched": 2,
                    "swept": 1,
                    "executed": 3,
                }
            })
        );
        assert_eq!(
            serde_json::from_value::<RegressionSuitesMaintenanceResult>(json!({
                "dispatched": 2,
                "swept": 1,
            }))
            .unwrap()
            .executed,
            0
        );
    }

    #[test]
    fn response_contract_is_stable() {
        let response = MaintenanceRunResponse::TelemetryAlerts(TelemetryAlertsMaintenanceResult {
            evaluated: 12,
            triggered: 2,
            resolved: 1,
        });

        assert_eq!(
            serde_json::to_value(response).unwrap(),
            json!({
                "job": "telemetry_alerts",
                "result": {
                    "evaluated": 12,
                    "triggered": 2,
                    "resolved": 1
                }
            })
        );
    }
}
