use crate::{enrollment::unix_time, state::StateStore};
use anyhow::{Context, Result, bail, ensure};
use rusqlite::{OptionalExtension, params};
use std::{future::Future, path::Path, time::Duration};
use tokio_util::sync::CancellationToken;

const REBOOT_DISPATCH_WINDOW: i64 = 120;
/// After this long on the same boot, a requested or unknown reboot did not happen.
const REBOOT_OUTCOME_WINDOW: i64 = 600;
const UPDATE_STAGING_WINDOW: i64 = 600;
const MAX_WATCH_BACKOFF: Duration = Duration::from_secs(30);

pub fn boot_id() -> Result<String> {
    #[cfg(target_os = "linux")]
    {
        let id = std::fs::read_to_string("/proc/sys/kernel/random/boot_id")?;
        return Ok(uuid::Uuid::parse_str(id.trim())?.to_string());
    }
    #[cfg(not(target_os = "linux"))]
    {
        let timestamp = sysinfo::System::boot_time();
        ensure!(
            timestamp > 0,
            "Operating system boot identity is unavailable"
        );
        Ok(format!("{}-{timestamp}", std::env::consts::OS))
    }
}

fn open(state_dir: &Path) -> Result<StateStore> {
    StateStore::open(&state_dir.join("management.sqlite"))
}

/// A reboot or agent update that has not reached an outcome yet.
pub(crate) struct ActiveOperation {
    pub operation_id: String,
    pub kind: &'static str,
    pub state: String,
    pub created_at: i64,
    /// The journal principal that issued it.
    pub principal: Option<String>,
}

/// At most one host operation is in progress: a new one is refused while another is.
pub(crate) fn active_operation(store: &StateStore) -> Result<Option<ActiveOperation>> {
    Ok(store
        .connection
        .query_row(
            "SELECT h.operation_id,h.kind,h.state,h.created_at,m.principal FROM host_operations h LEFT JOIN management_operations m ON m.operation_id=h.operation_id WHERE h.state IN ('pending','staging','draining','requesting','requested','unknown') ORDER BY h.created_at DESC,h.operation_id LIMIT 1",
            [],
            |row| {
                Ok(ActiveOperation {
                    operation_id: row.get(0)?,
                    kind: if row.get::<_, String>(1)? == "reboot" {
                        "reboot"
                    } else {
                        "update_agent"
                    },
                    state: row.get(2)?,
                    created_at: row.get(3)?,
                    principal: row.get(4)?,
                })
            },
        )
        .optional()?)
}

/// Move a host operation to `state`, optionally only from `expected`, and describe it to the caller.
fn transition(
    store: &StateStore,
    kind: &str,
    operation: &str,
    expected: Option<&str>,
    state: &str,
    detail: Option<&str>,
) -> Result<bool> {
    let changed = store.connection.execute(
        "UPDATE host_operations SET state=?2 WHERE operation_id=?1 AND kind=?3 AND (?4 IS NULL OR state=?4)",
        params![operation, state, kind, expected],
    )? == 1;
    if let (true, Some(detail)) = (changed, detail) {
        store.connection.execute(
            "UPDATE management_operations SET result_json=json_set(result_json,'$.state',?2,'$.result.'||?3,?4) WHERE operation_id=?1",
            params![operation, state, kind, detail],
        )?;
    }
    Ok(changed)
}

fn update_result(store: &StateStore, operation: &str, state: &str, detail: &str) -> Result<()> {
    transition(store, "update", operation, None, state, Some(detail))?;
    Ok(())
}

/// Observe reboot completion only after a different OS boot, never after an agent restart.
pub fn reconcile_boot(state_dir: &Path, boot_id: &str) -> Result<()> {
    let store = open(state_dir)?;
    store.connection.execute("UPDATE host_operations SET state='completed' WHERE kind='reboot' AND state IN ('pending','draining','requesting','requested','unknown') AND boot_id<>?1",[boot_id])?;
    store.connection.execute("UPDATE host_operations SET state='pending' WHERE kind='reboot' AND state='draining' AND boot_id=?1", [boot_id])?;
    store.connection.execute("UPDATE host_operations SET state='unknown' WHERE kind='reboot' AND state='requesting' AND boot_id=?1", [boot_id])?;
    store.connection.execute("UPDATE management_operations SET result_json=json_set(result_json,'$.state','completed','$.result.reboot','completed') WHERE operation_id IN (SELECT operation_id FROM host_operations WHERE kind='reboot' AND state='completed')",[])?;
    expire_reboot_outcomes(&store, boot_id, unix_time()?)?;
    reconcile_updates(state_dir, &store, boot_id)
}

