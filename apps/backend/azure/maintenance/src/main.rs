#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

use std::{env, process::ExitCode, time::Duration};

use chrono::{DateTime, Utc};
use flow_like_types_contracts::maintenance::{
    MaintenanceJob, MaintenanceRunRequest, MaintenanceRunResponse,
};
use reqwest::StatusCode;
use tracing_subscriber::{EnvFilter, layer::SubscriberExt, util::SubscriberInitExt};

const HTTP_CONNECT_TIMEOUT_SECS: u64 = 5;
const HTTP_TIMEOUT_SECS: u64 = 120;
const MIN_TOKEN_BYTES: usize = 32;
/// Container Apps injects the same execution name into every replica of one
/// job execution, retries included, so it is the retry-stable key suffix; the
/// next scheduled or manually started execution gets a fresh one.
const EXECUTION_NAME_ENV: &str = "CONTAINER_APP_JOB_EXECUTION_NAME";
/// `API_BASE_URL` is the canonical name every other API caller uses; the Azure
/// root only sets `API_URL` on this job, so both are accepted in this order.
const API_BASE_URL_ENVS: &[&str] = &["API_BASE_URL", "API_URL"];
/// `MAINTENANCE_JOB=all` runs these in order. Each is its own request with its
/// own idempotency key, so a failure in one never masks the job that passed.
const ALL_JOBS: &[MaintenanceJob] = &[
    MaintenanceJob::TelemetryAlerts,
    MaintenanceJob::CacheCleanup,
];

struct Config {
    api_base_url: String,
    /// Which of `API_BASE_URL_ENVS` supplied the value, for the startup log.
    api_base_url_variable: &'static str,
    maintenance_token: String,
    jobs: Vec<MaintenanceJob>,
    execution_name: Option<String>,
}

impl Config {
    fn from_env() -> Result<Self, String> {
        let allow_insecure = env::var("ALLOW_INSECURE_API_BASE_URL")
            .ok()
            .is_some_and(|value| {
                matches!(value.trim().to_ascii_lowercase().as_str(), "1" | "true")
            });
        let (api_base_url_variable, raw_api_base_url) = select_api_base_url(non_blank_env)?;
        let api_base_url = normalize_api_base_url(&raw_api_base_url, allow_insecure)
            .map_err(|error| format!("{error} (read from {api_base_url_variable})"))?;

        let maintenance_token = env::var("MAINTENANCE_TOKEN")
            .map_err(|_| "MAINTENANCE_TOKEN not set".to_string())?
            .trim()
            .to_string();
        if maintenance_token.len() < MIN_TOKEN_BYTES {
            return Err("MAINTENANCE_TOKEN must contain at least 32 bytes".to_string());
        }

        let jobs = parse_maintenance_jobs(non_blank_env("MAINTENANCE_JOB").as_deref())?;
        let execution_name = non_blank_env(EXECUTION_NAME_ENV);

        Ok(Self {
            api_base_url,
            api_base_url_variable,
            maintenance_token,
            jobs,
            execution_name,
        })
    }
}

fn non_blank_env(name: &str) -> Option<String> {
    env::var(name)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn select_api_base_url(
    lookup: impl Fn(&str) -> Option<String>,
) -> Result<(&'static str, String), String> {
    API_BASE_URL_ENVS
        .iter()
        .find_map(|name| lookup(name).map(|value| (*name, value)))
        .ok_or_else(|| "API_BASE_URL not set (API_URL is accepted as a fallback)".to_string())
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

/// Routes the value through the shared serde contract so the allowlist lives in
/// exactly one place (`flow_like_types_contracts::maintenance`), the same way the AWS
/// scheduler payload was typed instead of string-matched.
fn parse_maintenance_jobs(value: Option<&str>) -> Result<Vec<MaintenanceJob>, String> {
    match value.map(str::trim) {
        None | Some("") | Some("all") => Ok(ALL_JOBS.to_vec()),
        Some(name) => {
            serde_json::from_value::<MaintenanceJob>(serde_json::Value::String(name.to_string()))
                .map(|job| vec![job])
                .map_err(|error| {
                    format!(
                        "MAINTENANCE_JOB must be telemetry_alerts, cache_cleanup or all: {error}"
                    )
                })
        }
    }
}

fn maintenance_url(api_base_url: &str) -> String {
    format!(
        "{}/api/v1/maintenance/run",
        api_base_url.trim_end_matches('/')
    )
}

/// Blank values count as absent so a run outside Container Apps (`docker run`,
/// a local shell) still gets a deterministic minute-scoped key.
fn execution_context_value(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|value| !value.is_empty())
}

/// The platform starts the job at its scheduled minute; flooring `now` lets a
/// retry inside that minute share the key while the next run gets a new one.
fn scheduled_minute(now: DateTime<Utc>) -> String {
    now.format("%Y-%m-%dT%H:%M:00Z").to_string()
}

fn build_idempotency_key(
    job: MaintenanceJob,
    execution_name: Option<&str>,
    now: DateTime<Utc>,
) -> String {
    match execution_context_value(execution_name) {
        Some(execution_name) => format!("{}:{}", job.as_str(), execution_name),
        None => format!("{}:{}", job.as_str(), scheduled_minute(now)),
    }
}

fn is_transient_status(status: StatusCode) -> bool {
    status.is_server_error()
        || matches!(
            status,
            StatusCode::REQUEST_TIMEOUT | StatusCode::TOO_MANY_REQUESTS
        )
}

fn build_http_client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(HTTP_CONNECT_TIMEOUT_SECS))
        .timeout(Duration::from_secs(HTTP_TIMEOUT_SECS))
        .build()
        .map_err(|error| format!("Failed to build HTTP client: {error}"))
}

