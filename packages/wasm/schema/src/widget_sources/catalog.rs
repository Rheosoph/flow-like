use serde::Deserialize;
use sha2::{Digest, Sha256};

pub(super) const MAX_PROVIDERS: usize = 300;
pub(super) const MAX_PROVIDER_NAME_CHARS: usize = 40;

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct CatalogFile {
    pub catalog_version: u32,
    pub providers_sha256: String,
    pub public_suffix_rules: RulesPin,
    pub providers: Vec<Provider>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct RulesPin {
    pub psl_version: String,
    pub sha256: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct Provider {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub about_key: Option<String>,
    pub docs: Vec<String>,
    #[serde(rename = "match")]
    pub matches: Vec<ProviderMatch>,
    #[serde(default)]
    pub receives: Option<bool>,
    #[serde(default)]
    pub user_content: Option<bool>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ProviderMatch {
    #[serde(rename = "type")]
    pub kind: MatchType,
    pub domain: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(super) enum MatchType {
    ServiceHost,
    ServiceSubtree,
    SharedSuffix,
    SharedHost,
    SharedSubtree,
    Attribution,
}

impl MatchType {
    pub fn is_service(self) -> bool {
        matches!(self, MatchType::ServiceHost | MatchType::ServiceSubtree)
    }

    pub fn is_shared(self) -> bool {
        matches!(
            self,
            MatchType::SharedSuffix | MatchType::SharedHost | MatchType::SharedSubtree
        )
    }
}

pub(super) fn parse_catalog(text: &str) -> Result<(CatalogFile, String), String> {
    let value: serde_json::Value = serde_json::from_str(text)
        .map_err(|error| format!("widget_source_catalog.json is not JSON: {error}"))?;
    let providers_sha256 = value
        .get("providers")
        .map(|providers| sha256_hex(canonical_json(providers).as_bytes()))
        .ok_or_else(|| "widget_source_catalog.json has no providers".to_string())?;
    let catalog = serde_json::from_value(value)
        .map_err(|error| format!("widget_source_catalog.json is malformed: {error}"))?;
    Ok((catalog, providers_sha256))
}

pub(super) fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// Object keys sorted, no whitespace; mirrors `canonicalJson` in
/// `tools/widget-sources/lib.ts`.
pub(super) fn canonical_json(value: &serde_json::Value) -> String {
    let mut out = String::new();
    write_canonical_json(value, &mut out);
    out
}

fn write_canonical_json(value: &serde_json::Value, out: &mut String) {
    match value {
        serde_json::Value::Object(map) => {
            let mut entries: Vec<_> = map.iter().collect();
            entries.sort_by(|a, b| a.0.cmp(b.0));
            out.push('{');
            for (index, (key, entry)) in entries.into_iter().enumerate() {
                if index > 0 {
                    out.push(',');
                }
                out.push_str(&serde_json::Value::String(key.clone()).to_string());
                out.push(':');
                write_canonical_json(entry, out);
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
        other => out.push_str(&other.to_string()),
    }
}

pub(super) fn is_kebab_case(id: &str) -> bool {
    !id.is_empty()
        && id.split('-').all(|part| {
            !part.is_empty()
                && part
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
        })
}

pub(super) fn is_about_key_shape(key: &str) -> bool {
    key.strip_prefix("widgetSourceAbout").is_some_and(|rest| {
        rest.starts_with(|c: char| c.is_ascii_uppercase())
            && rest.chars().all(|c| c.is_ascii_alphanumeric())
    })
}

/// Structural problems of one catalog domain pattern, PSL-independent.
pub(super) fn domain_shape_error(kind: MatchType, domain: &str) -> Option<&'static str> {
    let labels: Vec<&str> = domain.split('.').collect();
    let dns_label = |label: &&str| {
        let bytes = label.as_bytes();
        let alnum = |b: &u8| b.is_ascii_lowercase() || b.is_ascii_digit();
        (1..=63).contains(&bytes.len())
            && alnum(&bytes[0])
            && alnum(&bytes[bytes.len() - 1])
            && bytes.iter().all(|b| alnum(b) || *b == b'-')
    };
    if !labels.iter().all(|label| *label == "*" || dns_label(label)) {
        return Some("labels must be lowercase DNS labels or *");
    }
    let literal_tail = labels
        .iter()
        .rev()
        .take_while(|label| **label != "*")
        .count();
    if literal_tail < 2 {
        return Some("needs at least 2 literal rightmost labels");
    }
    if labels.contains(&"*") && kind != MatchType::SharedSuffix {
        return Some("* is only allowed in shared-suffix domains");
    }
    None
}
