//! Widget network policy.
//!
//! A widget contract may declare CSP extensions (`csp`) as purpose groups next
//! to its browser capabilities. [`WidgetPolicy`] is the effective set a host
//! grants after consent and preview stripping: the declared sources of every
//! purpose plus the runtime sources the viewer approved for declared network
//! inputs. Its digest binds grants to exactly that set. Sources are accepted
//! or rejected, never normalized; the bundler normalizes authoring input before
//! it reaches this grammar.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::sync::LazyLock;
use unicode_normalization::UnicodeNormalization;
use unicode_normalization::char::is_combining_mark;

use crate::widget::{ContractInput, ContractInputType, WidgetContract};
use crate::widget_frame::widget_document_csp;
use crate::widget_sources::{NetworkPurposeInput, NetworkRuntimeInput, WidgetNetwork};

/// Upper bound on declared sources across all directives and purposes.
pub const MAX_WIDGET_CSP_SOURCES: usize = 16;

/// Upper bound on Σ(len + 1) over the declared sources.
pub const MAX_WIDGET_CSP_SOURCE_BYTES: usize = 1536;

pub const MAX_WIDGET_CSP_PURPOSES: usize = 8;

/// Upper bound on network inputs across all purposes.
pub const MAX_WIDGET_NETWORK_INPUTS: usize = 8;

/// Upper bound on the segments of a network input path, root included.
pub const MAX_WIDGET_INPUT_PATH_SEGMENTS: usize = 6;

pub const MAX_WIDGET_TEMPLATE_SUBDOMAINS: usize = 16;

/// Upper bound on the entries of an effective policy (declared plus runtime).
pub const MAX_WIDGET_EFFECTIVE_CSP_SOURCES: usize = 32;

/// Upper bound on distinct runtime sources accepted for one widget.
pub const MAX_WIDGET_RUNTIME_SOURCES: usize = 8;

/// Upper bound on the (directive, source) entries runtime sources add.
pub const MAX_WIDGET_RUNTIME_ENTRIES: usize = 16;

/// Upper bound on the canonical JSON of accepted runtime slots, which is also
/// the URL carrier before encoding.
pub const MAX_WIDGET_RUNTIME_BYTES: usize = 1024;

/// Budget of a served document CSP header, sandbox included.
pub const MAX_WIDGET_DOCUMENT_CSP_BYTES: usize = 3584;

pub const MAX_RUNTIME_REQUEST_SLOTS: usize = 8;
pub const MAX_RUNTIME_REQUEST_SOURCES: usize = 32;
pub const MAX_RUNTIME_REQUEST_SOURCE_BYTES: usize = 300;

/// Domain separator hashed in front of the canonical policy JSON.
pub const WIDGET_POLICY_DIGEST_DOMAIN: &str = "flow-like-widget-policy/v1\n";

/// Domain separator hashed in front of the canonical runtime sources JSON.
pub const WIDGET_RUNTIME_DIGEST_DOMAIN: &str = "flow-like-widget-runtime/v1\n";

/// Widget inputs the host injects itself; network inputs never read them.
pub const HOST_RESERVED_INPUT_KEYS: &[&str] = &["publicMediaGrants"];

pub const WIDGET_POLICY_SOURCE_LOCAL: &str = "local";
pub const WIDGET_POLICY_SOURCE_HUB: &str = "hub";

/// `invalidReason` prefix of a declared policy over the document CSP budget.
pub const DECLARED_CSP_TOO_LARGE: &str = "csp-too-large";

pub const RUNTIME_REJECTION_UNKNOWN_SLOT: &str = "unknown-slot";
pub const RUNTIME_REJECTION_PLATFORM_STORAGE: &str = "platform-storage";
pub const RUNTIME_REJECTION_RESERVED_HOST: &str = "reserved-host";
pub const RUNTIME_INVALID_TOO_MANY_SOURCES: &str = "too-many-sources";
pub const RUNTIME_INVALID_TOO_LARGE: &str = "too-large";
pub const RUNTIME_UNAVAILABLE_ENGINE: &str = "engine";

const MAX_HOST_LEN: usize = 253;

const MAX_APP_ID_LEN: usize = 64;

const RESERVED_NAME_SUFFIXES: &[&str] = &[
    "localhost",
    "local",
    "internal",
    "lan",
    "home.arpa",
    "test",
    "example",
    "invalid",
    "onion",
    "nip.io",
    "sslip.io",
    "traefik.me",
    "localtest.me",
    "lvh.me",
    "localhost.direct",
];

const REASON_MIN_CHARS: usize = 8;
const REASON_MAX_CHARS: usize = 120;
const REASON_MIN_LETTERS: usize = 3;
const REASON_MAX_MARK_RUN: usize = 2;
const REASON_PUNCTUATION: &[char] = &[',', '.', ':', ';', '(', ')', '-', '\'', '/', '&'];
const REASON_PRODUCT: &str = "flowlike";
const REASON_ASSURANCE_WORDS: &[&str] = &[
    "verified",
    "trusted",
    "approved",
    "official",
    "certified",
    "secure",
    "securely",
    "safe",
    "safely",
];
const REASON_ATTRIBUTION_WORDS: &[&str] = &[
    "app",
    "apps",
    "application",
    "admin",
    "admins",
    "administrator",
    "administrators",
    "organization",
    "organisation",
    "company",
    "workspace",
    "project",
    "owner",
];

/// `registry:<host>` descriptor source of an installed registry package.
pub fn registry_policy_source(hub_host: &str) -> String {
    format!("registry:{hub_host}")
}

/// A CSP fetch directive a widget contract may extend.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub enum CspDirective {
    ConnectSrc,
    ImgSrc,
    FontSrc,
    MediaSrc,
    StyleSrc,
}

impl CspDirective {
    pub const ALL: [CspDirective; 5] = [
        CspDirective::ConnectSrc,
        CspDirective::ImgSrc,
        CspDirective::FontSrc,
        CspDirective::MediaSrc,
        CspDirective::StyleSrc,
    ];

    /// Key inside the contract's `csp` purposes, e.g. `connectSrc`.
    pub fn contract_key(self) -> &'static str {
        match self {
            CspDirective::ConnectSrc => "connectSrc",
            CspDirective::ImgSrc => "imgSrc",
            CspDirective::FontSrc => "fontSrc",
            CspDirective::MediaSrc => "mediaSrc",
            CspDirective::StyleSrc => "styleSrc",
        }
    }

    /// Directive name in a CSP header, e.g. `connect-src`.
    pub fn csp_name(self) -> &'static str {
        match self {
            CspDirective::ConnectSrc => "connect-src",
            CspDirective::ImgSrc => "img-src",
            CspDirective::FontSrc => "font-src",
            CspDirective::MediaSrc => "media-src",
            CspDirective::StyleSrc => "style-src",
        }
    }

    pub fn allowed_schemes(self) -> &'static [&'static str] {
        match self {
            CspDirective::ConnectSrc => &["https", "wss"],
            _ => &["https"],
        }
    }

    pub fn from_contract_key(key: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|directive| directive.contract_key() == key)
    }
}

impl fmt::Display for CspDirective {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.contract_key())
    }
}

/// Why a CSP source was rejected. [`code`](Self::code) is the stable
/// identifier shared with the TypeScript mirror through the fixture.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CspSourceRejection {
    Empty,
    NonAscii,
    Wildcard,
    Keyword,
    ForbiddenCharacter,
    Uppercase,
    Userinfo,
    QueryOrFragment,
    IpLiteral,
    MissingScheme,
    SchemeNotAllowed,
    Port,
    Path,
    InvalidHost,
    ReservedName,
}

impl CspSourceRejection {
    pub fn code(self) -> &'static str {
        match self {
            CspSourceRejection::Empty => "empty",
            CspSourceRejection::NonAscii => "non-ascii",
            CspSourceRejection::Wildcard => "wildcard",
            CspSourceRejection::Keyword => "keyword",
            CspSourceRejection::ForbiddenCharacter => "forbidden-character",
            CspSourceRejection::Uppercase => "uppercase",
            CspSourceRejection::Userinfo => "userinfo",
            CspSourceRejection::QueryOrFragment => "query-or-fragment",
            CspSourceRejection::IpLiteral => "ip-literal",
            CspSourceRejection::MissingScheme => "missing-scheme",
            CspSourceRejection::SchemeNotAllowed => "scheme-not-allowed",
            CspSourceRejection::Port => "port",
            CspSourceRejection::Path => "path",
            CspSourceRejection::InvalidHost => "invalid-host",
            CspSourceRejection::ReservedName => "reserved-name",
        }
    }

    fn message(self) -> &'static str {
        match self {
            CspSourceRejection::Empty => "source is empty",
            CspSourceRejection::NonAscii => "non-ASCII characters are not allowed (use punycode)",
            CspSourceRejection::Wildcard => {
                "wildcards are only allowed as a leading \"*.\" label of the host"
            }
            CspSourceRejection::Keyword => "keywords, nonces and hashes are not allowed",
            CspSourceRejection::ForbiddenCharacter => "contains a forbidden character",
            CspSourceRejection::Uppercase => "uppercase characters are not allowed",
            CspSourceRejection::Userinfo => "userinfo is not allowed",
            CspSourceRejection::QueryOrFragment => "queries and fragments are not allowed",
            CspSourceRejection::IpLiteral => "IP literals are not allowed",
            CspSourceRejection::MissingScheme => "must have the form scheme://host",
            CspSourceRejection::SchemeNotAllowed => "scheme is not allowed for this directive",
            CspSourceRejection::Port => "ports are not allowed",
            CspSourceRejection::Path => "paths are not allowed",
            CspSourceRejection::InvalidHost => "host is not a valid public DNS name",
            CspSourceRejection::ReservedName => "host uses a reserved or local-only name",
        }
    }
}

impl fmt::Display for CspSourceRejection {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.message())
    }
}

impl std::error::Error for CspSourceRejection {}

/// Checks one source against the structural grammar
/// `scheme "://" [ "*." ] host`. Checks run in this order, and the first
/// failing check is the reason: empty, non-ascii, wildcard (a `*` anywhere
/// but a leading `*.` label: the source must match `^[a-z]+://\*\.[^*]+$`),
/// keyword (leading `'`), forbidden-character (whitespace, controls, `;`, `,`,
/// quotes), uppercase, userinfo (`@`), query-or-fragment (`?`, `#`),
/// ip-literal (`[`, `]`), forbidden-character (outside `[a-z0-9.:/-]` and the
/// accepted `*`), missing-scheme (no non-empty scheme before `://`),
/// scheme-not-allowed, port (`:` after the host), path (`/` after the host),
/// then the host checks on the host without `*.`: invalid-host (empty, too
/// long, empty label), ip-literal (numeric last label), invalid-host (label
/// grammar), reserved-name, invalid-host (fewer than two labels, TLD grammar).
/// Public suffix checks on wildcard bases live in `widget_sources`.
pub fn validate_csp_source(
    directive: CspDirective,
    source: &str,
) -> Result<(), CspSourceRejection> {
    if source.is_empty() {
        return Err(CspSourceRejection::Empty);
    }
    if !source.is_ascii() {
        return Err(CspSourceRejection::NonAscii);
    }
    if source.contains('*') && !is_wildcard_form(source) {
        return Err(CspSourceRejection::Wildcard);
    }
    if source.starts_with('\'') {
        return Err(CspSourceRejection::Keyword);
    }
    if source.bytes().any(|b| {
        b.is_ascii_whitespace() || b.is_ascii_control() || matches!(b, b';' | b',' | b'"' | b'\'')
    }) {
        return Err(CspSourceRejection::ForbiddenCharacter);
    }
    if source.bytes().any(|b| b.is_ascii_uppercase()) {
        return Err(CspSourceRejection::Uppercase);
    }
    if source.contains('@') {
        return Err(CspSourceRejection::Userinfo);
    }
    if source.contains(['?', '#']) {
        return Err(CspSourceRejection::QueryOrFragment);
    }
    if source.contains(['[', ']']) {
        return Err(CspSourceRejection::IpLiteral);
    }
    if !source.bytes().all(|b| {
        b.is_ascii_lowercase()
            || b.is_ascii_digit()
            || matches!(b, b'.' | b':' | b'/' | b'-' | b'*')
    }) {
        return Err(CspSourceRejection::ForbiddenCharacter);
    }
    let Some((scheme, rest)) = source
        .split_once("://")
        .filter(|(scheme, _)| !scheme.is_empty())
    else {
        return Err(CspSourceRejection::MissingScheme);
    };
    if !directive.allowed_schemes().contains(&scheme) {
        return Err(CspSourceRejection::SchemeNotAllowed);
    }
    let (host, tail) = rest
        .find([':', '/'])
        .map_or((rest, ""), |index| rest.split_at(index));
    if tail.starts_with(':') {
        return Err(CspSourceRejection::Port);
    }
    if tail.starts_with('/') {
        return Err(CspSourceRejection::Path);
    }
    match host.strip_prefix("*.") {
        Some(base) => validate_host(base, MAX_HOST_LEN - 2),
        None => validate_host(host, MAX_HOST_LEN),
    }
}

fn is_wildcard_form(source: &str) -> bool {
    source.split_once("://").is_some_and(|(scheme, rest)| {
        !scheme.is_empty()
            && scheme.bytes().all(|b| b.is_ascii_lowercase())
            && rest
                .strip_prefix("*.")
                .is_some_and(|tail| !tail.is_empty() && !tail.contains('*'))
    })
}

fn validate_host(host: &str, max_len: usize) -> Result<(), CspSourceRejection> {
    if host.is_empty() || host.len() > max_len {
        return Err(CspSourceRejection::InvalidHost);
    }
    let labels: Vec<&str> = host.split('.').collect();
    if labels.iter().any(|label| label.is_empty()) {
        return Err(CspSourceRejection::InvalidHost);
    }
    let tld = labels[labels.len() - 1];
    if tld.bytes().all(|b| b.is_ascii_digit()) {
        return Err(CspSourceRejection::IpLiteral);
    }
    if !labels.iter().all(|label| is_dns_label(label)) {
        return Err(CspSourceRejection::InvalidHost);
    }
    if RESERVED_NAME_SUFFIXES
        .iter()
        .any(|suffix| host_matches(host, suffix))
    {
        return Err(CspSourceRejection::ReservedName);
    }
    if labels.len() < 2 || !is_tld(tld) {
        return Err(CspSourceRejection::InvalidHost);
    }
    Ok(())
}

fn is_dns_label(label: &str) -> bool {
    let bytes = label.as_bytes();
    let alnum = |b: &u8| b.is_ascii_lowercase() || b.is_ascii_digit();
    (1..=63).contains(&bytes.len())
        && alnum(&bytes[0])
        && alnum(&bytes[bytes.len() - 1])
        && bytes.iter().all(|b| alnum(b) || *b == b'-')
}

fn is_tld(label: &str) -> bool {
    match label.strip_prefix("xn--") {
        Some(rest) => {
            (1..=59).contains(&rest.len())
                && rest
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        }
        None => (2..=63).contains(&label.len()) && label.bytes().all(|b| b.is_ascii_lowercase()),
    }
}

fn host_matches(host: &str, reserved: &str) -> bool {
    host == reserved
        || host
            .strip_suffix(reserved)
            .is_some_and(|prefix| prefix.ends_with('.'))
}

/// Checks a platform-storage path source (`scheme://host/segment/…/`, §14.4.5):
/// the origin passes [`validate_csp_source`] as an exact host, and the path is
/// one or more `[A-Za-z0-9._-]` segments (never `.` or `..`) ending in `/`.
pub fn validate_path_source(
    directive: CspDirective,
    source: &str,
) -> Result<(), CspSourceRejection> {
    let Some((origin, path)) = split_path_source(source) else {
        return validate_csp_source(directive, source).and(Err(CspSourceRejection::Path));
    };
    if origin.contains('*') {
        return Err(CspSourceRejection::Wildcard);
    }
    validate_csp_source(directive, origin)?;
    let valid_path = path
        .strip_prefix('/')
        .and_then(|inner| inner.strip_suffix('/'))
        .is_some_and(|inner| {
            !inner.is_empty()
                && inner.split('/').all(|segment| {
                    !segment.is_empty()
                        && segment != "."
                        && segment != ".."
                        && segment
                            .bytes()
                            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
                })
        });
    if valid_path {
        Ok(())
    } else {
        Err(CspSourceRejection::Path)
    }
}

fn split_path_source(source: &str) -> Option<(&str, &str)> {
    let (scheme, rest) = source.split_once("://")?;
    let slash = rest.find('/')?;
    Some(source.split_at(scheme.len() + 3 + slash))
}

/// Whether a source carries a path, i.e. is a platform-storage path source.
pub fn is_path_source(source: &str) -> bool {
    split_path_source(source).is_some()
}

/// Whether a source is a leading-label wildcard (`scheme://*.host`).
pub fn is_wildcard_source(source: &str) -> bool {
    csp_source_host(source).is_some_and(|host| host.starts_with("*."))
}

/// Host of a grammar-valid source: `scheme://host` or `scheme://*.host`
/// (wildcard included), or the host of a path source.
pub fn csp_source_host(source: &str) -> Option<&str> {
    source
        .split_once("://")
        .map(|(_, rest)| rest.split('/').next().unwrap_or(rest))
}

/// Lowercase host of a reserved-host entry, which may be a bare host, an
/// authority (`host:port`) or an origin/URL (`https://host:port/path`).
pub fn reserved_host(value: &str) -> Option<String> {
    let value = value.trim();
    let without_scheme = value.split_once("://").map_or(value, |(_, rest)| rest);
    let authority = without_scheme
        .split(['/', '?', '#'])
        .next()
        .unwrap_or_default();
    let authority = authority
        .rsplit_once('@')
        .map_or(authority, |(_, host)| host);
    let host = if let Some(bracketed) = authority.strip_prefix('[') {
        bracketed.split(']').next().unwrap_or_default()
    } else {
        authority.split(':').next().unwrap_or_default()
    };
    let host = host.trim_end_matches('.').to_ascii_lowercase();
    (!host.is_empty()).then_some(host)
}

/// Whether `host` equals, or is a subdomain of, any reserved entry.
pub fn is_reserved_host(host: &str, reserved_hosts: &[String]) -> bool {
    let host = host.to_ascii_lowercase();
    reserved_hosts
        .iter()
        .filter_map(|entry| reserved_host(entry))
        .any(|reserved| host_matches(&host, &reserved))
}

/// Reserved coverage of a source host. An exact host is covered as in
/// [`is_reserved_host`]; a wildcard `*.B` is covered when `B` equals or is
/// under a reserved host, or when a reserved host lies under `B`.
pub fn is_reserved_source_host(host: &str, reserved_hosts: &[String]) -> bool {
    let Some(base) = host.strip_prefix("*.") else {
        return is_reserved_host(host, reserved_hosts);
    };
    let base = base.to_ascii_lowercase();
    reserved_hosts
        .iter()
        .filter_map(|entry| reserved_host(entry))
        .any(|reserved| host_matches(&base, &reserved) || host_matches(&reserved, &base))
}

