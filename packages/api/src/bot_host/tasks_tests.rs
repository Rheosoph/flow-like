use super::{
    fixture::{at, database as inbox_database, insert_bot},
    tasks::{Fired, due_in, fire_locked_in, schedule_of},
};
use crate::{
    entity::{bot, bot_task, bot_turn, inbox_entry},
    routes::{
        bots::tasks::{
            BotTaskInfo, CreateBotTaskRequest, UpdateBotTaskRequest, create_in, delete_in, list_in,
            update_in,
        },
        inbox::triggers::{trigger_conversation, write_trigger_in},
    },
};
use chrono::{DateTime, Duration, FixedOffset, Utc};
use flow_like_types::{
    bot_task::{
        CreateBotTask, MAX_TASKS_PER_BOT, TaskCreator, TaskKind, UpdateBotTask, fire_message_id,
    },
    bot_turn::{STORED_QUEUED, turn_id},
    inbox::{InboxEntryKind, TriggerBody, TriggerSource},
};
use sea_orm::{
    ActiveModelTrait, ActiveValue::Set, ColumnTrait, ConnectionTrait, DatabaseConnection,
    EntityTrait, PaginatorTrait, QueryFilter, Schema, sea_query::Table,
};

const OWNER: &str = "user-a";
const BOT: &str = "bot_a";

async fn database() -> DatabaseConnection {
    let db = inbox_database().await;
    let schema = Schema::new(db.get_database_backend());
    let source = schema.create_table_from_entity(bot_task::Entity);
    let mut table = Table::create();
    table.table(source.get_table_name().unwrap().clone());
    for column in source.get_columns() {
        table.col(column.clone());
    }
    db.execute(&table).await.unwrap();
    insert_bot(&db, BOT, OWNER, "SERVER").await;
    allow_scheduling(&db, BOT).await;
    db
}

async fn allow_scheduling(db: &DatabaseConnection, bot_id: &str) {
    bot::ActiveModel {
        id: Set(bot_id.into()),
        capabilities: Set(vec!["scheduling".to_string()].into()),
        ..Default::default()
    }
    .update(db)
    .await
    .unwrap();
}

fn now() -> DateTime<FixedOffset> {
    Utc::now().fixed_offset()
}

fn request(kind: TaskKind) -> CreateBotTask {
    CreateBotTask {
        kind: Some(kind),
        at: (kind == TaskKind::Once).then(|| (Utc::now() + Duration::hours(2)).to_rfc3339()),
        cron: (kind == TaskKind::Cron).then(|| "0 9 * * *".to_string()),
        tz: (kind == TaskKind::Cron).then(|| "Europe/Berlin".to_string()),
        instruction: "Check the report".into(),
        title: Some("Report".into()),
        enabled: None,
        monitoring: false,
    }
}

async fn make(db: &DatabaseConnection, kind: TaskKind) -> bot_task::Model {
    create_in(db, OWNER, BOT, request(kind), TaskCreator::Owner, now())
        .await
        .unwrap()
}

async fn make_due(
    db: &DatabaseConnection,
    kind: TaskKind,
    due: DateTime<FixedOffset>,
) -> bot_task::Model {
    let task = make(db, kind).await;
    bot_task::ActiveModel {
        id: Set(task.id),
        next_run_at: Set(Some(due)),
        ..Default::default()
    }
    .update(db)
    .await
    .unwrap()
}

async fn fire(
    db: &DatabaseConnection,
    task: &bot_task::Model,
    when: DateTime<FixedOffset>,
) -> Fired {
    let conversation = trigger_conversation(db, OWNER, BOT, when)
        .await
        .unwrap()
        .expect("plain conversation");
    fire_locked_in(db, &conversation, task, when).await.unwrap()
}

async fn triggers(db: &DatabaseConnection) -> Vec<inbox_entry::Model> {
    inbox_entry::Entity::find()
        .filter(inbox_entry::Column::Kind.eq("TRIGGER"))
        .filter(inbox_entry::Column::InboxId.eq(BOT))
        .all(db)
        .await
        .unwrap()
}

async fn reload(db: &DatabaseConnection, id: &str) -> bot_task::Model {
    bot_task::Entity::find_by_id(id)
        .one(db)
        .await
        .unwrap()
        .unwrap()
}

