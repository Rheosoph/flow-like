//! Consent grants for package widgets served over `flow-widget://`.
//!
//! JavaScript receives only an opaque 64-hex id. The approved effective policy
//! (declared plus accepted runtime sources), the declared policy it extends and
//! their binding to one unpacked bundle widget stay in this process, expire,
//! and are resolved again by the protocol handler on every wrapper and entry
//! request. Desktop URLs never carry a runtime component.

use std::{
    collections::HashMap,
    path::Path,
    sync::{LazyLock, Mutex, MutexGuard, PoisonError},
    time::{Duration, Instant},
};

use flow_like_types::rand::{TryRngCore, rngs::OsRng};
use flow_like_wasm::widget::is_valid_widget_id;
use flow_like_wasm::widget_bundle::{
    BUNDLE_MANIFEST_PATH, read_unpacked_widget_contract, widget_store_dir,
};
use flow_like_wasm::widget_frame::{
    is_desktop_grant_id, is_valid_bundle_hash, is_valid_package_id,
};
use flow_like_wasm::widget_policy::{
    Engine, EngineGate, EngineId, WidgetPolicy, WidgetPolicyDescriptor, WidgetPolicySubject,
    WidgetRuntimeContext, WidgetRuntimeSourceRequest, narrow_policy_for_engine,
    validate_runtime_request_shape, widget_engine_gate,
};
use flow_like_wasm::widget_sources::{describe_network, validate_wildcard_bases};
use serde::{Deserialize, Serialize};

pub(crate) const WIDGET_GRANT_TTL: Duration = Duration::from_secs(24 * 60 * 60);
pub(crate) const MAX_WIDGET_GRANTS: usize = 4096;
pub(crate) const POLICY_CHANGED_ERROR: &str = "policy_changed";
pub(crate) const INVALID_RUNTIME_SOURCES_ERROR: &str = "invalid_runtime_sources";
pub(crate) const RUNTIME_SOURCES_IN_PREVIEW_ERROR: &str = "runtime_sources_in_preview";

pub(crate) static WIDGET_GRANTS: LazyLock<WidgetGrantRegistry> =
    LazyLock::new(|| WidgetGrantRegistry::new(WIDGET_GRANT_TTL, MAX_WIDGET_GRANTS));

/// Engine gate of this build's webview, evaluated once per process.
pub(crate) static WIDGET_ENGINE_GATE: LazyLock<EngineGate> = LazyLock::new(|| {
    let version = tauri::webview_version()
        .inspect_err(|error| tracing::warn!(%error, "Failed to read the webview version"))
        .ok();
    let engine = webview_engine_id(std::env::consts::OS, version.as_deref());
    let gate = widget_engine_gate(&engine);
    tracing::debug!(?engine, ?gate, "Widget engine gate evaluated");
    gate
});

/// Webview engine of a target OS: WebView2 (Chromium) on Windows, the system
/// WebView (Chromium) on Android, WebKit on macOS, iOS and Linux. The version
/// is the webview's own `major.minor`; an unparsable version keeps the engine
/// so unbounded gate rows still apply.
pub(crate) fn webview_engine_id(target_os: &str, webview_version: Option<&str>) -> EngineId {
    let engine = match target_os {
        "windows" | "android" => Engine::Chromium,
        "macos" | "ios" | "linux" => Engine::WebKit,
        _ => return EngineId::unknown(),
    };
    EngineId {
        engine,
        platform: Some(target_os.to_string()),
        version: webview_version.and_then(parse_webview_version),
    }
}

fn parse_webview_version(version: &str) -> Option<(u32, u32)> {
    let leading_digits = |text: &str| text.bytes().take_while(u8::is_ascii_digit).count();
    let version = version.trim();
    let major_len = leading_digits(version);
    if !(1..=9).contains(&major_len) {
        return None;
    }
    let major = version[..major_len].parse().ok()?;
    let rest = &version[major_len..];
    if rest.is_empty() {
        return Some((major, 0));
    }
    let minor = rest.strip_prefix('.')?;
    let minor_len = leading_digits(minor);
    if !(1..=9).contains(&minor_len) {
        return None;
    }
    Some((major, minor[..minor_len].parse().ok()?))
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct WidgetGrantBinding {
    pub package_id: String,
    pub bundle_hash: String,
    pub widget_id: String,
    pub preview: bool,
}

/// What a grant approved: the effective policy and the declared policy it
/// extends, so an engine gate can tell runtime from declared sources.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct GrantedWidgetPolicy {
    pub effective: WidgetPolicy,
    pub declared: WidgetPolicy,
}

impl GrantedWidgetPolicy {
    /// The policy a document is served with on an engine.
    pub(crate) fn served(&self, gate: &EngineGate) -> WidgetPolicy {
        narrow_policy_for_engine(&self.effective, &self.declared, gate)
    }
}

#[derive(Debug)]
struct WidgetGrant {
    binding: WidgetGrantBinding,
    granted: GrantedWidgetPolicy,
    expires_at: Instant,
    sequence: u64,
}

#[derive(Debug, Default)]
struct WidgetGrantTable {
    grants: HashMap<String, WidgetGrant>,
    next_sequence: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MintedWidgetGrant {
    pub id: String,
    pub expires_in: Duration,
}

#[derive(Debug)]
pub(crate) struct WidgetGrantRegistry {
    table: Mutex<WidgetGrantTable>,
    ttl: Duration,
    capacity: usize,
}

impl WidgetGrantRegistry {
    pub(crate) fn new(ttl: Duration, capacity: usize) -> Self {
        Self {
            table: Mutex::new(WidgetGrantTable::default()),
            ttl,
            capacity,
        }
    }

    pub(crate) fn ttl(&self) -> Duration {
        self.ttl
    }

