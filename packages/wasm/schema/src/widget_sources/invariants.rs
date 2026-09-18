use std::collections::HashSet;

use super::WIDGET_SOURCE_ABOUT_KEYS;
use super::catalog::{
    MAX_PROVIDER_NAME_CHARS, MAX_PROVIDERS, domain_shape_error, is_about_key_shape, is_kebab_case,
};
use super::index::SourceIndex;
use super::text::{contains_address, has_mixed_script, has_only_allowed_characters};

/// §14.3.2 catalog invariants over one loaded data set.
pub(super) fn check(index: &SourceIndex) -> Result<(), Vec<String>> {
    let catalog = &index.catalog;
    let mut errors = Vec::new();

    if catalog.public_suffix_rules.sha256 != index.rules_sha256 {
        errors.push(format!(
            "publicSuffixRules.sha256 {} does not match public_suffix_rules.txt ({}); run mise run widget-sources:generate",
            catalog.public_suffix_rules.sha256, index.rules_sha256
        ));
    }
    if catalog.public_suffix_rules.psl_version != index.psl_version {
        errors.push(format!(
            "publicSuffixRules.pslVersion {} does not match public_suffix_rules.txt ({}); run mise run widget-sources:generate",
            catalog.public_suffix_rules.psl_version, index.psl_version
        ));
    }
    if catalog.providers_sha256 != index.providers_sha256 {
        errors.push(format!(
            "providersSha256 {} does not match the providers ({}); run mise run widget-sources:generate",
            catalog.providers_sha256, index.providers_sha256
        ));
    }
    if catalog.providers.len() > MAX_PROVIDERS {
        errors.push(format!(
            "catalog has {} providers; at most {MAX_PROVIDERS} are allowed",
            catalog.providers.len()
        ));
    }

    let mut ids = HashSet::new();
    let mut matches = HashSet::new();
    for provider in &catalog.providers {
        let id = &provider.id;
        if !is_kebab_case(id) {
            errors.push(format!("provider id {id:?} is not kebab-case"));
        }
        if !ids.insert(id.as_str()) {
            errors.push(format!("provider id {id:?} is not unique"));
        }
        let name_chars = provider.name.chars().count();
        if !(1..=MAX_PROVIDER_NAME_CHARS).contains(&name_chars) {
            errors.push(format!(
                "provider {id}: name has {name_chars} characters (1-{MAX_PROVIDER_NAME_CHARS})"
            ));
        }
        if !has_only_allowed_characters(&provider.name) {
            errors.push(format!(
                "provider {id}: name contains a forbidden character"
            ));
        }
        if has_mixed_script(&provider.name) {
            errors.push(format!("provider {id}: name mixes scripts"));
        }
        if contains_address(&provider.name, |label| index.is_icann_tld(label)) {
            errors.push(format!("provider {id}: name contains an address"));
        }
        if let Some(key) = &provider.about_key
            && !(is_about_key_shape(key) && WIDGET_SOURCE_ABOUT_KEYS.contains(&key.as_str()))
        {
            errors.push(format!("provider {id}: unknown aboutKey {key:?}"));
        }
        if provider.docs.is_empty() {
            errors.push(format!(
                "provider {id}: docs must list at least one https URL"
            ));
        }
        for doc in &provider.docs {
            if !doc.starts_with("https://") || doc.len() <= "https://".len() {
                errors.push(format!(
                    "provider {id}: docs entry {doc:?} is not an https URL"
                ));
            }
        }
        if provider.matches.is_empty() {
            errors.push(format!("provider {id}: match is empty"));
        }
        for matched in &provider.matches {
            let domain = &matched.domain;
            if !matches.insert((matched.kind, domain.as_str())) {
                errors.push(format!("provider {id}: match {domain:?} repeats"));
            }
            if let Some(problem) = domain_shape_error(matched.kind, domain) {
                errors.push(format!("provider {id}: domain {domain:?}: {problem}"));
            } else if index.is_icann_suffix(literal_tail(domain)) {
                errors.push(format!(
                    "provider {id}: domain {domain:?} is an ICANN public suffix"
                ));
            }
            if provider.receives.is_none() {
                errors.push(format!("provider {id}: {domain:?} requires receives"));
            }
            if matched.kind.is_service() && provider.user_content.is_none() {
                errors.push(format!("provider {id}: {domain:?} requires userContent"));
            }
        }
    }

    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}

fn literal_tail(domain: &str) -> &str {
    domain
        .rfind("*.")
        .map_or(domain, |index| &domain[index + 2..])
}
