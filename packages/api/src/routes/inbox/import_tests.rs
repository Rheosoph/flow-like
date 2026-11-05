use super::{
    INBOX_MEMBER_LIMIT, INBOX_MEMBER_NOT_ALLOWED, INBOX_MESSAGE_ID_TAKEN, Reader,
    conversations::{NewConversation, start_direct, start_group},
    entries::{send_in, validate_send},
    import::{
        INBOX_CONVERSATION_TAKEN, INBOX_IMPORT_TOO_LARGE, ImportConversationRequest, ImportPlan,
        Imported, MAX_IMPORT_COPIES, MAX_IMPORT_ENTRIES, MAX_IMPORT_WRITES, check_writes,
        destination, former_bots, import_in, own_bots, validate_import,
    },
    members::Principal,
    model::SendEntryRequest,
    view::conversations_for,
};
use crate::{
    entity::{bot, conversation, conversation_member, inbox_entry},
    error::ApiError,
    routes::bots::import::LOCAL_ID_MAX_CHARS,
};
use axum::http::StatusCode;
use chrono::{DateTime, FixedOffset};
use flow_like_types::inbox::{
    ConversationEncryption, ConversationKind, InboxEntryKind, MAX_CONVERSATION_MEMBERS,
    PrincipalKind, direct_pair_key,
};
use sea_orm::{
    ActiveModelTrait,
    ActiveValue::Set,
    ColumnTrait, ConnectOptions, ConnectionTrait, Database, DatabaseConnection, EntityTrait,
    PaginatorTrait, QueryFilter, QueryOrder, Schema,
    sea_query::{Expr, Table},
};
use serde_json::{Value, json};
use std::collections::BTreeSet;

/// The conversation's id on the device: only an import key in the account.
const LOCAL: &str = "tz4a98xxat96iws9zmbrgj3a";

const INBOX_TABLES: [&str; 5] = [
    r#"CREATE TABLE public."Conversation" (
        "id" TEXT PRIMARY KEY, "kind" TEXT NOT NULL, "title" TEXT, "encryption" TEXT NOT NULL,
        "directKey" TEXT UNIQUE, "createdBy" TEXT NOT NULL, "lastActivityAt" TEXT NOT NULL,
        "createdAt" TEXT NOT NULL, "updatedAt" TEXT NOT NULL, "importKey" TEXT, "lastSeq" INTEGER,
        "image" TEXT, UNIQUE ("createdBy", "importKey"))"#,
    r#"CREATE TABLE public."ConversationMember" (
        "conversationId" TEXT NOT NULL, "principalId" TEXT NOT NULL,
        "principalKind" TEXT NOT NULL, "addedBy" TEXT, "joinedAt" TEXT NOT NULL,
        "leftAt" TEXT, "lastReadAt" TEXT,
        "status" TEXT NOT NULL DEFAULT 'ACTIVE', "limits" TEXT, "invitedBy" TEXT, "invitedAt" TEXT,
        "acceptedAt" TEXT, PRIMARY KEY ("conversationId", "principalId"))"#,
    r#"CREATE TABLE public."InboxEntry" (
        "id" TEXT PRIMARY KEY, "inboxId" TEXT NOT NULL, "conversationId" TEXT NOT NULL,
        "messageId" TEXT NOT NULL, "senderId" TEXT NOT NULL, "kind" TEXT NOT NULL,
        "envelope" TEXT NOT NULL, "createdAt" TEXT NOT NULL, "seq" INTEGER, UNIQUE ("inboxId", "messageId"),
        UNIQUE ("inboxId", "conversationId", "seq"))"#,
    r#"CREATE TABLE public."App" ("id" TEXT PRIMARY KEY, "botId" TEXT UNIQUE)"#,
    r#"CREATE TABLE public."BotTurn" (
        "id" TEXT PRIMARY KEY, "botId" TEXT NOT NULL, "ownerId" TEXT NOT NULL,
        "conversationId" TEXT NOT NULL, "triggerMessageId" TEXT NOT NULL,
        "hostKind" TEXT NOT NULL, "status" TEXT NOT NULL, "attempts" INTEGER NOT NULL,
        "hostId" TEXT, "leaseUntil" TEXT, "operationId" TEXT, "replyMessageId" TEXT,
        "errorCode" TEXT, "hops" INTEGER NOT NULL, "requesterId" TEXT,
        "audience" TEXT NOT NULL DEFAULT 'OWNER', "createdAt" TEXT NOT NULL,
        "updatedAt" TEXT NOT NULL, UNIQUE ("botId", "triggerMessageId"))"#,
];

async fn database() -> DatabaseConnection {
    let mut options = ConnectOptions::new("sqlite::memory:");
    options.max_connections(1);
    let db = Database::connect(options).await.unwrap();
    db.execute_unprepared("ATTACH DATABASE ':memory:' AS public")
        .await
        .unwrap();
    let schema = Schema::new(db.get_database_backend());
    for source in [
        schema.create_table_from_entity(bot::Entity),
        schema.create_table_from_entity(crate::entity::mutation_lock::Entity),
    ] {
        let mut table = Table::create();
        table.table(source.get_table_name().unwrap().clone());
        for column in source.get_columns() {
            table.col(column.clone());
        }
        db.execute(&table).await.unwrap();
    }
    for statement in INBOX_TABLES
        .iter()
        .chain(&super::marks::TEST_TABLES)
        .chain(&super::blocks::TEST_TABLES)
    {
        db.execute_unprepared(statement).await.unwrap();
    }
    db
}

