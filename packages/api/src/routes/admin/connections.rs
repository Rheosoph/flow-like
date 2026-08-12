use crate::{
    entity::app_connection,
    error::ApiError,
    middleware::jwt::AppUser,
    permission::global_permission::GlobalPermission,
    routes::app::connection::{
        app_meta_lookup,
        graph::{
            ProcessFlow, ProcessGraphEdge, ProcessGraphNode, ProcessGraphQuery,
            ProcessGraphResponse, app_category_lookup, content_stats, flow_window, load_notes,
            observed_flows_global, presign_media,
        },
        role_name_lookup, role_permission_lookup, status_to_string,
    },
    state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Query, State},
};
use sea_orm::{EntityTrait, QuerySelect};
use std::collections::HashSet;

const MAX_ADMIN_CONNECTIONS: u64 = 5000;

#[utoipa::path(
    get,
    path = "/admin/connections/graph",
    tag = "admin",
    description = "Platform-wide process graph: every app connection and every call chain observed at runtime, with full app metadata and process notes.",
    params(
        ("days" = Option<i64>, Query, description = "Time window in days for observed flows (default 30)")
    ),
    responses(
        (status = 200, description = "Global process graph", body = ProcessGraphResponse),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden")
    ),
    security(
        ("bearer_auth" = []),
        ("api_key" = []),
        ("pat" = [])
    )
)]
#[tracing::instrument(name = "GET /admin/connections/graph", skip_all)]
pub async fn get_global_connection_graph(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Query(query): Query<ProcessGraphQuery>,
) -> Result<Json<ProcessGraphResponse>, ApiError> {
    user.check_global_permission(&state, GlobalPermission::Admin)
        .await?;

    let since = flow_window(&query);

    let connections = app_connection::Entity::find()
        .limit(MAX_ADMIN_CONNECTIONS)
        .all(&state.db)
        .await?;
    let flows = observed_flows_global(&state, since).await?;

    let mut node_ids: HashSet<String> = HashSet::new();
    for connection in &connections {
        node_ids.insert(connection.source_app_id.clone());
        node_ids.insert(connection.target_app_id.clone());
    }
    for flow in &flows {
        for id in &flow.path {
            node_ids.insert(id.clone());
        }
    }
    let node_ids: Vec<String> = node_ids.into_iter().collect();

    let app_meta = app_meta_lookup(&state, &node_ids).await?;
    let notes = load_notes(&state, &node_ids).await?;
    let role_ids: Vec<String> = connections
        .iter()
        .filter_map(|c| c.role_id.clone())
        .collect();
    let role_names = role_name_lookup(&state, &role_ids).await?;
    let role_permissions = role_permission_lookup(&state, &role_ids).await?;
    let content = content_stats(&state, &node_ids).await?;
    let media = presign_media(&state, &app_meta).await;
    let categories = app_category_lookup(&state, &node_ids).await;

    let nodes = node_ids
        .iter()
        .map(|id| {
            let meta = app_meta.get(id);
            let media = media.get(id);
            ProcessGraphNode {
                id: id.clone(),
                name: meta.map(|m| m.name.clone()),
                description: meta.and_then(|m| m.description.clone()),
                icon: media.and_then(|(icon, _)| icon.clone()),
                banner: media.and_then(|(_, banner)| banner.clone()),
                unknown: false,
                is_current: false,
                can_annotate: true,
                notes: notes.get(id).cloned().unwrap_or_default(),
                tags: meta.map(|m| m.tags.clone()).unwrap_or_default(),
                category: categories.get(id).cloned(),
                website: meta.and_then(|m| m.website.clone()),
                docs_url: meta.and_then(|m| m.docs_url.clone()),
                content: content.get(id).cloned(),
            }
        })
        .collect();

    let edges = connections
        .into_iter()
        .map(|connection| ProcessGraphEdge {
            source: connection.source_app_id.clone(),
            target: connection.target_app_id.clone(),
            status: status_to_string(&connection.status),
            role_name: connection
                .role_id
                .as_ref()
                .and_then(|id| role_names.get(id).cloned()),
            role_permissions: connection
                .role_id
                .as_ref()
                .and_then(|id| role_permissions.get(id).copied()),
        })
        .collect();

    let flows = flows
        .into_iter()
        .map(|flow| ProcessFlow {
            path: flow.path,
            run_count: flow.run_count,
            failed_count: flow.failed_count,
            avg_duration_ms: flow.avg_duration_ms,
            last_run_at: flow.last_run_at,
            event_name: flow.event_name,
            event_type: flow.event_type,
        })
        .collect();

    Ok(Json(ProcessGraphResponse {
        nodes,
        edges,
        flows,
    }))
}
