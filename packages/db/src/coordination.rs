use sea_orm::{ConnectionTrait, DatabaseTransaction, DbErr, Statement};

pub const TOUCH_LOCK_ROW_SQL: &str = r#"INSERT INTO "MutationLock" ("id", "updatedAt") VALUES ($1, now()) ON CONFLICT ("id") DO UPDATE SET "updatedAt" = EXCLUDED."updatedAt""#;

/// Write the retained coordination row once, including when the key is new.
/// The conflict arm must update: DO NOTHING would not protect reads that follow
/// on optimistic engines. Lease ownership and expiry are preserved on conflict.
pub async fn touch_lock_row<C: ConnectionTrait>(connection: &C, lock_id: i64) -> Result<(), DbErr> {
    let result = connection
        .execute_raw(Statement::from_sql_and_values(
            connection.get_database_backend(),
            TOUCH_LOCK_ROW_SQL,
            [lock_id.into()],
        ))
        .await?;
    if result.rows_affected() != 1 {
        return Err(DbErr::RecordNotFound(format!(
            "mutation lock row {lock_id} was not written"
        )));
    }
    Ok(())
}

/// The API and workers must derive identical keys for the same resource.
pub fn transaction_lock_id(domain: &str, parts: &[&str]) -> i64 {
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"flow-like.transaction-lock/v1\0");
    for part in std::iter::once(domain).chain(parts.iter().copied()) {
        hasher.update(&(part.len() as u64).to_be_bytes());
        hasher.update(part.as_bytes());
    }
    i64::from_be_bytes(hasher.finalize().as_bytes()[..8].try_into().unwrap())
}

/// Coordinate a short transaction across the API and workers. Every writer uses
/// the same retained row; optimistic engines retry the losing transaction.
pub async fn app_capacity(txn: &DatabaseTransaction, app_id: &str) -> Result<(), DbErr> {
    touch_lock_row(txn, transaction_lock_id("app-capacity", &[app_id])).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{DbDialect, RetryPolicy, retry_transaction};
    use sea_orm::{ConnectOptions, Database, DatabaseBackend};

    #[tokio::test]
    #[ignore = "requires FLOW_LIKE_COORDINATION_TEST_DATABASE_URL pointing to an empty disposable PostgreSQL database"]
    async fn retained_upsert_coordinates_first_and_existing_rows_without_changing_leases() {
        let url = std::env::var("FLOW_LIKE_COORDINATION_TEST_DATABASE_URL").unwrap();
        let mut options = ConnectOptions::new(url);
        options.max_connections(8).min_connections(1);
        let db = Database::connect(options).await.unwrap();
        for query in [
            r#"CREATE TABLE "MutationLock" (id BIGINT PRIMARY KEY, owner TEXT, "expiresAt" TIMESTAMPTZ(3), "updatedAt" TIMESTAMPTZ(3) NOT NULL DEFAULT now())"#,
            r#"CREATE TABLE "CoordinationCounter" (id TEXT PRIMARY KEY, total BIGINT NOT NULL)"#,
            r#"INSERT INTO "CoordinationCounter" VALUES ('app', 0)"#,
        ] {
            db.execute_raw(Statement::from_string(DatabaseBackend::Postgres, query))
                .await
                .unwrap();
        }

        let id = transaction_lock_id("app-capacity", &["app"]);
        // The first round races on an absent row; the second starts with a retained row.
        for expected in [8, 16] {
            let mut tasks = Vec::new();
            for worker in 0..8 {
                let db = db.clone();
                tasks.push(tokio::spawn(async move {
                    retry_transaction(
                        &db,
                        DbDialect::Postgres,
                        None,
                        &RetryPolicy::default(),
                        move |txn| {
                            Box::pin(async move {
                                if worker % 2 == 0 {
                                    app_capacity(txn, "app").await?;
                                } else {
                                    touch_lock_row(txn, id).await?;
                                }
                                let row = txn
                                    .query_one_raw(Statement::from_string(
                                        DatabaseBackend::Postgres,
                                        r#"SELECT total FROM "CoordinationCounter" WHERE id='app'"#,
                                    ))
                                    .await?
                                    .unwrap();
                                let total = row.try_get::<i64>("", "total")?;
                                tokio::task::yield_now().await;
                                txn.execute_raw(Statement::from_sql_and_values(
                                    DatabaseBackend::Postgres,
                                    r#"UPDATE "CoordinationCounter" SET total=$1 WHERE id='app'"#,
                                    [(total + 1).into()],
                                ))
                                .await?;
                                Ok::<(), DbErr>(())
                            })
                        },
                    )
                    .await
                    .unwrap();
                }));
            }
            for task in tasks {
                task.await.unwrap();
            }
            let row = db
                .query_one_raw(Statement::from_string(
                    DatabaseBackend::Postgres,
                    r#"SELECT total FROM "CoordinationCounter" WHERE id='app'"#,
                ))
                .await
                .unwrap()
                .unwrap();
            assert_eq!(row.try_get::<i64>("", "total").unwrap(), expected);
        }
        db.execute_raw(Statement::from_sql_and_values(DatabaseBackend::Postgres, r#"UPDATE "MutationLock" SET owner='lease-owner', "expiresAt"='2030-01-01T00:00:00Z' WHERE id=$1"#, [id.into()])).await.unwrap();
        touch_lock_row(&db, id).await.unwrap();
        let row = db
            .query_one_raw(Statement::from_sql_and_values(
                DatabaseBackend::Postgres,
                r#"SELECT owner, "expiresAt"::text AS expiry FROM "MutationLock" WHERE id=$1"#,
                [id.into()],
            ))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(row.try_get::<String>("", "owner").unwrap(), "lease-owner");
        assert!(
            row.try_get::<String>("", "expiry")
                .unwrap()
                .starts_with("2030-01-01")
        );
        db.close().await.unwrap();
    }
}
