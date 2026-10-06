use crate::entity::{execution_usage_tracking, sea_orm_active_enums::ExecutionStatus};
use chrono::{DateTime, FixedOffset};
use sea_orm::{ConnectOptions, ConnectionTrait, Database, DatabaseConnection, EntityTrait, Set};

pub async fn database() -> DatabaseConnection {
    let mut options = ConnectOptions::new("sqlite::memory:");
    options.max_connections(1);
    let db = Database::connect(options).await.unwrap();
    db.execute_unprepared("ATTACH DATABASE ':memory:' AS public")
        .await
        .unwrap();
    db.execute_unprepared(
        r#"CREATE TABLE public."ExecutionUsageTracking" (
            id TEXT PRIMARY KEY, instance TEXT, "boardId" TEXT NOT NULL,
            "nodeId" TEXT NOT NULL, version TEXT NOT NULL, microseconds INTEGER NOT NULL,
            "userId" TEXT, "appId" TEXT, "createdAt" TEXT NOT NULL,
            "updatedAt" TEXT NOT NULL, "technicalUserId" TEXT, status TEXT NOT NULL
        )"#,
    )
    .await
    .unwrap();
    db
}

pub async fn insert(
    db: &DatabaseConnection,
    id: &str,
    user: &str,
    app: Option<&str>,
    at: DateTime<FixedOffset>,
    status: ExecutionStatus,
    microseconds: i64,
) {
    execution_usage_tracking::Entity::insert(execution_usage_tracking::ActiveModel {
        id: Set(id.to_owned()),
        instance: Set(None),
        board_id: Set("board".into()),
        node_id: Set("entry".into()),
        version: Set("1".into()),
        microseconds: Set(microseconds),
        user_id: Set(Some(user.to_owned())),
        app_id: Set(app.map(str::to_owned)),
        created_at: Set(at),
        updated_at: Set(at),
        technical_user_id: Set(None),
        status: Set(status),
    })
    .exec(db)
    .await
    .unwrap();
}
