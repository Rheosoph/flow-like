use crate::entity::pat;
use sea_orm::{
    ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter, prelude::DateTimeWithTimeZone,
};

pub(super) fn lookup_parts(token: &str) -> Option<(&str, String)> {
    let mut parts = token.strip_prefix("pat_")?.split('.');
    let id = parts.next()?.trim();
    let secret = parts.next()?;
    if id.is_empty() || secret.is_empty() || parts.next().is_some() {
        return None;
    }
    Some((id, blake3::hash(secret.as_bytes()).to_hex().to_string()))
}

/// PAT revocation is a database operation, so every request reads the current
/// credential instead of trusting a process-local authentication cache.
pub(super) async fn lookup_current<C: ConnectionTrait>(
    db: &C,
    token: &str,
    now: DateTimeWithTimeZone,
) -> Result<Option<pat::Model>, sea_orm::DbErr> {
    let Some((id, secret_hash)) = lookup_parts(token) else {
        return Ok(None);
    };
    let record = pat::Entity::find()
        .filter(pat::Column::Id.eq(id))
        .filter(pat::Column::Key.eq(secret_hash))
        .one(db)
        .await?;
    Ok(record.filter(|record| record.valid_until.is_none_or(|expiry| expiry >= now)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use sea_orm::{
        ActiveValue::Set,
        ConnectOptions, Database, DatabaseConnection, Schema,
        sea_query::{Expr, Table},
    };

    async fn database() -> DatabaseConnection {
        let mut options = ConnectOptions::new("sqlite::memory:");
        options.max_connections(1);
        let db = Database::connect(options).await.unwrap();
        db.execute_unprepared("ATTACH DATABASE ':memory:' AS public")
            .await
            .unwrap();
        let source = Schema::new(db.get_database_backend()).create_table_from_entity(pat::Entity);
        let mut table = Table::create();
        table.table(source.get_table_name().unwrap().clone());
        for column in source.get_columns() {
            table.col(column.clone());
        }
        db.execute(&table).await.unwrap();
        let now = chrono::Utc::now().fixed_offset();
        pat::Entity::insert(pat::ActiveModel {
            id: Set("id".into()),
            user_id: Set("owner".into()),
            name: Set("test".into()),
            key: Set(blake3::hash(b"secret").to_hex().to_string()),
            permissions: Set(1),
            valid_until: Set(Some(now + chrono::Duration::minutes(1))),
            created_at: Set(now),
            updated_at: Set(now),
        })
        .exec(&db)
        .await
        .unwrap();
        db
    }

    #[tokio::test]
    async fn every_replica_sees_revocation_and_permission_changes() {
        let first = database().await;
        let second = first.clone();
        let now = chrono::Utc::now().fixed_offset();
        for db in [&first, &second] {
            assert!(
                lookup_current(db, "pat_id.secret", now)
                    .await
                    .unwrap()
                    .is_some()
            );
        }
        pat::Entity::update_many()
            .col_expr(pat::Column::Permissions, Expr::value(2))
            .exec(&first)
            .await
            .unwrap();
        assert_eq!(
            lookup_current(&second, "pat_id.secret", now)
                .await
                .unwrap()
                .unwrap()
                .permissions,
            2
        );
        pat::Entity::delete_by_id("id").exec(&first).await.unwrap();
        for db in [&first, &second] {
            assert!(
                lookup_current(db, "pat_id.secret", now)
                    .await
                    .unwrap()
                    .is_none()
            );
        }
    }

    #[tokio::test]
    async fn expiry_and_secret_are_checked_on_every_lookup() {
        let db = database().await;
        let now = chrono::Utc::now().fixed_offset();
        let pat = lookup_current(&db, "pat_id.secret", now)
            .await
            .unwrap()
            .unwrap();
        let expiry = pat.valid_until.unwrap();
        assert!(
            lookup_current(&db, "pat_id.secret", expiry)
                .await
                .unwrap()
                .is_some()
        );
        assert!(
            lookup_current(&db, "pat_id.secret", expiry + chrono::Duration::seconds(1))
                .await
                .unwrap()
                .is_none()
        );
        for token in [
            "pat_id.wrong",
            "pat_id.",
            "pat_id.secret.extra",
            "pat_.secret",
        ] {
            assert!(
                lookup_current(&db, token, now).await.unwrap().is_none(),
                "{token}"
            );
        }
    }
}
