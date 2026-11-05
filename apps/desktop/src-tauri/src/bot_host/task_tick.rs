//! Firing scheduled tasks on the desktop. Local bots' tasks fire from the local store: a due task
//! writes its `trigger` entry and the store's change notifier wakes the host, which answers it
//! like a message. Tasks of account bots that live here are fired through the hub, which claims
//! each run once. A task missed while the app was closed fires once on the first tick and its
//! next run is the first one after that, never one per missed run.

use crate::functions::local_bots::{LocalBots, on_store};
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
