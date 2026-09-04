//! Server-side WASM Package Registry
//!
//! Stores WASM binaries in CDN/content bucket and metadata in PostgreSQL.
//! Custom nodes are public after admin approval.

use super::types::{
    MetaSummary, PackageSource, PackageStatus, PackageSummary, PackageVersion, PublishResponse,
    RegistryEntry, RegistryIndex, SearchFilters, SearchResults, SortField,
};
use crate::deletion::{DeletionRoot, job};
use crate::entity::sea_orm_active_enums::{
    WasmCompilationStatus, WasmPackageCategory, WasmPackageVisibility,
};
use crate::entity::{
    meta, user, wasm_package, wasm_package_author, wasm_package_review, wasm_package_user,
    wasm_package_version,
};
use flow_like::a2ui::micro_widget::{PackageWidgetRef, PackageWidgetSource};
use flow_like_storage::files::store::FlowLikeStore;
use flow_like_storage::object_store::PutPayload;
use flow_like_storage::object_store::path::Path;
use flow_like_types::create_id;
use flow_like_wasm_schema::manifest::{
    PackageManifest, PackageNodeEntry, PackagePermissions, PackageWidgetEntry,
};
use flow_like_wasm_schema::widget_bundle::{WidgetBundleReader, sha256_hex};
use sea_orm::sea_query::ExprTrait;
use sea_orm::{
    ActiveModelTrait, ActiveValue::Set, ColumnTrait, DatabaseConnection, EntityTrait,
    PaginatorTrait, QueryFilter, QueryOrder, QuerySelect, QueryTrait, TransactionTrait,
    sea_query::Expr,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::Duration;
use utoipa::ToSchema;

use flow_like_wasm_schema::runtime::WASMTIME_MAJOR_VERSION;

/// CDN path prefix for WASM packages
const WASM_PACKAGES_PATH: &str = "wasm";
pub const WASM_COMPILED_PATH: &str = "wasm-compiled";
/// CDN path prefix for stored widget bundles (`.flwb`)
pub const WIDGET_BUNDLES_PATH: &str = "widget-bundles";
/// CDN path prefix for unpacked widget bundle entries
pub const WIDGET_ASSETS_PATH: &str = "widget-assets";

/// Whether a (server-built) manifest ships a WASM node artifact.
/// Widgets-only packages declare widgets but carry no (or an empty) wasm path/hash.
pub fn manifest_has_wasm(manifest: &PackageManifest) -> bool {
    manifest.widgets.is_empty()
        || manifest.wasm_hash.as_deref().is_some_and(|h| !h.is_empty())
        || manifest.wasm_path.as_deref().is_some_and(|p| !p.is_empty())
}

/// Validate an uploaded widget bundle against the package manifest:
/// - whole-file sha256 must equal the manifest `widget_bundle_hash`
/// - the bundle itself must pass `WidgetBundleReader::validate`
/// - the bundle must be built for this package id / version
/// - the widget id sets of manifest and bundle must match exactly
/// - every manifest `widgets[*].contract` must match the bundle's
///   `contract.json` byte-for-byte (compared via canonical serde_json
///   serialization of both parsed `WidgetContract`s)
///
/// Returns `(widget_bundle_hash, widget_bundle_size)` on success.
pub fn validate_manifest_widget_bundle(
    manifest: &PackageManifest,
    bundle_bytes: &[u8],
) -> flow_like_types::Result<(String, i64)> {
    if manifest.widgets.is_empty() {
        return Err(flow_like_types::anyhow!(
            "Manifest for '{}' declares no widgets but a widget bundle was provided",
            manifest.id
        ));
    }

    let actual_hash = sha256_hex(bundle_bytes);
    let declared_hash = manifest
        .widget_bundle_hash
        .as_deref()
        .filter(|h| !h.is_empty())
        .ok_or_else(|| {
            flow_like_types::anyhow!(
                "Manifest for '{}' declares widgets but no widget_bundle_hash",
                manifest.id
            )
        })?;
    if declared_hash != actual_hash {
        return Err(flow_like_types::anyhow!(
            "Widget bundle hash mismatch: manifest declares {}, uploaded bundle is {}",
            declared_hash,
            actual_hash
        ));
    }

    let mut reader = WidgetBundleReader::from_bytes(bundle_bytes.to_vec())?;
    reader.validate().map_err(|errors| {
        flow_like_types::anyhow!("Invalid widget bundle: {}", errors.join("; "))
    })?;

    let bundle_manifest = reader.manifest().clone();
    if bundle_manifest.package_id != manifest.id {
        return Err(flow_like_types::anyhow!(
            "Widget bundle was built for package '{}', expected '{}'",
            bundle_manifest.package_id,
            manifest.id
        ));
    }
    if bundle_manifest.package_version != manifest.version {
        return Err(flow_like_types::anyhow!(
            "Widget bundle was built for version '{}', expected '{}'",
            bundle_manifest.package_version,
            manifest.version
        ));
    }

    let manifest_ids: std::collections::BTreeSet<&str> =
        manifest.widgets.iter().map(|w| w.id.as_str()).collect();
    let bundle_ids: std::collections::BTreeSet<&str> = bundle_manifest
        .widgets
        .iter()
        .map(|w| w.id.as_str())
        .collect();
    if manifest_ids != bundle_ids {
        return Err(flow_like_types::anyhow!(
            "Widget set mismatch between manifest [{}] and bundle [{}]",
            manifest_ids.into_iter().collect::<Vec<_>>().join(", "),
            bundle_ids.into_iter().collect::<Vec<_>>().join(", ")
        ));
    }

    for entry in &manifest.widgets {
        let bundle_contract = reader.contract(&entry.id)?;
        let manifest_json = serde_json::to_string(&entry.contract)?;
        let bundle_json = serde_json::to_string(&bundle_contract)?;
        if manifest_json != bundle_json {
            return Err(flow_like_types::anyhow!(
                "Contract mismatch for widget '{}': manifest and bundle contract.json differ",
                entry.id
            ));
        }
    }

    Ok((actual_hash, bundle_bytes.len() as i64))
}

/// Unpack a widget bundle into the bucket under
/// `widget-assets/{package_id}/{version}/…` (bundle-internal layout) so the
/// web app can load widget documents straight from CDN objects.
///
/// Entries are verified by `WidgetBundleReader::unpack` before upload.
/// Returns the number of uploaded objects.
pub async fn unpack_widget_bundle_to_assets(
    store: &FlowLikeStore,
    package_id: &str,
    version: &str,
    bundle_bytes: Vec<u8>,
) -> flow_like_types::Result<usize> {
    fn collect_files(
        root: &std::path::Path,
        dir: &std::path::Path,
        out: &mut Vec<(String, Vec<u8>)>,
    ) -> flow_like_types::Result<()> {
        for entry in std::fs::read_dir(dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.is_dir() {
                collect_files(root, &path, out)?;
            } else {
                let rel = path
                    .strip_prefix(root)
                    .map_err(|e| flow_like_types::anyhow!("Path outside unpack root: {}", e))?
                    .components()
                    .map(|c| c.as_os_str().to_string_lossy().into_owned())
                    .collect::<Vec<_>>()
                    .join("/");
                out.push((rel, std::fs::read(&path)?));
            }
        }
        Ok(())
    }

    let entries = flow_like_types::tokio::task::spawn_blocking(
        move || -> flow_like_types::Result<Vec<(String, Vec<u8>)>> {
            let staging = tempfile::tempdir()?;
            let dest = staging.path().join("bundle");
            let mut reader = WidgetBundleReader::from_bytes(bundle_bytes)?;
            reader.unpack(&dest)?;
            let mut out = Vec::new();
            collect_files(&dest, &dest, &mut out)?;
            Ok(out)
        },
    )
    .await??;

    let base = Path::from(WIDGET_ASSETS_PATH)
        .child(package_id)
        .child(version);
    let mut uploaded = 0usize;
    for (rel, data) in entries {
        let mut object_path = base.clone();
        for segment in rel.split('/') {
            object_path = object_path.child(segment);
        }
        store
            .as_generic()
            .put(&object_path, PutPayload::from(data))
            .await?;
        uploaded += 1;
    }
    Ok(uploaded)
}

pub fn with_current_wasmtime_version(existing: Option<Vec<String>>) -> Vec<String> {
    let mut versions = existing.unwrap_or_default();
    let current = WASMTIME_MAJOR_VERSION.to_string();
    if !versions.iter().any(|version| version == &current) {
        versions.push(current);
    }
    versions
}

/// Build a platform key from OS and architecture strings.
fn platform_key_for(os: &str, arch: &str) -> String {
    let arch = if os == "ios" && arch == "aarch64" {
        "pulley64"
    } else {
        arch
    };
    format!("{}-{}-wt{}", os, arch, WASMTIME_MAJOR_VERSION)
}

/// Normalize legacy client platform keys to the executable platform we can
/// safely serve. iOS native arm64 `.cwasm` artifacts are not safe to execute
/// in App Store/TestFlight builds; Pulley is the iOS execution target.
pub fn normalize_target_platform_key(target_platform: &str) -> String {
    if let Some(version) = target_platform.strip_prefix("ios-aarch64-wt") {
        return format!("ios-pulley64-wt{}", version);
    }
    target_platform.to_string()
}

/// Platform key for the current host process.
pub fn host_platform_key() -> String {
    platform_key_for(std::env::consts::OS, std::env::consts::ARCH)
}

/// Map `(os, arch)` to a wasmtime target triple for cross-compilation.
/// Returns `None` when the requested platform matches the current host
/// (no cross-compilation needed).
fn target_triple_for(os: &str, arch: &str) -> Option<&'static str> {
    if os == "ios" {
        // iOS AOT artifacts must be Pulley bytecode. Native arm64 `.cwasm`
        // artifacts still contain executable machine code that iOS can reject
        // at runtime because it is not part of the app's signed code pages.
        return match arch {
            "aarch64" | "pulley64" => Some("pulley64"),
            _ => None,
        };
    }

    let host_os = std::env::consts::OS;
    let host_arch = std::env::consts::ARCH;
    if os == host_os && arch == host_arch {
        return None; // host – no cross-compilation required
    }
    match (os, arch) {
        ("linux", "x86_64") => Some("x86_64-unknown-linux-gnu"),
        ("linux", "aarch64") => Some("aarch64-unknown-linux-gnu"),
        ("macos", "x86_64") => Some("x86_64-apple-darwin"),
        ("macos", "aarch64") => Some("aarch64-apple-darwin"),
        ("windows", "x86_64") => Some("x86_64-pc-windows-msvc"),
        ("windows", "aarch64") => Some("aarch64-pc-windows-msvc"),
        ("android", "aarch64") => Some("aarch64-linux-android"),
        ("android", "x86_64") => Some("x86_64-linux-android"),
        _ => None,
    }
}

/// Lightweight target specification used by the API to enumerate compilation
/// targets. Does not include presigned URLs — those are generated by the
/// dispatcher when building a `CompilationJob`.
#[derive(Clone, Debug)]
pub struct TargetSpec {
    pub platform_key: String,
    pub cross_triple: Option<String>,
}

/// The platform key the executor expects.
///
/// Read from `EXECUTOR_PLATFORM` (e.g. `linux-x86_64-wt43`).
/// Falls back to the host platform key when unset.
pub fn executor_target_platform() -> String {
    std::env::var("EXECUTOR_PLATFORM").unwrap_or_else(|_| host_platform_key())
}

/// Every platform the system knows how to compile for.
///
/// Used by the dispatcher to generate presigned upload URLs for **all**
/// potential targets so the external worker can compile whichever subset
/// it supports without the API needing to know the worker's configuration.
const ALL_KNOWN_PLATFORMS: &[(&str, &str)] = &[
    ("linux", "x86_64"),
    ("linux", "aarch64"),
    ("macos", "x86_64"),
    ("macos", "aarch64"),
    ("windows", "x86_64"),
    ("windows", "aarch64"),
    ("ios", "pulley64"),
    ("android", "aarch64"),
    ("android", "x86_64"),
];

pub fn all_known_targets() -> Vec<TargetSpec> {
    ALL_KNOWN_PLATFORMS
        .iter()
        .map(|(os, arch)| TargetSpec {
            platform_key: platform_key_for(os, arch),
            cross_triple: target_triple_for(os, arch).map(String::from),
        })
        .collect()
}