/// Test times lie in the past, before [`now`].
fn at(seconds: i64) -> DateTime<FixedOffset> {
    DateTime::from_timestamp(1_700_000_000 + seconds, 0)
        .unwrap()
        .fixed_offset()
}

fn now() -> DateTime<FixedOffset> {
    at(10_000)
}

async fn insert_bot(db: &DatabaseConnection, id: &str, owner_id: &str, deleted: bool) {
    bot::ActiveModel {
        id: Set(id.into()),
        owner_id: Set(owner_id.into()),
        system_key: Set(None),
        preset_id: Set(None),
        name: Set(Some(id.into())),
        description: Set(None),
        instructions: Set(None),
        appearance: Set(json!({ "kind": "pip" })),
        model_policy: Set(json!({})),
        classification: Set(None),
        pii: Set(false),
        capabilities: Set(Default::default()),
        profile_id: Set(None),
        location: Set("DESKTOP".into()),
        device_id: Set(None),
        created_at: Set(at(0)),
        updated_at: Set(at(0)),
        deleted_at: Set(deleted.then(|| at(1))),
        import_key: Set(None),
        embedding_model: Set(None),
        stt_model: Set(None),
        tts_model: Set(None),
        image_model: Set(None),
        vision_model: Set(None),
        subtask_model: Set(None),
    }
    .insert(db)
    .await
    .unwrap();
}

fn bot_principal(id: &str) -> Principal {
    Principal {
        id: id.into(),
        kind: PrincipalKind::Bot,
    }
}

fn entry(message_id: &str, sender_id: &str, kind: &str, body: Value, second: i64) -> Value {
    json!({
        "message_id": message_id,
        "sender_id": sender_id,
        "kind": kind,
        "envelope": { "v": 1, "enc": "none", "body": body },
        "created_at": at(second),
    })
}

fn message(message_id: &str, sender_id: &str, second: i64) -> Value {
    entry(
        message_id,
        sender_id,
        "message",
        json!({ "text": message_id }),
        second,
    )
}

fn request(
    local_id: &str,
    kind: &str,
    members: &[&str],
    entries: Vec<Value>,
) -> ImportConversationRequest {
    serde_json::from_value(json!({
        "local_id": local_id,
        "kind": kind,
        "members": members,
        "entries": entries,
    }))
    .unwrap()
}

fn plan(caller: &str, request: ImportConversationRequest) -> ImportPlan {
    validate_import(caller, request, now()).unwrap()
}

/// What the handler runs: the members and other senders checked, then the destination and the
/// import (the locks need Postgres).
async fn import(
    db: &DatabaseConnection,
    caller: &str,
    plan: &ImportPlan,
) -> Result<Imported, ApiError> {
    own_bots(db, caller, &plan.conversation.members).await?;
    former_bots(db, caller, &plan.former).await?;
    let destination = destination(db, caller, plan).await?;
    import_in(db, caller, plan, destination, now()).await
}

fn direct(entries: Vec<Value>) -> ImportPlan {
    plan(
        "user-a",
        request(LOCAL, "direct", &["user-a", "bot_1"], entries),
    )
}

fn assert_code(error: &ApiError, status: StatusCode, code: &str) {
    assert_eq!(
        (error.status(), error.public_code()),
        (status, code),
        "{error:?}"
    );
}

fn names(error: &ApiError, id: &str) -> bool {
    error
        .public_message()
        .is_some_and(|message| message.contains(&format!("'{id}'")))
}

/// The inboxes holding a copy of the message, with the copy's time.
async fn copies(db: &DatabaseConnection, message_id: &str) -> Vec<(String, DateTime<FixedOffset>)> {
    inbox_entry::Entity::find()
        .filter(inbox_entry::Column::MessageId.eq(message_id))
        .order_by_asc(inbox_entry::Column::InboxId)
        .all(db)
        .await
        .unwrap()
        .into_iter()
        .map(|copy| (copy.inbox_id, copy.created_at))
        .collect()
}

async fn marker(
    db: &DatabaseConnection,
    conversation_id: &str,
    principal: &str,
) -> Option<DateTime<FixedOffset>> {
    conversation_member::Entity::find_by_id((conversation_id.to_string(), principal.to_string()))
        .one(db)
        .await
        .unwrap()
        .unwrap()
        .last_read_at
}

async fn stored_conversation(db: &DatabaseConnection, id: &str) -> conversation::Model {
    conversation::Entity::find_by_id(id)
        .one(db)
        .await
        .unwrap()
        .unwrap()
}

async fn unread(db: &DatabaseConnection, reader: &str, conversation_id: &str) -> u64 {
    conversations_for(db, &Reader::own(reader), Some(conversation_id))
        .await
        .unwrap()[0]
        .unread_count
}

async fn conversation_count(db: &DatabaseConnection) -> u64 {
    conversation::Entity::find().count(db).await.unwrap()
}

