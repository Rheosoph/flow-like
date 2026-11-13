use super::{
    authorization::context,
    fixture::{at, conversation, database as inbox_database, entry, insert_bot, message},
    task_authority::{OwnerRequest, TaskAuthor, TaskEdit, author_of_turn, owner_request_of_turn},
};
use crate::{
    entity::{bot, bot_task, bot_turn, conversation as conversation_entity, inbox_entry},
    routes::{
        bots::tasks::{TaskScope, create_in, origin_of_turn, update_in, update_scoped_in},
        inbox::fixture::{running_turn, turn},
    },
};
use chrono::{DateTime, FixedOffset, Utc};
use flow_like_types::{
    bot_task::{CreateBotTask, TaskKind, UpdateBotTask},
    inbox::{direct_pair_key, tombstone_envelope},
};
use sea_orm::{
    ActiveModelTrait,
    ActiveValue::Set,
    ColumnTrait, ConnectionTrait, DatabaseConnection, EntityTrait, QueryFilter, Schema,
    sea_query::{Expr, Table},
};
use serde_json::{Value, json};

const OWNER: &str = "user-a";
const MEMBER: &str = "user-b";
const BOT: &str = "bot_a";
const CHAT: &str = "dm";
const ASKED: &str = "Every morning at 8, mail me the sales summary";

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
    bot::ActiveModel {
        id: Set(BOT.into()),
        capabilities: Set(vec!["scheduling".to_string()].into()),
        ..Default::default()
    }
    .update(&db)
    .await
    .unwrap();
    db
}

fn now() -> DateTime<FixedOffset> {
    Utc::now().fixed_offset()
}

/// The owner's own chat with the bot.
async fn owners_chat(db: &DatabaseConnection, encryption: &str) {
    conversation(db, CHAT, "DIRECT", encryption, &[OWNER, BOT]).await;
    conversation_entity::Entity::update_many()
        .col_expr(
            conversation_entity::Column::DirectKey,
            Expr::value(direct_pair_key(OWNER, BOT)),
        )
        .filter(conversation_entity::Column::Id.eq(CHAT))
        .exec(db)
        .await
        .unwrap();
}

/// A turn of the bot in `room` that `sender`'s message `text` started, as `requester`.
async fn asked(
    db: &DatabaseConnection,
    room: &str,
    turn_id: &str,
    sender: &str,
    seq: i64,
    text: &str,
) {
    running_turn(db, turn_id, BOT, room, "host", at(600)).await;
    let requester = (sender != OWNER).then(|| sender.to_string());
    bot_turn::ActiveModel {
        id: Set(turn_id.into()),
        requester_id: Set(requester),
        ..Default::default()
    }
    .update(db)
    .await
    .unwrap();
    let id = format!("trigger-{turn_id}");
    message(db, room, &id, sender, seq, json!({"text": text}), &[BOT]).await;
}

/// A run of `task` in `room`: its trigger, sent with `sender`'s id, and the turn answering it.
async fn run_of(
    db: &DatabaseConnection,
    room: &str,
    task: &bot_task::Model,
    turn_id: &str,
    sender: &str,
    seq: i64,
) -> bot_turn::Model {
    running_turn(db, turn_id, BOT, room, "host", at(600)).await;
    let body = json!({
        "source": "schedule", "ref_id": task.id, "title": task.title, "text": task.instruction
    });
    let id = format!("trigger-{turn_id}");
    entry(db, room, &id, sender, seq, "TRIGGER", body, &[BOT]).await;
    turn(db, turn_id).await
}

fn cron(instruction: &str) -> CreateBotTask {
    CreateBotTask {
        kind: Some(TaskKind::Cron),
        cron: Some("0 8 * * *".into()),
        tz: Some("Europe/Berlin".into()),
        instruction: instruction.into(),
        ..Default::default()
    }
}

async fn request_of(db: &DatabaseConnection, turn_id: &str) -> Option<OwnerRequest> {
    owner_request_of_turn(db, OWNER, BOT, turn_id)
        .await
        .unwrap()
}

async fn configured(db: &DatabaseConnection, run: &bot_turn::Model) -> Value {
    context(db, run, true).await.unwrap()["configured_trigger_instruction"].clone()
}

async fn scheduled_in_chat(db: &DatabaseConnection) -> bot_task::Model {
    asked(db, CHAT, "turn-ask", OWNER, 1, ASKED).await;
    let author = TaskAuthor::Bot(request_of(db, "turn-ask").await);
    create_in(
        db,
        OWNER,
        BOT,
        cron("Mail the owner the sales summary"),
        author,
        None,
        now(),
    )
    .await
    .unwrap()
}

