//! Cloud Run Jobs port of the AWS maintenance Lambda.
//!
//! The job holds no database credential and no GCP credential at all. It runs
//! once, POSTs the allowlisted maintenance jobs to the API with a bearer token,
//! and exits non-zero on any failure so Cloud Run's task retry and the job's
//! execution history carry the outcome. Every validation the Lambda performs on
//! its inputs is kept; only the transport (Lambda event → process environment)
//! and the lifecycle (handler → run once) change.

#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

use std::{env, process::ExitCode, time::Duration};

use anyhow::{Result, anyhow, bail};
use chrono::{DateTime, SecondsFormat, Timelike, Utc};
use flow_like_gcp_data::metadata::ensure_no_forbidden_credential_env;
use flow_like_types_contracts::maintenance::{
    MaintenanceJob, MaintenanceRunRequest, MaintenanceRunResponse,
};
use reqwest::StatusCode;
use tracing_subscriber::{EnvFilter, layer::SubscriberExt, util::SubscriberInitExt};

const HTTP_CONNECT_TIMEOUT_SECS: u64 = 5;
/// The API's Cloud Run service terminates a request at its own
/// `timeout_seconds` (300 in the root module), so waiting longer buys nothing,
/// while giving up earlier would turn a slow-but-succeeding daily sweep into a
/// spurious failure plus a retry that reruns the same sweep. Raise it together
/// with the API's request timeout, never independently.
const HTTP_TIMEOUT_SECS: u64 = 300;
const MIN_TOKEN_BYTES: usize = 32;
const IDEMPOTENCY_KEY_HEADER: &str = "Idempotency-Key";
/// Set by Cloud Run Jobs on every task of an execution and stable across task
/// retries within it, which is exactly the boundary the idempotency key wants:
/// a retried attempt repeats the key, a fresh execution (Cloud Scheduler retry,
/// `gcloud run jobs execute`) gets a new one.
const CLOUD_RUN_EXECUTION_ENV: &str = "CLOUD_RUN_EXECUTION";

/// Where the suffix of the `Idempotency-Key` comes from.
enum RunScope {
    /// The Cloud Run execution name.
    Execution(String),
    /// `now` floored to the minute, RFC 3339 UTC. Cloud Scheduler invokes the
    /// job at its scheduled minute, so an in-minute retry repeats the key and
    /// the next day's run does not. Only used when the platform gave no
    /// execution name, i.e. outside Cloud Run Jobs.
    ScheduledMinute(String),
}

impl RunScope {
    fn suffix(&self) -> &str {
        match self {
            Self::Execution(name) | Self::ScheduledMinute(name) => name,
        }
    }
}

struct Config {
    api_base_url: String,
    maintenance_token: String,
    jobs: Vec<MaintenanceJob>,
    scope: RunScope,
}

impl Config {
    fn from_env() -> Result<Self> {
        // First, before anything reads a URL or builds a client: `HTTPS_PROXY`
        // and friends would route the bearer token through whoever set them,
        // and the guard's other entries would make this image a credential
        // holder it was never meant to be.
        ensure_no_forbidden_credential_env()
            .map_err(|error| anyhow!("forbidden environment: {error}"))?;

        let api_base_url = resolve_api_base_url(
            optional_env("API_BASE_URL"),
            optional_env("API_URL"),
            allow_insecure_api_base_url(),
        )?;
        let maintenance_token = validate_token(
            &env::var("MAINTENANCE_TOKEN").map_err(|_| anyhow!("MAINTENANCE_TOKEN not set"))?,
        )?;
        let jobs = select_jobs(optional_env("MAINTENANCE_JOB").as_deref())?;
        let scope = run_scope(optional_env(CLOUD_RUN_EXECUTION_ENV).as_deref(), Utc::now());

        Ok(Self {
            api_base_url,
            maintenance_token,
            jobs,
            scope,
        })
    }
}

