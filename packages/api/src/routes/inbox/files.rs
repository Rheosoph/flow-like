//! Conversation files: a file an active member uploads into a conversation, stored by the
//! platform under `conversations/<conversation id>/`, not in any app. The uploader mints the
//! upload (their storage capacity pays), the server verifies and hashes the object when the
//! upload completes, and a message carries it as an attachment whose handle names the
//! conversation (`conversation_<id>`). Members download through short-lived signed URLs; bots in
//! the conversation read the object through their tools. Deleting the message marks its files
//! deleted, and [`sweep`] removes the objects and rows, together with files nobody sent.

use super::{Reader, acting_reader, active_membership, model::ReaderQuery, now_millis};
use crate::{
    capacity,
    entity::{conversation, conversation_file},
    error::ApiError,
    middleware::jwt::AppUser,
    routes::app::meta::sanitize_ext,
    state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, Query, State},
    http::StatusCode,
};
use chrono::{DateTime, Duration as ChronoDuration, FixedOffset};
use flow_like_storage::object_store::{
    self, ObjectStore, ObjectStoreExt, path::Path as ObjectPath,
};
use flow_like_types::{
    bot_message::{
        CONVERSATION_FILE_APP_PREFIX, MAX_CONVERSATION_FILE_BYTES, MAX_FILE_NAME_CHARS,
        WorkspaceHandle, is_mime,
    },
    create_id,
    inbox::ConversationEncryption,
    mime_guess,
};
use futures::StreamExt;
use sea_orm::{
    ActiveModelTrait, ActiveValue::Set, ColumnTrait, Condition, ConnectionTrait, EntityTrait,
    QueryFilter, QueryOrder, QuerySelect, sea_query::Expr,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::time::Duration;
use utoipa::ToSchema;

pub(crate) const ATTACHMENTS_ENCRYPTED_UNSUPPORTED: &str = "ATTACHMENTS_ENCRYPTED_UNSUPPORTED";
pub(crate) const INBOX_FILE_NOT_FOUND: &str = "INBOX_FILE_NOT_FOUND";
pub(crate) const INBOX_FILE_TOO_LARGE: &str = "INBOX_FILE_TOO_LARGE";
pub(crate) const INBOX_FILE_NOT_READY: &str = "INBOX_FILE_NOT_READY";
pub(crate) const INBOX_FILE_TAKEN: &str = "INBOX_FILE_TAKEN";

pub(crate) const FILE_PENDING: &str = "PENDING";
pub(crate) const FILE_READY: &str = "READY";

const UPLOAD_URL_TTL: Duration = Duration::from_secs(15 * 60);
const DOWNLOAD_URL_TTL: Duration = Duration::from_secs(10 * 60);
const UNSENT_GRACE_HOURS: i64 = 24;
const SWEEP_PAGE: u64 = 100;
const OCTET_STREAM: &str = "application/octet-stream";

/// The object key of a conversation file.
pub(crate) fn object_key(conversation_id: &str, file_name: &str) -> String {
    format!("conversations/{conversation_id}/{file_name}")
}

/// The conversation a handle's `app_id` names, when it is a conversation file's.
pub(crate) fn conversation_of_app(app_id: &str) -> Option<&str> {
    app_id.strip_prefix(CONVERSATION_FILE_APP_PREFIX)
}

/// A conversation file a message points at, as the send checks it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ConversationFileUse {
    pub conversation_id: String,
    pub file_id: String,
    pub handle: WorkspaceHandle,
}

#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct MintFileRequest {
    /// The file name shown with the attachment: 1 to 255 characters, no line breaks.
    pub name: String,
    /// `type/subtype`; guessed from the name when omitted.
    pub mime: Option<String>,
    /// The size you are about to upload, 1 byte to 35 MiB.
    pub bytes: u64,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct ConversationFileUpload {
    pub file_id: String,
    /// Upload the bytes here with one `PUT`, then call `complete`.
    pub upload_url: String,
    pub upload_expires_at: DateTime<FixedOffset>,
    pub max_bytes: u64,
}

