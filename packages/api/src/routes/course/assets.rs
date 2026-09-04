use crate::{
    entity::{course_asset, sea_orm_active_enums::AssetKind},
    error::ApiError,
    middleware::jwt::AppUser,
    permission::global_permission::GlobalPermission,
    routes::course::access::{ensure_course_exists, ensure_course_readable},
    state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, Query, State},
};
use flow_like_storage::Path as FlowPath;
use flow_like_types::{anyhow, create_id};
use sea_orm::{
    ActiveModelTrait, ActiveValue::Set, ColumnTrait, EntityTrait, IntoActiveModel, QueryFilter,
    QueryOrder,
};
use serde::{Deserialize, Serialize};
use std::time::Duration;
use utoipa::ToSchema;

const ASSET_NAME_MAX_LEN: usize = 64;
const SIGNED_URL_TTL_SECS: u64 = 60 * 60 * 24;

#[derive(Clone, Copy, Debug, Deserialize, Serialize, ToSchema, PartialEq, Eq)]
#[serde(rename_all = "UPPERCASE")]
pub enum CourseAssetKind {
    Image,
    Video,
    Audio,
    Document,
}

impl From<AssetKind> for CourseAssetKind {
    fn from(value: AssetKind) -> Self {
        match value {
            AssetKind::Image => CourseAssetKind::Image,
            AssetKind::Video => CourseAssetKind::Video,
            AssetKind::Audio => CourseAssetKind::Audio,
            AssetKind::Document => CourseAssetKind::Document,
        }
    }
}

impl From<CourseAssetKind> for AssetKind {
    fn from(value: CourseAssetKind) -> Self {
        match value {
            CourseAssetKind::Image => AssetKind::Image,
            CourseAssetKind::Video => AssetKind::Video,
            CourseAssetKind::Audio => AssetKind::Audio,
            CourseAssetKind::Document => AssetKind::Document,
        }
    }
}

#[derive(Clone, Debug, Serialize, ToSchema)]
pub struct CourseAssetView {
    pub id: String,
    pub course_id: String,
    pub name: String,
    pub filename: String,
    pub mime_type: String,
    pub size: i32,
    pub kind: CourseAssetKind,
    pub created_at: String,
    pub updated_at: String,
}

impl From<course_asset::Model> for CourseAssetView {
    fn from(value: course_asset::Model) -> Self {
        Self {
            id: value.id,
            course_id: value.course_id,
            name: value.name,
            filename: value.filename,
            mime_type: value.mime_type,
            size: value.size,
            kind: value.kind.into(),
            created_at: value.created_at.to_rfc3339(),
            updated_at: value.updated_at.to_rfc3339(),
        }
    }
}

#[derive(Clone, Debug, Deserialize, ToSchema)]
pub struct CreateCourseAssetBody {
    pub name: String,
    pub filename: String,
    pub mime_type: String,
    pub size: i32,
    pub kind: CourseAssetKind,
    pub extension: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct CreateCourseAssetResponse {
    pub asset: CourseAssetView,
    pub signed_url: String,
}

#[derive(Clone, Debug, Deserialize, ToSchema)]
pub struct UpdateCourseAssetBody {
    pub name: String,
}

pub fn course_asset_storage_path(course_id: &str, file_name: &str) -> FlowPath {
    FlowPath::from("media")
        .child("courses")
        .child(course_id)
        .child("assets")
        .child(file_name)
}

fn normalize_extension(extension: &str) -> Result<String, ApiError> {
    let extension = extension
        .trim()
        .trim_start_matches('.')
        .to_ascii_lowercase();
    if extension.is_empty()
        || extension.len() > 10
        || !extension.chars().all(|ch| ch.is_ascii_alphanumeric())
    {
        return Err(ApiError::bad_request("Invalid asset extension"));
    }
    Ok(extension)
}

fn validate_asset_name(name: &str) -> Result<String, ApiError> {
    let trimmed = name.trim();
    if trimmed.is_empty() || trimmed.len() > ASSET_NAME_MAX_LEN {
        return Err(ApiError::bad_request(
            "Asset name must be between 1 and 64 characters",
        ));
    }
    let first = trimmed.chars().next().unwrap_or('_');
    if !first.is_ascii_alphabetic() && first != '_' {
        return Err(ApiError::bad_request(
            "Asset name must start with a letter or underscore",
        ));
    }
    if !trimmed
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    {
        return Err(ApiError::bad_request(
            "Asset name may only contain letters, digits, underscores and dashes",
        ));
    }
    Ok(trimmed.to_string())
}

#[derive(Clone, Debug, Deserialize, ToSchema, Default)]
pub struct ListCourseAssetsQuery {
    pub kind: Option<CourseAssetKind>,
}

#[utoipa::path(
    get,
    path = "/courses/{course_id}/assets",
    tag = "courses",
    description = "List uploaded assets for a course. Authors can reference assets in lesson content via @AssetName.",
    params(
        ("course_id" = String, Path, description = "Course identifier"),
        ("kind" = Option<String>, Query, description = "Filter by asset kind: IMAGE, VIDEO, AUDIO, DOCUMENT")
    ),
    responses(
        (status = 200, description = "List of course assets", body = Vec<CourseAssetView>),
        (status = 403, description = "Forbidden"),
        (status = 404, description = "Course not found")
    )
)]
#[tracing::instrument(name = "GET /courses/{course_id}/assets", skip(state, user, query))]
pub async fn list_course_assets(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(course_id): Path<String>,
    Query(query): Query<ListCourseAssetsQuery>,
) -> Result<Json<Vec<CourseAssetView>>, ApiError> {
    ensure_course_readable(&state, &user, &course_id).await?;

    let mut q = course_asset::Entity::find()
        .filter(course_asset::Column::CourseId.eq(&course_id))
        .order_by_asc(course_asset::Column::Name);
    if let Some(kind) = query.kind {
        q = q.filter(course_asset::Column::Kind.eq(AssetKind::from(kind)));
    }
    let assets = q.all(&state.db).await?;
    Ok(Json(assets.into_iter().map(Into::into).collect()))
}

