use std::collections::HashSet;

use super::catalog::{CatalogFile, MatchType, Provider, parse_catalog, sha256_hex};
use super::rules::{
    RuleForm, RuleSection, SuffixRules, label_count, last_labels, parse_rules_file,
    strict_ancestors,
};
use super::{SourceProvenance, WidgetSourceClass, WidgetSourceKind, WidgetSourceLevel};

#[derive(Debug)]
pub(super) struct CatalogEntry {
    pub provider: usize,
    pub kind: MatchType,
    pub domain: String,
    pub receives: bool,
    pub user_content: bool,
}

#[derive(Debug, Clone, Copy)]
pub(super) struct SuffixMatch {
    pub labels: usize,
    pub receives: bool,
    pub entry: Option<usize>,
}

#[derive(Debug, Clone, Copy)]
pub(super) struct PathMatch {
    pub labels: usize,
    pub subtree: bool,
    pub entry: usize,
}

#[derive(Debug, Clone, Copy)]
pub(super) struct BelowMatch {
    pub level: WidgetSourceLevel,
    pub entry: Option<usize>,
}

#[derive(Debug)]
pub(super) struct SourceIndex {
    pub psl_version: String,
    pub psl_unix_secs: i64,
    pub rules_sha256: String,
    pub providers_sha256: String,
    pub catalog: CatalogFile,
    pub entries: Vec<CatalogEntry>,
    icann: SuffixRules,
    private: SuffixRules,
    icann_tlds: HashSet<String>,
    icann_below: HashSet<String>,
    private_below: HashSet<String>,
}

impl SourceIndex {
    pub fn build(rules_text: &str, catalog_text: &str) -> Result<Self, String> {
        let rules_file = parse_rules_file(rules_text)?;
        let (catalog, providers_sha256) = parse_catalog(catalog_text)?;
        let entries = catalog_entries(&catalog.providers);
        let receiving_attributions: Vec<String> = entries
            .iter()
            .filter(|entry| entry.kind == MatchType::Attribution && entry.receives)
            .map(|entry| entry.domain.clone())
            .collect();

        let mut icann = SuffixRules::default();
        let mut private = SuffixRules::default();
        let mut icann_tlds = HashSet::new();
        for rule in &rules_file.rules {
            match rule.section {
                RuleSection::Icann => {
                    icann.insert(rule);
                    icann_tlds.insert(last_labels(&rule.domain, 1).to_string());
                }
                RuleSection::Private
                    if !receiving_attributions
                        .iter()
                        .any(|attribution| at_or_under(&rule.domain, attribution)) =>
                {
                    private.insert(rule);
                }
                RuleSection::Private => {}
            }
        }

        let mut index = Self {
            psl_version: rules_file.psl_version,
            psl_unix_secs: rules_file.psl_unix_secs,
            rules_sha256: sha256_hex(rules_text.as_bytes()),
            providers_sha256,
            catalog,
            entries,
            icann,
            private,
            icann_tlds,
            icann_below: HashSet::new(),
            private_below: HashSet::new(),
        };
        index.icann_below = index.below_set(&rules_file.rules, RuleSection::Icann, &[]);
        index.private_below = index.below_set(
            &rules_file.rules,
            RuleSection::Private,
            &receiving_attributions,
        );
        Ok(index)
    }

    fn below_set(
        &self,
        rules: &[super::rules::Rule],
        section: RuleSection,
        excluded_under: &[String],
    ) -> HashSet<String> {
        let mut below = HashSet::new();
        for rule in rules.iter().filter(|rule| rule.section == section) {
            if excluded_under
                .iter()
                .any(|attribution| at_or_under(&rule.domain, attribution))
            {
                continue;
            }
            let own = (rule.form == RuleForm::Wildcard).then_some(rule.domain.as_str());
            let candidates = match rule.form {
                RuleForm::Exception => continue,
                _ => own.into_iter().chain(strict_ancestors(&rule.domain)),
            };
            for candidate in candidates {
                if !self.is_icann_suffix(candidate) {
                    below.insert(candidate.to_string());
                }
            }
        }
        below
    }

    pub fn provider(&self, entry: usize) -> &Provider {
        &self.catalog.providers[self.entries[entry].provider]
    }

