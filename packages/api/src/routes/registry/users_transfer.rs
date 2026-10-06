use crate::{
    entity::wasm_package_user, error::ApiError,
    permission::wasm_package_permission::WasmPackagePermission,
};
use sea_orm::{ColumnTrait, DatabaseTransaction, EntityTrait, QueryFilter, sea_query::Expr};

pub(super) async fn transfer_in_transaction(
    txn: &DatabaseTransaction,
    package_id: &str,
    caller_id: &str,
    target_id: &str,
) -> Result<wasm_package_user::Model, ApiError> {
    let caller = wasm_package_user::Entity::find()
        .filter(wasm_package_user::Column::PackageId.eq(package_id))
        .filter(wasm_package_user::Column::UserId.eq(caller_id))
        .one(txn)
        .await?
        .ok_or_else(|| ApiError::forbidden("Only the current Owner can transfer ownership"))?;
    if !WasmPackagePermission::from_bits_truncate(caller.permission)
        .contains(WasmPackagePermission::Owner)
    {
        return Err(ApiError::forbidden(
            "Only the current Owner can transfer ownership",
        ));
    }
    if caller_id == target_id {
        return Ok(caller);
    }
    let target = wasm_package_user::Entity::find()
        .filter(wasm_package_user::Column::PackageId.eq(package_id))
        .filter(wasm_package_user::Column::UserId.eq(target_id))
        .one(txn)
        .await?
        .ok_or_else(|| ApiError::not_found("Target user not found on this package"))?;
    let target_permission = WasmPackagePermission::from_bits_truncate(target.permission);
    if target_permission.contains(WasmPackagePermission::Owner) {
        return Err(ApiError::bad_request(
            "Target must have a non-owner permission",
        ));
    }
    let demoted_permission = if target_permission.contains(WasmPackagePermission::Buyer) {
        WasmPackagePermission::Maintainer
    } else {
        target_permission
    };

    // Both transfers contend on the same Owner row. The losing transaction
    // cannot promote its target after the first transfer consumes this grant.
    replace_permission(txn, &caller, demoted_permission).await?;
    replace_permission(txn, &target, WasmPackagePermission::Owner).await?;
    Ok(wasm_package_user::Model {
        permission: WasmPackagePermission::Owner.bits(),
        ..target
    })
}