#[tokio::test]
async fn a_direct_conversation_moves_under_a_minted_id_with_its_history() {
    let db = database().await;
    insert_bot(&db, "bot_1", "user-a", false).await;
    let reaction = json!({ "target": "m_2", "emoji": "👍", "op": "add" });
    let history = direct(vec![
        message("m_1", "bot_1", 10),
        message("m_2", "user-a", 20),
        entry("r_1", "bot_1", "reaction", reaction, 30),
        message("m_3", "user-a", 40),
    ]);
    let imported = import(&db, "user-a", &history).await.unwrap();
    assert_eq!(
        (imported.created, imported.imported, imported.skipped),
        (true, 4, 0)
    );
    let id = imported.conversation_id;
    assert_ne!(id, LOCAL, "the server mints the id");
    assert!(
        id.len() >= 16
            && id
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit()),
        "{id}"
    );

    let stored = stored_conversation(&db, &id).await;
    assert_eq!(
        (
            stored.kind.as_str(),
            stored.encryption.as_str(),
            stored.created_by.as_str(),
            stored.import_key.as_deref()
        ),
        ("DIRECT", "PLAIN", "user-a", Some(LOCAL))
    );
    assert_eq!(stored.direct_key, Some(direct_pair_key("user-a", "bot_1")));
    assert_eq!(
        (
            stored.created_at,
            stored.last_activity_at,
            stored.updated_at
        ),
        (at(10), at(40), now()),
        "activity follows messages, not reactions"
    );
    for (message_id, second) in [("m_1", 10), ("m_2", 20), ("r_1", 30), ("m_3", 40)] {
        assert_eq!(
            copies(&db, message_id).await,
            [("bot_1".into(), at(second)), ("user-a".into(), at(second))]
        );
    }
    let member = conversation_member::Entity::find_by_id((id.clone(), "bot_1".to_string()))
        .one(&db)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        (
            member.principal_kind.as_str(),
            member.added_by.as_deref(),
            member.joined_at
        ),
        ("BOT", Some("user-a"), at(10))
    );

    assert_eq!(marker(&db, &id, "user-a").await, Some(at(40)));
    assert_eq!(
        marker(&db, &id, "bot_1").await,
        Some(at(30)),
        "the bot has read up to the newest entry it sent"
    );
    assert_eq!(unread(&db, "user-a", &id).await, 0);
    assert_eq!(
        unread(&db, "bot_1", &id).await,
        1,
        "m_3 came after the bot's reaction and waits for it"
    );
    let listed = conversations_for(&db, &Reader::own("user-a"), None)
        .await
        .unwrap();
    assert_eq!(
        listed[0]
            .last_entry
            .as_ref()
            .map(|entry| entry.message_id.as_str()),
        Some("m_3")
    );
}

#[tokio::test]
async fn importing_again_with_the_same_local_id_adds_to_the_same_conversation() {
    let db = database().await;
    insert_bot(&db, "bot_1", "user-a", false).await;
    let first = vec![message("m_1", "bot_1", 10), message("m_2", "user-a", 20)];
    let imported = import(&db, "user-a", &direct(first.clone())).await.unwrap();
    assert_eq!((imported.imported, imported.skipped), (2, 0));
    let id = imported.conversation_id;

    let mut longer = first;
    longer.push(message("m_3", "bot_1", 30));
    for (expected_imported, expected_skipped) in [(1, 2), (0, 3)] {
        let again = import(&db, "user-a", &direct(longer.clone()))
            .await
            .unwrap();
        assert_eq!(
            (
                again.conversation_id.as_str(),
                again.created,
                again.imported,
                again.skipped
            ),
            (id.as_str(), false, expected_imported, expected_skipped)
        );
    }
    let twice = vec![message("m_4", "bot_1", 40), message("m_4", "bot_1", 40)];
    let imported = import(&db, "user-a", &direct(twice)).await.unwrap();
    assert_eq!((imported.imported, imported.skipped), (1, 1));

    assert_eq!(copies(&db, "m_1").await.len(), 2);
    assert_eq!(copies(&db, "m_4").await.len(), 2);
    assert_eq!(stored_conversation(&db, &id).await.last_activity_at, at(40));
    assert_eq!(marker(&db, &id, "user-a").await, Some(at(40)));
    assert_eq!(marker(&db, &id, "bot_1").await, Some(at(40)));
    assert_eq!(conversation_count(&db).await, 1);
}

#[tokio::test]
async fn the_import_key_belongs_to_the_caller_and_a_keyed_conversation_stays_its_destination() {
    let db = database().await;
    insert_bot(&db, "bot_1", "user-a", false).await;
    insert_bot(&db, "bot_2", "user-a", false).await;
    insert_bot(&db, "bot_b", "user-b", false).await;
    let mine = import(&db, "user-a", &direct(vec![message("m_1", "bot_1", 10)]))
        .await
        .unwrap();
    let theirs = plan(
        "user-b",
        request(
            LOCAL,
            "direct",
            &["user-b", "bot_b"],
            vec![message("n_1", "bot_b", 10)],
        ),
    );
    let other = import(&db, "user-b", &theirs).await.unwrap();
    assert!(
        other.created,
        "the same local id of another person is their own key"
    );
    assert_ne!(other.conversation_id, mine.conversation_id);
    assert_eq!(
        stored_conversation(&db, &other.conversation_id)
            .await
            .import_key
            .as_deref(),
        Some(LOCAL)
    );

    let group = |entries: Vec<Value>| {
        plan(
            "user-a",
            request(
                "local-group",
                "group",
                &["user-a", "bot_1", "bot_2"],
                entries,
            ),
        )
    };
    let crew = import(&db, "user-a", &group(vec![message("g_1", "bot_1", 10)]))
        .await
        .unwrap();
    assert!(crew.created);
    assert_eq!(conversation_count(&db).await, 3);
    conversation_member::Entity::update_many()
        .col_expr(conversation_member::Column::LeftAt, Expr::value(at(15)))
        .filter(conversation_member::Column::ConversationId.eq(crew.conversation_id.as_str()))
        .exec(&db)
        .await
        .unwrap();
    let error = import(&db, "user-a", &group(vec![message("g_2", "bot_1", 20)]))
        .await
        .unwrap_err();
    assert_code(&error, StatusCode::CONFLICT, INBOX_CONVERSATION_TAKEN);
    assert_eq!(conversation_count(&db).await, 3);
}

