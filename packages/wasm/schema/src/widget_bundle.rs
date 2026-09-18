//! `.flwb` (Flow-Like Widget Bundle) reader/writer
//!
//! A widget bundle is a single deterministic ZIP container holding every
//! micro widget of a package: one `index.html` (+ `contract.json`) per
//! widget under `widgets/{id}/`, shared content-hashed chunks under
//! `shared/`, and a `bundle.json` manifest at the root. Integrity is
//! layered: a whole-file sha256 (stored in the package manifest) plus a
//! per-entry sha256 in `bundle.json` for serving entries individually.

use crate::manifest::PackageWidgetEntry;
use crate::widget::{WIDGET_PROTOCOL, WidgetContract, is_valid_widget_id};
use crate::widget_frame::is_valid_package_id;
use anyhow::{Context, Result, anyhow, bail};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::io::{Cursor, Read, Seek, Write};
use std::path::{Component, Path, PathBuf};
use zip::{ZipArchive, ZipWriter};

/// Current bundle format version (`bundle.json` -> `formatVersion`)
pub const BUNDLE_FORMAT_VERSION: u32 = 1;
/// Manifest entry name inside the archive
pub const BUNDLE_MANIFEST_PATH: &str = "bundle.json";
/// Canonical file extension
pub const WIDGET_BUNDLE_EXTENSION: &str = "flwb";
/// Media type of the bundle artifact
pub const WIDGET_BUNDLE_MEDIA_TYPE: &str = "application/vnd.flow-like.widget-bundle";
/// Largest uncompressed archive entry a reader accepts. The size a ZIP
/// declares for an entry only bounds preallocation, never the read.
pub const MAX_WIDGET_BUNDLE_ENTRY_BYTES: u64 = 64 * 1024 * 1024;

/// A shared content-hashed chunk referenced by one or more widgets
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct BundleSharedEntry {
    pub path: String,
    /// `sha256:<hex>` of the entry bytes
    pub hash: String,
}

/// Advisory size report for a widget entry
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct BundleSizeHint {
    pub raw: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gzip: Option<u64>,
}

/// A single widget inside the bundle
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct BundleWidgetEntry {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    /// Archive path of the widget document, e.g. `widgets/{id}/index.html`
    pub entry: String,
    /// Archive path of the widget contract, e.g. `widgets/{id}/contract.json`
    pub contract: String,
    /// `sha256:<hex>` of the entry document bytes
    pub entry_hash: String,
    /// Shared chunk paths this widget references
    #[serde(default)]
    pub assets: Vec<String>,
    /// Framework group the widget was built from (informational)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub framework: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size_hint: Option<BundleSizeHint>,
}

/// `bundle.json` — the bundle manifest
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct WidgetBundleManifest {
    pub format_version: u32,
    pub package_id: String,
    pub package_version: String,
    /// Host<->widget postMessage protocol version, e.g. `flw/1`
    pub protocol: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_at: Option<String>,
    #[serde(default)]
    pub shared: Vec<BundleSharedEntry>,
    #[serde(default)]
    pub widgets: Vec<BundleWidgetEntry>,
}

/// sha256 of raw bytes as lowercase hex
pub fn sha256_hex(data: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(data);
    hex_encode(&hasher.finalize())
}

/// Per-entry hash format used inside `bundle.json`
pub fn entry_hash(data: &[u8]) -> String {
    format!("sha256:{}", sha256_hex(data))
}

fn hex_encode(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{:02x}", b)).collect()
}

/// Content-addressed unpack location of an installed widget bundle.
/// Layout inside mirrors the bundle: `widgets/{widget_id}/…`, `shared/…`, `bundle.json`.
pub fn widget_store_dir(
    cache_dir: &Path,
    package_id: &str,
    bundle_hash: &str,
) -> std::path::PathBuf {
    cache_dir.join("widgets").join(package_id).join(bundle_hash)
}

/// Archive path of a widget's entry document
pub fn widget_entry_path(widget_id: &str) -> String {
    format!("widgets/{widget_id}/index.html")
}

/// Archive path of a widget's contract
pub fn widget_contract_path(widget_id: &str) -> String {
    format!("widgets/{widget_id}/contract.json")
}

/// Reads and validates the contract of an unpacked widget from `bundle.json`
/// and the contract path it declares, exactly as install validated it.
pub fn read_unpacked_widget_contract(store_dir: &Path, widget_id: &str) -> Result<WidgetContract> {
    if !is_valid_widget_id(widget_id) {
        bail!("Invalid widget id {:?}", widget_id);
    }
    let manifest_path = store_dir.join(BUNDLE_MANIFEST_PATH);
    let manifest_bytes = std::fs::read(&manifest_path)
        .with_context(|| format!("Failed to read {}", manifest_path.display()))?;
    let manifest: WidgetBundleManifest = serde_json::from_slice(&manifest_bytes)
        .with_context(|| format!("Failed to parse {}", manifest_path.display()))?;
    let widget = manifest
        .widgets
        .iter()
        .find(|widget| widget.id == widget_id)
        .ok_or_else(|| {
            anyhow!(
                "Widget '{}' not found in {}",
                widget_id,
                manifest_path.display()
            )
        })?;
    let expected = widget_contract_path(widget_id);
    if widget.contract != expected {
        bail!(
            "Widget '{}' declares contract path '{}' instead of '{}'",
            widget_id,
            widget.contract,
            expected
        );
    }
    let contract_path = store_dir.join(&expected);
    let bytes = std::fs::read(&contract_path)
        .with_context(|| format!("Failed to read {}", contract_path.display()))?;
    let contract: WidgetContract = serde_json::from_slice(&bytes)
        .with_context(|| format!("Failed to parse {}", contract_path.display()))?;
    if contract.id != widget_id {
        bail!(
            "Contract id '{}' does not match widget id '{}'",
            contract.id,
            widget_id
        );
    }
    contract.validate().map_err(|errors| {
        anyhow!(
            "Invalid contract for widget '{}': {}",
            widget_id,
            errors.join("; ")
        )
    })?;
    Ok(contract)
}

