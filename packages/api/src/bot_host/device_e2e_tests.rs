//! A device bot in end-to-end encrypted conversations: the device publishes only its bots' keys,
//! gets an encrypted turn only once its bot can open it, reads only its members' keys and hands
//! in nothing the hub could read.

use super::{
    fixture::{assert_code, conversation, database, delete_bot, delivery, insert_bot, recipient},
    ledger::{ClaimRequest, claim_device_in, enqueue_in},
    recovery,
    worker::{CODE_MODEL, commit_reply_in},
};
use crate::{
    devices::{
        bots::checked_reply,
        bots_grant::has_finished_checkpoint,
        bots_keys::{BOT_DEVICE_E2E_KEY_MISSING, can_open_sealed, checked_key, ensure_openable},
    },
    entity::{bot, bot_turn, conversation, inbox_entry, json_types::StringList},
    middleware::jwt::{AppUser, BotUser},
    routes::inbox::{
        INBOX_ENCRYPTION_MISMATCH,
        keys::{members_of_bot_conversation, register_in},
        model::{KeysQuery, RegisterInboxKeyRequest},
        now_millis,
    },
};
use axum::http::StatusCode;
use flow_like_types::{
    bot::BotLocation,
    bot_turn::{
        BotRunCheckpoint, STORED_FAILED, STORED_RUNNING, TurnHostKind, failure_message_id,
        reply_message_id, turn_id,
    },
    device_bot::DeviceReplyRequest,
    inbox::ConversationKind,
};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, ConnectionTrait, DatabaseConnection, EntityTrait, QueryFilter,
    TransactionTrait,
};
use serde_json::{Value, json};
use std::collections::BTreeMap;

const OWNER: &str = "user-a";
const OTHER_OWNER: &str = "user-b";
const STRANGER: &str = "user-z";
const HOST: &str = "device:boot-1";
const SEALED_CHAT: &str = "c_e2e";
const PLAIN_CHAT: &str = "c_plain";

fn device_of(bot: &str) -> String {
    format!("device-of-{bot}")
}

/// `bot_a` (encryption) and `bot_b` on their own devices, `bot_c` of another owner, `bot_d` on a
/// desktop; `bot_a` has a queued turn in an encrypted and in a plain direct chat.
async fn devices() -> DatabaseConnection {
    let db = database().await;
    db.execute_unprepared(
        r#"CREATE TABLE public."InboxKey" (
            "principalId" TEXT NOT NULL, "keyId" TEXT NOT NULL, "algorithm" TEXT NOT NULL,
            "publicKey" TEXT NOT NULL, "createdAt" TEXT NOT NULL, "revokedAt" TEXT,
            PRIMARY KEY ("principalId", "keyId"))"#,
    )
    .await
    .unwrap();
    insert_bot(&db, "bot_a", OWNER, "DEVICE").await;
    insert_bot(&db, "bot_b", OWNER, "DEVICE").await;
    insert_bot(&db, "bot_c", OTHER_OWNER, "DEVICE").await;
    insert_bot(&db, "bot_d", OWNER, "DESKTOP").await;
    set_capabilities(&db, "bot_a", &["encryption"]).await;
    conversation(&db, SEALED_CHAT, "DIRECT", "E2E", &[OWNER, "bot_a"]).await;
    conversation(&db, PLAIN_CHAT, "DIRECT", "PLAIN", &[OWNER, "bot_a"]).await;
    conversation(&db, "c_other", "DIRECT", "E2E", &[OWNER, "bot_b"]).await;
    for (chat, trigger) in [(SEALED_CHAT, "m1"), (PLAIN_CHAT, "m2")] {
        let mut target = recipient("bot_a", OWNER, BotLocation::Device);
        target.conversation_id = chat.into();
        target.trigger_message_id = trigger.into();
        enqueue_in(
            &db,
            &delivery(ConversationKind::Direct, vec![target]),
            now_millis(),
        )
        .await
        .unwrap();
    }
    db
}

