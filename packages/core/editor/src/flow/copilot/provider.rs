use async_trait::async_trait;
use flow_like_ast::model::{Container, TypeRef};
use flow_like_ast::{SigParam, Signature, to_camel_case};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use super::declarations::{
    DeclarationMatch, declaration_semantic_evidence, render_declaration_matches,
    search_declarations,
};
use super::search::score_catalog_metadata;
use super::types::{NodeMetadata, PinMetadata};
use crate::flow::ast::catalog_names;
use crate::flow::node::Node;
use crate::flow::pin::{Pin, PinType};
use crate::flow::variable::VariableType;

const MAX_DECLARATION_USAGE_NOTE_BYTES: usize = 6_000;
const MAX_DECLARATION_USAGE_SCHEMA_PINS: usize = 4;
const MAX_DECLARATION_USAGE_SCHEMA_FIELDS: usize = 16;
const MAX_DECLARATION_USAGE_COMPANIONS: usize = 8;
pub(crate) const MAX_DECLARATION_PRIORITY_BLOCK_BYTES: usize = 630;

pub(crate) const DECLARATION_PRIORITY_BEGIN: &str = "// <flowpilot-declaration-priority>\n";
pub(crate) const DECLARATION_PRIORITY_END: &str = "// </flowpilot-declaration-priority>\n";
pub(crate) const DECLARATION_RESOLUTION_PREFIX: &str = "// flowpilot.declaration-resolution/v1 ";

const MIN_DECLARATION_RESOLUTION_SCORE: i32 = 90;
const STRONG_DECLARATION_RESOLUTION_SCORE: i32 = 180;
const MIN_DECLARATION_RESOLUTION_MARGIN: i32 = 20;
const MAX_DECLARATION_RESOLUTION_CANDIDATES: usize = 5;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum DeclarationResolutionStatus {
    Exact,
    Resolved,
    Ambiguous,
    Unresolved,
}

