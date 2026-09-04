use crate::{
    audit_branch, ensure_permission,
    entity::{
        app, invitation, membership, meta,
        sea_orm_active_enums::{NotificationType, Visibility},
        user,
    },
    error::ApiError,
    middleware::jwt::AppUser,
    permission::role_permission::RolePermissions,
    push_notifications::{DispatchNotificationInput, dispatch_notification},
    state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, State},
};
use flow_like_types::{anyhow, create_id};
use sea_orm::{
    ActiveModelTrait, ActiveValue::Set, ColumnTrait, EntityTrait, PaginatorTrait, QueryFilter,
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
#[tracing::instrument(name = "PUT /apps/{app_id}/team/invite", skip(state, user, params))]
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
        return Err(ApiError::bad_request("You cannot invite yourself"));
    }

    let caller_sub = user.sub()?;
    let max_prototype = state.platform_config.max_users_prototype.unwrap_or(-1);
    let invitation_id = create_id();

    let app_name = state
        .transaction(|txn| {
            let app_id = app_id.clone();
            let caller_sub = caller_sub.clone();
            let invitee = params.sub.clone();
            let message = params.message.clone();
            let invitation_id = invitation_id.clone();
            Box::pin(async move {
                let (app, meta) = app::Entity::find_by_id(app_id.clone())
                    .find_also_related(meta::Entity)
                    .filter(meta::Column::Lang.eq("en"))
                    .one(txn)
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
                        caller_sub,
                        app_id
                    );
                    return Err(ApiError::FORBIDDEN);
                }

                let member = membership::Entity::find()
                    .filter(membership::Column::AppId.eq(app_id.clone()))
                    .filter(membership::Column::UserId.eq(caller_sub.clone()))
                    .one(txn)
                    .await?
                    .ok_or(ApiError::FORBIDDEN)?;

                let user_already_member = membership::Entity::find()
                    .filter(membership::Column::AppId.eq(app_id.clone()))
                    .filter(membership::Column::UserId.eq(invitee.clone()))
                    .one(txn)
                    .await?;

                if user_already_member.is_some() {
                    tracing::warn!(
                        "User {} is trying to invite {} to app {} but the user is already a member",
                        caller_sub,
                        invitee,
                        app_id
                    );
                    return Err(ApiError::conflict("User is already a member of this app"));
                }

                if max_prototype > 0
                    && !matches!(
                        app.visibility,
                        Visibility::Public | Visibility::PublicRequestAccess
                    )
                {
                    let count = membership::Entity::find()
                        .filter(membership::Column::AppId.eq(app_id.clone()))
                        .count(txn)
                        .await?;

                    if count >= max_prototype as u64 {
                        tracing::warn!(
                            "User {} is trying to invite a user to app {} but the app has reached its user limit",
                            caller_sub,
                            app_id
                        );
                        return Err(ApiError::FORBIDDEN);
                    }
                }

                // Invitation.user_id is a foreign key to User; a sub with no row (typo'd id
                // or a never-signed-in user from an API-key/PAT caller) would fail the
                // insert with an opaque 500. Surface a clean 404 instead.
                let invitee_exists = user::Entity::find_by_id(invitee.clone())
                    .one(txn)
                    .await?
                    .is_some();

                if !invitee_exists {
                    tracing::warn!(
                        "User {} is trying to invite unknown user {} to app {}",
                        caller_sub,
                        invitee,
                        app_id
                    );
                    return Err(ApiError::not_found("User not found"));
                }

                let existing_invite = invitation::Entity::find()
                    .filter(invitation::Column::AppId.eq(app_id.clone()))
                    .filter(invitation::Column::UserId.eq(invitee.clone()))
                    .one(txn)
                    .await?;

                if existing_invite.is_some() {
                    tracing::warn!(
                        "User {} is trying to invite {} to app {} but the user already has an invite",
                        caller_sub,
                        invitee,
                        app_id
                    );
                    return Err(ApiError::conflict(
                        "This user already has a pending invitation",
                    ));
                }

                invitation::ActiveModel {
                    id: Set(invitation_id),
                    app_id: Set(app_id),
                    created_at: Set(chrono::Utc::now().fixed_offset()),
                    updated_at: Set(chrono::Utc::now().fixed_offset()),
                    by_member_id: Set(member.id),
                    message: Set(message),
                    user_id: Set(invitee),
                    name: Set(meta
                        .as_ref()
                        .map_or("Unknown App".to_string(), |m| m.name.clone())),
                    description: Set(meta.as_ref().and_then(|m| m.description.clone())),
                }
                .insert(txn)
                .await?;

                Ok::<_, ApiError>(
                    meta.as_ref()
                        .map_or("an app".to_string(), |m| m.name.clone()),
                )
            })
        })
        .await?;

    if let Err(error) = dispatch_notification(
        &state,
        DispatchNotificationInput {
            user_id: params.sub.clone(),
            app_id: Some(app_id.clone()),
            title: format!("You've been invited to {}", app_name),
            description: Some("Open your invitations to accept or decline.".to_string()),
            icon: Some("mail".to_string()),
            link: Some("/notifications".to_string()),
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
