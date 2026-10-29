//! Locked (write-once) files. The same actions exist twice: under the app for
//! users, PATs and API keys, and under `/execution` for a running flow's executor.

use crate::error::ApiError;
use crate::locked_storage::{
    self as locked, Caller, CreateLockedUploadRequest, DownloadLockedItemRequest,
    ExtendLockedRetentionRequest, GetLockedItemRequest, ListLockedItemsRequest,
    ListLockedItemsResponse, LockedDownload, LockedEmpty, LockedItemDetail, LockedItemVersion,
    LockedStorageStatus, LockedUploadTicket, LockedVersionRef, PurgeLockedItemRequest,
    SetLockedAutoDeleteRequest, SetLockedLegalHoldRequest,
};
use crate::middleware::jwt::{AppUser, ExecutorUse};
use crate::state::AppState;
use axum::{
    Extension, Json, Router,
    extract::{Path, State},
    routing::{get, post},
};

const CHANGE_LOCKED_FILES: ExecutorUse = ExecutorUse::Write("change locked files");

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/status", get(status))
        .route("/list", post(list))
        .route("/get", post(get_item))
        .route("/upload", post(upload))
        .route("/commit", post(commit))
        .route("/abort", post(abort))
        .route("/download", post(download))
        .route("/retention", post(extend_retention))
        .route("/legal-hold", post(set_legal_hold))
        .route("/auto-delete", post(set_auto_delete))
        .route("/purge", post(purge))
}

/// Mounted at `/execution/apps/{app_id}/locked`; every path is allowlisted in the
/// execution gateway (`apps/backend/execution-manager/src/gateway/policy.rs`).
pub fn executor_routes() -> Router<AppState> {
    Router::new()
        .route("/status", get(executor_status))
        .route("/list", post(executor_list))
        .route("/get", post(executor_get_item))
        .route("/upload", post(executor_upload))
        .route("/commit", post(executor_commit))
        .route("/abort", post(executor_abort))
        .route("/download", post(executor_download))
        .route("/retention", post(executor_extend_retention))
        .route("/legal-hold", post(executor_set_legal_hold))
        .route("/auto-delete", post(executor_set_auto_delete))
        .route("/purge", post(executor_purge))
}

#[utoipa::path(
    get, path = "/apps/{app_id}/locked/status", tag = "locked-storage",
    description = "Whether this server offers locked files and with which retention rules. Any app member may ask.",
    params(("app_id" = String, Path, description = "Application ID")),
    responses((status = 200, description = "Locked storage capabilities", body = LockedStorageStatus),
        (status = 403, description = "Not a member of this app")),
    security(("bearer_auth" = []), ("api_key" = []), ("pat" = []))
)]
pub async fn status(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(app_id): Path<String>,
) -> Result<Json<LockedStorageStatus>, ApiError> {
    Caller::for_user(&state, &user, &app_id).await?;
    Ok(Json(locked::status(&state)))
}

#[utoipa::path(
    post, path = "/apps/{app_id}/locked/list", tag = "locked-storage",
    description = "List locked files in a folder, with the newest stored version of each and the subfolders. Personal files are only listed for their owner.",
    params(("app_id" = String, Path, description = "Application ID")),
    request_body = ListLockedItemsRequest,
    responses((status = 200, description = "Locked files", body = ListLockedItemsResponse),
        (status = 403, description = "Missing ReadFiles permission"),
        (status = 503, description = "Locked storage is not configured")),
    security(("bearer_auth" = []), ("api_key" = []), ("pat" = []))
)]
pub async fn list(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(app_id): Path<String>,
    Json(request): Json<ListLockedItemsRequest>,
) -> Result<Json<ListLockedItemsResponse>, ApiError> {
    let caller = Caller::for_user(&state, &user, &app_id).await?;
    Ok(Json(locked::list(&state, &caller, &app_id, request).await?))
}

#[utoipa::path(
    post, path = "/apps/{app_id}/locked/get", tag = "locked-storage",
    description = "A locked file with its full version history, by id or by folder and name.",
    params(("app_id" = String, Path, description = "Application ID")),
    request_body = GetLockedItemRequest,
    responses((status = 200, description = "The locked file and its versions", body = LockedItemDetail),
        (status = 404, description = "No such locked file")),
    security(("bearer_auth" = []), ("api_key" = []), ("pat" = []))
)]
pub async fn get_item(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(app_id): Path<String>,
    Json(request): Json<GetLockedItemRequest>,
) -> Result<Json<LockedItemDetail>, ApiError> {
    let caller = Caller::for_user(&state, &user, &app_id).await?;
    Ok(Json(locked::get(&state, &caller, &app_id, request).await?))
}

