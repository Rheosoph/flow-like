//! Sink-trigger API client.
//!
//! Two calls, both authenticated with the pre-minted cron-scoped
//! `SINK_TRIGGER_JWT`: `GET /sink/schedules` and `POST /sink/trigger/async`.
//! The wire shapes are those of `packages/api/src/routes/sink/trigger.rs`
//! (`CronScheduleInfo`, `ServiceTriggerRequest`, `ServiceTriggerResponse`).
//! The trait exists so the tick algorithm can be tested against a mock without
//! a server; `ApiClient` is the only production implementation.

use crate::TickConfig;
use async_trait::async_trait;
use chrono::{DateTime, SecondsFormat, Utc};
use reqwest::StatusCode;
use serde::{Deserialize, Serialize};
use std::time::Duration;

const HTTP_CONNECT_TIMEOUT_SECS: u64 = 5;
const HTTP_TIMEOUT_SECS: u64 = 30;
const MAX_ERROR_BODY_BYTES: usize = 2_048;

#[derive(Debug, thiserror::Error)]
pub enum ApiError {
    #[error("API configuration error: {0}")]
    Configuration(String),
    #[error("API transport error: {0}")]
    Transport(String),
    #[error("API returned HTTP {status}: {body}")]
    Status { status: StatusCode, body: String },
    /// HTTP 200 with `success: false`: the API accepted the request but did not
    /// start a run (event disabled, executor unavailable, ...). It is a fire
    /// failure like any other so the operator sees it in the job's exit code.
    #[error("API declined the trigger: {0}")]
    Declined(String),
    #[error("API response could not be parsed: {0}")]
    Parse(String),
}

/// One row of `GET /sink/schedules`. Extra fields are ignored so the API can
/// grow the shape without breaking the tick.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct CronScheduleInfo {
    pub id: String,
    pub event_id: String,
    pub cron_expression: String,
    pub app_id: String,
    pub enabled: bool,
    #[serde(default)]
    pub last_triggered: Option<DateTime<Utc>>,
    #[serde(default)]
    pub next_trigger: Option<DateTime<Utc>>,
}

#[derive(Serialize)]
struct ServiceTriggerRequest<'a> {
    event_id: &'a str,
    sink_type: &'static str,
    payload: TriggerPayload,
}

#[derive(Serialize)]
struct TriggerPayload {
    scheduled_for: String,
}

#[derive(Deserialize)]
struct ServiceTriggerResponse {
    success: bool,
    #[serde(default)]
    run_id: Option<String>,
    #[serde(default)]
    error: Option<String>,
}

#[async_trait]
pub trait TriggerApi: Send + Sync {
    async fn list_cron_schedules(&self) -> Result<Vec<CronScheduleInfo>, ApiError>;
    async fn trigger(&self, event_id: &str, occurrence: DateTime<Utc>) -> Result<(), ApiError>;
}

/// The occurrence in the form used for both the `Idempotency-Key` and the
/// `scheduled_for` payload field. Occurrences are whole seconds, so `Secs`
/// loses nothing and keeps the key identical across ticks that recompute it.
pub fn occurrence_rfc3339(occurrence: DateTime<Utc>) -> String {
    occurrence.to_rfc3339_opts(SecondsFormat::Secs, true)
}

/// `cron:{event_id}:{occurrence}` — the API caches the response under this key
/// for ~15 minutes per replica, so a retried POST for the same occurrence returns
/// the cached result instead of starting a second run.
pub fn idempotency_key(event_id: &str, occurrence: DateTime<Utc>) -> String {
    format!("cron:{event_id}:{}", occurrence_rfc3339(occurrence))
}

#[derive(Clone)]
pub struct ApiClient {
    http: reqwest::Client,
    schedules_url: String,
    trigger_url: String,
    jwt: crate::Secret,
}

impl std::fmt::Debug for ApiClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ApiClient")
            .field("schedules_url", &self.schedules_url)
            .field("trigger_url", &self.trigger_url)
            .finish_non_exhaustive()
    }
}

impl ApiClient {
    /// `config.api_base_url` must already be normalised (see
    /// [`crate::normalize_api_base_url`]); the client refuses plaintext HTTP at
    /// the transport layer unless the same override that let the URL through
    /// is set, so a later misconfiguration cannot downgrade the connection.
    pub fn new(config: &TickConfig) -> Result<Self, ApiError> {
        let http = reqwest::Client::builder()
            .https_only(!config.allow_insecure_api_base_url)
            .connect_timeout(Duration::from_secs(HTTP_CONNECT_TIMEOUT_SECS))
            .timeout(Duration::from_secs(HTTP_TIMEOUT_SECS))
            .user_agent(concat!(
                "flow-like-scheduler-tick/",
                env!("CARGO_PKG_VERSION")
            ))
            .build()
            .map_err(|error| ApiError::Configuration(error.to_string()))?;
        let base = config.api_base_url.trim_end_matches('/');
        Ok(Self {
            http,
            schedules_url: format!("{base}/api/v1/sink/schedules"),
            trigger_url: format!("{base}/api/v1/sink/trigger/async"),
            jwt: config.jwt.clone(),
        })
    }
}

