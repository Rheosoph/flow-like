use crate::{
    entity::{invitation, notification, sea_orm_active_enums::NotificationType},
    error::ApiError,
    middleware::jwt::AppUser,
    push_notifications::{
        DispatchNotificationInput, PushDispatchStatus, dispatch_notification_with_status,
    },
    state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, Query, State},
};
use sea_orm::{
    ActiveModelTrait, ActiveValue::Set, ColumnTrait, Condition, EntityTrait, PaginatorTrait,
    QueryFilter, QueryOrder, QuerySelect, sea_query::Expr,
};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

#[derive(Debug, Clone, Default, Deserialize, Serialize, ToSchema)]
pub struct NotificationOverview {
    pub invites_count: u64,
    pub notifications_count: u64,
    pub unread_count: u64,
}

#[derive(Debug, Clone, Deserialize, IntoParams)]
pub struct ListNotificationsParams {
    pub limit: Option<u64>,
    pub offset: Option<u64>,
    pub unread_only: Option<bool>,
    /// `WORKFLOW` or `SYSTEM`. Filtering here rather than over the returned page
    /// keeps a caller asking for one kind from missing every row the page cut
    /// off. An unrecognised value is ignored rather than rejected, so a client
    /// sending a kind this version does not know keeps its unfiltered list.
    pub notification_type: Option<String>,
}

fn notification_type_filter(value: Option<&str>) -> Option<NotificationType> {
    match value?.trim().to_ascii_uppercase().as_str() {
        "WORKFLOW" => Some(NotificationType::Workflow),
        "SYSTEM" => Some(NotificationType::System),
        _ => None,
    }
}

#[utoipa::path(
    get,
    path = "/user/notifications",
    tag = "user",
    responses(
        (status = 200, description = "Notification overview with counts", body = NotificationOverview),
        (status = 401, description = "Unauthorized")
    ),
    security(
        ("bearer_auth" = [])
    )
)]
#[tracing::instrument(name = "GET /user/notifications", skip(state, user))]
pub async fn get_notifications(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
) -> Result<Json<NotificationOverview>, ApiError> {
    let sub = user.sub()?;

    let invites_count = invitation::Entity::find()
        .filter(invitation::Column::UserId.eq(sub.clone()))
        .count(&state.db)
        .await?;

    let notifications_count = notification::Entity::find()
        .filter(notification::Column::UserId.eq(sub.clone()))
        .count(&state.db)
        .await?;

    let unread_count = notification::Entity::find()
        .filter(notification::Column::UserId.eq(sub))
        .filter(notification::Column::Read.eq(false))
        .count(&state.db)
        .await?;

    Ok(Json(NotificationOverview {
        invites_count,
        notifications_count,
        unread_count,
    }))
}

#[utoipa::path(
    get,
    path = "/user/notifications/list",
    tag = "user",
    params(ListNotificationsParams),
    responses(
        (status = 200, description = "List of notifications"),
        (status = 401, description = "Unauthorized")
    ),
    security(
        ("bearer_auth" = [])
    )
)]
#[tracing::instrument(name = "GET /user/notifications/list", skip_all)]
pub async fn list_notifications(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Query(params): Query<ListNotificationsParams>,
) -> Result<Json<Vec<notification::Model>>, ApiError> {
    let sub = user.sub()?;

    let limit = params.limit.unwrap_or(20).min(100);
    let offset = params.offset.unwrap_or(0);

    let mut query = notification::Entity::find()
        .filter(notification::Column::UserId.eq(sub))
        .order_by_desc(notification::Column::CreatedAt);

    if params.unread_only.unwrap_or(false) {
        query = query.filter(notification::Column::Read.eq(false));
    }

    if let Some(kind) = notification_type_filter(params.notification_type.as_deref()) {
        query = query.filter(notification::Column::Type.eq(kind));
    }

    let mut notifications = query.limit(limit).offset(offset).all(&state.db).await?;
    crate::notification_images::refresh_notification_icons(&state, &mut notifications).await?;

    Ok(Json(notifications))
}

#[utoipa::path(
    post,
    path = "/user/notifications/{id}/read",
    tag = "user",
    params(
        ("id" = String, Path, description = "Notification ID")
    ),
    responses(
        (status = 200, description = "Notification marked as read"),
        (status = 401, description = "Unauthorized"),
        (status = 404, description = "Notification not found")
    ),
    security(
        ("bearer_auth" = [])
    )
)]
#[tracing::instrument(name = "POST /user/notifications/{id}/read", skip(state, user))]
pub async fn mark_notification_read(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(notification_id): Path<String>,
) -> Result<Json<()>, ApiError> {
    let sub = user.sub()?;

    let notification = notification::Entity::find_by_id(notification_id.clone())
        .filter(notification::Column::UserId.eq(sub))
        .one(&state.db)
        .await?
        .ok_or(ApiError::NOT_FOUND)?;

    let mut active: notification::ActiveModel = notification.into();
    active.read = Set(true);
    active.read_at = Set(Some(chrono::Utc::now().fixed_offset()));
    active.update(&state.db).await?;

    Ok(Json(()))
}

