//! Process cases — end-to-end runs reconstructed from the correlation spine.
//!
//! A "case" is a causal execution tree: a root run (no `parent_run_id`) plus
//! every run it transitively triggered across apps and events. This is the
//! process-mining case notion — the thing that flows through the process — and
//! it's reconstructed with a recursive walk of `parent_run_id`, so it holds
//! even before `trace_id` denormalization is fully populated.
//!
//! Apps the requesting user cannot access are pseudonymized with the same
//! keying as the process graph, so masked case hops resolve to the graph's
//! "unknown" nodes instead of leaking raw app ids.

use crate::{
    ensure_permission, error::ApiError, middleware::jwt::AppUser,
    permission::role_permission::RolePermissions, routes::app::connection::graph::mask_app_id,
    state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, Query, State},
};
use sea_orm::{
    ColumnTrait, ConnectionTrait, DatabaseBackend, EntityTrait, QueryFilter, QuerySelect, Statement,
};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use utoipa::ToSchema;

const MAX_CASES: i64 = 500;
/// Hard cap on the parent walk. Legit chains are bounded by the connection
/// depth (8); this additionally protects the recursive CTE from corrupted
/// parent links (a cycle would otherwise recurse without terminating).
const MAX_CASE_DEPTH: i32 = 16;

#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct ProcessCasesQuery {
    /// Time window in days (default 30, max 365)
    pub days: Option<i64>,
}

