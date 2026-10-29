use super::backend::{ChecksumCheck, LockedBackend, StoredObject, UploadSpec};
use super::config::{LockedStorageConfig, SETTINGS, SETTINGS_DOCUMENT, parse_settings_document};
use super::paths::{
    child_folders, like_escape, normalize_folder, normalize_name, object_key, path_key,
};
use super::types::*;
use crate::audit_branch;
use crate::entity::sea_orm_active_enums::{
    AuditActorType, LockedEnforcement, LockedItemScope, LockedVersionStatus,
};
use crate::entity::{locked_item, locked_item_version};
use crate::error::ApiError;
use crate::middleware::jwt::{AppPermissionResponse, AppUser, ExecutorUse};
use crate::permission::role_permission::RolePermissions;
use crate::state::AppState;
use axum::http::StatusCode;
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use chrono::{DateTime, Utc};
use flow_like::flow::execution::ExecutionPrincipal;
use flow_like_secrets::{ExposeSecret, SecretRef, SecretStore};
use flow_like_types::create_id;
use sea_orm::sea_query::{Expr, ExprTrait, Func};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, Condition, ConnectionTrait, DatabaseTransaction, EntityTrait,
    QueryFilter, QueryOrder, QuerySelect, Set, Statement,
};
use std::collections::HashMap;
use std::time::Duration;

/// Window to start the PUT; the store checks expiry when a request begins, so a
/// large upload that started in time still completes.
pub const UPLOAD_URL_TTL: Duration = Duration::from_secs(60 * 60);
/// Short, because clients reuse a download link until its stated expiry.
pub const DOWNLOAD_URL_TTL: Duration = Duration::from_secs(10 * 60);
const DEFAULT_LIST_LIMIT: u32 = 100;
const MAX_LIST_LIMIT: u32 = 500;
const MAX_LISTED_FOLDERS: u64 = 5_000;
const SWEEP_BATCH: u64 = 200;
/// A PUT that started before its URL expired may still be streaming a large file.
const ABANDONED_AFTER: chrono::TimeDelta = chrono::TimeDelta::hours(24);
const COMMIT_ATTEMPTS: usize = 5;

pub struct LockedStorage {
    pub config: LockedStorageConfig,
    pub backend: LockedBackend,
}

impl LockedStorage {
    /// Settings come from the `LOCKED_STORAGE` document in the secret store, with
    /// set variables taking precedence over it.
    pub async fn load(secrets: &SecretStore) -> flow_like_types::Result<Option<Self>> {
        use tracing::instrument::WithSubscriber;
        let document = secrets
            .get_secret_string(&SecretRef::new(SETTINGS_DOCUMENT))
            .with_subscriber(tracing::subscriber::NoSubscriber::default())
            .await;
        let mut settings = match document {
            Ok(document) => parse_settings_document(document.expose_secret())?,
            Err(error) if error.is_not_found() => HashMap::new(),
            Err(error) => {
                return Err(flow_like_types::anyhow!(
                    "reading {SETTINGS_DOCUMENT} from the secret store failed: {error}"
                ));
            }
        };
        for name in SETTINGS {
            if let Some(value) = crate::storage_config::non_empty_env(name) {
                settings.insert((*name).to_owned(), value);
            }
        }
        let Some(config) = LockedStorageConfig::from_lookup(|name| {
            settings
                .get(name)
                .cloned()
                .or_else(|| crate::storage_config::non_empty_env(name))
        })?
        else {
            return Ok(None);
        };
        let backend = LockedBackend::from_config(&config).await?;
        tracing::info!(
            bucket = %config.bucket,
            provider = ?config.provider,
            enforcement = ?config.enforcement,
            "Locked storage enabled"
        );
        Ok(Some(Self { config, backend }))
    }

    pub fn status(&self) -> LockedStorageStatus {
        let config = &self.config;
        LockedStorageStatus {
            enabled: true,
            provider: Some(config.provider),
            enforcement: Some(config.enforcement),
            retention_mode: config.mode,
            default_retention_days: config.default_retention_days,
            min_retention_days: config.min_retention_days,
            max_retention_days: config.max_retention_days,
            max_file_bytes: config.max_file_bytes,
            supports_legal_hold: config.provider.supports_legal_hold(),
            supports_retention_extension: config.enforcement == LockedEnforcement::Object,
        }
    }
}

pub fn disabled_status() -> LockedStorageStatus {
    LockedStorageStatus {
        enabled: false,
        provider: None,
        enforcement: None,
        retention_mode: crate::entity::sea_orm_active_enums::LockedRetentionMode::Compliance,
        default_retention_days: 0,
        min_retention_days: 0,
        max_retention_days: 0,
        max_file_bytes: 0,
        supports_legal_hold: false,
        supports_retention_extension: false,
    }
}

fn storage(state: &AppState) -> Result<&LockedStorage, ApiError> {
    state.locked_storage.as_deref().ok_or_else(|| {
        ApiError::service_unavailable("Locked storage is not configured on this deployment.")
    })
}

/// The authenticated principal and its role in the app, resolved once per request.
pub struct Caller {
    user: AppUser,
    permission: AppPermissionResponse,
}

impl Caller {
    pub async fn for_user(
        state: &AppState,
        user: &AppUser,
        app_id: &str,
    ) -> Result<Self, ApiError> {
        if matches!(user, AppUser::Executor(_)) {
            return Err(ApiError::forbidden(
                "Executors use the /execution locked storage routes",
            ));
        }
        Ok(Self {
            permission: user.app_permission(app_id, state).await?,
            user: user.clone(),
        })
    }

    pub async fn for_executor(
        state: &AppState,
        user: &AppUser,
        app_id: &str,
        executor_use: ExecutorUse,
    ) -> Result<Self, ApiError> {
        if !matches!(user, AppUser::Executor(_)) {
            return Err(ApiError::forbidden(
                "This route only accepts executor tokens",
            ));
        }
        Ok(Self {
            permission: user
                .execution_app_permission(app_id, state, executor_use)
                .await?,
            user: user.clone(),
        })
    }

    fn require(&self, permission: RolePermissions) -> Result<(), ApiError> {
        if self.permission.has_permission(permission) {
            Ok(())
        } else {
            Err(ApiError::forbidden(format!(
                "Your role lacks the {permission:?} permission for locked files"
            )))
        }
    }

    fn is_admin(&self) -> bool {
        self.permission.has_permission(RolePermissions::Admin)
    }

    /// Deleting project records is an admin decision, also when it is scheduled at upload.
    fn may_purge(&self, scope: LockedItemScope) -> Result<(), ApiError> {
        if scope == LockedItemScope::Project && !self.is_admin() {
            return Err(ApiError::forbidden(
                "Only admins can delete expired project locked files",
            ));
        }
        Ok(())
    }

    /// Personal locked files belong to a person; keys and app connections have none.
    fn human(&self) -> Result<&str, ApiError> {
        match (
            &self.permission.principal,
            &self.permission.effective_user_id,
        ) {
            (ExecutionPrincipal::User, Some(user_id)) => Ok(user_id),
            _ => Err(ApiError::forbidden(
                "Personal locked files require a signed-in user",
            )),
        }
    }

    fn actor_id(&self) -> String {
        self.permission
            .effective_user_id
            .clone()
            .unwrap_or_else(|| self.permission.identifier.clone())
    }

    fn owns(&self, item: &locked_item::Model) -> bool {
        match item.scope {
            LockedItemScope::Project => true,
            LockedItemScope::User => self
                .human()
                .ok()
                .is_some_and(|user| item.owner_id.as_deref() == Some(user)),
        }
    }

    fn owner_for(&self, scope: LockedItemScope) -> Result<Option<String>, ApiError> {
        match scope {
            LockedItemScope::Project => Ok(None),
            LockedItemScope::User => self.human().map(|user| Some(user.to_owned())),
        }
    }
}

fn now() -> DateTime<Utc> {
    Utc::now()
}

fn db_time(at: DateTime<Utc>) -> sea_orm::prelude::DateTimeWithTimeZone {
    at.fixed_offset()
}

fn precondition(message: impl Into<String>) -> ApiError {
    ApiError::coded(
        StatusCode::PRECONDITION_FAILED,
        "PRECONDITION_FAILED",
        message,
    )
}

