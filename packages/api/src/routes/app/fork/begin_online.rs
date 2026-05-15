use std::sync::Arc;

use crate::{
    credentials::{CredentialsAccess, RuntimeCredentials},
    entity::{
        app, membership, role,
        sea_orm_active_enums::{ExecutionMode, Status, Visibility},
    },
    error::ApiError,
    middleware::jwt::AppUser,
    permission::role_permission::RolePermissions,
    state::AppState,
};
use axum::{Extension, Json, extract::State};
use flow_like::{
    app::{App, AppStatus as CoreAppStatus},
    bit::Metadata,
};
use flow_like_storage::Path;
use flow_like_types::create_id;
use futures_util::TryStreamExt;
use sea_orm::{
    ActiveModelTrait,
    ActiveValue::{NotSet, Set},
    EntityTrait, IntoActiveModel, TransactionTrait,
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
    /// Opaque session id for the upload + finalize flow. Pass back
    /// to `POST /apps/fork/online/{session_id}/finalize`.
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
    let new_app_id = create_id();
    let now = chrono::Utc::now().naive_utc();
    let source_app_id = body.source_app_id.clone();

    // Materialize an empty destination app + Owner/Admin/Member
    // roles + caller membership, all in one transaction. The app
    // starts in `Offline` visibility / `Inactive` status so it is
    // hidden from listings; finalize flips it to `Private` / `Active`
    // once the upload is verified.
    let new_app_id_db = new_app_id.clone();
    let sub_owned = sub.clone();
    let source_app_id_owned = source_app_id.clone();
    state
        .db
        .transaction::<_, (), sea_orm::DbErr>(|txn| {
            Box::pin(async move {
                let new_app = app::ActiveModel {
                    id: Set(new_app_id_db.clone()),
                    status: Set(Status::Inactive),
                    visibility: Set(Visibility::Offline),
                    changelog: Set(None),
                    default_role_id: NotSet,
                    owner_role_id: NotSet,
                    primary_category: Set(None),
                    secondary_category: Set(None),
                    rating_sum: Set(0),
                    rating_count: Set(0),
                    download_count: Set(0),
                    interactions_count: Set(0),
                    avg_rating: Set(None),
                    relevance_score: Set(None),
                    total_size: Set(0),
                    price: Set(0),
                    version: Set(None),
                    execution_mode: Set(ExecutionMode::Any),
                    bits: Set(Some(Vec::new())),
                    allow_forking: Set(false),
                    forked_from: Set(source_app_id_owned),
                    forked_at: Set(Some(now)),
                    created_at: Set(now),
                    updated_at: Set(now),
                };
                let inserted_app = new_app.insert(txn).await?;

                let owner_role = role::ActiveModel {
                    id: Set(create_id()),
                    name: Set("Owner".to_string()),
                    description: Set(Some("Owner role".to_string())),
                    permissions: Set(RolePermissions::Owner.bits()),
                    app_id: Set(Some(new_app_id_db.clone())),
                    attributes: NotSet,
                    created_at: Set(now),
                    updated_at: Set(now),
                };
                let owner_role = owner_role.insert(txn).await?;

                let admin_role = role::ActiveModel {
                    id: Set(create_id()),
                    name: Set("Admin".to_string()),
                    description: Set(Some("Admin role".to_string())),
                    permissions: Set(RolePermissions::Admin.bits()),
                    app_id: Set(Some(new_app_id_db.clone())),
                    attributes: NotSet,
                    created_at: Set(now),
                    updated_at: Set(now),
                };
                admin_role.insert(txn).await?;

                let mut member_perms = RolePermissions::ReadTemplates;
                member_perms.insert(RolePermissions::ExecuteEvents);
                member_perms.insert(RolePermissions::ListEvents);
                let member_role = role::ActiveModel {
                    id: Set(create_id()),
                    name: Set("User".to_string()),
                    description: Set(Some("User role".to_string())),
                    permissions: Set(member_perms.bits()),
                    app_id: Set(Some(new_app_id_db.clone())),
                    attributes: NotSet,
                    created_at: Set(now),
                    updated_at: Set(now),
                };
                let member_role = member_role.insert(txn).await?;

                let mut active_app = inserted_app.into_active_model();
                active_app.owner_role_id = Set(Some(owner_role.id.clone()));
                active_app.default_role_id = Set(Some(member_role.id.clone()));
                active_app.update(txn).await?;

                let mship = membership::ActiveModel {
                    id: Set(create_id()),
                    user_id: Set(sub_owned.clone()),
                    app_id: Set(new_app_id_db.clone()),
                    role_id: Set(owner_role.id.clone()),
                    joined_via: NotSet,
                    created_at: Set(now),
                    updated_at: Set(now),
                };
                mship.insert(txn).await?;
                Ok(())
            })
        })
        .await
        .map_err(|e| match e {
            sea_orm::TransactionError::Connection(err) => ApiError::from(err),
            sea_orm::TransactionError::Transaction(err) => ApiError::from(err),
        })?;

    // The follow-up sync uses the regular app-edit endpoints. Those
    // endpoints load `manifest.app`, so the hidden destination needs a
    // minimal on-disk app before the desktop starts pushing boards,
    // pages, widgets, templates and events.
    let bootstrap_result = async {
        let credentials = state.master_credentials().await?;
        let flow_like_state = Arc::new(credentials.to_state(state.clone()).await?);
        let mut metadata = Metadata::default();
        metadata.name = "Forked app".to_string();
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
        cleanup_failed_online_fork_allocation(&state, &new_app_id).await;
        return Err(err);
    }

    // Issue **content-only** scoped credentials so the desktop can
    // upload metadata/, upload/, storage/ directly to the destination
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
        fork_session_id: create_id(),
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

async fn cleanup_failed_online_fork_allocation(state: &AppState, app_id: &str) {
    if let Err(err) = delete_app_storage_prefixes(state, app_id).await {
        tracing::warn!(
            app_id,
            error = %err,
            "failed to clean storage after offline-to-online fork allocation bootstrap error"
        );
    }

    if let Err(err) = app::Entity::delete_by_id(app_id).exec(&state.db).await {
        tracing::warn!(
            app_id,
            error = %err,
            "failed to clean database row after offline-to-online fork allocation bootstrap error"
        );
    }
}

async fn delete_app_storage_prefixes(state: &AppState, app_id: &str) -> Result<(), ApiError> {
    let credentials = state.master_credentials().await?;
    let prefix = Path::from("apps").child(app_id.to_string());

    for meta_store in [true, false] {
        let store = credentials.to_store(meta_store).await?.as_generic();
        let locations: Vec<Path> = store
            .list(Some(&prefix))
            .map_ok(|m| m.location)
            .try_collect()
            .await
            .map_err(|err| ApiError::internal(format!("list app storage prefix: {err}")))?;

        for location in locations {
            store
                .delete(&location)
                .await
                .map_err(|err| ApiError::internal(format!("delete app storage object: {err}")))?;
        }
    }

    Ok(())
}
