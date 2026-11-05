//! PostgreSQL checks of what SQLite can't show: claims racing for one turn, the per-owner cap
//! under racing claims on different conversations, and the reply of a turn sent twice at once.
//! Each test makes its own database next to the one the URL names:
//! `FLOW_LIKE_TEST_DATABASE_URL=postgres://… cargo test -p flow-like-api --features db-chaos --lib
//! -- bot_host::chaos_tests`

use super::{
    fixture::{conversation, delivery, insert_bot, message, recipient},
    ledger::{ClaimRequest, Refusal, claim_in, claim_with, enqueue_in},
    worker::{CODE_TIMEOUT, commit_reply, reply_request},
};
use crate::{
    db::{DbDialect, RetryPolicy, retry_transaction},
    entity::{bot_turn, inbox_entry},
    routes::inbox::now_millis,
};
use flow_like_types::{
    bot::BotLocation,
    bot_turn::{
        MAX_RUNNING_PER_OWNER, STORED_DONE, STORED_FAILED, STORED_RUNNING, TurnHostKind,
        failure_message_id, reply_message_id, turn_id,
    },
    inbox::ConversationKind,
};
use futures::future::join_all;
use sea_orm::{
    ColumnTrait, ConnectionTrait, Database, DatabaseConnection, EntityTrait, PaginatorTrait,
    QueryFilter,
    sqlx::postgres::{PgConnectOptions, PgPoolOptions},
};
use serde_json::json;
use std::str::FromStr;

const MIGRATIONS: [&str; 4] = [
    include_str!("../../prisma/migrations/20261101120000_bots/migration.sql"),
    include_str!("../../prisma/migrations/20261101130000_inbox/migration.sql"),
    include_str!("../../prisma/migrations/20261102120000_import_keys/migration.sql"),
    include_str!("../../prisma/migrations/20261102140000_bots_wave_b/migration.sql"),
];
const TABLES: [&str; 5] = [
    "\"Bot\"",
    "\"Conversation\"",
    "\"ConversationMember\"",
    "\"InboxEntry\"",
    "\"BotTurn\"",
];

/// The committed DDL of the tables these tests write, without foreign keys to tables they don't
/// make, and the lock row table.
fn schema() -> Vec<String> {
    let mut statements: Vec<String> = MIGRATIONS
        .iter()
        .map(|migration| {
            migration
                .lines()
                .filter(|line| !line.trim_start().starts_with("--"))
                .collect::<Vec<_>>()
                .join("\n")
        })
        .flat_map(|migration| {
            migration
                .split(';')
                .map(str::trim)
                .filter(|statement| {
                    !statement.is_empty()
                        && !statement.contains("FOREIGN KEY")
                        && TABLES.iter().any(|table| statement.contains(table))
                })
                .map(str::to_string)
                .collect::<Vec<_>>()
        })
        .collect();
    statements.push(
        r#"CREATE TABLE "MutationLock" ("id" BIGINT PRIMARY KEY, "updatedAt" TIMESTAMPTZ(3) DEFAULT now())"#
            .to_string(),
    );
    statements
}

struct Scratch {
    admin: DatabaseConnection,
    db: DatabaseConnection,
    name: String,
}