#[utoipa::path(
    post, path = "/apps/{app_id}/locked/upload", tag = "locked-storage",
    description = "Start uploading a new locked file or a new version of one. Returns a signed upload URL for exactly these bytes; send the returned headers unchanged, then commit. The size counts against the app owner's storage.",
    params(("app_id" = String, Path, description = "Application ID")),
    request_body = CreateLockedUploadRequest,
    responses((status = 200, description = "Upload ticket", body = LockedUploadTicket),
        (status = 409, description = "A locked file with this name already exists"),
        (status = 422, description = "Invalid name, checksum, size or retention"),
        (status = 402, description = "Storage is full")),
    security(("bearer_auth" = []), ("api_key" = []), ("pat" = []))
)]
pub async fn upload(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(app_id): Path<String>,
    Json(request): Json<CreateLockedUploadRequest>,
) -> Result<Json<LockedUploadTicket>, ApiError> {
    let caller = Caller::for_user(&state, &user, &app_id).await?;
    Ok(Json(
        locked::upload(&state, &caller, &app_id, request).await?,
    ))
}

#[utoipa::path(
    post, path = "/apps/{app_id}/locked/commit", tag = "locked-storage",
    description = "Finish an upload: the server checks size and checksum, locks the bytes for the retention period and numbers the version. Safe to repeat.",
    params(("app_id" = String, Path, description = "Application ID")),
    request_body = LockedVersionRef,
    responses((status = 200, description = "The locked file with the new version", body = LockedItemDetail),
        (status = 412, description = "The upload is missing or does not match its checksum")),
    security(("bearer_auth" = []), ("api_key" = []), ("pat" = []))
)]
pub async fn commit(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(app_id): Path<String>,
    Json(request): Json<LockedVersionRef>,
) -> Result<Json<LockedItemDetail>, ApiError> {
    let caller = Caller::for_user(&state, &user, &app_id).await?;
    Ok(Json(
        locked::commit(&state, &caller, &app_id, request).await?,
    ))
}

#[utoipa::path(
    post, path = "/apps/{app_id}/locked/abort", tag = "locked-storage",
    description = "Cancel an upload that was not committed and delete its bytes.",
    params(("app_id" = String, Path, description = "Application ID")),
    request_body = LockedVersionRef,
    responses((status = 200, description = "Upload discarded", body = LockedEmpty),
        (status = 409, description = "Already committed, or the bucket already retains the bytes")),
    security(("bearer_auth" = []), ("api_key" = []), ("pat" = []))
)]
pub async fn abort(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(app_id): Path<String>,
    Json(request): Json<LockedVersionRef>,
) -> Result<Json<LockedEmpty>, ApiError> {
    let caller = Caller::for_user(&state, &user, &app_id).await?;
    Ok(Json(
        locked::abort(&state, &caller, &app_id, request).await?,
    ))
}

#[utoipa::path(
    post, path = "/apps/{app_id}/locked/download", tag = "locked-storage",
    description = "A short-lived download link for one stored version (the newest when no version is given).",
    params(("app_id" = String, Path, description = "Application ID")),
    request_body = DownloadLockedItemRequest,
    responses((status = 200, description = "Signed download link", body = LockedDownload),
        (status = 404, description = "No such stored version")),
    security(("bearer_auth" = []), ("api_key" = []), ("pat" = []))
)]
pub async fn download(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(app_id): Path<String>,
    Json(request): Json<DownloadLockedItemRequest>,
) -> Result<Json<LockedDownload>, ApiError> {
    let caller = Caller::for_user(&state, &user, &app_id).await?;
    Ok(Json(
        locked::download(&state, &caller, &app_id, request).await?,
    ))
}

#[utoipa::path(
    post, path = "/apps/{app_id}/locked/retention", tag = "locked-storage",
    description = "Extend how long a version stays locked. Retention can never be shortened. Admins only.",
    params(("app_id" = String, Path, description = "Application ID")),
    request_body = ExtendLockedRetentionRequest,
    responses((status = 200, description = "The updated version", body = LockedItemVersion),
        (status = 409, description = "Not later than the current retention, or retention is set by the bucket")),
    security(("bearer_auth" = []), ("api_key" = []), ("pat" = []))
)]
pub async fn extend_retention(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(app_id): Path<String>,
    Json(request): Json<ExtendLockedRetentionRequest>,
) -> Result<Json<LockedItemVersion>, ApiError> {
    let caller = Caller::for_user(&state, &user, &app_id).await?;
    Ok(Json(
        locked::extend_retention(&state, &caller, &app_id, request).await?,
    ))
}

