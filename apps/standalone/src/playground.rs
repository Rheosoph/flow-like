//! A bot's browser and computer on this device: the hidden `playground-worker` subcommand the
//! agent starts once per bot, and the settings the agent starts it with.

use std::path::PathBuf;

use anyhow::Result;

/// Whether this build carries the bots' browser and computer.
pub const AVAILABLE: bool = cfg!(all(
    feature = "playground",
    any(target_os = "macos", windows, target_os = "linux")
));

#[cfg(all(
    feature = "playground",
    any(target_os = "macos", windows, target_os = "linux")
))]
pub use flow_like_bot_machine::{MachineHost, Mode};

#[cfg(all(
    feature = "playground",
    any(target_os = "macos", windows, target_os = "linux")
))]
static SHARED: std::sync::OnceLock<MachineHost> = std::sync::OnceLock::new();

/// The machine host of this agent: one worker process per bot, started from this executable. The
/// bot host and the owner's live view must reach the same bots, so there is one per process.
#[cfg(all(
    feature = "playground",
    any(target_os = "macos", windows, target_os = "linux")
))]
pub fn machine_host(data_dir: PathBuf) -> Result<MachineHost> {
    if let Some(host) = SHARED.get() {
        return Ok(host.clone());
    }
    let program = std::env::current_exe().map_err(|e| {
        anyhow::anyhow!("Cannot find this executable to start playground workers: {e}")
    })?;
    Ok(SHARED
        .get_or_init(|| MachineHost::new(data_dir, Mode::Worker { program }))
        .clone())
}

/// The host `machine_host` created, if the bot host started one.
#[cfg(all(
    feature = "playground",
    any(target_os = "macos", windows, target_os = "linux")
))]
pub fn shared_machine_host() -> Option<MachineHost> {
    SHARED.get().cloned()
}

#[cfg(all(
    feature = "playground",
    any(target_os = "macos", windows, target_os = "linux")
))]
pub async fn run_worker(
    bot: String,
    data_dir: PathBuf,
    display: Option<String>,
    headful: bool,
) -> Result<()> {
    flow_like_bot_machine::run_worker(flow_like_bot_machine::WorkerArgs {
        bot,
        data_dir,
        display,
        headful,
    })
    .await
    .map_err(|e| anyhow::anyhow!("{e:#}"))
}

#[cfg(not(all(
    feature = "playground",
    any(target_os = "macos", windows, target_os = "linux")
)))]
pub async fn run_worker(
    _bot: String,
    _data_dir: PathBuf,
    _display: Option<String>,
    _headful: bool,
) -> Result<()> {
    anyhow::bail!(
        "This build of flow-like-standalone has no playground (browser and computer for bots)"
    )
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_flag_follows_the_feature_and_the_platform() {
        assert_eq!(
            super::AVAILABLE,
            cfg!(feature = "playground")
                && cfg!(any(target_os = "macos", windows, target_os = "linux"))
        );
    }
}