#[utoipa::path(
    post,
    path = "/courses/{course_id}/assets",
    tag = "courses",
    description = "Create a course asset record and return a signed upload URL. The client uploads the file directly to the returned URL.",
    params(("course_id" = String, Path, description = "Course identifier")),
    request_body = CreateCourseAssetBody,
    responses(
        (status = 200, description = "Asset created with upload URL", body = CreateCourseAssetResponse),
        (status = 400, description = "Invalid asset metadata"),
        (status = 403, description = "Forbidden"),
        (status = 404, description = "Course not found"),
        (status = 409, description = "An asset with this name already exists")
    )
)]
#[tracing::instrument(name = "POST /courses/{course_id}/assets", skip(state, user, body))]
pub async fn create_course_asset(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(course_id): Path<String>,
    Json(body): Json<CreateCourseAssetBody>,
) -> Result<Json<CreateCourseAssetResponse>, ApiError> {
    user.check_global_permission(&state, GlobalPermission::WriteCourses)
        .await?;
    ensure_course_exists(&state, &course_id).await?;

    let name = validate_asset_name(&body.name)?;
    let extension = normalize_extension(&body.extension)?;
    if body.size < 0 {
        return Err(ApiError::bad_request("Asset size must be non-negative"));
    }
    let mime_type = body.mime_type.trim();
    if mime_type.is_empty() || mime_type.len() > 255 {
        return Err(ApiError::bad_request("Invalid mime type"));
    }
    let filename = body.filename.trim();
    if filename.is_empty() || filename.len() > 255 {
        return Err(ApiError::bad_request("Invalid filename"));
    }

    let existing = course_asset::Entity::find()
        .filter(course_asset::Column::CourseId.eq(&course_id))
        .filter(course_asset::Column::Name.eq(&name))
        .one(&state.db)
        .await?;
    if existing.is_some() {
        return Err(ApiError::conflict(
            "An asset with this name already exists for this course",
        ));
    }

    let asset_id = create_id();
    let storage_key = format!("{asset_id}.{extension}");
    let now = chrono::Utc::now().fixed_offset();

    let active = course_asset::ActiveModel {
        id: Set(asset_id.clone()),
        course_id: Set(course_id.clone()),
        name: Set(name),
        storage_key: Set(storage_key.clone()),
        filename: Set(filename.to_string()),
        mime_type: Set(mime_type.to_string()),
        size: Set(body.size),
        kind: Set(AssetKind::from(body.kind)),
        created_at: Set(now),
        updated_at: Set(now),
    };
    let saved = active.insert(&state.db).await?;

    let master_store = state.master_credentials().await?;
    let master_store = master_store.to_store(false).await?;
    let path = course_asset_storage_path(&course_id, &storage_key);
    let signed_url = master_store
        .sign("PUT", &path, Duration::from_secs(SIGNED_URL_TTL_SECS))
        .await
        .map_err(|e| {
            let id = create_id();
            tracing::error!(
                "[{}] Failed to sign upload URL for course asset '{}' - {:?}",
                id,
                storage_key,
                e
            );
            ApiError::internal_error(anyhow!(
                "Failed to create signed upload URL, reference ID: {}",
                id
            ))
        })?;

    Ok(Json(CreateCourseAssetResponse {
        asset: saved.into(),
        signed_url: signed_url.to_string(),
    }))
}