fn parse_sha256(value: &str) -> Result<[u8; 32], ApiError> {
    let bytes = hex::decode(value.trim())
        .ok()
        .and_then(|bytes| <[u8; 32]>::try_from(bytes).ok())
        .ok_or_else(|| ApiError::unprocessable("sha256 must be 64 hexadecimal characters."))?;
    Ok(bytes)
}

async fn load_item(
    state: &AppState,
    app_id: &str,
    item_id: &str,
) -> Result<locked_item::Model, ApiError> {
    locked_item::Entity::find_by_id(item_id)
        .filter(locked_item::Column::AppId.eq(app_id))
        .one(&state.db)
        .await?
        .ok_or_else(|| ApiError::not_found(format!("Locked file {item_id} was not found")))
}

/// Another user's personal file answers 404, so its existence does not leak.
async fn visible_item(
    state: &AppState,
    caller: &Caller,
    app_id: &str,
    item_id: &str,
    permission: RolePermissions,
) -> Result<locked_item::Model, ApiError> {
    caller.require(permission)?;
    let item = load_item(state, app_id, item_id).await?;
    if !caller.owns(&item) {
        return Err(ApiError::not_found(format!(
            "Locked file {item_id} was not found"
        )));
    }
    Ok(item)
}

async fn load_version(
    state: &AppState,
    item: &locked_item::Model,
    version_id: &str,
) -> Result<locked_item_version::Model, ApiError> {
    locked_item_version::Entity::find_by_id(version_id)
        .filter(locked_item_version::Column::ItemId.eq(&item.id))
        .one(&state.db)
        .await?
        .ok_or_else(|| {
            ApiError::not_found(format!(
                "Version {version_id} of locked file {} was not found",
                item.id
            ))
        })
}

async fn newest_stored_versions<C: ConnectionTrait>(
    db: &C,
    item_ids: &[String],
) -> Result<std::collections::HashMap<String, locked_item_version::Model>, ApiError> {
    if item_ids.is_empty() {
        return Ok(Default::default());
    }
    let placeholders = (1..=item_ids.len())
        .map(|index| format!("${index}"))
        .collect::<Vec<_>>()
        .join(",");
    let sql = format!(
        r#"SELECT * FROM "LockedItemVersion" WHERE ("itemId", "version") IN (
            SELECT "itemId", MAX("version") FROM "LockedItemVersion"
            WHERE "itemId" IN ({placeholders}) AND "status" = 'LOCKED' GROUP BY "itemId")"#
    );
    let rows = locked_item_version::Entity::find()
        .from_raw_sql(Statement::from_sql_and_values(
            db.get_database_backend(),
            sql,
            item_ids.iter().map(|id| id.clone().into()),
        ))
        .all(db)
        .await?;
    Ok(rows
        .into_iter()
        .map(|row| (row.item_id.clone(), row))
        .collect())
}

async fn item_with_latest<C: ConnectionTrait>(
    db: &C,
    item: locked_item::Model,
) -> Result<LockedItem, ApiError> {
    let mut latest = newest_stored_versions(db, std::slice::from_ref(&item.id)).await?;
    let latest = latest
        .remove(&item.id)
        .map(|version| LockedItemVersion::from_model(version, now()));
    Ok(LockedItem::from_model(item, latest))
}

async fn detail(
    state: &AppState,
    caller: &Caller,
    item: locked_item::Model,
) -> Result<LockedItemDetail, ApiError> {
    let at = now();
    let actor = caller.actor_id();
    let admin = caller.is_admin();
    let versions = locked_item_version::Entity::find()
        .filter(locked_item_version::Column::ItemId.eq(&item.id))
        .order_by_desc(locked_item_version::Column::CreatedAt)
        .all(&state.db)
        .await?
        .into_iter()
        .filter(|version| {
            version.status != LockedVersionStatus::Pending || admin || version.uploaded_by == actor
        })
        .map(|version| LockedItemVersion::from_model(version, at))
        .collect::<Vec<_>>();
    let latest = versions
        .iter()
        .filter(|version| version.status == LockedVersionStatus::Locked)
        .max_by_key(|version| version.version)
        .cloned();
    let mut versions = versions;
    versions.sort_by(|a, b| {
        b.version
            .unwrap_or(i32::MAX)
            .cmp(&a.version.unwrap_or(i32::MAX))
            .then(b.created_at.cmp(&a.created_at))
    });
    Ok(LockedItemDetail {
        item: LockedItem::from_model(item, latest),
        versions,
    })
}

pub fn status(state: &AppState) -> LockedStorageStatus {
    state
        .locked_storage
        .as_deref()
        .map(LockedStorage::status)
        .unwrap_or_else(disabled_status)
}

#[derive(serde::Serialize, serde::Deserialize)]
struct ListCursor {
    name: String,
    id: String,
}

fn encode_cursor(item: &locked_item::Model) -> String {
    let cursor = ListCursor {
        name: item.name.clone(),
        id: item.id.clone(),
    };
    URL_SAFE_NO_PAD.encode(flow_like_types::json::to_vec(&cursor).unwrap_or_default())
}

fn decode_cursor(cursor: &str) -> Result<ListCursor, ApiError> {
    URL_SAFE_NO_PAD
        .decode(cursor)
        .ok()
        .and_then(|bytes| flow_like_types::json::from_slice(&bytes).ok())
        .ok_or_else(|| ApiError::bad_request("The listing cursor is invalid."))
}

pub async fn list(
    state: &AppState,
    caller: &Caller,
    app_id: &str,
    request: ListLockedItemsRequest,
) -> Result<ListLockedItemsResponse, ApiError> {
    storage(state)?;
    caller.require(RolePermissions::ReadFiles)?;
    let scope = request.scope.unwrap_or(LockedItemScope::Project);
    let owner = caller.owner_for(scope)?;
    let folder = normalize_folder(request.folder.as_deref().unwrap_or_default())?;
    let limit = request
        .limit
        .unwrap_or(DEFAULT_LIST_LIMIT)
        .clamp(1, MAX_LIST_LIMIT);

    let mut scoped = Condition::all()
        .add(locked_item::Column::AppId.eq(app_id))
        .add(locked_item::Column::Scope.eq(scope))
        .add(locked_item::Column::LiveVersions.gt(0));
    if let Some(owner) = &owner {
        scoped = scoped.add(locked_item::Column::OwnerId.eq(owner));
    }
    let below = format!("{}/%", like_escape(&folder));

    let mut query = locked_item::Entity::find().filter(scoped.clone());
    query = match (request.recursive, folder.is_empty()) {
        (true, true) => query,
        (true, false) => query.filter(
            Condition::any()
                .add(locked_item::Column::Folder.eq(&folder))
                .add(locked_item::Column::Folder.like(below.as_str())),
        ),
        (false, _) => query.filter(locked_item::Column::Folder.eq(&folder)),
    };
    if let Some(search) = request
        .search
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        query = query.filter(
            Expr::expr(Func::lower(Expr::col((
                locked_item::Entity,
                locked_item::Column::Name,
            ))))
            .like(format!("%{}%", like_escape(&search.to_lowercase()))),
        );
    }
    if let Some(cursor) = request.cursor.as_deref().filter(|c| !c.is_empty()) {
        let cursor = decode_cursor(cursor)?;
        query = query.filter(
            Condition::any()
                .add(locked_item::Column::Name.gt(&cursor.name))
                .add(
                    Condition::all()
                        .add(locked_item::Column::Name.eq(&cursor.name))
                        .add(locked_item::Column::Id.gt(&cursor.id)),
                ),
        );
    }
    let mut items = query
        .order_by_asc(locked_item::Column::Name)
        .order_by_asc(locked_item::Column::Id)
        .limit(u64::from(limit) + 1)
        .all(&state.db)
        .await?;
    let next_cursor = (items.len() > limit as usize).then(|| {
        items.truncate(limit as usize);
        items.last().map(encode_cursor)
    });

    let folders = if request.recursive || request.cursor.is_some() {
        Vec::new()
    } else {
        let mut folder_query = locked_item::Entity::find()
            .select_only()
            .column(locked_item::Column::Folder)
            .distinct()
            .filter(scoped);
        folder_query = if folder.is_empty() {
            folder_query.filter(locked_item::Column::Folder.ne(""))
        } else {
            folder_query.filter(locked_item::Column::Folder.like(below.as_str()))
        };
        let stored: Vec<String> = folder_query
            .limit(MAX_LISTED_FOLDERS)
            .into_tuple()
            .all(&state.db)
            .await?;
        child_folders(&folder, stored.iter().map(String::as_str))
    };

    let ids = items.iter().map(|item| item.id.clone()).collect::<Vec<_>>();
    let mut latest = newest_stored_versions(&state.db, &ids).await?;
    let at = now();
    Ok(ListLockedItemsResponse {
        items: items
            .into_iter()
            .map(|item| {
                let newest = latest
                    .remove(&item.id)
                    .map(|version| LockedItemVersion::from_model(version, at));
                LockedItem::from_model(item, newest)
            })
            .collect(),
        folders,
        next_cursor: next_cursor.flatten(),
    })
}

