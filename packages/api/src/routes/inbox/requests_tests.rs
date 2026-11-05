use super::{
    INBOX_REQUEST_LIMIT, MEMBER_ACTIVE, MEMBER_BLOCKED, MEMBER_DECLINED, MEMBER_PENDING, Reader,
    conversations::{NewConversation, start_direct_as, start_group},
    entries::send_in,
    fixture::{
        assert_code, at, bot_principal, copies, database, insert_bot, outgoing, says, text, user,
    },
    invitations::{accept_in, classify, decline_in, invitations_for, invite_in, set_limits_in},
    model::InboxEntry,
    person_push::{is_muted, notify_message, push_targets, set_muted_in},
    requests::{MAX_HELD_MESSAGES, answer_by_writing, block_in, deliver_held_in, plan_direct},
    view::conversations_for,
};
use crate::{
    bot_notify::NoticeSink,
    entity::{bot_turn, conversation_member, membership},
    push_notifications::DispatchNotificationInput,
};
use async_trait::async_trait;
use axum::http::StatusCode;
use flow_like_types::{
    Result,
    bot_effective::BotRoomLimits,
    inbox::{ConversationEncryption, ConversationKind, InboxEntryKind},
};
use sea_orm::{
    ActiveModelTrait, ActiveValue::Set, ColumnTrait, ConnectionTrait, DatabaseConnection,
    EntityTrait, PaginatorTrait, QueryFilter,
};
use serde_json::json;
use std::{collections::BTreeMap, sync::Mutex};

async fn share_app(db: &DatabaseConnection, user_id: &str, app_id: &str) {
    membership::ActiveModel {
        id: Set(format!("{user_id}-{app_id}")),
        user_id: Set(user_id.into()),
        app_id: Set(app_id.into()),
        role_id: Set("role".into()),
        created_at: Set(at(0)),
        updated_at: Set(at(0)),
        joined_via: Set(None),
    }
    .insert(db)
    .await
    .unwrap();
}

/// user-a and user-b share an app; user-c has no account yet. bot_a is user-a's, bot_b user-b's.
async fn world() -> DatabaseConnection {
    let db = database().await;
    insert_bot(&db, "bot_a", "user-a", "SERVER").await;
    insert_bot(&db, "bot_b", "user-b", "SERVER").await;
    share_app(&db, "user-a", "app-1").await;
    share_app(&db, "user-b", "app-1").await;
    db.execute_unprepared(
        r#"INSERT INTO public."User" ("id", "name") VALUES ('user-a', 'Ada'), ('user-b', 'Bea')"#,
    )
    .await
    .unwrap();
    db
}

/// user-a messages `other` for the first time and the request conversation `dm` exists.
async fn request_to(db: &DatabaseConnection, other: &str) {
    let plan = plan_direct(db, "user-a", other, ConversationEncryption::Plain)
        .await
        .unwrap();
    assert!(plan.request);
    let id = start_direct_as(
        db,
        "user-a",
        &plan.other,
        ConversationEncryption::Plain,
        true,
        at(1),
    )
    .await
    .unwrap();
    db.execute_unprepared(&format!(
        r#"UPDATE public."Conversation" SET "id" = 'dm' WHERE "id" = '{id}'"#
    ))
    .await
    .unwrap();
    db.execute_unprepared(&format!(
        r#"UPDATE public."ConversationMember" SET "conversationId" = 'dm' WHERE "conversationId" = '{id}'"#
    ))
    .await
    .unwrap();
}

async fn status_of(db: &DatabaseConnection, conversation: &str, principal: &str) -> String {
    conversation_member::Entity::find_by_id((conversation.to_string(), principal.to_string()))
        .one(db)
        .await
        .unwrap()
        .unwrap()
        .status
}

async fn turns(db: &DatabaseConnection) -> u64 {
    bot_turn::Entity::find().count(db).await.unwrap()
}

async fn inbox_of(db: &DatabaseConnection, who: &str) -> Vec<String> {
    use crate::entity::inbox_entry;
    use sea_orm::QueryOrder;
    inbox_entry::Entity::find()
        .filter(inbox_entry::Column::InboxId.eq(who))
        .order_by_asc(inbox_entry::Column::Seq)
        .all(db)
        .await
        .unwrap()
        .into_iter()
        .map(|entry| entry.message_id)
        .collect()
}

