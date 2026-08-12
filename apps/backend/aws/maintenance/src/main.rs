#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

use std::{env, sync::OnceLock, time::Duration};

use flow_like_types::{
    maintenance::{MaintenanceJob, MaintenanceRunRequest, MaintenanceRunResponse},
    tokio,
};
use lambda_runtime::{Error, LambdaEvent, run, service_fn, tracing};
use reqwest::StatusCode;
use serde::{Deserialize, Serialize};

const HTTP_CONNECT_TIMEOUT_SECS: u64 = 5;
const HTTP_TIMEOUT_SECS: u64 = 120;
const MIN_TOKEN_BYTES: usize = 32;
const SCHEDULER_CONTEXT_PLACEHOLDER_PREFIX: &str = "<aws.scheduler.";

static API_BASE_URL: OnceLock<String> = OnceLock::new();
static HTTP_CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
static MAINTENANCE_TOKEN: OnceLock<String> = OnceLock::new();

#[derive(Debug, Deserialize, Serialize)]
struct ScheduledMaintenancePayload {
    job: MaintenanceJob,
    #[serde(default)]
    scheduled_time: Option<String>,
    #[serde(default)]
    schedule_arn: Option<String>,
    #[serde(default)]
    execution_id: Option<String>,
    #[serde(default)]
    attempt_number: Option<String>,
}

fn get_http_client() -> Result<&'static reqwest::Client, Error> {
    if let Some(client) = HTTP_CLIENT.get() {
        return Ok(client);
    }

    let client = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(HTTP_CONNECT_TIMEOUT_SECS))
        .timeout(Duration::from_secs(HTTP_TIMEOUT_SECS))
        .build()
        .map_err(|error| {
            tracing::error!(%error, "Failed to build maintenance HTTP client");
            Error::from(format!("Failed to build HTTP client: {error}"))
        })?;

    let _ = HTTP_CLIENT.set(client);
    HTTP_CLIENT
        .get()
        .ok_or_else(|| Error::from("HTTP client failed to initialize"))
}

fn get_api_base_url() -> Result<&'static str, Error> {
    if let Some(value) = API_BASE_URL.get() {
        return Ok(value);
    }

    let value = env::var("API_BASE_URL").map_err(|_| Error::from("API_BASE_URL not set"))?;
    let allow_insecure = env::var("ALLOW_INSECURE_API_BASE_URL")
        .ok()
        .is_some_and(|value| matches!(value.trim().to_ascii_lowercase().as_str(), "1" | "true"));
    let value = normalize_api_base_url(&value, allow_insecure).map_err(Error::from)?;

    let _ = API_BASE_URL.set(value);
    Ok(API_BASE_URL
        .get()
        .expect("API_BASE_URL must be initialized"))
}

fn normalize_api_base_url(value: &str, allow_insecure: bool) -> Result<String, String> {
    let value = value.trim();
    let parsed = reqwest::Url::parse(value)
        .map_err(|error| format!("API_BASE_URL is not a valid URL: {error}"))?;

    if parsed.host_str().is_none() || parsed.cannot_be_a_base() {
        return Err("API_BASE_URL must be an absolute HTTP(S) URL".to_string());
    }
    if !parsed.username().is_empty() || parsed.password().is_some() {
        return Err("API_BASE_URL must not contain credentials".to_string());
    }
    if parsed.query().is_some() || parsed.fragment().is_some() {
        return Err("API_BASE_URL must not contain a query or fragment".to_string());
    }
    if parsed.scheme() != "https" && !(allow_insecure && parsed.scheme() == "http") {
        return Err(
            "API_BASE_URL must use HTTPS; set ALLOW_INSECURE_API_BASE_URL=1 only for trusted development or private networking"
                .to_string(),
        );
    }

    Ok(parsed.as_str().trim_end_matches('/').to_string())
}

fn get_maintenance_token() -> Result<&'static str, Error> {
    if let Some(value) = MAINTENANCE_TOKEN.get() {
        return Ok(value);
    }

    let value =
        env::var("MAINTENANCE_TOKEN").map_err(|_| Error::from("MAINTENANCE_TOKEN not set"))?;
    let value = value.trim().to_string();
    if value.len() < MIN_TOKEN_BYTES {
        return Err(Error::from(
            "MAINTENANCE_TOKEN must contain at least 32 bytes",
        ));
    }

    let _ = MAINTENANCE_TOKEN.set(value);
    Ok(MAINTENANCE_TOKEN
        .get()
        .expect("MAINTENANCE_TOKEN must be initialized"))
}

