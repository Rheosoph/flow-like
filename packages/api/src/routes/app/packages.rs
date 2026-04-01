use axum::{
    Extension, Json, Router,
    extract::{Path, Query, State},
    routing::{delete, get, post},
};
use chrono::{DateTime, Utc};
use sea_orm::{ActiveModelTrait, ActiveValue::Set, ColumnTrait, EntityTrait, QueryFilter};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::{
    ensure_permission,
    entity::{
        app_package, membership, meta, sea_orm_active_enums::WasmPackageVisibility, wasm_package,
    },
    error::ApiError,
    middleware::jwt::AppUser,
    permission::role_permission::RolePermissions,
    routes::{LanguageParams, registry::types::MetaSummary},
    state::AppState,
};
use flow_like_types::create_id;

#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct AddPackageRequest {
    pub package_id: String,
    pub version: String,
    pub auto_update: bool,
}

#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct UpdatePackageRequest {
    pub version: Option<String>,
    pub auto_update: Option<bool>,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct AppPackageResponse {
    pub id: String,
    pub app_id: String,
    pub package_id: String,
    pub package_name: Option<String>,
    pub package_description: Option<String>,
    pub latest_version: Option<String>,
    #[serde(rename = "version")]
    pub pinned_version: String,
    pub auto_update: bool,
    pub status: Option<String>,
    pub visibility: Option<String>,
    pub verified: Option<bool>,
    pub keywords: Option<Vec<String>>,
    pub added_at: DateTime<Utc>,
    pub stale: bool,
    pub metadata: Option<MetaSummary>,
}

impl AppPackageResponse {
    fn from_model(
        model: &app_package::Model,
        pkg: Option<&wasm_package::Model>,
        meta: Option<&meta::Model>,
    ) -> Self {
        Self {
            id: model.id.clone(),
            app_id: model.app_id.clone(),
            package_id: model.package_id.clone(),
            package_name: pkg.map(|p| p.name.clone()),
            package_description: pkg.map(|p| p.description.clone()),
            latest_version: pkg.map(|p| p.version.clone()),
            pinned_version: model.version.clone(),
            auto_update: model.auto_update,
            status: pkg.map(|p| format!("{:?}", p.status)),
            visibility: pkg.map(|p| format!("{:?}", p.visibility)),
            verified: pkg.map(|p| p.verified),
            keywords: pkg.and_then(|p| p.keywords.clone()),
            added_at: DateTime::from_naive_utc_and_offset(model.added_at, Utc),
            stale: model.stale,
            metadata: meta.map(MetaSummary::from_model),
        }
    }
}

fn pick_best_meta<'a>(metas: &'a [meta::Model], language: &str) -> Option<&'a meta::Model> {
    MetaSummary::pick_best(metas, language)
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct PatchInfo {
    pub package_id: String,
    pub version: String,
    pub nodes: serde_json::Value,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct PackageUpdateInfo {
    pub package_id: String,
    pub package_name: String,
    pub current_version: String,
    pub latest_version: String,
}

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/", get(list_packages).post(add_package))
        .route("/updates", get(check_updates))
        .route(
            "/{package_id}",
            delete(remove_package).patch(update_package),
        )
        .route("/{package_id}/reactivate", post(reactivate_package))
        .route("/{package_id}/patch-info", get(get_patch_info))
}

#[utoipa::path(
    get,
    path = "/apps/{app_id}/packages",
    tag = "packages",
    description = "List all WASM packages added to this app.",
    params(
        ("app_id" = String, Path, description = "Application ID"),
        ("language" = Option<String>, Query, description = "Language code (default: en)")
    ),
    responses(
        (status = 200, description = "List of packages", body = Vec<AppPackageResponse>),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden")
    ),
    security(
        ("bearer_auth" = []),
        ("api_key" = []),
        ("pat" = [])
    )
)]
#[tracing::instrument(name = "GET /apps/{app_id}/packages", skip(state, user))]
pub async fn list_packages(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(app_id): Path<String>,
    Query(query): Query<LanguageParams>,
) -> Result<Json<Vec<AppPackageResponse>>, ApiError> {
    ensure_permission!(user, &app_id, &state, RolePermissions::ReadBoards);
    let language = query.language.as_deref().unwrap_or("en");

    let packages = app_package::Entity::find()
        .filter(app_package::Column::AppId.eq(&app_id))
        .all(&state.db)
        .await?;

    if packages.is_empty() {
        return Ok(Json(Vec::new()));
    }

    let package_ids: Vec<String> = packages.iter().map(|p| p.package_id.clone()).collect();

    let wasm_with_meta = wasm_package::Entity::find()
        .filter(wasm_package::Column::Id.is_in(package_ids))
        .filter(
            meta::Column::Lang
                .eq(language)
                .or(meta::Column::Lang.eq("en")),
        )
        .find_with_related(meta::Entity)
        .all(&state.db)
        .await?;

    let pkg_map: std::collections::HashMap<String, (wasm_package::Model, Vec<meta::Model>)> =
        wasm_with_meta
            .into_iter()
            .map(|(wp, metas)| (wp.id.clone(), (wp, metas)))
            .collect();

    let responses = packages
        .iter()
        .map(|p| {
            let (pkg, meta) = if let Some((wp, metas)) = pkg_map.get(&p.package_id) {
                (Some(wp), pick_best_meta(metas, language))
            } else {
                (None, None)
            };
            AppPackageResponse::from_model(p, pkg, meta)
        })
        .collect();

    Ok(Json(responses))
}

