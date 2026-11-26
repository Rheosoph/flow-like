use super::{
    INBOX_DIRECT_FIXED, INBOX_LINK_LIMIT, INBOX_LINK_NOT_ALLOWED, INBOX_LINK_NOT_FOUND,
    INBOX_LINK_USED_UP, INBOX_MEMBER_LIMIT, MEMBER_ACTIVE, MEMBER_PENDING,
    conversations::{NewConversation, start_direct, start_group},
    fixture::{assert_code, at, database, insert_bot, user},
    invitations::{classify, invite_in},
    links::{
        CreateConversationLinkRequest, EmailInvite, LinkJoin, MAX_LINKS,
        MAX_OWNER_EMAIL_INVITES_PER_DAY, NewLink, create_email_invite_in, create_link_in,
        join_link_in, links_in, live_link, revoke_link_in, validate_link,
    },
    members::remove_in,
};
use crate::entity::{conversation_invite_link, conversation_member, inbox_entry};
use axum::http::StatusCode;
use chrono::Duration;
use flow_like_bot_tools::rooms::EMAIL_INVITE_DAYS;
use flow_like_types::inbox::{ConversationEncryption, ConversationKind, MAX_CONVERSATION_MEMBERS};
use sea_orm::{ColumnTrait, ConnectionTrait, DatabaseConnection, EntityTrait, QueryFilter};
use std::collections::BTreeMap;

const UNLIMITED: NewLink = NewLink {
    name: None,
    max_uses: -1,
    expires_in: None,
};

fn group(encryption: ConversationEncryption) -> NewConversation {
    NewConversation {
        kind: ConversationKind::Group,
        title: Some("crew".into()),
        members: Vec::new(),
        encryption,
    }
}

/// `room`: a plain group user-a started with user-b and user-c. `sealed`: an encrypted group of
/// user-a. Nobody else is in either.
async fn world() -> DatabaseConnection {
    let db = database().await;
    db.execute_unprepared(
        r#"CREATE TABLE public."ConversationInviteLink" (
            "id" TEXT PRIMARY KEY, "conversationId" TEXT NOT NULL, "token" TEXT NOT NULL UNIQUE,
            "name" TEXT, "createdBy" TEXT NOT NULL, "maxUses" INTEGER NOT NULL,
            "countJoined" INTEGER NOT NULL, "expiresAt" TEXT, "createdAt" TEXT NOT NULL,
            "updatedAt" TEXT NOT NULL, "email" TEXT, "reason" TEXT)"#,
    )
    .await
    .unwrap();
    db.execute_unprepared(
        r#"INSERT INTO public."User" ("id", "name") VALUES ('stranger', 'Sam'), ('other', 'Oli')"#,
    )
    .await
    .unwrap();
    start_group(
        &db,
        "room",
        "user-a",
        &group(ConversationEncryption::Plain),
        &[user("user-b"), user("user-c")],
        at(0),
    )
    .await
    .unwrap();
    start_group(
        &db,
        "sealed",
        "user-a",
        &group(ConversationEncryption::E2e),
        &[],
        at(0),
    )
    .await
    .unwrap();
    db
}

async fn link(
    db: &DatabaseConnection,
    creator: &str,
    shape: &NewLink,
) -> conversation_invite_link::Model {
    create_link_in(db, "room", creator, shape, at(10))
        .await
        .unwrap()
}

async fn member(db: &DatabaseConnection, principal: &str) -> Option<conversation_member::Model> {
    conversation_member::Entity::find_by_id(("room".to_string(), principal.to_string()))
        .one(db)
        .await
        .unwrap()
}

async fn uses(db: &DatabaseConnection, link_id: &str) -> i64 {
    conversation_invite_link::Entity::find_by_id(link_id)
        .one(db)
        .await
        .unwrap()
        .unwrap()
        .count_joined
}

