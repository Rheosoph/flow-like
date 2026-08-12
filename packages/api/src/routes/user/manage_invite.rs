use crate::{
    entity::{
        app::{self},
        invitation, membership,
        sea_orm_active_enums::Visibility,
    },
    error::ApiError,
    middleware::jwt::AppUser,
    state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, State},
};
use flow_like_types::create_id;
use sea_orm::{
    ActiveModelTrait, ActiveValue::Set, ColumnTrait, EntityTrait, PaginatorTrait, QueryFilter,
    TransactionTrait, sea_query::OnConflict,
};

#[utoipa::path(
    delete,
    path = "/user/invites/{invite_id}",
    tag = "user",
    params(
        ("invite_id" = String, Path, description = "Invitation ID to reject")
    ),
    responses(
        (status = 200, description = "Invitation rejected"),
        (status = 401, description = "Unauthorized")
    ),
    security(
        ("bearer_auth" = [])
    )
)]
#[tracing::instrument(name = "DELETE /user/invites/{invite_id}", skip(state, user))]
pub async fn reject_invite(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(invite_id): Path<String>,
) -> Result<Json<()>, ApiError> {
    let sub = user.sub()?;

    invitation::Entity::delete_many()
        .filter(invitation::Column::Id.eq(invite_id.clone()))
        .filter(invitation::Column::UserId.eq(sub))
        .exec(&state.db)
        .await?;

    Ok(Json(()))
}

#[utoipa::path(
    post,
    path = "/user/invites/{invite_id}",
    tag = "user",
    params(
        ("invite_id" = String, Path, description = "Invitation ID to accept")
    ),
    responses(
        (status = 200, description = "Invitation accepted"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden - app is private or user limit reached"),
        (status = 404, description = "Invitation not found")
    ),
    security(
        ("bearer_auth" = [])
    )
)]
#[tracing::instrument(name = "POST /user/invites/{invite_id}", skip(state, user))]
pub async fn accept_invite(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(invite_id): Path<String>,
) -> Result<Json<()>, ApiError> {
    let sub = user.sub()?;

    let max_prototype = state.platform_config.max_users_prototype.unwrap_or(-1);

    let txn = state.db.begin().await?;

    let (invite, app) = invitation::Entity::find_by_id(invite_id.clone())
        .filter(invitation::Column::UserId.eq(sub.clone()))
        .find_also_related(app::Entity)
        .one(&txn)
        .await?
        .ok_or(ApiError::NOT_FOUND)?;

    let app = app.ok_or(ApiError::NOT_FOUND)?;
    let default_role = app.default_role_id.ok_or(ApiError::NOT_FOUND)?;

    if matches!(app.visibility, Visibility::Offline | Visibility::Private) {
        tracing::warn!(
            "User {} is trying to accept an invite to app {} but the app is private or offline",
            sub,
            app.id
        );
        return Err(ApiError::FORBIDDEN);
    }

    if max_prototype > 0 && app.visibility == Visibility::Prototype {
        let user_count = membership::Entity::find()
            .filter(membership::Column::AppId.eq(app.id.clone()))
            .count(&txn)
            .await?;

        if user_count >= max_prototype as u64 {
            tracing::warn!(
                "User {} is trying to accept an invite to app {} but the user limit has been reached",
                sub,
                app.id
            );
            return Err(ApiError::FORBIDDEN);
        }
    }

    // The user may already be a member via an invite link or approved join
    // request while a stale invitation lingers. Inserting again would hit the
    // (userId, appId) unique constraint and 500; instead, clear the invite and
    // report success so the pending card disappears and the counts settle.
    let already_member = membership::Entity::find()
        .filter(membership::Column::AppId.eq(app.id.clone()))
        .filter(membership::Column::UserId.eq(sub.clone()))
        .one(&txn)
        .await?
        .is_some();

    if already_member {
        let invite: invitation::ActiveModel = invite.into();
        invite.delete(&txn).await?;
        txn.commit().await?;
        return Ok(Json(()));
    }

    let membership = membership::ActiveModel {
        id: Set(create_id()),
        user_id: Set(sub),
        app_id: Set(app.id),
        role_id: Set(default_role),
        created_at: Set(chrono::Utc::now().naive_utc()),
        updated_at: Set(chrono::Utc::now().naive_utc()),
        joined_via: Set(Some("invite".to_string())),
    };

    // do_nothing guards the residual race with a concurrent join on the same
    // (userId, appId); either way the invite is consumed below.
    membership::Entity::insert(membership)
        .on_conflict(
            OnConflict::columns([membership::Column::UserId, membership::Column::AppId])
                .do_nothing()
                .to_owned(),
        )
        .exec_without_returning(&txn)
        .await?;

    let invite: invitation::ActiveModel = invite.into();
    invite.delete(&txn).await?;

    txn.commit().await?;

    Ok(Json(()))
}