    fn table(&self) -> MutexGuard<'_, WidgetGrantTable> {
        self.table.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Stores `granted` under an id. A live grant with the same binding and
    /// policies is reused and its expiry refreshed; otherwise expired grants
    /// are purged, the oldest are evicted at capacity and a fresh id is drawn.
    pub(crate) fn mint(
        &self,
        binding: WidgetGrantBinding,
        granted: GrantedWidgetPolicy,
    ) -> anyhow::Result<MintedWidgetGrant> {
        let now = Instant::now();
        let expires_at = now
            .checked_add(self.ttl)
            .ok_or_else(|| anyhow::anyhow!("Widget grant expiry overflowed"))?;
        let mut table = self.table();
        table.grants.retain(|_, grant| grant.expires_at > now);
        let sequence = table.next_sequence;
        table.next_sequence += 1;
        let minted = |id: String| MintedWidgetGrant {
            id,
            expires_in: expires_at.saturating_duration_since(now),
        };

        if let Some((id, grant)) = table
            .grants
            .iter_mut()
            .find(|(_, grant)| grant.binding == binding && grant.granted == granted)
        {
            grant.expires_at = expires_at;
            grant.sequence = sequence;
            return Ok(minted(id.clone()));
        }

        while table.grants.len() >= self.capacity {
            let Some(oldest) = table
                .grants
                .iter()
                .min_by_key(|(_, grant)| grant.sequence)
                .map(|(id, _)| id.clone())
            else {
                break;
            };
            table.grants.remove(&oldest);
        }
        let grant_id = loop {
            let candidate = new_grant_id()?;
            if !table.grants.contains_key(&candidate) {
                break candidate;
            }
        };
        table.grants.insert(
            grant_id.clone(),
            WidgetGrant {
                binding,
                granted,
                expires_at,
                sequence,
            },
        );
        Ok(minted(grant_id))
    }

    /// Policies of a live grant whose binding is exactly (package, bundle, widget).
    pub(crate) fn resolve(
        &self,
        grant_id: &str,
        package_id: &str,
        bundle_hash: &str,
        widget_id: &str,
    ) -> Option<GrantedWidgetPolicy> {
        if !is_desktop_grant_id(grant_id) {
            return None;
        }
        let now = Instant::now();
        let mut table = self.table();
        if table.grants.get(grant_id)?.expires_at <= now {
            table.grants.remove(grant_id);
            return None;
        }
        let grant = table.grants.get(grant_id)?;
        let binding = &grant.binding;
        (binding.package_id == package_id
            && binding.bundle_hash == bundle_hash
            && binding.widget_id == widget_id)
            .then(|| grant.granted.clone())
    }

