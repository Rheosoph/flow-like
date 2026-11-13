use super::{Fixture, TestKey, create, fixture};
use crate::{Bot, LOCAL_OWNER_ID};
use chrono::{Duration, Utc};
use flow_like_types::{
    bot_task::{BotTask, CreateBotTask, MAX_TASKS_PER_BOT, TaskCreator, TaskKind, UpdateBotTask},
    inbox::{ConversationKind, InboxEntryKind},
};
use serde_json::json;

fn scheduler_bot(f: &Fixture) -> Bot {
    f.store
        .create_bot(create(json!({
            "name": "Clock",
            "appearance": { "kind": "pip" },
            "capabilities": ["scheduling"],
        })))
        .unwrap()
}

fn once(minutes: i64) -> CreateBotTask {
    CreateBotTask {
        kind: Some(TaskKind::Once),
        at: Some((Utc::now() + Duration::minutes(minutes)).to_rfc3339()),
        instruction: "Check the report".into(),
        title: Some("Report".into()),
        ..Default::default()
    }
}

fn hourly() -> CreateBotTask {
    CreateBotTask {
        kind: Some(TaskKind::Cron),
        cron: Some("0 * * * *".into()),
        tz: Some("Europe/Berlin".into()),
        instruction: "Hourly sweep".into(),
        ..Default::default()
    }
}

fn micros(task: &BotTask) -> i64 {
    chrono::DateTime::parse_from_rfc3339(task.next_run_at.as_deref().unwrap())
        .unwrap()
        .timestamp_micros()
}

fn triggers(f: &Fixture, bot: &Bot) -> Vec<crate::InboxEntry> {
    let conversation = f.direct(bot);
    f.all_entries(&conversation.id)
        .into_iter()
        .filter(|entry| entry.kind == InboxEntryKind::Trigger)
        .collect()
}

fn later(task: &BotTask) -> i64 {
    micros(task) + 1_000
}

#[test]
fn a_task_needs_the_scheduling_capability_a_valid_schedule_and_room() {
    let f = fixture();
    let plain = f.bot("Plain");
    assert!(
        f.store
            .create_task(&plain.id, once(5), TaskCreator::Owner)
            .is_err()
    );
    let bot = scheduler_bot(&f);
    let task = f
        .store
        .create_task(&bot.id, once(5), TaskCreator::Bot)
        .unwrap();
    assert_eq!(
        (task.kind, task.created_by, task.enabled),
        (TaskKind::Once, TaskCreator::Bot, true)
    );
    assert_eq!(task.owner_id, LOCAL_OWNER_ID);
    for bad in [
        CreateBotTask {
            at: Some("2001-01-01T00:00:00Z".into()),
            ..once(5)
        },
        CreateBotTask {
            cron: Some("* * * * *".into()),
            kind: Some(TaskKind::Cron),
            at: None,
            ..once(5)
        },
        CreateBotTask {
            instruction: "  ".into(),
            ..once(5)
        },
    ] {
        assert!(
            f.store
                .create_task(&bot.id, bad, TaskCreator::Owner)
                .is_err()
        );
    }
    for _ in 1..MAX_TASKS_PER_BOT {
        f.store
            .create_task(&bot.id, once(5), TaskCreator::Owner)
            .unwrap();
    }
    assert!(
        f.store
            .create_task(&bot.id, once(5), TaskCreator::Owner)
            .is_err()
    );
    assert_eq!(f.store.tasks(&bot.id).unwrap().len(), MAX_TASKS_PER_BOT);
}

#[test]
fn a_due_task_writes_one_trigger_from_the_person_and_wakes_the_inbox() {
    let f = fixture();
    let bot = scheduler_bot(&f);
    let task = f
        .store
        .create_task(&bot.id, once(5), TaskCreator::Owner)
        .unwrap();
    let before = f.changes.take();
    assert!(
        f.store
            .fire_due_tasks(micros(&task) - 1)
            .unwrap()
            .is_empty()
    );
    let fired = f.store.fire_due_tasks(later(&task)).unwrap();
    assert_eq!(fired.len(), 1);
    assert!(!fired[0].enabled && fired[0].next_run_at.is_none());
    let entries = triggers(&f, &bot);
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].sender_id, LOCAL_OWNER_ID);
    assert_eq!(entries[0].envelope["body"]["source"], "schedule");
    assert_eq!(entries[0].envelope["body"]["ref_id"], task.id);
    assert_eq!(entries[0].envelope["body"]["monitoring"], false);
    assert!(f.changes.take().len() >= 1 || !before.is_empty());
}