#[async_trait]
impl TriggerApi for ApiClient {
    async fn list_cron_schedules(&self) -> Result<Vec<CronScheduleInfo>, ApiError> {
        let response = self
            .http
            .get(&self.schedules_url)
            .bearer_auth(self.jwt.expose())
            .send()
            .await
            .map_err(|error| ApiError::Transport(error.to_string()))?;
        let status = response.status();
        let body = response
            .bytes()
            .await
            .map_err(|error| ApiError::Transport(error.to_string()))?;
        if !status.is_success() {
            return Err(status_error(status, &body));
        }
        serde_json::from_slice(&body).map_err(|error| ApiError::Parse(error.to_string()))
    }

    async fn trigger(&self, event_id: &str, occurrence: DateTime<Utc>) -> Result<(), ApiError> {
        let request = ServiceTriggerRequest {
            event_id,
            sink_type: "cron",
            payload: TriggerPayload {
                scheduled_for: occurrence_rfc3339(occurrence),
            },
        };
        let response = self
            .http
            .post(&self.trigger_url)
            .bearer_auth(self.jwt.expose())
            .header("Idempotency-Key", idempotency_key(event_id, occurrence))
            .json(&request)
            .send()
            .await
            .map_err(|error| ApiError::Transport(error.to_string()))?;
        let status = response.status();
        let body = response
            .bytes()
            .await
            .map_err(|error| ApiError::Transport(error.to_string()))?;
        if !status.is_success() {
            return Err(status_error(status, &body));
        }
        let parsed: ServiceTriggerResponse =
            serde_json::from_slice(&body).map_err(|error| ApiError::Parse(error.to_string()))?;
        if !parsed.success {
            return Err(ApiError::Declined(
                parsed
                    .error
                    .unwrap_or_else(|| "no reason given".to_string()),
            ));
        }
        tracing::debug!(
            event_id,
            occurrence = %occurrence_rfc3339(occurrence),
            run_id = parsed.run_id.as_deref().unwrap_or("-"),
            "cron trigger accepted"
        );
        Ok(())
    }
}

fn status_error(status: StatusCode, body: &[u8]) -> ApiError {
    let body = &body[..body.len().min(MAX_ERROR_BODY_BYTES)];
    ApiError::Status {
        status,
        body: String::from_utf8_lossy(body).into_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn schedule_rows_deserialize_from_the_api_shape() {
        let rows: Vec<CronScheduleInfo> = serde_json::from_value(serde_json::json!([
            {
                "id": "evt_1",
                "event_id": "evt_1",
                "cron_expression": "*/5 * * * *",
                "app_id": "app_1",
                "enabled": true,
                "last_triggered": null,
                "next_trigger": null,
                "some_future_field": 42
            }
        ]))
        .unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].event_id, "evt_1");
        assert_eq!(rows[0].app_id, "app_1");
        assert!(rows[0].enabled);
        assert!(rows[0].last_triggered.is_none());
    }

    #[test]
    fn idempotency_key_and_payload_share_the_same_rfc3339_form() {
        let occurrence = Utc.with_ymd_and_hms(2026, 8, 15, 12, 5, 0).unwrap();
        assert_eq!(
            idempotency_key("evt_1", occurrence),
            "cron:evt_1:2026-08-15T12:05:00Z"
        );
        let request = ServiceTriggerRequest {
            event_id: "evt_1",
            sink_type: "cron",
            payload: TriggerPayload {
                scheduled_for: occurrence_rfc3339(occurrence),
            },
        };
        assert_eq!(
            serde_json::to_value(&request).unwrap(),
            serde_json::json!({
                "event_id": "evt_1",
                "sink_type": "cron",
                "payload": { "scheduled_for": "2026-08-15T12:05:00Z" }
            })
        );
    }

    #[test]
    fn declined_triggers_surface_the_api_reason() {
        let parsed: ServiceTriggerResponse =
            serde_json::from_str(r#"{"success":false,"run_id":null,"error":"event disabled"}"#)
                .unwrap();
        assert!(!parsed.success);
        assert_eq!(parsed.error.as_deref(), Some("event disabled"));
    }
}
