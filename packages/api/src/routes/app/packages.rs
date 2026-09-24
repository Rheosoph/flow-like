use axum::{
    Extension, Json, Router,
    extract::{Path, Query, State},
    routing::{delete, get, post},
};
use chrono::{DateTime, Utc};
use sea_orm::sea_query::ExprTrait;
use sea_orm::{ActiveModelTrait, ActiveValue::Set, ColumnTrait, EntityTrait, QueryFilter};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::{
    ensure_permission,
    entity::{
        app_package, membership, meta, wasm_package, wasm_package_user, wasm_package_version,
    },
    error::ApiError,
    middleware::jwt::AppUser,
    package_license::{self, PackageLicense},
    permission::role_permission::RolePermissions,
    routes::{LanguageParams, registry::types::MetaSummary},
    state::AppState,
};
use flow_like_types::create_id;
use std::collections::{HashMap, HashSet};

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
    pub price: i64,
    pub license: PackageLicense,
    /// Whether the caller holds the package and could license it here.
    pub viewer_has_package: bool,
}

/// Per-request facts the response needs beyond the pin and its package.
struct PinContext {
    holders: HashMap<String, String>,
    viewer_packages: HashSet<String>,
    now: DateTime<Utc>,
}

impl PinContext {
    async fn load(
        state: &AppState,
        pins: &[app_package::Model],
        viewer: Option<&str>,
    ) -> Result<Self, ApiError> {
        let membership_ids: Vec<String> = pins
            .iter()
            .filter_map(|pin| pin.membership_id.clone())
            .collect();
        let holders = if membership_ids.is_empty() {
            HashMap::new()
        } else {
            membership::Entity::find()
                .filter(membership::Column::Id.is_in(membership_ids))
                .all(&state.db)
                .await?
                .into_iter()
                .map(|member| (member.id, member.user_id))
                .collect()
        };
        let viewer_packages = match viewer {
            Some(viewer) if !pins.is_empty() => wasm_package_user::Entity::find()
                .filter(wasm_package_user::Column::UserId.eq(viewer))
                .filter(
                    wasm_package_user::Column::PackageId
                        .is_in(pins.iter().map(|pin| pin.package_id.clone())),
                )
                .filter(wasm_package_user::Column::Permission.ne(0))
                .all(&state.db)
                .await?
                .into_iter()
                .map(|row| row.package_id)
                .collect(),
            _ => HashSet::new(),
        };
        Ok(Self {
            holders,
            viewer_packages,
            now: Utc::now(),
        })
    }

    fn respond(
        &self,
        model: &app_package::Model,
        pkg: Option<&wasm_package::Model>,
        meta: Option<&meta::Model>,
    ) -> AppPackageResponse {
        let holder = model
            .membership_id
            .as_ref()
            .and_then(|id| self.holders.get(id).cloned());
        let viewer_has_package = pkg.is_some_and(|pkg| {
            !package_license::requires_license(pkg) || self.viewer_packages.contains(&pkg.id)
        });
        AppPackageResponse::from_model(
            model,
            pkg,
            meta,
            package_license::license(model, pkg, holder, self.now),
            viewer_has_package,
        )
    }
}

impl AppPackageResponse {
    fn from_model(
        model: &app_package::Model,
        pkg: Option<&wasm_package::Model>,
        meta: Option<&meta::Model>,
        license: PackageLicense,
        viewer_has_package: bool,
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
            keywords: pkg.and_then(|p| p.keywords.clone().map(Into::into)),
            added_at: model.added_at.to_utc(),
            stale: model.stale,
            metadata: meta.map(MetaSummary::from_model),
            price: pkg.map_or(0, |p| p.price),
            license,
            viewer_has_package,
        }
    }
}

fn pick_best_meta<'a>(metas: &'a [meta::Model], language: &str) -> Option<&'a meta::Model> {
    MetaSummary::pick_best(metas, language)
}

/// The response for a single pin, with its package, best meta and licence.
async fn pin_response(
    state: &AppState,
    pin: &app_package::Model,
    language: &str,
    viewer: Option<&str>,
) -> Result<AppPackageResponse, ApiError> {
    let pkg = wasm_package::Entity::find_by_id(&pin.package_id)
        .one(&state.db)
        .await?;
    let metas = meta::Entity::find()
        .filter(meta::Column::WasmPackageId.eq(&pin.package_id))
        .filter(
            meta::Column::Lang
                .eq(language)
                .or(meta::Column::Lang.eq("en")),
        )
        .all(&state.db)
        .await?;
    let context = PinContext::load(state, std::slice::from_ref(pin), viewer).await?;
    Ok(context.respond(pin, pkg.as_ref(), pick_best_meta(&metas, language)))
}