#[test]
fn monitoring_opt_in_survives_edits_reopen_and_firing() {
    let f = fixture();
    let bot = scheduler_bot(&f);
    let task = f
        .store
        .create_task(
            &bot.id,
            CreateBotTask {
                monitoring: true,
                ..hourly()
            },
            TaskCreator::Owner,
        )
        .unwrap();
    assert!(task.monitoring);
    let renamed = f
        .store
        .update_task(
            &bot.id,
            &task.id,
            UpdateBotTask {
                title: Some("Check for changes".into()),
                ..Default::default()
            },
        )
        .unwrap();
    assert!(renamed.monitoring);
    let reopened = f.open_again(TestKey::default()).unwrap();
    assert!(reopened.tasks(&bot.id).unwrap()[0].monitoring);
    let fired = reopened.fire_due_tasks(later(&renamed)).unwrap();
    assert!(fired[0].monitoring);
    let entries = triggers(&f, &bot);
    assert_eq!(entries[0].sender_id, LOCAL_OWNER_ID);
    let body = flow_like_types::inbox::TriggerBody::parse(&entries[0].envelope["body"]).unwrap();
    assert!(body.monitoring);
    assert_eq!(body.ref_id, task.id);
    assert_eq!(body.source, flow_like_types::inbox::TriggerSource::Schedule);

    let regular = reopened
        .update_task(
            &bot.id,
            &task.id,
            UpdateBotTask {
                monitoring: Some(false),
                ..Default::default()
            },
        )
        .unwrap();
    assert!(!regular.monitoring);
    reopened.fire_due_tasks(later(&regular)).unwrap();
    let entries = triggers(&f, &bot);
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0].envelope["body"]["monitoring"], false);
}

#[test]
fn legacy_tasks_and_wire_requests_remain_regular_reminders() {
    let f = fixture();
    let bot = scheduler_bot(&f);
    let mut request = serde_json::to_value(once(5)).unwrap();
    request.as_object_mut().unwrap().remove("monitoring");
    let legacy: CreateBotTask = serde_json::from_value(request).unwrap();
    assert!(!legacy.monitoring);
    let task = f
        .store
        .create_task(&bot.id, legacy, TaskCreator::Owner)
        .unwrap();
    let mut view = serde_json::to_value(&task).unwrap();
    view.as_object_mut().unwrap().remove("monitoring");
    assert!(!serde_json::from_value::<BotTask>(view).unwrap().monitoring);
    assert_eq!(
        serde_json::from_value::<UpdateBotTask>(json!({"title":"Renamed"}))
            .unwrap()
            .monitoring,
        None
    );

    let raw = f.raw();
    raw.execute_batch("ALTER TABLE tasks DROP COLUMN monitoring;")
        .unwrap();
    let migration = crate::schema::MIGRATIONS
        .iter()
        .find(|migration| migration.contains("ALTER TABLE tasks ADD COLUMN monitoring"))
        .unwrap();
    raw.execute_batch(migration).unwrap();
    let reopened = f.open_again(TestKey::default()).unwrap();
    assert!(!reopened.tasks(&bot.id).unwrap()[0].monitoring);
    reopened.fire_due_tasks(later(&task)).unwrap();
    assert_eq!(triggers(&f, &bot)[0].envelope["body"]["monitoring"], false);
    let legacy_trigger = flow_like_types::inbox::TriggerBody::parse(&json!({
        "source":"schedule", "ref_id":task.id, "title":"Reminder", "text":"Check the report"
    }))
    .unwrap();
    assert!(!legacy_trigger.monitoring);
}

#[test]
fn ticks_race_for_a_run_and_one_wins() {
    let f = fixture();
    let bot = scheduler_bot(&f);
    let task = f
        .store
        .create_task(&bot.id, once(5), TaskCreator::Owner)
        .unwrap();
    let now = later(&task);
    let store = f.store.clone();
    let handles: Vec<_> = (0..8)
        .map(|_| {
            let store = store.clone();
            std::thread::spawn(move || store.fire_due_tasks(now).map(|fired| fired.len()))
        })
        .collect();
    let fired: usize = handles
        .into_iter()
        .map(|h| h.join().unwrap().unwrap())
        .sum();
    assert_eq!(fired, 1);
    assert_eq!(triggers(&f, &bot).len(), 1);
}

