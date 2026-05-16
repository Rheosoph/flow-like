use crate::{
    credentials::{CredentialsAccess, RuntimeCredentials},
    error::ApiError,
    middleware::jwt::AppUser,
    permission::fork_permission::{ForkTargetKind, check_can_fork},
    state::AppState,
    utils::fork::{
        ForkReport, MetaBlob, compute_offline_fork_bundle, detect_meta_in_content_store,
        preview::{RemoteTokenSite, compute_app_size_and_count, detect_remote_token_sites},
    },
};
use axum::{
    Extension, Json,
    extract::{Path, State},
};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

#[derive(Clone, Debug, Default, Deserialize, Serialize, ToSchema)]
pub struct BeginOfflineForkBody {
    // Body is intentionally empty today — offline forks don't need a
    // remote-event token (Remote-mode events are *dropped* from the
    // bundle, not token-replaced) and don't need a language hint
    // (metadata files live in the content store and are pulled
    // verbatim via the signed prefix). Kept as a struct so future
    // additions don't require changing the route signature.
}

#[derive(Clone, Debug, Serialize, ToSchema)]
pub struct BeginOfflineForkResponse {
    /// Server-generated id the desktop should record locally for the
    /// new fork. Offline apps don't live in the API DB — there is no
    /// destination App row. The desktop owns the destination's local
    /// state.
    pub new_app_id: String,
    /// Opaque session id for tracking + telemetry.
    pub fork_session_id: String,
    /// Remapped + secret-stripped + token-rewritten inline artifacts
    /// (manifest, boards, events, widgets, templates, pages,
    /// versioned forms, DB-backed metadata files, and app metadata
    /// media). Each
    /// `MetaBlob.data_b64` is the exact bytes that would have been
    /// written to disk on a destination — the desktop
    /// base64-decodes and writes it under
    /// `apps/{new_app_id}/{relative_path}`, choosing the local meta
    /// or content store based on the relative path.
    pub meta_blobs: Vec<MetaBlob>,
    /// Bucket-relative prefix of the **source** content store the
    /// desktop should pull from (e.g. `apps/{src_app_id}`). Combined
    /// with `shared_credentials` this gives prefix-isolated read
    /// access — list once, download in parallel, no per-object
    /// signing. The desktop translates `metadata/{widgets|templates|
    /// pages}/{src_id}/...` paths to their destination ids
    /// client-side using `id_map` before writing locally.
    pub source_content_prefix: String,
    /// Scoped read credentials for `source_content_prefix`. Single
    /// signature, valid until `expires_at`. The desktop builds an
    /// `object_store` client from these to list + GET. Does **not**
    /// grant access to the source's meta store — meta artifacts
    /// already arrived inline in `meta_blobs`.
    pub shared_credentials: serde_json::Value,
    /// Wall-clock timestamp (ISO8601) after which the credentials
    /// stop working.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<chrono::DateTime<chrono::Utc>>,
    /// Detected token sites on the source — informational. The
    /// replacement / clearing already happened during the bundle
    /// build step.
    pub remote_token_sites: Vec<RemoteTokenSite>,
    /// Skipped resources (packages, OAuth sinks, secrets) the user
    /// should be told about post-fork.
    pub report: ForkReport,
    /// Files in the source content prefix flagged as suspicious
    /// (e.g. `.board`/`.event` files in the content store). Empty in
    /// healthy deployments — populated only when the deployment has a
    /// store-split misconfiguration. Surface in the UI as a warning
    /// so the operator can investigate.
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub content_store_leaks: Vec<String>,
}