#[tokio::test]
async fn a_direct_conversation_the_pair_already_has_takes_the_history() {
    let db = database().await;
    insert_bot(&db, "bot_1", "user-a", false).await;
    let pair = start_direct(
        &db,
        "user-a",
        &bot_principal("bot_1"),
        ConversationEncryption::Plain,
        at(5),
    )
    .await
    .unwrap();

    for (history, imported_count) in [
        (vec![message("m_1", "bot_1", 10)], 1),
        (
            vec![message("m_1", "bot_1", 10), message("m_2", "user-a", 20)],
            1,
        ),
    ] {
        let imported = import(&db, "user-a", &direct(history)).await.unwrap();
        assert_eq!(
            (
                imported.conversation_id.as_str(),
                imported.created,
                imported.imported
            ),
            (pair.as_str(), false, imported_count)
        );
    }
    assert_eq!(conversation_count(&db).await, 1);
    assert_eq!(
        stored_conversation(&db, &pair).await.last_activity_at,
        at(20)
    );
}

#[tokio::test]
async fn members_are_the_caller_and_the_caller_s_live_bots() {
    let db = database().await;
    insert_bot(&db, "bot_mine", "user-a", false).await;
    insert_bot(&db, "bot_gone", "user-a", true).await;
    insert_bot(&db, "bot_theirs", "user-b", false).await;
    assert_eq!(
        own_bots(&db, "user-a", &["bot_mine".into()]).await.unwrap(),
        [bot_principal("bot_mine")]
    );
    for (asked, refused) in [
        (vec!["bot_mine", "bot_theirs"], "bot_theirs"),
        (vec!["bot_gone"], "bot_gone"),
        (vec!["user-b"], "user-b"),
        (vec!["bot_nobody"], "bot_nobody"),
    ] {
        let asked: Vec<String> = asked.into_iter().map(str::to_string).collect();
        let error = own_bots(&db, "user-a", &asked).await.unwrap_err();
        assert_code(&error, StatusCode::FORBIDDEN, INBOX_MEMBER_NOT_ALLOWED);
        assert!(names(&error, refused), "{error:?}");
    }
    let foreign = plan(
        "user-a",
        request(
            LOCAL,
            "group",
            &["user-a", "bot_mine", "bot_theirs"],
            Vec::new(),
        ),
    );
    let error = import(&db, "user-a", &foreign).await.unwrap_err();
    assert_code(&error, StatusCode::FORBIDDEN, INBOX_MEMBER_NOT_ALLOWED);
    assert_eq!(conversation_count(&db).await, 0);

    let crowd: Vec<String> = std::iter::once("user-a".to_string())
        .chain((0..MAX_CONVERSATION_MEMBERS).map(|n| format!("bot_{n}")))
        .collect();
    let crowd: Vec<&str> = crowd.iter().map(String::as_str).collect();
    let error =
        validate_import("user-a", request(LOCAL, "group", &crowd, Vec::new()), now()).unwrap_err();
    assert_code(&error, StatusCode::CONFLICT, INBOX_MEMBER_LIMIT);
    let too_long = "x".repeat(LOCAL_ID_MAX_CHARS + 1);
    for (local_id, kind, members) in [
        (LOCAL, "direct", vec!["bot_mine"]),
        (LOCAL, "direct", vec!["user-a", "bot_mine", "bot_gone"]),
        (LOCAL, "group", vec!["user-a"]),
        ("", "direct", vec!["user-a", "bot_mine"]),
        (too_long.as_str(), "direct", vec!["user-a", "bot_mine"]),
    ] {
        let error = validate_import(
            "user-a",
            request(local_id, kind, &members, Vec::new()),
            now(),
        )
        .unwrap_err();
        assert_eq!(
            error.status(),
            StatusCode::BAD_REQUEST,
            "{local_id} {kind} {members:?}"
        );
    }
    let longest = "Y".repeat(LOCAL_ID_MAX_CHARS);
    assert!(
        validate_import(
            "user-a",
            request(&longest, "direct", &["user-a", "bot_mine"], Vec::new()),
            now()
        )
        .is_ok(),
        "any 1 to 128 characters are a key"
    );
}

