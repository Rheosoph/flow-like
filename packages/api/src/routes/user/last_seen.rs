//! When a person last had a signed-in app open. Every realtime connect refreshes it, and an open
//! app reconnects every few minutes, so it is kept to the hour: at most one write an hour.

use chrono::{DateTime, Duration, FixedOffset};
use sea_orm::{
    ColumnTrait, Condition, ConnectionTrait, DbErr, EntityTrait, QueryFilter, sea_query::Expr,
};

use crate::entity::user;

const PRECISION_MINUTES: i64 = 60;

/// Records that `user_id` is around at `now`, unless that is already known for the last hour.
/// Returns whether it wrote.
pub(crate) async fn mark_seen<C: ConnectionTrait>(
    db: &C,
    user_id: &str,
    now: DateTime<FixedOffset>,
) -> Result<bool, DbErr> {
    let written = user::Entity::update_many()
        .col_expr(user::Column::LastSeenAt, Expr::value(now))
        .filter(user::Column::Id.eq(user_id))
        .filter(
            Condition::any()
                .add(user::Column::LastSeenAt.is_null())
                .add(user::Column::LastSeenAt.lt(now - Duration::minutes(PRECISION_MINUTES))),
        )
        .exec(db)
        .await?;
    Ok(written.rows_affected == 1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use sea_orm::{ConnectOptions, Database, DatabaseConnection, QuerySelect};

    async fn database() -> DatabaseConnection {
        let mut options = ConnectOptions::new("sqlite::memory:");
        options.max_connections(1);
        let db = Database::connect(options).await.unwrap();
        db.execute_unprepared("ATTACH DATABASE ':memory:' AS public")
            .await
            .unwrap();
        db.execute_unprepared(
            r#"CREATE TABLE public."User" ("id" TEXT PRIMARY KEY, "lastSeenAt" TEXT);
               INSERT INTO public."User" ("id") VALUES ('ana'), ('ben')"#,
        )
        .await
        .unwrap();
        db
    }

    fn at(minutes: i64) -> DateTime<FixedOffset> {
        DateTime::from_timestamp(1_800_000_000 + minutes * 60, 0)
            .unwrap()
            .fixed_offset()
    }

    async fn seen(db: &DatabaseConnection, id: &str) -> Option<DateTime<FixedOffset>> {
        user::Entity::find_by_id(id)
            .select_only()
            .column(user::Column::LastSeenAt)
            .into_tuple::<Option<DateTime<FixedOffset>>>()
            .one(db)
            .await
            .unwrap()
            .flatten()
    }

    #[tokio::test]
    async fn the_first_connect_writes_and_reconnects_within_the_hour_do_not() {
        let db = database().await;
        assert!(mark_seen(&db, "ana", at(0)).await.unwrap());
        assert!(!mark_seen(&db, "ana", at(15)).await.unwrap());
        assert!(!mark_seen(&db, "ana", at(59)).await.unwrap());
        assert_eq!(seen(&db, "ana").await, Some(at(0)));
        assert!(mark_seen(&db, "ana", at(61)).await.unwrap());
        assert_eq!(seen(&db, "ana").await, Some(at(61)));
        assert_eq!(seen(&db, "ben").await, None);
        assert!(!mark_seen(&db, "nobody", at(0)).await.unwrap());
    }
}