#[utoipa::path(
    post, path = "/apps/{app_id}/locked/legal-hold", tag = "locked-storage",
    description = "Place or lift a legal hold. A held version cannot be deleted even after its retention ends. Admins only.",
    params(("app_id" = String, Path, description = "Application ID")),
    request_body = SetLockedLegalHoldRequest,
    responses((status = 200, description = "The updated version", body = LockedItemVersion),
        (status = 409, description = "This storage provider has no legal holds")),
    security(("bearer_auth" = []), ("api_key" = []), ("pat" = []))
)]
pub async fn set_legal_hold(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(app_id): Path<String>,
    Json(request): Json<SetLockedLegalHoldRequest>,
) -> Result<Json<LockedItemVersion>, ApiError> {
    let caller = Caller::for_user(&state, &user, &app_id).await?;
    Ok(Json(
        locked::set_legal_hold(&state, &caller, &app_id, request).await?,
    ))
}

#[utoipa::path(
    post, path = "/apps/{app_id}/locked/auto-delete", tag = "locked-storage",
    description = "Choose whether a version is deleted automatically once its retention ends and no legal hold remains. The retention itself does not change. Owners of personal files and admins of project files only.",
    params(("app_id" = String, Path, description = "Application ID")),
    request_body = SetLockedAutoDeleteRequest,
    responses((status = 200, description = "The updated version", body = LockedItemVersion),
        (status = 409, description = "The version is not a stored, committed version")),
    security(("bearer_auth" = []), ("api_key" = []), ("pat" = []))
)]
pub async fn set_auto_delete(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(app_id): Path<String>,
    Json(request): Json<SetLockedAutoDeleteRequest>,
) -> Result<Json<LockedItemVersion>, ApiError> {
    let caller = Caller::for_user(&state, &user, &app_id).await?;
    Ok(Json(
        locked::set_auto_delete(&state, &caller, &app_id, request).await?,
    ))
}

#[utoipa::path(
    post, path = "/apps/{app_id}/locked/purge", tag = "locked-storage",
    description = "Delete the stored bytes of versions whose retention has ended and frees their storage. The record of each version stays. Owners for personal files, admins for project files.",
    params(("app_id" = String, Path, description = "Application ID")),
    request_body = PurgeLockedItemRequest,
    responses((status = 200, description = "The locked file after purging", body = LockedItemDetail),
        (status = 409, description = "A version is still retained or under legal hold")),
    security(("bearer_auth" = []), ("api_key" = []), ("pat" = []))
)]
pub async fn purge(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(app_id): Path<String>,
    Json(request): Json<PurgeLockedItemRequest>,
) -> Result<Json<LockedItemDetail>, ApiError> {
    let caller = Caller::for_user(&state, &user, &app_id).await?;
    Ok(Json(
        locked::purge(&state, &caller, &app_id, request).await?,
    ))
}

#[utoipa::path(
    get, path = "/execution/apps/{app_id}/locked/status", tag = "execution",
    description = "Locked storage capabilities for the running flow's app. Executor only.",
    params(("app_id" = String, Path, description = "Application ID")),
    responses((status = 200, description = "Locked storage capabilities", body = LockedStorageStatus)),
    security(("executor_jwt" = []))
)]
pub async fn executor_status(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(app_id): Path<String>,
) -> Result<Json<LockedStorageStatus>, ApiError> {
    Caller::for_executor(&state, &user, &app_id, ExecutorUse::Read).await?;
    Ok(Json(locked::status(&state)))
}

#[utoipa::path(
    post, path = "/execution/apps/{app_id}/locked/list", tag = "execution",
    description = "List locked files for the running flow, as the user it runs for. Executor only.",
    params(("app_id" = String, Path, description = "Application ID")),
    request_body = ListLockedItemsRequest,
    responses((status = 200, description = "Locked files", body = ListLockedItemsResponse)),
    security(("executor_jwt" = []))
)]
pub async fn executor_list(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(app_id): Path<String>,
    Json(request): Json<ListLockedItemsRequest>,
) -> Result<Json<ListLockedItemsResponse>, ApiError> {
    let caller = Caller::for_executor(&state, &user, &app_id, ExecutorUse::Read).await?;
    Ok(Json(locked::list(&state, &caller, &app_id, request).await?))
}