impl DeclarationResolutionStatus {
    pub(crate) fn is_confident(self) -> bool {
        matches!(self, Self::Exact | Self::Resolved)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct DeclarationResolutionCandidate {
    pub function_name: String,
    pub node_type: String,
    pub score: i32,
    pub confidence_basis_points: u16,
    pub accepted: bool,
    pub reason_codes: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct DeclarationResolutionMetadata {
    pub query: String,
    pub status: DeclarationResolutionStatus,
    pub top_score: Option<i32>,
    pub runner_up_score: Option<i32>,
    pub margin: Option<i32>,
    pub reason_codes: Vec<String>,
    pub candidates: Vec<DeclarationResolutionCandidate>,
}

pub(crate) fn parse_declaration_resolution_metadata(
    rendered: &str,
) -> Option<DeclarationResolutionMetadata> {
    rendered.lines().find_map(|line| {
        line.strip_prefix(DECLARATION_RESOLUTION_PREFIX)
            .and_then(|payload| serde_json::from_str(payload).ok())
    })
}

const IMAP_CHAIN_NOTE: &str = "// IMAP: `imap::inbox({ connection: connection, inbox: \"INBOX\" })` -> `imap::listMails({ inbox: inbox, filter: \"UNSEEN\" })`; loop `for (const ref of refs)`, then `imap::fetchMail({ emailRef: ref })` (or `ref.fetchMail()`); read with `email::getContent`, `email::getHeaders`, and `email::addressToFields`; after success use `imap::markSeen({ email: ref, markAsSeen: true })`.";

/// Trait for providing catalog search functionality
#[async_trait]
pub trait CatalogProvider: Send + Sync {
    async fn search(&self, query: &str) -> Vec<NodeMetadata>;
    async fn search_by_pin_type(&self, pin_type: &str, is_input: bool) -> Vec<NodeMetadata>;
    async fn filter_by_category(&self, category_prefix: &str) -> Vec<NodeMetadata>;
    async fn get_node_metadata(&self, node_type: &str) -> Option<NodeMetadata>;
    async fn get_all_nodes(&self) -> Vec<String>;

    /// Return metadata for the full catalog. FlowScript reconciliation uses this to resolve
    /// parsed calls (`ns::alias`, `x.alias()` or the legacy camelCase name) back to catalog node
    /// types without asking the model to manually emit command JSON.
    async fn get_all_metadata(&self) -> Vec<NodeMetadata> {
        let node_types = self.get_all_nodes().await;
        let mut metadata = Vec::with_capacity(node_types.len());
        for node_type in node_types {
            if let Some(meta) = self.get_node_metadata(&node_type).await {
                metadata.push(meta);
            }
        }
        metadata
    }

    /// Render `.flow.d`-style FlowScript declarations for nodes matching `query`.
    ///
    /// This is FlowPilot's type-reference lookup: instead of inspecting nodes pin-by-pin, the
    /// agent retrieves the exact `function ns::alias(this: T, { … }): R;` signatures (qualified
    /// name, receiver, typed params, `// impure` marker) for the nodes it wants to write in
    /// FlowScript, plus the `use ns::*` idiom that makes the bare alias resolve. The default
    /// implementation derives signatures from the same metadata `search` returns, so every
    /// provider — including ones that inject third-party packages into the catalog — supports it
    /// without extra wiring.
    async fn get_declarations(&self, query: &str) -> String {
        if query.trim().is_empty() {
            return render_declaration_matches(query, &search_declarations(query));
        }
        let snapshot = DeclarationCatalogSnapshot::new(self.get_all_metadata().await);
        render_declarations_from_snapshot(query, &snapshot)
    }

    /// Render several declaration queries against one immutable catalog snapshot.
    ///
    /// Catalog enumeration can involve package discovery and metadata I/O. Sharing both the
    /// metadata and its function-name index keeps a multi-query lookup coherent and avoids doing
    /// that work once per query. Providers that override declaration rendering should override
    /// this method as well if they need semantics other than the metadata-backed default.
    async fn get_declarations_batch(&self, queries: &[String]) -> Vec<String> {
        if queries.iter().all(|query| query.trim().is_empty()) {
            return queries
                .iter()
                .map(|query| render_declaration_matches(query, &search_declarations(query)))
                .collect();
        }
        let snapshot = DeclarationCatalogSnapshot::new(self.get_all_metadata().await);
        queries
            .iter()
            .map(|query| render_declarations_from_snapshot(query, &snapshot))
            .collect()
    }
}

struct DeclarationCatalogSnapshot {
    all_metadata: Vec<NodeMetadata>,
    function_names: Vec<String>,
    metadata_by_function: BTreeMap<String, Vec<usize>>,
    available_functions: BTreeMap<String, String>,
    imap_chain_compatible: bool,
}

impl DeclarationCatalogSnapshot {
    fn new(mut all_metadata: Vec<NodeMetadata>) -> Self {
        all_metadata.sort_by(|left, right| left.name.cmp(&right.name));
        let mut function_names = Vec::with_capacity(all_metadata.len());
        let mut metadata_by_function: BTreeMap<String, Vec<usize>> = BTreeMap::new();
        for (index, metadata) in all_metadata.iter().enumerate() {
            let signature = metadata_to_signature(metadata);
            let function_name = signature.qualified();
            // Every accepted spelling resolves to the node: qualified, legacy flat, node type.
            for spelling in [
                function_name.clone(),
                signature.display.clone(),
                metadata.name.clone(),
            ] {
                let indices = metadata_by_function.entry(spelling).or_default();
                if !indices.contains(&index) {
                    indices.push(index);
                }
            }
            function_names.push(function_name);
        }
        // Companion hints must obey the same ambiguity rule as declarations. Otherwise a direct
        // declaration can be correctly suppressed while a usage note still recommends the same
        // unresolvable FlowScript display name.
        let available_functions = all_metadata
            .iter()
            .zip(function_names.iter())
            .filter(|(_, function_name)| {
                metadata_by_function
                    .get(*function_name)
                    .is_some_and(|indices| indices.len() == 1)
            })
            .map(|(metadata, function_name)| (metadata.name.clone(), function_name.clone()))
            .collect();
        let imap_chain_compatible = imap_chain_is_compatible(&all_metadata);
        Self {
            all_metadata,
            function_names,
            metadata_by_function,
            available_functions,
            imap_chain_compatible,
        }
    }

    /// Live catalog rows for an embedded match, by node type first (the stable identity), then
    /// by the qualified and legacy spellings.
    fn indices_for(&self, matched: &DeclarationMatch) -> Option<&Vec<usize>> {
        [
            matched.node_type.as_str(),
            matched.function_name.as_str(),
            matched.flat.as_str(),
        ]
        .into_iter()
        .filter(|spelling| !spelling.is_empty())
        .find_map(|spelling| self.metadata_by_function.get(spelling))
    }
}

/// The live catalog's names and signature are authoritative for an embedded match.
fn refresh_match_from_signature(matched: &mut DeclarationMatch, signature: &Signature) {
    matched.function_name = signature.qualified();
    matched.node_type = signature.node_type.clone();
    matched.namespace = signature.namespace_path().join("::");
    matched.alias = signature.alias_name().to_string();
    matched.flat = signature.display.clone();
    matched.receiver = signature
        .receiver_param()
        .map(|param| to_camel_case(&param.name));
    matched.signature_line = signature.signature_line();
    matched.impure = signature.impure;
}

#[derive(Debug, Clone)]
struct AssessedDeclarationCandidate {
    metadata_index: usize,
    function_name: String,
    score: i32,
    confidence_basis_points: u16,
    accepted: bool,
    exact_symbol: bool,
    reason_codes: Vec<String>,
}

fn assess_declaration_candidate(
    query: &str,
    metadata_index: usize,
    snapshot: &DeclarationCatalogSnapshot,
) -> AssessedDeclarationCandidate {
    let metadata = &snapshot.all_metadata[metadata_index];
    let function_name = snapshot.function_names[metadata_index].clone();
    let evidence = declaration_semantic_evidence(
        query,
        &function_name,
        &metadata.name,
        &metadata.friendly_name,
        &metadata.description,
        metadata.category.as_deref(),
        &metadata.capability_tags,
    );
    let lexical_score = score_catalog_metadata(metadata, query).max(0);
    let mut score = lexical_score
        .saturating_add((evidence.strong_matched_token_count as i32).saturating_mul(40))
        .saturating_add(
            (evidence
                .matched_token_count
                .saturating_sub(evidence.strong_matched_token_count) as i32)
                .saturating_mul(15),
        );
    if evidence.exact_symbol {
        score = 100_000;
    }

    let lexical_component = ((score.clamp(0, 200) as u32) * 2_000 / 200) as u16;
    let confidence_basis_points = if evidence.exact_symbol {
        10_000
    } else {
        (((evidence.coverage_basis_points as u32) * 5_000 / 10_000)
            + ((evidence.strong_coverage_basis_points as u32) * 3_000 / 10_000)
            + lexical_component as u32)
            .min(10_000) as u16
    };
    let mut reason_codes = evidence.reason_codes.clone();
    let evidence_accepts = evidence.accepts();
    let score_accepts = score >= MIN_DECLARATION_RESOLUTION_SCORE;
    if score_accepts {
        reason_codes.push("calibrated_score_threshold".to_string());
    } else if !evidence.exact_symbol {
        reason_codes.push("lexical_score_below_threshold".to_string());
    }
    let accepted = evidence.exact_symbol || (evidence_accepts && score_accepts);

    AssessedDeclarationCandidate {
        metadata_index,
        function_name,
        score,
        confidence_basis_points,
        accepted,
        exact_symbol: evidence.exact_symbol,
        reason_codes,
    }
}

fn declaration_resolution_metadata(
    query: &str,
    assessments: &[AssessedDeclarationCandidate],
    has_ambiguous_live_symbol: bool,
    snapshot: &DeclarationCatalogSnapshot,
) -> DeclarationResolutionMetadata {
    let mut ranked = assessments.to_vec();
    ranked.sort_by(|left, right| {
        right
            .accepted
            .cmp(&left.accepted)
            .then_with(|| right.exact_symbol.cmp(&left.exact_symbol))
            .then_with(|| {
                right
                    .score
                    .cmp(&left.score)
                    .then_with(|| left.function_name.cmp(&right.function_name))
            })
    });
    let top_score = ranked.first().map(|candidate| candidate.score);
    let runner_up_score = ranked.get(1).map(|candidate| candidate.score);
    let margin = top_score.map(|top| top.saturating_sub(runner_up_score.unwrap_or_default()));
    let accepted = ranked
        .iter()
        .filter(|candidate| candidate.accepted)
        .collect::<Vec<_>>();
    let exact = accepted.iter().any(|candidate| candidate.exact_symbol);
    let weak_low_margin = accepted.first().is_some_and(|candidate| {
        candidate.score < STRONG_DECLARATION_RESOLUTION_SCORE
            && accepted.get(1).is_some()
            && margin.unwrap_or_default() < MIN_DECLARATION_RESOLUTION_MARGIN
    });
    let status = if exact {
        DeclarationResolutionStatus::Exact
    } else if accepted.is_empty() {
        if has_ambiguous_live_symbol {
            DeclarationResolutionStatus::Ambiguous
        } else {
            DeclarationResolutionStatus::Unresolved
        }
    } else if weak_low_margin {
        DeclarationResolutionStatus::Ambiguous
    } else {
        DeclarationResolutionStatus::Resolved
    };
    let mut reason_codes = ranked
        .first()
        .map(|candidate| candidate.reason_codes.clone())
        .unwrap_or_else(|| vec!["no_live_catalog_candidate".to_string()]);
    match status {
        DeclarationResolutionStatus::Exact => {
            reason_codes.push("unique_live_exact_symbol".to_string())
        }
        DeclarationResolutionStatus::Resolved => {
            reason_codes.push("confident_live_resolution".to_string())
        }
        DeclarationResolutionStatus::Ambiguous => {
            reason_codes.push("selection_ambiguous".to_string())
        }
        DeclarationResolutionStatus::Unresolved => {
            reason_codes.push("resolver_abstained".to_string())
        }
    }
    reason_codes.sort();
    reason_codes.dedup();

    DeclarationResolutionMetadata {
        query: query.to_string(),
        status,
        top_score,
        runner_up_score,
        margin,
        reason_codes,
        candidates: ranked
            .into_iter()
            .take(MAX_DECLARATION_RESOLUTION_CANDIDATES)
            .map(|candidate| DeclarationResolutionCandidate {
                function_name: candidate.function_name,
                node_type: snapshot.all_metadata[candidate.metadata_index].name.clone(),
                score: candidate.score,
                confidence_basis_points: candidate.confidence_basis_points,
                accepted: candidate.accepted,
                reason_codes: candidate.reason_codes,
            })
            .collect(),
    }
}

fn render_declaration_resolution_header(metadata: &DeclarationResolutionMetadata) -> String {
    let payload = serde_json::to_string(metadata).unwrap_or_else(|_| {
        r#"{"status":"unresolved","reason_codes":["resolution_metadata_serialization_failed"]}"#
            .to_string()
    });
    format!("{DECLARATION_RESOLUTION_PREFIX}{payload}\n")
}

fn render_declarations_from_snapshot(query: &str, snapshot: &DeclarationCatalogSnapshot) -> String {
    let embedded_matches = search_declarations(query);
    if query.trim().is_empty() {
        return render_declaration_matches(query, &embedded_matches);
    }

    let mut unavailable_function_names = Vec::new();
    let mut ambiguous_function_names = Vec::new();
    let mut assessments = Vec::new();
    let mut live_signature_override_count = 0usize;
    let mut embedded_node_types: HashSet<String> = HashSet::new();
    let mut declaration_matches = embedded_matches
        .into_iter()
        .filter_map(|mut matched| match snapshot.indices_for(&matched) {
            None => {
                unavailable_function_names.push(matched.function_name);
                None
            }
            Some(indices) if indices.len() == 1 => {
                let metadata_index = indices[0];
                let metadata = &snapshot.all_metadata[metadata_index];
                embedded_node_types.insert(metadata.name.clone());
                let assessment = assess_declaration_candidate(query, metadata_index, snapshot);
                let accepted = assessment.accepted;
                matched.score = assessment.score;
                assessments.push(assessment);
                if !accepted {
                    return None;
                }
                refresh_match_from_signature(&mut matched, &metadata_to_signature(metadata));
                live_signature_override_count += 1;
                Some(matched)
            }
            Some(_) => {
                ambiguous_function_names.push(matched.function_name);
                None
            }
        })
        .collect::<Vec<_>>();
    declaration_matches.sort_by(|left, right| {
        right
            .score
            .cmp(&left.score)
            .then_with(|| left.function_name.cmp(&right.function_name))
    });
    unavailable_function_names.sort();
    unavailable_function_names.dedup();
    ambiguous_function_names.sort();
    ambiguous_function_names.dedup();

    let mut live_matches: Vec<(i32, usize)> = snapshot
        .all_metadata
        .iter()
        .enumerate()
        .filter_map(|(index, meta)| {
            let function_name = &snapshot.function_names[index];
            if embedded_node_types.contains(&meta.name)
                || snapshot
                    .metadata_by_function
                    .get(function_name)
                    .is_some_and(|metadata| metadata.len() != 1)
            {
                return None;
            }
            let assessment = assess_declaration_candidate(query, index, snapshot);
            let score = assessment.score;
            let considered = score > 0;
            let accepted = assessment.accepted;
            if considered {
                assessments.push(assessment);
            }
            (considered && accepted).then_some((score, index))
        })
        .collect();
    live_matches.sort_by(|left, right| {
        right.0.cmp(&left.0).then_with(|| {
            snapshot.all_metadata[left.1]
                .name
                .cmp(&snapshot.all_metadata[right.1].name)
        })
    });
    let mut live_matches: Vec<NodeMetadata> = live_matches
        .into_iter()
        .take(12)
        .map(|(_, index)| snapshot.all_metadata[index].clone())
        .collect();

    let resolution = declaration_resolution_metadata(
        query,
        &assessments,
        !ambiguous_function_names.is_empty(),
        snapshot,
    );
    let resolution_header = render_declaration_resolution_header(&resolution);
    if !resolution.status.is_confident() {
        declaration_matches.clear();
        live_matches.clear();
        live_signature_override_count = 0;
    }

    if declaration_matches.is_empty() && live_matches.is_empty() {
        let mut out = render_declaration_matches(query, &[]);
        out.push_str(&format!(
            "\n// Calibrated resolver status: {:?}. No declaration is authorized for this query; refine the operation/service words or use an exact live function symbol.\n",
            resolution.status
        ));
        append_low_confidence_candidate_note(&mut out, &assessments);
        append_unavailable_declaration_note(&mut out, &unavailable_function_names);
        append_ambiguous_declaration_note(&mut out, &ambiguous_function_names);
        return format!("{resolution_header}{out}");
    }

    let mut out = if declaration_matches.is_empty() {
        format!(
            "// FlowScript declarations matched {query:?} from the live app catalog provider.\n\
                 // The embedded .flow.d index had no direct hit, so these compact signatures were rendered from metadata.\n\n",
        )
    } else {
        render_declaration_matches(query, &declaration_matches)
    };
    if live_signature_override_count > 0 {
        out.push_str(
            "// Same-name signatures above were verified against unique live catalog metadata; the live pin contract is authoritative.\n",
        );
    }
    append_unavailable_declaration_note(&mut out, &unavailable_function_names);
    append_ambiguous_declaration_note(&mut out, &ambiguous_function_names);

    if !live_matches.is_empty() && !declaration_matches.is_empty() {
        out.push_str(
            "\n// Additional live app catalog declarations, including installed package nodes:\n",
        );
    }
    if !live_matches.is_empty() {
        let live_namespaces = live_matches
            .iter()
            .map(|meta| metadata_to_signature(meta).namespace_path().join("::"))
            .filter(|namespace| !namespace.is_empty())
            .collect::<BTreeSet<_>>();
        for namespace in live_namespaces {
            out.push_str(&format!("// use {namespace}::*\n"));
        }
        out.push('\n');
    }

    let start_idx = if declaration_matches.is_empty() {
        0
    } else {
        declaration_matches.len()
    };
    for (idx, meta) in live_matches.iter().enumerate() {
        let signature = metadata_to_signature(meta);
        let qualified = signature.qualified();
        out.push_str(&format!(
            "{}. {} — {} [{}]\n",
            start_idx + idx + 1,
            qualified,
            signature
                .doc
                .as_deref()
                .map(compact_doc_line)
                .unwrap_or_else(|| signature
                    .friendly
                    .clone()
                    .unwrap_or_else(|| qualified.clone())),
            meta.category
                .clone()
                .unwrap_or_else(|| "catalog".to_string())
        ));
        out.push_str("   ");
        out.push_str(&metadata_signature_line(&signature));
        out.push('\n');
        if let Some(receiver) = signature.receiver_param() {
            out.push_str(&format!(
                "   // {qualified}({{ … }})  or  {}.{}(…)\n",
                to_camel_case(&receiver.name),
                signature.alias_name()
            ));
        }
        out.push('\n');
    }

    let mut usage_metadata = Vec::new();
    let mut seen_usage = HashSet::new();
    for matched in &declaration_matches {
        let Some(metadata) = snapshot.indices_for(matched).and_then(|indices| {
            (indices.len() == 1).then(|| snapshot.all_metadata[indices[0]].clone())
        }) else {
            continue;
        };
        if seen_usage.insert(metadata.name.clone()) {
            usage_metadata.push(metadata);
        }
    }
    for metadata in &live_matches {
        if seen_usage.insert(metadata.name.clone()) {
            usage_metadata.push(metadata.clone());
        }
    }

    let resolved_top_function = resolution
        .candidates
        .iter()
        .find(|candidate| candidate.accepted)
        .map(|candidate| candidate.function_name.as_str());
    let top_signature = resolved_top_function
        .and_then(|function_name| {
            declaration_matches
                .iter()
                .find(|matched| matched.function_name == function_name)
        })
        .map(|matched| {
            if matched.impure {
                format!("{}  // impure", matched.signature_line)
            } else {
                matched.signature_line.clone()
            }
        })
        .or_else(|| {
            resolved_top_function
                .and_then(|function_name| {
                    live_matches.iter().find(|metadata| {
                        metadata_to_signature(metadata).qualified() == function_name
                    })
                })
                .map(metadata_to_signature)
                .map(|signature| metadata_signature_line(&signature))
        });
    let priority_metadata_index = resolved_top_function.and_then(|top_function_name| {
        usage_metadata
            .iter()
            .position(|metadata| metadata_to_signature(metadata).qualified() == top_function_name)
    });
    let priority_metadata = priority_metadata_index.and_then(|index| usage_metadata.get(index));
    let priority_block = top_signature
        .as_deref()
        .map(|signature| {
            render_declaration_priority_block(
                signature,
                priority_metadata,
                &snapshot.available_functions,
                snapshot.imap_chain_compatible,
            )
        })
        .unwrap_or_default();
    let usage_notes = render_catalog_usage_notes_to(
        &usage_metadata,
        &snapshot.available_functions,
        MAX_DECLARATION_USAGE_NOTE_BYTES,
        snapshot.imap_chain_compatible
            && !priority_metadata.is_some_and(|metadata| is_imap_chain_node(&metadata.name)),
    );

    if !priority_block.is_empty() {
        out.insert_str(0, &priority_block);
    }
    if !usage_notes.is_empty() {
        out.push('\n');
        out.push_str(&usage_notes);
    }
    format!("{resolution_header}{out}")
}

/// A bare abstention is a discovery dead end in authoring runs (catalog browse tools are hidden
/// there): the model either rephrases the query indefinitely or guesses function names into
/// unknown-declaration diagnostics. Surface the nearest rejected candidates as bare symbols with
/// their rejection reason — never as authorized signatures — so the model can verify one with an
/// exact-symbol follow-up lookup, which always resolves. check_flowscript stays the hard gate
/// against a wrong pick.
fn append_low_confidence_candidate_note(
    out: &mut String,
    assessments: &[AssessedDeclarationCandidate],
) {
    let mut rejected: Vec<&AssessedDeclarationCandidate> = assessments
        .iter()
        .filter(|assessment| !assessment.accepted && assessment.score > 0)
        .collect();
    rejected.sort_by(|left, right| {
        right
            .score
            .cmp(&left.score)
            .then_with(|| left.function_name.cmp(&right.function_name))
    });
    rejected.dedup_by(|left, right| left.function_name == right.function_name);
    if rejected.is_empty() {
        return;
    }
    out.push_str(
        "// Nearest UNVERIFIED candidates the calibrated resolver rejected for this query. They are\n\
         // NOT resolved signatures: to use one, first confirm it with a follow-up get_declarations\n\
         // call on the exact symbol below, and let check_flowscript diagnostics arbitrate.\n",
    );
    for assessment in rejected.into_iter().take(3) {
        let reason = assessment
            .reason_codes
            .iter()
            .find(|code| code.starts_with("missing_strong_anchor:"))
            .or_else(|| assessment.reason_codes.first())
            .map(String::as_str)
            .unwrap_or("below_calibrated_threshold");
        out.push_str(&format!(
            "//   ? {} (score {}; not accepted: {reason})\n",
            assessment.function_name, assessment.score
        ));
    }
}

fn append_unavailable_declaration_note(out: &mut String, function_names: &[String]) {
    if function_names.is_empty() {
        return;
    }
    out.push_str(&format!(
        "\n// Embedded declaration candidates unavailable in the live catalog were omitted: {}. Install or load the providing catalog package before using these calls.\n",
        function_names.join(", ")
    ));
}

fn append_ambiguous_declaration_note(out: &mut String, function_names: &[String]) {
    if function_names.is_empty() {
        return;
    }
    out.push_str(&format!(
        "\n// Ambiguous live catalog declarations omitted (multiple nodes map to the same FlowScript function): {}. Resolve the catalog collision before using these calls.\n",
        function_names.join(", ")
    ));
}

fn render_declaration_priority_block(
    exact_signature: &str,
    metadata: Option<&NodeMetadata>,
    available_functions: &BTreeMap<String, String>,
    include_imap_chain: bool,
) -> String {
    let fixed_bytes = DECLARATION_PRIORITY_BEGIN
        .len()
        .saturating_add(exact_signature.len())
        .saturating_add(1)
        .saturating_add(DECLARATION_PRIORITY_END.len());
    let mut remaining_usage_bytes =
        MAX_DECLARATION_PRIORITY_BLOCK_BYTES.saturating_sub(fixed_bytes);

    let mut block = String::from(DECLARATION_PRIORITY_BEGIN);
    block.push_str(exact_signature);
    block.push('\n');
    if let Some(metadata) = metadata {
        for line in catalog_usage_note_lines(
            std::slice::from_ref(metadata),
            available_functions,
            include_imap_chain,
        ) {
            let required_bytes = line.len().saturating_add(1);
            if required_bytes > remaining_usage_bytes {
                continue;
            }
            block.push_str(&line);
            block.push('\n');
            remaining_usage_bytes -= required_bytes;
        }
    }
    block.push_str(DECLARATION_PRIORITY_END);
    block
}

#[cfg(test)]
fn render_catalog_usage_notes(
    metadata: &[NodeMetadata],
    available_functions: &BTreeMap<String, String>,
) -> String {
    render_catalog_usage_notes_to(
        metadata,
        available_functions,
        MAX_DECLARATION_USAGE_NOTE_BYTES,
        true,
    )
}

fn catalog_usage_note_lines(
    metadata: &[NodeMetadata],
    available_functions: &BTreeMap<String, String>,
    include_imap_chain: bool,
) -> Vec<String> {
    let mut lines = Vec::new();
    if include_imap_chain
        && metadata
            .iter()
            .any(|metadata| is_imap_chain_node(&metadata.name))
    {
        lines.push(IMAP_CHAIN_NOTE.to_string());
    }

    for metadata in metadata {
        let function_name = metadata_to_signature(metadata).qualified();
        let required_inputs = required_input_names(metadata);
        if !required_inputs.is_empty() {
            lines.push(format!(
                "// {function_name} required inputs: {}.",
                summarize_repeated_names(&required_inputs)
            ));
        }

        for (name, count) in repeated_input_names(metadata) {
            let arguments = (1..=count)
                .map(|index| format!("{name}: value{index}"))
                .collect::<Vec<_>>()
                .join(", ");
            lines.push(format!(
                "// {function_name} repeats input `{name}` {count} times: {function_name}({{ {arguments} }}). Repeat the exact key in declaration order; do not rename it or add [#N]."
            ));
        }

        let mut schema_pin_count = 0usize;
        for (direction, pin) in metadata
            .inputs
            .iter()
            .map(|pin| ("input", pin))
            .chain(metadata.outputs.iter().map(|pin| ("output", pin)))
        {
            if schema_pin_count >= MAX_DECLARATION_USAGE_SCHEMA_PINS || pin.data_type != "Struct" {
                continue;
            }
            let Some(summary) = compact_schema_summary(pin.schema.as_deref()) else {
                continue;
            };
            lines.push(format!(
                "// {function_name} {direction} `{}` ({}) schema: {summary}.",
                to_camel_case(&pin.name),
                pin_type_label(pin)
            ));
            schema_pin_count += 1;
        }

        let mut companions = metadata
            .companion_nodes
            .iter()
            .filter_map(|node_name| available_functions.get(node_name))
            .cloned()
            .collect::<Vec<_>>();
        companions.sort();
        companions.dedup();
        companions.truncate(MAX_DECLARATION_USAGE_COMPANIONS);
        if !companions.is_empty() {
            lines.push(format!(
                "// {function_name} companion calls: {}.",
                companions.join(", ")
            ));
        }
    }
    lines
}

fn render_catalog_usage_notes_to(
    metadata: &[NodeMetadata],
    available_functions: &BTreeMap<String, String>,
    max_bytes: usize,
    include_imap_chain: bool,
) -> String {
    let lines = catalog_usage_note_lines(metadata, available_functions, include_imap_chain);

    if lines.is_empty() {
        return String::new();
    }

    let header = "// Live catalog usage notes (authoritative for the matched declarations):\n";
    const NOTICE: &str = "// [Additional live catalog usage notes omitted for size.]\n";
    if max_bytes < header.len().saturating_add(NOTICE.len()) {
        return String::new();
    }
    let line_budget = max_bytes.saturating_sub(NOTICE.len());
    let mut output = String::from(header);
    let mut omitted = false;
    for line in lines {
        let required_bytes = line.len().saturating_add(1);
        if output.len().saturating_add(required_bytes) > line_budget {
            omitted = true;
            continue;
        }
        output.push_str(&line);
        output.push('\n');
    }
    if omitted {
        output.push_str(NOTICE);
    }
    output
}

fn is_imap_chain_node(node_name: &str) -> bool {
    matches!(
        node_name,
        "email_imap_connect"
            | "mail_imap_inbox"
            | "mail_imap_list"
            | "email_imap_inbox_fetch_mail"
            | "email_imap_mark_seen"
    )
}

fn imap_chain_is_compatible(metadata: &[NodeMetadata]) -> bool {
    let Some(connect) = unique_catalog_node(metadata, "email_imap_connect") else {
        return false;
    };
    let Some(inbox) = unique_catalog_node(metadata, "mail_imap_inbox") else {
        return false;
    };
    let Some(list) = unique_catalog_node(metadata, "mail_imap_list") else {
        return false;
    };
    let Some(for_each) = unique_catalog_node(metadata, "control_for_each") else {
        return false;
    };
    let Some(fetch) = unique_catalog_node(metadata, "email_imap_inbox_fetch_mail") else {
        return false;
    };
    let Some(content) = unique_catalog_node(metadata, "email_get_content") else {
        return false;
    };
    let Some(headers) = unique_catalog_node(metadata, "email_get_headers") else {
        return false;
    };
    let Some(address_fields) = unique_catalog_node(metadata, "mail_address_fields") else {
        return false;
    };
    let Some(mark_seen) = unique_catalog_node(metadata, "email_imap_mark_seen") else {
        return false;
    };

    has_output_shape(connect, "Struct", "Normal")
        && has_input_shape(inbox, "connection", "Struct", "Normal")
        && has_input_shape(inbox, "inbox", "String", "Normal")
        && has_output_shape(inbox, "Struct", "Normal")
        && has_input_shape(list, "inbox", "Struct", "Normal")
        && has_input_shape(list, "filter", "String", "Normal")
        && has_output_shape(list, "Struct", "Array")
        && has_input_container(for_each, "array", "Array")
        && has_output_container(for_each, "Normal")
        && has_input_shape(fetch, "emailRef", "Struct", "Normal")
        && has_output_shape(fetch, "Struct", "Normal")
        && has_input_shape(content, "email", "Struct", "Normal")
        && has_input_shape(headers, "email", "Struct", "Normal")
        && has_output_shape(headers, "Struct", "Normal")
        && has_input_shape(address_fields, "address", "Struct", "Normal")
        && has_input_shape(mark_seen, "email", "Struct", "Normal")
        && has_input_shape(mark_seen, "markAsSeen", "Boolean", "Normal")
}

fn unique_catalog_node<'a>(metadata: &'a [NodeMetadata], name: &str) -> Option<&'a NodeMetadata> {
    let mut matches = metadata.iter().filter(|metadata| metadata.name == name);
    let matched = matches.next()?;
    matches.next().is_none().then_some(matched)
}

fn has_input_shape(metadata: &NodeMetadata, name: &str, data_type: &str, value_type: &str) -> bool {
    metadata.inputs.iter().any(|pin| {
        pin.data_type != "Execution"
            && to_camel_case(&pin.name) == name
            && pin.data_type == data_type
            && pin.value_type == value_type
    })
}

fn has_input_container(metadata: &NodeMetadata, name: &str, value_type: &str) -> bool {
    metadata.inputs.iter().any(|pin| {
        pin.data_type != "Execution"
            && to_camel_case(&pin.name) == name
            && pin.value_type == value_type
    })
}

fn has_output_shape(metadata: &NodeMetadata, data_type: &str, value_type: &str) -> bool {
    metadata
        .outputs
        .iter()
        .any(|pin| pin.data_type == data_type && pin.value_type == value_type)
}

fn has_output_container(metadata: &NodeMetadata, value_type: &str) -> bool {
    metadata
        .outputs
        .iter()
        .any(|pin| pin.data_type != "Execution" && pin.value_type == value_type)
}

fn required_input_names(metadata: &NodeMetadata) -> Vec<String> {
    if !metadata.required_inputs.is_empty() {
        return metadata
            .required_inputs
            .iter()
            .map(|name| to_camel_case(name))
            .collect();
    }
    metadata
        .inputs
        .iter()
        .filter(|pin| pin.data_type != "Execution" && pin.default_value.is_none())
        .map(|pin| to_camel_case(&pin.name))
        .collect()
}

fn repeated_input_names(metadata: &NodeMetadata) -> Vec<(String, usize)> {
    let mut order = Vec::new();
    let mut counts = HashMap::new();
    for pin in metadata
        .inputs
        .iter()
        .filter(|pin| pin.data_type != "Execution")
    {
        let name = to_camel_case(&pin.name);
        if !counts.contains_key(&name) {
            order.push(name.clone());
        }
        *counts.entry(name).or_insert(0usize) += 1;
    }
    order
        .into_iter()
        .filter_map(|name| {
            let count = counts.get(&name).copied().unwrap_or_default();
            (count > 1).then_some((name, count))
        })
        .collect()
}

fn summarize_repeated_names(names: &[String]) -> String {
    let mut order = Vec::new();
    let mut counts = HashMap::new();
    for name in names {
        if !counts.contains_key(name) {
            order.push(name.clone());
        }
        *counts.entry(name.clone()).or_insert(0usize) += 1;
    }
    order
        .into_iter()
        .map(
            |name| match counts.get(&name).copied().unwrap_or_default() {
                0 | 1 => name,
                count => format!("{name} x{count}"),
            },
        )
        .collect::<Vec<_>>()
        .join(", ")
}

fn pin_type_label(pin: &PinMetadata) -> String {
    let base = base_type(&pin.data_type);
    match container(&pin.value_type) {
        Container::Normal => base.to_string(),
        Container::Array => format!("{base}[]"),
        Container::Map => format!("Map<string, {base}>"),
        Container::Set => format!("Set<{base}>"),
    }
}

fn compact_schema_summary(schema: Option<&str>) -> Option<String> {
    let root = flow_like_types::json::from_str::<flow_like_types::Value>(schema?).ok()?;
    let object = resolve_schema_node(&root, &root);
    let properties = object.get("properties")?.as_object()?;
    if properties.is_empty() {
        return None;
    }
    let required = object
        .get("required")
        .and_then(flow_like_types::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(flow_like_types::Value::as_str)
        .collect::<HashSet<_>>();
    let mut fields = properties
        .iter()
        .map(|(name, field_schema)| {
            let optional = if required.contains(name.as_str()) {
                ""
            } else {
                "?"
            };
            format!(
                "{name}{optional}: {}",
                schema_type_label(&root, field_schema)
            )
        })
        .collect::<Vec<_>>();
    fields.sort();
    let omitted = fields
        .len()
        .saturating_sub(MAX_DECLARATION_USAGE_SCHEMA_FIELDS);
    fields.truncate(MAX_DECLARATION_USAGE_SCHEMA_FIELDS);
    if omitted > 0 {
        fields.push(format!("... +{omitted} fields"));
    }
    let title = object
        .get("title")
        .or_else(|| root.get("title"))
        .and_then(flow_like_types::Value::as_str)
        .map(|title| format!("{title} "))
        .unwrap_or_default();
    Some(format!("{title}{{ {} }}", fields.join(", ")))
}

fn resolve_schema_node<'a>(
    root: &'a flow_like_types::Value,
    schema: &'a flow_like_types::Value,
) -> &'a flow_like_types::Value {
    let mut current = schema;
    for _ in 0..8 {
        if let Some(reference) = current
            .get("$ref")
            .and_then(flow_like_types::Value::as_str)
            .and_then(|reference| reference.strip_prefix('#'))
            && let Some(resolved) = root.pointer(reference)
        {
            current = resolved;
            continue;
        }
        if let Some(variants) = current
            .get("anyOf")
            .or_else(|| current.get("oneOf"))
            .and_then(flow_like_types::Value::as_array)
            && let Some(resolved) = variants.iter().find(|variant| !schema_is_null(variant))
        {
            current = resolved;
            continue;
        }
        break;
    }
    current
}

