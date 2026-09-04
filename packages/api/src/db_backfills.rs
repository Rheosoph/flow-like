use crate::db::{DEFAULT_WRITE_CHUNK, DbDialect, update_in_batches};
use crate::entity::{technical_user, user};
use sea_orm::sea_query::{Expr, Func, SimpleExpr};
use sea_orm::{ColumnTrait, Condition, DatabaseConnection, DbErr};

/// Repairs rows older than the columns that now carry defaults, one bounded chunk of
/// ids per transaction so the sweep fits every engine's per-transaction row cap.
pub async fn run_startup_backfills(
    db: &DatabaseConnection,
    dialect: DbDialect,
) -> Result<(), DbErr> {
    let users = backfill_legacy_user_defaults(db, dialect).await?;
    let technical_users = backfill_legacy_technical_user_creators(db, dialect).await?;
    if users > 0 || technical_users > 0 {
        tracing::info!(%dialect, users, technical_users, "startup backfills repaired legacy rows");
    }
    Ok(())
}

fn coalesce<C: ColumnTrait>(column: C, fallback: impl Into<SimpleExpr>) -> SimpleExpr {
    Func::coalesce([Expr::col(column).into(), fallback.into()]).into()
}

async fn backfill_legacy_user_defaults(
    db: &DatabaseConnection,
    dialect: DbDialect,
) -> Result<u64, DbErr> {
    use user::Column;

    let now = Expr::current_timestamp();
    let condition = Condition::any()
        .add(Column::Permission.is_null())
        .add(Column::TutorialCompleted.is_null())
        .add(Column::Status.is_null())
        .add(Column::Tier.is_null())
        .add(Column::TotalSize.is_null())
        .add(Column::TotalLlmPrice.is_null())
        .add(Column::TotalEmbeddingPrice.is_null())
        .add(Column::CreatedAt.is_null())
        .add(Column::UpdatedAt.is_null());
    let set = vec![
        (Column::Permission, coalesce(Column::Permission, 0i64)),
        (
            Column::TutorialCompleted,
            coalesce(Column::TutorialCompleted, false),
        ),
        (Column::Status, coalesce(Column::Status, "ACTIVE")),
        (Column::Tier, coalesce(Column::Tier, "FREE")),
        (Column::TotalSize, coalesce(Column::TotalSize, 0i64)),
        (Column::TotalLlmPrice, coalesce(Column::TotalLlmPrice, 0i64)),
        (
            Column::TotalEmbeddingPrice,
            coalesce(Column::TotalEmbeddingPrice, 0i64),
        ),
        (Column::CreatedAt, coalesce(Column::CreatedAt, now.clone())),
        (Column::UpdatedAt, coalesce(Column::UpdatedAt, now)),
    ];

    update_in_batches::<user::Entity>(db, dialect, condition, set, DEFAULT_WRITE_CHUNK).await
}

/// The membership holding the app's owner role, correlated to the row being updated.
const OWNER_MEMBERSHIP_SQL: &str = r#"
SELECT m."{column}"
FROM "Membership" m
JOIN "App" a ON a."id" = m."appId"
WHERE a."id" = "TechnicalUser"."appId"
  AND a."ownerRoleId" IS NOT NULL
  AND m."roleId" = a."ownerRoleId"
LIMIT 1
"#;

fn owner_membership(column: &str) -> SimpleExpr {
    Expr::cust(format!(
        "({})",
        OWNER_MEMBERSHIP_SQL.replace("{column}", column)
    ))
}

async fn backfill_legacy_technical_user_creators(
    db: &DatabaseConnection,
    dialect: DbDialect,
) -> Result<u64, DbErr> {
    use technical_user::Column;

    let condition = Condition::all()
        .add(Column::CreatorUserId.is_null())
        .add(Expr::cust(format!(
            "EXISTS ({})",
            OWNER_MEMBERSHIP_SQL.replace("{column}", "id")
        )));
    let set = vec![
        (Column::CreatorUserId, owner_membership("userId")),
        (
            Column::CreatorMembershipId,
            coalesce(Column::CreatorMembershipId, owner_membership("id")),
        ),
        (Column::UpdatedAt, Expr::current_timestamp()),
    ];

    update_in_batches::<technical_user::Entity>(db, dialect, condition, set, DEFAULT_WRITE_CHUNK)
        .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_owner_membership_subquery_is_correlated_to_the_updated_row() {
        let sql = OWNER_MEMBERSHIP_SQL.replace("{column}", "userId");
        assert!(sql.contains(r#"a."id" = "TechnicalUser"."appId""#));
        assert!(sql.contains(r#"m."userId""#));
        assert!(sql.contains("LIMIT 1"));
    }
}