/// `(principal_id, by)` of every member entry in `inbox`.
async fn member_entries(db: &DatabaseConnection, inbox: &str) -> Vec<(String, String)> {
    inbox_entry::Entity::find()
        .filter(inbox_entry::Column::InboxId.eq(inbox))
        .filter(inbox_entry::Column::ConversationId.eq("room"))
        .filter(inbox_entry::Column::Kind.eq("MEMBER"))
        .all(db)
        .await
        .unwrap()
        .into_iter()
        .map(|entry| {
            let body = &entry.envelope["body"];
            (
                body["principal_id"].as_str().unwrap().to_string(),
                body["by"].as_str().unwrap().to_string(),
            )
        })
        .collect()
}

#[test]
fn requests_are_normalized_and_bounded() {
    let shape = validate_link(CreateConversationLinkRequest {
        name: Some("  Friends  ".into()),
        max_uses: Some(0),
        expires_in_hours: Some(-5),
    })
    .unwrap();
    assert_eq!(
        shape,
        NewLink {
            name: Some("Friends".into()),
            max_uses: -1,
            expires_in: None,
        }
    );
    let limited = validate_link(CreateConversationLinkRequest {
        name: Some("   ".into()),
        max_uses: Some(5),
        expires_in_hours: Some(24),
    })
    .unwrap();
    assert_eq!(limited.name, None);
    assert_eq!(limited.max_uses, 5);
    assert_eq!(limited.expires_in, Some(Duration::hours(24)));
    for request in [
        CreateConversationLinkRequest {
            name: Some("x".repeat(81)),
            ..Default::default()
        },
        CreateConversationLinkRequest {
            max_uses: Some(10_001),
            ..Default::default()
        },
        CreateConversationLinkRequest {
            expires_in_hours: Some(24 * 365 + 1),
            ..Default::default()
        },
    ] {
        let error = validate_link(request).unwrap_err();
        assert_eq!(error.status(), StatusCode::BAD_REQUEST);
    }
}

#[tokio::test]
async fn anyone_signed_in_joins_at_once_and_everyone_sees_it() {
    let db = world().await;
    let made = link(&db, "user-b", &UNLIMITED).await;
    assert_eq!(made.token.len(), 43, "32 random bytes, URL-safe base64");
    assert_eq!(made.max_uses, -1);

    let joined = join_link_in(&db, &made.token, "stranger", at(20))
        .await
        .unwrap();
    assert_eq!(
        joined,
        LinkJoin {
            conversation_id: "room".into(),
            link_id: made.id.clone(),
            joined: true,
            wake: vec![
                "user-a".into(),
                "user-b".into(),
                "user-c".into(),
                "stranger".into()
            ],
        }
    );
    let row = member(&db, "stranger").await.unwrap();
    assert_eq!(row.status, MEMBER_ACTIVE);
    assert_eq!(row.added_by.as_deref(), Some("user-b"));
    assert_eq!(uses(&db, &made.id).await, 1);
    for inbox in ["user-a", "user-c", "stranger"] {
        assert!(
            member_entries(&db, inbox)
                .await
                .contains(&("stranger".to_string(), "stranger".to_string())),
            "{inbox} sees 'stranger joined'"
        );
    }

    let again = join_link_in(&db, &made.token, "stranger", at(30))
        .await
        .unwrap();
    assert!(!again.joined, "a retried join changes nothing");
    assert!(again.wake.is_empty());
    assert_eq!(uses(&db, &made.id).await, 1);
    assert_eq!(
        member_entries(&db, "user-a")
            .await
            .iter()
            .filter(|(principal, _)| principal == "stranger")
            .count(),
        1
    );
}