fn schema_is_null(schema: &flow_like_types::Value) -> bool {
    match schema.get("type") {
        Some(flow_like_types::Value::String(value)) => value == "null",
        Some(flow_like_types::Value::Array(values)) => values
            .iter()
            .all(|value| value.as_str().is_some_and(|value| value == "null")),
        _ => false,
    }
}

fn schema_type_label(root: &flow_like_types::Value, schema: &flow_like_types::Value) -> String {
    if let Some(reference) = schema.get("$ref").and_then(flow_like_types::Value::as_str) {
        let resolved = resolve_schema_node(root, schema);
        if !std::ptr::eq(resolved, schema) && resolved.get("properties").is_none() {
            return schema_type_label(root, resolved);
        }
        return reference
            .rsplit('/')
            .next()
            .filter(|name| !name.is_empty())
            .unwrap_or("Struct")
            .to_string();
    }
    if let Some(variants) = schema
        .get("anyOf")
        .or_else(|| schema.get("oneOf"))
        .and_then(flow_like_types::Value::as_array)
    {
        let mut labels = variants
            .iter()
            .filter(|variant| !schema_is_null(variant))
            .map(|variant| schema_type_label(root, variant))
            .collect::<Vec<_>>();
        labels.sort();
        labels.dedup();
        if !labels.is_empty() {
            return labels.join(" | ");
        }
    }
    if let Some(types) = schema
        .get("type")
        .and_then(flow_like_types::Value::as_array)
    {
        let mut labels = types
            .iter()
            .filter_map(flow_like_types::Value::as_str)
            .filter(|kind| *kind != "null")
            .map(|kind| schema_scalar_type_label(kind, schema))
            .collect::<Vec<_>>();
        labels.sort();
        labels.dedup();
        if !labels.is_empty() {
            return labels.join(" | ");
        }
    }
    let resolved = resolve_schema_node(root, schema);
    match resolved
        .get("type")
        .and_then(flow_like_types::Value::as_str)
    {
        Some("array") => {
            let item = resolved
                .get("items")
                .map(|items| schema_type_label(root, items))
                .unwrap_or_else(|| "any".to_string());
            format!("{item}[]")
        }
        Some(kind) => schema_scalar_type_label(kind, resolved),
        None if resolved.get("properties").is_some() => "Struct".to_string(),
        None => "any".to_string(),
    }
}