/// A same-boot reboot that never happened must not block rollouts or later host operations.
fn expire_reboot_outcomes(store: &StateStore, boot_id: &str, now: i64) -> Result<()> {
    let stale: Vec<(String, String)> = store
        .connection
        .prepare("SELECT operation_id,state FROM host_operations WHERE kind='reboot' AND boot_id=?1 AND state IN ('requested','unknown') AND created_at<?2")?
        .query_map(params![boot_id, now.saturating_sub(REBOOT_OUTCOME_WINDOW)], |r| {
            Ok((r.get(0)?, r.get(1)?))
        })?
        .collect::<Result<_, _>>()?;
    for (operation, state) in stale {
        if transition(
            store,
            "reboot",
            &operation,
            Some(state.as_str()),
            "failed",
            Some("The device did not reboot; the request was not repeated"),
        )? {
            tracing::warn!(operation_id = %operation, previous_state = %state, "Reboot request did not restart the OS; marked it failed");
        }
    }
    Ok(())
}

fn reconcile_updates(state_dir: &Path, store: &StateStore, boot_id: &str) -> Result<()> {
    let updates: Vec<(String,String,String)> = store.connection.prepare("SELECT operation_id,state,boot_id FROM host_operations WHERE kind='update' AND state NOT IN ('completed','rolled_back','failed')")?.query_map([],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?)))?.collect::<Result<_,_>>()?;
    for (operation, state, accepted_boot) in updates {
        match crate::release::update::operation_outcome(state_dir, &operation) {
            Ok(outcome)
                if matches!(
                    outcome.state.as_str(),
                    "completed" | "rolled_back" | "failed"
                ) =>
            {
                update_result(store, &operation, &outcome.state, &outcome.state)?
            }
            _ if accepted_boot != boot_id => update_result(
                store,
                &operation,
                "failed",
                "Device rebooted before the update completed",
            )?,
            Ok(outcome) if outcome.state == "staged" && state == "draining" => update_result(
                store,
                &operation,
                "staging",
                "Resuming verified candidate staging",
            )?,
            // Activation runs only in a drained agent, so a live agent sees a dead activation here.
            Ok(outcome)
                if matches!(state.as_str(), "requesting" | "unknown")
                    && matches!(outcome.state.as_str(), "staged" | "armed") =>
            {
                match crate::release::update::abandon(state_dir, &operation) {
                    Ok(Some(outcome)) => {
                        tracing::warn!(operation_id = %operation, "Update activation stopped before the new binary was installed");
                        update_result(
                            store,
                            &operation,
                            &outcome.state,
                            "Activation stopped before the new binary was installed; the previous binary remains",
                        )?;
                    }
                    Ok(None) => {}
                    Err(error) => {
                        tracing::warn!(operation_id = %operation, "Stalled update activation could not be abandoned yet: {error:#}")
                    }
                }
            }
            // Only boot recovery may restore the binary, so the operation must not block a reboot.
            Ok(outcome)
                if matches!(state.as_str(), "requesting" | "unknown")
                    && outcome.state == "swapped"
                    && crate::release::update::orphaned_swap(state_dir, &operation)
                        .unwrap_or(false) =>
            {
                tracing::warn!(operation_id = %operation, "Update watchdog stopped without confirming or restoring the new binary");
                update_result(
                    store,
                    &operation,
                    "failed",
                    "The update watchdog stopped without confirming the new binary; the next reboot restores the previous release",
                )?;
            }
            _ => (),
        }
    }
    Ok(())
}

fn watch_backoff(failures: u32) -> Duration {
    Duration::from_secs(1u64 << failures.min(5)).min(MAX_WATCH_BACKOFF)
}

/// Run `step` every second until it reports completion or `stop` is cancelled.
/// Per-iteration failures are logged and retried with backoff so host operations keep expiring.
/// Each check is reported as the health of background task `task`.
async fn watch<F, Fut>(name: &str, task: &str, stop: &CancellationToken, mut step: F)
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<bool>>,
{
    let mut tick = tokio::time::interval(Duration::from_secs(1));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let mut failures = 0u32;
    loop {
        tokio::select! { _ = stop.cancelled() => return, _ = tick.tick() => () }
        let checked = step().await;
        crate::diagnostics::global().report_error(task, &checked);
        match checked {
            Ok(true) => return,
            Ok(false) => failures = 0,
            Err(error) => {
                failures = failures.saturating_add(1);
                let delay = watch_backoff(failures);
                tracing::warn!(
                    watcher = name,
                    failures,
                    "Host {name} watcher check failed; retrying in {}s: {error:#}",
                    delay.as_secs()
                );
                tokio::select! { _ = stop.cancelled() => return, _ = tokio::time::sleep(delay) => () }
            }
        }
    }
}

