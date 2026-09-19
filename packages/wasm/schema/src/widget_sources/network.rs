use std::collections::BTreeMap;

use super::index::SourceIndex;
use super::{
    NetworkPurposeInput, NetworkRuntimeInput, SourceProvenance, WIDGET_SOURCE_STALE_AFTER_SECS,
    WidgetNetwork, WidgetNetworkPurpose, WidgetNetworkSource, WidgetSourceClass, WidgetSourceKind,
    WidgetSourceLevel, classify_with,
};
use crate::widget_policy::{CspDirective, csp_source_host, is_path_source};

pub(super) fn describe_network_with(
    index: &SourceIndex,
    purposes: &[NetworkPurposeInput],
    runtime: &[NetworkRuntimeInput],
    now_unix_secs: i64,
) -> Option<WidgetNetwork> {
    if purposes.is_empty() || runtime.iter().any(|input| input.purpose >= purposes.len()) {
        return None;
    }
    let stale = now_unix_secs.saturating_sub(index.psl_unix_secs) > WIDGET_SOURCE_STALE_AFTER_SECS;
    let purposes = purposes
        .iter()
        .enumerate()
        .map(|(position, purpose)| describe_purpose(index, position, purpose, runtime, stale))
        .collect::<Option<Vec<_>>>()?;
    let level = purposes
        .iter()
        .map(|purpose| purpose.level)
        .max()
        .unwrap_or(WidgetSourceLevel::Known);
    Some(WidgetNetwork {
        level,
        catalog_version: index.catalog.catalog_version,
        psl_version: index.psl_version.clone(),
        stale,
        purposes,
    })
}

fn describe_purpose(
    index: &SourceIndex,
    position: usize,
    purpose: &NetworkPurposeInput,
    runtime: &[NetworkRuntimeInput],
    stale: bool,
) -> Option<WidgetNetworkPurpose> {
    let mut declared: BTreeMap<&str, Vec<CspDirective>> = BTreeMap::new();
    for (directive, source) in &purpose.declared {
        declared.entry(source).or_default().push(*directive);
    }
    let mut accepted: BTreeMap<(&str, &str), Vec<CspDirective>> = BTreeMap::new();
    for input in runtime.iter().filter(|input| input.purpose == position) {
        accepted
            .entry((&input.slot, &input.source))
            .or_default()
            .push(input.directive);
    }

    let declared = declared
        .into_iter()
        .map(|(source, directives)| (source, directives, SourceProvenance::Declared, None));
    let accepted = accepted.into_iter().map(|((slot, source), directives)| {
        (source, directives, SourceProvenance::Runtime, Some(slot))
    });
    let sources = declared
        .chain(accepted)
        .map(|(source, mut directives, origin, slot)| {
            let mut class = if origin == SourceProvenance::Runtime && is_path_source(source) {
                platform_storage_class(source)?
            } else {
                classify_with(index, source, origin).ok()?
            };
            if stale
                && matches!(
                    class.kind,
                    WidgetSourceKind::Subdomains | WidgetSourceKind::TenantSubdomains
                )
            {
                class.level = WidgetSourceLevel::Broad;
            }
            directives.sort();
            directives.dedup();
            Some(WidgetNetworkSource {
                source: source.to_string(),
                directives,
                origin,
                slot: slot.map(str::to_string),
                class,
            })
        })
        .collect::<Option<Vec<_>>>()?;

    let floor = if purpose.inputs.is_empty() {
        WidgetSourceLevel::Known
    } else {
        WidgetSourceLevel::External
    };
    let level = sources
        .iter()
        .map(|source| source.class.level)
        .max()
        .unwrap_or(floor);
    Some(WidgetNetworkPurpose {
        reason: purpose.reason.clone(),
        level,
        inputs: purpose.inputs.clone(),
        sources,
    })
}

/// Runtime path sources are only ever the server-derived scope of this app's
/// Flow-Like storage (§14.4.5), which reaches nothing but this app's files.
fn platform_storage_class(source: &str) -> Option<WidgetSourceClass> {
    let host = csp_source_host(source)?.to_string();
    Some(WidgetSourceClass {
        kind: WidgetSourceKind::PlatformStorage,
        level: WidgetSourceLevel::Known,
        emphasis: host.clone(),
        host,
        provider_id: None,
        provider: None,
        about_key: None,
    })
}