/// Checked as a string so a Linux hub rejects what only Windows resolves as
/// a path prefix (`C:x`, `C:/x`) or an alternate data stream (`a.html:x`).
fn is_safe_entry_path(path: &str) -> bool {
    !path.is_empty()
        && !path.starts_with('/')
        && !path.contains(['\\', ':', '\0'])
        && !path
            .split('/')
            .any(|seg| seg.is_empty() || seg == "." || seg == "..")
}

fn is_safe_archive_name(name: &str) -> bool {
    is_safe_entry_path(name.strip_suffix('/').unwrap_or(name))
}

fn staged_entry_target(staging: &Path, name: &str) -> Result<PathBuf> {
    let relative = Path::new(name);
    let target = staging.join(relative);
    if !relative
        .components()
        .all(|component| matches!(component, Component::Normal(_)))
        || !target.starts_with(staging)
    {
        bail!(
            "Widget bundle entry {} resolves outside the unpack directory {}",
            name,
            staging.display()
        );
    }
    Ok(target)
}

/// Path as case-insensitive filesystems that strip trailing dots and spaces
/// (macOS, Windows) resolve it; `None` when a segment resolves to nothing.
fn folded_archive_path(path: &str) -> Option<String> {
    path.split('/')
        .map(|segment| {
            let folded = segment.to_uppercase().to_lowercase();
            let trimmed = folded.trim_end_matches(['.', ' ']);
            (!trimmed.is_empty()).then(|| trimmed.to_string())
        })
        .collect::<Option<Vec<_>>>()
        .map(|segments| segments.join("/"))
}

/// Archive names that would overwrite each other when unpacked.
fn archive_name_collisions(names: &[String]) -> Vec<String> {
    let mut errors = Vec::new();
    let mut files: HashMap<String, &str> = HashMap::new();
    let mut directories: HashMap<String, &str> = HashMap::new();
    let entries: Vec<(&str, String)> = names
        .iter()
        .filter(|name| !name.ends_with('/'))
        .filter_map(|name| match folded_archive_path(name) {
            Some(folded) => Some((name.as_str(), folded)),
            None => {
                errors.push(format!(
                    "Widget bundle entry '{}' has a path segment made only of dots or spaces",
                    name
                ));
                None
            }
        })
        .collect();
    for (name, folded) in &entries {
        if let Some(existing) = files.insert(folded.clone(), name) {
            errors.push(format!(
                "Widget bundle entries '{}' and '{}' collide on case-insensitive filesystems",
                existing, name
            ));
        }
        let mut prefix = folded.as_str();
        while let Some((parent, _)) = prefix.rsplit_once('/') {
            directories.entry(parent.to_string()).or_insert(name);
            prefix = parent;
        }
    }
    for (name, folded) in &entries {
        if let Some(other) = directories.get(folded) {
            errors.push(format!(
                "Widget bundle entry '{}' collides with directory of '{}' on case-insensitive filesystems",
                name, other
            ));
        }
    }
    errors
}

/// Reads one archive entry of at most `limit` bytes. The declared size is
/// checked up front and caps the preallocation; the read itself stops one
/// byte past the limit, whatever the archive claims.
fn read_bounded_entry(
    entry: impl Read,
    declared_size: u64,
    limit: u64,
    path: &str,
) -> Result<Vec<u8>> {
    let too_large = || {
        anyhow!(
            "Widget bundle entry {} is larger than {} bytes",
            path,
            limit
        )
    };
    if declared_size > limit {
        return Err(too_large());
    }
    let mut data = Vec::with_capacity(usize::try_from(declared_size).unwrap_or(0));
    entry.take(limit + 1).read_to_end(&mut data)?;
    if data.len() as u64 > limit {
        return Err(too_large());
    }
    Ok(data)
}

/// Reader over a `.flwb` archive with manifest parsing and entry verification
pub struct WidgetBundleReader<R: Read + Seek> {
    archive: ZipArchive<R>,
    manifest: WidgetBundleManifest,
}

impl WidgetBundleReader<Cursor<Vec<u8>>> {
    pub fn from_bytes(bytes: Vec<u8>) -> Result<Self> {
        Self::new(Cursor::new(bytes))
    }
}

impl WidgetBundleReader<std::fs::File> {
    pub fn open(path: &Path) -> Result<Self> {
        let file = std::fs::File::open(path)
            .with_context(|| format!("Failed to open widget bundle at {}", path.display()))?;
        Self::new(file)
    }
}

impl<R: Read + Seek> WidgetBundleReader<R> {
    pub fn new(reader: R) -> Result<Self> {
        let mut archive = ZipArchive::new(reader).context("Failed to read widget bundle ZIP")?;
        let manifest = {
            let entry = archive
                .by_name(BUNDLE_MANIFEST_PATH)
                .with_context(|| format!("Widget bundle is missing {}", BUNDLE_MANIFEST_PATH))?;
            let declared_size = entry.size();
            let content = read_bounded_entry(
                entry,
                declared_size,
                MAX_WIDGET_BUNDLE_ENTRY_BYTES,
                BUNDLE_MANIFEST_PATH,
            )?;
            serde_json::from_slice::<WidgetBundleManifest>(&content)
                .context("Failed to parse bundle.json")?
        };
        Ok(Self { archive, manifest })
    }

    pub fn manifest(&self) -> &WidgetBundleManifest {
        &self.manifest
    }

    /// Read a single archive entry without hash verification
    pub fn read_entry_raw(&mut self, path: &str) -> Result<Vec<u8>> {
        if !is_safe_entry_path(path) {
            bail!("Unsafe widget bundle entry path: {}", path);
        }
        let entry = self
            .archive
            .by_name(path)
            .with_context(|| format!("Widget bundle entry not found: {}", path))?;
        let declared_size = entry.size();
        read_bounded_entry(entry, declared_size, MAX_WIDGET_BUNDLE_ENTRY_BYTES, path)
    }

    /// Read an entry and verify it against its `bundle.json` hash (if listed)
    pub fn read_entry(&mut self, path: &str) -> Result<Vec<u8>> {
        let expected = self.declared_hash(path);
        let data = self.read_entry_raw(path)?;
        if let Some(expected) = expected {
            let actual = entry_hash(&data);
            if actual != expected {
                bail!(
                    "Hash mismatch for widget bundle entry {}: expected {}, got {}",
                    path,
                    expected,
                    actual
                );
            }
        }
        Ok(data)
    }

