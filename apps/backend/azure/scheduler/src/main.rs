#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

mod config;

use flow_like_scheduler_tick::{ApiClient, exit_code, run_tick, store::cosmos::CosmosTickStore};
use std::process::ExitCode;
use tracing_subscriber::{EnvFilter, layer::SubscriberExt, util::SubscriberInitExt};

/// One Container Apps job execution is one tick: guard the environment, build
/// the Cosmos store, run the tick at `now`, exit with the report's verdict. The
/// job's own retry policy handles a non-zero exit; nothing loops in-process.
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
        Ok(code) => code,
        Err(error) => {
            tracing::error!(error = %error, "Azure scheduler tick did not run");
            ExitCode::FAILURE
        }
    }
}

async fn run() -> Result<ExitCode, Box<dyn std::error::Error>> {
    let config = config::Config::from_env()?;
    // No network here: the first Cosmos request happens inside the tick, so an
    // identity or RBAC problem surfaces per schedule in the report, not as a
    // startup failure.
    let store = CosmosTickStore::from_env()?;
    let api = ApiClient::new(&config.tick)?;
    let now = chrono::Utc::now();

    tracing::info!(
        api_base_url = %config.tick.api_base_url,
        store = ?store,
        client_id = %config.client_id,
        max_catchup_secs = config.tick.max_catchup.as_secs(),
        deadline_secs = config.tick.deadline.as_secs(),
        fanout = config.tick.fanout,
        %now,
        "starting Azure scheduler tick with managed identity"
    );

    let report = run_tick(&config.tick, &store, &api, now).await?;
    // 0 or 1 by contract; the cast cannot truncate.
    Ok(ExitCode::from(exit_code(&report) as u8))
}
