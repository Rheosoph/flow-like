//! `.flwb` (Flow-Like Widget Bundle) reader/writer
//!
//! A widget bundle is a single deterministic ZIP container holding every
//! micro widget of a package: one `index.html` (+ `contract.json`) per
//! widget under `widgets/{id}/`, shared content-hashed chunks under
//! `shared/`, and a `bundle.json` manifest at the root. Integrity is
//! layered: a whole-file sha256 (stored in the package manifest) plus a
//! per-entry sha256 in `bundle.json` for serving entries individually.

use crate::manifest::PackageWidgetEntry;
use crate::widget::{WidgetContract, WIDGET_PROTOCOL};
use anyhow::{anyhow, bail, Context, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashSet};
use std::io::{Cursor, Read, Seek, Write};
use std::path::Path;
use zip::{ZipArchive, ZipWriter};

/// Current bundle format version (`bundle.json` -> `formatVersion`)
pub const BUNDLE_FORMAT_VERSION: u32 = 1;
/// Manifest entry name inside the archive
pub const BUNDLE_MANIFEST_PATH: &str = "bundle.json";
/// Canonical file extension
pub const WIDGET_BUNDLE_EXTENSION: &str = "flwb";
/// Media type of the bundle artifact
pub const WIDGET_BUNDLE_MEDIA_TYPE: &str = "application/vnd.flow-like.widget-bundle";

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

fn is_safe_entry_path(path: &str) -> bool {
    !path.is_empty()
        && !path.starts_with('/')
        && !path.contains('\\')
        && !path
            .split('/')
            .any(|seg| seg.is_empty() || seg == "." || seg == "..")
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
            let mut entry = archive
                .by_name(BUNDLE_MANIFEST_PATH)
                .with_context(|| format!("Widget bundle is missing {}", BUNDLE_MANIFEST_PATH))?;
            let mut content = String::new();
            entry.read_to_string(&mut content)?;
            serde_json::from_str::<WidgetBundleManifest>(&content)
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
        let mut entry = self
            .archive
            .by_name(path)
            .with_context(|| format!("Widget bundle entry not found: {}", path))?;
        let mut data = Vec::with_capacity(entry.size() as usize);
        entry.read_to_end(&mut data)?;
        Ok(data)
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
        }
        if manifest.widgets.is_empty() {
            errors.push("Bundle contains no widgets".to_string());
        }

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
            let prefix = format!("widgets/{}/", widget.id);
            if !widget.entry.starts_with(&prefix) || !is_safe_entry_path(&widget.entry) {
                errors.push(format!(
                    "Widget '{}' entry path '{}' must live under {}",
                    widget.id, widget.entry, prefix
                ));
            }
            if !widget.contract.starts_with(&prefix) || !is_safe_entry_path(&widget.contract) {
                errors.push(format!(
                    "Widget '{}' contract path '{}' must live under {}",
                    widget.id, widget.contract, prefix
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
        for name in names {
            if name.ends_with('/') {
                continue;
            }
            if !is_safe_entry_path(&name) {
                bail!("Unsafe widget bundle entry path: {}", name);
            }
            let data = self.read_entry(&name)?;
            let target = staging.join(&name);
            if let Some(dir) = target.parent() {
                std::fs::create_dir_all(dir)?;
            }
            std::fs::write(&target, data)?;
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

            let entry_path = format!("widgets/{}/index.html", widget.id);
            let contract_path = format!("widgets/{}/contract.json", widget.id);
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
        assert!(tampered
            .read_entry("widgets/sales-chart/index.html")
            .is_err());
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