#[utoipa::path(
    put,
    path = "/courses/{course_id}/assets/{asset_id}",
    tag = "courses",
    description = "Rename a course asset. References that use the old name in lesson content will need to be updated.",
    params(
        ("course_id" = String, Path, description = "Course identifier"),
        ("asset_id" = String, Path, description = "Asset identifier")
    ),
    request_body = UpdateCourseAssetBody,
    responses(
        (status = 200, description = "Asset updated", body = CourseAssetView),
        (status = 400, description = "Invalid asset name"),
        (status = 403, description = "Forbidden"),
        (status = 404, description = "Asset or course not found"),
        (status = 409, description = "Another asset already uses this name")
    )
)]
#[tracing::instrument(
    name = "PUT /courses/{course_id}/assets/{asset_id}",
    skip(state, user, body)
)]
pub async fn update_course_asset(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((course_id, asset_id)): Path<(String, String)>,
    Json(body): Json<UpdateCourseAssetBody>,
) -> Result<Json<CourseAssetView>, ApiError> {
    user.check_global_permission(&state, GlobalPermission::WriteCourses)
        .await?;

    let asset = course_asset::Entity::find_by_id(&asset_id)
        .one(&state.db)
        .await?
        .ok_or(ApiError::NOT_FOUND)?;
    if asset.course_id != course_id {
        return Err(ApiError::NOT_FOUND);
    }

    let new_name = validate_asset_name(&body.name)?;
    if new_name != asset.name {
        let conflict = course_asset::Entity::find()
            .filter(course_asset::Column::CourseId.eq(&course_id))
            .filter(course_asset::Column::Name.eq(&new_name))
            .one(&state.db)
            .await?;
        if conflict.is_some() {
            return Err(ApiError::conflict(
                "Another asset already uses this name for this course",
            ));
        }
    }

    let mut active = asset.into_active_model();
    active.name = Set(new_name);
    active.updated_at = Set(chrono::Utc::now().fixed_offset());
    let saved = active.update(&state.db).await?;
    Ok(Json(saved.into()))
}

#[derive(Clone, Debug, Serialize, ToSchema)]
pub struct OptimizeCourseAssetResponse {
    pub asset: CourseAssetView,
    pub previous_size: i32,
    pub previous_mime_type: String,
}

