use crate::entity::sea_orm_active_enums::{
    WasmPackageStatus, WasmPackageVisibility, WasmReviewAction,
};
use crate::entity::{meta, wasm_package, wasm_package_review};
use crate::error::ApiError;
use crate::middleware::jwt::AppUser;
use crate::routes::registry::server::PackageReview;
use crate::state::AppState;
use axum::extract::{Path, Query, State};
use axum::{Extension, Json};
use flow_like_storage::Path as FlowPath;
use flow_like_types::{anyhow, create_id};
use sea_orm::{
    ActiveModelTrait, ActiveValue::Set, ColumnTrait, EntityTrait, QueryFilter, TransactionTrait,
};
use serde::{Deserialize, Serialize};
use std::time::Duration;
use utoipa::ToSchema;

#[derive(Debug, Serialize, ToSchema)]
pub struct ReadmeResponse {
    pub readme: Option<String>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct UpdateReadmeRequest {
    pub readme: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct MessageResponse {
    pub message: String,
}

/// GET /registry/package/{package_id}/publication-reviews
/// Retrieve internal publication review history for package maintainers.
#[utoipa::path(
    get,
    path = "/registry/package/{package_id}/publication-reviews",
    tag = "registry",
    params(("package_id" = String, Path, description = "Package ID")),
    responses(
        (status = 200, description = "Package publication review history", body = [PackageReview]),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
        (status = 404, description = "Package not found"),
        (status = 503, description = "WASM registry not configured")
    )
)]
pub async fn get_publication_reviews(
    Extension(user): Extension<AppUser>,
    State(state): State<AppState>,
    Path(package_id): Path<String>,
) -> Result<Json<Vec<PackageReview>>, ApiError> {
    let user_id = user
        .sub()
        .map_err(|_| ApiError::unauthorized("Authentication required"))?;

    let registry = state
        .wasm_registry
        .as_ref()
        .ok_or_else(|| ApiError::service_unavailable("WASM registry not configured"))?;

    let _package = wasm_package::Entity::find_by_id(&package_id)
        .one(&state.db)
        .await?
        .ok_or(ApiError::NOT_FOUND)?;

    crate::ensure_wasm_permission!(
        state,
        &user_id,
        &package_id,
        WasmPackagePermission::Maintainer
    );

    Ok(Json(registry.get_reviews(&package_id).await?))
}

/// GET /registry/package/{package_id}/readme
/// Retrieve the readme for a package. Public packages are accessible to all authenticated users;
/// private packages require the caller to be a package user.
#[utoipa::path(
    get,
    path = "/registry/package/{package_id}/readme",
    tag = "registry",
    params(("package_id" = String, Path, description = "Package ID")),
    responses(
        (status = 200, description = "Package readme", body = ReadmeResponse),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
        (status = 404, description = "Package not found"),
        (status = 503, description = "WASM registry not configured")
    )
)]
pub async fn get_readme(
    Extension(user): Extension<AppUser>,
    State(state): State<AppState>,
    Path(package_id): Path<String>,
) -> Result<Json<ReadmeResponse>, ApiError> {
    let user_id = user
        .sub()
        .map_err(|_| ApiError::unauthorized("Authentication required"))?;

    let _registry = state
        .wasm_registry
        .as_ref()
        .ok_or_else(|| ApiError::service_unavailable("WASM registry not configured"))?;

    let package = wasm_package::Entity::find_by_id(&package_id)
        .one(&state.db)
        .await?
        .ok_or(ApiError::NOT_FOUND)?;

    if package.visibility == WasmPackageVisibility::Private {
        let access = crate::check_wasm_access!(state, &user_id, &package_id);
        if access.is_none() {
            return Err(ApiError::FORBIDDEN);
        }
    }

    Ok(Json(ReadmeResponse {
        readme: package.readme,
    }))
}