/// One reconstructed process case (a full causal execution tree).
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct ProcessCase {
    /// Case id = the root run id of the tree.
    pub case_id: String,
    /// App that started the case.
    pub root_app_id: String,
    /// Event that started the case, if any.
    pub root_event_name: Option<String>,
    pub root_event_type: Option<String>,
    /// Apps the case traversed, in call order (masked when inaccessible).
    pub apps: Vec<String>,
    pub run_count: i64,
    pub failed_count: i64,
    /// Business/object keys tagged on the root run.
    #[schema(value_type = Object)]
    pub correlation_keys: Option<serde_json::Value>,
    /// Aggregate status: `Failed` if any run failed, else `Running` if any is
    /// still in flight, else `Completed`.
    pub status: String,
    /// Unix timestamp the case started (earliest run start).
    pub started_at: Option<i64>,
    /// Unix timestamp of the most recent activity in the case.
    pub last_activity_at: i64,
    /// Wall-clock span of the case in milliseconds (last completion − first
    /// start), when both are known.
    pub duration_ms: Option<f64>,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct ProcessCasesResponse {
    pub cases: Vec<ProcessCase>,
}

/// One run within a case's causal tree, with timing for waterfall rendering.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct ProcessCaseRun {
    pub run_id: String,
    /// App the run executed in (masked when inaccessible).
    pub app_id: String,
    pub parent_run_id: Option<String>,
    /// Distance from the case root (0 = root).
    pub depth: i32,
    /// Run status: PENDING, RUNNING, COMPLETED, FAILED, CANCELLED, TIMEOUT.
    pub status: String,
    pub event_name: Option<String>,
    pub event_type: Option<String>,
    /// Unix timestamp the run started (falls back to creation time).
    pub started_at: i64,
    pub completed_at: Option<i64>,
    /// Last update — end fallback for runs without a completion time.
    pub updated_at: i64,
    pub duration_ms: Option<f64>,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct ProcessCaseDetailResponse {
    pub runs: Vec<ProcessCaseRun>,
}

struct RawCase {
    case_id: String,
    root_app_id: String,
    root_event_name: Option<String>,
    root_event_type: Option<String>,
    apps: Vec<String>,
    run_count: i64,
    failed_count: i64,
    running_count: i64,
    correlation_keys: Option<serde_json::Value>,
    started_at: Option<chrono::NaiveDateTime>,
    last_activity_at: chrono::NaiveDateTime,
    duration_ms: Option<f64>,
}

#[utoipa::path(
    get,
    path = "/apps/{app_id}/connections/cases",
    tag = "team",
    description = "End-to-end process cases the app started, reconstructed from the run correlation spine (parent_run_id). Each case is a causal execution tree spanning apps and events.",
    params(
        ("app_id" = String, Path, description = "Application ID"),
        ("days" = Option<i64>, Query, description = "Time window in days (default 30)")
    ),
    responses(
        (status = 200, description = "Reconstructed process cases", body = ProcessCasesResponse),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden")
    ),
    security(("bearer_auth" = []), ("api_key" = []), ("pat" = []))
)]
#[tracing::instrument(
    name = "GET /apps/{app_id}/connections/cases",
    skip(state, user, query)
)]
pub async fn list_process_cases(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(app_id): Path<String>,
    Query(query): Query<ProcessCasesQuery>,
) -> Result<Json<ProcessCasesResponse>, ApiError> {
    let permission = ensure_permission!(user, &app_id, &state, RolePermissions::Owner);
    let viewer_sub = permission.effective_user_id().ok();

    let days = query.days.unwrap_or(30).clamp(1, 365);
    let since = chrono::Utc::now().naive_utc() - chrono::Duration::days(days);

    // Roots are runs the app started (no parent) in the window; the recursive
    // arm walks parent_run_id down through every app the case reached. `depth`
    // both bounds the recursion and preserves call order for the app path.
    let stmt = Statement::from_sql_and_values(
        DatabaseBackend::Postgres,
        r#"WITH RECURSIVE tree AS (
               SELECT id AS root_id, id AS run_id, 0 AS depth
               FROM "public"."ExecutionRun"
               WHERE "parentRunId" IS NULL AND "appId" = $1 AND "updatedAt" >= $2
             UNION ALL
               SELECT t.root_id, r.id, t.depth + 1
               FROM "public"."ExecutionRun" r
               JOIN tree t ON r."parentRunId" = t.run_id
               WHERE t.depth < $3
           )
           SELECT
               t.root_id AS case_id,
               root."appId" AS root_app_id,
               e.name AS root_event_name,
               e."eventType" AS root_event_type,
               root."correlationKeys" AS correlation_keys,
               COUNT(*)::BIGINT AS run_count,
               COUNT(*) FILTER (WHERE r.status IN ('FAILED', 'CANCELLED', 'TIMEOUT'))::BIGINT AS failed_count,
               COUNT(*) FILTER (WHERE r.status IN ('PENDING', 'RUNNING'))::BIGINT AS running_count,
               ARRAY_AGG(r."appId" ORDER BY t.depth, r."createdAt") AS apps,
               MIN(r."startedAt") AS started_at,
               MAX(GREATEST(r."completedAt", r."updatedAt")) AS last_activity_at,
               (EXTRACT(EPOCH FROM (MAX(r."completedAt") - MIN(r."startedAt"))) * 1000.0)::double precision AS duration_ms
           FROM tree t
           JOIN "public"."ExecutionRun" r ON r.id = t.run_id
           JOIN "public"."ExecutionRun" root ON root.id = t.root_id
           LEFT JOIN "public"."Event" e ON e.id = root."eventId"
           GROUP BY t.root_id, root."appId", root."correlationKeys", e.name, e."eventType"
           ORDER BY last_activity_at DESC
           LIMIT $4"#,
        [
            app_id.clone().into(),
            since.into(),
            MAX_CASE_DEPTH.into(),
            MAX_CASES.into(),
        ],
    );

    let rows = state.db.query_all(stmt).await?;
    let mut raw_cases = Vec::with_capacity(rows.len());
    let mut all_app_ids: HashSet<String> = HashSet::new();
    for row in &rows {
        let apps: Vec<String> = row.try_get("", "apps")?;
        all_app_ids.extend(apps.iter().cloned());
        raw_cases.push(RawCase {
            case_id: row.try_get("", "case_id")?,
            root_app_id: row.try_get("", "root_app_id")?,
            root_event_name: row.try_get("", "root_event_name")?,
            root_event_type: row.try_get("", "root_event_type")?,
            apps,
            run_count: row.try_get("", "run_count")?,
            failed_count: row.try_get("", "failed_count")?,
            running_count: row.try_get("", "running_count")?,
            correlation_keys: row.try_get("", "correlation_keys")?,
            started_at: row.try_get("", "started_at")?,
            last_activity_at: row.try_get("", "last_activity_at")?,
            duration_ms: row.try_get("", "duration_ms")?,
        });
    }

    // Same masking rule as the process graph: the viewer only sees apps they
    // are a member of; everything else is pseudonymized with the same keying,
    // so masked case hops line up with the graph's "unknown" nodes.
    let mut accessible: HashSet<String> = HashSet::from([app_id.clone()]);
    if let Some(sub) = &viewer_sub
        && !all_app_ids.is_empty()
    {
        use crate::entity::membership;
        let member_of: Vec<String> = membership::Entity::find()
            .filter(membership::Column::UserId.eq(sub))
            .filter(membership::Column::AppId.is_in(all_app_ids.iter().cloned()))
            .select_only()
            .column(membership::Column::AppId)
            .into_tuple()
            .all(&state.db)
            .await?;
        accessible.extend(member_of);
    }
    let display_id = |id: &str| -> String {
        if accessible.contains(id) {
            id.to_string()
        } else {
            mask_app_id(id, &app_id)
        }
    };

    let cases = raw_cases
        .into_iter()
        .map(|raw| {
            let status = if raw.failed_count > 0 {
                "Failed"
            } else if raw.running_count > 0 {
                "Running"
            } else {
                "Completed"
            };
            // Dedupe while preserving first-visit (call) order.
            let mut seen = HashSet::new();
            let apps: Vec<String> = raw
                .apps
                .iter()
                .filter(|id| seen.insert((*id).clone()))
                .map(|id| display_id(id))
                .collect();

            ProcessCase {
                case_id: raw.case_id,
                root_app_id: display_id(&raw.root_app_id),
                root_event_name: raw.root_event_name,
                root_event_type: raw.root_event_type,
                apps,
                run_count: raw.run_count,
                failed_count: raw.failed_count,
                correlation_keys: raw.correlation_keys,
                status: status.to_string(),
                started_at: raw.started_at.map(|dt| dt.and_utc().timestamp()),
                last_activity_at: raw.last_activity_at.and_utc().timestamp(),
                duration_ms: raw.duration_ms,
            }
        })
        .collect();

    Ok(Json(ProcessCasesResponse { cases }))
}

