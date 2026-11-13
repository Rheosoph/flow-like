use super::{
    fixture::{assert_code, at, database as inbox_database, insert_bot},
    tasks::{Fired, due_in, fire_locked_in, schedule_of},
};
use crate::{
    entity::{bot, bot_task, bot_turn, conversation_member, inbox_entry},
    routes::{
        bots::tasks::{
            BotTaskConversation, BotTaskInfo, CreateBotTaskRequest, TASK_CONVERSATION_ENCRYPTED,
            TaskOrigin, TaskScope, UpdateBotTaskRequest, create_in, delete_in, delete_scoped_in,
            infos_of, list_in, list_scoped_in, origin_of_turn, scope_of_turn, update_in,
            update_scoped_in,
        },
        inbox::{
            fixture::{bot_principal, group, running_turn, user},
            triggers::{trigger_conversation, write_trigger_in},
        },
    },
};
use chrono::{DateTime, Duration, FixedOffset, Utc};
use flow_like_types::{
    bot_task::{
        CreateBotTask, MAX_TASKS_PER_BOT, TaskCreator, TaskKind, UpdateBotTask, fire_message_id,
    },
    bot_turn::{STORED_QUEUED, turn_id},
    inbox::{ConversationEncryption, InboxEntryKind, TriggerBody, TriggerSource},
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
    create_in(db, OWNER, BOT, request(kind), TaskCreator::Owner, None, now())
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
        None,
        now(),
    )
    .await
    .unwrap();
    assert_eq!(task.monitoring, Some(true));
    assert!(
        BotTaskInfo::of(list_in(&db, OWNER, BOT).await.unwrap().remove(0), None).monitoring
    );
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
    assert!(BotTaskInfo::of(reload(&db, &task.id).await, None).monitoring);
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
    assert!(!BotTaskInfo::of(reload(&db, &task.id).await, None).monitoring);
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
    assert!(!BotTaskInfo::of(legacy.clone(), None).monitoring);
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
        None,
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
        None,
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
        None,
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
        let error = create_in(&db, OWNER, BOT, bad.clone(), TaskCreator::Owner, None, now())
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

#[tokio::test]
async fn tasks_of_bots_without_the_scheduling_capability_are_not_due() {
    let db = database().await;
    make_due(&db, TaskKind::Once, at(-60)).await;
    assert_eq!(due_in(&db, now()).await.unwrap().len(), 1);
    bot::ActiveModel {
        id: Set(BOT.into()),
        capabilities: Set(Vec::<String>::new().into()),
        ..Default::default()
    }
    .update(&db)
    .await
    .unwrap();
    assert!(due_in(&db, now()).await.unwrap().is_empty());
    allow_scheduling(&db, BOT).await;
    assert_eq!(due_in(&db, now()).await.unwrap().len(), 1);
}

const MEMBER: &str = "user-b";
const MEMBER_BOT: &str = "bot_b";

/// `team`: the owner, a member, the owner's bot and the member's bot, which reads along.
async fn team(db: &DatabaseConnection, encryption: ConversationEncryption) {
    insert_bot(db, MEMBER_BOT, MEMBER, "SERVER").await;
    let others = [user(MEMBER), bot_principal(BOT), bot_principal(MEMBER_BOT)];
    group(db, "team", OWNER, &others, encryption).await;
}

/// A turn of the owner's bot in `conversation_id` that `requester` woke.
async fn asked_in(db: &DatabaseConnection, turn: &str, conversation_id: &str, requester: &str) {
    running_turn(db, turn, BOT, conversation_id, "host", at(600)).await;
    bot_turn::ActiveModel {
        id: Set(turn.into()),
        requester_id: Set(Some(requester.into())),
        ..Default::default()
    }
    .update(db)
    .await
    .unwrap();
}