/// PUT /registry/package/{package_id}/readme
/// Update the readme for a package. Requires OWNER or MAINTAINER permission.
#[utoipa::path(
    put,
    path = "/registry/package/{package_id}/readme",
    tag = "registry",
    params(("package_id" = String, Path, description = "Package ID")),
    request_body = UpdateReadmeRequest,
    responses(
        (status = 200, description = "Readme updated", body = MessageResponse),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
        (status = 404, description = "Package not found"),
        (status = 503, description = "WASM registry not configured")
    )
)]
pub async fn update_readme(
    Extension(user): Extension<AppUser>,
    State(state): State<AppState>,
    Path(package_id): Path<String>,
    Json(body): Json<UpdateReadmeRequest>,
) -> Result<Json<MessageResponse>, ApiError> {
    let user_id = user
        .sub()
        .map_err(|_| ApiError::unauthorized("Authentication required"))?;

    let _registry = state
        .wasm_registry
        .as_ref()
        .ok_or_else(|| ApiError::service_unavailable("WASM registry not configured"))?;

    let _package = wasm_package::Entity::find_by_id(&package_id)
        .one(&state.db)
        .await?
        .ok_or(ApiError::NOT_FOUND)?;

    crate::ensure_wasm_permission!(
        state,
        &user_id,
        &package_id,
        WasmPackagePermission::Maintainer
    );

    let update = wasm_package::ActiveModel {
        id: Set(package_id.clone()),
        readme: Set(Some(body.readme.clone())),
        updated_at: Set(chrono::Utc::now().naive_utc()),
        ..Default::default()
    };
    update.update(&state.db).await?;

    Ok(Json(MessageResponse {
        message: "Readme updated".to_string(),
    }))
}

/// POST /registry/package/{package_id}/request-publication
/// Request publication of a private package. Only the package OWNER can request this.
/// The package must be active and currently private.
#[utoipa::path(
    post,
    path = "/registry/package/{package_id}/request-publication",
    tag = "registry",
    params(("package_id" = String, Path, description = "Package ID")),
    responses(
        (status = 200, description = "Publication requested", body = MessageResponse),
        (status = 400, description = "Package not eligible for publication"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
        (status = 404, description = "Package not found"),
        (status = 503, description = "WASM registry not configured")
    )
)]
pub async fn request_publication(
    Extension(user): Extension<AppUser>,
    State(state): State<AppState>,
    Path(package_id): Path<String>,
) -> Result<Json<MessageResponse>, ApiError> {
    let user_id = user
        .sub()
        .map_err(|_| ApiError::unauthorized("Authentication required"))?;

    let _registry = state
        .wasm_registry
        .as_ref()
        .ok_or_else(|| ApiError::service_unavailable("WASM registry not configured"))?;

    let package = wasm_package::Entity::find_by_id(&package_id)
        .one(&state.db)
        .await?
        .ok_or(ApiError::NOT_FOUND)?;

    crate::ensure_wasm_permission!(state, &user_id, &package_id, WasmPackagePermission::Owner);

    if package.status != WasmPackageStatus::Active {
        return Err(ApiError::bad_request("Package must be active"));
    }

    if package.visibility != WasmPackageVisibility::Private {
        return Err(ApiError::bad_request("Package is already public"));
    }

    wasm_package::ActiveModel {
        id: Set(package_id.clone()),
        status: Set(WasmPackageStatus::PendingReview),
        updated_at: Set(chrono::Utc::now().naive_utc()),
        ..Default::default()
    }
    .update(&state.db)
    .await?;

    let review = wasm_package_review::ActiveModel {
        id: Set(create_id()),
        package_id: Set(package_id.clone()),
        reviewer_id: Set(user_id),
        action: Set(WasmReviewAction::Submitted),
        comment: Set(Some("Publication requested".to_string())),
        internal_note: Set(None),
        security_score: Set(None),
        code_quality_score: Set(None),
        documentation_score: Set(None),
        created_at: Set(chrono::Utc::now().naive_utc()),
    };
    review.insert(&state.db).await?;

    Ok(Json(MessageResponse {
        message: "Publication review requested".to_string(),
    }))
}

// ---------------------------------------------------------------------------
// Package Meta (localized metadata)
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, ToSchema)]
pub struct MetaLangQuery {
    #[serde(default = "default_lang")]
    pub language: String,
}

