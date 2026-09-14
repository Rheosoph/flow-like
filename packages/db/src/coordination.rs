use sea_orm::{ConnectionTrait, DatabaseTransaction, DbErr, Statement};

/// Coordinate a short transaction across the API and workers. Every writer uses
/// the same retained row; optimistic engines retry the losing transaction.
pub async fn app_capacity(txn: &DatabaseTransaction, app_id: &str) -> Result<(), DbErr> {
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"flow-like.transaction-lock/v1\0");
    for part in ["app-capacity", app_id] {
        hasher.update(&(part.len() as u64).to_be_bytes());
        hasher.update(part.as_bytes());
    }
    let id = i64::from_be_bytes(hasher.finalize().as_bytes()[..8].try_into().unwrap());
    for sql in [
        r#"INSERT INTO "MutationLock" (id) VALUES ($1) ON CONFLICT (id) DO NOTHING"#,
        r#"UPDATE "MutationLock" SET "updatedAt" = now() WHERE id = $1"#,
    ] {
        txn.execute_raw(Statement::from_sql_and_values(
            txn.get_database_backend(),
            sql,
            [id.into()],
        ))
        .await?;
    }
    Ok(())
}