#[utoipa::path(
    post,
    path = "/apps/{app_id}/packages",
    tag = "packages",
    description = "Add a WASM package to this app.",
    params(
        ("app_id" = String, Path, description = "Application ID"),
        ("language" = Option<String>, Query, description = "Language code (default: en)")
    ),
    request_body = AddPackageRequest,
    responses(
        (status = 200, description = "Package added", body = AppPackageResponse),
        (status = 400, description = "Bad request"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
        (status = 404, description = "Package not found")
    ),
    security(
        ("bearer_auth" = []),
        ("api_key" = []),
        ("pat" = [])
    )
)]
#[tracing::instrument(name = "POST /apps/{app_id}/packages", skip(state, user, request))]
pub async fn add_package(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(app_id): Path<String>,
    Query(query): Query<LanguageParams>,
    Json(request): Json<AddPackageRequest>,
) -> Result<Json<AppPackageResponse>, ApiError> {
    let sub = ensure_permission!(user, &app_id, &state, RolePermissions::Admin);
    let user_id = sub.sub()?;
    let language = query.language.as_deref().unwrap_or("en");

    let wasm_pkg = wasm_package::Entity::find_by_id(&request.package_id)
        .one(&state.db)
        .await?
        .ok_or(ApiError::not_found("Package not found"))?;

    if wasm_pkg.visibility == WasmPackageVisibility::Private {
        let access = crate::check_wasm_access!(state, &user_id, &request.package_id);
        if access.is_none() {
            return Err(ApiError::FORBIDDEN);
        }
    }

    let mem = membership::Entity::find()
        .filter(membership::Column::AppId.eq(&app_id))
        .filter(membership::Column::UserId.eq(&user_id))
        .one(&state.db)
        .await?
        .ok_or(ApiError::bad_request("Not a member of this app"))?;

    let now = Utc::now().naive_utc();
    let model = app_package::ActiveModel {
        id: Set(create_id()),
        app_id: Set(app_id.clone()),
        membership_id: Set(Some(mem.id)),
        package_id: Set(request.package_id.clone()),
        version: Set(request.version.clone()),
        added_at: Set(now),
        auto_update: Set(request.auto_update),
        stale: Set(false),
    };

    let inserted = model.insert(&state.db).await?;

    let metas = meta::Entity::find()
        .filter(meta::Column::WasmPackageId.eq(&request.package_id))
        .filter(
            meta::Column::Lang
                .eq(language)
                .or(meta::Column::Lang.eq("en")),
        )
        .all(&state.db)
        .await?;

    Ok(Json(AppPackageResponse::from_model(
        &inserted,
        Some(&wasm_pkg),
        pick_best_meta(&metas, language),
    )))
}

#[utoipa::path(
    delete,
    path = "/apps/{app_id}/packages/{package_id}",
    tag = "packages",
    description = "Remove a WASM package from this app.",
    params(
        ("app_id" = String, Path, description = "Application ID"),
        ("package_id" = String, Path, description = "Package ID")
    ),
    responses(
        (status = 200, description = "Package removed"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
        (status = 404, description = "Package not found in app")
    ),
    security(
        ("bearer_auth" = []),
        ("api_key" = []),
        ("pat" = [])
    )
)]
#[tracing::instrument(
    name = "DELETE /apps/{app_id}/packages/{package_id}",
    skip(state, user)
)]
pub async fn remove_package(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((app_id, package_id)): Path<(String, String)>,
) -> Result<Json<()>, ApiError> {
    ensure_permission!(user, &app_id, &state, RolePermissions::Admin);

    let result = app_package::Entity::delete_many()
        .filter(app_package::Column::AppId.eq(&app_id))
        .filter(app_package::Column::PackageId.eq(&package_id))
        .exec(&state.db)
        .await?;

    if result.rows_affected == 0 {
        return Err(ApiError::not_found("Package not found in app"));
    }

    Ok(Json(()))
}