pub async fn get(
    state: &AppState,
    caller: &Caller,
    app_id: &str,
    request: GetLockedItemRequest,
) -> Result<LockedItemDetail, ApiError> {
    storage(state)?;
    let item = match request.item_id.as_deref().filter(|id| !id.is_empty()) {
        Some(item_id) => {
            visible_item(state, caller, app_id, item_id, RolePermissions::ReadFiles).await?
        }
        None => {
            caller.require(RolePermissions::ReadFiles)?;
            let scope = request.scope.unwrap_or(LockedItemScope::Project);
            let owner = caller.owner_for(scope)?;
            let folder = normalize_folder(request.folder.as_deref().unwrap_or_default())?;
            let name = normalize_name(request.name.as_deref().unwrap_or_default())?;
            let key = path_key(scope, owner.as_deref(), &folder, &name);
            locked_item::Entity::find()
                .filter(locked_item::Column::AppId.eq(app_id))
                .filter(locked_item::Column::PathKey.eq(&key))
                .filter(locked_item::Column::LatestVersion.gt(0))
                .one(&state.db)
                .await?
                .ok_or_else(|| ApiError::not_found(format!("No locked file at {folder}/{name}")))?
        }
    };
    detail(state, caller, item).await
}

/// Reuses an item a previous upload created but never committed.
async fn target_item(
    state: &AppState,
    caller: &Caller,
    app_id: &str,
    request: &CreateLockedUploadRequest,
    file_name: &str,
) -> Result<locked_item::Model, ApiError> {
    if let Some(item_id) = request.item_id.as_deref().filter(|id| !id.is_empty()) {
        let item =
            visible_item(state, caller, app_id, item_id, RolePermissions::WriteFiles).await?;
        if request.delete_after_retention {
            caller.may_purge(item.scope)?;
        }
        return Ok(item);
    }
    let scope = request.scope.unwrap_or(LockedItemScope::Project);
    if request.delete_after_retention {
        caller.may_purge(scope)?;
    }
    let owner = caller.owner_for(scope)?;
    let folder = normalize_folder(request.folder.as_deref().unwrap_or_default())?;
    let name = normalize_name(request.name.as_deref().unwrap_or(file_name))?;
    let key = path_key(scope, owner.as_deref(), &folder, &name);
    let existing = || {
        locked_item::Entity::find()
            .filter(locked_item::Column::AppId.eq(app_id))
            .filter(locked_item::Column::PathKey.eq(&key))
            .one(&state.db)
    };
    let check = |item: locked_item::Model| {
        if item.live_versions > 0 && request.if_exists == IfExists::Error {
            Err(ApiError::conflict(format!(
                "A locked file named '{}' already exists in this folder; upload a new version instead.",
                item.name
            )))
        } else {
            Ok(item)
        }
    };
    if let Some(item) = existing().await? {
        return check(item);
    }
    let at = db_time(now());
    let created = locked_item::ActiveModel {
        id: Set(create_id()),
        app_id: Set(app_id.to_owned()),
        scope: Set(scope),
        owner_id: Set(owner),
        path_key: Set(key.clone()),
        name: Set(name),
        folder: Set(folder),
        content_type: Set(None),
        latest_version: Set(0),
        latest_size: Set(0),
        live_versions: Set(0),
        live_size: Set(0),
        metadata: Set(request.metadata.clone()),
        created_by: Set(caller.actor_id()),
        created_at: Set(at),
        updated_at: Set(at),
    }
    .insert(&state.db)
    .await;
    match created {
        Ok(item) => Ok(item),
        // A concurrent upload created the same path between the read and the insert.
        Err(error) => match existing().await? {
            Some(item) => check(item),
            None => Err(error.into()),
        },
    }
}

pub async fn upload(
    state: &AppState,
    caller: &Caller,
    app_id: &str,
    request: CreateLockedUploadRequest,
) -> Result<LockedUploadTicket, ApiError> {
    let storage = storage(state)?;
    caller.require(RolePermissions::WriteFiles)?;
    let config = &storage.config;
    let sha256 = parse_sha256(&request.sha256)?;
    if request.size > config.max_file_bytes {
        return Err(ApiError::unprocessable(format!(
            "Locked files can be at most {} bytes; this one has {}.",
            config.max_file_bytes, request.size
        )));
    }
    let retention_days = config
        .retention_days(request.retention_days)
        .map_err(ApiError::unprocessable)?;
    let file_name = normalize_name(&request.file_name)?;
    let content_type = request
        .content_type
        .clone()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| {
            flow_like_types::mime_guess::from_path(&file_name)
                .first_or_octet_stream()
                .to_string()
        });
    let size = i64::try_from(request.size)
        .map_err(|_| ApiError::unprocessable("The file is too large."))?;

    crate::capacity::check_storage_write(state, app_id, &caller.actor_id(), size).await?;
    let item = target_item(state, caller, app_id, &request, &file_name).await?;

    let version_id = create_id();
    let key = object_key(
        app_id,
        item.scope,
        item.owner_id.as_deref(),
        &item.id,
        &version_id,
    )?;
    let signed = storage
        .backend
        .sign_upload(
            &UploadSpec {
                key: &key,
                content_type: &content_type,
                sha256,
            },
            UPLOAD_URL_TTL,
        )
        .await?;
    let at = now();
    let expires_at = at + chrono::Duration::from_std(UPLOAD_URL_TTL).unwrap_or_default();
    let version = locked_item_version::ActiveModel {
        id: Set(version_id.clone()),
        item_id: Set(item.id.clone()),
        app_id: Set(app_id.to_owned()),
        version: Set(None),
        status: Set(LockedVersionStatus::Pending),
        object_key: Set(key),
        provider_version_id: Set(None),
        file_name: Set(file_name),
        content_type: Set(content_type),
        size: Set(size),
        sha256: Set(hex::encode(sha256)),
        checksum_verified: Set(false),
        retention_mode: Set(config.mode),
        retain_until: Set(db_time(
            at + chrono::Duration::days(i64::from(retention_days)),
        )),
        enforcement: Set(config.enforcement),
        legal_hold: Set(false),
        delete_after_retention: Set(Some(request.delete_after_retention)),
        comment: Set(request
            .comment
            .clone()
            .filter(|comment| !comment.trim().is_empty())),
        metadata: Set(request.metadata.clone()),
        uploaded_by: Set(caller.actor_id()),
        upload_expires_at: Set(db_time(expires_at)),
        created_at: Set(db_time(at)),
        locked_at: Set(None),
        purged_at: Set(None),
        purged_by: Set(None),
    }
    .insert(&state.db)
    .await?;

    audit_branch!(
        state,
        caller.user,
        app_id,
        "locked_file.upload.authorize",
        "LockedFile",
        &item.id,
        serde_json::json!({
            "version_id": version_id,
            "file_name": version.file_name,
            "size": size,
            "sha256": version.sha256,
            "retention_days": retention_days,
            "delete_after_retention": request.delete_after_retention,
        })
    );

    Ok(LockedUploadTicket {
        item: item_with_latest(&state.db, item).await?,
        version: LockedItemVersion::from_model(version, at),
        upload: LockedUploadTarget {
            url: signed.url,
            method: "PUT".to_owned(),
            headers: signed.headers,
            expires_at,
        },
    })
}