/// A finished upload: put these fields into the attachment part's `handle`, with the file name in
/// the part's `name`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, ToSchema)]
pub struct ConversationFileReady {
    pub file_id: String,
    pub name: String,
    pub app_id: String,
    pub path: String,
    pub bytes: u64,
    pub sha256: String,
    pub mime: String,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct ConversationFileDownload {
    pub url: String,
    pub expires_at: DateTime<FixedOffset>,
    pub name: String,
    pub mime: String,
    pub bytes: u64,
}

fn file_not_found(file_id: &str) -> ApiError {
    ApiError::coded(
        StatusCode::NOT_FOUND,
        INBOX_FILE_NOT_FOUND,
        format!("File '{file_id}' not found in this conversation"),
    )
}

pub(crate) fn stored_file_name(file: &conversation_file::Model) -> String {
    match extension_of(&file.name) {
        Some(extension) => format!("{}.{extension}", file.id),
        None => file.id.clone(),
    }
}

fn extension_of(name: &str) -> Option<String> {
    sanitize_ext(name.rsplit_once('.').map(|(_, extension)| extension))
}

fn ready_of(file: &conversation_file::Model) -> ConversationFileReady {
    ConversationFileReady {
        file_id: file.id.clone(),
        name: file.name.clone(),
        app_id: format!("{CONVERSATION_FILE_APP_PREFIX}{}", file.conversation_id),
        path: stored_file_name(file),
        bytes: file.bytes.max(0) as u64,
        sha256: file.sha256.clone().unwrap_or_default(),
        mime: file.mime.clone(),
    }
}

/// The name, the type and the size of an upload, checked against the contract's limits.
pub(crate) fn validate_request(request: &MintFileRequest) -> Result<(String, String), ApiError> {
    let name = request.name.trim();
    let length = name.chars().count();
    if !(1..=MAX_FILE_NAME_CHARS).contains(&length) || name.chars().any(char::is_control) {
        return Err(ApiError::bad_request(format!(
            "Upload name must be 1 to {MAX_FILE_NAME_CHARS} characters without line breaks; got {length} characters"
        )));
    }
    if request.bytes == 0 {
        return Err(ApiError::bad_request(format!(
            "File '{name}' is empty; a conversation file has at least 1 byte"
        )));
    }
    if request.bytes > MAX_CONVERSATION_FILE_BYTES {
        return Err(ApiError::coded(
            StatusCode::PAYLOAD_TOO_LARGE,
            INBOX_FILE_TOO_LARGE,
            format!(
                "File '{name}' is {} bytes; a conversation file has at most {MAX_CONVERSATION_FILE_BYTES} bytes",
                request.bytes
            ),
        ));
    }
    let mime = request
        .mime
        .as_deref()
        .map(|mime| {
            mime.split(';')
                .next()
                .unwrap_or_default()
                .trim()
                .to_ascii_lowercase()
        })
        .filter(|mime| is_mime(mime))
        .or_else(|| {
            mime_guess::from_path(name)
                .first()
                .map(|mime| mime.to_string())
        })
        .unwrap_or_else(|| OCTET_STREAM.to_string());
    Ok((name.to_string(), mime))
}

/// Bytes the payer's uploads hold, sent or not: they count against the same capacity.
async fn held_bytes<C: ConnectionTrait>(db: &C, uploader: &str) -> Result<i64, ApiError> {
    let files = conversation_file::Entity::find()
        .select_only()
        .column(conversation_file::Column::Bytes)
        .filter(conversation_file::Column::UploaderId.eq(uploader))
        .filter(conversation_file::Column::DeletedAt.is_null())
        .into_tuple::<i64>()
        .all(db)
        .await?;
    Ok(files
        .into_iter()
        .fold(0i64, |sum, bytes| sum.saturating_add(bytes)))
}

/// The conversation `inbox` is an active member of, in a plain (not end-to-end encrypted) form.
async fn plain_conversation<C: ConnectionTrait>(
    db: &C,
    conversation_id: &str,
    inbox: &str,
) -> Result<conversation::Model, ApiError> {
    active_membership(db, conversation_id, inbox).await?;
    let conversation = conversation::Entity::find_by_id(conversation_id)
        .one(db)
        .await?
        .ok_or_else(|| super::conversation_not_found(conversation_id))?;
    if ConversationEncryption::parse(&conversation.encryption)
        != Some(ConversationEncryption::Plain)
    {
        return Err(ApiError::coded(
            StatusCode::CONFLICT,
            ATTACHMENTS_ENCRYPTED_UNSUPPORTED,
            format!(
                "Conversation '{conversation_id}' is end-to-end encrypted; it can't carry attachments yet"
            ),
        ));
    }
    Ok(conversation)
}

/// Records an upload the member is about to make. The caller checked capacity.
pub(crate) async fn mint_in<C: ConnectionTrait>(
    db: &C,
    conversation_id: &str,
    uploader: &str,
    request: &MintFileRequest,
    now: DateTime<FixedOffset>,
) -> Result<conversation_file::Model, ApiError> {
    plain_conversation(db, conversation_id, uploader).await?;
    let (name, mime) = validate_request(request)?;
    let bytes = i64::try_from(request.bytes)
        .map_err(|_| ApiError::bad_request("Upload size is too large."))?;
    let row = conversation_file::ActiveModel {
        id: Set(create_id()),
        conversation_id: Set(conversation_id.to_string()),
        uploader_id: Set(uploader.to_string()),
        name: Set(name),
        mime: Set(mime),
        bytes: Set(bytes),
        sha256: Set(None),
        status: Set(FILE_PENDING.to_string()),
        message_id: Set(None),
        deleted_at: Set(None),
        created_at: Set(now),
    };
    Ok(row.insert(db).await?)
}

async fn uploaded_by<C: ConnectionTrait>(
    db: &C,
    conversation_id: &str,
    uploader: &str,
    file_id: &str,
) -> Result<conversation_file::Model, ApiError> {
    active_membership(db, conversation_id, uploader).await?;
    conversation_file::Entity::find_by_id(file_id)
        .filter(conversation_file::Column::ConversationId.eq(conversation_id))
        .filter(conversation_file::Column::UploaderId.eq(uploader))
        .filter(conversation_file::Column::DeletedAt.is_null())
        .one(db)
        .await?
        .ok_or_else(|| file_not_found(file_id))
}

/// Checks the uploaded object against what was announced, hashes it and marks the file ready.
pub(crate) async fn complete_in<C: ConnectionTrait>(
    db: &C,
    store: &dyn ObjectStore,
    conversation_id: &str,
    uploader: &str,
    file_id: &str,
) -> Result<ConversationFileReady, ApiError> {
    let file = uploaded_by(db, conversation_id, uploader, file_id).await?;
    if file.status == FILE_READY {
        return Ok(ready_of(&file));
    }
    let location = ObjectPath::from(object_key(conversation_id, &stored_file_name(&file)));
    let size = match store.head(&location).await {
        Ok(meta) => meta.size,
        Err(object_store::Error::NotFound { .. }) => {
            return Err(ApiError::coded(
                StatusCode::CONFLICT,
                INBOX_FILE_NOT_READY,
                format!(
                    "File '{file_id}' has not been uploaded yet; PUT the bytes to its upload url first"
                ),
            ));
        }
        Err(error) => {
            return Err(ApiError::internal(format!(
                "head file '{file_id}': {error}"
            )));
        }
    };
    if size > file.bytes.max(0) as u64 || size > MAX_CONVERSATION_FILE_BYTES {
        mark_deleted(db, &[file.id.clone()], super::now_millis()).await?;
        return Err(ApiError::coded(
            StatusCode::PAYLOAD_TOO_LARGE,
            INBOX_FILE_TOO_LARGE,
            format!(
                "File '{file_id}' holds {size} bytes but {} were announced; the upload was dropped",
                file.bytes
            ),
        ));
    }
    let sha256 = hash_object(store, &location)
        .await
        .map_err(|error| ApiError::internal(format!("hash file '{file_id}': {error}")))?;
    let mut ready: conversation_file::ActiveModel = file.into();
    ready.bytes = Set(size as i64);
    ready.sha256 = Set(Some(sha256));
    ready.status = Set(FILE_READY.to_string());
    Ok(ready_of(&ready.update(db).await?))
}

pub(super) async fn hash_object(
    store: &dyn ObjectStore,
    location: &ObjectPath,
) -> Result<String, object_store::Error> {
    let mut stream = store.get(location).await?.into_stream();
    let mut hasher = Sha256::new();
    while let Some(chunk) = stream.next().await {
        hasher.update(chunk?);
    }
    Ok(hex::encode(hasher.finalize()))
}

/// A ready file in a conversation the reader belongs to. Attached files require a visible
/// message copy, so clearing history also removes that reader's access to its attachments.
pub(crate) async fn readable_file<C: ConnectionTrait>(
    db: &C,
    conversation_id: &str,
    reader: &Reader,
    file_id: &str,
) -> Result<conversation_file::Model, ApiError> {
    let membership = active_membership(db, conversation_id, &reader.inbox).await?;
    let file = conversation_file::Entity::find_by_id(file_id)
        .filter(conversation_file::Column::ConversationId.eq(conversation_id))
        .filter(conversation_file::Column::DeletedAt.is_null())
        .one(db)
        .await?
        .ok_or_else(|| file_not_found(file_id))?;
    if membership.cleared_at.is_some() || file.message_id.is_some() {
        let visible = if let Some(message_id) = &file.message_id {
            crate::entity::inbox_entry::Entity::find()
                .filter(crate::entity::inbox_entry::Column::InboxId.eq(&reader.inbox))
                .filter(crate::entity::inbox_entry::Column::ConversationId.eq(conversation_id))
                .filter(crate::entity::inbox_entry::Column::MessageId.eq(message_id))
                .filter(super::deletion::visible_entries())
                .one(db)
                .await?
                .is_some()
        } else {
            membership
                .cleared_legacy_at
                .is_none_or(|cleared| file.created_at > cleared)
        };
        if !visible {
            return Err(file_not_found(file_id));
        }
    }
    if file.status != FILE_READY {
        return Err(ApiError::coded(
            StatusCode::CONFLICT,
            INBOX_FILE_NOT_READY,
            format!("File '{file_id}' is still being uploaded"),
        ));
    }
    Ok(file)
}

const MAX_VISION_IMAGES: u64 = 4;
/// The largest picture a model call downloads to fit into its provider's inline limit.
const MAX_VISION_IMAGE_BYTES: i64 = 32 * 1024 * 1024;
/// How long the bot turn answering a message reads its pictures, tool rounds included.
const VISION_URL_TTL: Duration = Duration::from_secs(60 * 60);

/// The pictures a message carries, as signed links for a vision model. The model's media handling
/// hands them to a provider that fetches links and inlines them, fitted to its limits, for any
/// other. Only files of `conversation_id` that are ready and not deleted; a file that can't be
/// signed is skipped.
pub(crate) async fn message_images(
    state: &AppState,
    conversation_id: &str,
    message_id: &str,
) -> Vec<flow_like::flow::copilot::ChatImage> {
    let files = conversation_file::Entity::find()
        .filter(conversation_file::Column::ConversationId.eq(conversation_id))
        .filter(conversation_file::Column::MessageId.eq(message_id))
        .filter(conversation_file::Column::Status.eq(FILE_READY))
        .filter(conversation_file::Column::DeletedAt.is_null())
        .filter(conversation_file::Column::Mime.like("image/%"))
        .filter(conversation_file::Column::Bytes.lte(MAX_VISION_IMAGE_BYTES))
        .limit(MAX_VISION_IMAGES)
        .all(&state.db)
        .await
        .unwrap_or_default();
    if files.is_empty() {
        return Vec::new();
    }
    let Ok(master) = state.master_credentials().await else {
        return Vec::new();
    };
    let Ok(store) = master.to_store(false).await else {
        return Vec::new();
    };
    let ttl = master.signing_ttl(VISION_URL_TTL);
    let mut images = Vec::with_capacity(files.len());
    for file in files {
        let location = ObjectPath::from(object_key(conversation_id, &stored_file_name(&file)));
        match store.sign("GET", &location, ttl).await {
            Ok(url) => images.push(flow_like::flow::copilot::ChatImage::link(
                url.to_string(),
                file.mime,
            )),
            Err(error) => {
                tracing::warn!(%error, file_id = %file.id, "Signing a conversation image for a bot turn failed");
            }
        }
    }
    images
}

/// Fails unless `principal` is an active member of the conversation.
pub(crate) async fn ensure_member<C: ConnectionTrait>(
    db: &C,
    conversation_id: &str,
    principal: &str,
) -> Result<(), ApiError> {
    active_membership(db, conversation_id, principal)
        .await
        .map(|_| ())
}

/// The object a bot's tools read: the file's key, after the bot's membership is checked.
pub(crate) async fn bot_readable_key<C: ConnectionTrait>(
    db: &C,
    conversation_id: &str,
    principal: &str,
    path: &str,
) -> Result<String, ApiError> {
    active_membership(db, conversation_id, principal).await?;
    let file_id = path.split_once('.').map_or(path, |(id, _)| id);
    let file = conversation_file::Entity::find_by_id(file_id)
        .filter(conversation_file::Column::ConversationId.eq(conversation_id))
        .filter(conversation_file::Column::DeletedAt.is_null())
        .filter(conversation_file::Column::Status.eq(FILE_READY))
        .one(db)
        .await?
        .ok_or_else(|| file_not_found(file_id))?;
    Ok(object_key(conversation_id, &stored_file_name(&file)))
}

/// Ties the files a message carries to it: each must be a ready file of this conversation that
/// the sender uploaded, with exactly the size, hash and type the handle states, and not carried
/// by another message already.
pub(crate) async fn claim<C: ConnectionTrait>(
    txn: &C,
    conversation_id: &str,
    sender: &str,
    message_id: &str,
    uses: &[ConversationFileUse],
) -> Result<(), ApiError> {
    for used in uses {
        let refused = |reason: String| {
            ApiError::coded(
                StatusCode::FORBIDDEN,
                super::INBOX_HANDLE_NOT_ALLOWED,
                format!(
                    "'{sender}' can't attach conversation file '{}': {reason}",
                    used.file_id
                ),
            )
        };
        if used.conversation_id != conversation_id {
            return Err(refused(format!(
                "it was uploaded to conversation '{}', not to '{conversation_id}'",
                used.conversation_id
            )));
        }
        let file = conversation_file::Entity::find_by_id(used.file_id.as_str())
            .filter(conversation_file::Column::ConversationId.eq(conversation_id))
            .filter(conversation_file::Column::DeletedAt.is_null())
            .one(txn)
            .await?
            .ok_or_else(|| refused("no such file in this conversation".to_string()))?;
        if file.uploader_id != sender {
            return Err(refused("only its uploader attaches it".to_string()));
        }
        if file.status != FILE_READY {
            return Err(ApiError::coded(
                StatusCode::CONFLICT,
                INBOX_FILE_NOT_READY,
                format!(
                    "File '{}' is not uploaded completely; complete the upload before sending",
                    used.file_id
                ),
            ));
        }
        let ready = ready_of(&file);
        let handle = &used.handle;
        if handle.path != ready.path
            || handle.bytes != ready.bytes
            || handle.sha256 != ready.sha256
            || handle.mime != ready.mime
        {
            return Err(refused(
                "the handle's path, bytes, sha256 and mime must be the ones the upload returned"
                    .to_string(),
            ));
        }
        if file
            .message_id
            .as_deref()
            .is_some_and(|owner| owner != message_id)
        {
            return Err(ApiError::coded(
                StatusCode::CONFLICT,
                INBOX_FILE_TAKEN,
                format!(
                    "File '{}' belongs to another message; upload it again",
                    used.file_id
                ),
            ));
        }
        conversation_file::Entity::update_many()
            .col_expr(
                conversation_file::Column::MessageId,
                Expr::value(message_id),
            )
            .filter(conversation_file::Column::Id.eq(used.file_id.as_str()))
            .exec(txn)
            .await?;
    }
    Ok(())
}

async fn mark_deleted<C: ConnectionTrait>(
    db: &C,
    ids: &[String],
    now: DateTime<FixedOffset>,
) -> Result<(), ApiError> {
    conversation_file::Entity::update_many()
        .col_expr(conversation_file::Column::DeletedAt, Expr::value(now))
        .filter(conversation_file::Column::Id.is_in(ids.iter().cloned()))
        .exec(db)
        .await?;
    Ok(())
}

/// A deleted message takes its files with it: they stop being readable at once and [`sweep`]
/// removes the objects.
pub(crate) async fn drop_message_files<C: ConnectionTrait>(
    txn: &C,
    conversation_id: &str,
    message_id: &str,
    now: DateTime<FixedOffset>,
) -> Result<(), ApiError> {
    conversation_file::Entity::update_many()
        .col_expr(conversation_file::Column::DeletedAt, Expr::value(now))
        .filter(conversation_file::Column::ConversationId.eq(conversation_id))
        .filter(conversation_file::Column::MessageId.eq(message_id))
        .filter(conversation_file::Column::DeletedAt.is_null())
        .exec(txn)
        .await?;
    Ok(())
}

/// Removes the objects and rows of deleted files and of files nobody sent within a day. Returns
/// how many rows it removed; a page per call, so a backlog is finished by the next one.
pub(crate) async fn sweep_in<C: ConnectionTrait>(
    db: &C,
    store: &dyn ObjectStore,
    now: DateTime<FixedOffset>,
) -> Result<u64, ApiError> {
    let unsent_before = now - ChronoDuration::hours(UNSENT_GRACE_HOURS);
    let due = Condition::any()
        .add(conversation_file::Column::DeletedAt.is_not_null())
        .add(
            Condition::all()
                .add(conversation_file::Column::MessageId.is_null())
                .add(conversation_file::Column::CreatedAt.lt(unsent_before)),
        );
    let files = conversation_file::Entity::find()
        .filter(due)
        .order_by_asc(conversation_file::Column::CreatedAt)
        .limit(SWEEP_PAGE)
        .all(db)
        .await?;
    let mut removed = Vec::with_capacity(files.len());
    for file in &files {
        let location = ObjectPath::from(object_key(&file.conversation_id, &stored_file_name(file)));
        match store.delete(&location).await {
            Ok(()) | Err(object_store::Error::NotFound { .. }) => removed.push(file.id.clone()),
            Err(error) => {
                tracing::warn!(%error, file_id = %file.id, "Deleting a conversation file object failed");
            }
        }
    }
    if removed.is_empty() {
        return Ok(0);
    }
    let count = removed.len() as u64;
    conversation_file::Entity::delete_many()
        .filter(conversation_file::Column::Id.is_in(removed))
        .exec(db)
        .await?;
    Ok(count)
}

/// [`sweep_in`] against the platform's content store.
pub(crate) async fn sweep(state: &AppState) -> Result<u64, ApiError> {
    let store = content_store(state).await?;
    sweep_in(&state.db, store.as_generic().as_ref(), now_millis()).await
}

pub(super) async fn content_store(
    state: &AppState,
) -> Result<flow_like_storage::files::store::FlowLikeStore, ApiError> {
    let master = state.master_credentials().await?;
    Ok(master.to_store(false).await?)
}

#[utoipa::path(
    post,
    path = "/inbox/conversations/{conversation_id}/files",
    tag = "Inbox",
    params(("conversation_id" = String, Path, description = "The conversation to upload into")),
    request_body = MintFileRequest,
    responses(
        (status = 200, description = "Where to upload the file. PUT the bytes to `upload_url` within 15 minutes, then complete the upload.", body = ConversationFileUpload),
        (status = 400, description = "The name or size is invalid"),
        (status = 401, description = "Not signed in"),
        (status = 404, description = "You are not an active member of this conversation"),
        (status = 409, description = "ATTACHMENTS_ENCRYPTED_UNSUPPORTED: end-to-end encrypted conversations can't carry attachments yet"),
        (status = 413, description = "INBOX_FILE_TOO_LARGE: a conversation file has at most 35 MiB"),
        (status = 429, description = "Your storage capacity is used up")
    ),
    security(("bearer_auth" = []))
)]
#[tracing::instrument(
    name = "POST /inbox/conversations/{conversation_id}/files",
    skip(state, user, body)
)]
pub async fn mint_file(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(conversation_id): Path<String>,
    Json(body): Json<MintFileRequest>,
) -> Result<Json<ConversationFileUpload>, ApiError> {
    let reader = acting_reader(&state.db, &user, None, &conversation_id).await?;
    let payer = reader.owner.clone().unwrap_or_else(|| reader.inbox.clone());
    plain_conversation(&state.db, &conversation_id, &reader.inbox).await?;
    validate_request(&body)?;
    let held = held_bytes(&state.db, &payer).await?;
    let requested = i64::try_from(body.bytes)
        .unwrap_or(i64::MAX)
        .saturating_add(held);
    capacity::check_storage_write(&state, "", &payer, requested).await?;
    let now = now_millis();
    let file = mint_in(&state.db, &conversation_id, &reader.inbox, &body, now).await?;
    let master = state.master_credentials().await?;
    let store = master.to_store(false).await?;
    let location = ObjectPath::from(object_key(&conversation_id, &stored_file_name(&file)));
    let ttl = master.signing_ttl(UPLOAD_URL_TTL);
    let upload_url = store.sign("PUT", &location, ttl).await?;
    Ok(Json(ConversationFileUpload {
        file_id: file.id,
        upload_url: upload_url.to_string(),
        upload_expires_at: now + ChronoDuration::seconds(ttl.as_secs() as i64),
        max_bytes: MAX_CONVERSATION_FILE_BYTES,
    }))
}

