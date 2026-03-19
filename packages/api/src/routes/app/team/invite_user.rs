use crate::{
    audit_branch, ensure_permission,
    entity::{app, invitation, membership, meta, sea_orm_active_enums::{Visibility, NotificationType}},
    error::ApiError,
    middleware::jwt::AppUser,
    push_notifications::{DispatchNotificationInput, dispatch_notification},
    permission::role_permission::RolePermissions,
    state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, State},
};
use flow_like_types::{anyhow, create_id};
use sea_orm::{
    ActiveModelTrait, ActiveValue::Set, ColumnTrait, EntityTrait, PaginatorTrait, QueryFilter,
    TransactionTrait,
};
use utoipa::ToSchema;

#[derive(Debug, Clone, serde::Deserialize, ToSchema)]
pub struct InviteUserParams {
    pub message: Option<String>,
    pub sub: String,
}

#[utoipa::path(
    put,
    path = "/apps/{app_id}/team/invite",
    tag = "team",
    description = "Invite a user to the app.",
    params(
        ("app_id" = String, Path, description = "Application ID")
    ),
    request_body = InviteUserParams,
    responses(
        (status = 200, description = "Invite created", body = ()),
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
#[tracing::instrument(name = "PUT /apps/{app_id}/team/invite", skip(state, user))]
pub async fn invite_user(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(app_id): Path<String>,
    Json(params): Json<InviteUserParams>,
) -> Result<Json<()>, ApiError> {
    let permission = ensure_permission!(user, &app_id, &state, RolePermissions::Admin);

    if params.sub == permission.sub()? {
        tracing::warn!(
            "User {} is trying to invite themself to app {}",
            user.sub()?,
            app_id
        );
        return Err(ApiError::FORBIDDEN);
    }

    let txn = state.db.begin().await?;

    let max_prototype = state.platform_config.max_users_prototype.unwrap_or(-1);

    let (app, meta) = app::Entity::find_by_id(app_id.clone())
        .find_also_related(meta::Entity)
        .filter(meta::Column::Lang.eq("en"))
        .one(&txn)
        .await?
        .ok_or(ApiError::NOT_FOUND)?;

    if app.default_role_id.is_none() {
        tracing::warn!(
            "App {} does not have a default role set, cannot invite user",
            app_id
        );
        return Err(ApiError::internal_error(anyhow!(
            "App does not have a default role set"
        )));
    }

    if matches!(app.visibility, Visibility::Private | Visibility::Offline) {
        tracing::warn!(
            "User {} is trying to invite a user to app {} but the app is not public",
            user.sub()?,
            app_id
        );
        return Err(ApiError::FORBIDDEN);
    }

    let member = membership::Entity::find()
        .filter(membership::Column::AppId.eq(app_id.clone()))
        .filter(membership::Column::UserId.eq(user.sub()?))
        .one(&txn)
        .await?
        .ok_or(ApiError::FORBIDDEN)?;

    let user_already_member = membership::Entity::find()
        .filter(membership::Column::AppId.eq(app_id.clone()))
        .filter(membership::Column::UserId.eq(params.sub.clone()))
        .one(&txn)
        .await?;

    if user_already_member.is_some() {
        tracing::warn!(
            "User {} is trying to invite {} to app {} but the user is already a member",
            user.sub()?,
            params.sub,
            app_id
        );
        return Err(ApiError::FORBIDDEN);
    }

    if max_prototype > 0
        && !matches!(
            app.visibility,
            Visibility::Public | Visibility::PublicRequestAccess
        )
    {
        let count = membership::Entity::find()
            .filter(membership::Column::AppId.eq(app_id.clone()))
            .count(&txn)
            .await?;

        if count >= max_prototype as u64 {
            tracing::warn!(
                "User {} is trying to invite a user to app {} but the app has reached its user limit",
                user.sub()?,
                app_id
            );
            return Err(ApiError::FORBIDDEN);
        }
    }

    let existing_invite = invitation::Entity::find()
        .filter(invitation::Column::AppId.eq(app_id.clone()))
        .filter(invitation::Column::UserId.eq(params.sub.clone()))
        .one(&txn)
        .await?;

    if existing_invite.is_some() {
        tracing::warn!(
            "User {} is trying to invite {} to app {} but the user already has an invite",
            user.sub()?,
            params.sub,
            app_id
        );
        return Err(ApiError::FORBIDDEN);
    }

    let invitation = invitation::ActiveModel {
        id: Set(create_id()),
        app_id: Set(app_id.clone()),
        created_at: Set(chrono::Utc::now().naive_utc()),
        updated_at: Set(chrono::Utc::now().naive_utc()),
        by_member_id: Set(member.id.clone()),
        message: Set(params.message),
        user_id: Set(params.sub.clone()),
        name: Set(meta
            .as_ref()
            .map_or("Unknown App".to_string(), |m| m.name.clone())),
        description: Set(meta.as_ref().and_then(|m| m.description.clone())),
    };

    invitation.insert(&txn).await?;
    txn.commit().await?;

    let app_name = meta
        .as_ref()
        .map_or("an app".to_string(), |m| m.name.clone());

    if let Err(error) = dispatch_notification(
        &state,
        DispatchNotificationInput {
            user_id: params.sub.clone(),
            app_id: Some(app_id.clone()),
            title: format!("You've been invited to {}", app_name),
            description: Some("Open your invitations to accept or decline.".to_string()),
            icon: Some("mail".to_string()),
            link: Some("/invites".to_string()),
            image: None,
            notification_type: NotificationType::System,
            source_run_id: None,
            source_node_id: None,
        },
    )
    .await
    {
        tracing::warn!(error = %error, "Failed to dispatch invitation notification");
    }

    audit_branch!(
        state,
        user,
        app_id,
        "membership.invite",
        "Invitation",
        params.sub,
        "User invited"
    );
    Ok(Json(()))
}
