use super::{
    model::{DataSourceDriver, ExternalDataSource},
    query_source::{load_source, opened_source, store_check},
    validation,
};
use crate::{
    data_source_secrets::{Sealers, SecretScope},
    entity::app_data_source,
};
use axum::http::StatusCode;
use flow_like_query::{ExternalConnection, SecretFields, SecretMaterial};
use sea_orm::{
    ActiveModelTrait, ActiveValue::Set, ConnectOptions, ConnectionTrait, Database,
    DatabaseConnection, Schema, sea_query::Table,
};
use serde_json::json;
use std::collections::BTreeMap;

const KEY: [u8; 32] = [9; 32];
const PASSWORD: &str = "pa55-word-value";

async fn database() -> DatabaseConnection {
    let mut options = ConnectOptions::new("sqlite::memory:");
    options.max_connections(1);
    let db = Database::connect(options).await.unwrap();
    db.execute_unprepared("ATTACH DATABASE ':memory:' AS public")
        .await
        .unwrap();
    let schema = Schema::new(db.get_database_backend());
    let source = schema.create_table_from_entity(app_data_source::Entity);
    let mut table = Table::create();
    table.table(source.get_table_name().unwrap().clone());
    for column in source.get_columns() {
        table.col(column.clone());
    }
    db.execute(&table).await.unwrap();
    db
}

fn connection() -> ExternalConnection {
    let config = json!({ "host": "db.example.com", "port": 5432, "database": "sales" });
    validation::connection(DataSourceDriver::Postgres, config.as_object().unwrap()).unwrap()
}

async fn sealed(sealers: &Sealers, app_id: &str, source_id: &str) -> String {
    let fields = SecretFields(BTreeMap::from([
        ("username".to_string(), "reader".to_string()),
        ("password".to_string(), PASSWORD.to_string()),
    ]));
    sealers
        .seal_fields(SecretScope { app_id, source_id }, &fields)
        .await
        .unwrap()
        .encode()
}

async fn insert(
    db: &DatabaseConnection,
    app_id: &str,
    source_id: &str,
    secret: Option<String>,
) -> app_data_source::Model {
    let now = chrono::Utc::now().fixed_offset();
    app_data_source::ActiveModel {
        id: Set(source_id.to_string()),
        app_id: Set(app_id.to_string()),
        name: Set("Warehouse".to_string()),
        alias: Set(format!("warehouse_{source_id}")),
        driver: Set("postgres".to_string()),
        config: Set(validation::stored_config(&connection())),
        sealed_by: Set(secret.as_ref().map(|_| "derived:test".to_string())),
        secret: Set(secret),
        secret_fields: Set(vec!["password".to_string(), "username".to_string()].into()),
        created_by: Set("user_1".to_string()),
        last_checked_at: Set(None),
        last_check_status: Set(None),
        created_at: Set(now),
        updated_at: Set(now),
    }
    .insert(db)
    .await
    .unwrap()
}

#[tokio::test]
async fn a_stored_source_opens_only_for_its_app_and_source() {
    let db = database().await;
    let sealers = Sealers::new(&KEY);
    let stored = sealed(&sealers, "app_a", "src_1").await;
    insert(&db, "app_a", "src_1", Some(stored.clone())).await;

    let source = opened_source(&db, &sealers, "app_a", "src_1")
        .await
        .unwrap();
    assert_eq!(source.source_id, "src_1");
    assert_eq!(source.app_id, "app_a");
    assert_eq!(source.connection, connection());
    let SecretMaterial::Plain { fields } = &source.secret else {
        panic!("the secret was not opened");
    };
    assert_eq!(fields.get("username"), Some("reader"));
    assert_eq!(fields.get("password"), Some(PASSWORD));

    let error = opened_source(&db, &sealers, "app_b", "src_1")
        .await
        .unwrap_err();
    assert_eq!(error.status(), StatusCode::NOT_FOUND);

    insert(&db, "app_a", "src_2", Some(stored)).await;
    let error = opened_source(&db, &sealers, "app_a", "src_2")
        .await
        .unwrap_err();
    assert_eq!(error.status(), StatusCode::INTERNAL_SERVER_ERROR);
    assert!(!format!("{error:?}").contains(PASSWORD));

    let rotated = Sealers::new(&[8; 32]);
    assert!(
        opened_source(&db, &rotated, "app_a", "src_1")
            .await
            .is_err()
    );
}

#[tokio::test]
async fn a_source_without_credentials_connects_without_a_secret() {
    let db = database().await;
    insert(&db, "app_a", "src_1", None).await;
    let source = opened_source(&db, &Sealers::new(&KEY), "app_a", "src_1")
        .await
        .unwrap();
    assert!(matches!(source.secret, SecretMaterial::None));
}

#[tokio::test]
async fn responses_name_the_secret_fields_but_never_carry_the_secret() {
    let db = database().await;
    let sealers = Sealers::new(&KEY);
    let stored = sealed(&sealers, "app_a", "src_1").await;
    let row = insert(&db, "app_a", "src_1", Some(stored.clone())).await;

    let response = serde_json::to_value(ExternalDataSource::try_from(row).unwrap()).unwrap();
    let object = response.as_object().unwrap();
    assert!(!object.contains_key("secret") && !object.contains_key("sealed_by"));
    assert_eq!(response["secret_fields"], json!(["password", "username"]));
    assert_eq!(response["driver"], json!("postgres"));
    assert_eq!(response["config"]["schema"], json!("public"));
    let text = response.to_string();
    assert!(!text.contains(&stored) && !text.contains("v1:derived") && !text.contains(PASSWORD));
}

#[tokio::test]
async fn checks_are_recorded_for_the_owning_app_only() {
    let db = database().await;
    insert(&db, "app_a", "src_1", None).await;

    store_check(&db, "app_a", "src_1", "ok").await.unwrap();
    let row = load_source(&db, "app_a", "src_1").await.unwrap();
    assert_eq!(row.last_check_status.as_deref(), Some("ok"));
    assert!(row.last_checked_at.is_some());

    store_check(&db, "app_a", "src_1", &"x".repeat(5000))
        .await
        .unwrap();
    let row = load_source(&db, "app_a", "src_1").await.unwrap();
    assert_eq!(row.last_check_status.map(|status| status.len()), Some(1000));

    let error = store_check(&db, "app_b", "src_1", "ok").await.unwrap_err();
    assert_eq!(error.status(), StatusCode::NOT_FOUND);
}