fn optional_env(name: &str) -> Option<String> {
    env::var(name)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn allow_insecure_api_base_url() -> bool {
    env::var("ALLOW_INSECURE_API_BASE_URL")
        .ok()
        .is_some_and(|value| matches!(value.trim().to_ascii_lowercase().as_str(), "1" | "true"))
}

/// Azure passes `API_URL`, AWS passes `API_BASE_URL`, and the GCP root passes
/// both with the same value. `API_BASE_URL` wins so a deployment that sets both
/// differently behaves the same on every cloud.
fn resolve_api_base_url(
    api_base_url: Option<String>,
    api_url: Option<String>,
    allow_insecure: bool,
) -> Result<String> {
    let value = api_base_url
        .or(api_url)
        .ok_or_else(|| anyhow!("API_BASE_URL not set (API_URL is accepted as an alias)"))?;
    normalize_api_base_url(&value, allow_insecure).map_err(|error| anyhow!(error))
}

fn normalize_api_base_url(
    value: &str,
    allow_insecure: bool,
) -> std::result::Result<String, String> {
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

fn validate_token(value: &str) -> Result<String> {
    let value = value.trim();
    if value.len() < MIN_TOKEN_BYTES {
        bail!("MAINTENANCE_TOKEN must contain at least {MIN_TOKEN_BYTES} bytes");
    }
    Ok(value.to_string())
}

/// `all` is the deployed default: the GCP root schedules one daily execution
/// and sets no `MAINTENANCE_JOB`, so every maintenance job runs from that
/// single execution.
fn select_jobs(value: Option<&str>) -> Result<Vec<MaintenanceJob>> {
    match value.map(str::to_ascii_lowercase).as_deref() {
        None | Some("all") => Ok(vec![
            MaintenanceJob::TelemetryAlerts,
            MaintenanceJob::CacheCleanup,
            MaintenanceJob::RunSweep,
            MaintenanceJob::StateCleanup,
        ]),
        Some("telemetry_alerts") => Ok(vec![MaintenanceJob::TelemetryAlerts]),
        Some("cache_cleanup") => Ok(vec![MaintenanceJob::CacheCleanup]),
        Some("run_sweep") => Ok(vec![MaintenanceJob::RunSweep]),
        Some("state_cleanup") => Ok(vec![MaintenanceJob::StateCleanup]),
        Some(other) => bail!(
            "MAINTENANCE_JOB must be 'telemetry_alerts', 'cache_cleanup', 'run_sweep', 'state_cleanup' or 'all', not {other:?}"
        ),
    }
}

fn run_scope(execution: Option<&str>, now: DateTime<Utc>) -> RunScope {
    match execution {
        Some(name) if is_header_safe_name(name) => RunScope::Execution(name.to_string()),
        Some(name) => {
            // Cloud Run only ever generates `<job>-<suffix>`; anything else was
            // set by hand. Falling back rather than failing mirrors how the
            // Lambda treats an unexpanded scheduler placeholder — the minute
            // key still dedups an in-minute retry, so nothing is lost.
            tracing::warn!(
                value = %name,
                "CLOUD_RUN_EXECUTION is not a Cloud Run execution name; keying on the scheduled minute instead"
            );
            RunScope::ScheduledMinute(scheduled_minute(now))
        }
        None => RunScope::ScheduledMinute(scheduled_minute(now)),
    }
}

/// The name ends up inside a header, so it must be visible ASCII; the accepted
/// set is a superset of what Cloud Run generates and a subset of what a header
/// value allows.
fn is_header_safe_name(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
}

fn scheduled_minute(now: DateTime<Utc>) -> String {
    now.with_second(0)
        .and_then(|now| now.with_nanosecond(0))
        .unwrap_or(now)
        .to_rfc3339_opts(SecondsFormat::Secs, true)
}

fn build_idempotency_key(job: MaintenanceJob, scope: &RunScope) -> String {
    format!("{}:{}", job.as_str(), scope.suffix())
}

fn maintenance_url(api_base_url: &str) -> String {
    format!(
        "{}/api/v1/maintenance/run",
        api_base_url.trim_end_matches('/')
    )
}

fn is_transient_status(status: StatusCode) -> bool {
    status.is_server_error()
        || matches!(
            status,
            StatusCode::REQUEST_TIMEOUT | StatusCode::TOO_MANY_REQUESTS
        )
}

fn http_client() -> Result<reqwest::Client> {
    reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(HTTP_CONNECT_TIMEOUT_SECS))
        .timeout(Duration::from_secs(HTTP_TIMEOUT_SECS))
        // The environment guard already rejects every proxy variable; this
        // makes the client's behaviour independent of that guard rather than
        // downstream of it. The token goes to API_BASE_URL and nowhere else.
        .no_proxy()
        .build()
        .map_err(|error| anyhow!("failed to build HTTP client: {error}"))
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

    match run().await {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            tracing::error!(error = %error, "maintenance run failed");
            ExitCode::FAILURE
        }
    }
}