#[utoipa::path(
    delete,
    path = "/user/notifications/{id}",
    tag = "user",
    params(
        ("id" = String, Path, description = "Notification ID")
    ),
    responses(
        (status = 200, description = "Notification deleted"),
        (status = 401, description = "Unauthorized"),
        (status = 404, description = "Notification not found")
    ),
    security(
        ("bearer_auth" = [])
    )
)]
#[tracing::instrument(name = "DELETE /user/notifications/{id}", skip(state, user))]
pub async fn delete_notification(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(notification_id): Path<String>,
) -> Result<Json<()>, ApiError> {
    let sub = user.sub()?;

    let notification = notification::Entity::find_by_id(notification_id.clone())
        .filter(notification::Column::UserId.eq(sub))
        .one(&state.db)
        .await?
        .ok_or(ApiError::NOT_FOUND)?;

    let active: notification::ActiveModel = notification.into();
    active.delete(&state.db).await?;

    Ok(Json(()))
}

#[utoipa::path(
    post,
    path = "/user/notifications/read-all",
    tag = "user",
    responses(
        (status = 200, description = "Number of notifications marked as read", body = u64),
        (status = 401, description = "Unauthorized")
    ),
    security(
        ("bearer_auth" = [])
    )
)]
#[tracing::instrument(name = "POST /user/notifications/read-all", skip(state, user))]
pub async fn mark_all_read(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
) -> Result<Json<u64>, ApiError> {
    let sub = user.sub()?;

    let updated = crate::db::update_in_batches::<notification::Entity>(
        &state.db,
        state.db_dialect,
        Condition::all()
            .add(notification::Column::UserId.eq(sub))
            .add(notification::Column::Read.eq(false)),
        vec![
            (notification::Column::Read, Expr::value(true)),
            (
                notification::Column::ReadAt,
                Expr::value(chrono::Utc::now().fixed_offset()),
            ),
        ],
        crate::db::DEFAULT_WRITE_CHUNK,
    )
    .await?;

    Ok(Json(updated))
}

#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct CreateUserNotificationParams {
    pub title: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub icon: Option<String>,
    #[serde(default)]
    pub link: Option<String>,
    #[serde(default)]
    pub app_id: Option<String>,
    #[serde(default)]
    pub run_id: Option<String>,
    #[serde(default)]
    pub node_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct CreateUserNotificationResponse {
    pub id: String,
    pub persisted: bool,
    pub success: bool,
    pub push_status: PushDispatchStatus,
}

/// POST /user/notifications/create
///
/// Create a notification for the authenticated user.
/// Used by offline/local workflows that don't have app-scoped board context.
/// If `app_id` is provided and exists, it will be associated with the notification.
#[utoipa::path(
    post,
    path = "/user/notifications/create",
    tag = "user",
    description = "Create a notification for the current user (for offline/local workflows).",
    request_body = CreateUserNotificationParams,
    responses(
        (status = 200, description = "Notification created", body = CreateUserNotificationResponse),
        (status = 401, description = "Unauthorized")
    ),
    security(
        ("bearer_auth" = []),
        ("pat" = [])
    )
)]
#[tracing::instrument(name = "POST /user/notifications/create", skip_all)]
pub async fn create_user_notification(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Json(params): Json<CreateUserNotificationParams>,
) -> Result<Json<CreateUserNotificationResponse>, ApiError> {
    let sub = user.sub()?;

    // If app_id provided, verify it exists — but don't require it
    let app_id = if let Some(ref id) = params.app_id {
        let exists = crate::entity::app::Entity::find_by_id(id)
            .one(&state.db)
            .await?
            .is_some();
        if exists { Some(id.clone()) } else { None }
    } else {
        None
    };

    let result = dispatch_notification_with_status(
        &state,
        DispatchNotificationInput {
            user_id: sub,
            app_id,
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
    .await
    .map_err(|e| {
        ApiError::internal_error(flow_like_types::anyhow!(
            "Failed to create notification: {}",
            e
        ))
    })?;

    Ok(Json(CreateUserNotificationResponse {
        id: result.id,
        persisted: true,
        success: result.push_status.is_success(),
        push_status: result.push_status,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_kinds_parse_case_insensitively() {
        assert_eq!(
            notification_type_filter(Some("WORKFLOW")),
            Some(NotificationType::Workflow)
        );
        assert_eq!(
            notification_type_filter(Some(" workflow ")),
            Some(NotificationType::Workflow)
        );
        assert_eq!(
            notification_type_filter(Some("system")),
            Some(NotificationType::System)
        );
    }

    #[test]
    fn absent_and_unknown_kinds_leave_the_list_unfiltered() {
        assert_eq!(notification_type_filter(None), None);
        assert_eq!(notification_type_filter(Some("all")), None);
        assert_eq!(notification_type_filter(Some("")), None);
        assert_eq!(notification_type_filter(Some("DIGEST")), None);
    }
}