async fn set_capabilities(db: &DatabaseConnection, bot_id: &str, capabilities: &[&str]) {
    let mut row: bot::ActiveModel = bot::Entity::find_by_id(bot_id)
        .one(db)
        .await
        .unwrap()
        .unwrap()
        .into();
    row.capabilities = sea_orm::ActiveValue::Set(StringList(
        capabilities.iter().map(|c| c.to_string()).collect(),
    ));
    row.update(db).await.unwrap();
}

async fn claim(db: &DatabaseConnection, trigger: &str) -> bot_turn::Model {
    claim_device_in(
        db,
        &device_of("bot_a"),
        &ClaimRequest {
            turn_id: turn_id("bot_a", trigger),
            owner_id: Some(OWNER.into()),
            host_kind: TurnHostKind::Device,
            host_id: HOST.into(),
            now: now_millis(),
        },
    )
    .await
    .unwrap()
    .unwrap()
    .turn
}

async fn chat(db: &DatabaseConnection, id: &str) -> conversation::Model {
    conversation::Entity::find_by_id(id)
        .one(db)
        .await
        .unwrap()
        .unwrap()
}

async fn turn_row(db: &DatabaseConnection, id: &str) -> bot_turn::Model {
    bot_turn::Entity::find_by_id(id)
        .one(db)
        .await
        .unwrap()
        .unwrap()
}

fn key_request(bot_id: Option<&str>, key_id: &str) -> RegisterInboxKeyRequest {
    RegisterInboxKeyRequest {
        principal_id: bot_id.map(str::to_owned),
        key_id: key_id.into(),
        algorithm: "hpke-x25519-sha256-chacha20poly1305".into(),
        public_key: "cHVibGljLWtleQ".into(),
    }
}

async fn publish(db: &DatabaseConnection, bot_id: &str, key_id: &str) {
    let (bot, key) = checked_key(
        db,
        OWNER,
        &device_of(bot_id),
        &key_request(Some(bot_id), key_id),
    )
    .await
    .unwrap();
    register_in(db, &bot, &key, now_millis()).await.unwrap();
}

fn seal(key_id: &str) -> Value {
    json!({ "v": 1, "enc": "hpke", "key_id": key_id, "enc_key": "ZQ==", "ct": "Yw==" })
}

fn sealed_reply(turn: &bot_turn::Model, message_id: String) -> DeviceReplyRequest {
    DeviceReplyRequest {
        host_id: HOST.into(),
        message_id,
        envelope: Value::Null,
        envelopes: Some(BTreeMap::from([
            (OWNER.to_owned(), seal("ik_owner")),
            (turn.bot_id.clone(), seal("ik_bot")),
        ])),
        failure_code: None,
        usage: None,
    }
}