async fn run() -> Result<()> {
    let config = Config::from_env()?;
    let client = http_client()?;

    let job_names: Vec<&str> = config.jobs.iter().map(|job| job.as_str()).collect();
    tracing::info!(
        jobs = ?job_names,
        api_base_url = %config.api_base_url,
        idempotency_suffix = %config.scope.suffix(),
        cloud_run_job = ?optional_env("CLOUD_RUN_JOB"),
        cloud_run_execution = ?optional_env(CLOUD_RUN_EXECUTION_ENV),
        cloud_run_task_index = ?optional_env("CLOUD_RUN_TASK_INDEX"),
        cloud_run_task_attempt = ?optional_env("CLOUD_RUN_TASK_ATTEMPT"),
        "starting Flow-Like GCP maintenance job"
    );

    // Every selected job runs even after an earlier one failed: a broken
    // telemetry evaluation must not starve the cache sweep, and the retry that
    // follows a non-zero exit re-issues the same keys, so the API sees the
    // succeeded jobs again only as dedup-able repeats.
    let mut failed = 0usize;
    for job in &config.jobs {
        if let Err(error) = run_job(&client, &config, *job).await {
            tracing::error!(job = job.as_str(), error = %error, "maintenance job failed");
            failed += 1;
        }
    }

    if failed > 0 {
        bail!(
            "{failed} of {} maintenance job(s) failed",
            config.jobs.len()
        );
    }
    tracing::info!(jobs = config.jobs.len(), "maintenance run completed");
    Ok(())
}