/// Verifies the stored bytes, locks them and assigns the version number. Shared by
/// `commit` and the sweep that settles bucket-locked uploads nobody committed.
async fn finalize(
    state: &AppState,
    storage: &LockedStorage,
    item: &locked_item::Model,
    version: &locked_item_version::Model,
    stored: StoredObject,
) -> Result<(), ApiError> {
    let expected = parse_sha256(&version.sha256)?;
    let size = u64::try_from(version.size).unwrap_or_default();
    if stored.size != size {
        return Err(precondition(format!(
            "The uploaded object has {} bytes but {size} were announced; abort and upload again.",
            stored.size
        )));
    }
    let verified = match stored.checksum {
        ChecksumCheck::Matches => true,
        ChecksumCheck::Differs => {
            return Err(precondition(
                "The uploaded bytes do not match the announced SHA-256; abort and upload again.",
            ));
        }
        ChecksumCheck::Unknown if stored.size <= storage.config.verify_max_bytes => {
            let (digest, _) = storage
                .backend
                .sha256(&version.object_key, stored.version_id.as_deref())
                .await?;
            if digest != expected {
                return Err(precondition(
                    "The uploaded bytes do not match the announced SHA-256; abort and upload again.",
                ));
            }
            true
        }
        ChecksumCheck::Unknown => false,
    };

    let at = now();
    let retain_until = match storage.config.enforcement {
        LockedEnforcement::Object => {
            let days = Ord::max((version.retain_until - version.created_at).num_days(), 1);
            let wanted = at + chrono::Duration::days(days);
            match stored.retain_until {
                Some(current) if current >= wanted => current,
                current => {
                    let until = current.map_or(wanted, |current| Ord::max(current, wanted));
                    storage
                        .backend
                        .lock(
                            &version.object_key,
                            stored.version_id.as_deref(),
                            version.retention_mode,
                            until,
                        )
                        .await?;
                    until
                }
            }
        }
        LockedEnforcement::Bucket => stored.retain_until.unwrap_or_else(|| {
            at + chrono::Duration::days(i64::from(storage.config.default_retention_days))
        }),
    };

    for _ in 0..COMMIT_ATTEMPTS {
        let current = load_item(state, &item.app_id, &item.id).await?;
        let (item, version, stored_version) =
            (current.clone(), version.clone(), stored.version_id.clone());
        let committed = state
            .transaction(move |txn| {
                let (item, version, stored_version) =
                    (item.clone(), version.clone(), stored_version.clone());
                Box::pin(async move {
                    commit_rows(
                        txn,
                        &item,
                        &version,
                        stored_version,
                        verified,
                        retain_until,
                        at,
                    )
                    .await
                })
            })
            .await?;
        if committed {
            return Ok(());
        }
    }
    Err(ApiError::conflict(
        "Too many concurrent uploads to this locked file; commit again.",
    ))
}

/// `false` when another commit claimed the next version number first.
async fn commit_rows(
    txn: &DatabaseTransaction,
    item: &locked_item::Model,
    version: &locked_item_version::Model,
    provider_version_id: Option<String>,
    checksum_verified: bool,
    retain_until: DateTime<Utc>,
    at: DateTime<Utc>,
) -> Result<bool, ApiError> {
    let number = item.latest_version + 1;
    let claimed = locked_item::Entity::update_many()
        .col_expr(locked_item::Column::LatestVersion, Expr::value(number))
        .col_expr(locked_item::Column::LatestSize, Expr::value(version.size))
        .col_expr(
            locked_item::Column::LiveVersions,
            Expr::col(locked_item::Column::LiveVersions).add(1),
        )
        .col_expr(
            locked_item::Column::LiveSize,
            Expr::col(locked_item::Column::LiveSize).add(version.size),
        )
        .col_expr(
            locked_item::Column::ContentType,
            Expr::value(version.content_type.clone()),
        )
        .col_expr(locked_item::Column::UpdatedAt, Expr::value(db_time(at)))
        .filter(locked_item::Column::Id.eq(&item.id))
        .filter(locked_item::Column::LatestVersion.eq(item.latest_version))
        .exec(txn)
        .await?
        .rows_affected;
    if claimed == 0 {
        return Ok(false);
    }
    let locked = locked_item_version::Entity::update_many()
        .col_expr(locked_item_version::Column::Version, Expr::value(number))
        .col_expr(
            locked_item_version::Column::Status,
            Expr::value(LockedVersionStatus::Locked),
        )
        .col_expr(
            locked_item_version::Column::ProviderVersionId,
            Expr::value(provider_version_id),
        )
        .col_expr(
            locked_item_version::Column::ChecksumVerified,
            Expr::value(checksum_verified),
        )
        .col_expr(
            locked_item_version::Column::RetainUntil,
            Expr::value(db_time(retain_until)),
        )
        .col_expr(
            locked_item_version::Column::LockedAt,
            Expr::value(db_time(at)),
        )
        .filter(locked_item_version::Column::Id.eq(&version.id))
        .filter(locked_item_version::Column::Status.eq(LockedVersionStatus::Pending))
        .exec(txn)
        .await?
        .rows_affected;
    if locked == 0 {
        return Err(ApiError::conflict(
            "This upload was already committed or discarded.",
        ));
    }
    crate::capacity::adjust_locked_storage(
        txn,
        &item.app_id,
        item.owner_id.as_deref(),
        version.size,
    )
    .await?;
    Ok(true)
}

pub async fn commit(
    state: &AppState,
    caller: &Caller,
    app_id: &str,
    request: LockedVersionRef,
) -> Result<LockedItemDetail, ApiError> {
    let storage = storage(state)?;
    let item = visible_item(
        state,
        caller,
        app_id,
        &request.item_id,
        RolePermissions::WriteFiles,
    )
    .await?;
    let version = load_version(state, &item, &request.version_id).await?;
    if version.uploaded_by != caller.actor_id() && !caller.is_admin() {
        return Err(ApiError::forbidden(
            "Only the uploader or an admin can commit this upload",
        ));
    }
    match version.status {
        LockedVersionStatus::Locked => return detail(state, caller, item).await,
        LockedVersionStatus::Purged => {
            return Err(ApiError::conflict("This version was already purged."));
        }
        LockedVersionStatus::Pending => {}
    }
    let expected = parse_sha256(&version.sha256)?;
    let stored = storage
        .backend
        .inspect(&version.object_key, &expected)
        .await?
        .ok_or_else(|| precondition("The upload has not arrived in storage yet."))?;
    if let Err(error) = finalize(state, storage, &item, &version, stored).await {
        let current = load_version(state, &item, &version.id).await?;
        if current.status != LockedVersionStatus::Locked {
            return Err(error);
        }
    }

    audit_branch!(
        state,
        caller.user,
        app_id,
        "locked_file.version.commit",
        "LockedFile",
        &item.id,
        serde_json::json!({
            "version_id": version.id,
            "file_name": version.file_name,
            "size": version.size,
            "sha256": version.sha256,
        })
    );
    let item = load_item(state, app_id, &item.id).await?;
    detail(state, caller, item).await
}

async fn discard_pending(
    state: &AppState,
    item: &locked_item::Model,
    version: &locked_item_version::Model,
) -> Result<(), ApiError> {
    locked_item_version::Entity::delete_many()
        .filter(locked_item_version::Column::Id.eq(&version.id))
        .filter(locked_item_version::Column::Status.eq(LockedVersionStatus::Pending))
        .exec(&state.db)
        .await?;
    if item.latest_version == 0 {
        let remaining = locked_item_version::Entity::find()
            .filter(locked_item_version::Column::ItemId.eq(&item.id))
            .one(&state.db)
            .await?;
        if remaining.is_none() {
            locked_item::Entity::delete_many()
                .filter(locked_item::Column::Id.eq(&item.id))
                .filter(locked_item::Column::LatestVersion.eq(0))
                .exec(&state.db)
                .await?;
        }
    }
    Ok(())
}

pub async fn abort(
    state: &AppState,
    caller: &Caller,
    app_id: &str,
    request: LockedVersionRef,
) -> Result<LockedEmpty, ApiError> {
    let storage = storage(state)?;
    let item = visible_item(
        state,
        caller,
        app_id,
        &request.item_id,
        RolePermissions::WriteFiles,
    )
    .await?;
    let version = load_version(state, &item, &request.version_id).await?;
    if version.uploaded_by != caller.actor_id() && !caller.is_admin() {
        return Err(ApiError::forbidden(
            "Only the uploader or an admin can abort this upload",
        ));
    }
    if version.status != LockedVersionStatus::Pending {
        return Err(ApiError::conflict("Only pending uploads can be aborted."));
    }
    let expected = parse_sha256(&version.sha256)?;
    if let Some(stored) = storage
        .backend
        .inspect(&version.object_key, &expected)
        .await?
    {
        if let Err(error) = storage
            .backend
            .delete(&version.object_key, stored.version_id.as_deref())
            .await
        {
            tracing::info!(%error, version_id = %version.id, "Aborted locked upload is retained by the bucket");
            return Err(ApiError::conflict(
                "The uploaded bytes are already retained by the bucket policy; commit the upload instead.",
            ));
        }
    }
    discard_pending(state, &item, &version).await?;
    audit_branch!(
        state,
        caller.user,
        app_id,
        "locked_file.version.abort",
        "LockedFile",
        &item.id,
        serde_json::json!({ "version_id": version.id })
    );
    Ok(LockedEmpty {})
}