fn schema_scalar_type_label(kind: &str, schema: &flow_like_types::Value) -> String {
    match kind {
        "integer" => "int",
        "number" => "float",
        "boolean" => "bool",
        "object" => "Struct",
        "string"
            if matches!(
                schema
                    .get("format")
                    .and_then(flow_like_types::Value::as_str),
                Some("date") | Some("date-time")
            ) =>
        {
            "Date"
        }
        "string" => "string",
        "null" => "null",
        other => other,
    }
    .to_string()
}

fn compact_doc_line(doc: &str) -> String {
    const MAX_SUMMARY_CHARS: usize = 120;
    let doc = doc.replace('\n', " ");
    let mut out = String::with_capacity(doc.len().min(MAX_SUMMARY_CHARS));
    for (idx, ch) in doc.trim().chars().enumerate() {
        if idx >= MAX_SUMMARY_CHARS {
            out.push_str("...");
            return out;
        }
        out.push(ch);
    }
    out
}

fn metadata_signature_line(signature: &crate::flow::ast::Signature) -> String {
    let line = signature.signature_line();
    if signature.impure {
        format!("{line}  // impure")
    } else {
        line
    }
}

/// FlowScript base type for a metadata pin's `data_type` string (the `Debug` spelling of the
/// core `VariableType`). Unknown / generic types collapse to `any`.
fn base_type(data_type: &str) -> &'static str {
    match data_type {
        "String" => "string",
        "Integer" => "int",
        "Float" => "float",
        "Boolean" => "bool",
        "Date" => "Date",
        "PathBuf" => "Path",
        "Struct" => "Struct",
        "Byte" => "bytes",
        "Execution" => "exec",
        _ => "any",
    }
}

