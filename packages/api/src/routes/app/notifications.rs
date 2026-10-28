use crate::{
    entity::{membership, sea_orm_active_enums::NotificationType},
    error::ApiError,
    execution::subject::account_user_id,
    middleware::jwt::{AppUser, ExecutorUse},
    permission::role_permission::RolePermissions,
    push_notifications::{
        DispatchNotificationInput, PushDispatchStatus, dispatch_notification_with_status,
    },
    routes::app::events::db::get_event_from_db,
    state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, State},
};
use flow_like::flow::{board::Board, execution::ExecutionPrincipal, node::Node};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

const LOCAL_EXECUTION_SUB: &str = "local";
const ALLOWED_NOTIFICATION_NODES: [&str; 2] = ["notify_user", "notify_project_user"];
const HOSTED_NOTIFIES_MEMBERS_ONLY: &str =
    "Hosted frontend runs can only notify members of this app";
const NO_USER_TO_NOTIFY: &str =
    "This run has no user account to notify; name a member of this app as the target";
const NOT_A_NOTIFYING_RUN: &str =
    "Only runs of this app started by a user or an API key can send notifications";

/// Notifications come from callers acting for a user, or from runs an API key started.
/// App connections, the runs they start and bare API keys send none.
fn may_send_notifications(
    user: &AppUser,
    principal: ExecutionPrincipal,
    sub: Option<&str>,
) -> bool {
    sub.is_some()
        || (principal == ExecutionPrincipal::ApiKey && matches!(user, AppUser::Executor(_)))
}

/// The user a notification goes to: the requested target, else the run's own user.
fn notification_target(requested: Option<&str>, caller: Option<&str>) -> Result<String, ApiError> {
    requested
        .map(str::trim)
        .filter(|target| !target.is_empty() && *target != LOCAL_EXECUTION_SUB)
        .or(caller)
        .map(ToOwned::to_owned)
        .ok_or_else(|| ApiError::forbidden(NO_USER_TO_NOTIFY))
}

/// Whether the target has to be a member of the app. A hosted-frontend run serves
/// visitors who may be anyone, so everyone it notifies, its visitor included, must be a
/// member; any other run may notify its own user.
fn target_must_be_member(hosted_frontend: bool, target: &str, caller: Option<&str>) -> bool {
    hosted_frontend || caller != Some(target)
}

async fn is_member(state: &AppState, app_id: &str, target: &str) -> Result<bool, ApiError> {
    let Some(user_id) = account_user_id(target) else {
        return Ok(false);
    };
    Ok(membership::Entity::find()
        .filter(membership::Column::AppId.eq(app_id))
        .filter(membership::Column::UserId.eq(user_id))
        .one(&state.db)
        .await?
        .is_some())
}

fn board_nodes(board: &Board) -> impl Iterator<Item = &Node> {
    board
        .nodes
        .values()
        .chain(board.layers.values().flat_map(|layer| layer.nodes.values()))
}

fn find_board_node<'a>(board: &'a Board, node_id: &str) -> Option<&'a Node> {
    board.nodes.get(node_id).or_else(|| {
        board
            .layers
            .values()
            .find_map(|layer| layer.nodes.get(node_id))
    })
}

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
    pub persisted: bool,
    pub success: bool,
    pub push_status: PushDispatchStatus,
}