fn license_lapsed() -> ApiError {
    ApiError::coded(
        axum::http::StatusCode::BAD_REQUEST,
        "PACKAGE_LICENSE_LAPSED",
        "No admin or owner of this project has this package, so it takes no updates. An admin or the owner who has the package needs to reactivate it.",
    )
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
    description = "List all WASM packages added to this app with their price and project licence state.",
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
#[tracing::instrument(name = "GET /apps/{app_id}/packages", skip(state, user, query))]
pub async fn list_packages(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(app_id): Path<String>,
    Query(query): Query<LanguageParams>,
) -> Result<Json<Vec<AppPackageResponse>>, ApiError> {
    ensure_permission!(user, &app_id, &state, RolePermissions::ReadBoards);
    let language = query.language.as_deref().unwrap_or("en");

    // Self-heal: a holder who left through a path without a licence hook, or an
    // admin who just got the package, is reflected before the list is shown.
    let lapses = package_license::reconcile_app(&state.db, &app_id).await?;
    package_license::notify_lapses(&state, &lapses).await;

    let packages = app_package::Entity::find()
        .filter(app_package::Column::AppId.eq(&app_id))
        .all(&state.db)
        .await?;

    if packages.is_empty() {
        return Ok(Json(Vec::new()));
    }

    let viewer = user.sub().ok();
    let context = PinContext::load(&state, &packages, viewer.as_deref()).await?;
    let package_ids: Vec<String> = packages.iter().map(|p| p.package_id.clone()).collect();

    // Packages and metas load separately: a package without a meta row in the
    // requested languages still has a price and a licence to report.
    let pkg_map: HashMap<String, wasm_package::Model> = wasm_package::Entity::find()
        .filter(wasm_package::Column::Id.is_in(package_ids.clone()))
        .all(&state.db)
        .await?
        .into_iter()
        .map(|wp| (wp.id.clone(), wp))
        .collect();
    let mut metas_by_package: HashMap<String, Vec<meta::Model>> = HashMap::new();
    for meta in meta::Entity::find()
        .filter(meta::Column::WasmPackageId.is_in(package_ids))
        .filter(
            meta::Column::Lang
                .eq(language)
                .or(meta::Column::Lang.eq("en")),
        )
        .all(&state.db)
        .await?
    {
        if let Some(package_id) = meta.wasm_package_id.clone() {
            metas_by_package.entry(package_id).or_default().push(meta);
        }
    }

    let responses = packages
        .iter()
        .map(|p| {
            let meta = metas_by_package
                .get(&p.package_id)
                .and_then(|metas| pick_best_meta(metas, language));
            context.respond(p, pkg_map.get(&p.package_id), meta)
        })
        .collect();

    Ok(Json(responses))
}

#[utoipa::path(
    post,
    path = "/apps/{app_id}/packages",
    tag = "packages",
    description = "Add a WASM package to this app. The calling admin or owner must hold the package (bought, granted or free) and becomes its licence holder for the project.",
    params(
        ("app_id" = String, Path, description = "Application ID"),
        ("language" = Option<String>, Query, description = "Language code (default: en)")
    ),
    request_body = AddPackageRequest,
    responses(
        (status = 200, description = "Package added", body = AppPackageResponse),
        (status = 400, description = "Bad request"),
        (status = 401, description = "Unauthorized"),
        (status = 402, description = "Paid package the caller has not bought (PACKAGE_LICENSE_REQUIRED)"),
        (status = 403, description = "Forbidden, or no access to a private or request-access package (PACKAGE_ACCESS_REQUIRED)"),
        (status = 404, description = "Package not found")
    ),
    security(
        ("bearer_auth" = []),
        ("api_key" = []),
        ("pat" = [])
    )
)]
#[tracing::instrument(
    name = "POST /apps/{app_id}/packages",
    skip(state, user, query, request)
)]
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

    package_license::ensure_holds(&state.db, &user_id, &wasm_pkg).await?;

    let mem = membership::Entity::find()
        .filter(membership::Column::AppId.eq(&app_id))
        .filter(membership::Column::UserId.eq(&user_id))
        .one(&state.db)
        .await?
        .ok_or(ApiError::bad_request("Not a member of this app"))?;

    let now = Utc::now().fixed_offset();
    let model = app_package::ActiveModel {
        id: Set(create_id()),
        app_id: Set(app_id.clone()),
        membership_id: Set(Some(mem.id)),
        package_id: Set(request.package_id.clone()),
        version: Set(request.version.clone()),
        added_at: Set(now),
        auto_update: Set(request.auto_update),
        stale: Set(false),
        stale_since: Set(None),
    };

    let inserted = model.insert(&state.db).await?;

    Ok(Json(
        pin_response(&state, &inserted, language, Some(&user_id)).await?,
    ))
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
        (status = 400, description = "The package licence lapsed in this project (PACKAGE_LICENSE_LAPSED)"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
        (status = 404, description = "Package not found in app, or requested version not published")
    ),
    security(
        ("bearer_auth" = []),
        ("api_key" = []),
        ("pat" = [])
    )
)]
#[tracing::instrument(
    name = "PATCH /apps/{app_id}/packages/{package_id}",
    skip(state, user, query, request)
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
        return Err(license_lapsed());
    }

    let mut active: app_package::ActiveModel = existing.into();

    if let Some(version) = request.version {
        // Reject versions that don't exist as published records — a bad pin
        // would silently drop the package's nodes from the app catalog.
        let version_exists = wasm_package_version::Entity::find()
            .filter(wasm_package_version::Column::PackageId.eq(&package_id))
            .filter(wasm_package_version::Column::Version.eq(&version))
            .one(&state.db)
            .await?
            .is_some();
        if !version_exists {
            return Err(ApiError::not_found(format!(
                "Version '{}' not found for package '{}'",
                version, package_id
            )));
        }
        active.version = Set(version);
    }
    if let Some(auto_update) = request.auto_update {
        active.auto_update = Set(auto_update);
    }

    let updated = active.update(&state.db).await?;

    let viewer = user.sub().ok();
    Ok(Json(
        pin_response(&state, &updated, language, viewer.as_deref()).await?,
    ))
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

    // Return the node definitions for the PINNED version, not the package's
    // latest snapshot (wasm_package.nodes). Patching a board against the latest
    // nodes when the app is pinned to an older version would apply the wrong
    // pin signatures. This mirrors how execution (wasm_resolve.rs) and the app
    // catalog (wasm_catalog.rs) resolve per-version node definitions.
    let version_record = wasm_package_version::Entity::find()
        .filter(wasm_package_version::Column::PackageId.eq(&package_id))
        .filter(wasm_package_version::Column::Version.eq(&app_pkg.version))
        .one(&state.db)
        .await?
        .ok_or_else(|| {
            tracing::error!(
                package_id = %package_id,
                version = %app_pkg.version,
                "patch-info: pinned WASM package version not found"
            );
            ApiError::not_found("Pinned WASM package version not found")
        })?;

    Ok(Json(PatchInfo {
        package_id,
        version: app_pkg.version,
        nodes: version_record.nodes,
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

    // A lapsed pin takes no updates until someone licenses it again.
    let packages = app_package::Entity::find()
        .filter(app_package::Column::AppId.eq(&app_id))
        .filter(app_package::Column::Stale.eq(false))
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
    description = "License a lapsed package again. The calling admin or owner must hold the package and becomes its licence holder.",
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
    skip(state, user, query)
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

    package_license::ensure_holds(&state.db, &user_id, &wasm_pkg).await?;

    let mem = membership::Entity::find()
        .filter(membership::Column::AppId.eq(&app_id))
        .filter(membership::Column::UserId.eq(&user_id))
        .one(&state.db)
        .await?
        .ok_or(ApiError::bad_request("Not a member of this app"))?;

    let mut active: app_package::ActiveModel = existing.into();
    active.stale = Set(false);
    active.stale_since = Set(None);
    active.membership_id = Set(Some(mem.id));
    let updated = active.update(&state.db).await?;

    Ok(Json(
        pin_response(&state, &updated, language, Some(&user_id)).await?,
    ))
}