fn default_lang() -> String {
    "en".to_string()
}

#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct UpsertPackageMetaRequest {
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub long_description: Option<String>,
    #[serde(default)]
    pub tags: Option<Vec<String>>,
    #[serde(default)]
    pub website: Option<String>,
    #[serde(default)]
    pub support_url: Option<String>,
    #[serde(default)]
    pub docs_url: Option<String>,
    #[serde(default)]
    pub use_case: Option<String>,
    #[serde(default)]
    pub release_notes: Option<String>,
    #[serde(default)]
    pub age_rating: Option<i64>,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct PackageMetaResponse {
    pub id: String,
    pub lang: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub long_description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tags: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thumbnail: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub website: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub support_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub docs_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub use_case: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub release_notes: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub preview_media: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub age_rating: Option<i64>,
}

impl From<meta::Model> for PackageMetaResponse {
    fn from(m: meta::Model) -> Self {
        Self {
            id: m.id,
            lang: m.lang,
            name: m.name,
            description: m.description,
            long_description: m.long_description,
            tags: m.tags,
            icon: m.icon,
            thumbnail: m.thumbnail,
            website: m.website,
            support_url: m.support_url,
            docs_url: m.docs_url,
            use_case: m.use_case,
            release_notes: m.release_notes,
            preview_media: m.preview_media,
            age_rating: m.age_rating,
        }
    }
}

/// GET /registry/package/{package_id}/meta
/// Get localized metadata for a package.
#[utoipa::path(
    get,
    path = "/registry/package/{package_id}/meta",
    tag = "registry",
    description = "Get localized metadata for a package.",
    params(
        ("package_id" = String, Path, description = "Package ID"),
        ("language" = Option<String>, Query, description = "Language code (default: en)")
    ),
    responses(
        (status = 200, description = "Package meta", body = PackageMetaResponse),
        (status = 401, description = "Unauthorized"),
        (status = 404, description = "Not found"),
        (status = 503, description = "WASM registry not configured")
    )
)]
pub async fn get_meta(
    Extension(user): Extension<AppUser>,
    State(state): State<AppState>,
    Path(package_id): Path<String>,
    Query(query): Query<MetaLangQuery>,
) -> Result<Json<PackageMetaResponse>, ApiError> {
    let sub = user.sub()?;

    state
        .wasm_registry
        .as_ref()
        .ok_or_else(|| ApiError::service_unavailable("WASM registry not configured"))?;

    let package = wasm_package::Entity::find_by_id(&package_id)
        .one(&state.db)
        .await?
        .ok_or(ApiError::NOT_FOUND)?;

    if package.visibility == WasmPackageVisibility::Private {
        let access = crate::check_wasm_access!(state, &sub, &package_id);
        if access.is_none() {
            return Err(ApiError::FORBIDDEN);
        }
    }

    let metas = meta::Entity::find()
        .filter(meta::Column::WasmPackageId.eq(&package_id))
        .filter(
            meta::Column::Lang
                .eq(&query.language)
                .or(meta::Column::Lang.eq("en")),
        )
        .all(&state.db)
        .await?;

    let best = metas
        .iter()
        .find(|m| m.lang == query.language)
        .or_else(|| metas.iter().find(|m| m.lang == "en"))
        .or_else(|| metas.first())
        .ok_or(ApiError::NOT_FOUND)?;

    let mut resp = PackageMetaResponse::from(best.clone());

    // Presign icon, thumbnail and preview_media CUIDs to full URLs
    let prefix = FlowPath::from("media")
        .child("packages")
        .child(package_id.as_str());
    if let Ok(master_creds) = state.master_credentials().await
        && let Ok(store) = master_creds.to_store(false).await
    {
        if let Some(icon) = &resp.icon
            && !icon.starts_with("http://")
            && !icon.starts_with("https://")
        {
            let path = prefix.child(format!("{icon}.webp"));
            if let Ok(url) = store.sign("GET", &path, Duration::from_secs(86400)).await {
                resp.icon = Some(url.to_string());
            }
        }
        if let Some(thumb) = &resp.thumbnail
            && !thumb.starts_with("http://")
            && !thumb.starts_with("https://")
        {
            let path = prefix.child(format!("{thumb}.webp"));
            if let Ok(url) = store.sign("GET", &path, Duration::from_secs(86400)).await {
                resp.thumbnail = Some(url.to_string());
            }
        }
        if let Some(media) = &mut resp.preview_media {
            for item in media.iter_mut() {
                if !item.starts_with("http://") && !item.starts_with("https://") {
                    let path = prefix.child(format!("{item}.webp"));
                    if let Ok(url) = store.sign("GET", &path, Duration::from_secs(86400)).await {
                        *item = url.to_string();
                    }
                }
            }
        }
    }

    Ok(Json(resp))
}