#[tokio::main]
async fn main() -> ExitCode {
    tracing_subscriber::registry()
        .with(EnvFilter::try_from_default_env().unwrap_or_else(|_| {
            EnvFilter::new("info")
                .add_directive("hyper=warn".parse().expect("valid filter"))
                .add_directive("rustls=warn".parse().expect("valid filter"))
                .add_directive("tokio=warn".parse().expect("valid filter"))
        }))
        .with(tracing_subscriber::fmt::layer())
        .init();

    let config = match Config::from_env() {
        Ok(config) => config,
        Err(error) => {
            tracing::error!(%error, "Invalid maintenance job configuration");
            return ExitCode::FAILURE;
        }
    };
    let client = match build_http_client() {
        Ok(client) => client,
        Err(error) => {
            tracing::error!(%error, "Failed to build maintenance HTTP client");
            return ExitCode::FAILURE;
        }
    };

    tracing::info!(
        api_base_url = %config.api_base_url,
        api_base_url_variable = config.api_base_url_variable,
        jobs = ?config.jobs.iter().map(|job| job.as_str()).collect::<Vec<_>>(),
        execution_name = config.execution_name.as_deref(),
        "Starting maintenance run"
    );

    // One timestamp for the whole run: in `all` mode the second job must not
    // roll into a new minute-scoped key just because the first one was slow.
    let started_at = Utc::now();
    let mut failed = 0usize;
    for &job in &config.jobs {
        let idempotency_key =
            build_idempotency_key(job, config.execution_name.as_deref(), started_at);
        if let Err(error) = run_job(&client, &config, job, &idempotency_key).await {
            tracing::error!(job = job.as_str(), %error, "Maintenance job failed");
            failed += 1;
        }
    }

    if failed > 0 {
        tracing::error!(
            failed,
            requested = config.jobs.len(),
            "Maintenance run finished with failures"
        );
        return ExitCode::FAILURE;
    }

    tracing::info!(requested = config.jobs.len(), "Maintenance run completed");
    ExitCode::SUCCESS
}

