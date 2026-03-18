use crate::{
    ensure_permission,
    entity::{membership, notification, sea_orm_active_enums::NotificationType},
    error::ApiError,
    middleware::jwt::AppUser,
    permission::role_permission::RolePermissions,
    routes::app::events::db::get_event_from_db,
    state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, State},
};
use flow_like_types::create_id;
use sea_orm::{ActiveModelTrait, ActiveValue::Set, ColumnTrait, EntityTrait, QueryFilter};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct CreateNotificationParams {
    /// Event ID that triggered this execution.
    /// Used to resolve the board and verify that notifications are allowed for it.
    pub event_id: String,
    /// Target user's sub. If not provided, notifies the executing user.
    pub target_user_sub: Option<String>,
    pub title: String,
    pub description: Option<String>,
    pub icon: Option<String>,
    pub link: Option<String>,
    /// Run ID that triggered this notification (for tracking)
    pub run_id: Option<String>,
    /// Node ID that created this notification (for tracking)
    pub node_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct CreateNotificationResponse {
    pub id: String,
    pub success: bool,
}

/// POST /apps/{app_id}/notifications/create
///
/// Create a notification from a workflow execution.
/// - Caller must have ExecuteEvents permission in the project
/// - event_id is required and is used to resolve the board
/// - The resolved board must contain a Notify User node, otherwise the request is denied
/// - If target_user_sub is provided, that user must be a member of the project
/// - If target_user_sub is not provided, notifies the executing user (from JWT sub)
#[utoipa::path(
    post,
    path = "/apps/{app_id}/notifications/create",
    tag = "notifications",
    description = "Create a user notification for an event run.",
    params(
        ("app_id" = String, Path, description = "Application ID")
    ),
    request_body = CreateNotificationParams,
    responses(
        (status = 200, description = "Notification created", body = CreateNotificationResponse),
        (status = 400, description = "Bad request"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden")
    ),
    security(
        ("bearer_auth" = []),
        ("api_key" = []),
        ("pat" = [])
    )
)]
#[tracing::instrument(name = "POST /apps/{app_id}/notifications/create", skip(state, user))]
pub async fn create_notification(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(app_id): Path<String>,
    Json(params): Json<CreateNotificationParams>,
) -> Result<Json<CreateNotificationResponse>, ApiError> {
    // Verify caller has execution permission in the project
    let caller = ensure_permission!(user, &app_id, &state, RolePermissions::ExecuteEvents);
    let caller_sub = caller.sub()?;

    if params.event_id.trim().is_empty() {
        return Err(ApiError::bad_request("event_id is required".to_string()));
    }

    // Get event from database (validates event belongs to this app)
    let event = get_event_from_db(&state.db, &params.event_id, &app_id).await.map_err(|e| {
        tracing::warn!(error = %e, event_id = %params.event_id, "Failed to resolve event for notification");
        ApiError::FORBIDDEN
    })?;

    let board = state
        .master_board(
            &caller_sub,
            &app_id,
            &event.board_id,
            &state,
            event.board_version,
        )
        .await
        .map_err(|e| {
            tracing::warn!(error = %e, board_id = %event.board_id, "Failed to resolve board for notification");
            ApiError::FORBIDDEN
        })?;

    let allowed_notification_nodes = ["notify_user", "notify_project_user"];

    let board_has_notification_node = board
        .nodes
        .values()
        .any(|node| allowed_notification_nodes.contains(&node.name.as_str()));

    if !board_has_notification_node {
        tracing::warn!(
            caller = %caller_sub,
            app_id = %app_id,
            event_id = %params.event_id,
            board_id = %event.board_id,
            "Denied notification create: board has no notification node"
        );
        return Err(ApiError::FORBIDDEN);
    }

    if let Some(ref source_node_id) = params.node_id {
        let node = board.nodes.get(source_node_id);
        let allowed = node
            .map(|n| allowed_notification_nodes.contains(&n.name.as_str()))
            .unwrap_or(false);

        if !allowed {
            tracing::warn!(
                caller = %caller_sub,
                app_id = %app_id,
                event_id = %params.event_id,
                board_id = %event.board_id,
                source_node_id = %source_node_id,
                "Denied notification create: source node is not a notification node"
            );
            return Err(ApiError::FORBIDDEN);
        }
    }

    // Determine target user
    let target_sub = params.target_user_sub.unwrap_or_else(|| caller_sub.clone());

    // If targeting a different user, verify they are a member of the project
    if target_sub != caller_sub {
        let target_membership = membership::Entity::find()
            .filter(membership::Column::AppId.eq(app_id.clone()))
            .filter(membership::Column::UserId.eq(target_sub.clone()))
            .one(&state.db)
            .await?;

        if target_membership.is_none() {
            tracing::warn!(
                caller = %caller_sub,
                target = %target_sub,
                app_id = %app_id,
                "Attempted to notify user who is not a member of the project"
            );
            return Err(ApiError::FORBIDDEN);
        }
    }

    // Create the notification
    let notification_id = create_id();
    let notification = notification::ActiveModel {
        id: Set(notification_id.clone()),
        user_id: Set(target_sub),
        app_id: Set(Some(app_id)),
        title: Set(params.title),
        description: Set(params.description),
        icon: Set(params.icon),
        link: Set(params.link),
        r#type: Set(NotificationType::Workflow),
        read: Set(false),
        source_run_id: Set(params.run_id),
        source_node_id: Set(params.node_id),
        created_at: Set(chrono::Utc::now().naive_utc()),
        read_at: Set(None),
    };

    notification.insert(&state.db).await?;

    Ok(Json(CreateNotificationResponse {
        id: notification_id,
        success: true,
    }))
}