/// Stop the supervisor before asking the OS to reboot. A dispatched request is never retried.
pub async fn watch_reboot(state_dir: &Path, boot_id: &str, stop: CancellationToken) {
    watch(
        "reboot",
        crate::diagnostics::REBOOT_WATCHER,
        &stop,
        || async { claim_reboot(state_dir, boot_id, &stop) },
    )
    .await
}

fn claim_reboot(state_dir: &Path, boot_id: &str, stop: &CancellationToken) -> Result<bool> {
    let store = open(state_dir)?;
    let now = unix_time()?;
    expire_reboot_outcomes(&store, boot_id, now)?;
    let operation:Option<(String,i64)>=store.connection.query_row("SELECT operation_id,created_at FROM host_operations WHERE kind='reboot' AND state='pending' AND boot_id=?1 ORDER BY created_at LIMIT 1",[boot_id],|r|Ok((r.get(0)?,r.get(1)?))).optional()?;
    let Some((operation, created_at)) = operation else {
        return Ok(false);
    };
    let age = now.saturating_sub(created_at);
    if age > REBOOT_DISPATCH_WINDOW {
        transition(
            &store,
            "reboot",
            &operation,
            Some("pending"),
            "failed",
            Some("Reboot dispatch window expired"),
        )?;
        return Ok(false);
    }
    // Leave time for the encrypted accepted response to reach the caller.
    if age < 2 {
        return Ok(false);
    }
    if transition(
        &store,
        "reboot",
        &operation,
        Some("pending"),
        "draining",
        None,
    )? {
        stop.cancel();
        return Ok(true);
    }
    Ok(false)
}

/// Ask the OS to reboot a drained host. A failed or unknown request returns an error so the
/// service manager restarts the agent and its workloads instead of leaving the device idle.
pub async fn dispatch_reboot(state_dir: &Path, boot_id: &str) -> Result<()> {
    let store = open(state_dir)?;
    let operation:Option<String>=store.connection.query_row("SELECT operation_id FROM host_operations WHERE kind='reboot' AND state='draining' AND boot_id=?1 ORDER BY created_at LIMIT 1",[boot_id],|r|r.get(0)).optional()?;
    let Some(operation) = operation else {
        return Ok(());
    };
    // The separate state survives a lost command response and refuses resubmission.
    if !transition(
        &store,
        "reboot",
        &operation,
        Some("draining"),
        "requesting",
        None,
    )? {
        return Ok(());
    }
    #[cfg(target_os = "linux")]
    let result = tokio::time::timeout(
        std::time::Duration::from_secs(15),
        tokio::process::Command::new("systemctl")
            .args(["reboot", "--no-ask-password"])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status(),
    )
    .await;
    #[cfg(not(target_os = "linux"))]
    let result: Result<
        Result<std::process::ExitStatus, std::io::Error>,
        tokio::time::error::Elapsed,
    > = Ok(Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "Remote reboot currently requires Linux systemd",
    )));
    let (state, detail) = match &result {
        Ok(Ok(status)) if status.success() => ("requested", None),
        Err(_) => (
            "unknown",
            Some("OS reboot outcome is unknown; the request will not be repeated"),
        ),
        _ => ("failed", Some("OS rejected reboot; check host permission")),
    };
    transition(
        &store,
        "reboot",
        &operation,
        Some("requesting"),
        state,
        detail,
    )?;
    match result {
        Ok(Ok(status)) if status.success() => Ok(()),
        Ok(Ok(status)) => bail!(
            "OS rejected reboot operation {operation} ({status}); restarting the agent to resume workloads"
        ),
        Ok(Err(error)) => Err(error).with_context(|| {
            format!(
                "Request OS reboot for operation {operation}; restarting the agent to resume workloads"
            )
        }),
        Err(_) => bail!(
            "OS reboot request for operation {operation} timed out; restarting the agent to resume workloads"
        ),
    }
}

/// Stage a verified binary while workloads run; only a complete candidate requests a drain.
pub async fn watch_update(
    state_dir: &Path,
    device_id: &str,
    boot_id: &str,
    run_id: &str,
    stop: CancellationToken,
) {
    watch("update", crate::diagnostics::UPDATE_WATCHER, &stop, || {
        stage_pending_update(state_dir, device_id, boot_id, run_id, &stop)
    })
    .await
}