/// Author information for display
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct AuthorInfo {
    pub user_id: String,
    pub username: Option<String>,
    pub name: Option<String>,
    pub avatar: Option<String>,
    pub role: Option<String>,
}

/// Extended package summary with additional metadata for UI
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct PackageDetails {
    pub id: String,
    pub name: String,
    pub description: String,
    pub version: String,
    pub authors: Vec<AuthorInfo>,
    pub license: Option<String>,
    pub homepage: Option<String>,
    pub repository: Option<String>,
    pub keywords: Vec<String>,
    pub status: String,
    pub verified: bool,
    pub download_count: u64,
    pub wasm_size: u64,
    pub nodes: serde_json::Value,
    #[serde(default)]
    pub widgets: serde_json::Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub widget_bundle_hash: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub widget_bundle_size: Option<u64>,
    pub permissions: serde_json::Value,
    pub price: i64,
    pub visibility: String,
    pub primary_category: Option<String>,
    pub secondary_category: Option<String>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
    pub published_at: Option<chrono::DateTime<chrono::Utc>>,
}

/// Package review entry
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct PackageReview {
    pub id: String,
    pub package_id: String,
    pub reviewer_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reviewer: Option<AuthorInfo>,
    pub action: String,
    pub comment: Option<String>,
    pub security_score: Option<i32>,
    pub code_quality_score: Option<i32>,
    pub documentation_score: Option<i32>,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

/// Request to submit a review
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ReviewRequest {
    pub action: String, // "approve", "reject", "request_changes", "comment", "flag"
    pub comment: Option<String>,
    pub internal_note: Option<String>,
    pub security_score: Option<i32>,
    pub code_quality_score: Option<i32>,
    pub documentation_score: Option<i32>,
}

/// Statistics for the registry
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct RegistryStats {
    pub total_packages: i64,
    pub total_versions: i64,
    pub total_downloads: i64,
    pub pending_review: i64,
    pub active_packages: i64,
    pub rejected_packages: i64,
    pub verified_packages: i64,
}

fn status_to_string(status: &crate::entity::sea_orm_active_enums::WasmPackageStatus) -> String {
    use crate::entity::sea_orm_active_enums::WasmPackageStatus;
    match status {
        WasmPackageStatus::PendingReview => "pending_review".to_string(),
        WasmPackageStatus::Active => "active".to_string(),
        WasmPackageStatus::Rejected => "rejected".to_string(),
        WasmPackageStatus::Deprecated => "deprecated".to_string(),
        WasmPackageStatus::Disabled => "disabled".to_string(),
    }
}

fn visibility_to_string(v: &WasmPackageVisibility) -> String {
    match v {
        WasmPackageVisibility::Public => "public".to_string(),
        WasmPackageVisibility::Private => "private".to_string(),
        WasmPackageVisibility::PublicRequestAccess => "public_request_access".to_string(),
    }
}

fn status_to_enum(status: &str) -> crate::entity::sea_orm_active_enums::WasmPackageStatus {
    use crate::entity::sea_orm_active_enums::WasmPackageStatus;
    match status {
        "pending_review" => WasmPackageStatus::PendingReview,
        "active" => WasmPackageStatus::Active,
        "rejected" => WasmPackageStatus::Rejected,
        "deprecated" => WasmPackageStatus::Deprecated,
        "disabled" => WasmPackageStatus::Disabled,
        _ => WasmPackageStatus::PendingReview,
    }
}

fn status_to_package_status(
    status: &crate::entity::sea_orm_active_enums::WasmPackageStatus,
) -> PackageStatus {
    use crate::entity::sea_orm_active_enums::WasmPackageStatus;
    match status {
        WasmPackageStatus::Active => PackageStatus::Active,
        WasmPackageStatus::Deprecated => PackageStatus::Deprecated,
        WasmPackageStatus::PendingReview => PackageStatus::PendingReview,
        WasmPackageStatus::Rejected => PackageStatus::Rejected,
        WasmPackageStatus::Disabled => PackageStatus::Disabled,
    }
}

fn package_version_from_model(v: wasm_package_version::Model) -> PackageVersion {
    PackageVersion {
        version: v.version,
        wasm_hash: v.wasm_hash,
        wasm_size: v.wasm_size as u64,
        status: status_to_package_status(&v.status),
        download_url: None,
        published_at: v.published_at.to_utc(),
        min_flow_like_version: v.min_flow_like_version,
        release_notes: v.release_notes,
        yanked: v.yanked,
        widget_bundle_hash: v.widget_bundle_hash.filter(|h| !h.is_empty()),
        widget_bundle_size: v.widget_bundle_size.map(|s| s as u64),
    }
}

fn db_cat_to_string(cat: &WasmPackageCategory) -> String {
    serde_json::to_value(cat)
        .ok()
        .and_then(|v| v.as_str().map(String::from))
        .unwrap_or_else(|| "OTHER".to_string())
}

fn db_cat_to_manifest(
    cat: &WasmPackageCategory,
) -> flow_like_wasm_schema::manifest::WasmPackageCategory {
    let json = serde_json::to_value(cat).unwrap_or(serde_json::Value::String("OTHER".into()));
    serde_json::from_value(json)
        .unwrap_or(flow_like_wasm_schema::manifest::WasmPackageCategory::Other)
}

fn manifest_cat_to_db(
    cat: &flow_like_wasm_schema::manifest::WasmPackageCategory,
) -> WasmPackageCategory {
    let json = serde_json::to_value(cat).unwrap_or(serde_json::Value::String("OTHER".into()));
    serde_json::from_value(json).unwrap_or(WasmPackageCategory::Other)
}

/// Server-side registry for managing WASM packages
/// Uses PostgreSQL for metadata and CDN for WASM binaries
pub struct ServerRegistry {
    db: DatabaseConnection,
    content_bucket: Arc<FlowLikeStore>,
    meta_bucket: Arc<FlowLikeStore>,
    compilation_dispatcher: Option<Arc<crate::compilation::CompilationDispatcher>>,
}

impl ServerRegistry {
    pub fn new(
        db: DatabaseConnection,
        content_bucket: Arc<FlowLikeStore>,
        meta_bucket: Arc<FlowLikeStore>,
    ) -> Self {
        Self {
            db,
            content_bucket,
            meta_bucket,
            compilation_dispatcher: None,
        }
    }

    pub fn with_compilation_dispatcher(
        mut self,
        dispatcher: Arc<crate::compilation::CompilationDispatcher>,
    ) -> Self {
        self.compilation_dispatcher = Some(dispatcher);
        self
    }

    /// Get storage path for a WASM package version
    fn wasm_path(package_id: &str, version: &str) -> Path {
        Path::from(WASM_PACKAGES_PATH)
            .child(package_id)
            .child(version)
            .child("node.wasm")
    }

    /// Get storage path for a widget bundle version
    fn widget_bundle_path(package_id: &str, version: &str) -> Path {
        Path::from(WIDGET_BUNDLES_PATH)
            .child(package_id)
            .child(format!("{}.flwb", version))
    }

    async fn resolve_wasm_path(
        &self,
        package_id: &str,
        version: &str,
    ) -> flow_like_types::Result<Path> {
        if let Some(version_record) = wasm_package_version::Entity::find()
            .filter(wasm_package_version::Column::PackageId.eq(package_id))
            .filter(wasm_package_version::Column::Version.eq(version))
            .one(&self.db)
            .await?
        {
            return Ok(Path::from(version_record.wasm_path.as_str()));
        }

        if let Some(package_record) = wasm_package::Entity::find_by_id(package_id)
            .filter(wasm_package::Column::Version.eq(version))
            .one(&self.db)
            .await?
        {
            return Ok(Path::from(package_record.wasm_path.as_str()));
        }

        Ok(Self::wasm_path(package_id, version))
    }

    /// Fetch meta models for a batch of packages, filtered by language + English fallback.
    async fn fetch_meta_map(
        &self,
        packages: &[wasm_package::Model],
        language: &str,
    ) -> flow_like_types::Result<std::collections::HashMap<String, Vec<meta::Model>>> {
        if packages.is_empty() {
            return Ok(std::collections::HashMap::new());
        }
        let pkg_ids: Vec<String> = packages.iter().map(|p| p.id.clone()).collect();
        let metas = meta::Entity::find()
            .filter(meta::Column::WasmPackageId.is_in(pkg_ids))
            .filter(
                meta::Column::Lang
                    .eq(language)
                    .or(meta::Column::Lang.eq("en")),
            )
            .all(&self.db)
            .await?;
        Ok(metas
            .into_iter()
            .fold(std::collections::HashMap::new(), |mut acc, m| {
                if let Some(ref pkg_id) = m.wasm_package_id {
                    acc.entry(pkg_id.clone()).or_default().push(m);
                }
                acc
            }))
    }

    /// Get a presigned PUT URL for uploading a WASM binary to temporary storage
    pub async fn get_upload_url(&self, tmp_path: &str) -> flow_like_types::Result<String> {
        let path = Path::from(tmp_path);
        let url = self
            .content_bucket
            .sign("PUT", &path, Duration::from_secs(3600))
            .await?;
        Ok(url.to_string())
    }

    // Node extraction and AOT compilation deliberately do not live here. Both
    // load an uploaded module into a wasmtime engine, and the API process holds
    // the platform's database, mail, scheduler and bucket credentials. Every
    // deployment routes that work to a compiler worker, which reports results
    // back through `crate::compilation::callback`.

    pub async fn recompile_version(
        &self,
        sub: String,
        package_id: &str,
        version: &str,
    ) -> flow_like_types::Result<()> {
        tracing::info!(
            backend = ?self.compilation_dispatcher.as_ref().and_then(|d| d.backend()),
            pkg = %package_id,
            ver = %version,
            "Recompilation dispatch"
        );

        let wasm_path = Self::wasm_path(package_id, version).to_string();

        let version_record = wasm_package_version::Entity::find()
            .filter(wasm_package_version::Column::PackageId.eq(package_id))
            .filter(wasm_package_version::Column::Version.eq(version))
            .one(&self.db)
            .await?
            .ok_or_else(|| flow_like_types::anyhow!("Version not found"))?;

        let dispatcher = self
            .compilation_dispatcher
            .clone()
            .ok_or_else(|| flow_like_types::anyhow!("Compilation dispatcher not configured"))?;
        let params = crate::compilation::DispatchParams {
            package_id: package_id.to_string(),
            version: version.to_string(),
            wasm_path,
            wasm_hash: version_record.wasm_hash,
        };
        let resp = dispatcher
            .dispatch(sub, params)
            .await
            .map_err(|e| flow_like_types::anyhow!("Dispatch failed: {e}"))?;
        tracing::info!(
            pkg = %package_id,
            ver = %version,
            job_id = %resp.job_id,
            backend = %resp.backend,
            "Recompilation job dispatched"
        );

        Ok(())
    }

    /// Get a signed URL or CDN URL for downloading a WASM file
    async fn get_download_url(
        &self,
        package_id: &str,
        version: &str,
    ) -> flow_like_types::Result<String> {
        let path = self.resolve_wasm_path(package_id, version).await?;

        // Otherwise generate a signed URL (valid for 1 hour)
        let url = self
            .content_bucket
            .sign("GET", &path, Duration::from_secs(3600))
            .await?;
        Ok(url.to_string())
    }

    /// Generate a presigned GET URL for a compiled `.cwasm` artifact and read
    /// its blake3 checksum directly from storage.
    ///
    /// Returns `(cwasm_url, cwasm_checksum)`.
    ///
    /// `target_platform` is the platform key the executor expects
    /// (e.g. `linux-x86_64-wt40`).  Pass [`executor_target_platform()`] when the
    /// caller doesn't have a more specific value.
    pub async fn sign_cwasm_url(
        &self,
        package_id: &str,
        version: &str,
        target_platform: &str,
    ) -> flow_like_types::Result<(String, String)> {
        let target_platform = normalize_target_platform_key(target_platform);
        let base = Path::from(WASM_COMPILED_PATH)
            .child(package_id)
            .child(version);

        let cwasm_path = base.child(format!("{}.cwasm", target_platform));
        let checksum_path = base.child(format!("{}.cwasm.b3", target_platform));

        let cwasm_url = self
            .meta_bucket
            .sign("GET", &cwasm_path, Duration::from_secs(3600))
            .await?;

        let checksum_bytes = self
            .meta_bucket
            .as_generic()
            .get(&checksum_path)
            .await?
            .bytes()
            .await?;
        let cwasm_checksum = String::from_utf8(checksum_bytes.to_vec())
            .map(|s| s.trim().to_string())
            .map_err(|e| flow_like_types::anyhow!("invalid checksum encoding: {}", e))?;

        Ok((cwasm_url.to_string(), cwasm_checksum))
    }

    /// Presigned GET URL for a stored widget bundle, when the version ships one.
    pub async fn sign_widget_bundle_url(
        &self,
        package_id: &str,
        version: &str,
    ) -> flow_like_types::Result<Option<String>> {
        let record = wasm_package_version::Entity::find()
            .filter(wasm_package_version::Column::PackageId.eq(package_id))
            .filter(wasm_package_version::Column::Version.eq(version))
            .one(&self.db)
            .await?;

        let has_bundle = record
            .as_ref()
            .and_then(|r| r.widget_bundle_hash.as_deref())
            .is_some_and(|h| !h.is_empty());
        if !has_bundle {
            return Ok(None);
        }

        let path = Self::widget_bundle_path(package_id, version);
        let url = self
            .content_bucket
            .sign("GET", &path, Duration::from_secs(3600))
            .await?;
        Ok(Some(url.to_string()))
    }

    /// Read the stored widget bundle (`.flwb`) for a package version.
    pub async fn get_widget_bundle_bytes(
        &self,
        package_id: &str,
        version: &str,
    ) -> flow_like_types::Result<Vec<u8>> {
        let path = Self::widget_bundle_path(package_id, version);
        let data = self.content_bucket.as_generic().get(&path).await?;
        Ok(data.bytes().await?.to_vec())
    }

    /// Read one unpacked widget asset object, if the post-publish unpack has
    /// already produced it. Returns `Ok(None)` when the object does not exist.
    pub async fn get_widget_asset_object(
        &self,
        package_id: &str,
        version: &str,
        entry_path: &str,
    ) -> flow_like_types::Result<Option<Vec<u8>>> {
        let mut path = Path::from(WIDGET_ASSETS_PATH)
            .child(package_id)
            .child(version);
        for segment in entry_path.split('/').filter(|s| !s.is_empty()) {
            path = path.child(segment);
        }
        match self.content_bucket.as_generic().get(&path).await {
            Ok(data) => Ok(Some(data.bytes().await?.to_vec())),
            Err(flow_like_storage::object_store::Error::NotFound { .. }) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    /// Get the registry index (list of all publicly accessible packages)
    pub async fn get_index(&self) -> flow_like_types::Result<RegistryIndex> {
        use crate::entity::sea_orm_active_enums::WasmPackageStatus;
        use sea_orm::Condition;

        let packages = wasm_package::Entity::find()
            .filter(wasm_package::Column::Status.eq(WasmPackageStatus::Active))
            .filter(
                Condition::any()
                    .add(wasm_package::Column::Visibility.eq(WasmPackageVisibility::Public))
                    .add(
                        wasm_package::Column::Visibility
                            .eq(WasmPackageVisibility::PublicRequestAccess),
                    ),
            )
            .order_by_desc(wasm_package::Column::DownloadCount)
            .all(&self.db)
            .await?;

        let summaries: Vec<PackageSummary> = packages
            .into_iter()
            .map(|pkg| {
                let vis = visibility_to_string(&pkg.visibility);
                PackageSummary {
                    id: pkg.id,
                    name: pkg.name,
                    description: pkg.description,
                    latest_version: pkg.version,
                    download_count: pkg.download_count as u64,
                    status: PackageStatus::Active,
                    keywords: pkg.keywords.unwrap_or_default().into(),
                    verified: pkg.verified,
                    price: pkg.price,
                    visibility: vis,
                    primary_category: pkg.primary_category.as_ref().map(db_cat_to_string),
                    secondary_category: pkg.secondary_category.as_ref().map(db_cat_to_string),
                    avg_rating: pkg.avg_rating,
                    rating_count: pkg.rating_count,
                    metadata: None,
                    capabilities: capability_tags_from_json(pkg.permissions),
                }
            })
            .collect();

        Ok(RegistryIndex {
            name: "Flow-Like WASM Registry".to_string(),
            url: String::new(),
            updated_at: chrono::Utc::now(),
            packages: summaries,
        })
    }

    /// Get registry statistics (admin)
    pub async fn get_stats(&self) -> flow_like_types::Result<RegistryStats> {
        use crate::entity::sea_orm_active_enums::WasmPackageStatus;

        let total_packages = wasm_package::Entity::find().count(&self.db).await? as i64;

        let total_versions = wasm_package_version::Entity::find().count(&self.db).await? as i64;

        let active_packages = wasm_package::Entity::find()
            .filter(wasm_package::Column::Status.eq(WasmPackageStatus::Active))
            .count(&self.db)
            .await? as i64;

        let pending_packages = wasm_package::Entity::find()
            .filter(wasm_package::Column::Status.eq(WasmPackageStatus::PendingReview))
            .count(&self.db)
            .await?;

        let pending_version_packages = wasm_package::Entity::find()
            .filter(wasm_package::Column::Status.ne(WasmPackageStatus::PendingReview))
            .filter(
                wasm_package::Column::Id.in_subquery(
                    wasm_package_version::Entity::find()
                        .select_only()
                        .column(wasm_package_version::Column::PackageId)
                        .filter(
                            wasm_package_version::Column::Status
                                .eq(WasmPackageStatus::PendingReview),
                        )
                        .into_query(),
                ),
            )
            .count(&self.db)
            .await?;

        let pending_review = (pending_packages + pending_version_packages) as i64;

        let rejected_packages = wasm_package::Entity::find()
            .filter(wasm_package::Column::Status.eq(WasmPackageStatus::Rejected))
            .count(&self.db)
            .await? as i64;

        let verified_packages = wasm_package::Entity::find()
            .filter(wasm_package::Column::Verified.eq(true))
            .count(&self.db)
            .await? as i64;

        // Sum all download counts. SUM over zero rows is NULL, so the decoded
        // value must be Option<i64>; `.one()` adds the outer row Option.
        let downloads_result: Option<Option<i64>> = wasm_package::Entity::find()
            .select_only()
            .expr_as(
                Expr::cust(r#"CAST(SUM("downloadCount") AS BIGINT)"#),
                "total",
            )
            .into_tuple()
            .one(&self.db)
            .await?;

        Ok(RegistryStats {
            total_packages,
            total_versions,
            total_downloads: downloads_result.flatten().unwrap_or(0),
            pending_review,
            active_packages,
            rejected_packages,
            verified_packages,
        })
    }

    /// Get a package entry by ID (public - only returns active packages)
    pub async fn get_package(&self, id: &str) -> flow_like_types::Result<Option<RegistryEntry>> {
        use crate::entity::sea_orm_active_enums::{WasmPackageStatus, WasmPackageVisibility};

        let Some(pkg) = wasm_package::Entity::find_by_id(id)
            .filter(wasm_package::Column::Status.eq(WasmPackageStatus::Active))
            .one(&self.db)
            .await?
        else {
            return Ok(None);
        };

        // Private packages always expose all versions to anyone who has access.
        let show_all = pkg.visibility == WasmPackageVisibility::Private;
        self.build_registry_entry(pkg, show_all).await.map(Some)
    }

    /// Get a package entry by ID regardless of status — for owners/maintainers
    /// who need to see their own pending/disabled packages.
    pub async fn get_package_any_status(
        &self,
        id: &str,
    ) -> flow_like_types::Result<Option<RegistryEntry>> {
        let Some(pkg) = wasm_package::Entity::find_by_id(id).one(&self.db).await? else {
            return Ok(None);
        };

        // any-status callers are always maintainers/admins — show everything.
        self.build_registry_entry(pkg, true).await.map(Some)
    }

    /// Fetch a package applying correct version-visibility rules for a given viewer:
    /// - Private → all versions (to any caller with access)
    /// - Public / PublicRequestAccess + owner/maintainer → all versions
    /// - Public / PublicRequestAccess + regular user → approved versions only
    ///
    /// Access control (who may call this) is the caller's responsibility.
    pub async fn get_package_as_viewer(
        &self,
        id: &str,
        viewer_sub: Option<&str>,
    ) -> flow_like_types::Result<Option<RegistryEntry>> {
        use crate::entity::sea_orm_active_enums::WasmPackageVisibility;

        let Some(pkg) = wasm_package::Entity::find_by_id(id).one(&self.db).await? else {
            return Ok(None);
        };

        let show_all = match pkg.visibility {
            WasmPackageVisibility::Private => true,
            _ => match viewer_sub {
                Some(sub) => self.can_view_unapproved_versions(sub, id).await?,
                None => false,
            },
        };

        self.build_registry_entry(pkg, show_all).await.map(Some)
    }

    /// Returns `true` when `user_id` can manage `package_id`.
    async fn can_view_unapproved_versions(
        &self,
        user_id: &str,
        package_id: &str,
    ) -> flow_like_types::Result<bool> {
        let record = wasm_package_user::Entity::find()
            .filter(wasm_package_user::Column::PackageId.eq(package_id))
            .filter(wasm_package_user::Column::UserId.eq(user_id))
            .one(&self.db)
            .await?;
        let Some(record) = record else {
            return Ok(false);
        };

        let permission =
            crate::permission::wasm_package_permission::WasmPackagePermission::from_bits_truncate(
                record.permission,
            );
        Ok(permission.has_permission(
            crate::permission::wasm_package_permission::WasmPackagePermission::Maintainer,
        ))
    }

    async fn latest_pending_version(
        &self,
        package_id: &str,
    ) -> flow_like_types::Result<Option<wasm_package_version::Model>> {
        use crate::entity::sea_orm_active_enums::WasmPackageStatus;

        let version = wasm_package_version::Entity::find()
            .filter(wasm_package_version::Column::PackageId.eq(package_id))
            .filter(wasm_package_version::Column::Status.eq(WasmPackageStatus::PendingReview))
            .order_by_desc(wasm_package_version::Column::PublishedAt)
            .one(&self.db)
            .await?;
        Ok(version)
    }

    async fn build_package_details(
        &self,
        pkg: wasm_package::Model,
        review_version: Option<wasm_package_version::Model>,
    ) -> flow_like_types::Result<PackageDetails> {
        let authors = self.get_package_authors(&pkg.id).await?;

        let (
            version,
            status,
            wasm_size,
            nodes,
            widgets,
            widget_bundle_hash,
            widget_bundle_size,
            published_at,
        ) = if let Some(version) = review_version {
            (
                version.version,
                status_to_string(&version.status),
                version.wasm_size as u64,
                version.nodes,
                version.widgets,
                version.widget_bundle_hash.filter(|h| !h.is_empty()),
                version.widget_bundle_size.map(|s| s as u64),
                Some(version.published_at.to_utc()),
            )
        } else {
            (
                pkg.version.clone(),
                status_to_string(&pkg.status),
                pkg.wasm_size as u64,
                pkg.nodes.clone(),
                pkg.widgets.clone(),
                pkg.widget_bundle_hash.clone().filter(|h| !h.is_empty()),
                pkg.widget_bundle_size.map(|s| s as u64),
                pkg.published_at.map(|dt| dt.to_utc()),
            )
        };

        Ok(PackageDetails {
            id: pkg.id,
            name: pkg.name,
            description: pkg.description,
            version,
            authors,
            license: pkg.license,
            homepage: pkg.homepage,
            repository: pkg.repository,
            keywords: pkg.keywords.unwrap_or_default().into(),
            status,
            verified: pkg.verified,
            download_count: pkg.download_count as u64,
            wasm_size,
            nodes,
            widgets,
            widget_bundle_hash,
            widget_bundle_size,
            permissions: pkg.permissions,
            price: pkg.price,
            visibility: visibility_to_string(&pkg.visibility),
            primary_category: pkg.primary_category.as_ref().map(db_cat_to_string),
            secondary_category: pkg.secondary_category.as_ref().map(db_cat_to_string),
            created_at: pkg.created_at.to_utc(),
            updated_at: pkg.updated_at.to_utc(),
            published_at,
        })
    }

    /// Get a package entry by ID (admin - returns any status)
    pub async fn get_package_admin(
        &self,
        id: &str,
    ) -> flow_like_types::Result<Option<PackageDetails>> {
        let Some(pkg) = wasm_package::Entity::find_by_id(id).one(&self.db).await? else {
            return Ok(None);
        };

        let review_version = self.latest_pending_version(&pkg.id).await?;
        self.build_package_details(pkg, review_version)
            .await
            .map(Some)
    }

    /// Get authors for a package with user information
    async fn get_package_authors(
        &self,
        package_id: &str,
    ) -> flow_like_types::Result<Vec<AuthorInfo>> {
        let author_records = wasm_package_author::Entity::find()
            .filter(wasm_package_author::Column::PackageId.eq(package_id))
            .all(&self.db)
            .await?;

        let mut authors = Vec::new();
        for record in author_records {
            let user_info = user::Entity::find_by_id(&record.user_id)
                .one(&self.db)
                .await?;

            authors.push(AuthorInfo {
                user_id: record.user_id,
                username: user_info.as_ref().and_then(|u| u.username.clone()),
                name: user_info.as_ref().and_then(|u| u.name.clone()),
                avatar: user_info.and_then(|u| u.avatar),
                role: record.role,
            });
        }

        Ok(authors)
    }

    async fn get_user_author_info(
        &self,
        user_id: &str,
    ) -> flow_like_types::Result<Option<AuthorInfo>> {
        let user_info = user::Entity::find_by_id(user_id).one(&self.db).await?;

        Ok(user_info.map(|user_info| AuthorInfo {
            user_id: user_id.to_string(),
            username: user_info.username,
            name: user_info.name,
            avatar: user_info.avatar,
            role: None,
        }))
    }

    /// Build a RegistryEntry from a package model.
    /// `show_all_versions`: when `true`, all versions are included regardless of
    /// approval status; when `false`, only `Active` versions are returned.
    async fn build_registry_entry(
        &self,
        pkg: wasm_package::Model,
        show_all_versions: bool,
    ) -> flow_like_types::Result<RegistryEntry> {
        use crate::entity::sea_orm_active_enums::WasmPackageStatus;

        let mut version_query = wasm_package_version::Entity::find()
            .filter(wasm_package_version::Column::PackageId.eq(&pkg.id))
            .order_by_desc(wasm_package_version::Column::PublishedAt);

        if !show_all_versions {
            version_query = version_query
                .filter(wasm_package_version::Column::Status.eq(WasmPackageStatus::Active));
        }

        let versions = version_query.all(&self.db).await?;

        // Get authors from junction table
        let author_infos = self.get_package_authors(&pkg.id).await?;
        let authors: Vec<flow_like_wasm_schema::manifest::PackageAuthor> = author_infos
            .into_iter()
            .map(|a| flow_like_wasm_schema::manifest::PackageAuthor {
                name: a.name.or(a.username).unwrap_or(a.user_id),
                email: None,
                url: None,
            })
            .collect();

        // Widgets from the parent package row; fall back to the latest
        // fetched version (mirrors the nodes fallback below) so pending
        // widget packages surface their widgets before approval.
        let mut widgets: Vec<PackageWidgetEntry> =
            serde_json::from_value(pkg.widgets.clone()).unwrap_or_default();
        let mut widget_bundle_hash = pkg.widget_bundle_hash.clone().filter(|h| !h.is_empty());
        if let Some(latest_v) = versions.first() {
            if widgets.is_empty() {
                widgets = serde_json::from_value(latest_v.widgets.clone()).unwrap_or_default();
            }
            if widget_bundle_hash.is_none() {
                widget_bundle_hash = latest_v
                    .widget_bundle_hash
                    .clone()
                    .filter(|h| !h.is_empty());
            }
        }

        let manifest = PackageManifest {
            manifest_version: flow_like_wasm_schema::manifest::MANIFEST_VERSION,
            id: pkg.id.clone(),
            name: pkg.name.clone(),
            description: pkg.description.clone(),
            version: pkg.version.clone(),
            authors,
            license: pkg.license.clone(),
            homepage: pkg.homepage.clone(),
            repository: pkg.repository.clone(),
            keywords: pkg.keywords.unwrap_or_default().into(),
            permissions: serde_json::from_value(pkg.permissions.clone()).unwrap_or_default(),
            primary_category: pkg.primary_category.as_ref().map(db_cat_to_manifest),
            secondary_category: pkg.secondary_category.as_ref().map(db_cat_to_manifest),
            min_flow_like_version: None,
            wasm_path: Some(pkg.wasm_path.clone()),
            wasm_hash: Some(pkg.wasm_hash.clone()),
            widgets,
            widget_bundle_path: None,
            widget_bundle_hash,
            metadata: Default::default(),
        };

        let package_versions: Vec<PackageVersion> = versions
            .into_iter()
            .map(package_version_from_model)
            .collect();

        // Prefer nodes from the latest version; fall back to the parent package.
        let mut nodes: Vec<PackageNodeEntry> =
            serde_json::from_value(pkg.nodes.clone()).unwrap_or_default();
        if nodes.is_empty()
            && let Some(latest_v) = wasm_package_version::Entity::find()
                .filter(wasm_package_version::Column::PackageId.eq(&pkg.id))
                .order_by_desc(wasm_package_version::Column::PublishedAt)
                .one(&self.db)
                .await?
        {
            nodes = serde_json::from_value(latest_v.nodes).unwrap_or_default();
        }

        let vis = visibility_to_string(&pkg.visibility);
        let price = pkg.price;

        Ok(RegistryEntry {
            id: pkg.id.clone(),
            manifest,
            nodes,
            versions: package_versions,
            status: status_to_package_status(&pkg.status),
            download_count: pkg.download_count as u64,
            created_at: pkg.created_at.to_utc(),
            updated_at: pkg.updated_at.to_utc(),
            source: PackageSource::Remote {
                registry_url: String::new(),
                download_url: String::new(),
            },
            verified: pkg.verified,
            price,
            visibility: vis,
            avg_rating: pkg.avg_rating,
            rating_count: pkg.rating_count,
            rating_sum: pkg.rating_sum,
            current_user_permission: None,
        })
    }

    /// Search packages with filters
    pub async fn search(&self, filters: &SearchFilters) -> flow_like_types::Result<SearchResults> {
        use crate::entity::sea_orm_active_enums::WasmPackageStatus;

        let mut query = wasm_package::Entity::find();

        // Only show active packages unless including deprecated
        if !filters.include_deprecated {
            query = query.filter(wasm_package::Column::Status.eq(WasmPackageStatus::Active));
        }

        // Only show public-facing packages in the basic search
        query = query.filter(
            sea_orm::Condition::any()
                .add(wasm_package::Column::Visibility.eq(WasmPackageVisibility::Public))
                .add(
                    wasm_package::Column::Visibility.eq(WasmPackageVisibility::PublicRequestAccess),
                ),
        );

        // Filter by verified only
        if filters.verified_only {
            query = query.filter(wasm_package::Column::Verified.eq(true));
        }

        // Text search (name, description, keywords)
        if let Some(q) = &filters.query {
            let pattern = format!("%{}%", q.to_lowercase());
            query = query.filter(
                wasm_package::Column::Name
                    .contains(&pattern)
                    .or(wasm_package::Column::Description.contains(&pattern))
                    .or(wasm_package::Column::Id.contains(&pattern)),
            );
        }

        // Filter by category
        if let Some(cat) = &filters.category
            && let Ok(cat_enum) = serde_json::from_value::<WasmPackageCategory>(
                serde_json::Value::String(cat.clone()),
            )
        {
            query = query.filter(
                sea_orm::Condition::any()
                    .add(wasm_package::Column::PrimaryCategory.eq(cat_enum.clone()))
                    .add(wasm_package::Column::SecondaryCategory.eq(cat_enum)),
            );
        }

        // Get total count before pagination
        let total_count = query.clone().count(&self.db).await? as usize;

        // Apply sorting
        query = match filters.sort_by {
            SortField::Downloads => {
                if filters.sort_desc {
                    query.order_by_desc(wasm_package::Column::DownloadCount)
                } else {
                    query.order_by_asc(wasm_package::Column::DownloadCount)
                }
            }
            SortField::Name => {
                if filters.sort_desc {
                    query.order_by_desc(wasm_package::Column::Name)
                } else {
                    query.order_by_asc(wasm_package::Column::Name)
                }
            }
            SortField::UpdatedAt => {
                if filters.sort_desc {
                    query.order_by_desc(wasm_package::Column::UpdatedAt)
                } else {
                    query.order_by_asc(wasm_package::Column::UpdatedAt)
                }
            }
            SortField::CreatedAt => {
                if filters.sort_desc {
                    query.order_by_desc(wasm_package::Column::CreatedAt)
                } else {
                    query.order_by_asc(wasm_package::Column::CreatedAt)
                }
            }
            SortField::Relevance => {
                // For relevance, sort by downloads as a proxy for popularity
                if filters.sort_desc {
                    query.order_by_desc(wasm_package::Column::DownloadCount)
                } else {
                    query.order_by_asc(wasm_package::Column::DownloadCount)
                }
            }
        };

        // Apply pagination
        let packages = query
            .offset(filters.offset as u64)
            .limit(filters.limit as u64)
            .all(&self.db)
            .await?;

        let language = filters.language.as_deref().unwrap_or("en");
        let meta_map = self.fetch_meta_map(&packages, language).await?;

        let summaries: Vec<PackageSummary> = packages
            .into_iter()
            .map(|pkg| {
                let vis = visibility_to_string(&pkg.visibility);
                let resolved_meta = meta_map
                    .get(&pkg.id)
                    .and_then(|metas| MetaSummary::pick_best(metas, language))
                    .map(MetaSummary::from_model);
                PackageSummary {
                    id: pkg.id,
                    name: pkg.name,
                    description: pkg.description,
                    latest_version: pkg.version,
                    download_count: pkg.download_count as u64,
                    status: status_to_package_status(&pkg.status),
                    keywords: pkg.keywords.unwrap_or_default().into(),
                    verified: pkg.verified,
                    price: pkg.price,
                    visibility: vis,
                    primary_category: pkg.primary_category.as_ref().map(db_cat_to_string),
                    secondary_category: pkg.secondary_category.as_ref().map(db_cat_to_string),
                    avg_rating: pkg.avg_rating,
                    rating_count: pkg.rating_count,
                    metadata: resolved_meta,
                    capabilities: capability_tags_from_json(pkg.permissions),
                }
            })
            .collect();

        Ok(SearchResults {
            packages: summaries,
            total_count,
            offset: filters.offset,
            limit: filters.limit,
        })
    }

    /// Search packages with visibility filtering.
    /// Public packages are always returned. When `include_own` is true and a
    /// caller_id is provided, private packages the caller has access to are
    /// included as well. When `owned_only` is true, only packages the caller
    /// has explicit access to (via wasm_package_user) are returned.
    pub async fn search_with_visibility(
        &self,
        filters: &SearchFilters,
        caller_id: Option<&str>,
        include_own: bool,
        owned_only: bool,
    ) -> flow_like_types::Result<SearchResults> {
        use crate::entity::sea_orm_active_enums::WasmPackageStatus;
        use sea_orm::Condition;

        let mut query = wasm_package::Entity::find();

        if filters.verified_only {
            query = query.filter(wasm_package::Column::Verified.eq(true));
        }

        // Combined status + visibility filtering.
        // The user's own packages bypass the Active-only status filter so they
        // can see PendingReview / Disabled / etc. packages they own.
        if owned_only {
            if let Some(uid) = caller_id {
                let user_package_ids: Vec<String> = wasm_package_user::Entity::find()
                    .filter(wasm_package_user::Column::UserId.eq(uid))
                    .all(&self.db)
                    .await?
                    .into_iter()
                    .map(|r| r.package_id)
                    .collect();

                let mut cond =
                    Condition::all().add(wasm_package::Column::Id.is_in(user_package_ids));
                if !filters.include_deprecated {
                    cond = cond.add(wasm_package::Column::Status.ne(WasmPackageStatus::Deprecated));
                }
                if !filters.include_disabled {
                    cond = cond.add(wasm_package::Column::Status.ne(WasmPackageStatus::Disabled));
                }
                query = query.filter(cond);
            } else {
                return Ok(SearchResults {
                    packages: vec![],
                    total_count: 0,
                    offset: filters.offset,
                    limit: filters.limit,
                });
            }
        } else {
            match (caller_id, include_own) {
                (Some(uid), true) => {
                    let user_package_ids: Vec<String> = wasm_package_user::Entity::find()
                        .filter(wasm_package_user::Column::UserId.eq(uid))
                        .all(&self.db)
                        .await?
                        .into_iter()
                        .map(|r| r.package_id)
                        .collect();

                    // Public/PublicRequestAccess must be Active (unless include_deprecated),
                    // but user's own packages bypass the status filter.
                    let mut public_cond = Condition::any()
                        .add(wasm_package::Column::Visibility.eq(WasmPackageVisibility::Public))
                        .add(
                            wasm_package::Column::Visibility
                                .eq(WasmPackageVisibility::PublicRequestAccess),
                        );

                    if !filters.include_deprecated {
                        public_cond = Condition::all()
                            .add(public_cond)
                            .add(wasm_package::Column::Status.eq(WasmPackageStatus::Active));
                    }

                    let mut own_cond =
                        Condition::all().add(wasm_package::Column::Id.is_in(user_package_ids));
                    if !filters.include_deprecated {
                        own_cond = own_cond
                            .add(wasm_package::Column::Status.ne(WasmPackageStatus::Deprecated));
                    }

                    query = query.filter(Condition::any().add(public_cond).add(own_cond));
                }
                _ => {
                    query = query.filter(
                        Condition::any()
                            .add(wasm_package::Column::Visibility.eq(WasmPackageVisibility::Public))
                            .add(
                                wasm_package::Column::Visibility
                                    .eq(WasmPackageVisibility::PublicRequestAccess),
                            ),
                    );
                    if !filters.include_deprecated {
                        query = query
                            .filter(wasm_package::Column::Status.eq(WasmPackageStatus::Active));
                    }
                }
            }
        }

        if let Some(q) = &filters.query {
            let pattern = format!("%{}%", q.to_lowercase());
            query = query.filter(
                wasm_package::Column::Name
                    .contains(&pattern)
                    .or(wasm_package::Column::Description.contains(&pattern))
                    .or(wasm_package::Column::Id.contains(&pattern)),
            );
        }

        // Filter by category
        if let Some(cat) = &filters.category
            && let Ok(cat_enum) = serde_json::from_value::<WasmPackageCategory>(
                serde_json::Value::String(cat.clone()),
            )
        {
            query = query.filter(
                sea_orm::Condition::any()
                    .add(wasm_package::Column::PrimaryCategory.eq(cat_enum.clone()))
                    .add(wasm_package::Column::SecondaryCategory.eq(cat_enum)),
            );
        }

        let total_count = query.clone().count(&self.db).await? as usize;

        query = match filters.sort_by {
            SortField::Downloads => {
                if filters.sort_desc {
                    query.order_by_desc(wasm_package::Column::DownloadCount)
                } else {
                    query.order_by_asc(wasm_package::Column::DownloadCount)
                }
            }
            SortField::Name => {
                if filters.sort_desc {
                    query.order_by_desc(wasm_package::Column::Name)
                } else {
                    query.order_by_asc(wasm_package::Column::Name)
                }
            }
            SortField::UpdatedAt => {
                if filters.sort_desc {
                    query.order_by_desc(wasm_package::Column::UpdatedAt)
                } else {
                    query.order_by_asc(wasm_package::Column::UpdatedAt)
                }
            }
            SortField::CreatedAt => {
                if filters.sort_desc {
                    query.order_by_desc(wasm_package::Column::CreatedAt)
                } else {
                    query.order_by_asc(wasm_package::Column::CreatedAt)
                }
            }
            SortField::Relevance => {
                if filters.sort_desc {
                    query.order_by_desc(wasm_package::Column::DownloadCount)
                } else {
                    query.order_by_asc(wasm_package::Column::DownloadCount)
                }
            }
        };

        let packages = query
            .offset(filters.offset as u64)
            .limit(filters.limit as u64)
            .all(&self.db)
            .await?;

        let language = filters.language.as_deref().unwrap_or("en");
        let meta_map = self.fetch_meta_map(&packages, language).await?;

        let summaries: Vec<PackageSummary> = packages
            .into_iter()
            .map(|pkg| {
                let vis = visibility_to_string(&pkg.visibility);
                let resolved_meta = meta_map
                    .get(&pkg.id)
                    .and_then(|metas| MetaSummary::pick_best(metas, language))
                    .map(MetaSummary::from_model);
                PackageSummary {
                    id: pkg.id,
                    name: pkg.name,
                    description: pkg.description,
                    latest_version: pkg.version,
                    download_count: pkg.download_count as u64,
                    status: status_to_package_status(&pkg.status),
                    keywords: pkg.keywords.unwrap_or_default().into(),
                    verified: pkg.verified,
                    price: pkg.price,
                    visibility: vis,
                    primary_category: pkg.primary_category.as_ref().map(db_cat_to_string),
                    secondary_category: pkg.secondary_category.as_ref().map(db_cat_to_string),
                    avg_rating: pkg.avg_rating,
                    rating_count: pkg.rating_count,
                    metadata: resolved_meta,
                    capabilities: capability_tags_from_json(pkg.permissions),
                }
            })
            .collect();

        Ok(SearchResults {
            packages: summaries,
            total_count,
            offset: filters.offset,
            limit: filters.limit,
        })
    }

    /// Get download URL for a package (signed URL or CDN URL)
    pub async fn get_wasm_url(
        &self,
        package_id: &str,
        version: Option<&str>,
    ) -> flow_like_types::Result<(String, PackageManifest, String)> {
        let Some(entry) = self.get_package(package_id).await? else {
            return Err(flow_like_types::anyhow!(
                "Package not found: {}",
                package_id
            ));
        };

        // Prefer the version from the approved-versions list; fall back to the
        // version field on the package row itself (e.g. still pending approval).
        let version_str = if let Some(v) = version {
            entry
                .get_version(v)
                .map(|v| v.version.clone())
                .ok_or_else(|| flow_like_types::anyhow!("Version not found: {}", v))?
        } else {
            entry
                .latest_version()
                .map(|v| v.version.clone())
                .unwrap_or_else(|| entry.manifest.version.clone())
        };

        let download_url = self.get_download_url(package_id, &version_str).await?;

        Ok((download_url, entry.manifest, version_str))
    }

    /// Get download URL for a package using viewer-aware package resolution.
    ///
    /// The caller is responsible for enforcing access control and deciding
    /// whether non-active packages should be reachable for the provided viewer.
    pub async fn get_wasm_url_as_viewer(
        &self,
        package_id: &str,
        version: Option<&str>,
        viewer_sub: Option<&str>,
    ) -> flow_like_types::Result<(Option<String>, PackageManifest, String)> {
        let Some(entry) = self.get_package_as_viewer(package_id, viewer_sub).await? else {
            return Err(flow_like_types::anyhow!(
                "Package not found: {}",
                package_id
            ));
        };

        let version_str = if let Some(v) = version {
            entry
                .get_version(v)
                .map(|v| v.version.clone())
                .ok_or_else(|| flow_like_types::anyhow!("Version not found: {}", v))?
        } else {
            entry
                .latest_version()
                .map(|v| v.version.clone())
                .unwrap_or_else(|| entry.manifest.version.clone())
        };

        // Widgets-only packages carry no WASM artifact to sign
        let download_url = if manifest_has_wasm(&entry.manifest) {
            Some(self.get_download_url(package_id, &version_str).await?)
        } else {
            None
        };

        Ok((download_url, entry.manifest, version_str))
    }

    /// Download package WASM binary directly (for backward compatibility)
    pub async fn download_wasm(
        &self,
        package_id: &str,
        version: Option<&str>,
    ) -> flow_like_types::Result<(Vec<u8>, PackageManifest, String)> {
        let Some(entry) = self.get_package(package_id).await? else {
            return Err(flow_like_types::anyhow!(
                "Package not found: {}",
                package_id
            ));
        };

        let ver = if let Some(v) = version {
            entry
                .get_version(v)
                .ok_or_else(|| flow_like_types::anyhow!("Version not found: {}", v))?
                .clone()
        } else {
            entry
                .latest_version()
                .ok_or_else(|| flow_like_types::anyhow!("No versions available"))?
                .clone()
        };

        let path = self.resolve_wasm_path(package_id, &ver.version).await?;
        let data = self.content_bucket.as_generic().get(&path).await?;
        let bytes = data.bytes().await?.to_vec();

        Ok((bytes, entry.manifest, ver.version))
    }
    /// Finalize a two-step publish: construct the tmp path from the submitter
    /// identity, fetch WASM, hash, move to final location, create DB records.
    pub async fn finalize_publish(
        &self,
        manifest: PackageManifest,
        submitter_id: &str,
        _submitter_email: Option<String>,
    ) -> flow_like_types::Result<PublishResponse> {
        use crate::entity::sea_orm_active_enums::{WasmPackageStatus, WasmReviewAction};

        let now = chrono::Utc::now().fixed_offset();

        let existing_version = wasm_package_version::Entity::find()
            .filter(wasm_package_version::Column::PackageId.eq(&manifest.id))
            .filter(wasm_package_version::Column::Version.eq(&manifest.version))
            .one(&self.db)
            .await?;

        if existing_version.is_some() {
            return Err(flow_like_types::anyhow!(
                "Version {} already exists for package {}",
                manifest.version,
                manifest.id
            ));
        }

        // A package must ship at least one node (WASM artifact) or one widget.
        // The WASM upload stays mandatory for packages without widgets; a
        // missing WASM upload is only tolerated for widgets-only packages.
        let tmp_path = format!(
            "tmp/wasm/{}/{}/{}.wasm",
            submitter_id, manifest.id, manifest.version
        );
        let tmp_object_path = Path::from(tmp_path.as_str());
        let wasm_data: Option<Vec<u8>> = match self
            .content_bucket
            .as_generic()
            .get(&tmp_object_path)
            .await
        {
            Ok(data) => Some(data.bytes().await?.to_vec()),
            Err(_) if !manifest.widgets.is_empty() => None,
            Err(_) => {
                return Err(flow_like_types::anyhow!(
                    "WASM not found at tmp path — upload may have failed. A package must ship at least one node (WASM artifact) or one widget."
                ));
            }
        };
        let has_wasm = wasm_data.is_some();

        if let Some(wasm) = &wasm_data
            && (wasm.len() < 8 || &wasm[0..4] != b"\0asm")
        {
            return Err(flow_like_types::anyhow!("Invalid WASM binary"));
        }

        let hash = wasm_data
            .as_ref()
            .map(|wasm| blake3::hash(wasm).to_hex().to_string())
            .unwrap_or_default();
        let size = wasm_data
            .as_ref()
            .map(|wasm| wasm.len() as i64)
            .unwrap_or(0);

        // Check for hash duplicates (non-blocking flag)
        let duplicate_info = if has_wasm {
            wasm_package_version::Entity::find()
                .filter(wasm_package_version::Column::WasmHash.eq(&hash))
                .one(&self.db)
                .await?
        } else {
            None
        };

        let (dup_pkg_id, dup_version, dup_flagged) = if let Some(dup) = &duplicate_info {
            (
                Some(dup.package_id.clone()),
                Some(dup.version.clone()),
                true,
            )
        } else {
            (None, None, false)
        };

        // Validate, store, and unpack the widget bundle when widgets are declared
        let widgets_json = serde_json::to_value(&manifest.widgets)?;
        let mut widget_bundle_hash: Option<String> = None;
        let mut widget_bundle_size: Option<i64> = None;
        if !manifest.widgets.is_empty() {
            let bundle_tmp_path = format!(
                "tmp/widget-bundles/{}/{}/{}.flwb",
                submitter_id, manifest.id, manifest.version
            );
            let bundle_data = self
                .content_bucket
                .as_generic()
                .get(&Path::from(bundle_tmp_path.as_str()))
                .await
                .map_err(|_| {
                    flow_like_types::anyhow!(
                        "Widget bundle not found at tmp path — upload may have failed. Packages declaring widgets must upload a widget bundle."
                    )
                })?;
            let bundle_bytes = bundle_data.bytes().await?.to_vec();

            let (bundle_hash, bundle_size) = {
                let manifest_for_validation = manifest.clone();
                let bytes = bundle_bytes.clone();
                flow_like_types::tokio::task::spawn_blocking(move || {
                    validate_manifest_widget_bundle(&manifest_for_validation, &bytes)
                })
                .await??
            };

            let final_bundle_path = Self::widget_bundle_path(&manifest.id, &manifest.version);
            self.content_bucket
                .as_generic()
                .put(&final_bundle_path, PutPayload::from(bundle_bytes.clone()))
                .await?;

            // Unpack into widget-assets/ so the web app can load entries from
            // the CDN. The widget-asset fallback route serves entries straight
            // from the stored bundle, so a failure here degrades performance,
            // not correctness.
            if let Err(e) = unpack_widget_bundle_to_assets(
                &self.content_bucket,
                &manifest.id,
                &manifest.version,
                bundle_bytes,
            )
            .await
            {
                tracing::error!(
                    pkg = %manifest.id,
                    ver = %manifest.version,
                    err = %e,
                    "Failed to unpack widget bundle into widget-assets"
                );
            }

            widget_bundle_hash = Some(bundle_hash);
            widget_bundle_size = Some(bundle_size);
        }

        // Move WASM from tmp to final path
        let final_wasm_path = Self::wasm_path(&manifest.id, &manifest.version);
        if let Some(wasm) = wasm_data {
            self.content_bucket
                .as_generic()
                .put(&final_wasm_path, PutPayload::from(wasm))
                .await?;
        }
        let stored_wasm_path = if has_wasm {
            final_wasm_path.to_string()
        } else {
            String::new()
        };

        let existing_package = wasm_package::Entity::find_by_id(&manifest.id)
            .one(&self.db)
            .await?;
        let is_existing_package = existing_package.is_some();

        let is_private_package = existing_package
            .as_ref()
            .map(|p| p.visibility == WasmPackageVisibility::Private)
            .unwrap_or(true);

        // `DELETE /admin/packages/{id}` answers `202` with this row still
        // present and only disabled, so a re-publish under the same manifest
        // id reaches either branch below while the drain is still pending.
        // Cancelling in the transaction that writes the row is what stops the
        // worker from later deleting the package that was just republished.
        let package_write = self.db.begin().await?;
        job::cancel(&package_write, DeletionRoot::WasmPackage, &manifest.id).await?;
        if existing_package.is_some() {
            // Existing package: only bump updated_at.
            // Version-specific fields (version, wasm_path, wasm_hash, nodes, etc.)
            // are NOT updated on the parent package until this version is approved.
            let update_model = wasm_package::ActiveModel {
                id: Set(manifest.id.clone()),
                updated_at: Set(now),
                ..Default::default()
            };
            update_model.update(&package_write).await?;
        } else {
            let package_model = wasm_package::ActiveModel {
                id: Set(manifest.id.clone()),
                name: Set(manifest.name.clone()),
                description: Set(manifest.description.clone()),
                version: Set(manifest.version.clone()),
                license: Set(manifest.license.clone()),
                homepage: Set(manifest.homepage.clone()),
                repository: Set(manifest.repository.clone()),
                keywords: Set(Some(manifest.keywords.clone().into())),
                primary_category: Set(manifest.primary_category.as_ref().map(manifest_cat_to_db)),
                secondary_category: Set(manifest
                    .secondary_category
                    .as_ref()
                    .map(manifest_cat_to_db)),
                status: Set(WasmPackageStatus::Active),
                visibility: Set(WasmPackageVisibility::Private),
                verified: Set(false),
                download_count: Set(0),
                wasm_path: Set(stored_wasm_path.clone()),
                wasm_hash: Set(hash.clone()),
                wasm_size: Set(size),
                widget_bundle_hash: Set(widget_bundle_hash.clone()),
                widget_bundle_size: Set(widget_bundle_size),
                nodes: Set(serde_json::Value::Array(vec![])),
                widgets: Set(widgets_json.clone()),
                permissions: Set(serde_json::to_value(&manifest.permissions)?),
                readme: Set(None),
                price: Set(0),
                created_at: Set(now),
                updated_at: Set(now),
                published_at: Set(None),
                rating_sum: Set(0),
                rating_count: Set(0),
                avg_rating: Set(None),
            };
            package_model.insert(&package_write).await?;

            let user_model = wasm_package_user::ActiveModel {
                id: Set(create_id()),
                package_id: Set(manifest.id.clone()),
                user_id: Set(submitter_id.to_string()),
                permission: Set(
                    crate::permission::wasm_package_permission::WasmPackagePermission::Owner.bits(),
                ),
                granted_by: Set(None),
                granted_at: Set(now),
            };
            user_model.insert(&package_write).await?;

            // Auto-create default English meta from manifest
            let meta_model = meta::ActiveModel {
                id: Set(create_id()),
                lang: Set("en".to_string()),
                name: Set(manifest.name.clone()),
                description: Set(Some(manifest.description.clone())),
                long_description: Set(None),
                tags: Set(if manifest.keywords.is_empty() {
                    None
                } else {
                    Some(manifest.keywords.clone().into())
                }),
                icon: Set(None),
                thumbnail: Set(None),
                website: Set(manifest.homepage.clone()),
                support_url: Set(None),
                docs_url: Set(None),
                use_case: Set(None),
                release_notes: Set(None),
                preview_media: Set(None),
                age_rating: Set(None),
                wasm_package_id: Set(Some(manifest.id.clone())),
                group_id: Set(None),
                app_id: Set(None),
                bit_id: Set(None),
                course_id: Set(None),
                template_id: Set(None),
                widget_id: Set(None),
                organization_specific_values: Set(None),
                created_at: Set(now),
                updated_at: Set(now),
            };
            meta_model.insert(&package_write).await?;
        }
        package_write.commit().await?;

        let version_id = create_id();
        let compile_hash = hash.clone();
        let version_model = wasm_package_version::ActiveModel {
            id: Set(version_id.clone()),
            package_id: Set(manifest.id.clone()),
            version: Set(manifest.version.clone()),
            wasm_path: Set(stored_wasm_path.clone()),
            wasm_hash: Set(hash.clone()),
            wasm_size: Set(size),
            widget_bundle_hash: Set(widget_bundle_hash.clone()),
            widget_bundle_size: Set(widget_bundle_size),
            nodes: Set(serde_json::Value::Array(vec![])),
            widgets: Set(widgets_json.clone()),
            release_notes: Set(None),
            min_flow_like_version: Set(None),
            yanked: Set(false),
            status: Set(WasmPackageStatus::PendingReview),
            // Widgets-only packages have nothing to compile
            compilation_status: Set(if has_wasm {
                WasmCompilationStatus::Pending
            } else {
                WasmCompilationStatus::Compiled
            }),
            compiled_platforms: Set(Some(Default::default())),
            supported_wasmtime_versions: Set(Some(Default::default())),
            compilation_error: Set(None),
            duplicate_of_package_id: Set(dup_pkg_id),
            duplicate_of_version: Set(dup_version),
            duplicate_flagged: Set(dup_flagged),
            published_at: Set(now),
            approved_at: Set(None),
        };
        version_model.insert(&self.db).await?;

        let submitted_review = wasm_package_review::ActiveModel {
            id: Set(create_id()),
            package_id: Set(manifest.id.clone()),
            reviewer_id: Set(submitter_id.to_string()),
            action: Set(WasmReviewAction::Submitted),
            comment: Set(Some(if is_existing_package {
                format!("Version {} submitted for review", manifest.version)
            } else {
                format!("Initial version {} submitted", manifest.version)
            })),
            internal_note: Set(None),
            security_score: Set(None),
            code_quality_score: Set(None),
            documentation_score: Set(None),
            created_at: Set(now),
        };
        submitted_review.insert(&self.db).await?;

        if !has_wasm {
            // Widgets-only package: nothing to compile. Auto-approve private
            // packages immediately (mirrors the inline compilation auto-approval).
            if is_private_package {
                let now_approve = chrono::Utc::now().fixed_offset();
                let _ = wasm_package_version::ActiveModel {
                    id: Set(version_id.clone()),
                    status: Set(WasmPackageStatus::Active),
                    approved_at: Set(Some(now_approve)),
                    ..Default::default()
                }
                .update(&self.db)
                .await;

                let _ = wasm_package::ActiveModel {
                    id: Set(manifest.id.clone()),
                    version: Set(manifest.version.clone()),
                    wasm_path: Set(stored_wasm_path.clone()),
                    wasm_hash: Set(compile_hash.clone()),
                    wasm_size: Set(size),
                    widgets: Set(widgets_json.clone()),
                    widget_bundle_hash: Set(widget_bundle_hash.clone()),
                    widget_bundle_size: Set(widget_bundle_size),
                    updated_at: Set(now_approve),
                    ..Default::default()
                }
                .update(&self.db)
                .await;
            }
        } else {
            // Dispatch compilation based on configured backend
            let compile_db = self.db.clone();
            let compile_pkg_id = manifest.id.clone();
            let compile_version = manifest.version.clone();
            let compile_wasm_path = final_wasm_path.to_string();

            tracing::info!(
                backend = ?self.compilation_dispatcher.as_ref().and_then(|d| d.backend()),
                "Compilation dispatch"
            );

            // Dispatched synchronously (not tokio::spawn — Lambda shuts down after response).
            // Everything the worker reports back is applied by the compilation
            // callback: node entries, wasmtime versions, private auto-approval
            // and promotion to the parent package.
            {
                let dispatcher = self.compilation_dispatcher.clone().ok_or_else(|| {
                    flow_like_types::anyhow!("Compilation dispatcher not configured")
                })?;
                let sub = submitter_id.to_string();
                let params = crate::compilation::DispatchParams {
                    package_id: compile_pkg_id.clone(),
                    version: compile_version.clone(),
                    wasm_path: compile_wasm_path,
                    wasm_hash: compile_hash,
                };
                match dispatcher.dispatch(sub, params).await {
                    Ok(resp) => {
                        tracing::info!(
                            pkg = %compile_pkg_id,
                            ver = %compile_version,
                            job_id = %resp.job_id,
                            backend = %resp.backend,
                            "Compilation job dispatched"
                        );
                    }
                    Err(e) => {
                        tracing::error!(
                            pkg = %compile_pkg_id,
                            ver = %compile_version,
                            err = %e,
                            "Failed to dispatch compilation job"
                        );
                        let _ = wasm_package_version::ActiveModel {
                            id: Set(version_id.clone()),
                            compilation_status: Set(WasmCompilationStatus::LocalOnly),
                            compilation_error: Set(Some(format!("Dispatch failed: {e}"))),
                            ..Default::default()
                        }
                        .update(&compile_db)
                        .await;
                    }
                }
            }
        }

        let message = if dup_flagged {
            "Package submitted for review. Note: WASM hash matches an existing package — flagged for review.".to_string()
        } else {
            "Package submitted for review. An admin will review it shortly.".to_string()
        };

        Ok(PublishResponse {
            success: true,
            package_id: manifest.id,
            version: manifest.version,
            message: Some(message),
        })
    }

    /// Increment download count for a package (fire and forget)
    pub async fn increment_downloads(
        &self,
        state: &crate::state::State,
        package_id: &str,
    ) -> flow_like_types::Result<()> {
        state
            .transaction(|txn| {
                let package_id = package_id.to_string();
                Box::pin(async move {
                    wasm_package::Entity::update_many()
                        .col_expr(
                            wasm_package::Column::DownloadCount,
                            Expr::col(wasm_package::Column::DownloadCount).add(1),
                        )
                        .filter(wasm_package::Column::Id.eq(package_id))
                        .exec(txn)
                        .await?;
                    Ok::<_, sea_orm::DbErr>(())
                })
            })
            .await?;
        Ok(())
    }

    /// Get all versions for a package (unfiltered — admin/internal use)
    pub async fn get_versions(
        &self,
        package_id: &str,
    ) -> flow_like_types::Result<Vec<PackageVersion>> {
        let versions = wasm_package_version::Entity::find()
            .filter(wasm_package_version::Column::PackageId.eq(package_id))
            .order_by_desc(wasm_package_version::Column::PublishedAt)
            .all(&self.db)
            .await?;

        Ok(versions
            .into_iter()
            .map(package_version_from_model)
            .collect())
    }

    /// Get only approved (Active status) versions for a package
    pub async fn get_versions_approved(
        &self,
        package_id: &str,
    ) -> flow_like_types::Result<Vec<PackageVersion>> {
        use crate::entity::sea_orm_active_enums::WasmPackageStatus;

        let versions = wasm_package_version::Entity::find()
            .filter(wasm_package_version::Column::PackageId.eq(package_id))
            .filter(wasm_package_version::Column::Status.eq(WasmPackageStatus::Active))
            .order_by_desc(wasm_package_version::Column::PublishedAt)
            .all(&self.db)
            .await?;

        Ok(versions
            .into_iter()
            .map(package_version_from_model)
            .collect())
    }

    // ==================== ADMIN METHODS ====================

    /// List packages for admin (includes all statuses)
    pub async fn list_packages_admin(
        &self,
        status_filter: Option<&str>,
        offset: usize,
        limit: usize,
    ) -> flow_like_types::Result<(Vec<PackageDetails>, usize)> {
        use crate::entity::sea_orm_active_enums::WasmPackageStatus;

        let mut query = wasm_package::Entity::find();

        if let Some(status) = status_filter {
            if status == "pending_review" {
                query = query.filter(
                    sea_orm::Condition::any()
                        .add(wasm_package::Column::Status.eq(WasmPackageStatus::PendingReview))
                        .add(
                            wasm_package::Column::Id.in_subquery(
                                wasm_package_version::Entity::find()
                                    .select_only()
                                    .column(wasm_package_version::Column::PackageId)
                                    .filter(
                                        wasm_package_version::Column::Status
                                            .eq(WasmPackageStatus::PendingReview),
                                    )
                                    .into_query(),
                            ),
                        ),
                );
            } else {
                query = query.filter(wasm_package::Column::Status.eq(status_to_enum(status)));
            }
        }

        let total_count = query.clone().count(&self.db).await? as usize;

        let packages = query
            .order_by_desc(wasm_package::Column::CreatedAt)
            .offset(offset as u64)
            .limit(limit as u64)
            .all(&self.db)
            .await?;

        let mut details: Vec<PackageDetails> = Vec::with_capacity(packages.len());
        for pkg in packages {
            let review_version = self.latest_pending_version(&pkg.id).await?;
            details.push(self.build_package_details(pkg, review_version).await?);
        }

        Ok((details, total_count))
    }

    /// Submit a review for a package
    pub async fn submit_review(
        &self,
        package_id: &str,
        reviewer_id: &str,
        review: ReviewRequest,
    ) -> flow_like_types::Result<PackageReview> {
        use crate::entity::sea_orm_active_enums::{WasmPackageStatus, WasmReviewAction};

        let now = chrono::Utc::now().fixed_offset();

        // Verify package exists
        let Some(pkg) = wasm_package::Entity::find_by_id(package_id)
            .one(&self.db)
            .await?
        else {
            return Err(flow_like_types::anyhow!(
                "Package not found: {}",
                package_id
            ));
        };

        let action = match review.action.as_str() {
            "approve" => WasmReviewAction::Approved,
            "reject" => WasmReviewAction::Rejected,
            "request_changes" => WasmReviewAction::RequestedChanges,
            "comment" => WasmReviewAction::Commented,
            "flag" => WasmReviewAction::Flagged,
            _ => {
                return Err(flow_like_types::anyhow!(
                    "Invalid review action: {}",
                    review.action
                ));
            }
        };

        let pending_versions = wasm_package_version::Entity::find()
            .filter(wasm_package_version::Column::PackageId.eq(package_id))
            .filter(wasm_package_version::Column::Status.eq(WasmPackageStatus::PendingReview))
            .order_by_desc(wasm_package_version::Column::PublishedAt)
            .all(&self.db)
            .await?;

        match action {
            WasmReviewAction::Approved => {
                for pv in &pending_versions {
                    wasm_package_version::ActiveModel {
                        id: Set(pv.id.clone()),
                        status: Set(WasmPackageStatus::Active),
                        approved_at: Set(Some(now)),
                        ..Default::default()
                    }
                    .update(&self.db)
                    .await?;
                }

                let mut update_model = wasm_package::ActiveModel {
                    id: Set(package_id.to_string()),
                    status: Set(WasmPackageStatus::Active),
                    published_at: Set(Some(now)),
                    updated_at: Set(now),
                    ..Default::default()
                };

                if pkg.status == WasmPackageStatus::PendingReview {
                    update_model.visibility = Set(WasmPackageVisibility::Public);
                }

                if let Some(latest) = pending_versions.first() {
                    update_model.version = Set(latest.version.clone());
                    update_model.wasm_path = Set(latest.wasm_path.clone());
                    update_model.wasm_hash = Set(latest.wasm_hash.clone());
                    update_model.wasm_size = Set(latest.wasm_size);
                    update_model.nodes = Set(latest.nodes.clone());
                    update_model.widgets = Set(latest.widgets.clone());
                    update_model.widget_bundle_hash = Set(latest.widget_bundle_hash.clone());
                    update_model.widget_bundle_size = Set(latest.widget_bundle_size);
                }

                update_model.update(&self.db).await?;
            }
            WasmReviewAction::Rejected => {
                for pv in &pending_versions {
                    wasm_package_version::ActiveModel {
                        id: Set(pv.id.clone()),
                        status: Set(WasmPackageStatus::Rejected),
                        ..Default::default()
                    }
                    .update(&self.db)
                    .await?;
                }

                let mut update_model = wasm_package::ActiveModel {
                    id: Set(package_id.to_string()),
                    updated_at: Set(now),
                    ..Default::default()
                };

                if pending_versions.is_empty() || pkg.status == WasmPackageStatus::PendingReview {
                    update_model.status = Set(WasmPackageStatus::Rejected);
                }

                update_model.update(&self.db).await?;
            }
            _ => {}
        }

        // Create review record
        let review_id = create_id();
        let review_model = wasm_package_review::ActiveModel {
            id: Set(review_id.clone()),
            package_id: Set(package_id.to_string()),
            reviewer_id: Set(reviewer_id.to_string()),
            action: Set(action),
            comment: Set(review.comment.clone()),
            internal_note: Set(review.internal_note),
            security_score: Set(review.security_score),
            code_quality_score: Set(review.code_quality_score),
            documentation_score: Set(review.documentation_score),
            created_at: Set(now),
        };
        review_model.insert(&self.db).await?;

        Ok(PackageReview {
            id: review_id,
            package_id: package_id.to_string(),
            reviewer_id: reviewer_id.to_string(),
            reviewer: self.get_user_author_info(reviewer_id).await?,
            action: review.action,
            comment: review.comment,
            security_score: review.security_score,
            code_quality_score: review.code_quality_score,
            documentation_score: review.documentation_score,
            created_at: now.to_utc(),
        })
    }

    /// Get reviews for a package
    pub async fn get_reviews(
        &self,
        package_id: &str,
    ) -> flow_like_types::Result<Vec<PackageReview>> {
        let reviews = wasm_package_review::Entity::find()
            .filter(wasm_package_review::Column::PackageId.eq(package_id))
            .order_by_desc(wasm_package_review::Column::CreatedAt)
            .all(&self.db)
            .await?;

        let mut resolved_reviews = Vec::with_capacity(reviews.len());

        for review in reviews {
            let action_str = match review.action {
                crate::entity::sea_orm_active_enums::WasmReviewAction::Submitted => "submitted",
                crate::entity::sea_orm_active_enums::WasmReviewAction::Approved => "approve",
                crate::entity::sea_orm_active_enums::WasmReviewAction::Rejected => "reject",
                crate::entity::sea_orm_active_enums::WasmReviewAction::RequestedChanges => {
                    "request_changes"
                }
                crate::entity::sea_orm_active_enums::WasmReviewAction::Commented => "comment",
                crate::entity::sea_orm_active_enums::WasmReviewAction::Flagged => "flag",
            };

            resolved_reviews.push(PackageReview {
                id: review.id,
                package_id: review.package_id,
                reviewer_id: review.reviewer_id.clone(),
                reviewer: self.get_user_author_info(&review.reviewer_id).await?,
                action: action_str.to_string(),
                comment: review.comment,
                security_score: review.security_score,
                code_quality_score: review.code_quality_score,
                documentation_score: review.documentation_score,
                created_at: review.created_at.to_utc(),
            });
        }

        Ok(resolved_reviews)
    }

    /// Update package status directly (admin)
    pub async fn update_status(
        &self,
        package_id: &str,
        status: &str,
        verified: Option<bool>,
    ) -> flow_like_types::Result<()> {
        let now = chrono::Utc::now().fixed_offset();
        let new_status = status_to_enum(status);

        let mut update_model = wasm_package::ActiveModel {
            id: Set(package_id.to_string()),
            status: Set(new_status.clone()),
            updated_at: Set(now),
            ..Default::default()
        };

        if let Some(v) = verified {
            update_model.verified = Set(v);
        }

        if new_status == crate::entity::sea_orm_active_enums::WasmPackageStatus::Active {
            update_model.published_at = Set(Some(now));
        }

        update_model.update(&self.db).await?;
        Ok(())
    }

    // ==================== AUTHOR MANAGEMENT ====================

    /// Add an author to a package
    pub async fn add_author(
        &self,
        package_id: &str,
        user_id: &str,
        role: Option<String>,
    ) -> flow_like_types::Result<AuthorInfo> {
        let now = chrono::Utc::now().fixed_offset();

        // Verify package exists
        let Some(_pkg) = wasm_package::Entity::find_by_id(package_id)
            .one(&self.db)
            .await?
        else {
            return Err(flow_like_types::anyhow!(
                "Package not found: {}",
                package_id
            ));
        };

        // Verify user exists
        let Some(user_record) = user::Entity::find_by_id(user_id).one(&self.db).await? else {
            return Err(flow_like_types::anyhow!("User not found: {}", user_id));
        };

        // Check if already an author
        let existing = wasm_package_author::Entity::find()
            .filter(wasm_package_author::Column::PackageId.eq(package_id))
            .filter(wasm_package_author::Column::UserId.eq(user_id))
            .one(&self.db)
            .await?;

        if existing.is_some() {
            return Err(flow_like_types::anyhow!(
                "User is already an author of this package"
            ));
        }

        let author_model = wasm_package_author::ActiveModel {
            id: Set(create_id()),
            package_id: Set(package_id.to_string()),
            user_id: Set(user_id.to_string()),
            role: Set(role.clone()),
            added_at: Set(now),
        };
        author_model.insert(&self.db).await?;

        Ok(AuthorInfo {
            user_id: user_id.to_string(),
            username: user_record.username,
            name: user_record.name,
            avatar: user_record.avatar,
            role,
        })
    }

    /// Remove an author from a package
    pub async fn remove_author(
        &self,
        package_id: &str,
        user_id: &str,
    ) -> flow_like_types::Result<()> {
        let result = wasm_package_author::Entity::delete_many()
            .filter(wasm_package_author::Column::PackageId.eq(package_id))
            .filter(wasm_package_author::Column::UserId.eq(user_id))
            .exec(&self.db)
            .await?;

        if result.rows_affected == 0 {
            return Err(flow_like_types::anyhow!(
                "Author not found for this package"
            ));
        }

        Ok(())
    }

    /// Get packages authored by a user
    pub async fn get_user_packages(
        &self,
        user_id: &str,
    ) -> flow_like_types::Result<Vec<PackageSummary>> {
        let author_records = wasm_package_author::Entity::find()
            .filter(wasm_package_author::Column::UserId.eq(user_id))
            .all(&self.db)
            .await?;

        let package_ids: Vec<String> = author_records.into_iter().map(|a| a.package_id).collect();

        if package_ids.is_empty() {
            return Ok(Vec::new());
        }

        let packages = wasm_package::Entity::find()
            .filter(wasm_package::Column::Id.is_in(package_ids))
            .all(&self.db)
            .await?;

        Ok(packages
            .into_iter()
            .map(|pkg| {
                let vis = visibility_to_string(&pkg.visibility);
                PackageSummary {
                    id: pkg.id,
                    name: pkg.name,
                    description: pkg.description,
                    latest_version: pkg.version,
                    download_count: pkg.download_count as u64,
                    status: status_to_package_status(&pkg.status),
                    keywords: pkg.keywords.unwrap_or_default().into(),
                    verified: pkg.verified,
                    price: pkg.price,
                    visibility: vis,
                    primary_category: pkg.primary_category.as_ref().map(db_cat_to_string),
                    secondary_category: pkg.secondary_category.as_ref().map(db_cat_to_string),
                    avg_rating: pkg.avg_rating,
                    rating_count: pkg.rating_count,
                    metadata: None,
                    capabilities: capability_tags_from_json(pkg.permissions),
                }
            })
            .collect())
    }
}

/// Derive the listing capability tags from a stored `permissions` blob.
///
/// The permissions column is written by the publish flow, so a row that predates
/// a manifest change (or carries anything unparseable) simply lists no
/// capabilities rather than failing the whole listing.
fn capability_tags_from_json(raw: serde_json::Value) -> Vec<String> {
    serde_json::from_value::<PackagePermissions>(raw)
        .map(|permissions| permissions.capability_tags())
        .unwrap_or_default()
}

#[derive(Deserialize)]
struct StoredPackageWidget {
    id: String,
    name: String,
    #[serde(default)]
    description: String,
    contract: serde_json::Value,
}

fn package_widget_refs_from_version(
    package_id: &str,
    package_version: &str,
    widget_bundle_hash: Option<&str>,
    widgets: serde_json::Value,
) -> flow_like_types::Result<Vec<PackageWidgetRef>> {
    let entries: Vec<StoredPackageWidget> = serde_json::from_value(widgets).map_err(|error| {
        flow_like_types::anyhow!(
            "Invalid widget manifest for package '{}' version '{}': {}",
            package_id,
            package_version,
            error
        )
    })?;
    let widget_bundle_hash = widget_bundle_hash
        .filter(|hash| !hash.is_empty())
        .map(str::to_owned);
    let mut widget_refs = Vec::with_capacity(entries.len());

    for entry in entries {
        widget_refs.push(PackageWidgetRef {
            package_id: package_id.to_string(),
            package_version: package_version.to_string(),
            widget_id: entry.id,
            name: entry.name,
            description: entry.description,
            bundle_hash: widget_bundle_hash.clone(),
            contract: entry.contract,
        });
    }

    Ok(widget_refs)
}

#[flow_like_types::async_trait]
impl PackageWidgetSource for ServerRegistry {
    async fn list_widgets(
        &self,
        packages: &std::collections::HashMap<String, String>,
    ) -> flow_like_types::Result<Vec<PackageWidgetRef>> {
        if packages.is_empty() {
            return Ok(Vec::new());
        }

        let mut pinned = sea_orm::Condition::any();
        for (package_id, version) in packages {
            pinned = pinned.add(
                sea_orm::Condition::all()
                    .add(wasm_package_version::Column::PackageId.eq(package_id))
                    .add(wasm_package_version::Column::Version.eq(version)),
            );
        }

        let mut versions_by_pin: std::collections::HashMap<
            (String, String),
            wasm_package_version::Model,
        > = wasm_package_version::Entity::find()
            .filter(pinned)
            .all(&self.db)
            .await?
            .into_iter()
            .map(|record| ((record.package_id.clone(), record.version.clone()), record))
            .collect();

        let mut widgets = Vec::new();
        for (package_id, version) in packages {
            let Some(record) = versions_by_pin.remove(&(package_id.clone(), version.clone()))
            else {
                tracing::warn!(
                    package_id,
                    version,
                    "Pinned package widget version not found; skipping its widgets"
                );
                continue;
            };

            match package_widget_refs_from_version(
                package_id,
                version,
                record.widget_bundle_hash.as_deref(),
                record.widgets,
            ) {
                Ok(package_widgets) => widgets.extend(package_widgets),
                Err(error) => tracing::warn!(
                    package_id,
                    version,
                    %error,
                    "Pinned package widget manifest is invalid; skipping its widgets"
                ),
            }
        }

        widgets.sort_by(|a, b| {
            a.package_id
                .cmp(&b.package_id)
                .then_with(|| a.name.cmp(&b.name))
                .then_with(|| a.widget_id.cmp(&b.widget_id))
        });

        Ok(widgets)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use flow_like_wasm_schema::widget::{ContractInput, ContractInputType, WidgetContract};
    use flow_like_wasm_schema::widget_bundle::{BuilderWidget, WidgetBundleBuilder};

    fn contract_with_input(widget_id: &str) -> WidgetContract {
        let mut contract = WidgetContract::new(widget_id);
        contract.inputs.insert(
            "title".into(),
            ContractInput {
                input_type: ContractInputType::String,
                description: None,
                default: Some(serde_json::json!("Hello")),
                choices: None,
                min: None,
                max: None,
                schema: None,
                optional: false,
            },
        );
        contract
    }

    fn build_bundle(package_id: &str, version: &str, widget_id: &str) -> (Vec<u8>, String) {
        WidgetBundleBuilder::new(package_id, version)
            .created_at("2026-07-31T00:00:00Z")
            .add_widget(BuilderWidget {
                id: widget_id.to_string(),
                name: widget_id.to_string(),
                description: "test widget".into(),
                framework: Some("vanilla".into()),
                entry_html: b"<html><body>test</body></html>".to_vec(),
                contract: contract_with_input(widget_id),
                assets: vec![],
                thumbnail: None,
            })
            .build()
            .unwrap()
    }

    fn manifest_with_widget(
        package_id: &str,
        version: &str,
        widget_id: &str,
        bundle_hash: &str,
        contract: WidgetContract,
    ) -> PackageManifest {
        let mut manifest = PackageManifest::new(package_id, "Test", version, "test package");
        manifest
            .widgets
            .push(flow_like_wasm_schema::manifest::PackageWidgetEntry {
                id: widget_id.to_string(),
                name: widget_id.to_string(),
                description: "test widget".into(),
                icon: None,
                thumbnail: None,
                contract,
                keywords: vec![],
            });
        manifest.widget_bundle_hash = Some(bundle_hash.to_string());
        manifest
    }

    #[test]
    fn package_widget_refs_preserve_pinned_version_and_contract() {
        let contract = contract_with_input("kpi-card");
        let mut contract_json = serde_json::to_value(contract).unwrap();
        contract_json
            .as_object_mut()
            .unwrap()
            .insert("futureContractField".into(), serde_json::json!(true));
        let widgets = serde_json::json!([{
            "id": "kpi-card",
            "name": "KPI Card",
            "description": "Shows one metric",
            "contract": contract_json.clone(),
            "futureManifestField": "ignored safely"
        }]);

        let refs = package_widget_refs_from_version(
            "com.example.widgets",
            "1.2.3",
            Some("bundle-hash"),
            widgets,
        )
        .unwrap();

        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].package_id, "com.example.widgets");
        assert_eq!(refs[0].package_version, "1.2.3");
        assert_eq!(refs[0].widget_id, "kpi-card");
        assert_eq!(refs[0].name, "KPI Card");
        assert_eq!(refs[0].description, "Shows one metric");
        assert_eq!(refs[0].bundle_hash.as_deref(), Some("bundle-hash"));
        assert_eq!(refs[0].contract, contract_json);
    }

    #[test]
    fn package_widget_refs_reject_invalid_manifest_json() {
        let error = package_widget_refs_from_version(
            "com.example.widgets",
            "1.2.3",
            Some(""),
            serde_json::json!({ "not": "a widget list" }),
        )
        .unwrap_err();

        let message = error.to_string();
        assert!(message.contains("com.example.widgets"));
        assert!(message.contains("1.2.3"));
    }

    #[test]
    fn test_validate_manifest_widget_bundle_ok() {
        let (bytes, hash) = build_bundle("com.example.w", "1.0.0", "kpi-card");
        let manifest = manifest_with_widget(
            "com.example.w",
            "1.0.0",
            "kpi-card",
            &hash,
            contract_with_input("kpi-card"),
        );

        let (validated_hash, size) = validate_manifest_widget_bundle(&manifest, &bytes).unwrap();
        assert_eq!(validated_hash, hash);
        assert_eq!(size, bytes.len() as i64);
    }

    #[test]
    fn test_validate_manifest_widget_bundle_hash_mismatch() {
        let (bytes, _hash) = build_bundle("com.example.w", "1.0.0", "kpi-card");
        let manifest = manifest_with_widget(
            "com.example.w",
            "1.0.0",
            "kpi-card",
            "deadbeef",
            contract_with_input("kpi-card"),
        );

        let err = validate_manifest_widget_bundle(&manifest, &bytes).unwrap_err();
        assert!(err.to_string().contains("hash mismatch"));
    }

    #[test]
    fn test_validate_manifest_widget_bundle_contract_mismatch() {
        let (bytes, hash) = build_bundle("com.example.w", "1.0.0", "kpi-card");
        // Manifest carries a structurally different contract (no inputs)
        let manifest = manifest_with_widget(
            "com.example.w",
            "1.0.0",
            "kpi-card",
            &hash,
            WidgetContract::new("kpi-card"),
        );

        let err = validate_manifest_widget_bundle(&manifest, &bytes).unwrap_err();
        assert!(err.to_string().contains("Contract mismatch"));
    }

    #[test]
    fn test_validate_manifest_widget_bundle_widget_set_mismatch() {
        let (bytes, hash) = build_bundle("com.example.w", "1.0.0", "kpi-card");
        let mut manifest = manifest_with_widget(
            "com.example.w",
            "1.0.0",
            "kpi-card",
            &hash,
            contract_with_input("kpi-card"),
        );
        manifest
            .widgets
            .push(flow_like_wasm_schema::manifest::PackageWidgetEntry {
                id: "extra-widget".into(),
                name: "Extra".into(),
                description: "not in bundle".into(),
                icon: None,
                thumbnail: None,
                contract: WidgetContract::new("extra-widget"),
                keywords: vec![],
            });

        let err = validate_manifest_widget_bundle(&manifest, &bytes).unwrap_err();
        assert!(err.to_string().contains("Widget set mismatch"));
    }

    #[test]
    fn test_validate_manifest_widget_bundle_wrong_package() {
        let (bytes, hash) = build_bundle("com.example.other", "1.0.0", "kpi-card");
        let manifest = manifest_with_widget(
            "com.example.w",
            "1.0.0",
            "kpi-card",
            &hash,
            contract_with_input("kpi-card"),
        );

        let err = validate_manifest_widget_bundle(&manifest, &bytes).unwrap_err();
        assert!(err.to_string().contains("built for package"));
    }

    #[test]
    fn test_validate_manifest_widget_bundle_tampered() {
        let (mut bytes, _hash) = build_bundle("com.example.w", "1.0.0", "kpi-card");
        let mid = bytes.len() / 2;
        bytes[mid] ^= 0xff;
        let tampered_hash = sha256_hex(&bytes);
        let manifest = manifest_with_widget(
            "com.example.w",
            "1.0.0",
            "kpi-card",
            &tampered_hash,
            contract_with_input("kpi-card"),
        );

        assert!(validate_manifest_widget_bundle(&manifest, &bytes).is_err());
    }

    #[tokio::test]
    async fn test_unpack_widget_bundle_to_assets() {
        let (bytes, _hash) = build_bundle("com.example.w", "1.0.0", "kpi-card");
        let store = FlowLikeStore::Memory(Arc::new(
            flow_like_storage::object_store::memory::InMemory::new(),
        ));

        let uploaded = unpack_widget_bundle_to_assets(&store, "com.example.w", "1.0.0", bytes)
            .await
            .unwrap();
        assert!(uploaded >= 3, "expected bundle.json + entry + contract");

        for entry in [
            "bundle.json",
            "widgets/kpi-card/index.html",
            "widgets/kpi-card/contract.json",
        ] {
            let mut path = Path::from(WIDGET_ASSETS_PATH)
                .child("com.example.w")
                .child("1.0.0");
            for segment in entry.split('/') {
                path = path.child(segment);
            }
            let object = store.as_generic().get(&path).await;
            assert!(object.is_ok(), "missing unpacked asset: {}", entry);
        }
    }

    #[test]
    fn test_manifest_has_wasm() {
        let mut manifest = PackageManifest::new("com.example.n", "Nodes", "1.0.0", "nodes only");
        assert!(manifest_has_wasm(&manifest));

        manifest
            .widgets
            .push(flow_like_wasm_schema::manifest::PackageWidgetEntry {
                id: "w".into(),
                name: "W".into(),
                description: String::new(),
                icon: None,
                thumbnail: None,
                contract: WidgetContract::new("w"),
                keywords: vec![],
            });
        assert!(!manifest_has_wasm(&manifest));

        manifest.wasm_hash = Some(String::new());
        assert!(!manifest_has_wasm(&manifest));

        manifest.wasm_hash = Some("abc".into());
        assert!(manifest_has_wasm(&manifest));
    }
}