#[tokio::test]
async fn a_bot_that_left_keeps_what_it_said_without_getting_a_copy() {
    let db = database().await;
    insert_bot(&db, "bot_1", "user-a", false).await;
    insert_bot(&db, "bot_left", "user-a", false).await;
    insert_bot(&db, "bot_gone", "user-a", true).await;
    insert_bot(&db, "bot_theirs", "user-b", false).await;
    let group = |entries: Vec<Value>| {
        plan(
            "user-a",
            request(LOCAL, "group", &["user-a", "bot_1"], entries),
        )
    };
    let history = group(vec![
        message("m_1", "bot_left", 10),
        message("m_2", "bot_gone", 20),
        message("m_3", "user-a", 30),
    ]);
    assert_eq!(
        history.former,
        BTreeSet::from(["bot_gone".to_string(), "bot_left".to_string()])
    );
    let imported = import(&db, "user-a", &history).await.unwrap();
    assert_eq!(imported.imported, 3);
    for (message_id, second) in [("m_1", 10), ("m_2", 20)] {
        assert_eq!(
            copies(&db, message_id).await,
            [("bot_1".into(), at(second)), ("user-a".into(), at(second))],
            "only members get copies"
        );
    }
    let again = import(&db, "user-a", &history).await.unwrap();
    assert_eq!(
        (again.imported, again.skipped),
        (0, 3),
        "a repeat finds the former bots' entries in your inbox"
    );

    for stranger in ["bot_theirs", "user-b", "bot_nobody"] {
        let message_id = format!("x_{stranger}");
        let foreign = group(vec![message(&message_id, stranger, 40)]);
        let error = import(&db, "user-a", &foreign).await.unwrap_err();
        assert_code(&error, StatusCode::FORBIDDEN, INBOX_MEMBER_NOT_ALLOWED);
        assert!(names(&error, stranger), "{error:?}");
        assert!(copies(&db, &message_id).await.is_empty());
    }
}

#[tokio::test]
async fn each_bot_has_read_up_to_the_newest_entry_it_sent_and_markers_never_move_back() {
    let db = database().await;
    for id in ["bot_1", "bot_2", "bot_3"] {
        insert_bot(&db, id, "user-a", false).await;
    }
    let members = ["user-a", "bot_1", "bot_2", "bot_3"];
    let reaction = json!({ "target": "m_2", "emoji": "👍", "op": "add" });
    let history = plan(
        "user-a",
        request(
            LOCAL,
            "group",
            &members,
            vec![
                message("m_1", "user-a", 10),
                message("b_1", "bot_1", 20),
                message("m_2", "user-a", 30),
                entry("r_1", "bot_2", "reaction", reaction, 40),
                message("m_3", "user-a", 50),
            ],
        ),
    );
    let id = import(&db, "user-a", &history)
        .await
        .unwrap()
        .conversation_id;
    let read = [
        ("user-a", Some(at(50)), 0),
        ("bot_1", Some(at(20)), 2),
        ("bot_2", Some(at(40)), 1),
        ("bot_3", None, 4),
    ];
    for (reader, expected_marker, expected_unread) in read {
        assert_eq!(marker(&db, &id, reader).await, expected_marker, "{reader}");
        assert_eq!(unread(&db, reader, &id).await, expected_unread, "{reader}");
    }

    let older = plan(
        "user-a",
        request(LOCAL, "group", &members, vec![message("b_0", "bot_1", 5)]),
    );
    assert_eq!(import(&db, "user-a", &older).await.unwrap().imported, 1);
    assert_eq!(marker(&db, &id, "bot_1").await, Some(at(20)));
    assert_eq!(marker(&db, &id, "user-a").await, Some(at(50)));
}

#[test]
fn entries_are_plain_events_from_1970_on() {
    let sealed = json!({
        "message_id": "m_sealed",
        "sender_id": "bot_1",
        "kind": "message",
        "envelope": { "v": 1, "enc": "hpke", "key_id": "k", "enc_key": "ZQ==", "ct": "Yw==" },
        "created_at": at(1),
    });
    let mut newer_version = message("m_v2", "bot_1", 1);
    newer_version["envelope"]["v"] = json!(2);
    let mut ancient = message("m_1969", "bot_1", 1);
    ancient["created_at"] = json!("1969-12-31T23:59:59.999Z");
    for (bad, refused) in [
        (sealed, "m_sealed"),
        (
            entry("m_join", "user-a", "member", json!({ "op": "join" }), 1),
            "m_join",
        ),
        (
            entry("m_react", "bot_1", "reaction", json!({ "emoji": "👍" }), 1),
            "m_react",
        ),
        (message("bad id", "user-a", 1), "bad id"),
        (newer_version, "m_v2"),
        (ancient, "m_1969"),
    ] {
        let body = request(LOCAL, "direct", &["user-a", "bot_1"], vec![bad]);
        let error = validate_import("user-a", body, now()).unwrap_err();
        assert_eq!(error.status(), StatusCode::BAD_REQUEST, "{refused}");
        assert!(names(&error, refused), "{error:?}");
    }
    let mut epoch = message("m_epoch", "bot_1", 1);
    epoch["created_at"] = json!("1970-01-01T00:00:00Z");
    assert!(
        validate_import(
            "user-a",
            request(LOCAL, "direct", &["user-a", "bot_1"], vec![epoch]),
            now()
        )
        .is_ok()
    );

    let reused = vec![message("m_1", "user-a", 1), message("m_1", "bot_1", 2)];
    let body = request(LOCAL, "direct", &["user-a", "bot_1"], reused);
    let error = validate_import("user-a", body, now()).unwrap_err();
    assert_code(&error, StatusCode::CONFLICT, INBOX_MESSAGE_ID_TAKEN);
}

#[test]
fn times_from_the_future_count_as_now_and_keep_milliseconds() {
    let mut later = message("m_later", "bot_1", 0);
    later["created_at"] = json!("2099-01-01T00:00:00Z");
    let mut precise = message("m_precise", "bot_1", 0);
    precise["created_at"] = json!("2023-11-14T22:13:20.123456+01:00");
    let checked = direct(vec![later, precise]);
    assert_eq!(checked.events[0].created_at, now());
    assert_eq!(
        checked.events[1].created_at,
        "2023-11-14T21:13:20.123Z"
            .parse::<DateTime<FixedOffset>>()
            .unwrap()
    );
    assert_eq!(checked.events[1].created_at.offset().local_minus_utc(), 0);
}

