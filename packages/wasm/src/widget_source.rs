//! Resolves an app's package widgets from installed package manifests.
//!
//! [`flow_like::a2ui::micro_widget::WidgetProvider`] asks a host-registered
//! [`PackageWidgetSource`] for the widgets of the packages an app declares. The
//! installed manifest carries the full typed contract, so a widget can be listed
//! and its pins generated without opening the widget bundle.

use crate::client::RegistryClient;
use crate::manifest::{PackageManifest, PackageWidgetEntry};
use crate::registry::InstalledPackage;
use flow_like::a2ui::micro_widget::{PackageWidgetRef, PackageWidgetSource};
use flow_like_types::async_trait;
use std::collections::HashMap;

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

#[async_trait]
impl PackageWidgetSource for RegistryClient {
    async fn list_widgets(
        &self,
        packages: &HashMap<String, String>,
    ) -> flow_like_types::Result<Vec<PackageWidgetRef>> {
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

        Ok(widgets)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::{InstalledVersion, PackageSource};
    use crate::widget::WidgetContract;
    use std::path::PathBuf;

    fn widget_entry(id: &str, name: &str) -> PackageWidgetEntry {
        PackageWidgetEntry {
            id: id.to_string(),
            name: name.to_string(),
            description: "A widget".to_string(),
            icon: None,
            thumbnail: None,
            contract: WidgetContract::new(id),
            keywords: Vec::new(),
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
}