#[utoipa::path(
    post,
    path = "/inbox/conversations/{conversation_id}/files/{file_id}/complete",
    tag = "Inbox",
    params(
        ("conversation_id" = String, Path, description = "The conversation"),
        ("file_id" = String, Path, description = "The file id the upload returned")
    ),
    responses(
        (status = 200, description = "The file is stored. Put these fields into the attachment part's handle (`app_id`, `path`, `bytes`, `sha256`, `mime`) and the file name into its `name`.", body = ConversationFileReady),
        (status = 401, description = "Not signed in"),
        (status = 404, description = "INBOX_FILE_NOT_FOUND: you uploaded no such file here, or you are not in this conversation"),
        (status = 409, description = "INBOX_FILE_NOT_READY: the bytes have not been uploaded"),
        (status = 413, description = "INBOX_FILE_TOO_LARGE: the upload holds more bytes than announced")
    ),
    security(("bearer_auth" = []))
)]
#[tracing::instrument(
    name = "POST /inbox/conversations/{conversation_id}/files/{file_id}/complete",
    skip(state, user)
)]
pub async fn complete_file(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((conversation_id, file_id)): Path<(String, String)>,
) -> Result<Json<ConversationFileReady>, ApiError> {
    let reader = acting_reader(&state.db, &user, None, &conversation_id).await?;
    let store = content_store(&state).await?;
    let ready = complete_in(
        &state.db,
        store.as_generic().as_ref(),
        &conversation_id,
        &reader.inbox,
        &file_id,
    )
    .await?;
    Ok(Json(ready))
}

