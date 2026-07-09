//! Cross-app process graph
//!
//! Combines the static app-connection topology (who *may* call whom) with the
//! observed call chains recorded on execution runs (`callerAppChain`), so full
//! cross-app processes are visible directly — no after-the-fact process
//! mining required.
//!
//! Two views exist: platform admins get the whole graph
//! (`GET /admin/connections/graph`), app owners/admins get the subgraph their
//! app participates in (`GET /apps/{app_id}/connections/graph`) with apps the
//! requesting user cannot access masked server-side as "unknown".

use crate::{
    ensure_permission,
    entity::{app_connection, app_process_note},
    error::ApiError,
    middleware::jwt::AppUser,
    permission::role_permission::RolePermissions,
    routes::app::connection::{
        app_meta_lookup, role_name_lookup, role_permission_lookup, status_to_string,
    },
    state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, Query, State},
};
use sea_orm::{
    ColumnTrait, ConnectionTrait, DatabaseBackend, EntityTrait, QueryFilter, QueryOrder,
    QuerySelect, Statement,
};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use utoipa::ToSchema;

const MAX_GRAPH_DEPTH: usize = 6;
const MAX_GRAPH_NODES: usize = 200;
const MAX_FLOWS: u64 = 500;
const MAX_NOTES: u64 = 1000;

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct ProcessNoteInfo {
    pub id: String,
    pub author_user_id: Option<String>,
    pub content: String,
    pub created_at: i64,
    pub updated_at: i64,
}