#[utoipa::path(
    patch,
    path = "/apps/{app_id}/packages/{package_id}",
    tag = "packages",
    description = "Update version or auto-update settings for a WASM package in this app.",
    params(
        ("app_id" = String, Path, description = "Application ID"),
        ("package_id" = String, Path, description = "Package ID"),
        ("language" = Option<String>, Query, description = "Language code (default: en)")
    ),
    request_body = UpdatePackageRequest,
    responses(
        (status = 200, description = "Package updated", body = AppPackageResponse),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
        (status = 404, description = "Package not found in app")
    ),
    security(
        ("bearer_auth" = []),
        ("api_key" = []),
        ("pat" = [])
    )
)]
#[tracing::instrument(
    name = "PATCH /apps/{app_id}/packages/{package_id}",
    skip(state, user, request)
)]
pub async fn update_package(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((app_id, package_id)): Path<(String, String)>,
    Query(query): Query<LanguageParams>,
    Json(request): Json<UpdatePackageRequest>,
) -> Result<Json<AppPackageResponse>, ApiError> {
    ensure_permission!(user, &app_id, &state, RolePermissions::Admin);
    let language = query.language.as_deref().unwrap_or("en");

    let existing = app_package::Entity::find()
        .filter(app_package::Column::AppId.eq(&app_id))
        .filter(app_package::Column::PackageId.eq(&package_id))
        .one(&state.db)
        .await?
        .ok_or(ApiError::not_found("Package not found in app"))?;

    if existing.stale {
        return Err(ApiError::bad_request(
            "Package is stale and cannot be updated",
        ));
    }

    let mut active: app_package::ActiveModel = existing.into();

    if let Some(version) = request.version {
        active.version = Set(version);
    }
    if let Some(auto_update) = request.auto_update {
        active.auto_update = Set(auto_update);
    }

    let updated = active.update(&state.db).await?;

    let wasm_with_meta = wasm_package::Entity::find_by_id(&updated.package_id)
        .filter(
            meta::Column::Lang
                .eq(language)
                .or(meta::Column::Lang.eq("en")),
        )
        .find_with_related(meta::Entity)
        .all(&state.db)
        .await?;

    let (pkg, meta) = wasm_with_meta
        .first()
        .map(|(wp, metas)| (Some(wp), pick_best_meta(metas, language)))
        .unwrap_or((None, None));

    Ok(Json(AppPackageResponse::from_model(&updated, pkg, meta)))
}

#[utoipa::path(
    get,
    path = "/apps/{app_id}/packages/{package_id}/patch-info",
    tag = "packages",
    description = "Get node definitions for the pinned version of a package — used to patch boards after a version update.",
    params(
        ("app_id" = String, Path, description = "Application ID"),
        ("package_id" = String, Path, description = "Package ID")
    ),
    responses(
        (status = 200, description = "Patch info", body = PatchInfo),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
        (status = 404, description = "Not found")
    ),
    security(
        ("bearer_auth" = []),
        ("api_key" = []),
        ("pat" = [])
    )
)]
#[tracing::instrument(
    name = "GET /apps/{app_id}/packages/{package_id}/patch-info",
    skip(state, user)
)]
pub async fn get_patch_info(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((app_id, package_id)): Path<(String, String)>,
) -> Result<Json<PatchInfo>, ApiError> {
    ensure_permission!(user, &app_id, &state, RolePermissions::ReadBoards);

    let app_pkg = app_package::Entity::find()
        .filter(app_package::Column::AppId.eq(&app_id))
        .filter(app_package::Column::PackageId.eq(&package_id))
        .one(&state.db)
        .await?
        .ok_or(ApiError::not_found("Package not in app"))?;

    let wasm_pkg = wasm_package::Entity::find_by_id(&package_id)
        .one(&state.db)
        .await?
        .ok_or(ApiError::not_found("Package not found"))?;

    Ok(Json(PatchInfo {
        package_id,
        version: app_pkg.version,
        nodes: wasm_pkg.nodes,
    }))
}