// One call's rows stay under DSQL's 3,000 per transaction: copies and the tombstones its deletions
// rewrite, member rows, bot locks, bot read markers, the import key's and the conversation's locks,
// the conversation, its positions and its activity.
const _: () = assert!(MAX_IMPORT_COPIES <= MAX_IMPORT_WRITES);
const _: () = assert!(MAX_IMPORT_WRITES + 3 * MAX_CONVERSATION_MEMBERS + 5 < 3_000);

#[tokio::test]
async fn one_call_stays_far_below_the_row_cap() {
    let db = database().await;
    let bots = ["bot_1", "bot_2", "bot_3"];
    for id in bots {
        insert_bot(&db, id, "user-a", false).await;
    }
    let members = ["user-a", "bot_1", "bot_2", "bot_3"];
    let many = |count: usize, senders: &[&str]| -> Vec<Value> {
        (0..count)
            .map(|n| message(&format!("m_{n}"), senders[n % senders.len()], n as i64))
            .collect()
    };
    let largest = MAX_IMPORT_COPIES / members.len();
    let too_many = request(LOCAL, "group", &members, many(largest + 1, &bots));
    let error = validate_import("user-a", too_many, now()).unwrap_err();
    assert_eq!(error.status(), StatusCode::BAD_REQUEST);

    let full = plan(
        "user-a",
        request(LOCAL, "group", &members, many(largest, &bots)),
    );
    let imported = import(&db, "user-a", &full).await.unwrap();
    assert_eq!(imported.imported as usize, largest);
    let written = inbox_entry::Entity::find().count(&db).await.unwrap();
    assert_eq!(written as usize, MAX_IMPORT_COPIES);

    let pair = ["user-a", "bot_1"];
    let fits = request(LOCAL, "direct", &pair, many(MAX_IMPORT_ENTRIES, &pair));
    assert!(validate_import("user-a", fits, now()).is_ok());
    let over = request(LOCAL, "direct", &pair, many(MAX_IMPORT_ENTRIES + 1, &pair));
    let error = validate_import("user-a", over, now()).unwrap_err();
    assert_eq!(error.status(), StatusCode::BAD_REQUEST);
}

#[test]
fn one_call_writes_at_most_eight_mebibytes_of_envelopes() {
    let members = ["user-a", "bot_1", "bot_2", "bot_3"];
    let heavy = |count: usize| -> Vec<Value> {
        (0..count)
            .map(|n| {
                entry(
                    &format!("h_{n}"),
                    "bot_1",
                    "message",
                    json!({ "text": "x".repeat(60_000) }),
                    n as i64,
                )
            })
            .collect()
    };
    assert!(
        validate_import(
            "user-a",
            request(LOCAL, "group", &members, heavy(30)),
            now()
        )
        .is_ok(),
        "30 × ~60 KB × 4 members stays under 8 MiB"
    );
    let error = validate_import(
        "user-a",
        request(LOCAL, "group", &members, heavy(40)),
        now(),
    )
    .unwrap_err();
    assert_code(
        &error,
        StatusCode::PAYLOAD_TOO_LARGE,
        INBOX_IMPORT_TOO_LARGE,
    );
    let pair = ["user-a", "bot_1"];
    assert!(
        validate_import("user-a", request(LOCAL, "direct", &pair, heavy(40)), now()).is_ok(),
        "the same entries fit for two members"
    );
}

#[tokio::test]
async fn a_conversation_with_other_members_or_settings_is_taken() {
    let db = database().await;
    for id in ["bot_1", "bot_2", "bot_3"] {
        insert_bot(&db, id, "user-a", false).await;
    }
    insert_bot(&db, "bot_b", "user-b", false).await;
    let group = |members: &[&str]| plan("user-a", request(LOCAL, "group", members, Vec::new()));
    import(&db, "user-a", &group(&["user-a", "bot_1", "bot_2"]))
        .await
        .unwrap();
    let error = import(&db, "user-a", &group(&["user-a", "bot_1"]))
        .await
        .unwrap_err();
    assert_code(&error, StatusCode::CONFLICT, INBOX_CONVERSATION_TAKEN);
    let error = import(&db, "user-a", &direct(Vec::new()))
        .await
        .unwrap_err();
    assert_code(&error, StatusCode::CONFLICT, INBOX_CONVERSATION_TAKEN);

    let theirs = NewConversation {
        kind: ConversationKind::Group,
        title: None,
        members: Vec::new(),
        encryption: ConversationEncryption::Plain,
    };
    let other_id = "b0000000000000000000000b";
    start_group(
        &db,
        other_id,
        "user-b",
        &theirs,
        &[bot_principal("bot_b")],
        at(1),
    )
    .await
    .unwrap();
    let named_like_theirs = plan(
        "user-a",
        request(
            other_id,
            "group",
            &["user-a", "bot_1"],
            vec![message("m_x", "bot_1", 2)],
        ),
    );
    let imported = import(&db, "user-a", &named_like_theirs).await.unwrap();
    assert!(
        imported.created && imported.conversation_id != other_id,
        "a local id is only a key, never another conversation's id"
    );
    assert_eq!(
        copies(&db, "m_x").await,
        [("bot_1".into(), at(2)), ("user-a".into(), at(2))]
    );

    start_direct(
        &db,
        "user-a",
        &bot_principal("bot_3"),
        ConversationEncryption::E2e,
        at(3),
    )
    .await
    .unwrap();
    let sealed_pair = plan(
        "user-a",
        request(
            "e0000000000000000000000e",
            "direct",
            &["user-a", "bot_3"],
            Vec::new(),
        ),
    );
    let error = import(&db, "user-a", &sealed_pair).await.unwrap_err();
    assert_code(&error, StatusCode::CONFLICT, INBOX_CONVERSATION_TAKEN);
}

