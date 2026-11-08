use super::{
    INBOX_INVITE_LIMIT, INBOX_INVITE_NOT_ALLOWED, INBOX_MEMBER_NOT_ALLOWED, MEMBER_ACTIVE,
    MEMBER_DECLINED, MEMBER_PENDING, Reader,
    conversations::{NewConversation, create_group_in, start_group},
    entries::send_in,
    fixture::{assert_code, at, copies, database, delete_bot, insert_bot, says, text, user},
    invitations::{
        InviteMode, accept_in, cancel_in, classify, decline_in, invitable, invitations_for,
        invite_in, set_limits_in,
    },
    view::conversations_for,
};
use crate::{
    bot_permission::room_limits,
    db::{DbDialect, RetryPolicy, retry_transaction},
    entity::{conversation, conversation_member, inbox_entry, membership},
    error::ApiError,
};
use axum::http::StatusCode;
use flow_like_types::{
    bot_effective::BotRoomLimits,
    inbox::{ConversationEncryption, ConversationKind, PrincipalKind},
};
use sea_orm::{
    ActiveModelTrait, ActiveValue::Set, ColumnTrait, ConnectionTrait, DatabaseConnection,
    EntityTrait, QueryFilter,
};
use std::collections::BTreeMap;

const NO_LIMITS: BTreeMap<String, BotRoomLimits> = BTreeMap::new();

