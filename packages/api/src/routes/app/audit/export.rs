use axum::{
    Extension,
    body::Body,
    extract::{Path, Query, State},
    http::{StatusCode, header},
    response::Response,
};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

use crate::{
    audit::{
        export::{self, MAX_PAGE_RECORDS, MAX_PAGE_SEALS, NDJSON_CONTENT_TYPE},
        record::ACTIVITY_SUFFIX,
    },
    ensure_permission,
    error::ApiError,
    middleware::jwt::AppUser,
    permission::role_permission::RolePermissions,
    state::AppState,
};

pub const NEXT_SEQ_HEADER: &str = "X-FlowLike-Audit-Next-Seq";

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum AuditExportClass {
    /// Records kept as evidence and archived.
    #[default]
    Evidence,
    /// Verbose-only records with the shorter activity retention.
    Activity,
}

#[derive(Debug, Deserialize, IntoParams)]
pub struct AuditExportParams {
    /// Which of the app's chains to read. Defaults to `evidence`.
    pub class: Option<AuditExportClass>,
    /// Return seals after this sequence: 0 for the start, then the previous page's
    /// `X-FlowLike-Audit-Next-Seq`.
    pub after_seq: Option<i64>,
    /// Seals per page, default and maximum 100.
    pub limit: Option<u64>,
}

#[utoipa::path(
    get,
    path = "/apps/{app_id}/audit/export",
    tag = "audit",
    description = "Export the app's audit chain as newline-delimited JSON: the signed epochs, then each seal followed by its records with their raw values, anchored seals only. A watermark line comes first when records before the cursor were pruned. Verify offline with the epoch signatures, the seal inclusion proofs and the record hashes. Continue with `after_seq` set to the `X-FlowLike-Audit-Next-Seq` header, which is absent when there is nothing new. Requires Owner on the app.",
    params(
        ("app_id" = String, Path, description = "Application ID"),
        AuditExportParams
    ),
    responses(
        (status = 200, description = "One page of the chain as NDJSON; empty when there is nothing new", body = String, content_type = "application/x-ndjson",
            headers(("X-FlowLike-Audit-Next-Seq" = i64, description = "Sequence to pass as `after_seq` for the next page"))),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden")
    ),
    security(
        ("bearer_auth" = []),
        ("api_key" = []),
        ("pat" = [])
    )
)]
#[tracing::instrument(name = "GET /apps/{app_id}/audit/export", skip(state, user, params))]
pub async fn export_records(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(app_id): Path<String>,
    Query(params): Query<AuditExportParams>,
) -> Result<Response, ApiError> {
    ensure_permission!(user, &app_id, &state, RolePermissions::Owner);
    let chain_id = match params.class.unwrap_or_default() {
        AuditExportClass::Evidence => app_id.clone(),
        AuditExportClass::Activity => format!("{app_id}{ACTIVITY_SUFFIX}"),
    };
    let after_seq = params.after_seq.unwrap_or(0).max(0);
    let limit = params
        .limit
        .unwrap_or(MAX_PAGE_SEALS)
        .clamp(1, MAX_PAGE_SEALS);
    let page = export::page(&state.db, &chain_id, after_seq, limit, MAX_PAGE_RECORDS).await?;

    let mut response = Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, NDJSON_CONTENT_TYPE);
    if let Some(last_seq) = page.last_seq {
        response = response.header(NEXT_SEQ_HEADER, last_seq.to_string());
    }
    response.body(Body::from(page.body)).map_err(|error| {
        ApiError::internal(format!(
            "audit export response for chain {chain_id} could not be built: {error}"
        ))
    })
}