#[utoipa::path(
    post,
    path = "/courses/{course_id}/assets/{asset_id}/optimize",
    tag = "courses",
    description = "Re-encode an image asset as WebP (quality 85). The original storage object is deleted and the asset record is updated in place.",
    params(
        ("course_id" = String, Path, description = "Course identifier"),
        ("asset_id" = String, Path, description = "Asset identifier")
    ),
    responses(
        (status = 200, description = "Asset optimized", body = OptimizeCourseAssetResponse),
        (status = 400, description = "Asset is not optimizable (wrong kind, already WebP, or undecodable)"),
        (status = 403, description = "Forbidden"),
        (status = 404, description = "Asset or course not found")
    )
)]
#[tracing::instrument(
    name = "POST /courses/{course_id}/assets/{asset_id}/optimize",
    skip(state, user)
)]
pub async fn optimize_course_asset(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((course_id, asset_id)): Path<(String, String)>,
) -> Result<Json<OptimizeCourseAssetResponse>, ApiError> {
    user.check_global_permission(&state, GlobalPermission::WriteCourses)
        .await?;

    let asset = course_asset::Entity::find_by_id(&asset_id)
        .one(&state.db)
        .await?
        .ok_or(ApiError::NOT_FOUND)?;
    if asset.course_id != course_id {
        return Err(ApiError::NOT_FOUND);
    }
    if asset.kind != AssetKind::Image {
        return Err(ApiError::bad_request(
            "Only image assets can be optimized to WebP",
        ));
    }
    if asset.mime_type.eq_ignore_ascii_case("image/webp") {
        return Err(ApiError::bad_request("Asset is already in WebP format"));
    }

    let previous_size = asset.size;
    let previous_mime_type = asset.mime_type.clone();
    let previous_storage_key = asset.storage_key.clone();

    let creds = state.master_credentials().await?;
    let store = creds.to_store(false).await?;
    let old_path = course_asset_storage_path(&course_id, &previous_storage_key);

    let bytes = store
        .as_generic()
        .get(&old_path)
        .await
        .map_err(|e| ApiError::internal_error(anyhow!("Failed to fetch original asset: {e}")))?
        .bytes()
        .await
        .map_err(|e| {
            ApiError::internal_error(anyhow!("Failed to read original asset bytes: {e}"))
        })?;

    let bytes_for_encode = bytes.to_vec();
    let webp_bytes =
        flow_like_types::tokio::task::spawn_blocking(move || encode_to_webp(&bytes_for_encode))
            .await
            .map_err(|e| ApiError::internal_error(anyhow!("Image encoder panic: {e}")))??;

    let new_storage_key = format!("{}.webp", create_id());
    let new_path = course_asset_storage_path(&course_id, &new_storage_key);
    let new_size = webp_bytes.len();
    if new_size > i32::MAX as usize {
        return Err(ApiError::bad_request("Optimized image is too large"));
    }

    store
        .put(&new_path, bytes::Bytes::from(webp_bytes))
        .await
        .map_err(|e| ApiError::internal_error(anyhow!("Failed to upload optimized asset: {e}")))?;

    let new_filename = match asset.filename.rsplit_once('.') {
        Some((stem, _ext)) if !stem.is_empty() => format!("{stem}.webp"),
        _ => format!("{}.webp", asset.filename),
    };

    let mut active = asset.clone().into_active_model();
    active.storage_key = Set(new_storage_key);
    active.filename = Set(new_filename);
    active.mime_type = Set("image/webp".to_string());
    active.size = Set(new_size as i32);
    active.updated_at = Set(chrono::Utc::now().fixed_offset());
    let saved = match active.update(&state.db).await {
        Ok(saved) => saved,
        Err(err) => {
            // Roll back the new upload so we don't leak storage on DB failure.
            if let Err(cleanup_err) = store.as_generic().delete(&new_path).await {
                tracing::warn!(
                    "Failed to clean up orphaned optimized asset at {} after DB error: {:?}",
                    new_path,
                    cleanup_err
                );
            }
            return Err(err.into());
        }
    };

    if let Err(err) = store.as_generic().delete(&old_path).await {
        tracing::warn!(
            "Failed to delete original asset at {} after optimization: {:?}",
            old_path,
            err
        );
    }

    Ok(Json(OptimizeCourseAssetResponse {
        asset: saved.into(),
        previous_size,
        previous_mime_type,
    }))
}

fn encode_to_webp(bytes: &[u8]) -> Result<Vec<u8>, ApiError> {
    let dyn_img = image::load_from_memory(bytes)
        .map_err(|e| ApiError::bad_request(format!("Could not decode image: {e}")))?;
    let rgba = dyn_img.to_rgba8();
    let (width, height) = (rgba.width(), rgba.height());
    if width == 0 || height == 0 {
        return Err(ApiError::bad_request("Image has zero dimensions"));
    }
    let encoder = webp::Encoder::from_rgba(rgba.as_raw(), width, height);
    let memory = encoder.encode(85.0);
    Ok(memory.to_vec())
}

#[utoipa::path(
    delete,
    path = "/courses/{course_id}/assets/{asset_id}",
    tag = "courses",
    description = "Delete a course asset and remove the underlying file from storage. References to the asset in lesson content will stop resolving.",
    params(
        ("course_id" = String, Path, description = "Course identifier"),
        ("asset_id" = String, Path, description = "Asset identifier")
    ),
    responses(
        (status = 200, description = "Asset deleted"),
        (status = 403, description = "Forbidden"),
        (status = 404, description = "Asset or course not found")
    )
)]
#[tracing::instrument(
    name = "DELETE /courses/{course_id}/assets/{asset_id}",
    skip(state, user)
)]
pub async fn delete_course_asset(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((course_id, asset_id)): Path<(String, String)>,
) -> Result<Json<()>, ApiError> {
    user.check_global_permission(&state, GlobalPermission::WriteCourses)
        .await?;

    let asset = course_asset::Entity::find_by_id(&asset_id)
        .one(&state.db)
        .await?
        .ok_or(ApiError::NOT_FOUND)?;
    if asset.course_id != course_id {
        return Err(ApiError::NOT_FOUND);
    }

    let storage_key = asset.storage_key.clone();
    course_asset::Entity::delete_by_id(&asset_id)
        .exec(&state.db)
        .await?;

    if let Ok(creds) = state.master_credentials().await
        && let Ok(store) = creds.to_store(false).await
    {
        let path = course_asset_storage_path(&course_id, &storage_key);
        if let Err(err) = store.as_generic().delete(&path).await {
            tracing::warn!("Failed to delete course asset file at {}: {:?}", path, err);
        }
    }

    Ok(Json(()))
}
