use anyhow::{Result, ensure};
#[cfg_attr(not(feature = "runtime"), allow(unused_imports))]
pub(crate) use flow_like_offline_writes::fs::private_directory;
pub use flow_like_offline_writes::{
    BufferedFiles, BufferedTable, BufferingConfig, Outbox, OutboxReader, OutboxStatus,
    QueuedOperation,
};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

const MAX_SCOPES: usize = 64;

fn store_root(state_dir: &Path, placement: &str) -> PathBuf {
    state_dir
        .join("placement-data")
        .join(placement)
        .join("current")
        .join("store")
}

fn scopes_root(state_dir: &Path, placement: &str) -> PathBuf {
    store_root(state_dir, placement)
        .join(".standalone-outbox")
        .join(placement)
}

/// The authorization scopes that hold a queue; none before the first buffered write.
fn scopes(state_dir: &Path, placement: &str) -> Result<Vec<String>> {
    let entries = match std::fs::read_dir(scopes_root(state_dir, placement)) {
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
            result.len() < MAX_SCOPES,
            "Offline scope inventory exceeds its management limit"
        );
        ensure!(
            entry.path().join("queue.sqlite").is_file(),
            "Offline scope has no queue database"
        );
        result.push(entry.file_name().to_string_lossy().into_owned());
    }
    Ok(result)
}

pub(crate) fn for_placement(
    state_dir: &Path,
    config: &crate::config::PlacementConfig,
) -> Result<Vec<Outbox>> {
    let limits = config.offline_writes.clone().unwrap_or_default();
    let parent = store_root(state_dir, &config.id);
    scopes(state_dir, &config.id)?
        .iter()
        .map(|scope| Outbox::open(&parent, &config.id, scope, limits.clone()))
        .collect()
}

/// A scope names a directory, so only the 64 lowercase hex digits of a scope digest pass.
pub(crate) fn is_scope(scope: &str) -> bool {
    scope.len() == 64
        && scope
            .bytes()
            .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
}

/// One scope's queue, opened without creating or changing any file. `None` when the
/// placement has no queue for that scope.
pub(crate) fn reader(
    state_dir: &Path,
    placement: &str,
    scope: &str,
) -> Result<Option<OutboxReader>> {
    ensure!(is_scope(scope), "Invalid offline authorization scope");
    Outbox::open_read_only(&scopes_root(state_dir, placement).join(scope))
}

/// Totals over every scope of a placement, read without loading a queued write.
/// `needs_attention` counts the scopes whose queue waits for a person. Resource names,
/// error text and payloads stay on the device.
pub(crate) fn summary(state_dir: &Path, placement: &str) -> Result<Value> {
    let root = scopes_root(state_dir, placement);
    let (mut scopes_read, mut pending_count, mut pending_bytes) = (0u64, 0u64, 0u64);
    let (mut quarantined, mut needs_attention, mut mirror_error) = (0u64, 0u64, false);
    let mut oldest_at = None::<i64>;
    for scope in scopes(state_dir, placement)? {
        let Some(queue) = Outbox::open_read_only(&root.join(scope))? else {
            continue;
        };
        let totals = queue.totals()?;
        scopes_read += 1;
        pending_count = pending_count.saturating_add(totals.pending_count);
        pending_bytes = pending_bytes.saturating_add(totals.pending_bytes);
        oldest_at = match (oldest_at, totals.oldest_at) {
            (Some(known), Some(queued)) => Some(known.min(queued)),
            (known, queued) => known.or(queued),
        };
        quarantined += u64::from(totals.quarantined);
        needs_attention += u64::from(totals.needs_operator);
        mirror_error |= totals.mirror_error;
    }
    Ok(json!({
        "scopes": scopes_read,
        "pending_count": pending_count,
        "pending_bytes": pending_bytes,
        "oldest_at": oldest_at,
        "quarantined_scopes": quarantined,
        "needs_attention": needs_attention,
        "mirror_error": mirror_error,
    }))
}

#[cfg(test)]
pub(crate) mod test_support {
    use super::*;