impl Scratch {
    async fn remove(self) {
        self.db.close().await.unwrap();
        self.admin
            .execute_unprepared(&format!(r#"DROP DATABASE "{}" WITH (FORCE)"#, self.name))
            .await
            .unwrap();
    }
}

async fn postgres() -> Option<Scratch> {
    let Ok(url) = std::env::var("FLOW_LIKE_TEST_DATABASE_URL") else {
        eprintln!("skipping bot turn concurrency test: FLOW_LIKE_TEST_DATABASE_URL is unset");
        return None;
    };
    let admin = Database::connect(&url).await.unwrap();
    let name = format!(
        "turns_{}",
        flow_like_types::create_id().to_ascii_lowercase()
    );
    admin
        .execute_unprepared(&format!(r#"CREATE DATABASE "{name}""#))
        .await
        .unwrap();
    let pool = PgPoolOptions::new()
        .max_connections(16)
        .connect_with(PgConnectOptions::from_str(&url).unwrap().database(&name))
        .await
        .unwrap();
    let db = DatabaseConnection::from(pool);
    for statement in schema() {
        db.execute_unprepared(&statement)
            .await
            .unwrap_or_else(|error| panic!("{statement}: {error}"));
    }
    Some(Scratch { admin, db, name })
}

async fn queue(db: &DatabaseConnection, bot_id: &str, owner: &str, conversation_id: &str) {
    let mut wake = recipient(bot_id, owner, BotLocation::Server);
    wake.conversation_id = conversation_id.into();
    let queued = enqueue_in(
        db,
        &delivery(ConversationKind::Direct, vec![wake]),
        now_millis(),
    )
    .await
    .unwrap();
    assert_eq!(queued, 1);
}

fn claim_request(bot_id: &str, host: &str) -> ClaimRequest {
    ClaimRequest {
        turn_id: turn_id(bot_id, "m1"),
        owner_id: None,
        host_kind: TurnHostKind::Server,
        host_id: host.into(),
        now: now_millis(),
    }
}

#[tokio::test]
async fn racing_claims_with_the_lock_have_one_winner() {
    let Some(scratch) = postgres().await else {
        return;
    };
    let db = scratch.db.clone();
    insert_bot(&db, "bot_1", "user-a", "SERVER").await;
    queue(&db, "bot_1", "user-a", "c1").await;
    let results = join_all((0..8).map(|n| {
        let db = db.clone();
        async move {
            claim_with(
                &db,
                DbDialect::Postgres,
                claim_request("bot_1", &format!("h{n}")),
            )
            .await
            .unwrap()
        }
    }))
    .await;
    let winners: Vec<_> = results.iter().filter(|result| result.is_ok()).collect();
    assert_eq!(winners.len(), 1, "{results:?}");
    assert!(
        results
            .iter()
            .filter_map(|result| result.as_ref().err())
            .all(|refusal| matches!(refusal, Refusal::NotQueued(_) | Refusal::Lost))
    );
    let held = bot_turn::Entity::find_by_id(turn_id("bot_1", "m1"))
        .one(&db)
        .await
        .unwrap()
        .unwrap();
    assert_eq!((held.status.as_str(), held.attempts), (STORED_RUNNING, 1));
    scratch.remove().await;
}

#[tokio::test]
async fn racing_claims_without_the_lock_still_have_one_winner() {
    let Some(scratch) = postgres().await else {
        return;
    };
    let db = scratch.db.clone();
    insert_bot(&db, "bot_1", "user-a", "SERVER").await;
    queue(&db, "bot_1", "user-a", "c1").await;
    let results = join_all((0..8).map(|n| {
        let db = db.clone();
        async move {
            retry_transaction(
                &db,
                DbDialect::Postgres,
                None,
                &RetryPolicy::default(),
                |txn| {
                    let request = claim_request("bot_1", &format!("h{n}"));
                    Box::pin(async move { claim_in(txn, &request).await })
                },
            )
            .await
            .unwrap()
        }
    }))
    .await;
    assert_eq!(
        results.iter().filter(|result| result.is_ok()).count(),
        1,
        "the compare-and-set alone lets one claim through: {results:?}"
    );
    let held = bot_turn::Entity::find_by_id(turn_id("bot_1", "m1"))
        .one(&db)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(held.attempts, 1);
    scratch.remove().await;
}

#[tokio::test]
async fn racing_claims_on_different_conversations_stay_under_the_owner_cap() {
    let Some(scratch) = postgres().await else {
        return;
    };
    let db = scratch.db.clone();
    for n in 0..8 {
        let bot = format!("bot_{n}");
        insert_bot(&db, &bot, "user-a", "SERVER").await;
        queue(&db, &bot, "user-a", &format!("c{n}")).await;
    }
    let results = join_all((0..8).map(|n| {
        let db = db.clone();
        async move {
            claim_with(
                &db,
                DbDialect::Postgres,
                claim_request(&format!("bot_{n}"), "h1"),
            )
            .await
            .unwrap()
        }
    }))
    .await;
    let won = results.iter().filter(|result| result.is_ok()).count();
    assert_eq!(won, MAX_RUNNING_PER_OWNER, "{results:?}");
    assert!(
        results
            .iter()
            .filter_map(|result| result.as_ref().err())
            .all(|refusal| *refusal == Refusal::OwnerBusy)
    );
    let running = bot_turn::Entity::find()
        .filter(bot_turn::Column::Status.eq(STORED_RUNNING))
        .count(&db)
        .await
        .unwrap();
    assert_eq!(running as usize, MAX_RUNNING_PER_OWNER);
    scratch.remove().await;
}

async fn answered_conversation(db: &DatabaseConnection) -> bot_turn::Model {
    insert_bot(db, "bot_1", "user-a", "SERVER").await;
    conversation(db, "c1", "DIRECT", "PLAIN", &["user-a", "bot_1"]).await;
    message(
        db,
        "c1",
        "m1",
        "user-a",
        1,
        json!({ "text": "hi" }),
        &["user-a", "bot_1"],
    )
    .await;
    db.execute_unprepared(r#"UPDATE "Conversation" SET "lastSeq" = 1"#)
        .await
        .unwrap();
    queue(db, "bot_1", "user-a", "c1").await;
    claim_with(db, DbDialect::Postgres, claim_request("bot_1", "h1"))
        .await
        .unwrap()
        .unwrap()
        .turn
}

#[tokio::test]
async fn the_reply_sent_twice_at_once_is_one_message_and_one_completion() {
    let Some(scratch) = postgres().await else {
        return;
    };
    let db = scratch.db.clone();
    let turn = answered_conversation(&db).await;
    let sends = join_all((0..4).map(|_| {
        let (db, turn) = (db.clone(), turn.clone());
        async move {
            let request = reply_request(&turn, reply_message_id("bot_1", "m1"), "answer", None);
            commit_reply(&db, DbDialect::Postgres, &turn, "h1", request, None).await
        }
    }))
    .await;
    let entries: Vec<_> = sends
        .into_iter()
        .map(|sent| sent.unwrap_or_else(|error| panic!("{error:?}")).entry.id)
        .collect();
    assert!(
        entries.windows(2).all(|pair| pair[0] == pair[1]),
        "{entries:?}"
    );
    let copies = inbox_entry::Entity::find()
        .filter(inbox_entry::Column::MessageId.eq(reply_message_id("bot_1", "m1")))
        .count(&db)
        .await
        .unwrap();
    assert_eq!(copies, 2);
    let done = bot_turn::Entity::find_by_id(&turn.id)
        .one(&db)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(done.status, STORED_DONE);
    assert_eq!(done.reply_message_id, Some(reply_message_id("bot_1", "m1")));
    scratch.remove().await;
}

#[tokio::test]
async fn a_failure_report_goes_out_under_the_lock_and_ends_the_turn_failed() {
    let Some(scratch) = postgres().await else {
        return;
    };
    let db = scratch.db.clone();
    let turn = answered_conversation(&db).await;
    let request = reply_request(
        &turn,
        failure_message_id("bot_1", "m1"),
        "I ran out of time",
        Some(CODE_TIMEOUT),
    );
    let sent = commit_reply(
        &db,
        DbDialect::Postgres,
        &turn,
        "h1",
        request,
        Some(CODE_TIMEOUT),
    )
    .await
    .unwrap();
    assert_eq!(
        sent.entry.envelope["body"]["error"]["code"],
        json!(CODE_TIMEOUT)
    );
    let failed = bot_turn::Entity::find_by_id(&turn.id)
        .one(&db)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        (failed.status.as_str(), failed.error_code.as_deref()),
        (STORED_FAILED, Some(CODE_TIMEOUT))
    );
    let other_host = commit_reply(
        &db,
        DbDialect::Postgres,
        &turn,
        "h2",
        reply_request(&turn, reply_message_id("bot_1", "m1"), "late", None),
        None,
    )
    .await
    .unwrap_err();
    assert_eq!(
        other_host.public_code(),
        crate::routes::inbox::BOT_TURN_NOT_CLAIMED
    );
    scratch.remove().await;
}
