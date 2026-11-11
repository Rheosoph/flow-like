use super::{
    INBOX_DIRECT_FIXED, INBOX_ENCRYPTION_MISMATCH, INBOX_ENVELOPES_MISMATCH, INBOX_KEY_EXISTS,
    INBOX_KEY_LIMIT, INBOX_MEMBER_LIMIT, INBOX_MESSAGE_ID_TAKEN, INBOX_NOT_YOUR_MESSAGE,
    INBOX_REMOVE_NOT_ALLOWED, INBOX_TARGET_NOT_FOUND, Reader, active_membership,
    conversations::{
        NewConversation, member_group, record_room_change, start_direct, start_group,
        validate_new_conversation,
    },
    entries::{Outgoing, entries_page, mark_read_in, page_bounds, send_in, validate_send},
    image::compare_and_set,
    insert_copies,
    keys::{
        MAX_ACTIVE_KEYS, active_keys, register_in, requested_principals, revoke_in, validate_key,
    },
    members::{Principal, add_in, end_memberships, member_copies, remove_in},
    model::{
        CreateConversationRequest, EntriesQuery, InboxEntriesPage, InboxEntry,
        RegisterInboxKeyRequest, SendEntryRequest,
    },
    reader_membership,
    retirement::{
        CONVERSATIONS_PER_RETIREMENT, PURGE_PAGE, active_conversations, leave, purge_inbox,
    },
    view::conversations_for,
};
use crate::{
    entity::{bot, conversation, inbox_entry, inbox_key, membership},
    error::ApiError,
};
use axum::http::StatusCode;
use chrono::{DateTime, FixedOffset};
use flow_like_types::inbox::{
    ConversationEncryption, ConversationKind, EntryCursor, InboxEntryKind,
    MAX_CONVERSATION_MEMBERS, MemberOp, PrincipalKind, RoomEventBody,
};
use sea_orm::{
    ActiveModelTrait, ActiveValue::Set, ColumnTrait, ConnectOptions, ConnectionTrait, Database,
    DatabaseConnection, EntityTrait, PaginatorTrait, QueryFilter, QueryOrder, Schema,
    sea_query::Table,
};
use serde_json::{Value, json};
use std::collections::BTreeMap;

const INBOX_TABLES: [&str; 6] = [
    r#"CREATE TABLE public."User" (
        "id" TEXT PRIMARY KEY, "name" TEXT, "email" TEXT, "username" TEXT, "preferredUsername" TEXT)"#,
    r#"CREATE TABLE public."Conversation" (
        "id" TEXT PRIMARY KEY, "kind" TEXT NOT NULL, "title" TEXT, "encryption" TEXT NOT NULL,
        "directKey" TEXT UNIQUE, "createdBy" TEXT NOT NULL, "lastActivityAt" TEXT NOT NULL,
        "createdAt" TEXT NOT NULL, "updatedAt" TEXT NOT NULL, "importKey" TEXT, "lastSeq" INTEGER,
        "image" TEXT)"#,
    r#"CREATE TABLE public."ConversationMember" (
        "conversationId" TEXT NOT NULL, "principalId" TEXT NOT NULL,
        "principalKind" TEXT NOT NULL, "addedBy" TEXT, "joinedAt" TEXT NOT NULL,
        "leftAt" TEXT, "lastReadAt" TEXT,
        "status" TEXT NOT NULL DEFAULT 'ACTIVE', "limits" TEXT, "invitedBy" TEXT, "invitedAt" TEXT,
        "acceptedAt" TEXT, "hiddenAt" TEXT, "clearedAt" TEXT, "clearedLegacyAt" TEXT, "clearedSeq" INTEGER, PRIMARY KEY ("conversationId", "principalId"))"#,
    r#"CREATE TABLE public."InboxEntry" (
        "id" TEXT PRIMARY KEY, "inboxId" TEXT NOT NULL, "conversationId" TEXT NOT NULL,
        "messageId" TEXT NOT NULL, "senderId" TEXT NOT NULL, "kind" TEXT NOT NULL,
        "envelope" TEXT NOT NULL, "createdAt" TEXT NOT NULL, "seq" INTEGER, "cleared" BOOLEAN, UNIQUE ("inboxId", "messageId"),
        UNIQUE ("inboxId", "conversationId", "seq"))"#,
    r#"CREATE TABLE public."InboxKey" (
        "principalId" TEXT NOT NULL, "keyId" TEXT NOT NULL, "algorithm" TEXT NOT NULL,
        "publicKey" TEXT NOT NULL, "createdAt" TEXT NOT NULL, "revokedAt" TEXT,
        PRIMARY KEY ("principalId", "keyId"))"#,
    r#"CREATE TABLE public."ConversationFile" (
        "id" TEXT PRIMARY KEY, "conversationId" TEXT NOT NULL, "uploaderId" TEXT NOT NULL,
        "name" TEXT NOT NULL, "mime" TEXT NOT NULL, "bytes" INTEGER NOT NULL, "sha256" TEXT,
        "status" TEXT NOT NULL DEFAULT 'PENDING', "messageId" TEXT, "deletedAt" TEXT,
        "createdAt" TEXT NOT NULL)"#,
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
        schema.create_table_from_entity(membership::Entity),
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
        .chain([&super::fixture::BOT_TURN_TABLE])
    {
        db.execute_unprepared(statement).await.unwrap();
    }
    db
}

/// Test times lie in the past: deleting a bot stamps its leave entries with the real now.
fn at(seconds: i64) -> DateTime<FixedOffset> {
    DateTime::from_timestamp(1_700_000_000 + seconds, 0)
        .unwrap()
        .fixed_offset()
}

fn user(id: &str) -> Principal {
    Principal {
        id: id.into(),
        kind: PrincipalKind::User,
    }
}

fn bot_principal(id: &str) -> Principal {
    Principal {
        id: id.into(),
        kind: PrincipalKind::Bot,
    }
}

/// user-a reading as their bot `bot_1`.
fn as_bot_1() -> Reader {
    Reader {
        inbox: "bot_1".into(),
        owner: Some("user-a".into()),
    }
}

fn assert_code(error: &ApiError, status: StatusCode, code: &str) {
    assert_eq!(error.status(), status, "{error:?}");
    assert_eq!(error.public_code(), code, "{error:?}");
}

