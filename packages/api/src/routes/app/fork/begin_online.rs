use std::sync::Arc;

use crate::{
    credentials::{CredentialsAccess, RuntimeCredentials},
    error::ApiError,
    middleware::jwt::AppUser,
    state::AppState,
    utils::fork::job::{self, ForkJobSpec},
};
use axum::{Extension, Json, extract::State};
use flow_like::{
    app::{App, AppStatus as CoreAppStatus},
    bit::Metadata,
};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

#[derive(Clone, Debug, Default, Deserialize, Serialize, ToSchema)]
pub struct BeginOnlineForkBody {
    /// Caller-reported source app id from the desktop. Stored in
    /// `forked_from` for lineage; the server can't independently
    /// verify it (the source lives on the desktop). May be `None`
    /// when the desktop is forking a local-only app.
    #[serde(default)]
    pub source_app_id: Option<String>,
    /// Local size + count, computed by the desktop before calling.
    /// The server enforces the deployment cap against these values
    /// upfront; if the desktop later uploads more, the finalize
    /// step rejects the bundle.
    pub summary: BundleSummary,
    /// Optional language hint for the destination's default metadata.
    /// Falls back to "en".
    #[serde(default)]
    pub language: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, ToSchema)]
pub struct BundleSummary {
    pub total_size_bytes: u64,
    pub total_object_count: u64,
}

#[derive(Clone, Debug, Serialize, ToSchema)]
pub struct BeginOnlineForkResponse {
    /// Destination app id allocated by the server. The desktop
    /// uploads to `apps/{new_app_id}/...` using the returned
    /// scoped credentials.
    pub new_app_id: String,
    /// The fork job that owns the upload. Readable through
    /// `GET /apps/fork/jobs/{job_id}`; it completes when
    /// `POST /apps/{new_app_id}/fork/online/finalize` succeeds and is
    /// aborted (rows, storage and the app removed) if the upload is
    /// abandoned past its expiry.
    pub fork_session_id: String,
    /// Path the desktop should treat as the upload root (matches
    /// the destination prefix on the server's master store).
    pub upload_path: String,
    /// Scoped credentials with `EditAppContent` access — the desktop
    /// uses these only for content objects. Meta objects are pushed
    /// through app-edit endpoints so DB state stays in sync.
    pub shared_credentials: serde_json::Value,
    /// Optional credentials expiration (ISO8601). The desktop must
    /// finish the upload before this and call `finalize`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expiration: Option<chrono::DateTime<chrono::Utc>>,
}

