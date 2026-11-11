//! Removes email addresses and account ids from the names and usernames of existing
//! accounts, giving each a username derived from its name or address where possible.

use crate::{
    error::ApiError,
    middleware::jwt::AppUser,
    permission::global_permission::GlobalPermission,
    routes::user::handle::{IdentityRepairReport, repair_identities},
    state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Query, State},
};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

const DEFAULT_PAGE: u64 = 100;
const MAX_PAGE: u64 = 200;

#[derive(Debug, Deserialize, IntoParams)]
pub struct RepairIdentitiesQuery {
    /// Continue after this account id, from the previous page's `next_cursor`.
    pub after: Option<String>,
    /// Accounts per page, 1 to 200. Defaults to 100.
    pub limit: Option<u64>,
    /// Report what would change without writing anything.
    #[serde(default)]
    pub dry_run: bool,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct RepairIdentitiesPage {
    #[serde(flatten)]
    pub report: IdentityRepairReport,
    /// Pass as `after` for the next page; absent after the last one.
    pub next_cursor: Option<String>,
}

#[utoipa::path(
    post,
    path = "/admin/users/repair-identities",
    tag = "admin",
    params(RepairIdentitiesQuery),
    responses(
        (status = 200, description = "What this page of accounts changed, or would change on a dry run", body = RepairIdentitiesPage),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Requires Admin permission")
    ),
    description = "Removes email addresses and account ids from account names and usernames, one page of accounts at a time. Accounts get a username derived from their name or address; accounts without either choose one at their next sign-in. Usernames people chose stay. Requires Admin permission."
)]
#[tracing::instrument(name = "POST /admin/users/repair-identities", skip_all)]
pub async fn repair(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Query(query): Query<RepairIdentitiesQuery>,
) -> Result<Json<RepairIdentitiesPage>, ApiError> {
    user.check_global_permission(&state, GlobalPermission::Admin)
        .await?;
    let limit = query.limit.unwrap_or(DEFAULT_PAGE).clamp(1, MAX_PAGE);
    let (report, next_cursor) =
        repair_identities(&state.db, query.after.as_deref(), limit, query.dry_run).await?;
    if !query.dry_run {
        tracing::info!(
            scanned = report.scanned,
            handles_set = report.handles_set,
            handles_cleared = report.handles_cleared,
            names_cleared = report.names_cleared,
            skipped = report.skipped,
            "repaired account names and usernames"
        );
    }
    Ok(Json(RepairIdentitiesPage {
        report,
        next_cursor,
    }))
}
