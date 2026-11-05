use super::connectors::{NewConnector, create_in, owned_connector};
use crate::{
    entity::bot_connector,
    routes::inbox::fixture::{database as inbox_database, insert_bot},
};
use axum::http::StatusCode;
use flow_like_types::bot_connector::{ConnectorKind, ConnectorPolicy};
use sea_orm::{ColumnTrait, ConnectionTrait, DatabaseConnection, EntityTrait, QueryFilter};

async fn database() -> DatabaseConnection {
    let db = inbox_database().await;
    db.execute_unprepared(
        r#"CREATE TABLE public."BotConnector" (
            "id" TEXT PRIMARY KEY, "botId" TEXT NOT NULL, "kind" TEXT NOT NULL,
            "label" TEXT NOT NULL, "providerId" TEXT, "url" TEXT, "authKind" TEXT NOT NULL,
            "authRef" TEXT, "grantEncrypted" TEXT, "scopes" TEXT NOT NULL,
            "defaultPolicy" TEXT NOT NULL, "toolPolicies" TEXT NOT NULL,
            "toolsSnapshot" TEXT NOT NULL, "confirmedHashes" TEXT NOT NULL, "createdAt" TEXT NOT NULL, "updatedAt" TEXT NOT NULL)"#,
    )
    .await
    .unwrap();
    db.execute_unprepared(
        r#"CREATE TABLE public."BotSecret" (
            "botId" TEXT NOT NULL, "name" TEXT NOT NULL, "encryption" TEXT NOT NULL,
            "ciphertext" TEXT NOT NULL, "description" TEXT, "createdAt" TEXT NOT NULL,
            "updatedAt" TEXT NOT NULL, "lastUsedAt" TEXT, PRIMARY KEY ("botId", "name"))"#,
    )
    .await
    .unwrap();
    db
}

fn mcp(label: &str) -> NewConnector {
    NewConnector {
        kind: ConnectorKind::Mcp,
        label: label.into(),
        url: Some("https://mcp.example.com/mcp".into()),
        provider_id: None,
        auth_kind: "NONE",
        auth_ref: None,
        grant_encrypted: None,
        scopes: Vec::new(),
        policy: ConnectorPolicy::Ask,
    }
}

#[tokio::test]
async fn a_connector_is_created_only_on_the_callers_own_bot() {
    let db = database().await;
    insert_bot(&db, "bot-a", "user-a", "SERVER").await;
    let made = create_in(&db, "user-a", "bot-a", mcp("Docs"))
        .await
        .unwrap();
    assert_eq!(made.bot_id, "bot-a");
    assert_eq!(made.default_policy, "ask");
    let stranger = create_in(&db, "user-b", "bot-a", mcp("Mine"))
        .await
        .unwrap_err();
    assert_eq!(stranger.status(), StatusCode::NOT_FOUND);
    let rows = bot_connector::Entity::find().all(&db).await.unwrap();
    assert_eq!(rows.len(), 1);
}

#[tokio::test]
async fn a_connector_is_read_only_through_its_own_bot_and_owner() {
    let db = database().await;
    insert_bot(&db, "bot-a", "user-a", "SERVER").await;
    insert_bot(&db, "bot-b", "user-b", "SERVER").await;
    let made = create_in(&db, "user-a", "bot-a", mcp("Docs"))
        .await
        .unwrap();
    assert!(
        owned_connector(&db, "user-a", "bot-a", &made.id)
            .await
            .is_ok()
    );
    for (owner, bot) in [
        ("user-b", "bot-a"),
        ("user-b", "bot-b"),
        ("user-a", "bot-b"),
    ] {
        assert!(
            owned_connector(&db, owner, bot, &made.id).await.is_err(),
            "{owner} {bot}"
        );
    }
}

#[tokio::test]
async fn labels_are_unique_per_bot_and_the_secret_must_exist() {
    let db = database().await;
    insert_bot(&db, "bot-a", "user-a", "SERVER").await;
    insert_bot(&db, "bot-b", "user-a", "SERVER").await;
    create_in(&db, "user-a", "bot-a", mcp("Docs"))
        .await
        .unwrap();
    assert!(
        create_in(&db, "user-a", "bot-a", mcp("docs"))
            .await
            .is_err()
    );
    assert!(create_in(&db, "user-a", "bot-b", mcp("Docs")).await.is_ok());
    let mut signed = mcp("Signed");
    signed.auth_kind = "SECRET";
    signed.auth_ref = Some("DOCS_TOKEN".into());
    assert!(create_in(&db, "user-a", "bot-a", signed).await.is_err());
    let listed = bot_connector::Entity::find()
        .filter(bot_connector::Column::BotId.eq("bot-a"))
        .all(&db)
        .await
        .unwrap();
    assert_eq!(listed.len(), 1);
}