pub async fn download(
    state: &AppState,
    caller: &Caller,
    app_id: &str,
    request: DownloadLockedItemRequest,
) -> Result<LockedDownload, ApiError> {
    let storage = storage(state)?;
    let item = visible_item(
        state,
        caller,
        app_id,
        &request.item_id,
        RolePermissions::ReadFiles,
    )
    .await?;
    let mut query = locked_item_version::Entity::find()
        .filter(locked_item_version::Column::ItemId.eq(&item.id))
        .filter(locked_item_version::Column::Status.eq(LockedVersionStatus::Locked));
    if let Some(number) = request.version {
        query = query.filter(locked_item_version::Column::Version.eq(number));
    }
    let version = query
        .order_by_desc(locked_item_version::Column::Version)
        .one(&state.db)
        .await?
        .ok_or_else(|| match request.version {
            Some(number) => ApiError::not_found(format!(
                "Version {number} of locked file {} is not stored",
                item.id
            )),
            None => ApiError::not_found(format!("Locked file {} has no stored version", item.id)),
        })?;
    let url = storage
        .backend
        .sign_download(
            &version.object_key,
            version.provider_version_id.as_deref(),
            &version.file_name,
            &version.content_type,
            DOWNLOAD_URL_TTL,
        )
        .await?;
    audit_branch!(
        state,
        caller.user,
        app_id,
        "locked_file.access.authorize",
        "LockedFile",
        &item.id,
        serde_json::json!({ "version_id": version.id, "version": version.version })
    );
    Ok(LockedDownload {
        item_id: item.id,
        version_id: version.id,
        version: version.version.unwrap_or_default(),
        file_name: version.file_name,
        content_type: version.content_type,
        size: version.size,
        sha256: version.sha256,
        url,
        expires_at: now() + chrono::Duration::from_std(DOWNLOAD_URL_TTL).unwrap_or_default(),
    })
}

async fn locked_version_for_admin(
    state: &AppState,
    caller: &Caller,
    app_id: &str,
    item_id: &str,
    version_id: &str,
) -> Result<(locked_item::Model, locked_item_version::Model), ApiError> {
    caller.require(RolePermissions::Admin)?;
    let item = visible_item(state, caller, app_id, item_id, RolePermissions::ReadFiles).await?;
    let version = committed_version(state, &item, version_id).await?;
    Ok((item, version))
}

async fn committed_version(
    state: &AppState,
    item: &locked_item::Model,
    version_id: &str,
) -> Result<locked_item_version::Model, ApiError> {
    let version = load_version(state, item, version_id).await?;
    if version.status != LockedVersionStatus::Locked {
        return Err(ApiError::conflict(
            "Only stored, committed versions can be changed.",
        ));
    }
    Ok(version)
}

pub async fn extend_retention(
    state: &AppState,
    caller: &Caller,
    app_id: &str,
    request: ExtendLockedRetentionRequest,
) -> Result<LockedItemVersion, ApiError> {
    let storage = storage(state)?;
    if storage.config.enforcement != LockedEnforcement::Object {
        return Err(ApiError::conflict(
            "Retention is enforced by the bucket policy on this deployment and cannot be changed per file.",
        ));
    }
    let (item, version) =
        locked_version_for_admin(state, caller, app_id, &request.item_id, &request.version_id)
            .await?;
    let current = version.retain_until.with_timezone(&Utc);
    if request.retain_until <= current {
        return Err(ApiError::conflict(format!(
            "Retention can only be extended; it already runs until {}.",
            current.to_rfc3339()
        )));
    }
    let ceiling = now() + chrono::Duration::days(i64::from(storage.config.max_retention_days));
    if request.retain_until > ceiling {
        return Err(ApiError::unprocessable(format!(
            "Retention can run at most {} days from today.",
            storage.config.max_retention_days
        )));
    }
    storage
        .backend
        .lock(
            &version.object_key,
            version.provider_version_id.as_deref(),
            version.retention_mode,
            request.retain_until,
        )
        .await?;
    let mut active: locked_item_version::ActiveModel = version.into();
    active.retain_until = Set(db_time(request.retain_until));
    let version = active.update(&state.db).await?;
    audit_branch!(
        state,
        caller.user,
        app_id,
        "locked_file.retention.extend",
        "LockedFile",
        &item.id,
        serde_json::json!({
            "version_id": version.id,
            "from": current,
            "to": request.retain_until,
        })
    );
    Ok(LockedItemVersion::from_model(version, now()))
}

pub async fn set_legal_hold(
    state: &AppState,
    caller: &Caller,
    app_id: &str,
    request: SetLockedLegalHoldRequest,
) -> Result<LockedItemVersion, ApiError> {
    let storage = storage(state)?;
    if !storage.config.provider.supports_legal_hold() {
        return Err(ApiError::conflict(
            "This storage provider has no legal holds.",
        ));
    }
    let (item, version) =
        locked_version_for_admin(state, caller, app_id, &request.item_id, &request.version_id)
            .await?;
    storage
        .backend
        .legal_hold(
            &version.object_key,
            version.provider_version_id.as_deref(),
            request.enabled,
        )
        .await?;
    let mut active: locked_item_version::ActiveModel = version.into();
    active.legal_hold = Set(request.enabled);
    let version = active.update(&state.db).await?;
    audit_branch!(
        state,
        caller.user,
        app_id,
        "locked_file.legal_hold.set",
        "LockedFile",
        &item.id,
        serde_json::json!({ "version_id": version.id, "enabled": request.enabled })
    );
    Ok(LockedItemVersion::from_model(version, now()))
}

/// Retention stays as it is either way, so this needs only the right to purge the file.
pub async fn set_auto_delete(
    state: &AppState,
    caller: &Caller,
    app_id: &str,
    request: SetLockedAutoDeleteRequest,
) -> Result<LockedItemVersion, ApiError> {
    storage(state)?;
    let item = visible_item(
        state,
        caller,
        app_id,
        &request.item_id,
        RolePermissions::WriteFiles,
    )
    .await?;
    caller.may_purge(item.scope)?;
    let version = committed_version(state, &item, &request.version_id).await?;
    let mut active: locked_item_version::ActiveModel = version.into();
    active.delete_after_retention = Set(Some(request.enabled));
    let version = active.update(&state.db).await?;
    audit_branch!(
        state,
        caller.user,
        app_id,
        "locked_file.auto_delete.set",
        "LockedFile",
        &item.id,
        serde_json::json!({ "version_id": version.id, "enabled": request.enabled })
    );
    Ok(LockedItemVersion::from_model(version, now()))
}

pub async fn purge(
    state: &AppState,
    caller: &Caller,
    app_id: &str,
    request: PurgeLockedItemRequest,
) -> Result<LockedItemDetail, ApiError> {
    let storage = storage(state)?;
    let item = visible_item(
        state,
        caller,
        app_id,
        &request.item_id,
        RolePermissions::WriteFiles,
    )
    .await?;
    caller.may_purge(item.scope)?;
    let mut query = locked_item_version::Entity::find()
        .filter(locked_item_version::Column::ItemId.eq(&item.id))
        .filter(locked_item_version::Column::Status.eq(LockedVersionStatus::Locked));
    if let Some(version_id) = request.version_id.as_deref().filter(|id| !id.is_empty()) {
        query = query.filter(locked_item_version::Column::Id.eq(version_id));
    }
    let versions = query.all(&state.db).await?;
    if versions.is_empty() {
        return Err(ApiError::not_found("No stored version matches."));
    }
    let at = now();
    if let Some(blocked) = versions
        .iter()
        .filter(|version| version.legal_hold || version.retain_until.with_timezone(&Utc) > at)
        .min_by_key(|version| version.retain_until)
    {
        return Err(ApiError::conflict(if blocked.legal_hold {
            format!(
                "Version {} is under legal hold.",
                blocked.version.unwrap_or_default()
            )
        } else {
            format!(
                "Version {} is retained until {}; it can be deleted after that.",
                blocked.version.unwrap_or_default(),
                blocked.retain_until.with_timezone(&Utc).to_rfc3339()
            )
        }));
    }

    let purged_by = caller.actor_id();
    for version in versions {
        purge_version(state, storage, &item, &version, &purged_by, at).await?;
        audit_branch!(
            state,
            caller.user,
            app_id,
            "locked_file.version.purge",
            "LockedFile",
            &item.id,
            serde_json::json!({
                "version_id": version.id,
                "version": version.version,
                "sha256": version.sha256,
                "size": version.size,
            })
        );
    }
    let item = load_item(state, app_id, &item.id).await?;
    detail(state, caller, item).await
}