/// CSP sources per directive: the flattened declaration of a contract, or an
/// effective policy's network sources.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WidgetCsp {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub connect_src: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub img_src: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub font_src: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub media_src: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub style_src: Vec<String>,
}

impl WidgetCsp {
    pub fn is_empty(&self) -> bool {
        CspDirective::ALL
            .into_iter()
            .all(|directive| self.sources(directive).is_empty())
    }

    pub fn sources(&self, directive: CspDirective) -> &[String] {
        match directive {
            CspDirective::ConnectSrc => &self.connect_src,
            CspDirective::ImgSrc => &self.img_src,
            CspDirective::FontSrc => &self.font_src,
            CspDirective::MediaSrc => &self.media_src,
            CspDirective::StyleSrc => &self.style_src,
        }
    }

    pub fn sources_mut(&mut self, directive: CspDirective) -> &mut Vec<String> {
        match directive {
            CspDirective::ConnectSrc => &mut self.connect_src,
            CspDirective::ImgSrc => &mut self.img_src,
            CspDirective::FontSrc => &mut self.font_src,
            CspDirective::MediaSrc => &mut self.media_src,
            CspDirective::StyleSrc => &mut self.style_src,
        }
    }

    pub fn source_count(&self) -> usize {
        CspDirective::ALL
            .into_iter()
            .map(|directive| self.sources(directive).len())
            .sum()
    }

    /// Σ(len + 1) over every entry: the bytes the sources add to a header.
    pub fn source_bytes(&self) -> usize {
        self.entries().map(|(_, source)| source.len() + 1).sum()
    }

    pub fn entries(&self) -> impl Iterator<Item = (CspDirective, &str)> {
        CspDirective::ALL.into_iter().flat_map(move |directive| {
            self.sources(directive)
                .iter()
                .map(move |source| (directive, source.as_str()))
        })
    }

    /// Sorts every list bytewise and removes duplicates.
    pub fn canonicalize(&mut self) {
        for directive in CspDirective::ALL {
            let sources = self.sources_mut(directive);
            sources.sort();
            sources.dedup();
        }
    }

    /// Declared sources: exact or wildcard grammar, canonical order, at most
    /// [`MAX_WIDGET_CSP_SOURCES`] entries and [`MAX_WIDGET_CSP_SOURCE_BYTES`].
    /// Reserved hosts are deployment-specific; see [`WidgetPolicy::validate`].
    pub fn validate_declared(&self) -> Result<(), Vec<String>> {
        let mut errors = self.grammar_errors(validate_csp_source);
        let count = self.source_count();
        if count > MAX_WIDGET_CSP_SOURCES {
            errors.push(format!(
                "csp declares {count} sources; at most {MAX_WIDGET_CSP_SOURCES} are allowed"
            ));
        }
        let bytes = self.source_bytes();
        if bytes > MAX_WIDGET_CSP_SOURCE_BYTES {
            errors.push(format!(
                "csp sources take {bytes} bytes; at most {MAX_WIDGET_CSP_SOURCE_BYTES} are allowed"
            ));
        }
        into_result(errors)
    }

    /// Effective sources: exact, wildcard or platform-storage path grammar,
    /// canonical order and at most [`MAX_WIDGET_EFFECTIVE_CSP_SOURCES`] entries.
    pub fn validate_effective(&self) -> Result<(), Vec<String>> {
        let mut errors = self.grammar_errors(|directive, source| {
            if is_path_source(source) {
                validate_path_source(directive, source)
            } else {
                validate_csp_source(directive, source)
            }
        });
        let count = self.source_count();
        if count > MAX_WIDGET_EFFECTIVE_CSP_SOURCES {
            errors.push(format!(
                "csp holds {count} sources; at most {MAX_WIDGET_EFFECTIVE_CSP_SOURCES} are allowed"
            ));
        }
        into_result(errors)
    }

    fn grammar_errors(
        &self,
        validate: impl Fn(CspDirective, &str) -> Result<(), CspSourceRejection>,
    ) -> Vec<String> {
        let mut errors = Vec::new();
        for directive in CspDirective::ALL {
            let sources = self.sources(directive);
            for source in sources {
                if let Err(rejection) = validate(directive, source) {
                    errors.push(format!(
                        "Invalid csp source \"{source}\" in {directive}: {rejection}"
                    ));
                }
            }
            if !is_strictly_ascending(sources) {
                errors.push(format!(
                    "csp {directive} must be sorted ascending without duplicates"
                ));
            }
        }
        errors
    }
}

fn is_strictly_ascending<T: Ord>(items: &[T]) -> bool {
    items.windows(2).all(|pair| pair[0] < pair[1])
}

fn into_result(errors: Vec<String>) -> Result<(), Vec<String>> {
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}

/// One purpose group of a contract's `csp` (§14.2): a publisher reason plus
/// static sources and/or network inputs.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WidgetCspPurpose {
    pub reason: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub connect_src: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub img_src: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub font_src: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub media_src: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub style_src: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub inputs: Vec<WidgetNetworkInput>,
}

/// A widget input whose string values the host turns into runtime sources.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WidgetNetworkInput {
    /// `root *( "." key / "[]" / ".*" )`
    pub path: String,
    pub directives: Vec<CspDirective>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub template: Option<WidgetUrlTemplate>,
}

/// Explicit subdomain expansion of `{s}` placeholders in runtime URLs.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WidgetUrlTemplate {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub subdomains: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subdomains_input: Option<String>,
}

impl WidgetUrlTemplate {
    pub fn is_empty(&self) -> bool {
        self.subdomains.is_empty() && self.subdomains_input.is_none()
    }

    fn problems(&self, inputs: &BTreeMap<String, ContractInput>) -> Vec<String> {
        let mut problems = Vec::new();
        if self.is_empty() {
            problems
                .push("template declares no subdomains or subdomainsInput; omit it".to_string());
        }
        if self.subdomains.len() > MAX_WIDGET_TEMPLATE_SUBDOMAINS {
            problems.push(format!(
                "template declares {} subdomains; at most {MAX_WIDGET_TEMPLATE_SUBDOMAINS} are allowed",
                self.subdomains.len()
            ));
        }
        for subdomain in &self.subdomains {
            if !is_dns_label(subdomain) {
                problems.push(format!(
                    "template subdomain \"{subdomain}\" is not a lowercase DNS label"
                ));
            }
        }
        if !is_strictly_ascending(&self.subdomains) {
            problems.push("template subdomains must be sorted ascending without duplicates".into());
        }
        if let Some(name) = &self.subdomains_input {
            if HOST_RESERVED_INPUT_KEYS.contains(&name.as_str()) {
                problems.push(format!(
                    "subdomainsInput \"{name}\" is reserved by the host"
                ));
            } else {
                match inputs.get(name).map(|input| input.input_type) {
                    Some(ContractInputType::String | ContractInputType::Json) => {}
                    Some(_) => problems.push(format!(
                        "subdomainsInput \"{name}\" must name a string or json input"
                    )),
                    None => problems.push(format!(
                        "subdomainsInput \"{name}\" is not a contract input"
                    )),
                }
            }
        }
        problems
    }
}

/// One step of a network input path after its root.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WidgetInputPathSegment<'a> {
    /// `.key`
    Key(&'a str),
    /// `[]`
    Items,
    /// `.*`
    Values,
}

/// Parses `root *( "." key / "[]" / ".*" )` with `key = [A-Za-z_][A-Za-z0-9_]*`.
pub fn parse_widget_input_path(path: &str) -> Option<(&str, Vec<WidgetInputPathSegment<'_>>)> {
    let root_len = member_key_len(path)?;
    let (root, mut rest) = path.split_at(root_len);
    let mut segments = Vec::new();
    while !rest.is_empty() {
        if let Some(tail) = rest.strip_prefix("[]") {
            segments.push(WidgetInputPathSegment::Items);
            rest = tail;
        } else if let Some(tail) = rest.strip_prefix(".*") {
            segments.push(WidgetInputPathSegment::Values);
            rest = tail;
        } else {
            let tail = rest.strip_prefix('.')?;
            let len = member_key_len(tail)?;
            segments.push(WidgetInputPathSegment::Key(&tail[..len]));
            rest = &tail[len..];
        }
    }
    Some((root, segments))
}

pub fn is_valid_widget_input_path(path: &str) -> bool {
    parse_widget_input_path(path)
        .is_some_and(|(_, segments)| segments.len() < MAX_WIDGET_INPUT_PATH_SEGMENTS)
}

fn member_key_len(text: &str) -> Option<usize> {
    let bytes = text.as_bytes();
    let first = *bytes.first()?;
    if !(first.is_ascii_alphabetic() || first == b'_') {
        return None;
    }
    Some(
        1 + bytes[1..]
            .iter()
            .take_while(|b| b.is_ascii_alphanumeric() || **b == b'_')
            .count(),
    )
}

impl WidgetNetworkInput {
    fn problems(&self, inputs: &BTreeMap<String, ContractInput>) -> Vec<String> {
        let mut problems = Vec::new();
        match parse_widget_input_path(&self.path) {
            None => problems.push(
                "path must match root *( \".\" key / \"[]\" / \".*\" ) with keys [A-Za-z_][A-Za-z0-9_]*"
                    .to_string(),
            ),
            Some((root, segments)) => {
                let count = segments.len() + 1;
                if count > MAX_WIDGET_INPUT_PATH_SEGMENTS {
                    problems.push(format!(
                        "path has {count} segments; at most {MAX_WIDGET_INPUT_PATH_SEGMENTS} are allowed"
                    ));
                }
                if HOST_RESERVED_INPUT_KEYS.contains(&root) {
                    problems.push(format!("root \"{root}\" is reserved by the host"));
                } else {
                    match inputs.get(root).map(|input| input.input_type) {
                        None => {
                            problems.push(format!("root \"{root}\" is not a contract input"))
                        }
                        Some(ContractInputType::String) if !segments.is_empty() => problems.push(
                            format!("root \"{root}\" is a string input and must be the whole path"),
                        ),
                        Some(ContractInputType::String | ContractInputType::Json) => {}
                        Some(_) => problems.push(format!(
                            "root \"{root}\" must be a string or json input; declare static sources for fixed choices"
                        )),
                    }
                }
            }
        }
        if self.directives.is_empty() {
            problems.push("declares no directives".into());
        }
        if !is_strictly_ascending(&self.directives) {
            problems.push(
                "directives must follow connectSrc, imgSrc, fontSrc, mediaSrc, styleSrc order without duplicates"
                    .into(),
            );
        }
        if let Some(template) = &self.template {
            problems.extend(template.problems(inputs));
        }
        problems
    }
}

impl WidgetCspPurpose {
    pub fn sources(&self, directive: CspDirective) -> &[String] {
        match directive {
            CspDirective::ConnectSrc => &self.connect_src,
            CspDirective::ImgSrc => &self.img_src,
            CspDirective::FontSrc => &self.font_src,
            CspDirective::MediaSrc => &self.media_src,
            CspDirective::StyleSrc => &self.style_src,
        }
    }

    pub fn sources_mut(&mut self, directive: CspDirective) -> &mut Vec<String> {
        match directive {
            CspDirective::ConnectSrc => &mut self.connect_src,
            CspDirective::ImgSrc => &mut self.img_src,
            CspDirective::FontSrc => &mut self.font_src,
            CspDirective::MediaSrc => &mut self.media_src,
            CspDirective::StyleSrc => &mut self.style_src,
        }
    }

    pub fn source_count(&self) -> usize {
        CspDirective::ALL
            .into_iter()
            .map(|directive| self.sources(directive).len())
            .sum()
    }

    pub fn entries(&self) -> impl Iterator<Item = (CspDirective, &str)> {
        CspDirective::ALL.into_iter().flat_map(move |directive| {
            self.sources(directive)
                .iter()
                .map(move |source| (directive, source.as_str()))
        })
    }

    /// Canonical form (§14.2.2): source lists sorted and deduplicated,
    /// directives in [`CspDirective::ALL`] order, template subdomains sorted
    /// and deduplicated, empty templates removed and inputs sorted by path.
    /// Group order and duplicate input paths are left for validation.
    pub fn canonicalize(&mut self) {
        for directive in CspDirective::ALL {
            let sources = self.sources_mut(directive);
            sources.sort();
            sources.dedup();
        }
        for input in &mut self.inputs {
            input.directives.sort();
            input.directives.dedup();
            if let Some(template) = &mut input.template {
                template.subdomains.sort();
                template.subdomains.dedup();
            }
            if input
                .template
                .as_ref()
                .is_some_and(WidgetUrlTemplate::is_empty)
            {
                input.template = None;
            }
        }
        self.inputs.sort_by(|a, b| a.path.cmp(&b.path));
    }

    fn problems(&self, inputs: &BTreeMap<String, ContractInput>) -> Vec<String> {
        let mut problems = Vec::new();
        if let Err(rejection) = validate_widget_csp_reason(&self.reason, !self.inputs.is_empty()) {
            problems.push(format!("{rejection} ({})", rejection.code()));
        }
        if self.source_count() == 0 && self.inputs.is_empty() {
            problems.push("declares no sources and no inputs".into());
        }
        for directive in CspDirective::ALL {
            let sources = self.sources(directive);
            for source in sources {
                if let Err(rejection) = validate_csp_source(directive, source) {
                    problems.push(format!(
                        "Invalid csp source \"{source}\" in {directive}: {rejection}"
                    ));
                }
            }
            if !is_strictly_ascending(sources) {
                problems.push(format!(
                    "{directive} must be sorted ascending without duplicates"
                ));
            }
        }
        if !self
            .inputs
            .windows(2)
            .all(|pair| pair[0].path < pair[1].path)
        {
            problems.push("inputs must be sorted by path without duplicates".into());
        }
        for input in &self.inputs {
            problems.extend(
                input
                    .problems(inputs)
                    .into_iter()
                    .map(|problem| format!("input \"{}\": {problem}", input.path)),
            );
        }
        problems
    }
}

/// Flattens purposes into the per-directive union, sorted and deduplicated.
pub fn flatten_csp_purposes(purposes: &[WidgetCspPurpose]) -> WidgetCsp {
    let mut csp = WidgetCsp::default();
    for (directive, source) in purposes.iter().flat_map(WidgetCspPurpose::entries) {
        csp.sources_mut(directive).push(source.to_string());
    }
    csp.canonicalize();
    csp
}

/// Every PSL-free rule of a contract's purpose groups (§14.2.3, §14.2.4
/// rules 1–7 and 9–12). Errors read `Widget '<id>': csp purpose <n>: …` with
/// zero-based purpose indexes.
pub(crate) fn validate_csp_purposes(
    widget_id: &str,
    purposes: &[WidgetCspPurpose],
    inputs: &BTreeMap<String, ContractInput>,
) -> Vec<String> {
    if purposes.is_empty() {
        return vec![format!(
            "Widget '{widget_id}' declares an empty csp; omit csp when it declares no purposes"
        )];
    }
    let mut errors = Vec::new();
    if purposes.len() > MAX_WIDGET_CSP_PURPOSES {
        errors.push(format!(
            "Widget '{widget_id}': csp declares {} purposes; at most {MAX_WIDGET_CSP_PURPOSES} are allowed",
            purposes.len()
        ));
    }
    let mut source_owners: BTreeMap<&str, usize> = BTreeMap::new();
    let mut input_owners: BTreeMap<&str, usize> = BTreeMap::new();
    let mut reason_owners: BTreeMap<String, usize> = BTreeMap::new();
    for (index, purpose) in purposes.iter().enumerate() {
        errors.extend(
            purpose
                .problems(inputs)
                .into_iter()
                .map(|problem| format!("Widget '{widget_id}': csp purpose {index}: {problem}")),
        );
        let own_sources: BTreeSet<&str> = purpose.entries().map(|(_, source)| source).collect();
        for source in own_sources {
            match source_owners.get(source) {
                Some(first) => errors.push(format!(
                    "Widget '{widget_id}': csp source \"{source}\" is declared in purposes {first} and {index}"
                )),
                None => {
                    source_owners.insert(source, index);
                }
            }
        }
        let own_inputs: BTreeSet<&str> = purpose
            .inputs
            .iter()
            .map(|input| input.path.as_str())
            .collect();
        for path in own_inputs {
            match input_owners.get(path) {
                Some(first) => errors.push(format!(
                    "Widget '{widget_id}': csp input \"{path}\" is declared in purposes {first} and {index}"
                )),
                None => {
                    input_owners.insert(path, index);
                }
            }
        }
        if !purpose.reason.is_empty() {
            let folded = fold_widget_csp_reason(&purpose.reason);
            match reason_owners.get(&folded) {
                Some(first) => errors.push(format!(
                    "Widget '{widget_id}': csp purposes {first} and {index}: {} ({})",
                    WidgetCspReasonRejection::Duplicate,
                    WidgetCspReasonRejection::Duplicate.code()
                )),
                None => {
                    reason_owners.insert(folded, index);
                }
            }
        }
    }
    let input_count: usize = purposes.iter().map(|purpose| purpose.inputs.len()).sum();
    if input_count > MAX_WIDGET_NETWORK_INPUTS {
        errors.push(format!(
            "Widget '{widget_id}': csp declares {input_count} network inputs; at most {MAX_WIDGET_NETWORK_INPUTS} are allowed"
        ));
    }
    let declared = flatten_csp_purposes(purposes);
    let count = declared.source_count();
    if count > MAX_WIDGET_CSP_SOURCES {
        errors.push(format!(
            "Widget '{widget_id}': csp declares {count} sources; at most {MAX_WIDGET_CSP_SOURCES} are allowed"
        ));
    }
    let bytes = declared.source_bytes();
    if bytes > MAX_WIDGET_CSP_SOURCE_BYTES {
        errors.push(format!(
            "Widget '{widget_id}': csp sources take {bytes} bytes; at most {MAX_WIDGET_CSP_SOURCE_BYTES} are allowed"
        ));
    }
    errors
}

/// Why a purpose reason was rejected (§14.2.4). Rule 8
/// (`reason-contains-address`) needs PSL data and lives in `widget_sources`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WidgetCspReasonRejection {
    Empty,
    NotNfc,
    ForbiddenCharacter,
    Whitespace,
    Length,
    TooFewLetters,
    MixedScript,
    MentionsProduct,
    ClaimsAssurance,
    ClaimsAttribution,
    Duplicate,
}