    fn declared_hash(&self, path: &str) -> Option<String> {
        if let Some(shared) = self.manifest.shared.iter().find(|s| s.path == path) {
            return Some(shared.hash.clone());
        }
        self.manifest
            .widgets
            .iter()
            .find(|w| w.entry == path)
            .map(|w| w.entry_hash.clone())
    }

    /// Parse and validate the contract of a widget declared in the manifest
    pub fn contract(&mut self, widget_id: &str) -> Result<WidgetContract> {
        let contract_path = self
            .manifest
            .widgets
            .iter()
            .find(|w| w.id == widget_id)
            .map(|w| w.contract.clone())
            .ok_or_else(|| anyhow!("Widget '{}' not found in bundle manifest", widget_id))?;
        let data = self.read_entry_raw(&contract_path)?;
        let contract: WidgetContract = serde_json::from_slice(&data)
            .with_context(|| format!("Failed to parse contract for widget '{}'", widget_id))?;
        Ok(contract)
    }

    /// Derive package-manifest widget entries from the bundle.
    ///
    /// The bundle is authoritative for id, name, description and the typed
    /// contract; optional presentation fields (icon, thumbnail, keywords) are
    /// carried over from a manifest-declared entry with the same id. Local
    /// developer manifests (`flow-like.toml`) usually declare no widgets at
    /// all — the bundler discovers them — so this is what puts contracts into
    /// the installed manifest, where every widget consumer reads them.
    pub fn manifest_widgets(
        &mut self,
        declared: &[PackageWidgetEntry],
    ) -> Result<Vec<PackageWidgetEntry>> {
        let bundled = self.manifest.widgets.clone();
        bundled
            .iter()
            .map(|entry| {
                let contract = self.contract(&entry.id)?;
                let declared = declared.iter().find(|d| d.id == entry.id);
                let description = if entry.description.is_empty() {
                    declared.map(|d| d.description.clone()).unwrap_or_default()
                } else {
                    entry.description.clone()
                };
                Ok(PackageWidgetEntry {
                    id: entry.id.clone(),
                    name: entry.name.clone(),
                    description,
                    icon: declared.and_then(|d| d.icon.clone()),
                    thumbnail: declared.and_then(|d| d.thumbnail.clone()),
                    contract,
                    keywords: declared.map(|d| d.keywords.clone()).unwrap_or_default(),
                    network: None,
                })
            })
            .collect()
    }

    /// Validate the whole bundle: manifest consistency, entry presence,
    /// per-entry hashes, contract validity. Returns all problems found.
    pub fn validate(&mut self) -> Result<(), Vec<String>> {
        let mut errors = Vec::new();
        let manifest = self.manifest.clone();

        if manifest.format_version == 0 || manifest.format_version > BUNDLE_FORMAT_VERSION {
            errors.push(format!(
                "Unsupported bundle format version {} (supported: 1..={})",
                manifest.format_version, BUNDLE_FORMAT_VERSION
            ));
        }
        if manifest.protocol != WIDGET_PROTOCOL {
            errors.push(format!(
                "Unsupported widget protocol '{}' (expected '{}')",
                manifest.protocol, WIDGET_PROTOCOL
            ));
        }
        if manifest.package_id.is_empty() {
            errors.push("Bundle manifest is missing packageId".to_string());
        } else if !is_valid_package_id(&manifest.package_id) {
            errors.push(format!(
                "Invalid bundle packageId {:?}: use only letters, digits, '.', '_' and '-'",
                manifest.package_id
            ));
        }
        if manifest.widgets.is_empty() {
            errors.push("Bundle contains no widgets".to_string());
        }

        let names: Vec<String> = self.archive.file_names().map(str::to_string).collect();
        errors.extend(
            names
                .iter()
                .filter(|name| !is_safe_archive_name(name))
                .map(|name| format!("Unsafe widget bundle entry path: {}", name)),
        );
        errors.extend(archive_name_collisions(&names));

        let mut seen_ids = HashSet::new();
        let shared_paths: HashSet<&str> = manifest.shared.iter().map(|s| s.path.as_str()).collect();

        for shared in &manifest.shared {
            if !is_safe_entry_path(&shared.path) || !shared.path.starts_with("shared/") {
                errors.push(format!("Invalid shared chunk path: {}", shared.path));
                continue;
            }
            match self.read_entry_raw(&shared.path) {
                Ok(data) => {
                    let actual = entry_hash(&data);
                    if actual != shared.hash {
                        errors.push(format!(
                            "Hash mismatch for shared chunk {}: expected {}, got {}",
                            shared.path, shared.hash, actual
                        ));
                    }
                }
                Err(e) => errors.push(format!("{}", e)),
            }
        }

        for widget in &manifest.widgets {
            if !seen_ids.insert(widget.id.clone()) {
                errors.push(format!("Duplicate widget id in bundle: {}", widget.id));
            }
            let expected_entry = widget_entry_path(&widget.id);
            if widget.entry != expected_entry {
                errors.push(format!(
                    "Widget '{}' entry path '{}' must be '{}'",
                    widget.id, widget.entry, expected_entry
                ));
            }
            let expected_contract = widget_contract_path(&widget.id);
            if widget.contract != expected_contract {
                errors.push(format!(
                    "Widget '{}' contract path '{}' must be '{}'",
                    widget.id, widget.contract, expected_contract
                ));
            }

            match self.read_entry_raw(&widget.entry) {
                Ok(data) => {
                    let actual = entry_hash(&data);
                    if actual != widget.entry_hash {
                        errors.push(format!(
                            "Hash mismatch for widget entry {}: expected {}, got {}",
                            widget.entry, widget.entry_hash, actual
                        ));
                    }
                }
                Err(e) => errors.push(format!("{}", e)),
            }

            match self.read_entry_raw(&widget.contract) {
                Ok(data) => match serde_json::from_slice::<WidgetContract>(&data) {
                    Ok(contract) => {
                        if contract.id != widget.id {
                            errors.push(format!(
                                "Contract id '{}' does not match widget id '{}'",
                                contract.id, widget.id
                            ));
                        }
                        if let Err(contract_errors) = contract.validate() {
                            errors.extend(contract_errors);
                        }
                    }
                    Err(e) => errors.push(format!(
                        "Failed to parse contract for widget '{}': {}",
                        widget.id, e
                    )),
                },
                Err(e) => errors.push(format!("{}", e)),
            }

            for asset in &widget.assets {
                if !shared_paths.contains(asset.as_str()) {
                    errors.push(format!(
                        "Widget '{}' references undeclared asset: {}",
                        widget.id, asset
                    ));
                }
            }
        }

        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors)
        }
    }

    /// Unpack the full archive into `dest_dir` atomically: entries are
    /// verified and written to a temporary sibling directory, which is then
    /// renamed into place (no torn state on failure).
    pub fn unpack(&mut self, dest_dir: &Path) -> Result<()> {
        self.validate()
            .map_err(|errors| anyhow!("Widget bundle validation failed: {}", errors.join("; ")))?;

        let parent = dest_dir
            .parent()
            .ok_or_else(|| anyhow!("Unpack destination has no parent: {}", dest_dir.display()))?;
        std::fs::create_dir_all(parent)?;

        let staging = parent.join(format!(
            ".{}.partial-{}",
            dest_dir
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("widget-bundle"),
            std::process::id()
        ));
        if staging.exists() {
            std::fs::remove_dir_all(&staging)?;
        }
        std::fs::create_dir_all(&staging)?;

        let result = self.unpack_into(&staging);
        if let Err(e) = result {
            let _ = std::fs::remove_dir_all(&staging);
            return Err(e);
        }

        if dest_dir.exists() {
            std::fs::remove_dir_all(dest_dir)?;
        }
        std::fs::rename(&staging, dest_dir).with_context(|| {
            format!(
                "Failed to move unpacked widget bundle into place at {}",
                dest_dir.display()
            )
        })?;
        Ok(())
    }

    fn unpack_into(&mut self, staging: &Path) -> Result<()> {
        let names: Vec<String> = self.archive.file_names().map(|n| n.to_string()).collect();
        let declared: HashSet<String> = std::iter::once(BUNDLE_MANIFEST_PATH.to_string())
            .chain(
                self.manifest
                    .shared
                    .iter()
                    .map(|shared| shared.path.clone()),
            )
            .chain(
                self.manifest
                    .widgets
                    .iter()
                    .flat_map(|widget| [widget.entry.clone(), widget.contract.clone()]),
            )
            .collect();
        let mut written = Vec::new();
        for name in names {
            if name.ends_with('/') {
                continue;
            }
            if !is_safe_entry_path(&name) {
                bail!("Unsafe widget bundle entry path: {}", name);
            }
            let target = staged_entry_target(staging, &name)?;
            let data = self.read_entry(&name)?;
            if let Some(dir) = target.parent() {
                std::fs::create_dir_all(dir)?;
            }
            if declared.contains(&name) {
                written.push((target.clone(), entry_hash(&data)));
            }
            std::fs::write(&target, data)?;
        }
        for (target, expected) in written {
            let actual = entry_hash(&std::fs::read(&target)?);
            if actual != expected {
                bail!(
                    "Widget bundle entry {} was overwritten during unpack by an entry with an aliasing name",
                    target.display()
                );
            }
        }
        Ok(())
    }
}