#[utoipa::path(
    get,
    path = "/apps/{app_id}/packages/updates",
    tag = "packages",
    description = "Check for available version updates for all packages in this app.",
    params(
        ("app_id" = String, Path, description = "Application ID")
    ),
    responses(
        (status = 200, description = "Update info list", body = Vec<PackageUpdateInfo>),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden")
    ),
    security(
        ("bearer_auth" = []),
        ("api_key" = []),
        ("pat" = [])
    )
)]
#[tracing::instrument(name = "GET /apps/{app_id}/packages/updates", skip(state, user))]
pub async fn check_updates(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(app_id): Path<String>,
) -> Result<Json<Vec<PackageUpdateInfo>>, ApiError> {
    ensure_permission!(user, &app_id, &state, RolePermissions::ReadBoards);

    let packages = app_package::Entity::find()
        .filter(app_package::Column::AppId.eq(&app_id))
        .all(&state.db)
        .await?;

    if packages.is_empty() {
        return Ok(Json(Vec::new()));
    }

    let pkg_ids: Vec<String> = packages.iter().map(|p| p.package_id.clone()).collect();
    let wasm_pkgs = wasm_package::Entity::find()
        .filter(wasm_package::Column::Id.is_in(pkg_ids))
        .all(&state.db)
        .await?;

    let latest_map: std::collections::HashMap<String, (String, String)> = wasm_pkgs
        .into_iter()
        .map(|wp| (wp.id.clone(), (wp.version, wp.name)))
        .collect();

    let updates: Vec<PackageUpdateInfo> = packages
        .iter()
        .filter_map(|p| {
            let (latest, name) = latest_map.get(&p.package_id)?;
            if latest != &p.version {
                Some(PackageUpdateInfo {
                    package_id: p.package_id.clone(),
                    package_name: name.clone(),
                    current_version: p.version.clone(),
                    latest_version: latest.clone(),
                })
            } else {
                None
            }
        })
        .collect();

    Ok(Json(updates))
}

#[utoipa::path(
    post,
    path = "/apps/{app_id}/packages/{package_id}/reactivate",
    tag = "packages",
    description = "Reactivate a stale package. The caller must be an admin and have access to the package.",
    params(
        ("app_id" = String, Path, description = "Application ID"),
        ("package_id" = String, Path, description = "Package ID"),
        ("language" = Option<String>, Query, description = "Language code (default: en)")
    ),
    responses(
        (status = 200, description = "Package reactivated", body = AppPackageResponse),
        (status = 400, description = "Package is not stale"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden / no access to this package"),
        (status = 404, description = "Package not found in app")
    ),
    security(
        ("bearer_auth" = []),
        ("api_key" = []),
        ("pat" = [])
    )
)]
#[tracing::instrument(
    name = "POST /apps/{app_id}/packages/{package_id}/reactivate",
    skip(state, user)
)]
pub async fn reactivate_package(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((app_id, package_id)): Path<(String, String)>,
    Query(query): Query<LanguageParams>,
) -> Result<Json<AppPackageResponse>, ApiError> {
    let sub = ensure_permission!(user, &app_id, &state, RolePermissions::Admin);
    let user_id = sub.sub()?;
    let language = query.language.as_deref().unwrap_or("en");

    let existing = app_package::Entity::find()
        .filter(app_package::Column::AppId.eq(&app_id))
        .filter(app_package::Column::PackageId.eq(&package_id))
        .one(&state.db)
        .await?
        .ok_or(ApiError::not_found("Package not found in app"))?;

    if !existing.stale {
        return Err(ApiError::bad_request("Package is not stale"));
    }

    let wasm_pkg = wasm_package::Entity::find_by_id(&package_id)
        .one(&state.db)
        .await?
        .ok_or(ApiError::not_found("Package no longer exists in registry"))?;

    if wasm_pkg.visibility == WasmPackageVisibility::Private {
        let access = crate::check_wasm_access!(state, &user_id, &package_id);
        if access.is_none() {
            return Err(ApiError::forbidden(
                "You do not have access to this package",
            ));
        }
    }

    let mem = membership::Entity::find()
        .filter(membership::Column::AppId.eq(&app_id))
        .filter(membership::Column::UserId.eq(&user_id))
        .one(&state.db)
        .await?
        .ok_or(ApiError::bad_request("Not a member of this app"))?;

    let mut active: app_package::ActiveModel = existing.into();
    active.stale = Set(false);
    active.membership_id = Set(Some(mem.id));
    let updated = active.update(&state.db).await?;

    let metas = meta::Entity::find()
        .filter(meta::Column::WasmPackageId.eq(&package_id))
        .filter(
            meta::Column::Lang
                .eq(language)
                .or(meta::Column::Lang.eq("en")),
        )
        .all(&state.db)
        .await?;

    Ok(Json(AppPackageResponse::from_model(
        &updated,
        Some(&wasm_pkg),
        pick_best_meta(&metas, language),
    )))
}