#[tokio::test]
async fn a_trigger_lands_in_the_bots_copy_and_queues_a_turn_with_no_hops() {
    let db = database().await;
    let task = make_due(&db, TaskKind::Once, at(-60)).await;
    let fired = fire(&db, &task, now()).await;
    assert!(matches!(fired, Fired::Sent(..)));
    let entries = triggers(&db).await;
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].sender_id, OWNER);
    assert_eq!(entries[0].envelope["body"]["source"], "schedule");
    assert_eq!(entries[0].envelope["body"]["ref_id"], task.id);
    assert_eq!(entries[0].envelope["body"]["title"], "Report");
    assert_eq!(entries[0].envelope["body"]["monitoring"], false);
    let message_id = fire_message_id(&task.id, task.next_run_at.unwrap().with_timezone(&Utc));
    let turn = bot_turn::Entity::find_by_id(turn_id(BOT, &message_id))
        .one(&db)
        .await
        .unwrap()
        .expect("a turn for the trigger");
    assert_eq!(turn.status, STORED_QUEUED);
    assert_eq!(turn.hops, 0);
}

#[tokio::test]
async fn monitoring_opt_in_roundtrips_and_fires_as_trusted_schedule_metadata() {
    let db = database().await;
    let task = create_in(
        &db,
        OWNER,
        BOT,
        CreateBotTask {
            monitoring: true,
            ..request(TaskKind::Cron)
        },
        TaskCreator::Owner,
        now(),
    )
    .await
    .unwrap();
    assert_eq!(task.monitoring, Some(true));
    assert!(BotTaskInfo::from(list_in(&db, OWNER, BOT).await.unwrap().remove(0)).monitoring);
    let renamed = update_in(
        &db,
        OWNER,
        BOT,
        &task.id,
        UpdateBotTask {
            title: Some("Watch for changes".into()),
            ..Default::default()
        },
        now(),
    )
    .await
    .unwrap();
    assert_eq!(renamed.monitoring, Some(true));
    assert!(BotTaskInfo::from(reload(&db, &task.id).await).monitoring);
    assert!(matches!(
        fire(&db, &renamed, renamed.next_run_at.unwrap()).await,
        Fired::Sent(..)
    ));
    let entries = triggers(&db).await;
    assert_eq!(entries[0].sender_id, OWNER);
    let body = TriggerBody::parse(&entries[0].envelope["body"]).unwrap();
    assert!(body.monitoring);
    assert_eq!(body.source, TriggerSource::Schedule);
    assert_eq!(body.ref_id, task.id);

    let regular = update_in(
        &db,
        OWNER,
        BOT,
        &task.id,
        UpdateBotTask {
            monitoring: Some(false),
            ..Default::default()
        },
        now(),
    )
    .await
    .unwrap();
    assert_eq!(regular.monitoring, Some(false));
    assert!(!BotTaskInfo::from(reload(&db, &task.id).await).monitoring);
    assert!(matches!(
        fire(&db, &regular, regular.next_run_at.unwrap()).await,
        Fired::Sent(..)
    ));
    assert!(
        triggers(&db)
            .await
            .iter()
            .any(|entry| entry.envelope["body"]["monitoring"] == false)
    );
}

#[tokio::test]
async fn legacy_null_monitoring_is_a_regular_reminder_and_new_rows_default_false() {
    let db = database().await;
    let task = make_due(&db, TaskKind::Once, at(-60)).await;
    assert_eq!(task.monitoring, Some(false));
    let mut defaulted: bot_task::ActiveModel = task.clone().into();
    defaulted.id = Set("task-default-monitoring".into());
    defaulted.monitoring = sea_orm::ActiveValue::NotSet;
    assert_eq!(defaulted.insert(&db).await.unwrap().monitoring, Some(false));
    let legacy = bot_task::ActiveModel {
        id: Set(task.id),
        monitoring: Set(None),
        ..Default::default()
    }
    .update(&db)
    .await
    .unwrap();
    assert_eq!(reload(&db, &legacy.id).await.monitoring, None);
    assert!(!BotTaskInfo::from(legacy.clone()).monitoring);
    assert!(matches!(fire(&db, &legacy, now()).await, Fired::Sent(..)));
    assert_eq!(triggers(&db).await[0].envelope["body"]["monitoring"], false);
}

#[tokio::test]
async fn a_monitoring_edit_invalidates_an_already_selected_due_task() {
    let db = database().await;
    let task = make_due(&db, TaskKind::Once, at(-60)).await;
    let stale = update_in(
        &db,
        OWNER,
        BOT,
        &task.id,
        UpdateBotTask {
            monitoring: Some(true),
            ..Default::default()
        },
        task.updated_at,
    )
    .await
    .unwrap();
    let current = update_in(
        &db,
        OWNER,
        BOT,
        &task.id,
        UpdateBotTask {
            monitoring: Some(false),
            ..Default::default()
        },
        task.updated_at,
    )
    .await
    .unwrap();
    assert!(matches!(fire(&db, &stale, now()).await, Fired::Lost));
    assert!(triggers(&db).await.is_empty());
    assert!(matches!(fire(&db, &current, now()).await, Fired::Sent(..)));
    assert_eq!(triggers(&db).await[0].envelope["body"]["monitoring"], false);
}

