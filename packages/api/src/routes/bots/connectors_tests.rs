use super::connectors::{NewConnector, create_in, owned_connector, refuse_switched_off};
use crate::{
    entity::bot_connector,
    routes::inbox::fixture::{database as inbox_database, insert_bot},
};
use axum::http::StatusCode;
use flow_like_types::bot_connector::{ConnectorKind, ConnectorPolicy};
use sea_orm::{
    ActiveModelTrait, ActiveValue::Set, ColumnTrait, ConnectionTrait, DatabaseConnection,
    EntityTrait, QueryFilter,
};

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

async fn with_policies(
    db: &DatabaseConnection,
    row: &bot_connector::Model,
    tool_policies: serde_json::Value,
    tools: serde_json::Value,
) -> bot_connector::Model {
    let mut active: bot_connector::ActiveModel = row.clone().into();
    active.tool_policies = Set(tool_policies);
    active.tools_snapshot = Set(tools);
    active.update(db).await.unwrap()
}

fn refusal_code(row: &bot_connector::Model) -> Option<String> {
    refuse_switched_off(row).err().map(|error| {
        assert_eq!(error.status(), StatusCode::FORBIDDEN);
        error.public_code().to_string()
    })
}

#[tokio::test]
async fn the_credential_is_refused_only_when_every_call_is_never() {
    let db = database().await;
    insert_bot(&db, "bot-a", "user-a", "SERVER").await;
    let mut never = mcp("Off");
    never.policy = ConnectorPolicy::Never;
    let off = create_in(&db, "user-a", "bot-a", never).await.unwrap();
    assert_eq!(
        refusal_code(&off).as_deref(),
        Some("BOT_CONNECTOR_SWITCHED_OFF")
    );

    let hinted = with_policies(
        &db,
        &off,
        serde_json::json!({}),
        serde_json::json!([{ "name": "search", "read_only_hint": true }]),
    )
    .await;
    assert_eq!(refusal_code(&hinted), None, "a read-only tool still runs");

    let all_off = with_policies(
        &db,
        &hinted,
        serde_json::json!({ "search": "never" }),
        serde_json::json!([{ "name": "search", "read_only_hint": true }]),
    )
    .await;
    assert!(refusal_code(&all_off).is_some());

    let one_on = with_policies(
        &db,
        &all_off,
        serde_json::json!({ "search": "never", "fetch": "ask" }),
        serde_json::json!([{ "name": "search" }]),
    )
    .await;
    assert_eq!(refusal_code(&one_on), None, "an ask tool keeps the sign-in");

    let ask = create_in(&db, "user-a", "bot-a", mcp("Asks"))
        .await
        .unwrap();
    assert_eq!(refusal_code(&ask), None);
}

#[tokio::test]
async fn an_account_is_refused_only_when_reads_and_writes_are_never() {
    let db = database().await;
    insert_bot(&db, "bot-a", "user-a", "SERVER").await;
    let mut account = mcp("Mail");
    account.kind = ConnectorKind::Oauth;
    account.url = None;
    account.policy = ConnectorPolicy::Never;
    let row = create_in(&db, "user-a", "bot-a", account).await.unwrap();
    assert_eq!(refusal_code(&row), None, "reads run unless switched off");

    let writes_off = with_policies(
        &db,
        &row,
        serde_json::json!({ "write": "never" }),
        serde_json::json!([]),
    )
    .await;
    assert_eq!(refusal_code(&writes_off), None);

    let all_off = with_policies(
        &db,
        &writes_off,
        serde_json::json!({ "read": "never", "write": "never" }),
        serde_json::json!([]),
    )
    .await;
    assert_eq!(
        refusal_code(&all_off).as_deref(),
        Some("BOT_CONNECTOR_SWITCHED_OFF")
    );
}
