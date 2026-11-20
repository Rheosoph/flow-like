use crate::{
    error::ApiError,
    middleware::jwt::AppUser,
    permission::fork_permission::{ForkTargetKind, check_can_fork},
    state::AppState,
    utils::fork::{
        ForkPolicy, ForkReport, MetaBlob, compute_offline_fork_bundle,
        db_schema::ForkTableSchema,
        detect_meta_in_content_store,
        preview::{RemoteTokenSite, detect_remote_token_sites, ensure_fork_within_limits},
        transfer::{SignedFile, TransferSigner, download_staging_prefix},
    },
};
use axum::{
    Extension, Json,
    extract::{Path, State},
};
use flow_like_storage::{Path as StorePath, join_object_path, object_store::ObjectStoreExt};
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
    /// The app being forked. Its content files come page by page from
    /// `GET /apps/{source_app_id}/fork/offline/files?seed={new_app_id}`.
    pub source_app_id: String,
    /// Remapped + secret-stripped + token-rewritten artifacts (manifest,
    /// boards, events, widgets, templates, pages, versioned forms,
    /// DB-backed metadata files and app metadata media), each as a signed
    /// download of the exact bytes to write under
    /// `apps/{new_app_id}/{path}`. They wait under `tmp/`, which expires.
    pub meta_files: Vec<SignedFile>,
    /// When the signed downloads stop working.
    pub expires_at: chrono::DateTime<chrono::Utc>,
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
    /// The source owner's fork policy — display only. The server already
    /// left out what it excludes.
    pub fork_policy: ForkPolicy,
    /// Arrow schemas for a schema-only database fork. The desktop creates
    /// each table empty in its local project database.
    #[serde(default)]
    pub db_table_schemas: Vec<ForkTableSchema>,
}

/// Begin a fork to an offline (desktop) destination.
///
/// What the server does:
/// 1. Permission + size + token gate (no DB row is created on the
///    destination — offline apps live only on the desktop).
/// 2. Run the fork pipeline in memory and stage the remapped + stripped
///    manifest, boards, events, widgets, templates, pages, and
///    DB-backed metadata files under `tmp/`, each with a signed download.
///    Content files (metadata/, upload/, storage/) follow page by page
///    from `GET /apps/{app_id}/fork/offline/files`, also signed; staged
///    DB metadata wins over a content file with the same destination path.
/// 3. Sanity-check that no `.board`/`.event`/`.template`/`.widget`/
///    `.page` files have leaked into the source content store.
#[utoipa::path(
    post,
    path = "/apps/{app_id}/fork/offline/begin",
    tag = "forking",
    description = "Prepare a copy of an app for this device and return signed downloads of its re-keyed boards, events and pages.",
    params(
        ("app_id" = String, Path, description = "Source application ID"),
    ),
    request_body = BeginOfflineForkBody,
    responses(
        (status = 200, description = "Fork prepared; fetch its files through the signed downloads", body = BeginOfflineForkResponse),
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
    let src_app = check_can_fork(&user, &app_id, &state, ForkTargetKind::Offline).await?;

    let policy = ForkPolicy::from_app_row(&src_app);
    ensure_fork_within_limits(&state, &app_id, &policy).await?;

    // Detect token sites informationally — `compute_offline_fork_bundle`
    // *drops* Remote-mode events, so we don't need a replacement
    // token. The list lets the UI tell the user "these N events
    // didn't make it because they only run server-side."
    let token_sites = detect_remote_token_sites(&state, &app_id).await?;

    let forker = match &user {
        AppUser::Unauthorized => None,
        _ => Some(user.sub()?),
    };
    // `body` is currently empty (see `BeginOfflineForkBody` doc) but
    // kept on the signature so adding fields later is non-breaking.
    let _ = body;

    // Build the offline bundle directly. No `fork_app_with_visibility`
    // call, no destination storage prefix on the server, no DB rows.
    // Just read source meta, remap + strip + drop-remote, return
    // base64 blobs.
    let bundle = compute_offline_fork_bundle(&state, &app_id, forker.as_deref()).await?;

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

    // The re-keyed artifacts wait under `tmp/` for the desktop to fetch
    // them with signed downloads; content files follow through
    // `/fork/offline/files`. No storage credential leaves the server.
    let session_id = flow_like_types::create_id();
    let signer = TransferSigner::content(&state).await?;
    let meta_files = stage_meta_blobs(
        &signer,
        &download_staging_prefix(&app_id, &session_id),
        bundle.blobs,
        &bundle.content_exclude_prefixes,
    )
    .await?;
    Ok(Json(BeginOfflineForkResponse {
        new_app_id: bundle.new_app_id,
        fork_session_id: session_id,
        source_app_id: app_id,
        meta_files,
        expires_at: signer.expires_at(),
        remote_token_sites: token_sites,
        report: ForkReport {
            id_map: bundle.id_map,
            skipped: bundle.skipped,
            warnings: bundle.warnings,
            ..Default::default()
        },
        content_store_leaks,
        fork_policy: bundle.policy,
        db_table_schemas: bundle.db_table_schemas,
    }))
}

const STAGE_CONCURRENCY: usize = 16;

/// Write each artifact the policy keeps under `staging` and sign its
/// download. `manifest.app` always travels: without it there is no app.
async fn stage_meta_blobs(
    signer: &TransferSigner,
    staging: &StorePath,
    blobs: Vec<MetaBlob>,
    exclude_prefixes: &[String],
) -> Result<Vec<SignedFile>, ApiError> {
    use base64::Engine as _;
    use futures::{StreamExt, TryStreamExt};

    let store = signer.store();
    futures::stream::iter(blobs.into_iter().filter(|blob| {
        let path = blob.relative_path.trim_matches('/');
        path == "manifest.app"
            || !exclude_prefixes
                .iter()
                .any(|prefix| path.starts_with(prefix.as_str()))
    }))
    .map(|blob| {
        let store = store.clone();
        async move {
            let path = blob.relative_path.trim_matches('/').to_string();
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(&blob.data_b64)
                .map_err(|e| ApiError::internal(format!("decode fork artifact {path}: {e}")))?;
            let size = bytes.len() as u64;
            let key = join_object_path(staging, &path);
            store
                .put(&key, bytes.into())
                .await
                .map_err(|e| ApiError::internal(format!("stage fork artifact {path}: {e}")))?;
            signer.download(path, &key, Some(size)).await
        }
    })
    .buffer_unordered(STAGE_CONCURRENCY)
    .try_collect()
    .await
}
