use crate::{enrollment::unix_time, state::StateStore};
use anyhow::{Result, ensure};
use rusqlite::{OptionalExtension, params};
use std::path::Path;
use tokio_util::sync::CancellationToken;

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

/// Observe reboot completion only after a different OS boot, never after an agent restart.
pub fn reconcile_boot(state_dir: &Path, boot_id: &str) -> Result<()> {
    let store = StateStore::open(&state_dir.join("management.sqlite"))?;
    store.connection.execute("UPDATE host_operations SET state='completed' WHERE kind='reboot' AND state IN ('pending','draining','requesting','requested','unknown') AND boot_id<>?1",[boot_id])?;
    store.connection.execute("UPDATE host_operations SET state='pending' WHERE kind='reboot' AND state='draining' AND boot_id=?1", [boot_id])?;
    store.connection.execute("UPDATE host_operations SET state='unknown' WHERE kind='reboot' AND state='requesting' AND boot_id=?1", [boot_id])?;
    store.connection.execute("UPDATE management_operations SET result_json=json_set(result_json,'$.state','completed','$.result.reboot','completed') WHERE operation_id IN (SELECT operation_id FROM host_operations WHERE kind='reboot' AND state='completed')",[])?;
    reconcile_updates(state_dir, &store, boot_id)
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
            _ => (),
        }
    }
    Ok(())
}

/// Stop the supervisor before asking the OS to reboot. A dispatched request is never retried.
pub async fn watch_reboot(state_dir: &Path, boot_id: &str, stop: CancellationToken) -> Result<()> {
    let mut tick = tokio::time::interval(std::time::Duration::from_secs(1));
    loop {
        tokio::select! {_=stop.cancelled()=>return Ok(()),_=tick.tick()=>()}
        let store = StateStore::open(&state_dir.join("management.sqlite"))?;
        let operation:Option<(String,i64)>=store.connection.query_row("SELECT operation_id,created_at FROM host_operations WHERE kind='reboot' AND state='pending' AND boot_id=?1 ORDER BY created_at LIMIT 1",[boot_id],|r|Ok((r.get(0)?,r.get(1)?))).optional()?;
        if let Some((operation, created_at)) = operation {
            if unix_time()?.saturating_sub(created_at) > 120 {
                store.connection.execute("UPDATE host_operations SET state='failed' WHERE operation_id=?1 AND state='pending'",[&operation])?;
                store.connection.execute("UPDATE management_operations SET result_json=json_set(result_json,'$.state','failed','$.result.reboot','Reboot dispatch window expired') WHERE operation_id=?1",[&operation])?;
                continue;
            }
            // Leave time for the encrypted accepted response to reach the caller.
            if unix_time()?.saturating_sub(created_at) < 2 {
                continue;
            }
            let claimed=store.connection.execute("UPDATE host_operations SET state='draining' WHERE operation_id=?1 AND state='pending'",[&operation])?;
            if claimed == 1 {
                stop.cancel();
            }
            return Ok(());
        }
    }
}

