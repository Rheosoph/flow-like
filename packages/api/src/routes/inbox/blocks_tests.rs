use super::{
    MEMBER_BLOCKED, Reader,
    blocks::{INBOX_BLOCKED, block_in, blocked_between, blocks_for, unblock_in},
    conversations::{NewConversation, start_direct, start_group},
    entries::send_in,
    fixture::{assert_code, at, bot_principal, copies, database, insert_bot, text, user},
    invitations::{accept_in, classify, invitations_for, invite_in},
    members::remove_in,
    requests::answer_by_writing,
    view::conversations_for,
};
use crate::entity::conversation_member;
use axum::http::StatusCode;
use flow_like_types::inbox::{ConversationEncryption, ConversationKind};
use sea_orm::{
    ColumnTrait, ConnectionTrait, DatabaseConnection, EntityTrait, QueryFilter, sea_query::Expr,
};

async fn world() -> DatabaseConnection {
    let db = database().await;
    db.execute_unprepared(r#"INSERT INTO public."User" ("id", "name") VALUES ('alice', 'Alice'), ('bob', 'Bob'), ('carol', 'Carol')"#).await.unwrap();
    insert_bot(&db, "alice-bot", "alice", "SERVER").await;
    insert_bot(&db, "bob-bot", "bob", "SERVER").await;
    db
}

fn group() -> NewConversation {
    NewConversation {
        kind: ConversationKind::Group,
        title: Some("Room".into()),
        members: vec![],
        encryption: ConversationEncryption::Plain,
    }
}

#[tokio::test]
async fn active_directs_stop_in_both_directions_keep_history_and_resume_after_unblock() {
    let db = world().await;
    let id = start_direct(
        &db,
        "alice",
        &user("bob"),
        ConversationEncryption::Plain,
        at(0),
    )
    .await
    .unwrap();
    send_in(&db, &id, "alice", &text("before"), at(1))
        .await
        .unwrap();
    block_in(&db, "bob", "alice", at(2)).await.unwrap();
    for sender in ["alice", "bob"] {
        let err = send_in(&db, &id, sender, &text(sender), at(3))
            .await
            .unwrap_err();
        assert_code(&err, StatusCode::FORBIDDEN, INBOX_BLOCKED);
        assert!(copies(&db, sender).await.is_empty());
    }
    assert_eq!(copies(&db, "before").await.len(), 2);
    assert_eq!(
        conversations_for(&db, &Reader::own("bob"), None)
            .await
            .unwrap()
            .len(),
        1
    );
    let err = start_direct(
        &db,
        "alice",
        &user("bob"),
        ConversationEncryption::Plain,
        at(4),
    )
    .await
    .unwrap_err();
    assert_code(&err, StatusCode::FORBIDDEN, INBOX_BLOCKED);
    assert!(answer_by_writing(&db, &id, "bob", at(4)).await.is_err());
    unblock_in(&db, "bob", "alice").await.unwrap();
    send_in(&db, &id, "alice", &text("after"), at(5))
        .await
        .unwrap();
    assert_eq!(copies(&db, "after").await.len(), 2);
}

#[tokio::test]
async fn user_blocks_cover_both_accounts_bots_including_bots_created_later() {
    let db = world().await;
    block_in(&db, "alice", "bob", at(0)).await.unwrap();
    insert_bot(&db, "future-bot", "bob", "SERVER").await;
    for a in ["alice", "alice-bot"] {
        for b in ["bob", "bob-bot", "future-bot"] {
            assert!(
                blocked_between(&db, a, &[b.into()], false)
                    .await
                    .unwrap()
                    .contains(b)
            );
            assert!(
                blocked_between(&db, b, &[a.into()], false)
                    .await
                    .unwrap()
                    .contains(a)
            );
        }
    }
    assert!(
        blocked_between(&db, "alice", &["carol".into(), "alice-bot".into()], false)
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn a_bot_block_stops_its_replies_without_blocking_its_owner_or_sibling() {
    let db = world().await;
    insert_bot(&db, "sibling", "bob", "SERVER").await;
    let id = start_direct(
        &db,
        "alice",
        &bot_principal("bob-bot"),
        ConversationEncryption::Plain,
        at(0),
    )
    .await
    .unwrap();
    block_in(&db, "alice", "bob-bot", at(1)).await.unwrap();
    let err = send_in(&db, &id, "bob-bot", &text("reply"), at(2))
        .await
        .unwrap_err();
    assert_code(&err, StatusCode::FORBIDDEN, INBOX_BLOCKED);
    assert!(copies(&db, "reply").await.is_empty());
    assert!(
        blocked_between(&db, "alice", &["bob".into(), "sibling".into()], false)
            .await
            .unwrap()
            .is_empty()
    );
    assert_eq!(blocks_for(&db, "alice").await.unwrap()[0].kind, "bot");
    assert!(blocks_for(&db, "bob").await.unwrap().is_empty());
}

#[tokio::test]
async fn blocked_invitations_are_hidden_cannot_be_accepted_and_are_not_reissued() {
    let db = world().await;
    start_group(&db, "room", "bob", &group(), &[], at(0))
        .await
        .unwrap();
    let invited = classify(&db, "bob", &["alice".into(), "alice-bot".into()])
        .await
        .unwrap();
    invite_in(&db, "room", "bob", &invited, &Default::default(), at(1))
        .await
        .unwrap();
    assert_eq!(invitations_for(&db, "alice").await.unwrap().len(), 2);
    block_in(&db, "alice", "bob", at(2)).await.unwrap();
    assert!(invitations_for(&db, "alice").await.unwrap().is_empty());
    for principal in ["alice", "alice-bot"] {
        let err = accept_in(&db, "room", principal, "alice", None, at(3))
            .await
            .unwrap_err();
        assert_code(&err, StatusCode::FORBIDDEN, INBOX_BLOCKED);
    }
    start_group(&db, "other-room", "bob", &group(), &[], at(4))
        .await
        .unwrap();
    assert!(
        invite_in(
            &db,
            "other-room",
            "bob",
            &invited,
            &Default::default(),
            at(4)
        )
        .await
        .unwrap()
        .invited
        .is_empty()
    );
}

#[tokio::test]
async fn blocks_are_idempotent_reject_self_and_own_bots_and_preserve_other_owners_blocks() {
    let db = world().await;
    for target in ["alice", "alice-bot"] {
        assert_eq!(
            block_in(&db, "alice", target, at(0))
                .await
                .unwrap_err()
                .status(),
            StatusCode::BAD_REQUEST
        );
    }
    assert_eq!(
        block_in(&db, "alice", "missing", at(0))
            .await
            .unwrap_err()
            .status(),
        StatusCode::NOT_FOUND
    );
    block_in(&db, "alice", "bob", at(1)).await.unwrap();
    block_in(&db, "alice", "bob", at(2)).await.unwrap();
    block_in(&db, "carol", "bob", at(3)).await.unwrap();
    let blocks = blocks_for(&db, "alice").await.unwrap();
    assert_eq!(blocks.len(), 1);
    assert_eq!(blocks[0].created_at, at(1));
    unblock_in(&db, "alice", "bob").await.unwrap();
    unblock_in(&db, "alice", "bob").await.unwrap();
    assert!(blocks_for(&db, "alice").await.unwrap().is_empty());
    assert_eq!(blocks_for(&db, "carol").await.unwrap().len(), 1);
}

#[tokio::test]
async fn legacy_request_blocks_are_listed_enforced_and_can_be_unblocked() {
    let db = world().await;
    let id = super::conversations::start_direct_as(
        &db,
        "bob",
        &user("alice"),
        ConversationEncryption::Plain,
        true,
        at(0),
    )
    .await
    .unwrap();
    conversation_member::Entity::update_many()
        .col_expr(
            conversation_member::Column::Status,
            Expr::value(MEMBER_BLOCKED),
        )
        .filter(conversation_member::Column::ConversationId.eq(&id))
        .filter(conversation_member::Column::PrincipalId.eq("alice"))
        .exec(&db)
        .await
        .unwrap();
    assert_eq!(
        blocks_for(&db, "alice").await.unwrap()[0].principal_id,
        "bob"
    );
    assert!(answer_by_writing(&db, &id, "alice", at(1)).await.is_err());
    unblock_in(&db, "alice", "bob").await.unwrap();
    answer_by_writing(&db, &id, "alice", at(2)).await.unwrap();
    assert!(blocks_for(&db, "alice").await.unwrap().is_empty());
}

#[tokio::test]
async fn shared_rooms_stay_intact_and_leaving_removes_your_bots_and_future_delivery() {
    let db = world().await;
    start_group(
        &db,
        "room",
        "bob",
        &group(),
        &[user("alice"), bot_principal("alice-bot")],
        at(0),
    )
    .await
    .unwrap();
    block_in(&db, "alice", "bob", at(1)).await.unwrap();
    send_in(&db, "room", "bob", &text("shared"), at(2))
        .await
        .unwrap();
    assert_eq!(copies(&db, "shared").await.len(), 3);
    remove_in(&db, "room", "alice", "alice", at(3))
        .await
        .unwrap();
    send_in(&db, "room", "bob", &text("left"), at(4))
        .await
        .unwrap();
    assert_eq!(copies(&db, "left").await.len(), 1);
    assert!(
        send_in(&db, "room", "alice", &text("no-access"), at(5))
            .await
            .is_err()
    );
}

#[tokio::test]
async fn blocked_directs_cannot_issue_realtime_keys_or_create_outreach_rooms() {
    let db = world().await;
    let id = start_direct(
        &db,
        "alice",
        &user("bob"),
        ConversationEncryption::Plain,
        at(0),
    )
    .await
    .unwrap();
    block_in(&db, "bob", "alice", at(1)).await.unwrap();
    let key = super::presence::room_key_in(&db, &[7; 32], &id, "alice", at(2).date_naive()).await;
    assert_eq!(key.err().unwrap().public_code(), INBOX_BLOCKED);
    assert!(
        super::outreach::contacts_in(&db, "alice")
            .await
            .unwrap()
            .is_empty()
    );
    let err =
        super::outreach::start_outreach_in(&db, "alice", "alice-bot", "bob", "Hi".into(), at(2))
            .await
            .unwrap_err();
    assert_code(&err, StatusCode::FORBIDDEN, INBOX_BLOCKED);
}

#[tokio::test]
async fn blocked_invitations_do_not_fill_the_visible_page() {
    let db = world().await;
    start_group(&db, "allowed-room", "carol", &group(), &[], at(0))
        .await
        .unwrap();
    let allowed = classify(&db, "carol", &["alice".into()]).await.unwrap();
    invite_in(
        &db,
        "allowed-room",
        "carol",
        &allowed,
        &Default::default(),
        at(1),
    )
    .await
    .unwrap();

    let invited = classify(&db, "bob", &["alice".into(), "alice-bot".into()])
        .await
        .unwrap();
    for index in 0..101 {
        let room = format!("blocked-room-{index:03}");
        start_group(&db, &room, "bob", &group(), &[], at(0))
            .await
            .unwrap();
        invite_in(&db, &room, "bob", &invited, &Default::default(), at(2))
            .await
            .unwrap();
    }
    let before = invitations_for(&db, "alice").await.unwrap();
    assert_eq!(before.len(), 200);
    assert!(
        before
            .iter()
            .all(|invitation| invitation.invited_by.as_deref() == Some("bob"))
    );

    block_in(&db, "alice", "bob", at(3)).await.unwrap();
    let visible = invitations_for(&db, "alice").await.unwrap();
    assert_eq!(visible.len(), 1);
    assert_eq!(visible[0].conversation_id, "allowed-room");
    assert_eq!(visible[0].principal_id, "alice");
    assert_eq!(visible[0].invited_by.as_deref(), Some("carol"));

    unblock_in(&db, "alice", "bob").await.unwrap();
    assert_eq!(invitations_for(&db, "alice").await.unwrap().len(), 200);
}
