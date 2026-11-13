use super::{BOT_INVITE_REFUSED, InvitePlan, plan_invite, settle_invite};
use crate::{
    entity::conversation_member,
    routes::inbox::{
        MEMBER_PENDING,
        blocks::block_in,
        fixture::{assert_code, at, bot_principal, database, group, insert_bot, user},
        invitations::{cancel_in, invite_in},
    },
};
use axum::http::StatusCode;
use flow_like_bot_tools::{InviteOutcome, InviteTarget};
use flow_like_types::inbox::ConversationEncryption;
use sea_orm::{ConnectionTrait, DatabaseConnection, EntityTrait};

const ROOM: &str = "room";
const BOT: &str = "olga-bot";

async fn world() -> DatabaseConnection {
    let db = database().await;
    db.execute_unprepared(
        r#"INSERT INTO public."User" ("id", "name", "email", "username", "preferredUsername") VALUES
            ('olga', 'Olga', 'olga@example.org', 'google_1', 'olga'),
            ('mia', 'Mia', 'mia@example.org', 'google_2', 'mia'),
            ('ana', 'Ana Lima', 'Ana.Lima@Example.org', 'google_3', 'ana.lima'),
            ('legacy', NULL, 'legacy@example.org', 'google_4', 'google_4'),
            ('twin-a', 'Twin A', 'twin@example.org', 'google_5', 'twin-a'),
            ('twin-b', 'Twin B', 'TWIN@example.org', 'google_6', 'twin-b')"#,
    )
    .await
    .unwrap();
    insert_bot(&db, BOT, "olga", "SERVER").await;
    group(
        &db,
        ROOM,
        "mia",
        &[bot_principal(BOT)],
        ConversationEncryption::Plain,
    )
    .await;
    db
}

fn target(value: &str) -> InviteTarget {
    InviteTarget::parse(value).expect(value)
}

async fn plan(db: &DatabaseConnection, value: &str) -> InvitePlan {
    plan_invite(db, BOT, ROOM, &target(value)).await.unwrap()
}

fn invited_id(plan: &InvitePlan) -> &str {
    match plan {
        InvitePlan::Invite { invitee, .. } => &invitee.principal.id,
        InvitePlan::Settled(outcome) => panic!("expected an invitation, got {outcome:?}"),
    }
}

fn settled(plan: InvitePlan) -> InviteOutcome {
    match plan {
        InvitePlan::Settled(outcome) => outcome,
        InvitePlan::Invite { invitee, .. } => panic!("expected no write for {invitee:?}"),
    }
}

#[tokio::test]
async fn a_username_or_an_address_resolves_exactly_ignoring_case() {
    let db = world().await;
    for value in [
        "ana.lima",
        "@ANA.LIMA",
        "ana.lima@example.org",
        "ANA.LIMA@EXAMPLE.ORG",
    ] {
        let plan = plan(&db, value).await;
        assert_eq!(invited_id(&plan), "ana", "{value}");
        let InvitePlan::Invite { name, .. } = plan else {
            unreachable!()
        };
        assert_eq!(name, "Ana Lima");
    }
}

#[tokio::test]
async fn nothing_short_of_the_whole_username_or_address_finds_anyone() {
    let db = world().await;
    for value in [
        "ana",
        "lima",
        "Ana",
        "ana.lim",
        "ana.lima@example.com",
        "google_3",
        "google_4",
    ] {
        assert_eq!(
            settled(plan(&db, value).await),
            InviteOutcome::NoSuchPerson,
            "{value}"
        );
    }
}

#[tokio::test]
async fn an_address_two_accounts_share_asks_for_a_username() {
    let db = world().await;
    let InviteOutcome::NotInvitable { why } = settled(plan(&db, "twin@example.org").await) else {
        panic!("expected a refusal");
    };
    assert!(why.contains("username"), "{why}");
    assert_eq!(invited_id(&plan(&db, "twin-b").await), "twin-b");
}

