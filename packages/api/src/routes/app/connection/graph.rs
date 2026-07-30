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
    entity::{app, app_connection, app_process_note, event, page, template, widget},
    error::ApiError,
    middleware::jwt::AppUser,
    permission::role_permission::RolePermissions,
    routes::app::connection::{
        AppMetaPreview, app_meta_lookup, role_name_lookup, role_permission_lookup, status_to_string,
    },
    state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, Query, State},
};
use flow_like::bit::Metadata;
use flow_like_storage::Path as FlowPath;
use sea_orm::{
    ColumnTrait, ConnectionTrait, DatabaseBackend, EntityTrait, QueryFilter, QueryOrder,
    QuerySelect, Statement, sea_query::Expr,
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
    /// Presigned icon URL (withheld for masked apps)
    pub icon: Option<String>,
    /// Presigned banner/thumbnail URL (withheld for masked apps)
    pub banner: Option<String>,
    /// True if the requesting user has no access to this app; all metadata
    /// and notes are withheld and the id is pseudonymized.
    pub unknown: bool,
    /// True for the app the graph was requested for (owner view only)
    pub is_current: bool,
    /// True if the requester may add/edit process notes on this app
    pub can_annotate: bool,
    /// Process documentation attached by the app's owners/admins
    pub notes: Vec<ProcessNoteInfo>,
    /// Descriptive tags from the app's metadata (withheld for masked apps)
    pub tags: Vec<String>,
    /// Primary category, e.g. `Productivity` (withheld for masked apps)
    pub category: Option<String>,
    /// External website URL from the app's metadata
    pub website: Option<String>,
    /// Documentation URL from the app's metadata
    pub docs_url: Option<String>,
    /// Summary of what the app contains (withheld for masked apps)
    pub content: Option<AppContentStats>,
}

/// Cheap Postgres-derived counts of the content an app holds. Excludes counts
/// that would require per-app object-store/lancedb calls (boards, tables,
/// files) — those are fetched lazily elsewhere, not in the graph endpoint.
#[derive(Debug, Clone, Default, Serialize, ToSchema)]
pub struct AppContentStats {
    pub events: i64,
    pub pages: i64,
    pub templates: i64,
    pub widgets: i64,
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
    /// How many of those runs ended in a failure/timeout/cancellation
    pub failed_count: i64,
    /// Mean wall-clock duration of completed runs, in milliseconds
    pub avg_duration_ms: Option<f64>,
    /// Unix timestamp of the most recent run on this path
    pub last_run_at: i64,
    /// Name of the event executed on the terminal app (withheld when the
    /// terminal app is masked from the requester)
    pub event_name: Option<String>,
    /// Type of that event, e.g. `simple_chat`, `rest`, `mcp`
    pub event_type: Option<String>,
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
            // Once the node cap is hit an endpoint may not have made it into
            // the set — drop such edges so the graph never references a node
            // that isn't returned.
            if nodes.contains(&connection.source_app_id)
                && nodes.contains(&connection.target_app_id)
            {
                edges.insert(connection.id.clone(), connection);
            }
        }
        frontier = next_frontier;
    }

    Ok((nodes, edges.into_values().collect()))
}

pub(crate) struct ObservedFlow {
    pub path: Vec<String>,
    pub run_count: i64,
    pub failed_count: i64,
    pub avg_duration_ms: Option<f64>,
    pub last_run_at: i64,
    pub event_name: Option<String>,
    pub event_type: Option<String>,
}

fn flow_from_row(row: &sea_orm::QueryResult) -> Result<ObservedFlow, ApiError> {
    let chain: Vec<String> = row.try_get("", "callerAppChain")?;
    let app_id: String = row.try_get("", "appId")?;
    let run_count: i64 = row.try_get("", "run_count")?;
    let failed_count: i64 = row.try_get("", "failed_count")?;
    let avg_duration_ms: Option<f64> = row.try_get("", "avg_duration_ms")?;
    let last_run: chrono::NaiveDateTime = row.try_get("", "last_run")?;
    let event_name: Option<String> = row.try_get("", "event_name")?;
    let event_type: Option<String> = row.try_get("", "event_type")?;
    let mut path = chain;
    path.push(app_id);
    Ok(ObservedFlow {
        path,
        run_count,
        failed_count,
        avg_duration_ms,
        last_run_at: last_run.and_utc().timestamp(),
        event_name,
        event_type,
    })
}