#[tokio::test]
async fn the_first_message_to_a_person_is_held_without_waking_or_counting() {
    let db = world().await;
    request_to(&db, "user-b").await;

    let sent = send_in(&db, "dm", "user-a", &text("m1"), at(2))
        .await
        .unwrap();
    assert_eq!(sent.wake, ["user-a"], "the recipient isn't woken");
    assert!(sent.bots.is_empty());
    assert_eq!(
        copies(&db, "m1")
            .await
            .into_iter()
            .map(|copy| copy.inbox_id)
            .collect::<Vec<_>>(),
        ["user-a"],
        "only the sender holds a copy"
    );

    let theirs = conversations_for(&db, &Reader::own("user-b"), None)
        .await
        .unwrap();
    assert!(theirs.is_empty(), "a request isn't a conversation yet");

    let requests = invitations_for(&db, "user-b").await.unwrap();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].conversation_kind, "direct");
    assert_eq!(requests[0].held_count, 1);
    assert_eq!(requests[0].invited_by.as_deref(), Some("user-a"));

    let mine = conversations_for(&db, &Reader::own("user-a"), None)
        .await
        .unwrap();
    assert_eq!(mine[0].pending.len(), 1);
    assert_eq!(mine[0].pending[0].status, "pending");
}

#[tokio::test]
async fn accepting_shows_every_held_message_as_unread() {
    let db = world().await;
    request_to(&db, "user-b").await;
    for (id, secs) in [("m1", 2), ("m2", 3)] {
        send_in(&db, "dm", "user-a", &text(id), at(secs))
            .await
            .unwrap();
    }

    let wake = accept_in(&db, "dm", "user-b", "user-b", None, at(10))
        .await
        .unwrap();
    assert!(wake.contains(&"user-a".to_string()) && wake.contains(&"user-b".to_string()));

    assert_eq!(inbox_of(&db, "user-b").await, ["m1", "m2"]);
    let theirs = conversations_for(&db, &Reader::own("user-b"), None)
        .await
        .unwrap();
    assert_eq!(theirs.len(), 1);
    assert_eq!(theirs[0].unread_count, 2);
    assert!(invitations_for(&db, "user-b").await.unwrap().is_empty());

    let reply = send_in(&db, "dm", "user-b", &text("r1"), at(11))
        .await
        .unwrap();
    assert!(reply.wake.contains(&"user-a".to_string()));
    let mine = conversations_for(&db, &Reader::own("user-a"), None)
        .await
        .unwrap();
    assert!(mine[0].pending.is_empty());
    assert_eq!(mine[0].unread_count, 1);
}

#[tokio::test]
async fn declining_hides_the_request_and_the_sender_keeps_seeing_pending() {
    let db = world().await;
    request_to(&db, "user-b").await;
    send_in(&db, "dm", "user-a", &text("m1"), at(2))
        .await
        .unwrap();

    let wake = decline_in(&db, "dm", "user-b", "user-b").await.unwrap();
    assert!(wake.is_empty(), "the sender isn't told");
    assert_eq!(status_of(&db, "dm", "user-b").await, MEMBER_DECLINED);
    assert!(invitations_for(&db, "user-b").await.unwrap().is_empty());

    send_in(&db, "dm", "user-a", &text("m2"), at(3))
        .await
        .unwrap();
    assert_eq!(inbox_of(&db, "user-b").await, Vec::<String>::new());
    let mine = conversations_for(&db, &Reader::own("user-a"), None)
        .await
        .unwrap();
    assert_eq!(
        mine[0].pending[0].status, "pending",
        "declined looks pending"
    );
}

#[tokio::test]
async fn blocking_also_declines_their_other_invitations_and_stops_new_ones() {
    let db = world().await;
    request_to(&db, "user-b").await;
    let plan = NewConversation {
        kind: ConversationKind::Group,
        title: Some("room".into()),
        members: Vec::new(),
        encryption: ConversationEncryption::Plain,
    };
    start_group(&db, "room", "user-a", &plan, &[], at(1))
        .await
        .unwrap();
    let ids = vec!["user-b".to_string()];
    let invitees = classify(&db, "user-a", &ids).await.unwrap();
    let none = BTreeMap::new();
    invite_in(&db, "room", "user-a", &invitees, &none, at(2))
        .await
        .unwrap();
    assert_eq!(status_of(&db, "room", "user-b").await, MEMBER_PENDING);

    block_in(&db, "dm", "user-b", "user-b").await.unwrap();
    assert_eq!(status_of(&db, "dm", "user-b").await, MEMBER_BLOCKED);
    assert_eq!(status_of(&db, "room", "user-b").await, MEMBER_DECLINED);

    conversation_member::Entity::delete_by_id(("room".to_string(), "user-b".to_string()))
        .exec(&db)
        .await
        .unwrap();
    let again = invite_in(&db, "room", "user-a", &invitees, &none, at(3))
        .await
        .unwrap();
    assert!(
        again.invited.is_empty(),
        "a blocked person isn't invited again"
    );
}