    /// A writable queue where a placement's runtime would create it.
    pub(crate) fn queue(state_dir: &Path, placement: &str, scope: &str) -> Result<Outbox> {
        let mut parent = state_dir.to_path_buf();
        for part in ["placement-data", placement, "current", "store"] {
            parent.push(part);
            private_directory(&parent)?;
        }
        Outbox::open(&parent, placement, scope, Default::default())
    }
}

#[cfg(test)]
mod tests {
    use super::{test_support::queue, *};

    /// A scope with two queued writes; returns the bytes it holds.
    fn healthy_scope(root: &Path) -> Result<u64> {
        let healthy = queue(root, "api", &"a".repeat(64))?;
        for at in [300, 200] {
            let rows = json!({"rows":[{"secret":"private-row"}]});
            healthy.enqueue("private-table-name", rows, None, at)?;
        }
        Ok(healthy.status()?.pending_bytes)
    }

    /// A revoked scope whose only write waits for a person; returns the bytes it holds.
    fn revoked_scope(root: &Path) -> Result<u64> {
        let revoked = queue(root, "api", &"b".repeat(64))?;
        let body = json!({"body":"private-file"});
        let head = revoked.enqueue("private-file-path", body, None, 100)?;
        revoked.block(&head.operation_id, "conflict", "private-error-text")?;
        revoked.quarantine("Cloud access was revoked")?;
        revoked.mirror_error("private-table-name", Some("private-mirror-text"))?;
        Ok(revoked.status()?.pending_bytes)
    }

    /// Every file below the placement's queues, with its content.
    fn queue_files(root: &Path) -> Vec<(PathBuf, Vec<u8>)> {
        let mut files = Vec::new();
        let mut pending = vec![scopes_root(root, "api")];
        while let Some(directory) = pending.pop() {
            for entry in std::fs::read_dir(directory).unwrap() {
                let path = entry.unwrap().path();
                if path.is_dir() {
                    pending.push(path);
                } else {
                    files.push((path.clone(), std::fs::read(path).unwrap()));
                }
            }
        }
        files.sort();
        files
    }

    #[test]
    fn summary_adds_up_scopes_without_naming_resources_or_changing_files() -> Result<()> {
        let temp = tempfile::tempdir()?;
        let root = temp.path();
        assert_eq!(
            summary(root, "api")?,
            json!({"scopes":0,"pending_count":0,"pending_bytes":0,"oldest_at":null,"quarantined_scopes":0,"needs_attention":0,"mirror_error":false})
        );
        let pending_bytes = healthy_scope(root)? + revoked_scope(root)?;
        let before = queue_files(root);
        let totals = summary(root, "api")?;
        assert_eq!(
            totals,
            json!({"scopes":2,"pending_count":3,"pending_bytes":pending_bytes,"oldest_at":100,"quarantined_scopes":1,"needs_attention":1,"mirror_error":true})
        );
        assert!(!totals.to_string().contains("private"));
        assert_eq!(queue_files(root), before);
        Ok(())
    }

    #[test]
    fn readers_open_only_existing_scopes_named_by_a_digest() -> Result<()> {
        let temp = tempfile::tempdir()?;
        let root = temp.path();
        let scope = "c".repeat(64);
        queue(root, "api", &scope)?.enqueue("table", json!(1), None, 100)?;
        let existing = reader(root, "api", &scope)?.unwrap();
        assert_eq!(existing.status()?.pending_count, 1);
        assert!(reader(root, "api", &"d".repeat(64))?.is_none());
        assert!(reader(root, "other", &scope)?.is_none());
        let invalid = [
            String::new(),
            "..".into(),
            "../../../management".into(),
            "C".repeat(64),
            "g".repeat(64),
            "c".repeat(63),
            format!("{}/", "c".repeat(63)),
        ];
        assert!(
            invalid
                .iter()
                .all(|scope| reader(root, "api", scope).is_err())
        );
        assert!(!scopes_root(root, "api").join("d".repeat(64)).exists());
        Ok(())
    }
}