#[test]
fn missed_cron_runs_fire_once_and_the_next_run_is_after_now() {
    let f = fixture();
    let bot = scheduler_bot(&f);
    let task = f
        .store
        .create_task(&bot.id, hourly(), TaskCreator::Owner)
        .unwrap();
    let a_week_later = micros(&task) + Duration::days(7).num_microseconds().unwrap();
    let fired = f.store.fire_due_tasks(a_week_later).unwrap();
    assert_eq!(fired.len(), 1);
    assert_eq!(
        triggers(&f, &bot).len(),
        1,
        "a week of hourly runs, one trigger"
    );
    assert!(micros(&fired[0]) > a_week_later);
    assert!(f.store.fire_due_tasks(a_week_later).unwrap().is_empty());
}

#[test]
fn paused_tasks_do_not_fire_and_resuming_recomputes_the_next_run() {
    let f = fixture();
    let bot = scheduler_bot(&f);
    let task = f
        .store
        .create_task(&bot.id, hourly(), TaskCreator::Owner)
        .unwrap();
    let paused = f
        .store
        .update_task(
            &bot.id,
            &task.id,
            UpdateBotTask {
                enabled: Some(false),
                ..Default::default()
            },
        )
        .unwrap();
    assert!(!paused.enabled);
    assert!(
        f.store
            .fire_due_tasks(micros(&task) + 10_000)
            .unwrap()
            .is_empty()
    );
    let resumed = f
        .store
        .update_task(
            &bot.id,
            &task.id,
            UpdateBotTask {
                enabled: Some(true),
                ..Default::default()
            },
        )
        .unwrap();
    assert!(resumed.enabled && micros(&resumed) > Utc::now().timestamp_micros());
}

