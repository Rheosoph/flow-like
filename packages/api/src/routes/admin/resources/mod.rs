//! Health and capacity of the services this deployment runs on.
//!
//! Every probe reachable from here must be O(1) or explicitly bounded. An operator
//! refreshes this page while something is on fire, so a metric that needs a full
//! object-store listing or a `COUNT(*)` would make the dashboard the most expensive
//! request the deployment serves — and it would get slower exactly as the data grows,
//! which is when the numbers matter most. Anything that costs a scan belongs in a
//! scheduled snapshot job that writes a row this endpoint can read for free.

pub mod cache;
pub mod database;
pub mod storage;
pub mod types;

pub use types::*;

use std::{future::Future, time::Duration};

use axum::{Extension, Json, extract::State};

use crate::{
    error::ApiError, middleware::jwt::AppUser, permission::global_permission::GlobalPermission,
    state::AppState,
};

/// Key for the shared 60-second response cache. One key for the whole payload: the
/// probes are only meaningful as a set, and a partially refreshed dashboard would show
/// a database and a cache sampled minutes apart as if they were one moment.
const RESPONSE_CACHE_KEY: &str = "admin:resources";

/// Ceiling for a single probe.
///
/// A backend that has stopped answering must not hold the request open — the whole
/// point of probing concurrently is that a dead Redis still lets the operator read the
/// database numbers.
const PROBE_BUDGET: Duration = Duration::from_secs(4);

async fn within_budget<T>(probe: impl Future<Output = T>) -> Option<T> {
    flow_like_types::tokio::time::timeout(PROBE_BUDGET, probe)
        .await
        .ok()
}

fn budget_exceeded(id: &str, kind: ResourceKind, label: &str) -> ResourceStatus {
    ResourceStatus::new(id, kind, label, "unknown").failed(format!(
        "Probe did not answer within its {}s budget",
        PROBE_BUDGET.as_secs()
    ))
}

/// GET /admin/resources
#[utoipa::path(
    get,
    path = "/admin/resources",
    tag = "admin",
    description = "Status, capacity and throughput of the services this deployment runs on: database, cache, execution state store and object storage. Backends that fail to answer are reported individually, so one outage never hides the rest.",
    responses(
        (status = 200, description = "Status of every backing service", body = AdminResourcesResponse),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden — Admin permission required")
    )
)]
#[tracing::instrument(name = "GET /admin/resources", skip(state, user))]
pub async fn get_resources(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
) -> Result<Json<AdminResourcesResponse>, ApiError> {
    user.check_global_permission(&state, GlobalPermission::Admin)
        .await?;

    if let Some(cached) = state.get_cache::<AdminResourcesResponse>(RESPONSE_CACHE_KEY) {
        return Ok(Json(AdminResourcesResponse {
            cached: true,
            ..cached
        }));
    }

    let (cache, state_store, database, storage) = futures::future::join4(
        within_budget(cache::probe(&state)),
        within_budget(cache::probe_state_store(&state)),
        within_budget(database::probe(&state)),
        within_budget(storage::probe(&state)),
    )
    .await;

    let (database, database_detail) = database.unwrap_or_else(|| {
        (
            budget_exceeded("database", ResourceKind::Database, "Database"),
            None,
        )
    });

    let cache = cache.unwrap_or_else(|| budget_exceeded("cache", ResourceKind::Cache, "Cache"));
    let state_store = state_store.unwrap_or_else(|| {
        budget_exceeded("state-store", ResourceKind::StateStore, "Execution state")
    });
    let storage = storage
        .unwrap_or_else(|| vec![budget_exceeded("storage", ResourceKind::Storage, "Storage")]);

    let mut resources = vec![database, cache, state_store];
    resources.extend(storage);

    let response = AdminResourcesResponse {
        generated_at: chrono::Utc::now().to_rfc3339(),
        cached: false,
        resources,
        database_detail,
    };

    state.set_cache(RESPONSE_CACHE_KEY.to_string(), response.clone());

    Ok(Json(response))
}