/// A widget being added to a [`WidgetBundleBuilder`]
pub struct BuilderWidget {
    pub id: String,
    pub name: String,
    pub description: String,
    pub framework: Option<String>,
    /// Fully built, self-contained (or chunk-referencing) widget document
    pub entry_html: Vec<u8>,
    pub contract: WidgetContract,
    /// Shared chunk paths (`shared/…`) this widget references
    pub assets: Vec<String>,
    /// Optional preview image stored as `widgets/{id}/thumbnail.webp`
    pub thumbnail: Option<Vec<u8>>,
}

/// Deterministic `.flwb` writer. The canonical producer is the
/// `@flow-like/widget-bundler` CLI; this builder mirrors its output for
/// tests and server-side tooling.
pub struct WidgetBundleBuilder {
    package_id: String,
    package_version: String,
    created_at: Option<String>,
    shared: BTreeMap<String, Vec<u8>>,
    widgets: Vec<BuilderWidget>,
}

impl WidgetBundleBuilder {
    pub fn new(package_id: &str, package_version: &str) -> Self {
        Self {
            package_id: package_id.to_string(),
            package_version: package_version.to_string(),
            created_at: None,
            shared: BTreeMap::new(),
            widgets: Vec::new(),
        }
    }

    /// Timestamp stamped into `bundle.json`; keep fixed for deterministic builds
    pub fn created_at(mut self, timestamp: &str) -> Self {
        self.created_at = Some(timestamp.to_string());
        self
    }

    /// Add a shared chunk; `filename` lands under `shared/`
    pub fn add_shared_chunk(mut self, filename: &str, data: Vec<u8>) -> Self {
        self.shared.insert(format!("shared/{}", filename), data);
        self
    }

    pub fn add_widget(mut self, widget: BuilderWidget) -> Self {
        self.widgets.push(widget);
        self
    }