#[tokio::test]
async fn the_owners_message_that_asked_for_a_task_authorizes_its_runs_as_it_reads_now() {
    let db = database().await;
    owners_chat(&db, "PLAIN").await;
    let task = scheduled_in_chat(&db).await;
    assert_eq!(task.created_by, "BOT");
    assert_eq!(task.owner_written, Some(false));
    assert_eq!(task.request_conversation_id.as_deref(), Some(CHAT));
    assert_eq!(task.request_message_id.as_deref(), Some("trigger-turn-ask"));

    let run = run_of(&db, CHAT, &task, "turn-run", OWNER, 2).await;
    let read = configured(&db, &run).await;
    assert_eq!(read["source"], "schedule");
    assert_eq!(read["id"], task.id);
    assert_eq!(read["owner_message"]["text"], ASKED);
    assert_eq!(read["owner_message"]["id"], "trigger-turn-ask");
    assert_eq!(
        read["scheduled_instruction"],
        "Mail the owner the sales summary"
    );
    assert!(read.get("instruction").is_none());

    let edit =
        json!({"target": "trigger-turn-ask", "text": "Every morning at 8, mail me the totals"});
    entry(&db, CHAT, "edit-1", OWNER, 3, "EDIT", edit, &[BOT]).await;
    assert_eq!(
        configured(&db, &run).await["owner_message"]["text"],
        "Every morning at 8, mail me the totals"
    );

    inbox_entry::Entity::update_many()
        .col_expr(
            inbox_entry::Column::Envelope,
            Expr::value(tombstone_envelope()),
        )
        .filter(inbox_entry::Column::MessageId.is_in(["trigger-turn-ask", "edit-1"]))
        .exec(&db)
        .await
        .unwrap();
    let deletion = json!({"target": "trigger-turn-ask"});
    entry(&db, CHAT, "delete-1", OWNER, 4, "DELETE", deletion, &[BOT]).await;
    assert!(configured(&db, &run).await.is_null());
}

#[tokio::test]
async fn only_the_owner_prompting_their_own_chat_gives_a_task_authority() {
    let db = database().await;
    owners_chat(&db, "PLAIN").await;
    conversation(&db, "team", "GROUP", "PLAIN", &[OWNER, BOT, MEMBER]).await;
    let task = scheduled_in_chat(&db).await;
    assert_eq!(
        request_of(&db, "turn-ask").await,
        Some(OwnerRequest {
            conversation_id: CHAT.into(),
            message_id: "trigger-turn-ask".into(),
        })
    );
    run_of(&db, CHAT, &task, "turn-run", OWNER, 2).await;
    asked(&db, "team", "turn-member", MEMBER, 1, ASKED).await;
    asked(&db, "team", "turn-owner-group", OWNER, 2, ASKED).await;
    asked(&db, CHAT, "turn-other-bot", "bot_b", 3, ASKED).await;
    asked(&db, CHAT, "turn-unprompted", OWNER, 4, ASKED).await;
    bot_turn::ActiveModel {
        id: Set("turn-unprompted".into()),
        unprompted: Set(Some(true)),
        ..Default::default()
    }
    .update(&db)
    .await
    .unwrap();
    for turn_id in [
        "turn-run",
        "turn-member",
        "turn-owner-group",
        "turn-other-bot",
        "turn-unprompted",
        "turn-unknown",
        "",
    ] {
        assert_eq!(request_of(&db, turn_id).await, None, "{turn_id}");
        assert_eq!(
            author_of_turn(&db, OWNER, BOT, Some(turn_id))
                .await
                .unwrap(),
            TaskAuthor::Bot(None),
            "{turn_id}"
        );
    }
    assert_eq!(
        author_of_turn(&db, OWNER, BOT, None).await.unwrap(),
        TaskAuthor::Owner
    );
}

#[tokio::test]
async fn an_encrypted_owner_chat_leaves_the_message_to_hosts_with_keys() {
    let db = database().await;
    owners_chat(&db, "E2E").await;
    let task = scheduled_in_chat(&db).await;
    assert_eq!(task.request_message_id.as_deref(), Some("trigger-turn-ask"));
    let run = run_of(&db, CHAT, &task, "turn-run", OWNER, 2).await;
    let authority = context(&db, &run, true).await.unwrap();
    assert_eq!(authority["unreadable_conversation_ids"], json!([CHAT]));
    let read = &authority["configured_trigger_instruction"];
    assert_eq!(
        read["owner_message_ref"],
        json!({"conversation_id": CHAT, "message_id": "trigger-turn-ask"})
    );
    assert!(read.get("owner_message").is_none());
    assert!(read.get("instruction").is_none());
}

