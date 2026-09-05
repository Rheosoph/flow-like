//! Object state and aggregate totals share one transaction. Tombstones preserve event ordering.

use flow_like_db::{retry_transaction, DbDialect, RetryPolicy};
use sea_orm::{ConnectionTrait, DatabaseConnection, DbErr, Statement};

#[derive(Clone, Debug)]
pub struct Observation {
    pub bucket: String,
    pub key: String,
    pub app_id: String,
    pub user_id: Option<String>,
    pub sequencer: String,
    pub legacy_size: i64,
}

impl Observation {
    fn id(&self) -> String {
        let mut hasher = blake3::Hasher::new();
        hasher.update(self.bucket.as_bytes());
        hasher.update(&[0]);
        hasher.update(self.key.as_bytes());
        hasher.finalize().to_hex().to_string()
    }

    pub async fn already_accounted(&self, db: &DatabaseConnection) -> Result<bool, DbErr> {
        Ok(db
            .query_one_raw(Statement::from_sql_and_values(
                db.get_database_backend(),
                r#"SELECT "id" FROM "FileAccountingObject" WHERE "id" = $1"#,
                [self.id().into()],
            ))
            .await?
            .is_some())
    }
}

pub fn normalize_sequencer(value: &str) -> Result<String, String> {
    if value.is_empty() || !value.bytes().all(|ch| ch.is_ascii_hexdigit()) {
        return Err("S3 event sequencer must be a nonempty hexadecimal number".into());
    }
    let trimmed = value.trim_start_matches('0');
    Ok(if trimmed.is_empty() {
        "0".into()
    } else {
        trimmed.to_ascii_lowercase()
    })
}

fn newer_than(candidate: &str, previous: &str) -> bool {
    previous.is_empty()
        || candidate.len() > previous.len()
        || (candidate.len() == previous.len() && candidate > previous)
}

#[cfg(test)]
async fn apply(
    db: &DatabaseConnection,
    dialect: DbDialect,
    observation: Observation,
    size: i64,
) -> Result<(), DbErr> {
    apply_current(db, dialect, observation, move || async move { Ok(size) }).await
}

