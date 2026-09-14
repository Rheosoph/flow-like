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
use sea_orm::{ConnectionTrait, Statement};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

#[derive(Clone, Debug, Default, Deserialize, Serialize, ToSchema)]
pub struct BeginOnlineForkBody {
    /// Source lineage reported by Studio. Project-limit exemption requires
    /// server verification of public access, purchase, or retained fork proof.
    /// A project created only on the desktop may have no cloud source.
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
    pub project_limit_exempt: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project_limit_notice: Option<String>,
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
    let bytes = i64::try_from(body.summary.total_size_bytes)
        .map_err(|_| ApiError::bad_request("The fork is too large."))?;
    crate::capacity::check_storage_write(&state, "", &sub, bytes).await?;
    let source_app_id = body.source_app_id.clone();
    let language = body.language.clone().unwrap_or_else(|| "en".to_string());
    let mut spec = ForkJobSpec::offline_upload(&language);
    if let Some(source_id) = source_app_id.as_deref() {
        spec.public_fork_source_id = verified_upload_source(&state, &user, &sub, source_id).await?;
    }
    let project_limit_exempt = spec.public_fork_source_id.is_some();
    let project_limit_notice = (source_app_id.is_some() && !project_limit_exempt).then(|| {
        "The public or purchased source could not be verified. This upload uses a project slot."
            .to_owned()
    });

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
        spec,
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
        project_limit_exempt,
        project_limit_notice,
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

async fn verified_upload_source(
    state: &AppState,
    user: &AppUser,
    payer: &str,
    source_id: &str,
) -> Result<Option<String>, ApiError> {
    // A previous server-owned fork allocation proves this user's origin even after
    // the original public source was retired. The uploaded manifest is not evidence.
    if let Some(row) = state.db.query_one_raw(Statement::from_sql_and_values(state.db.get_database_backend(),
        r#"SELECT "publicForkSourceId" FROM "ProjectCapacity" WHERE ("appId" = $1 OR "publicForkSourceId" = $1) AND "payerId" = $2 AND "publicForkSourceId" IS NOT NULL LIMIT 1"#, [source_id.into(),payer.into()])).await? {
        return Ok(row.try_get("", "publicForkSourceId")?);
    }
    let source = match crate::permission::fork_permission::check_can_fork(
        user,
        source_id,
        state,
        crate::permission::fork_permission::ForkTargetKind::Online,
    )
    .await
    {
        Ok(source) => source,
        Err(error) => {
            let error = ApiError::from(error);
            if matches!(
                error.status(),
                axum::http::StatusCode::NOT_FOUND | axum::http::StatusCode::FORBIDDEN
            ) {
                return Ok(None);
            }
            return Err(error);
        }
    };
    if let Some(source) = crate::capacity::public_fork_source(&source.visibility, &source.id) {
        return Ok(Some(source));
    }
    let purchased = state.db.query_one_raw(Statement::from_sql_and_values(state.db.get_database_backend(),
        r#"SELECT "id" FROM "AppPurchase" WHERE "appId" = $1 AND "userId" = $2 AND "status" = 'COMPLETED' AND "completedAt" IS NOT NULL AND "refundedAt" IS NULL LIMIT 1"#, [source_id.into(),payer.into()])).await?.is_some();
    Ok(purchased.then(|| source.id))
}