    /// Produce the archive bytes and the whole-file sha256 hex (the
    /// `widget_bundle_hash` stored in the package manifest)
    pub fn build(mut self) -> Result<(Vec<u8>, String)> {
        if self.widgets.is_empty() {
            bail!("Widget bundle must contain at least one widget");
        }
        self.widgets.sort_by(|a, b| a.id.cmp(&b.id));

        let mut entries: BTreeMap<String, Vec<u8>> = BTreeMap::new();
        let mut manifest = WidgetBundleManifest {
            format_version: BUNDLE_FORMAT_VERSION,
            package_id: self.package_id.clone(),
            package_version: self.package_version.clone(),
            protocol: WIDGET_PROTOCOL.to_string(),
            created_at: self.created_at.clone(),
            shared: Vec::new(),
            widgets: Vec::new(),
        };

        for (path, data) in &self.shared {
            manifest.shared.push(BundleSharedEntry {
                path: path.clone(),
                hash: entry_hash(data),
            });
            entries.insert(path.clone(), data.clone());
        }

        let mut seen = HashSet::new();
        for widget in &self.widgets {
            if !seen.insert(widget.id.clone()) {
                bail!("Duplicate widget id in bundle: {}", widget.id);
            }
            if widget.contract.id != widget.id {
                bail!(
                    "Contract id '{}' does not match widget id '{}'",
                    widget.contract.id,
                    widget.id
                );
            }
            widget.contract.validate().map_err(|e| {
                anyhow!(
                    "Invalid contract for widget '{}': {}",
                    widget.id,
                    e.join("; ")
                )
            })?;
            for asset in &widget.assets {
                if !self.shared.contains_key(asset) {
                    bail!(
                        "Widget '{}' references missing shared chunk: {}",
                        widget.id,
                        asset
                    );
                }
            }

            let entry_path = widget_entry_path(&widget.id);
            let contract_path = widget_contract_path(&widget.id);
            let contract_json = serde_json::to_vec_pretty(&widget.contract)?;

            manifest.widgets.push(BundleWidgetEntry {
                id: widget.id.clone(),
                name: widget.name.clone(),
                description: widget.description.clone(),
                entry: entry_path.clone(),
                contract: contract_path.clone(),
                entry_hash: entry_hash(&widget.entry_html),
                assets: widget.assets.clone(),
                framework: widget.framework.clone(),
                size_hint: Some(BundleSizeHint {
                    raw: widget.entry_html.len() as u64,
                    gzip: None,
                }),
            });

            entries.insert(entry_path, widget.entry_html.clone());
            entries.insert(contract_path, contract_json);
            if let Some(thumbnail) = &widget.thumbnail {
                entries.insert(
                    format!("widgets/{}/thumbnail.webp", widget.id),
                    thumbnail.clone(),
                );
            }
        }

        entries.insert(
            BUNDLE_MANIFEST_PATH.to_string(),
            serde_json::to_vec_pretty(&manifest)?,
        );

        let mut cursor = Cursor::new(Vec::new());
        {
            let mut writer = ZipWriter::new(&mut cursor);
            let options = zip::write::FileOptions::<zip::write::ExtendedFileOptions>::default()
                .compression_method(zip::CompressionMethod::Deflated)
                .last_modified_time(zip::DateTime::default());
            for (path, data) in &entries {
                writer.start_file(path, options.clone())?;
                writer.write_all(data)?;
            }
            writer.finish()?;
        }

        let bytes = cursor.into_inner();
        let hash = sha256_hex(&bytes);
        Ok((bytes, hash))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::widget::{ContractInput, ContractInputType};
    use serde_json::json;

    fn sample_widget(id: &str) -> BuilderWidget {
        let mut contract = WidgetContract::new(id);
        contract.inputs.insert(
            "title".into(),
            ContractInput {
                input_type: ContractInputType::String,
                description: None,
                default: Some(json!("Hello")),
                choices: None,
                min: None,
                max: None,
                schema: None,
                optional: false,
            },
        );
        BuilderWidget {
            id: id.to_string(),
            name: id.to_string(),
            description: "test widget".into(),
            framework: Some("react".into()),
            entry_html: format!("<html><body>{}</body></html>", id).into_bytes(),
            contract,
            assets: vec!["shared/react-abc123.js".into()],
            thumbnail: None,
        }
    }

    fn sample_bundle() -> (Vec<u8>, String) {
        WidgetBundleBuilder::new("com.example.sales", "1.2.0")
            .created_at("2026-07-21T12:00:00Z")
            .add_shared_chunk("react-abc123.js", b"console.log('react runtime')".to_vec())
            .add_widget(sample_widget("sales-chart"))
            .add_widget(sample_widget("kpi-card"))
            .build()
            .unwrap()
    }

    fn with_claimed_size(mut bytes: Vec<u8>, name: &str, size: u32) -> Vec<u8> {
        let signature = 0x0201_4b50u32.to_le_bytes();
        let mut offset = 0;
        while let Some(found) = bytes[offset..].windows(4).position(|w| w == signature) {
            let header = offset + found;
            let name_len = u16::from_le_bytes([bytes[header + 28], bytes[header + 29]]) as usize;
            if &bytes[header + 46..header + 46 + name_len] == name.as_bytes() {
                bytes[header + 24..header + 28].copy_from_slice(&size.to_le_bytes());
                return bytes;
            }
            offset = header + 4;
        }
        panic!("{name} has no central directory entry");
    }

    #[test]
    fn bounded_reads_stop_at_the_limit_whatever_the_archive_claims() {
        let bytes = [7u8; 33];
        assert_eq!(
            read_bounded_entry(&bytes[..32], 32, 32, "a").unwrap().len(),
            32
        );
        for (data, claimed) in [(&bytes[..], 0), (&bytes[..1], u64::MAX)] {
            let error = read_bounded_entry(data, claimed, 32, "a").unwrap_err();
            assert_eq!(
                error.to_string(),
                "Widget bundle entry a is larger than 32 bytes"
            );
        }
    }

    #[test]
    fn claimed_entry_sizes_never_drive_allocation() {
        let entry = "widgets/kpi-card/index.html";
        let (bytes, _) = sample_bundle();
        let mut reader =
            WidgetBundleReader::from_bytes(with_claimed_size(bytes, entry, 0xFFFF_FFF0)).unwrap();
        let error = reader.read_entry_raw(entry).unwrap_err();
        assert!(error.to_string().contains("is larger than"), "{error}");
        let errors = reader.validate().unwrap_err();
        assert!(
            errors
                .iter()
                .any(|error| error.contains(&format!("{entry} is larger than"))),
            "{errors:?}"
        );

        let (bytes, _) = sample_bundle();
        let manifest = with_claimed_size(bytes, BUNDLE_MANIFEST_PATH, 0xFFFF_FFF0);
        let error = WidgetBundleReader::from_bytes(manifest)
            .err()
            .expect("an oversized manifest is refused");
        assert!(
            error.to_string().contains("bundle.json is larger than"),
            "{error}"
        );
    }

    #[test]
    fn test_build_is_deterministic() {
        let (bytes_a, hash_a) = sample_bundle();
        let (bytes_b, hash_b) = sample_bundle();
        assert_eq!(bytes_a, bytes_b);
        assert_eq!(hash_a, hash_b);
        assert_eq!(hash_a, sha256_hex(&bytes_a));
    }

    #[test]
    fn test_read_and_validate_roundtrip() {
        let (bytes, _) = sample_bundle();
        let mut reader = WidgetBundleReader::from_bytes(bytes).unwrap();

        assert_eq!(reader.manifest().package_id, "com.example.sales");
        assert_eq!(reader.manifest().protocol, WIDGET_PROTOCOL);
        assert_eq!(reader.manifest().widgets.len(), 2);
        assert!(reader.validate().is_ok());

        let contract = reader.contract("sales-chart").unwrap();
        assert_eq!(contract.id, "sales-chart");
        assert!(contract.inputs.contains_key("title"));

        let entry = reader.read_entry("widgets/kpi-card/index.html").unwrap();
        assert!(String::from_utf8(entry).unwrap().contains("kpi-card"));
    }

    #[test]
    fn test_tampered_entry_fails_validation() {
        let (bytes, _) = sample_bundle();
        let mut reader = WidgetBundleReader::from_bytes(bytes.clone()).unwrap();
        let original = reader.read_entry("widgets/sales-chart/index.html").unwrap();

        // Rebuild the ZIP with a modified entry but the original bundle.json
        let manifest_bytes = reader.read_entry_raw(BUNDLE_MANIFEST_PATH).unwrap();
        let mut cursor = Cursor::new(Vec::new());
        {
            let mut writer = ZipWriter::new(&mut cursor);
            let options = zip::write::FileOptions::<zip::write::ExtendedFileOptions>::default()
                .compression_method(zip::CompressionMethod::Deflated);
            let mut archive = ZipArchive::new(Cursor::new(bytes)).unwrap();
            let names: Vec<String> = archive.file_names().map(|n| n.to_string()).collect();
            for name in names {
                let data = if name == "widgets/sales-chart/index.html" {
                    b"<html>tampered</html>".to_vec()
                } else if name == BUNDLE_MANIFEST_PATH {
                    manifest_bytes.clone()
                } else {
                    let mut entry = archive.by_name(&name).unwrap();
                    let mut data = Vec::new();
                    entry.read_to_end(&mut data).unwrap();
                    data
                };
                writer.start_file(&name, options.clone()).unwrap();
                writer.write_all(&data).unwrap();
            }
            writer.finish().unwrap();
        }

        let mut tampered = WidgetBundleReader::from_bytes(cursor.into_inner()).unwrap();
        let errors = tampered.validate().unwrap_err();
        assert!(errors.iter().any(|e| e.contains("Hash mismatch")));
        assert!(
            tampered
                .read_entry("widgets/sales-chart/index.html")
                .is_err()
        );
        assert_ne!(original, b"<html>tampered</html>");
    }

    #[test]
    fn test_unpack_atomic() {
        let temp = tempfile::tempdir().unwrap();
        let dest = temp
            .path()
            .join("store")
            .join("com.example.sales")
            .join("abc123");

        let (bytes, _) = sample_bundle();
        let mut reader = WidgetBundleReader::from_bytes(bytes).unwrap();
        reader.unpack(&dest).unwrap();

        assert!(dest.join("bundle.json").exists());
        assert!(dest.join("widgets/sales-chart/index.html").exists());
        assert!(dest.join("widgets/sales-chart/contract.json").exists());
        assert!(dest.join("shared/react-abc123.js").exists());

        let leftovers: Vec<_> = std::fs::read_dir(dest.parent().unwrap())
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().contains("partial"))
            .collect();
        assert!(leftovers.is_empty());
    }