#[test]
fn the_route_is_documented() {
    use utoipa::OpenApi;
    let spec = serde_json::to_value(crate::openapi::ApiDoc::openapi()).unwrap();
    for status in ["200", "400", "401", "403", "409", "413"] {
        let pointer = format!("/paths/~1inbox~1import/post/responses/{status}");
        assert!(spec.pointer(&pointer).is_some(), "{status}");
    }
    let schemas = &spec["components"]["schemas"];
    for schema in [
        "ImportConversationRequest",
        "ImportConversationEntry",
        "ImportConversationResponse",
    ] {
        assert!(schemas.get(schema).is_some(), "{schema}");
    }
    let request = &schemas["ImportConversationRequest"]["properties"];
    assert!(request.get("local_id").is_some() && request.get("id").is_none());
}

#[tokio::test]
async fn a_message_id_naming_another_event_is_taken() {
    let db = database().await;
    insert_bot(&db, "bot_1", "user-a", false).await;
    let team = NewConversation {
        kind: ConversationKind::Group,
        title: Some("team".into()),
        members: Vec::new(),
        encryption: ConversationEncryption::Plain,
    };
    start_group(
        &db,
        "team",
        "user-a",
        &team,
        &[bot_principal("bot_1")],
        at(1),
    )
    .await
    .unwrap();
    let hello = validate_send(SendEntryRequest {
        message_id: "msg_1".into(),
        kind: InboxEntryKind::Message,
        envelope: Some(json!({ "v": 1, "enc": "none", "body": { "text": "Hi" } })),
        envelopes: None,
        turn: None,
    })
    .unwrap();
    send_in(&db, "team", "user-a", &hello, at(2)).await.unwrap();

    for sender in ["user-a", "bot_1"] {
        let reused = direct(vec![message("msg_1", sender, 3)]);
        let error = import(&db, "user-a", &reused).await.unwrap_err();
        assert_code(&error, StatusCode::CONFLICT, INBOX_MESSAGE_ID_TAKEN);
        assert!(names(&error, "msg_1"), "{error:?}");
    }
    assert_eq!(copies(&db, "msg_1").await.len(), 2);
    let imported = import(&db, "user-a", &direct(vec![message("msg_2", "bot_1", 3)]))
        .await
        .unwrap();
    assert_eq!(imported.imported, 1);
}

/// The positions the caller's copy holds, with their message ids.
async fn positions(db: &DatabaseConnection, conversation_id: &str) -> Vec<(i64, String)> {
    inbox_entry::Entity::find()
        .filter(inbox_entry::Column::InboxId.eq("user-a"))
        .filter(inbox_entry::Column::ConversationId.eq(conversation_id))
        .order_by_asc(inbox_entry::Column::Seq)
        .all(db)
        .await
        .unwrap()
        .into_iter()
        .map(|entry| (entry.seq.unwrap(), entry.message_id))
        .collect()
}

fn numbered(pairs: &[(i64, &str)]) -> Vec<(i64, String)> {
    pairs
        .iter()
        .map(|(seq, id)| (*seq, id.to_string()))
        .collect()
}

#[tokio::test]
async fn imported_history_takes_positions_in_the_order_sent() {
    let db = database().await;
    insert_bot(&db, "bot_1", "user-a", false).await;
    let pair = start_direct(
        &db,
        "user-a",
        &bot_principal("bot_1"),
        ConversationEncryption::Plain,
        at(5),
    )
    .await
    .unwrap();
    let hello = validate_send(SendEntryRequest {
        message_id: "live_1".into(),
        kind: InboxEntryKind::Message,
        envelope: Some(json!({ "v": 1, "enc": "none", "body": { "text": "Hi" } })),
        envelopes: None,
        turn: None,
    })
    .unwrap();
    send_in(&db, &pair, "user-a", &hello, at(500))
        .await
        .unwrap();

    let older = vec![
        message("m_2", "user-a", 20),
        message("m_1", "bot_1", 10),
        message("m_3", "bot_1", 30),
    ];
    import(&db, "user-a", &direct(older.clone())).await.unwrap();
    let expected = numbered(&[(1, "live_1"), (2, "m_2"), (3, "m_1"), (4, "m_3")]);
    assert_eq!(
        positions(&db, &pair).await,
        expected,
        "the order sent, after what the conversation already held"
    );
    let mut longer = older;
    longer.push(message("m_4", "user-a", 40));
    import(&db, "user-a", &direct(longer)).await.unwrap();
    let bot_copy: Vec<Option<i64>> = inbox_entry::Entity::find()
        .filter(inbox_entry::Column::InboxId.eq("bot_1"))
        .order_by_asc(inbox_entry::Column::Seq)
        .all(&db)
        .await
        .unwrap()
        .into_iter()
        .map(|entry| entry.seq)
        .collect();
    assert_eq!(bot_copy, [Some(1), Some(2), Some(3), Some(4), Some(5)]);
    assert_eq!(stored_conversation(&db, &pair).await.last_seq, Some(5));
}