async fn run_job(
    client: &reqwest::Client,
    config: &Config,
    job: MaintenanceJob,
    idempotency_key: &str,
) -> Result<(), String> {
    tracing::info!(
        job = job.as_str(),
        idempotency_key = %idempotency_key,
        "Running scheduled maintenance job"
    );

    let response = client
        .post(maintenance_url(&config.api_base_url))
        .bearer_auth(&config.maintenance_token)
        .header("Idempotency-Key", idempotency_key)
        .json(&MaintenanceRunRequest::from(job))
        .send()
        .await
        .map_err(|error| {
            tracing::error!(%error, job = job.as_str(), "Maintenance API request failed");
            format!("Maintenance API request failed: {error}")
        })?;

    let status = response.status();
    let body = response.text().await.map_err(|error| {
        tracing::error!(%error, "Failed to read maintenance API response");
        format!("Failed to read maintenance API response: {error}")
    })?;

    if !status.is_success() {
        if !is_transient_status(status) {
            tracing::error!(
                %status,
                body = %body,
                job = job.as_str(),
                "Maintenance API rejected the request; failing the job so the Container Apps execution history and retry policy surface the configuration error"
            );
        } else {
            tracing::error!(
                %status,
                body = %body,
                job = job.as_str(),
                "Maintenance API returned a transient error"
            );
        }

        return Err(format!("Maintenance API error: {status} - {body}"));
    }

    let parsed: MaintenanceRunResponse = serde_json::from_str(&body).map_err(|error| {
        tracing::error!(
            %error,
            body = %body,
            "Maintenance API returned an invalid success response"
        );
        format!("Invalid maintenance API response: {error}")
    })?;

    match (job, parsed) {
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
            // job so the mismatch surfaces instead of being logged as success.
            tracing::error!(
                requested = job.as_str(),
                response = ?response,
                "Maintenance API responded for a different job"
            );
            return Err(format!(
                "Maintenance API responded for a different job than {}",
                job.as_str()
            ));
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    const EXECUTION_NAME: &str = "flow-like-dev-maintenance-abc123";

    fn at(hour: u32, minute: u32, second: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 7, 27, hour, minute, second)
            .unwrap()
    }

    #[test]
    fn maintenance_job_env_uses_the_allowlisted_job_contract() {
        assert_eq!(
            parse_maintenance_jobs(Some("telemetry_alerts")).unwrap(),
            vec![MaintenanceJob::TelemetryAlerts]
        );
        assert_eq!(
            parse_maintenance_jobs(Some(" cache_cleanup ")).unwrap(),
            vec![MaintenanceJob::CacheCleanup]
        );
    }

    #[test]
    fn unknown_jobs_are_rejected() {
        assert!(parse_maintenance_jobs(Some("arbitrary_sql")).is_err());
        assert!(parse_maintenance_jobs(Some("telemetry_alerts,cache_cleanup")).is_err());
        assert!(parse_maintenance_jobs(Some("TelemetryAlerts")).is_err());
    }

    #[test]
    fn all_is_the_default_and_runs_both_jobs_in_order() {
        let both = vec![
            MaintenanceJob::TelemetryAlerts,
            MaintenanceJob::CacheCleanup,
        ];
        assert_eq!(parse_maintenance_jobs(None).unwrap(), both);
        assert_eq!(parse_maintenance_jobs(Some("")).unwrap(), both);
        assert_eq!(parse_maintenance_jobs(Some(" all ")).unwrap(), both);
    }

    #[test]
    fn api_base_url_wins_over_api_url() {
        let both = |name: &str| match name {
            "API_BASE_URL" => Some("https://canonical.example.com".to_string()),
            "API_URL" => Some("https://api".to_string()),
            _ => None,
        };
        assert_eq!(
            select_api_base_url(both).unwrap(),
            ("API_BASE_URL", "https://canonical.example.com".to_string())
        );

        let only_api_url = |name: &str| (name == "API_URL").then(|| "https://api".to_string());
        assert_eq!(
            select_api_base_url(only_api_url).unwrap(),
            ("API_URL", "https://api".to_string())
        );

        assert!(select_api_base_url(|_| None).is_err());
    }

    #[test]
    fn idempotency_key_is_stable_across_replica_retries() {
        assert_eq!(
            build_idempotency_key(
                MaintenanceJob::TelemetryAlerts,
                Some(EXECUTION_NAME),
                at(3, 0, 5)
            ),
            build_idempotency_key(
                MaintenanceJob::TelemetryAlerts,
                Some(EXECUTION_NAME),
                at(3, 7, 41)
            )
        );
        assert_eq!(
            build_idempotency_key(
                MaintenanceJob::TelemetryAlerts,
                Some(EXECUTION_NAME),
                at(3, 0, 5)
            ),
            "telemetry_alerts:flow-like-dev-maintenance-abc123"
        );
    }

    #[test]
    fn idempotency_key_falls_back_to_the_scheduled_minute() {
        assert_eq!(
            build_idempotency_key(MaintenanceJob::CacheCleanup, None, at(3, 0, 37)),
            "cache_cleanup:2026-07-27T03:00:00Z"
        );
        assert_eq!(
            build_idempotency_key(MaintenanceJob::CacheCleanup, None, at(3, 0, 0)),
            build_idempotency_key(MaintenanceJob::CacheCleanup, None, at(3, 0, 59))
        );
        assert_ne!(
            build_idempotency_key(MaintenanceJob::CacheCleanup, None, at(3, 0, 59)),
            build_idempotency_key(MaintenanceJob::CacheCleanup, None, at(3, 1, 0))
        );
    }

    #[test]
    fn jobs_never_share_an_idempotency_key() {
        assert_ne!(
            build_idempotency_key(
                MaintenanceJob::TelemetryAlerts,
                Some(EXECUTION_NAME),
                at(3, 0, 0)
            ),
            build_idempotency_key(
                MaintenanceJob::CacheCleanup,
                Some(EXECUTION_NAME),
                at(3, 0, 0)
            )
        );
    }

    #[test]
    fn blank_execution_names_are_ignored() {
        assert_eq!(
            build_idempotency_key(MaintenanceJob::TelemetryAlerts, Some("   "), at(3, 0, 12)),
            "telemetry_alerts:2026-07-27T03:00:00Z"
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
