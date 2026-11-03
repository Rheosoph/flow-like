//! Firing scheduled tasks on the desktop. Local bots' tasks fire from the local store: a due task
//! writes its `trigger` entry and the store's change notifier wakes the host, which answers it
//! like a message. Tasks of account bots that live here are fired through the hub, which claims
//! each run once. A task missed while the app was closed fires once on the first tick and its
//! next run is the first one after that, never one per missed run.

use super::hub::{HubFailure, TurnHub};
use crate::functions::local_bots::{LocalBots, on_store};
use serde_json::Value;
use std::time::Duration;
use tauri::{AppHandle, Manager};

pub const TICK: Duration = Duration::from_secs(30);

pub(super) async fn run_local(app: AppHandle) {
    let mut tick = tokio::time::interval(TICK);
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        tick.tick().await;
        let Some(bots) = app.try_state::<LocalBots>() else {
            continue;
        };
        let now = chrono::Utc::now().timestamp_micros();
        match on_store(&bots, move |store| store.fire_due_tasks(now)).await {
            Ok(fired) if !fired.is_empty() => {
                tracing::info!(count = fired.len(), "Bot host: fired scheduled tasks");
            }
            Ok(_) => {}
            Err(error) => tracing::warn!(%error, "Bot host: firing scheduled tasks failed"),
        }
    }
}

fn is_due(task: &Value, now: chrono::DateTime<chrono::Utc>) -> bool {
    task.get("enabled").and_then(Value::as_bool) == Some(true)
        && task
            .get("next_run_at")
            .and_then(Value::as_str)
            .and_then(|at| chrono::DateTime::parse_from_rfc3339(at).ok())
            .is_some_and(|at| at <= now)
}

/// The ids of `bot_id`'s tasks whose next run has come, from the hub's task list.
pub(super) fn due_task_ids(list: &Value, now: chrono::DateTime<chrono::Utc>) -> Vec<String> {
    list.get("tasks")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|task| is_due(task, now))
        .filter_map(|task| task.get("id").and_then(Value::as_str))
        .map(str::to_owned)
        .collect()
}

/// Fires the due tasks of the account bots that live on this desktop; the hub claims each run
/// once, so a second desktop or the server never writes it twice. Returns how many this call
/// fired.
pub(super) async fn fire_account_tasks(hub: &dyn TurnHub) -> Result<usize, HubFailure> {
    let mut fired = 0;
    for bot_id in hub.desktop_bots().await? {
        let list = hub
            .read(&format!("bots/{bot_id}/tasks"), Vec::new())
            .await?;
        for task_id in due_task_ids(&list, chrono::Utc::now()) {
            let path = format!("bots/{bot_id}/tasks/{task_id}/fire");
            let answer = hub.write(&path, Vec::new(), Value::Null).await?;
            if answer.get("fired").and_then(Value::as_bool) == Some(true) {
                fired += 1;
            }
        }
    }
    Ok(fired)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn only_enabled_tasks_past_their_time_are_due() {
        let now = chrono::DateTime::parse_from_rfc3339("2026-11-03T12:00:00Z")
            .unwrap()
            .with_timezone(&chrono::Utc);
        let list = json!({ "tasks": [
            { "id": "past", "enabled": true, "next_run_at": "2026-11-03T11:59:00Z" },
            { "id": "now", "enabled": true, "next_run_at": "2026-11-03T12:00:00Z" },
            { "id": "later", "enabled": true, "next_run_at": "2026-11-03T12:00:01Z" },
            { "id": "paused", "enabled": false, "next_run_at": "2026-11-03T11:00:00Z" },
            { "id": "done", "enabled": true, "next_run_at": null },
        ]});
        assert_eq!(due_task_ids(&list, now), ["past", "now"]);
        assert!(due_task_ids(&json!({}), now).is_empty());
    }
}