#[tokio::test]
async fn imported_files_come_from_the_sending_bot_s_workspace() {
    let db = database().await;
    insert_bot(&db, "bot_1", "user-a", false).await;
    db.execute_unprepared(
        r#"INSERT INTO public."App" ("id", "botId") VALUES ('app_ws1', 'bot_1')"#,
    )
    .await
    .unwrap();
    let with_file = |message_id: &str, sender: &str, app_id: &str| {
        entry(
            message_id,
            sender,
            "message",
            json!({
                "text": "See the attachment.",
                "parts": [{ "kind": "attachment", "id": "a1", "name": "report.pdf",
                            "handle": { "app_id": app_id, "path": "files/report.pdf",
                                        "bytes": 100_000, "sha256": "c".repeat(64),
                                        "mime": "application/pdf" } }],
            }),
            10,
        )
    };
    for (message_id, sender, app_id) in
        [("f_1", "user-a", "app_ws1"), ("f_2", "bot_1", "app_other")]
    {
        let error = import(
            &db,
            "user-a",
            &direct(vec![with_file(message_id, sender, app_id)]),
        )
        .await
        .unwrap_err();
        assert_code(
            &error,
            StatusCode::FORBIDDEN,
            super::INBOX_HANDLE_NOT_ALLOWED,
        );
        assert!(copies(&db, message_id).await.is_empty());
    }
    assert_eq!(
        conversation_count(&db).await,
        0,
        "nothing starts on a refusal"
    );
    let imported = import(
        &db,
        "user-a",
        &direct(vec![with_file("f_3", "bot_1", "app_ws1")]),
    )
    .await
    .unwrap();
    assert_eq!(imported.imported, 1);
    let malformed = with_file("f_4", "bot_1", "app ws1");
    let body = request(LOCAL, "direct", &["user-a", "bot_1"], vec![malformed]);
    let error = validate_import("user-a", body, now()).unwrap_err();
    assert_eq!(error.status(), StatusCode::BAD_REQUEST);
    assert!(names(&error, "f_4"), "{error:?}");
}

fn edit_of(message_id: &str, sender_id: &str, target: &str, second: i64) -> Value {
    entry(
        message_id,
        sender_id,
        "edit",
        json!({ "target": target, "text": message_id }),
        second,
    )
}

fn delete_of(message_id: &str, sender_id: &str, target: &str, second: i64) -> Value {
    entry(
        message_id,
        sender_id,
        "delete",
        json!({ "target": target }),
        second,
    )
}

async fn tombstoned(db: &DatabaseConnection, message_id: &str) -> bool {
    let held = inbox_entry::Entity::find()
        .filter(inbox_entry::Column::MessageId.eq(message_id))
        .all(db)
        .await
        .unwrap();
    !held.is_empty()
        && held.iter().all(|copy| {
            copy.envelope == json!({ "v": 1, "enc": "none", "body": { "deleted": true } })
        })
}

#[tokio::test]
async fn imported_deletions_leave_tombstones_like_live_ones() {
    let db = database().await;
    insert_bot(&db, "bot_1", "user-a", false).await;
    let first_call = vec![
        message("m_1", "bot_1", 10),
        edit_of("e_1", "bot_1", "m_1", 11),
        message("m_2", "user-a", 12),
        delete_of("d_2", "user-a", "m_2", 13),
        message("m_3", "user-a", 14),
    ];
    let imported = import(&db, "user-a", &direct(first_call.clone()))
        .await
        .unwrap();
    assert_eq!(imported.imported, 5);
    assert!(tombstoned(&db, "m_2").await, "deleted in the same call");
    for kept in ["m_1", "e_1", "m_3", "d_2"] {
        assert!(!tombstoned(&db, kept).await, "{kept}");
    }

    let mut second_call = first_call;
    second_call.push(delete_of("d_1", "bot_1", "m_1", 20));
    let imported = import(&db, "user-a", &direct(second_call)).await.unwrap();
    assert_eq!((imported.imported, imported.skipped), (1, 5));
    for gone in ["m_1", "e_1"] {
        assert!(
            tombstoned(&db, gone).await,
            "{gone} was deleted in a later call"
        );
    }
    assert!(!tombstoned(&db, "m_3").await);
}

#[test]
fn one_call_writes_and_rewrites_within_its_budget() {
    let events: Vec<_> = (0..MAX_IMPORT_WRITES / 2)
        .map(|n| {
            validate_import(
                "user-a",
                request(
                    LOCAL,
                    "direct",
                    &["user-a", "bot_1"],
                    vec![message(&format!("m_{n}"), "bot_1", 1)],
                ),
                now(),
            )
            .unwrap()
            .events
            .remove(0)
        })
        .collect();
    let fresh: Vec<_> = events.iter().collect();
    assert!(check_writes(&fresh, &[], 2).is_ok());
    let earlier = [("bot_1", "m_old", vec!["e_old".to_string()])];
    let error = check_writes(&fresh, &earlier, 2).unwrap_err();
    assert_code(
        &error,
        StatusCode::PAYLOAD_TOO_LARGE,
        INBOX_IMPORT_TOO_LARGE,
    );
    assert!(check_writes(&fresh[2..], &earlier, 2).is_ok());
}