#[test]
fn deleting_a_task_or_its_bot_removes_it_and_it_never_fires() {
    let f = fixture();
    let bot = scheduler_bot(&f);
    let task = f
        .store
        .create_task(&bot.id, once(5), TaskCreator::Owner)
        .unwrap();
    f.store.delete_task(&bot.id, &task.id).unwrap();
    assert!(f.store.delete_task(&bot.id, &task.id).is_err());
    assert!(f.store.fire_due_tasks(later(&task)).unwrap().is_empty());
    let other = f
        .store
        .create_task(&bot.id, once(5), TaskCreator::Owner)
        .unwrap();
    f.store.delete_bot(&bot.id).unwrap();
    let raw: i64 = f
        .raw()
        .query_row(
            "SELECT COUNT(*) FROM tasks WHERE id = ?1",
            [&other.id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(raw, 0);
}

#[test]
fn a_trigger_is_idempotent_by_message_id() {
    let f = fixture();
    let bot = scheduler_bot(&f);
    let body = flow_like_types::inbox::TriggerBody {
        monitoring: false,
        source: flow_like_types::inbox::TriggerSource::Webhook,
        ref_id: "hook".into(),
        title: "Hook".into(),
        text: "Go".into(),
        payload: None,
        payload_handle: None,
        bot_id: None,
    };
    let first = f.store.post_trigger(&bot.id, "tk-1", &body).unwrap();
    let again = f.store.post_trigger(&bot.id, "tk-1", &body).unwrap();
    assert_eq!(first.id, again.id);
    assert_eq!(triggers(&f, &bot).len(), 1);
    let malformed = f.send(
        &first.conversation_id,
        LOCAL_OWNER_ID,
        "tk-2",
        "trigger",
        json!({}),
    );
    assert!(
        malformed.is_err(),
        "a trigger needs a source, a ref and a title"
    );
}

fn group_triggers(f: &Fixture, conversation_id: &str) -> Vec<crate::InboxEntry> {
    f.all_entries(conversation_id)
        .into_iter()
        .filter(|entry| entry.kind == InboxEntryKind::Trigger)
        .collect()
}

#[test]
fn a_task_scheduled_in_a_group_fires_there_naming_its_bot() {
    let f = fixture();
    let bot = scheduler_bot(&f);
    let other = f.bot("Other");
    let group = f.group(&[&bot, &other]);
    let direct = f.direct(&bot);
    let private = f
        .store
        .create_task_in(&bot.id, once(5), TaskCreator::Bot, Some(&direct.id))
        .unwrap();
    assert_eq!(private.conversation, None);
    f.store.delete_task(&bot.id, &private.id).unwrap();

    let task = f
        .store
        .create_task_in(&bot.id, once(5), TaskCreator::Bot, Some(&group.id))
        .unwrap();
    let place = task.conversation.clone().expect("the task fires in its group");
    assert_eq!(
        (place.id.as_str(), place.kind, place.title.as_deref()),
        (group.id.as_str(), ConversationKind::Group, Some("Crew"))
    );
    assert_eq!(f.store.fire_due_tasks(later(&task)).unwrap().len(), 1);
    let fired = group_triggers(&f, &group.id);
    assert_eq!(fired.len(), 1);
    assert_eq!(fired[0].sender_id, LOCAL_OWNER_ID);
    assert_eq!(fired[0].envelope["body"]["bot_id"], json!(bot.id));
    assert!(triggers(&f, &bot).is_empty());
}

#[test]
fn a_group_task_pauses_once_its_bot_left_the_group() {
    let f = fixture();
    let bot = scheduler_bot(&f);
    let other = f.bot("Other");
    let group = f.group(&[&bot, &other]);
    let task = f
        .store
        .create_task_in(&bot.id, hourly(), TaskCreator::Bot, Some(&group.id))
        .unwrap();
    f.store.remove_member(&group.id, &bot.id).unwrap();
    assert!(f.store.fire_due_tasks(later(&task)).unwrap().is_empty());
    let paused = f
        .store
        .tasks(&bot.id)
        .unwrap()
        .into_iter()
        .find(|row| row.id == task.id)
        .unwrap();
    assert!(!paused.enabled);
    assert_eq!(paused.last_run_at, None);
    assert!(group_triggers(&f, &group.id).is_empty());
    assert!(triggers(&f, &bot).is_empty());
}

#[test]
fn a_group_turn_sees_and_changes_only_the_tasks_of_its_group() {
    let f = fixture();
    let bot = scheduler_bot(&f);
    let other = f.bot("Other");
    let group = f.group(&[&bot, &other]);
    let private = f
        .store
        .create_task(&bot.id, once(5), TaskCreator::Bot)
        .unwrap();
    let shared = f
        .store
        .create_task_in(&bot.id, once(6), TaskCreator::Bot, Some(&group.id))
        .unwrap();

    let in_group = f.store.tasks_in_turn(&bot.id, Some(&group.id)).unwrap();
    assert_eq!(
        in_group.iter().map(|task| task.id.as_str()).collect::<Vec<_>>(),
        [shared.id.as_str()]
    );
    let direct = f.direct(&bot);
    assert_eq!(f.store.tasks_in_turn(&bot.id, Some(&direct.id)).unwrap().len(), 2);
    assert_eq!(f.store.tasks_in_turn(&bot.id, None).unwrap().len(), 2);

    let rename = || UpdateBotTask {
        title: Some("Changed".into()),
        ..Default::default()
    };
    let hidden = f
        .store
        .update_task_in_turn(&bot.id, &private.id, rename(), Some(&group.id))
        .unwrap_err();
    let missing = f
        .store
        .update_task_in_turn(&bot.id, "task_missing", rename(), Some(&group.id))
        .unwrap_err();
    assert_eq!(
        hidden.to_string().replace(&private.id, "X"),
        missing.to_string().replace("task_missing", "X")
    );
    assert!(f
        .store
        .delete_task_in_turn(&bot.id, &private.id, Some(&group.id))
        .is_err());
    assert_eq!(f.store.tasks(&bot.id).unwrap().len(), 2);

    f.store
        .update_task_in_turn(&bot.id, &shared.id, rename(), Some(&group.id))
        .unwrap();
    f.store
        .delete_task_in_turn(&bot.id, &shared.id, Some(&group.id))
        .unwrap();
    f.store
        .delete_task_in_turn(&bot.id, &private.id, Some(&direct.id))
        .unwrap();
    assert!(f.store.tasks(&bot.id).unwrap().is_empty());
}
