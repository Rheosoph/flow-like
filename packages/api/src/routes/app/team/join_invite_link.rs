use crate::{
    audit_branch,
    entity::{app, invite_link, membership, sea_orm_active_enums::Visibility},
    error::ApiError,
    middleware::jwt::AppUser,
    routes::user::ensure_user_exists,
    state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, State},
};
use flow_like_types::create_id;
use sea_orm::{
    ActiveValue::Set,
    ColumnTrait, Condition, EntityTrait, PaginatorTrait, QueryFilter, TransactionTrait,
    sea_query::{Expr, OnConflict},
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
#[tracing::instrument(
    name = "POST /apps/{app_id}/team/link/join/{token}",
    skip(state, user, token)
)]
pub async fn join_invite_link(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((app_id, token)): Path<(String, String)>,
) -> Result<Json<()>, ApiError> {
    let sub = user.sub()?;
    ensure_user_exists(&state, &sub).await?;
    // Tokens are credentials: never log or audit them in full.
    let token_hint: String = token.chars().take(8).collect();

    let max_prototype = state.platform_config.max_users_prototype.unwrap_or(-1);

    let membership_exists = membership::Entity::find()
        .filter(membership::Column::AppId.eq(app_id.clone()))
        .filter(membership::Column::UserId.eq(sub.clone()))
        .one(&state.db)
        .await?;

    if membership_exists.is_some() {
        tracing::debug!(
            "User {} redeemed an invite link for app {} but is already a member",
            sub,
            app_id
        );
        return Ok(Json(()));
    }

    let txn = state.db.begin().await?;

    let (invite_link, app) = invite_link::Entity::find()
        .filter(invite_link::Column::Token.eq(token.clone()))
        .filter(invite_link::Column::AppId.eq(app_id.clone()))
        .find_also_related(app::Entity)
        .one(&txn)
        .await?
        .ok_or_else(|| {
            tracing::warn!(
                "User {} attempted to join app {} with invalid invite token {}…",
                sub,
                app_id,
                token_hint
            );
            ApiError::NOT_FOUND
        })?;

    if let Some(expires_at) = invite_link.expires_at
        && expires_at < chrono::Utc::now().naive_utc()
    {
        tracing::warn!(
            "User {} attempted to join app {} with expired invite link {}",
            sub,
            app_id,
            invite_link.id
        );
        return Err(ApiError::NOT_FOUND);
    }

    let app = app.ok_or(ApiError::NOT_FOUND)?;

    if matches!(app.visibility, Visibility::Private | Visibility::Offline) {
        tracing::warn!(
            "User {} is trying to join app {} but the app is not public",
            sub,
            app_id
        );
        return Err(ApiError::FORBIDDEN);
    }

    let default_role_id = app.default_role_id.ok_or(ApiError::NOT_FOUND)?;

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

    // Atomic claim: the WHERE guard makes concurrent redeems of a limited link
    // serialize on the row instead of racing the earlier SELECT's snapshot.
    let claim = invite_link::Entity::update_many()
        .col_expr(
            invite_link::Column::CountJoined,
            Expr::col(invite_link::Column::CountJoined).add(1),
        )
        .col_expr(
            invite_link::Column::UpdatedAt,
            Expr::value(chrono::Utc::now().naive_utc()),
        )
        .filter(invite_link::Column::Id.eq(invite_link.id.clone()))
        .filter(
            Condition::any()
                .add(invite_link::Column::MaxUses.lte(0))
                .add(
                    Expr::col(invite_link::Column::CountJoined)
                        .lt(Expr::col(invite_link::Column::MaxUses)),
                ),
        )
        .exec(&txn)
        .await?;

    if claim.rows_affected == 0 {
        tracing::warn!(
            "User {} is trying to join app {} but invite link {} has reached its maximum uses",
            sub,
            app_id,
            invite_link.id
        );
        return Err(ApiError::FORBIDDEN);
    }

    // Invite links intentionally bypass the purchase flow: a link is a team
    // grant from an admin, not a storefront entry point.
    let new_membership = membership::ActiveModel {
        id: Set(create_id()),
        user_id: Set(sub.clone()),
        app_id: Set(app_id.clone()),
        role_id: Set(default_role_id),
        created_at: Set(chrono::Utc::now().naive_utc()),
        updated_at: Set(chrono::Utc::now().naive_utc()),
        joined_via: Set(Some("invite_link".to_string())),
    };

    let inserted = membership::Entity::insert(new_membership)
        .on_conflict(
            OnConflict::columns([membership::Column::UserId, membership::Column::AppId])
                .do_nothing()
                .to_owned(),
        )
        .exec_without_returning(&txn)
        .await?;

    if inserted == 0 {
        // Concurrent redeem by the same user: roll back so the counter claim
        // above is released, then report success like the fast path.
        txn.rollback().await?;
        return Ok(Json(()));
    }

    txn.commit().await?;

    let link_id = invite_link.id;
    audit_branch!(
        state,
        user,
        app_id,
        "membership.join",
        "InviteLink",
        link_id,
        "User joined via invite link"
    );
    Ok(Json(()))
}