impl WidgetCspReasonRejection {
    pub fn code(self) -> &'static str {
        match self {
            WidgetCspReasonRejection::Empty => "reason-empty",
            WidgetCspReasonRejection::NotNfc => "reason-not-nfc",
            WidgetCspReasonRejection::ForbiddenCharacter => "reason-forbidden-character",
            WidgetCspReasonRejection::Whitespace => "reason-whitespace",
            WidgetCspReasonRejection::Length => "reason-length",
            WidgetCspReasonRejection::TooFewLetters => "reason-too-few-letters",
            WidgetCspReasonRejection::MixedScript => "reason-mixed-script",
            WidgetCspReasonRejection::MentionsProduct => "reason-mentions-product",
            WidgetCspReasonRejection::ClaimsAssurance => "reason-claims-assurance",
            WidgetCspReasonRejection::ClaimsAttribution => "reason-claims-attribution",
            WidgetCspReasonRejection::Duplicate => "reason-duplicate",
        }
    }

    fn message(self) -> &'static str {
        match self {
            WidgetCspReasonRejection::Empty => "reason is empty",
            WidgetCspReasonRejection::NotNfc => "reason must be NFC-normalized",
            WidgetCspReasonRejection::ForbiddenCharacter => {
                "reason may only contain letters, marks (at most 2 in a row), numbers, spaces and , . : ; ( ) - ' / &"
            }
            WidgetCspReasonRejection::Whitespace => {
                "reason must not start or end with a space or contain two spaces in a row"
            }
            WidgetCspReasonRejection::Length => "reason must be 8 to 120 characters long",
            WidgetCspReasonRejection::TooFewLetters => "reason must contain at least 3 letters",
            WidgetCspReasonRejection::MixedScript => {
                "reason mixes ASCII letters with letters of another script in one word"
            }
            WidgetCspReasonRejection::MentionsProduct => "reason must not mention Flow-Like",
            WidgetCspReasonRejection::ClaimsAssurance => {
                "reason must not claim that sources are verified, trusted, approved, official, certified, secure or safe"
            }
            WidgetCspReasonRejection::ClaimsAttribution => {
                "reason of a purpose with inputs must not claim who provides the addresses"
            }
            WidgetCspReasonRejection::Duplicate => "reasons of two purposes must differ",
        }
    }
}

impl fmt::Display for WidgetCspReasonRejection {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.message())
    }
}

impl std::error::Error for WidgetCspReasonRejection {}

/// Reason rules 1–7 and 9–11 of §14.2.4, in order; the first failure is the
/// code. Rule 12 compares purposes ([`fold_widget_csp_reason`]) and rule 8 is
/// PSL-bound, so neither runs here.
pub fn validate_widget_csp_reason(
    reason: &str,
    has_inputs: bool,
) -> Result<(), WidgetCspReasonRejection> {
    if reason.is_empty() {
        return Err(WidgetCspReasonRejection::Empty);
    }
    if !unicode_normalization::is_nfc(reason) {
        return Err(WidgetCspReasonRejection::NotNfc);
    }
    if !reason_characters_allowed(reason) {
        return Err(WidgetCspReasonRejection::ForbiddenCharacter);
    }
    if reason.starts_with(' ') || reason.ends_with(' ') || reason.contains("  ") {
        return Err(WidgetCspReasonRejection::Whitespace);
    }
    if !(REASON_MIN_CHARS..=REASON_MAX_CHARS).contains(&reason.chars().count()) {
        return Err(WidgetCspReasonRejection::Length);
    }
    if reason.chars().filter(|c| c.is_alphabetic()).count() < REASON_MIN_LETTERS {
        return Err(WidgetCspReasonRejection::TooFewLetters);
    }
    if has_mixed_script_word(reason) {
        return Err(WidgetCspReasonRejection::MixedScript);
    }
    let folded = fold_widget_csp_reason(reason);
    let letters_and_numbers: String = folded
        .chars()
        .filter(|c| c.is_alphabetic() || c.is_numeric())
        .collect();
    if letters_and_numbers.contains(REASON_PRODUCT) {
        return Err(WidgetCspReasonRejection::MentionsProduct);
    }
    if reason_words(&folded).any(|word| REASON_ASSURANCE_WORDS.contains(&word)) {
        return Err(WidgetCspReasonRejection::ClaimsAssurance);
    }
    if has_inputs && reason_words(&folded).any(|word| REASON_ATTRIBUTION_WORDS.contains(&word)) {
        return Err(WidgetCspReasonRejection::ClaimsAttribution);
    }
    Ok(())
}

/// `f` of §14.2.4: NFKC, U+3002 between ASCII alphanumerics replaced by `.`,
/// then lowercase.
pub fn fold_widget_csp_reason(reason: &str) -> String {
    let normalized: Vec<char> = reason.nfkc().collect();
    let replaced: String = normalized
        .iter()
        .enumerate()
        .map(|(index, &c)| {
            let between_alphanumerics = c == '\u{3002}'
                && index > 0
                && normalized[index - 1].is_ascii_alphanumeric()
                && normalized
                    .get(index + 1)
                    .is_some_and(char::is_ascii_alphanumeric);
            if between_alphanumerics { '.' } else { c }
        })
        .collect();
    replaced.to_lowercase()
}

fn reason_characters_allowed(reason: &str) -> bool {
    let chars: Vec<char> = reason.chars().collect();
    let mut mark_run = 0;
    for (index, &c) in chars.iter().enumerate() {
        if is_combining_mark(c) {
            mark_run += 1;
            if mark_run > REASON_MAX_MARK_RUN {
                return false;
            }
            continue;
        }
        mark_run = 0;
        let joiner = matches!(c, '\u{200C}' | '\u{200D}')
            && index > 0
            && is_joinable(chars[index - 1])
            && chars.get(index + 1).is_some_and(|next| is_joinable(*next));
        if !(c.is_alphabetic()
            || c.is_numeric()
            || c == ' '
            || REASON_PUNCTUATION.contains(&c)
            || joiner)
        {
            return false;
        }
    }
    true
}

fn is_joinable(c: char) -> bool {
    !c.is_ascii() && (c.is_alphabetic() || is_combining_mark(c))
}

fn has_mixed_script_word(reason: &str) -> bool {
    reason
        .split(|c: char| !(c.is_alphabetic() || is_combining_mark(c)))
        .any(|word| {
            word.chars().any(|c| c.is_ascii_alphabetic())
                && word.chars().any(|c| {
                    c.is_alphabetic()
                        && !c.is_ascii()
                        && !('\u{00C0}'..='\u{024F}').contains(&c)
                        && !('\u{1E00}'..='\u{1EFF}').contains(&c)
                })
        })
}

fn reason_words(folded: &str) -> impl Iterator<Item = &str> {
    folded
        .split(|c: char| !(c.is_alphabetic() || c.is_numeric() || is_combining_mark(c)))
        .filter(|word| !word.is_empty())
}

/// Effective widget policy: browser capabilities plus granted CSP sources.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WidgetPolicy {
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub workers: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub wasm: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub media: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub microphone: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub downloads: bool,
    #[serde(default, skip_serializing_if = "WidgetCsp::is_empty")]
    pub csp: WidgetCsp,
}

impl WidgetPolicy {
    /// Declared policy of a contract with every purpose flattened. Previews
    /// never get network sources, media or microphone.
    pub fn from_contract(contract: &WidgetContract, preview: bool) -> Self {
        let capabilities = contract.capabilities.clone().unwrap_or_default();
        let enabled = |flag: Option<bool>| flag == Some(true);
        Self {
            workers: enabled(capabilities.workers),
            wasm: enabled(capabilities.wasm),
            media: !preview && enabled(capabilities.media),
            microphone: !preview && enabled(capabilities.microphone),
            downloads: enabled(capabilities.downloads),
            csp: if preview {
                WidgetCsp::default()
            } else {
                contract.declared_csp()
            },
        }
    }

    pub fn is_empty(&self) -> bool {
        !self.workers
            && !self.wasm
            && !self.media
            && !self.microphone
            && !self.downloads
            && self.csp.is_empty()
    }

    /// Declared validation including deployment-specific reserved hosts (hub
    /// domain, request authority, frontend origins, profile hubs) and their
    /// wildcard coverage. Any failure makes the whole policy invalid.
    pub fn validate(&self, reserved_hosts: &[String]) -> Result<(), Vec<String>> {
        let mut errors = self.csp.validate_declared().err().unwrap_or_default();
        for (directive, source) in self.csp.entries() {
            if validate_csp_source(directive, source).is_err() {
                continue;
            }
            let Some(host) = csp_source_host(source)
                .filter(|host| is_reserved_source_host(host, reserved_hosts))
            else {
                continue;
            };
            if host.starts_with("*.") {
                errors.push(format!(
                    "csp source \"{source}\" in {directive} covers a reserved host"
                ));
            } else {
                errors.push(format!(
                    "csp source \"{source}\" in {directive} targets reserved host \"{host}\""
                ));
            }
        }
        into_result(errors)
    }

    /// Serialized policy with object keys sorted and no whitespace.
    pub fn canonical_json(&self) -> String {
        canonical_json(&serde_json::to_value(self).unwrap_or_default())
    }

    /// `sha256:<hex>` over the domain separator and the canonical JSON.
    pub fn digest(&self) -> String {
        domain_digest(WIDGET_POLICY_DIGEST_DOMAIN, &self.canonical_json())
    }
}

/// JSON with object keys sorted bytewise and no whitespace.
pub(crate) fn canonical_json(value: &serde_json::Value) -> String {
    let mut out = String::new();
    write_canonical_json(value, &mut out);
    out
}

fn write_canonical_json(value: &serde_json::Value, out: &mut String) {
    match value {
        serde_json::Value::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            out.push('{');
            for (index, key) in keys.into_iter().enumerate() {
                if index > 0 {
                    out.push(',');
                }
                out.push_str(&serde_json::Value::String(key.clone()).to_string());
                out.push(':');
                write_canonical_json(&map[key], out);
            }
            out.push('}');
        }
        serde_json::Value::Array(items) => {
            out.push('[');
            for (index, item) in items.iter().enumerate() {
                if index > 0 {
                    out.push(',');
                }
                write_canonical_json(item, out);
            }
            out.push(']');
        }
        scalar => out.push_str(&scalar.to_string()),
    }
}

fn domain_digest(domain: &str, body: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(domain.as_bytes());
    hasher.update(body.as_bytes());
    let hex: String = hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    format!("sha256:{hex}")
}

/// Canonical JSON of runtime slots: the digest body part and the stateless
/// URL carrier before encoding.
pub fn runtime_slots_canonical_json(slots: &BTreeMap<String, Vec<String>>) -> String {
    canonical_json(&serde_json::to_value(slots).unwrap_or_default())
}

/// Browser engine family of a widget document request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Engine {
    Chromium,
    #[serde(rename = "webkit")]
    WebKit,
    Gecko,
    Unknown,
}

/// Engine, platform (`ios`, `android`, `windows`, `macos`, `linux`,
/// `chromeos`) and `(major, minor)` version the gate rows match on.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct EngineId {
    pub engine: Engine,
    pub platform: Option<String>,
    pub version: Option<(u32, u32)>,
}

impl EngineId {
    pub fn unknown() -> Self {
        Self {
            engine: Engine::Unknown,
            platform: None,
            version: None,
        }
    }
}

/// Classifies a `User-Agent`: iOS is always WebKit (OS version); otherwise
/// `Chrome/` or `Chromium/` is Chromium, `Version/…Safari/` is WebKit and
/// `Firefox/` is Gecko. The UA is chosen by the viewer's browser, never by the
/// publisher, so a lying UA only affects its own viewer.
pub fn engine_from_user_agent(user_agent: &str) -> EngineId {
    let platform = user_agent_platform(user_agent);
    let (engine, version) = if platform == Some("ios") {
        (
            Engine::WebKit,
            ios_version(user_agent).or_else(|| version_after(user_agent, "Version/")),
        )
    } else if user_agent.contains("Chrome/") || user_agent.contains("Chromium/") {
        (
            Engine::Chromium,
            version_after(user_agent, "Chrome/").or_else(|| version_after(user_agent, "Chromium/")),
        )
    } else if user_agent.contains("Version/") && user_agent.contains("Safari/") {
        (Engine::WebKit, version_after(user_agent, "Version/"))
    } else if user_agent.contains("Firefox/") {
        (Engine::Gecko, version_after(user_agent, "Firefox/"))
    } else {
        (Engine::Unknown, None)
    };
    EngineId {
        engine,
        platform: platform.map(str::to_string),
        version: if engine == Engine::Unknown {
            None
        } else {
            version
        },
    }
}

fn user_agent_platform(user_agent: &str) -> Option<&'static str> {
    if ["iPhone", "iPad", "iPod"]
        .iter()
        .any(|marker| user_agent.contains(marker))
    {
        Some("ios")
    } else if user_agent.contains("Android") {
        Some("android")
    } else if user_agent.contains("Windows") {
        Some("windows")
    } else if user_agent.contains("CrOS") {
        Some("chromeos")
    } else if user_agent.contains("Macintosh") || user_agent.contains("Mac OS X") {
        Some("macos")
    } else if user_agent.contains("Linux") || user_agent.contains("X11") {
        Some("linux")
    } else {
        None
    }
}

fn version_after(user_agent: &str, marker: &str) -> Option<(u32, u32)> {
    let start = user_agent.find(marker)? + marker.len();
    parse_version(&user_agent[start..], '.')
}

fn ios_version(user_agent: &str) -> Option<(u32, u32)> {
    user_agent
        .match_indices(" OS ")
        .find_map(|(index, marker)| parse_version(&user_agent[index + marker.len()..], '_'))
}

fn parse_version(text: &str, separator: char) -> Option<(u32, u32)> {
    let digits = |text: &str| text.bytes().take_while(u8::is_ascii_digit).count();
    let major_len = digits(text);
    if !(1..=9).contains(&major_len) {
        return None;
    }
    let major = text[..major_len].parse().ok()?;
    let minor = text[major_len..]
        .strip_prefix(separator)
        .map(|rest| &rest[..digits(rest).min(9)])
        .filter(|minor| !minor.is_empty())
        .map_or(Some(0), |minor| minor.parse().ok())?;
    Some((major, minor))
}

/// A gateable engine feature (§14.4.9, §14.9).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum EngineFeature {
    RuntimeSources,
    WildcardSources,
    LocalMedia,
    DeclaredSources,
}

/// Features an engine may use. Every feature is a denylist: an unknown or
/// unmatched engine gets all of them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct EngineGate {
    pub runtime_sources: bool,
    pub wildcard_sources: bool,
    pub local_media: bool,
    pub declared_sources: bool,
}

impl EngineGate {
    pub const OPEN: EngineGate = EngineGate {
        runtime_sources: true,
        wildcard_sources: true,
        local_media: true,
        declared_sources: true,
    };

    const CLOSED: EngineGate = EngineGate {
        runtime_sources: false,
        wildcard_sources: false,
        local_media: false,
        declared_sources: false,
    };

    pub fn allows(&self, feature: EngineFeature) -> bool {
        match feature {
            EngineFeature::RuntimeSources => self.runtime_sources,
            EngineFeature::WildcardSources => self.wildcard_sources,
            EngineFeature::LocalMedia => self.local_media,
            EngineFeature::DeclaredSources => self.declared_sources,
        }
    }

    fn deny(&mut self, feature: EngineFeature) {
        match feature {
            EngineFeature::RuntimeSources => self.runtime_sources = false,
            EngineFeature::WildcardSources => self.wildcard_sources = false,
            EngineFeature::LocalMedia => self.local_media = false,
            EngineFeature::DeclaredSources => self.declared_sources = false,
        }
    }

    /// The descriptor's `engine` object.
    pub fn support(&self) -> WidgetEngineSupport {
        WidgetEngineSupport {
            wildcard_sources: self.wildcard_sources,
            runtime_sources: self.runtime_sources,
            local_media: self.local_media,
        }
    }
}

impl Default for EngineGate {
    fn default() -> Self {
        Self::OPEN
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct EngineGateRow {
    engine: Engine,
    #[serde(default)]
    platform: Option<String>,
    #[serde(default)]
    min_version: Option<(u32, u32)>,
    #[serde(default)]
    max_version: Option<(u32, u32)>,
    deny: Vec<EngineFeature>,
}

impl EngineGateRow {
    fn matches(&self, id: &EngineId) -> bool {
        self.engine == id.engine
            && self
                .platform
                .as_deref()
                .is_none_or(|platform| id.platform.as_deref() == Some(platform))
            && self
                .min_version
                .is_none_or(|min| id.version.is_some_and(|version| version >= min))
            && self
                .max_version
                .is_none_or(|max| id.version.is_some_and(|version| version <= max))
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct EngineGateTable {
    rows: Vec<EngineGateRow>,
}

const ENGINE_GATES_JSON: &str = include_str!("../data/widget_engine_gates.json");

static ENGINE_GATE_ROWS: LazyLock<Result<Vec<EngineGateRow>, String>> =
    LazyLock::new(|| parse_engine_gate_rows(ENGINE_GATES_JSON));

fn parse_engine_gate_rows(json: &str) -> Result<Vec<EngineGateRow>, String> {
    let table: EngineGateTable = serde_json::from_str(json)
        .map_err(|error| format!("widget_engine_gates.json is invalid: {error}"))?;
    for (index, row) in table.rows.iter().enumerate() {
        if row.engine == Engine::Unknown || row.deny.is_empty() {
            return Err(format!(
                "widget_engine_gates.json row {index} must name a known engine and deny at least one feature"
            ));
        }
        if let (Some(min), Some(max)) = (row.min_version, row.max_version)
            && min > max
        {
            return Err(format!(
                "widget_engine_gates.json row {index} has minVersion above maxVersion"
            ));
        }
    }
    Ok(table.rows)
}

fn gate_from_rows(rows: &[EngineGateRow], id: &EngineId) -> EngineGate {
    let mut gate = EngineGate::OPEN;
    for row in rows.iter().filter(|row| row.matches(id)) {
        for feature in &row.deny {
            gate.deny(*feature);
        }
    }
    gate
}

/// The gate of an engine from the embedded probe rows. Rows only record
/// failures, so an unmatched engine gets every feature; unreadable rows close
/// every feature.
pub fn widget_engine_gate(id: &EngineId) -> EngineGate {
    match ENGINE_GATE_ROWS.as_ref() {
        Ok(rows) => gate_from_rows(rows, id),
        Err(_) => EngineGate::CLOSED,
    }
}

/// Narrows an effective policy for an engine. It only ever removes sources:
/// runtime entries (not in `declared`), declared entries and wildcard sources
/// per the gate. Local media is dropped by the document CSP builder.
pub fn narrow_policy_for_engine(
    effective: &WidgetPolicy,
    declared: &WidgetPolicy,
    gate: &EngineGate,
) -> WidgetPolicy {
    let mut narrowed = effective.clone();
    for directive in CspDirective::ALL {
        let declared_sources = declared.csp.sources(directive);
        narrowed.csp.sources_mut(directive).retain(|source| {
            let is_declared = declared_sources.contains(source);
            (gate.declared_sources || !is_declared)
                && (gate.runtime_sources || is_declared)
                && (gate.wildcard_sources || !is_wildcard_source(source))
        });
    }
    narrowed
}

/// Runtime sources the host extracted for one declared network input.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WidgetRuntimeSourceRequest {
    pub slot: String,
    pub sources: Vec<String>,
}

impl WidgetRuntimeSourceRequest {
    /// Request entries of stored or carried slots, in slot order.
    pub fn from_slots(slots: &BTreeMap<String, Vec<String>>) -> Vec<Self> {
        slots
            .iter()
            .map(|(slot, sources)| Self {
                slot: slot.clone(),
                sources: sources.clone(),
            })
            .collect()
    }
}

/// Shape limits of a describe or mint request (§14.4.4). A failure is a
/// client bug: web answers 400 `INVALID_RUNTIME_SOURCES`, desktop
/// `invalid_runtime_sources: …`.
pub fn validate_runtime_request_shape(
    request: &[WidgetRuntimeSourceRequest],
) -> Result<(), String> {
    if request.len() > MAX_RUNTIME_REQUEST_SLOTS {
        return Err(format!(
            "{} runtime source slots; at most {MAX_RUNTIME_REQUEST_SLOTS} are allowed",
            request.len()
        ));
    }
    let mut slots = BTreeSet::new();
    for entry in request {
        if !slots.insert(entry.slot.as_str()) {
            return Err(format!(
                "runtime source slot \"{}\" appears twice",
                entry.slot
            ));
        }
    }
    let total: usize = request.iter().map(|entry| entry.sources.len()).sum();
    if total > MAX_RUNTIME_REQUEST_SOURCES {
        return Err(format!(
            "{total} runtime sources; at most {MAX_RUNTIME_REQUEST_SOURCES} are allowed"
        ));
    }
    if let Some(source) = request
        .iter()
        .flat_map(|entry| &entry.sources)
        .find(|source| source.len() > MAX_RUNTIME_REQUEST_SOURCE_BYTES)
    {
        return Err(format!(
            "a runtime source is {} bytes; at most {MAX_RUNTIME_REQUEST_SOURCE_BYTES} are allowed",
            source.len()
        ));
    }
    Ok(())
}

/// A path scope of this deployment's Flow-Like content storage. Runtime
/// sources may reach it only as `origin + path_prefix + appId + "/"`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PlatformStorageScope {
    /// `https://host`
    pub origin: String,
    /// `/…/` ending in a slash, e.g. `/apps/` or `/{bucket}/apps/`
    pub path_prefix: String,
}

impl PlatformStorageScope {
    /// Lowercase host of the origin.
    pub fn host(&self) -> Option<String> {
        reserved_host(&self.origin)
    }

    /// The app's path source, when the scope and app id are well-formed.
    pub fn app_source(&self, app_id: &str) -> Option<String> {
        if !is_valid_widget_app_id(app_id)
            || !self.path_prefix.starts_with('/')
            || !self.path_prefix.ends_with('/')
        {
            return None;
        }
        let source = format!("{}{}{app_id}/", self.origin, self.path_prefix);
        validate_path_source(CspDirective::ConnectSrc, &source)
            .is_ok()
            .then_some(source)
    }
}

/// App ids that may name a platform-storage scope: `[A-Za-z0-9_-]{1,64}`.
pub fn is_valid_widget_app_id(app_id: &str) -> bool {
    (1..=MAX_APP_ID_LEN).contains(&app_id.len())
        && app_id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-'))
}

/// Deployment and request facts runtime derivation depends on.
#[derive(Debug, Clone, Copy)]
pub struct WidgetRuntimeContext<'a> {
    pub reserved_hosts: &'a [String],
    pub platform_storage: &'a [PlatformStorageScope],
    pub app_id: Option<&'a str>,
    /// Serving prefixes the document CSP budget is measured with.
    pub bundle_sources: &'a [String],
    pub engine: EngineGate,
}

/// Accepted runtime sources of one grant. Its digest binds a web token to the
/// runtime component carried in the frame and document URL.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WidgetRuntimeSources {
    pub package_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    pub bundle_hash: String,
    pub widget_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub app_id: Option<String>,
    pub slots: BTreeMap<String, Vec<String>>,
}