#[utoipa::path(
    get,
    path = "/inbox/conversations/{conversation_id}/files/{file_id}",
    tag = "Inbox",
    params(
        ("conversation_id" = String, Path, description = "The conversation"),
        ("file_id" = String, Path, description = "The file id of the attachment's handle path, without its extension"),
        ReaderQuery
    ),
    responses(
        (status = 200, description = "A signed download URL that works for a few minutes; request a new one when it lapses.", body = ConversationFileDownload),
        (status = 401, description = "Not signed in"),
        (status = 404, description = "You (or the bot you read as) are not in this conversation, or the file was deleted"),
        (status = 409, description = "INBOX_FILE_NOT_READY: the upload is not complete")
    ),
    security(("bearer_auth" = []))
)]
#[tracing::instrument(
    name = "GET /inbox/conversations/{conversation_id}/files/{file_id}",
    skip(state, user)
)]
pub async fn download_file(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((conversation_id, file_id)): Path<(String, String)>,
    Query(query): Query<ReaderQuery>,
) -> Result<Json<ConversationFileDownload>, ApiError> {
    let reader = acting_reader(&state.db, &user, query.as_bot.as_deref(), &conversation_id).await?;
    let file = readable_file(&state.db, &conversation_id, &reader, &file_id).await?;
    let master = state.master_credentials().await?;
    let store = master.to_store(false).await?;
    let location = ObjectPath::from(object_key(&conversation_id, &stored_file_name(&file)));
    let ttl = master.signing_ttl(DOWNLOAD_URL_TTL);
    let url = store.sign("GET", &location, ttl).await?;
    Ok(Json(ConversationFileDownload {
        url: url.to_string(),
        expires_at: now_millis() + ChronoDuration::seconds(ttl.as_secs() as i64),
        name: file.name,
        mime: file.mime,
        bytes: file.bytes.max(0) as u64,
    }))
}