/// A due once task the member asked the owner's bot for in `team`.
async fn group_task(db: &DatabaseConnection) -> bot_task::Model {
    team(db, ConversationEncryption::Plain).await;
    asked_in(db, "turn-team", "team", MEMBER).await;
    let origin = origin_of_turn(db, OWNER, BOT, "turn-team")
        .await
        .unwrap()
        .expect("a task asked for in a group fires there");
    let task = create_in(
        db,
        OWNER,
        BOT,
        request(TaskKind::Once),
        TaskCreator::Bot,
        Some(&origin),
        now(),
    )
    .await
    .unwrap();
    bot_task::ActiveModel {
        id: Set(task.id),
        next_run_at: Set(Some(at(-60))),
        ..Default::default()
    }
    .update(db)
    .await
    .unwrap()
}

#[tokio::test]
async fn a_task_asked_for_in_a_group_fires_there_as_the_member_and_wakes_only_its_bot() {
    let db = database().await;
    let task = group_task(&db).await;
    assert_eq!(task.conversation_id.as_deref(), Some("team"));
    assert_eq!(task.requested_by.as_deref(), Some(MEMBER));
    let infos = infos_of(&db, list_in(&db, OWNER, BOT).await.unwrap())
        .await
        .unwrap();
    assert_eq!(
        infos[0].conversation,
        Some(BotTaskConversation {
            id: "team".into(),
            kind: "group".into(),
            title: Some("team".into()),
        })
    );

    let Fired::Sent(_, bots) = fire_locked_in(&db, "team", &task, now()).await.unwrap() else {
        panic!("the task fires in its group");
    };
    let woken: Vec<&str> = bots.iter().map(|bot| bot.bot_id.as_str()).collect();
    assert_eq!(woken, [BOT]);
    let entries = triggers(&db).await;
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].conversation_id, "team");
    assert_eq!(entries[0].sender_id, MEMBER);
    let body = TriggerBody::parse(&entries[0].envelope["body"]).unwrap();
    assert_eq!(body.bot_id.as_deref(), Some(BOT));
    let turn = bot_turn::Entity::find()
        .filter(bot_turn::Column::BotId.eq(BOT))
        .filter(bot_turn::Column::TriggerMessageId.eq(entries[0].message_id.as_str()))
        .one(&db)
        .await
        .unwrap()
        .expect("a turn for the trigger");
    assert_eq!(turn.requester_id.as_deref(), Some(MEMBER));
}

#[tokio::test]
async fn a_group_task_pauses_once_the_member_who_asked_left() {
    let db = database().await;
    let task = group_task(&db).await;
    conversation_member::ActiveModel {
        conversation_id: Set("team".into()),
        principal_id: Set(MEMBER.into()),
        left_at: Set(Some(at(1))),
        ..Default::default()
    }
    .update(&db)
    .await
    .unwrap();
    assert!(matches!(
        fire_locked_in(&db, "team", &task, now()).await.unwrap(),
        Fired::Paused
    ));
    let paused = reload(&db, &task.id).await;
    assert!(!paused.enabled);
    assert!(paused.last_run_at.is_none());
    assert!(triggers(&db).await.is_empty());
    assert!(due_in(&db, now()).await.unwrap().is_empty());
    assert!(matches!(
        fire_locked_in(&db, "team", &task, now()).await.unwrap(),
        Fired::Lost
    ));
}

#[tokio::test]
async fn turns_in_the_owner_chat_or_woken_by_a_bot_keep_tasks_in_the_owner_chat() {
    let db = database().await;
    team(&db, ConversationEncryption::Plain).await;
    let direct = trigger_conversation(&db, OWNER, BOT, now())
        .await
        .unwrap()
        .expect("plain conversation");
    asked_in(&db, "turn-direct", &direct, OWNER).await;
    asked_in(&db, "turn-bot", "team", MEMBER_BOT).await;
    asked_in(&db, "turn-owner", "team", OWNER).await;
    for turn in ["turn-direct", "turn-bot", "turn-unknown"] {
        assert_eq!(
            origin_of_turn(&db, OWNER, BOT, turn).await.unwrap(),
            None,
            "{turn}"
        );
    }
    assert_eq!(
        origin_of_turn(&db, MEMBER, BOT, "turn-owner").await.unwrap(),
        None
    );
    assert_eq!(
        origin_of_turn(&db, OWNER, BOT, "turn-owner").await.unwrap(),
        Some(TaskOrigin {
            conversation_id: "team".into(),
            requested_by: OWNER.into(),
        })
    );
}

