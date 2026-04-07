use crate::{
    ensure_permission,
    entity::{membership, sea_orm_active_enums::NotificationType},
    error::ApiError,
    middleware::jwt::AppUser,
    permission::role_permission::RolePermissions,
    push_notifications::{DispatchNotificationInput, dispatch_notification},
    routes::app::events::db::get_event_from_db,
    state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, State},
};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct CreateNotificationParams {
    /// Event ID that triggered this execution.
    /// Used to resolve the board and verify that notifications are allowed for it.
    pub event_id: Option<String>,
    /// Board ID for manual board executions that are not tied to an event.
    /// Used when `event_id` is not available.
    pub board_id: Option<String>,
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
/// - Either `event_id` or `board_id` is required to resolve the board
/// - The resolved board must contain a notification node, otherwise the request is denied
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

    let event_id = params
        .event_id
        .as_deref()
        .map(str::trim)
        .filter(|event_id| !event_id.is_empty());
    let board_id = params
        .board_id
        .as_deref()
        .map(str::trim)
        .filter(|board_id| !board_id.is_empty());

    let (board, resolved_board_id, resolved_event_id) = if let Some(event_id) = event_id {
        let event = get_event_from_db(&state.db, event_id, &app_id)
            .await
            .map_err(|e| {
                tracing::warn!(error = %e, event_id = %event_id, "Failed to resolve event for notification");
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

        (board, event.board_id, Some(event_id.to_string()))
    } else if let Some(board_id) = board_id {
        let board = state
            .master_board(&caller_sub, &app_id, board_id, &state, None)
            .await
            .map_err(|e| {
                tracing::warn!(error = %e, board_id = %board_id, "Failed to resolve board for notification");
                ApiError::FORBIDDEN
            })?;

        (board, board_id.to_string(), None)
    } else {
        return Err(ApiError::bad_request(
            "either event_id or board_id is required".to_string(),
        ));
    };

    let allowed_notification_nodes = ["notify_user", "notify_project_user"];

    let board_has_notification_node = board
        .nodes
        .values()
        .any(|node| allowed_notification_nodes.contains(&node.name.as_str()));

    if !board_has_notification_node {
        tracing::warn!(
            caller = %caller_sub,
            app_id = %app_id,
            event_id = ?resolved_event_id,
            board_id = %resolved_board_id,
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
                event_id = ?resolved_event_id,
                board_id = %resolved_board_id,
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

    let notification_id = dispatch_notification(
        &state,
        DispatchNotificationInput {
            user_id: target_sub,
            app_id: Some(app_id),
            title: params.title,
            description: params.description,
            icon: params.icon,
            link: params.link,
            image: None,
            notification_type: NotificationType::Workflow,
            source_run_id: params.run_id,
            source_node_id: params.node_id,
        },
    )
    .await?;

    Ok(Json(CreateNotificationResponse {
        id: notification_id,
        success: true,
    }))
}