async fn run_job(client: &reqwest::Client, config: &Config, job: MaintenanceJob) -> Result<()> {
    let idempotency_key = build_idempotency_key(job, &config.scope);
    tracing::info!(
        job = job.as_str(),
        idempotency_key = %idempotency_key,
        "running maintenance job"
    );

    let response = client
        .post(maintenance_url(&config.api_base_url))
        .bearer_auth(&config.maintenance_token)
        .header(IDEMPOTENCY_KEY_HEADER, &idempotency_key)
        .json(&MaintenanceRunRequest::from(job))
        .send()
        .await
        .map_err(|error| anyhow!("maintenance API request failed: {error}"))?;

    let status = response.status();
    let body = response
        .text()
        .await
        .map_err(|error| anyhow!("failed to read maintenance API response: {error}"))?;

    if !status.is_success() {
        if is_transient_status(status) {
            tracing::error!(
                %status,
                body = %body,
                job = job.as_str(),
                "maintenance API returned a transient error"
            );
        } else {
            tracing::error!(
                %status,
                body = %body,
                job = job.as_str(),
                "maintenance API rejected the request; failing the task so the Cloud Run job's execution history and the alert policies surface the configuration error"
            );
        }
        bail!("maintenance API error: {status} - {body}");
    }

    let parsed: MaintenanceRunResponse = serde_json::from_str(&body).map_err(|error| {
        tracing::error!(
            %error,
            body = %body,
            "maintenance API returned an invalid success response"
        );
        anyhow!("invalid maintenance API response: {error}")
    })?;

    match (job, parsed) {
        (MaintenanceJob::TelemetryAlerts, MaintenanceRunResponse::TelemetryAlerts(result)) => {
            tracing::info!(
                evaluated = result.evaluated,
                triggered = result.triggered,
                resolved = result.resolved,
                "telemetry alert maintenance completed"
            )
        }
        (MaintenanceJob::CacheCleanup, MaintenanceRunResponse::CacheCleanup(result)) => {
            tracing::info!(
                deleted = result.deleted,
                "cache cleanup maintenance completed"
            )
        }
        (MaintenanceJob::RunSweep, MaintenanceRunResponse::RunSweep(result)) => {
            tracing::info!(
                swept = result.swept,
                grace_secs = result.grace_secs,
                batch_size = result.batch_size,
                batch_full = result.swept >= result.batch_size,
                "run sweep maintenance completed"
            )
        }
        (MaintenanceJob::StateCleanup, MaintenanceRunResponse::StateCleanup(result)) => {
            tracing::info!(
                deleted_runs = result.deleted_runs,
                deleted_events = result.deleted_events,
                "state cleanup maintenance completed"
            )
        }
        (job, response) => {
            // The API answered for a different job than we asked for. Fail the
            // job so the mismatch surfaces instead of being logged as success.
            tracing::error!(
                requested = job.as_str(),
                response = ?response,
                "maintenance API responded for a different job"
            );
            bail!(
                "maintenance API responded for a different job than {}",
                job.as_str()
            );
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn at(hour: u32, minute: u32, second: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 8, 15, hour, minute, second)
            .unwrap()
    }

    #[test]
    fn job_selection_defaults_to_all_in_declaration_order() {
        assert_eq!(
            select_jobs(None).unwrap(),
            vec![
                MaintenanceJob::TelemetryAlerts,
                MaintenanceJob::CacheCleanup,
                MaintenanceJob::RunSweep,
                MaintenanceJob::StateCleanup
            ]
        );
        assert_eq!(
            select_jobs(Some("all")).unwrap(),
            vec![
                MaintenanceJob::TelemetryAlerts,
                MaintenanceJob::CacheCleanup,
                MaintenanceJob::RunSweep,
                MaintenanceJob::StateCleanup
            ]
        );
        assert_eq!(
            select_jobs(Some("telemetry_alerts")).unwrap(),
            vec![MaintenanceJob::TelemetryAlerts]
        );
        assert_eq!(
            select_jobs(Some("CACHE_CLEANUP")).unwrap(),
            vec![MaintenanceJob::CacheCleanup]
        );
        assert_eq!(
            select_jobs(Some("RUN_SWEEP")).unwrap(),
            vec![MaintenanceJob::RunSweep]
        );
        assert_eq!(
            select_jobs(Some("state_cleanup")).unwrap(),
            vec![MaintenanceJob::StateCleanup]
        );
    }

    #[test]
    fn unknown_jobs_are_rejected() {
        assert!(select_jobs(Some("arbitrary_sql")).is_err());
        assert!(select_jobs(Some("telemetry_alerts,cache_cleanup")).is_err());
    }

    #[test]
    fn idempotency_key_prefers_the_cloud_run_execution() {
        let scope = run_scope(Some("flow-like-maintenance-abcde"), at(3, 0, 12));
        let first = build_idempotency_key(MaintenanceJob::TelemetryAlerts, &scope);
        let retry = build_idempotency_key(
            MaintenanceJob::TelemetryAlerts,
            &run_scope(Some("flow-like-maintenance-abcde"), at(3, 41, 59)),
        );

        assert_eq!(first, "telemetry_alerts:flow-like-maintenance-abcde");
        assert_eq!(first, retry);
        assert_ne!(
            first,
            build_idempotency_key(MaintenanceJob::CacheCleanup, &scope)
        );
    }

    #[test]
    fn idempotency_key_falls_back_to_the_scheduled_minute() {
        let scope = run_scope(None, at(3, 0, 47));
        assert_eq!(
            build_idempotency_key(MaintenanceJob::CacheCleanup, &scope),
            "cache_cleanup:2026-08-15T03:00:00Z"
        );
        assert_eq!(
            scheduled_minute(at(3, 0, 0)),
            scheduled_minute(at(3, 0, 59))
        );
        assert_ne!(
            scheduled_minute(at(3, 0, 59)),
            scheduled_minute(at(3, 1, 0))
        );
    }

    #[test]
    fn execution_names_that_cannot_be_a_header_are_ignored() {
        assert!(matches!(
            run_scope(Some("has space"), at(3, 0, 0)),
            RunScope::ScheduledMinute(_)
        ));
        assert!(matches!(
            run_scope(Some("newline\ninjected"), at(3, 0, 0)),
            RunScope::ScheduledMinute(_)
        ));
        assert!(matches!(
            run_scope(Some("<cloud.run.execution>"), at(3, 0, 0)),
            RunScope::ScheduledMinute(_)
        ));
        assert!(matches!(
            run_scope(Some("flow-like-maintenance-abcde"), at(3, 0, 0)),
            RunScope::Execution(_)
        ));
    }

    #[test]
    fn api_base_url_wins_over_the_api_url_alias() {
        assert_eq!(
            resolve_api_base_url(
                Some("https://api.example.com/".to_string()),
                Some("https://other.example.com".to_string()),
                false
            )
            .unwrap(),
            "https://api.example.com"
        );
        assert_eq!(
            resolve_api_base_url(None, Some("https://other.example.com".to_string()), false)
                .unwrap(),
            "https://other.example.com"
        );
        assert!(resolve_api_base_url(None, None, false).is_err());
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
        assert!(normalize_api_base_url("https://user:pw@api.example.com", false).is_err());
        assert!(normalize_api_base_url("https://api.example.com/?x=1", false).is_err());
    }

    #[test]
    fn short_tokens_are_rejected() {
        assert!(validate_token("too-short").is_err());
        assert_eq!(
            validate_token("  0123456789abcdef0123456789abcdef  ").unwrap(),
            "0123456789abcdef0123456789abcdef"
        );
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