fn names(error: &ApiError, id: &str) -> bool {
    error
        .public_message()
        .is_some_and(|message| message.contains(&format!("'{id}'")))
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
        location: Set("SERVER".into()),
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

/// A group `id` started by `user-a` with `others`.
async fn group(
    db: &DatabaseConnection,
    id: &str,
    encryption: ConversationEncryption,
    others: &[Principal],
) {
    let plan = NewConversation {
        kind: ConversationKind::Group,
        title: Some(id.into()),
        members: Vec::new(),
        encryption,
    };
    start_group(db, id, "user-a", &plan, others, at(0))
        .await
        .unwrap();
}

/// `team`: user-a, user-b and user-a's bot_1.
async fn team(db: &DatabaseConnection) {
    let others = [user("user-b"), bot_principal("bot_1")];
    group(db, "team", ConversationEncryption::Plain, &others).await;
}

async fn direct(db: &DatabaseConnection, creator: &str, other: &Principal, second: i64) -> String {
    start_direct(
        db,
        creator,
        other,
        ConversationEncryption::Plain,
        at(second),
    )
    .await
    .unwrap()
}

fn send_request(
    message_id: &str,
    kind: InboxEntryKind,
    envelope: Option<Value>,
    envelopes: Option<BTreeMap<String, Value>>,
) -> SendEntryRequest {
    SendEntryRequest {
        message_id: message_id.into(),
        kind,
        envelope,
        envelopes,
        turn: None,
    }
}

fn plain(body: Value) -> Option<Value> {
    Some(json!({ "v": 1, "enc": "none", "body": body }))
}

fn seal(key_id: &str) -> Value {
    json!({ "v": 1, "enc": "hpke", "key_id": key_id, "enc_key": "ZQ==", "ct": "Yw==" })
}

fn outgoing(message_id: &str, kind: InboxEntryKind, body: Value) -> Outgoing {
    validate_send(send_request(message_id, kind, plain(body), None)).unwrap()
}

fn text(message_id: &str) -> Outgoing {
    outgoing(
        message_id,
        InboxEntryKind::Message,
        json!({ "text": message_id }),
    )
}

fn sealed(envelopes: &[(&str, &str)]) -> Outgoing {
    let envelopes = envelopes
        .iter()
        .map(|(principal, key)| (principal.to_string(), seal(key)))
        .collect();
    validate_send(send_request(
        "msg_sealed",
        InboxEntryKind::Message,
        None,
        Some(envelopes),
    ))
    .unwrap()
}

async fn copies(db: &DatabaseConnection, message_id: &str) -> Vec<inbox_entry::Model> {
    inbox_entry::Entity::find()
        .filter(inbox_entry::Column::MessageId.eq(message_id))
        .order_by_asc(inbox_entry::Column::InboxId)
        .all(db)
        .await
        .unwrap()
}

async fn member_bodies(
    db: &DatabaseConnection,
    inbox_id: &str,
    conversation_id: &str,
) -> Vec<Value> {
    inbox_entry::Entity::find()
        .filter(inbox_entry::Column::InboxId.eq(inbox_id))
        .filter(inbox_entry::Column::ConversationId.eq(conversation_id))
        .filter(inbox_entry::Column::Kind.eq("MEMBER"))
        .order_by_asc(inbox_entry::Column::CreatedAt)
        .order_by_asc(inbox_entry::Column::Id)
        .all(db)
        .await
        .unwrap()
        .into_iter()
        .map(|entry| entry.envelope["body"].clone())
        .collect()
}

async fn last_activity(db: &DatabaseConnection, conversation_id: &str) -> DateTime<FixedOffset> {
    conversation::Entity::find_by_id(conversation_id)
        .one(db)
        .await
        .unwrap()
        .unwrap()
        .last_activity_at
}

async fn unread(db: &DatabaseConnection, reader: &str, conversation_id: &str) -> u64 {
    conversations_for(db, &Reader::own(reader), Some(conversation_id))
        .await
        .unwrap()[0]
        .unread_count
}

fn create(
    kind: ConversationKind,
    members: &[&str],
    title: Option<&str>,
) -> CreateConversationRequest {
    CreateConversationRequest {
        kind,
        title: title.map(str::to_string),
        members: members.iter().map(|id| id.to_string()).collect(),
        encryption: None,
        bot_limits: Default::default(),
    }
}

#[test]
fn new_conversations_name_their_members_once_and_follow_the_kind_rules() {
    let members = ["me", " bot_1 ", "bot_1"];
    let direct = validate_new_conversation("me", create(ConversationKind::Direct, &members, None));
    let direct = direct.unwrap();
    assert_eq!(direct.members, ["bot_1"]);
    assert_eq!(direct.encryption, ConversationEncryption::Plain);
    let untitled =
        validate_new_conversation("me", create(ConversationKind::Group, &["a"], Some("   ")));
    assert_eq!(untitled.unwrap().title, None);

    let long = "x".repeat(121);
    for (kind, members, title) in [
        (ConversationKind::Direct, vec!["a", "b"], None),
        (ConversationKind::Direct, vec!["me"], None),
        (ConversationKind::Direct, vec!["a"], Some("Name")),
        (ConversationKind::Group, vec![], Some("Empty")),
        (ConversationKind::Group, vec!["a"], Some(long.as_str())),
    ] {
        let error = validate_new_conversation("me", create(kind, &members, title)).unwrap_err();
        assert_eq!(error.status(), StatusCode::BAD_REQUEST, "{members:?}");
    }
}

#[test]
fn group_setup_settings_cannot_be_applied_to_a_direct_chat() {
    let mut request = create(ConversationKind::Direct, &["bot_1"], None);
    request.bot_limits.insert(
        "bot_1".into(),
        flow_like_types::bot_effective::BotRoomLimits {
            share_memory: true,
            ..Default::default()
        },
    );
    assert!(validate_new_conversation("me", request).is_err());
}

#[test]
fn a_new_group_holds_at_most_fifty_members_with_its_creator() {
    let crowd: Vec<String> = (0..50).map(|n| format!("user-{n}")).collect();
    let crowd: Vec<&str> = crowd.iter().map(String::as_str).collect();
    let error =
        validate_new_conversation("me", create(ConversationKind::Group, &crowd, None)).unwrap_err();
    assert_code(&error, StatusCode::CONFLICT, INBOX_MEMBER_LIMIT);
    let fits = create(ConversationKind::Group, &crowd[..49], None);
    assert!(validate_new_conversation("me", fits).is_ok());
}

#[tokio::test]
async fn a_direct_conversation_is_one_per_pair_in_either_order() {
    let db = database().await;
    let first = direct(&db, "user-a", &bot_principal("bot_1"), 0).await;
    let again = start_direct(
        &db,
        "user-a",
        &bot_principal("bot_1"),
        ConversationEncryption::E2e,
        at(5),
    )
    .await
    .unwrap();
    let reversed = direct(&db, "bot_1", &user("user-a"), 9).await;
    assert_eq!((&first, &again), (&reversed, &reversed));
    assert_eq!(conversation::Entity::find().count(&db).await.unwrap(), 1);

    let listed = conversations_for(&db, &Reader::own("user-a"), None)
        .await
        .unwrap();
    let opened = &listed[0];
    assert_eq!(listed.len(), 1);
    assert_eq!(
        (
            opened.kind.as_str(),
            opened.encryption.as_str(),
            opened.created_by.as_str()
        ),
        ("direct", "plain", "user-a")
    );
    assert_eq!((opened.title.as_deref(), opened.unread_count), (None, 0));
    assert_eq!(opened.last_entry, None);
    let members: Vec<(&str, &str, Option<&str>)> = opened
        .members
        .iter()
        .map(|m| {
            (
                m.principal_id.as_str(),
                m.kind.as_str(),
                m.added_by.as_deref(),
            )
        })
        .collect();
    assert_eq!(
        members,
        [("bot_1", "bot", Some("user-a")), ("user-a", "user", None)]
    );
}

#[tokio::test]
async fn a_room_roster_includes_public_names_for_active_people_and_bots() {
    let db = database().await;
    insert_bot(&db, "bot_1", "user-a", false).await;
    db.execute_unprepared(r#"UPDATE public."Bot" SET "name" = 'Researcher' WHERE "id" = 'bot_1'"#)
        .await
        .unwrap();
    db.execute_unprepared(
        r#"INSERT INTO public."User" ("id", "name", "preferredUsername") VALUES
        ('user-a', 'Alex', NULL), ('user-b', NULL, 'Bea'), ('outsider', 'Outside', NULL)"#,
    )
    .await
    .unwrap();
    team(&db).await;
    let listed = conversations_for(&db, &as_bot_1(), Some("team"))
        .await
        .unwrap();
    let roster: BTreeMap<_, _> = listed[0]
        .members
        .iter()
        .map(|member| (member.principal_id.as_str(), member.name.as_deref()))
        .collect();
    assert_eq!(
        roster,
        BTreeMap::from([
            ("bot_1", Some("Researcher")),
            ("user-a", Some("Alex")),
            ("user-b", Some("Bea")),
        ])
    );
    for (system, expected) in [("flux", "Flux"), ("general", "Assistant")] {
        db.execute_unprepared(&format!(r#"UPDATE public."Bot" SET "name" = NULL, "systemKey" = '{system}' WHERE "id" = 'bot_1'"#)).await.unwrap();
        let listed = conversations_for(&db, &as_bot_1(), Some("team"))
            .await
            .unwrap();
        assert_eq!(
            listed[0]
                .members
                .iter()
                .find(|member| member.principal_id == "bot_1")
                .unwrap()
                .name
                .as_deref(),
            Some(expected)
        );
    }
    end_memberships(&db, &["team".to_owned()], &["user-b".to_owned()], at(10))
        .await
        .unwrap();
    let listed = conversations_for(&db, &as_bot_1(), Some("team"))
        .await
        .unwrap();
    assert!(
        !listed[0]
            .members
            .iter()
            .any(|member| member.principal_id == "user-b")
    );
}

#[tokio::test]
async fn bot_acknowledgement_ingress_keeps_waits_but_marked_human_input_still_queues() {
    use crate::entity::{bot_decision_wait, bot_turn};

    let db = super::fixture::database().await;
    for id in ["bot_1", "bot_2"] {
        insert_bot(&db, id, "user-a", false).await;
    }
    group(
        &db,
        "team",
        ConversationEncryption::Plain,
        &[
            user("user-b"),
            bot_principal("bot_1"),
            bot_principal("bot_2"),
        ],
    )
    .await;
    bot::ActiveModel {
        id: Set("bot_1".into()),
        model_policy: Set(json!({ "decision": "decision-bit" })),
        ..Default::default()
    }
    .update(&db)
    .await
    .unwrap();
    bot_decision_wait::ActiveModel {
        bot_id: Set("bot_1".into()),
        conversation_id: Set("team".into()),
        participant_id: Set("bot_2".into()),
        trigger_message_id: Set("pending-question".into()),
        after_seq: Set(0),
        created_at: Set(at(1)),
        expires_at: Set(at(100)),
    }
    .insert(&db)
    .await
    .unwrap();

    let marked = |message_id: &str| {
        validate_send(send_request(
            message_id,
            InboxEntryKind::Message,
            plain(json!({
                "text": "Understood.",
                "mentions": ["bot_1"],
                "system_one_acknowledgement": true
            })),
            None,
        ))
        .unwrap()
    };
    let acknowledgement = send_in(&db, "team", "bot_2", &marked("bot_ack"), at(10))
        .await
        .unwrap();
    assert!(acknowledgement.bots.is_empty());
    assert_eq!(bot_turn::Entity::find().count(&db).await.unwrap(), 0);
    let wait_key = ("bot_1".to_owned(), "team".to_owned());
    assert!(
        bot_decision_wait::Entity::find_by_id(wait_key.clone())
            .one(&db)
            .await
            .unwrap()
            .is_some()
    );
    assert!(
        copies(&db, "bot_ack")
            .await
            .iter()
            .all(|entry| entry.envelope["body"]["system_one_acknowledgement"] == true)
    );

    let human = send_in(&db, "team", "user-b", &marked("human_ack"), at(11))
        .await
        .unwrap();
    assert!(
        human
            .bots
            .iter()
            .any(|recipient| recipient.bot_id == "bot_1")
    );
    assert_eq!(
        bot_turn::Entity::find()
            .filter(bot_turn::Column::BotId.eq("bot_1"))
            .filter(bot_turn::Column::TriggerMessageId.eq("human_ack"))
            .count(&db)
            .await
            .unwrap(),
        1
    );
    assert!(
        bot_decision_wait::Entity::find_by_id(wait_key)
            .one(&db)
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn sending_copies_into_every_active_inbox_and_a_repeat_returns_the_first_copy() {
    let db = database().await;
    team(&db).await;
    assert_eq!(member_bodies(&db, "bot_1", "team").await.len(), 2);

    let hello = text("msg_1");
    let sent = send_in(&db, "team", "user-a", &hello, at(10))
        .await
        .unwrap();
    let cursor = EntryCursor::new(at(10).timestamp_micros(), sent.entry.id.as_str());
    assert_eq!(sent.entry.cursor, cursor.to_string());
    assert_eq!(
        (sent.entry.kind.as_str(), sent.entry.sender_id.as_str()),
        ("message", "user-a")
    );
    assert_eq!(sent.wake, ["user-a", "user-b"]);

    let fanned = copies(&db, "msg_1").await;
    let inboxes: Vec<&str> = fanned.iter().map(|copy| copy.inbox_id.as_str()).collect();
    assert_eq!(inboxes, ["bot_1", "user-a", "user-b"]);
    assert!(fanned.iter().all(|copy| copy.created_at == at(10)
        && copy.kind == "MESSAGE"
        && copy.envelope["body"]["text"] == "msg_1"));
    let own = fanned
        .iter()
        .find(|copy| copy.inbox_id == "user-a")
        .unwrap();
    assert_eq!(InboxEntry::from(own.clone()), sent.entry);
    assert_eq!(last_activity(&db, "team").await, at(10));

    let repeat = send_in(&db, "team", "user-a", &hello, at(20))
        .await
        .unwrap();
    assert_eq!(repeat.entry, sent.entry);
    assert!(repeat.wake.is_empty());
    assert_eq!(copies(&db, "msg_1").await.len(), 3);
    assert_eq!(last_activity(&db, "team").await, at(10));
}

#[tokio::test]
async fn reactions_act_on_any_message_but_edits_and_deletions_only_on_your_own() {
    let db = database().await;
    team(&db).await;
    send_in(&db, "team", "user-a", &text("msg_1"), at(10))
        .await
        .unwrap();
    let react = |id: &str, target: &str| {
        let body = json!({ "target": target, "emoji": "👍", "op": "add" });
        outgoing(id, InboxEntryKind::Reaction, body)
    };

    send_in(&db, "team", "user-b", &react("r_1", "msg_1"), at(30))
        .await
        .unwrap();
    assert_eq!(
        last_activity(&db, "team").await,
        at(10),
        "only messages move activity"
    );
    let missing = send_in(&db, "team", "user-b", &react("r_2", "msg_missing"), at(31)).await;
    assert_code(
        &missing.unwrap_err(),
        StatusCode::UNPROCESSABLE_ENTITY,
        INBOX_TARGET_NOT_FOUND,
    );
    let on_reaction = send_in(&db, "team", "user-b", &react("r_3", "r_1"), at(31)).await;
    assert_code(
        &on_reaction.unwrap_err(),
        StatusCode::UNPROCESSABLE_ENTITY,
        INBOX_TARGET_NOT_FOUND,
    );

    let edit = outgoing(
        "e_1",
        InboxEntryKind::Edit,
        json!({ "target": "msg_1", "text": "mine" }),
    );
    let foreign = send_in(&db, "team", "user-b", &edit, at(32))
        .await
        .unwrap_err();
    assert_code(&foreign, StatusCode::FORBIDDEN, INBOX_NOT_YOUR_MESSAGE);
    send_in(&db, "team", "user-a", &edit, at(33)).await.unwrap();
    let delete = outgoing("d_1", InboxEntryKind::Delete, json!({ "target": "msg_1" }));
    send_in(&db, "team", "user-a", &delete, at(34))
        .await
        .unwrap();
}

#[tokio::test]
async fn a_message_id_names_one_event_and_only_members_send() {
    let db = database().await;
    team(&db).await;
    let hello = text("msg_1");
    send_in(&db, "team", "user-a", &hello, at(10))
        .await
        .unwrap();

    let other = direct(&db, "user-a", &user("user-b"), 40).await;
    let reused = send_in(&db, &other, "user-a", &hello, at(41))
        .await
        .unwrap_err();
    assert_code(&reused, StatusCode::CONFLICT, INBOX_MESSAGE_ID_TAKEN);
    let collides = send_in(&db, &other, "user-b", &hello, at(42))
        .await
        .unwrap_err();
    assert_code(&collides, StatusCode::CONFLICT, INBOX_MESSAGE_ID_TAKEN);

    let outsider = send_in(&db, "team", "user-c", &text("msg_6"), at(50)).await;
    assert_eq!(outsider.unwrap_err().status(), StatusCode::NOT_FOUND);
    let wrong_form = send_in(&db, "team", "user-a", &sealed(&[("user-a", "k")]), at(51)).await;
    assert_eq!(wrong_form.unwrap_err().status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn end_to_end_sends_need_exactly_one_sealed_envelope_per_active_member() {
    let db = database().await;
    group(
        &db,
        "secret",
        ConversationEncryption::E2e,
        &[user("user-b")],
    )
    .await;

    let missing = send_in(
        &db,
        "secret",
        "user-a",
        &sealed(&[("user-a", "key-a")]),
        at(1),
    )
    .await
    .unwrap_err();
    assert_code(&missing, StatusCode::CONFLICT, INBOX_ENVELOPES_MISMATCH);
    assert!(names(&missing, "user-b"), "{missing:?}");
    let stranger = sealed(&[
        ("user-a", "key-a"),
        ("user-b", "key-b"),
        ("user-c", "key-c"),
    ]);
    let error = send_in(&db, "secret", "user-a", &stranger, at(2))
        .await
        .unwrap_err();
    assert_code(&error, StatusCode::CONFLICT, INBOX_ENVELOPES_MISMATCH);
    assert!(names(&error, "user-c"), "{error:?}");
    let refused = send_in(&db, "secret", "user-a", &text("msg_plain"), at(3))
        .await
        .unwrap_err();
    assert_code(&refused, StatusCode::BAD_REQUEST, INBOX_ENCRYPTION_MISMATCH);
}

#[tokio::test]
async fn every_member_of_an_end_to_end_conversation_gets_its_own_envelope() {
    let db = database().await;
    group(
        &db,
        "secret",
        ConversationEncryption::E2e,
        &[user("user-b")],
    )
    .await;
    let both = sealed(&[("user-a", "key-a"), ("user-b", "key-b")]);
    send_in(&db, "secret", "user-a", &both, at(4))
        .await
        .unwrap();

    let keys: Vec<(String, Value)> = copies(&db, "msg_sealed")
        .await
        .into_iter()
        .map(|copy| (copy.inbox_id, copy.envelope["key_id"].clone()))
        .collect();
    assert_eq!(
        keys,
        [
            ("user-a".into(), json!("key-a")),
            ("user-b".into(), json!("key-b"))
        ]
    );
    assert_eq!(
        member_bodies(&db, "user-b", "secret").await,
        [json!({ "op": "join", "principal_id": "user-b", "by": "user-a" })],
        "membership stays plain metadata"
    );
}

fn malformed_sends() -> Vec<SendEntryRequest> {
    use InboxEntryKind::{Member, Message, Reaction};
    let hi = || plain(json!({ "text": "Hi" }));
    let one = |envelope: Value| Some(BTreeMap::from([("a".to_string(), envelope)]));
    vec![
        send_request("bad id", Message, hi(), None),
        send_request("msg_1", Member, hi(), None),
        send_request("msg_1", Message, None, None),
        send_request("msg_1", Message, hi(), one(seal("k"))),
        send_request("msg_1", Message, Some(seal("k")), None),
        send_request("msg_1", Message, None, one(hi().unwrap())),
        send_request("msg_1", Message, None, Some(BTreeMap::new())),
        send_request("msg_1", Reaction, plain(json!({ "emoji": "👍" })), None),
        send_request(
            "msg_1",
            Message,
            Some(json!({ "v": 2, "enc": "none", "body": {} })),
            None,
        ),
    ]
}

#[test]
fn sends_are_checked_before_any_database_work() {
    for request in malformed_sends() {
        let error = validate_send(request.clone()).unwrap_err();
        assert_eq!(error.status(), StatusCode::BAD_REQUEST, "{request:?}");
    }
    let envelopes = Some(BTreeMap::from([("a".to_string(), seal("k"))]));
    let sealed_reaction = send_request("msg_1", InboxEntryKind::Reaction, None, envelopes);
    assert!(
        validate_send(sealed_reaction).is_ok(),
        "the server can't see a sealed target"
    );
}

#[tokio::test]
async fn the_list_shows_unread_counts_and_newest_messages_newest_activity_first() {
    let db = database().await;
    team(&db).await;
    let pair = direct(&db, "user-a", &user("user-c"), 1).await;
    send_in(&db, "team", "user-b", &text("b_1"), at(10))
        .await
        .unwrap();
    send_in(&db, "team", "user-b", &text("b_2"), at(11))
        .await
        .unwrap();
    send_in(&db, "team", "user-a", &text("a_1"), at(12))
        .await
        .unwrap();
    send_in(&db, &pair, "user-c", &text("c_1"), at(13))
        .await
        .unwrap();
    let body = json!({ "target": "a_1", "emoji": "🎉", "op": "add" });
    let react = outgoing("b_react", InboxEntryKind::Reaction, body);
    send_in(&db, "team", "user-b", &react, at(14))
        .await
        .unwrap();

    let listed = conversations_for(&db, &Reader::own("user-a"), None)
        .await
        .unwrap();
    let summary: Vec<(&str, u64, Option<&str>)> = listed
        .iter()
        .map(|c| {
            let last = c.last_entry.as_ref().map(|entry| entry.message_id.as_str());
            (c.id.as_str(), c.unread_count, last)
        })
        .collect();
    assert_eq!(
        summary,
        [(pair.as_str(), 1, Some("c_1")), ("team", 2, Some("a_1"))]
    );
    assert_eq!(
        (listed[1].members.len(), listed[1].last_activity_at),
        (3, at(12))
    );

    let as_bot = conversations_for(&db, &as_bot_1(), None).await.unwrap();
    assert_eq!((as_bot.len(), as_bot[0].unread_count), (1, 3));
    assert!(
        conversations_for(&db, &Reader::own("user-c"), Some("team"))
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn read_markers_move_forward_only() {
    let db = database().await;
    team(&db).await;
    send_in(&db, "team", "user-b", &text("b_1"), at(10))
        .await
        .unwrap();
    send_in(&db, "team", "user-b", &text("b_2"), at(11))
        .await
        .unwrap();
    assert_eq!(unread(&db, "user-a", "team").await, 2);

    let first = copy_cursor(&db, "user-a", "team", "b_1").await;
    let up_to = EntryCursor::parse(&first).unwrap();
    let marked = mark_read_in(&db, "team", "user-a", Some(&up_to), at(100))
        .await
        .unwrap();
    assert_eq!(marked, (at(10), true));
    assert_eq!(unread(&db, "user-a", "team").await, 1);
    let marked = mark_read_in(&db, "team", "user-a", None, at(100))
        .await
        .unwrap();
    assert_eq!(marked, (at(100), true));
    assert_eq!(unread(&db, "user-a", "team").await, 0);
    let stale = mark_read_in(&db, "team", "user-a", Some(&up_to), at(200))
        .await
        .unwrap();
    assert_eq!(stale, (at(100), false), "a marker never moves back");

    let foreign = EntryCursor::new(at(13).timestamp_micros(), "missing");
    let error = mark_read_in(&db, "team", "user-a", Some(&foreign), at(300)).await;
    assert_eq!(error.unwrap_err().status(), StatusCode::NOT_FOUND);
    let outsider = mark_read_in(&db, "team", "user-z", None, at(300)).await;
    assert_eq!(outsider.unwrap_err().status(), StatusCode::NOT_FOUND);
}

/// Five messages of user-b in `team`, after the join entry; returns their cursors, oldest first.
async fn five_messages(db: &DatabaseConnection) -> Vec<String> {
    group(db, "team", ConversationEncryption::Plain, &[user("user-b")]).await;
    let mut cursors = Vec::new();
    for second in 1..=5 {
        let message_id = format!("m_{second}");
        send_in(db, "team", "user-b", &text(&message_id), at(second * 10))
            .await
            .unwrap();
        cursors.push(copy_cursor(db, "user-a", "team", &message_id).await);
    }
    cursors
}

/// Every member holds its own copy of an event, so cursors are only meaningful in the reader's inbox.
async fn copy_cursor(
    db: &DatabaseConnection,
    reader: &str,
    conversation: &str,
    message_id: &str,
) -> String {
    let query = EntriesQuery {
        as_bot: None,
        before: None,
        after: None,
        limit: Some(100),
        ..EntriesQuery::default()
    };
    let bounds = page_bounds(&query).unwrap();
    entries_page(db, reader, conversation, &bounds)
        .await
        .unwrap()
        .entries
        .into_iter()
        .find(|entry| entry.message_id == message_id)
        .map(|entry| entry.cursor)
        .unwrap_or_else(|| panic!("{reader} holds no copy of {message_id} in {conversation}"))
}

async fn page(
    db: &DatabaseConnection,
    before: Option<&String>,
    after: Option<&String>,
    limit: u64,
) -> (Vec<String>, Option<String>) {
    let query = EntriesQuery {
        as_bot: None,
        before: before.cloned(),
        after: after.cloned(),
        limit: Some(limit),
        ..EntriesQuery::default()
    };
    let bounds = page_bounds(&query).unwrap();
    let InboxEntriesPage {
        entries,
        next_before,
        ..
    } = entries_page(db, "user-a", "team", &bounds).await.unwrap();
    let ids = entries
        .into_iter()
        .filter(|entry| entry.kind == "message")
        .map(|entry| entry.message_id)
        .collect();
    (ids, next_before)
}

fn ids(names: &[&str]) -> Vec<String> {
    names.iter().map(|name| name.to_string()).collect()
}

#[tokio::test]
async fn recovery_message_lookup_is_scoped_and_honors_cleared_history() {
    let db = database().await;
    five_messages(&db).await;
    let latest = page(&db, None, None, 2).await.0;
    assert!(!latest.contains(&"m_1".to_owned()));
    let read = super::entries::entry_page_by_message(&db, "user-a", "team", "m_1")
        .await
        .unwrap();
    assert_eq!(read.entries.len(), 1);
    assert_eq!(read.entries[0].message_id, "m_1");
    assert!(read.next_before.is_none());
    for (reader, room) in [("user-z", "team"), ("user-a", "another-room")] {
        assert!(
            super::entries::entry_page_by_message(&db, reader, room, "m_1")
                .await
                .unwrap()
                .entries
                .is_empty()
        );
    }
    inbox_entry::Entity::update_many()
        .col_expr(
            inbox_entry::Column::Cleared,
            sea_orm::sea_query::Expr::value(true),
        )
        .filter(inbox_entry::Column::InboxId.eq("user-a"))
        .filter(inbox_entry::Column::MessageId.eq("m_1"))
        .exec(&db)
        .await
        .unwrap();
    assert!(
        super::entries::entry_page_by_message(&db, "user-a", "team", "m_1")
            .await
            .unwrap()
            .entries
            .is_empty()
    );
    assert_eq!(
        super::entries::entry_page_by_message(&db, "user-b", "team", "m_1")
            .await
            .unwrap()
            .entries
            .len(),
        1
    );
}

#[test]
fn recovery_message_lookup_rejects_ambiguous_selectors() {
    let query = EntriesQuery {
        message_id: Some("original".into()),
        ..Default::default()
    };
    assert_eq!(
        super::entries::exact_message(&query).unwrap(),
        Some("original")
    );
    for invalid in [
        EntriesQuery {
            message_id: Some("bad/id".into()),
            ..query.clone()
        },
        EntriesQuery {
            before: Some("cursor".into()),
            ..query.clone()
        },
        EntriesQuery {
            after: Some("cursor".into()),
            ..query.clone()
        },
        EntriesQuery {
            from_seq: Some(1),
            ..query.clone()
        },
        EntriesQuery {
            around_seq: Some(1),
            ..query.clone()
        },
    ] {
        assert!(super::entries::exact_message(&invalid).is_err());
    }
}

#[tokio::test]
async fn pages_walk_back_with_before() {
    let db = database().await;
    let c = five_messages(&db).await;
    assert_eq!(
        page(&db, None, None, 2).await,
        (ids(&["m_5", "m_4"]), Some(c[3].clone()))
    );
    assert_eq!(
        page(&db, Some(&c[3]), None, 2).await,
        (ids(&["m_3", "m_2"]), Some(c[1].clone()))
    );
    let (oldest, next) = page(&db, Some(&c[1]), None, 1).await;
    assert_eq!(oldest, ids(&["m_1"]));
    assert!(next.is_some(), "the join entry is older still");
    assert_eq!(page(&db, Some(&c[1]), None, 2).await, (ids(&["m_1"]), None));
}

#[tokio::test]
async fn polls_with_after_say_when_entries_remain_in_between() {
    let db = database().await;
    let c = five_messages(&db).await;
    assert_eq!(
        page(&db, None, Some(&c[0]), 2).await,
        (ids(&["m_5", "m_4"]), Some(c[3].clone())),
        "entries between `after` and the page stay to be loaded"
    );
    assert_eq!(
        page(&db, None, Some(&c[2]), 2).await,
        (ids(&["m_5", "m_4"]), None)
    );
    assert_eq!(
        page(&db, Some(&c[4]), Some(&c[1]), 5).await,
        (ids(&["m_4", "m_3"]), None)
    );
    assert_eq!(page(&db, None, Some(&c[4]), 5).await, (Vec::new(), None));
}

#[tokio::test]
async fn entries_of_the_same_millisecond_order_by_id() {
    let db = database().await;
    group(
        &db,
        "team",
        ConversationEncryption::Plain,
        &[user("user-b")],
    )
    .await;
    let mut sent = Vec::new();
    for id in ["same_1", "same_2", "same_3"] {
        let entry = send_in(&db, "team", "user-a", &text(id), at(5))
            .await
            .unwrap()
            .entry;
        sent.push(EntryCursor::parse(&entry.cursor).unwrap());
    }
    sent.sort_by(|a, b| b.cmp(a));

    let mut walked = Vec::new();
    let mut before: Option<String> = None;
    for _ in 0..3 {
        let query = EntriesQuery {
            as_bot: None,
            before: before.clone(),
            after: None,
            limit: Some(1),
            ..EntriesQuery::default()
        };
        let bounds = page_bounds(&query).unwrap();
        let entry = entries_page(&db, "user-a", "team", &bounds)
            .await
            .unwrap()
            .entries[0]
            .clone();
        walked.push(EntryCursor::parse(&entry.cursor).unwrap());
        before = Some(entry.cursor);
    }
    assert_eq!(walked, sent);
}

#[test]
fn page_bounds_clamp_the_limit_and_refuse_malformed_cursors() {
    let query = |limit: Option<u64>, before: Option<&str>| EntriesQuery {
        as_bot: None,
        before: before.map(str::to_string),
        after: None,
        limit,
        ..EntriesQuery::default()
    };
    assert_eq!(page_bounds(&query(None, None)).unwrap().limit, 50);
    assert_eq!(page_bounds(&query(Some(0), None)).unwrap().limit, 1);
    assert_eq!(page_bounds(&query(Some(500), None)).unwrap().limit, 100);
    assert_eq!(page_bounds(&query(None, Some(" "))).unwrap().before, None);
    let bounds = page_bounds(&query(None, Some("1700000000000000:abc"))).unwrap();
    let position = bounds.before.unwrap();
    assert_eq!((position.at, position.entry_id.as_str()), (at(0), "abc"));
    for bad in ["abc", "12:", "9999999999999999999:abc"] {
        let error = page_bounds(&query(None, Some(bad))).unwrap_err();
        assert_eq!(error.status(), StatusCode::BAD_REQUEST, "{bad}");
        assert!(error.public_message().unwrap().contains("before"), "{bad}");
    }
}

#[tokio::test]
async fn any_member_adds_and_everyone_gets_a_join_entry() {
    let db = database().await;
    group(
        &db,
        "team",
        ConversationEncryption::Plain,
        &[user("user-b")],
    )
    .await;
    let joining = [user("user-c"), bot_principal("bot_b")];
    let woken = add_in(&db, "team", "user-b", &joining, at(10))
        .await
        .unwrap();
    assert_eq!(woken, ["user-a", "user-b", "user-c"]);

    let mut joins = member_bodies(&db, "user-c", "team").await;
    joins.sort_by_key(|body| body["principal_id"].to_string());
    assert_eq!(
        joins,
        [
            json!({ "op": "join", "principal_id": "bot_b", "by": "user-b" }),
            json!({ "op": "join", "principal_id": "user-c", "by": "user-b" }),
        ]
    );
    let again = add_in(&db, "team", "user-a", &[user("user-c")], at(11)).await;
    assert!(again.unwrap().is_empty(), "current members are skipped");
    let outsider = add_in(&db, "team", "user-z", &[user("user-y")], at(12)).await;
    assert_eq!(outsider.unwrap_err().status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn members_leave_themselves_owners_remove_bots_and_creators_anyone() {
    let db = database().await;
    insert_bot(&db, "bot_b", "user-b", false).await;
    group(
        &db,
        "team",
        ConversationEncryption::Plain,
        &[user("user-b")],
    )
    .await;
    let joining = [user("user-c"), bot_principal("bot_b")];
    add_in(&db, "team", "user-b", &joining, at(10))
        .await
        .unwrap();

    let refused = remove_in(&db, "team", "user-b", "user-c", at(20))
        .await
        .unwrap_err();
    assert_code(&refused, StatusCode::FORBIDDEN, INBOX_REMOVE_NOT_ALLOWED);
    remove_in(&db, "team", "user-b", "bot_b", at(21))
        .await
        .unwrap();
    let woken = remove_in(&db, "team", "user-a", "user-c", at(22))
        .await
        .unwrap();
    assert_eq!(woken, ["user-a", "user-b", "user-c"]);
    let leave = json!({ "op": "leave", "principal_id": "user-c", "by": "user-a" });
    for inbox in ["user-b", "user-c"] {
        assert_eq!(
            member_bodies(&db, inbox, "team").await.last(),
            Some(&leave),
            "{inbox}"
        );
    }
    assert!(active_membership(&db, "team", "user-c").await.is_err());
    let gone = remove_in(&db, "team", "user-a", "user-c", at(23))
        .await
        .unwrap_err();
    assert_eq!(gone.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn a_member_who_left_reads_nothing_until_added_back() {
    let db = database().await;
    group(
        &db,
        "team",
        ConversationEncryption::Plain,
        &[user("user-b"), user("user-c")],
    )
    .await;
    remove_in(&db, "team", "user-c", "user-c", at(10))
        .await
        .unwrap();
    let listed = conversations_for(&db, &Reader::own("user-c"), None)
        .await
        .unwrap();
    assert!(listed.iter().all(|conversation| conversation.id != "team"));

    add_in(&db, "team", "user-b", &[user("user-c")], at(30))
        .await
        .unwrap();
    let back = active_membership(&db, "team", "user-c").await.unwrap();
    assert_eq!(
        (back.joined_at, back.added_by.as_deref()),
        (at(30), Some("user-b"))
    );
    assert_eq!(
        conversations_for(&db, &Reader::own("user-c"), None)
            .await
            .unwrap()
            .len(),
        1
    );
}

#[tokio::test]
async fn a_group_picture_changes_only_from_the_one_that_was_read() {
    let db = database().await;
    group(
        &db,
        "pictured",
        ConversationEncryption::Plain,
        &[user("user-b")],
    )
    .await;
    assert!(member_group(&db, "pictured", "user-b").await.is_ok());

    assert!(
        compare_and_set(&db, "pictured", None, Some("aa"), at(1))
            .await
            .unwrap()
    );
    assert!(
        !compare_and_set(&db, "pictured", None, Some("bb"), at(2))
            .await
            .unwrap(),
        "a writer that read no picture loses to the one that set it"
    );
    let seen = conversations_for(&db, &Reader::own("user-b"), Some("pictured"))
        .await
        .unwrap();
    assert_eq!(
        (seen[0].image.as_deref(), seen[0].image_url.as_deref()),
        (Some("aa"), None)
    );
    assert!(
        compare_and_set(&db, "pictured", Some("aa"), None, at(3))
            .await
            .unwrap()
    );
    let cleared = conversations_for(&db, &Reader::own("user-a"), Some("pictured"))
        .await
        .unwrap();
    assert_eq!(cleared[0].image, None);

    let pair = direct(&db, "user-a", &user("user-d"), 40).await;
    let fixed = member_group(&db, &pair, "user-a").await.unwrap_err();
    assert_code(&fixed, StatusCode::CONFLICT, INBOX_DIRECT_FIXED);
    assert_eq!(
        member_group(&db, "pictured", "user-z")
            .await
            .unwrap_err()
            .status(),
        StatusCode::NOT_FOUND
    );
}

#[tokio::test]
async fn a_room_change_lands_in_every_members_history_at_the_next_position() {
    let db = database().await;
    group(&db, "named", ConversationEncryption::E2e, &[user("user-b")]).await;
    let before = last_activity(&db, "named").await;
    record_room_change(
        &db,
        "named",
        &RoomEventBody::title(Some("Launch crew".into()), "user-b"),
        at(5),
    )
    .await
    .unwrap();
    let written = inbox_entry::Entity::find()
        .filter(inbox_entry::Column::Kind.eq("ROOM"))
        .order_by_asc(inbox_entry::Column::InboxId)
        .all(&db)
        .await
        .unwrap();
    assert_eq!(
        written
            .iter()
            .map(|entry| entry.inbox_id.as_str())
            .collect::<Vec<_>>(),
        ["user-a", "user-b"]
    );
    assert!(written[0].message_id.starts_with("room:"));
    assert_eq!(written[0].message_id, written[1].message_id);
    assert_eq!(written[0].sender_id, "user-b");
    assert_eq!(written[0].seq, Some(2), "after the one join");
    assert_eq!(
        written[0].envelope,
        json!({
            "v": 1,
            "enc": "none",
            "body": { "change": "title", "title": "Launch crew", "by": "user-b" },
        }),
        "plain even in an end-to-end encrypted room"
    );
    assert_eq!(last_activity(&db, "named").await, before);
    let listed = conversations_for(&db, &Reader::own("user-a"), Some("named"))
        .await
        .unwrap();
    assert_eq!(
        (listed[0].unread_count, listed[0].last_entry.as_ref()),
        (0, None)
    );
}

#[tokio::test]
async fn direct_conversations_keep_their_two_members() {
    let db = database().await;
    let pair = direct(&db, "user-a", &user("user-d"), 40).await;
    let fixed = add_in(&db, &pair, "user-a", &[user("user-e")], at(41))
        .await
        .unwrap_err();
    assert_code(&fixed, StatusCode::CONFLICT, INBOX_DIRECT_FIXED);
    let fixed = remove_in(&db, &pair, "user-a", "user-d", at(42))
        .await
        .unwrap_err();
    assert_code(&fixed, StatusCode::CONFLICT, INBOX_DIRECT_FIXED);
}

#[tokio::test]
async fn a_conversation_never_grows_past_fifty_members() {
    let db = database().await;
    let others: Vec<Principal> = (0..49).map(|n| user(&format!("user-{n:02}"))).collect();
    group(&db, "crowd", ConversationEncryption::Plain, &others).await;
    let joins = inbox_entry::Entity::find()
        .filter(inbox_entry::Column::InboxId.eq("user-a"))
        .count(&db)
        .await
        .unwrap();
    assert_eq!(joins, 49);
    let error = add_in(&db, "crowd", "user-a", &[user("user-z")], at(1))
        .await
        .unwrap_err();
    assert_code(&error, StatusCode::CONFLICT, INBOX_MEMBER_LIMIT);
    remove_in(&db, "crowd", "user-00", "user-00", at(2))
        .await
        .unwrap();
    add_in(&db, "crowd", "user-a", &[user("user-z")], at(3))
        .await
        .unwrap();
}

#[tokio::test]
async fn a_deleted_bot_leaves_every_conversation_with_a_leave_entry() {
    let db = database().await;
    let others = [user("user-b"), bot_principal("bot_1")];
    group(&db, "team", ConversationEncryption::E2e, &others).await;
    let pair = direct(&db, "user-a", &bot_principal("bot_1"), 1).await;

    let mut candidates = active_conversations(&db, "bot_1", 10).await.unwrap();
    let mut in_id_order = vec!["team".to_string(), pair.clone()];
    in_id_order.sort();
    assert_eq!(candidates, in_id_order);
    let woken = leave(&db, "user-a", "bot_1", &candidates, at(5))
        .await
        .unwrap();
    assert_eq!(woken.into_iter().collect::<Vec<_>>(), ["user-a", "user-b"]);
    assert!(
        active_conversations(&db, "bot_1", 10)
            .await
            .unwrap()
            .is_empty()
    );

    let left = json!({ "op": "leave", "principal_id": "bot_1", "by": "user-a" });
    for (inbox, conversation_id) in [
        ("user-a", "team"),
        ("user-b", "team"),
        ("user-a", pair.as_str()),
    ] {
        let last = member_bodies(&db, inbox, conversation_id).await.pop();
        assert_eq!(last, Some(left.clone()), "{inbox} in {conversation_id}");
    }
    assert!(
        !member_bodies(&db, "bot_1", "team").await.contains(&left),
        "the bot's own inbox is purged next, so it gets no copy"
    );
    candidates.push("elsewhere".into());
    let again = leave(&db, "user-a", "bot_1", &candidates, at(6))
        .await
        .unwrap();
    assert!(again.is_empty(), "leaving twice writes nothing");
    assert_eq!(
        member_bodies(&db, "user-b", "team").await.last(),
        Some(&left)
    );
}

#[tokio::test]
async fn retirement_batches_are_bounded_and_go_in_id_order() {
    assert!(CONVERSATIONS_PER_RETIREMENT * (MAX_CONVERSATION_MEMBERS + 2) <= 2_000);
    let db = database().await;
    for id in ["team-c", "team-a", "team-b"] {
        group(
            &db,
            id,
            ConversationEncryption::Plain,
            &[bot_principal("bot_1")],
        )
        .await;
    }
    let first = active_conversations(&db, "bot_1", 2).await.unwrap();
    assert_eq!(first, ["team-a", "team-b"]);
    leave(&db, "user-a", "bot_1", &first, at(5)).await.unwrap();
    assert_eq!(
        active_conversations(&db, "bot_1", 2).await.unwrap(),
        ["team-c"]
    );
}

#[tokio::test]
async fn a_deleted_bot_s_inbox_and_keys_are_purged_page_by_page() {
    let db = database().await;
    let bot_copies = (0..PURGE_PAGE + 3).flat_map(|n| {
        let joined = format!("principal-{n}");
        member_copies(
            "team",
            MemberOp::Join,
            &joined,
            "user-a",
            &[bot_principal("bot_1")],
            at(0),
            n as i64 + 1,
        )
    });
    let own_copy = member_copies(
        "team",
        MemberOp::Join,
        "x",
        "user-a",
        &[user("user-a")],
        at(0),
        1,
    );
    insert_copies(&db, bot_copies.chain(own_copy).collect())
        .await
        .unwrap();
    for (principal, key_id) in [
        ("bot_1", "key-1"),
        ("bot_1", "key-2"),
        ("bot_1", "key-3"),
        ("user-a", "key-1"),
    ] {
        let key = validate_key(&key_request(key_id, "cGs=")).unwrap();
        register_in(&db, principal, &key, at(1)).await.unwrap();
    }
    revoke_in(&db, "bot_1", "key-3", at(2)).await.unwrap();

    assert_eq!(purge_inbox(&db, "bot_1").await.unwrap(), PURGE_PAGE + 6);
    for (principal, entries, keys) in [("bot_1", 0, 0), ("user-a", 1, 1)] {
        let held_entries = inbox_entry::Entity::find()
            .filter(inbox_entry::Column::InboxId.eq(principal))
            .count(&db)
            .await
            .unwrap();
        let held_keys = inbox_key::Entity::find()
            .filter(inbox_key::Column::PrincipalId.eq(principal))
            .count(&db)
            .await
            .unwrap();
        assert_eq!((held_entries, held_keys), (entries, keys), "{principal}");
    }
    assert_eq!(purge_inbox(&db, "bot_1").await.unwrap(), 0);
}

#[tokio::test]
async fn a_person_who_leaves_takes_their_bots_along() {
    let db = database().await;
    insert_bot(&db, "bot_b", "user-b", false).await;
    insert_bot(&db, "bot_c", "user-c", false).await;
    let people = [user("user-b"), user("user-c")];
    group(&db, "team", ConversationEncryption::Plain, &people).await;
    add_in(&db, "team", "user-b", &[bot_principal("bot_b")], at(10))
        .await
        .unwrap();
    add_in(&db, "team", "user-c", &[bot_principal("bot_c")], at(11))
        .await
        .unwrap();

    let woken = remove_in(&db, "team", "user-a", "user-b", at(20))
        .await
        .unwrap();
    assert_eq!(woken, ["user-a", "user-b", "user-c"]);
    for gone in ["user-b", "bot_b"] {
        assert!(
            active_membership(&db, "team", gone).await.is_err(),
            "{gone}"
        );
    }
    active_membership(&db, "team", "bot_c").await.unwrap();
    for inbox in ["user-c", "bot_b"] {
        let bodies = member_bodies(&db, inbox, "team").await;
        for principal in ["user-b", "bot_b"] {
            let left = json!({ "op": "leave", "principal_id": principal, "by": "user-a" });
            assert!(bodies.contains(&left), "{inbox} misses {principal} leaving");
        }
    }

    remove_in(&db, "team", "user-c", "user-c", at(30))
        .await
        .unwrap();
    assert!(active_membership(&db, "team", "bot_c").await.is_err());
}

#[tokio::test]
async fn reading_as_a_bot_follows_the_bot_not_its_owner() {
    let db = database().await;
    team(&db).await;
    let pair = direct(&db, "user-a", &bot_principal("bot_1"), 1).await;
    reader_membership(&db, "team", &as_bot_1()).await.unwrap();

    end_memberships(&db, &["team".to_string()], &["user-a".to_string()], at(30))
        .await
        .unwrap();
    reader_membership(&db, "team", &as_bot_1()).await.unwrap();
    reader_membership(&db, &pair, &as_bot_1()).await.unwrap();
    let mut listed: Vec<String> = conversations_for(&db, &as_bot_1(), None)
        .await
        .unwrap()
        .into_iter()
        .map(|conversation| conversation.id)
        .collect();
    listed.sort();
    let mut both = vec![pair.clone(), "team".to_string()];
    both.sort();
    assert_eq!(listed, both);
    end_memberships(&db, &["team".to_string()], &["bot_1".to_string()], at(31))
        .await
        .unwrap();
    let error = reader_membership(&db, "team", &as_bot_1())
        .await
        .unwrap_err();
    assert_eq!(error.status(), StatusCode::NOT_FOUND);
    assert!(
        conversations_for(&db, &as_bot_1(), Some("team"))
            .await
            .unwrap()
            .is_empty()
    );
}

fn key_request(key_id: &str, public_key: &str) -> RegisterInboxKeyRequest {
    RegisterInboxKeyRequest {
        principal_id: None,
        key_id: key_id.into(),
        algorithm: "hpke-x25519-sha256-chacha20poly1305".into(),
        public_key: public_key.into(),
    }
}

#[test]
fn key_fields_are_trimmed_tokens() {
    for (key_id, public_key) in [("bad id", "cGs="), ("key-1", "with space"), ("key-1", "")] {
        let error = validate_key(&key_request(key_id, public_key)).unwrap_err();
        assert_eq!(
            error.status(),
            StatusCode::BAD_REQUEST,
            "{key_id} {public_key}"
        );
    }
    let key = validate_key(&key_request(" device:1 ", " cGs= ")).unwrap();
    assert_eq!(
        (key.key_id.as_str(), key.public_key.as_str()),
        ("device:1", "cGs=")
    );

    assert_eq!(
        requested_principals(Some(" b , a,,b ")).unwrap(),
        ["a", "b"]
    );
    assert!(requested_principals(None).unwrap().is_empty());
    let many = (0..51)
        .map(|n| format!("p{n}"))
        .collect::<Vec<_>>()
        .join(",");
    let error = requested_principals(Some(&many)).unwrap_err();
    assert_eq!(error.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn keys_are_registered_once_and_only_active_keys_are_listed() {
    let db = database().await;
    let key = validate_key(&key_request("device:1", "cGs=")).unwrap();
    let stored = register_in(&db, "user-a", &key, at(0)).await.unwrap();
    assert_eq!(
        register_in(&db, "user-a", &key, at(5)).await.unwrap(),
        stored
    );
    let changed = validate_key(&key_request("device:1", "b3RoZXI=")).unwrap();
    let error = register_in(&db, "user-a", &changed, at(6))
        .await
        .unwrap_err();
    assert_code(&error, StatusCode::CONFLICT, INBOX_KEY_EXISTS);

    let revoked = validate_key(&key_request("device:0", "b2xk")).unwrap();
    register_in(&db, "user-a", &revoked, at(1)).await.unwrap();
    db.execute_unprepared(
        r#"UPDATE public."InboxKey" SET "revokedAt" = '2027-01-01T00:00:00+00:00' WHERE "keyId" = 'device:0'"#,
    )
    .await
    .unwrap();
    let listed = active_keys(&db, &["user-a".into(), "user-b".into()])
        .await
        .unwrap();
    let ids: Vec<&str> = listed.iter().map(|key| key.key_id.as_str()).collect();
    assert_eq!(ids, ["device:1"]);
    let error = register_in(&db, "user-a", &revoked, at(7))
        .await
        .unwrap_err();
    assert_code(&error, StatusCode::CONFLICT, INBOX_KEY_EXISTS);
}

#[tokio::test]
async fn a_principal_holds_at_most_twenty_active_keys() {
    let db = database().await;
    for n in 0..MAX_ACTIVE_KEYS {
        let key = validate_key(&key_request(&format!("bulk-{n}"), "cGs=")).unwrap();
        register_in(&db, "user-a", &key, at(10)).await.unwrap();
    }
    let one_more = validate_key(&key_request("bulk-last", "cGs=")).unwrap();
    let error = register_in(&db, "user-a", &one_more, at(11))
        .await
        .unwrap_err();
    assert_code(&error, StatusCode::CONFLICT, INBOX_KEY_LIMIT);
    register_in(&db, "user-b", &one_more, at(11)).await.unwrap();
}

#[tokio::test]
async fn revoking_a_key_frees_its_slot_and_keeps_its_id_taken() {
    let db = database().await;
    for n in 0..MAX_ACTIVE_KEYS {
        let key = validate_key(&key_request(&format!("bulk-{n}"), "cGs=")).unwrap();
        register_in(&db, "user-a", &key, at(10)).await.unwrap();
    }
    let one_more = validate_key(&key_request("bulk-last", "cGs=")).unwrap();
    let full = register_in(&db, "user-a", &one_more, at(11))
        .await
        .unwrap_err();
    assert_code(&full, StatusCode::CONFLICT, INBOX_KEY_LIMIT);

    assert!(revoke_in(&db, "user-a", "bulk-0", at(12)).await.unwrap());
    assert!(
        !revoke_in(&db, "user-a", "bulk-0", at(13)).await.unwrap(),
        "revoking twice changes nothing"
    );
    register_in(&db, "user-a", &one_more, at(14)).await.unwrap();
    let listed = active_keys(&db, &["user-a".into()]).await.unwrap();
    assert_eq!(listed.len() as u64, MAX_ACTIVE_KEYS);
    assert!(listed.iter().all(|key| key.key_id != "bulk-0"));

    let reused = validate_key(&key_request("bulk-0", "cGs=")).unwrap();
    let taken = register_in(&db, "user-a", &reused, at(15))
        .await
        .unwrap_err();
    assert_code(&taken, StatusCode::CONFLICT, INBOX_KEY_EXISTS);
    for (principal, key_id) in [("user-a", "missing"), ("user-b", "bulk-1")] {
        let error = revoke_in(&db, principal, key_id, at(16)).await.unwrap_err();
        assert_eq!(
            error.status(),
            StatusCode::NOT_FOUND,
            "{principal} {key_id}"
        );
    }
}

#[tokio::test]
async fn only_the_owners_answer_to_a_card_stays_silent_to_the_bot() {
    let db = super::fixture::database().await;
    insert_bot(&db, "bot_1", "user-a", false).await;
    team(&db).await;
    let card = outgoing(
        "card1",
        InboxEntryKind::Message,
        json!({
            "text": "I need your approval: run it",
            "approval": {
                "id": "card1", "turn_id": "t1", "tool": "run", "summary": "run it",
                "scope": "app:1", "args_hash": "h1"
            }
        }),
    );
    send_in(&db, "team", "bot_1", &card, at(1)).await.unwrap();
    let answer = |message_id: &str| {
        outgoing(
            message_id,
            InboxEntryKind::Message,
            json!({
                "text": "",
                "reply_to": "card1",
                "mentions": ["bot_1"],
                "approval_response": { "approval_id": "card1", "decision": "once" }
            }),
        )
    };
    let right_id = flow_like_types::tool_grant::approval_response_message_id("card1");
    let woke = |sent: &super::entries::Sent| {
        sent.bots
            .iter()
            .any(|recipient| recipient.bot_id == "bot_1")
    };
    let owner = send_in(&db, "team", "user-a", &answer(&right_id), at(2))
        .await
        .unwrap();
    assert!(
        !woke(&owner),
        "the owner's answer to the card wakes no turn"
    );
    let wrong_id = send_in(&db, "team", "user-a", &answer("not_the_answer_id"), at(3))
        .await
        .unwrap();
    assert!(
        woke(&wrong_id),
        "an answer under another message id is a message"
    );
    let stranger = send_in(
        &db,
        "team",
        "user-b",
        &outgoing(
            "stranger_answer",
            InboxEntryKind::Message,
            json!({
                "text": "", "reply_to": "card1", "mentions": ["bot_1"],
                "approval_response": { "approval_id": "card1", "decision": "once" }
            }),
        ),
        at(4),
    )
    .await
    .unwrap();
    assert!(
        woke(&stranger),
        "a non-owner's approval_response is a message"
    );
}