/// POST /apps/{app_id}/notifications/create
///
/// Create a notification from a workflow execution.
/// - Caller must have ExecuteEvents permission in the project and act for a user, or be
///   a run started with an API key; app connections and bare API keys are refused
/// - Either `event_id` or `board_id` is required to resolve the board
/// - The resolved board must contain a notification node, otherwise the request is denied
/// - If target_user_sub is provided, that user must be a member of the project
/// - If target_user_sub is not provided, notifies the executing user (from JWT sub);
///   a run without a user account, such as one started with an API key, must name one
/// - A hosted frontend run may only notify members, its visitor included
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
        (status = 403, description = "Forbidden: execution permission required, the caller is an app connection or a bare API key, the run is a regression replay, the target is not a project member (hosted frontend runs always need one), or the run has no user and names no target")
    ),
    security(
        ("bearer_auth" = []),
        ("api_key" = []),
        ("pat" = [])
    )
)]
#[tracing::instrument(
    name = "POST /apps/{app_id}/notifications/create",
    skip(state, user, params)
)]
pub async fn create_notification(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(app_id): Path<String>,
    Json(params): Json<CreateNotificationParams>,
) -> Result<Json<CreateNotificationResponse>, ApiError> {
    // Verify caller has execution permission in the project
    let caller = user
        .execution_app_permission(&app_id, &state, ExecutorUse::Write("send notifications"))
        .await?;
    let caller_id = caller.identifier();

    if !caller.has_permission(RolePermissions::ExecuteEvents) {
        state.invalidate_permission(&caller_id, &app_id);
        return Err(ApiError::FORBIDDEN);
    }
    if !may_send_notifications(&user, caller.principal, caller.sub.as_deref()) {
        return Err(ApiError::forbidden(NOT_A_NOTIFYING_RUN));
    }

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
                &caller_id,
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
            .master_board(&caller_id, &app_id, board_id, &state, None)
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

    let board_has_notification_node =
        board_nodes(&board).any(|node| ALLOWED_NOTIFICATION_NODES.contains(&node.name.as_str()));

    if !board_has_notification_node {
        tracing::warn!(
            caller = %caller_id,
            app_id = %app_id,
            event_id = ?resolved_event_id,
            board_id = %resolved_board_id,
            "Denied notification create: board has no notification node"
        );
        return Err(ApiError::FORBIDDEN);
    }

    if let Some(ref source_node_id) = params.node_id {
        let node = find_board_node(&board, source_node_id);
        let allowed = node
            .map(|n| ALLOWED_NOTIFICATION_NODES.contains(&n.name.as_str()))
            .unwrap_or(false);

        if !allowed {
            tracing::warn!(
                caller = %caller_id,
                app_id = %app_id,
                event_id = ?resolved_event_id,
                board_id = %resolved_board_id,
                source_node_id = %source_node_id,
                "Denied notification create: source node is not a notification node"
            );
            return Err(ApiError::FORBIDDEN);
        }
    }

    let target_sub = notification_target(params.target_user_sub.as_deref(), caller.sub.as_deref())?;
    let hosted_frontend = matches!(&user, AppUser::Executor(executor) if executor.hosted_frontend);
    if target_must_be_member(hosted_frontend, &target_sub, caller.sub.as_deref())
        && !is_member(&state, &app_id, &target_sub).await?
    {
        tracing::warn!(
            caller = %caller_id,
            target = %target_sub,
            app_id = %app_id,
            hosted_frontend,
            "Attempted to notify user who is not a member of the project"
        );
        return Err(if hosted_frontend {
            ApiError::forbidden(HOSTED_NOTIFIES_MEMBERS_ONLY)
        } else {
            ApiError::FORBIDDEN
        });
    }

    let result = dispatch_notification_with_status(
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
    fn hosted_runs_notify_members_only() {
        let visitor = Some("visitor-1");
        assert!(target_must_be_member(true, "visitor-1", visitor));
        assert!(target_must_be_member(true, "member-1", visitor));
        assert!(target_must_be_member(
            true,
            "frontend:event-1:session-1",
            Some("frontend:event-1:session-1")
        ));
        assert!(!target_must_be_member(false, "user-1", Some("user-1")));
        assert!(target_must_be_member(false, "user-2", Some("user-1")));
        assert!(target_must_be_member(false, "user-1", None));
    }

    #[test]
    fn only_user_and_api_key_runs_send_notifications() {
        use crate::middleware::jwt::{ApiKey, ConnectedAppUser, ExecutorUser, OpenIDUser};
        let run = |technical_user_id: Option<&str>, app_chain: Option<Vec<String>>| {
            AppUser::Executor(ExecutorUser {
                sub: "user-1".into(),
                app_id: "app-1".into(),
                run_id: "run-1".into(),
                technical_user_id: technical_user_id.map(str::to_owned),
                app_chain,
                correlation: None,
                shadow: false,
                hosted_frontend: false,
                event_id: None,
                custom_bits_owner: None,
                unattended: false,
            })
        };
        let human = AppUser::OpenID(OpenIDUser {
            sub: "user-1".into(),
            access_token: "token".into(),
        });
        let key = AppUser::APIKey(ApiKey {
            key_id: "key-1".into(),
            api_key: "secret".into(),
            app_id: "app-1".into(),
            creator_user_id: Some("user-1".into()),
        });
        let connection = AppUser::ConnectedApp(ConnectedAppUser {
            sub: Some("user-1".into()),
            origin_app_id: "origin-app".into(),
            target_app_id: "app-1".into(),
            app_chain: vec!["origin-app".into()],
            technical_user_id: None,
            run_id: None,
            correlation: None,
        });
        let (user, api_key, connected) = (
            ExecutionPrincipal::User,
            ExecutionPrincipal::ApiKey,
            ExecutionPrincipal::ConnectedApp,
        );
        assert!(may_send_notifications(&human, user, Some("user-1")));
        assert!(may_send_notifications(
            &run(None, None),
            user,
            Some("user-1")
        ));
        assert!(may_send_notifications(
            &run(Some("key-1"), None),
            api_key,
            None
        ));
        assert!(!may_send_notifications(&key, api_key, None));
        assert!(!may_send_notifications(&connection, connected, None));
        let started_by_connection = run(None, Some(vec!["origin-app".into()]));
        assert!(!may_send_notifications(
            &started_by_connection,
            connected,
            None
        ));
    }

    #[test]
    fn a_run_notifies_its_own_user_unless_it_names_a_target() {
        let caller = Some("user-1");
        assert_eq!(notification_target(None, caller).unwrap(), "user-1");
        assert_eq!(
            notification_target(Some(" user-2 "), caller).unwrap(),
            "user-2"
        );
        for blank in ["", "   ", LOCAL_EXECUTION_SUB] {
            assert_eq!(notification_target(Some(blank), caller).unwrap(), "user-1");
        }
        assert_eq!(notification_target(Some("user-2"), None).unwrap(), "user-2");
        let refusal = notification_target(None, None).unwrap_err();
        assert_eq!(refusal.status(), axum::http::StatusCode::FORBIDDEN);
        assert_eq!(refusal.public_message(), Some(NO_USER_TO_NOTIFY));
    }

    #[test]
    fn pseudo_subjects_are_never_members() {
        let source = include_str!("notifications.rs");
        let body = source
            .split_once("\nasync fn is_member(")
            .expect("membership is checked here")
            .1;
        let body = body.split_once("\n}\n").map_or(body, |(body, _)| body);
        let pseudo = body
            .find("account_user_id(target)")
            .expect("pseudo subjects skip the lookup");
        assert!(pseudo < body.find("membership::Entity::find()").unwrap());
    }
}
