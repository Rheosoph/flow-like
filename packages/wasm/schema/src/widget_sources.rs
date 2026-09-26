//! Classification of widget network sources into warning levels, with the
//! provider facts the consent UI turns into explanations (spec §14.3).

use std::fmt;
use std::sync::LazyLock;

use serde::{Deserialize, Serialize};

use crate::widget_policy::{
    CspDirective, CspSourceRejection, csp_source_host, validate_csp_source,
};

mod catalog;
mod index;
mod invariants;
mod network;
mod rules;
mod text;

#[cfg(test)]
mod tests;

use index::SourceIndex;

/// Ordered from least to most parties that can be on the other end.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "lowercase")]
pub enum WidgetSourceLevel {
    Known,
    External,
    Shared,
    Broad,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "kebab-case")]
pub enum WidgetSourceKind {
    Service,
    Exact,
    Subdomains,
    TenantHost,
    TenantSubdomains,
    SharedHost,
    SharedWildcard,
    PlatformHost,
    PlatformStorage,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "lowercase")]
pub enum SourceProvenance {
    Declared,
    Runtime,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct WidgetSourceClass {
    pub kind: WidgetSourceKind,
    pub level: WidgetSourceLevel,
    pub host: String,
    pub emphasis: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub about_key: Option<String>,
}

/// Display-only `network` block of a widget policy descriptor (§14.2.6).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct WidgetNetwork {
    pub level: WidgetSourceLevel,
    pub catalog_version: u32,
    pub psl_version: String,
    pub stale: bool,
    pub purposes: Vec<WidgetNetworkPurpose>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct WidgetNetworkPurpose {
    pub reason: String,
    pub level: WidgetSourceLevel,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub inputs: Vec<String>,
    pub sources: Vec<WidgetNetworkSource>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct WidgetNetworkSource {
    pub source: String,
    pub directives: Vec<CspDirective>,
    pub origin: SourceProvenance,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub slot: Option<String>,
    #[serde(flatten)]
    pub class: WidgetSourceClass,
}

/// One purpose group as the classifier needs it, built by the descriptor code
/// from the contract so this module never depends on the contract types.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NetworkPurposeInput {
    pub reason: String,
    pub declared: Vec<(CspDirective, String)>,
    pub inputs: Vec<String>,
}

/// A runtime source the server accepted for a declared input slot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NetworkRuntimeInput {
    pub purpose: usize,
    pub slot: String,
    pub directive: CspDirective,
    pub source: String,
}

/// A build whose PSL is older than this at describe time is `stale`.
pub const WIDGET_SOURCE_STALE_AFTER_SECS: i64 = 180 * 86_400;

/// Every About key a catalog provider may reference (§14.3.5).
pub const WIDGET_SOURCE_ABOUT_KEYS: [&str; 14] = [
    "widgetSourceAboutObjectStorage",
    "widgetSourceAboutSharedCdn",
    "widgetSourceAboutAppHosting",
    "widgetSourceAboutCloudPlatform",
    "widgetSourceAboutPackageCdn",
    "widgetSourceAboutLibraryCdn",
    "widgetSourceAboutCodeHosting",
    "widgetSourceAboutUserContent",
    "widgetSourceAboutMapData",
    "widgetSourceAboutMapTiles",
    "widgetSourceAboutSatelliteImagery",
    "widgetSourceAboutWebFonts",
    "widgetSourceAboutRequestCapture",
    "widgetSourceAboutTunnel",
];

const PUBLIC_SUFFIX_RULES: &str = include_str!("../data/public_suffix_rules.txt");
const WIDGET_SOURCE_CATALOG: &str = include_str!("../data/widget_source_catalog.json");

static WIDGET_SOURCE_DATA: LazyLock<Result<SourceIndex, String>> =
    LazyLock::new(|| SourceIndex::build(PUBLIC_SUFFIX_RULES, WIDGET_SOURCE_CATALOG));

/// Why a source could not be classified. [`code`](Self::code) is the stable
/// identifier shared with the TypeScript mirror through the fixture.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WidgetSourceRejection {
    Source(CspSourceRejection),
    WildcardPublicSuffix,
    DataUnavailable,
}

impl WidgetSourceRejection {
    pub fn code(self) -> &'static str {
        match self {
            WidgetSourceRejection::Source(rejection) => rejection.code(),
            WidgetSourceRejection::WildcardPublicSuffix => "wildcard-public-suffix",
            WidgetSourceRejection::DataUnavailable => "source-data-unavailable",
        }
    }
}

impl fmt::Display for WidgetSourceRejection {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            WidgetSourceRejection::Source(rejection) => rejection.fmt(formatter),
            WidgetSourceRejection::WildcardPublicSuffix => {
                formatter.write_str("wildcard base is a public suffix or spans public suffixes")
            }
            WidgetSourceRejection::DataUnavailable => {
                formatter.write_str("widget source classification data is unavailable")
            }
        }
    }
}