#[tokio::test]
async fn an_encrypted_group_refuses_a_task_that_could_never_fire() {
    let db = database().await;
    team(&db, ConversationEncryption::E2e).await;
    asked_in(&db, "turn-sealed", "team", MEMBER).await;
    let error = origin_of_turn(&db, OWNER, BOT, "turn-sealed")
        .await
        .unwrap_err();
    assert_code(
        &error,
        axum::http::StatusCode::CONFLICT,
        TASK_CONVERSATION_ENCRYPTED,
    );
}

async fn ask_in_team(db: &DatabaseConnection, turn: &str, requester: &str) -> TaskOrigin {
    asked_in(db, turn, "team", requester).await;
    let origin = origin_of_turn(db, OWNER, BOT, turn)
        .await
        .unwrap()
        .expect("a turn in a group has an origin");
    create_in(
        db,
        OWNER,
        BOT,
        request(TaskKind::Once),
        TaskCreator::Bot,
        Some(&origin),
        now(),
    )
    .await
    .unwrap();
    origin
}

async fn retitle(
    db: &DatabaseConnection,
    task_id: &str,
    scope: &TaskScope,
) -> Result<bot_task::Model, crate::error::ApiError> {
    update_scoped_in(
        db,
        OWNER,
        BOT,
        task_id,
        UpdateBotTask {
            title: Some("Changed".into()),
            ..Default::default()
        },
        now(),
        scope,
    )
    .await
}

#[tokio::test]
async fn a_room_turn_lists_only_the_tasks_of_that_room() {
    let db = database().await;
    let private = make(&db, TaskKind::Cron).await;
    let room_task = group_task(&db).await;
    let scope = scope_of_turn(&db, OWNER, BOT, "turn-team").await.unwrap();
    let listed = list_scoped_in(&db, OWNER, BOT, &scope).await.unwrap();
    assert_eq!(
        listed.iter().map(|row| row.id.as_str()).collect::<Vec<_>>(),
        [room_task.id.as_str()]
    );
    assert_eq!(list_in(&db, OWNER, BOT).await.unwrap().len(), 2);
    assert!(listed.iter().all(|row| row.id != private.id));
}

#[tokio::test]
async fn a_room_turn_cannot_change_or_cancel_a_private_task() {
    let db = database().await;
    let private = make(&db, TaskKind::Cron).await;
    group_task(&db).await;
    let scope = scope_of_turn(&db, OWNER, BOT, "turn-team").await.unwrap();
    assert!(matches!(scope, TaskScope::Conversation(_)));
    let missing = retitle(&db, "task_missing", &scope).await.unwrap_err();
    let hidden = retitle(&db, &private.id, &scope).await.unwrap_err();
    assert_eq!(hidden.status(), axum::http::StatusCode::NOT_FOUND);
    assert_eq!(
        hidden.to_string().replace(&private.id, "X"),
        missing.to_string().replace("task_missing", "X")
    );
    let cancelled = delete_scoped_in(&db, OWNER, BOT, &private.id, &scope)
        .await
        .unwrap_err();
    assert_eq!(cancelled.status(), axum::http::StatusCode::NOT_FOUND);
    assert_eq!(reload(&db, &private.id).await.title, private.title);
}

#[tokio::test]
async fn a_turn_the_server_does_not_know_reaches_no_task() {
    let db = database().await;
    let private = make(&db, TaskKind::Cron).await;
    let room_task = group_task(&db).await;
    let scope = scope_of_turn(&db, OWNER, BOT, "turn-unknown")
        .await
        .unwrap();
    assert_eq!(scope, TaskScope::Nothing);
    assert!(
        list_scoped_in(&db, OWNER, BOT, &scope)
            .await
            .unwrap()
            .is_empty()
    );
    for task in [&private, &room_task] {
        let refused = retitle(&db, &task.id, &scope).await.unwrap_err();
        assert_eq!(refused.status(), axum::http::StatusCode::NOT_FOUND);
        let refused = delete_scoped_in(&db, OWNER, BOT, &task.id, &scope)
            .await
            .unwrap_err();
        assert_eq!(refused.status(), axum::http::StatusCode::NOT_FOUND);
    }
}