/// PUT /registry/package/{package_id}/meta
/// Create or update localized metadata for a package. Requires Maintainer permission.
#[utoipa::path(
    put,
    path = "/registry/package/{package_id}/meta",
    tag = "registry",
    description = "Create or update localized metadata for a package.",
    params(
        ("package_id" = String, Path, description = "Package ID"),
        ("language" = Option<String>, Query, description = "Language code (default: en)")
    ),
    request_body = UpsertPackageMetaRequest,
    responses(
        (status = 200, description = "Meta updated", body = PackageMetaResponse),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
        (status = 404, description = "Package not found"),
        (status = 503, description = "WASM registry not configured")
    )
)]
pub async fn upsert_meta(
    Extension(user): Extension<AppUser>,
    State(state): State<AppState>,
    Path(package_id): Path<String>,
    Query(query): Query<MetaLangQuery>,
    Json(body): Json<UpsertPackageMetaRequest>,
) -> Result<Json<PackageMetaResponse>, ApiError> {
    let sub = user.sub()?;

    state
        .wasm_registry
        .as_ref()
        .ok_or_else(|| ApiError::service_unavailable("WASM registry not configured"))?;

    crate::ensure_wasm_permission!(state, &sub, &package_id, WasmPackagePermission::Maintainer);

    let now = chrono::Utc::now().naive_utc();
    let lang = &query.language;

    let existing = meta::Entity::find()
        .filter(meta::Column::WasmPackageId.eq(&package_id))
        .filter(meta::Column::Lang.eq(lang))
        .one(&state.db)
        .await?;

    let result = if let Some(existing_meta) = existing {
        let mut active: meta::ActiveModel = existing_meta.clone().into();
        active.name = Set(body.name);
        active.description = Set(body.description);
        active.long_description = Set(body.long_description);
        active.tags = Set(body.tags);
        // Preserve icon, thumbnail and preview_media — managed via push/remove media endpoints
        active.icon = Set(existing_meta.icon);
        active.thumbnail = Set(existing_meta.thumbnail);
        active.preview_media = Set(existing_meta.preview_media);
        active.website = Set(body.website);
        active.support_url = Set(body.support_url);
        active.docs_url = Set(body.docs_url);
        active.use_case = Set(body.use_case);
        active.release_notes = Set(body.release_notes);
        active.age_rating = Set(body.age_rating);
        active.updated_at = Set(now);
        active.update(&state.db).await?
    } else {
        let new_meta = meta::ActiveModel {
            id: Set(create_id()),
            lang: Set(lang.clone()),
            name: Set(body.name),
            description: Set(body.description),
            long_description: Set(body.long_description),
            tags: Set(body.tags),
            icon: Set(None),
            thumbnail: Set(None),
            website: Set(body.website),
            support_url: Set(body.support_url),
            docs_url: Set(body.docs_url),
            use_case: Set(body.use_case),
            release_notes: Set(body.release_notes),
            preview_media: Set(None),
            age_rating: Set(body.age_rating),
            wasm_package_id: Set(Some(package_id)),
            app_id: Set(None),
            bit_id: Set(None),
            course_id: Set(None),
            template_id: Set(None),
            widget_id: Set(None),
            organization_specific_values: Set(None),
            created_at: Set(now),
            updated_at: Set(now),
        };
        new_meta.insert(&state.db).await?
    };

    Ok(Json(PackageMetaResponse::from(result)))
}