impl std::error::Error for WidgetSourceRejection {}

impl From<CspSourceRejection> for WidgetSourceRejection {
    fn from(rejection: CspSourceRejection) -> Self {
        WidgetSourceRejection::Source(rejection)
    }
}

fn source_index() -> Result<&'static SourceIndex, WidgetSourceRejection> {
    WIDGET_SOURCE_DATA
        .as_ref()
        .map_err(|_| WidgetSourceRejection::DataUnavailable)
}

/// Parses the embedded PSL rules and catalog; call at API and desktop startup.
pub fn warm_widget_source_data() -> Result<(), String> {
    WIDGET_SOURCE_DATA
        .as_ref()
        .map(|_| ())
        .map_err(Clone::clone)
}

/// `(catalogVersion, pslVersion)` of the compiled-in data, `(0, "")` when it
/// failed to load.
pub fn widget_source_data_versions() -> (u32, String) {
    source_index().map_or((0, String::new()), |index| {
        (index.catalog.catalog_version, index.psl_version.clone())
    })
}

/// Classifies one grammar-valid source (§14.3.3). Runtime sources are never
/// wildcards and are at least `external`.
pub fn classify_widget_source(
    source: &str,
    provenance: SourceProvenance,
) -> Result<WidgetSourceClass, WidgetSourceRejection> {
    classify_with(source_index()?, source, provenance)
}

fn classify_with(
    index: &SourceIndex,
    source: &str,
    provenance: SourceProvenance,
) -> Result<WidgetSourceClass, WidgetSourceRejection> {
    let (host, wildcard) = source_host(source)?;
    if wildcard && provenance == SourceProvenance::Runtime {
        return Err(CspSourceRejection::Wildcard.into());
    }
    if wildcard && is_public_suffix_base(index, &host[2..]) {
        return Err(WidgetSourceRejection::WildcardPublicSuffix);
    }
    Ok(index.classify(host, wildcard, provenance))
}

/// Host of a source under the §14.3.1 structural grammar, and whether it is a
/// leading-label wildcard (`*.` kept in the host).
fn source_host(source: &str) -> Result<(&str, bool), CspSourceRejection> {
    validate_csp_source(CspDirective::ConnectSrc, source)?;
    let host = csp_source_host(source).ok_or(CspSourceRejection::MissingScheme)?;
    Ok((host, host.starts_with("*.")))
}

fn wildcard_base(source: &str) -> Option<&str> {
    csp_source_host(source).and_then(|host| host.strip_prefix("*."))
}

fn is_public_suffix_base(index: &SourceIndex, base: &str) -> bool {
    index.is_icann_suffix(base) || index.is_icann_below(base)
}

/// PSL check for declared wildcards (§14.3.1, code `wildcard-public-suffix`).
/// Sources without a leading `*.` are ignored; grammar errors are reported by
/// the structural validators.
pub fn validate_wildcard_bases<'a>(
    sources: impl IntoIterator<Item = &'a str>,
) -> Result<(), Vec<String>> {
    let wildcards: Vec<(&str, &str)> = sources
        .into_iter()
        .filter_map(|source| wildcard_base(source).map(|base| (source, base)))
        .collect();
    if wildcards.is_empty() {
        return Ok(());
    }
    let index = match source_index() {
        Ok(index) => index,
        Err(rejection) => return Err(vec![format!("Cannot check csp wildcards: {rejection}")]),
    };
    let errors: Vec<String> = wildcards
        .into_iter()
        .filter(|(_, base)| is_public_suffix_base(index, base))
        .map(|(source, _)| {
            format!(
                "Invalid csp source \"{source}\": {}",
                WidgetSourceRejection::WildcardPublicSuffix
            )
        })
        .collect();
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}

/// Reason rule 8 `reason-contains-address` (§14.2.4). Publish and bundler
/// only. Without classification data every dotted token with a last label of
/// two or more characters counts as an address.
pub fn reason_contains_address(reason: &str) -> bool {
    let index = source_index().ok();
    text::contains_address(reason, |label| {
        index.is_none_or(|index| index.is_icann_tld(label))
    })
}

/// Display-only `network` block of a descriptor (§14.2.6, §14.3.4). `None` when a
/// source cannot be classified, a runtime input names a missing purpose, there
/// are no purposes, or the data failed to load.
pub fn describe_network(
    purposes: &[NetworkPurposeInput],
    runtime: &[NetworkRuntimeInput],
    now_unix_secs: i64,
) -> Option<WidgetNetwork> {
    network::describe_network_with(source_index().ok()?, purposes, runtime, now_unix_secs)
}

/// §14.3.2 catalog invariants, including the pins against the embedded files.
pub fn widget_sources_catalog_invariants() -> Result<(), Vec<String>> {
    let index = WIDGET_SOURCE_DATA
        .as_ref()
        .map_err(|error| vec![error.clone()])?;
    invariants::check(index)
}
