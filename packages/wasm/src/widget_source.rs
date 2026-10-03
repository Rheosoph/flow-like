//! Resolves an app's package widgets from installed package manifests.
//!
//! [`flow_like::a2ui::micro_widget::WidgetProvider`] asks a host-registered
//! [`PackageWidgetSource`] for the widgets of the packages an app pins. On a
//! device the pins are the `packages` map of the app manifest in its meta store;
//! a hub does not keep that map in step and answers from its database. The
//! installed manifest carries the full typed contract, so a widget can be listed
//! and its node pins generated without opening the widget bundle.

use crate::client::RegistryClient;
use crate::manifest::{PackageManifest, PackageWidgetEntry};
use crate::registry::InstalledPackage;
use flow_like::a2ui::micro_widget::{PackageWidgetRef, PackageWidgetSource};
use flow_like::app::App;
use flow_like::state::FlowLikeState;
use flow_like_types::async_trait;
use std::collections::HashMap;
use std::sync::Arc;

/// Manifest of the version an app pinned, falling back to the active install.
///
/// A pinned version that is not installed still lists its widgets from the
/// active version — the alternative is a silently empty dropdown, and the
/// reported `package_version` always names the manifest actually used.
fn resolve_manifest<'a>(
    installed: &'a InstalledPackage,
    requested_version: &str,
) -> (&'a str, &'a PackageManifest, Option<String>) {
    if let Some(version) = installed.get_version(requested_version) {
        let bundle_hash = version
            .widget_bundle_hash
            .clone()
            .or_else(|| version.manifest.widget_bundle_hash.clone());
        return (&version.version, &version.manifest, bundle_hash);
    }

    (
        &installed.version,
        &installed.manifest,
        installed.manifest.widget_bundle_hash.clone(),
    )
}

fn to_widget_ref(
    package_id: &str,
    package_version: &str,
    bundle_hash: Option<String>,
    entry: &PackageWidgetEntry,
) -> Option<PackageWidgetRef> {
    Some(PackageWidgetRef {
        package_id: package_id.to_string(),
        package_version: package_version.to_string(),
        widget_id: entry.id.clone(),
        name: entry.name.clone(),
        description: entry.description.clone(),
        bundle_hash,
        contract: serde_json::to_value(&entry.contract).ok()?,
    })
}

/// Widget entries an installed package contributes for the pinned version.
pub fn installed_package_widgets(
    installed: &InstalledPackage,
    requested_version: &str,
) -> Vec<PackageWidgetRef> {
    let (version, manifest, bundle_hash) = resolve_manifest(installed, requested_version);
    manifest
        .widgets
        .iter()
        .filter_map(|entry| to_widget_ref(&installed.id, version, bundle_hash.clone(), entry))
        .collect()
}

impl RegistryClient {
    /// Widget entries of the given pins (`package_id -> version`), from the
    /// manifests installed on this device. Only these packages are considered —
    /// never everything installed locally.
    pub async fn pinned_widgets(
        &self,
        packages: &HashMap<String, String>,
    ) -> Vec<PackageWidgetRef> {
        let mut widgets = Vec::new();

        for (package_id, version) in packages {
            let Some(installed) = self.get_installed(package_id).await else {
                continue;
            };
            widgets.extend(installed_package_widgets(&installed, version));
        }

        // Stable order so the selector dropdown does not reshuffle between loads.
        widgets.sort_by(|a, b| {
            a.package_id
                .cmp(&b.package_id)
                .then_with(|| a.name.cmp(&b.name))
                .then_with(|| a.widget_id.cmp(&b.widget_id))
        });

        widgets
    }
}