async fn replace_permission(
    txn: &DatabaseTransaction,
    previous: &wasm_package_user::Model,
    permission: WasmPackagePermission,
) -> Result<(), ApiError> {
    let result = wasm_package_user::Entity::update_many()
        .col_expr(
            wasm_package_user::Column::Permission,
            Expr::value(permission.bits()),
        )
        .filter(wasm_package_user::Column::Id.eq(&previous.id))
        .filter(wasm_package_user::Column::Permission.eq(previous.permission))
        .exec(txn)
        .await?;
    if result.rows_affected != 1 {
        return Err(ApiError::conflict(
            "Package permission changed; retry the transfer",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use sea_orm::{
        ConnectOptions, ConnectionTrait, Database, DatabaseConnection, Schema, TransactionTrait,
        sea_query::Table,
    };

    async fn database() -> DatabaseConnection {
        let mut options = ConnectOptions::new("sqlite::memory:");
        options.max_connections(1);
        let db = Database::connect(options).await.unwrap();
        db.execute_unprepared("ATTACH DATABASE ':memory:' AS public")
            .await
            .unwrap();
        let schema = Schema::new(db.get_database_backend());
        let source = schema.create_table_from_entity(wasm_package_user::Entity);
        let mut table = Table::create();
        table.table(source.get_table_name().unwrap().clone());
        for column in source.get_columns() {
            table.col(column.clone());
        }
        // Only the grant table is needed; its production foreign keys belong
        // to entities outside this isolated transaction fixture.
        db.execute(&table).await.unwrap();
        db.execute_unprepared(r#"INSERT INTO public."WasmPackageUser" (id,"packageId","userId",permission,"grantedAt") VALUES ('owner','pkg','owner',1,'2026-01-01T00:00:00+00:00'),('first','pkg','first',2,'2026-01-01T00:00:00+00:00'),('second','pkg','second',4,'2026-01-01T00:00:00+00:00')"#).await.unwrap();
        db
    }

    #[tokio::test]
    async fn one_owner_grant_can_only_be_consumed_once() {
        let db = database().await;
        let stale_owner = wasm_package_user::Entity::find_by_id("owner")
            .one(&db)
            .await
            .unwrap()
            .unwrap();
        let txn = db.begin().await.unwrap();
        transfer_in_transaction(&txn, "pkg", "owner", "first")
            .await
            .unwrap();
        txn.commit().await.unwrap();

        let txn = db.begin().await.unwrap();
        assert!(
            replace_permission(&txn, &stale_owner, WasmPackagePermission::User)
                .await
                .is_err()
        );
        assert!(
            transfer_in_transaction(&txn, "pkg", "owner", "second")
                .await
                .is_err()
        );
        txn.rollback().await.unwrap();
        let owners = wasm_package_user::Entity::find()
            .filter(wasm_package_user::Column::Permission.eq(WasmPackagePermission::Owner.bits()))
            .all(&db)
            .await
            .unwrap();
        assert_eq!(owners.len(), 1);
        assert_eq!(owners[0].user_id, "first");
    }

    #[tokio::test]
    async fn failed_promotion_rolls_back_the_owner_demotion() {
        let db = database().await;
        db.execute_unprepared("CREATE TRIGGER public.reject_promotion BEFORE UPDATE ON WasmPackageUser WHEN NEW.id = 'first' AND NEW.permission = 1 BEGIN SELECT RAISE(ABORT, 'injected promotion failure'); END").await.unwrap();
        let txn = db.begin().await.unwrap();
        assert!(
            transfer_in_transaction(&txn, "pkg", "owner", "first")
                .await
                .is_err()
        );
        txn.rollback().await.unwrap();
        let owner = wasm_package_user::Entity::find_by_id("owner")
            .one(&db)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(owner.permission, WasmPackagePermission::Owner.bits());
    }

    #[tokio::test]
    async fn direct_demotion_and_a_stale_transfer_cannot_both_consume_the_owner_grant() {
        for demotion_first in [true, false] {
            let db = database().await;
            let original_owner = wasm_package_user::Entity::find_by_id("owner")
                .one(&db)
                .await
                .unwrap()
                .unwrap();

            let first = db.begin().await.unwrap();
            if demotion_first {
                replace_permission(&first, &original_owner, WasmPackagePermission::User)
                    .await
                    .unwrap();
            } else {
                transfer_in_transaction(&first, "pkg", "owner", "first")
                    .await
                    .unwrap();
            }
            first.commit().await.unwrap();

            let stale = db.begin().await.unwrap();
            let attempted_role = if demotion_first {
                WasmPackagePermission::Maintainer
            } else {
                WasmPackagePermission::User
            };
            assert!(
                replace_permission(&stale, &original_owner, attempted_role)
                    .await
                    .is_err()
            );
            assert!(
                transfer_in_transaction(&stale, "pkg", "owner", "second")
                    .await
                    .is_err()
            );
            stale.rollback().await.unwrap();

            let previous = wasm_package_user::Entity::find_by_id("owner")
                .one(&db)
                .await
                .unwrap()
                .unwrap();
            let target = wasm_package_user::Entity::find_by_id("first")
                .one(&db)
                .await
                .unwrap()
                .unwrap();
            assert_eq!(
                previous.permission,
                if demotion_first {
                    WasmPackagePermission::User.bits()
                } else {
                    WasmPackagePermission::Maintainer.bits()
                }
            );
            assert_eq!(
                target.permission,
                if demotion_first {
                    WasmPackagePermission::Maintainer.bits()
                } else {
                    WasmPackagePermission::Owner.bits()
                }
            );
        }
    }

    #[tokio::test]
    async fn self_transfer_is_an_authorized_no_op() {
        let db = database().await;
        let txn = db.begin().await.unwrap();
        assert_eq!(
            transfer_in_transaction(&txn, "pkg", "owner", "owner")
                .await
                .unwrap()
                .permission,
            WasmPackagePermission::Owner.bits()
        );
        assert!(
            transfer_in_transaction(&txn, "pkg", "first", "first")
                .await
                .is_err()
        );
        txn.commit().await.unwrap();
        let owner = wasm_package_user::Entity::find_by_id("owner")
            .one(&db)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(owner.permission, WasmPackagePermission::Owner.bits());
    }

    #[tokio::test]
    async fn transfer_preserves_a_targets_zero_permission_for_the_previous_owner() {
        let db = database().await;
        db.execute_unprepared(
            "UPDATE public.WasmPackageUser SET permission = 0 WHERE id = 'first'",
        )
        .await
        .unwrap();
        let txn = db.begin().await.unwrap();
        let target = transfer_in_transaction(&txn, "pkg", "owner", "first")
            .await
            .unwrap();
        assert_eq!(target.permission, WasmPackagePermission::Owner.bits());
        let previous = wasm_package_user::Entity::find_by_id("owner")
            .one(&txn)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(previous.permission, 0);
        txn.rollback().await.unwrap();
    }

    #[tokio::test]
    async fn transfers_keep_recognized_roles_when_rows_contain_unknown_bits() {
        let db = database().await;
        db.execute_unprepared("UPDATE public.WasmPackageUser SET permission = permission | 64")
            .await
            .unwrap();
        let txn = db.begin().await.unwrap();
        let target = transfer_in_transaction(&txn, "pkg", "owner", "first")
            .await
            .unwrap();
        assert_eq!(target.permission, WasmPackagePermission::Owner.bits());
        let previous = wasm_package_user::Entity::find_by_id("owner")
            .one(&txn)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            previous.permission,
            WasmPackagePermission::Maintainer.bits()
        );
        txn.rollback().await.unwrap();
    }
}
