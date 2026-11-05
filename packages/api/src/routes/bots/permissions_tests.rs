use super::permissions::{BOT_PERMISSION_SELF, grant_in, list_in, revoke_in};
use crate::{
    entity::bot_permission,
    routes::inbox::fixture::{assert_code, at, database as inbox_database, delete_bot, insert_bot},
};
use axum::http::StatusCode;
use sea_orm::{ConnectionTrait, DatabaseConnection, EntityTrait, PaginatorTrait};

pub(super) async fn database() -> DatabaseConnection {
    let db = inbox_database().await;
    for statement in [
        r#"CREATE TABLE public."BotPermission" (
            "id" TEXT PRIMARY KEY, "ownerId" TEXT NOT NULL, "fromBotId" TEXT NOT NULL,
            "toBotId" TEXT NOT NULL, "grantedAt" TEXT NOT NULL, "grantedBy" TEXT NOT NULL,
            UNIQUE ("fromBotId", "toBotId"))"#,
        r#"CREATE TABLE public."BotHostHeartbeat" (
            "botId" TEXT NOT NULL, "hostId" TEXT NOT NULL, "hostKind" TEXT NOT NULL,
            "lastSeenAt" TEXT NOT NULL, PRIMARY KEY ("botId", "hostId"))"#,
    ] {
        db.execute_unprepared(statement).await.unwrap();
    }
    db
}

async fn two_bots(db: &DatabaseConnection) {
    insert_bot(db, "bot_a", "user-a", "SERVER").await;
    insert_bot(db, "bot_b", "user-a", "DESKTOP").await;
    insert_bot(db, "bot_other", "user-b", "SERVER").await;
}

async fn grants(db: &DatabaseConnection) -> u64 {
    bot_permission::Entity::find().count(db).await.unwrap()
}

#[tokio::test]
async fn a_grant_points_one_way_and_lists_on_both_bots() {
    let db = database().await;
    two_bots(&db).await;
    let (grant, created) = grant_in(&db, "user-a", "bot_a", "bot_b", at(5))
        .await
        .unwrap();
    assert!(created);
    assert_eq!(
        (
            grant.from_bot_id.as_str(),
            grant.to_bot_id.as_str(),
            grant.granted_by.as_str()
        ),
        ("bot_a", "bot_b", "user-a")
    );

    let from = list_in(&db, "user-a", "bot_a").await.unwrap();
    assert_eq!((from.granted.len(), from.received.len()), (1, 0));
    let to = list_in(&db, "user-a", "bot_b").await.unwrap();
    assert_eq!((to.granted.len(), to.received.len()), (0, 1));
    assert_eq!(to.received[0], from.granted[0]);
}

#[tokio::test]
async fn granting_again_returns_the_first_grant_and_writes_nothing() {
    let db = database().await;
    two_bots(&db).await;
    let (first, created) = grant_in(&db, "user-a", "bot_a", "bot_b", at(5))
        .await
        .unwrap();
    assert!(created);
    let (second, created) = grant_in(&db, "user-a", "bot_a", "bot_b", at(900))
        .await
        .unwrap();
    assert!(!created);
    assert_eq!(first, second);
    assert_eq!(grants(&db).await, 1);

    let (reverse, created) = grant_in(&db, "user-a", "bot_b", "bot_a", at(7))
        .await
        .unwrap();
    assert!(created);
    assert_ne!(reverse.id, first.id);
    assert_eq!(grants(&db).await, 2);
}

#[tokio::test]
async fn a_bot_is_not_granted_to_itself() {
    let db = database().await;
    two_bots(&db).await;
    let error = grant_in(&db, "user-a", "bot_a", "bot_a", at(5))
        .await
        .unwrap_err();
    assert_code(&error, StatusCode::BAD_REQUEST, BOT_PERMISSION_SELF);
    assert_eq!(grants(&db).await, 0);
}

#[tokio::test]
async fn a_grant_needs_both_bots_to_be_the_callers_and_live() {
    let db = database().await;
    two_bots(&db).await;
    insert_bot(&db, "bot_gone", "user-a", "SERVER").await;
    delete_bot(&db, "bot_gone").await;

    for (from, to) in [
        ("bot_a", "bot_other"),
        ("bot_other", "bot_a"),
        ("bot_a", "bot_gone"),
        ("bot_gone", "bot_a"),
        ("bot_a", "bot_missing"),
    ] {
        let error = grant_in(&db, "user-a", from, to, at(5)).await.unwrap_err();
        assert_eq!(error.status(), StatusCode::NOT_FOUND, "{from} -> {to}");
    }
    assert_eq!(grants(&db).await, 0);

    let error = grant_in(&db, "user-b", "bot_a", "bot_b", at(5))
        .await
        .unwrap_err();
    assert_eq!(error.status(), StatusCode::NOT_FOUND);
    assert_eq!(grants(&db).await, 0);
}

#[tokio::test]
async fn a_revoke_removes_the_grant_and_repeats_harmlessly() {
    let db = database().await;
    two_bots(&db).await;
    grant_in(&db, "user-a", "bot_a", "bot_b", at(5))
        .await
        .unwrap();
    grant_in(&db, "user-a", "bot_b", "bot_a", at(6))
        .await
        .unwrap();

    assert!(revoke_in(&db, "user-a", "bot_a", "bot_b").await.unwrap());
    assert!(!revoke_in(&db, "user-a", "bot_a", "bot_b").await.unwrap());
    let kept = list_in(&db, "user-a", "bot_a").await.unwrap();
    assert_eq!((kept.granted.len(), kept.received.len()), (0, 1));
}

#[tokio::test]
async fn a_revoke_needs_both_bots_to_be_the_callers_and_live() {
    let db = database().await;
    two_bots(&db).await;
    grant_in(&db, "user-a", "bot_a", "bot_b", at(5))
        .await
        .unwrap();

    let foreign = revoke_in(&db, "user-b", "bot_a", "bot_b")
        .await
        .unwrap_err();
    assert_eq!(foreign.status(), StatusCode::NOT_FOUND);
    for (from, to) in [("bot_a", "bot_other"), ("bot_other", "bot_a")] {
        let mixed = revoke_in(&db, "user-a", from, to).await.unwrap_err();
        assert_eq!(mixed.status(), StatusCode::NOT_FOUND, "{from} -> {to}");
    }
    delete_bot(&db, "bot_b").await;
    let deleted = revoke_in(&db, "user-a", "bot_a", "bot_b")
        .await
        .unwrap_err();
    assert_eq!(deleted.status(), StatusCode::NOT_FOUND);
    assert_eq!(grants(&db).await, 1);
}

#[tokio::test]
async fn listing_is_scoped_to_the_callers_live_bot() {
    let db = database().await;
    two_bots(&db).await;
    grant_in(&db, "user-a", "bot_a", "bot_b", at(5))
        .await
        .unwrap();

    for (owner, bot) in [
        ("user-b", "bot_a"),
        ("user-a", "bot_other"),
        ("user-a", "nope"),
    ] {
        let error = list_in(&db, owner, bot).await.unwrap_err();
        assert_eq!(error.status(), StatusCode::NOT_FOUND, "{owner} {bot}");
    }
}

#[test]
fn the_router_registers_every_route_without_conflict() {
    let _ = super::routes();
}
