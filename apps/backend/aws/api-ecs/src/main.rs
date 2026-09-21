#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

#[path = "../../shared/api_bootstrap.rs"]
mod bootstrap;
mod config;
#[path = "../../../shared/api_hardening.rs"]
mod hardening;
mod health;
mod server;
mod telemetry;

use flow_like_api::audit::worker::{
    self as audit_worker, AuditWorkerContext, bucket as audit_bucket,
};
use flow_like_api::{construct_router, construct_router_with_cors};
use flow_like_aws_data::dsql::{DsqlConfig, MAX_CONNECTIONS_ENV};
use std::process::ExitCode;
use std::sync::Arc;
use std::time::Duration;
use tracing::Instrument;

const APPLICATION_NAME: &str = "flow-like-aws-api-ecs";
/// One task serves many requests at once, unlike a Lambda instance.
const DEFAULT_DSQL_CONNECTIONS: u32 = 32;
/// Work still running after the drain belongs to requests that were cut off.
const RUNTIME_SHUTDOWN: Duration = Duration::from_secs(1);

fn main() -> ExitCode {
    if std::env::args().nth(1).as_deref() == Some("healthcheck") {
        return match config::port() {
            Ok(port) => health::probe(port),
            Err(error) => {
                eprintln!("{error}");
                ExitCode::FAILURE
            }
        };
    }
    let config = match config::Config::from_env() {
        Ok(config) => config,
        Err(error) => {
            eprintln!("invalid configuration: {error}");
            return ExitCode::FAILURE;
        }
    };
    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .thread_name("api-worker")
        .build()
    {
        Ok(runtime) => runtime,
        Err(error) => {
            eprintln!("failed to start the Tokio runtime: {error}");
            return ExitCode::FAILURE;
        }
    };
    let result = runtime.block_on(run(config));
    runtime.shutdown_timeout(RUNTIME_SHUTDOWN);
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
}

async fn run(config: config::Config) -> Result<(), bootstrap::BootstrapError> {
    let telemetry = telemetry::init()?;
    let result = serve(&config).await;
    if let Err(error) = &result {
        tracing::error!(%error, "API stopped");
    }
    telemetry.shutdown().await;
    result
}

async fn serve(config: &config::Config) -> Result<(), bootstrap::BootstrapError> {
    // Unset keeps the Lambda API's permissive policy; bearer tokens, not
    // cookies, authenticate the API.
    let cors = std::env::var_os("CORS_ALLOWED_ORIGINS")
        .map(|_| hardening::cors_from_env())
        .transpose()
        .map_err(|error| format!("invalid CORS_ALLOWED_ORIGINS: {error}"))?;
    let mut dsql = DsqlConfig::from_env()?;
    if let Some(dsql) = dsql.as_mut()
        && std::env::var_os(MAX_CONNECTIONS_ENV).is_none()
    {
        dsql.max_connections = DEFAULT_DSQL_CONNECTIONS;
    }

    let api = bootstrap::initialize(APPLICATION_NAME, dsql)
        .instrument(
            tracing::info_span!(target: "flow_like::observability", parent: None, "api.initialize"),
        )
        .await?;
    // Timers keep ticking in a task, so the token rotates ahead of expiry
    // instead of being checked on every request.
    let _token_refresh = api
        .dsql
        .as_ref()
        .map(|database| database.spawn_background_refresh());
    // Set AUDIT_WORKER=off when the audit worker Lambda holds the audit key and bucket.
    let _audit_worker = if audit_bucket::in_process_worker_enabled() {
        let context = AuditWorkerContext::from_state(&api.state)?;
        Some(audit_worker::spawn(
            Arc::new(context),
            audit_worker::TICK_INTERVAL,
        ))
    } else {
        None
    };
    let _payments_worker = flow_like_api::payments::worker::spawn(api.state.clone());
    let router = match cors {
        Some(cors) => construct_router_with_cors(api.state, cors),
        None => construct_router(api.state),
    };
    server::serve(config, router).await?;
    Ok(())
}