/// FlowScript container shape for a metadata pin's `value_type` string.
fn container(value_type: &str) -> Container {
    match value_type {
        "Array" => Container::Array,
        "HashMap" => Container::Map,
        "HashSet" => Container::Set,
        _ => Container::Normal,
    }
}

fn pin_to_sig_param(pin: &PinMetadata) -> SigParam {
    let doc = {
        let d = pin.description.trim();
        (!d.is_empty()).then(|| d.to_string())
    };
    SigParam {
        name: pin.name.clone(),
        ty: TypeRef::new(base_type(&pin.data_type), container(&pin.value_type)),
        optional: pin
            .default_value
            .as_ref()
            .is_some_and(|v| !v.trim().is_empty()),
        doc,
        schema: pin.schema.clone(),
    }
}

/// Convert a board/catalog pin into the metadata shape FlowPilot and FlowScript use.
pub fn pin_to_metadata(pin: &Pin) -> PinMetadata {
    let is_generic = pin.data_type == VariableType::Generic;
    let enforce_schema = pin
        .options
        .as_ref()
        .and_then(|options| options.enforce_schema)
        .unwrap_or(false);
    let valid_values = pin
        .options
        .as_ref()
        .and_then(|options| options.valid_values.clone());

    PinMetadata {
        name: pin.name.clone(),
        friendly_name: pin.friendly_name.clone(),
        description: pin.description.clone(),
        data_type: format!("{:?}", pin.data_type),
        value_type: format!("{:?}", pin.value_type),
        default_value: pin
            .default_value
            .as_ref()
            .map(|value| String::from_utf8_lossy(value).to_string())
            .filter(|value| !value.is_empty() && value != "null"),
        schema: pin.schema.clone(),
        is_generic,
        valid_values,
        enforce_schema,
    }
}

/// Convert a board/catalog node into the metadata shape FlowPilot and FlowScript use.
pub fn node_to_metadata(node: &Node) -> NodeMetadata {
    let derived_category = node
        .name
        .to_lowercase()
        .split("::")
        .nth(1)
        .unwrap_or("")
        .to_string();
    let category = if derived_category.is_empty() {
        node.category.clone()
    } else {
        derived_category
    };

    let mut inputs: Vec<&Pin> = node
        .pins
        .values()
        .filter(|pin| pin.pin_type == PinType::Input)
        .collect();
    inputs.sort_by_key(|pin| (pin.index, pin.name.clone()));

    let mut outputs: Vec<&Pin> = node
        .pins
        .values()
        .filter(|pin| pin.pin_type == PinType::Output)
        .collect();
    outputs.sort_by_key(|pin| (pin.index, pin.name.clone()));

    super::search::enrich_node_metadata(NodeMetadata {
        name: node.name.clone(),
        friendly_name: node.friendly_name.clone(),
        description: node.description.clone(),
        inputs: inputs.into_iter().map(pin_to_metadata).collect(),
        outputs: outputs.into_iter().map(pin_to_metadata).collect(),
        category: Some(category),
        required_inputs: Vec::new(),
        companion_nodes: Vec::new(),
        capability_tags: Vec::new(),
        namespace: Some(node.flowscript_namespace()),
        alias: Some(node.flowscript_alias()),
        receiver: node.receiver.clone().or_else(|| node.flowscript_receiver()),
    })
}

