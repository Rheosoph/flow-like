//! Object state and aggregate totals share one transaction. Tombstones preserve event ordering.

use flow_like_db::{retry_transaction, DbDialect, RetryPolicy};
use sea_orm::{
    ConnectionTrait, DatabaseConnection, DatabaseTransaction, DbErr, QueryResult, Statement,
};

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
}

struct Accounted {
    size: i64,
    sequencer: String,
    payer_id: Option<String>,
}

impl Accounted {
    fn from_row(row: &QueryResult) -> Result<Self, DbErr> {
        Ok(Self {
            size: row.try_get("", "size")?,
            sequencer: row.try_get("", "sequencer")?,
            payer_id: row.try_get("", "payerId")?,
        })
    }
}

fn select_accounted(txn: &DatabaseTransaction, id: &str) -> Statement {
    Statement::from_sql_and_values(
        txn.get_database_backend(),
        r#"SELECT "size", "sequencer", "payerId" FROM "FileAccountingObject" WHERE "id" = $1"#,
        [id.to_owned().into()],
    )
}

/// The object's row under the app lock, and whether this transaction created it.
/// A row the snapshot saw is read directly: the insert's conflict arm writes nothing, so it
/// coordinates nothing. Tombstone and app cleanup delete rows without the app lock, so an
/// absent row is still inserted, and a created row's state is known without reading it back.
async fn accounted_object(
    txn: &DatabaseTransaction,
    observation: &Observation,
    id: &str,
    seen: bool,
) -> Result<(Accounted, bool), DbErr> {
    if seen {
        if let Some(row) = txn.query_one_raw(select_accounted(txn, id)).await? {
            return Ok((Accounted::from_row(&row)?, false));
        }
    }
    // The legacy row is read-only after cutover. Import its contribution exactly once;
    // The retained row write coordinates simultaneous first events on both engines.
    let inserted = txn.execute_raw(Statement::from_sql_and_values(txn.get_database_backend(),
        r#"INSERT INTO "FileAccountingObject" ("id", "bucket", "objectKey", "appId", "userId", "size", "sequencer", "updatedAt") VALUES ($1, $2, $3, $4, $5, $6, '', now()) ON CONFLICT ("id") DO NOTHING"#,
        [id.to_owned().into(), observation.bucket.clone().into(), observation.key.clone().into(), observation.app_id.clone().into(), observation.user_id.clone().into(), observation.legacy_size.into()]
    )).await?.rows_affected();
    if inserted == 1 {
        return Ok((
            Accounted {
                size: observation.legacy_size,
                sequencer: String::new(),
                payer_id: None,
            },
            true,
        ));
    }
    let row = txn
        .query_one_raw(select_accounted(txn, id))
        .await?
        .ok_or_else(|| DbErr::Custom("object accounting row disappeared".into()))?;
    Ok((Accounted::from_row(&row)?, false))
}

