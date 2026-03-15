use crate::{
    audit_branch,
    entity::{app, invite_link, membership, sea_orm_active_enums::Visibility},
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
    TransactionTrait,
};

#[utoipa::path(
    post,
    path = "/apps/{app_id}/team/link/join/{token}",
    tag = "team",
    description = "Join an app via invite link.",
    params(
        ("app_id" = String, Path, description = "Application ID"),
        ("token" = String, Path, description = "Invite token")
    ),
    responses(
        (status = 200, description = "Joined app", body = ()),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
        (status = 404, description = "Not found")
    ),
    security(
        ("bearer_auth" = []),
        ("api_key" = []),
        ("pat" = [])
    )
)]
#[tracing::instrument(name = "POST /apps/{app_id}/team/link/join/{token}", skip(state, user))]
pub async fn join_invite_link(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((app_id, token)): Path<(String, String)>,
) -> Result<Json<()>, ApiError> {
    let sub = user.sub()?;
    let txn = state.db.begin().await?;

    let max_prototype = state.platform_config.max_users_prototype.unwrap_or(-1);

    let membership_exists = membership::Entity::find()
        .filter(membership::Column::AppId.eq(app_id.clone()))
        .filter(membership::Column::UserId.eq(sub.clone()))
        .one(&txn)
        .await?;

    if membership_exists.is_some() {
        tracing::warn!(
            "User {} is trying to join app {} but is already a member",
            sub,
            app_id
        );
        return Err(ApiError::FORBIDDEN);
    }

    let (invite_link, app) = invite_link::Entity::find()
        .filter(invite_link::Column::Token.eq(token.clone()))
        .filter(invite_link::Column::AppId.eq(app_id.clone()))
        .find_also_related(app::Entity)
        .one(&txn)
        .await?
        .ok_or_else(|| {
            tracing::warn!(
                "User {} attempted to join app {} with invalid invite token {}",
                sub,
                app_id,
                token
            );
            ApiError::NOT_FOUND
        })?;

    let current_count = invite_link.count_joined;
    let max_uses = invite_link.max_uses;

    if max_uses > 0 && current_count >= max_uses {
        tracing::warn!(
            "User {} is trying to join app {} but the invite link has reached its maximum uses",
            sub,
            app_id
        );
        return Err(ApiError::FORBIDDEN);
    }

    let app = app.ok_or(ApiError::NOT_FOUND)?;

    if matches!(app.visibility, Visibility::Private | Visibility::Offline) {
        tracing::warn!(
            "User {} is trying to invite a user to app {} but the app is not public",
            user.sub()?,
            app_id
        );
        return Err(ApiError::FORBIDDEN);
    }

    let default_role_id = app.default_role_id.ok_or(ApiError::NOT_FOUND)?;

    if matches!(app.visibility, Visibility::Offline | Visibility::Private) {
        tracing::warn!(
            "User {} is trying to join app {} but the app is private or offline",
            sub,
            app_id
        );

        return Err(ApiError::FORBIDDEN);
    }

    if max_prototype > 0 && app.visibility == Visibility::Prototype {
        let user_count = membership::Entity::find()
            .filter(membership::Column::AppId.eq(app_id.clone()))
            .count(&txn)
            .await?;

        if user_count >= max_prototype as u64 {
            tracing::warn!(
                "User {} is trying to accept an invite to app {} but the user limit has been reached",
                sub,
                app_id
            );
            return Err(ApiError::FORBIDDEN);
        }
    }

    let membership = membership::ActiveModel {
        id: Set(create_id()),
        user_id: Set(sub.clone()),
        app_id: Set(app_id.clone()),
        role_id: Set(default_role_id),
        created_at: Set(chrono::Utc::now().naive_utc()),
        updated_at: Set(chrono::Utc::now().naive_utc()),
        joined_via: Set(Some("invite_link".to_string())),
    };

    membership.insert(&txn).await?;

    let mut invite_link: invite_link::ActiveModel = invite_link.into();
    invite_link.count_joined = Set(current_count + 1);
    invite_link.updated_at = Set(chrono::Utc::now().naive_utc());
    invite_link.update(&txn).await?;

    txn.commit().await?;

    audit_branch!(state, user, app_id, "membership.join", "InviteLink", token, "User joined via invite link");
    Ok(Json(()))
}