/// Build a FlowScript [`Signature`] from catalog [`NodeMetadata`].
///
/// Mirrors `flow::ast::node_to_signature` but works off the already-flattened metadata the
/// providers expose, so the copilot can render declarations without re-reading the catalog.
/// Execution pins carry control flow (not data) so they are excluded from params and instead set
/// the `impure` flag.
pub fn metadata_to_signature(meta: &NodeMetadata) -> Signature {
    let impure = meta
        .inputs
        .iter()
        .chain(meta.outputs.iter())
        .any(|p| p.data_type == "Execution");

    let inputs = meta
        .inputs
        .iter()
        .filter(|p| p.data_type != "Execution")
        .map(pin_to_sig_param)
        .collect();
    let outputs = meta
        .outputs
        .iter()
        .filter(|p| p.data_type != "Execution")
        .map(pin_to_sig_param)
        .collect();

    let friendly = {
        let f = meta.friendly_name.trim();
        (!f.is_empty()).then(|| f.to_string())
    };
    let category = meta
        .category
        .as_ref()
        .map(|c| c.trim())
        .filter(|c| !c.is_empty())
        .map(|c| c.to_string());
    let doc = {
        let d = meta.description.trim();
        (!d.is_empty()).then(|| d.to_string())
    };

    let names = catalog_names(meta);
    Signature {
        node_type: meta.name.clone(),
        display: to_camel_case(&meta.name),
        friendly,
        category,
        package: None,
        inputs,
        outputs,
        impure,
        doc,
        namespace: Some(names.namespace).filter(|namespace| !namespace.is_empty()),
        alias: Some(names.alias),
        receiver: names.receiver,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use flow_like_types::tokio;
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    fn pin(
        name: &str,
        data_type: &str,
        value_type: &str,
        default_value: Option<&str>,
        schema: Option<&str>,
    ) -> PinMetadata {
        PinMetadata {
            name: name.to_string(),
            friendly_name: name.to_string(),
            description: String::new(),
            data_type: data_type.to_string(),
            value_type: value_type.to_string(),
            default_value: default_value.map(str::to_string),
            schema: schema.map(str::to_string),
            is_generic: data_type == "Generic",
            valid_values: None,
            enforce_schema: schema.is_some(),
        }
    }

    /// Explicit names for the catalog nodes these tests imitate, mirroring the bake-in:
    /// production metadata always carries effective names, so the synthetic metadata must too.
    fn baked_names(name: &str) -> (Option<&str>, Option<&str>, Option<&str>) {
        match name {
            "email_imap_connect" => (Some("imap"), Some("connect"), None),
            "mail_imap_inbox" => (Some("imap"), Some("inbox"), Some("connection")),
            "mail_imap_list" => (Some("imap"), Some("listMails"), Some("inbox")),
            "email_imap_inbox_fetch_mail" => (Some("imap"), Some("fetchMail"), Some("email_ref")),
            "email_imap_mark_seen" => (Some("imap"), Some("markSeen"), Some("email")),
            "email_get_content" => (Some("email"), Some("getContent"), Some("email")),
            "email_get_headers" => (Some("email"), Some("getHeaders"), Some("email")),
            "mail_address_fields" => (Some("email"), Some("addressToFields"), None),
            "hybrid_search_local_db" => (None, Some("hybridSearch"), None),
            "open_local_db" => (None, Some("open"), None),
            "upsert_local_db" => (None, Some("upsert"), None),
            _ => (None, None, None),
        }
    }

    fn metadata(name: &str, inputs: Vec<PinMetadata>, outputs: Vec<PinMetadata>) -> NodeMetadata {
        let (namespace, alias, receiver) = baked_names(name);
        NodeMetadata {
            name: name.to_string(),
            friendly_name: name.to_string(),
            description: format!("Catalog metadata for {name}"),
            inputs,
            outputs,
            category: Some("test".to_string()),
            required_inputs: Vec::new(),
            companion_nodes: Vec::new(),
            capability_tags: Vec::new(),
            namespace: namespace.map(str::to_string),
            alias: alias.map(str::to_string),
            receiver: receiver.map(str::to_string),
        }
    }

    #[test]
    fn compact_schema_summary_preserves_temporal_field_types() {
        let schema = r##"{
            "title":"AuditRow",
            "type":"object",
            "properties":{
                "created_at":{"type":"string","format":"date-time"},
                "event_dates":{"type":"array","items":{"type":"string","format":"date"}},
                "label":{"type":"string"},
                "observed_at":{"anyOf":[{"type":"null"},{"$ref":"#/$defs/UtcInstant"}]},
                "updated_at":{"type":["string","null"],"format":"date-time"}
            },
            "required":["created_at"],
            "$defs":{"UtcInstant":{"type":"string","format":"date-time"}}
        }"##;

        assert_eq!(
            compact_schema_summary(Some(schema)).as_deref(),
            Some(
                "AuditRow { created_at: Date, event_dates?: Date[], label?: string, observed_at?: Date, updated_at?: Date }"
            )
        );
    }

    struct LiveOnlyProvider {
        nodes: Vec<NodeMetadata>,
    }

    struct CountingLiveProvider {
        nodes: Vec<NodeMetadata>,
        metadata_reads: Arc<AtomicUsize>,
    }

    #[async_trait]
    impl CatalogProvider for LiveOnlyProvider {
        async fn search(&self, _query: &str) -> Vec<NodeMetadata> {
            self.nodes.clone()
        }

        async fn search_by_pin_type(&self, _pin_type: &str, _is_input: bool) -> Vec<NodeMetadata> {
            Vec::new()
        }

        async fn filter_by_category(&self, _category_prefix: &str) -> Vec<NodeMetadata> {
            Vec::new()
        }

        async fn get_node_metadata(&self, node_type: &str) -> Option<NodeMetadata> {
            self.nodes
                .iter()
                .find(|node| node.name == node_type)
                .cloned()
        }

        async fn get_all_nodes(&self) -> Vec<String> {
            self.nodes.iter().map(|node| node.name.clone()).collect()
        }

        async fn get_all_metadata(&self) -> Vec<NodeMetadata> {
            self.nodes.clone()
        }
    }

    #[async_trait]
    impl CatalogProvider for CountingLiveProvider {
        async fn search(&self, _query: &str) -> Vec<NodeMetadata> {
            self.nodes.clone()
        }

        async fn search_by_pin_type(&self, _pin_type: &str, _is_input: bool) -> Vec<NodeMetadata> {
            Vec::new()
        }

        async fn filter_by_category(&self, _category_prefix: &str) -> Vec<NodeMetadata> {
            Vec::new()
        }

        async fn get_node_metadata(&self, node_type: &str) -> Option<NodeMetadata> {
            self.nodes
                .iter()
                .find(|node| node.name == node_type)
                .cloned()
        }

        async fn get_all_nodes(&self) -> Vec<String> {
            self.nodes.iter().map(|node| node.name.clone()).collect()
        }

        async fn get_all_metadata(&self) -> Vec<NodeMetadata> {
            self.metadata_reads.fetch_add(1, Ordering::SeqCst);
            self.nodes.clone()
        }
    }

    #[tokio::test]
    async fn declaration_batch_reuses_one_live_catalog_snapshot_and_index() {
        let metadata_reads = Arc::new(AtomicUsize::new(0));
        let provider = CountingLiveProvider {
            nodes: vec![
                metadata(
                    "bool_or",
                    vec![
                        pin("left", "Boolean", "Normal", None, None),
                        pin("right", "Boolean", "Normal", None, None),
                    ],
                    vec![pin("result", "Boolean", "Normal", None, None)],
                ),
                metadata(
                    "custom_package_database_export",
                    Vec::new(),
                    vec![pin("rows", "Struct", "Array", None, None)],
                ),
                metadata(
                    "string_replace",
                    vec![pin("left", "String", "Normal", None, None)],
                    Vec::new(),
                ),
                metadata(
                    "string_replace",
                    vec![pin("right", "Integer", "Normal", None, None)],
                    Vec::new(),
                ),
            ],
            metadata_reads: metadata_reads.clone(),
        };
        let queries = vec![
            "boolean or".to_string(),
            "database export".to_string(),
            "string replace".to_string(),
        ];

        let declarations = provider.get_declarations_batch(&queries).await;

        assert_eq!(metadata_reads.load(Ordering::SeqCst), 1);
        assert_eq!(declarations.len(), queries.len());
        assert!(
            declarations[0].contains("function test::boolOr({ left: bool, right: bool }): bool;")
        );
        assert!(declarations[0].contains("live pin contract is authoritative"));
        assert!(declarations[1].contains("function test::customPackageDatabaseExport("));
        assert!(!declarations[2].contains("function test::stringReplace("));
        assert!(declarations[2].contains("Ambiguous live catalog declarations omitted"));
    }

    #[tokio::test]
    async fn calibrated_resolution_abstains_on_incident_false_mappings() {
        let incidents = [
            (
                "datafusion create session",
                "df_sql_query",
                "Runs a SQL query inside an existing DataFusion session.",
                "dfSqlQuery",
                "create",
            ),
            (
                "register Lance",
                "open_local_db",
                "Opens a Lance database that can later be registered in another engine.",
                "open",
                "register",
            ),
            (
                "hybrid search",
                "open_local_db",
                "Opens a Lance database that can later participate in hybrid search.",
                "open",
                "hybrid",
            ),
            (
                "upsert",
                "set_insert_ref",
                "Upsert-style insertion into a mutable set reference.",
                "setInsertRef",
                "upsert",
            ),
            (
                "chunk",
                "embed_document",
                "Chunks a document internally and returns its embedding.",
                "embedDocument",
                "chunk",
            ),
            (
                "integer compare",
                "faker_integer",
                "Generates integer values for comparison tests.",
                "fakerInteger",
                "compare",
            ),
            (
                "markdown",
                "ai_audio_text_to_speech",
                "Reads Markdown-flavored text aloud with speech synthesis.",
                "aiAudioTextToSpeech",
                "markdown",
            ),
            (
                "notifications",
                "browser_clear_console_logs",
                "Clears browser console logs used while debugging notifications.",
                "browserClearConsoleLogs",
                "notification",
            ),
        ];

        for (query, node_type, description, wrong_function, missing_anchor) in incidents {
            let mut decoy = metadata(node_type, Vec::new(), Vec::new());
            decoy.description = description.to_string();
            let provider = LiveOnlyProvider { nodes: vec![decoy] };

            let declarations = provider.get_declarations(query).await;
            let resolution = parse_declaration_resolution_metadata(&declarations)
                .expect("every non-empty live lookup returns machine-readable resolution data");

            assert_eq!(
                resolution.status,
                DeclarationResolutionStatus::Unresolved,
                "{query:?} must abstain: {declarations}"
            );
            assert!(resolution.top_score.is_some(), "{query:?}: {declarations}");
            assert!(resolution.margin.is_some(), "{query:?}: {declarations}");
            assert!(
                resolution
                    .reason_codes
                    .iter()
                    .any(|reason| reason == &format!("missing_strong_anchor:{missing_anchor}")),
                "{query:?} did not explain its abstention: {resolution:?}"
            );
            assert!(
                !declarations.contains(&format!("function test::{wrong_function}(")),
                "{query:?} leaked the incident decoy: {declarations}"
            );
        }
    }

    #[tokio::test]
    async fn calibrated_resolution_prefers_the_semantic_operation_over_incident_decoys() {
        let cases = [
            (
                "datafusion create session",
                "df_create_session",
                "Creates a new DataFusion session.",
                "df_sql_query",
                "Runs SQL against an existing DataFusion session.",
                "dfCreateSession",
                "dfSqlQuery",
            ),
            (
                "register Lance",
                "df_register_lance",
                "Registers a Lance table in DataFusion.",
                "open_local_db",
                "Opens a Lance database that may later be registered.",
                "dfRegisterLance",
                "open",
            ),
            (
                "hybrid search",
                "hybrid_search_local_db",
                "Runs hybrid search against a local database.",
                "open_local_db",
                "Opens a database used by later hybrid search calls.",
                "hybridSearch",
                "open",
            ),
            (
                "upsert",
                "upsert_local_db",
                "Upserts one row in a local database.",
                "set_insert_ref",
                "Upsert-style insertion into a mutable set.",
                "upsert",
                "setInsertRef",
            ),
            (
                "chunk",
                "chunk_text",
                "Chunks input text into bounded pieces.",
                "embed_document",
                "Chunks a document internally before embedding.",
                "chunkText",
                "embedDocument",
            ),
            (
                "integer compare",
                "int_equal",
                "Compares two integers for equality.",
                "faker_integer",
                "Generates integer values for comparison tests.",
                "intEqual",
                "fakerInteger",
            ),
            (
                "markdown",
                "ai_processing_pages_to_markdown",
                "Converts document pages to Markdown.",
                "ai_audio_text_to_speech",
                "Reads Markdown-flavored text aloud.",
                "aiProcessingPagesToMarkdown",
                "aiAudioTextToSpeech",
            ),
            (
                "notifications",
                "data_microsoft_copilot_subscribe_notifications",
                "Subscribes to Microsoft Copilot notifications.",
                "browser_clear_console_logs",
                "Clears browser logs used to debug notifications.",
                "dataMicrosoftCopilotSubscribeNotifications",
                "browserClearConsoleLogs",
            ),
        ];

        for (
            query,
            correct_type,
            correct_description,
            decoy_type,
            decoy_description,
            correct_function,
            decoy_function,
        ) in cases
        {
            let mut correct = metadata(correct_type, Vec::new(), Vec::new());
            correct.description = correct_description.to_string();
            let mut decoy = metadata(decoy_type, Vec::new(), Vec::new());
            decoy.description = decoy_description.to_string();
            let provider = LiveOnlyProvider {
                nodes: vec![correct, decoy],
            };

            let declarations = provider.get_declarations(query).await;
            let resolution = parse_declaration_resolution_metadata(&declarations).unwrap();

            assert_eq!(
                resolution.status,
                DeclarationResolutionStatus::Resolved,
                "{query:?}: {declarations}"
            );
            assert_eq!(
                resolution
                    .candidates
                    .iter()
                    .find(|candidate| candidate.accepted)
                    .map(|candidate| candidate.function_name.as_str()),
                Some(format!("test::{correct_function}").as_str()),
                "{query:?}: {resolution:?}"
            );
            assert!(
                declarations.contains(&format!("function test::{correct_function}(")),
                "{query:?}: {declarations}"
            );
            assert!(
                !declarations.contains(&format!("function test::{decoy_function}(")),
                "{query:?}: {declarations}"
            );
        }
    }

    #[tokio::test]
    async fn exact_unique_live_symbol_is_machine_classified_as_exact() {
        let provider = LiveOnlyProvider {
            nodes: vec![metadata(
                "df_sql_query",
                vec![pin("session", "Struct", "Normal", None, None)],
                vec![pin("rows", "Struct", "Array", None, None)],
            )],
        };

        let declarations = provider
            .get_declarations("dfSqlQuery exact live signature")
            .await;
        let resolution = parse_declaration_resolution_metadata(&declarations).unwrap();

        assert_eq!(resolution.status, DeclarationResolutionStatus::Exact);
        assert_eq!(resolution.top_score, Some(100_000));
        assert!(
            resolution
                .reason_codes
                .contains(&"unique_live_exact_symbol".to_string())
        );
        assert!(declarations.contains("function test::dfSqlQuery("));
    }

    #[tokio::test]
    async fn declarations_filter_stale_embedded_availability_while_keeping_live_matches() {
        let stale_embedded_function = search_declarations("database")
            .into_iter()
            .next()
            .expect("database query should have an embedded declaration")
            .function_name;
        let provider = LiveOnlyProvider {
            nodes: vec![NodeMetadata {
                name: "custom_package_database_export".to_string(),
                friendly_name: "Package Database Export".to_string(),
                description: "Exports database rows through an installed package node.".to_string(),
                inputs: Vec::new(),
                outputs: Vec::new(),
                category: Some("packages/database".to_string()),
                required_inputs: Vec::new(),
                companion_nodes: Vec::new(),
                capability_tags: Vec::new(),
                namespace: None,
                alias: None,
                receiver: None,
            }],
        };

        let declarations = provider.get_declarations("database").await;

        assert!(declarations.contains("embedded .flow.d index"));
        assert!(declarations.contains("customPackageDatabaseExport"));
        assert!(declarations.contains("unavailable in the live catalog were omitted"));
        assert!(declarations.contains(&stale_embedded_function));
        assert!(!declarations.contains(&format!("function {stale_embedded_function}(")));
        assert!(declarations.contains("function packages::database::customPackageDatabaseExport("));
    }

    #[tokio::test]
    async fn declarations_explain_repeated_same_name_arguments_in_order() {
        let mut boolean_or = metadata(
            "bool_or",
            vec![
                pin("boolean", "Boolean", "Normal", Some("false"), None),
                pin("boolean", "Boolean", "Normal", Some("false"), None),
            ],
            vec![pin("result", "Boolean", "Normal", None, None)],
        );
        boolean_or.description = "Boolean OR operation".to_string();
        let provider = LiveOnlyProvider {
            nodes: vec![boolean_or],
        };

        let declarations = provider.get_declarations("boolean or").await;

        assert!(declarations.contains("function test::boolOr("));
        assert!(declarations.contains("boolOr({ boolean: value1, boolean: value2 })"));
        assert!(declarations.contains("Repeat the exact key in declaration order"));
        assert!(declarations.contains("do not rename it or add [#N]"));
    }

    #[tokio::test]
    async fn unique_live_metadata_overrides_a_same_name_embedded_signature() {
        let provider = LiveOnlyProvider {
            nodes: vec![metadata(
                "bool_or",
                vec![
                    pin("left", "Boolean", "Normal", None, None),
                    pin("right", "Boolean", "Normal", None, None),
                ],
                vec![pin("result", "Boolean", "Normal", None, None)],
            )],
        };

        let declarations = provider.get_declarations("boolean or").await;

        assert!(
            declarations.contains("function test::boolOr({ left: bool, right: bool }): bool;"),
            "{declarations}"
        );
        assert!(!declarations.contains("boolOr({ boolean?: bool, boolean?: bool })"));
        assert!(declarations.contains("live pin contract is authoritative"));
        let priority = declarations
            .split_once(DECLARATION_PRIORITY_BEGIN)
            .and_then(|(_, rest)| rest.split_once(DECLARATION_PRIORITY_END))
            .map(|(priority, _)| priority)
            .unwrap();
        assert!(priority.contains("left: bool, right: bool"));
    }

    #[tokio::test]
    async fn ambiguous_same_name_live_metadata_is_not_paired_or_declared() {
        let provider = LiveOnlyProvider {
            nodes: vec![
                metadata(
                    "bool_or",
                    vec![pin("left", "Boolean", "Normal", None, None)],
                    vec![pin("result", "Boolean", "Normal", None, None)],
                ),
                metadata(
                    "bool_or",
                    vec![pin("operand", "String", "Normal", None, None)],
                    vec![pin("result", "String", "Normal", None, None)],
                ),
            ],
        };

        let declarations = provider.get_declarations("boolean or").await;

        assert!(!declarations.contains("function test::boolOr("));
        assert!(declarations.contains("Ambiguous live catalog declarations omitted"));
        assert!(declarations.contains("bool::or"));
        assert!(!declarations.contains("boolOr required inputs"));
        assert!(!declarations.contains("bool::or required inputs"));
    }

    #[tokio::test]
    async fn ambiguous_live_function_is_not_recommended_as_a_companion() {
        let mut gate = metadata(
            "logic_gate",
            vec![pin("value", "Boolean", "Normal", None, None)],
            vec![pin("result", "Boolean", "Normal", None, None)],
        );
        gate.companion_nodes = vec!["bool_or".to_string()];
        let provider = LiveOnlyProvider {
            nodes: vec![
                gate,
                metadata(
                    "bool_or",
                    vec![pin("left", "Boolean", "Normal", None, None)],
                    vec![pin("result", "Boolean", "Normal", None, None)],
                ),
                metadata(
                    "bool_or",
                    vec![pin("operand", "String", "Normal", None, None)],
                    vec![pin("result", "String", "Normal", None, None)],
                ),
            ],
        };

        let declarations = provider.get_declarations("logic gate").await;

        assert!(declarations.contains("function test::logicGate("));
        assert!(!declarations.contains("logicGate companion calls: boolOr"));
    }

    #[tokio::test]
    async fn declarations_include_required_schema_companions_and_one_imap_chain() {
        let email_schema = r##"{
            "title":"Email",
            "type":"object",
            "properties":{
                "from":{"$ref":"#/$defs/MailAddress"},
                "plain":{"type":["string","null"]},
                "subject":{"type":"string"},
                "uid":{"type":"integer"}
            },
            "required":["subject","uid"],
            "$defs":{"MailAddress":{"type":"object","properties":{"email":{"type":"string"}},"required":["email"]}}
        }"##;
        let mut fetch = metadata(
            "email_imap_inbox_fetch_mail",
            vec![
                pin("exec_in", "Execution", "Normal", None, None),
                pin("email_ref", "Struct", "Normal", None, None),
            ],
            vec![
                pin("exec_out", "Execution", "Normal", None, None),
                pin("email", "Struct", "Normal", None, Some(email_schema)),
            ],
        );
        fetch.required_inputs = vec!["email_ref".to_string()];
        fetch.companion_nodes = vec![
            "email_imap_connect".to_string(),
            "mail_imap_inbox".to_string(),
            "mail_imap_list".to_string(),
            "email_imap_mark_seen".to_string(),
        ];
        let provider = LiveOnlyProvider {
            nodes: vec![
                fetch,
                metadata(
                    "email_imap_connect",
                    Vec::new(),
                    vec![pin("connection", "Struct", "Normal", None, None)],
                ),
                metadata(
                    "mail_imap_inbox",
                    vec![
                        pin("connection", "Struct", "Normal", None, None),
                        pin("inbox", "String", "Normal", Some("INBOX"), None),
                    ],
                    vec![pin("inbox_struct", "Struct", "Normal", None, None)],
                ),
                metadata(
                    "mail_imap_list",
                    vec![
                        pin("inbox", "Struct", "Normal", None, None),
                        pin("filter", "String", "Normal", Some("ALL"), None),
                    ],
                    vec![pin("emails", "Struct", "Array", None, None)],
                ),
                metadata(
                    "control_for_each",
                    vec![pin("array", "Generic", "Array", None, None)],
                    vec![pin("value", "Generic", "Normal", None, None)],
                ),
                metadata(
                    "email_get_content",
                    vec![pin("email", "Struct", "Normal", None, None)],
                    vec![pin("plain", "String", "Normal", None, None)],
                ),
                metadata(
                    "email_get_headers",
                    vec![pin("email", "Struct", "Normal", None, None)],
                    vec![pin("from", "Struct", "Normal", None, None)],
                ),
                metadata(
                    "mail_address_fields",
                    vec![pin("address", "Struct", "Normal", None, None)],
                    vec![pin("email", "String", "Normal", None, None)],
                ),
                metadata(
                    "email_imap_mark_seen",
                    vec![
                        pin("email", "Struct", "Normal", None, None),
                        pin("mark_as_seen", "Boolean", "Normal", Some("true"), None),
                    ],
                    vec![pin("email_ref", "Struct", "Normal", None, None)],
                ),
            ],
        };

        let declarations = provider.get_declarations("imap fetch mail").await;

        assert!(declarations.contains("fetchMail required inputs: emailRef"));
        assert!(declarations.contains("schema: Email {"));
        assert!(declarations.contains("from?: MailAddress"));
        assert!(declarations.contains("plain?: string"));
        assert!(declarations.contains("subject: string"));
        assert!(declarations.contains("uid: int"));
        assert!(declarations.contains("imap::inbox({ connection: connection, inbox: \"INBOX\" })"));
        assert!(declarations.contains("imap::listMails({ inbox: inbox, filter: \"UNSEEN\" })"));
        assert!(declarations.contains("filter: \"UNSEEN\""));
        assert!(declarations.contains("for (const ref of refs)"));
        assert!(declarations.contains("imap::fetchMail({ emailRef: ref })"));
        assert!(declarations.contains("email::getContent"));
        assert!(declarations.contains("email::getHeaders"));
        assert!(declarations.contains("email::addressToFields"));
        assert!(declarations.contains("imap::markSeen({ email: ref, markAsSeen: true })"));
        assert!(declarations.contains(DECLARATION_PRIORITY_BEGIN));
        assert!(declarations.contains(DECLARATION_PRIORITY_END));
        let priority_start = declarations.find(DECLARATION_PRIORITY_BEGIN).unwrap()
            + DECLARATION_PRIORITY_BEGIN.len();
        assert!(
            declarations[priority_start..].starts_with("function imap::fetchMail("),
            "{declarations}"
        );
        for companion in [
            "imap::connect",
            "imap::inbox",
            "listMails",
            "imap::markSeen",
        ] {
            assert!(
                declarations.contains(companion),
                "missing {companion}: {declarations}"
            );
        }
        assert_eq!(declarations.matches("// IMAP:").count(), 1);
    }

    #[tokio::test]
    async fn declarations_suppress_imap_recipe_when_live_pin_contract_is_incompatible() {
        let provider = LiveOnlyProvider {
            nodes: vec![
                metadata(
                    "email_imap_connect",
                    Vec::new(),
                    vec![pin("connection", "Struct", "Normal", None, None)],
                ),
                metadata(
                    "mail_imap_inbox",
                    vec![
                        pin("connection", "Struct", "Normal", None, None),
                        pin("inbox", "String", "Normal", Some("INBOX"), None),
                    ],
                    vec![pin("inbox_struct", "Struct", "Normal", None, None)],
                ),
                metadata(
                    "mail_imap_list",
                    vec![
                        pin("inbox", "Struct", "Normal", None, None),
                        pin("query", "String", "Normal", Some("ALL"), None),
                    ],
                    vec![pin("emails", "Struct", "Array", None, None)],
                ),
                metadata(
                    "control_for_each",
                    vec![pin("array", "Generic", "Array", None, None)],
                    vec![pin("value", "Generic", "Normal", None, None)],
                ),
                metadata(
                    "email_imap_inbox_fetch_mail",
                    vec![pin("email_ref", "Struct", "Normal", None, None)],
                    vec![pin("email", "Struct", "Normal", None, None)],
                ),
                metadata(
                    "email_get_content",
                    vec![pin("email", "Struct", "Normal", None, None)],
                    Vec::new(),
                ),
                metadata(
                    "email_get_headers",
                    vec![pin("email", "Struct", "Normal", None, None)],
                    vec![pin("from", "Struct", "Normal", None, None)],
                ),
                metadata(
                    "mail_address_fields",
                    vec![pin("address", "Struct", "Normal", None, None)],
                    Vec::new(),
                ),
                metadata(
                    "email_imap_mark_seen",
                    vec![
                        pin("email", "Struct", "Normal", None, None),
                        pin("mark_as_seen", "Boolean", "Normal", Some("true"), None),
                    ],
                    Vec::new(),
                ),
            ],
        };

        let declarations = provider.get_declarations("imap fetch mail").await;

        assert!(declarations.contains("function imap::fetchMail"));
        assert!(!declarations.contains("// IMAP:"));
        assert!(!declarations.contains("filter: \"UNSEEN\""));
    }

    #[test]
    fn catalog_usage_notes_are_bounded_and_deterministic() {
        let schema = format!(
            r#"{{"title":"Large","type":"object","properties":{{{}}}}}"#,
            (0..40)
                .map(|index| format!(r#""field_{index}":{{"type":"string"}}"#))
                .collect::<Vec<_>>()
                .join(",")
        );
        let metadata = (0..40)
            .map(|index| {
                metadata(
                    &format!("custom_schema_node_{index:02}"),
                    vec![pin("payload", "Struct", "Normal", None, Some(&schema))],
                    Vec::new(),
                )
            })
            .collect::<Vec<_>>();
        let available = metadata
            .iter()
            .map(|metadata| {
                (
                    metadata.name.clone(),
                    metadata_to_signature(metadata).display,
                )
            })
            .collect::<BTreeMap<_, _>>();

        let first = render_catalog_usage_notes(&metadata, &available);
        let second = render_catalog_usage_notes(&metadata, &available);

        assert_eq!(first, second);
        assert!(first.len() <= MAX_DECLARATION_USAGE_NOTE_BYTES);
        assert!(first.contains("Additional live catalog usage notes omitted for size"));
    }
}