/// Begin a fork to an offline (desktop) destination.
///
/// What the server does:
/// 1. Permission + size + token gate (no DB row is created on the
///    destination — offline apps live only on the desktop).
/// 2. Run the fork pipeline in memory, so the remapped + stripped
///    manifest, boards, events, widgets, templates, pages, and
///    DB-backed metadata files can ship inline.
/// 3. Issue scoped read credentials for the **source content**
///    prefix; the desktop pulls legacy metadata/, upload/, storage/
///    directly via `object_store` — no per-object signing, no fan-out
///    through the API server. Inline DB metadata wins over legacy
///    metadata files with the same destination path.
/// 4. Sanity-check that no `.board`/`.event`/`.template`/`.widget`/
///    `.page` files have leaked into the source content store.
#[utoipa::path(
    post,
    path = "/apps/{app_id}/fork/offline/begin",
    tag = "forking",
    description = "Compute an offline-bundle fork in memory and return scoped read credentials over the source content prefix.",
    params(
        ("app_id" = String, Path, description = "Source application ID"),
    ),
    request_body = BeginOfflineForkBody,
    responses(
        (status = 200, description = "Fork computed; pull content via the returned scoped credentials", body = BeginOfflineForkResponse),
        (status = 400, description = "Source app exceeds size cap, or remote tokens detected without `remote_event_token`"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
        (status = 404, description = "Source app not found"),
        (status = 503, description = "Forking is disabled by the deployment configuration")
    )
)]
#[tracing::instrument(
    name = "POST /apps/{app_id}/fork/offline/begin",
    skip(state, user, body)
)]
pub async fn begin_offline_fork(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(app_id): Path<String>,
    Json(body): Json<BeginOfflineForkBody>,
) -> Result<Json<BeginOfflineForkResponse>, ApiError> {
    let _src_app = check_can_fork(&user, &app_id, &state, ForkTargetKind::Offline).await?;

    let (total_size, total_count) = compute_app_size_and_count(&state, &app_id).await?;
    let max_size = state.platform_config.forking.max_size_bytes;
    let max_count = state.platform_config.forking.max_file_count;
    if total_size > max_size {
        return Err(ApiError::bad_request(format!(
            "source app exceeds the deployment's fork size cap ({} bytes > {} bytes)",
            total_size, max_size
        )));
    }
    if total_count > max_count {
        return Err(ApiError::bad_request(format!(
            "source app exceeds the deployment's fork file-count cap ({} > {})",
            total_count, max_count
        )));
    }

    // Detect token sites informationally — `compute_offline_fork_bundle`
    // *drops* Remote-mode events, so we don't need a replacement
    // token. The list lets the UI tell the user "these N events
    // didn't make it because they only run server-side."
    let token_sites = detect_remote_token_sites(&state, &app_id).await?;

    let user_sub = user.sub()?;
    // `body` is currently empty (see `BeginOfflineForkBody` doc) but
    // kept on the signature so adding fields later is non-breaking.
    let _ = body;

    // Build the offline bundle directly. No `fork_app_with_visibility`
    // call, no destination storage prefix on the server, no DB rows.
    // Just read source meta, remap + strip + drop-remote, return
    // base64 blobs.
    let bundle = compute_offline_fork_bundle(&state, &app_id).await?;

    // Sanity check: a healthy deployment never has `.board` /
    // `.event` files under the *content* store. Don't fail the fork
    // — just surface so an operator can investigate.
    let content_store_leaks = detect_meta_in_content_store(&state, &app_id)
        .await
        .unwrap_or_default();
    if !content_store_leaks.is_empty() {
        tracing::warn!(
            src_app = %app_id,
            leaks = content_store_leaks.len(),
            "found meta-store-shaped files in the source content store",
        );
    }

    // Sign read access to the **source content** prefix only — boards
    // / events / widgets / templates / pages live in the meta bucket
    // and may carry secrets that we strip server-side before they
    // ship in `meta_blobs`. `ReadAppContent` deliberately omits the
    // meta bucket from the policy so a misbehaving client can't
    // bypass the API by GETting raw `*.board` / `*.event` files.
    let scoped = RuntimeCredentials::scoped(
        &user_sub,
        &app_id,
        &state,
        CredentialsAccess::ReadAppContent,
    )
    .await
    .map_err(|e| {
        tracing::error!("Failed to generate scoped read credentials for fork: {}", e);
        ApiError::internal("Failed to generate fork content credentials")
    })?;
    let expires_at = credentials_expiration(&scoped);
    let shared_credentials =
        serde_json::to_value(scoped.into_shared_credentials()).map_err(|e| {
            tracing::error!("Failed to serialize shared credentials for fork: {}", e);
            ApiError::internal("Failed to serialize fork content credentials")
        })?;

    let session_id = flow_like_types::create_id();
    let source_content_prefix = format!("apps/{}", app_id);
    Ok(Json(BeginOfflineForkResponse {
        new_app_id: bundle.new_app_id,
        fork_session_id: session_id,
        meta_blobs: bundle.blobs,
        source_content_prefix,
        shared_credentials,
        expires_at,
        remote_token_sites: token_sites,
        report: ForkReport {
            id_map: bundle.id_map,
            skipped: bundle.skipped,
            ..Default::default()
        },
        content_store_leaks,
    }))
}

fn credentials_expiration(creds: &RuntimeCredentials) -> Option<chrono::DateTime<chrono::Utc>> {
    match creds {
        #[cfg(feature = "aws")]
        RuntimeCredentials::Aws(aws) => aws.expiration,
        #[cfg(feature = "azure")]
        RuntimeCredentials::Azure(azure) => azure.expiration,
        #[cfg(feature = "gcp")]
        RuntimeCredentials::Gcp(gcp) => gcp.expiration,
        #[cfg(feature = "r2")]
        RuntimeCredentials::R2(r2) => r2.expiration,
        RuntimeCredentials::Mixed(mixed) => credentials_expiration(&mixed.content),
    }
}