#[async_trait]
impl PackageWidgetSource for RegistryClient {
    async fn list_widgets(
        &self,
        app_id: &str,
        state: Arc<FlowLikeState>,
    ) -> flow_like_types::Result<Vec<PackageWidgetRef>> {
        // A device has no pin database: its copy of the app manifest carries them.
        let app = App::load(app_id.to_string(), state).await?;
        Ok(self.pinned_widgets(&app.packages).await)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::{InstalledVersion, PackageSource, RegistryConfig};
    use crate::widget::WidgetContract;
    use crate::widget_bundle::{BuilderWidget, WidgetBundleBuilder};
    use flow_like::bit::Metadata;
    use flow_like::state::FlowLikeConfig;
    use flow_like::utils::http::HTTPClient;
    use flow_like_storage::files::store::FlowLikeStore;
    use flow_like_storage::object_store::memory::InMemory;
    use std::path::{Path, PathBuf};

    fn widget_entry(id: &str, name: &str) -> PackageWidgetEntry {
        PackageWidgetEntry {
            id: id.to_string(),
            name: name.to_string(),
            description: "A widget".to_string(),
            icon: None,
            thumbnail: None,
            contract: WidgetContract::new(id),
            keywords: Vec::new(),
            network: None,
        }
    }

    fn manifest(version: &str, widgets: Vec<PackageWidgetEntry>, hash: &str) -> PackageManifest {
        let mut manifest = PackageManifest::new("com.example.sales", "Sales", version, "Sales");
        manifest.widgets = widgets;
        manifest.widget_bundle_hash = Some(hash.to_string());
        manifest
    }

    fn installed(active: PackageManifest, versions: Vec<InstalledVersion>) -> InstalledPackage {
        InstalledPackage {
            id: "com.example.sales".to_string(),
            version: active.version.clone(),
            source: PackageSource::Local {
                path: PathBuf::from("/tmp/sales"),
            },
            installed_at: chrono::Utc::now(),
            wasm_path: PathBuf::from("/tmp/sales.wasm"),
            manifest: active,
            versions: versions
                .into_iter()
                .map(|v| (v.version.clone(), v))
                .collect(),
            metadata: None,
            wasm_hash: None,
        }
    }

    fn installed_version(manifest: PackageManifest, bundle_hash: Option<&str>) -> InstalledVersion {
        InstalledVersion {
            version: manifest.version.clone(),
            wasm_path: PathBuf::from("/tmp/sales.wasm"),
            installed_at: chrono::Utc::now(),
            manifest,
            metadata: None,
            wasm_hash: None,
            widget_bundle_path: None,
            widget_bundle_hash: bundle_hash.map(str::to_string),
        }
    }

    #[test]
    fn lists_widgets_of_the_pinned_version() {
        let pinned = manifest("1.0.0", vec![widget_entry("kpi-card", "KPI Card")], "old");
        let active = manifest(
            "2.0.0",
            vec![widget_entry("sales-chart", "Sales Chart")],
            "new",
        );
        let package = installed(active, vec![installed_version(pinned, Some("pinned-hash"))]);

        let widgets = installed_package_widgets(&package, "1.0.0");

        assert_eq!(widgets.len(), 1);
        assert_eq!(widgets[0].widget_id, "kpi-card");
        assert_eq!(widgets[0].package_version, "1.0.0");
        assert_eq!(widgets[0].bundle_hash.as_deref(), Some("pinned-hash"));
        assert_eq!(widgets[0].selector(), "pkg:com.example.sales/kpi-card");
    }

    #[test]
    fn falls_back_to_the_active_version_when_the_pin_is_not_installed() {
        let active = manifest(
            "2.0.0",
            vec![widget_entry("sales-chart", "Sales Chart")],
            "new",
        );
        let package = installed(active, Vec::new());

        let widgets = installed_package_widgets(&package, "1.0.0");

        assert_eq!(widgets.len(), 1);
        assert_eq!(widgets[0].widget_id, "sales-chart");
        assert_eq!(widgets[0].package_version, "2.0.0");
        assert_eq!(widgets[0].bundle_hash.as_deref(), Some("new"));
    }

    #[test]
    fn packages_without_widgets_contribute_nothing() {
        let package = installed(manifest("1.0.0", Vec::new(), "none"), Vec::new());
        assert!(installed_package_widgets(&package, "1.0.0").is_empty());
    }

    #[test]
    fn contract_is_carried_over_verbatim() {
        let entry = widget_entry("kpi-card", "KPI Card");
        let expected = serde_json::to_value(&entry.contract).unwrap();
        let package = installed(manifest("1.0.0", vec![entry], "hash"), Vec::new());

        let widgets = installed_package_widgets(&package, "1.0.0");

        assert_eq!(widgets[0].contract, expected);
        assert!(widgets[0].parsed_contract().is_ok());
    }

    fn memory_state() -> Arc<FlowLikeState> {
        let store = FlowLikeStore::Memory(Arc::new(InMemory::new()));
        Arc::new(FlowLikeState::new(
            FlowLikeConfig::with_default_store(store),
            HTTPClient::new_without_refetch(),
        ))
    }

    async fn save_app(state: &Arc<FlowLikeState>, app_id: &str, pins: &[(&str, &str)]) {
        let mut app = App::new(
            Some(app_id.to_string()),
            Metadata::default(),
            Vec::new(),
            state.clone(),
        )
        .await
        .expect("test app should be created");
        app.packages = pins
            .iter()
            .map(|(package_id, version)| (package_id.to_string(), version.to_string()))
            .collect();
        app.save().await.expect("app manifest should save");
    }

    async fn device_client(root: &Path) -> RegistryClient {
        let config = RegistryConfig {
            cache_dir: root.join("cache"),
            ..Default::default()
        };
        let client = RegistryClient::new(config).unwrap();
        client.init().await.unwrap();
        client
    }

    // A local bundle is the one install that needs no registry to answer.
    async fn install_local_package(client: &RegistryClient, root: &Path, package_id: &str) {
        let (bundle, _) = WidgetBundleBuilder::new(package_id, "1.0.0")
            .add_widget(BuilderWidget {
                id: "kpi-card".to_string(),
                name: "KPI Card".to_string(),
                description: "A widget".to_string(),
                framework: None,
                entry_html: b"<html><body></body></html>".to_vec(),
                contract: WidgetContract::new("kpi-card"),
                assets: Vec::new(),
                thumbnail: None,
            })
            .build()
            .unwrap();
        let project_dir = root.join(package_id);
        std::fs::create_dir_all(&project_dir).unwrap();
        std::fs::write(project_dir.join("widgets.flwb"), bundle).unwrap();

        let mut package = PackageManifest::new(package_id, package_id, "1.0.0", "A package");
        package.widget_bundle_path = Some("widgets.flwb".to_string());
        client
            .register_local_package(&project_dir.join("node.wasm"), package)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn only_packages_pinned_in_the_app_manifest_contribute_widgets() {
        let temp = tempfile::tempdir().unwrap();
        let client = device_client(temp.path()).await;
        install_local_package(&client, temp.path(), "com.example.sales").await;
        install_local_package(&client, temp.path(), "com.example.maps").await;
        let state = memory_state();
        save_app(&state, "app-1", &[("com.example.sales", "1.0.0")]).await;

        let widgets = PackageWidgetSource::list_widgets(&client, "app-1", state)
            .await
            .unwrap();

        let selectors: Vec<String> = widgets.iter().map(PackageWidgetRef::selector).collect();
        assert_eq!(
            selectors,
            ["pkg:com.example.sales/kpi-card"],
            "com.example.maps is installed on the device but the app does not pin it"
        );
    }

    #[tokio::test]
    async fn an_app_that_pins_no_package_lists_no_widgets() {
        let temp = tempfile::tempdir().unwrap();
        let client = device_client(temp.path()).await;
        install_local_package(&client, temp.path(), "com.example.sales").await;
        let state = memory_state();
        save_app(&state, "app-1", &[]).await;

        let widgets = PackageWidgetSource::list_widgets(&client, "app-1", state)
            .await
            .unwrap();

        assert!(widgets.is_empty());
    }

    #[tokio::test]
    async fn a_missing_app_manifest_is_an_error_not_an_empty_list() {
        let temp = tempfile::tempdir().unwrap();
        let client = device_client(temp.path()).await;

        let listed = PackageWidgetSource::list_widgets(&client, "app-1", memory_state()).await;

        assert!(listed.is_err());
    }
}