fn maintenance_url(api_base_url: &str) -> String {
    format!(
        "{}/api/v1/maintenance/run",
        api_base_url.trim_end_matches('/')
    )
}

fn scheduler_context_value(value: &Option<String>) -> Option<&str> {
    value
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .filter(|value| !value.starts_with(SCHEDULER_CONTEXT_PLACEHOLDER_PREFIX))
}

fn build_idempotency_key(payload: &ScheduledMaintenancePayload, request_id: &str) -> String {
    if let Some(scheduled_time) = scheduler_context_value(&payload.scheduled_time) {
        let schedule_id =
            scheduler_context_value(&payload.schedule_arn).unwrap_or_else(|| payload.job.as_str());
        return format!(
            "aws-scheduler:{}:{}:{}",
            payload.job.as_str(),
            schedule_id,
            scheduled_time
        );
    }

    if let Some(execution_id) = scheduler_context_value(&payload.execution_id) {
        let schedule_id =
            scheduler_context_value(&payload.schedule_arn).unwrap_or_else(|| payload.job.as_str());
        return format!(
            "aws-scheduler-execution:{}:{}:{}",
            payload.job.as_str(),
            schedule_id,
            execution_id
        );
    }

    format!("lambda-request:{request_id}")
}

fn is_transient_status(status: StatusCode) -> bool {
    status.is_server_error()
        || matches!(
            status,
            StatusCode::REQUEST_TIMEOUT | StatusCode::TOO_MANY_REQUESTS
        )
}

#[tokio::main]
async fn main() -> Result<(), Error> {
    tracing::init_default_subscriber();
    run(service_fn(maintenance_handler)).await
}