#[tokio::test]
async fn members_and_open_invitations_are_reported_by_name_without_a_write() {
    let db = world().await;
    assert_eq!(
        settled(plan(&db, "mia").await),
        InviteOutcome::AlreadyMember { name: "Mia".into() }
    );
    let InvitePlan::Invite { invitee, name } = plan(&db, "ana.lima").await else {
        panic!("expected an invitation");
    };
    let outcome = invite_in(&db, ROOM, BOT, &[invitee], &Default::default(), at(1))
        .await
        .unwrap();
    assert_eq!(outcome.invited.len(), 1);
    assert_eq!(
        settle_invite(&db, ROOM, "ana", name, true).await.unwrap(),
        InviteOutcome::Invited {
            name: "Ana Lima".into()
        }
    );
    let row = conversation_member::Entity::find_by_id((ROOM.to_string(), "ana".to_string()))
        .one(&db)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(row.status, MEMBER_PENDING);
    assert_eq!(row.invited_by.as_deref(), Some(BOT));
    assert_eq!(
        settled(plan(&db, "ana.lima@example.org").await),
        InviteOutcome::AlreadyInvited {
            name: "Ana Lima".into()
        }
    );
}

#[tokio::test]
async fn a_person_who_blocked_the_owner_gets_no_invitation_from_the_bot() {
    let db = world().await;
    block_in(&db, "ana", "olga", at(1)).await.unwrap();
    let InvitePlan::Invite { invitee, name } = plan(&db, "ana.lima").await else {
        panic!("the plan does not read blocks");
    };
    let outcome = invite_in(&db, ROOM, BOT, &[invitee], &Default::default(), at(2))
        .await
        .unwrap();
    assert!(outcome.invited.is_empty());
    assert!(matches!(
        settle_invite(&db, ROOM, "ana", name, false).await.unwrap(),
        InviteOutcome::NotInvitable { .. }
    ));
}

#[tokio::test]
async fn only_a_bot_in_a_group_invites() {
    let db = world().await;
    insert_bot(&db, "outsider", "olga", "SERVER").await;
    let error = plan_invite(&db, "outsider", ROOM, &target("ana.lima"))
        .await
        .unwrap_err();
    assert_eq!(error.status(), StatusCode::NOT_FOUND, "{error:?}");
    let direct = crate::routes::inbox::conversations::start_direct(
        &db,
        "olga",
        &bot_principal(BOT),
        ConversationEncryption::Plain,
        at(1),
    )
    .await
    .unwrap();
    let error = plan_invite(&db, BOT, &direct, &target("ana.lima"))
        .await
        .unwrap_err();
    assert_code(&error, StatusCode::CONFLICT, BOT_INVITE_REFUSED);
    group(
        &db,
        "sealed",
        "mia",
        &[bot_principal(BOT)],
        ConversationEncryption::E2e,
    )
    .await;
    let error = plan_invite(&db, BOT, "sealed", &target("ana.lima"))
        .await
        .unwrap_err();
    assert_code(&error, StatusCode::CONFLICT, BOT_INVITE_REFUSED);
}

#[tokio::test]
async fn the_bots_owner_may_withdraw_its_invitation_and_a_stranger_may_not() {
    let db = world().await;
    let invitee = vec![crate::routes::inbox::invitations::Invitee {
        principal: user("ana"),
        mode: crate::routes::inbox::invitations::InviteMode::Invite,
        owner_id: None,
    }];
    invite_in(&db, ROOM, BOT, &invitee, &Default::default(), at(1))
        .await
        .unwrap();
    assert!(cancel_in(&db, ROOM, "ana", "olga-stranger").await.is_err());
    cancel_in(&db, ROOM, "ana", "olga").await.unwrap();
    assert!(
        conversation_member::Entity::find_by_id((ROOM.to_string(), "ana".to_string()))
            .one(&db)
            .await
            .unwrap()
            .is_none()
    );
}