#[tokio::test]
async fn only_a_request_can_be_blocked_and_only_by_its_recipient() {
    let db = world().await;
    request_to(&db, "user-b").await;
    let wrong = block_in(&db, "dm", "user-b", "user-a").await.unwrap_err();
    assert_eq!(wrong.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn a_request_to_someone_elses_bot_reaches_its_owner() {
    let db = world().await;
    request_to(&db, "bot_b").await;
    let sent = send_in(&db, "dm", "user-a", &text("hi_bot"), at(2))
        .await
        .unwrap();
    assert!(sent.bots.is_empty(), "the bot isn't a member yet");
    assert_eq!(turns(&db).await, 0);
    assert!(inbox_of(&db, "bot_b").await.is_empty());

    assert!(invitations_for(&db, "user-a").await.unwrap().is_empty());
    let asked = invitations_for(&db, "user-b").await.unwrap();
    assert_eq!(asked.len(), 1);
    assert_eq!(asked[0].principal_id, "bot_b");
    assert_eq!(asked[0].kind, "bot");
    assert_eq!(asked[0].held_count, 1);

    let stranger = accept_in(&db, "dm", "bot_b", "user-a", None, at(3))
        .await
        .unwrap_err();
    assert_eq!(stranger.status(), StatusCode::NOT_FOUND);

    accept_in(&db, "dm", "bot_b", "user-b", None, at(4))
        .await
        .unwrap();
    assert_eq!(inbox_of(&db, "bot_b").await, ["hi_bot"]);
    let recipients = deliver_held_in(&db, "dm", "bot_b").await.unwrap();
    assert_eq!(recipients.len(), 1);
    assert_eq!(recipients[0].owner_id, "user-b");
    assert_eq!(turns(&db).await, 1, "the bot answers what it held");
}

#[tokio::test]
async fn your_own_bot_and_encrypted_chats_with_people_are_not_requests() {
    let db = world().await;
    let own = plan_direct(&db, "user-a", "bot_a", ConversationEncryption::Plain)
        .await
        .unwrap();
    assert!(!own.request);
    let sealed = plan_direct(&db, "user-a", "user-b", ConversationEncryption::E2e)
        .await
        .unwrap();
    assert!(!sealed.request);
    let sealed_bot = plan_direct(&db, "user-a", "bot_b", ConversationEncryption::E2e)
        .await
        .unwrap_err();
    assert_eq!(sealed_bot.status(), StatusCode::FORBIDDEN);
    let nobody = plan_direct(&db, "user-a", "user-c", ConversationEncryption::Plain)
        .await
        .unwrap_err();
    assert_eq!(nobody.status(), StatusCode::FORBIDDEN);
    db.execute_unprepared(r#"INSERT INTO public."User" ("id", "name") VALUES ('user-c', 'Cy')"#)
        .await
        .unwrap();
    let stranger = plan_direct(&db, "user-a", "user-c", ConversationEncryption::Plain)
        .await
        .unwrap();
    assert!(stranger.request, "a person sharing no app gets a request");
}

#[tokio::test]
async fn writing_back_to_a_request_accepts_it() {
    let db = world().await;
    request_to(&db, "user-b").await;
    send_in(&db, "dm", "user-a", &text("m1"), at(2))
        .await
        .unwrap();
    decline_in(&db, "dm", "user-b", "user-b").await.unwrap();

    answer_by_writing(&db, "dm", "user-b", at(5)).await.unwrap();
    assert_eq!(status_of(&db, "dm", "user-b").await, MEMBER_ACTIVE);
    assert_eq!(inbox_of(&db, "user-b").await, ["m1"]);
    send_in(&db, "dm", "user-b", &text("r1"), at(6))
        .await
        .unwrap();
    assert!(
        answer_by_writing(&db, "dm", "user-a", at(7))
            .await
            .unwrap()
            .is_empty(),
        "an active member has nothing to answer"
    );
}

#[tokio::test]
async fn a_sender_can_leave_only_a_few_messages_before_an_answer() {
    let db = world().await;
    request_to(&db, "user-b").await;
    for index in 0..MAX_HELD_MESSAGES {
        send_in(
            &db,
            "dm",
            "user-a",
            &text(&format!("m{index}")),
            at(10 + index as i64),
        )
        .await
        .unwrap();
    }
    let over = send_in(&db, "dm", "user-a", &text("one-too-many"), at(99))
        .await
        .unwrap_err();
    assert_code(&over, StatusCode::CONFLICT, INBOX_REQUEST_LIMIT);

    accept_in(&db, "dm", "user-b", "user-b", None, at(100))
        .await
        .unwrap();
    send_in(&db, "dm", "user-a", &text("after-accept"), at(101))
        .await
        .unwrap();
}

#[derive(Default)]
struct Recorder(Mutex<Vec<(String, DispatchNotificationInput)>>);

#[async_trait]
impl NoticeSink for Recorder {
    async fn send(&self, id: &str, input: DispatchNotificationInput) -> Result<()> {
        self.0.lock().unwrap().push((id.to_string(), input));
        Ok(())
    }
}

impl Recorder {
    fn sent(&self) -> Vec<(String, DispatchNotificationInput)> {
        self.0.lock().unwrap().clone()
    }
}

async fn entry(db: &DatabaseConnection, message_id: &str, inbox: &str) -> InboxEntry {
    copies(db, message_id)
        .await
        .into_iter()
        .find(|copy| copy.inbox_id == inbox)
        .map(InboxEntry::from)
        .unwrap()
}

#[tokio::test]
async fn no_push_while_the_request_is_pending() {
    let db = world().await;
    request_to(&db, "user-b").await;
    send_in(&db, "dm", "user-a", &text("m1"), at(2))
        .await
        .unwrap();
    let sink = Recorder::default();
    let told = notify_message(&db, &sink, &entry(&db, "m1", "user-a").await)
        .await
        .unwrap();
    assert_eq!(told, 0);
    assert!(sink.sent().is_empty());

    let members = conversation_member::Entity::find().all(&db).await.unwrap();
    assert!(push_targets(&members, "user-a").is_empty());
}

#[tokio::test]
async fn person_messages_push_once_per_recipient_grouped_by_conversation() {
    let db = world().await;
    request_to(&db, "user-b").await;
    accept_in(&db, "dm", "user-b", "user-b", None, at(2))
        .await
        .unwrap();
    send_in(
        &db,
        "dm",
        "user-a",
        &outgoing(
            "m1",
            InboxEntryKind::Message,
            json!({ "text": "  hello   there " }),
        ),
        at(3),
    )
    .await
    .unwrap();

    let sink = Recorder::default();
    let sent = entry(&db, "m1", "user-a").await;
    assert_eq!(notify_message(&db, &sink, &sent).await.unwrap(), 1);
    let again = notify_message(&db, &sink, &sent).await.unwrap();
    assert_eq!(again, 1);
    let recorded = sink.sent();
    assert_eq!(recorded[0].0, recorded[1].0, "a repeat carries the same id");
    let (_, input) = &recorded[0];
    assert_eq!(input.user_id, "user-b");
    assert_eq!(input.title, "Ada");
    assert_eq!(input.description.as_deref(), Some("hello there"));
    assert_eq!(input.source_run_id.as_deref(), Some("chat:dm"));
    assert_eq!(input.link.as_deref(), Some("/bots?conversation=dm"));
}

#[tokio::test]
async fn a_muted_conversation_sends_no_push_but_still_counts_unread() {
    let db = world().await;
    request_to(&db, "user-b").await;
    accept_in(&db, "dm", "user-b", "user-b", None, at(2))
        .await
        .unwrap();
    assert_eq!(set_muted_in(&db, "dm", "user-b", true).await.unwrap(), 1);
    send_in(&db, "dm", "user-a", &text("m1"), at(3))
        .await
        .unwrap();

    let sink = Recorder::default();
    let told = notify_message(&db, &sink, &entry(&db, "m1", "user-a").await)
        .await
        .unwrap();
    assert_eq!(told, 0);
    let theirs = conversations_for(&db, &Reader::own("user-b"), None)
        .await
        .unwrap();
    assert!(theirs[0].muted);
    assert_eq!(theirs[0].unread_count, 1);

    set_muted_in(&db, "dm", "user-b", false).await.unwrap();
    let theirs = conversations_for(&db, &Reader::own("user-b"), None)
        .await
        .unwrap();
    assert!(!theirs[0].muted);
    assert!(!is_muted(None));
}

#[tokio::test]
async fn rooms_push_with_the_room_title_and_skip_bots_and_the_sender() {
    let db = world().await;
    let plan = NewConversation {
        kind: ConversationKind::Group,
        title: Some("Launch".into()),
        members: Vec::new(),
        encryption: ConversationEncryption::Plain,
    };
    start_group(
        &db,
        "room",
        "user-a",
        &plan,
        &[user("user-b"), bot_principal("bot_a")],
        at(1),
    )
    .await
    .unwrap();
    send_in(&db, "room", "user-a", &text("m1"), at(2))
        .await
        .unwrap();
    let sink = Recorder::default();
    let told = notify_message(&db, &sink, &entry(&db, "m1", "user-a").await)
        .await
        .unwrap();
    assert_eq!(told, 1);
    assert_eq!(sink.sent()[0].1.user_id, "user-b");
    assert_eq!(sink.sent()[0].1.title, "Ada in Launch");
}

#[tokio::test]
async fn a_mentioned_only_bot_wakes_on_a_mention_in_a_room_and_not_on_plain_chatter() {
    let db = world().await;
    let plan = NewConversation {
        kind: ConversationKind::Group,
        title: Some("Launch".into()),
        members: Vec::new(),
        encryption: ConversationEncryption::Plain,
    };
    start_group(
        &db,
        "room",
        "user-a",
        &plan,
        &[user("user-b"), bot_principal("bot_a")],
        at(1),
    )
    .await
    .unwrap();
    let mentioned_only = BotRoomLimits {
        mentioned_only: true,
        ..Default::default()
    };
    set_limits_in(&db, "room", "bot_a", "user-a", Some(&mentioned_only))
        .await
        .unwrap();
    let quiet = send_in(&db, "room", "user-b", &text("chatter"), at(2))
        .await
        .unwrap();
    assert!(quiet.bots.iter().all(|bot| !bot.mentioned));
    assert_eq!(turns(&db).await, 0, "no mention, no turn in a room");

    let ping = outgoing(
        "ping",
        InboxEntryKind::Message,
        json!({ "text": "@Scout look", "mentions": ["bot_a"] }),
    );
    let sent = send_in(&db, "room", "user-b", &ping, at(3)).await.unwrap();
    assert_eq!(sent.bots.len(), 1);
    assert!(sent.bots[0].mentioned);
    assert_eq!(turns(&db).await, 1);
}

#[tokio::test]
async fn a_bot_that_is_not_a_member_is_not_woken_by_a_mention() {
    let db = world().await;
    let plan = NewConversation {
        kind: ConversationKind::Group,
        title: Some("Launch".into()),
        members: Vec::new(),
        encryption: ConversationEncryption::Plain,
    };
    start_group(&db, "room", "user-a", &plan, &[user("user-b")], at(1))
        .await
        .unwrap();
    let ping = outgoing(
        "ping",
        InboxEntryKind::Message,
        json!({ "text": "@bot_a hi", "mentions": ["bot_a"] }),
    );
    let sent = send_in(&db, "room", "user-b", &ping, at(2)).await.unwrap();
    assert!(sent.bots.is_empty());
    assert_eq!(turns(&db).await, 0);
}

async fn launch_room(db: &DatabaseConnection) {
    let plan = NewConversation {
        kind: ConversationKind::Group,
        title: Some("Launch".into()),
        members: Vec::new(),
        encryption: ConversationEncryption::Plain,
    };
    start_group(
        db,
        "room",
        "user-a",
        &plan,
        &[user("user-b"), bot_principal("bot_a")],
        at(1),
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn a_started_call_pushes_once_and_neither_call_entry_counts_as_unread() {
    let db = world().await;
    launch_room(&db).await;
    let started = outgoing("call_1", InboxEntryKind::Call, json!({ "op": "started" }));
    send_in(&db, "room", "user-a", &started, at(2))
        .await
        .unwrap();
    let ended = outgoing(
        "end_1",
        InboxEntryKind::Call,
        json!({ "op": "ended", "call": "call_1", "duration_ms": 61_000 }),
    );
    send_in(&db, "room", "user-a", &ended, at(3)).await.unwrap();

    let sink = Recorder::default();
    let told = notify_message(&db, &sink, &entry(&db, "call_1", "user-a").await)
        .await
        .unwrap();
    assert_eq!(told, 1);
    let (_, input) = &sink.sent()[0];
    assert_eq!(input.user_id, "user-b");
    assert_eq!(input.title, "Ada in Launch");
    assert_eq!(input.description.as_deref(), Some("Started a call"));
    assert_eq!(
        input.link.as_deref(),
        Some("/bots?conversation=room&call=call_1")
    );

    let told = notify_message(&db, &sink, &entry(&db, "end_1", "user-a").await)
        .await
        .unwrap();
    assert_eq!(told, 0, "an ended call sends no push");
    assert_eq!(sink.sent().len(), 1);

    let theirs = conversations_for(&db, &Reader::own("user-b"), None)
        .await
        .unwrap();
    assert_eq!(theirs[0].unread_count, 0);
    assert!(theirs[0].last_entry.is_none());
    assert_eq!(turns(&db).await, 0, "a call wakes no bot");
}

#[tokio::test]
async fn a_muted_member_gets_no_call_push() {
    let db = world().await;
    launch_room(&db).await;
    set_muted_in(&db, "room", "user-b", true).await.unwrap();
    let started = outgoing("call_1", InboxEntryKind::Call, json!({ "op": "started" }));
    send_in(&db, "room", "user-a", &started, at(2))
        .await
        .unwrap();
    let sink = Recorder::default();
    let told = notify_message(&db, &sink, &entry(&db, "call_1", "user-a").await)
        .await
        .unwrap();
    assert_eq!(told, 0);
    assert!(sink.sent().is_empty());
}

#[tokio::test]
async fn a_transcript_wakes_only_the_bots_it_mentions_and_counts_as_no_unread() {
    let db = world().await;
    launch_room(&db).await;
    let line = |id: &str, mentions: Vec<&str>| {
        outgoing(
            id,
            InboxEntryKind::Transcript,
            json!({
                "call": "call_1",
                "text": "what is the plan",
                "offset_ms": 4_000,
                "duration_ms": 2_500,
                "mentions": mentions,
            }),
        )
    };
    let quiet = send_in(&db, "room", "user-a", &line("t_1", vec![]), at(2))
        .await
        .unwrap();
    assert!(quiet.bots.iter().all(|bot| !bot.mentioned));
    assert_eq!(turns(&db).await, 0, "no mention, no turn");

    let named = send_in(&db, "room", "user-a", &line("t_2", vec!["bot_a"]), at(3))
        .await
        .unwrap();
    assert!(named.bots[0].mentioned);
    assert_eq!(turns(&db).await, 1);

    let sink = Recorder::default();
    let told = notify_message(&db, &sink, &entry(&db, "t_2", "user-a").await)
        .await
        .unwrap();
    assert_eq!(told, 0, "a transcript sends no push");
    let theirs = conversations_for(&db, &Reader::own("user-b"), None)
        .await
        .unwrap();
    assert_eq!(theirs[0].unread_count, 0);
}

#[tokio::test]
async fn a_transcript_naming_a_stranger_is_refused() {
    let db = world().await;
    launch_room(&db).await;
    let line = outgoing(
        "t_1",
        InboxEntryKind::Transcript,
        json!({
            "call": "call_1",
            "text": "hello",
            "offset_ms": 0,
            "duration_ms": 900,
            "mentions": ["bot_b"],
        }),
    );
    let error = send_in(&db, "room", "user-a", &line, at(2))
        .await
        .unwrap_err();
    assert_eq!(error.status(), StatusCode::BAD_REQUEST);
    assert!(says(&error, "bot_b"), "{error:?}");
    assert!(copies(&db, "t_1").await.is_empty());
}

#[tokio::test]
async fn calls_are_refused_in_end_to_end_encrypted_conversations() {
    let db = world().await;
    launch_room(&db).await;
    db.execute_unprepared(
        r#"UPDATE public."Conversation" SET "encryption" = 'E2E' WHERE "id" = 'room'"#,
    )
    .await
    .unwrap();
    for sent in [
        outgoing("call_1", InboxEntryKind::Call, json!({ "op": "started" })),
        outgoing(
            "t_1",
            InboxEntryKind::Transcript,
            json!({ "call": "call_1", "text": "hi", "offset_ms": 0, "duration_ms": 10 }),
        ),
    ] {
        let error = send_in(&db, "room", "user-a", &sent, at(2))
            .await
            .unwrap_err();
        assert_eq!(error.status(), StatusCode::BAD_REQUEST);
        assert!(says(&error, "end-to-end"), "{error:?}");
    }
    assert!(copies(&db, "call_1").await.is_empty());
}