async fn maintenance_handler(event: LambdaEvent<ScheduledMaintenancePayload>) -> Result<(), Error> {
    let api_base_url = get_api_base_url()?;
    let maintenance_token = get_maintenance_token()?;
    let payload = event.payload;
    let idempotency_key = build_idempotency_key(&payload, &event.context.request_id);

    tracing::info!(
        job = payload.job.as_str(),
        idempotency_key = %idempotency_key,
        scheduled_time = ?scheduler_context_value(&payload.scheduled_time),
        schedule_arn = ?scheduler_context_value(&payload.schedule_arn),
        execution_id = ?scheduler_context_value(&payload.execution_id),
        attempt_number = ?scheduler_context_value(&payload.attempt_number),
        "Running scheduled maintenance job"
    );

    let response = get_http_client()?
        .post(maintenance_url(api_base_url))
        .bearer_auth(maintenance_token)
        .header("Idempotency-Key", &idempotency_key)
        .json(&MaintenanceRunRequest::from(payload.job))
        .send()
        .await
        .map_err(|error| {
            tracing::error!(
                %error,
                job = payload.job.as_str(),
                "Maintenance API request failed"
            );
            Error::from(format!("Maintenance API request failed: {error}"))
        })?;

    let status = response.status();
    let body = response.text().await.map_err(|error| {
        tracing::error!(%error, "Failed to read maintenance API response");
        Error::from(format!("Failed to read maintenance API response: {error}"))
    })?;

    if !status.is_success() {
        if !is_transient_status(status) {
            tracing::error!(
                %status,
                body = %body,
                job = payload.job.as_str(),
                "Maintenance API rejected the request; failing the invocation so Lambda asynchronous retry handling and the on-failure destination surface the configuration error"
            );
        } else {
            tracing::error!(
                %status,
                body = %body,
                job = payload.job.as_str(),
                "Maintenance API returned a transient error"
            );
        }

        return Err(Error::from(format!(
            "Maintenance API error: {status} - {body}"
        )));
    }

    let parsed: MaintenanceRunResponse = serde_json::from_str(&body).map_err(|error| {
        tracing::error!(
            %error,
            body = %body,
            "Maintenance API returned an invalid success response"
        );
        Error::from(format!("Invalid maintenance API response: {error}"))
    })?;

    match (payload.job, parsed) {
        (MaintenanceJob::TelemetryAlerts, MaintenanceRunResponse::TelemetryAlerts(result)) => {
            tracing::info!(
                evaluated = result.evaluated,
                triggered = result.triggered,
                resolved = result.resolved,
                "Telemetry alert maintenance completed"
            )
        }
        (MaintenanceJob::CacheCleanup, MaintenanceRunResponse::CacheCleanup(result)) => {
            tracing::info!(
                deleted = result.deleted,
                "Cache cleanup maintenance completed"
            )
        }
        (job, response) => {
            // The API answered for a different job than we asked for. Fail the
            // invocation so the mismatch surfaces instead of being logged as success.
            tracing::error!(
                requested = job.as_str(),
                response = ?response,
                "Maintenance API responded for a different job"
            );
            return Err(Error::from(format!(
                "Maintenance API responded for a different job than {}",
                job.as_str()
            )));
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn payload(scheduled_time: Option<&str>) -> ScheduledMaintenancePayload {
        ScheduledMaintenancePayload {
            job: MaintenanceJob::TelemetryAlerts,
            scheduled_time: scheduled_time.map(str::to_string),
            schedule_arn: Some(
                "arn:aws:scheduler:eu-west-1:123456789012:schedule/default/telemetry-alerts"
                    .to_string(),
            ),
            execution_id: None,
            attempt_number: None,
        }
    }

    #[test]
    fn scheduler_payload_uses_the_allowlisted_job_contract() {
        let parsed: ScheduledMaintenancePayload = serde_json::from_value(serde_json::json!({
            "job": "telemetry_alerts",
            "scheduled_time": "2026-07-27T10:00:00Z"
        }))
        .unwrap();

        assert_eq!(parsed.job, MaintenanceJob::TelemetryAlerts);
    }

    #[test]
    fn unknown_jobs_are_rejected() {
        assert!(
            serde_json::from_value::<ScheduledMaintenancePayload>(
                serde_json::json!({ "job": "arbitrary_sql" })
            )
            .is_err()
        );
    }

    #[test]
    fn idempotency_key_is_stable_across_lambda_attempts() {
        let payload = payload(Some("2026-07-27T10:00:00Z"));

        assert_eq!(
            build_idempotency_key(&payload, "first-request"),
            build_idempotency_key(&payload, "retry-request")
        );
        assert_eq!(
            build_idempotency_key(&payload, "first-request"),
            "aws-scheduler:telemetry_alerts:arn:aws:scheduler:eu-west-1:123456789012:schedule/default/telemetry-alerts:2026-07-27T10:00:00Z"
        );
    }

    #[test]
    fn idempotency_key_falls_back_to_the_lambda_request_id() {
        assert_eq!(
            build_idempotency_key(&payload(None), "request-1"),
            "lambda-request:request-1"
        );
    }

    #[test]
    fn execution_id_is_a_stable_fallback_for_scheduler_events() {
        let mut payload = payload(None);
        payload.execution_id = Some("scheduler-execution-1".to_string());

        assert_eq!(
            build_idempotency_key(&payload, "first-request"),
            build_idempotency_key(&payload, "lambda-async-retry")
        );
        assert_eq!(
            build_idempotency_key(&payload, "first-request"),
            "aws-scheduler-execution:telemetry_alerts:arn:aws:scheduler:eu-west-1:123456789012:schedule/default/telemetry-alerts:scheduler-execution-1"
        );
    }

    #[test]
    fn unexpanded_scheduler_placeholders_are_ignored() {
        let mut payload = payload(Some("<aws.scheduler.scheduled-time>"));
        payload.schedule_arn = Some("<aws.scheduler.schedule-arn>".to_string());

        assert_eq!(
            build_idempotency_key(&payload, "request-1"),
            "lambda-request:request-1"
        );
    }

    #[test]
    fn url_is_normalized() {
        assert_eq!(
            maintenance_url("https://api.example.com/"),
            "https://api.example.com/api/v1/maintenance/run"
        );
        assert_eq!(
            normalize_api_base_url(" https://api.example.com/ ", false).unwrap(),
            "https://api.example.com"
        );
    }

    #[test]
    fn insecure_api_urls_require_an_explicit_override() {
        assert!(normalize_api_base_url("http://api.internal", false).is_err());
        assert_eq!(
            normalize_api_base_url("http://api.internal/", true).unwrap(),
            "http://api.internal"
        );
        assert!(normalize_api_base_url("ftp://api.example.com", true).is_err());
    }

    #[test]
    fn http_status_classification_marks_only_transient_failures() {
        assert!(is_transient_status(StatusCode::REQUEST_TIMEOUT));
        assert!(is_transient_status(StatusCode::TOO_MANY_REQUESTS));
        assert!(is_transient_status(StatusCode::BAD_GATEWAY));
        assert!(!is_transient_status(StatusCode::UNAUTHORIZED));
        assert!(!is_transient_status(StatusCode::NOT_FOUND));
    }
}
