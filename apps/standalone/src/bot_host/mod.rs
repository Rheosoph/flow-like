//! The device agent's bot host: answers the turns of the bots bound to this device. The generic
//! claim/lease/reply loop is `flow-like-bot-host`; this module brings the device hub, the runner
//! and the tool environment.
//!
//! The bots' inbox keys for end-to-end encrypted conversations live in the state directory
//! (`e2e`); only their public halves reach the hub.
//!
//! A bot run needs a `FlowLikeState` for its model factories and HTTP client. The agent's main
//! process runs no flows (placements are child processes), so the host builds one bare state
//! once, with no stores registered: files, tables and memory reach the bot's workspace through
//! the credentials the hub presigns for the turn, never through this state.

pub mod backends;
pub mod container;
mod container_broker;
pub mod container_child;
mod container_wire;
pub mod docker_probe;
pub mod e2e;
pub mod env;
pub mod hub;
pub mod runner;

use crate::enrollment::DeviceSession;
use env::Machines;
use flow_like::{
    state::{FlowLikeConfig, FlowLikeState},
    utils::http::HTTPClient,
};
use flow_like_bot_host::{
    account::{AccountConfig, AccountHost},
    host::{TurnEvents, TurnInfo},
};
use hub::{DeviceApi, DeviceTurnHub};
use runner::DeviceRunner;
use std::{path::Path, sync::Arc};
use tokio::sync::Semaphore;
use tokio_util::sync::CancellationToken;

pub const CONCURRENT_TURNS: usize = 2;
const STOPPED_CODE: &str = "BOT_TURN_HOST_STOPPED";

struct LogEvents;

impl TurnEvents for LogEvents {
    fn turn(&self, info: &TurnInfo) {
        tracing::debug!(
            turn_id = %info.id,
            bot_id = %info.bot_id,
            status = ?info.status,
            error_code = ?info.error_code,
            "Bot host: turn changed"
        );
    }
}

pub fn host_id(device_id: &str) -> String {
    format!("device:{device_id}")
}

pub fn config(host_id: String) -> AccountConfig {
    let mut config = AccountConfig::new(host_id);
    config.fire_tasks = false;
    config
}

#[cfg(all(
    feature = "playground",
    any(target_os = "macos", windows, target_os = "linux")
))]
fn machines(state_dir: &Path) -> Option<Arc<dyn Machines>> {
    match crate::playground::machine_host(state_dir.join("bot-machine")) {
        Ok(host) => Some(Arc::new(host)),
        Err(error) => {
            tracing::warn!("Bot host: browser and computer use are unavailable: {error:#}");
            None
        }
    }
}

#[cfg(not(all(
    feature = "playground",
    any(target_os = "macos", windows, target_os = "linux")
)))]
fn machines(_state_dir: &Path) -> Option<Arc<dyn Machines>> {
    None
}

/// Serves the device's bots until `cancel`; then hands the turns it holds back and tells the hub
/// this host is gone.
pub async fn run(
    state_dir: std::path::PathBuf,
    device: Arc<DeviceSession>,
    cancel: CancellationToken,
) -> anyhow::Result<()> {
    let coordinator = container::Coordinator::new(state_dir.clone(), device, cancel.clone());
    let result = run_with_api(state_dir, coordinator.native_api(), cancel).await;
    coordinator.shutdown().await;
    result
}

/// Runs a complete bot host with either the enrolled device API or one bot's scoped parent pipe.
pub async fn run_with_api(
    state_dir: std::path::PathBuf,
    api: Arc<dyn DeviceApi>,
    cancel: CancellationToken,
) -> anyhow::Result<()> {
    let host_id = host_id(api.device_id());
    let base_url = api.base_url().to_owned();
    flow_like_model_provider::embedding::proxy_config::set_api_base_url(&base_url);
    let hub = Arc::new(DeviceTurnHub::new(api, host_id.clone()));
    let state = Arc::new(FlowLikeState::new(
        FlowLikeConfig::new(),
        HTTPClient::new_without_refetch(),
    ));
    let runner = Arc::new(DeviceRunner::new(
        state,
        base_url,
        machines(&state_dir),
        &state_dir,
    ));
    let host = AccountHost::new(
        hub.clone(),
        runner.clone(),
        Arc::new(LogEvents),
        Arc::new(Semaphore::new(CONCURRENT_TURNS)),
        config(host_id),
    );
    let keys = match e2e::host(&state_dir, &hub) {
        Ok(e2e) => {
            host.enable_e2e(e2e.clone());
            runner.enable_e2e(e2e.clone());
            Some(tokio::spawn(e2e::publish_keys(hub.as_ref().clone(), e2e)))
        }
        Err(error) => {
            tracing::warn!("Bot host: end-to-end encrypted chats are unavailable: {error:#}");
            None
        }
    };
    tokio::select! {
        () = host.clone().run() => {}
        () = cancel.cancelled() => {}
    }
    host.withdraw().await;
    if let Some(keys) = keys {
        keys.abort();
    }
    hub.release_all(STOPPED_CODE).await;
    if let Err(failure) = hub.forget().await {
        tracing::debug!(%failure, "Bot host: the hub did not record the host as gone");
    }
    Ok(())
}
