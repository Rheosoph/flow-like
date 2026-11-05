//! Firing the scheduled tasks of the account bots that live on a host, through the hub.

use crate::hub::{HubFailure, TurnHub};
use serde_json::Value;

fn is_due(task: &Value, now: chrono::DateTime<chrono::Utc>) -> bool {
    task.get("enabled").and_then(Value::as_bool) == Some(true)
        && task
            .get("next_run_at")
            .and_then(Value::as_str)
            .and_then(|at| chrono::DateTime::parse_from_rfc3339(at).ok())
            .is_some_and(|at| at <= now)
}

/// The ids of `bot_id`'s tasks whose next run has come, from the hub's task list.
pub fn due_task_ids(list: &Value, now: chrono::DateTime<chrono::Utc>) -> Vec<String> {
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
pub async fn fire_account_tasks(hub: &dyn TurnHub) -> Result<usize, HubFailure> {
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