impl From<app_process_note::Model> for ProcessNoteInfo {
    fn from(model: app_process_note::Model) -> Self {
        Self {
            id: model.id,
            author_user_id: model.author_user_id,
            content: model.content,
            created_at: model.created_at.and_utc().timestamp(),
            updated_at: model.updated_at.and_utc().timestamp(),
        }
    }
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct ProcessGraphNode {
    /// App id, or a stable pseudonym (`unknown::…`) for masked apps
    pub id: String,
    pub name: Option<String>,
    pub description: Option<String>,
    pub icon: Option<String>,
    /// True if the requesting user has no access to this app; all metadata
    /// and notes are withheld and the id is pseudonymized.
    pub unknown: bool,
    /// True for the app the graph was requested for (owner view only)
    pub is_current: bool,
    /// True if the requester may add/edit process notes on this app
    pub can_annotate: bool,
    /// Process documentation attached by the app's owners/admins
    pub notes: Vec<ProcessNoteInfo>,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct ProcessGraphEdge {
    pub source: String,
    pub target: String,
    /// "PENDING" or "ACTIVE"
    pub status: String,
    pub role_name: Option<String>,
    /// Raw permission bits granted to the source app by the connection role.
    /// Only present when the requester may see the target app's details.
    pub role_permissions: Option<i64>,
}

/// An observed call chain: the apps a process actually traversed, aggregated
/// over the selected time window.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct ProcessFlow {
    /// Apps in call order; the last element is the app whose runs were
    /// recorded. Masked apps appear as their pseudonym.
    pub path: Vec<String>,
    pub run_count: i64,
    /// Unix timestamp of the most recent run on this path
    pub last_run_at: i64,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct ProcessGraphResponse {
    pub nodes: Vec<ProcessGraphNode>,
    pub edges: Vec<ProcessGraphEdge>,
    pub flows: Vec<ProcessFlow>,
}

#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct ProcessGraphQuery {
    /// Time window in days for observed flows (default 30, max 365)
    pub days: Option<i64>,
}

pub(crate) fn flow_window(query: &ProcessGraphQuery) -> chrono::NaiveDateTime {
    let days = query.days.unwrap_or(30).clamp(1, 365);
    chrono::Utc::now().naive_utc() - chrono::Duration::days(days)
}

/// BFS over the connection topology in both directions, starting at `seed`.
pub(crate) async fn collect_subgraph(
    state: &AppState,
    seed: &str,
) -> Result<(HashSet<String>, Vec<app_connection::Model>), ApiError> {
    let mut nodes: HashSet<String> = HashSet::from([seed.to_string()]);
    let mut edges: HashMap<String, app_connection::Model> = HashMap::new();
    let mut frontier: Vec<String> = vec![seed.to_string()];

    for _ in 0..MAX_GRAPH_DEPTH {
        if frontier.is_empty() || nodes.len() >= MAX_GRAPH_NODES {
            break;
        }
        let connections = app_connection::Entity::find()
            .filter(
                app_connection::Column::SourceAppId
                    .is_in(frontier.clone())
                    .or(app_connection::Column::TargetAppId.is_in(frontier.clone())),
            )
            .limit(MAX_GRAPH_NODES as u64 * 2)
            .all(&state.db)
            .await?;

        let mut next_frontier = Vec::new();
        for connection in connections {
            for app_id in [&connection.source_app_id, &connection.target_app_id] {
                if nodes.len() < MAX_GRAPH_NODES && nodes.insert(app_id.clone()) {
                    next_frontier.push(app_id.clone());
                }
            }
            edges.insert(connection.id.clone(), connection);
        }
        frontier = next_frontier;
    }

    Ok((nodes, edges.into_values().collect()))
}

pub(crate) struct ObservedFlow {
    pub path: Vec<String>,
    pub run_count: i64,
    pub last_run_at: i64,
}

fn flow_from_row(row: &sea_orm::QueryResult) -> Result<ObservedFlow, ApiError> {
    let chain: Vec<String> = row.try_get("", "callerAppChain")?;
    let app_id: String = row.try_get("", "appId")?;
    let run_count: i64 = row.try_get("", "run_count")?;
    let last_run: chrono::NaiveDateTime = row.try_get("", "last_run")?;
    let mut path = chain;
    path.push(app_id);
    Ok(ObservedFlow {
        path,
        run_count,
        last_run_at: last_run.and_utc().timestamp(),
    })
}

/// Observed chains an app participates in — as terminal app or anywhere in
/// the recorded chain.
pub(crate) async fn observed_flows_for_app(
    state: &AppState,
    app_id: &str,
    since: chrono::NaiveDateTime,
) -> Result<Vec<ObservedFlow>, ApiError> {
    let stmt = Statement::from_sql_and_values(
        DatabaseBackend::Postgres,
        r#"SELECT "callerAppChain", "appId", COUNT(*)::BIGINT AS run_count, MAX("updatedAt") AS last_run
           FROM "public"."ExecutionRun"
           WHERE ("callerAppChain" @> $1 OR ("appId" = $2 AND array_length("callerAppChain", 1) > 0))
             AND "updatedAt" >= $3
           GROUP BY "callerAppChain", "appId"
           ORDER BY run_count DESC
           LIMIT $4"#,
        [
            vec![app_id.to_string()].into(),
            app_id.into(),
            since.into(),
            (MAX_FLOWS as i64).into(),
        ],
    );

    let rows = state.db.query_all(stmt).await?;
    rows.iter().map(flow_from_row).collect()
}

/// All observed chains platform-wide.
pub(crate) async fn observed_flows_global(
    state: &AppState,
    since: chrono::NaiveDateTime,
) -> Result<Vec<ObservedFlow>, ApiError> {
    let stmt = Statement::from_sql_and_values(
        DatabaseBackend::Postgres,
        r#"SELECT "callerAppChain", "appId", COUNT(*)::BIGINT AS run_count, MAX("updatedAt") AS last_run
           FROM "public"."ExecutionRun"
           WHERE array_length("callerAppChain", 1) > 0
             AND "updatedAt" >= $1
           GROUP BY "callerAppChain", "appId"
           ORDER BY run_count DESC
           LIMIT $2"#,
        [since.into(), (MAX_FLOWS as i64).into()],
    );

    let rows = state.db.query_all(stmt).await?;
    rows.iter().map(flow_from_row).collect()
}

pub(crate) async fn load_notes(
    state: &AppState,
    app_ids: &[String],
) -> Result<HashMap<String, Vec<ProcessNoteInfo>>, ApiError> {
    if app_ids.is_empty() {
        return Ok(HashMap::new());
    }
    let notes = app_process_note::Entity::find()
        .filter(app_process_note::Column::AppId.is_in(app_ids.iter().cloned()))
        .order_by_asc(app_process_note::Column::CreatedAt)
        .limit(MAX_NOTES)
        .all(&state.db)
        .await?;

    let mut lookup: HashMap<String, Vec<ProcessNoteInfo>> = HashMap::new();
    for note in notes {
        lookup
            .entry(note.app_id.clone())
            .or_default()
            .push(note.into());
    }
    Ok(lookup)
}

/// Stable pseudonym for an app the viewer must not identify. Keyed by the
/// viewing app so the same hidden app renders consistently within one view
/// but cannot be correlated across apps.
fn mask_app_id(app_id: &str, viewer_app_id: &str) -> String {
    let mut hasher = blake3::Hasher::new();
    hasher.update(viewer_app_id.as_bytes());
    hasher.update(b"::");
    hasher.update(app_id.as_bytes());
    let hash = hasher.finalize().to_hex().to_string();
    format!("unknown::{}", &hash[..12])
}

#[utoipa::path(
    get,
    path = "/apps/{app_id}/connections/graph",
    tag = "team",
    description = "Process graph of the app: connected apps, dependencies, and the call chains observed at runtime. Apps the requesting user has no access to are masked as unknown.",
    params(
        ("app_id" = String, Path, description = "Application ID"),
        ("days" = Option<i64>, Query, description = "Time window in days for observed flows (default 30)")
    ),
    responses(
        (status = 200, description = "Process graph", body = ProcessGraphResponse),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden")
    ),
    security(
        ("bearer_auth" = []),
        ("api_key" = []),
        ("pat" = [])
    )
)]
#[tracing::instrument(name = "GET /apps/{app_id}/connections/graph", skip(state, user))]
pub async fn get_connection_graph(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(app_id): Path<String>,
    Query(query): Query<ProcessGraphQuery>,
) -> Result<Json<ProcessGraphResponse>, ApiError> {
    let permission = ensure_permission!(user, &app_id, &state, RolePermissions::Owner);
    let viewer_sub = permission.effective_user_id().ok();

    let since = flow_window(&query);
    let (mut node_ids, connections) = collect_subgraph(&state, &app_id).await?;
    let flows = observed_flows_for_app(&state, &app_id, since).await?;
    for flow in &flows {
        for id in &flow.path {
            node_ids.insert(id.clone());
        }
    }

    // Backend-side masking: the viewer only sees apps they are a member of;
    // everything else is pseudonymized with metadata and notes withheld.
    let node_ids: Vec<String> = node_ids.into_iter().collect();
    let mut accessible: HashSet<String> = HashSet::from([app_id.clone()]);
    if let Some(sub) = &viewer_sub {
        use crate::entity::membership;
        let member_of: Vec<String> = membership::Entity::find()
            .filter(membership::Column::UserId.eq(sub))
            .filter(membership::Column::AppId.is_in(node_ids.clone()))
            .select_only()
            .column(membership::Column::AppId)
            .into_tuple()
            .all(&state.db)
            .await?;
        accessible.extend(member_of);
    }

    let visible_ids: Vec<String> = node_ids
        .iter()
        .filter(|id| accessible.contains(*id))
        .cloned()
        .collect();
    let app_meta = app_meta_lookup(&state, &visible_ids).await?;
    let notes = load_notes(&state, &visible_ids).await?;
    let role_ids: Vec<String> = connections
        .iter()
        .filter_map(|c| c.role_id.clone())
        .collect();
    let role_names = role_name_lookup(&state, &role_ids).await?;
    let role_permissions = role_permission_lookup(&state, &role_ids).await?;

    let display_id = |id: &str| -> String {
        if accessible.contains(id) {
            id.to_string()
        } else {
            mask_app_id(id, &app_id)
        }
    };

    let mut nodes: Vec<ProcessGraphNode> = Vec::with_capacity(node_ids.len());
    for id in &node_ids {
        let visible = accessible.contains(id);
        let meta = if visible { app_meta.get(id) } else { None };
        nodes.push(ProcessGraphNode {
            id: display_id(id),
            name: meta.map(|m| m.name.clone()),
            description: meta.and_then(|m| m.description.clone()),
            icon: meta.and_then(|m| m.icon.clone()),
            unknown: !visible,
            is_current: id == &app_id,
            can_annotate: id == &app_id,
            notes: if visible {
                notes.get(id).cloned().unwrap_or_default()
            } else {
                Vec::new()
            },
        });
    }

    let edges = connections
        .into_iter()
        .map(|connection| ProcessGraphEdge {
            source: display_id(&connection.source_app_id),
            target: display_id(&connection.target_app_id),
            status: status_to_string(&connection.status),
            role_name: connection
                .role_id
                .as_ref()
                .filter(|_| accessible.contains(&connection.target_app_id))
                .and_then(|id| role_names.get(id).cloned()),
            role_permissions: connection
                .role_id
                .as_ref()
                .filter(|_| accessible.contains(&connection.target_app_id))
                .and_then(|id| role_permissions.get(id).copied()),
        })
        .collect();

    let flows = flows
        .into_iter()
        .map(|flow| ProcessFlow {
            path: flow.path.iter().map(|id| display_id(id)).collect(),
            run_count: flow.run_count,
            last_run_at: flow.last_run_at,
        })
        .collect();

    Ok(Json(ProcessGraphResponse {
        nodes,
        edges,
        flows,
    }))
}
