use crate::{
    ensure_permission,
    entity::{
        app_connection, event, event_remote_registration, role,
        sea_orm_active_enums::AppConnectionStatus,
    },
    error::ApiError,
    middleware::jwt::AppUser,
    permission::role_permission::{RolePermissions, has_role_permission},
    state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, State},
};
use sea_orm::{ColumnTrait, Condition, EntityTrait, QueryFilter, QueryOrder, QuerySelect};
use serde::Serialize;
use utoipa::ToSchema;

/// Event types another app can invoke through a connection. Cron/webhook/
/// sink events fire from their own triggers and are not remotely callable.
pub const REMOTE_CALLABLE_EVENT_TYPES: &[&str] = &["simple_chat", "rest", "mcp"];

/// A connected app can call chat events (role-gated, no public surface) and
/// REST/MCP events explicitly marked `INTERNAL`. Public REST/MCP events live
/// only on the public inbound routers and are not reachable via the proxy.
fn connection_callable_condition() -> Condition {
    Condition::any()
        .add(event::Column::EventType.eq("simple_chat"))
        .add(event::Column::Exposure.eq("INTERNAL"))
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct RemoteEvent {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub event_type: String,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct RemoteRestRoute {
    pub method: String,
    pub path: String,
    /// Template parameters extracted from the path (`{name}` segments)
    pub params: Vec<String>,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct RemoteRestFile {
    pub path: String,
    pub directory: bool,
    pub content_type: Option<String>,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct RemoteMcpTool {
    pub name: String,
    pub description: Option<String>,
    /// JSON schema of the tool arguments
    #[schema(value_type = Object)]
    pub input_schema: Option<flow_like_types::Value>,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct RemoteMcpResource {
    pub uri: String,
    pub name: Option<String>,
    pub description: Option<String>,
    pub mime_type: Option<String>,
}

/// Everything the flow editor needs to build typed pins for a remote event:
/// REST routes/files or MCP tools/resources, derived from the event's
/// materialized registrations. Chat events carry no registrations — their
/// payload shape is static.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct RemoteEventDetail {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub event_type: String,
    pub rest_routes: Vec<RemoteRestRoute>,
    pub rest_files: Vec<RemoteRestFile>,
    pub mcp_tools: Vec<RemoteMcpTool>,
    pub mcp_resources: Vec<RemoteMcpResource>,
}

fn template_params(path: &str) -> Vec<String> {
    path.split('/')
        .filter_map(|segment| {
            segment
                .strip_prefix('{')
                .and_then(|s| s.strip_suffix('}'))
                .map(|s| s.to_string())
        })
        .collect()
}

async fn ensure_connection_role(
    state: &AppState,
    app_id: &str,
    target_app_id: &str,
    required: RolePermissions,
) -> Result<(), ApiError> {
    let connection = app_connection::Entity::find()
        .filter(
            app_connection::Column::SourceAppId
                .eq(app_id)
                .and(app_connection::Column::TargetAppId.eq(target_app_id))
                .and(app_connection::Column::Status.eq(AppConnectionStatus::Active)),
        )
        .one(&state.db)
        .await?
        .ok_or_else(|| ApiError::not_found("No active connection to the target app"))?;

    let role_id = connection.role_id.ok_or(ApiError::FORBIDDEN)?;
    let role_model = role::Entity::find_by_id(&role_id)
        .filter(role::Column::AppId.eq(target_app_id))
        .one(&state.db)
        .await?
        .ok_or(ApiError::FORBIDDEN)?;

    let permissions = RolePermissions::from_bits(role_model.permissions)
        .ok_or_else(|| ApiError::internal("Invalid role permission bits"))?;
    if !has_role_permission(&permissions, required) {
        return Err(ApiError::forbidden(
            "The connection role does not allow this operation",
        ));
    }
    Ok(())
}

#[utoipa::path(
    get,
    path = "/apps/{app_id}/connections/{target_app_id}/events/{event_id}/detail",
    tag = "team",
    description = "Typed details of a connected app's event: REST routes and files, or MCP tools and resources. Used by the flow editor to build matching pins.",
    params(
        ("app_id" = String, Path, description = "Application ID (the connected/origin app)"),
        ("target_app_id" = String, Path, description = "The app the event belongs to"),
        ("event_id" = String, Path, description = "Event ID")
    ),
    responses(
        (status = 200, description = "Remote event details", body = RemoteEventDetail),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
        (status = 404, description = "Event or connection not found")
    ),
    security(
        ("bearer_auth" = []),
        ("api_key" = []),
        ("pat" = [])
    )
)]
#[tracing::instrument(
    name = "GET /apps/{app_id}/connections/{target_app_id}/events/{event_id}/detail",
    skip(state, user)
)]
pub async fn get_remote_event_detail(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((app_id, target_app_id, event_id)): Path<(String, String, String)>,
) -> Result<Json<RemoteEventDetail>, ApiError> {
    ensure_permission!(user, &app_id, &state, RolePermissions::ReadBoards);
    ensure_connection_role(
        &state,
        &app_id,
        &target_app_id,
        RolePermissions::ExecuteEvents,
    )
    .await?;

    let event_row = event::Entity::find_by_id(&event_id)
        .filter(event::Column::AppId.eq(&target_app_id))
        .filter(event::Column::Active.eq(true))
        .filter(event::Column::EventType.is_in(REMOTE_CALLABLE_EVENT_TYPES.iter().copied()))
        .filter(connection_callable_condition())
        .one(&state.db)
        .await?
        .ok_or_else(|| ApiError::not_found("Event not found"))?;

    let mut detail = RemoteEventDetail {
        id: event_row.id.clone(),
        name: event_row.name.clone(),
        description: event_row.description.clone(),
        event_type: event_row.event_type.clone(),
        rest_routes: Vec::new(),
        rest_files: Vec::new(),
        mcp_tools: Vec::new(),
        mcp_resources: Vec::new(),
    };

    let Some(version) = event_row.last_setup_version.clone() else {
        return Ok(Json(detail));
    };

    let registrations = event_remote_registration::Entity::find()
        .filter(event_remote_registration::Column::AppId.eq(&target_app_id))
        .filter(event_remote_registration::Column::EventId.eq(&event_id))
        .filter(event_remote_registration::Column::EventVersion.eq(&version))
        .limit(500)
        .all(&state.db)
        .await?;

    for registration in registrations {
        match registration.kind.as_str() {
            "rest_fn" => {
                detail.rest_routes.push(RemoteRestRoute {
                    method: registration.method.clone().unwrap_or_else(|| "ANY".into()),
                    params: template_params(&registration.path),
                    path: registration.path,
                });
            }
            "rest_file" => {
                let extras = registration.extras_json.as_ref();
                detail.rest_files.push(RemoteRestFile {
                    path: registration.path.clone(),
                    directory: extras
                        .and_then(|v| v.get("directory"))
                        .and_then(|v| v.as_bool())
                        .unwrap_or(false),
                    content_type: extras
                        .and_then(|v| v.get("content_type"))
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string()),
                });
            }
            "mcp_tool" => {
                let extras = registration.extras_json.as_ref();
                detail.mcp_tools.push(RemoteMcpTool {
                    name: extras
                        .and_then(|v| v.get("name"))
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string())
                        .unwrap_or_else(|| registration.path.clone()),
                    description: extras
                        .and_then(|v| v.get("description"))
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string()),
                    input_schema: registration.schema_json.clone(),
                });
            }
            "mcp_resource" => {
                let extras = registration.extras_json.as_ref();
                detail.mcp_resources.push(RemoteMcpResource {
                    uri: extras
                        .and_then(|v| v.get("uri"))
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string())
                        .unwrap_or_else(|| registration.path.clone()),
                    name: extras
                        .and_then(|v| v.get("name"))
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string()),
                    description: extras
                        .and_then(|v| v.get("description"))
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string()),
                    mime_type: extras
                        .and_then(|v| v.get("mimeType").or_else(|| v.get("mime_type")))
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string()),
                });
            }
            _ => {}
        }
    }

    Ok(Json(detail))
}

