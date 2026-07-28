//! Shared contract between the maintenance Lambda and the API.
//!
//! Keep this surface deliberately small: the Lambda may request one named,
//! allowlisted job per invocation, while the API owns all data access and job
//! implementation details.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MaintenanceJob {
    TelemetryAlerts,
}

impl MaintenanceJob {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::TelemetryAlerts => "telemetry_alerts",
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(tag = "job", rename_all = "snake_case")]
pub enum MaintenanceRunRequest {
    TelemetryAlerts,
}

impl MaintenanceRunRequest {
    pub const fn job(self) -> MaintenanceJob {
        match self {
            Self::TelemetryAlerts => MaintenanceJob::TelemetryAlerts,
        }
    }
}

impl From<MaintenanceJob> for MaintenanceRunRequest {
    fn from(job: MaintenanceJob) -> Self {
        match job {
            MaintenanceJob::TelemetryAlerts => Self::TelemetryAlerts,
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TelemetryAlertsMaintenanceResult {
    pub evaluated: u64,
    pub triggered: u64,
    pub resolved: u64,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(tag = "job", content = "result", rename_all = "snake_case")]
pub enum MaintenanceRunResponse {
    TelemetryAlerts(TelemetryAlertsMaintenanceResult),
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
