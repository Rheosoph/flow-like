//! PostgreSQL checks of what SQLite can't show: writers racing on one conversation, each in a
//! retried transaction that takes the conversation's lock, with commit conflicts injected the way
//! DSQL raises them. Each test makes its own database next to the one the URL names:
//! `FLOW_LIKE_TEST_DATABASE_URL=postgres://… cargo test -p flow-like-api --features db-chaos --lib
//! -- routes::inbox::concurrency_tests`

use super::{
    BOT_TURN_NOT_CLAIMED,
    conversations::{NewConversation, start_group},
    entries::{Outgoing, Sent, send_in},
    fixture::{at, bot_principal, insert_bot, running_turn, text, turn, user},
    lock_conversation,
    members::Principal,
    model::TurnClaim,
    now_millis,
    turns::{TURN_DONE, reply_as_bot},
};
use crate::{
    db::{DbDialect, RetryPolicy, testing::Chaos},
    entity::{conversation, inbox_entry},
    error::ApiError,
};
use flow_like_types::inbox::{ConversationEncryption, ConversationKind};
use futures::future::join_all;
use sea_orm::{
    ColumnTrait, ConnectionTrait, Database, DatabaseConnection, EntityTrait, QueryFilter,
    QueryOrder,
    sqlx::postgres::{PgConnectOptions, PgPoolOptions},
};
use std::{
    collections::{BTreeMap, BTreeSet},
    str::FromStr,
    sync::Arc,
};