/// Allocate a destination app for an offline → online fork. The
/// desktop computes the bundle locally (already secret-stripped and
/// token-rewritten on its side per the same rules `fork_app` enforces
/// server-side) and uploads it directly to object storage using the
/// scoped credentials this endpoint returns. Once the upload is
/// complete the desktop calls `POST /apps/fork/online/{session_id}/finalize`
/// to flip the destination to `Private` and (eventually) materialize
/// the DB rows from the uploaded manifest.
#[utoipa::path(
    post,
    path = "/apps/fork/online/begin",
    tag = "forking",
    description = "Allocate a destination app + scoped credentials for an offline → online fork upload.",
    request_body = BeginOnlineForkBody,
    responses(
        (status = 200, description = "Destination allocated; upload via the returned credentials, then call finalize", body = BeginOnlineForkResponse),
        (status = 400, description = "Bundle exceeds the deployment's size or file-count cap"),
        (status = 401, description = "Unauthorized"),
        (status = 503, description = "Forking is disabled by the deployment configuration")
    )
)]
#[tracing::instrument(name = "POST /apps/fork/online/begin", skip(state, user, body))]
pub async fn begin_online_fork(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Json(body): Json<BeginOnlineForkBody>,
) -> Result<Json<BeginOnlineForkResponse>, ApiError> {
    if !state.platform_config.forking.enabled {
        return Err(ApiError::not_implemented(
            "forking is disabled by the deployment configuration",
        ));
    }

    let max_size = state.platform_config.forking.max_size_bytes;
    let max_count = state.platform_config.forking.max_file_count;
    if body.summary.total_size_bytes > max_size {
        return Err(ApiError::bad_request(format!(
            "bundle exceeds the deployment's fork size cap ({} bytes > {} bytes)",
            body.summary.total_size_bytes, max_size
        )));
    }
    if body.summary.total_object_count > max_count {
        return Err(ApiError::bad_request(format!(
            "bundle exceeds the deployment's fork file-count cap ({} > {})",
            body.summary.total_object_count, max_count
        )));
    }

    let sub = user.sub()?;
    let source_app_id = body.source_app_id.clone();
    let language = body.language.clone().unwrap_or_else(|| "en".to_string());

    // The fork job is the "upload in progress" marker: its `allocate`
    // step materializes the hidden destination app (`Offline` /
    // `Inactive`), the Owner / Admin / User roles and the caller's
    // membership. Finalize flips the app to `Private` / `Active` and
    // completes the job; an upload that never finalizes is aborted by the
    // job sweeper once it expires.
    let fork_job = job::enqueue(
        &state,
        source_app_id.as_deref().unwrap_or(job::OFFLINE_SOURCE),
        &sub,
        ForkJobSpec::offline_upload(&language),
    )
    .await?;
    let (fork_job, _) = job::run_pass(&state, fork_job).await?;
    let new_app_id = fork_job.dest_app_id.clone();

    // The follow-up sync uses the regular app-edit endpoints. Those
    // endpoints load `manifest.app`, so the hidden destination needs a
    // minimal on-disk app before the desktop starts pushing boards,
    // pages, widgets, templates and events.
    let bootstrap_result = async {
        let credentials = state.master_credentials().await?;
        let flow_like_state = Arc::new(credentials.to_state(state.clone()).await?);
        let metadata = Metadata {
            name: "Forked app".to_string(),
            ..Default::default()
        };
        let mut drive_app = App::new(
            Some(new_app_id.clone()),
            metadata,
            Vec::new(),
            flow_like_state.clone(),
        )
        .await?;
        drive_app.status = CoreAppStatus::Inactive;
        drive_app.forked_from = source_app_id;
        drive_app.forked_at = Some(std::time::SystemTime::now());
        drive_app.save().await?;
        App::load(new_app_id.clone(), flow_like_state).await?;
        Ok::<(), ApiError>(())
    }
    .await;
    if let Err(err) = bootstrap_result {
        if let Err(abort_err) = job::abort(&state, &fork_job).await {
            tracing::warn!(
                app_id = %new_app_id,
                error = %abort_err,
                "failed to clean up after offline-to-online fork allocation bootstrap error"
            );
        }
        return Err(err);
    }

    // Issue **content-only** scoped credentials so the desktop can
    // upload metadata/, media/, upload/, storage/ directly to the destination
    // prefix without putting the API server in the data path. Boards
    // / events / widgets / templates / pages live in the meta bucket
    // and must be pushed via the normal app-edit endpoints — that
    // path runs role-permission gates and per-resource validation
    // (event-schedule checks, page-event coupling, sink registration,
    // secret stripping on write). Granting `EditApp` here would let a
    // misbehaving desktop drop arbitrary `.board` / `.event` files
    // server-side and bypass every guard.
    let scoped =
        RuntimeCredentials::scoped(&sub, &new_app_id, &state, CredentialsAccess::EditAppContent)
            .await
            .map_err(|e| {
                tracing::error!("Failed to generate scoped credentials for fork: {}", e);
                ApiError::internal("Failed to generate fork upload credentials")
            })?;

    let shared_credentials = serde_json::to_value(scoped.clone().into_shared_credentials())
        .map_err(|e| {
            tracing::error!("Failed to serialize shared credentials for fork: {}", e);
            ApiError::internal("Failed to serialize shared credentials")
        })?;

    let expiration = credentials_expiration(&scoped);

    let upload_path = format!("apps/{}", new_app_id);
    Ok(Json(BeginOnlineForkResponse {
        new_app_id,
        fork_session_id: fork_job.id,
        upload_path,
        shared_credentials,
        expiration,
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