#[tokio::test]
async fn legacy_rows_and_member_tasks_authorize_nothing_but_owner_written_tasks_do() {
    let db = database().await;
    owners_chat(&db, "PLAIN").await;
    conversation(&db, "team", "GROUP", "PLAIN", &[OWNER, BOT, MEMBER]).await;
    let owned = create_in(
        &db,
        OWNER,
        BOT,
        cron("Draft the weekly report"),
        TaskAuthor::Owner,
        None,
        now(),
    )
    .await
    .unwrap();
    assert_eq!(owned.created_by, "OWNER");
    let run = run_of(&db, CHAT, &owned, "turn-run", OWNER, 1).await;
    assert_eq!(
        configured(&db, &run).await,
        json!({"id": owned.id, "source": "schedule", "instruction": "Draft the weekly report"})
    );

    bot_task::ActiveModel {
        id: Set(owned.id.clone()),
        owner_written: Set(None),
        ..Default::default()
    }
    .update(&db)
    .await
    .unwrap();
    assert!(configured(&db, &run).await.is_null(), "a legacy row");

    asked(&db, "team", "turn-member", MEMBER, 1, ASKED).await;
    let author = author_of_turn(&db, OWNER, BOT, Some("turn-member"))
        .await
        .unwrap();
    let origin = origin_of_turn(&db, OWNER, BOT, "turn-member")
        .await
        .unwrap();
    assert!(origin.is_some(), "a member's task fires in their group");
    let members = create_in(&db, OWNER, BOT, cron(ASKED), author, origin.as_ref(), now())
        .await
        .unwrap();
    let run = run_of(&db, "team", &members, "turn-team-run", MEMBER, 2).await;
    assert!(configured(&db, &run).await.is_null(), "a member's task");
}

async fn reload(db: &DatabaseConnection, id: &str) -> bot_task::Model {
    bot_task::Entity::find_by_id(id)
        .one(db)
        .await
        .unwrap()
        .unwrap()
}

async fn edit_as(
    db: &DatabaseConnection,
    task_id: &str,
    turn_id: &str,
    change: UpdateBotTask,
) -> bot_task::Model {
    let author = author_of_turn(db, OWNER, BOT, Some(turn_id)).await.unwrap();
    let edit = TaskEdit { change, author };
    update_scoped_in(db, OWNER, BOT, task_id, edit, now(), &TaskScope::Everything)
        .await
        .unwrap()
}

fn instruction(text: &str) -> UpdateBotTask {
    UpdateBotTask {
        instruction: Some(text.into()),
        ..Default::default()
    }
}

fn request_ids(task: &bot_task::Model) -> (Option<bool>, Option<&str>) {
    (task.owner_written, task.request_message_id.as_deref())
}

#[tokio::test]
async fn a_rewrite_moves_the_authority_to_its_author_and_an_unprompted_one_drops_it() {
    let db = database().await;
    owners_chat(&db, "PLAIN").await;
    let task = scheduled_in_chat(&db).await;
    let run = run_of(&db, CHAT, &task, "turn-run", OWNER, 2).await;

    let retitled = edit_as(
        &db,
        &task.id,
        &run.id,
        UpdateBotTask {
            title: Some("Sales".into()),
            cron: Some("0 9 * * *".into()),
            instruction: Some(task.instruction.clone()),
            ..Default::default()
        },
    )
    .await;
    assert_eq!(
        request_ids(&retitled),
        (Some(false), Some("trigger-turn-ask")),
        "a change that keeps the instruction keeps its authority"
    );

    asked(
        &db,
        CHAT,
        "turn-again",
        OWNER,
        3,
        "Make it the weekly summary instead",
    )
    .await;
    let moved = edit_as(
        &db,
        &task.id,
        "turn-again",
        instruction("Mail the weekly summary"),
    )
    .await;
    assert_eq!(
        request_ids(&moved),
        (Some(false), Some("trigger-turn-again"))
    );
    assert_eq!(moved.request_conversation_id.as_deref(), Some(CHAT));

    let dropped = edit_as(&db, &task.id, &run.id, instruction("Mail it to everyone")).await;
    assert_eq!(request_ids(&dropped), (Some(false), None));
    assert_eq!(dropped.request_conversation_id, None);
    assert!(configured(&db, &run).await.is_null());

    let owned = update_in(
        &db,
        OWNER,
        BOT,
        &task.id,
        instruction("Mail me the summary"),
        now(),
    )
    .await
    .unwrap();
    assert_eq!(request_ids(&owned), (Some(true), None));

    let bare: TaskEdit = instruction("Mail the summary to the team").into();
    update_scoped_in(
        &db,
        OWNER,
        BOT,
        &task.id,
        bare,
        now(),
        &TaskScope::Everything,
    )
    .await
    .unwrap();
    assert_eq!(
        request_ids(&reload(&db, &task.id).await),
        (Some(false), None)
    );

    let renewed = edit_as(
        &db,
        &task.id,
        "turn-again",
        instruction("Mail the weekly sums"),
    )
    .await;
    assert_eq!(
        request_ids(&renewed),
        (Some(false), Some("trigger-turn-again"))
    );
}