    #[test]
    fn test_missing_widget_rejected() {
        let result = WidgetBundleBuilder::new("com.example.empty", "1.0.0").build();
        assert!(result.is_err());
    }

    fn archive_entries(bytes: Vec<u8>) -> Vec<(String, Vec<u8>)> {
        let mut archive = ZipArchive::new(Cursor::new(bytes)).unwrap();
        let names: Vec<String> = archive.file_names().map(str::to_string).collect();
        names
            .into_iter()
            .map(|name| {
                let mut data = Vec::new();
                archive
                    .by_name(&name)
                    .unwrap()
                    .read_to_end(&mut data)
                    .unwrap();
                (name, data)
            })
            .collect()
    }

    fn write_archive(entries: &[(String, Vec<u8>)]) -> Vec<u8> {
        let mut cursor = Cursor::new(Vec::new());
        {
            let mut writer = ZipWriter::new(&mut cursor);
            let options = zip::write::FileOptions::<zip::write::ExtendedFileOptions>::default()
                .compression_method(zip::CompressionMethod::Deflated);
            for (name, data) in entries {
                writer.start_file(name, options.clone()).unwrap();
                writer.write_all(data).unwrap();
            }
            writer.finish().unwrap();
        }
        cursor.into_inner()
    }

    fn with_extra_entry(name: &str, data: &[u8]) -> Vec<u8> {
        let (bytes, _) = sample_bundle();
        let mut entries = archive_entries(bytes);
        entries.push((name.to_string(), data.to_vec()));
        write_archive(&entries)
    }

    fn with_manifest(edit: impl FnOnce(&mut WidgetBundleManifest)) -> Vec<u8> {
        let (bytes, _) = sample_bundle();
        let mut entries = archive_entries(bytes);
        let (_, manifest_bytes) = entries
            .iter_mut()
            .find(|(name, _)| name == BUNDLE_MANIFEST_PATH)
            .unwrap();
        let mut manifest: WidgetBundleManifest = serde_json::from_slice(manifest_bytes).unwrap();
        edit(&mut manifest);
        *manifest_bytes = serde_json::to_vec_pretty(&manifest).unwrap();
        write_archive(&entries)
    }