    pub fn icann_suffix<'a>(&self, host: &'a str) -> &'a str {
        let labels = self.icann.suffix_labels(host, true).unwrap_or(1);
        last_labels(host, labels)
    }

    pub fn is_icann_suffix(&self, host: &str) -> bool {
        self.icann_suffix(host).len() == host.len()
    }

    pub fn icann_registrable<'a>(&self, host: &'a str) -> Option<&'a str> {
        let labels = self.icann.suffix_labels(host, true).unwrap_or(1);
        (label_count(host) > labels).then(|| last_labels(host, labels + 1))
    }

    pub fn is_icann_below(&self, host: &str) -> bool {
        self.icann_below.contains(host)
    }

    pub fn is_icann_tld(&self, label: &str) -> bool {
        self.icann_tlds.contains(label)
    }

    /// Level of a wildcard over `host` that spans shared suffixes, shared
    /// hosts or a receiving attribution; named only when one provider applies.
    pub fn shared_below(&self, host: &str) -> Option<BelowMatch> {
        let host_labels = label_count(host);
        let mut level = self
            .private_below
            .contains(host)
            .then_some(WidgetSourceLevel::Broad);
        let mut first_entry = None;
        let mut providers = HashSet::new();
        for (index, entry) in self.entries.iter().enumerate() {
            let contributes = if entry.kind.is_shared() {
                label_count(&entry.domain) > host_labels
                    && pattern_matches_tail(&entry.domain, host)
            } else {
                entry.kind == MatchType::Attribution && entry.receives && entry.domain == host
            };
            if contributes {
                level = level.max(Some(shared_level(entry.receives)));
                providers.insert(entry.provider);
                first_entry.get_or_insert(index);
            }
        }
        level.map(|level| BelowMatch {
            level,
            entry: first_entry.filter(|_| providers.len() == 1),
        })
    }

    pub fn shared_suffix_of(&self, host: &str) -> Option<SuffixMatch> {
        let host_labels = label_count(host);
        let catalog = self
            .entries
            .iter()
            .enumerate()
            .filter(|(_, entry)| entry.kind == MatchType::SharedSuffix)
            .filter(|(_, entry)| {
                label_count(&entry.domain) <= host_labels
                    && pattern_matches_tail(&entry.domain, host)
            })
            .map(|(index, entry)| (label_count(&entry.domain), entry.receives, index))
            .fold(
                None,
                |best: Option<(usize, bool, usize)>, candidate| match best {
                    Some(best) if best.0 >= candidate.0 => Some(best),
                    _ => Some(candidate),
                },
            );
        let private = self.private.suffix_labels(host, false);
        match (catalog, private) {
            (Some((labels, receives, entry)), private)
                if private.is_none_or(|private| labels >= private) =>
            {
                Some(SuffixMatch {
                    labels,
                    receives,
                    entry: Some(entry),
                })
            }
            (catalog, Some(labels)) => Some(SuffixMatch {
                labels,
                receives: true,
                entry: catalog.map(|(_, _, entry)| entry),
            }),
            _ => None,
        }
    }

    pub fn shared_path_of(&self, host: &str, wildcard: bool) -> Option<PathMatch> {
        self.longest_entry(host, |kind| match kind {
            MatchType::SharedHost if !wildcard => Some(false),
            MatchType::SharedSubtree => Some(true),
            _ => None,
        })
        .map(|(entry, subtree)| PathMatch {
            labels: label_count(&self.entries[entry].domain),
            subtree,
            entry,
        })
    }

    pub fn service_of(&self, host: &str, wildcard: bool) -> Option<usize> {
        self.longest_entry(host, |kind| match kind {
            MatchType::ServiceHost if !wildcard => Some(false),
            MatchType::ServiceSubtree => Some(true),
            _ => None,
        })
        .map(|(entry, _)| entry)
    }

    pub fn attribution_of(&self, host: &str) -> Option<usize> {
        self.longest_entry(host, |kind| {
            (kind == MatchType::Attribution).then_some(true)
        })
        .map(|(entry, _)| entry)
    }

    /// Longest entry whose domain equals `host` (`Some(false)` kinds) or
    /// covers it (`Some(true)` kinds); on equal length a covering entry wins.
    fn longest_entry(
        &self,
        host: &str,
        subtree_kind: impl Fn(MatchType) -> Option<bool>,
    ) -> Option<(usize, bool)> {
        let host_labels = label_count(host);
        self.entries
            .iter()
            .enumerate()
            .filter_map(|(index, entry)| {
                let subtree = subtree_kind(entry.kind)?;
                let entry_labels = label_count(&entry.domain);
                let matches = if subtree {
                    entry_labels <= host_labels && pattern_matches_tail(&entry.domain, host)
                } else {
                    entry_labels == host_labels && pattern_matches_tail(&entry.domain, host)
                };
                matches.then_some((entry_labels, subtree, index))
            })
            .fold(
                None,
                |best: Option<(usize, bool, usize)>, candidate| match best {
                    Some(best) if (best.0, best.1) >= (candidate.0, candidate.1) => Some(best),
                    _ => Some(candidate),
                },
            )
            .map(|(_, subtree, index)| (index, subtree))
    }

    pub fn classify(
        &self,
        host: &str,
        wildcard: bool,
        provenance: SourceProvenance,
    ) -> WidgetSourceClass {
        use WidgetSourceKind as Kind;
        use WidgetSourceLevel as Level;

        let base = host.strip_prefix("*.").unwrap_or(host);
        let base_labels = label_count(base);
        let path = self.shared_path_of(base, wildcard);
        let suffix = self.shared_suffix_of(base);
        let service = self.service_of(base, wildcard);
        let attribution = self.attribution_of(base);
        let attribution_receives = attribution.is_some_and(|entry| self.entries[entry].receives);
        let registrable = || self.icann_registrable(base).unwrap_or(base);
        let tenant = |suffix: SuffixMatch| last_labels(base, suffix.labels + 1);
        let path_level = |path: PathMatch| shared_level(self.entries[path.entry].receives);
        let service_level = |entry: usize| {
            let entry = &self.entries[entry];
            let mut level = Level::Known;
            if entry.receives {
                level = level.max(Level::External);
            }
            if entry.user_content {
                level = level.max(Level::Shared);
            }
            level
        };
        let suffix_at_base = suffix.filter(|suffix| suffix.labels == base_labels);
        let (kind, level, emphasis, entry) = if wildcard {
            if let Some(path) = path.filter(|path| path.subtree) {
                (
                    Kind::SharedWildcard,
                    path_level(path),
                    base,
                    Some(path.entry),
                )
            } else if let Some(suffix) = suffix_at_base {
                (
                    Kind::SharedWildcard,
                    shared_level(suffix.receives),
                    base,
                    suffix.entry,
                )
            } else if let Some(below) = self.shared_below(base) {
                (Kind::SharedWildcard, below.level, base, below.entry)
            } else if let Some(suffix) = suffix {
                (
                    Kind::TenantSubdomains,
                    Level::External,
                    tenant(suffix),
                    suffix.entry,
                )
            } else if let Some(service) = service {
                (
                    Kind::Service,
                    service_level(service),
                    registrable(),
                    Some(service),
                )
            } else if attribution_receives {
                (Kind::SharedWildcard, Level::Broad, base, attribution)
            } else {
                (Kind::Subdomains, Level::External, registrable(), None)
            }
        } else if let Some(path) =
            path.filter(|path| path.labels >= suffix.map_or(0, |suffix| suffix.labels))
        {
            (Kind::SharedHost, path_level(path), base, Some(path.entry))
        } else if let Some(suffix) = suffix_at_base {
            (
                Kind::SharedHost,
                shared_level(suffix.receives),
                base,
                suffix.entry,
            )
        } else if let Some(service) = service {
            (
                Kind::Service,
                service_level(service),
                registrable(),
                Some(service),
            )
        } else if let Some(suffix) = suffix {
            (
                Kind::TenantHost,
                Level::External,
                tenant(suffix),
                suffix.entry,
            )
        } else if attribution_receives {
            (Kind::PlatformHost, Level::Broad, base, attribution)
        } else {
            (Kind::Exact, Level::External, registrable(), None)
        };

        let level = match provenance {
            SourceProvenance::Runtime => level.max(Level::External),
            SourceProvenance::Declared => level,
        };
        let provider = entry.or(attribution).map(|entry| self.provider(entry));
        WidgetSourceClass {
            kind,
            level,
            host: host.to_string(),
            emphasis: emphasis.to_string(),
            provider_id: provider.map(|provider| provider.id.clone()),
            provider: provider.map(|provider| provider.name.clone()),
            about_key: provider.and_then(|provider| provider.about_key.clone()),
        }
    }
}