/// Deletes the stored bytes of one expired version; its row stays as `PURGED` evidence.
async fn purge_version(
    state: &AppState,
    storage: &LockedStorage,
    item: &locked_item::Model,
    version: &locked_item_version::Model,
    purged_by: &str,
    at: DateTime<Utc>,
) -> Result<(), ApiError> {
    storage
        .backend
        .delete(&version.object_key, version.provider_version_id.as_deref())
        .await
        .map_err(|error| {
            ApiError::conflict(format!(
                "The store still retains version {}: {error}",
                version.version.unwrap_or_default()
            ))
        })?;
    let (row_item, row_version, row_purger) = (item.clone(), version.clone(), purged_by.to_owned());
    state
        .transaction(move |txn| {
            let (item, version, purged_by) =
                (row_item.clone(), row_version.clone(), row_purger.clone());
            Box::pin(async move { purge_rows(txn, &item, &version, &purged_by, at).await })
        })
        .await
}

async fn purge_rows(
    txn: &DatabaseTransaction,
    item: &locked_item::Model,
    version: &locked_item_version::Model,
    purged_by: &str,
    at: DateTime<Utc>,
) -> Result<(), ApiError> {
    let purged = locked_item_version::Entity::update_many()
        .col_expr(
            locked_item_version::Column::Status,
            Expr::value(LockedVersionStatus::Purged),
        )
        .col_expr(
            locked_item_version::Column::PurgedAt,
            Expr::value(db_time(at)),
        )
        .col_expr(
            locked_item_version::Column::PurgedBy,
            Expr::value(purged_by.to_owned()),
        )
        .filter(locked_item_version::Column::Id.eq(&version.id))
        .filter(locked_item_version::Column::Status.eq(LockedVersionStatus::Locked))
        .exec(txn)
        .await?
        .rows_affected;
    if purged == 0 {
        return Ok(());
    }
    let newest_size = newest_stored_versions(txn, std::slice::from_ref(&item.id))
        .await?
        .remove(&item.id)
        .map_or(0, |newest| newest.size);
    locked_item::Entity::update_many()
        .col_expr(
            locked_item::Column::LiveVersions,
            Expr::col(locked_item::Column::LiveVersions).sub(1),
        )
        .col_expr(
            locked_item::Column::LiveSize,
            Expr::col(locked_item::Column::LiveSize).sub(version.size),
        )
        .col_expr(locked_item::Column::LatestSize, Expr::value(newest_size))
        .col_expr(locked_item::Column::UpdatedAt, Expr::value(db_time(at)))
        .filter(locked_item::Column::Id.eq(&item.id))
        .exec(txn)
        .await?;
    crate::capacity::adjust_locked_storage(
        txn,
        &item.app_id,
        item.owner_id.as_deref(),
        -version.size,
    )
    .await
}

/// Settles uploads whose ticket expired without a commit. Unlocked objects are
/// deleted; objects a bucket rule already retains are committed and charged,
/// since they occupy storage either way.
pub async fn sweep_abandoned_uploads(state: &AppState) -> Result<u64, ApiError> {
    let Some(storage) = state.locked_storage.as_deref() else {
        return Ok(0);
    };
    let cutoff = now() - ABANDONED_AFTER;
    let pending = locked_item_version::Entity::find()
        .filter(locked_item_version::Column::Status.eq(LockedVersionStatus::Pending))
        .filter(locked_item_version::Column::UploadExpiresAt.lt(db_time(cutoff)))
        .order_by_asc(locked_item_version::Column::UploadExpiresAt)
        .limit(SWEEP_BATCH)
        .all(&state.db)
        .await?;
    let mut settled = 0;
    for version in pending {
        let item = match load_item(state, &version.app_id, &version.item_id).await {
            Ok(item) => item,
            Err(_) => {
                locked_item_version::Entity::delete_by_id(version.id.clone())
                    .exec(&state.db)
                    .await?;
                continue;
            }
        };
        if let Err(error) = settle_abandoned(state, storage, &item, &version).await {
            tracing::warn!(%error, version_id = %version.id, "Abandoned locked upload could not be settled");
            continue;
        }
        settled += 1;
    }
    Ok(settled)
}

async fn settle_abandoned(
    state: &AppState,
    storage: &LockedStorage,
    item: &locked_item::Model,
    version: &locked_item_version::Model,
) -> Result<(), ApiError> {
    let expected = parse_sha256(&version.sha256)?;
    let Some(stored) = storage
        .backend
        .inspect(&version.object_key, &expected)
        .await?
    else {
        return discard_pending(state, item, version).await;
    };
    // Already retained bytes cannot be deleted and must be charged: a bucket rule
    // locked them at write, or a commit locked them and failed before its rows.
    let retained = storage.config.enforcement == LockedEnforcement::Bucket
        || stored.retain_until.is_some_and(|until| until > now());
    match storage
        .backend
        .delete(&version.object_key, stored.version_id.as_deref())
        .await
    {
        Ok(()) => discard_pending(state, item, version).await,
        Err(_) if retained => {
            finalize(state, storage, item, version, stored).await?;
            tracing::info!(version_id = %version.id, "Committed a retained upload nobody committed");
            Ok(())
        }
        Err(error) => Err(error.into()),
    }
}

/// `purgedBy` and audit actor of the versions the sweep deletes.
const RETENTION_PURGER: &str = "system:retention";
const RETENTION_SWEEP_LIMIT: usize = 1_000;

/// Versions marked for deletion that may be deleted at `at`, oldest retention first,
/// after the `(retainUntil, id)` keyset position of `after`.
async fn due_for_deletion<C: ConnectionTrait>(
    db: &C,
    at: DateTime<Utc>,
    after: Option<&locked_item_version::Model>,
) -> Result<Vec<locked_item_version::Model>, ApiError> {
    let mut query = locked_item_version::Entity::find()
        .filter(locked_item_version::Column::Status.eq(LockedVersionStatus::Locked))
        .filter(locked_item_version::Column::DeleteAfterRetention.eq(true))
        .filter(locked_item_version::Column::LegalHold.eq(false))
        .filter(locked_item_version::Column::RetainUntil.lte(db_time(at)));
    if let Some(last) = after {
        query = query.filter(
            Condition::any()
                .add(locked_item_version::Column::RetainUntil.gt(last.retain_until))
                .add(
                    Condition::all()
                        .add(locked_item_version::Column::RetainUntil.eq(last.retain_until))
                        .add(locked_item_version::Column::Id.gt(last.id.clone())),
                ),
        );
    }
    Ok(query
        .order_by_asc(locked_item_version::Column::RetainUntil)
        .order_by_asc(locked_item_version::Column::Id)
        .limit(SWEEP_BATCH)
        .all(db)
        .await?)
}

/// Purges versions uploaded with `delete_after_retention` once their retention ended
/// and no legal hold remains. A version the store still refuses is retried next run;
/// the keyset walk keeps it from starving the versions behind it.
pub async fn sweep_expired_versions(state: &AppState) -> Result<u64, ApiError> {
    let Some(storage) = state.locked_storage.as_deref() else {
        return Ok(0);
    };
    let at = now();
    let mut purged = 0;
    let mut seen = 0;
    let mut after = None;
    loop {
        let page = due_for_deletion(&state.db, at, after.as_ref()).await?;
        seen += page.len();
        let last_page = page.len() < SWEEP_BATCH as usize || seen >= RETENTION_SWEEP_LIMIT;
        for version in &page {
            match purge_due(state, storage, version, at).await {
                Ok(()) => purged += 1,
                Err(error) => {
                    tracing::warn!(%error, version_id = %version.id, "Locked version due for deletion could not be purged");
                }
            }
        }
        if last_page {
            return Ok(purged);
        }
        after = page.into_iter().last();
    }
}