#[utoipa::path(
    post, path = "/execution/apps/{app_id}/locked/get", tag = "execution",
    description = "A locked file with its version history. Executor only.",
    params(("app_id" = String, Path, description = "Application ID")),
    request_body = GetLockedItemRequest,
    responses((status = 200, description = "The locked file and its versions", body = LockedItemDetail)),
    security(("executor_jwt" = []))
)]
pub async fn executor_get_item(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(app_id): Path<String>,
    Json(request): Json<GetLockedItemRequest>,
) -> Result<Json<LockedItemDetail>, ApiError> {
    let caller = Caller::for_executor(&state, &user, &app_id, ExecutorUse::Read).await?;
    Ok(Json(locked::get(&state, &caller, &app_id, request).await?))
}

#[utoipa::path(
    post, path = "/execution/apps/{app_id}/locked/upload", tag = "execution",
    description = "Start a locked upload from a running flow. Executor only.",
    params(("app_id" = String, Path, description = "Application ID")),
    request_body = CreateLockedUploadRequest,
    responses((status = 200, description = "Upload ticket", body = LockedUploadTicket),
        (status = 403, description = "Forbidden: the run's role lacks the permission, or the run is a regression replay")),
    security(("executor_jwt" = []))
)]
pub async fn executor_upload(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(app_id): Path<String>,
    Json(request): Json<CreateLockedUploadRequest>,
) -> Result<Json<LockedUploadTicket>, ApiError> {
    let caller = Caller::for_executor(&state, &user, &app_id, CHANGE_LOCKED_FILES).await?;
    Ok(Json(
        locked::upload(&state, &caller, &app_id, request).await?,
    ))
}

#[utoipa::path(
    post, path = "/execution/apps/{app_id}/locked/commit", tag = "execution",
    description = "Finish a locked upload from a running flow. Executor only.",
    params(("app_id" = String, Path, description = "Application ID")),
    request_body = LockedVersionRef,
    responses((status = 200, description = "The locked file with the new version", body = LockedItemDetail),
        (status = 403, description = "Forbidden: the run's role lacks the permission, or the run is a regression replay")),
    security(("executor_jwt" = []))
)]
pub async fn executor_commit(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(app_id): Path<String>,
    Json(request): Json<LockedVersionRef>,
) -> Result<Json<LockedItemDetail>, ApiError> {
    let caller = Caller::for_executor(&state, &user, &app_id, CHANGE_LOCKED_FILES).await?;
    Ok(Json(
        locked::commit(&state, &caller, &app_id, request).await?,
    ))
}

#[utoipa::path(
    post, path = "/execution/apps/{app_id}/locked/abort", tag = "execution",
    description = "Cancel an uncommitted locked upload. Executor only.",
    params(("app_id" = String, Path, description = "Application ID")),
    request_body = LockedVersionRef,
    responses((status = 200, description = "Upload discarded", body = LockedEmpty),
        (status = 403, description = "Forbidden: the run's role lacks the permission, or the run is a regression replay")),
    security(("executor_jwt" = []))
)]
pub async fn executor_abort(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(app_id): Path<String>,
    Json(request): Json<LockedVersionRef>,
) -> Result<Json<LockedEmpty>, ApiError> {
    let caller = Caller::for_executor(&state, &user, &app_id, CHANGE_LOCKED_FILES).await?;
    Ok(Json(
        locked::abort(&state, &caller, &app_id, request).await?,
    ))
}

#[utoipa::path(
    post, path = "/execution/apps/{app_id}/locked/download", tag = "execution",
    description = "A short-lived download link for one stored version. Executor only.",
    params(("app_id" = String, Path, description = "Application ID")),
    request_body = DownloadLockedItemRequest,
    responses((status = 200, description = "Signed download link", body = LockedDownload)),
    security(("executor_jwt" = []))
)]
pub async fn executor_download(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(app_id): Path<String>,
    Json(request): Json<DownloadLockedItemRequest>,
) -> Result<Json<LockedDownload>, ApiError> {
    let caller = Caller::for_executor(&state, &user, &app_id, ExecutorUse::Read).await?;
    Ok(Json(
        locked::download(&state, &caller, &app_id, request).await?,
    ))
}

#[utoipa::path(
    post, path = "/execution/apps/{app_id}/locked/retention", tag = "execution",
    description = "Extend a version's retention when the run's user is an admin. Executor only.",
    params(("app_id" = String, Path, description = "Application ID")),
    request_body = ExtendLockedRetentionRequest,
    responses((status = 200, description = "The updated version", body = LockedItemVersion),
        (status = 403, description = "Forbidden: the run's role lacks the permission, or the run is a regression replay")),
    security(("executor_jwt" = []))
)]
pub async fn executor_extend_retention(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(app_id): Path<String>,
    Json(request): Json<ExtendLockedRetentionRequest>,
) -> Result<Json<LockedItemVersion>, ApiError> {
    let caller = Caller::for_executor(&state, &user, &app_id, CHANGE_LOCKED_FILES).await?;
    Ok(Json(
        locked::extend_retention(&state, &caller, &app_id, request).await?,
    ))
}