fn fail_staging(state_dir: &Path, store: &StateStore, operation: &str, detail: &str) -> Result<()> {
    // A journal left staged would refuse every later update until the next boot.
    if let Err(error) = crate::release::update::abandon(state_dir, operation) {
        tracing::warn!(operation_id = %operation, "Staged update candidate was not discarded: {error:#}");
    }
    update_result(store, operation, "failed", detail)
}

async fn stage_pending_update(
    state_dir: &Path,
    device_id: &str,
    boot_id: &str,
    run_id: &str,
    stop: &CancellationToken,
) -> Result<bool> {
    let store = open(state_dir)?;
    // The independent watchdog can finish after the new agent has already started.
    reconcile_updates(state_dir, &store, boot_id)?;
    let pending:Option<(String,i64,String)>=store.connection.query_row("SELECT operation_id,created_at,payload_json FROM host_operations WHERE kind='update' AND state IN ('pending','staging') AND boot_id=?1 ORDER BY created_at LIMIT 1",[boot_id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional()?;
    let Some((operation, created, payload)) = pending else {
        return Ok(false);
    };
    if unix_time()?.saturating_sub(created) > UPDATE_STAGING_WINDOW {
        fail_staging(
            state_dir,
            &store,
            &operation,
            "Update staging window expired",
        )?;
        return Ok(false);
    }
    let release_jws = serde_json::from_str::<serde_json::Value>(&payload)
        .ok()
        .and_then(|payload| payload["release_jws"].as_str().map(str::to_owned));
    let Some(release_jws) = release_jws else {
        tracing::warn!(operation_id = %operation, "Update request has no release manifest");
        fail_staging(
            state_dir,
            &store,
            &operation,
            "Update release manifest is missing",
        )?;
        return Ok(false);
    };
    update_result(&store, &operation, "staging", "Verifying candidate")?;
    let trust_path = state_dir.join("release-trust.json");
    let staging = crate::release::update::stage(
        state_dir,
        &trust_path,
        &release_jws,
        &operation,
        device_id,
        boot_id,
        run_id,
    );
    let staged =
        tokio::select! { _ = stop.cancelled() => return Ok(true), result = staging => result };
    match staged {
        Ok(_) => {
            update_result(
                &store,
                &operation,
                "draining",
                "Candidate verified; draining workloads",
            )?;
            stop.cancel();
            Ok(true)
        }
        Err(error) => {
            tracing::warn!(operation_id = %operation, "Update candidate could not be staged: {error:#}");
            fail_staging(
                state_dir,
                &store,
                &operation,
                "Candidate verification or host readiness failed",
            )?;
            Ok(false)
        }
    }
}

pub async fn dispatch_update(state_dir: &Path, boot_id: &str) -> Result<()> {
    let store = open(state_dir)?;
    let operation:Option<String>=store.connection.query_row("SELECT operation_id FROM host_operations WHERE kind='update' AND state='draining' AND boot_id=?1",[boot_id],|r|r.get(0)).optional()?;
    let Some(operation) = operation else {
        return Ok(());
    };
    update_result(
        &store,
        &operation,
        "requesting",
        "Activating verified candidate",
    )?;
    let Err(error) = crate::release::update::activate(state_dir, &operation).await else {
        return Ok(());
    };
    match crate::release::update::operation_outcome(state_dir, &operation) {
        Ok(outcome) if outcome.state == "failed" => update_result(
            &store,
            &operation,
            "failed",
            "Activation failed before the new binary was installed; the previous binary remains",
        )?,
        // A watchdog may already be running. Its journal, checked on startup, owns the outcome.
        _ => update_result(
            &store,
            &operation,
            "unknown",
            "Activation needs watchdog reconciliation",
        )?,
    }
    Err(error.context(format!("Activate update operation {operation}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store_with_operations(
        rows: &[(&str, &str, &str, i64)],
    ) -> Result<(tempfile::TempDir, StateStore)> {
        let directory = tempfile::tempdir()?;
        let store = open(directory.path())?;
        for (operation, kind, state, created_at) in rows {
            store.connection.execute(
                "INSERT INTO host_operations(operation_id,kind,boot_id,state,created_at) VALUES(?1,?2,'boot',?3,?4)",
                params![operation, kind, state, created_at],
            )?;
        }
        Ok((directory, store))
    }

    fn state(store: &StateStore, operation: &str) -> Result<String> {
        Ok(store.connection.query_row(
            "SELECT state FROM host_operations WHERE operation_id=?1",
            [operation],
            |r| r.get(0),
        )?)
    }

    #[test]
    fn same_boot_reboot_outcomes_expire_instead_of_blocking_forever() -> Result<()> {
        let now = 10_000;
        let old = now - REBOOT_OUTCOME_WINDOW - 1;
        let (_directory, store) = store_with_operations(&[
            ("unknown", "reboot", "unknown", old),
            ("requested", "reboot", "requested", old),
            ("recent", "reboot", "unknown", now - 5),
            ("update", "update", "unknown", old),
        ])?;
        expire_reboot_outcomes(&store, "boot", now)?;
        assert_eq!(state(&store, "unknown")?, "failed");
        assert_eq!(state(&store, "requested")?, "failed");
        assert_eq!(state(&store, "recent")?, "unknown");
        assert_eq!(state(&store, "update")?, "unknown");
        expire_reboot_outcomes(&store, "other-boot", now + REBOOT_OUTCOME_WINDOW)?;
        assert_eq!(state(&store, "recent")?, "unknown");
        Ok(())
    }

    #[test]
    fn transitions_apply_only_from_the_expected_state_and_kind() -> Result<()> {
        let (_directory, store) = store_with_operations(&[("op", "reboot", "draining", 0)])?;
        assert!(!transition(&store, "update", "op", None, "failed", None)?);
        assert!(!transition(
            &store,
            "reboot",
            "op",
            Some("pending"),
            "failed",
            None
        )?);
        assert!(transition(
            &store,
            "reboot",
            "op",
            Some("draining"),
            "requesting",
            None
        )?);
        assert_eq!(state(&store, "op")?, "requesting");
        Ok(())
    }

    #[tokio::test]
    async fn watchers_survive_failed_checks_and_stop_only_on_completion_or_cancel() -> Result<()> {
        use crate::diagnostics::{TaskFailure, TaskState, test_support::health};
        use std::sync::atomic::{AtomicU32, Ordering};
        let stop = CancellationToken::new();
        let calls = AtomicU32::new(0);
        let failing = Some((TaskState::Failing, Some(TaskFailure::Internal)));
        watch("test", "recovering_test_watcher", &stop, || async {
            match calls.fetch_add(1, Ordering::SeqCst) {
                0 => anyhow::bail!("database is locked"),
                _ => {
                    assert_eq!(health("recovering_test_watcher"), failing);
                    Ok(true)
                }
            }
        })
        .await;
        assert_eq!(calls.load(Ordering::SeqCst), 2);
        assert!(!stop.is_cancelled());
        assert_eq!(
            health("recovering_test_watcher"),
            Some((TaskState::Ok, None))
        );

        let cancel = stop.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(50)).await;
            cancel.cancel();
        });
        tokio::time::timeout(
            Duration::from_secs(1),
            watch("test", "failing_test_watcher", &stop, || async {
                Err::<bool, _>(anyhow::anyhow!("disk is full"))
            }),
        )
        .await
        .context("A cancelled watcher must not wait out its backoff")?;
        assert_eq!(health("failing_test_watcher"), failing);
        assert_eq!(watch_backoff(1), Duration::from_secs(2));
        assert_eq!(watch_backoff(40), MAX_WATCH_BACKOFF);
        Ok(())
    }

    #[test]
    fn the_active_operation_names_its_kind_and_issuer_until_it_has_an_outcome() -> Result<()> {
        let (_directory, store) = store_with_operations(&[
            ("finished", "reboot", "failed", 50),
            ("update", "update", "staging", 100),
        ])?;
        store.connection.execute(
            "INSERT INTO management_operations(operation_id,request_digest,principal,accepted_at,result_json) VALUES('update','digest','owner-user:owner',100,'{}')",
            [],
        )?;
        let active = active_operation(&store)?.context("An update is staging")?;
        assert_eq!(active.operation_id, "update");
        assert_eq!(active.kind, "update_agent");
        assert_eq!(active.state, "staging");
        assert_eq!(active.created_at, 100);
        assert_eq!(active.principal.as_deref(), Some("owner-user:owner"));
        store.connection.execute(
            "DELETE FROM management_operations WHERE operation_id='update'",
            [],
        )?;
        assert!(active_operation(&store)?.unwrap().principal.is_none());
        transition(&store, "update", "update", None, "completed", None)?;
        assert!(active_operation(&store)?.is_none());
        Ok(())
    }

    #[cfg(not(target_os = "linux"))]
    #[tokio::test]
    async fn an_unsupported_reboot_fails_loudly_so_the_agent_restarts() -> Result<()> {
        let (directory, store) = store_with_operations(&[("op", "reboot", "draining", 0)])?;
        assert!(dispatch_reboot(directory.path(), "boot").await.is_err());
        assert_eq!(state(&store, "op")?, "failed");
        dispatch_reboot(directory.path(), "boot").await?;
        Ok(())
    }
}
