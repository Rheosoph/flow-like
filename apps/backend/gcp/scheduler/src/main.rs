//! GCP scheduler tick — the per-minute Cloud Run job that dispatches cron sinks.
//!
//! On GCP the API's `SINK_SCHEDULER_PROVIDER` is unset, so nothing inside the
//! API fires cron schedules; this job is the only dispatcher. Every minute Cloud
//! Scheduler starts one execution, the execution runs exactly one tick from
//! `flow-like-scheduler-tick` against the Firestore `scheduler` collection, and
//! exits. Overlap between executions (a slow tick, a Cloud Run retry) is safe
//! because each schedule is claimed with a compare-and-swap on the document's
//! `updateTime` before anything is fired.
//!
//! The binary is deliberately thin: environment guard, backend pin, store
//! selection, one call into the shared crate, exit code.

#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

use chrono::Utc;
use flow_like_gcp_data::metadata::ensure_no_forbidden_credential_env;
use flow_like_scheduler_tick::{
    ApiClient, TickConfig, exit_code, require_state_backend, run_tick,
    store::firestore::FirestoreTickStore,
};
use std::process::ExitCode;
use tracing_subscriber::{EnvFilter, layer::SubscriberExt, util::SubscriberInitExt};

/// The only store this image links. `SCHEDULER_STATE_BACKEND` must say so; the
/// shared crate refuses any other value so a job handed the Azure environment
/// fails at boot instead of writing cron state where no other tick reads it.
const STATE_BACKEND: &str = "firestore";

/// Startup refusals exit with this code so the job history separates
/// "misconfigured, nothing ran" from "the tick ran and something failed"
/// (exit 1) without opening the log.
const EXIT_CONFIGURATION: u8 = 2;

/// This job authenticates to the API with a pre-minted, cron-scoped
/// `SINK_TRIGGER_JWT` and must never hold the secret that signs it: with
/// `SINK_SECRET` in hand it could mint a token for any sink type and any app.
/// Its presence means the job was handed the API's environment.
const FORBIDDEN_SIGNING_SECRETS: &[&str] = &["SINK_SECRET"];

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

    let (config, store, api) = match start() {
        Ok(started) => started,
        Err(error) => {
            tracing::error!(%error, "refusing to run the scheduler tick");
            return ExitCode::from(EXIT_CONFIGURATION);
        }
    };

    // Cloud Run puts the execution and attempt in the environment rather than
    // in the log payload; carrying them into the start line is what lets an
    // operator tell a retried attempt's fires apart from the first attempt's.
    let cloud_run_execution = optional_env("CLOUD_RUN_EXECUTION");
    let cloud_run_task_attempt = optional_env("CLOUD_RUN_TASK_ATTEMPT");
    tracing::info!(
        api_base_url = %config.api_base_url,
        state_backend = STATE_BACKEND,
        store = ?store,
        max_catchup_secs = config.max_catchup.as_secs(),
        deadline_secs = config.deadline.as_secs(),
        fanout = config.fanout,
        max_occurrences = config.max_occurrences,
        cloud_run_execution = cloud_run_execution.as_deref().unwrap_or("-"),
        cloud_run_task_attempt = cloud_run_task_attempt.as_deref().unwrap_or("-"),
        "starting GCP scheduler tick"
    );

    match run_tick(&config, &store, &api, Utc::now()).await {
        Ok(report) => {
            if exit_code(&report) == 0 {
                ExitCode::SUCCESS
            } else {
                ExitCode::FAILURE
            }
        }
        // Only the schedule listing can fail here; every per-schedule error is
        // already folded into the report. Nothing was claimed, so the next
        // minute's tick catches up through the capped window.
        Err(error) => {
            tracing::error!(%error, "scheduler tick failed before any schedule was processed");
            ExitCode::FAILURE
        }
    }
}

/// Everything that has to be true before the first network call. Ordered so the
/// cheapest, most consequential checks run first: an environment that could
/// redirect the workload identity or the bearer token is rejected before the
/// token is even read.
fn start() -> Result<(TickConfig, FirestoreTickStore, ApiClient), Box<dyn std::error::Error>> {
    reject_forbidden_environment()?;
    require_state_backend(STATE_BACKEND)?;
    let config = TickConfig::from_env()?;
    let store = FirestoreTickStore::from_env()?;
    let api = ApiClient::new(&config)?;
    Ok((config, store, api))
}

/// Same posture as gcp-api and gcp-queue-worker. The shared credential scan
/// covers key files, ambient tokens, metadata-server redirects and the proxy
/// variables — the last of which matter twice here, because `reqwest` honours
/// them and the bearer JWT would travel through whatever they name. The
/// Firestore client refuses its own emulator and endpoint overrides when it is
/// built; the open-ended override families are matched by shape so a service
/// this image starts using tomorrow is covered without extending a list.
fn reject_forbidden_environment() -> Result<(), Box<dyn std::error::Error>> {
    ensure_no_forbidden_credential_env()?;

    for (name, _) in std::env::vars_os() {
        let name = name.to_string_lossy();
        if is_forbidden_endpoint_override(&name) {
            return Err(format!(
                "{name} is set; the scheduler only talks to Google's published endpoints with \
                 its own workload identity"
            )
            .into());
        }
    }

    for name in FORBIDDEN_SIGNING_SECRETS {
        if std::env::var_os(name).is_some() {
            return Err(format!(
                "{name} is set; the scheduler holds a pre-minted SINK_TRIGGER_JWT and must never \
                 hold the secret that signs it"
            )
            .into());
        }
    }

    Ok(())
}

fn is_forbidden_endpoint_override(name: &str) -> bool {
    name.starts_with("CLOUDSDK_API_ENDPOINT_OVERRIDES_")
        || (name.starts_with("GOOGLE_") && name.ends_with("_CUSTOM_ENDPOINT"))
}

fn optional_env(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_the_open_ended_endpoint_override_families() {
        assert!(is_forbidden_endpoint_override(
            "CLOUDSDK_API_ENDPOINT_OVERRIDES_FIRESTORE"
        ));
        assert!(is_forbidden_endpoint_override(
            "GOOGLE_FIRESTORE_CUSTOM_ENDPOINT"
        ));
        assert!(!is_forbidden_endpoint_override("GOOGLE_PROJECT_ID"));
        assert!(!is_forbidden_endpoint_override(
            "FIRESTORE_SCHEDULER_COLLECTION"
        ));
        assert!(!is_forbidden_endpoint_override("CUSTOM_ENDPOINT"));
    }
}