// ---------------------------------------------------------------------------
// Package Media (icon, thumbnail, preview_media)
// ---------------------------------------------------------------------------

/// Media item type for packages — reuses the same pattern as app media.
#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum PackageMediaItem {
    Icon,
    Thumbnail,
    Preview,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct PackageMediaQuery {
    #[serde(default = "default_lang")]
    pub language: String,
    pub item: PackageMediaItem,
    pub extension: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct PushMediaResponse {
    pub signed_url: String,
}

/// PUT /registry/package/{package_id}/meta/media
/// Get a signed upload URL for package media (icon, thumbnail, preview).
/// Uses the same storage path pattern as app media: `media/packages/{package_id}/{cuid}.{ext}`.
/// Image transformation Lambdas process files in the `media/` prefix.
#[utoipa::path(
    put,
    path = "/registry/package/{package_id}/meta/media",
    tag = "registry",
    description = "Get a signed upload URL for package media.",
    params(
        ("package_id" = String, Path, description = "Package ID"),
        ("language" = Option<String>, Query, description = "Language code (default en)"),
        ("item" = String, Query, description = "Media item: icon, thumbnail, preview"),
        ("extension" = String, Query, description = "File extension (e.g. png, webp)")
    ),
    responses(
        (status = 200, description = "Signed upload URL", body = PushMediaResponse),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
        (status = 404, description = "Not found"),
        (status = 503, description = "WASM registry not configured")
    )
)]
pub async fn push_package_media(
    Extension(user): Extension<AppUser>,
    State(state): State<AppState>,
    Path(package_id): Path<String>,
    Query(query): Query<PackageMediaQuery>,
) -> Result<Json<PushMediaResponse>, ApiError> {
    let user_id = user.sub()?;

    state
        .wasm_registry
        .as_ref()
        .ok_or_else(|| ApiError::service_unavailable("WASM registry not configured"))?;

    crate::ensure_wasm_permission!(
        state,
        &user_id,
        &package_id,
        WasmPackagePermission::Maintainer
    );

    let txn = state.db.begin().await?;

    let existing_meta = meta::Entity::find()
        .filter(meta::Column::WasmPackageId.eq(&package_id))
        .filter(meta::Column::Lang.eq(&query.language))
        .one(&txn)
        .await?
        .ok_or(ApiError::NOT_FOUND)?;

    let mut existing_preview = existing_meta.preview_media.clone().unwrap_or_default();
    let mut model: meta::ActiveModel = existing_meta.clone().into();
    model.updated_at = Set(chrono::Utc::now().naive_utc());
    let item_id = create_id();
    let item_name = format!("{}.{}", item_id, query.extension);

    let master_store = state.master_credentials().await?;
    let master_store = master_store.to_store(false).await?;

    match &query.item {
        PackageMediaItem::Icon => {
            if let Some(icon) = &existing_meta.icon {
                let file_name = format!("{icon}.webp");
                let path = FlowPath::from("media")
                    .child("packages")
                    .child(package_id.as_str())
                    .child(file_name);
                if let Err(err) = master_store.as_generic().delete(&path).await {
                    tracing::error!("Failed to delete existing icon at {}: {:?}", path, err);
                }
            }
            model.icon = Set(Some(item_id));
        }
        PackageMediaItem::Thumbnail => {
            if let Some(thumbnail) = &existing_meta.thumbnail {
                let file_name = format!("{thumbnail}.webp");
                let path = FlowPath::from("media")
                    .child("packages")
                    .child(package_id.as_str())
                    .child(file_name);
                if let Err(err) = master_store.as_generic().delete(&path).await {
                    tracing::error!("Failed to delete existing thumbnail at {}: {:?}", path, err);
                }
            }
            model.thumbnail = Set(Some(item_id));
        }
        PackageMediaItem::Preview => {
            existing_preview.push(item_id.clone());
            model.preview_media = Set(Some(existing_preview));
        }
    }

    model.update(&txn).await?;
    let path = FlowPath::from("media")
        .child("packages")
        .child(package_id.as_str())
        .child(item_name.clone());
    let signed_url = master_store
        .sign("PUT", &path, Duration::from_secs(60 * 60 * 24))
        .await
        .map_err(|e| {
            let id = create_id();
            tracing::error!(
                "[{}] Failed to sign URL for media item '{}' - {:?}",
                id,
                item_name,
                e
            );
            ApiError::internal_error(anyhow!("Failed to create signed URL, reference ID: {}", id))
        })?;

    txn.commit().await?;
    Ok(Json(PushMediaResponse {
        signed_url: signed_url.to_string(),
    }))
}