async fn purge_due(
    state: &AppState,
    storage: &LockedStorage,
    version: &locked_item_version::Model,
    at: DateTime<Utc>,
) -> Result<(), ApiError> {
    let item = load_item(state, &version.app_id, &version.item_id).await?;
    purge_version(state, storage, &item, version, RETENTION_PURGER, at).await?;
    crate::audit::record_entry(
        state,
        crate::audit::AuditRecordInput {
            actor_id: RETENTION_PURGER.to_owned(),
            actor_type: AuditActorType::System,
            actor_ip: None,
            action: "locked_file.version.purge".to_owned(),
            resource_type: "LockedFile".to_owned(),
            resource_id: item.id.clone(),
            scope: Some(item.app_id.clone()),
            details: Some(serde_json::json!({
                "version_id": version.id,
                "version": version.version,
                "sha256": version.sha256,
                "size": version.size,
                "delete_after_retention": true,
            })),
        },
    )
    .await;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(feature = "aws")]
    #[flow_like_types::tokio::test]
    async fn settings_come_from_the_secret_store_document() {
        use flow_like_secrets::{FileProviderConfig, ProviderConfig, SecretStoreConfig};
        let store = |path: &std::path::Path| {
            SecretStore::new(
                SecretStoreConfig::default()
                    .with_allow_env_override(false)
                    .with_provider(ProviderConfig::File(FileProviderConfig {
                        root_path: path.to_owned(),
                        trim_trailing_newline: true,
                    })),
            )
            .unwrap()
        };
        let dir = tempfile::tempdir().unwrap();
        assert!(
            LockedStorage::load(&store(dir.path()))
                .await
                .unwrap()
                .is_none()
        );

        std::fs::write(
            dir.path().join(SETTINGS_DOCUMENT),
            r#"{"LOCKED_BUCKET":"doc-locked","LOCKED_BUCKET_PROVIDER":"s3","LOCKED_BUCKET_RETENTION_DAYS":"2920"}"#,
        )
        .unwrap();
        let storage = LockedStorage::load(&store(dir.path()))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(storage.config.bucket, "doc-locked");
        assert_eq!(storage.config.default_retention_days, 2920);

        std::fs::write(
            dir.path().join(SETTINGS_DOCUMENT),
            r#"{"LOCKED_BUCKETS":"typo"}"#,
        )
        .unwrap();
        assert!(LockedStorage::load(&store(dir.path())).await.is_err());
    }

    #[test]
    fn executor_callers_keep_the_use_their_route_declared() {
        let source = include_str!("service.rs");
        let body = source
            .split_once("\n    pub async fn for_executor(")
            .expect("executor callers are built here")
            .1;
        let body = body.split_once("\n    }\n").map_or(body, |(body, _)| body);
        assert!(body.contains(".execution_app_permission(app_id, state, executor_use)"));
    }

    #[test]
    fn sha256_must_be_hex_of_32_bytes() {
        let digest = "a".repeat(64);
        assert_eq!(parse_sha256(&digest).unwrap(), [0xaa; 32]);
        assert_eq!(parse_sha256(&digest.to_uppercase()).unwrap(), [0xaa; 32]);
        assert!(parse_sha256("abc").is_err());
        assert!(parse_sha256(&"g".repeat(64)).is_err());
    }

    #[test]
    fn cursors_round_trip() {
        let item = locked_item::Model {
            id: "i1".into(),
            app_id: "a".into(),
            scope: LockedItemScope::Project,
            owner_id: None,
            path_key: "k".into(),
            name: "Übersicht.pdf".into(),
            folder: String::new(),
            content_type: None,
            latest_version: 1,
            latest_size: 1,
            live_versions: 1,
            live_size: 1,
            metadata: None,
            created_by: "u".into(),
            created_at: db_time(now()),
            updated_at: db_time(now()),
        };
        let cursor = decode_cursor(&encode_cursor(&item)).unwrap();
        assert_eq!(
            (cursor.name.as_str(), cursor.id.as_str()),
            ("Übersicht.pdf", "i1")
        );
        assert!(decode_cursor("not a cursor").is_err());
    }

    /// The entities pin the `public` schema, so each run gets its own database.
    async fn scratch_database(url: &str) -> sea_orm::DatabaseConnection {
        use sea_orm::sqlx::postgres::{PgConnectOptions, PgPoolOptions};
        use sea_orm::{Database, DatabaseBackend};
        use std::str::FromStr;
        let name = format!("locked_{}", create_id());
        Database::connect(url)
            .await
            .unwrap()
            .execute_raw(Statement::from_string(
                DatabaseBackend::Postgres,
                format!("CREATE DATABASE {name}"),
            ))
            .await
            .unwrap();
        let pool = PgPoolOptions::new()
            .max_connections(4)
            .connect_with(PgConnectOptions::from_str(url).unwrap().database(&name))
            .await
            .unwrap();
        let db = sea_orm::DatabaseConnection::from(pool);
        let fixtures = [
            r#"CREATE TABLE "User" (id TEXT PRIMARY KEY, "totalSize" BIGINT NOT NULL DEFAULT 0)"#,
            r#"CREATE TABLE "App" (id TEXT PRIMARY KEY, "ownerRoleId" TEXT, "totalSize" BIGINT NOT NULL DEFAULT 0)"#,
            r#"CREATE TABLE "Membership" ("userId" TEXT, "roleId" TEXT, "appId" TEXT)"#,
            r#"CREATE TABLE "FileAccountingObject" (id TEXT PRIMARY KEY)"#,
        ];
        let migrations = [
            include_str!("../../prisma/migrations/20260913120002_account_capacity/migration.sql"),
            include_str!("../../prisma/migrations/20261021140000_locked_storage/migration.sql"),
            include_str!(
                "../../prisma/migrations/20261022120000_locked_delete_after_retention/migration.sql"
            ),
        ];
        let statements = fixtures.into_iter().map(str::to_owned).chain(
            migrations
                .iter()
                .flat_map(|sql| sql.split(';'))
                .map(|sql| {
                    sql.lines()
                        .filter(|line| !line.trim_start().starts_with("--"))
                        .collect::<Vec<_>>()
                        .join("\n")
                })
                .filter(|sql| !sql.trim().is_empty()),
        );
        for statement in statements {
            db.execute_raw(Statement::from_string(DatabaseBackend::Postgres, statement))
                .await
                .unwrap();
        }
        for statement in [
            r#"INSERT INTO "User" (id) VALUES ('payer'), ('member')"#,
            r#"INSERT INTO "App" (id, "ownerRoleId") VALUES ('app', 'owner')"#,
            r#"INSERT INTO "Membership" VALUES ('payer', 'owner', 'app')"#,
            r#"INSERT INTO "ProjectCapacity" ("appId", "payerId", counted) VALUES ('app', 'payer', true)"#,
            r#"INSERT INTO "AccountCapacity" ("payerId", initialized) VALUES ('payer', true)"#,
        ] {
            db.execute_raw(Statement::from_string(DatabaseBackend::Postgres, statement))
                .await
                .unwrap();
        }
        db
    }

    async fn pending(
        db: &sea_orm::DatabaseConnection,
        item: &locked_item::Model,
        size: i64,
    ) -> locked_item_version::Model {
        let at = now();
        let id = create_id();
        locked_item_version::ActiveModel {
            id: Set(id.clone()),
            item_id: Set(item.id.clone()),
            app_id: Set(item.app_id.clone()),
            version: Set(None),
            status: Set(LockedVersionStatus::Pending),
            object_key: Set(
                object_key("app", item.scope, item.owner_id.as_deref(), &item.id, &id).unwrap(),
            ),
            provider_version_id: Set(None),
            file_name: Set("Übersicht (2)#1.pdf".into()),
            content_type: Set("application/pdf".into()),
            size: Set(size),
            sha256: Set("ab".repeat(32)),
            checksum_verified: Set(false),
            retention_mode: Set(
                crate::entity::sea_orm_active_enums::LockedRetentionMode::Compliance,
            ),
            retain_until: Set(db_time(at + chrono::Duration::days(10))),
            enforcement: Set(LockedEnforcement::Object),
            legal_hold: Set(false),
            delete_after_retention: Set(None),
            comment: Set(None),
            metadata: Set(Some(serde_json::json!({ "invoice": 42 }))),
            uploaded_by: Set("member".into()),
            upload_expires_at: Set(db_time(at)),
            created_at: Set(db_time(at)),
            locked_at: Set(None),
            purged_at: Set(None),
            purged_by: Set(None),
        }
        .insert(db)
        .await
        .unwrap()
    }

    async fn locked_version(
        db: &sea_orm::DatabaseConnection,
        item: &locked_item::Model,
        retain_until: DateTime<Utc>,
        delete_after_retention: Option<bool>,
        legal_hold: bool,
        status: LockedVersionStatus,
    ) -> locked_item_version::Model {
        let mut version: locked_item_version::ActiveModel = pending(db, item, 10).await.into();
        version.status = Set(status);
        version.retain_until = Set(db_time(retain_until));
        version.delete_after_retention = Set(delete_after_retention);
        version.legal_hold = Set(legal_hold);
        version.update(db).await.unwrap()
    }

    async fn user_item(db: &sea_orm::DatabaseConnection) -> locked_item::Model {
        let at = db_time(now());
        locked_item::ActiveModel {
            id: Set(create_id()),
            app_id: Set("app".into()),
            scope: Set(LockedItemScope::User),
            owner_id: Set(Some("member".into())),
            path_key: Set(path_key(
                LockedItemScope::User,
                Some("member"),
                "Übersicht",
                "Übersicht (2)#1.pdf",
            )),
            name: Set("Übersicht (2)#1.pdf".into()),
            folder: Set("Übersicht".into()),
            content_type: Set(None),
            latest_version: Set(0),
            latest_size: Set(0),
            live_versions: Set(0),
            live_size: Set(0),
            metadata: Set(None),
            created_by: Set("member".into()),
            created_at: Set(at),
            updated_at: Set(at),
        }
        .insert(db)
        .await
        .unwrap()
    }

    async fn scalar(db: &sea_orm::DatabaseConnection, sql: &str) -> i64 {
        db.query_one_raw(Statement::from_string(
            sea_orm::DatabaseBackend::Postgres,
            sql,
        ))
        .await
        .unwrap()
        .unwrap()
        .try_get_by_index(0)
        .unwrap()
    }

    #[flow_like_types::tokio::test]
    async fn postgres_commits_number_versions_charge_and_purge_credit_storage() {
        use sea_orm::TransactionTrait;
        let Ok(url) = std::env::var("FLOW_LIKE_TEST_DATABASE_URL") else {
            eprintln!(
                "skipping locked storage integration test: FLOW_LIKE_TEST_DATABASE_URL is unset"
            );
            return;
        };
        let db = scratch_database(&url).await;
        let item = user_item(&db).await;

        let first = pending(&db, &item, 100).await;
        let txn = db.begin().await.unwrap();
        let committed = commit_rows(&txn, &item, &first, Some("v-1".into()), true, now(), now())
            .await
            .unwrap();
        txn.commit().await.unwrap();
        assert!(committed);

        let stale = pending(&db, &item, 50).await;
        let txn = db.begin().await.unwrap();
        assert!(
            !commit_rows(&txn, &item, &stale, None, true, now(), now())
                .await
                .unwrap()
        );
        txn.rollback().await.unwrap();

        let item = locked_item::Entity::find_by_id(item.id.clone())
            .one(&db)
            .await
            .unwrap()
            .unwrap();
        let txn = db.begin().await.unwrap();
        assert!(
            commit_rows(&txn, &item, &stale, None, false, now(), now())
                .await
                .unwrap()
        );
        txn.commit().await.unwrap();

        let item = locked_item::Entity::find_by_id(item.id.clone())
            .one(&db)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            (
                item.latest_version,
                item.live_versions,
                item.live_size,
                item.latest_size
            ),
            (2, 2, 150, 50)
        );
        let stored = locked_item_version::Entity::find_by_id(first.id.clone())
            .one(&db)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            (
                stored.version,
                stored.status,
                stored.provider_version_id.as_deref()
            ),
            (Some(1), LockedVersionStatus::Locked, Some("v-1"))
        );
        assert_eq!(
            scalar(
                &db,
                r#"SELECT "storageBytes" FROM "AccountCapacity" WHERE "payerId"='payer'"#
            )
            .await,
            150
        );
        assert_eq!(
            scalar(&db, r#"SELECT "totalSize" FROM "App" WHERE id='app'"#).await,
            150
        );
        assert_eq!(
            scalar(&db, r#"SELECT "totalSize" FROM "User" WHERE id='member'"#).await,
            150
        );

        let newest = newest_stored_versions(&db, std::slice::from_ref(&item.id))
            .await
            .unwrap();
        assert_eq!(newest[&item.id].version, Some(2));

        let txn = db.begin().await.unwrap();
        purge_rows(&txn, &item, &stored, "payer", now())
            .await
            .unwrap();
        txn.commit().await.unwrap();
        let item = locked_item::Entity::find_by_id(item.id.clone())
            .one(&db)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            (
                item.latest_version,
                item.live_versions,
                item.live_size,
                item.latest_size
            ),
            (2, 1, 50, 50)
        );
        let purged = locked_item_version::Entity::find_by_id(first.id.clone())
            .one(&db)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            (purged.status, purged.purged_by.as_deref()),
            (LockedVersionStatus::Purged, Some("payer"))
        );
        assert_eq!(
            scalar(
                &db,
                r#"SELECT "storageBytes" FROM "AccountCapacity" WHERE "payerId"='payer'"#
            )
            .await,
            50
        );
        assert_eq!(
            scalar(&db, r#"SELECT "totalSize" FROM "User" WHERE id='member'"#).await,
            50
        );

        let txn = db.begin().await.unwrap();
        purge_rows(&txn, &item, &stored, "payer", now())
            .await
            .unwrap();
        txn.commit().await.unwrap();
        assert_eq!(
            scalar(
                &db,
                r#"SELECT "storageBytes" FROM "AccountCapacity" WHERE "payerId"='payer'"#
            )
            .await,
            50
        );
    }

    #[flow_like_types::tokio::test]
    async fn postgres_finds_only_versions_due_for_deletion_in_keyset_order() {
        let Ok(url) = std::env::var("FLOW_LIKE_TEST_DATABASE_URL") else {
            eprintln!(
                "skipping locked storage integration test: FLOW_LIKE_TEST_DATABASE_URL is unset"
            );
            return;
        };
        let db = scratch_database(&url).await;
        let item = user_item(&db).await;
        let at = now();
        let ended = at - chrono::Duration::days(1);
        let locked = LockedVersionStatus::Locked;

        let mut due = vec![
            locked_version(&db, &item, ended, Some(true), false, locked).await,
            locked_version(&db, &item, ended, Some(true), false, locked).await,
            locked_version(
                &db,
                &item,
                at - chrono::Duration::days(2),
                Some(true),
                false,
                locked,
            )
            .await,
        ];
        for (retain_until, delete_after_retention, legal_hold, status) in [
            (at + chrono::Duration::days(1), Some(true), false, locked),
            (ended, Some(false), false, locked),
            (ended, None, false, locked),
            (ended, Some(true), true, locked),
            (ended, Some(true), false, LockedVersionStatus::Purged),
            (ended, Some(true), false, LockedVersionStatus::Pending),
        ] {
            locked_version(
                &db,
                &item,
                retain_until,
                delete_after_retention,
                legal_hold,
                status,
            )
            .await;
        }
        due.sort_by(|a, b| (a.retain_until, &a.id).cmp(&(b.retain_until, &b.id)));
        let ids = |versions: &[locked_item_version::Model]| {
            versions
                .iter()
                .map(|version| version.id.clone())
                .collect::<Vec<_>>()
        };

        let found = due_for_deletion(&db, at, None).await.unwrap();
        assert_eq!(ids(&found), ids(&due));
        let rest = due_for_deletion(&db, at, Some(&found[1])).await.unwrap();
        assert_eq!(ids(&rest), ids(&due[2..]));
        assert!(
            due_for_deletion(&db, at, Some(&found[2]))
                .await
                .unwrap()
                .is_empty()
        );
    }
}
