use anyhow::{Result, ensure};
#[cfg_attr(not(feature = "runtime"), allow(unused_imports))]
pub(crate) use flow_like_offline_writes::fs::private_directory;
pub use flow_like_offline_writes::{
    BufferedFiles, BufferedTable, BufferingConfig, Outbox, OutboxStatus, QueuedOperation,
};
use std::path::Path;

pub(crate) fn for_placement(
    state_dir: &Path,
    config: &crate::config::PlacementConfig,
) -> Result<Vec<Outbox>> {
    let limits = config.offline_writes.clone().unwrap_or_default();
    let parent = state_dir
        .join("placement-data")
        .join(&config.id)
        .join("current")
        .join("store");
    let root = parent.join(".standalone-outbox").join(&config.id);
    let entries = match std::fs::read_dir(&root) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error.into()),
    };
    let mut result = Vec::new();
    for entry in entries {
        let entry = entry?;
        ensure!(
            entry.file_type()?.is_dir(),
            "Unexpected file in offline authorization scopes"
        );
        ensure!(
            result.len() < 64,
            "Offline scope inventory exceeds its management limit"
        );
        let scope = entry.file_name().to_string_lossy().into_owned();
        ensure!(
            entry.path().join("queue.sqlite").is_file(),
            "Offline scope has no queue database"
        );
        result.push(Outbox::open(&parent, &config.id, &scope, limits.clone())?);
    }
    Ok(result)
}