/// Read current storage state after acquiring the object write intent. Retried attempts
/// resample storage, so a later SQL commit cannot carry an earlier S3 observation.
pub async fn apply_current<F, Fut>(
    db: &DatabaseConnection,
    dialect: DbDialect,
    observation: Observation,
    read_current_size: F,
) -> Result<(), DbErr>
where
    F: Fn() -> Fut + Send + Sync + 'static,
    Fut: std::future::Future<Output = Result<i64, DbErr>> + Send,
{
    if observation.legacy_size < 0 {
        return Err(DbErr::Custom("object size must not be negative".into()));
    }
    let id = observation.id();
    let read_current_size = std::sync::Arc::new(read_current_size);
    retry_transaction(db, dialect, None, &RetryPolicy::idempotent(), move |txn| {
        let observation = observation.clone();
        let id = id.clone();
        let read_current_size = read_current_size.clone();
        Box::pin(async move {
            // The legacy row is read-only after cutover. Import its contribution exactly once;
            // ON CONFLICT plus FOR UPDATE coordinates simultaneous first events on both engines.
            txn.execute_raw(Statement::from_sql_and_values(txn.get_database_backend(),
                r#"INSERT INTO "FileAccountingObject" ("id", "bucket", "objectKey", "appId", "userId", "size", "sequencer", "updatedAt") VALUES ($1, $2, $3, $4, $5, $6, '', now()) ON CONFLICT ("id") DO NOTHING"#,
                [id.clone().into(), observation.bucket.into(), observation.key.into(), observation.app_id.clone().into(), observation.user_id.clone().into(), observation.legacy_size.into()]
            )).await?;
            let row = txn.query_one_raw(Statement::from_sql_and_values(txn.get_database_backend(),
                r#"SELECT "size", "sequencer" FROM "FileAccountingObject" WHERE "id" = $1 FOR UPDATE"#,
                [id.clone().into()]
            )).await?.ok_or_else(|| DbErr::Custom("object accounting row disappeared".into()))?;
            let old_size: i64 = row.try_get("", "size")?;
            let old_sequencer: String = row.try_get("", "sequencer")?;
            if !newer_than(&observation.sequencer, &old_sequencer) {
                return Ok(());
            }
            let size = read_current_size().await?;
            if size < 0 {
                return Err(DbErr::Custom("object size must not be negative".into()));
            }
            let delta = size.checked_sub(old_size)
                .ok_or_else(|| DbErr::Custom("object size delta overflow".into()))?;
            txn.execute_raw(Statement::from_sql_and_values(txn.get_database_backend(),
                r#"UPDATE "FileAccountingObject" SET "size" = $2, "sequencer" = $3, "updatedAt" = now() WHERE "id" = $1"#,
                [id.into(), size.into(), observation.sequencer.into()]
            )).await?;
            if delta != 0 {
                // A late storage event can outlive its app or user. Updating zero rows is valid:
                // the retained object tombstone still prevents a duplicate event being counted.
                txn.execute_raw(Statement::from_sql_and_values(txn.get_database_backend(),
                    r#"UPDATE "App" SET "totalSize" = "totalSize" + $2 WHERE "id" = $1"#,
                    [observation.app_id.into(), delta.into()]
                )).await?;
                if let Some(user_id) = observation.user_id {
                    txn.execute_raw(Statement::from_sql_and_values(txn.get_database_backend(),
                        r#"UPDATE "User" SET "totalSize" = "totalSize" + $2 WHERE "id" = $1"#,
                        [user_id.into(), delta.into()]
                    )).await?;
                }
            }
            Ok(())
        })
    }).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn s3_sequencers_compare_as_hex_numbers_with_arbitrary_width() {
        assert_eq!(normalize_sequencer("0000AF").unwrap(), "af");
        assert!(newer_than("100", "ff"));
        assert!(!newer_than("ff", "100"));
        assert!(!newer_than("af", "af"));
        assert!(newer_than("0", ""));
        assert!(normalize_sequencer("").is_err());
        assert!(normalize_sequencer("xyz").is_err());
    }

    #[test]
    fn identity_includes_bucket_and_does_not_exceed_index_limits() {
        let mut row = Observation {
            bucket: "a".into(),
            key: "x".repeat(1024),
            app_id: "app".into(),
            user_id: None,
            sequencer: "1".into(),
            legacy_size: 0,
        };
        let first = row.id();
        row.bucket = "b".into();
        assert_ne!(first, row.id());
        assert_eq!(first.len(), 64);
    }

    /// Run against an explicitly configured disposable PostgreSQL database. The test owns
    /// a unique schema and checks actual rollback and replay behavior, including legacy import.
    #[tokio::test]
    async fn postgres_accounting_rollbacks_replays_and_ordering() {
        use sea_orm::{
            sqlx::postgres::{PgConnectOptions, PgPoolOptions},
            Database, DatabaseBackend,
        };
        use std::str::FromStr;
        let Ok(url) = std::env::var("FLOW_LIKE_TEST_DATABASE_URL") else {
            eprintln!("skipping PostgreSQL accounting test: FLOW_LIKE_TEST_DATABASE_URL is unset");
            return;
        };
        let admin = Database::connect(&url).await.unwrap();
        let schema = format!(
            "file_accounting_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        admin
            .execute_raw(Statement::from_string(
                DatabaseBackend::Postgres,
                format!("CREATE SCHEMA {schema}"),
            ))
            .await
            .unwrap();
        let pool = PgPoolOptions::new()
            .max_connections(4)
            .connect_with(
                PgConnectOptions::from_str(&url)
                    .unwrap()
                    .options([("search_path", schema.as_str())]),
            )
            .await
            .unwrap();
        let db = DatabaseConnection::from(pool);
        async fn sql(db: &DatabaseConnection, sql: &str) {
            db.execute_raw(Statement::from_string(
                sea_orm::DatabaseBackend::Postgres,
                sql,
            ))
            .await
            .unwrap();
        }
        async fn value(db: &DatabaseConnection, sql: &str) -> i64 {
            db.query_one_raw(Statement::from_string(
                sea_orm::DatabaseBackend::Postgres,
                sql,
            ))
            .await
            .unwrap()
            .unwrap()
            .try_get("", "n")
            .unwrap()
        }
        sql(&db, r#"CREATE TABLE "App" (id TEXT PRIMARY KEY, "totalSize" BIGINT NOT NULL, CONSTRAINT fail_overwrite CHECK ("totalSize" <= 100))"#).await;
        sql(
            &db,
            r#"CREATE TABLE "User" (id TEXT PRIMARY KEY, "totalSize" BIGINT NOT NULL)"#,
        )
        .await;
        sql(&db, r#"CREATE TABLE "FileAccountingObject" (id TEXT PRIMARY KEY, bucket TEXT, "objectKey" TEXT, "appId" TEXT, "userId" TEXT, size BIGINT, sequencer TEXT, "updatedAt" TIMESTAMPTZ)"#).await;
        sql(&db, r#"INSERT INTO "App" VALUES ('app',100)"#).await;
        sql(&db, r#"INSERT INTO "User" VALUES ('user',100)"#).await;
        let observation = Observation {
            bucket: "bucket".into(),
            key: "users/user/apps/app/file".into(),
            app_id: "app".into(),
            user_id: Some("user".into()),
            sequencer: "2".into(),
            legacy_size: 100,
        };
        assert!(apply(&db, DbDialect::Postgres, observation.clone(), 150)
            .await
            .is_err());
        assert_eq!(
            value(
                &db,
                r#"SELECT count(*)::BIGINT AS n FROM "FileAccountingObject""#
            )
            .await,
            0
        );
        assert_eq!(
            value(&db, r#"SELECT "totalSize" AS n FROM "App""#).await,
            100
        );
        sql(&db, r#"ALTER TABLE "App" DROP CONSTRAINT fail_overwrite"#).await;
        apply(&db, DbDialect::Postgres, observation.clone(), 150)
            .await
            .unwrap();
        apply(&db, DbDialect::Postgres, observation.clone(), 150)
            .await
            .unwrap();
        assert_eq!(
            value(&db, r#"SELECT "totalSize" AS n FROM "App""#).await,
            150
        );
        assert_eq!(
            value(&db, r#"SELECT "totalSize" AS n FROM "User""#).await,
            150
        );
        let stale = Observation {
            sequencer: "1".into(),
            ..observation.clone()
        };
        apply(&db, DbDialect::Postgres, stale, 50).await.unwrap();
        assert_eq!(
            value(&db, r#"SELECT size AS n FROM "FileAccountingObject""#).await,
            150
        );

        sql(
            &db,
            r#"ALTER TABLE "App" ADD CONSTRAINT fail_delete CHECK ("totalSize" > 0)"#,
        )
        .await;
        let deletion = Observation {
            sequencer: "3".into(),
            ..observation.clone()
        };
        assert!(apply(&db, DbDialect::Postgres, deletion.clone(), 0)
            .await
            .is_err());
        assert_eq!(
            value(&db, r#"SELECT size AS n FROM "FileAccountingObject""#).await,
            150
        );
        sql(&db, r#"ALTER TABLE "App" DROP CONSTRAINT fail_delete"#).await;
        apply(&db, DbDialect::Postgres, deletion.clone(), 0)
            .await
            .unwrap();
        apply(&db, DbDialect::Postgres, deletion.clone(), 0)
            .await
            .unwrap();
        assert_eq!(value(&db, r#"SELECT "totalSize" AS n FROM "App""#).await, 0);
        assert_eq!(
            value(&db, r#"SELECT "totalSize" AS n FROM "User""#).await,
            0
        );

        let recreated = Observation {
            sequencer: "4".into(),
            ..observation.clone()
        };
        let (first, duplicate) = tokio::join!(
            apply(&db, DbDialect::Postgres, recreated.clone(), 200),
            apply(&db, DbDialect::Postgres, recreated, 200)
        );
        first.unwrap();
        duplicate.unwrap();
        apply(&db, DbDialect::Postgres, deletion, 0).await.unwrap();
        assert_eq!(
            value(&db, r#"SELECT "totalSize" AS n FROM "App""#).await,
            200
        );
        assert_eq!(
            value(&db, r#"SELECT "totalSize" AS n FROM "User""#).await,
            200
        );

        // A second event must not sample S3 before the first transaction releases the object.
        // Otherwise its earlier sample could overwrite a newer sample when its commit runs last.
        use std::sync::{
            atomic::{AtomicBool, AtomicI64, Ordering},
            Arc,
        };
        let entered = Arc::new(tokio::sync::Notify::new());
        let release = Arc::new(tokio::sync::Notify::new());
        let current_size = Arc::new(AtomicI64::new(200));
        let second_sampled = Arc::new(AtomicBool::new(false));
        let first = {
            let db = db.clone();
            let entered = entered.clone();
            let release = release.clone();
            let current_size = current_size.clone();
            let event = Observation {
                sequencer: "5".into(),
                ..observation.clone()
            };
            tokio::spawn(async move {
                apply_current(&db, DbDialect::Postgres, event, move || {
                    let entered = entered.clone();
                    let release = release.clone();
                    let current_size = current_size.clone();
                    async move {
                        entered.notify_one();
                        release.notified().await;
                        Ok(current_size.load(Ordering::SeqCst))
                    }
                })
                .await
            })
        };
        entered.notified().await;
        let second = {
            let db = db.clone();
            let sampled = second_sampled.clone();
            let current_size = current_size.clone();
            let event = Observation {
                sequencer: "6".into(),
                ..observation.clone()
            };
            tokio::spawn(async move {
                apply_current(&db, DbDialect::Postgres, event, move || {
                    let sampled = sampled.clone();
                    let current_size = current_size.clone();
                    async move {
                        sampled.store(true, Ordering::SeqCst);
                        Ok(current_size.load(Ordering::SeqCst))
                    }
                })
                .await
            })
        };
        tokio::time::sleep(std::time::Duration::from_millis(30)).await;
        assert!(!second_sampled.load(Ordering::SeqCst));
        current_size.store(300, Ordering::SeqCst);
        release.notify_one();
        first.await.unwrap().unwrap();
        second.await.unwrap().unwrap();
        assert_eq!(
            value(&db, r#"SELECT "totalSize" AS n FROM "App""#).await,
            300
        );
        let next = Observation {
            sequencer: "7".into(),
            ..observation
        };
        assert!(
            apply_current(&db, DbDialect::Postgres, next.clone(), || async {
                Err(DbErr::Custom("S3 unavailable".into()))
            })
            .await
            .is_err()
        );
        assert_eq!(
            value(&db, r#"SELECT size AS n FROM "FileAccountingObject""#).await,
            300
        );
        apply(&db, DbDialect::Postgres, next, 400).await.unwrap();
        assert_eq!(
            value(&db, r#"SELECT "totalSize" AS n FROM "App""#).await,
            400
        );
        db.close().await.unwrap();
        admin
            .execute_raw(Statement::from_string(
                DatabaseBackend::Postgres,
                format!("DROP SCHEMA {schema} CASCADE"),
            ))
            .await
            .unwrap();
    }
}