#[test]
fn legacy_api_requests_do_not_opt_into_monitoring() {
    let request: CreateBotTaskRequest = serde_json::from_value(serde_json::json!({
        "kind":"once", "at":"2027-01-01T00:00:00Z", "instruction":"Remind me to call"
    }))
    .unwrap();
    assert!(!request.monitoring);
    let update: UpdateBotTaskRequest =
        serde_json::from_value(serde_json::json!({"title":"Rename"})).unwrap();
    assert_eq!(update.monitoring, None);
    let update: UpdateBotTaskRequest =
        serde_json::from_value(serde_json::json!({"monitoring":false})).unwrap();
    assert_eq!(update.monitoring, Some(false));
}

#[tokio::test]
async fn a_trigger_in_a_direct_conversation_wakes_the_bot() {
    let db = database().await;
    let conversation = trigger_conversation(&db, OWNER, BOT, now())
        .await
        .unwrap()
        .unwrap();
    let body = |bot_id: Option<&str>| TriggerBody {
        monitoring: false,
        source: TriggerSource::Webhook,
        ref_id: "hook".into(),
        title: "Hook".into(),
        text: "Do it".into(),
        payload: None,
        payload_handle: None,
        bot_id: bot_id.map(str::to_string),
    };
    write_trigger_in(&db, &conversation, OWNER, "tk-direct", &body(None), now())
        .await
        .unwrap();
    assert!(
        bot_turn::Entity::find_by_id(turn_id(BOT, "tk-direct"))
            .one(&db)
            .await
            .unwrap()
            .is_some(),
        "a direct conversation always wakes the bot"
    );
}

#[tokio::test]
async fn only_one_of_many_sweeps_fires_a_run() {
    let db = database().await;
    let task = make_due(&db, TaskKind::Cron, at(-3_600)).await;
    let conversation = trigger_conversation(&db, OWNER, BOT, now())
        .await
        .unwrap()
        .unwrap();
    let attempts = (0..8).map(|_| fire_locked_in(&db, &conversation, &task, now()));
    let results = futures::future::join_all(attempts).await;
    let fired = results
        .into_iter()
        .filter(|result| matches!(result, Ok(Fired::Sent(..))))
        .count();
    assert_eq!(fired, 1);
    assert_eq!(triggers(&db).await.len(), 1);
    assert_eq!(
        bot_turn::Entity::find().count(&db).await.unwrap(),
        1,
        "one run, one turn"
    );
}

#[tokio::test]
async fn a_second_sweep_after_the_first_finds_nothing_due() {
    let db = database().await;
    let task = make_due(&db, TaskKind::Once, at(-60)).await;
    assert_eq!(due_in(&db, now()).await.unwrap().len(), 1);
    fire(&db, &task, now()).await;
    assert!(due_in(&db, now()).await.unwrap().is_empty());
    assert!(matches!(fire(&db, &task, now()).await, Fired::Lost));
    assert_eq!(triggers(&db).await.len(), 1);
}

#[tokio::test]
async fn a_once_task_is_spent_after_its_run() {
    let db = database().await;
    let task = make_due(&db, TaskKind::Once, at(-60)).await;
    fire(&db, &task, now()).await;
    let after = reload(&db, &task.id).await;
    assert!(!after.enabled);
    assert!(after.next_run_at.is_none());
    assert!(after.last_run_at.is_some());
}

#[tokio::test]
async fn missed_runs_fire_once_and_the_next_run_is_in_the_future() {
    let db = database().await;
    let week_ago = (Utc::now() - Duration::days(7)).fixed_offset();
    let task = make_due(&db, TaskKind::Cron, week_ago).await;
    let sweep = now();
    fire(&db, &task, sweep).await;
    assert_eq!(triggers(&db).await.len(), 1, "seven missed days, one run");
    let after = reload(&db, &task.id).await;
    assert!(after.enabled);
    assert!(after.next_run_at.unwrap() > sweep);
    assert!(due_in(&db, sweep).await.unwrap().is_empty());
    assert!(schedule_of(&after).is_some());
}