#[tokio::test]
async fn a_device_publishes_keys_only_for_the_live_bots_bound_to_it() {
    let db = devices().await;
    let own = device_of("bot_a");
    let (bot, _) = checked_key(&db, OWNER, &own, &key_request(Some("bot_a"), "ik_1"))
        .await
        .unwrap();
    assert_eq!(bot, "bot_a");
    for (owner, bot_id) in [
        (OWNER, "bot_b"),
        (OWNER, "bot_d"),
        (OWNER, "bot_c"),
        (OTHER_OWNER, "bot_a"),
        (OWNER, OWNER),
    ] {
        let refused = checked_key(&db, owner, &own, &key_request(Some(bot_id), "ik_1"))
            .await
            .unwrap_err();
        assert_eq!(refused.status(), StatusCode::NOT_FOUND, "{owner} {bot_id}");
    }
    let unnamed = checked_key(&db, OWNER, &own, &key_request(None, "ik_1"))
        .await
        .unwrap_err();
    assert_eq!(unnamed.status(), StatusCode::BAD_REQUEST);
    let malformed = checked_key(&db, OWNER, &own, &key_request(Some("bot_a"), "bad key"))
        .await
        .unwrap_err();
    assert_eq!(malformed.status(), StatusCode::BAD_REQUEST);
    delete_bot(&db, "bot_a").await;
    let deleted = checked_key(&db, OWNER, &own, &key_request(Some("bot_a"), "ik_1"))
        .await
        .unwrap_err();
    assert_eq!(deleted.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn an_encrypted_turn_reaches_the_device_only_once_its_bot_published_a_key() {
    let db = devices().await;
    let sealed = chat(&db, SEALED_CHAT).await;
    let turn = claim(&db, "m1").await;

    let refused = ensure_openable(&db, &turn, HOST, &sealed)
        .await
        .unwrap_err();
    assert_code(&refused, StatusCode::CONFLICT, BOT_DEVICE_E2E_KEY_MISSING);
    let failed = turn_row(&db, &turn.id).await;
    assert_eq!(failed.status, STORED_FAILED);
    assert_eq!(
        failed.error_code.as_deref(),
        Some(BOT_DEVICE_E2E_KEY_MISSING)
    );

    publish(&db, "bot_a", "ik_device").await;
    assert!(can_open_sealed(&db, "bot_a").await.unwrap());
    let plain_turn = claim(&db, "m2").await;
    ensure_openable(&db, &plain_turn, HOST, &chat(&db, PLAIN_CHAT).await)
        .await
        .unwrap();
    ensure_openable(&db, &plain_turn, HOST, &sealed)
        .await
        .unwrap();
    assert_eq!(turn_row(&db, &plain_turn.id).await.status, STORED_RUNNING);

    set_capabilities(&db, "bot_a", &[]).await;
    assert!(!can_open_sealed(&db, "bot_a").await.unwrap());
    set_capabilities(&db, "bot_a", &["encryption"]).await;
    db.execute_unprepared(
        r#"UPDATE public."InboxKey" SET "revokedAt" = '2026-01-01T00:00:00+00:00' WHERE "keyId" = 'ik_device'"#,
    )
    .await
    .unwrap();
    assert!(!can_open_sealed(&db, "bot_a").await.unwrap());
    publish(&db, "bot_a", "ik_next").await;
    delete_bot(&db, "bot_a").await;
    assert!(!can_open_sealed(&db, "bot_a").await.unwrap());
}

#[tokio::test]
async fn a_bot_token_reads_only_the_keys_of_its_conversations_members() {
    let db = devices().await;
    let turn = claim(&db, "m1").await;
    let token = AppUser::Bot(BotUser {
        bot_id: "bot_a".into(),
        turn_id: turn.id.clone(),
        conv_id: None,
    });
    let query = |conversation: Option<&str>, as_bot: Option<&str>| KeysQuery {
        principals: None,
        conversation_id: conversation.map(str::to_owned),
        as_bot: as_bot.map(str::to_owned),
    };
    let members = [OWNER.to_owned(), "bot_a".to_owned()];
    members_of_bot_conversation(
        &db,
        &token,
        "bot_a",
        &query(Some(SEALED_CHAT), Some("bot_a")),
        &members,
    )
    .await
    .unwrap();
    let stranger = members_of_bot_conversation(
        &db,
        &token,
        "bot_a",
        &query(Some(SEALED_CHAT), None),
        &[OWNER.to_owned(), STRANGER.to_owned()],
    )
    .await
    .unwrap_err();
    assert_eq!(stranger.status(), StatusCode::FORBIDDEN);
    let unnamed = members_of_bot_conversation(&db, &token, "bot_a", &query(None, None), &members)
        .await
        .unwrap_err();
    assert_eq!(unnamed.status(), StatusCode::BAD_REQUEST);
    let elsewhere = members_of_bot_conversation(
        &db,
        &token,
        "bot_a",
        &query(Some("c_other"), None),
        &members,
    )
    .await
    .unwrap_err();
    assert_eq!(elsewhere.status(), StatusCode::NOT_FOUND);
    let posing = members_of_bot_conversation(
        &db,
        &token,
        "bot_a",
        &query(Some(SEALED_CHAT), Some("bot_b")),
        &members,
    )
    .await
    .unwrap_err();
    assert_eq!(posing.status(), StatusCode::FORBIDDEN);
    let room_token = AppUser::Bot(BotUser {
        bot_id: "bot_a".into(),
        turn_id: turn.id.clone(),
        conv_id: Some(PLAIN_CHAT.into()),
    });
    let other_room = members_of_bot_conversation(
        &db,
        &room_token,
        "bot_a",
        &query(Some(SEALED_CHAT), None),
        &members,
    )
    .await
    .unwrap_err();
    assert_eq!(other_room.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn a_device_hands_in_an_encrypted_reply_sealed_and_never_plain() {
    let db = devices().await;
    let turn = claim(&db, "m1").await;

    let plain = DeviceReplyRequest {
        envelope: json!({ "v": 1, "enc": "none", "body": { "text": "launch code 4711" } }),
        envelopes: None,
        ..sealed_reply(&turn, reply_message_id("bot_a", "m1"))
    };
    let (request, failure) = checked_reply(&turn, plain).unwrap();
    let txn = db.begin().await.unwrap();
    let refused = commit_reply_in(&txn, &turn, HOST, request, failure)
        .await
        .unwrap_err();
    txn.rollback().await.unwrap();
    assert_code(&refused, StatusCode::BAD_REQUEST, INBOX_ENCRYPTION_MISMATCH);

    let both = DeviceReplyRequest {
        envelope: json!({ "v": 1, "enc": "none", "body": { "text": "x" } }),
        ..sealed_reply(&turn, reply_message_id("bot_a", "m1"))
    };
    assert_eq!(
        checked_reply(&turn, both).unwrap_err().status(),
        StatusCode::BAD_REQUEST
    );

    let (request, failure) =
        checked_reply(&turn, sealed_reply(&turn, reply_message_id("bot_a", "m1"))).unwrap();
    commit_reply_in(&db, &turn, HOST, request, failure)
        .await
        .unwrap();
    let copies = inbox_entry::Entity::find()
        .filter(inbox_entry::Column::MessageId.eq(reply_message_id("bot_a", "m1")))
        .all(&db)
        .await
        .unwrap();
    assert_eq!(copies.len(), 2);
    for copy in &copies {
        assert_ne!(copy.envelope["enc"], "none", "{copy:?}");
        assert!(copy.envelope.get("body").is_none(), "{copy:?}");
    }
    assert!(!format!("{copies:?}").contains("4711"));
}

#[tokio::test]
async fn a_sealed_failure_report_names_its_code_and_fails_the_turn() {
    let db = devices().await;
    let turn = claim(&db, "m1").await;
    let report = DeviceReplyRequest {
        failure_code: Some(CODE_MODEL.into()),
        ..sealed_reply(&turn, failure_message_id("bot_a", "m1"))
    };
    let (request, failure) = checked_reply(&turn, report).unwrap();
    commit_reply_in(&db, &turn, HOST, request, failure)
        .await
        .unwrap();
    let ended = turn_row(&db, &turn.id).await;
    assert_eq!(ended.status, STORED_FAILED);
    assert_eq!(ended.error_code.as_deref(), Some(CODE_MODEL));
}

#[tokio::test]
async fn a_sealed_checkpoint_round_trips_and_keeps_the_turn_alive() {
    let db = devices().await;
    let turn = claim(&db, "m1").await;
    let claim = recovery::Claim {
        turn_id: turn.id.clone(),
        owner_id: OWNER.into(),
        host_id: HOST.into(),
        attempt: u32::try_from(turn.attempts).unwrap(),
    };
    let checkpoint = |state: Value| BotRunCheckpoint {
        revision: 1,
        started_at_ms: 1_000,
        deadline_at_ms: 31_000,
        state,
    };
    let save = |saved: BotRunCheckpoint| {
        let (db, claim) = (&db, &claim);
        async move {
            let txn = db.begin().await.unwrap();
            let result = recovery::save_in(&txn, claim, &saved, now_millis()).await;
            match &result {
                Ok(()) => txn.commit().await.unwrap(),
                Err(_) => txn.rollback().await.unwrap(),
            }
            result
        }
    };
    let plain = checkpoint(json!({ "phase": "ready", "history": "launch code 4711" }));
    let refused = save(plain).await.unwrap_err();
    assert_eq!(refused.status(), StatusCode::BAD_REQUEST);

    let sealed = checkpoint(json!({ "sealed_checkpoint": 1, "envelope": seal("ik_device") }));
    save(sealed.clone()).await.unwrap();
    assert_eq!(
        recovery::load_in(&db, &claim, now_millis()).await.unwrap(),
        Some(sealed)
    );
    assert!(!has_finished_checkpoint(&db, &turn, HOST).await.unwrap());
}