impl WidgetRuntimeSources {
    pub fn canonical_json(&self) -> String {
        canonical_json(&serde_json::to_value(self).unwrap_or_default())
    }

    /// `sha256:<hex>` over [`WIDGET_RUNTIME_DIGEST_DOMAIN`] and the canonical JSON.
    pub fn digest(&self) -> String {
        domain_digest(WIDGET_RUNTIME_DIGEST_DOMAIN, &self.canonical_json())
    }

    pub fn request(&self) -> Vec<WidgetRuntimeSourceRequest> {
        WidgetRuntimeSourceRequest::from_slots(&self.slots)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "lowercase")]
pub enum WidgetPolicyStatus {
    Ok,
    Invalid,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "lowercase")]
pub enum WidgetRuntimeStatus {
    /// No runtime sources were requested.
    None,
    Ok,
    /// Runtime sources cannot be granted here (`invalidReason: "engine"`).
    Unavailable,
    /// The accepted set exceeds a cap; the policy stays declared-only.
    Invalid,
}

/// A requested runtime source the server refused. `code` is a
/// [`CspSourceRejection::code`] or `unknown-slot`, `platform-storage`,
/// `reserved-host`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WidgetRuntimeRejection {
    pub slot: String,
    pub source: String,
    pub code: String,
}

/// The descriptor's `runtime` object.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct WidgetRuntimeDescriptor {
    pub status: WidgetRuntimeStatus,
    /// Digest of the declared policy; equals `policyDigest` without runtime.
    pub declared_digest: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runtime_digest: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub rejected: Vec<WidgetRuntimeRejection>,
    /// `too-many-sources` or `too-large` (invalid), `engine` (unavailable)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub invalid_reason: Option<String>,
    /// Accepted runtime sources, present exactly when `runtimeDigest` is.
    /// Server-side only: mint encodes its slots into the URL carrier.
    #[serde(skip)]
    pub sources: Option<WidgetRuntimeSources>,
    /// Accepted (slot, directive, source) entries after fan-out.
    #[serde(skip)]
    pub entries: Vec<NetworkRuntimeInput>,
}

impl WidgetRuntimeDescriptor {
    fn declared_only(declared_digest: String) -> Self {
        Self {
            status: WidgetRuntimeStatus::None,
            declared_digest,
            runtime_digest: None,
            rejected: Vec::new(),
            invalid_reason: None,
            sources: None,
            entries: Vec::new(),
        }
    }
}

/// The descriptor's `engine` object.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct WidgetEngineSupport {
    pub wildcard_sources: bool,
    pub runtime_sources: bool,
    pub local_media: bool,
}

/// A declared network input as the host needs it for extraction.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WidgetNetworkInputSlot {
    pub path: String,
    /// Zero-based index of the purpose that declares the input.
    pub purpose: usize,
    pub directives: Vec<CspDirective>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub template: Option<WidgetUrlTemplate>,
}

/// Which widget a descriptor describes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WidgetPolicySubject {
    pub source: String,
    pub package_id: String,
    pub package_version: Option<String>,
    pub bundle_hash: String,
    pub widget_id: String,
    pub preview: bool,
}

/// Authoritative, server-derived policy of one widget. The host renders
/// consent from it and mints grants against its digest.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct WidgetPolicyDescriptor {
    /// `registry:<hub host>`, `local` or `hub`
    pub source: String,
    pub package_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub package_version: Option<String>,
    pub bundle_hash: String,
    pub widget_id: String,
    pub preview: bool,
    pub status: WidgetPolicyStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub invalid_reason: Option<String>,
    /// Effective policy (declared plus accepted runtime) after preview
    /// stripping; empty when `status` is `invalid`
    pub policy: WidgetPolicy,
    pub policy_digest: String,
    /// Declared network inputs; empty when invalid or preview
    #[serde(default)]
    pub network_inputs: Vec<WidgetNetworkInputSlot>,
    /// Omitted when invalid or preview
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub platform_storage: Option<Vec<PlatformStorageScope>>,
    /// Omitted when invalid or preview
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runtime: Option<WidgetRuntimeDescriptor>,
    /// Omitted when invalid or preview; set per request, never memoized
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub engine: Option<WidgetEngineSupport>,
    /// Display-only classification, filled by describe handlers
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub network: Option<WidgetNetwork>,
}

impl WidgetPolicyDescriptor {
    /// Declared-only descriptor without platform storage, bundle budget or
    /// engine facts. Describe handlers use [`Self::describe_with_runtime`].
    pub fn describe(
        subject: WidgetPolicySubject,
        contract: &WidgetContract,
        reserved_hosts: &[String],
    ) -> Self {
        let context = WidgetRuntimeContext {
            reserved_hosts,
            platform_storage: &[],
            app_id: None,
            bundle_sources: &[],
            engine: EngineGate::OPEN,
        };
        Self::describe_with_runtime(subject, contract, &[], &context)
    }

    /// Derives the descriptor for describe, mint and document serving
    /// (§14.4.5). An invalid contract, a widget id mismatch, a reserved host
    /// or a declared policy over the document CSP budget yields `invalid` with
    /// the baseline policy. Runtime sources are derived only for valid,
    /// non-preview descriptors.
    pub fn describe_with_runtime(
        subject: WidgetPolicySubject,
        contract: &WidgetContract,
        request: &[WidgetRuntimeSourceRequest],
        context: &WidgetRuntimeContext<'_>,
    ) -> Self {
        let declared = WidgetPolicy::from_contract(contract, subject.preview);
        let mut errors = contract.validate().err().unwrap_or_default();
        if contract.id != subject.widget_id {
            errors.push(format!(
                "Contract id '{}' does not match widget id '{}'",
                contract.id, subject.widget_id
            ));
        }
        if let Err(policy_errors) = declared.validate(context.reserved_hosts) {
            errors.extend(policy_errors);
        }
        if let Err(wildcard_errors) = crate::widget_sources::validate_wildcard_bases(
            declared.csp.entries().map(|(_, source)| source),
        ) {
            errors.extend(wildcard_errors);
        }
        if errors.is_empty() {
            let bytes = widget_document_csp(context.bundle_sources, &declared, true, true).len();
            if bytes > MAX_WIDGET_DOCUMENT_CSP_BYTES {
                errors.push(format!(
                    "{DECLARED_CSP_TOO_LARGE}: the document CSP would take {bytes} bytes; at most {MAX_WIDGET_DOCUMENT_CSP_BYTES} are allowed"
                ));
            }
        }
        if !errors.is_empty() {
            return Self::invalid(subject, errors.join("; "));
        }
        let preview = subject.preview;
        let mut descriptor = Self::with_policy(subject, WidgetPolicyStatus::Ok, None, declared);
        if preview {
            return descriptor;
        }
        descriptor.network_inputs = network_input_slots(contract);
        descriptor.platform_storage = Some(context.platform_storage.to_vec());
        descriptor.engine = Some(context.engine.support());
        let (runtime, effective) = derive_runtime(&descriptor, request, context);
        if let Some(effective) = effective {
            descriptor.policy_digest = effective.digest();
            descriptor.policy = effective;
        }
        descriptor.runtime = Some(runtime);
        descriptor
    }

    /// An `invalid` descriptor at the baseline policy.
    pub fn invalid(subject: WidgetPolicySubject, reason: String) -> Self {
        Self::with_policy(
            subject,
            WidgetPolicyStatus::Invalid,
            Some(reason),
            WidgetPolicy::default(),
        )
    }

    fn with_policy(
        subject: WidgetPolicySubject,
        status: WidgetPolicyStatus,
        invalid_reason: Option<String>,
        policy: WidgetPolicy,
    ) -> Self {
        Self {
            source: subject.source,
            package_id: subject.package_id,
            package_version: subject.package_version,
            bundle_hash: subject.bundle_hash,
            widget_id: subject.widget_id,
            preview: subject.preview,
            status,
            invalid_reason,
            policy_digest: policy.digest(),
            policy,
            network_inputs: Vec::new(),
            platform_storage: None,
            runtime: None,
            engine: None,
            network: None,
        }
    }

    pub fn is_ok(&self) -> bool {
        self.status == WidgetPolicyStatus::Ok
    }

    /// Digest of the declared part: `runtime.declaredDigest`, else `policyDigest`.
    pub fn declared_digest(&self) -> &str {
        self.runtime
            .as_ref()
            .map_or(&self.policy_digest, |runtime| &runtime.declared_digest)
    }

    /// Accepted runtime sources, when the descriptor carries any.
    pub fn runtime_sources(&self) -> Option<&WidgetRuntimeSources> {
        self.runtime
            .as_ref()
            .and_then(|runtime| runtime.sources.as_ref())
    }

    /// Inputs of `widget_sources::describe_network`: every purpose of the
    /// contract with its declared entries and input paths, and every accepted
    /// runtime entry. Both are empty for invalid and preview descriptors.
    pub fn classifier_inputs(
        &self,
        contract: &WidgetContract,
    ) -> (Vec<NetworkPurposeInput>, Vec<NetworkRuntimeInput>) {
        if !self.is_ok() || self.preview {
            return (Vec::new(), Vec::new());
        }
        let purposes = contract
            .csp
            .iter()
            .flatten()
            .map(|purpose| NetworkPurposeInput {
                reason: purpose.reason.clone(),
                declared: purpose
                    .entries()
                    .map(|(directive, source)| (directive, source.to_string()))
                    .collect(),
                inputs: purpose
                    .inputs
                    .iter()
                    .map(|input| input.path.clone())
                    .collect(),
            })
            .collect();
        let runtime = self
            .runtime
            .as_ref()
            .map(|runtime| runtime.entries.clone())
            .unwrap_or_default();
        (purposes, runtime)
    }
}

fn network_input_slots(contract: &WidgetContract) -> Vec<WidgetNetworkInputSlot> {
    contract
        .csp
        .iter()
        .flatten()
        .enumerate()
        .flat_map(|(purpose, group)| {
            group
                .inputs
                .iter()
                .map(move |input| WidgetNetworkInputSlot {
                    path: input.path.clone(),
                    purpose,
                    directives: input.directives.clone(),
                    template: input.template.clone(),
                })
        })
        .collect()
}

fn derive_runtime(
    declared: &WidgetPolicyDescriptor,
    request: &[WidgetRuntimeSourceRequest],
    context: &WidgetRuntimeContext<'_>,
) -> (WidgetRuntimeDescriptor, Option<WidgetPolicy>) {
    let mut runtime = WidgetRuntimeDescriptor::declared_only(declared.policy_digest.clone());
    let mut requested: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
    for entry in request {
        requested
            .entry(entry.slot.as_str())
            .or_default()
            .extend(entry.sources.iter().map(String::as_str));
    }
    if requested.values().all(BTreeSet::is_empty) {
        return (runtime, None);
    }
    if !context.engine.runtime_sources {
        runtime.status = WidgetRuntimeStatus::Unavailable;
        runtime.invalid_reason = Some(RUNTIME_UNAVAILABLE_ENGINE.into());
        return (runtime, None);
    }
    runtime.status = WidgetRuntimeStatus::Ok;
    let slots: BTreeMap<&str, &WidgetNetworkInputSlot> = declared
        .network_inputs
        .iter()
        .map(|slot| (slot.path.as_str(), slot))
        .collect();
    let mut accepted: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut additions = WidgetCsp::default();
    let mut entries = Vec::new();
    for (slot, sources) in requested {
        for source in sources {
            let directives = slots
                .get(slot)
                .ok_or(RUNTIME_REJECTION_UNKNOWN_SLOT)
                .and_then(|input| {
                    accept_runtime_source(input, source, &declared.policy.csp, context)
                });
            match directives {
                Ok(directives) if directives.is_empty() => {}
                Ok(directives) => {
                    accepted
                        .entry(slot.to_string())
                        .or_default()
                        .push(source.to_string());
                    for directive in directives {
                        additions.sources_mut(directive).push(source.to_string());
                        entries.push(NetworkRuntimeInput {
                            purpose: slots[slot].purpose,
                            slot: slot.to_string(),
                            directive,
                            source: source.to_string(),
                        });
                    }
                }
                Err(code) => runtime.rejected.push(WidgetRuntimeRejection {
                    slot: slot.to_string(),
                    source: source.to_string(),
                    code: code.to_string(),
                }),
            }
        }
    }
    if accepted.is_empty() {
        return (runtime, None);
    }
    let distinct: BTreeSet<&String> = accepted.values().flatten().collect();
    if distinct.len() > MAX_WIDGET_RUNTIME_SOURCES {
        runtime.status = WidgetRuntimeStatus::Invalid;
        runtime.invalid_reason = Some(RUNTIME_INVALID_TOO_MANY_SOURCES.into());
        return (runtime, None);
    }
    additions.canonicalize();
    let mut effective = declared.policy.clone();
    for (directive, source) in additions.entries() {
        effective
            .csp
            .sources_mut(directive)
            .push(source.to_string());
    }
    effective.csp.canonicalize();
    let too_large = additions.source_count() > MAX_WIDGET_RUNTIME_ENTRIES
        || runtime_slots_canonical_json(&accepted).len() > MAX_WIDGET_RUNTIME_BYTES
        || effective.csp.source_count() > MAX_WIDGET_EFFECTIVE_CSP_SOURCES
        || widget_document_csp(context.bundle_sources, &effective, true, true).len()
            > MAX_WIDGET_DOCUMENT_CSP_BYTES;
    if too_large {
        runtime.status = WidgetRuntimeStatus::Invalid;
        runtime.invalid_reason = Some(RUNTIME_INVALID_TOO_LARGE.into());
        return (runtime, None);
    }
    let record = WidgetRuntimeSources {
        package_id: declared.package_id.clone(),
        version: declared.package_version.clone(),
        bundle_hash: declared.bundle_hash.clone(),
        widget_id: declared.widget_id.clone(),
        app_id: context.app_id.map(str::to_string),
        slots: accepted,
    };
    runtime.runtime_digest = Some(record.digest());
    runtime.sources = Some(record);
    runtime.entries = entries;
    (runtime, Some(effective))
}

/// Steps 4b–4d of §14.4.5 for one source of a declared slot: the directives
/// it adds. An empty list means every fitting directive already declares it.
fn accept_runtime_source(
    input: &WidgetNetworkInputSlot,
    source: &str,
    declared: &WidgetCsp,
    context: &WidgetRuntimeContext<'_>,
) -> Result<Vec<CspDirective>, &'static str> {
    let host = runtime_source_host(source, context)?;
    let scheme = source.split_once("://").map_or("", |(scheme, _)| scheme);
    let mut directives = Vec::new();
    let mut already_declared = false;
    for directive in input.directives.iter().copied() {
        if !directive.allowed_schemes().contains(&scheme) {
            continue;
        }
        if declared
            .sources(directive)
            .iter()
            .any(|held| held == source)
        {
            already_declared = true;
        } else {
            directives.push(directive);
        }
    }
    if directives.is_empty() {
        return if already_declared {
            Ok(directives)
        } else {
            Err(CspSourceRejection::SchemeNotAllowed.code())
        };
    }
    if is_reserved_host(host, context.reserved_hosts) {
        return Err(RUNTIME_REJECTION_RESERVED_HOST);
    }
    Ok(directives)
}