#[utoipa::path(
    post, path = "/execution/apps/{app_id}/locked/legal-hold", tag = "execution",
    description = "Place or lift a legal hold when the run's user is an admin. Executor only.",
    params(("app_id" = String, Path, description = "Application ID")),
    request_body = SetLockedLegalHoldRequest,
    responses((status = 200, description = "The updated version", body = LockedItemVersion),
        (status = 403, description = "Forbidden: the run's role lacks the permission, or the run is a regression replay")),
    security(("executor_jwt" = []))
)]
pub async fn executor_set_legal_hold(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(app_id): Path<String>,
    Json(request): Json<SetLockedLegalHoldRequest>,
) -> Result<Json<LockedItemVersion>, ApiError> {
    let caller = Caller::for_executor(&state, &user, &app_id, CHANGE_LOCKED_FILES).await?;
    Ok(Json(
        locked::set_legal_hold(&state, &caller, &app_id, request).await?,
    ))
}

#[utoipa::path(
    post, path = "/execution/apps/{app_id}/locked/auto-delete", tag = "execution",
    description = "Choose whether a version is deleted automatically once its retention ends, when the run's user may delete the file. Executor only.",
    params(("app_id" = String, Path, description = "Application ID")),
    request_body = SetLockedAutoDeleteRequest,
    responses((status = 200, description = "The updated version", body = LockedItemVersion),
        (status = 403, description = "Forbidden: the run's role lacks the permission, or the run is a regression replay")),
    security(("executor_jwt" = []))
)]
pub async fn executor_set_auto_delete(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(app_id): Path<String>,
    Json(request): Json<SetLockedAutoDeleteRequest>,
) -> Result<Json<LockedItemVersion>, ApiError> {
    let caller = Caller::for_executor(&state, &user, &app_id, CHANGE_LOCKED_FILES).await?;
    Ok(Json(
        locked::set_auto_delete(&state, &caller, &app_id, request).await?,
    ))
}

#[utoipa::path(
    post, path = "/execution/apps/{app_id}/locked/purge", tag = "execution",
    description = "Delete versions whose retention has ended. Executor only.",
    params(("app_id" = String, Path, description = "Application ID")),
    request_body = PurgeLockedItemRequest,
    responses((status = 200, description = "The locked file after purging", body = LockedItemDetail),
        (status = 403, description = "Forbidden: the run's role lacks the permission, or the run is a regression replay")),
    security(("executor_jwt" = []))
)]
pub async fn executor_purge(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(app_id): Path<String>,
    Json(request): Json<PurgeLockedItemRequest>,
) -> Result<Json<LockedItemDetail>, ApiError> {
    let caller = Caller::for_executor(&state, &user, &app_id, CHANGE_LOCKED_FILES).await?;
    Ok(Json(
        locked::purge(&state, &caller, &app_id, request).await?,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn handler_body<'a>(source: &'a str, handler: &str) -> &'a str {
        let body = source
            .split_once(&format!("\npub async fn {handler}("))
            .unwrap_or_else(|| panic!("{handler} exists"))
            .1;
        body.split_once("\n}\n").map_or(body, |(body, _)| body)
    }

    #[test]
    fn each_executor_handler_declares_whether_it_changes_locked_files() {
        let source = include_str!("locked.rs");
        let read = "Caller::for_executor(&state, &user, &app_id, ExecutorUse::Read)";
        let write = "Caller::for_executor(&state, &user, &app_id, CHANGE_LOCKED_FILES)";
        for handler in [
            "executor_status",
            "executor_list",
            "executor_get_item",
            "executor_download",
        ] {
            let body = handler_body(source, handler);
            assert!(body.contains(read), "{handler} only reads");
            assert!(!body.contains(write), "{handler} only reads");
        }
        for handler in [
            "executor_upload",
            "executor_commit",
            "executor_abort",
            "executor_extend_retention",
            "executor_set_legal_hold",
            "executor_set_auto_delete",
            "executor_purge",
        ] {
            let body = handler_body(source, handler);
            assert!(body.contains(write), "{handler} changes locked files");
            assert!(!body.contains(read), "{handler} changes locked files");
        }
        assert_eq!(
            CHANGE_LOCKED_FILES,
            ExecutorUse::Write("change locked files")
        );
        let handlers = source
            .split_once("#[cfg(test)]")
            .map_or(source, |(handlers, _)| handlers);
        assert_eq!(
            handlers.matches("Caller::for_executor(").count(),
            11,
            "every executor handler is classified above"
        );
    }
}