#[utoipa::path(
    get,
    path = "/apps/{app_id}/connections/{target_app_id}/events",
    tag = "team",
    description = "List the active events of a connected app that this app may invoke. Requires an active connection whose role allows executing events.",
    params(
        ("app_id" = String, Path, description = "Application ID (the connected/origin app)"),
        ("target_app_id" = String, Path, description = "The app whose events should be listed")
    ),
    responses(
        (status = 200, description = "Invocable events of the connected app", body = Vec<RemoteEvent>),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
        (status = 404, description = "No active connection to the target app")
    ),
    security(
        ("bearer_auth" = []),
        ("api_key" = []),
        ("pat" = [])
    )
)]
#[tracing::instrument(
    name = "GET /apps/{app_id}/connections/{target_app_id}/events",
    skip(state, user)
)]
pub async fn get_remote_events(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((app_id, target_app_id)): Path<(String, String)>,
) -> Result<Json<Vec<RemoteEvent>>, ApiError> {
    ensure_permission!(user, &app_id, &state, RolePermissions::ReadBoards);
    ensure_connection_role(
        &state,
        &app_id,
        &target_app_id,
        RolePermissions::ExecuteEvents,
    )
    .await?;

    let events = event::Entity::find()
        .filter(event::Column::AppId.eq(&target_app_id))
        .filter(event::Column::Active.eq(true))
        .filter(event::Column::EventType.is_in(REMOTE_CALLABLE_EVENT_TYPES.iter().copied()))
        .filter(connection_callable_condition())
        .order_by_asc(event::Column::Name)
        .limit(200)
        .all(&state.db)
        .await?;

    Ok(Json(
        events
            .into_iter()
            .map(|event| RemoteEvent {
                id: event.id,
                name: event.name,
                description: event.description,
                event_type: event.event_type,
            })
            .collect(),
    ))
}