    #[test]
    fn archive_names_colliding_after_folding_are_rejected() {
        for alias in [
            "widgets/sales-chart/CONTRACT.json",
            "Widgets/sales-chart/contract.json",
            "widgets/sales-chart/contract.json.",
            "widgets/sales-chart/contract.json ",
            "widgets/sales-chart. /index.html",
            "BUNDLE.JSON",
            "bundle.json...",
            "widgets/sales-chart/contract.j\u{17f}on",
            "shared/REACT-ABC123.js",
        ] {
            let mut reader =
                WidgetBundleReader::from_bytes(with_extra_entry(alias, b"{}")).unwrap();
            let errors = reader.validate().unwrap_err();
            assert!(
                errors.iter().any(|error| error.contains("collide")),
                "{alias:?} must collide: {errors:?}"
            );
            let temp = tempfile::tempdir().unwrap();
            assert!(reader.unpack(&temp.path().join("store")).is_err());
        }

        let (bytes, _) = sample_bundle();
        let mut entries = archive_entries(bytes);
        entries.push(("widgets/sales-chart/kpi.svg".into(), b"<svg/>".to_vec()));
        entries.push((
            "widgets/sales-chart/\u{212a}pi.svg".into(),
            b"<svg/>".to_vec(),
        ));
        let mut kelvin = WidgetBundleReader::from_bytes(write_archive(&entries)).unwrap();
        assert!(
            kelvin
                .validate()
                .unwrap_err()
                .iter()
                .any(|error| error.contains("collide"))
        );

        let mut file_over_directory =
            WidgetBundleReader::from_bytes(with_extra_entry("WIDGETS/Sales-Chart", b"x")).unwrap();
        assert!(
            file_over_directory
                .validate()
                .unwrap_err()
                .iter()
                .any(|error| error.contains("collides with directory"))
        );

        let mut dots_only =
            WidgetBundleReader::from_bytes(with_extra_entry("widgets/.../x.svg", b"x")).unwrap();
        assert!(
            dots_only
                .validate()
                .unwrap_err()
                .iter()
                .any(|error| error.contains("only of dots or spaces"))
        );

        let mut distinct = WidgetBundleReader::from_bytes(with_extra_entry(
            "widgets/sales-chart/logo.svg",
            b"<svg/>",
        ))
        .unwrap();
        assert!(distinct.validate().is_ok());
    }

    #[test]
    fn drive_prefixed_stream_and_nul_entry_names_are_rejected_on_every_os() {
        for name in [
            "C:/x.txt",
            "C:x.txt",
            "c:/ProgramData/Microsoft/Windows/Start Menu/Programs/StartUp/x.bat",
            "widgets/sales-chart/index.html:ads",
            "widgets/sales-chart/index.html::$DATA",
            "shared/react\0.js",
        ] {
            let mut reader = WidgetBundleReader::from_bytes(with_extra_entry(name, b"x")).unwrap();
            let expected = format!("Unsafe widget bundle entry path: {}", name);
            let errors = reader.validate().unwrap_err();
            assert!(errors.contains(&expected), "{name:?}: {errors:?}");

            let temp = tempfile::tempdir().unwrap();
            let dest = temp.path().join("store");
            let error = reader.unpack(&dest).unwrap_err().to_string();
            assert!(error.contains(&expected), "{name:?}: {error}");
            assert!(!dest.exists());

            let staging = temp.path().join("staging");
            std::fs::create_dir_all(&staging).unwrap();
            let error = reader.unpack_into(&staging).unwrap_err().to_string();
            assert!(error.contains(&expected), "{name:?}: {error}");
            assert_eq!(
                std::fs::read_dir(temp.path()).unwrap().count(),
                1,
                "{name:?} must not write next to the staging directory"
            );
        }

        let mut unsafe_directory =
            WidgetBundleReader::from_bytes(with_extra_entry("C:/", b"")).unwrap();
        assert!(
            unsafe_directory
                .validate()
                .unwrap_err()
                .contains(&"Unsafe widget bundle entry path: C:/".to_string())
        );
    }

    #[test]
    fn directory_entries_with_safe_names_stay_valid() {
        let (bytes, _) = sample_bundle();
        let entries = archive_entries(bytes);
        let mut cursor = Cursor::new(Vec::new());
        {
            let mut writer = ZipWriter::new(&mut cursor);
            let options = zip::write::FileOptions::<zip::write::ExtendedFileOptions>::default()
                .compression_method(zip::CompressionMethod::Deflated);
            for directory in ["shared/", "widgets/", "widgets/sales-chart/"] {
                writer.add_directory(directory, options.clone()).unwrap();
            }
            for (name, data) in &entries {
                writer.start_file(name, options.clone()).unwrap();
                writer.write_all(data).unwrap();
            }
            writer.finish().unwrap();
        }
        let mut reader = WidgetBundleReader::from_bytes(cursor.into_inner()).unwrap();
        assert!(reader.validate().is_ok());
        let temp = tempfile::tempdir().unwrap();
        let dest = temp.path().join("store");
        reader.unpack(&dest).unwrap();
        assert!(dest.join("widgets/sales-chart/index.html").exists());
    }

    #[test]
    fn staged_entry_targets_stay_inside_the_staging_directory() {
        let temp = tempfile::tempdir().unwrap();
        let staging = temp.path().join("staging");
        assert_eq!(
            staged_entry_target(&staging, "widgets/sales-chart/index.html").unwrap(),
            staging
                .join("widgets")
                .join("sales-chart")
                .join("index.html")
        );
        let mut escaping = vec!["../x.txt", "widgets/../../x.txt", "/x.txt", "./x.txt"];
        if cfg!(windows) {
            escaping.extend([
                "C:x.txt",
                "C:/x.txt",
                "C:\\x.txt",
                "\\\\server\\share\\x.txt",
            ]);
        }
        for name in escaping {
            let error = staged_entry_target(&staging, name).unwrap_err().to_string();
            assert!(
                error.contains("resolves outside the unpack directory"),
                "{name:?}: {error}"
            );
        }
    }

    #[test]
    fn unpack_detects_declared_entries_overwritten_by_aliases() {
        let (bytes, _) = sample_bundle();
        let mut entries = archive_entries(bytes);
        entries.push((
            "widgets/sales-chart/CONTRACT.json".to_string(),
            br#"{"contractVersion":1,"id":"sales-chart","capabilities":{"workers":true}}"#.to_vec(),
        ));
        let mut reader = WidgetBundleReader::from_bytes(write_archive(&entries)).unwrap();

        let temp = tempfile::tempdir().unwrap();
        let probe = temp.path().join("probe");
        std::fs::write(&probe, b"x").unwrap();
        let case_insensitive = temp.path().join("PROBE").exists();

        let staging = temp.path().join("staging");
        std::fs::create_dir_all(&staging).unwrap();
        let result = reader.unpack_into(&staging);
        if case_insensitive {
            assert!(
                result
                    .unwrap_err()
                    .to_string()
                    .contains("overwritten during unpack")
            );
        } else {
            assert!(result.is_ok());
        }
    }

