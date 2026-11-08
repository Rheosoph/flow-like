use super::*;
use crate::recovery::{CappedRecovery, HubRecovery, remaining_deadline};
use flow_like::flow::copilot::run_recovery::BotRunRecovery;
use flow_like_types::bot_turn::BotRunCheckpoint;

#[tokio::test]
async fn cancellation_reports_failure_when_the_hub_cannot_persist_it() {
    let hub = Arc::new(DeviceHub {
        cancel_refused: true,
        ..Default::default()
    });
    *hub.entries.lock().unwrap() = vec![plain("hi")];
    let f = Fixture::with_hub(Mode::Hang, hub, config());
    f.until("the active run", |f| *f.runner.started.lock().unwrap() == 1)
        .await;
    let failure = f.host.cancel("bt_1").await.unwrap_err();
    assert!(
        failure
            .message
            .contains("cancellation could not reach the hub")
    );
    f.until("local cancellation", |f| {
        !f.runner.cancelled.lock().unwrap().is_empty()
    })
    .await;
    assert!(!f.hub.has(|call| matches!(call, Call::Send(_))));
}

#[tokio::test]
async fn cancellation_succeeds_only_after_the_hub_accepts_it() {
    let f = Fixture::start(Mode::Hang, vec![plain("hi")]);
    f.until("the active run", |f| *f.runner.started.lock().unwrap() == 1)
        .await;
    assert!(f.host.cancel("bt_1").await.unwrap());
    f.until("local cancellation", |f| {
        f.status() == Some(TurnStatus::Cancelled)
    })
    .await;
    assert!(!f.host.cancel("bt_1").await.unwrap());
}

#[tokio::test]
async fn recovery_fetches_an_old_trigger_without_replacing_recent_room_updates() {
    let hub = Arc::new(DeviceHub {
        recovery_enabled: true,
        ..Default::default()
    });
    let original = plain("Continue the original task");
    *hub.original_trigger.lock().unwrap() = Some(original.clone());
    *hub.entries.lock().unwrap() = (2..102)
        .rev()
        .map(|seq| {
            let mut entry = plain("A newer message");
            entry.id = format!("e{seq}");
            entry.message_id = format!("m{seq}");
            entry.seq = Some(seq);
            entry
        })
        .collect();
    let entries = hub.entries_for_turn("bot", "conv", "m1").await.unwrap();
    assert_eq!(entries.len(), 101);
    assert_eq!(entries.first().unwrap().message_id, "m101");
    assert_eq!(entries.last().unwrap().message_id, original.message_id);
    assert_eq!(
        *hub.trigger_reads.lock().unwrap(),
        vec![vec![
            ("as", "bot".to_owned()),
            ("message_id", "m1".to_owned())
        ]]
    );
}

#[tokio::test]
async fn trigger_lookup_does_not_accept_a_different_conversation() {
    let hub = DeviceHub {
        recovery_enabled: true,
        ..Default::default()
    };
    let mut original = plain("This belongs elsewhere");
    original.conversation_id = "another-room".into();
    *hub.original_trigger.lock().unwrap() = Some(original);
    assert!(hub.entries_for_turn("bot", "conv", "m1").await.is_err());
}

#[tokio::test]
async fn a_restarted_host_loads_the_same_deadline_and_fences_every_write_by_claim() {
    let hub = Arc::new(DeviceHub::default());
    let mut turn = turn("bt_saved");
    turn.attempts = 2;
    let recovery = HubRecovery::new(hub.clone(), &turn, HOST, 99_000);
    let checkpoint = BotRunCheckpoint {
        revision: 1,
        started_at_ms: 1000,
        deadline_at_ms: 31_000,
        state: json!({"phase":"ready"}),
    };
    recovery.save(&checkpoint).await.unwrap();
    let restarted = HubRecovery::new(hub.clone(), &turn, HOST, 999_000);
    assert_eq!(restarted.load().await.unwrap(), Some(checkpoint));
    assert_eq!(restarted.deadline_at_ms(), Some(31_000));
    assert!(
        hub.checkpoint_calls
            .lock()
            .unwrap()
            .iter()
            .all(|call| call == &("bot".into(), "bt_saved".into(), HOST.into(), 2))
    );
}

#[tokio::test]
async fn a_lost_claim_cannot_load_or_advance_execution_state() {
    let hub = Arc::new(DeviceHub {
        checkpoint_lease_lost: true,
        ..Default::default()
    });
    let recovery = HubRecovery::new(hub.clone(), &turn("bt_saved"), HOST, 31_000);
    assert!(recovery.load().await.is_err());
    assert!(
        recovery
            .save(&BotRunCheckpoint {
                revision: 1,
                started_at_ms: 1000,
                deadline_at_ms: 31_000,
                state: json!({"phase":"ready"})
            })
            .await
            .is_err()
    );
    assert!(hub.checkpoint.lock().unwrap().is_none());
}

#[test]
fn expired_runs_only_receive_a_short_finalization_window() {
    let recovery: Arc<dyn BotRunRecovery> = Arc::new(HubRecovery::new(
        Arc::new(DeviceHub::default()),
        &turn("bt_saved"),
        HOST,
        1,
    ));
    assert_eq!(
        remaining_deadline(Some(&recovery), Duration::from_secs(1800)),
        Duration::from_secs(30)
    );
    assert_eq!(
        remaining_deadline(None, Duration::from_secs(1800)),
        Duration::from_secs(1800)
    );
    assert_eq!(
        remaining_deadline(Some(&recovery), Duration::from_secs(5)),
        Duration::from_secs(5)
    );
    let active: Arc<dyn BotRunRecovery> = Arc::new(HubRecovery::new(
        Arc::new(DeviceHub::default()),
        &turn("bt_active"),
        HOST,
        Utc::now().timestamp_millis() + 5_000,
    ));
    assert!(remaining_deadline(Some(&active), Duration::from_secs(1800)) <= Duration::from_secs(5));
}

#[tokio::test]
async fn a_shorter_device_claim_caps_execution_without_rewriting_saved_deadlines() {
    let hub = Arc::new(DeviceHub::default());
    let saved = BotRunCheckpoint {
        revision: 1,
        started_at_ms: 1000,
        deadline_at_ms: 31_000,
        state: json!({"phase":"ready"}),
    };
    let inner: Arc<dyn BotRunRecovery> = Arc::new(HubRecovery::new(
        hub.clone(),
        &turn("bt_saved"),
        HOST,
        31_000,
    ));
    inner.save(&saved).await.unwrap();
    let capped = CappedRecovery::new(inner, 11_000);
    assert_eq!(capped.load().await.unwrap(), Some(saved.clone()));
    assert_eq!(capped.deadline_at_ms(), Some(11_000));
    assert_eq!(
        hub.checkpoint
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .deadline_at_ms,
        31_000
    );
}