fn catalog_entries(providers: &[Provider]) -> Vec<CatalogEntry> {
    providers
        .iter()
        .enumerate()
        .flat_map(|(provider_index, provider)| {
            provider.matches.iter().map(move |matched| CatalogEntry {
                provider: provider_index,
                kind: matched.kind,
                domain: matched.domain.clone(),
                receives: provider.receives.unwrap_or(true),
                user_content: provider.user_content.unwrap_or(true),
            })
        })
        .collect()
}

fn shared_level(receives: bool) -> WidgetSourceLevel {
    if receives {
        WidgetSourceLevel::Broad
    } else {
        WidgetSourceLevel::Shared
    }
}

fn at_or_under(host: &str, domain: &str) -> bool {
    host == domain
        || host
            .strip_suffix(domain)
            .is_some_and(|prefix| prefix.ends_with('.'))
}

/// Whether the rightmost labels of `host` match every label of `pattern`
/// (`*` matches one label), or, when `host` is shorter, whether all of
/// `host` matches the rightmost labels of `pattern`.
fn pattern_matches_tail(pattern: &str, host: &str) -> bool {
    pattern
        .rsplit('.')
        .zip(host.rsplit('.'))
        .all(|(pattern_label, host_label)| pattern_label == "*" || pattern_label == host_label)
}