fn runtime_source_host<'s>(
    source: &'s str,
    context: &WidgetRuntimeContext<'_>,
) -> Result<&'s str, &'static str> {
    if source.contains('*') {
        return Err(CspSourceRejection::Wildcard.code());
    }
    let host = csp_source_host(source).unwrap_or_default();
    if is_path_source(source) {
        let in_scope = context.app_id.is_some_and(|app_id| {
            context
                .platform_storage
                .iter()
                .any(|scope| scope.app_source(app_id).as_deref() == Some(source))
        });
        return if in_scope {
            Ok(host)
        } else {
            Err(RUNTIME_REJECTION_PLATFORM_STORAGE)
        };
    }
    validate_csp_source(CspDirective::ConnectSrc, source).map_err(CspSourceRejection::code)?;
    if context
        .platform_storage
        .iter()
        .any(|scope| scope.host().as_deref() == Some(host))
    {
        return Err(RUNTIME_REJECTION_PLATFORM_STORAGE);
    }
    Ok(host)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::widget::WidgetCapabilities;
    use crate::widget_frame::{decode_runtime_component, encode_runtime_component};
    use serde_json::{Value, json};

    const FIXTURE: &str = include_str!("../tests/fixtures/widget_csp.json");
    const EXTRACTION_FIXTURE: &str =
        include_str!("../tests/fixtures/widget_runtime_extraction.json");

    fn fixture() -> Value {
        serde_json::from_str(FIXTURE).expect("widget_csp.json must parse")
    }

    fn extraction_fixture() -> Value {
        serde_json::from_str(EXTRACTION_FIXTURE).expect("widget_runtime_extraction.json must parse")
    }

    fn directive_of(case: &Value) -> CspDirective {
        let key = case["directive"].as_str().unwrap();
        CspDirective::from_contract_key(key).unwrap_or_else(|| panic!("unknown directive {key}"))
    }

    fn strings(items: &[&str]) -> Vec<String> {
        items.iter().map(|item| item.to_string()).collect()
    }

    fn csp(connect: &[&str], img: &[&str]) -> WidgetCsp {
        WidgetCsp {
            connect_src: strings(connect),
            img_src: strings(img),
            ..WidgetCsp::default()
        }
    }

    fn purpose(reason: &str) -> WidgetCspPurpose {
        WidgetCspPurpose {
            reason: reason.into(),
            ..WidgetCspPurpose::default()
        }
    }

    fn sourced_purpose(reason: &str, connect: &[&str], img: &[&str]) -> WidgetCspPurpose {
        WidgetCspPurpose {
            connect_src: strings(connect),
            img_src: strings(img),
            ..purpose(reason)
        }
    }

    fn input(input_type: ContractInputType) -> ContractInput {
        ContractInput {
            input_type,
            description: None,
            default: None,
            choices: None,
            min: None,
            max: None,
            schema: None,
            optional: true,
        }
    }

    fn network_input(path: &str, directives: &[CspDirective]) -> WidgetNetworkInput {
        WidgetNetworkInput {
            path: path.into(),
            directives: directives.to_vec(),
            template: None,
        }
    }

    fn contract_with(
        capabilities: Option<WidgetCapabilities>,
        purposes: Vec<WidgetCspPurpose>,
    ) -> WidgetContract {
        let mut contract = WidgetContract::new("live-map").with_csp(purposes);
        contract.capabilities = capabilities;
        contract
    }

    fn full_capabilities() -> WidgetCapabilities {
        WidgetCapabilities {
            workers: Some(true),
            media: Some(true),
            microphone: Some(true),
            wasm: Some(true),
            downloads: Some(true),
        }
    }

    fn subject(preview: bool) -> WidgetPolicySubject {
        WidgetPolicySubject {
            source: registry_policy_source("hub.flow-like.com"),
            package_id: "com.example.maps".into(),
            package_version: None,
            bundle_hash: "a".repeat(64),
            widget_id: "live-map".into(),
            preview,
        }
    }

    fn web_subject() -> WidgetPolicySubject {
        WidgetPolicySubject {
            source: WIDGET_POLICY_SOURCE_HUB.into(),
            package_version: Some("1.4.0".into()),
            ..subject(false)
        }
    }

    const APP_STORAGE: &str = "https://s3.eu-central-1.amazonaws.com/flow-like-content/apps/app_1/";

    fn scopes() -> Vec<PlatformStorageScope> {
        vec![PlatformStorageScope {
            origin: "https://s3.eu-central-1.amazonaws.com".into(),
            path_prefix: "/flow-like-content/apps/".into(),
        }]
    }

    fn context<'a>(
        reserved_hosts: &'a [String],
        platform_storage: &'a [PlatformStorageScope],
        app_id: Option<&'a str>,
        bundle_sources: &'a [String],
    ) -> WidgetRuntimeContext<'a> {
        WidgetRuntimeContext {
            reserved_hosts,
            platform_storage,
            app_id,
            bundle_sources,
            engine: EngineGate::OPEN,
        }
    }

    fn request(entries: &[(&str, &[&str])]) -> Vec<WidgetRuntimeSourceRequest> {
        entries
            .iter()
            .map(|(slot, sources)| WidgetRuntimeSourceRequest {
                slot: slot.to_string(),
                sources: strings(sources),
            })
            .collect()
    }

    fn runtime_contract() -> WidgetContract {
        let mut contract = WidgetContract::new("live-map");
        for (name, input_type) in [
            ("apiUrl", ContractInputType::String),
            ("layers", ContractInputType::Json),
            ("tileSubdomains", ContractInputType::Json),
            ("tileUrl", ContractInputType::String),
            ("videoUrl", ContractInputType::String),
        ] {
            contract.inputs.insert(name.into(), input(input_type));
        }
        let mut runtime = purpose("Loads map tiles from tile servers given to it at runtime");
        runtime.inputs = vec![
            network_input(
                "videoUrl",
                &[CspDirective::ConnectSrc, CspDirective::MediaSrc],
            ),
            WidgetNetworkInput {
                template: Some(WidgetUrlTemplate {
                    subdomains_input: Some("tileSubdomains".into()),
                    ..WidgetUrlTemplate::default()
                }),
                ..network_input("tileUrl", &[CspDirective::ConnectSrc, CspDirective::ImgSrc])
            },
            network_input("apiUrl", &[CspDirective::ConnectSrc]),
            network_input("layers[].url", &[CspDirective::ImgSrc]),
        ];
        let mut contract = contract.with_csp(vec![
            sourced_purpose(
                "Loads globe terrain and imagery from Cesium ion",
                &["https://api.cesium.com"],
                &[],
            ),
            runtime,
        ]);
        contract.capabilities = Some(WidgetCapabilities {
            workers: Some(true),
            ..WidgetCapabilities::default()
        });
        contract
    }

    fn rejections(descriptor: &WidgetPolicyDescriptor) -> Vec<(String, String, String)> {
        descriptor
            .runtime
            .as_ref()
            .unwrap()
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

    #[test]
    fn fixture_accepted_sources_pass_the_grammar() {
        let fixture = fixture();
        let cases = fixture["acceptedSources"].as_array().unwrap();
        assert!(cases.len() >= 10);
        for case in cases {
            let source = case["source"].as_str().unwrap();
            assert_eq!(
                validate_csp_source(directive_of(case), source),
                Ok(()),
                "must accept {source:?}"
            );
        }
    }

    #[test]
    fn fixture_rejected_sources_fail_with_the_listed_reason() {
        let fixture = fixture();
        let cases = fixture["rejectedSources"].as_array().unwrap();
        assert!(cases.len() >= 40);
        for case in cases {
            let source = case["source"].as_str().unwrap();
            let reason = case["reason"].as_str().unwrap();
            let rejection = validate_csp_source(directive_of(case), source)
                .expect_err(&format!("must reject {source:?}"));
            assert_eq!(rejection.code(), reason, "reason for {source:?}");
        }
    }

    #[test]
    fn fixture_digests_match() {
        let fixture = fixture();
        let cases = fixture["digest"].as_array().unwrap();
        assert!(cases.len() >= 4);
        for case in cases {
            let policy: WidgetPolicy = serde_json::from_value(case["policy"].clone()).unwrap();
            assert_eq!(policy.digest(), case["digest"].as_str().unwrap());
        }
    }

    #[test]
    fn every_directive_maps_both_ways() {
        for directive in CspDirective::ALL {
            assert_eq!(
                CspDirective::from_contract_key(directive.contract_key()),
                Some(directive)
            );
            assert!(directive.csp_name().ends_with("-src"));
            assert_eq!(
                serde_json::to_value(directive).unwrap(),
                json!(directive.contract_key())
            );
        }
        assert_eq!(CspDirective::from_contract_key("scriptSrc"), None);
        assert_eq!(CspDirective::from_contract_key("frameSrc"), None);
        assert!(serde_json::from_value::<CspDirective>(json!("frameSrc")).is_err());
    }

    #[test]
    fn csp_rejects_unknown_directives_and_serializes_canonically() {
        for key in [
            "scriptSrc",
            "frameSrc",
            "workerSrc",
            "defaultSrc",
            "connect_src",
        ] {
            let value = json!({ key: ["https://api.example.org"] });
            assert!(
                serde_json::from_value::<WidgetCsp>(value).is_err(),
                "{key} must not parse"
            );
        }
        let parsed: WidgetCsp = serde_json::from_value(json!({
            "connectSrc": ["https://api.example.org"],
            "imgSrc": []
        }))
        .unwrap();
        assert_eq!(
            serde_json::to_value(&parsed).unwrap(),
            json!({ "connectSrc": ["https://api.example.org"] })
        );
        assert_eq!(
            serde_json::to_value(WidgetCsp::default()).unwrap(),
            json!({})
        );
    }

    #[test]
    fn declared_validation_enforces_grammar_order_count_and_bytes() {
        assert!(
            csp(&["https://*.example.org", "https://a.example.org"], &[])
                .validate_declared()
                .is_ok()
        );

        let unsorted = csp(&["https://b.example.org", "https://a.example.org"], &[]);
        assert!(
            unsorted.validate_declared().unwrap_err()[0]
                .contains("sorted ascending without duplicates")
        );
        let duplicated = csp(&["https://a.example.org", "https://a.example.org"], &[]);
        assert!(duplicated.validate_declared().is_err());

        let sixteen: Vec<String> = (0..16)
            .map(|i| format!("https://h{i:02}.example.org"))
            .collect();
        let mut at_cap = WidgetCsp {
            connect_src: sixteen[..8].to_vec(),
            img_src: sixteen[8..].to_vec(),
            ..WidgetCsp::default()
        };
        assert!(at_cap.validate_declared().is_ok());
        at_cap.font_src.push("https://fonts.example.org".into());
        assert!(
            at_cap
                .validate_declared()
                .unwrap_err()
                .iter()
                .any(|e| e.contains("at most 16"))
        );

        let long: Vec<String> = (0..15)
            .map(|i| {
                format!(
                    "https://h{i:02}-{}.{}.example.org",
                    "a".repeat(59),
                    "b".repeat(20)
                )
            })
            .collect();
        let heavy = WidgetCsp {
            connect_src: long,
            ..WidgetCsp::default()
        };
        assert!(heavy.source_bytes() > MAX_WIDGET_CSP_SOURCE_BYTES);
        assert!(
            heavy
                .validate_declared()
                .unwrap_err()
                .iter()
                .any(|e| e.contains("at most 1536"))
        );

        let bad = csp(&["http://api.example.org"], &["wss://tiles.example.org"]);
        let errors = bad.validate_declared().unwrap_err();
        assert_eq!(errors.len(), 2);
        assert!(errors[0].contains("connectSrc"));
        assert!(errors[1].contains("imgSrc"));

        assert!(csp(&[APP_STORAGE], &[]).validate_declared().is_err());
    }

    #[test]
    fn effective_validation_accepts_wildcards_and_platform_storage_paths() {
        let effective = csp(
            &[
                "https://*.s3.eu-central-1.amazonaws.com",
                APP_STORAGE,
                "wss://live.example.com",
            ],
            &[APP_STORAGE],
        );
        assert_eq!(effective.validate_effective(), Ok(()));

        for (source, code) in [
            ("https://s3.example.com/apps", "path"),
            ("https://s3.example.com/", "path"),
            ("https://s3.example.com//", "path"),
            ("https://s3.example.com/apps//x/", "path"),
            ("https://s3.example.com/apps/../x/", "path"),
            ("https://s3.example.com/apps/./", "path"),
            ("https://s3.example.com/apps/a b/", "path"),
            ("https://s3.example.com/apps/a;b/", "path"),
            ("https://a.com x/apps/", "forbidden-character"),
            ("https://s3.example.com/apps/%2e%2e/", "path"),
            ("https://s3.example.com/apps/?x=/", "path"),
            ("https://*.example.com/apps/", "wildcard"),
            ("https://s3.example.com:9000/apps/", "port"),
            ("http://s3.example.com/apps/", "scheme-not-allowed"),
            ("https://127.0.0.1/apps/", "ip-literal"),
            ("https://minio.localhost/apps/", "reserved-name"),
        ] {
            let result = if is_path_source(source) {
                validate_path_source(CspDirective::ConnectSrc, source)
            } else {
                validate_csp_source(CspDirective::ConnectSrc, source)
            };
            assert_eq!(
                result.map_err(CspSourceRejection::code),
                Err(code),
                "{source}"
            );
            assert!(
                csp(&[source], &[]).validate_effective().is_err(),
                "{source}"
            );
        }
        assert_eq!(
            validate_path_source(
                CspDirective::ImgSrc,
                "https://s3.example.com/Bucket_1/apps/A-b.c/"
            ),
            Ok(())
        );
        assert_eq!(
            validate_path_source(CspDirective::ImgSrc, "https://s3.example.com"),
            Err(CspSourceRejection::Path)
        );

        let many: Vec<String> = (0..33)
            .map(|i| format!("https://h{i:02}.example.org"))
            .collect();
        let at_cap = WidgetCsp {
            connect_src: many[..32].to_vec(),
            ..WidgetCsp::default()
        };
        assert!(at_cap.validate_effective().is_ok());
        let over = WidgetCsp {
            connect_src: many,
            ..WidgetCsp::default()
        };
        assert!(
            over.validate_effective()
                .unwrap_err()
                .iter()
                .any(|e| e.contains("at most 32"))
        );
        assert!(
            csp(&["https://b.example.org", "https://a.example.org"], &[])
                .validate_effective()
                .is_err()
        );
    }

    #[test]
    fn source_helpers_understand_wildcards_and_paths() {
        assert_eq!(
            csp_source_host("https://*.s3.amazonaws.com"),
            Some("*.s3.amazonaws.com")
        );
        assert_eq!(
            csp_source_host(APP_STORAGE),
            Some("s3.eu-central-1.amazonaws.com")
        );
        assert!(is_wildcard_source("wss://*.iot.example.com"));
        assert!(!is_wildcard_source("https://api.example.com"));
        assert!(is_path_source(APP_STORAGE));
        assert!(!is_path_source("https://api.example.com"));

        let base_251 = format!(
            "{}.{}.{}.{}.com",
            "a".repeat(63),
            "a".repeat(63),
            "a".repeat(63),
            "a".repeat(55)
        );
        assert_eq!(base_251.len(), 251);
        assert_eq!(
            validate_csp_source(CspDirective::ConnectSrc, &format!("https://*.{base_251}")),
            Ok(())
        );
        assert_eq!(
            validate_csp_source(CspDirective::ConnectSrc, &format!("https://*.a{base_251}")),
            Err(CspSourceRejection::InvalidHost)
        );
    }

    #[test]
    fn reserved_hosts_match_equal_and_subdomains_in_any_entry_form() {
        let reserved = vec![
            "flow-like.com".to_string(),
            "https://App.Example-Frontend.io:8443/path".to_string(),
            "api.internal-hub.net:443".to_string(),
            "user@[::1]:3000".to_string(),
            "".to_string(),
        ];
        assert!(is_reserved_host("flow-like.com", &reserved));
        assert!(is_reserved_host("api.flow-like.com", &reserved));
        assert!(is_reserved_host("deep.api.flow-like.com", &reserved));
        assert!(is_reserved_host("app.example-frontend.io", &reserved));
        assert!(is_reserved_host("cdn.app.example-frontend.io", &reserved));
        assert!(is_reserved_host("api.internal-hub.net", &reserved));
        assert!(!is_reserved_host("evilflow-like.com", &reserved));
        assert!(!is_reserved_host("flow-like.com.evil.org", &reserved));
        assert!(!is_reserved_host("example-frontend.io", &reserved));
        assert!(!is_reserved_host("internal-hub.net", &reserved));

        assert_eq!(
            reserved_host("HTTPS://Hub.Flow-Like.com./x?y#z"),
            Some("hub.flow-like.com".into())
        );
        assert_eq!(reserved_host("localhost:3000"), Some("localhost".into()));
        assert_eq!(reserved_host("[::1]:8080"), Some("::1".into()));
        assert_eq!(reserved_host("  "), None);
    }

    #[test]
    fn wildcards_cover_reserved_hosts_in_both_directions() {
        let reserved = vec![
            "flow-like.com".to_string(),
            "api.internal-hub.net".to_string(),
        ];
        for covered in [
            "*.flow-like.com",
            "*.eu.flow-like.com",
            "*.internal-hub.net",
            "*.api.internal-hub.net",
            "*.Flow-Like.com",
        ] {
            assert!(is_reserved_source_host(covered, &reserved), "{covered}");
        }
        for free in [
            "*.evilflow-like.com",
            "*.hub.net",
            "*.flow-like.com.evil.org",
            "*.example.com",
        ] {
            assert!(!is_reserved_source_host(free, &reserved), "{free}");
        }
        assert!(is_reserved_source_host("api.flow-like.com", &reserved));
        assert!(!is_reserved_source_host("internal-hub.net", &reserved));

        let policy = WidgetPolicy {
            csp: csp(
                &["https://*.internal-hub.net"],
                &["https://*.maps.example.com"],
            ),
            ..WidgetPolicy::default()
        };
        assert!(policy.validate(&[]).is_ok());
        let errors = policy.validate(&reserved).unwrap_err();
        assert_eq!(errors.len(), 1);
        assert!(
            errors[0]
                .contains("\"https://*.internal-hub.net\" in connectSrc covers a reserved host")
        );
    }

    #[test]
    fn policy_validation_rejects_reserved_hosts_and_grammar_failures() {
        let policy = WidgetPolicy {
            csp: csp(
                &["https://api.flow-like.com", "https://api.maptiler.com"],
                &["https://tiles.flow-like.com"],
            ),
            ..WidgetPolicy::default()
        };
        assert!(policy.validate(&[]).is_ok());
        let errors = policy.validate(&["flow-like.com".into()]).unwrap_err();
        assert_eq!(errors.len(), 2);
        assert!(errors[0].contains("reserved host \"api.flow-like.com\""));
        assert!(errors[1].contains("imgSrc"));

        let broken = WidgetPolicy {
            csp: csp(&["https://a.*.flow-like.com"], &[]),
            ..WidgetPolicy::default()
        };
        let errors = broken.validate(&["flow-like.com".into()]).unwrap_err();
        assert_eq!(errors.len(), 1);
        assert!(errors[0].contains("wildcards are only allowed"));
    }

    #[test]
    fn fixture_reasons_follow_the_rules_in_order() {
        let fixture = fixture();
        for case in fixture["acceptedReasons"].as_array().unwrap() {
            let reason = case["reason"].as_str().unwrap();
            let has_inputs = case["hasInputs"].as_bool().unwrap();
            assert_eq!(
                validate_widget_csp_reason(reason, has_inputs),
                Ok(()),
                "must accept {reason:?}"
            );
        }
        let rejected = fixture["rejectedReasons"].as_array().unwrap();
        assert!(rejected.len() >= 30);
        for case in rejected {
            let reason = case["reason"].as_str().unwrap();
            let has_inputs = case["hasInputs"].as_bool().unwrap();
            let rejection = validate_widget_csp_reason(reason, has_inputs)
                .expect_err(&format!("must reject {reason:?}"));
            assert_eq!(
                rejection.code(),
                case["code"].as_str().unwrap(),
                "{reason:?}"
            );
        }
        for case in fixture["duplicateReasons"].as_array().unwrap() {
            let reasons = case["reasons"].as_array().unwrap();
            let folded: Vec<String> = reasons
                .iter()
                .map(|reason| fold_widget_csp_reason(reason.as_str().unwrap()))
                .collect();
            assert_eq!(
                folded[0] == folded[1],
                case["duplicate"].as_bool().unwrap(),
                "{reasons:?}"
            );
        }
    }

    #[test]
    fn fixture_unicode_version_is_supported_by_both_tables() {
        let fixture = fixture();
        let pinned: Vec<u32> = fixture["unicodeVersion"]
            .as_str()
            .unwrap()
            .split('.')
            .map(|part| part.parse().unwrap())
            .collect();
        let pinned = (pinned[0], pinned[1], pinned[2]);
        let (major, minor, update) = char::UNICODE_VERSION;
        assert!((u32::from(major), u32::from(minor), u32::from(update)) >= pinned);
        let (major, minor, update) = unicode_normalization::UNICODE_VERSION;
        assert!((u32::from(major), u32::from(minor), u32::from(update)) >= pinned);
    }

    #[test]
    fn reason_folding_and_codes() {
        assert_eq!(fold_widget_csp_reason("Ｌｏａｄｓ ＭＡＰ"), "loads map");
        assert_eq!(fold_widget_csp_reason("cesium\u{3002}com"), "cesium.com");
        assert_eq!(fold_widget_csp_reason("地図\u{3002}com"), "地図\u{3002}com");
        let codes: BTreeSet<&str> = [
            WidgetCspReasonRejection::Empty,
            WidgetCspReasonRejection::NotNfc,
            WidgetCspReasonRejection::ForbiddenCharacter,
            WidgetCspReasonRejection::Whitespace,
            WidgetCspReasonRejection::Length,
            WidgetCspReasonRejection::TooFewLetters,
            WidgetCspReasonRejection::MixedScript,
            WidgetCspReasonRejection::MentionsProduct,
            WidgetCspReasonRejection::ClaimsAssurance,
            WidgetCspReasonRejection::ClaimsAttribution,
            WidgetCspReasonRejection::Duplicate,
        ]
        .into_iter()
        .map(WidgetCspReasonRejection::code)
        .collect();
        assert_eq!(codes.len(), 11);
        assert!(codes.iter().all(|code| code.starts_with("reason-")));
        assert_eq!(
            validate_widget_csp_reason("Loads x\u{0301}\u{0302} tiles", false),
            Ok(())
        );
        assert_eq!(
            validate_widget_csp_reason("کتاب\u{200C}ها load", false),
            Ok(())
        );
        assert_eq!(
            validate_widget_csp_reason("Loads\u{200C}tiles", false),
            Err(WidgetCspReasonRejection::ForbiddenCharacter)
        );
        assert_eq!(
            validate_widget_csp_reason("\u{200C}نقشه‌ها از سرور", false),
            Err(WidgetCspReasonRejection::ForbiddenCharacter)
        );
    }

    #[test]
    fn input_paths_parse_into_segments() {
        assert_eq!(
            parse_widget_input_path("config.layers[].sources.*.url"),
            Some((
                "config",
                vec![
                    WidgetInputPathSegment::Key("layers"),
                    WidgetInputPathSegment::Items,
                    WidgetInputPathSegment::Key("sources"),
                    WidgetInputPathSegment::Values,
                    WidgetInputPathSegment::Key("url"),
                ]
            ))
        );
        assert_eq!(
            parse_widget_input_path("tileUrl"),
            Some(("tileUrl", vec![]))
        );
        assert_eq!(
            parse_widget_input_path("_a[][]"),
            Some((
                "_a",
                vec![WidgetInputPathSegment::Items, WidgetInputPathSegment::Items]
            ))
        );
        for invalid in [
            "", "1a", "a.", "a..b", "a[0]", "a[", "a.*b", "a*", "a.1b", ".a", "a b", "a-b",
            "a.b-c", "ä",
        ] {
            assert_eq!(parse_widget_input_path(invalid), None, "{invalid:?}");
            assert!(!is_valid_widget_input_path(invalid));
        }
        assert!(is_valid_widget_input_path("a.b.c.d.e.f"));
        assert!(!is_valid_widget_input_path("a.b.c.d.e.f.g"));
    }

    #[test]
    fn fixture_grouped_contracts_canonicalize_flatten_and_validate() {
        let fixture = fixture();
        let cases = fixture["groupedContracts"].as_array().unwrap();
        assert!(cases.len() >= 3);
        for case in cases {
            let name = case["name"].as_str().unwrap();
            let mut authored: WidgetContract =
                serde_json::from_value(case["authored"].clone()).unwrap();
            let purposes = authored.csp.take().unwrap();
            let canonical = authored.with_csp(purposes);
            assert_eq!(
                serde_json::to_value(&canonical).unwrap(),
                case["canonical"],
                "{name}"
            );
            assert_eq!(canonical.validate(), Ok(()), "{name}");
            let declared: WidgetCsp = serde_json::from_value(case["declaredCsp"].clone()).unwrap();
            assert_eq!(canonical.declared_csp(), declared, "{name}");
            let parsed: WidgetContract = serde_json::from_value(case["canonical"].clone()).unwrap();
            assert_eq!(parsed.validate(), Ok(()), "{name}");

            let descriptor = WidgetPolicyDescriptor::describe(
                WidgetPolicySubject {
                    widget_id: canonical.id.clone(),
                    ..subject(false)
                },
                &canonical,
                &[],
            );
            assert!(
                descriptor.is_ok(),
                "{name}: {:?}",
                descriptor.invalid_reason
            );
            assert_eq!(descriptor.policy.csp, declared, "{name}");
            assert_eq!(
                serde_json::to_value(&descriptor.network_inputs).unwrap(),
                case["networkInputs"],
                "{name}"
            );
        }
    }

    #[test]
    fn fixture_invalid_contracts_report_the_listed_error() {
        let fixture = fixture();
        let cases = fixture["invalidContracts"].as_array().unwrap();
        assert!(cases.len() >= 25);
        for case in cases {
            let name = case["name"].as_str().unwrap();
            let expected = case["error"].as_str().unwrap();
            let contract: WidgetContract = serde_json::from_value(case["contract"].clone())
                .unwrap_or_else(|error| panic!("{name} must parse: {error}"));
            let errors = contract.validate().expect_err(name);
            assert!(
                errors.iter().any(|error| error.contains(expected)),
                "{name}: {errors:?}"
            );
            assert!(
                errors
                    .iter()
                    .all(|error| error.starts_with("Widget 'live-map'")),
                "{name}: {errors:?}"
            );
        }
        for case in fixture["unparsableContracts"].as_array().unwrap() {
            assert!(
                serde_json::from_value::<WidgetContract>(case["contract"].clone()).is_err(),
                "{}",
                case["name"]
            );
        }
    }

    #[test]
    fn purposes_canonicalize_without_reordering_groups() {
        let mut first = sourced_purpose(
            "Loads map tiles",
            &[
                "https://b.example.com",
                "https://a.example.com",
                "https://b.example.com",
            ],
            &[],
        );
        first.inputs = vec![
            WidgetNetworkInput {
                template: Some(WidgetUrlTemplate::default()),
                ..network_input(
                    "z",
                    &[
                        CspDirective::StyleSrc,
                        CspDirective::ConnectSrc,
                        CspDirective::StyleSrc,
                    ],
                )
            },
            network_input("a", &[CspDirective::ImgSrc]),
        ];
        let second = sourced_purpose("Loads labels", &[], &["https://c.example.com"]);
        let contract = WidgetContract::new("live-map").with_csp(vec![first, second]);
        let purposes = contract.csp.as_ref().unwrap();
        assert_eq!(purposes[0].reason, "Loads map tiles");
        assert_eq!(
            purposes[0].connect_src,
            strings(&["https://a.example.com", "https://b.example.com"])
        );
        assert_eq!(purposes[0].inputs[0].path, "a");
        assert_eq!(
            purposes[0].inputs[1].directives,
            vec![CspDirective::ConnectSrc, CspDirective::StyleSrc]
        );
        assert_eq!(purposes[0].inputs[1].template, None);
        assert_eq!(contract.contract_version, crate::widget::CONTRACT_VERSION);

        let emptied = contract.with_csp(Vec::new());
        assert!(emptied.csp.is_none());
        assert_eq!(
            emptied.contract_version,
            crate::widget::BASE_CONTRACT_VERSION
        );
        assert!(emptied.declared_csp().is_empty());
    }

    #[test]
    fn from_contract_flattens_purposes_and_preview_strips_network_media_and_microphone() {
        let contract = contract_with(
            Some(full_capabilities()),
            vec![
                sourced_purpose(
                    "Loads map tiles",
                    &["https://b.maptiler.com"],
                    &["https://b.maptiler.com"],
                ),
                sourced_purpose("Loads map labels", &["https://a.maptiler.com"], &[]),
            ],
        );
        let live = WidgetPolicy::from_contract(&contract, false);
        assert!(live.workers && live.wasm && live.media && live.microphone && live.downloads);
        assert_eq!(
            live.csp.connect_src,
            strings(&["https://a.maptiler.com", "https://b.maptiler.com"])
        );
        assert_eq!(live.csp.img_src, strings(&["https://b.maptiler.com"]));

        let preview = WidgetPolicy::from_contract(&contract, true);
        assert!(preview.workers && preview.wasm && preview.downloads);
        assert!(!preview.media && !preview.microphone);
        assert!(preview.csp.is_empty());

        let partial = contract_with(
            Some(WidgetCapabilities {
                workers: Some(false),
                wasm: Some(true),
                ..WidgetCapabilities::default()
            }),
            Vec::new(),
        );
        let policy = WidgetPolicy::from_contract(&partial, false);
        assert!(!policy.workers && policy.wasm && !policy.is_empty());
        assert!(WidgetPolicy::from_contract(&WidgetContract::new("plain"), false).is_empty());
    }

    #[test]
    fn policy_json_skips_false_and_empty_fields_and_denies_unknown_fields() {
        assert_eq!(
            serde_json::to_value(WidgetPolicy::default()).unwrap(),
            json!({})
        );
        let policy = WidgetPolicy {
            workers: true,
            downloads: true,
            csp: csp(&["wss://live.example.com"], &[]),
            ..WidgetPolicy::default()
        };
        assert_eq!(
            serde_json::to_string(&policy).unwrap(),
            r#"{"workers":true,"downloads":true,"csp":{"connectSrc":["wss://live.example.com"]}}"#
        );
        assert_eq!(
            policy.canonical_json(),
            r#"{"csp":{"connectSrc":["wss://live.example.com"]},"downloads":true,"workers":true}"#
        );
        assert!(serde_json::from_value::<WidgetPolicy>(json!({ "sameOrigin": true })).is_err());
        assert!(
            serde_json::from_value::<WidgetPolicy>(json!({ "csp": { "scriptSrc": [] } })).is_err()
        );
    }

    #[test]
    fn digest_is_stable_and_changes_for_any_added_host_or_capability() {
        let base = WidgetPolicy {
            workers: true,
            csp: csp(&["https://api.maptiler.com"], &[]),
            ..WidgetPolicy::default()
        };
        assert_eq!(base.digest(), base.clone().digest());
        assert!(base.digest().starts_with("sha256:"));
        assert_eq!(base.digest().len(), "sha256:".len() + 64);

        let mut more_hosts = base.clone();
        more_hosts
            .csp
            .connect_src
            .push("wss://live.example.com".into());
        assert_ne!(base.digest(), more_hosts.digest());

        let mut other_directive = base.clone();
        other_directive
            .csp
            .img_src
            .push("https://api.maptiler.com".into());
        assert_ne!(base.digest(), other_directive.digest());

        let mut moved = base.clone();
        moved.csp.img_src = std::mem::take(&mut moved.csp.connect_src);
        assert_ne!(base.digest(), moved.digest());

        let toggles: [fn(&mut WidgetPolicy); 5] = [
            |p| p.wasm = true,
            |p| p.media = true,
            |p| p.microphone = true,
            |p| p.downloads = true,
            |p| p.workers = false,
        ];
        for toggle in toggles {
            let mut changed = base.clone();
            toggle(&mut changed);
            assert_ne!(base.digest(), changed.digest());
        }
    }

    #[test]
    fn descriptor_serializes_the_pinned_shape() {
        let contract = contract_with(
            Some(WidgetCapabilities {
                workers: Some(true),
                ..WidgetCapabilities::default()
            }),
            vec![sourced_purpose(
                "Loads vector map tiles",
                &["https://api.maptiler.com"],
                &[],
            )],
        );
        let descriptor = WidgetPolicyDescriptor::describe(subject(false), &contract, &[]);
        assert!(descriptor.is_ok());
        let value = serde_json::to_value(&descriptor).unwrap();
        assert_eq!(
            value,
            json!({
                "source": "registry:hub.flow-like.com",
                "packageId": "com.example.maps",
                "bundleHash": "a".repeat(64),
                "widgetId": "live-map",
                "preview": false,
                "status": "ok",
                "policy": { "workers": true, "csp": { "connectSrc": ["https://api.maptiler.com"] } },
                "policyDigest": descriptor.policy.digest(),
                "networkInputs": [],
                "platformStorage": [],
                "runtime": { "status": "none", "declaredDigest": descriptor.policy.digest() },
                "engine": { "wildcardSources": true, "runtimeSources": true, "localMedia": true },
            })
        );
        let roundtrip: WidgetPolicyDescriptor = serde_json::from_value(value).unwrap();
        assert_eq!(roundtrip, descriptor);
        assert_eq!(descriptor.declared_digest(), descriptor.policy_digest);

        let mut web = subject(true);
        web.source = WIDGET_POLICY_SOURCE_HUB.into();
        web.package_version = Some("1.2.0".into());
        let preview = WidgetPolicyDescriptor::describe(web, &contract, &[]);
        assert_eq!(
            preview.policy,
            WidgetPolicy {
                workers: true,
                ..WidgetPolicy::default()
            }
        );
        let preview_json = serde_json::to_value(&preview).unwrap();
        assert_eq!(preview_json["packageVersion"], "1.2.0");
        assert_eq!(preview_json["networkInputs"], json!([]));
        for omitted in [
            "platformStorage",
            "runtime",
            "engine",
            "network",
            "invalidReason",
        ] {
            assert!(preview_json.get(omitted).is_none(), "{omitted}");
        }
    }

    #[test]
    fn descriptor_is_invalid_with_baseline_policy_on_reserved_host_or_bad_contract() {
        let contract = contract_with(
            Some(full_capabilities()),
            vec![sourced_purpose(
                "Loads vector map tiles",
                &["https://api.flow-like.com"],
                &[],
            )],
        );
        let reserved =
            WidgetPolicyDescriptor::describe(subject(false), &contract, &["flow-like.com".into()]);
        assert_eq!(reserved.status, WidgetPolicyStatus::Invalid);
        assert!(reserved.policy.is_empty());
        assert_eq!(reserved.policy_digest, WidgetPolicy::default().digest());
        assert!(
            reserved
                .invalid_reason
                .as_deref()
                .unwrap()
                .contains("reserved host")
        );
        let invalid_json = serde_json::to_value(&reserved).unwrap();
        assert_eq!(invalid_json["status"], "invalid");
        assert_eq!(invalid_json["networkInputs"], json!([]));
        for omitted in ["platformStorage", "runtime", "engine", "network"] {
            assert!(invalid_json.get(omitted).is_none(), "{omitted}");
        }

        let preview =
            WidgetPolicyDescriptor::describe(subject(true), &contract, &["flow-like.com".into()]);
        assert!(
            preview.is_ok(),
            "preview strips csp, so reserved hosts cannot invalidate it"
        );

        let wildcard = contract_with(
            None,
            vec![sourced_purpose(
                "Loads vector map tiles",
                &["https://*.flow-like.com"],
                &[],
            )],
        );
        let covered = WidgetPolicyDescriptor::describe(
            subject(false),
            &wildcard,
            &["api.flow-like.com".into()],
        );
        assert!(
            covered
                .invalid_reason
                .unwrap()
                .contains("covers a reserved host")
        );

        let mut unversioned = contract.clone();
        unversioned.contract_version = 1;
        let invalid = WidgetPolicyDescriptor::describe(subject(false), &unversioned, &[]);
        assert!(!invalid.is_ok());
        assert!(invalid.invalid_reason.unwrap().contains("contractVersion"));

        let mut foreign = subject(false);
        foreign.widget_id = "other-widget".into();
        let mismatch = WidgetPolicyDescriptor::describe(foreign, &contract, &[]);
        assert!(mismatch.invalid_reason.unwrap().contains("does not match"));

        let explicit = WidgetPolicyDescriptor::invalid(subject(false), "unreadable".into());
        assert_eq!(explicit.invalid_reason.as_deref(), Some("unreadable"));
        assert!(explicit.policy.is_empty() && explicit.runtime.is_none());
    }

    fn long_bundle_sources(count: usize) -> Vec<String> {
        (0..count)
            .map(|index| {
                format!(
                    "https://origin{index}.example.com/api/v1/registry/package/com.example.maps/widget-sandbox/{}/",
                    "1".repeat(160)
                )
            })
            .collect()
    }

    #[test]
    fn declared_policy_over_the_document_budget_is_invalid() {
        let contract = contract_with(
            Some(full_capabilities()),
            vec![sourced_purpose(
                "Loads vector map tiles",
                &["https://api.maptiler.com"],
                &[],
            )],
        );
        let bundle = long_bundle_sources(3);
        let within = WidgetPolicyDescriptor::describe_with_runtime(
            subject(false),
            &contract,
            &[],
            &context(&[], &[], None, &bundle[..1]),
        );
        assert!(within.is_ok());
        let over = WidgetPolicyDescriptor::describe_with_runtime(
            subject(false),
            &contract,
            &[],
            &context(&[], &[], None, &bundle),
        );
        assert!(!over.is_ok());
        assert!(
            over.invalid_reason
                .unwrap()
                .starts_with(DECLARED_CSP_TOO_LARGE)
        );
    }

    #[test]
    fn runtime_sources_fan_out_into_the_effective_policy() {
        let contract = runtime_contract();
        let storage = scopes();
        let descriptor = WidgetPolicyDescriptor::describe_with_runtime(
            web_subject(),
            &contract,
            &request(&[
                ("apiUrl", &["https://api.cesium.com"]),
                (
                    "tileUrl",
                    &[
                        "wss://live.example.com",
                        "https://api.cesium.com",
                        "https://a.tiles.example.com",
                    ],
                ),
                ("videoUrl", &["https://media.example.com"]),
            ]),
            &context(&[], &storage, Some("app_1"), &[]),
        );
        assert!(descriptor.is_ok());
        let declared = WidgetPolicy::from_contract(&contract, false);
        assert_eq!(
            descriptor.policy.csp.connect_src,
            strings(&[
                "https://a.tiles.example.com",
                "https://api.cesium.com",
                "https://media.example.com",
                "wss://live.example.com"
            ])
        );
        assert_eq!(
            descriptor.policy.csp.img_src,
            strings(&["https://a.tiles.example.com", "https://api.cesium.com"])
        );
        assert_eq!(
            descriptor.policy.csp.media_src,
            strings(&["https://media.example.com"])
        );
        assert!(descriptor.policy.workers);
        assert_eq!(descriptor.policy_digest, descriptor.policy.digest());

        let runtime = descriptor.runtime.as_ref().unwrap();
        assert_eq!(runtime.status, WidgetRuntimeStatus::Ok);
        assert_eq!(runtime.declared_digest, declared.digest());
        assert_eq!(descriptor.declared_digest(), declared.digest());
        assert!(runtime.rejected.is_empty());

        let record = WidgetRuntimeSources {
            package_id: "com.example.maps".into(),
            version: Some("1.4.0".into()),
            bundle_hash: "a".repeat(64),
            widget_id: "live-map".into(),
            app_id: Some("app_1".into()),
            slots: BTreeMap::from([
                (
                    "tileUrl".to_string(),
                    strings(&[
                        "https://a.tiles.example.com",
                        "https://api.cesium.com",
                        "wss://live.example.com",
                    ]),
                ),
                (
                    "videoUrl".to_string(),
                    strings(&["https://media.example.com"]),
                ),
            ]),
        };
        assert_eq!(descriptor.runtime_sources(), Some(&record));
        assert_eq!(
            runtime.runtime_digest.as_deref(),
            Some(record.digest().as_str())
        );

        let entries: Vec<(usize, &str, CspDirective, &str)> = runtime
            .entries
            .iter()
            .map(|entry| {
                (
                    entry.purpose,
                    entry.slot.as_str(),
                    entry.directive,
                    entry.source.as_str(),
                )
            })
            .collect();
        assert_eq!(
            entries,
            vec![
                (
                    1,
                    "tileUrl",
                    CspDirective::ConnectSrc,
                    "https://a.tiles.example.com"
                ),
                (
                    1,
                    "tileUrl",
                    CspDirective::ImgSrc,
                    "https://a.tiles.example.com"
                ),
                (1, "tileUrl", CspDirective::ImgSrc, "https://api.cesium.com"),
                (
                    1,
                    "tileUrl",
                    CspDirective::ConnectSrc,
                    "wss://live.example.com"
                ),
                (
                    1,
                    "videoUrl",
                    CspDirective::ConnectSrc,
                    "https://media.example.com"
                ),
                (
                    1,
                    "videoUrl",
                    CspDirective::MediaSrc,
                    "https://media.example.com"
                ),
            ]
        );

        let value = serde_json::to_value(&descriptor).unwrap();
        assert_eq!(
            value["runtime"],
            json!({
                "status": "ok",
                "declaredDigest": declared.digest(),
                "runtimeDigest": record.digest(),
            })
        );
        assert_eq!(
            value["networkInputs"],
            json!([
                { "path": "apiUrl", "purpose": 1, "directives": ["connectSrc"] },
                { "path": "layers[].url", "purpose": 1, "directives": ["imgSrc"] },
                { "path": "tileUrl", "purpose": 1, "directives": ["connectSrc", "imgSrc"], "template": { "subdomainsInput": "tileSubdomains" } },
                { "path": "videoUrl", "purpose": 1, "directives": ["connectSrc", "mediaSrc"] },
            ])
        );
        assert_eq!(
            value["platformStorage"],
            json!([{ "origin": "https://s3.eu-central-1.amazonaws.com", "pathPrefix": "/flow-like-content/apps/" }])
        );
        let parsed: WidgetPolicyDescriptor = serde_json::from_value(value).unwrap();
        assert_eq!(parsed.policy, descriptor.policy);
        assert_eq!(parsed.runtime_sources(), None);
    }

    #[test]
    fn runtime_candidates_are_rejected_individually_with_codes() {
        let contract = runtime_contract();
        let storage = scopes();
        let reserved = vec!["flow-like.com".to_string()];
        let requested = request(&[
            ("unknownSlot", &["https://a.example.com"]),
            (
                "layers[].url",
                &[
                    "wss://live.example.com",
                    APP_STORAGE,
                    "https://s3.eu-central-1.amazonaws.com/flow-like-content/apps/app_2/",
                    "https://s3.eu-central-1.amazonaws.com",
                    "https://s3.eu-central-1.amazonaws.com/flow-like-content/apps/app_1",
                ],
            ),
            (
                "tileUrl",
                &[
                    "https://*.tiles.example.com",
                    "https://tiles.example.com:8443",
                    "https://10.0.0.1",
                    "http://tiles.example.com",
                    "https://api.flow-like.com",
                    "https://a.nip.io",
                    "https://tiles.example.com/x/",
                    "https://Tiles.example.com",
                ],
            ),
        ]);
        let descriptor = WidgetPolicyDescriptor::describe_with_runtime(
            web_subject(),
            &contract,
            &requested,
            &context(&reserved, &storage, Some("app_1"), &[]),
        );
        let expected: Vec<(String, String, String)> = [
            (
                "layers[].url",
                "https://s3.eu-central-1.amazonaws.com",
                "platform-storage",
            ),
            (
                "layers[].url",
                "https://s3.eu-central-1.amazonaws.com/flow-like-content/apps/app_1",
                "platform-storage",
            ),
            (
                "layers[].url",
                "https://s3.eu-central-1.amazonaws.com/flow-like-content/apps/app_2/",
                "platform-storage",
            ),
            (
                "layers[].url",
                "wss://live.example.com",
                "scheme-not-allowed",
            ),
            ("tileUrl", "http://tiles.example.com", "scheme-not-allowed"),
            ("tileUrl", "https://*.tiles.example.com", "wildcard"),
            ("tileUrl", "https://10.0.0.1", "ip-literal"),
            ("tileUrl", "https://Tiles.example.com", "uppercase"),
            ("tileUrl", "https://a.nip.io", "reserved-name"),
            ("tileUrl", "https://api.flow-like.com", "reserved-host"),
            (
                "tileUrl",
                "https://tiles.example.com/x/",
                "platform-storage",
            ),
            ("tileUrl", "https://tiles.example.com:8443", "port"),
            ("unknownSlot", "https://a.example.com", "unknown-slot"),
        ]
        .into_iter()
        .map(|(slot, source, code)| (slot.to_string(), source.to_string(), code.to_string()))
        .collect();
        assert_eq!(rejections(&descriptor), expected);
        assert_eq!(descriptor.policy.csp.img_src, strings(&[APP_STORAGE]));
        assert_eq!(
            descriptor.runtime_sources().unwrap().slots,
            BTreeMap::from([("layers[].url".to_string(), strings(&[APP_STORAGE]))])
        );
        assert_eq!(descriptor.policy.csp.validate_effective(), Ok(()));

        let without_app = WidgetPolicyDescriptor::describe_with_runtime(
            web_subject(),
            &contract,
            &request(&[("layers[].url", &[APP_STORAGE])]),
            &context(&reserved, &storage, None, &[]),
        );
        assert_eq!(
            rejections(&without_app),
            vec![(
                "layers[].url".into(),
                APP_STORAGE.into(),
                "platform-storage".into()
            )]
        );
        let runtime = without_app.runtime.as_ref().unwrap();
        assert_eq!(runtime.status, WidgetRuntimeStatus::Ok);
        assert_eq!(runtime.runtime_digest, None);
        assert_eq!(without_app.policy_digest, runtime.declared_digest);

        let invalid_app = WidgetPolicyDescriptor::describe_with_runtime(
            web_subject(),
            &contract,
            &request(&[(
                "layers[].url",
                &["https://s3.eu-central-1.amazonaws.com/flow-like-content/apps/a.b/"],
            )]),
            &context(&reserved, &storage, Some("a.b"), &[]),
        );
        assert_eq!(rejections(&invalid_app)[0].2, "platform-storage");
    }

    #[test]
    fn runtime_caps_leave_the_policy_declared_only() {
        let contract = runtime_contract();
        let declared = WidgetPolicy::from_contract(&contract, false).digest();
        let hosts: Vec<String> = (1..=9)
            .map(|index| format!("https://h{index}.example.com"))
            .collect();
        let host_refs: Vec<&str> = hosts.iter().map(String::as_str).collect();

        let too_many = WidgetPolicyDescriptor::describe_with_runtime(
            web_subject(),
            &contract,
            &request(&[
                ("tileUrl", &host_refs[..5]),
                ("layers[].url", &host_refs[4..]),
            ]),
            &context(&[], &[], None, &[]),
        );
        let runtime = too_many.runtime.as_ref().unwrap();
        assert_eq!(runtime.status, WidgetRuntimeStatus::Invalid);
        assert_eq!(
            runtime.invalid_reason.as_deref(),
            Some(RUNTIME_INVALID_TOO_MANY_SOURCES)
        );
        assert_eq!(runtime.runtime_digest, None);
        assert!(runtime.sources.is_none() && runtime.entries.is_empty());
        assert_eq!(too_many.policy_digest, declared);

        let eight = WidgetPolicyDescriptor::describe_with_runtime(
            web_subject(),
            &contract,
            &request(&[
                ("tileUrl", &host_refs[..4]),
                ("layers[].url", &host_refs[3..8]),
            ]),
            &context(&[], &[], None, &[]),
        );
        assert_eq!(
            eight.runtime.as_ref().unwrap().status,
            WidgetRuntimeStatus::Ok
        );

        let mut wide = runtime_contract();
        wide.csp.as_mut().unwrap()[1].inputs[0].directives = CspDirective::ALL.to_vec();
        let entries = WidgetPolicyDescriptor::describe_with_runtime(
            web_subject(),
            &wide,
            &request(&[("apiUrl", &host_refs[..4])]),
            &context(&[], &[], None, &[]),
        );
        let runtime = entries.runtime.as_ref().unwrap();
        assert_eq!(
            runtime.invalid_reason.as_deref(),
            Some(RUNTIME_INVALID_TOO_LARGE)
        );
        assert_eq!(entries.policy_digest, declared);
        let within = WidgetPolicyDescriptor::describe_with_runtime(
            web_subject(),
            &wide,
            &request(&[("apiUrl", &host_refs[..3])]),
            &context(&[], &[], None, &[]),
        );
        assert_eq!(
            within.runtime.as_ref().unwrap().status,
            WidgetRuntimeStatus::Ok
        );

        let long_hosts: Vec<String> = (0..8)
            .map(|index| {
                format!(
                    "https://h{index}-{}.{}.example.com",
                    "a".repeat(60),
                    "b".repeat(50)
                )
            })
            .collect();
        let long_refs: Vec<&str> = long_hosts.iter().map(String::as_str).collect();
        let bytes = WidgetPolicyDescriptor::describe_with_runtime(
            web_subject(),
            &contract,
            &request(&[("apiUrl", &long_refs)]),
            &context(&[], &[], None, &[]),
        );
        assert_eq!(
            bytes.runtime.as_ref().unwrap().invalid_reason.as_deref(),
            Some(RUNTIME_INVALID_TOO_LARGE)
        );

        let medium_hosts: Vec<String> = (0..8)
            .map(|index| {
                format!(
                    "https://h{index}-{}.{}.example.com",
                    "c".repeat(40),
                    "d".repeat(30)
                )
            })
            .collect();
        let medium_refs: Vec<&str> = medium_hosts.iter().map(String::as_str).collect();
        let medium_request = request(&[("tileUrl", &medium_refs)]);
        let fits = WidgetPolicyDescriptor::describe_with_runtime(
            web_subject(),
            &contract,
            &medium_request,
            &context(&[], &[], None, &[]),
        );
        assert_eq!(
            fits.runtime.as_ref().unwrap().status,
            WidgetRuntimeStatus::Ok
        );
        let bundle = long_bundle_sources(2);
        let header = WidgetPolicyDescriptor::describe_with_runtime(
            web_subject(),
            &contract,
            &medium_request,
            &context(&[], &[], None, &bundle),
        );
        assert!(header.is_ok());
        assert_eq!(
            header.runtime.as_ref().unwrap().invalid_reason.as_deref(),
            Some(RUNTIME_INVALID_TOO_LARGE)
        );
    }

    #[test]
    fn declared_wildcards_over_public_suffixes_make_the_descriptor_invalid() {
        let mut contract = runtime_contract();
        let purpose = &mut contract.csp.as_mut().unwrap()[0];
        purpose.img_src.push("https://*.co.uk".into());
        purpose.canonicalize();
        let descriptor = WidgetPolicyDescriptor::describe_with_runtime(
            web_subject(),
            &contract,
            &[],
            &context(&[], &[], None, &[]),
        );
        assert_eq!(descriptor.status, WidgetPolicyStatus::Invalid);
        assert!(
            descriptor
                .invalid_reason
                .as_deref()
                .is_some_and(|reason| reason.contains("https://*.co.uk"))
        );
        assert!(descriptor.policy.csp.is_empty());
    }

    #[test]
    fn empty_requests_engine_gates_and_previews_carry_no_runtime_sources() {
        let contract = runtime_contract();
        let declared = WidgetPolicy::from_contract(&contract, false).digest();
        for empty in [request(&[]), request(&[("tileUrl", &[]), ("unknown", &[])])] {
            let descriptor = WidgetPolicyDescriptor::describe_with_runtime(
                web_subject(),
                &contract,
                &empty,
                &context(&[], &[], None, &[]),
            );
            assert_eq!(
                serde_json::to_value(descriptor.runtime.as_ref().unwrap()).unwrap(),
                json!({ "status": "none", "declaredDigest": declared })
            );
        }

        let gated = WidgetPolicyDescriptor::describe_with_runtime(
            web_subject(),
            &contract,
            &request(&[("tileUrl", &["https://a.tiles.example.com"])]),
            &WidgetRuntimeContext {
                engine: EngineGate {
                    runtime_sources: false,
                    ..EngineGate::OPEN
                },
                ..context(&[], &[], None, &[])
            },
        );
        assert_eq!(gated.policy_digest, declared);
        assert_eq!(
            serde_json::to_value(&gated).unwrap()["runtime"],
            json!({ "status": "unavailable", "declaredDigest": declared, "invalidReason": "engine" })
        );
        assert_eq!(
            serde_json::to_value(&gated).unwrap()["engine"],
            json!({ "wildcardSources": true, "runtimeSources": false, "localMedia": true })
        );

        let preview = WidgetPolicyDescriptor::describe_with_runtime(
            WidgetPolicySubject {
                preview: true,
                ..web_subject()
            },
            &contract,
            &request(&[("tileUrl", &["https://a.tiles.example.com"])]),
            &context(&[], &scopes(), Some("app_1"), &[]),
        );
        assert!(preview.is_ok());
        assert!(preview.policy.csp.is_empty());
        assert!(preview.network_inputs.is_empty());
        assert!(preview.runtime.is_none() && preview.engine.is_none());
        assert!(preview.platform_storage.is_none());
    }

    #[test]
    fn duplicate_request_entries_merge_before_derivation() {
        let contract = runtime_contract();
        let merged = WidgetPolicyDescriptor::describe_with_runtime(
            web_subject(),
            &contract,
            &request(&[(
                "tileUrl",
                &["https://a.tiles.example.com", "https://b.tiles.example.com"],
            )]),
            &context(&[], &[], None, &[]),
        );
        let split = WidgetPolicyDescriptor::describe_with_runtime(
            web_subject(),
            &contract,
            &request(&[
                (
                    "tileUrl",
                    &["https://b.tiles.example.com", "https://a.tiles.example.com"],
                ),
                ("tileUrl", &["https://a.tiles.example.com"]),
            ]),
            &context(&[], &[], None, &[]),
        );
        assert_eq!(merged, split);
    }

    #[test]
    fn runtime_record_digest_is_canonical_and_domain_separated() {
        let record = WidgetRuntimeSources {
            package_id: "com.example.maps".into(),
            version: None,
            bundle_hash: "b".repeat(64),
            widget_id: "live-map".into(),
            app_id: None,
            slots: BTreeMap::from([("tileUrl".to_string(), strings(&["https://a.example.com"]))]),
        };
        assert_eq!(
            record.canonical_json(),
            format!(
                r#"{{"bundleHash":"{}","packageId":"com.example.maps","slots":{{"tileUrl":["https://a.example.com"]}},"widgetId":"live-map"}}"#,
                "b".repeat(64)
            )
        );
        assert_eq!(
            record.digest(),
            domain_digest(WIDGET_RUNTIME_DIGEST_DOMAIN, &record.canonical_json())
        );
        assert_ne!(
            record.digest(),
            domain_digest(WIDGET_POLICY_DIGEST_DOMAIN, &record.canonical_json())
        );
        let with_app = WidgetRuntimeSources {
            app_id: Some("app_1".into()),
            ..record.clone()
        };
        assert_ne!(record.digest(), with_app.digest());
        let with_version = WidgetRuntimeSources {
            version: Some("1.0.0".into()),
            ..record.clone()
        };
        assert_ne!(record.digest(), with_version.digest());
        assert_eq!(
            record.request(),
            request(&[("tileUrl", &["https://a.example.com"])])
        );
        assert!(
            serde_json::from_value::<WidgetRuntimeSources>(json!({
                "packageId": "p", "bundleHash": "h", "widgetId": "w", "slots": {}, "extra": 1
            }))
            .is_err()
        );
        assert_eq!(
            runtime_slots_canonical_json(&record.slots),
            r#"{"tileUrl":["https://a.example.com"]}"#
        );
    }

    #[test]
    fn carried_runtime_component_rederives_the_same_grant() {
        let contract = runtime_contract();
        let storage = scopes();
        let reserved = vec!["flow-like.com".to_string()];
        let ctx = context(&reserved, &storage, Some("app_1"), &[]);
        let minted = WidgetPolicyDescriptor::describe_with_runtime(
            web_subject(),
            &contract,
            &request(&[
                ("layers[].url", &[APP_STORAGE, "https://tiles.example.com"]),
                ("tileUrl", &["https://a.tiles.example.com"]),
            ]),
            &ctx,
        );
        let record = minted.runtime_sources().unwrap();
        let component = encode_runtime_component(&record.slots).unwrap();
        let slots = decode_runtime_component(&component).unwrap();
        let served = WidgetPolicyDescriptor::describe_with_runtime(
            web_subject(),
            &contract,
            &WidgetRuntimeSourceRequest::from_slots(&slots),
            &ctx,
        );
        assert!(served.runtime.as_ref().unwrap().rejected.is_empty());
        assert_eq!(served.policy_digest, minted.policy_digest);
        assert_eq!(
            served.runtime.as_ref().unwrap().runtime_digest,
            minted.runtime.as_ref().unwrap().runtime_digest
        );

        let other_app = WidgetPolicyDescriptor::describe_with_runtime(
            web_subject(),
            &contract,
            &WidgetRuntimeSourceRequest::from_slots(&slots),
            &context(&reserved, &storage, Some("app_2"), &[]),
        );
        assert_ne!(other_app.policy_digest, minted.policy_digest);
        assert!(!other_app.runtime.as_ref().unwrap().rejected.is_empty());
    }

    #[test]
    fn classifier_inputs_follow_the_contract_and_accepted_entries() {
        let contract = runtime_contract();
        let descriptor = WidgetPolicyDescriptor::describe_with_runtime(
            web_subject(),
            &contract,
            &request(&[("videoUrl", &["https://media.example.com"])]),
            &context(&[], &[], None, &[]),
        );
        let (purposes, runtime) = descriptor.classifier_inputs(&contract);
        assert_eq!(
            purposes,
            vec![
                NetworkPurposeInput {
                    reason: "Loads globe terrain and imagery from Cesium ion".into(),
                    declared: vec![(CspDirective::ConnectSrc, "https://api.cesium.com".into())],
                    inputs: Vec::new(),
                },
                NetworkPurposeInput {
                    reason: "Loads map tiles from tile servers given to it at runtime".into(),
                    declared: Vec::new(),
                    inputs: strings(&["apiUrl", "layers[].url", "tileUrl", "videoUrl"]),
                },
            ]
        );
        assert_eq!(
            runtime,
            vec![
                NetworkRuntimeInput {
                    purpose: 1,
                    slot: "videoUrl".into(),
                    directive: CspDirective::ConnectSrc,
                    source: "https://media.example.com".into(),
                },
                NetworkRuntimeInput {
                    purpose: 1,
                    slot: "videoUrl".into(),
                    directive: CspDirective::MediaSrc,
                    source: "https://media.example.com".into(),
                },
            ]
        );
        let preview = WidgetPolicyDescriptor::describe(subject(true), &contract, &[]);
        assert_eq!(
            preview.classifier_inputs(&contract),
            (Vec::new(), Vec::new())
        );
    }

    #[test]
    fn runtime_request_shape_limits() {
        let nine: Vec<WidgetRuntimeSourceRequest> = (0..9)
            .map(|index| WidgetRuntimeSourceRequest {
                slot: format!("slot{index}"),
                sources: Vec::new(),
            })
            .collect();
        assert!(validate_runtime_request_shape(&nine[..8]).is_ok());
        assert!(
            validate_runtime_request_shape(&nine)
                .unwrap_err()
                .contains("slots")
        );
        assert!(
            validate_runtime_request_shape(&request(&[("a", &[]), ("a", &[])]))
                .unwrap_err()
                .contains("twice")
        );
        let many: Vec<String> = (0..33)
            .map(|index| format!("https://h{index}.example.com"))
            .collect();
        let many_refs: Vec<&str> = many.iter().map(String::as_str).collect();
        assert!(validate_runtime_request_shape(&request(&[("a", &many_refs[..32])])).is_ok());
        assert!(validate_runtime_request_shape(&request(&[("a", &many_refs)])).is_err());
        let long = format!("https://{}.example.com", "a".repeat(290));
        assert!(validate_runtime_request_shape(&request(&[("a", &[long.as_str()])])).is_err());
        assert!(
            serde_json::from_value::<WidgetRuntimeSourceRequest>(
                json!({ "slot": "a", "sources": [1] })
            )
            .is_err()
        );
        assert!(
            serde_json::from_value::<WidgetRuntimeSourceRequest>(
                json!({ "slot": "a", "sources": [], "x": 1 })
            )
            .is_err()
        );
    }

    #[test]
    fn platform_storage_scopes_build_app_sources() {
        let scope = &scopes()[0];
        assert_eq!(
            scope.host().as_deref(),
            Some("s3.eu-central-1.amazonaws.com")
        );
        assert_eq!(scope.app_source("app_1").as_deref(), Some(APP_STORAGE));
        let long = "a".repeat(65);
        for app_id in ["", "a.b", "a/b", long.as_str()] {
            assert_eq!(scope.app_source(app_id), None, "{app_id:?}");
        }
        assert!(is_valid_widget_app_id(&"A-z_9".repeat(12)));
        for (origin, prefix) in [
            ("https://s3.eu-central-1.amazonaws.com", "apps/"),
            ("https://s3.eu-central-1.amazonaws.com", "/apps"),
            ("http://minio.example.com", "/apps/"),
            ("https://minio.example.com:9000", "/apps/"),
            ("https://minio.example.com/", "/apps/"),
        ] {
            let scope = PlatformStorageScope {
                origin: origin.into(),
                path_prefix: prefix.into(),
            };
            assert_eq!(scope.app_source("app_1"), None, "{origin} {prefix}");
        }
        assert_eq!(
            serde_json::to_value(scope).unwrap(),
            json!({ "origin": "https://s3.eu-central-1.amazonaws.com", "pathPrefix": "/flow-like-content/apps/" })
        );
    }

    #[test]
    fn fixture_extraction_requests_derive_on_the_server() {
        let fixture = extraction_fixture();
        let codes: BTreeSet<&str> = fixture["issueCodes"]
            .as_array()
            .unwrap()
            .iter()
            .map(|code| code.as_str().unwrap())
            .collect();
        let cross_slot = fixture["crossSlotIssueSlot"].as_str().unwrap();
        let reserved: Vec<String> =
            serde_json::from_value(fixture["reservedHosts"].clone()).unwrap();
        let storage: Vec<PlatformStorageScope> =
            serde_json::from_value(fixture["platformStorage"].clone()).unwrap();
        let catalog: BTreeMap<String, WidgetNetworkInputSlot> =
            serde_json::from_value(fixture["slots"].clone()).unwrap();

        let mut contract = WidgetContract::new("live-map");
        for (name, kind) in fixture["inputs"].as_object().unwrap() {
            let input_type = serde_json::from_value(kind.clone()).unwrap();
            contract.inputs.insert(name.clone(), input(input_type));
        }
        let purposes: Vec<WidgetCspPurpose> = fixture["purposes"]
            .as_array()
            .unwrap()
            .iter()
            .enumerate()
            .map(|(index, reason)| WidgetCspPurpose {
                inputs: catalog
                    .values()
                    .filter(|slot| slot.purpose == index)
                    .map(|slot| WidgetNetworkInput {
                        path: slot.path.clone(),
                        directives: slot.directives.clone(),
                        template: slot.template.clone(),
                    })
                    .collect(),
                ..purpose(reason.as_str().unwrap())
            })
            .collect();
        let contract = contract.with_csp(purposes);
        assert_eq!(contract.validate(), Ok(()));
        let slots = network_input_slots(&contract);
        assert_eq!(slots.len(), catalog.len());
        for slot in &slots {
            assert_eq!(&catalog[&slot.path], slot);
        }

        let cases = fixture["cases"].as_array().unwrap();
        assert!(cases.len() >= 20);
        let mut names = BTreeSet::new();
        for case in cases {
            let name = case["name"].as_str().unwrap();
            assert!(names.insert(name), "duplicate case {name}");
            for slot in case["slots"].as_array().unwrap() {
                assert!(catalog.contains_key(slot.as_str().unwrap()), "{name}");
            }
            let expected: Vec<WidgetRuntimeSourceRequest> =
                serde_json::from_value(case["expected"]["request"].clone()).unwrap();
            assert!(
                expected.windows(2).all(|pair| pair[0].slot < pair[1].slot),
                "{name}: request slots must be sorted"
            );
            let mut distinct = BTreeSet::new();
            for entry in &expected {
                assert!(catalog.contains_key(&entry.slot), "{name}");
                assert!(!entry.sources.is_empty(), "{name}");
                assert!(
                    entry.sources.windows(2).all(|pair| pair[0] < pair[1]),
                    "{name}: sources must be sorted"
                );
                assert!(entry.sources.len() <= MAX_WIDGET_RUNTIME_SOURCES, "{name}");
                distinct.extend(entry.sources.iter().cloned());
            }
            assert!(distinct.len() <= MAX_WIDGET_RUNTIME_SOURCES, "{name}");
            assert_eq!(validate_runtime_request_shape(&expected), Ok(()), "{name}");

            let issues = case["expected"]["issues"].as_array().unwrap();
            let keys: Vec<(&str, &str)> = issues
                .iter()
                .map(|issue| {
                    (
                        issue["slot"].as_str().unwrap(),
                        issue["code"].as_str().unwrap(),
                    )
                })
                .collect();
            assert!(
                keys.windows(2).all(|pair| pair[0] < pair[1]),
                "{name}: issues must be sorted"
            );
            for issue in issues {
                let code = issue["code"].as_str().unwrap();
                let slot = issue["slot"].as_str().unwrap();
                assert!(codes.contains(code), "{name}: unknown code {code}");
                assert!(slot == cross_slot || catalog.contains_key(slot), "{name}");
                assert!(issue["count"].as_u64().unwrap() >= 1, "{name}");
            }

            let app_id = case["appId"].as_str();
            let descriptor = WidgetPolicyDescriptor::describe_with_runtime(
                web_subject(),
                &contract,
                &expected,
                &context(&reserved, &storage, app_id, &[]),
            );
            assert!(descriptor.is_ok(), "{name}");
            let runtime = descriptor.runtime.as_ref().unwrap();
            let rejected: Vec<WidgetRuntimeRejection> = case
                .get("server")
                .map(|server| serde_json::from_value(server["rejected"].clone()).unwrap())
                .unwrap_or_default();
            assert_eq!(runtime.rejected, rejected, "{name}");
            if expected.is_empty() {
                assert_eq!(runtime.status, WidgetRuntimeStatus::None, "{name}");
            } else {
                assert_eq!(runtime.status, WidgetRuntimeStatus::Ok, "{name}");
                let accepted = distinct.len() - rejected.len();
                assert_eq!(runtime.runtime_digest.is_some(), accepted > 0, "{name}");
            }
        }
    }

    #[test]
    fn user_agents_classify_engines_platforms_and_versions() {
        let cases = [
            (
                "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/128.0.6613.120 Safari/537.36",
                Engine::Chromium,
                Some("windows"),
                Some((128, 0)),
            ),
            (
                "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/128.0.0.0 Safari/537.36 Edg/128.0.2739.67",
                Engine::Chromium,
                Some("windows"),
                Some((128, 0)),
            ),
            (
                "Mozilla/5.0 (Linux; Android 14; Pixel 8 Build/AP2A; wv) AppleWebKit/537.36 (KHTML, like Gecko) Version/4.0 Chrome/128.0.6613.99 Mobile Safari/537.36",
                Engine::Chromium,
                Some("android"),
                Some((128, 0)),
            ),
            (
                "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.6 Safari/605.1.15",
                Engine::WebKit,
                Some("macos"),
                Some((17, 6)),
            ),
            (
                "Mozilla/5.0 (iPhone; CPU iPhone OS 17_5_1 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.5 Mobile/15E148 Safari/604.1",
                Engine::WebKit,
                Some("ios"),
                Some((17, 5)),
            ),
            (
                "Mozilla/5.0 (iPhone; CPU iPhone OS 18_0 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) CriOS/129.0.6668.46 Mobile/15E148 Safari/604.1",
                Engine::WebKit,
                Some("ios"),
                Some((18, 0)),
            ),
            (
                "Mozilla/5.0 (iPad; CPU OS 16_7 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) FxiOS/129.0 Mobile/15E148 Safari/605.1.15",
                Engine::WebKit,
                Some("ios"),
                Some((16, 7)),
            ),
            (
                "Mozilla/5.0 (X11; Linux x86_64; rv:130.0) Gecko/20100101 Firefox/130.0",
                Engine::Gecko,
                Some("linux"),
                Some((130, 0)),
            ),
            (
                "Mozilla/5.0 (X11; Ubuntu; Linux x86_64) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.0 Safari/605.1.15",
                Engine::WebKit,
                Some("linux"),
                Some((17, 0)),
            ),
            (
                "Mozilla/5.0 (X11; CrOS x86_64 14541.0.0) AppleWebKit/537.36 (KHTML, like Gecko) HeadlessChrome/127.0.0.0 Safari/537.36",
                Engine::Chromium,
                Some("chromeos"),
                Some((127, 0)),
            ),
            ("curl/8.7.1", Engine::Unknown, None, None),
            ("", Engine::Unknown, None, None),
            (
                "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko)",
                Engine::Unknown,
                Some("macos"),
                None,
            ),
            ("Firefox/", Engine::Gecko, None, None),
        ];
        for (user_agent, engine, platform, version) in cases {
            assert_eq!(
                engine_from_user_agent(user_agent),
                EngineId {
                    engine,
                    platform: platform.map(str::to_string),
                    version,
                },
                "{user_agent}"
            );
        }
        assert_eq!(parse_version("12", '.'), Some((12, 0)));
        assert_eq!(parse_version("12.x", '.'), Some((12, 0)));
        assert_eq!(parse_version("9999999999.1", '.'), None);
        assert_eq!(EngineId::unknown().engine, Engine::Unknown);
    }

    #[test]
    fn shipped_engine_gates_are_empty_and_open_every_engine() {
        let rows = parse_engine_gate_rows(ENGINE_GATES_JSON).unwrap();
        assert!(rows.is_empty());
        for user_agent in [
            "Mozilla/5.0 (iPhone; CPU iPhone OS 15_0 like Mac OS X) AppleWebKit/605.1.15",
            "Mozilla/5.0 (X11; Linux x86_64; rv:115.0) Gecko/20100101 Firefox/115.0",
            "anything",
        ] {
            assert_eq!(
                widget_engine_gate(&engine_from_user_agent(user_agent)),
                EngineGate::OPEN
            );
        }
        assert_eq!(EngineGate::default(), EngineGate::OPEN);
        assert_eq!(
            serde_json::to_value(EngineGate::OPEN.support()).unwrap(),
            json!({ "wildcardSources": true, "runtimeSources": true, "localMedia": true })
        );
    }

    #[test]
    fn engine_gate_rows_are_denylists_matched_by_engine_platform_and_version() {
        let rows = parse_engine_gate_rows(
            r#"{"rows": [
                {"engine": "webkit", "platform": "linux", "maxVersion": [2, 42], "deny": ["localMedia"]},
                {"engine": "webkit", "minVersion": [15, 0], "maxVersion": [16, 3], "deny": ["wildcardSources"]},
                {"engine": "chromium", "deny": ["runtimeSources", "declaredSources"]}
            ]}"#,
        )
        .unwrap();
        let id = |engine, platform: Option<&str>, version| EngineId {
            engine,
            platform: platform.map(str::to_string),
            version,
        };
        let gate = |id: EngineId| gate_from_rows(&rows, &id);

        assert_eq!(
            gate(id(Engine::WebKit, Some("linux"), Some((2, 40)))),
            EngineGate {
                local_media: false,
                ..EngineGate::OPEN
            }
        );
        assert_eq!(
            gate(id(Engine::WebKit, Some("linux"), Some((2, 44)))),
            EngineGate::OPEN
        );
        assert_eq!(
            gate(id(Engine::WebKit, Some("linux"), None)),
            EngineGate::OPEN
        );
        assert_eq!(
            gate(id(Engine::WebKit, Some("ios"), Some((16, 3)))),
            EngineGate {
                wildcard_sources: false,
                ..EngineGate::OPEN
            }
        );
        assert_eq!(
            gate(id(Engine::WebKit, Some("ios"), Some((16, 4)))),
            EngineGate::OPEN
        );
        assert_eq!(
            gate(id(Engine::WebKit, Some("macos"), Some((14, 9)))),
            EngineGate::OPEN
        );
        let chromium = gate(id(Engine::Chromium, None, None));
        assert!(!chromium.runtime_sources && !chromium.declared_sources);
        assert!(chromium.wildcard_sources && chromium.local_media);
        assert!(!chromium.allows(EngineFeature::RuntimeSources));
        assert!(chromium.allows(EngineFeature::LocalMedia));
        assert_eq!(gate(id(Engine::Gecko, None, None)), EngineGate::OPEN);
        assert_eq!(gate(EngineId::unknown()), EngineGate::OPEN);

        for invalid in [
            r#"{"rows": [{"engine": "unknown", "deny": ["localMedia"]}]}"#,
            r#"{"rows": [{"engine": "gecko", "deny": []}]}"#,
            r#"{"rows": [{"engine": "gecko", "minVersion": [3, 0], "maxVersion": [2, 0], "deny": ["localMedia"]}]}"#,
            r#"{"rows": [{"engine": "gecko", "allow": ["localMedia"], "deny": ["localMedia"]}]}"#,
            r#"{"rows": [{"engine": "presto", "deny": ["localMedia"]}]}"#,
            r#"{"rows": [{"engine": "gecko", "deny": ["frameSources"]}]}"#,
            r#"{"gates": []}"#,
        ] {
            assert!(parse_engine_gate_rows(invalid).is_err(), "{invalid}");
        }
        assert!(!EngineGate::CLOSED.support().runtime_sources);
    }

    #[test]
    fn engine_narrowing_only_removes_sources() {
        let declared = WidgetPolicy {
            workers: true,
            media: true,
            csp: csp(
                &["https://*.s3.example.com", "https://api.cesium.com"],
                &["https://*.s3.example.com"],
            ),
            ..WidgetPolicy::default()
        };
        let effective = WidgetPolicy {
            csp: csp(
                &[
                    "https://*.s3.example.com",
                    "https://a.tiles.example.com",
                    "https://api.cesium.com",
                ],
                &["https://*.s3.example.com", "https://a.tiles.example.com"],
            ),
            ..declared.clone()
        };
        assert_eq!(
            narrow_policy_for_engine(&effective, &declared, &EngineGate::OPEN),
            effective
        );

        let no_runtime = narrow_policy_for_engine(
            &effective,
            &declared,
            &EngineGate {
                runtime_sources: false,
                ..EngineGate::OPEN
            },
        );
        assert_eq!(no_runtime, declared);

        let no_wildcards = narrow_policy_for_engine(
            &effective,
            &declared,
            &EngineGate {
                wildcard_sources: false,
                ..EngineGate::OPEN
            },
        );
        assert_eq!(
            no_wildcards.csp,
            csp(
                &["https://a.tiles.example.com", "https://api.cesium.com"],
                &["https://a.tiles.example.com"]
            )
        );

        let no_declared = narrow_policy_for_engine(
            &effective,
            &declared,
            &EngineGate {
                declared_sources: false,
                ..EngineGate::OPEN
            },
        );
        assert_eq!(
            no_declared.csp,
            csp(
                &["https://a.tiles.example.com"],
                &["https://a.tiles.example.com"]
            )
        );
        assert!(no_declared.workers && no_declared.media);

        let closed = narrow_policy_for_engine(&effective, &declared, &EngineGate::CLOSED);
        assert!(closed.csp.is_empty());
        assert!(closed.workers && closed.media);
        for gate in [EngineGate::OPEN, EngineGate::CLOSED] {
            let narrowed = narrow_policy_for_engine(&effective, &declared, &gate);
            for (directive, source) in narrowed.csp.entries() {
                assert!(effective.csp.sources(directive).iter().any(|s| s == source));
            }
        }
    }
}