fn non_negative(size: i64) -> Result<i64, DbErr> {
    if size < 0 {
        return Err(DbErr::Custom("object size must not be negative".into()));
    }
    Ok(size)
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

#[cfg(test)]
async fn apply_current<F, Fut>(
    db: &DatabaseConnection,
    dialect: DbDialect,
    observation: Observation,
    read_current_size: F,
) -> Result<(), DbErr>
where
    F: Fn() -> Fut + Send + Sync + 'static,
    Fut: std::future::Future<Output = Result<i64, DbErr>> + Send,
{
    let legacy_size = observation.legacy_size;
    apply_with_baseline(
        db,
        dialect,
        observation,
        move || async move { Ok(legacy_size) },
        read_current_size,
    )
    .await
}

/// Sample storage without holding an app or account transaction. A changed object
/// snapshot rejects the sample, so an earlier HEAD cannot overwrite a later event.
/// The legacy baseline is read only when the first snapshot finds no accounting row.
pub async fn apply_with_baseline<L, LFut, F, Fut>(
    db: &DatabaseConnection,
    dialect: DbDialect,
    mut observation: Observation,
    read_legacy_size: L,
    read_current_size: F,
) -> Result<(), DbErr>
where
    L: Fn() -> LFut + Send + Sync + 'static,
    LFut: std::future::Future<Output = Result<i64, DbErr>> + Send,
    F: Fn() -> Fut + Send + Sync + 'static,
    Fut: std::future::Future<Output = Result<i64, DbErr>> + Send,
{
    non_negative(observation.legacy_size)?;
    let id = observation.id();
    let read_current_size = std::sync::Arc::new(read_current_size);
    for round in 0..8 {
        let snapshot = db
            .query_one_raw(Statement::from_sql_and_values(
                db.get_database_backend(),
                r#"SELECT "size", "sequencer" FROM "FileAccountingObject" WHERE "id"=$1"#,
                [id.clone().into()],
            ))
            .await?
            .map(|row| {
                Ok::<_, DbErr>((
                    row.try_get::<i64>("", "size")?,
                    row.try_get::<String>("", "sequencer")?,
                ))
            })
            .transpose()?;
        if snapshot
            .as_ref()
            .is_some_and(|(_, sequencer)| !newer_than(&observation.sequencer, sequencer))
        {
            return Ok(());
        }
        if round == 0 && snapshot.is_none() {
            observation.legacy_size = non_negative(read_legacy_size().await?)?;
        }
        let size = non_negative(read_current_size().await?)?;
        let applied = retry_transaction::<_, bool, DbErr>(db, dialect, None, &RetryPolicy::idempotent(), |txn| {
        let observation = observation.clone();
        let id = id.clone();
        let snapshot = snapshot.clone();
        Box::pin(async move {
            flow_like_db::coordination::app_capacity(txn, &observation.app_id).await?;
            let (accounted, inserted) = accounted_object(txn, &observation, &id, snapshot.is_some()).await?;
            let Accounted { size: old_size, sequencer: old_sequencer, payer_id: payer_snapshot } = accounted;
            if !newer_than(&observation.sequencer, &old_sequencer) { return Ok(true); }
            let unchanged = match &snapshot {
                Some((expected_size, expected_sequencer)) => *expected_size == old_size && expected_sequencer == &old_sequencer,
                None => inserted,
            };
            if !unchanged { return Ok(false); }
            let payer_id = txn.query_one_raw(Statement::from_sql_and_values(txn.get_database_backend(),
                r#"SELECT COALESCE((SELECT "payerId" FROM "ProjectCapacity" WHERE "appId" = $1),
                    (SELECT m."userId" FROM "Membership" m JOIN "App" a ON a."ownerRoleId" = m."roleId" AND a."id" = m."appId" WHERE a."id" = $1 ORDER BY m."userId" LIMIT 1)) AS "payerId""#,
                [observation.app_id.clone().into()])).await?.map(|row| row.try_get::<Option<String>>("", "payerId")).transpose()?.flatten().or(payer_snapshot);
            let delta = size.checked_sub(old_size)
                .ok_or_else(|| DbErr::Custom("object size delta overflow".into()))?;
            let mut account = None;
            if let Some(payer) = &payer_id {
                // An event that moves no bytes and has no upload grant changes nothing on the
                // account. Grants are only issued under this app lock, so none can appear
                // before commit; an existing grant is read again below, under the payer lock.
                let settles = delta != 0 || txn.query_one_raw(Statement::from_sql_and_values(txn.get_database_backend(),
                    r#"SELECT 1 AS "granted" FROM "StorageUploadGrant" WHERE "id" = $1"#, [id.clone().into()])).await?.is_some();
                if settles {
                    // Lock before changing App.totalSize: initialization reads that total under
                    // the same payer lock and must see either the before or after state.
                    txn.execute_raw(Statement::from_sql_and_values(txn.get_database_backend(),
                        r#"INSERT INTO "AccountCapacity" ("payerId") VALUES ($1) ON CONFLICT ("payerId") DO UPDATE SET "updatedAt"=now()"#,
                        [payer.clone().into()])).await?;
                    account = Some(payer);
                }
            }
            let updated = txn.execute_raw(Statement::from_sql_and_values(txn.get_database_backend(),
                r#"UPDATE "FileAccountingObject" SET "size" = $2, "sequencer" = $3, "payerId" = $4, "updatedAt" = now() WHERE "id" = $1 AND "size"=$5 AND "sequencer"=$6"#,
                [id.clone().into(), size.into(), observation.sequencer.into(), payer_id.clone().into(), old_size.into(), old_sequencer.into()]
            )).await?.rows_affected();
            if updated == 0 { return Ok(false); }
            let mut reserved_delta = None;
            if account.is_some() {
                if let Some(grant) = txn.query_one_raw(Statement::from_sql_and_values(txn.get_database_backend(),
                    r#"SELECT "maxBytes", "reservedBytes" FROM "StorageUploadGrant" WHERE "id" = $1"#, [id.clone().into()])).await? {
                    let max_bytes: i64 = grant.try_get("", "maxBytes")?;
                    let old_reserved: i64 = grant.try_get("", "reservedBytes")?;
                    let reserved = max_bytes.saturating_sub(size).max(0);
                    txn.execute_raw(Statement::from_sql_and_values(txn.get_database_backend(),
                        r#"UPDATE "StorageUploadGrant" SET "reservedBytes" = $2, "updatedAt" = now() WHERE "id" = $1"#, [id.clone().into(), reserved.into()])).await?;
                    reserved_delta = Some(reserved - old_reserved);
                }
            }
            if let Some(payer) = account {
                match reserved_delta {
                    // One write settles both totals; stored bytes still move only for a retained project.
                    Some(reserved_delta) => {
                        txn.execute_raw(Statement::from_sql_and_values(txn.get_database_backend(),
                            r#"UPDATE "AccountCapacity" SET "storageBytes" = CASE WHEN EXISTS(SELECT 1 FROM "ProjectCapacity" p WHERE p."appId"=$3 AND p."payerId"=$1) THEN GREATEST(0,"storageBytes" + $2) ELSE "storageBytes" END, "reservedStorageBytes" = GREATEST(0,"reservedStorageBytes" + $4), "updatedAt" = now() WHERE "payerId" = $1"#,
                            [payer.clone().into(), delta.into(), observation.app_id.clone().into(), reserved_delta.into()])).await?;
                    }
                    None if delta != 0 => {
                        txn.execute_raw(Statement::from_sql_and_values(txn.get_database_backend(),
                            r#"UPDATE "AccountCapacity" SET "storageBytes" = GREATEST(0,"storageBytes" + $2), "updatedAt" = now() WHERE "payerId" = $1 AND EXISTS(SELECT 1 FROM "ProjectCapacity" p WHERE p."appId"=$3 AND p."payerId"=$1)"#,
                            [payer.clone().into(), delta.into(), observation.app_id.clone().into()])).await?;
                    }
                    None => {}
                }
            }
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
            Ok(true)
        })
        }).await?;
        if applied {
            return Ok(());
        }
        tokio::task::yield_now().await;
    }
    Err(DbErr::Custom(
        "Object changed repeatedly while accounting; retry the notification".into(),
    ))
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
        sql(&db, r#"CREATE TABLE "MutationLock" (id BIGINT PRIMARY KEY, "updatedAt" TIMESTAMPTZ DEFAULT now())"#).await;
        sql(&db, r#"CREATE TABLE "App" (id TEXT PRIMARY KEY, "ownerRoleId" TEXT, "totalSize" BIGINT NOT NULL, CONSTRAINT fail_overwrite CHECK ("totalSize" <= 100))"#).await;
        sql(
            &db,
            r#"CREATE TABLE "User" (id TEXT PRIMARY KEY, "totalSize" BIGINT NOT NULL)"#,
        )
        .await;
        sql(&db, r#"CREATE TABLE "FileAccountingObject" (id TEXT PRIMARY KEY, bucket TEXT, "objectKey" TEXT, "appId" TEXT, "userId" TEXT, "payerId" TEXT, size BIGINT, sequencer TEXT, "updatedAt" TIMESTAMPTZ)"#).await;
        sql(&db, r#"INSERT INTO "App" VALUES ('app','owner-role',100)"#).await;
        sql(&db, r#"INSERT INTO "User" VALUES ('user',100)"#).await;
        sql(
            &db,
            r#"CREATE TABLE "Membership" ("userId" TEXT, "appId" TEXT, "roleId" TEXT)"#,
        )
        .await;
        sql(
            &db,
            r#"INSERT INTO "Membership" VALUES ('user','app','owner-role')"#,
        )
        .await;
        sql(
            &db,
            r#"CREATE TABLE "ProjectCapacity" ("appId" TEXT PRIMARY KEY, "payerId" TEXT)"#,
        )
        .await;
        sql(&db, r#"CREATE TABLE "AccountCapacity" ("payerId" TEXT PRIMARY KEY, "storageBytes" BIGINT DEFAULT 0, "reservedStorageBytes" BIGINT DEFAULT 0, "initialized" BOOLEAN DEFAULT false, "updatedAt" TIMESTAMPTZ)"#).await;
        sql(&db, r#"INSERT INTO "AccountCapacity" ("payerId", "storageBytes", "initialized") VALUES ('user',100,true)"#).await;
        sql(
            &db,
            r#"INSERT INTO "ProjectCapacity" VALUES ('app','user')"#,
        )
        .await;
        sql(&db, r#"CREATE TABLE "StorageUploadGrant" ("id" TEXT PRIMARY KEY, "maxBytes" BIGINT, "reservedBytes" BIGINT, "updatedAt" TIMESTAMPTZ)"#).await;
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

        // A slow HEAD holds no app/account lock. A newer event whose earlier
        // sample loses a race must resample before committing its larger sequencer.
        use std::sync::{
            atomic::{AtomicI64, AtomicUsize, Ordering},
            Arc,
        };
        let entered = Arc::new(tokio::sync::Notify::new());
        let release = Arc::new(tokio::sync::Notify::new());
        let current_size = Arc::new(AtomicI64::new(200));
        let samples = Arc::new(AtomicUsize::new(0));
        let first = {
            let db = db.clone();
            let entered = entered.clone();
            let release = release.clone();
            let current_size = current_size.clone();
            let samples = samples.clone();
            let event = Observation {
                sequencer: "6".into(),
                ..observation.clone()
            };
            tokio::spawn(async move {
                apply_current(&db, DbDialect::Postgres, event, move || {
                    let entered = entered.clone();
                    let release = release.clone();
                    let current_size = current_size.clone();
                    let samples = samples.clone();
                    async move {
                        let captured = current_size.load(Ordering::SeqCst);
                        if samples.fetch_add(1, Ordering::SeqCst) == 0 {
                            entered.notify_one();
                            release.notified().await;
                        }
                        Ok(captured)
                    }
                })
                .await
            })
        };
        entered.notified().await;
        // Both coordination locks remain available while the first HEAD waits.
        use sea_orm::TransactionTrait;
        let unrelated = db.begin().await.unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(1), async {
            flow_like_db::coordination::app_capacity(&unrelated, "app")
                .await
                .unwrap();
            unrelated
                .execute_raw(Statement::from_string(
                    DatabaseBackend::Postgres,
                    r#"UPDATE "AccountCapacity" SET "updatedAt"=now() WHERE "payerId"='user'"#,
                ))
                .await
                .unwrap();
        })
        .await
        .expect("HEAD must not hold broad SQL locks");
        unrelated.commit().await.unwrap();
        current_size.store(300, Ordering::SeqCst);
        tokio::time::timeout(
            std::time::Duration::from_secs(1),
            apply(
                &db,
                DbDialect::Postgres,
                Observation {
                    sequencer: "5".into(),
                    ..observation.clone()
                },
                300,
            ),
        )
        .await
        .expect("another event can finish during HEAD")
        .unwrap();
        release.notify_one();
        first.await.unwrap().unwrap();
        assert_eq!(
            samples.load(Ordering::SeqCst),
            2,
            "stale HEAD must be sampled again"
        );
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
        assert_eq!(
            value(
                &db,
                r#"SELECT "storageBytes" AS n FROM "AccountCapacity" WHERE "payerId" = 'user'"#
            )
            .await,
            400
        );
        // An object's payer survives deletion of the app and its membership.
        sql(&db, r#"DELETE FROM "Membership""#).await;
        sql(&db, r#"DELETE FROM "App""#).await;
        let late = Observation {
            bucket: "bucket".into(),
            key: "users/user/apps/app/file".into(),
            app_id: "app".into(),
            user_id: Some("user".into()),
            sequencer: "8".into(),
            legacy_size: 0,
        };
        apply(&db, DbDialect::Postgres, late.clone(), 0)
            .await
            .unwrap();
        apply(&db, DbDialect::Postgres, late, 0).await.unwrap();
        assert_eq!(
            value(
                &db,
                r#"SELECT "storageBytes" AS n FROM "AccountCapacity" WHERE "payerId" = 'user'"#
            )
            .await,
            0
        );
        let replayed = Observation {
            bucket: "bucket".into(),
            key: "users/user/apps/app/file".into(),
            app_id: "app".into(),
            user_id: Some("user".into()),
            sequencer: "9".into(),
            legacy_size: 0,
        };
        db.execute_raw(Statement::from_sql_and_values(DatabaseBackend::Postgres,
            r#"INSERT INTO "StorageUploadGrant" ("id","maxBytes","reservedBytes") VALUES ($1,400,400)"#, [replayed.id().into()])).await.unwrap();
        sql(
            &db,
            r#"UPDATE "AccountCapacity" SET "reservedStorageBytes" = 400"#,
        )
        .await;
        // A retained project already contributes while the account baseline is
        // incomplete. New events must update that partial total immediately.
        sql(&db, r#"UPDATE "AccountCapacity" SET initialized=false"#).await;
        apply(&db, DbDialect::Postgres, replayed.clone(), 400)
            .await
            .unwrap();
        assert_eq!(
            value(&db, r#"SELECT "storageBytes" AS n FROM "AccountCapacity""#).await,
            400
        );
        sql(&db, r#"UPDATE "AccountCapacity" SET initialized=true"#).await;
        apply(&db, DbDialect::Postgres, replayed.clone(), 400)
            .await
            .unwrap();
        assert_eq!(
            value(
                &db,
                r#"SELECT "storageBytes" + "reservedStorageBytes" AS n FROM "AccountCapacity""#
            )
            .await,
            400
        );
        assert_eq!(
            value(
                &db,
                r#"SELECT "reservedStorageBytes" AS n FROM "AccountCapacity""#
            )
            .await,
            0
        );
        apply(
            &db,
            DbDialect::Postgres,
            Observation {
                sequencer: "a".into(),
                ..replayed.clone()
            },
            0,
        )
        .await
        .unwrap();
        assert_eq!(
            value(
                &db,
                r#"SELECT "storageBytes" + "reservedStorageBytes" AS n FROM "AccountCapacity""#
            )
            .await,
            400
        );
        assert_eq!(
            value(
                &db,
                r#"SELECT "reservedStorageBytes" AS n FROM "AccountCapacity""#
            )
            .await,
            400
        );
        apply(
            &db,
            DbDialect::Postgres,
            Observation {
                sequencer: "b".into(),
                ..replayed
            },
            400,
        )
        .await
        .unwrap();
        assert_eq!(
            value(
                &db,
                r#"SELECT "storageBytes" + "reservedStorageBytes" AS n FROM "AccountCapacity""#
            )
            .await,
            400
        );
        // HEAD may run during an ownership transfer, but the SQL commit resolves
        // the current payer after acquiring the shared app lock.
        let transfer = db.begin().await.unwrap();
        flow_like_db::coordination::app_capacity(&transfer, "app")
            .await
            .unwrap();
        let sampled = Arc::new(tokio::sync::Notify::new());
        let observer = {
            let db = db.clone();
            let sampled = sampled.clone();
            tokio::spawn(async move {
                apply_current(
                    &db,
                    DbDialect::Postgres,
                    Observation {
                        bucket: "bucket".into(),
                        key: "users/user/apps/app/file".into(),
                        app_id: "app".into(),
                        user_id: Some("user".into()),
                        sequencer: "c".into(),
                        legacy_size: 0,
                    },
                    move || {
                        let sampled = sampled.clone();
                        async move {
                            sampled.notify_one();
                            Ok(450)
                        }
                    },
                )
                .await
                .unwrap();
            })
        };
        tokio::time::timeout(std::time::Duration::from_secs(1), sampled.notified())
            .await
            .expect("HEAD can finish while ownership transfer holds the app lock");
        for statement in [
            r#"DELETE FROM "StorageUploadGrant""#,
            r#"UPDATE "ProjectCapacity" SET "payerId"='recipient' WHERE "appId"='app'"#,
            r#"UPDATE "AccountCapacity" SET "storageBytes"=0 WHERE "payerId"='user'"#,
            r#"INSERT INTO "AccountCapacity" ("payerId","storageBytes",initialized) VALUES ('recipient',400,true)"#,
        ] {
            transfer
                .execute_raw(Statement::from_string(DatabaseBackend::Postgres, statement))
                .await
                .unwrap();
        }
        transfer.commit().await.unwrap();
        observer.await.unwrap();
        assert_eq!(
            value(
                &db,
                r#"SELECT "storageBytes" AS n FROM "AccountCapacity" WHERE "payerId"='recipient'"#
            )
            .await,
            450
        );
        assert_eq!(
            value(
                &db,
                r#"SELECT "storageBytes" AS n FROM "AccountCapacity" WHERE "payerId"='user'"#
            )
            .await,
            0
        );
        // An older delete sampled before a recreation cannot erase the new bytes.
        let delete_entered = Arc::new(tokio::sync::Notify::new());
        let delete_release = Arc::new(tokio::sync::Notify::new());
        let stale_delete = {
            let db = db.clone();
            let entered = delete_entered.clone();
            let release = delete_release.clone();
            tokio::spawn(async move {
                apply_current(
                    &db,
                    DbDialect::Postgres,
                    Observation {
                        bucket: "bucket".into(),
                        key: "users/user/apps/app/file".into(),
                        app_id: "app".into(),
                        user_id: Some("user".into()),
                        sequencer: "d".into(),
                        legacy_size: 0,
                    },
                    move || {
                        let entered = entered.clone();
                        let release = release.clone();
                        async move {
                            entered.notify_one();
                            release.notified().await;
                            Ok(0)
                        }
                    },
                )
                .await
                .unwrap();
            })
        };
        delete_entered.notified().await;
        apply(
            &db,
            DbDialect::Postgres,
            Observation {
                bucket: "bucket".into(),
                key: "users/user/apps/app/file".into(),
                app_id: "app".into(),
                user_id: Some("user".into()),
                sequencer: "e".into(),
                legacy_size: 0,
            },
            500,
        )
        .await
        .unwrap();
        delete_release.notify_one();
        stale_delete.await.unwrap();
        assert_eq!(
            value(&db, r#"SELECT size AS n FROM "FileAccountingObject""#).await,
            500
        );
        assert_eq!(
            value(
                &db,
                r#"SELECT "storageBytes" AS n FROM "AccountCapacity" WHERE "payerId"='recipient'"#
            )
            .await,
            500
        );
        let final_event = Observation {
            bucket: "bucket".into(),
            key: "users/user/apps/app/file".into(),
            app_id: "app".into(),
            user_id: Some("user".into()),
            sequencer: "e".into(),
            legacy_size: 0,
        };
        apply_current(&db, DbDialect::Postgres, final_event.clone(), || async {
            Err(DbErr::Custom("duplicate events must not call HEAD".into()))
        })
        .await
        .unwrap();
        // Continual same-size updates still invalidate old samples through their
        // sequencer. Stop after a bounded number and leave the notification retryable.
        let changing_db = db.clone();
        let sample_count = Arc::new(AtomicUsize::new(0));
        let observed_samples = sample_count.clone();
        let changed = apply_current(
            &db,
            DbDialect::Postgres,
            Observation {
                sequencer: "ff".into(),
                ..final_event
            },
            move || {
                let db = changing_db.clone();
                let sample_count = sample_count.clone();
                async move {
                    let next = 15 + sample_count.fetch_add(1, Ordering::SeqCst);
                    db.execute_raw(Statement::from_sql_and_values(
                        DatabaseBackend::Postgres,
                        r#"UPDATE "FileAccountingObject" SET sequencer=$1"#,
                        [format!("{next:x}").into()],
                    ))
                    .await?;
                    Ok(600)
                }
            },
        )
        .await
        .unwrap_err();
        assert!(changed.to_string().contains("retry the notification"));
        assert_eq!(observed_samples.load(Ordering::SeqCst), 8);
        assert_eq!(
            value(&db, r#"SELECT size AS n FROM "FileAccountingObject""#).await,
            500
        );
        // An event that moves no bytes and has no grant commits without the payer lock.
        let payer_held = db.begin().await.unwrap();
        payer_held
            .execute_raw(Statement::from_string(
                DatabaseBackend::Postgres,
                r#"UPDATE "AccountCapacity" SET "updatedAt"=now() WHERE "payerId"='recipient'"#,
            ))
            .await
            .unwrap();
        let empty = Observation {
            bucket: "bucket".into(),
            key: "users/user/apps/app/empty".into(),
            app_id: "app".into(),
            user_id: Some("user".into()),
            sequencer: "1".into(),
            legacy_size: 0,
        };
        tokio::time::timeout(
            std::time::Duration::from_secs(1),
            apply(&db, DbDialect::Postgres, empty.clone(), 0),
        )
        .await
        .expect("an event that changes nothing on the account must not wait for the payer lock")
        .unwrap();
        payer_held.rollback().await.unwrap();
        // The same event on a granted object still settles its reservation under that lock.
        let granted = Observation {
            key: "users/user/apps/app/granted".into(),
            ..empty
        };
        db.execute_raw(Statement::from_sql_and_values(DatabaseBackend::Postgres,
            r#"INSERT INTO "StorageUploadGrant" ("id","maxBytes","reservedBytes") VALUES ($1,100,40)"#, [granted.id().into()])).await.unwrap();
        let reserved_before = value(
            &db,
            r#"SELECT "reservedStorageBytes" AS n FROM "AccountCapacity" WHERE "payerId"='recipient'"#,
        )
        .await;
        apply(&db, DbDialect::Postgres, granted, 0).await.unwrap();
        assert_eq!(
            value(
                &db,
                r#"SELECT "reservedStorageBytes" AS n FROM "AccountCapacity" WHERE "payerId"='recipient'"#
            )
            .await,
            reserved_before + 60
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