#[tokio::test]
async fn paused_tasks_and_tasks_of_desktop_or_deleted_bots_do_not_fire_on_the_server() {
    let db = database().await;
    let paused = make_due(&db, TaskKind::Once, at(-60)).await;
    update_in(
        &db,
        OWNER,
        BOT,
        &paused.id,
        UpdateBotTask {
            enabled: Some(false),
            ..Default::default()
        },
        now(),
    )
    .await
    .unwrap();
    assert!(due_in(&db, now()).await.unwrap().is_empty());
    insert_bot(&db, "bot_d", OWNER, "DESKTOP").await;
    allow_scheduling(&db, "bot_d").await;
    let desktop = create_in(
        &db,
        OWNER,
        "bot_d",
        request(TaskKind::Once),
        TaskCreator::Owner,
        now(),
    )
    .await
    .unwrap();
    bot_task::ActiveModel {
        id: Set(desktop.id),
        next_run_at: Set(Some(at(-60))),
        ..Default::default()
    }
    .update(&db)
    .await
    .unwrap();
    assert!(due_in(&db, now()).await.unwrap().is_empty());
}

#[tokio::test]
async fn a_bot_keeps_at_most_fifty_tasks_and_needs_the_capability() {
    let db = database().await;
    for _ in 0..MAX_TASKS_PER_BOT {
        make(&db, TaskKind::Once).await;
    }
    let error = create_in(
        &db,
        OWNER,
        BOT,
        request(TaskKind::Once),
        TaskCreator::Bot,
        now(),
    )
    .await
    .unwrap_err();
    assert_eq!(error.status(), axum::http::StatusCode::CONFLICT);
    assert_eq!(
        list_in(&db, OWNER, BOT).await.unwrap().len(),
        MAX_TASKS_PER_BOT
    );

    insert_bot(&db, "bot_plain", OWNER, "SERVER").await;
    let refused = create_in(
        &db,
        OWNER,
        "bot_plain",
        request(TaskKind::Once),
        TaskCreator::Owner,
        now(),
    )
    .await
    .unwrap_err();
    assert_eq!(refused.status(), axum::http::StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn schedules_are_validated_when_saved() {
    let db = database().await;
    for bad in [
        CreateBotTask {
            cron: Some("* * * * *".into()),
            at: None,
            ..request(TaskKind::Cron)
        },
        CreateBotTask {
            cron: Some("0 9 * *".into()),
            at: None,
            ..request(TaskKind::Cron)
        },
        CreateBotTask {
            tz: Some("Mars/Base".into()),
            ..request(TaskKind::Cron)
        },
        CreateBotTask {
            at: Some("2001-01-01T00:00:00Z".into()),
            ..request(TaskKind::Once)
        },
        CreateBotTask {
            instruction: " ".into(),
            ..request(TaskKind::Once)
        },
    ] {
        let error = create_in(&db, OWNER, BOT, bad.clone(), TaskCreator::Owner, now())
            .await
            .unwrap_err();
        assert_eq!(
            error.status(),
            axum::http::StatusCode::BAD_REQUEST,
            "{bad:?}"
        );
    }
}

#[tokio::test]
async fn a_task_belongs_to_its_owner() {
    let db = database().await;
    let task = make(&db, TaskKind::Cron).await;
    assert_eq!(
        list_in(&db, "user-b", BOT).await.unwrap_err().status(),
        axum::http::StatusCode::NOT_FOUND
    );
    assert_eq!(
        delete_in(&db, "user-b", BOT, &task.id)
            .await
            .unwrap_err()
            .status(),
        axum::http::StatusCode::NOT_FOUND
    );
    delete_in(&db, OWNER, BOT, &task.id).await.unwrap();
    assert!(list_in(&db, OWNER, BOT).await.unwrap().is_empty());
}

#[tokio::test]
async fn pausing_and_resuming_moves_the_next_run_to_now_onwards() {
    let db = database().await;
    let task = make(&db, TaskKind::Cron).await;
    let paused = update_in(
        &db,
        OWNER,
        BOT,
        &task.id,
        UpdateBotTask {
            enabled: Some(false),
            ..Default::default()
        },
        now(),
    )
    .await
    .unwrap();
    assert!(!paused.enabled);
    let resumed = update_in(
        &db,
        OWNER,
        BOT,
        &task.id,
        UpdateBotTask {
            enabled: Some(true),
            ..Default::default()
        },
        now(),
    )
    .await
    .unwrap();
    assert!(resumed.enabled);
    assert!(resumed.next_run_at.unwrap() > now());
}

#[test]
fn a_trigger_is_not_sendable_by_clients() {
    assert!(!InboxEntryKind::Trigger.client_sendable());
    let request = crate::routes::inbox::model::SendEntryRequest {
        message_id: "m1".into(),
        kind: InboxEntryKind::Trigger,
        envelope: Some(
            TriggerBody {
                monitoring: true,
                source: TriggerSource::Schedule,
                ref_id: "t".into(),
                title: "t".into(),
                text: "t".into(),
                payload: None,
                payload_handle: None,
                bot_id: None,
            }
            .envelope(),
        ),
        envelopes: None,
        turn: None,
    };
    assert!(crate::routes::inbox::entries::validate_send(request).is_err());
}