#[utoipa::path(
    get,
    path = "/apps/{app_id}/connections/cases/{case_id}",
    tag = "team",
    description = "All runs of one process case as a causal tree with timing — the drill-down behind the case list.",
    params(
        ("app_id" = String, Path, description = "Application ID"),
        ("case_id" = String, Path, description = "Case id (the root run id)")
    ),
    responses(
        (status = 200, description = "Runs of the case in tree order", body = ProcessCaseDetailResponse),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
        (status = 404, description = "Case not found")
    ),
    security(("bearer_auth" = []), ("api_key" = []), ("pat" = []))
)]
#[tracing::instrument(
    name = "GET /apps/{app_id}/connections/cases/{case_id}",
    skip(state, user)
)]
pub async fn get_process_case(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((app_id, case_id)): Path<(String, String)>,
) -> Result<Json<ProcessCaseDetailResponse>, ApiError> {
    let permission = ensure_permission!(user, &app_id, &state, RolePermissions::Owner);
    let viewer_sub = permission.effective_user_id().ok();

    // The anchor pins the case root to this app, so a case id from another
    // app cannot be walked through this endpoint.
    let stmt = Statement::from_sql_and_values(
        DatabaseBackend::Postgres,
        r#"WITH RECURSIVE tree AS (
               SELECT id, 0 AS depth
               FROM "public"."ExecutionRun"
               WHERE id = $2 AND "appId" = $1 AND "parentRunId" IS NULL
             UNION ALL
               SELECT r.id, t.depth + 1
               FROM "public"."ExecutionRun" r
               JOIN tree t ON r."parentRunId" = t.id
               WHERE t.depth < $3
           )
           SELECT
               r.id AS run_id,
               r."appId" AS app_id,
               r."parentRunId" AS parent_run_id,
               t.depth,
               r.status::text AS status,
               e.name AS event_name,
               e."eventType" AS event_type,
               COALESCE(r."startedAt", r."createdAt") AS started_at,
               r."completedAt" AS completed_at,
               r."updatedAt" AS updated_at,
               (EXTRACT(EPOCH FROM (r."completedAt" - r."startedAt")) * 1000.0)::double precision AS duration_ms
           FROM tree t
           JOIN "public"."ExecutionRun" r ON r.id = t.id
           LEFT JOIN "public"."Event" e ON e.id = r."eventId"
           ORDER BY t.depth ASC, started_at ASC
           LIMIT 500"#,
        [app_id.clone().into(), case_id.into(), MAX_CASE_DEPTH.into()],
    );

    let rows = state.db.query_all(stmt).await?;
    if rows.is_empty() {
        return Err(ApiError::not_found("Case not found"));
    }

    struct RawRun {
        run_id: String,
        app_id: String,
        parent_run_id: Option<String>,
        depth: i32,
        status: String,
        event_name: Option<String>,
        event_type: Option<String>,
        started_at: chrono::NaiveDateTime,
        completed_at: Option<chrono::NaiveDateTime>,
        updated_at: chrono::NaiveDateTime,
        duration_ms: Option<f64>,
    }

    let mut raw_runs = Vec::with_capacity(rows.len());
    let mut all_app_ids: HashSet<String> = HashSet::new();
    for row in &rows {
        let run_app_id: String = row.try_get("", "app_id")?;
        all_app_ids.insert(run_app_id.clone());
        raw_runs.push(RawRun {
            run_id: row.try_get("", "run_id")?,
            app_id: run_app_id,
            parent_run_id: row.try_get("", "parent_run_id")?,
            depth: row.try_get("", "depth")?,
            status: row.try_get("", "status")?,
            event_name: row.try_get("", "event_name")?,
            event_type: row.try_get("", "event_type")?,
            started_at: row.try_get("", "started_at")?,
            completed_at: row.try_get("", "completed_at")?,
            updated_at: row.try_get("", "updated_at")?,
            duration_ms: row.try_get("", "duration_ms")?,
        });
    }

    let mut accessible: HashSet<String> = HashSet::from([app_id.clone()]);
    if let Some(sub) = &viewer_sub
        && !all_app_ids.is_empty()
    {
        use crate::entity::membership;
        let member_of: Vec<String> = membership::Entity::find()
            .filter(membership::Column::UserId.eq(sub))
            .filter(membership::Column::AppId.is_in(all_app_ids.iter().cloned()))
            .select_only()
            .column(membership::Column::AppId)
            .into_tuple()
            .all(&state.db)
            .await?;
        accessible.extend(member_of);
    }
    let display_id = |id: &str| -> String {
        if accessible.contains(id) {
            id.to_string()
        } else {
            mask_app_id(id, &app_id)
        }
    };

    let runs = raw_runs
        .into_iter()
        .map(|raw| ProcessCaseRun {
            run_id: raw.run_id,
            app_id: display_id(&raw.app_id),
            parent_run_id: raw.parent_run_id,
            depth: raw.depth,
            status: raw.status,
            event_name: raw.event_name,
            event_type: raw.event_type,
            started_at: raw.started_at.and_utc().timestamp(),
            completed_at: raw.completed_at.map(|dt| dt.and_utc().timestamp()),
            updated_at: raw.updated_at.and_utc().timestamp(),
            duration_ms: raw.duration_ms,
        })
        .collect();

    Ok(Json(ProcessCaseDetailResponse { runs }))
}