pub async fn dispatch_reboot(state_dir: &Path, boot_id: &str) -> Result<()> {
    let store = StateStore::open(&state_dir.join("management.sqlite"))?;
    let operation:Option<String>=store.connection.query_row("SELECT operation_id FROM host_operations WHERE kind='reboot' AND state='draining' AND boot_id=?1 ORDER BY created_at LIMIT 1",[boot_id],|r|r.get(0)).optional()?;
    let Some(operation) = operation else {
        return Ok(());
    };
    // The separate state survives a lost command response and refuses resubmission.
    let attempted = store.connection.execute(
        "UPDATE host_operations SET state='requesting' WHERE operation_id=?1 AND state='draining'",
        [&operation],
    )?;
    if attempted != 1 {
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
    let successful = matches!(&result,Ok(Ok(status)) if status.success());
    let unknown = result.is_err();
    store.connection.execute(
        "UPDATE host_operations SET state=?2 WHERE operation_id=?1 AND state='requesting'",
        params![
            operation,
            if successful {
                "requested"
            } else if unknown {
                "unknown"
            } else {
                "failed"
            }
        ],
    )?;
    if unknown {
        store.connection.execute("UPDATE management_operations SET result_json=json_set(result_json,'$.state','unknown','$.result.reboot','OS reboot outcome is unknown; the request will not be repeated') WHERE operation_id=?1",[operation])?;
    } else if !successful {
        store.connection.execute("UPDATE management_operations SET result_json=json_set(result_json,'$.state','failed','$.result.reboot','OS rejected reboot; check host permission') WHERE operation_id=?1",[operation])?;
    }
    Ok(())
}

fn update_result(store: &StateStore, operation: &str, state: &str, detail: &str) -> Result<()> {
    store.connection.execute(
        "UPDATE host_operations SET state=?2 WHERE operation_id=?1",
        params![operation, state],
    )?;
    store.connection.execute("UPDATE management_operations SET result_json=json_set(result_json,'$.state',?2,'$.result.update',?3) WHERE operation_id=?1", params![operation,state,detail])?;
    Ok(())
}

/// Stage a verified binary while workloads run; only a complete candidate requests a drain.
pub async fn watch_update(
    state_dir: &Path,
    device_id: &str,
    boot_id: &str,
    run_id: &str,
    stop: CancellationToken,
) -> Result<()> {
    let mut tick = tokio::time::interval(std::time::Duration::from_secs(1));
    loop {
        tokio::select! { _=stop.cancelled()=>return Ok(()), _=tick.tick()=>() }
        let store = StateStore::open(&state_dir.join("management.sqlite"))?;
        // The independent watchdog can finish after the new agent has already started.
        reconcile_updates(state_dir, &store, boot_id)?;
        let pending:Option<(String,i64,String)>=store.connection.query_row("SELECT operation_id,created_at,payload_json FROM host_operations WHERE kind='update' AND state IN ('pending','staging') AND boot_id=?1 ORDER BY created_at LIMIT 1",[boot_id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional()?;
        let Some((operation, created, payload)) = pending else {
            continue;
        };
        if unix_time()?.saturating_sub(created) > 600 {
            update_result(
                &store,
                &operation,
                "failed",
                "Update staging window expired",
            )?;
            continue;
        }
        let payload: serde_json::Value = serde_json::from_str(&payload)?;
        let compact = payload["release_jws"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("Update release manifest is missing"))?;
        update_result(&store, &operation, "staging", "Verifying candidate")?;
        let trust_path = state_dir.join("release-trust.json");
        let staging = crate::release::update::stage(
            state_dir,
            &trust_path,
            compact,
            &operation,
            device_id,
            boot_id,
            run_id,
        );
        let staged = tokio::select! { _=stop.cancelled()=>return Ok(()), result=staging=>result };
        match staged {
            Ok(_) => {
                update_result(
                    &store,
                    &operation,
                    "draining",
                    "Candidate verified; draining workloads",
                )?;
                stop.cancel();
                return Ok(());
            }
            Err(error) => {
                tracing::warn!(operation_id=%operation, "Update candidate could not be staged: {error}");
                update_result(
                    &store,
                    &operation,
                    "failed",
                    "Candidate verification or host readiness failed",
                )?;
            }
        }
    }
}

pub async fn dispatch_update(state_dir: &Path, boot_id: &str) -> Result<()> {
    let store = StateStore::open(&state_dir.join("management.sqlite"))?;
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
    if let Err(error) = crate::release::update::activate(state_dir, &operation).await {
        // A watchdog may already be running. Its journal, checked on startup, owns the outcome.
        update_result(
            &store,
            &operation,
            "unknown",
            "Activation needs watchdog reconciliation",
        )?;
        return Err(error);
    }
    Ok(())
}