async fn share_app(db: &DatabaseConnection, user_id: &str, app_id: &str) {
    db.execute_unprepared(&format!(
        r#"INSERT INTO public."User" ("id") VALUES ('{user_id}') ON CONFLICT DO NOTHING"#
    ))
    .await
    .unwrap();
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

async fn insert_user(db: &DatabaseConnection, id: &str, name: &str) {
    db.execute_unprepared(&format!(
        r#"INSERT INTO public."User" ("id", "name") VALUES ('{id}', '{name}')
           ON CONFLICT ("id") DO UPDATE SET "name" = excluded."name""#
    ))
    .await
    .unwrap();
}

/// `room`: started by user-a alone. user-a and user-b share an app; user-c shares none but is on
/// the hub. bot_a is user-a's, bot_b user-b's, bot_c user-c's.
async fn world() -> DatabaseConnection {
    let db = database().await;
    for (id, owner) in [
        ("bot_a", "user-a"),
        ("bot_b", "user-b"),
        ("bot_c", "user-c"),
    ] {
        insert_bot(&db, id, owner, "SERVER").await;
    }
    share_app(&db, "user-a", "app-1").await;
    share_app(&db, "user-b", "app-1").await;
    share_app(&db, "user-c", "app-2").await;
    insert_user(&db, "user-b", "Bea").await;
    insert_user(&db, "user-c", "Cy").await;
    let plan = NewConversation {
        kind: ConversationKind::Group,
        title: Some("room".into()),
        members: Vec::new(),
        encryption: ConversationEncryption::Plain,
    };
    start_group(&db, "room", "user-a", &plan, &[], at(0))
        .await
        .unwrap();
    db
}

async fn invite(db: &DatabaseConnection, who: &[&str]) -> super::invitations::InviteOutcome {
    let ids: Vec<String> = who.iter().map(|id| id.to_string()).collect();
    let invitees = classify(db, "user-a", &ids).await.unwrap();
    invite_in(db, "room", "user-a", &invitees, &NO_LIMITS, at(10))
        .await
        .unwrap()
}

async fn create_group(
    db: &DatabaseConnection,
    id: &str,
    limits: BTreeMap<String, BotRoomLimits>,
) -> Result<super::invitations::InviteOutcome, ApiError> {
    let plan = NewConversation {
        kind: ConversationKind::Group,
        title: Some("Memory setup".into()),
        members: vec!["bot_a".into(), "bot_b".into(), "user-b".into()],
        encryption: ConversationEncryption::Plain,
    };
    let invitees = classify(db, "user-a", &plan.members).await?;
    retry_transaction(
        db,
        DbDialect::default(),
        None,
        &RetryPolicy::default(),
        |txn| {
            let (id, plan, invitees, limits) = (
                id.to_string(),
                plan.clone(),
                invitees.clone(),
                limits.clone(),
            );
            Box::pin(async move {
                create_group_in(txn, &id, "user-a", &plan, &invitees, &limits, at(10)).await
            })
        },
    )
    .await
}

#[tokio::test]
async fn group_creation_commits_initial_memory_limits_with_memberships() {
    let db = world().await;
    let limits = BotRoomLimits {
        mask: Some(1),
        use_owner_memory: true,
        use_shared_memory: true,
        share_memory: true,
        ..Default::default()
    };
    let outcome = create_group(
        &db,
        "new-room",
        BTreeMap::from([("bot_a".into(), limits.clone())]),
    )
    .await
    .unwrap();

    assert_eq!(outcome.added, ["bot_a"]);
    assert_eq!(outcome.invited.len(), 2);
    assert_eq!(room_limits(&db, "bot_a", "new-room").await.unwrap(), limits);
    assert!(
        conversation::Entity::find_by_id("new-room")
            .one(&db)
            .await
            .unwrap()
            .is_some()
    );
    for principal in ["bot_b", "user-b"] {
        let member = conversation_member::Entity::find_by_id((
            "new-room".to_string(),
            principal.to_string(),
        ))
        .one(&db)
        .await
        .unwrap()
        .unwrap();
        assert_eq!(member.status, MEMBER_PENDING);
        assert!(member.limits.is_none());
    }
}

#[tokio::test]
async fn group_creation_rolls_back_when_initial_memory_limits_are_refused() {
    let db = world().await;
    for (bot_id, limits, reason) in [
        (
            "bot_b",
            BotRoomLimits {
                share_memory: true,
                ..Default::default()
            },
            "only be set up front on your own bots",
        ),
        (
            "bot_a",
            BotRoomLimits {
                mask: Some(-1),
                use_owner_memory: true,
                ..Default::default()
            },
            "can't be negative",
        ),
    ] {
        let error = create_group(
            &db,
            "rejected-room",
            BTreeMap::from([(bot_id.into(), limits)]),
        )
        .await
        .err()
        .expect("invalid initial limits must reject group creation");
        assert_eq!(error.status(), StatusCode::BAD_REQUEST);
        assert!(says(&error, reason));
        assert!(
            conversation::Entity::find_by_id("rejected-room")
                .one(&db)
                .await
                .unwrap()
                .is_none()
        );
        assert!(
            conversation_member::Entity::find()
                .filter(conversation_member::Column::ConversationId.eq("rejected-room"))
                .all(&db)
                .await
                .unwrap()
                .is_empty()
        );
        assert!(
            inbox_entry::Entity::find()
                .filter(inbox_entry::Column::ConversationId.eq("rejected-room"))
                .all(&db)
                .await
                .unwrap()
                .is_empty()
        );
    }
}

async fn status_of(db: &DatabaseConnection, principal: &str) -> Option<String> {
    conversation_member::Entity::find_by_id(("room".to_string(), principal.to_string()))
        .one(db)
        .await
        .unwrap()
        .map(|row| row.status)
}

async fn inbox_ids(db: &DatabaseConnection, message_id: &str) -> Vec<String> {
    copies(db, message_id)
        .await
        .into_iter()
        .map(|copy| copy.inbox_id)
        .collect()
}

#[tokio::test]
async fn an_invited_person_receives_nothing_until_they_accept() {
    let db = world().await;
    let outcome = invite(&db, &["user-b"]).await;
    assert_eq!(outcome.invited.len(), 1);
    assert!(outcome.added.is_empty());
    assert_eq!(
        status_of(&db, "user-b").await.as_deref(),
        Some(MEMBER_PENDING)
    );

    send_in(&db, "room", "user-a", &text("before"), at(11))
        .await
        .unwrap();
    assert_eq!(inbox_ids(&db, "before").await, ["user-a"]);
    let listed = conversations_for(&db, &Reader::own("user-b"), None)
        .await
        .unwrap();
    assert!(listed.is_empty(), "a pending invitee reads no conversation");
    let seen = conversations_for(&db, &Reader::own("user-a"), Some("room"))
        .await
        .unwrap();
    assert_eq!(seen[0].members.len(), 1);
    assert_eq!(seen[0].pending.len(), 1);
    assert_eq!(seen[0].pending[0].principal_id, "user-b");

    let woken = accept_in(&db, "room", "user-b", "user-b", None, at(20))
        .await
        .unwrap();
    assert_eq!(woken, ["user-a", "user-b"]);
    assert_eq!(
        status_of(&db, "user-b").await.as_deref(),
        Some(MEMBER_ACTIVE)
    );
    let joins: Vec<_> = conversation_member_events(&db, "user-b").await;
    assert_eq!(joins, ["user-b"], "the join entry reaches the new member");
    send_in(&db, "room", "user-a", &text("after"), at(21))
        .await
        .unwrap();
    assert_eq!(inbox_ids(&db, "after").await, ["user-a", "user-b"]);
    assert!(invitations_for(&db, "user-b").await.unwrap().is_empty());
}

async fn conversation_member_events(db: &DatabaseConnection, inbox: &str) -> Vec<String> {
    use crate::entity::inbox_entry;
    inbox_entry::Entity::find()
        .filter(inbox_entry::Column::InboxId.eq(inbox))
        .filter(inbox_entry::Column::Kind.eq("MEMBER"))
        .all(db)
        .await
        .unwrap()
        .into_iter()
        .map(|entry| {
            entry.envelope["body"]["principal_id"]
                .as_str()
                .unwrap()
                .to_string()
        })
        .collect()
}

#[tokio::test]
async fn no_member_entry_is_written_before_accepting() {
    let db = world().await;
    invite(&db, &["user-b"]).await;
    assert!(conversation_member_events(&db, "user-a").await.is_empty());
    decline_in(&db, "room", "user-b", "user-b").await.unwrap();
    assert!(conversation_member_events(&db, "user-a").await.is_empty());
}

#[tokio::test]
async fn only_the_invitee_answers_and_declining_stops_delivery() {
    let db = world().await;
    invite(&db, &["user-b"]).await;
    for stranger in ["user-a", "user-c"] {
        let error = accept_in(&db, "room", "user-b", stranger, None, at(20))
            .await
            .unwrap_err();
        assert_eq!(error.status(), StatusCode::NOT_FOUND, "{stranger} accepts");
        let error = decline_in(&db, "room", "user-b", stranger)
            .await
            .unwrap_err();
        assert_eq!(error.status(), StatusCode::NOT_FOUND, "{stranger} declines");
    }
    assert_eq!(
        status_of(&db, "user-b").await.as_deref(),
        Some(MEMBER_PENDING)
    );

    assert_eq!(
        decline_in(&db, "room", "user-b", "user-b").await.unwrap(),
        ["user-a"]
    );
    assert_eq!(
        status_of(&db, "user-b").await.as_deref(),
        Some(MEMBER_DECLINED)
    );
    send_in(&db, "room", "user-a", &text("m"), at(21))
        .await
        .unwrap();
    assert_eq!(inbox_ids(&db, "m").await, ["user-a"]);
    let again = accept_in(&db, "room", "user-b", "user-b", None, at(22))
        .await
        .unwrap_err();
    assert_eq!(
        again.status(),
        StatusCode::NOT_FOUND,
        "a declined invitation can't be accepted"
    );

    invite(&db, &["user-b"]).await;
    assert_eq!(
        status_of(&db, "user-b").await.as_deref(),
        Some(MEMBER_PENDING)
    );
}

#[tokio::test]
async fn the_inviter_or_the_creator_cancels() {
    let db = world().await;
    share_app(&db, "user-d", "app-1").await;
    let plan = NewConversation {
        kind: ConversationKind::Group,
        title: None,
        members: Vec::new(),
        encryption: ConversationEncryption::Plain,
    };
    start_group(&db, "other", "user-d", &plan, &[user("user-a")], at(1))
        .await
        .unwrap();
    let invitees = classify(&db, "user-a", &["user-b".to_string()])
        .await
        .unwrap();
    invite_in(&db, "other", "user-a", &invitees, &NO_LIMITS, at(10))
        .await
        .unwrap();

    let by_stranger = cancel_in(&db, "other", "user-b", "user-c")
        .await
        .unwrap_err();
    assert_code(
        &by_stranger,
        StatusCode::FORBIDDEN,
        INBOX_INVITE_NOT_ALLOWED,
    );
    cancel_in(&db, "other", "user-b", "user-d").await.unwrap();
    let gone = cancel_in(&db, "other", "user-b", "user-d")
        .await
        .unwrap_err();
    assert_eq!(gone.status(), StatusCode::NOT_FOUND);

    invite(&db, &["user-b"]).await;
    cancel_in(&db, "room", "user-b", "user-a").await.unwrap();
    assert_eq!(status_of(&db, "user-b").await, None);
}

#[tokio::test]
async fn another_persons_bot_joins_only_when_its_owner_accepts_with_limits() {
    let db = world().await;
    invite(&db, &["bot_b"]).await;
    assert_eq!(
        status_of(&db, "bot_b").await.as_deref(),
        Some(MEMBER_PENDING)
    );
    assert!(
        room_limits(&db, "bot_b", "room").await.is_err(),
        "pending grants nothing"
    );
    let mine = invitations_for(&db, "user-b").await.unwrap();
    assert_eq!(
        (
            mine.len(),
            mine[0].principal_id.as_str(),
            mine[0].kind.as_str()
        ),
        (1, "bot_b", "bot")
    );
    assert!(invitations_for(&db, "user-a").await.unwrap().is_empty());

    for not_owner in ["user-a", "user-c", "bot_b"] {
        let error = accept_in(&db, "room", "bot_b", not_owner, None, at(20))
            .await
            .unwrap_err();
        assert_eq!(error.status(), StatusCode::NOT_FOUND, "{not_owner}");
    }
    assert_eq!(
        status_of(&db, "bot_b").await.as_deref(),
        Some(MEMBER_PENDING)
    );

    let limits = BotRoomLimits {
        mask: Some(1),
        apps: Some(vec!["app-1".into()]),
        use_owner_memory: true,
        use_shared_memory: true,
        share_memory: true,
        ..Default::default()
    };
    accept_in(&db, "room", "bot_b", "user-b", Some(&limits), at(20))
        .await
        .unwrap();
    assert_eq!(
        status_of(&db, "bot_b").await.as_deref(),
        Some(MEMBER_ACTIVE)
    );
    assert_eq!(room_limits(&db, "bot_b", "room").await.unwrap(), limits);

    let own = conversations_for(&db, &Reader::own("user-b"), None)
        .await
        .unwrap();
    assert!(
        own.is_empty(),
        "the owner is not a member just because the bot is"
    );
    let seen_by_owner = conversations_for(&db, &Reader::own("user-a"), Some("room"))
        .await
        .unwrap();
    assert!(
        seen_by_owner[0]
            .members
            .iter()
            .all(|member| member.limits.is_none())
    );
    let as_bot = super::reader(&db, "user-b", Some("bot_b")).await.unwrap();
    let owners_view = conversations_for(&db, &as_bot, Some("room")).await.unwrap();
    let bot = owners_view[0]
        .members
        .iter()
        .find(|m| m.principal_id == "bot_b")
        .unwrap();
    assert!(bot.limits.is_some(), "the bot's owner sees the limits");
}

#[tokio::test]
async fn limits_are_edited_by_the_bots_owner_and_apply_next_call() {
    let db = world().await;
    invite(&db, &["bot_b"]).await;
    accept_in(&db, "room", "bot_b", "user-b", None, at(20))
        .await
        .unwrap();
    assert_eq!(
        room_limits(&db, "bot_b", "room").await.unwrap(),
        BotRoomLimits::default()
    );

    let narrow = BotRoomLimits {
        mask: Some(0),
        use_shared_memory: true,
        ..Default::default()
    };
    let refused = set_limits_in(&db, "room", "bot_b", "user-a", Some(&narrow))
        .await
        .unwrap_err();
    assert_eq!(refused.status(), StatusCode::NOT_FOUND);
    assert_eq!(
        room_limits(&db, "bot_b", "room").await.unwrap(),
        BotRoomLimits::default()
    );

    set_limits_in(&db, "room", "bot_b", "user-b", Some(&narrow))
        .await
        .unwrap();
    assert_eq!(room_limits(&db, "bot_b", "room").await.unwrap(), narrow);
    set_limits_in(&db, "room", "bot_b", "user-b", None)
        .await
        .unwrap();
    assert_eq!(
        room_limits(&db, "bot_b", "room").await.unwrap(),
        BotRoomLimits::default()
    );

    let absent = set_limits_in(&db, "room", "bot_a", "user-a", Some(&narrow))
        .await
        .unwrap_err();
    assert_eq!(
        absent.status(),
        StatusCode::NOT_FOUND,
        "a bot that isn't in the room"
    );
    let negative = BotRoomLimits {
        mask: Some(-1),
        ..Default::default()
    };
    let bad = set_limits_in(&db, "room", "bot_b", "user-b", Some(&negative))
        .await
        .unwrap_err();
    assert_eq!(bad.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn a_persons_invitation_takes_no_limits_and_own_bots_join_at_once() {
    let db = world().await;
    invite(&db, &["user-b"]).await;
    let limits = BotRoomLimits {
        mask: Some(1),
        use_owner_memory: true,
        share_memory: true,
        ..Default::default()
    };
    let error = accept_in(&db, "room", "user-b", "user-b", Some(&limits), at(20))
        .await
        .unwrap_err();
    assert_eq!(error.status(), StatusCode::BAD_REQUEST);

    let ids = vec!["bot_a".to_string()];
    let invitees = classify(&db, "user-a", &ids).await.unwrap();
    assert_eq!(invitees[0].mode, InviteMode::AddNow);
    let up_front = BTreeMap::from([("bot_a".to_string(), limits.clone())]);
    let outcome = invite_in(&db, "room", "user-a", &invitees, &up_front, at(12))
        .await
        .unwrap();
    assert_eq!(outcome.added, ["bot_a"]);
    assert_eq!(room_limits(&db, "bot_a", "room").await.unwrap(), limits);

    let stray = BTreeMap::from([("bot_b".to_string(), limits)]);
    let invitees = classify(&db, "user-a", &["bot_b".to_string()])
        .await
        .unwrap();
    let error = invite_in(&db, "room", "user-a", &invitees, &stray, at(13))
        .await
        .unwrap_err();
    assert_eq!(error.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn invitable_suggests_what_the_server_accepts_and_strangers_are_invited_too() {
    let db = world().await;
    insert_bot(&db, "bot_gone", "user-b", "SERVER").await;
    delete_bot(&db, "bot_gone").await;
    let listed = invitable(&db, "user-a", None, None).await.unwrap();
    let modes: BTreeMap<&str, InviteMode> = listed
        .iter()
        .map(|entry| (entry.principal_id.as_str(), entry.mode))
        .collect();
    assert_eq!(
        modes,
        BTreeMap::from([
            ("bot_a", InviteMode::AddNow),
            ("user-b", InviteMode::Invite),
            ("bot_b", InviteMode::Invite),
        ])
    );
    for entry in &listed {
        let accepted = classify(&db, "user-a", std::slice::from_ref(&entry.principal_id))
            .await
            .unwrap();
        assert_eq!(accepted[0].mode, entry.mode, "{}", entry.principal_id);
    }
    for (stranger, kind) in [
        ("user-c", PrincipalKind::User),
        ("bot_c", PrincipalKind::Bot),
    ] {
        let invitee = classify(&db, "user-a", &[stranger.to_string()])
            .await
            .unwrap();
        assert_eq!(invitee[0].mode, InviteMode::Invite, "{stranger}");
        assert_eq!(invitee[0].principal.kind, kind, "{stranger}");
    }
    for refused in ["bot_gone", "nobody"] {
        let error = classify(&db, "user-a", &[refused.to_string()])
            .await
            .unwrap_err();
        assert_code(&error, StatusCode::FORBIDDEN, INBOX_MEMBER_NOT_ALLOWED);
        assert!(says(&error, refused));
    }

    let named = invitable(&db, "user-a", Some("be"), None).await.unwrap();
    assert_eq!(named.len(), 1);
    assert_eq!(named[0].name.as_deref(), Some("Bea"));

    invite(&db, &["user-b"]).await;
    let marked = invitable(&db, "user-a", None, Some("room")).await.unwrap();
    let state = |id: &str| {
        marked
            .iter()
            .find(|e| e.principal_id == id)
            .unwrap()
            .state
            .clone()
    };
    assert_eq!(state("user-b").as_deref(), Some("invited"));
    assert_eq!(state("bot_b"), None);
}

#[tokio::test]
async fn a_conversation_holds_at_most_fifty_pending_invitations() {
    let db = world().await;
    let ids: Vec<String> = (0..51).map(|n| format!("guest-{n:02}")).collect();
    for id in &ids {
        share_app(&db, id, "app-1").await;
    }
    let invitees = classify(&db, "user-a", &ids[..50]).await.unwrap();
    invite_in(&db, "room", "user-a", &invitees, &NO_LIMITS, at(10))
        .await
        .unwrap();
    let more = classify(&db, "user-a", &ids[50..]).await.unwrap();
    let error = invite_in(&db, "room", "user-a", &more, &NO_LIMITS, at(11))
        .await
        .unwrap_err();
    assert_code(&error, StatusCode::CONFLICT, INBOX_INVITE_LIMIT);
    let again = invite_in(&db, "room", "user-a", &invitees, &NO_LIMITS, at(12))
        .await
        .unwrap();
    assert!(
        again.invited.is_empty(),
        "invitations already pending are skipped"
    );
}

#[tokio::test]
async fn only_members_invite() {
    let db = world().await;
    share_app(&db, "user-d", "app-1").await;
    let invitees = classify(&db, "user-d", &["user-b".to_string()])
        .await
        .unwrap();
    let error = invite_in(&db, "room", "user-d", &invitees, &NO_LIMITS, at(10))
        .await
        .unwrap_err();
    assert_eq!(error.status(), StatusCode::NOT_FOUND);
}
