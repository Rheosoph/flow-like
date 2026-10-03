//! Package download endpoint

use super::types::{DownloadRequest, DownloadResponse, MetaSummary};
use crate::entity::meta;
use crate::entity::sea_orm_active_enums::{WasmPackageStatus, WasmPackageVisibility};
use crate::entity::wasm_package;
use crate::entity::wasm_package_version;
use crate::error::ApiError;
use crate::middleware::jwt::AppUser;
use crate::state::AppState;
use axum::extract::State;
use axum::{Extension, Json};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QuerySelect};

/// POST /registry/download
/// Get download URL for a package WASM binary.
///
/// Access rules:
/// - Public + free (price <= 0): any authenticated user can download
/// - Public + paid: requires a completed purchase (wasm_package_user record)
/// - PublicRequestAccess: requires a wasm_package_user record (granted via join approval or purchase)
/// - Private: requires a wasm_package_user record
/// - With `app_id`: members of that project may download the pinned version
///   while an admin or the owner licenses it (or within the lapse grace period),
///   and a request without `version` gets the pinned version. Members who
///   cannot read the project's boards get the widget bundle without the node
///   binary (`PackageManifest::withhold_nodes`)
#[utoipa::path(
    post,
    path = "/registry/download",
    tag = "registry",
    description = "Get a download link for a package. With a project, you get the version the project uses unless you ask for another one, and members of the project can download it while the project's licence for it is valid. Members who cannot open the project's flows get the package's widgets without its nodes.",
    request_body = DownloadRequest,
    responses(
        (status = 200, description = "Download URL and package info", body = DownloadResponse),
        (status = 402, description = "Payment required"),
        (status = 403, description = "No access to this package, or only to its widgets while this version ships none (PACKAGE_NODES_WITHHELD)"),
        (status = 404, description = "Package not found, or the requested version is not available to you"),
        (status = 503, description = "WASM registry not configured")
    ),
    security(("bearer_auth" = []))
)]
pub async fn download(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Json(request): Json<DownloadRequest>,
) -> Result<Json<DownloadResponse>, ApiError> {
    let registry = state
        .wasm_registry
        .as_ref()
        .ok_or_else(|| ApiError::service_unavailable("WASM registry not configured"))?;
    let sub = user.sub().ok();

    let package = wasm_package::Entity::find_by_id(&request.package_id)
        .one(&state.db)
        .await
        .map_err(|e| ApiError::internal(format!("DB error: {}", e)))?
        .ok_or_else(|| ApiError::not_found("Package not found"))?;

    if package.status != WasmPackageStatus::Active {
        if let Some(ref user_id) = sub {
            let access = crate::check_wasm_access!(state, user_id, &request.package_id);
            if access.is_none() {
                return Err(ApiError::not_found("Package not found"));
            }
        } else {
            return Err(ApiError::not_found("Package not found"));
        }
    }

    let is_free_public = package.visibility == WasmPackageVisibility::Public && package.price <= 0;
    let project_version = match request.app_id.as_deref() {
        Some(app_id) => project_licensed_version(&state, &user, app_id, &request).await?,
        None => None,
    };

    // The node binary follows the caller's own access. A project's licence
    // opens it only to members who can run the project's boards themselves;
    // every other member gets the package's widgets alone.
    let mut withhold_nodes = false;
    if !is_free_public {
        let sub = sub
            .clone()
            .ok_or_else(|| ApiError::unauthorized("Authentication required for downloads"))?;

        let access = crate::check_wasm_access!(state, &sub, &request.package_id);
        if access.is_none() {
            if project_version.is_none() {
                return match package.visibility {
                    WasmPackageVisibility::Public if package.price > 0 => Err(
                        ApiError::purchase_required("Purchase required to download this package"),
                    ),
                    WasmPackageVisibility::PublicRequestAccess => Err(ApiError::forbidden(
                        "Access request required for this package",
                    )),
                    _ => Err(ApiError::FORBIDDEN),
                };
            }
            withhold_nodes = match request.app_id.as_deref() {
                Some(app_id) => {
                    !crate::package_license::reads_project_boards(&state, &user, app_id).await
                }
                None => true,
            };
        }
    }

    let can_manage = super::viewer_can_manage(&state, sub.as_deref(), &request.package_id).await?;
    let package_id = package.id.clone();
    let requested_version = request.version.clone().or(project_version);

    let Some((download_url, mut manifest, version, has_widget_bundle)) = registry
        .get_wasm_url_as_viewer(package, requested_version.as_deref(), can_manage)
        .await?
    else {
        return Err(ApiError::not_found(format!(
            "Version '{}' of package '{}' is not available",
            requested_version.as_deref().unwrap_or_default(),
            package_id
        )));
    };
    let download_url = if withhold_nodes {
        widgets_only_view(download_url, has_widget_bundle, &mut manifest)?
    } else {
        download_url
    };

    // The counter write overlaps the metadata read (icon, thumbnail, localized
    // name) but stays inside the request: on Lambda a detached task is frozen
    // with the invocation, mid-transaction.
    let (_, metas) = flow_like_types::tokio::join!(
        registry.increment_downloads(&state, &package_id),
        meta::Entity::find()
            .filter(meta::Column::WasmPackageId.eq(&package_id))
            .all(&state.db),
    );
    let mut metadata = metas
        .ok()
        .and_then(|metas| MetaSummary::pick_best(&metas, "en").map(MetaSummary::from_model));

    if let Some(meta) = &mut metadata
        && let Ok(master_creds) = state.master_credentials().await
        && let Ok(store) = master_creds.to_store(false).await
    {
        meta.presign_media(&package_id, &store).await;
    }

    // If the client specified a target platform, try to provide a presigned cwasm URL + checksum
    let (cwasm_download_url, cwasm_checksum) = if download_url.is_some()
        && let Some(ref target_platform) = request.target_platform
    {
        let storage_scope = state.storage_identity.meta.cache_scope();
        let compiled_download = async {
            let generation = wasm_package_version::Entity::find()
                .select_only()
                .column(wasm_package_version::Column::CompiledArtifactGeneration)
                .filter(wasm_package_version::Column::PackageId.eq(&request.package_id))
                .filter(wasm_package_version::Column::Version.eq(&version))
                .into_tuple::<Option<String>>()
                .one(&state.db)
                .await?
                .flatten();
            registry
                .sign_cwasm_url(
                    &request.package_id,
                    &version,
                    target_platform,
                    generation.as_deref(),
                    &state.cache,
                    storage_scope.as_deref(),
                )
                .await
        }
        .await;
        match compiled_download {
            Ok((cwasm, checksum)) => (Some(cwasm), Some(checksum)),
            Err(e) => {
                tracing::warn!(
                    "Failed to sign cwasm URL for {} ({}): {}",
                    request.package_id,
                    target_platform,
                    e
                );
                (None, None)
            }
        }
    } else {
        (None, None)
    };

    // Presign the widget bundle when the version ships widgets
    let widget_bundle_download_url = match registry
        .sign_widget_bundle_url(&request.package_id, &version, has_widget_bundle)
        .await
    {
        Ok(url) => url,
        Err(e) => {
            tracing::warn!(
                "Failed to sign widget bundle URL for {} ({}): {}",
                request.package_id,
                version,
                e
            );
            None
        }
    };

    Ok(Json(DownloadResponse {
        package_id,
        version,
        wasm_base64: String::new(),
        download_url,
        manifest,
        metadata,
        cwasm_download_url,
        cwasm_checksum,
        widget_bundle_download_url,
    }))
}