/// The `SELECT`/`GROUP BY` shared by both observed-flow queries. Groups by the
/// executed event so each row names the event, and casts the `status` enum to
/// text to count failures.
const OBSERVED_FLOW_SELECT: &str = r#"SELECT r."callerAppChain", r."appId", e.name AS event_name, e."eventType" AS event_type,
              COUNT(*)::BIGINT AS run_count,
              COUNT(*) FILTER (WHERE r.status IN ('FAILED', 'CANCELLED', 'TIMEOUT'))::BIGINT AS failed_count,
              AVG((EXTRACT(EPOCH FROM (r."completedAt" - r."startedAt")) * 1000.0)::double precision)
                  FILTER (WHERE r."completedAt" IS NOT NULL AND r."startedAt" IS NOT NULL) AS avg_duration_ms,
              MAX(r."updatedAt") AS last_run
       FROM "public"."ExecutionRun" r
       LEFT JOIN "public"."Event" e ON e.id = r."eventId""#;
const OBSERVED_FLOW_GROUP: &str =
    r#"GROUP BY r."callerAppChain", r."appId", e.name, e."eventType""#;

/// Observed chains an app participates in — as terminal app or anywhere in
/// the recorded chain.
pub(crate) async fn observed_flows_for_app(
    state: &AppState,
    app_id: &str,
    since: chrono::NaiveDateTime,
) -> Result<Vec<ObservedFlow>, ApiError> {
    let sql = format!(
        r#"{OBSERVED_FLOW_SELECT}
           WHERE (r."callerAppChain" @> $1 OR (r."appId" = $2 AND array_length(r."callerAppChain", 1) > 0))
             AND r."updatedAt" >= $3
           {OBSERVED_FLOW_GROUP}
           ORDER BY run_count DESC
           LIMIT $4"#
    );
    let stmt = Statement::from_sql_and_values(
        DatabaseBackend::Postgres,
        sql,
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
    let sql = format!(
        r#"{OBSERVED_FLOW_SELECT}
           WHERE array_length(r."callerAppChain", 1) > 0
             AND r."updatedAt" >= $1
           {OBSERVED_FLOW_GROUP}
           ORDER BY run_count DESC
           LIMIT $2"#
    );
    let stmt = Statement::from_sql_and_values(
        DatabaseBackend::Postgres,
        sql,
        [since.into(), (MAX_FLOWS as i64).into()],
    );

    let rows = state.db.query_all(stmt).await?;
    rows.iter().map(flow_from_row).collect()
}

/// Batched Postgres counts of the content each app holds. Only cheap, indexed
/// `appId` group-counts — no object-store/lancedb access.
pub(crate) async fn content_stats(
    state: &AppState,
    app_ids: &[String],
) -> Result<HashMap<String, AppContentStats>, ApiError> {
    let mut stats: HashMap<String, AppContentStats> = HashMap::new();
    if app_ids.is_empty() {
        return Ok(stats);
    }

    let events: Vec<(String, i64)> = event::Entity::find()
        .select_only()
        .column(event::Column::AppId)
        .column_as(Expr::col(event::Column::Id).count(), "cnt")
        .filter(event::Column::AppId.is_in(app_ids.iter().cloned()))
        .group_by(event::Column::AppId)
        .into_tuple()
        .all(&state.db)
        .await?;
    for (app_id, cnt) in events {
        stats.entry(app_id).or_default().events = cnt;
    }

    let pages: Vec<(String, i64)> = page::Entity::find()
        .select_only()
        .column(page::Column::AppId)
        .column_as(Expr::col(page::Column::Id).count(), "cnt")
        .filter(page::Column::AppId.is_in(app_ids.iter().cloned()))
        .group_by(page::Column::AppId)
        .into_tuple()
        .all(&state.db)
        .await?;
    for (app_id, cnt) in pages {
        stats.entry(app_id).or_default().pages = cnt;
    }

    let templates: Vec<(String, i64)> = template::Entity::find()
        .select_only()
        .column(template::Column::AppId)
        .column_as(Expr::col(template::Column::Id).count(), "cnt")
        .filter(template::Column::AppId.is_in(app_ids.iter().cloned()))
        .group_by(template::Column::AppId)
        .into_tuple()
        .all(&state.db)
        .await?;
    for (app_id, cnt) in templates {
        stats.entry(app_id).or_default().templates = cnt;
    }

    let widgets: Vec<(String, i64)> = widget::Entity::find()
        .select_only()
        .column(widget::Column::AppId)
        .column_as(Expr::col(widget::Column::Id).count(), "cnt")
        .filter(widget::Column::AppId.is_in(app_ids.iter().cloned()))
        .group_by(widget::Column::AppId)
        .into_tuple()
        .all(&state.db)
        .await?;
    for (app_id, cnt) in widgets {
        stats.entry(app_id).or_default().widgets = cnt;
    }

    Ok(stats)
}

/// Batched lookup of each app's primary category (serialized to its variant
/// name, e.g. `Productivity`). Best-effort — apps without a category are omitted.
pub(crate) async fn app_category_lookup(
    state: &AppState,
    app_ids: &[String],
) -> HashMap<String, String> {
    let mut out: HashMap<String, String> = HashMap::new();
    if app_ids.is_empty() {
        return out;
    }
    let Ok(rows) = app::Entity::find()
        .filter(app::Column::Id.is_in(app_ids.iter().cloned()))
        .all(&state.db)
        .await
    else {
        return out;
    };
    for row in rows {
        if let Some(category) = row.primary_category
            && let Ok(value) = serde_json::to_value(&category)
            && let Some(label) = value.as_str()
        {
            out.insert(row.id, label.to_string());
        }
    }
    out
}

/// Presigns the icon and banner (thumbnail) media keys of each app so clients
/// receive usable URLs instead of raw storage keys. Best-effort: on any store
/// error, apps are simply left without media rather than failing the request.
pub(crate) async fn presign_media(
    state: &AppState,
    metas: &HashMap<String, AppMetaPreview>,
) -> HashMap<String, (Option<String>, Option<String>)> {
    presign_media_under(state, "apps", metas).await
}

/// Same as [`presign_media`] but for entities whose media lives outside
/// `media/apps/…` — suites keep their artwork under `media/groups/{group_id}`
/// so it survives independently of any single app.
pub(crate) async fn presign_media_under(
    state: &AppState,
    segment: &str,
    metas: &HashMap<String, AppMetaPreview>,
) -> HashMap<String, (Option<String>, Option<String>)> {
    let mut out: HashMap<String, (Option<String>, Option<String>)> = HashMap::new();
    let store = match state.master_credentials().await {
        Ok(creds) => match creds.to_store(false).await {
            Ok(store) => store,
            Err(_) => return out,
        },
        Err(_) => return out,
    };

    // Presigns are independent — run them concurrently instead of one
    // object-store round trip at a time (the graph can hold up to 200 apps).
    let tasks = metas
        .iter()
        .filter(|(_, meta)| meta.icon.is_some() || meta.banner.is_some())
        .map(|(entity_id, meta)| {
            let store = &store;
            async move {
                let mut metadata = Metadata {
                    icon: meta.icon.clone(),
                    thumbnail: meta.banner.clone(),
                    ..Default::default()
                };
                let prefix = FlowPath::from("media")
                    .child(segment.to_string())
                    .child(entity_id.clone());
                metadata.presign(prefix, store).await;
                (entity_id.clone(), (metadata.icon, metadata.thumbnail))
            }
        });
    out.extend(futures::future::join_all(tasks).await);
    out
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
/// but cannot be correlated across apps. Shared with the process-cases
/// endpoint so masked apps resolve to the same graph node ids.
pub(crate) fn mask_app_id(app_id: &str, viewer_app_id: &str) -> String {
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
#[tracing::instrument(
    name = "GET /apps/{app_id}/connections/graph",
    skip(state, user, query)
)]
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
    let content = content_stats(&state, &visible_ids).await?;
    let media = presign_media(&state, &app_meta).await;
    let categories = app_category_lookup(&state, &visible_ids).await;

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
        let media = if visible { media.get(id) } else { None };
        nodes.push(ProcessGraphNode {
            id: display_id(id),
            name: meta.map(|m| m.name.clone()),
            description: meta.and_then(|m| m.description.clone()),
            icon: media.and_then(|(icon, _)| icon.clone()),
            banner: media.and_then(|(_, banner)| banner.clone()),
            unknown: !visible,
            is_current: id == &app_id,
            can_annotate: id == &app_id,
            notes: if visible {
                notes.get(id).cloned().unwrap_or_default()
            } else {
                Vec::new()
            },
            tags: meta.map(|m| m.tags.clone()).unwrap_or_default(),
            category: if visible {
                categories.get(id).cloned()
            } else {
                None
            },
            website: meta.and_then(|m| m.website.clone()),
            docs_url: meta.and_then(|m| m.docs_url.clone()),
            content: if visible {
                content.get(id).cloned()
            } else {
                None
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
        .map(|flow| {
            // The event belongs to the terminal (last) app; withhold its name
            // when that app is masked from the requester.
            let terminal_visible = flow
                .path
                .last()
                .map(|id| accessible.contains(id))
                .unwrap_or(false);
            ProcessFlow {
                path: flow.path.iter().map(|id| display_id(id)).collect(),
                run_count: flow.run_count,
                failed_count: flow.failed_count,
                avg_duration_ms: flow.avg_duration_ms,
                last_run_at: flow.last_run_at,
                event_name: terminal_visible.then_some(flow.event_name).flatten(),
                event_type: terminal_visible.then_some(flow.event_type).flatten(),
            }
        })
        .collect();

    Ok(Json(ProcessGraphResponse {
        nodes,
        edges,
        flows,
    }))
}