/// DELETE /registry/package/{package_id}/meta/media/{media_id}
/// Remove package media and delete the underlying file.
#[utoipa::path(
    delete,
    path = "/registry/package/{package_id}/meta/media/{media_id}",
    tag = "registry",
    description = "Remove package media and delete the underlying file.",
    params(
        ("package_id" = String, Path, description = "Package ID"),
        ("media_id" = String, Path, description = "Media CUID"),
        ("language" = Option<String>, Query, description = "Language code (default en)"),
        ("item" = String, Query, description = "Media item: icon, thumbnail, preview"),
        ("extension" = String, Query, description = "File extension")
    ),
    responses(
        (status = 200, description = "Media removed"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
        (status = 404, description = "Not found"),
        (status = 503, description = "WASM registry not configured")
    )
)]
pub async fn remove_package_media(
    Extension(user): Extension<AppUser>,
    State(state): State<AppState>,
    Path((package_id, media_id)): Path<(String, String)>,
    Query(query): Query<PackageMediaQuery>,
) -> Result<Json<()>, ApiError> {
    let user_id = user
        .sub()
        .map_err(|_| ApiError::unauthorized("Authentication required"))?;

    state
        .wasm_registry
        .as_ref()
        .ok_or_else(|| ApiError::service_unavailable("WASM registry not configured"))?;

    let _package = wasm_package::Entity::find_by_id(&package_id)
        .one(&state.db)
        .await?
        .ok_or(ApiError::NOT_FOUND)?;

    crate::ensure_wasm_permission!(
        state,
        &user_id,
        &package_id,
        WasmPackagePermission::Maintainer
    );

    let txn = state.db.begin().await?;

    let existing_meta = meta::Entity::find()
        .filter(meta::Column::WasmPackageId.eq(&package_id))
        .filter(meta::Column::Lang.eq(&query.language))
        .one(&txn)
        .await?
        .ok_or(ApiError::NOT_FOUND)?;

    let mut model: meta::ActiveModel = existing_meta.clone().into();
    model.updated_at = Set(chrono::Utc::now().naive_utc());

    match &query.item {
        PackageMediaItem::Icon => {
            if existing_meta.icon.as_deref() == Some(&media_id) {
                model.icon = Set(None);
            }
        }
        PackageMediaItem::Thumbnail => {
            if existing_meta.thumbnail.as_deref() == Some(&media_id) {
                model.thumbnail = Set(None);
            }
        }
        PackageMediaItem::Preview => {
            let mut existing_preview = existing_meta.preview_media.clone().unwrap_or_default();
            existing_preview.retain(|id| id != &media_id);
            model.preview_media = Set(Some(existing_preview));
        }
    }

    model.update(&txn).await?;

    let item_name = format!("{media_id}.webp");
    let master_store = state.master_credentials().await?;
    let master_store = master_store.to_store(false).await?;
    let path = FlowPath::from("media")
        .child("packages")
        .child(package_id.as_str())
        .child(item_name.clone());
    if let Err(e) = master_store.as_generic().delete(&path).await {
        tracing::error!("Failed to delete media file at {}: {:?}", path, e);
        return Err(ApiError::internal_error(anyhow!(
            "Failed to delete media file, reference ID: {}",
            create_id()
        )));
    }
    txn.commit().await?;

    Ok(Json(()))
}