#[tokio::test]
async fn blocked_people_cannot_join_each_others_links_or_consume_a_use() {
    use super::blocks::{INBOX_BLOCKED, block_in, unblock_in};

    let db = world().await;
    db.execute_unprepared(r#"INSERT INTO public."User" ("id", "name") VALUES ('user-b', 'Bea')"#)
        .await
        .unwrap();
    let made = link(&db, "user-b", &UNLIMITED).await;
    for (owner, principal) in [("user-b", "stranger"), ("stranger", "user-b")] {
        block_in(&db, owner, principal, at(15)).await.unwrap();
        let error = join_link_in(&db, &made.token, "stranger", at(20))
            .await
            .unwrap_err();
        assert_code(&error, StatusCode::FORBIDDEN, INBOX_BLOCKED);
        assert!(member(&db, "stranger").await.is_none());
        assert_eq!(uses(&db, &made.id).await, 0);
        unblock_in(&db, owner, principal).await.unwrap();
    }
    assert!(
        join_link_in(&db, &made.token, "stranger", at(30))
            .await
            .unwrap()
            .joined
    );
    assert_eq!(uses(&db, &made.id).await, 1);
}

#[tokio::test]
async fn a_limited_link_lets_exactly_that_many_in() {
    let db = world().await;
    let once = link(
        &db,
        "user-a",
        &NewLink {
            max_uses: 1,
            ..UNLIMITED
        },
    )
    .await;
    join_link_in(&db, &once.token, "stranger", at(20))
        .await
        .unwrap();
    assert!(
        live_link(&db, &once.token, at(21)).await.unwrap().is_none(),
        "a used-up link previews as missing"
    );
    let error = join_link_in(&db, &once.token, "other", at(21))
        .await
        .unwrap_err();
    assert_code(&error, StatusCode::FORBIDDEN, INBOX_LINK_USED_UP);
    assert!(member(&db, "other").await.is_none());
    assert!(
        join_link_in(&db, &once.token, "stranger", at(22))
            .await
            .is_ok(),
        "who used the last place can still retry"
    );
}

#[tokio::test]
async fn expired_revoked_or_orphaned_links_read_as_missing() {
    let db = world().await;
    let hourly = link(
        &db,
        "user-a",
        &NewLink {
            expires_in: Some(Duration::hours(1)),
            ..UNLIMITED
        },
    )
    .await;
    assert!(
        live_link(&db, &hourly.token, at(20))
            .await
            .unwrap()
            .is_some()
    );
    let late = at(10) + Duration::hours(1);
    assert!(live_link(&db, &hourly.token, late).await.unwrap().is_none());
    let error = join_link_in(&db, &hourly.token, "stranger", late)
        .await
        .unwrap_err();
    assert_code(&error, StatusCode::NOT_FOUND, INBOX_LINK_NOT_FOUND);

    let of_c = link(&db, "user-c", &UNLIMITED).await;
    remove_in(&db, "room", "user-c", "user-c", at(20))
        .await
        .unwrap();
    assert!(live_link(&db, &of_c.token, at(21)).await.unwrap().is_none());
    let error = join_link_in(&db, &of_c.token, "stranger", at(21))
        .await
        .unwrap_err();
    assert_code(&error, StatusCode::NOT_FOUND, INBOX_LINK_NOT_FOUND);

    for token in ["", "no such token", "../x", &"a".repeat(129)] {
        let error = join_link_in(&db, token, "stranger", at(22))
            .await
            .unwrap_err();
        assert_code(&error, StatusCode::NOT_FOUND, INBOX_LINK_NOT_FOUND);
    }
}

#[tokio::test]
async fn the_maker_of_a_link_or_the_group_revokes_it() {
    let db = world().await;
    let of_b = link(&db, "user-b", &UNLIMITED).await;
    let error = revoke_link_in(&db, "room", &of_b.id, "user-c")
        .await
        .unwrap_err();
    assert_code(&error, StatusCode::FORBIDDEN, INBOX_LINK_NOT_ALLOWED);
    let outsider = revoke_link_in(&db, "room", &of_b.id, "stranger")
        .await
        .unwrap_err();
    assert_eq!(outsider.status(), StatusCode::NOT_FOUND);
    revoke_link_in(&db, "room", &of_b.id, "user-a")
        .await
        .unwrap();
    let error = join_link_in(&db, &of_b.token, "stranger", at(20))
        .await
        .unwrap_err();
    assert_code(&error, StatusCode::NOT_FOUND, INBOX_LINK_NOT_FOUND);

    let of_c = link(&db, "user-c", &UNLIMITED).await;
    revoke_link_in(&db, "room", &of_c.id, "user-c")
        .await
        .unwrap();
    let gone = revoke_link_in(&db, "room", &of_c.id, "user-c")
        .await
        .unwrap_err();
    assert_eq!(gone.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn members_see_the_links_and_only_plain_groups_take_them() {
    let db = world().await;
    let first = link(&db, "user-a", &UNLIMITED).await;
    let second = create_link_in(&db, "room", "user-b", &UNLIMITED, at(11))
        .await
        .unwrap();
    let listed: Vec<String> = links_in(&db, "room", "user-c")
        .await
        .unwrap()
        .into_iter()
        .map(|row| row.id)
        .collect();
    assert_eq!(listed, [second.id, first.id], "newest first");
    let outsider = links_in(&db, "room", "stranger").await.unwrap_err();
    assert_eq!(outsider.status(), StatusCode::NOT_FOUND);
    let outsider = create_link_in(&db, "room", "stranger", &UNLIMITED, at(12))
        .await
        .unwrap_err();
    assert_eq!(outsider.status(), StatusCode::NOT_FOUND);

    let error = create_link_in(&db, "sealed", "user-a", &UNLIMITED, at(12))
        .await
        .unwrap_err();
    assert_code(&error, StatusCode::CONFLICT, INBOX_LINK_NOT_ALLOWED);

    let direct = start_direct(
        &db,
        "user-a",
        &user("user-b"),
        ConversationEncryption::Plain,
        at(12),
    )
    .await
    .unwrap();
    let error = create_link_in(&db, &direct, "user-a", &UNLIMITED, at(13))
        .await
        .unwrap_err();
    assert_code(&error, StatusCode::CONFLICT, INBOX_DIRECT_FIXED);
}

#[tokio::test]
async fn a_group_holds_at_most_twenty_links() {
    let db = world().await;
    let mut made = Vec::new();
    for _ in 0..MAX_LINKS {
        made.push(link(&db, "user-a", &UNLIMITED).await);
    }
    let error = create_link_in(&db, "room", "user-b", &UNLIMITED, at(20))
        .await
        .unwrap_err();
    assert_code(&error, StatusCode::CONFLICT, INBOX_LINK_LIMIT);
    revoke_link_in(&db, "room", &made[0].id, "user-a")
        .await
        .unwrap();
    create_link_in(&db, "room", "user-b", &UNLIMITED, at(21))
        .await
        .unwrap();
}

#[tokio::test]
async fn a_full_group_uses_no_place_of_the_link() {
    let db = world().await;
    let made = link(&db, "user-a", &UNLIMITED).await;
    let guests: Vec<_> = (0..MAX_CONVERSATION_MEMBERS - 3)
        .map(|n| user(&format!("guest-{n:02}")))
        .collect();
    super::members::add_in(&db, "room", "user-a", &guests, at(15))
        .await
        .unwrap();
    let error = join_link_in(&db, &made.token, "stranger", at(20))
        .await
        .unwrap_err();
    assert_code(&error, StatusCode::CONFLICT, INBOX_MEMBER_LIMIT);
    assert_eq!(uses(&db, &made.id).await, 0);
}

#[tokio::test]
async fn an_invitee_who_opens_a_link_is_in_at_once() {
    let db = world().await;
    let invitees = classify(&db, "user-a", &["stranger".to_string()])
        .await
        .unwrap();
    invite_in(&db, "room", "user-a", &invitees, &BTreeMap::new(), at(12))
        .await
        .unwrap();
    assert_eq!(
        member(&db, "stranger").await.unwrap().status,
        MEMBER_PENDING
    );
    let made = link(&db, "user-b", &UNLIMITED).await;
    assert!(
        join_link_in(&db, &made.token, "stranger", at(20))
            .await
            .unwrap()
            .joined
    );
    assert_eq!(member(&db, "stranger").await.unwrap().status, MEMBER_ACTIVE);
}

async fn email_invite(
    db: &DatabaseConnection,
    address: &str,
    now: chrono::DateTime<chrono::FixedOffset>,
) -> EmailInvite {
    email_invite_by(db, "user-c", "user-a", address, now).await
}

async fn email_invite_by(
    db: &DatabaseConnection,
    bot: &str,
    owner: &str,
    address: &str,
    now: chrono::DateTime<chrono::FixedOffset>,
) -> EmailInvite {
    create_email_invite_in(db, "room", bot, owner, address, "You run the venue.", now)
        .await
        .unwrap()
}

#[tokio::test]
async fn an_email_invitation_is_one_unlisted_join_for_its_address() {
    let db = world().await;
    let EmailInvite::Created(made) = email_invite(&db, " Ana@Example.org ", at(10)).await else {
        panic!("expected a new email invitation");
    };
    assert_eq!(made.email.as_deref(), Some("ana@example.org"));
    assert_eq!(made.reason.as_deref(), Some("You run the venue."));
    assert_eq!(made.max_uses, 1);
    assert_eq!(made.created_by, "user-c");
    assert_eq!(
        made.expires_at,
        Some(at(10) + Duration::days(EMAIL_INVITE_DAYS))
    );
    assert_eq!(
        email_invite(&db, "ana@example.org", at(11)).await,
        EmailInvite::Pending,
        "the address already holds a live invitation, so no second email goes out"
    );
    assert!(
        links_in(&db, "room", "user-a").await.unwrap().is_empty(),
        "members never see the invitee's link"
    );
    assert!(live_link(&db, &made.token, at(12)).await.unwrap().is_some());

    assert!(
        join_link_in(&db, &made.token, "stranger", at(20))
            .await
            .unwrap()
            .joined
    );
    assert_eq!(
        member(&db, "stranger").await.unwrap().added_by.as_deref(),
        Some("user-c")
    );
    let error = join_link_in(&db, &made.token, "other", at(21))
        .await
        .unwrap_err();
    assert_code(&error, StatusCode::FORBIDDEN, INBOX_LINK_USED_UP);
    assert!(
        matches!(
            email_invite(&db, "ana@example.org", at(22)).await,
            EmailInvite::Created(_)
        ),
        "a used invitation no longer holds the address"
    );
}

#[tokio::test]
async fn an_expired_email_invitation_frees_its_address() {
    let db = world().await;
    assert!(matches!(
        email_invite(&db, "ana@example.org", at(10)).await,
        EmailInvite::Created(_)
    ));
    let later = at(10) + Duration::days(EMAIL_INVITE_DAYS) + Duration::seconds(1);
    assert!(matches!(
        email_invite(&db, "ana@example.org", later).await,
        EmailInvite::Created(_)
    ));
}

#[tokio::test]
async fn an_owners_bots_share_twenty_email_invitations_a_day_beside_the_members_links() {
    let db = world().await;
    insert_bot(&db, "bot-one", "olga", "SERVER").await;
    insert_bot(&db, "bot-two", "olga", "SERVER").await;
    insert_bot(&db, "bot-mia", "mia", "SERVER").await;
    for index in 0..MAX_OWNER_EMAIL_INVITES_PER_DAY {
        let bot = if index % 2 == 0 { "bot-one" } else { "bot-two" };
        assert!(matches!(
            email_invite_by(&db, bot, "olga", &format!("p{index}@example.org"), at(10)).await,
            EmailInvite::Created(_)
        ));
    }
    assert_eq!(
        email_invite_by(&db, "bot-two", "olga", "late@example.org", at(11)).await,
        EmailInvite::DailyLimit,
        "a second bot of the same owner draws from the same day"
    );
    assert!(matches!(
        email_invite_by(&db, "bot-mia", "mia", "late@example.org", at(11)).await,
        EmailInvite::Created(_)
    ));
    let next_day = at(10) + Duration::days(1) + Duration::seconds(1);
    assert!(matches!(
        email_invite_by(&db, "bot-one", "olga", "later@example.org", next_day).await,
        EmailInvite::Created(_)
    ));
    for _ in 0..MAX_LINKS {
        link(&db, "user-a", &UNLIMITED).await;
    }
    assert_eq!(
        links_in(&db, "room", "user-a").await.unwrap().len() as u64,
        MAX_LINKS
    );
}