    /// Drops every grant of `package_id`, or only those of one widget.
    pub(crate) fn revoke(&self, package_id: &str, widget_id: Option<&str>) -> usize {
        let mut table = self.table();
        let before = table.grants.len();
        table.grants.retain(|_, grant| {
            grant.binding.package_id != package_id
                || widget_id.is_some_and(|widget_id| grant.binding.widget_id != widget_id)
        });
        before - table.grants.len()
    }
}

fn new_grant_id() -> anyhow::Result<String> {
    let mut bytes = [0u8; 32];
    OsRng
        .try_fill_bytes(&mut bytes)
        .map_err(|error| anyhow::anyhow!("Failed to generate a widget grant id: {error}"))?;
    Ok(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
}

/// Response of `registry_mint_widget_grant`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WidgetGrantMint {
    pub grant: Option<String>,
    pub expires_in: u64,
    pub policy_digest: String,
}

/// Shape checks of a describe or mint request. Failures are client bugs.
pub(crate) fn validate_widget_runtime_request(
    preview: bool,
    request: &[WidgetRuntimeSourceRequest],
) -> Result<(), String> {
    validate_runtime_request_shape(request)
        .map_err(|error| format!("{INVALID_RUNTIME_SOURCES_ERROR}: {error}"))?;
    if preview && !request.is_empty() {
        return Err(format!(
            "{RUNTIME_SOURCES_IN_PREVIEW_ERROR}: previews never get runtime sources, but {} slots were requested",
            request.len()
        ));
    }
    Ok(())
}

/// Derives the authoritative descriptor from the unpacked widget store,
/// including the runtime sources `request` asks for. A malformed request or a
/// missing bundle is an error; a bundle whose contract cannot be read, parsed
/// or validated, or whose declared wildcards span public suffixes, yields an
/// `invalid` descriptor with the baseline policy. `network_at` (unix seconds)
/// adds the display-only `network` classification.
pub(crate) fn describe_unpacked_widget(
    cache_dir: &Path,
    subject: WidgetPolicySubject,
    request: &[WidgetRuntimeSourceRequest],
    context: &WidgetRuntimeContext<'_>,
    network_at: Option<i64>,
) -> Result<WidgetPolicyDescriptor, String> {
    if !is_valid_package_id(&subject.package_id) {
        return Err(format!("Invalid package id {:?}", subject.package_id));
    }
    if !is_valid_bundle_hash(&subject.bundle_hash) {
        return Err(format!(
            "Invalid widget bundle hash {:?} for package '{}'",
            subject.bundle_hash, subject.package_id
        ));
    }
    if !is_valid_widget_id(&subject.widget_id) {
        return Err(format!(
            "Invalid widget id {:?} for package '{}'",
            subject.widget_id, subject.package_id
        ));
    }
    validate_widget_runtime_request(subject.preview, request)?;
    let store_dir = widget_store_dir(cache_dir, &subject.package_id, &subject.bundle_hash);
    if !store_dir.join(BUNDLE_MANIFEST_PATH).is_file() {
        return Err(format!(
            "Widget bundle {} of package '{}' is not installed",
            subject.bundle_hash, subject.package_id
        ));
    }
    let contract = match read_unpacked_widget_contract(&store_dir, &subject.widget_id) {
        Ok(contract) => contract,
        Err(error) => {
            return Ok(WidgetPolicyDescriptor::invalid(
                subject,
                format!("{error:#}"),
            ));
        }
    };
    let fallback = subject.clone();
    let mut descriptor =
        WidgetPolicyDescriptor::describe_with_runtime(subject, &contract, request, context);
    if descriptor.is_ok() && !descriptor.preview {
        let declared = contract.declared_csp();
        if let Err(errors) = validate_wildcard_bases(declared.entries().map(|(_, source)| source)) {
            return Ok(WidgetPolicyDescriptor::invalid(fallback, errors.join("; ")));
        }
    }
    if let Some(now) = network_at {
        let (purposes, runtime) = descriptor.classifier_inputs(&contract);
        descriptor.network = describe_network(&purposes, &runtime, now);
    }
    Ok(descriptor)
}

/// The declared policy under a descriptor's effective policy: the effective
/// policy without the accepted runtime entries, checked against the declared
/// digest.
fn declared_policy(descriptor: &WidgetPolicyDescriptor) -> Result<WidgetPolicy, String> {
    let mut declared = descriptor.policy.clone();
    for entry in descriptor
        .runtime
        .iter()
        .flat_map(|runtime| &runtime.entries)
    {
        declared
            .csp
            .sources_mut(entry.directive)
            .retain(|source| source != &entry.source);
    }
    let digest = declared.digest();
    if digest != descriptor.declared_digest() {
        return Err(format!(
            "Widget '{}' of package '{}': declared policy {digest} does not match declared digest {}",
            descriptor.widget_id,
            descriptor.package_id,
            descriptor.declared_digest()
        ));
    }
    Ok(declared)
}

/// Mints a grant for a freshly derived descriptor. The caller's digest must
/// match the re-derived one; an empty policy needs no grant.
pub(crate) fn mint_widget_grant(
    registry: &WidgetGrantRegistry,
    descriptor: &WidgetPolicyDescriptor,
    policy_digest: &str,
) -> Result<WidgetGrantMint, String> {
    if descriptor.policy_digest != policy_digest {
        return Err(format!(
            "{POLICY_CHANGED_ERROR}: widget '{}' of package '{}' now has policy {} instead of {}",
            descriptor.widget_id, descriptor.package_id, descriptor.policy_digest, policy_digest
        ));
    }
    if descriptor.policy.is_empty() {
        return Ok(WidgetGrantMint {
            grant: None,
            expires_in: registry.ttl().as_secs(),
            policy_digest: descriptor.policy_digest.clone(),
        });
    }
    let binding = WidgetGrantBinding {
        package_id: descriptor.package_id.clone(),
        bundle_hash: descriptor.bundle_hash.clone(),
        widget_id: descriptor.widget_id.clone(),
        preview: descriptor.preview,
    };
    let granted = GrantedWidgetPolicy {
        effective: descriptor.policy.clone(),
        declared: declared_policy(descriptor)?,
    };
    let minted = registry
        .mint(binding, granted)
        .map_err(|error| error.to_string())?;
    Ok(WidgetGrantMint {
        grant: Some(minted.id),
        expires_in: minted.expires_in.as_secs(),
        policy_digest: descriptor.policy_digest.clone(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::widget_protocol::bundle_source;
    use flow_like_wasm::widget::{ContractInput, ContractInputType, WidgetCapabilities};
    use flow_like_wasm::widget_policy::{
        CspDirective, PlatformStorageScope, WidgetCsp, WidgetCspPurpose, WidgetNetworkInput,
        WidgetNetworkInputSlot, WidgetPolicyStatus, WidgetRuntimeStatus,
    };
    use flow_like_wasm::widget_sources::SourceProvenance;
    use flow_like_wasm::{
        BuilderWidget, WidgetBundleBuilder, WidgetBundleManifest, WidgetBundleReader,
        WidgetContract,
    };
    use std::collections::BTreeSet;
    use std::path::PathBuf;

    const PACKAGE: &str = "com.example.maps";
    const APP_ID: &str = "app-1";
    const NOW: i64 = 1_789_603_200;
    const STORAGE_ORIGIN: &str = "https://flow-like-content.s3.eu-central-1.amazonaws.com";
    const RUNTIME_TILES: &str = "https://a.tiles.customer-maps.com";
    const MAPTILER: &str = "https://api.maptiler.com";
    const OSM_TILES: &str = "https://a.tile.openstreetmap.org";

    fn binding(package_id: &str, bundle_hash: &str, widget_id: &str) -> WidgetGrantBinding {
        WidgetGrantBinding {
            package_id: package_id.into(),
            bundle_hash: bundle_hash.into(),
            widget_id: widget_id.into(),
            preview: false,
        }
    }

    fn hash(fill: char) -> String {
        fill.to_string().repeat(64)
    }

    fn network_policy() -> WidgetPolicy {
        WidgetPolicy {
            workers: true,
            downloads: true,
            csp: WidgetCsp {
                connect_src: vec![MAPTILER.into()],
                ..WidgetCsp::default()
            },
            ..WidgetPolicy::default()
        }
    }

    fn with_runtime_tiles(policy: &WidgetPolicy) -> WidgetPolicy {
        let mut effective = policy.clone();
        effective.csp.connect_src.push(RUNTIME_TILES.into());
        effective.csp.canonicalize();
        effective
    }

    fn granted(policy: WidgetPolicy) -> GrantedWidgetPolicy {
        GrantedWidgetPolicy {
            effective: policy.clone(),
            declared: policy,
        }
    }

    fn mint_id(
        registry: &WidgetGrantRegistry,
        binding: WidgetGrantBinding,
        policy: WidgetPolicy,
    ) -> String {
        registry.mint(binding, granted(policy)).unwrap().id
    }

    #[test]
    fn widget_grant_ids_are_random_64_hex() {
        let registry = WidgetGrantRegistry::new(WIDGET_GRANT_TTL, 8);
        let first = mint_id(
            &registry,
            binding(PACKAGE, &hash('a'), "live-map"),
            network_policy(),
        );
        let second = mint_id(
            &registry,
            binding(PACKAGE, &hash('a'), "other-map"),
            network_policy(),
        );
        assert!(is_desktop_grant_id(&first) && is_desktop_grant_id(&second));
        assert_ne!(first, second);
    }

    #[test]
    fn widget_grant_resolves_only_its_exact_binding() {
        let registry = WidgetGrantRegistry::new(WIDGET_GRANT_TTL, 8);
        let grant = mint_id(
            &registry,
            binding(PACKAGE, &hash('a'), "live-map"),
            network_policy(),
        );

        assert_eq!(
            registry.resolve(&grant, PACKAGE, &hash('a'), "live-map"),
            Some(granted(network_policy()))
        );
        assert_eq!(
            registry.resolve(&grant, "com.example.other", &hash('a'), "live-map"),
            None
        );
        assert_eq!(
            registry.resolve(&grant, PACKAGE, &hash('b'), "live-map"),
            None
        );
        assert_eq!(
            registry.resolve(&grant, PACKAGE, &hash('a'), "other-map"),
            None
        );
        assert_eq!(
            registry.resolve(&hash('f'), PACKAGE, &hash('a'), "live-map"),
            None
        );
        assert_eq!(registry.resolve("0", PACKAGE, &hash('a'), "live-map"), None);
        assert_eq!(
            registry.resolve(&grant.to_uppercase(), PACKAGE, &hash('a'), "live-map"),
            None
        );
        assert_eq!(
            registry.resolve(&format!("{grant}~e30"), PACKAGE, &hash('a'), "live-map"),
            None
        );
        assert_eq!(
            registry.resolve(&grant, PACKAGE, &hash('a'), "live-map"),
            Some(granted(network_policy())),
            "a mismatched lookup must not consume the grant"
        );
    }

    #[test]
    fn widget_grant_expires() {
        let registry = WidgetGrantRegistry::new(Duration::ZERO, 8);
        let grant = mint_id(
            &registry,
            binding(PACKAGE, &hash('a'), "live-map"),
            network_policy(),
        );
        assert_eq!(
            registry.resolve(&grant, PACKAGE, &hash('a'), "live-map"),
            None
        );
        assert!(registry.table().grants.is_empty());

        let registry = WidgetGrantRegistry::new(WIDGET_GRANT_TTL, 8);
        assert_eq!(registry.ttl().as_secs(), 24 * 60 * 60);
    }

    #[test]
    fn widget_grant_cap_purges_expired_then_evicts_oldest() {
        let registry = WidgetGrantRegistry::new(WIDGET_GRANT_TTL, 3);
        let grants: Vec<String> = ["a", "b", "c", "d"]
            .iter()
            .map(|widget_id| {
                mint_id(
                    &registry,
                    binding(PACKAGE, &hash('a'), widget_id),
                    network_policy(),
                )
            })
            .collect();
        assert_eq!(registry.table().grants.len(), 3);
        assert_eq!(registry.resolve(&grants[0], PACKAGE, &hash('a'), "a"), None);
        for (grant, widget_id) in grants[1..].iter().zip(["b", "c", "d"]) {
            assert!(
                registry
                    .resolve(grant, PACKAGE, &hash('a'), widget_id)
                    .is_some()
            );
        }

        let refreshed = mint_id(
            &registry,
            binding(PACKAGE, &hash('a'), "b"),
            network_policy(),
        );
        assert_eq!(refreshed, grants[1]);
        mint_id(
            &registry,
            binding(PACKAGE, &hash('a'), "e"),
            network_policy(),
        );
        assert_eq!(registry.resolve(&grants[2], PACKAGE, &hash('a'), "c"), None);
        assert!(
            registry
                .resolve(&grants[1], PACKAGE, &hash('a'), "b")
                .is_some(),
            "a refreshed grant counts as recently used"
        );

        let expiring = WidgetGrantRegistry::new(Duration::ZERO, 2);
        for widget_id in ["a", "b", "c"] {
            mint_id(
                &expiring,
                binding(PACKAGE, &hash('a'), widget_id),
                network_policy(),
            );
            assert_eq!(expiring.table().grants.len(), 1);
        }
        assert_eq!(WIDGET_GRANTS.capacity, MAX_WIDGET_GRANTS);
    }

    #[test]
    fn widget_grant_dedup_reuses_a_live_grant_and_refreshes_its_expiry() {
        let registry = WidgetGrantRegistry::new(WIDGET_GRANT_TTL, 8);
        let live_map = || binding(PACKAGE, &hash('a'), "live-map");
        let first = registry
            .mint(live_map(), granted(network_policy()))
            .unwrap();
        assert_eq!(first.expires_in, WIDGET_GRANT_TTL);

        let near_expiry = Instant::now() + Duration::from_secs(2);
        registry
            .table()
            .grants
            .get_mut(&first.id)
            .unwrap()
            .expires_at = near_expiry;
        let again = registry
            .mint(live_map(), granted(network_policy()))
            .unwrap();
        assert_eq!(again.id, first.id);
        assert_eq!(again.expires_in, WIDGET_GRANT_TTL);
        let expires_at = registry.table().grants[&first.id].expires_at;
        assert!(expires_at > near_expiry + WIDGET_GRANT_TTL / 2);
        assert_eq!(registry.table().grants.len(), 1);

        let runtime = GrantedWidgetPolicy {
            effective: with_runtime_tiles(&network_policy()),
            declared: network_policy(),
        };
        let runtime_grant = registry.mint(live_map(), runtime.clone()).unwrap();
        assert_ne!(runtime_grant.id, first.id);
        let declared_as_runtime = GrantedWidgetPolicy {
            effective: runtime.effective.clone(),
            declared: runtime.effective.clone(),
        };
        assert_ne!(
            registry.mint(live_map(), declared_as_runtime).unwrap().id,
            runtime_grant.id,
            "the declared policy is part of the dedup key"
        );
        let mut preview = live_map();
        preview.preview = true;
        assert_ne!(
            registry
                .mint(preview, granted(network_policy()))
                .unwrap()
                .id,
            first.id
        );

        registry
            .table()
            .grants
            .get_mut(&first.id)
            .unwrap()
            .expires_at = Instant::now();
        let fresh = registry
            .mint(live_map(), granted(network_policy()))
            .unwrap();
        assert_ne!(fresh.id, first.id, "an expired grant is never revived");
        assert_eq!(
            registry.resolve(&first.id, PACKAGE, &hash('a'), "live-map"),
            None
        );
    }

    #[test]
    fn widget_grant_revoke_by_package_or_widget() {
        let registry = WidgetGrantRegistry::new(WIDGET_GRANT_TTL, 16);
        let map = mint_id(
            &registry,
            binding(PACKAGE, &hash('a'), "live-map"),
            network_policy(),
        );
        let chart = mint_id(
            &registry,
            binding(PACKAGE, &hash('b'), "chart"),
            network_policy(),
        );
        let other = mint_id(
            &registry,
            binding("com.example.other", &hash('a'), "live-map"),
            network_policy(),
        );

        assert_eq!(registry.revoke(PACKAGE, Some("live-map")), 1);
        assert_eq!(
            registry.resolve(&map, PACKAGE, &hash('a'), "live-map"),
            None
        );
        assert!(
            registry
                .resolve(&chart, PACKAGE, &hash('b'), "chart")
                .is_some()
        );

        assert_eq!(registry.revoke(PACKAGE, None), 1);
        assert_eq!(registry.resolve(&chart, PACKAGE, &hash('b'), "chart"), None);
        assert!(
            registry
                .resolve(&other, "com.example.other", &hash('a'), "live-map")
                .is_some()
        );
    }

    #[test]
    fn widget_engine_id_maps_the_platform_webview() {
        let id = |engine, platform: &str, version| EngineId {
            engine,
            platform: Some(platform.to_string()),
            version,
        };
        assert_eq!(
            webview_engine_id("windows", Some("130.0.2849.56")),
            id(Engine::Chromium, "windows", Some((130, 0)))
        );
        assert_eq!(
            webview_engine_id("android", Some("131.0.6778.39")),
            id(Engine::Chromium, "android", Some((131, 0)))
        );
        assert_eq!(
            webview_engine_id("macos", Some("20621.1.15.11.10")),
            id(Engine::WebKit, "macos", Some((20621, 1)))
        );
        assert_eq!(
            webview_engine_id("ios", Some("20621.1.15")),
            id(Engine::WebKit, "ios", Some((20621, 1)))
        );
        assert_eq!(
            webview_engine_id("linux", Some(" 2.46.1\n")),
            id(Engine::WebKit, "linux", Some((2, 46)))
        );
        assert_eq!(
            webview_engine_id("linux", Some("2")),
            id(Engine::WebKit, "linux", Some((2, 0)))
        );
        for garbage in [
            "",
            "webkit",
            "2.",
            "2.x",
            ".46",
            "v2.46",
            "1234567890.1",
            "2.1234567890",
        ] {
            assert_eq!(
                webview_engine_id("linux", Some(garbage)),
                id(Engine::WebKit, "linux", None),
                "{garbage:?}"
            );
        }
        assert_eq!(
            webview_engine_id("windows", None),
            id(Engine::Chromium, "windows", None)
        );
        assert_eq!(
            webview_engine_id("freebsd", Some("2.46.1")),
            EngineId::unknown()
        );
    }

    struct TempCache(PathBuf);
    impl TempCache {
        fn new(name: &str) -> Self {
            let dir = std::env::temp_dir().join(format!(
                "flow-widget-grants-test-{}-{}",
                std::process::id(),
                name
            ));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).expect("create temp cache");
            Self(dir)
        }
    }
    impl Drop for TempCache {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn purpose(reason: &str) -> WidgetCspPurpose {
        WidgetCspPurpose {
            reason: reason.into(),
            ..WidgetCspPurpose::default()
        }
    }

    fn map_contract() -> WidgetContract {
        let mut contract = WidgetContract::new("live-map");
        contract.inputs.insert(
            "tileUrl".into(),
            ContractInput {
                input_type: ContractInputType::String,
                description: None,
                default: None,
                choices: None,
                min: None,
                max: None,
                schema: None,
                optional: true,
            },
        );
        let mut contract = contract.with_csp(vec![
            WidgetCspPurpose {
                connect_src: vec![MAPTILER.into()],
                img_src: vec![OSM_TILES.into()],
                ..purpose("Loads map styles and tiles from MapTiler and OpenStreetMap")
            },
            WidgetCspPurpose {
                inputs: vec![WidgetNetworkInput {
                    path: "tileUrl".into(),
                    directives: vec![CspDirective::ConnectSrc, CspDirective::ImgSrc],
                    template: None,
                }],
                ..purpose("Loads map tiles from tile servers given to it at runtime")
            },
        ]);
        contract.capabilities = Some(WidgetCapabilities {
            workers: Some(true),
            media: Some(true),
            ..WidgetCapabilities::default()
        });
        contract
    }

    fn install_bundle(cache_dir: &Path, contract: WidgetContract) -> String {
        let (bytes, bundle_hash) = WidgetBundleBuilder::new(PACKAGE, "1.0.0")
            .created_at("2026-09-17T00:00:00Z")
            .add_widget(BuilderWidget {
                id: contract.id.clone(),
                name: contract.id.clone(),
                description: String::new(),
                framework: None,
                entry_html: b"<!doctype html><p>map</p>".to_vec(),
                contract,
                assets: Vec::new(),
                thumbnail: None,
            })
            .build()
            .expect("build bundle");
        WidgetBundleReader::from_bytes(bytes)
            .expect("open bundle")
            .unpack(&widget_store_dir(cache_dir, PACKAGE, &bundle_hash))
            .expect("unpack bundle");
        bundle_hash
    }

    fn subject(bundle_hash: &str, widget_id: &str, preview: bool) -> WidgetPolicySubject {
        WidgetPolicySubject {
            source: "local".into(),
            package_id: PACKAGE.into(),
            package_version: None,
            bundle_hash: bundle_hash.into(),
            widget_id: widget_id.into(),
            preview,
        }
    }

    fn storage() -> Vec<PlatformStorageScope> {
        vec![PlatformStorageScope {
            origin: STORAGE_ORIGIN.into(),
            path_prefix: "/apps/".into(),
        }]
    }

    fn storage_source(app_id: &str) -> String {
        format!("{STORAGE_ORIGIN}/apps/{app_id}/")
    }

    fn runtime(slot: &str, sources: &[&str]) -> WidgetRuntimeSourceRequest {
        WidgetRuntimeSourceRequest {
            slot: slot.into(),
            sources: sources.iter().map(|source| source.to_string()).collect(),
        }
    }

    struct Viewer<'a> {
        reserved: &'a [String],
        app_id: Option<&'a str>,
        engine: EngineGate,
        network_at: Option<i64>,
    }

    impl Default for Viewer<'_> {
        fn default() -> Self {
            Self {
                reserved: &[],
                app_id: None,
                engine: EngineGate::OPEN,
                network_at: Some(NOW),
            }
        }
    }

    impl Viewer<'_> {
        fn describe(
            &self,
            cache_dir: &Path,
            subject: WidgetPolicySubject,
            request: &[WidgetRuntimeSourceRequest],
        ) -> Result<WidgetPolicyDescriptor, String> {
            let storage = storage();
            let bundle = [bundle_source(&subject.package_id, &subject.bundle_hash)];
            let context = WidgetRuntimeContext {
                reserved_hosts: self.reserved,
                platform_storage: &storage,
                app_id: self.app_id,
                bundle_sources: &bundle,
                engine: self.engine,
            };
            describe_unpacked_widget(cache_dir, subject, request, &context, self.network_at)
        }
    }

    fn describe(cache_dir: &Path, subject: WidgetPolicySubject) -> WidgetPolicyDescriptor {
        Viewer::default().describe(cache_dir, subject, &[]).unwrap()
    }

    fn rejections(descriptor: &WidgetPolicyDescriptor) -> BTreeSet<(String, String, String)> {
        descriptor
            .runtime
            .as_ref()
            .expect("runtime block")
            .rejected
            .iter()
            .map(|rejection| {
                (
                    rejection.slot.clone(),
                    rejection.source.clone(),
                    rejection.code.clone(),
                )
            })
            .collect()
    }

    fn rejection(slot: &str, source: &str, code: &str) -> (String, String, String) {
        (slot.into(), source.into(), code.into())
    }

    #[test]
    fn widget_policy_describe_reads_the_unpacked_contract() {
        let cache = TempCache::new("describe");
        let bundle_hash = install_bundle(&cache.0, map_contract());

        let descriptor = describe(&cache.0, subject(&bundle_hash, "live-map", false));
        assert_eq!(descriptor.status, WidgetPolicyStatus::Ok);
        assert!(descriptor.policy.workers && descriptor.policy.media);
        assert_eq!(
            descriptor.policy.csp.connect_src,
            vec![MAPTILER.to_string()]
        );
        assert_eq!(descriptor.policy.csp.img_src, vec![OSM_TILES.to_string()]);
        assert_eq!(descriptor.policy_digest, descriptor.policy.digest());
        assert_eq!(
            descriptor.network_inputs,
            vec![WidgetNetworkInputSlot {
                path: "tileUrl".into(),
                purpose: 1,
                directives: vec![CspDirective::ConnectSrc, CspDirective::ImgSrc],
                template: None,
            }]
        );
        assert_eq!(descriptor.platform_storage, Some(storage()));
        let runtime = descriptor.runtime.as_ref().unwrap();
        assert_eq!(runtime.status, WidgetRuntimeStatus::None);
        assert_eq!(runtime.declared_digest, descriptor.policy_digest);
        assert_eq!(descriptor.engine, Some(EngineGate::OPEN.support()));
        let network = descriptor.network.as_ref().expect("describe classifies");
        assert_eq!(network.purposes.len(), 2);
        assert!(
            network
                .purposes
                .iter()
                .flat_map(|purpose| &purpose.sources)
                .all(|source| source.origin == SourceProvenance::Declared)
        );
        let json = serde_json::to_value(&descriptor).unwrap();
        assert_eq!(json["runtime"]["status"], "none");
        assert_eq!(json["networkInputs"][0]["path"], "tileUrl");
        assert_eq!(json["platformStorage"][0]["pathPrefix"], "/apps/");
        assert_eq!(json["engine"]["runtimeSources"], true);

        let preview = describe(&cache.0, subject(&bundle_hash, "live-map", true));
        assert!(preview.is_ok() && preview.preview);
        assert!(preview.policy.csp.is_empty() && !preview.policy.media);
        assert!(preview.policy.workers);
        assert!(preview.network_inputs.is_empty());
        assert!(preview.runtime.is_none() && preview.engine.is_none());
        assert!(preview.platform_storage.is_none() && preview.network.is_none());

        let reserved = ["https://tile.openstreetmap.org".to_string()];
        let reserved = Viewer {
            reserved: &reserved,
            ..Viewer::default()
        }
        .describe(&cache.0, subject(&bundle_hash, "live-map", false), &[])
        .unwrap();
        assert_eq!(reserved.status, WidgetPolicyStatus::Invalid);
        assert!(reserved.invalid_reason.unwrap().contains("reserved host"));
        assert!(reserved.policy.is_empty());
        assert_eq!(reserved.policy_digest, WidgetPolicy::default().digest());
        assert!(reserved.network.is_none() && reserved.runtime.is_none());

        let unclassified = Viewer {
            network_at: None,
            ..Viewer::default()
        }
        .describe(&cache.0, subject(&bundle_hash, "live-map", false), &[])
        .unwrap();
        assert!(unclassified.network.is_none());
        assert_eq!(unclassified.policy_digest, descriptor.policy_digest);
    }

    #[test]
    fn widget_policy_describe_follows_the_declared_contract_path() {
        let cache = TempCache::new("declared");
        let bundle_hash = install_bundle(&cache.0, map_contract());
        let store_dir = widget_store_dir(&cache.0, PACKAGE, &bundle_hash);

        let stray_dir = store_dir.join("widgets").join("live-map").join("alt");
        std::fs::create_dir_all(&stray_dir).unwrap();
        std::fs::write(
            stray_dir.join("contract.json"),
            serde_json::to_vec(&map_contract()).unwrap(),
        )
        .unwrap();
        let manifest_path = store_dir.join(BUNDLE_MANIFEST_PATH);
        let mut manifest: WidgetBundleManifest =
            serde_json::from_slice(&std::fs::read(&manifest_path).unwrap()).unwrap();
        manifest.widgets[0].contract = "widgets/live-map/alt/contract.json".into();
        std::fs::write(&manifest_path, serde_json::to_vec(&manifest).unwrap()).unwrap();

        let descriptor = describe(&cache.0, subject(&bundle_hash, "live-map", false));
        assert_eq!(descriptor.status, WidgetPolicyStatus::Invalid);
        assert!(descriptor.policy.is_empty());
        assert!(
            descriptor
                .invalid_reason
                .unwrap()
                .contains("declares contract path")
        );

        manifest.widgets[0].contract = "widgets/live-map/contract.json".into();
        std::fs::write(&manifest_path, serde_json::to_vec(&manifest).unwrap()).unwrap();
        let mut hostile = map_contract();
        hostile.csp.as_mut().unwrap()[0].connect_src = vec!["https://api.maptiler.com:8443".into()];
        std::fs::write(
            store_dir.join("widgets/live-map/contract.json"),
            serde_json::to_vec(&hostile).unwrap(),
        )
        .unwrap();
        let invalid = describe(&cache.0, subject(&bundle_hash, "live-map", false));
        assert_eq!(invalid.status, WidgetPolicyStatus::Invalid);
        assert!(invalid.policy.is_empty());

        assert!(
            describe(&cache.0, subject(&bundle_hash, "missing", false))
                .invalid_reason
                .is_some()
        );
        let viewer = Viewer::default();
        assert!(
            viewer
                .describe(&cache.0, subject(&hash('c'), "live-map", false), &[])
                .is_err()
        );
        assert!(
            viewer
                .describe(&cache.0, subject(&bundle_hash, "../x", false), &[])
                .is_err()
        );
        let mut bad_package = subject(&bundle_hash, "live-map", false);
        bad_package.package_id = "com example;".into();
        assert!(viewer.describe(&cache.0, bad_package, &[]).is_err());
    }

    #[test]
    fn widget_policy_describe_rejects_wildcards_over_public_suffixes() {
        let cache = TempCache::new("wildcards");
        let mut accepted = map_contract();
        accepted.csp.as_mut().unwrap()[0].img_src =
            vec!["https://*.tiles.customer-maps.com".into()];
        let bundle_hash = install_bundle(&cache.0, accepted);
        let descriptor = describe(&cache.0, subject(&bundle_hash, "live-map", false));
        assert!(descriptor.is_ok(), "{:?}", descriptor.invalid_reason);
        assert_eq!(
            descriptor.policy.csp.img_src,
            vec!["https://*.tiles.customer-maps.com".to_string()]
        );

        let mut spanning = map_contract();
        spanning.csp.as_mut().unwrap()[0].img_src = vec!["https://*.co.uk".into()];
        let bundle_hash = install_bundle(&cache.0, spanning);
        let invalid = describe(&cache.0, subject(&bundle_hash, "live-map", false));
        assert_eq!(invalid.status, WidgetPolicyStatus::Invalid);
        assert!(
            invalid
                .invalid_reason
                .as_deref()
                .unwrap()
                .contains("wildcard base is a public suffix"),
            "{:?}",
            invalid.invalid_reason
        );
        assert!(invalid.policy.is_empty() && invalid.network.is_none());
        assert!(describe(&cache.0, subject(&bundle_hash, "live-map", true)).is_ok());
    }

    #[test]
    fn widget_policy_describe_accepts_and_rejects_runtime_sources() {
        let cache = TempCache::new("runtime");
        let bundle_hash = install_bundle(&cache.0, map_contract());
        let declared = describe(&cache.0, subject(&bundle_hash, "live-map", false));
        let reserved = ["https://api.flow-like.com/api/v1".to_string()];
        let storage_path = storage_source(APP_ID);
        let request = [
            runtime(
                "tileUrl",
                &[
                    RUNTIME_TILES,
                    "https://api.flow-like.com",
                    "https://tiles.nip.io",
                    STORAGE_ORIGIN,
                    &storage_path,
                    "https://*.customer-maps.com",
                ],
            ),
            runtime("otherUrl", &["https://tiles.other-maps.com"]),
        ];
        let viewer = Viewer {
            reserved: &reserved,
            ..Viewer::default()
        };

        let descriptor = viewer
            .describe(&cache.0, subject(&bundle_hash, "live-map", false), &request)
            .unwrap();
        assert!(descriptor.is_ok());
        assert_eq!(
            descriptor.policy.csp.connect_src,
            vec![RUNTIME_TILES.to_string(), MAPTILER.to_string()]
        );
        assert_eq!(
            descriptor.policy.csp.img_src,
            vec![OSM_TILES.to_string(), RUNTIME_TILES.to_string()]
        );
        assert_eq!(descriptor.policy_digest, descriptor.policy.digest());
        assert_ne!(descriptor.policy_digest, declared.policy_digest);
        let runtime_block = descriptor.runtime.as_ref().unwrap();
        assert_eq!(runtime_block.status, WidgetRuntimeStatus::Ok);
        assert_eq!(runtime_block.declared_digest, declared.policy_digest);
        assert!(runtime_block.runtime_digest.is_some());
        assert_eq!(
            rejections(&descriptor),
            BTreeSet::from([
                rejection("otherUrl", "https://tiles.other-maps.com", "unknown-slot"),
                rejection("tileUrl", "https://*.customer-maps.com", "wildcard"),
                rejection("tileUrl", "https://api.flow-like.com", "reserved-host"),
                rejection("tileUrl", "https://tiles.nip.io", "reserved-name"),
                rejection("tileUrl", STORAGE_ORIGIN, "platform-storage"),
                rejection("tileUrl", &storage_path, "platform-storage"),
            ])
        );
        let network = descriptor.network.as_ref().expect("describe classifies");
        let runtime_source = network.purposes[1]
            .sources
            .iter()
            .find(|source| source.source == RUNTIME_TILES)
            .expect("runtime source classified under its purpose");
        assert_eq!(runtime_source.origin, SourceProvenance::Runtime);
        assert_eq!(runtime_source.slot.as_deref(), Some("tileUrl"));
        let json = serde_json::to_value(&descriptor).unwrap();
        assert_eq!(json["runtime"]["status"], "ok");
        assert_eq!(json["runtime"]["rejected"].as_array().unwrap().len(), 6);
        assert!(json["runtime"].get("sources").is_none());

        let empty = viewer
            .describe(
                &cache.0,
                subject(&bundle_hash, "live-map", false),
                &[runtime("tileUrl", &[])],
            )
            .unwrap();
        assert_eq!(empty.policy_digest, declared.policy_digest);
        assert_eq!(empty.runtime.unwrap().status, WidgetRuntimeStatus::None);
    }

    #[test]
    fn widget_policy_describe_accepts_platform_storage_only_for_the_app() {
        let cache = TempCache::new("storage");
        let bundle_hash = install_bundle(&cache.0, map_contract());
        let own = storage_source(APP_ID);
        let foreign = storage_source("other-app");
        let request = [runtime("tileUrl", &[&own, &foreign])];
        let viewer = Viewer {
            app_id: Some(APP_ID),
            ..Viewer::default()
        };

        let descriptor = viewer
            .describe(&cache.0, subject(&bundle_hash, "live-map", false), &request)
            .unwrap();
        assert!(descriptor.policy.csp.connect_src.contains(&own));
        assert!(descriptor.policy.csp.img_src.contains(&own));
        assert!(!descriptor.policy.csp.connect_src.contains(&foreign));
        assert_eq!(
            rejections(&descriptor),
            BTreeSet::from([rejection("tileUrl", &foreign, "platform-storage")])
        );
        let sources = descriptor.runtime_sources().expect("accepted sources");
        assert_eq!(sources.app_id.as_deref(), Some(APP_ID));
        assert_eq!(sources.slots["tileUrl"], vec![own.clone()]);

        let registry = WidgetGrantRegistry::new(WIDGET_GRANT_TTL, 8);
        let minted = mint_widget_grant(&registry, &descriptor, &descriptor.policy_digest).unwrap();
        let resolved = registry
            .resolve(&minted.grant.unwrap(), PACKAGE, &bundle_hash, "live-map")
            .unwrap();
        assert!(resolved.effective.csp.connect_src.contains(&own));
        assert!(!resolved.declared.csp.connect_src.contains(&own));

        let anonymous = Viewer::default()
            .describe(&cache.0, subject(&bundle_hash, "live-map", false), &request)
            .unwrap();
        assert_eq!(
            anonymous.policy_digest,
            describe(&cache.0, subject(&bundle_hash, "live-map", false)).policy_digest
        );
        assert_eq!(
            rejections(&anonymous),
            BTreeSet::from([
                rejection("tileUrl", &own, "platform-storage"),
                rejection("tileUrl", &foreign, "platform-storage"),
            ])
        );
    }

    #[test]
    fn widget_policy_describe_reports_an_engine_without_runtime_sources() {
        let cache = TempCache::new("engine");
        let bundle_hash = install_bundle(&cache.0, map_contract());
        let engine = EngineGate {
            runtime_sources: false,
            ..EngineGate::OPEN
        };
        let descriptor = Viewer {
            engine,
            ..Viewer::default()
        }
        .describe(
            &cache.0,
            subject(&bundle_hash, "live-map", false),
            &[runtime("tileUrl", &[RUNTIME_TILES])],
        )
        .unwrap();
        let declared = describe(&cache.0, subject(&bundle_hash, "live-map", false));
        assert_eq!(descriptor.policy, declared.policy);
        assert_eq!(descriptor.policy_digest, declared.policy_digest);
        let runtime_block = descriptor.runtime.as_ref().unwrap();
        assert_eq!(runtime_block.status, WidgetRuntimeStatus::Unavailable);
        assert_eq!(runtime_block.invalid_reason.as_deref(), Some("engine"));
        assert_eq!(descriptor.engine, Some(engine.support()));
    }

    #[test]
    fn widget_policy_request_shape_errors_are_prefixed() {
        let cache = TempCache::new("shape");
        let bundle_hash = install_bundle(&cache.0, map_contract());
        let viewer = Viewer::default();
        let live = || subject(&bundle_hash, "live-map", false);

        let too_many: Vec<_> = (0..9)
            .map(|index| runtime(&format!("slot{index}"), &[RUNTIME_TILES]))
            .collect();
        let duplicate = [
            runtime("tileUrl", &[RUNTIME_TILES]),
            runtime("tileUrl", &[MAPTILER]),
        ];
        let long_source = format!("https://{}.example.org", "a".repeat(300));
        let oversized = [runtime("tileUrl", &[&long_source])];
        let many_sources: Vec<String> = (0..33)
            .map(|index| format!("https://t{index}.customer-maps.com"))
            .collect();
        let many_sources = [WidgetRuntimeSourceRequest {
            slot: "tileUrl".into(),
            sources: many_sources,
        }];
        for request in [too_many.as_slice(), &duplicate, &oversized, &many_sources] {
            let error = viewer.describe(&cache.0, live(), request).unwrap_err();
            assert!(error.starts_with("invalid_runtime_sources: "), "{error}");
        }

        let error = viewer
            .describe(
                &cache.0,
                subject(&bundle_hash, "live-map", true),
                &[runtime("tileUrl", &[RUNTIME_TILES])],
            )
            .unwrap_err();
        assert!(error.starts_with("runtime_sources_in_preview: "), "{error}");
        assert!(
            viewer
                .describe(&cache.0, subject(&bundle_hash, "live-map", true), &[])
                .is_ok()
        );
    }

    #[test]
    fn widget_policy_mint_rejects_a_stale_digest_and_skips_empty_policies() {
        let cache = TempCache::new("mint");
        let bundle_hash = install_bundle(&cache.0, map_contract());
        let registry = WidgetGrantRegistry::new(WIDGET_GRANT_TTL, 8);
        let descriptor = describe(&cache.0, subject(&bundle_hash, "live-map", false));

        let stale = mint_widget_grant(&registry, &descriptor, &WidgetPolicy::default().digest())
            .unwrap_err();
        assert!(stale.starts_with("policy_changed: "), "{stale}");
        assert!(registry.table().grants.is_empty());

        let minted = mint_widget_grant(&registry, &descriptor, &descriptor.policy_digest).unwrap();
        let grant = minted.grant.expect("non-empty policy mints a grant");
        assert_eq!(minted.expires_in, WIDGET_GRANT_TTL.as_secs());
        assert_eq!(minted.policy_digest, descriptor.policy_digest);
        assert_eq!(
            registry.resolve(&grant, PACKAGE, &bundle_hash, "live-map"),
            Some(granted(descriptor.policy.clone()))
        );
        let json = serde_json::to_value(WidgetGrantMint {
            grant: None,
            expires_in: 60,
            policy_digest: "sha256:x".into(),
        })
        .unwrap();
        assert_eq!(
            json,
            serde_json::json!({ "grant": null, "expiresIn": 60, "policyDigest": "sha256:x" })
        );

        let cache = TempCache::new("mint-empty");
        let bundle_hash = install_bundle(&cache.0, WidgetContract::new("plain"));
        let plain = describe(&cache.0, subject(&bundle_hash, "plain", false));
        assert!(plain.is_ok() && plain.policy.is_empty());
        let minted = mint_widget_grant(&registry, &plain, &plain.policy_digest).unwrap();
        assert_eq!(minted.grant, None);
        assert_eq!(minted.policy_digest, WidgetPolicy::default().digest());
    }

    #[test]
    fn widget_policy_mint_stores_the_effective_and_declared_policy() {
        let cache = TempCache::new("mint-runtime");
        let bundle_hash = install_bundle(&cache.0, map_contract());
        let registry = WidgetGrantRegistry::new(WIDGET_GRANT_TTL, 8);
        let request = [runtime("tileUrl", &[RUNTIME_TILES])];
        let declared = describe(&cache.0, subject(&bundle_hash, "live-map", false));
        let described = Viewer::default()
            .describe(&cache.0, subject(&bundle_hash, "live-map", false), &request)
            .unwrap();
        let rederived = Viewer {
            network_at: None,
            ..Viewer::default()
        }
        .describe(&cache.0, subject(&bundle_hash, "live-map", false), &request)
        .unwrap();
        assert_eq!(rederived.policy_digest, described.policy_digest);

        let stale = mint_widget_grant(&registry, &rederived, &declared.policy_digest).unwrap_err();
        assert!(stale.starts_with("policy_changed: "), "{stale}");

        let minted = mint_widget_grant(&registry, &rederived, &described.policy_digest).unwrap();
        assert_eq!(minted.policy_digest, described.policy_digest);
        let grant = minted.grant.unwrap();
        let resolved = registry
            .resolve(&grant, PACKAGE, &bundle_hash, "live-map")
            .unwrap();
        assert_eq!(resolved.effective, described.policy);
        assert_eq!(resolved.declared, declared.policy);
        assert_eq!(resolved.declared.digest(), described.declared_digest());
        let no_runtime = EngineGate {
            runtime_sources: false,
            ..EngineGate::OPEN
        };
        assert_eq!(resolved.served(&no_runtime), declared.policy);
        assert_eq!(resolved.served(&EngineGate::OPEN), described.policy);

        let again = mint_widget_grant(&registry, &rederived, &described.policy_digest).unwrap();
        assert_eq!(again.grant.as_deref(), Some(grant.as_str()));
        assert_eq!(again.expires_in, WIDGET_GRANT_TTL.as_secs());
        let declared_grant =
            mint_widget_grant(&registry, &declared, &declared.policy_digest).unwrap();
        assert_ne!(declared_grant.grant.as_deref(), Some(grant.as_str()));
    }
}