const MIGRATIONS: [&str; 4] = [
    include_str!("../../../prisma/migrations/20261101120000_bots/migration.sql"),
    include_str!("../../../prisma/migrations/20261101130000_inbox/migration.sql"),
    include_str!("../../../prisma/migrations/20261102120000_import_keys/migration.sql"),
    include_str!("../../../prisma/migrations/20261102140000_bots_wave_b/migration.sql"),
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
        eprintln!("skipping inbox concurrency test: FLOW_LIKE_TEST_DATABASE_URL is unset");
        return None;
    };
    let admin = Database::connect(&url).await.unwrap();
    let name = format!(
        "inbox_{}",
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

/// A group `id` started by user-a with `others`, committed.
async fn group(db: &DatabaseConnection, id: &str, others: &[Principal]) {
    let plan = NewConversation {
        kind: ConversationKind::Group,
        title: Some(id.into()),
        members: Vec::new(),
        encryption: ConversationEncryption::Plain,
    };
    let txn = sea_orm::TransactionTrait::begin(db).await.unwrap();
    start_group(&txn, id, "user-a", &plan, others, at(0))
        .await
        .unwrap();
    txn.commit().await.unwrap();
}

/// `outgoing` sent by `sender` the way the handler sends it, conflicts injected.
async fn send(
    db: DatabaseConnection,
    chaos: Arc<Chaos>,
    sender: &'static str,
    outgoing: Outgoing,
) -> Result<Sent, ApiError> {
    let outgoing = Arc::new(outgoing);
    chaos
        .transaction(
            &db,
            DbDialect::Postgres,
            None,
            &RetryPolicy::default(),
            move |txn| {
                let outgoing = outgoing.clone();
                Box::pin(async move {
                    lock_conversation(txn, "team").await?;
                    send_in(txn, "team", sender, &outgoing, now_millis()).await
                })
            },
        )
        .await
}

#[tokio::test]
async fn concurrent_sends_take_gap_free_positions_shared_by_every_copy() {
    let Some(scratch) = postgres().await else {
        return;
    };
    let db = scratch.db.clone();
    group(&db, "team", &[user("user-b"), user("user-c")]).await;
    let chaos = Arc::new(Chaos::new(None, Some(3)));
    let senders = ["user-a", "user-b", "user-c"];
    let mut sends: Vec<(&'static str, String)> = (0..20)
        .map(|n| (senders[n % 3], format!("m_{n}")))
        .collect();
    sends.extend((0..4).map(|n| (senders[n % 3], format!("m_{n}"))));
    let results =
        join_all(sends.iter().map(|(sender, message_id)| {
            send(db.clone(), chaos.clone(), *sender, text(message_id))
        }))
        .await;

    let mut seq_of: BTreeMap<String, BTreeSet<i64>> = BTreeMap::new();
    for ((_, message_id), result) in sends.iter().zip(results) {
        let sent = result.unwrap_or_else(|error| panic!("{message_id}: {error:?}"));
        seq_of
            .entry(message_id.clone())
            .or_default()
            .insert(sent.entry.seq.unwrap());
    }
    assert!(
        seq_of.values().all(|seqs| seqs.len() == 1),
        "a repeat returns the first position: {seq_of:?}"
    );
    let handed_out: BTreeSet<i64> = seq_of.values().flatten().copied().collect();
    assert_eq!(handed_out, (3..=22).collect::<BTreeSet<_>>());
    for inbox in senders {
        let held: Vec<i64> = inbox_entry::Entity::find()
            .filter(inbox_entry::Column::InboxId.eq(inbox))
            .order_by_asc(inbox_entry::Column::Seq)
            .all(&db)
            .await
            .unwrap()
            .into_iter()
            .filter_map(|entry| entry.seq)
            .collect();
        assert_eq!(held, (1..=22).collect::<Vec<_>>(), "{inbox}");
    }
    for (message_id, seqs) in &seq_of {
        let copies = inbox_entry::Entity::find()
            .filter(inbox_entry::Column::MessageId.eq(message_id.as_str()))
            .all(&db)
            .await
            .unwrap();
        assert_eq!(copies.len(), 3, "{message_id}");
        assert!(copies.iter().all(|copy| seqs.contains(&copy.seq.unwrap())));
    }
    let stored = conversation::Entity::find_by_id("team")
        .one(&db)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stored.last_seq, Some(22));
    assert!(chaos.injected_conflicts() > 0, "retries ran");
    scratch.remove().await;
}

#[tokio::test]
async fn racing_replies_to_one_turn_complete_it_once() {
    let Some(scratch) = postgres().await else {
        return;
    };
    let db = scratch.db.clone();
    insert_bot(&db, "bot_1", "user-a", "DESKTOP").await;
    group(&db, "team", &[bot_principal("bot_1")]).await;
    let lease = now_millis() + chrono::Duration::seconds(60);
    running_turn(&db, "turn_1", "bot_1", "team", "host-1", lease).await;
    let claim = TurnClaim {
        id: "turn_1".into(),
        host_id: "host-1".into(),
    };
    let chaos = Arc::new(Chaos::new(None, Some(4)));
    let attempts: Vec<String> = (0..6)
        .map(|n| format!("re_{n}"))
        .chain(["re_0".to_string(), "re_0".to_string()])
        .collect();
    let results = join_all(attempts.iter().map(|message_id| {
        let (db, chaos, claim) = (db.clone(), chaos.clone(), claim.clone());
        let outgoing = Arc::new(text(message_id));
        async move {
            chaos
                .transaction(
                    &db,
                    DbDialect::Postgres,
                    None,
                    &RetryPolicy::default(),
                    move |txn| {
                        let (claim, outgoing) = (claim.clone(), outgoing.clone());
                        Box::pin(async move {
                            reply_as_bot(txn, "team", "bot_1", &claim, &outgoing, now_millis())
                                .await
                        })
                    },
                )
                .await
        }
    }))
    .await;

    let done = turn(&db, "turn_1").await;
    assert_eq!(done.status, TURN_DONE);
    let winner = done.reply_message_id.unwrap();
    for (message_id, result) in attempts.iter().zip(results) {
        match result {
            Ok(sent) => assert_eq!(sent.entry.message_id, winner, "{message_id}"),
            Err(error) => {
                assert_ne!(message_id, &winner);
                assert_eq!(error.public_code(), BOT_TURN_NOT_CLAIMED, "{error:?}");
            }
        }
    }
    let replies = inbox_entry::Entity::find()
        .filter(inbox_entry::Column::InboxId.eq("user-a"))
        .filter(inbox_entry::Column::SenderId.eq("bot_1"))
        .all(&db)
        .await
        .unwrap();
    assert_eq!(replies.len(), 1, "one reply per turn");
    scratch.remove().await;
}