    #[test]
    fn entry_and_contract_paths_must_be_exact() {
        let bytes = with_manifest(|manifest| {
            manifest.widgets[0].contract = "widgets/kpi-card/../kpi-card/contract.json".into();
            manifest.widgets[1].entry = "widgets/sales-chart/main.html".into();
        });
        let mut reader = WidgetBundleReader::from_bytes(bytes).unwrap();
        let errors = reader.validate().unwrap_err();
        assert!(errors.iter().any(|error| error.contains(
            "contract path 'widgets/kpi-card/../kpi-card/contract.json' must be 'widgets/kpi-card/contract.json'"
        )));
        assert!(errors.iter().any(|error| error.contains(
            "entry path 'widgets/sales-chart/main.html' must be 'widgets/sales-chart/index.html'"
        )));
    }

    #[test]
    fn bundle_package_id_must_be_valid() {
        for package_id in ["com example", "com;example", "com\"example", ".."] {
            let (bytes, _) = WidgetBundleBuilder::new(package_id, "1.0.0")
                .add_shared_chunk("react-abc123.js", b"console.log('react runtime')".to_vec())
                .add_widget(sample_widget("sales-chart"))
                .build()
                .unwrap();
            let mut reader = WidgetBundleReader::from_bytes(bytes).unwrap();
            assert!(
                reader
                    .validate()
                    .unwrap_err()
                    .iter()
                    .any(|error| error.contains("Invalid bundle packageId")),
                "{package_id:?}"
            );
        }
    }

    #[test]
    fn read_unpacked_widget_contract_follows_the_declared_contract() {
        let mut widget = sample_widget("live-map");
        widget.contract = widget
            .contract
            .with_csp(vec![crate::widget_policy::WidgetCspPurpose {
                reason: "Loads vector map tiles".into(),
                connect_src: vec!["https://api.maptiler.com".into()],
                ..Default::default()
            }]);
        let (bytes, _) = WidgetBundleBuilder::new("com.example.maps", "1.0.0")
            .add_shared_chunk("react-abc123.js", b"console.log('react runtime')".to_vec())
            .add_widget(widget)
            .add_widget(sample_widget("kpi-card"))
            .build()
            .unwrap();
        let temp = tempfile::tempdir().unwrap();
        let store = temp.path().join("store");
        WidgetBundleReader::from_bytes(bytes)
            .unwrap()
            .unpack(&store)
            .unwrap();

        let contract = read_unpacked_widget_contract(&store, "live-map").unwrap();
        assert_eq!(contract.contract_version, crate::widget::CONTRACT_VERSION);
        assert_eq!(
            contract.declared_csp().connect_src,
            vec!["https://api.maptiler.com"]
        );
        assert!(read_unpacked_widget_contract(&store, "kpi-card").is_ok());

        for widget_id in ["", "../live-map", "Live-Map", "missing"] {
            assert!(
                read_unpacked_widget_contract(&store, widget_id).is_err(),
                "{widget_id:?}"
            );
        }

        let manifest_path = store.join(BUNDLE_MANIFEST_PATH);
        let original = std::fs::read(&manifest_path).unwrap();
        let mut manifest: WidgetBundleManifest = serde_json::from_slice(&original).unwrap();
        let live_map = manifest
            .widgets
            .iter_mut()
            .find(|widget| widget.id == "live-map")
            .unwrap();
        live_map.contract = "widgets/kpi-card/contract.json".into();
        std::fs::write(&manifest_path, serde_json::to_vec(&manifest).unwrap()).unwrap();
        assert!(
            read_unpacked_widget_contract(&store, "live-map")
                .unwrap_err()
                .to_string()
                .contains("declares contract path")
        );
        std::fs::write(&manifest_path, &original).unwrap();

        let contract_path = store.join("widgets/live-map/contract.json");
        std::fs::write(
            &contract_path,
            br#"{"contractVersion":1,"id":"live-map","csp":[{"reason":"Loads vector map tiles","connectSrc":["https://api.maptiler.com"]}]}"#,
        )
        .unwrap();
        assert!(
            read_unpacked_widget_contract(&store, "live-map")
                .unwrap_err()
                .to_string()
                .contains("contractVersion")
        );
        std::fs::write(&contract_path, br#"{"contractVersion":1,"id":"kpi-card"}"#).unwrap();
        assert!(
            read_unpacked_widget_contract(&store, "live-map")
                .unwrap_err()
                .to_string()
                .contains("does not match")
        );
    }

    /// Cross-language interop: validates a bundle produced by
    /// `@flow-like/widget-bundler`. Run with:
    /// `FLWB_INTEROP_PATH=/path/to/widgets.flwb cargo test -p flow-like-wasm --lib test_validate_external_bundle -- --ignored`
    #[test]
    #[ignore = "requires FLWB_INTEROP_PATH pointing to a bundler-produced .flwb"]
    fn test_validate_external_bundle() {
        let path = std::env::var("FLWB_INTEROP_PATH").expect("FLWB_INTEROP_PATH not set");
        let mut reader = WidgetBundleReader::open(std::path::Path::new(&path)).unwrap();
        reader.validate().map_err(|e| e.join("\n")).unwrap();

        let manifest = reader.manifest().clone();
        assert!(!manifest.widgets.is_empty());
        for widget in &manifest.widgets {
            let contract = reader.contract(&widget.id).unwrap();
            contract.validate().map_err(|e| e.join("\n")).unwrap();
            assert_eq!(contract.id, widget.id);
        }

        let temp = tempfile::tempdir().unwrap();
        let dest = temp.path().join("unpacked");
        reader.unpack(&dest).unwrap();
        assert!(dest.join(BUNDLE_MANIFEST_PATH).exists());
    }
}