/// What a caller who may only load the package's widgets gets of a version:
/// its manifest without the node binary, and no link to it. Only a version
/// with a node binary has something to withhold; one that ships widgets alone
/// is the same for every caller and stays unmarked.
fn widgets_only_view(
    download_url: Option<String>,
    has_widget_bundle: Option<bool>,
    manifest: &mut flow_like_wasm_schema::manifest::PackageManifest,
) -> Result<Option<String>, ApiError> {
    if download_url.is_none() {
        return Ok(None);
    }
    if has_widget_bundle != Some(true) {
        return Err(ApiError::coded(
            axum::http::StatusCode::FORBIDDEN,
            "PACKAGE_NODES_WITHHELD",
            "This project lets you use the package's widgets, and this version ships none. Downloading its nodes needs access to the project's flows or to the package.",
        ));
    }
    manifest.withhold_nodes();
    Ok(None)
}

/// The pinned version a member of `app_id` may download through the project's
/// licence, or `None` when they are not a member, the pin expired or the
/// request names another version.
async fn project_licensed_version(
    state: &AppState,
    user: &AppUser,
    app_id: &str,
    request: &DownloadRequest,
) -> Result<Option<String>, ApiError> {
    Ok(
        crate::package_license::member_pinned_version(state, user, app_id, &request.package_id)
            .await?
            .filter(|pinned| {
                request
                    .version
                    .as_deref()
                    .is_none_or(|version| version == pinned)
            }),
    )
}

#[cfg(test)]
mod tests {
    use super::widgets_only_view;
    use flow_like_wasm_schema::manifest::PackageManifest;

    fn manifest() -> PackageManifest {
        let mut manifest = PackageManifest::new("com.acme.maps", "Maps", "1.0.0", "maps");
        manifest.wasm_path = Some("wasm/com.acme.maps/1.0.0/node.wasm".into());
        manifest.wasm_hash = Some("abc".into());
        manifest
    }

    #[test]
    fn the_widgets_only_view_drops_the_node_binary_and_its_link() {
        let mut withheld = manifest();
        let link = widgets_only_view(
            Some("https://cdn/node.wasm".into()),
            Some(true),
            &mut withheld,
        );
        assert_eq!(link.unwrap(), None);
        assert!(withheld.nodes_withheld());
        assert!(withheld.wasm_path.is_none() && withheld.wasm_hash.is_none());
    }

    #[test]
    fn a_version_without_widgets_has_nothing_to_give_a_widgets_only_caller() {
        for has_widget_bundle in [Some(false), None] {
            let mut untouched = manifest();
            let refused = widgets_only_view(
                Some("https://cdn/node.wasm".into()),
                has_widget_bundle,
                &mut untouched,
            )
            .unwrap_err();
            assert_eq!(refused.status(), axum::http::StatusCode::FORBIDDEN);
            assert_eq!(refused.public_code(), "PACKAGE_NODES_WITHHELD");
            assert!(!untouched.nodes_withheld());
        }
    }

    #[test]
    fn a_version_without_a_node_binary_is_delivered_unmarked() {
        let mut widgets = PackageManifest::new("com.acme.maps", "Maps", "1.0.0", "maps");
        let link = widgets_only_view(None, Some(true), &mut widgets);
        assert_eq!(link.unwrap(), None);
        assert!(!widgets.nodes_withheld());
        assert!(widgets.metadata.is_empty());
    }
}