#[tokio::test]
async fn a_turn_another_bot_asked_for_is_held_to_its_room() {
    let db = database().await;
    let private = make(&db, TaskKind::Cron).await;
    let room_task = group_task(&db).await;
    asked_in(&db, "turn-bot", "team", MEMBER_BOT).await;
    let scope = scope_of_turn(&db, OWNER, BOT, "turn-bot").await.unwrap();
    assert_eq!(
        scope,
        TaskScope::Conversation(TaskOrigin {
            conversation_id: "team".into(),
            requested_by: MEMBER_BOT.into(),
        })
    );
    let listed = list_scoped_in(&db, OWNER, BOT, &scope).await.unwrap();
    assert_eq!(
        listed.iter().map(|row| row.id.as_str()).collect::<Vec<_>>(),
        [room_task.id.as_str()]
    );
    for task in [&private, &room_task] {
        let refused = retitle(&db, &task.id, &scope).await.unwrap_err();
        assert_eq!(refused.status(), axum::http::StatusCode::NOT_FOUND);
    }
}

#[tokio::test]
async fn a_member_cannot_change_another_members_room_task_but_the_owner_can() {
    let db = database().await;
    let members = group_task(&db).await;
    let owners_scope = TaskScope::Conversation(ask_in_team(&db, "turn-owner-team", OWNER).await);
    let members_scope = scope_of_turn(&db, OWNER, BOT, "turn-team").await.unwrap();
    let owners_task = list_scoped_in(&db, OWNER, BOT, &owners_scope)
        .await
        .unwrap()
        .into_iter()
        .find(|row| row.requested_by.as_deref() == Some(OWNER))
        .unwrap();

    let refused = retitle(&db, &owners_task.id, &members_scope)
        .await
        .unwrap_err();
    assert_eq!(refused.status(), axum::http::StatusCode::NOT_FOUND);
    let refused = delete_scoped_in(&db, OWNER, BOT, &owners_task.id, &members_scope)
        .await
        .unwrap_err();
    assert_eq!(refused.status(), axum::http::StatusCode::NOT_FOUND);
    assert!(retitle(&db, &members.id, &members_scope).await.is_ok());

    assert!(retitle(&db, &members.id, &owners_scope).await.is_ok());
    delete_scoped_in(&db, OWNER, BOT, &members.id, &owners_scope)
        .await
        .unwrap();
}

#[tokio::test]
async fn the_owners_direct_chat_and_encrypted_rooms_are_scoped_correctly() {
    let db = database().await;
    let private = make(&db, TaskKind::Cron).await;
    group_task(&db).await;
    let direct = trigger_conversation(&db, OWNER, BOT, now())
        .await
        .unwrap()
        .expect("plain conversation");
    asked_in(&db, "turn-direct", &direct, OWNER).await;
    let scope = scope_of_turn(&db, OWNER, BOT, "turn-direct").await.unwrap();
    assert_eq!(scope, TaskScope::Everything);
    assert_eq!(
        list_scoped_in(&db, OWNER, BOT, &scope).await.unwrap().len(),
        2
    );
    assert!(retitle(&db, &private.id, &scope).await.is_ok());

    let sealed = database().await;
    team(&sealed, ConversationEncryption::E2e).await;
    asked_in(&sealed, "turn-sealed", "team", MEMBER).await;
    make(&sealed, TaskKind::Cron).await;
    let scope = scope_of_turn(&sealed, OWNER, BOT, "turn-sealed")
        .await
        .unwrap();
    assert!(matches!(scope, TaskScope::Conversation(_)));
    assert!(
        list_scoped_in(&sealed, OWNER, BOT, &scope)
            .await
            .unwrap()
            .is_empty()
    );
}
