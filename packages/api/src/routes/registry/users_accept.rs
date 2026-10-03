use crate::{
    entity::{sea_orm_active_enums::InvitationStatus, wasm_package_invitation, wasm_package_user},
    error::ApiError,
};
use sea_orm::{
    ActiveValue::Set,
    ColumnTrait, DatabaseTransaction, EntityTrait, QueryFilter,
    sea_query::{Expr, OnConflict},
};

pub(super) async fn accept_in_transaction(
    txn: &DatabaseTransaction,
    invitation_id: &str,
    caller_id: &str,
    membership_id: String,
) -> Result<wasm_package_user::Model, ApiError> {
    let invitation = wasm_package_invitation::Entity::find_by_id(invitation_id)
        .one(txn)
        .await?
        .ok_or_else(|| ApiError::not_found("Invitation not found"))?;

    if invitation.invitee_id != caller_id {
        return Err(ApiError::forbidden("This invitation is not for you"));
    }
    if invitation.status != InvitationStatus::Pending {
        return Err(ApiError::bad_request("Invitation is no longer pending"));
    }

    let now = chrono::Utc::now().fixed_offset();
    if invitation
        .expires_at
        .is_some_and(|expires_at| now > expires_at)
    {
        return Err(ApiError::bad_request("Invitation has expired"));
    }

    // Access may have been granted through another route since the invitation.
    // Keep that membership and its permission when consuming the pending invite.
    wasm_package_user::Entity::insert(wasm_package_user::ActiveModel {
        id: Set(membership_id),
        package_id: Set(invitation.package_id.clone()),
        user_id: Set(caller_id.to_string()),
        permission: Set(invitation.permission),
        granted_by: Set(Some(invitation.invited_by_id)),
        granted_at: Set(now),
    })
    .on_conflict(
        OnConflict::columns([
            wasm_package_user::Column::PackageId,
            wasm_package_user::Column::UserId,
        ])
        .do_nothing()
        .to_owned(),
    )
    .exec_without_returning(txn)
    .await?;

    let membership = wasm_package_user::Entity::find()
        .filter(wasm_package_user::Column::PackageId.eq(&invitation.package_id))
        .filter(wasm_package_user::Column::UserId.eq(caller_id))
        .one(txn)
        .await?
        .ok_or_else(|| ApiError::internal("Package membership missing after acceptance"))?;

    let updated = wasm_package_invitation::Entity::update_many()
        .col_expr(
            wasm_package_invitation::Column::Status,
            Expr::value(InvitationStatus::Accepted),
        )
        .filter(wasm_package_invitation::Column::Id.eq(invitation_id))
        .filter(wasm_package_invitation::Column::Status.eq(InvitationStatus::Pending))
        .exec(txn)
        .await?;
    if updated.rows_affected != 1 {
        return Err(ApiError::bad_request("Invitation is no longer pending"));
    }

    Ok(membership)
}

#[cfg(test)]
mod tests {
    use super::*;
    use sea_orm::{
        ConnectionTrait, Database, DatabaseBackend, Schema, TransactionTrait, sea_query::Table,
    };

    #[tokio::test]
    #[ignore = "requires FLOW_LIKE_INVITATION_TEST_DATABASE_URL pointing to a disposable PostgreSQL database"]
    async fn acceptance_preserves_existing_access_and_rolls_back_failed_grants() {
        let url = std::env::var("FLOW_LIKE_INVITATION_TEST_DATABASE_URL")
            .expect("set FLOW_LIKE_INVITATION_TEST_DATABASE_URL to a disposable database");
        let db = Database::connect(url).await.unwrap();
        let fixture = db.begin().await.unwrap();
        let schema = Schema::new(DatabaseBackend::Postgres);
        for source in [
            schema.create_table_from_entity(wasm_package_invitation::Entity),
            schema.create_table_from_entity(wasm_package_user::Entity),
        ] {
            let mut table = Table::create();
            table.table(source.get_table_name().unwrap().clone());
            for column in source.get_columns() {
                table.col(column.clone());
            }
            fixture.execute(&table).await.unwrap();
        }
        fixture.execute_unprepared(
            r#"ALTER TABLE "WasmPackageUser" ADD UNIQUE ("packageId", "userId");
            ALTER TABLE "WasmPackageInvitation" ADD CHECK (id <> 'rollback' OR status <> 'ACCEPTED');
            INSERT INTO "WasmPackageInvitation"
            (id,"packageId","invitedById","inviteeId",permission,status,"createdAt","expiresAt")
            VALUES
              ('fresh','fresh','owner','recipient',4,'PENDING',now(),null),
              ('existing','existing','owner','recipient',4,'PENDING',now(),null),
              ('foreign','foreign','owner','other',4,'PENDING',now(),null),
              ('expired','expired','owner','recipient',4,'PENDING',now(),now()-interval '1 day'),
              ('rejected','rejected','owner','recipient',4,'REJECTED',now(),null),
              ('rollback','rollback','owner','recipient',4,'PENDING',now(),null);
            INSERT INTO "WasmPackageUser" (id,"packageId","userId",permission,"grantedBy","grantedAt")
            VALUES ('existing-member','existing','recipient',2,'earlier-owner',now()-interval '1 day')"#,
        ).await.unwrap();

        let txn = fixture.begin().await.unwrap();
        let granted = accept_in_transaction(&txn, "fresh", "recipient", "new-member".into())
            .await
            .unwrap();
        txn.commit().await.unwrap();
        assert_eq!(granted.permission, 4);
        assert_eq!(granted.granted_by.as_deref(), Some("owner"));

        let existing = wasm_package_user::Entity::find_by_id("existing-member")
            .one(&fixture)
            .await
            .unwrap()
            .unwrap();
        let txn = fixture.begin().await.unwrap();
        let kept = accept_in_transaction(&txn, "existing", "recipient", "unused-member".into())
            .await
            .unwrap();
        txn.commit().await.unwrap();
        assert_eq!(kept, existing);
        for id in ["fresh", "existing"] {
            assert_eq!(
                wasm_package_invitation::Entity::find_by_id(id)
                    .one(&fixture)
                    .await
                    .unwrap()
                    .unwrap()
                    .status,
                InvitationStatus::Accepted,
            );
        }

        for (id, expected_message) in [
            ("foreign", "This invitation is not for you"),
            ("expired", "Invitation has expired"),
            ("rejected", "Invitation is no longer pending"),
            ("fresh", "Invitation is no longer pending"),
        ] {
            let txn = fixture.begin().await.unwrap();
            let error = accept_in_transaction(&txn, id, "recipient", format!("refused-{id}"))
                .await
                .unwrap_err();
            txn.rollback().await.unwrap();
            assert_eq!(error.public_message(), Some(expected_message));
        }

        // A failed status write must not leave a grant behind.
        let txn = fixture.begin().await.unwrap();
        assert!(
            accept_in_transaction(&txn, "rollback", "recipient", "rolled-back-member".into())
                .await
                .is_err()
        );
        txn.rollback().await.unwrap();
        assert!(
            wasm_package_user::Entity::find_by_id("rolled-back-member")
                .one(&fixture)
                .await
                .unwrap()
                .is_none()
        );
        assert_eq!(
            wasm_package_invitation::Entity::find_by_id("rollback")
                .one(&fixture)
                .await
                .unwrap()
                .unwrap()
                .status,
            InvitationStatus::Pending
        );
        assert_eq!(
            wasm_package_user::Entity::find()
                .all(&fixture)
                .await
                .unwrap()
                .len(),
            2
        );
        fixture.rollback().await.unwrap();
    }
}
