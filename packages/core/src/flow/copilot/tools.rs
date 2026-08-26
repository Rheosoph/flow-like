use std::{collections::HashSet, sync::Arc};

use rig::{completion::ToolDefinition, tool::Tool};
use schemars::schema_for;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use flow_like_ast::model::{
    Block as FlowScriptBlock, Call as FlowScriptCall, Expr as FlowScriptExpr,
    Stmt as FlowScriptStmt,
};

use super::ir_tools::{
    CheckFlowScriptArgs, CommitFlowScriptArgs, ExtendTimeBudgetArgs, FlowIrAcceptanceBinding,
    FlowIrDraftStore, MAX_BOARD_SCOPE_SEGMENTS, PatchFlowScriptArgs, PlanBoardScopeArgs,
    WriteFlowScriptArgs, accept_scope_plan,
};
use super::platform::PlatformToolBridge;
#[cfg(test)]
use super::provider::MAX_DECLARATION_PRIORITY_BLOCK_BYTES;
use super::provider::{
    CatalogProvider, DECLARATION_PRIORITY_BEGIN, DECLARATION_PRIORITY_END,
    parse_declaration_resolution_metadata,
};
use super::search::score_catalog_metadata;
use super::stream::stream_frame;
use super::tool_spec::{
    find_runtime_execution_tool_spec, find_workflow_context_tool_spec, missing_required_args,
};
use super::types::{BoardCommand, RunContext, TemplateInfo};
use crate::flow::ast::{
    ReconcileResult, RenderOptions, blocked_destructive_flowscript_message, board_to_flowscript,
    destructive_flowscript_command_summaries,
};
use crate::flow::board::Board;
use crate::state::FlowLikeState;

/// Console traces from FlowPilot tools are development diagnostics. Tool results and errors still
/// flow through the normal return path in every build.
macro_rules! flowpilot_debug_log {
    ($($arg:tt)*) => {
        #[cfg(debug_assertions)]
        {
            println!($($arg)*);
        }
    };
}

// ============================================================================
// Tool Error Types
// ============================================================================

#[derive(Debug, thiserror::Error)]
#[error("Catalog tool error")]
pub struct CatalogToolError;

#[derive(Debug, thiserror::Error)]
#[error("Template tool error")]
pub struct TemplateToolError;

#[derive(Debug, thiserror::Error)]
#[error("Get node details tool error: {0}")]
pub struct GetNodeDetailsToolError(pub String);

#[derive(Debug, thiserror::Error)]
#[error("Board inspection tool error: {0}")]
pub struct BoardInspectionToolError(pub String);

#[derive(Debug, thiserror::Error)]
#[error("Emit commands tool error")]
pub struct EmitCommandsToolError;

#[derive(Debug, thiserror::Error)]
#[error("Query logs tool error: {0}")]
pub struct QueryLogsToolError(pub String);

#[derive(Debug, thiserror::Error)]
#[error("FlowScript tool error: {0}")]
pub struct FlowScriptToolError(pub String);

#[derive(Debug, thiserror::Error)]
#[error("Runtime verification tool error: {0}")]
pub struct RuntimeVerificationToolError(pub String);

// ============================================================================
// Tool Argument Types
// ============================================================================

#[derive(Deserialize)]
pub struct SearchArgs {
    pub query: String,
}

#[derive(Deserialize)]
pub struct SearchByPinArgs {
    pub pin_type: String,
    pub is_input: bool,
}

#[derive(Deserialize)]
pub struct FilterCategoryArgs {
    pub category_prefix: String,
}

#[derive(Deserialize)]
pub struct SearchTemplatesArgs {
    pub query: String,
}

#[derive(Deserialize)]
pub struct ThinkingArgs {
    pub thought: String,
}

#[derive(Deserialize)]
pub struct GetNodeDetailsArgs {
    #[serde(default)]
    pub node_id: String,
    /// Batch form: inspect several nodes in ONE call.
    #[serde(default)]
    pub node_ids: Vec<String>,
}

/// Merge the single `node_id` and batch `node_ids` forms into the list of nodes to inspect.
pub fn node_detail_ids(args: &GetNodeDetailsArgs) -> Vec<String> {
    let mut ids: Vec<String> = Vec::new();
    if !args.node_id.trim().is_empty() {
        ids.push(args.node_id.clone());
    }
    for id in &args.node_ids {
        if !id.trim().is_empty() && !ids.iter().any(|existing| existing == id) {
            ids.push(id.clone());
        }
    }
    ids
}

/// Render details for every requested node, joined into one response.
pub fn build_multi_node_details_output(
    args: &GetNodeDetailsArgs,
    graph_context: &GraphContext,
) -> String {
    let ids = node_detail_ids(args);
    if ids.is_empty() {
        return "get_node_details needs `node_id` or a `node_ids` array.".to_string();
    }
    ids.iter()
        .map(|id| build_node_details_output(id, graph_context))
        .collect::<Vec<_>>()
        .join("\n\n")
}

#[derive(Deserialize)]
pub struct FindConnectableNodesArgs {
    pub node_id: String,
    pub pin_name: String,
    pub intent: Option<String>,
    pub limit: Option<usize>,
}

#[derive(Deserialize)]
pub struct EmitCommandsArgs {
    pub commands: Vec<BoardCommand>,
    pub explanation: String,
}

#[derive(Deserialize, Debug)]
pub struct QueryLogsArgs {
    /// Optional filter query (e.g., "log_level = 4" for errors, "node_id = 'abc123'")
    pub filter: Option<String>,
    /// Maximum number of logs to return
    pub limit: Option<usize>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ExecuteEventArgs {
    #[serde(default, skip_serializing_if = "Option::is_none", alias = "appId")]
    pub app_id: Option<String>,
    #[serde(alias = "eventId")]
    pub event_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub payload: Option<Value>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        alias = "streamState"
    )]
    pub stream_state: Option<bool>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ExecuteNodeArgs {
    #[serde(default, skip_serializing_if = "Option::is_none", alias = "appId")]
    pub app_id: Option<String>,
    #[serde(alias = "boardId")]
    pub board_id: String,
    #[serde(alias = "nodeId")]
    pub node_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub payload: Option<Value>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        alias = "streamState"
    )]
    pub stream_state: Option<bool>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct QueryExecutionLogsArgs {
    #[serde(default, skip_serializing_if = "Option::is_none", alias = "appId")]
    pub app_id: Option<String>,
    #[serde(alias = "boardId")]
    pub board_id: String,
    #[serde(alias = "runId")]
    pub run_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none", alias = "query")]
    pub filter: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub offset: Option<usize>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        alias = "runMetadata"
    )]
    pub run_metadata: Option<Value>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GetDeclarationsArgs {
    /// Free-text search for the kinds of nodes you want to call in FlowScript
    /// (e.g. "http request", "parse json", "invoke agent").
    #[serde(default)]
    pub query: String,
    /// Batch form: several focused searches answered in ONE call. Prefer this over
    /// multiple get_declarations round-trips.
    #[serde(default)]
    pub queries: Vec<String>,
}

// A production workflow commonly spans mail, persistence, AI, control-flow, conversion and
// formatting capabilities. Eight searches was too small for those plans and, worse, callers could
// not tell that later searches had been dropped. Keep a generous runtime safety bound while
// reporting anything beyond it explicitly in the tool result.
const MAX_DECLARATION_QUERIES: usize = 32;
const MAX_DECLARATION_QUERY_BYTES: usize = 160;
const MAX_REPORTED_OMITTED_DECLARATION_QUERIES: usize = 32;
// External MCP clients persist oversized tool results to a temporary file and then tempt the
// workflow agent to call a filesystem `Read` tool that is intentionally unavailable. Keep the
// declaration batch self-contained while preserving an equal slice for every requested capability.
const MAX_DECLARATION_RESPONSE_BYTES: usize = 24_000;
const DECLARATION_TRUNCATION_NOTICE: &str = "\n// [Additional matches omitted. Refine this capability only when a validation diagnostic names its node/pin or identifies a related comparison/type-conversion mismatch.]";
const DECLARATION_PRIORITY_TRUNCATION_NOTICE: &str =
    "\n// [Additional matches omitted; priority declaration retained.]";
const DECLARATION_SIGNATURE_TRUNCATION_NOTICE: &str =
    "\n// [Additional matches and usage notes omitted; exact declaration retained.]";
const DECLARATION_OUTPUT_OMISSION_NOTICE: &str = "// [Exact declaration omitted because it exceeds the bounded batch response. Call plan_board_scope exactly once unless the host already accepted a plan, then retain its active segment now; retry this capability in one focused get_declarations call only if a later compiler diagnostic still requires it.]";
const MAX_DECLARATION_PRIORITY_SECTION_IDENTITY_BYTES: usize = 48;

fn declaration_query_key(query: &str) -> String {
    query
        .split_whitespace()
        .map(str::to_lowercase)
        .collect::<Vec<_>>()
        .join(" ")
}

fn bound_declaration_query(query: &str) -> (String, bool) {
    let query = query.split_whitespace().collect::<Vec<_>>().join(" ");
    if query.len() <= MAX_DECLARATION_QUERY_BYTES {
        return (query, false);
    }
    let retained_bytes = MAX_DECLARATION_QUERY_BYTES.saturating_sub(3);
    let mut boundary = retained_bytes;
    while boundary > 0 && !query.is_char_boundary(boundary) {
        boundary -= 1;
    }
    (format!("{}...", &query[..boundary]), true)
}

fn normalized_declaration_queries(args: &GetDeclarationsArgs) -> (Vec<String>, usize) {
    let mut queries: Vec<String> = Vec::new();
    let mut seen = HashSet::new();
    let mut truncated_count = 0;
    for query in std::iter::once(&args.query).chain(args.queries.iter()) {
        let (query, truncated) = bound_declaration_query(query);
        if query.is_empty() {
            continue;
        }
        let key = declaration_query_key(&query);
        if seen.insert(key) {
            if truncated {
                truncated_count += 1;
            }
            queries.push(query);
        }
    }
    (queries, truncated_count)
}

/// Merge the single `query` and batch `queries` forms into the list of searches to run.
pub fn declaration_queries(args: &GetDeclarationsArgs) -> Vec<String> {
    normalized_declaration_queries(args).0
}

#[derive(Debug, PartialEq, Eq)]
struct DeclarationQueryBatch {
    processed: Vec<String>,
    omitted: Vec<String>,
    omitted_count: usize,
    truncated_query_count: usize,
}

fn declaration_query_batch(args: &GetDeclarationsArgs) -> DeclarationQueryBatch {
    let (mut queries, truncated_query_count) = normalized_declaration_queries(args);
    let omitted = if queries.len() > MAX_DECLARATION_QUERIES {
        queries.split_off(MAX_DECLARATION_QUERIES)
    } else {
        Vec::new()
    };
    let omitted_count = omitted.len();
    DeclarationQueryBatch {
        processed: queries,
        omitted: omitted
            .into_iter()
            .take(MAX_REPORTED_OMITTED_DECLARATION_QUERIES)
            .collect(),
        omitted_count,
        truncated_query_count,
    }
}

#[cfg(test)]
fn bound_declaration_sections(sections: &[String]) -> String {
    bound_declaration_sections_to(sections, MAX_DECLARATION_RESPONSE_BYTES)
}

fn bound_declaration_sections_to(sections: &[String], max_bytes: usize) -> String {
    bound_declaration_sections_vec_to(sections, max_bytes).join("\n")
}

fn bound_declaration_sections_vec_to(sections: &[String], max_bytes: usize) -> Vec<String> {
    if sections.is_empty() {
        return Vec::new();
    }

    let separator_bytes = sections.len().saturating_sub(1);
    let available_bytes = max_bytes.saturating_sub(separator_bytes);
    if sections.iter().map(String::len).sum::<usize>() <= available_bytes {
        return sections.to_vec();
    }

    let minimums = sections
        .iter()
        .map(|section| declaration_section_minimum_bytes(section))
        .collect::<Vec<_>>();
    let minimum_total = minimums.iter().sum::<usize>();
    let budgets = if minimum_total <= available_bytes {
        let distributable = available_bytes - minimum_total;
        let per_section_extra = distributable / sections.len();
        let extra_remainder = distributable % sections.len();
        minimums
            .into_iter()
            .enumerate()
            .map(|(index, minimum)| {
                minimum
                    .saturating_add(per_section_extra)
                    .saturating_add(usize::from(index < extra_remainder))
            })
            .collect::<Vec<_>>()
    } else {
        let per_section_bytes = available_bytes
            .checked_div(sections.len())
            .unwrap_or_default();
        vec![per_section_bytes; sections.len()]
    };

    sections
        .iter()
        .zip(budgets)
        .map(|(section, budget)| bound_declaration_section_to(section, budget))
        .collect()
}

fn declaration_priority_block(section: &str) -> Option<&str> {
    let start = section.find(DECLARATION_PRIORITY_BEGIN)?;
    let end = section[start..].find(DECLARATION_PRIORITY_END)?;
    let end = start
        .saturating_add(end)
        .saturating_add(DECLARATION_PRIORITY_END.len());
    section.get(start..end)
}

fn declaration_priority_projection(section: &str) -> Option<String> {
    let block = declaration_priority_block(section)?;
    let identity = declaration_section_identity_line(section);
    let identity = if identity.len() <= MAX_DECLARATION_PRIORITY_SECTION_IDENTITY_BYTES {
        identity
    } else {
        let retained = MAX_DECLARATION_PRIORITY_SECTION_IDENTITY_BYTES.saturating_sub(4);
        let boundary = identity
            .char_indices()
            .map(|(index, _)| index)
            .take_while(|index| *index <= retained)
            .last()
            .unwrap_or_default();
        format!("{}...\n", &identity[..boundary])
    };
    Some(format!("{identity}{block}"))
}

fn declaration_exact_signature_line(section: &str) -> Option<&str> {
    section
        .lines()
        .map(str::trim)
        .find(|line| flow_like_ast::is_signature_line(line))
}

fn declaration_section_identity(section: &str) -> String {
    let identity = declaration_section_identity_line(section);
    if identity.len() <= MAX_DECLARATION_PRIORITY_SECTION_IDENTITY_BYTES {
        return identity;
    }
    let retained = MAX_DECLARATION_PRIORITY_SECTION_IDENTITY_BYTES.saturating_sub(4);
    let boundary = identity
        .char_indices()
        .map(|(index, _)| index)
        .take_while(|index| *index <= retained)
        .last()
        .unwrap_or_default();
    format!("{}...\n", &identity[..boundary])
}

fn declaration_section_identity_line(section: &str) -> String {
    let line = section
        .lines()
        .find(|line| line.trim_start().starts_with("// declaration query:"))
        .or_else(|| section.lines().next())
        .unwrap_or_default();
    format!("{line}\n")
}

fn declaration_exact_projection(section: &str) -> Option<String> {
    let signature = declaration_exact_signature_line(section)?;
    let first_line = section.lines().next().map(str::trim).unwrap_or_default();
    if first_line == signature {
        return Some(format!("{signature}\n"));
    }
    Some(format!(
        "{}{signature}\n",
        declaration_section_identity(section)
    ))
}

fn declaration_section_minimum_bytes(section: &str) -> usize {
    if let Some(exact_projection) = declaration_exact_projection(section) {
        return exact_projection.len();
    }
    section
        .find('\n')
        .map(|index| index.saturating_add(1))
        .unwrap_or(section.len())
        .saturating_add(DECLARATION_TRUNCATION_NOTICE.len())
}

fn minimum_declaration_sections_bytes(sections: &[String]) -> usize {
    sections
        .iter()
        .map(|section| declaration_section_minimum_bytes(section))
        .sum::<usize>()
        .saturating_add(sections.len().saturating_sub(1))
}

fn preferred_declaration_sections_bytes(sections: &[String]) -> usize {
    sections
        .iter()
        .map(|section| {
            declaration_priority_projection(section)
                .map(|projection| {
                    projection
                        .len()
                        .saturating_add(DECLARATION_PRIORITY_TRUNCATION_NOTICE.len())
                })
                .unwrap_or_else(|| declaration_section_minimum_bytes(section))
        })
        .sum::<usize>()
        .saturating_add(sections.len().saturating_sub(1))
}

fn bound_declaration_section_to(section: &str, max_bytes: usize) -> String {
    if section.len() <= max_bytes {
        return section.to_string();
    }
    if let Some(mut priority_projection) = declaration_priority_projection(section)
        && priority_projection.len() <= max_bytes
    {
        if priority_projection
            .len()
            .saturating_add(DECLARATION_PRIORITY_TRUNCATION_NOTICE.len())
            <= max_bytes
        {
            priority_projection.push_str(DECLARATION_PRIORITY_TRUNCATION_NOTICE);
        }
        return priority_projection;
    }
    if let Some(mut exact_projection) = declaration_exact_projection(section)
        && exact_projection.len() <= max_bytes
    {
        if exact_projection
            .len()
            .saturating_add(DECLARATION_SIGNATURE_TRUNCATION_NOTICE.len())
            <= max_bytes
        {
            exact_projection.push_str(DECLARATION_SIGNATURE_TRUNCATION_NOTICE);
        }
        return exact_projection;
    }
    let notice = DECLARATION_TRUNCATION_NOTICE;
    let retained_bytes = max_bytes.saturating_sub(notice.len());
    let boundary = section
        .char_indices()
        .map(|(index, _)| index)
        .take_while(|index| *index <= retained_bytes)
        .last()
        .unwrap_or_default();
    let mut bounded = section[..boundary].to_string();
    bounded.push_str(notice);
    bounded
}

fn declaration_output_omission_section(query: &str) -> String {
    format!("// declaration query: {query}\n{DECLARATION_OUTPUT_OMISSION_NOTICE}")
}

fn bounded_section_retains_exact_signature(original: &str, bounded: &str) -> bool {
    let Some(signature) = declaration_exact_signature_line(original) else {
        return false;
    };
    bounded.lines().map(str::trim).any(|line| line == signature)
}

fn declaration_batch_header(
    batch: &DeclarationQueryBatch,
    sections: &[String],
    output_omitted: &[bool],
    bounded_sections: &[String],
) -> String {
    let mut matched_queries = Vec::new();
    let mut unmatched_queries = Vec::new();
    let mut output_omitted_queries = Vec::new();
    let mut resolution_summaries = Vec::new();
    for (index, query) in batch.processed.iter().enumerate() {
        let resolution = sections
            .get(index)
            .and_then(|section| parse_declaration_resolution_metadata(section));
        let provider_matched = resolution
            .as_ref()
            .is_some_and(|resolution| resolution.status.is_confident());
        resolution_summaries.push(match resolution {
            Some(resolution) => json!({
                "query": query,
                "status": resolution.status,
                "top_score": resolution.top_score,
                "margin": resolution.margin,
                "reason_codes": resolution.reason_codes,
            }),
            None => json!({
                "query": query,
                "status": "unresolved",
                "top_score": null,
                "margin": null,
                "reason_codes": ["missing_resolution_metadata"],
            }),
        });
        if output_omitted.get(index).copied().unwrap_or_default() && provider_matched {
            output_omitted_queries.push(query.clone());
        } else if provider_matched {
            matched_queries.push(query.clone());
        } else {
            unmatched_queries.push(query.clone());
        }
    }
    let complete = unmatched_queries.is_empty()
        && output_omitted_queries.is_empty()
        && batch.omitted_count == 0
        && batch.truncated_query_count == 0;
    let metadata = json!({
        "processed_count": batch.processed.len(),
        "processed_queries": batch.processed,
        "matched_count": matched_queries.len(),
        "matched_queries": matched_queries,
        "unmatched_count": unmatched_queries.len(),
        "unmatched_queries": unmatched_queries,
        "resolutions": resolution_summaries,
        "output_omitted_count": output_omitted_queries.len(),
        "output_omitted_queries": output_omitted_queries,
        "complete": complete,
        "omitted_count": batch.omitted_count,
        "omitted_queries": batch.omitted,
        "omitted_queries_truncated": batch.omitted_count > batch.omitted.len(),
        "truncated_query_count": batch.truncated_query_count,
    });
    let mut header = format!("// flowpilot.declaration-batch/v1 {}\n", metadata);
    if header.len() > MAX_DECLARATION_RESPONSE_BYTES / 2
        || header
            .len()
            .saturating_add(preferred_declaration_sections_bytes(bounded_sections))
            > MAX_DECLARATION_RESPONSE_BYTES
    {
        let mut compact_metadata = json!({
            "processed_count": batch.processed.len(),
            "matched_count": matched_queries.len(),
            "unmatched_count": unmatched_queries.len(),
            "output_omitted_count": output_omitted_queries.len(),
            "complete": complete,
            "omitted_count": batch.omitted_count,
            "query_names_omitted_for_size": true,
            "truncated_query_count": batch.truncated_query_count,
        });
        if let Some(metadata) = compact_metadata.as_object_mut() {
            if !unmatched_queries.is_empty() {
                metadata.insert("unmatched_queries".to_string(), json!(unmatched_queries));
            }
            if !output_omitted_queries.is_empty() {
                metadata.insert(
                    "output_omitted_queries".to_string(),
                    json!(output_omitted_queries),
                );
            }
        }
        header = format!("// flowpilot.declaration-batch/v1 {}\n", compact_metadata);
    }
    header
}

fn render_declaration_query_batch(batch: &DeclarationQueryBatch, sections: &[String]) -> String {
    let provider_matched = batch
        .processed
        .iter()
        .enumerate()
        .map(|(index, _)| {
            sections
                .get(index)
                .and_then(|section| parse_declaration_resolution_metadata(section))
                .is_some_and(|resolution| resolution.status.is_confident())
        })
        .collect::<Vec<_>>();
    let mut output_omitted = vec![false; batch.processed.len()];

    for _ in 0..=batch.processed.len() {
        let effective_sections = batch
            .processed
            .iter()
            .enumerate()
            .map(|(index, query)| {
                if output_omitted[index] {
                    declaration_output_omission_section(query)
                } else {
                    sections.get(index).cloned().unwrap_or_else(|| {
                        format!("// declaration query: {query}\n// No declaration result returned.")
                    })
                }
            })
            .collect::<Vec<_>>();
        let header =
            declaration_batch_header(batch, sections, &output_omitted, &effective_sections);
        let body_budget = MAX_DECLARATION_RESPONSE_BYTES.saturating_sub(header.len());
        let minimum_body_bytes = minimum_declaration_sections_bytes(&effective_sections);

        if minimum_body_bytes <= body_budget {
            let bounded_sections =
                bound_declaration_sections_vec_to(&effective_sections, body_budget);
            let mut newly_omitted = false;
            for index in 0..batch.processed.len() {
                if provider_matched[index]
                    && !output_omitted[index]
                    && !sections.get(index).is_some_and(|original| {
                        bounded_sections.get(index).is_some_and(|bounded| {
                            bounded_section_retains_exact_signature(original, bounded)
                        })
                    })
                {
                    output_omitted[index] = true;
                    newly_omitted = true;
                }
            }
            if newly_omitted {
                continue;
            }
            return format!("{header}{}", bounded_sections.join("\n"));
        }

        let next_omission = provider_matched
            .iter()
            .enumerate()
            .filter(|(index, matched)| **matched && !output_omitted[*index])
            .max_by_key(|(index, _)| {
                sections
                    .get(*index)
                    .map(|section| declaration_section_minimum_bytes(section))
                    .unwrap_or_default()
            })
            .map(|(index, _)| index);
        if let Some(index) = next_omission {
            output_omitted[index] = true;
            continue;
        }

        let bounded_sections = bound_declaration_sections_vec_to(&effective_sections, body_budget);
        return format!("{header}{}", bounded_sections.join("\n"));
    }

    let effective_sections = batch
        .processed
        .iter()
        .map(|query| declaration_output_omission_section(query))
        .collect::<Vec<_>>();
    let output_omitted = provider_matched;
    let header = declaration_batch_header(batch, sections, &output_omitted, &effective_sections);
    let body_budget = MAX_DECLARATION_RESPONSE_BYTES.saturating_sub(header.len());
    let body = bound_declaration_sections_to(&effective_sections, body_budget);
    format!("{header}{body}")
}

/// Run every declaration query against the provider and join the rendered sections.
pub async fn run_declaration_queries(
    provider: &Arc<dyn CatalogProvider>,
    args: &GetDeclarationsArgs,
) -> String {
    let batch = declaration_query_batch(args);
    if batch.processed.is_empty() {
        return provider.get_declarations("").await;
    }
    let declarations = provider.get_declarations_batch(&batch.processed).await;
    let sections = batch
        .processed
        .iter()
        .zip(declarations)
        .map(|(query, declarations)| format!("// declaration query: {query}\n{declarations}"))
        .collect::<Vec<_>>();
    render_declaration_query_batch(&batch, &sections)
}

#[derive(Deserialize)]
pub struct GetCurrentFlowScriptArgs {}

#[derive(Deserialize)]
pub struct EditFlowScriptArgs {
    /// The full edited FlowScript source for the board. Preserve the `//@n:<id>` anchor comments
    /// on existing statements so identities are matched; literal argument changes become pin
    /// updates. Removed anchored statements are blocked unless `allow_deletions` is true.
    #[serde(alias = "script", alias = "source", alias = "content")]
    pub flowscript: String,
    /// Explicit opt-in for destructive FlowScript edits. Leave false unless the user asked to
    /// remove existing board items.
    #[serde(default)]
    pub allow_deletions: bool,
}

// ============================================================================
// Catalog Search Tool
// ============================================================================

pub struct CatalogTool {
    pub provider: Arc<dyn CatalogProvider>,
}

impl Tool for CatalogTool {
    const NAME: &'static str = "catalog_search";

    type Error = CatalogToolError;
    type Args = SearchArgs;
    type Output = String;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: "catalog_search".to_string(),
            description: r#"Search the node catalog by functionality or name for read-only exploration and debugging.

WHEN TO USE: Explore catalog metadata when explaining a board or investigating a declaration issue.
FOR WORKFLOW EDITS: Prefer get_declarations → plan_board_scope exactly once unless already accepted → write_flowscript → patch_flowscript as needed → check_flowscript → commit_flowscript. get_declarations is backed by embedded .flow.d files and returns exact `ns::alias({ pin: type })` signatures plus the `use ns::*` idiom.
EXAMPLE QUERIES: "http request", "parse json", "loop array", "condition if", "open database""#.to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                        "description": "Natural language catalog metadata search. For FlowScript authoring, use get_declarations instead."
                    }
                },
                "required": ["query"]
            }),
        }
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        let matches = self.provider.search(&args.query).await;
        Ok(super::search::render_catalog_search_results(&matches))
    }
}

// ============================================================================
// Search By Pin Tool
// ============================================================================

pub struct SearchByPinTool {
    pub provider: Arc<dyn CatalogProvider>,
}

impl Tool for SearchByPinTool {
    const NAME: &'static str = "search_by_pin";

    type Error = CatalogToolError;
    type Args = SearchByPinArgs;
    type Output = String;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: "search_by_pin".to_string(),
            description: r#"Find nodes compatible with a specific pin type. Use this to find nodes that can connect to an existing node's pin.

WHEN TO USE: Finding what can connect to/from a specific pin type
EXAMPLES: search_by_pin("String", true) finds nodes with String input pins"#.to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "pin_type": {
                        "type": "string",
                        "description": "Data type: String, Integer, Float, Boolean, Struct, Generic, Date, PathBuf, Byte, Execution"
                    },
                    "is_input": {
                        "type": "boolean",
                        "description": "true = find nodes with this INPUT pin type, false = find nodes with this OUTPUT pin type"
                    }
                },
                "required": ["pin_type", "is_input"]
            }),
        }
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        let matches = self
            .provider
            .search_by_pin_type(&args.pin_type, args.is_input)
            .await;
        // Use compact format for token efficiency
        let compact: Vec<String> = matches.iter().map(|m| m.to_compact()).collect();
        Ok(compact.join("\n"))
    }
}

// ============================================================================
// Filter Category Tool
// ============================================================================

pub struct FilterCategoryTool {
    pub provider: Arc<dyn CatalogProvider>,
}

impl Tool for FilterCategoryTool {
    const NAME: &'static str = "filter_category";

    type Error = CatalogToolError;
    type Args = FilterCategoryArgs;
    type Output = String;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: "filter_category".to_string(),
            description: r#"Browse nodes by category. Categories are hierarchical (e.g., "flow/control", "data/transform").

WHEN TO USE: Exploring what nodes exist in a domain
COMMON CATEGORIES: flow, data, http, math, logic, string, array"#.to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "category_prefix": {
                        "type": "string",
                        "description": "Category prefix like 'flow', 'data', 'http', 'math'. Use '/' for subcategories: 'flow/control'"
                    }
                },
                "required": ["category_prefix"]
            }),
        }
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        let matches = self
            .provider
            .filter_by_category(&args.category_prefix)
            .await;
        // Use compact format for token efficiency
        let compact: Vec<String> = matches.iter().map(|m| m.to_compact()).collect();
        Ok(compact.join("\n"))
    }
}

// ============================================================================
// Search Templates Tool
// ============================================================================

pub struct SearchTemplatesTool {
    pub templates: Vec<TemplateInfo>,
    pub current_template_id: Option<String>,
}

impl Tool for SearchTemplatesTool {
    const NAME: &'static str = "search_templates";

    type Error = TemplateToolError;
    type Args = SearchTemplatesArgs;
    type Output = String;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: "search_templates".to_string(),
            description: r#"Search for workflow templates - reusable patterns that can be instantiated. Templates contain pre-built node configurations.

WHEN TO USE: User asks for a "template", "example", or common workflow pattern
RETURNS: Template info with node_types used (helpful for understanding structure)"#.to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                        "description": "Search by name, description, tags, or node types used in the template"
                    }
                },
                "required": ["query"]
            }),
        }
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        let query_lower = args.query.to_lowercase();

        // Filter matching templates, excluding current template being edited
        let mut matches: Vec<&TemplateInfo> = self
            .templates
            .iter()
            .filter(|t| {
                // Skip the current template being edited
                if let Some(ref current_id) = self.current_template_id
                    && &t.id == current_id
                {
                    return false;
                }
                t.name.to_lowercase().contains(&query_lower)
                    || t.description.to_lowercase().contains(&query_lower)
                    || t.tags
                        .iter()
                        .any(|tag| tag.to_lowercase().contains(&query_lower))
                    || t.node_types
                        .iter()
                        .any(|nt| nt.to_lowercase().contains(&query_lower))
            })
            .take(5) // Limit results to reduce context
            .collect();

        // Sort by relevance: exact name match first, then description match
        matches.sort_by(|a, b| {
            let a_name_match = a.name.to_lowercase().contains(&query_lower);
            let b_name_match = b.name.to_lowercase().contains(&query_lower);
            b_name_match.cmp(&a_name_match)
        });

        Ok(serde_json::to_string(&matches).unwrap_or_default())
    }
}

// ============================================================================
// Get Node Details Tool
// ============================================================================

use super::context::GraphContext;

pub struct GetNodeDetailsTool {
    pub graph_context: Arc<GraphContext>,
}

// ============================================================================
// List Board Nodes Tool
// ============================================================================

pub struct ListBoardNodesTool {
    pub graph_context: Arc<GraphContext>,
}

impl Tool for ListBoardNodesTool {
    const NAME: &'static str = "list_board_nodes";

    type Error = BoardInspectionToolError;
    type Args = serde_json::Value;
    type Output = String;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: "list_board_nodes".to_string(),
            description:
                r#"List all nodes and layers in the current workflow with their IDs and positions.

USE THIS FIRST to understand the workflow before making changes.

RETURNS:
- node_id: Use in get_node_details or visual-only MoveNode
- node_type: The node's catalog type
- friendly_name: Human-readable name
- position: {x, y} - use to compute visual layout targets

WORKFLOW:
1. list_board_nodes → see all nodes and positions
2. get_node_details on relevant node → get pin names
3. get_declarations → find signatures, then plan_board_scope exactly once unless already accepted,
   then write/patch/check/commit FlowScript for behavior;
   emit_commands is only for position-only MoveNode and canvas comments"#
                    .to_string(),
            parameters: json!({
                "type": "object",
                "properties": {},
                "additionalProperties": false
            }),
        }
    }

    async fn call(&self, _args: Self::Args) -> Result<Self::Output, Self::Error> {
        Ok(build_list_board_nodes_output(&self.graph_context))
    }
}

// ============================================================================
// Get Unconfigured Nodes Tool
// ============================================================================

pub struct GetUnconfiguredNodesTool {
    pub graph_context: Arc<GraphContext>,
}

impl Tool for GetUnconfiguredNodesTool {
    const NAME: &'static str = "get_unconfigured_nodes";

    type Error = BoardInspectionToolError;
    type Args = serde_json::Value;
    type Output = String;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: "get_unconfigured_nodes".to_string(),
            description: r#"Find nodes that need configuration - inputs with no value and no incoming connection.

WHEN TO USE:
- Check what needs to be configured in the workflow
- Find nodes that aren't fully set up
- Identify missing connections
- After planning or after failed FlowScript compiler diagnostics

RETURNS: List of nodes with their unconfigured non-execution input pins"#.to_string(),
            parameters: json!({
                "type": "object",
                "properties": {},
                "additionalProperties": false
            }),
        }
    }

    async fn call(&self, _args: Self::Args) -> Result<Self::Output, Self::Error> {
        Ok(build_unconfigured_nodes_output(&self.graph_context))
    }
}

// ============================================================================
// Find Connectable Nodes Tool
// ============================================================================

pub struct FindConnectableNodesTool {
    pub provider: Arc<dyn CatalogProvider>,
    pub graph_context: Arc<GraphContext>,
}

impl Tool for FindConnectableNodesTool {
    const NAME: &'static str = "find_connectable_nodes";

    type Error = BoardInspectionToolError;
    type Args = FindConnectableNodesArgs;
    type Output = String;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: "find_connectable_nodes".to_string(),
            description: r#"Read-only discovery of catalog nodes compatible with a specific existing pin, reranked by intent. Use it for explanation/debugging; author executable follow-up nodes through exact get_declarations signatures and the FlowScript lifecycle."#.to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "node_id": {
                        "type": "string",
                        "description": "Existing node or layer ID from the current graph"
                    },
                    "pin_name": {
                        "type": "string",
                        "description": "Pin name on that node/layer"
                    },
                    "intent": {
                        "type": "string",
                        "description": "Optional desired outcome for reranking, e.g. 'send email' or 'read unread inbox messages'"
                    },
                    "limit": {
                        "type": "integer",
                        "description": "Maximum candidates to return (default 8, max 20)"
                    }
                },
                "required": ["node_id", "pin_name"],
                "additionalProperties": false
            }),
        }
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        build_find_connectable_nodes_output(&self.graph_context, self.provider.as_ref(), args).await
    }
}

impl Tool for GetNodeDetailsTool {
    const NAME: &'static str = "get_node_details";

    type Error = GetNodeDetailsToolError;
    type Args = GetNodeDetailsArgs;
    type Output = String;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: "get_node_details".to_string(),
            description:
                r#"Get full details about nodes including position, all pins, and connections. BATCH-FIRST: pass every node you plan to touch in ONE call via `node_ids`.

Use this to explain/debug exact pins and to compute position-only MoveNode layout changes. It is
read-only; executable connection and pin edits must be authored in FlowScript.

RETURNS (per node):
- position: {x, y} - use this to compute absolute visual move targets
- inputs/outputs: Array of pins with {name, type, value}
- incoming/outgoing: Current connections

EXAMPLE USE:
1. Call get_node_details once with node_ids: [all nodes you will inspect or reposition]
2. Note their positions (e.g., {x: 500, y: 200})
3. Compute non-overlapping absolute MoveNode positions without changing layer membership
4. For behavior changes, use these details only as diagnostics and repair the retained FlowScript"#
                    .to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "node_ids": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "PREFERRED. All node IDs to inspect in one call (from list_board_nodes or context)."
                    },
                    "node_id": {
                        "type": "string",
                        "description": "Single-node fallback. Prefer `node_ids` with every relevant node batched."
                    }
                }
            }),
        }
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        Ok(build_multi_node_details_output(&args, &self.graph_context))
    }
}

/// Full JSON details of one node (pins, values, connections) or a not-found message. Single
/// source for the `get_node_details` tool across every backend executor.
pub fn build_node_details_output(node_id: &str, graph_context: &GraphContext) -> String {
    let Some(node_ctx) = graph_context.nodes.iter().find(|n| n.id == node_id) else {
        return format!("Node with ID '{}' not found in the current graph", node_id);
    };

    let incoming_edges: Vec<_> = graph_context
        .edges
        .iter()
        .filter(|e| e.to_node_id == node_id)
        .map(|e| {
            json!({
                "from_node": e.from_node_id,
                "from_pin": e.from_pin_name,
                "to_pin": e.to_pin_name
            })
        })
        .collect();

    let outgoing_edges: Vec<_> = graph_context
        .edges
        .iter()
        .filter(|e| e.from_node_id == node_id)
        .map(|e| {
            json!({
                "from_pin": e.from_pin_name,
                "to_node": e.to_node_id,
                "to_pin": e.to_pin_name
            })
        })
        .collect();

    let details = json!({
        "id": node_ctx.id,
        "node_type": node_ctx.node_type,
        "friendly_name": node_ctx.friendly_name,
        "position": { "x": node_ctx.position.0, "y": node_ctx.position.1 },
        "size": { "width": node_ctx.estimated_size.0, "height": node_ctx.estimated_size.1 },
        "inputs": node_ctx.inputs.iter().map(|p| {
            json!({
                "name": p.name,
                "type": p.type_name,
                "default_value": p.default_value
            })
        }).collect::<Vec<_>>(),
        "outputs": node_ctx.outputs.iter().map(|p| {
            json!({
                "name": p.name,
                "type": p.type_name
            })
        }).collect::<Vec<_>>(),
        "incoming_connections": incoming_edges,
        "outgoing_connections": outgoing_edges,
        "is_selected": graph_context.selected_nodes.contains(&node_id.to_string())
    });

    serde_json::to_string_pretty(&details).unwrap_or_default()
}

/// The `(name, description, parameters)` triple of a rig tool definition, so non-rig adapters
/// (Copilot SDK, MCP) can advertise exactly the same definition as the rig loop.
pub async fn tool_definition_parts<T: Tool>(tool: &T) -> (String, String, serde_json::Value) {
    let definition = tool.definition(String::new()).await;
    (
        definition.name,
        definition.description,
        definition.parameters,
    )
}

// ============================================================================
// Emit Commands Tool
// ============================================================================

/// Model-facing `emit_commands` definition. Its schema intentionally contains only board visuals
/// which FlowScript source cannot express. The complete `BoardCommand` transaction language and
/// its legacy tool remain available to host internals below.
pub struct ModelFacingEmitCommandsTool;

fn model_facing_emit_commands_parameters() -> Value {
    let position = || {
        json!({
            "type": "object",
            "properties": {
                "x": { "type": "number" },
                "y": { "type": "number" }
            },
            "required": ["x", "y"],
            "additionalProperties": false
        })
    };

    json!({
        "type": "object",
        "properties": {
            "commands": {
                "type": "array",
                "minItems": 1,
                "maxItems": 20,
                "description": "Visual-only board operations. Executable behavior must be authored with FlowScript.",
                "items": {
                    "oneOf": [
                        {
                            "type": "object",
                            "properties": {
                                "command_type": { "const": "MoveNode" },
                                "node_id": { "type": "string", "description": "Existing node id from board context" },
                                "position": position(),
                                "summary": { "type": "string" }
                            },
                            "required": ["command_type", "node_id", "position", "summary"],
                            "additionalProperties": false
                        },
                        {
                            "type": "object",
                            "properties": {
                                "command_type": { "const": "CreateComment" },
                                "content": { "type": "string" },
                                "position": position(),
                                "width": { "type": "number" },
                                "height": { "type": "number" },
                                "color": { "type": "string" },
                                "target_layer": { "type": "string", "description": "Optional existing visual layer id" },
                                "summary": { "type": "string" }
                            },
                            "required": ["command_type", "content", "position", "summary"],
                            "additionalProperties": false
                        },
                        {
                            "type": "object",
                            "properties": {
                                "command_type": { "const": "DeleteComment" },
                                "comment_id": { "type": "string" },
                                "summary": { "type": "string" }
                            },
                            "required": ["command_type", "comment_id", "summary"],
                            "additionalProperties": false
                        }
                    ]
                }
            },
            "explanation": {
                "type": "string",
                "description": "Overall explanation of the visual organization change"
            }
        },
        "required": ["commands", "explanation"],
        "additionalProperties": false
    })
}

impl Tool for ModelFacingEmitCommandsTool {
    const NAME: &'static str = "emit_commands";

    type Error = EmitCommandsToolError;
    type Args = EmitCommandsArgs;
    type Output = String;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: Self::NAME.to_string(),
            description: r#"Apply visual board organization which FlowScript source cannot express.

ALLOWED ONLY:
- MoveNode: reposition an existing node without changing layer membership
- CreateComment / DeleteComment: manage canvas notes

Executable workflow behavior is never accepted here. Node add/removal, connections, pin values,
variables, placeholders, function layers/references, and layer-membership moves are rejected;
direct layer creation/removal is also unavailable because it can rewrite executable membership.
Author executable behavior with write_flowscript, repair with patch_flowscript, validate
with check_flowscript, and queue with commit_flowscript."#
                .to_string(),
            parameters: model_facing_emit_commands_parameters(),
        }
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        let scope = super::validation::validate_model_facing_emit_commands_scope(&args);
        if !scope.errors.is_empty() {
            return Ok(super::validation::render_emit_commands_result(
                &args, &scope,
            ));
        }

        EmitCommandsTool.call(args).await
    }
}

/// Legacy complete command schema retained for host/internal compatibility. Model-facing board
/// builders must register `ModelFacingEmitCommandsTool` instead.
struct EmitCommandsTool;

impl Tool for EmitCommandsTool {
    const NAME: &'static str = "emit_commands";

    type Error = EmitCommandsToolError;
    type Args = EmitCommandsArgs;
    type Output = String;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: "emit_commands".to_string(),
            description: r#"Execute low-level graph modifications. Commands are batched and applied atomically with undo support.

PRIMARY WORKFLOW EDIT PATH:
Use get_declarations to search embedded .flow.d signatures, call plan_board_scope exactly once
unless the host already accepted a plan, then write_flowscript, repair with patch_flowscript,
check_flowscript, and commit_flowscript.

LOW-LEVEL FALLBACK WORKFLOW:
1. Use catalog_search to get exact node_type
2. Use get_node_details for pin names
3. Emit commands with ref_ids to chain operations

Use this directly only for layout-only MoveNode changes, placeholders/comments/layers, variables, or changes that cannot be represented as FlowScript.

COMMAND TYPES:
- AddNode: Add a node (requires node_type from catalog)
- AddPlaceholder: Add a placeholder with custom pins
- ConnectPins: Connect two pins (use pin NAME, not ID)
- UpdateNodePin: Set a pin's value
- RemoveNode: Delete a node
- CreateVariable/UpdateVariable/DeleteVariable
- CreateComment/DeleteComment
- CreateLayer/RemoveLayer

REF_IDS: Use '$0', '$1', etc. to reference nodes in same batch"#.to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "commands": {
                        "type": "array",
                        "description": "Commands to execute. Use ref_id ('$0', '$1') for cross-referencing new nodes.",
                        "items": {
                            "type": "object",
                            "oneOf": [
                                {
                                    "properties": {
                                        "command_type": { "const": "AddNode" },
                                        "node_type": { "type": "string", "description": "EXACT node_type from catalog_search (e.g., 'string_contains', 'control_branch')" },
                                        "ref_id": { "type": "string", "description": "Reference ID like '$0', '$1' to use in ConnectPins/UpdateNodePin" },
                                        "position": {
                                            "type": "object",
                                            "properties": { "x": { "type": "number" }, "y": { "type": "number" } }
                                        },
                                        "friendly_name": { "type": "string", "description": "Optional display name" },
                                        "additional_pins": {
                                            "type": "array",
                                            "description": "Additional non-execution Output pins. Supported only for events_generic; normally generated from FlowScript event parameters.",
                                            "items": {
                                                "type": "object",
                                                "properties": {
                                                    "name": { "type": "string" },
                                                    "friendly_name": { "type": "string" },
                                                    "description": { "type": "string" },
                                                    "pin_type": { "const": "Output" },
                                                    "data_type": { "type": "string", "enum": ["String", "Integer", "Float", "Boolean", "Struct", "Generic", "Date", "PathBuf", "Byte"] },
                                                    "value_type": { "type": "string", "enum": ["Normal", "Array", "HashMap", "HashSet"] }
                                                },
                                                "required": ["name", "friendly_name", "pin_type", "data_type"]
                                            }
                                        },
                                        "target_layer": { "type": "string", "description": "Layer ID for placement. Omit for root layer." },
                                        "summary": { "type": "string", "description": "Brief description, e.g. 'Add HTTP GET node'" }
                                    },
                                    "required": ["command_type", "node_type", "ref_id", "position", "summary"]
                                },
                                {
                                    "properties": {
                                        "command_type": { "const": "AddPlaceholder" },
                                        "name": { "type": "string", "description": "Name for the placeholder node (e.g., 'Process Order', 'Validate Input')" },
                                        "ref_id": { "type": "string", "description": "Reference ID for this placeholder (e.g., '$0', '$1') to use in subsequent commands" },
                                        "position": {
                                            "type": "object",
                                            "properties": { "x": { "type": "number" }, "y": { "type": "number" } }
                                        },
                                        "pins": {
                                            "type": "array",
                                            "description": "Custom pins to add to the placeholder (beyond the default exec_in/exec_out)",
                                            "items": {
                                                "type": "object",
                                                "properties": {
                                                    "name": { "type": "string", "description": "Internal name for the pin (e.g., 'order_data')" },
                                                    "friendly_name": { "type": "string", "description": "Display name (e.g., 'Order Data')" },
                                                    "description": { "type": "string", "description": "Optional description" },
                                                    "pin_type": { "type": "string", "enum": ["Input", "Output"], "description": "Whether this is an input or output pin" },
                                                    "data_type": { "type": "string", "enum": ["String", "Integer", "Float", "Boolean", "Struct", "Generic", "Execution"], "description": "The data type of the pin" },
                                                    "value_type": { "type": "string", "enum": ["Normal", "Array", "HashMap", "HashSet"], "description": "Value type (default: Normal)" }
                                                },
                                                "required": ["name", "friendly_name", "pin_type", "data_type"]
                                            }
                                        },
                                        "target_layer": { "type": "string", "description": "Layer ID to place the placeholder in. Use layer ID from context. Omit for root/current layer." },
                                        "summary": { "type": "string", "description": "Human-readable summary, e.g. 'Add placeholder for order processing'" }
                                    },
                                    "required": ["command_type", "name", "ref_id", "position", "summary"]
                                },
                                {
                                    "properties": {
                                        "command_type": { "const": "RemoveNode" },
                                        "node_id": { "type": "string", "description": "The ID of the node to remove" },
                                        "summary": { "type": "string", "description": "Human-readable summary, e.g. 'Remove the unused filter node'" }
                                    },
                                    "required": ["command_type", "node_id", "summary"]
                                },
                                {
                                    "properties": {
                                        "command_type": { "const": "ConnectPins" },
                                        "from_node": { "type": "string", "description": "Source node ID or ref_id (e.g., '$0')" },
                                        "from_pin": { "type": "string", "description": "Output pin NAME (not ID)" },
                                        "to_node": { "type": "string", "description": "Target node ID or ref_id (e.g., '$1')" },
                                        "to_pin": { "type": "string", "description": "Input pin NAME (not ID)" },
                                        "summary": { "type": "string", "description": "Human-readable summary, e.g. 'Connect output to input'" }
                                    },
                                    "required": ["command_type", "from_node", "from_pin", "to_node", "to_pin", "summary"]
                                },
                                {
                                    "properties": {
                                        "command_type": { "const": "DisconnectPins" },
                                        "from_node": { "type": "string" },
                                        "from_pin": { "type": "string" },
                                        "to_node": { "type": "string" },
                                        "to_pin": { "type": "string" },
                                        "summary": { "type": "string", "description": "Human-readable summary" }
                                    },
                                    "required": ["command_type", "from_node", "from_pin", "to_node", "to_pin", "summary"]
                                },
                                {
                                    "properties": {
                                        "command_type": { "const": "UpdateNodePin" },
                                        "node_id": { "type": "string", "description": "Node ID or ref_id (e.g., '$0')" },
                                        "pin_id": { "type": "string", "description": "Pin NAME (use internal name from catalog, not friendly_name)" },
                                        "value": { "description": "The new value for the pin" },
                                        "summary": { "type": "string", "description": "Human-readable summary, e.g. 'Set threshold to 0.5'" }
                                    },
                                    "required": ["command_type", "node_id", "pin_id", "value", "summary"]
                                },
                                {
                                    "properties": {
                                        "command_type": { "const": "MoveNode" },
                                        "node_id": { "type": "string" },
                                        "position": {
                                            "type": "object",
                                            "properties": { "x": { "type": "number" }, "y": { "type": "number" } },
                                            "required": ["x", "y"]
                                        },
                                        "target_layer": { "type": "string", "description": "Layer ID to move the node to. Use layer ID from context." },
                                        "summary": { "type": "string", "description": "Human-readable summary" }
                                    },
                                    "required": ["command_type", "node_id", "position", "summary"]
                                },
                                {
                                    "properties": {
                                        "command_type": { "const": "CreateVariable" },
                                        "variable_id": { "type": "string", "description": "Optional variable ID. Omit to let the frontend generate one." },
                                        "name": { "type": "string", "description": "Variable name" },
                                        "data_type": { "type": "string", "description": "Data type: String, Integer, Float, Boolean, Struct, etc." },
                                        "value_type": { "type": "string", "description": "Value type: Normal, Array, HashMap, HashSet" },
                                        "default_value": { "description": "Optional default value" },
                                        "description": { "type": "string", "description": "Optional description" },
                                        "category": { "type": "string", "description": "Optional UI category" },
                                        "schema": { "type": "string", "description": "Optional JSON Schema for Struct variables" },
                                        "exposed": { "type": "boolean" },
                                        "secret": { "type": "boolean" },
                                        "editable": { "type": "boolean" },
                                        "runtime_configured": { "type": "boolean" },
                                        "target_layer": { "type": "string", "description": "Optional layer ID for local variables" },
                                        "summary": { "type": "string", "description": "Human-readable summary" }
                                    },
                                    "required": ["command_type", "name", "data_type", "value_type", "summary"]
                                },
                                {
                                    "properties": {
                                        "command_type": { "const": "UpdateVariable" },
                                        "variable_id": { "type": "string", "description": "Variable ID from context" },
                                        "name": { "type": "string", "description": "Optional new name" },
                                        "data_type": { "type": "string", "description": "Optional new data type" },
                                        "value_type": { "type": "string", "description": "Optional new value type" },
                                        "default_value": { "description": "Optional new default value" },
                                        "clear_default_value": { "type": "boolean", "description": "Set true to remove the default value" },
                                        "description": { "type": "string", "description": "Optional new description" },
                                        "clear_description": { "type": "boolean", "description": "Set true to remove the description" },
                                        "category": { "type": "string", "description": "Optional new category" },
                                        "clear_category": { "type": "boolean", "description": "Set true to remove the category" },
                                        "schema": { "type": "string", "description": "Optional new JSON Schema" },
                                        "clear_schema": { "type": "boolean", "description": "Set true to remove the schema" },
                                        "exposed": { "type": "boolean" },
                                        "secret": { "type": "boolean" },
                                        "editable": { "type": "boolean" },
                                        "runtime_configured": { "type": "boolean" },
                                        "summary": { "type": "string", "description": "Human-readable summary" }
                                    },
                                    "required": ["command_type", "variable_id", "summary"]
                                },
                                {
                                    "properties": {
                                        "command_type": { "const": "DeleteVariable" },
                                        "variable_id": { "type": "string", "description": "Variable ID from context" },
                                        "summary": { "type": "string", "description": "Human-readable summary" }
                                    },
                                    "required": ["command_type", "variable_id", "summary"]
                                },
                                {
                                    "properties": {
                                        "command_type": { "const": "CreateComment" },
                                        "content": { "type": "string", "description": "Comment text" },
                                        "position": {
                                            "type": "object",
                                            "properties": { "x": { "type": "number" }, "y": { "type": "number" } }
                                        },
                                        "width": { "type": "number", "description": "Comment width in pixels (default: 200)" },
                                        "height": { "type": "number", "description": "Comment height in pixels (default: 100)" },
                                        "color": { "type": "string", "description": "Optional hex color (e.g. #FFD700)" },
                                        "target_layer": { "type": "string", "description": "Layer ID to place the comment in. Use layer ID from context. Omit for root/current layer." },
                                        "summary": { "type": "string", "description": "Human-readable summary" }
                                    },
                                    "required": ["command_type", "content", "position", "summary"]
                                },
                                {
                                    "properties": {
                                        "command_type": { "const": "DeleteComment" },
                                        "comment_id": { "type": "string", "description": "Comment ID from context" },
                                        "summary": { "type": "string", "description": "Human-readable summary" }
                                    },
                                    "required": ["command_type", "comment_id", "summary"]
                                },
                                {
                                    "properties": {
                                        "command_type": { "const": "CreateLayer" },
                                        "name": { "type": "string", "description": "Layer name" },
                                        "color": { "type": "string", "description": "Optional layer color" },
                                        "node_ids": { "type": "array", "items": { "type": "string" }, "description": "Node IDs to include" },
                                        "position": {
                                            "type": "object",
                                            "properties": { "x": { "type": "number" }, "y": { "type": "number" } }
                                        },
                                        "target_layer": { "type": "string", "description": "Parent layer ID for nesting. Use layer ID from context. Omit for root layer." },
                                        "summary": { "type": "string", "description": "Human-readable summary" }
                                    },
                                    "required": ["command_type", "name", "node_ids", "summary"]
                                },
                                {
                                    "properties": {
                                        "command_type": { "const": "RemoveLayer" },
                                        "layer_id": { "type": "string", "description": "Layer ID from context" },
                                        "summary": { "type": "string", "description": "Human-readable summary" }
                                    },
                                    "required": ["command_type", "layer_id", "summary"]
                                }
                            ]
                        }
                    },
                    "explanation": {
                        "type": "string",
                        "description": "Overall explanation of what these commands accomplish together"
                    }
                },
                "required": ["commands", "explanation"]
            }),
        }
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        // Build a human-readable summary for the model to understand what was done
        let mut summary_lines: Vec<String> = Vec::new();
        summary_lines.push(format!("✓ Queued {} commands:", args.commands.len()));

        for cmd in &args.commands {
            let cmd_summary = match cmd {
                BoardCommand::AddNode {
                    node_type,
                    ref_id,
                    friendly_name,
                    ..
                } => {
                    format!(
                        "  - AddNode: {} as {} (ref: {})",
                        friendly_name.as_deref().unwrap_or(node_type),
                        node_type,
                        ref_id.as_deref().unwrap_or("none")
                    )
                }
                BoardCommand::AddPlaceholder {
                    name, ref_id, pins, ..
                } => {
                    let pin_count = pins.as_ref().map(|p| p.len()).unwrap_or(0);
                    format!(
                        "  - AddPlaceholder: \"{}\" (ref: {}, {} custom pins)",
                        name,
                        ref_id.as_deref().unwrap_or("none"),
                        pin_count
                    )
                }
                BoardCommand::ConnectPins {
                    from_node,
                    from_pin,
                    to_node,
                    to_pin,
                    ..
                } => {
                    format!(
                        "  - Connect: {}.{} → {}.{}",
                        from_node, from_pin, to_node, to_pin
                    )
                }
                BoardCommand::RemoveNode { node_id, .. } => {
                    format!("  - RemoveNode: {}", node_id)
                }
                BoardCommand::UpdateNodePin {
                    node_id, pin_id, ..
                } => {
                    format!("  - UpdatePin: {}.{}", node_id, pin_id)
                }
                BoardCommand::CreateVariable { name, .. } => {
                    format!("  - CreateVariable: {}", name)
                }
                BoardCommand::UpdateVariable { variable_id, .. } => {
                    format!("  - UpdateVariable: {}", variable_id)
                }
                BoardCommand::RemoveVariable { variable_id, .. } => {
                    format!("  - DeleteVariable: {}", variable_id)
                }
                BoardCommand::AddComment {
                    content,
                    width,
                    height,
                    color,
                    ..
                } => {
                    let preview = if content.chars().count() > 30 {
                        let truncated: String = content.chars().take(30).collect();
                        format!("{}...", truncated)
                    } else {
                        content.clone()
                    };
                    let size_info = match (width, height) {
                        (Some(w), Some(h)) => format!(" ({}x{})", w, h),
                        _ => String::new(),
                    };
                    let color_info = color
                        .as_ref()
                        .map(|c| format!(" [{}]", c))
                        .unwrap_or_default();
                    format!("  - AddComment: \"{}\"{}{}", preview, size_info, color_info)
                }
                _ => format!("  - {:?}", cmd),
            };
            summary_lines.push(cmd_summary);
        }

        summary_lines.push(format!("\nExplanation: {}", args.explanation));
        summary_lines.push(
            "\n⚠️ These commands are now queued. Do NOT emit the same commands again.".to_string(),
        );

        // Return the commands as JSON wrapped in a special tag for parsing, plus the summary
        let commands_json = serde_json::to_string(&args.commands).unwrap_or_default();
        Ok(format!(
            "<commands>{}</commands>\n\n{}",
            commands_json,
            summary_lines.join("\n")
        ))
    }
}

// ============================================================================
// Runtime Verification Tools (desktop bridge)
// ============================================================================

fn workflow_context_tool_definition(name: &str) -> ToolDefinition {
    find_workflow_context_tool_spec(name)
        .expect("workflow context tool spec must exist")
        .to_tool_definition()
}

async fn call_workflow_context_tool(
    bridge: &Arc<dyn PlatformToolBridge>,
    name: &str,
    arguments: Value,
) -> Result<String, RuntimeVerificationToolError> {
    let spec = find_workflow_context_tool_spec(name)
        .ok_or_else(|| RuntimeVerificationToolError(format!("missing tool spec for {name}")))?;
    if let Some(error) = missing_required_args(&spec, &arguments) {
        return Err(RuntimeVerificationToolError(error));
    }
    Ok(bridge.call(name, arguments).await)
}

pub struct DatabaseContextTool {
    pub bridge: Arc<dyn PlatformToolBridge>,
}

impl Tool for DatabaseContextTool {
    const NAME: &'static str = "database_tool";
    type Error = RuntimeVerificationToolError;
    type Args = Value;
    type Output = String;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        workflow_context_tool_definition(Self::NAME)
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        call_workflow_context_tool(&self.bridge, Self::NAME, args).await
    }
}

pub struct StorageContextTool {
    pub bridge: Arc<dyn PlatformToolBridge>,
}

impl Tool for StorageContextTool {
    const NAME: &'static str = "storage_tool";
    type Error = RuntimeVerificationToolError;
    type Args = Value;
    type Output = String;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        workflow_context_tool_definition(Self::NAME)
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        call_workflow_context_tool(&self.bridge, Self::NAME, args).await
    }
}

pub struct UiInspectContextTool {
    pub bridge: Arc<dyn PlatformToolBridge>,
}

impl Tool for UiInspectContextTool {
    const NAME: &'static str = "ui_inspect";
    type Error = RuntimeVerificationToolError;
    type Args = Value;
    type Output = String;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        workflow_context_tool_definition(Self::NAME)
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        call_workflow_context_tool(&self.bridge, Self::NAME, args).await
    }
}

fn runtime_tool_definition(name: &str) -> ToolDefinition {
    find_runtime_execution_tool_spec(name)
        .expect("runtime execution tool spec must exist")
        .to_tool_definition()
}

fn runtime_tool_arguments<T: Serialize>(
    name: &str,
    args: T,
) -> Result<Value, RuntimeVerificationToolError> {
    let arguments = serde_json::to_value(args)
        .map_err(|error| RuntimeVerificationToolError(error.to_string()))?;
    let spec = find_runtime_execution_tool_spec(name)
        .ok_or_else(|| RuntimeVerificationToolError(format!("missing tool spec for {name}")))?;
    if let Some(error) = missing_required_args(&spec, &arguments) {
        return Err(RuntimeVerificationToolError(error));
    }
    Ok(arguments)
}

pub struct ExecuteEventTool {
    pub bridge: Arc<dyn PlatformToolBridge>,
}

impl Tool for ExecuteEventTool {
    const NAME: &'static str = "execute_event";

    type Error = RuntimeVerificationToolError;
    type Args = ExecuteEventArgs;
    type Output = String;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        runtime_tool_definition(Self::NAME)
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        let arguments = runtime_tool_arguments(Self::NAME, args)?;
        Ok(self.bridge.call(Self::NAME, arguments).await)
    }
}

pub struct ExecuteNodeTool {
    pub bridge: Arc<dyn PlatformToolBridge>,
}

impl Tool for ExecuteNodeTool {
    const NAME: &'static str = "execute_node";

    type Error = RuntimeVerificationToolError;
    type Args = ExecuteNodeArgs;
    type Output = String;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        runtime_tool_definition(Self::NAME)
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        let arguments = runtime_tool_arguments(Self::NAME, args)?;
        Ok(self.bridge.call(Self::NAME, arguments).await)
    }
}

pub struct QueryExecutionLogsTool {
    pub bridge: Arc<dyn PlatformToolBridge>,
}

impl Tool for QueryExecutionLogsTool {
    const NAME: &'static str = "query_execution_logs";

    type Error = RuntimeVerificationToolError;
    type Args = QueryExecutionLogsArgs;
    type Output = String;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        runtime_tool_definition(Self::NAME)
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        let arguments = runtime_tool_arguments(Self::NAME, args)?;
        Ok(self.bridge.call(Self::NAME, arguments).await)
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct RunBoardTestsArgs {
    #[serde(default, skip_serializing_if = "Option::is_none", alias = "appId")]
    pub app_id: Option<String>,
    #[serde(alias = "boardId")]
    pub board_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub filter: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none", alias = "maxTests")]
    pub max_tests: Option<u32>,
}

pub struct RunBoardTestsTool {
    pub bridge: Arc<dyn PlatformToolBridge>,
}

impl Tool for RunBoardTestsTool {
    const NAME: &'static str = "run_board_tests";

    type Error = RuntimeVerificationToolError;
    type Args = RunBoardTestsArgs;
    type Output = String;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        runtime_tool_definition(Self::NAME)
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        let arguments = runtime_tool_arguments(Self::NAME, args)?;
        Ok(self.bridge.call(Self::NAME, arguments).await)
    }
}

// ============================================================================
// Legacy selected-run Log Query Tool
// ============================================================================

pub struct QueryLogsTool {
    pub state: Arc<FlowLikeState>,
    pub run_context: Option<RunContext>,
}

impl Tool for QueryLogsTool {
    const NAME: &'static str = "query_logs";

    type Error = QueryLogsToolError;
    type Args = QueryLogsArgs;
    type Output = String;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: "query_logs".to_string(),
            description: r#"Query execution logs from a flow run. Useful for debugging errors and tracing execution.

LOG LEVELS: Debug(0), Info(1), Warn(2), Error(3), Fatal(4)

FILTER EXAMPLES:
- 'log_level >= 3' → Errors and fatal only
- 'node_id = "abc123"' → Logs from specific node
- 'message LIKE "%timeout%"' → Search in messages

RETURNS: Logs with level, message, node_id (use node_id with get_node_details)"#.to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "filter": {
                        "type": "string",
                        "description": "SQL-like filter: 'log_level >= 3', 'node_id = \"id\"', 'message LIKE \"%error%\"'"
                    },
                    "limit": {
                        "type": "integer",
                        "description": "Max logs to return (default: 50, max: 100)"
                    }
                },
                "required": []
            }),
        }
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        flowpilot_debug_log!("[QueryLogsTool] call() invoked with args: {:?}", args);

        let run_context = self.run_context.as_ref().ok_or_else(|| {
            flowpilot_debug_log!("[QueryLogsTool] ERROR: No run context available");
            QueryLogsToolError(
                "No run context available. User must select a run first.".to_string(),
            )
        })?;

        flowpilot_debug_log!(
            "[QueryLogsTool] run_context: app_id={}, run_id={}, board_id={}",
            run_context.app_id,
            run_context.run_id,
            run_context.board_id
        );

        let limit = args.limit.unwrap_or(50).min(100);
        let filter = args.filter.clone().unwrap_or_default();

        flowpilot_debug_log!("[QueryLogsTool] Using limit={}, filter='{}'", limit, filter);

        // Build LogMeta from RunContext
        #[cfg(feature = "flow-runtime")]
        let log_meta = crate::flow::execution::LogMeta {
            app_id: run_context.app_id.clone(),
            run_id: run_context.run_id.clone(),
            board_id: run_context.board_id.clone(),
            start: 0,
            end: 0,
            log_level: 0,
            version: String::new(),
            nodes: None,
            logs: None,
            node_id: String::new(),
            event_version: None,
            event_id: String::new(),
            payload: vec![],
            is_remote: false,
        };

        #[cfg(feature = "flow-runtime")]
        {
            flowpilot_debug_log!("[QueryLogsTool] Calling state.query_run()...");
            let logs = self
                .state
                .query_run(&log_meta, &filter, Some(limit), Some(0))
                .await
                .map_err(|e| {
                    flowpilot_debug_log!("[QueryLogsTool] ERROR querying logs: {}", e);
                    QueryLogsToolError(format!("Failed to query logs: {}", e))
                })?;

            flowpilot_debug_log!("[QueryLogsTool] Got {} logs", logs.len());

            if logs.is_empty() {
                let msg = if filter.is_empty() {
                    "No logs found for this run. The execution may have completed without producing any log output, or logs may have been cleared."
                } else {
                    "No logs matching your filter criteria. Try a broader search or check if the filter syntax is correct."
                };
                flowpilot_debug_log!("[QueryLogsTool] Returning empty message: {}", msg);
                return Ok(msg.to_string());
            }

            // Format logs for the AI
            let formatted_logs: Vec<serde_json::Value> = logs
                .iter()
                .map(|log| {
                    json!({
                        "level": match log.log_level {
                            crate::flow::execution::LogLevel::Debug => "Debug",
                            crate::flow::execution::LogLevel::Info => "Info",
                            crate::flow::execution::LogLevel::Warn => "Warn",
                            crate::flow::execution::LogLevel::Error => "Error",
                            crate::flow::execution::LogLevel::Fatal => "Fatal",
                        },
                        "message": log.message,
                        "node_id": log.node_id,
                    })
                })
                .collect();

            let result = serde_json::to_string_pretty(&formatted_logs).unwrap_or_default();
            flowpilot_debug_log!(
                "[QueryLogsTool] Returning {} bytes of formatted logs",
                result.len()
            );
            flowpilot_debug_log!(
                "[QueryLogsTool] First 500 chars: {}",
                &result[..result.len().min(500)]
            );
            Ok(result)
        }

        #[cfg(not(feature = "flow-runtime"))]
        {
            flowpilot_debug_log!("[QueryLogsTool] flow-runtime feature not enabled");
            Ok("Log querying is not available in this build.".to_string())
        }
    }
}

// ============================================================================
// FlowScript Tools
// ============================================================================

/// Return the live board rendered as anchored FlowScript.
///
/// This is intentionally a tool, even though the system prompt also includes the board source,
/// because long multi-step agents can lose the inline copy. Calling this immediately before
/// `write_flowscript` gives the model the exact current document from which to begin a retained
/// replacement draft.
pub struct GetCurrentFlowScriptTool {
    pub board: Arc<Board>,
}

impl Tool for GetCurrentFlowScriptTool {
    const NAME: &'static str = "get_current_flowscript";

    type Error = FlowScriptToolError;
    type Args = GetCurrentFlowScriptArgs;
    type Output = String;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: "get_current_flowscript".to_string(),
            description: r#"Return the current live board as anchored FlowScript.

The system prompt already embeds this exact render — do not call this before authoring; use it only
to re-read the board after the host applies an incremental segment. The returned document is
the source you must edit and submit in full to `write_flowscript`; preserve all `//@n:<id>` and
`//@l:<id>` anchors and every `@cache` decorator on functions you keep. `module name { ... }`
blocks are the board's namespaces; the written structure is authoritative: renaming an anchored
module block renames it, and moving an anchored function/event/module block into a different
module block moves it there — anchors keep identity, so reorganizing for readability is safe. Cache settings use
`@cache({ namespace: "...", ttlSeconds: 3600, scope: "user" })`; bare `@cache` defaults to the
`global` namespace, a 300-second lifetime, and app scope. Use `ttlSeconds: 0` for no expiry.
If existing context reports `ttl_seconds: null`, it is a permanent cache; preserve it as
explicit `ttlSeconds: 0` rather than applying the new omission default.
After a write/check diagnostic, do NOT read the unchanged live board again: continue from the
retained source and revision with patch_flowscript/check_flowscript/commit_flowscript."#
                .to_string(),
            parameters: json!({
                "type": "object",
                "properties": {}
            }),
        }
    }

    async fn call(&self, _args: Self::Args) -> Result<Self::Output, Self::Error> {
        Ok(board_to_flowscript(
            &self.board,
            &RenderOptions {
                anchors: true,
                ..Default::default()
            },
        ))
    }
}

/// Retrieve `.flow.d`-style FlowScript declarations for nodes matching a query.
///
/// This is the FlowScript counterpart to `catalog_search`/`get_node_details`: instead of
/// per-pin JSON, it returns the exact `function ns::alias(…)` signatures the agent should call when
/// writing FlowScript, including third-party package nodes injected into the catalog.
pub struct GetDeclarationsTool {
    pub provider: Arc<dyn CatalogProvider>,
}

impl Tool for GetDeclarationsTool {
    const NAME: &'static str = "get_declarations";

    type Error = FlowScriptToolError;
    type Args = GetDeclarationsArgs;
    type Output = String;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: "get_declarations".to_string(),
            description: r#"Look up FlowScript node declarations (.flow.d) by intent. The initial pass is ONE bounded, focused batch (at most 32 queries) for the highest-leverage catalog calls needed to establish the workflow's end-to-end shape — not an inventory of every utility operation.

Returns a compact ranked list of exact `function <ns>::<alias>(this: T, { pin: type, ... }): R;`
signatures per query, the `// use <ns>::*` line that lets you call the bare `<alias>({ ... })`, and
an `// impure` marker for side-effecting / control-flow nodes. A `this:` parameter names the
receiver pin: that node is also callable as a method on the value (`x.alias(...)`); the legacy
camelCase name still resolves. Exact live metadata also contributes bounded usage notes for
required and repeated pins, Struct schema fields, and companion calls/structural chains. Treat
those notes as authoritative: repeat same-name inputs in declaration order and never invent Struct
members. The result is deliberately bounded and self-contained. Never try to read a temporary/persisted-output path with filesystem tools; if
validation later names a failing node/pin or a comparison/type-conversion mismatch, use one focused
repair lookup for that diagnostic. Empty queries intentionally return guidance only, not the full
catalog.

WORKFLOW: plan the complete requested scope, then query the highest-leverage concrete catalog calls
that establish its critical path. Keep each search focused on one concrete node capability rather
than combining an entire subsystem into one query, e.g.
{"queries": ["open local database", "datafusion sql query", "for each loop", "instantiate widget",
"string format", "http fetch"]}. After ANY usable response, call `plan_board_scope` exactly once
unless the host already retained an accepted plan, then immediately call `write_flowscript` and
retain its ACTIVE SEGMENT, even when compiler repairs are expected. Do not make a second broad
declaration batch or chase `omitted_queries` / `unmatched_queries` before the first write. Defer
those searches until compiler diagnostics identify a concrete gap, then use one narrow repair lookup.

Use this BEFORE writing FlowScript so you call nodes by their exact qualified name with correctly
typed and exactly named arguments. This covers every package in the project's catalog, including
third-party ones."#
                .to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "queries": {
                        "type": "array",
                        "items": {
                            "type": "string",
                            "maxLength": MAX_DECLARATION_QUERY_BYTES
                        },
                        "minItems": 1,
                        "maxItems": MAX_DECLARATION_QUERIES,
                        "uniqueItems": true,
                        "description": "REQUIRED. One bounded initial batch of the highest-leverage concrete catalog calls needed to establish the end-to-end workflow shape; do not enumerate every utility operation. After any usable response, call plan_board_scope exactly once unless the host already accepted a plan, then write its active segment immediately and defer omitted/unmatched searches until compiler diagnostics. The result reports matched_queries, unmatched_queries, complete, and omitted_queries explicitly. Good entries: 'gmail imap fetch mail', 'smtp send email', 'open local database batch insert', 'datafusion sql register lance', 'hybrid vector search build index'."
                    },
                    "query": {
                        "type": "string",
                        "description": "Single-search fallback. Prefer one bounded `queries` batch for the highest-leverage initial calls, or one compiler-directed repair lookup after a draft exists.",
                        "maxLength": MAX_DECLARATION_QUERY_BYTES
                    }
                },
                "required": ["queries"],
                "additionalProperties": false
            }),
        }
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        Ok(run_declaration_queries(&self.provider, &args).await)
    }
}

/// Code-first FlowScript lifecycle tools. The immutable request binding is captured by the host
/// and never appears in model-authored JSON, while the retained store supplies revision CAS and
/// the existing atomic Apply/Dismiss claim boundary.
pub struct WriteFlowScriptTool {
    pub board: Arc<Board>,
    pub provider: Arc<dyn CatalogProvider>,
    pub store: Arc<FlowIrDraftStore>,
    pub acceptance_binding: FlowIrAcceptanceBinding,
}

pub struct PatchFlowScriptTool {
    pub board: Arc<Board>,
    pub provider: Arc<dyn CatalogProvider>,
    pub store: Arc<FlowIrDraftStore>,
    pub acceptance_binding: FlowIrAcceptanceBinding,
}

pub struct CheckFlowScriptTool {
    pub board: Arc<Board>,
    pub provider: Arc<dyn CatalogProvider>,
    pub store: Arc<FlowIrDraftStore>,
    pub acceptance_binding: FlowIrAcceptanceBinding,
}

pub struct CommitFlowScriptTool {
    pub board: Arc<Board>,
    pub provider: Arc<dyn CatalogProvider>,
    pub store: Arc<FlowIrDraftStore>,
    pub acceptance_binding: FlowIrAcceptanceBinding,
}

pub fn board_has_no_nodes(board: &Board) -> bool {
    board.nodes.is_empty() && board.layers.values().all(|layer| layer.nodes.is_empty())
}

pub fn flowscript_workspace_envelope(flowscript: &str, status: &str) -> String {
    serde_json::to_string(&json!({
        "source": flowscript,
        "status": status,
    }))
    .unwrap_or_default()
}

pub fn flowscript_workspace_tag(flowscript: &str, status: &str) -> String {
    stream_frame(
        "flowscript_workspace",
        &json!({
            "source": flowscript,
            "status": status,
        }),
    )
}

/// Replace double-quoted string literal contents with spaces so stub-marker scanning cannot trip
/// on legitimate user-facing text (labels, prompts, messages). Escapes are honored; comments and
/// code stay scannable.
fn strip_flowscript_string_literals(source: &str) -> String {
    let mut out = String::with_capacity(source.len());
    let mut chars = source.chars();
    while let Some(ch) = chars.next() {
        if ch != '"' {
            out.push(ch);
            continue;
        }
        out.push(' ');
        while let Some(inner) = chars.next() {
            match inner {
                '\\' => {
                    let _ = chars.next();
                }
                '"' => break,
                _ => {}
            }
        }
    }
    out
}

/// True when `marker` occurs in `haystack` with non-alphanumeric characters on both sides, so a
/// short marker like "todo" cannot match inside identifiers such as `todoList`.
fn contains_stub_marker(haystack: &str, marker: &str) -> bool {
    let mut search_from = 0;
    while let Some(offset) = haystack[search_from..].find(marker) {
        let start = search_from + offset;
        let end = start + marker.len();
        let bounded_left = haystack[..start]
            .chars()
            .next_back()
            .is_none_or(|ch| !ch.is_alphanumeric());
        let bounded_right = haystack[end..]
            .chars()
            .next()
            .is_none_or(|ch| !ch.is_alphanumeric());
        if bounded_left && bounded_right {
            return true;
        }
        search_from = end;
    }
    false
}

fn edit_flowscript_actionability_feedback(
    flowscript: &str,
    board_is_empty: bool,
    diagnostics: &[String],
) -> Option<String> {
    let lower = strip_flowscript_string_literals(flowscript).to_lowercase();
    let stub_markers = [
        "implementation plan",
        "implementation notes",
        "implementation should be wired",
        "function stubs",
        "fetcher stub",
        "enricher stub",
        "todo",
        "replace with",
        "when implemented",
        "wire with",
        "wire using",
        "catalog nodes:",
        "flowscript contains stubs",
        "automated nodes added",
        "clear wiring plan",
    ];

    if stub_markers
        .iter()
        .any(|marker| contains_stub_marker(&lower, marker))
    {
        return Some(
            "This edit looks like a plan/stub, not actionable FlowScript. `edit_flowscript` only creates board changes from real catalog calls. Do not submit TODOs, stub comments, lists of node names, or \"replace with\" instructions; call `get_declarations` for the missing signatures and submit concrete calls inside a function/event block."
                .to_string(),
        );
    }

    let missing_function_helpers = flowscript_missing_function_helpers(flowscript, diagnostics);
    if !missing_function_helpers.is_empty() {
        return Some(format!(
            "Local helper declaration(s) {} are missing the required `function` keyword. Write `function helperName(...) {{ ... }}` and keep each declaration in the same full document as its calls. These are local Function layers, not catalog nodes, so another declaration search will not fix them.",
            missing_function_helpers
                .iter()
                .map(|name| format!("`{name}`"))
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }

    if diagnostics.iter().any(|diagnostic| {
        diagnostic.contains("return value")
            && diagnostic.contains("no matching function return pin")
    }) {
        return Some(
            "A helper returns a value but declares no matching Function output pin. Add a named return signature, for example `function classify(body: string): (isSupport: bool) { ...; return result.value }`."
                .to_string(),
        );
    }

    if diagnostics
        .iter()
        .any(|diagnostic| diagnostic.contains("expected `Colon`, found `Assign`"))
    {
        return Some(
            "The submitted FlowScript used `=` where FlowScript expected an object/call-argument field separator. In FlowScript call arguments and object literals use colon syntax, e.g. `{ host: \"imap.gmail.com\", port: 993 }`, not `{ host = \"imap.gmail.com\" }`."
                .to_string(),
        );
    }

    if diagnostics
        .iter()
        .any(|diagnostic| diagnostic.contains("`const` binding requires a call expression"))
    {
        return Some(
            "Inside a function/event block, `const name = ...` can only bind the output of a node call. Do not bind literals, object literals, arrays, field access, or arithmetic with `const`; use local alias syntax like `let rows = []`, pass literals directly into a node call, or bind a real utility/catalog call."
                .to_string(),
        );
    }

    if diagnostics
        .iter()
        .any(|diagnostic| diagnostic.contains("labelled branch requires a call condition"))
    {
        return Some(
            "The submitted FlowScript used labelled branch syntax (`if (...) { // label ... }`) with a non-call condition. In FlowScript, labels after branch braces are reserved for call-based control nodes, so the condition must be a catalog/control-node call. For ordinary boolean checks, remove the trailing branch labels/comments and use plain `if (condition) { ... } else { ... }`, or use exact control-node declarations from `get_declarations`."
                .to_string(),
        );
    }

    if diagnostics
        .iter()
        .any(|diagnostic| diagnostic.contains("FlowScript parse error"))
    {
        return Some(
            "The submitted FlowScript did not parse. A common cause is putting node calls at the top level: top-level `const name: type = ...` declarations can only hold literal defaults and do not create nodes. Put catalog calls inside a function/event block, for example `function run() { const db = openLocalDb({ name: \"email_vectors\" }) }`, using exact signatures from `get_declarations`."
                .to_string(),
        );
    }

    if board_is_empty && !flowscript_has_executable_node_call(flowscript) {
        return Some(
            "The board is empty and this FlowScript contains no executable catalog calls. An empty eventsSimple()/eventsGeneric()/eventsChat() entry is only a future Event registration target, not a workflow implementation. Keep the complete draft and add real logic before submitting again."
                .to_string(),
        );
    }

    None
}

/// Detect top-level blocks that look like local helper declarations but omit the required
/// `function` keyword. Reconcile otherwise reports every call site as an unknown catalog node,
/// which sends agents into pointless declaration searches instead of fixing the local syntax.
pub fn flowscript_missing_function_helpers(
    flowscript: &str,
    diagnostics: &[String],
) -> Vec<String> {
    let mut helpers = Vec::new();
    for line in flowscript.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty()
            || trimmed.starts_with("function ")
            || trimmed.starts_with("if ")
            || trimmed.starts_with("for ")
            || trimmed.contains("//@n:")
            || !trimmed.ends_with('{')
        {
            continue;
        }
        let Some(open_paren) = trimmed.find('(') else {
            continue;
        };
        let name = trimmed[..open_paren].trim();
        if name.is_empty()
            || !name.chars().enumerate().all(|(index, ch)| {
                ch == '_' || ch.is_ascii_alphanumeric() && (index > 0 || !ch.is_ascii_digit())
            })
        {
            continue;
        }
        let unknown_call = format!("FlowScript call `{name}` does not match a catalog declaration");
        if diagnostics
            .iter()
            .any(|diagnostic| diagnostic.contains(&unknown_call))
            && !helpers.iter().any(|existing| existing == name)
        {
            helpers.push(name.to_string());
        }
    }
    helpers
}

pub fn flowscript_has_executable_node_call(flowscript: &str) -> bool {
    flowscript.lines().any(|line| {
        let trimmed = line.trim();
        if trimmed.is_empty()
            || trimmed.starts_with("//")
            || trimmed.starts_with('@')
            || trimmed.starts_with("if ")
            || trimmed.starts_with("for ")
            || trimmed.starts_with("return ")
            || trimmed.contains(") {")
        {
            return false;
        }

        if let Some(rest) = trimmed.strip_prefix("const ") {
            return rest
                .split_once('=')
                .is_some_and(|(_, rhs)| starts_with_call_expr(rhs));
        }

        starts_with_call_expr(trimmed)
    })
}

fn starts_with_call_expr(source: &str) -> bool {
    let source = source.trim_start();
    let Some(paren_idx) = source.find('(') else {
        return false;
    };
    let name = source[..paren_idx].trim();
    !name.is_empty()
        && name
            .chars()
            .next()
            .is_some_and(|ch| ch.is_ascii_alphabetic() || ch == '_')
        && name
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
}

/// Structural footprint of one FlowScript repair candidate.
///
/// The score intentionally ignores comments and raw byte length. A verbose plan must not outrank
/// executable workflow structure, while helper functions, variables, Event roots and catalog call
/// sites are useful evidence that a candidate still represents the user's full application.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FlowScriptCandidateProfile {
    pub call_sites: usize,
    pub meaningful_statements: usize,
    pub event_entries: usize,
    pub helper_functions: HashSet<String>,
    pub non_empty_helper_functions: HashSet<String>,
    pub helper_non_helper_call_sites: usize,
    pub helper_domain_call_sites: usize,
    pub event_names: HashSet<String>,
    pub events_calling_helpers: usize,
    pub top_level_variables: HashSet<String>,
    pub interfaces: HashSet<String>,
    pub call_names: HashSet<String>,
}

impl FlowScriptCandidateProfile {
    pub fn completeness_score(&self) -> usize {
        self.call_sites
            .saturating_mul(8)
            .saturating_add(self.meaningful_statements.saturating_mul(2))
            .saturating_add(self.helper_functions.len().saturating_mul(12))
            .saturating_add(self.event_entries.saturating_mul(6))
            .saturating_add(self.top_level_variables.len().saturating_mul(4))
            .saturating_add(self.interfaces.len().saturating_mul(4))
            .saturating_add(self.call_names.len().saturating_mul(2))
    }

    fn stable_scope_symbols(&self) -> HashSet<String> {
        let mut symbols = HashSet::new();
        symbols.extend(
            self.helper_functions
                .iter()
                .map(|name| format!("function:{name}")),
        );
        symbols.extend(self.event_names.iter().map(|name| format!("event:{name}")));
        symbols.extend(
            self.top_level_variables
                .iter()
                .map(|name| format!("variable:{name}")),
        );
        symbols.extend(
            self.interfaces
                .iter()
                .map(|name| format!("interface:{name}")),
        );
        symbols
    }
}

/// Evidence returned when a nominally valid candidate looks like an unrelated, much smaller
/// replacement for the best failed repair draft.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FlowScriptCandidateRegression {
    pub previous_call_sites: usize,
    pub candidate_call_sites: usize,
    pub previous_statements: usize,
    pub candidate_statements: usize,
    pub previous_scope_symbols: usize,
    pub retained_scope_symbols: usize,
}

#[derive(Debug, Clone)]
struct TrackedFlowScriptCandidate {
    source: String,
    profile: FlowScriptCandidateProfile,
    parses: bool,
    diagnostic_count: Option<usize>,
}

/// Per-chat repair memory used to keep the fullest failed candidate available until a real repair
/// succeeds. This state belongs to the agent loop, never to the board or a long-lived Copilot.
#[derive(Debug, Clone, Default)]
pub struct FlowScriptRepairTracker {
    best_failed: Option<TrackedFlowScriptCandidate>,
}

impl FlowScriptRepairTracker {
    /// Record a failed candidate. Returns `true` when it became the retained best draft, allowing
    /// callers to keep status/diagnostics aligned with that exact source.
    pub fn record_failed(&mut self, source: &str) -> bool {
        self.record_failed_with_diagnostics(source, None)
    }

    /// Record a failed candidate together with the number of diagnostics returned for that exact
    /// submission. Within the same application scope, a syntactically valid draft and then a
    /// draft with fewer validation diagnostics are stronger repair checkpoints than a slightly
    /// larger but less-correct predecessor. A dramatic scope collapse is never accepted merely
    /// because it happens to report fewer errors.
    pub fn record_failed_with_diagnostics(
        &mut self,
        source: &str,
        diagnostic_count: Option<usize>,
    ) -> bool {
        if source.trim().is_empty() {
            return false;
        }

        let profile = profile_flowscript_candidate(source);
        let parses = flow_like_ast::parse(source).is_ok();
        let replace = self.best_failed.as_ref().is_none_or(|current| {
            // Scope is a hard boundary around quality ranking. This prevents a one-node smoke
            // test with zero diagnostics from displacing a complete, repairable application.
            if candidate_shrink_evidence(&current.profile, &profile).is_some() {
                return false;
            }
            // Conversely, recover if an early tiny failure was recorded before the full draft.
            if candidate_shrink_evidence(&profile, &current.profile).is_some() {
                return true;
            }

            if parses != current.parses {
                return parses;
            }
            if let (Some(candidate_count), Some(current_count)) =
                (diagnostic_count, current.diagnostic_count)
                && candidate_count != current_count
            {
                return candidate_count < current_count;
            }

            profile.completeness_score() >= current.profile.completeness_score()
        });
        if replace {
            self.best_failed = Some(TrackedFlowScriptCandidate {
                source: source.to_string(),
                profile,
                parses,
                diagnostic_count,
            });
            return true;
        }
        false
    }

    pub fn best_failed_source(&self) -> Option<&str> {
        self.best_failed
            .as_ref()
            .map(|candidate| candidate.source.as_str())
    }

    pub fn queued_candidate_regression(
        &self,
        candidate: &str,
    ) -> Option<FlowScriptCandidateRegression> {
        let previous = self.best_failed.as_ref()?;
        detect_flowscript_candidate_regression(
            &previous.profile,
            &profile_flowscript_candidate(candidate),
        )
    }

    pub fn queued_candidate_modular_fallback(
        &self,
        candidate: &str,
    ) -> Option<FlowScriptCandidateRegression> {
        let previous = self.best_failed.as_ref()?;
        let candidate = profile_flowscript_candidate(candidate);
        is_deliberately_modular_partial(&candidate)
            .then(|| candidate_shrink_evidence(&previous.profile, &candidate))
            .flatten()
    }
}

/// Parse a candidate and measure its executable structure. Semantically invalid node names still
/// parse and therefore remain visible to the repair tracker. A small lexical fallback handles a
/// syntax-error draft well enough to retain its broad shape without pretending it is executable.
pub fn profile_flowscript_candidate(source: &str) -> FlowScriptCandidateProfile {
    match flow_like_ast::parse(source) {
        Ok(ast) => {
            let mut profile = FlowScriptCandidateProfile::default();
            profile.interfaces.extend(
                ast.interfaces
                    .iter()
                    .map(|interface| normalize_candidate_symbol(&interface.name)),
            );
            profile.top_level_variables.extend(
                ast.variables
                    .iter()
                    .map(|variable| normalize_candidate_symbol(&variable.name)),
            );
            profile.helper_functions.extend(
                ast.functions
                    .iter()
                    .map(|function| normalize_candidate_symbol(&function.name)),
            );
            for function in &ast.functions {
                let function_name = normalize_candidate_symbol(&function.name);
                let calls_before = profile.call_sites;
                profile_flowscript_block(&function.body, &mut profile);
                if profile.call_sites > calls_before {
                    profile.non_empty_helper_functions.insert(function_name);
                }
                let mut body_calls = Vec::new();
                collect_flowscript_block_call_names(&function.body, &mut body_calls);
                for call_name in body_calls {
                    if profile.helper_functions.contains(&call_name) {
                        continue;
                    }
                    profile.helper_non_helper_call_sites =
                        profile.helper_non_helper_call_sites.saturating_add(1);
                    if !is_trivial_smoke_call(&call_name) {
                        profile.helper_domain_call_sites =
                            profile.helper_domain_call_sites.saturating_add(1);
                    }
                }
            }
            for event in &ast.events {
                profile.event_entries = profile.event_entries.saturating_add(1);
                profile.event_names.insert(format!(
                    "{}:{}",
                    normalize_candidate_symbol(&event.name),
                    normalize_candidate_symbol(&event.node_type)
                ));
                if flowscript_block_calls_any(&event.body, &profile.helper_functions) {
                    profile.events_calling_helpers =
                        profile.events_calling_helpers.saturating_add(1);
                }
                profile_flowscript_block(&event.body, &mut profile);
            }
            // A detached chain is unreachable, not absent. Its nodes are still board scope the
            // shrink detector has to weigh, or an edit that leaves work detached reads as a
            // collapse to an empty draft.
            for block in &ast.detached {
                profile_flowscript_block(block, &mut profile);
            }
            profile
        }
        Err(_) => profile_flowscript_candidate_lexically(source),
    }
}

fn normalize_candidate_symbol(symbol: &str) -> String {
    symbol.trim().to_ascii_lowercase()
}

fn profile_flowscript_block(block: &FlowScriptBlock, profile: &mut FlowScriptCandidateProfile) {
    for statement in &block.stmts {
        if !matches!(statement, FlowScriptStmt::Comment(_)) {
            profile.meaningful_statements = profile.meaningful_statements.saturating_add(1);
        }
        match statement {
            FlowScriptStmt::Let { call, .. }
            | FlowScriptStmt::Destructure { call, .. }
            | FlowScriptStmt::Call { call, .. } => profile_flowscript_call(call, profile),
            FlowScriptStmt::Branch {
                call,
                condition,
                arms,
                ..
            } => {
                profile_flowscript_call(call, profile);
                if let Some(condition) = condition {
                    profile_flowscript_expr(condition, profile);
                }
                for arm in arms {
                    profile_flowscript_block(&arm.body, profile);
                }
            }
            FlowScriptStmt::Loop {
                call,
                iterable,
                body,
                ..
            } => {
                profile_flowscript_call(call, profile);
                if let Some(iterable) = iterable {
                    profile_flowscript_expr(iterable, profile);
                }
                profile_flowscript_block(body, profile);
            }
            FlowScriptStmt::Assign { value, .. }
            | FlowScriptStmt::FieldAssign { value, .. }
            | FlowScriptStmt::LocalAlias { value, .. } => profile_flowscript_expr(value, profile),
            FlowScriptStmt::Return { values, .. } => {
                for value in values {
                    profile_flowscript_expr(value, profile);
                }
            }
            FlowScriptStmt::Handler(event) => {
                profile.event_entries = profile.event_entries.saturating_add(1);
                profile.event_names.insert(format!(
                    "{}:{}",
                    normalize_candidate_symbol(&event.name),
                    normalize_candidate_symbol(&event.node_type)
                ));
                if flowscript_block_calls_any(&event.body, &profile.helper_functions) {
                    profile.events_calling_helpers =
                        profile.events_calling_helpers.saturating_add(1);
                }
                profile_flowscript_block(&event.body, profile);
            }
            FlowScriptStmt::Local(_) | FlowScriptStmt::Comment(_) => {}
        }
    }
}

fn flowscript_block_calls_any(block: &FlowScriptBlock, names: &HashSet<String>) -> bool {
    let mut calls = Vec::new();
    collect_flowscript_block_call_names(block, &mut calls);
    calls.iter().any(|call| names.contains(call))
}

fn collect_flowscript_block_call_names(block: &FlowScriptBlock, calls: &mut Vec<String>) {
    for statement in &block.stmts {
        match statement {
            FlowScriptStmt::Let { call, .. }
            | FlowScriptStmt::Destructure { call, .. }
            | FlowScriptStmt::Call { call, .. } => collect_flowscript_call_names(call, calls),
            FlowScriptStmt::Branch {
                call,
                condition,
                arms,
                ..
            } => {
                collect_flowscript_call_names(call, calls);
                if let Some(condition) = condition {
                    collect_flowscript_expr_call_names(condition, calls);
                }
                for arm in arms {
                    collect_flowscript_block_call_names(&arm.body, calls);
                }
            }
            FlowScriptStmt::Loop {
                call,
                iterable,
                body,
                ..
            } => {
                collect_flowscript_call_names(call, calls);
                if let Some(iterable) = iterable {
                    collect_flowscript_expr_call_names(iterable, calls);
                }
                collect_flowscript_block_call_names(body, calls);
            }
            FlowScriptStmt::Assign { value, .. }
            | FlowScriptStmt::FieldAssign { value, .. }
            | FlowScriptStmt::LocalAlias { value, .. } => {
                collect_flowscript_expr_call_names(value, calls)
            }
            FlowScriptStmt::Return { values, .. } => {
                for value in values {
                    collect_flowscript_expr_call_names(value, calls);
                }
            }
            FlowScriptStmt::Handler(event) => {
                collect_flowscript_block_call_names(&event.body, calls)
            }
            FlowScriptStmt::Local(_) | FlowScriptStmt::Comment(_) => {}
        }
    }
}

fn collect_flowscript_call_names(call: &FlowScriptCall, calls: &mut Vec<String>) {
    calls.push(normalize_candidate_symbol(
        if call.display.trim().is_empty() {
            &call.node_type
        } else {
            &call.display
        },
    ));
    for operand in flowscript_call_operands(call) {
        collect_flowscript_expr_call_names(operand, calls);
    }
}

/// Every expression a call evaluates: the method receiver, positional values and named values.
fn flowscript_call_operands(call: &FlowScriptCall) -> impl Iterator<Item = &FlowScriptExpr> {
    call.receiver
        .iter()
        .map(|receiver| receiver.as_ref())
        .chain(call.positional.iter())
        .chain(call.args.iter().map(|argument| &argument.value))
}

fn collect_flowscript_expr_call_names(expression: &FlowScriptExpr, calls: &mut Vec<String>) {
    match expression {
        FlowScriptExpr::Call(call) => collect_flowscript_call_names(call, calls),
        FlowScriptExpr::Field { base, .. } | FlowScriptExpr::Member { base, .. } => {
            collect_flowscript_expr_call_names(base, calls)
        }
        FlowScriptExpr::Object(fields) => {
            for field in fields {
                collect_flowscript_expr_call_names(&field.value, calls);
            }
        }
        FlowScriptExpr::Array(values) => {
            for value in values {
                collect_flowscript_expr_call_names(value, calls);
            }
        }
        FlowScriptExpr::Index { base, index } => {
            collect_flowscript_expr_call_names(base, calls);
            collect_flowscript_expr_call_names(index, calls);
        }
        FlowScriptExpr::Ternary {
            cond,
            then,
            otherwise,
        } => {
            collect_flowscript_expr_call_names(cond, calls);
            collect_flowscript_expr_call_names(then, calls);
            collect_flowscript_expr_call_names(otherwise, calls);
        }
        FlowScriptExpr::Binary { lhs, rhs, .. } => {
            collect_flowscript_expr_call_names(lhs, calls);
            collect_flowscript_expr_call_names(rhs, calls);
        }
        FlowScriptExpr::Template { parts } => {
            for part in parts {
                if let flow_like_ast::TemplatePart::Expr(expr) = part {
                    collect_flowscript_expr_call_names(expr, calls);
                }
            }
        }
        FlowScriptExpr::Ref(_) | FlowScriptExpr::Literal(_) => {}
    }
}

fn is_trivial_smoke_call(call_name: &str) -> bool {
    matches!(
        call_name,
        "log"
            | "loginfo"
            | "logdebug"
            | "logwarn"
            | "logerror"
            | "stringformat"
            | "structmake"
            | "structget"
            | "structset"
            | "arraypush"
            | "arrayget"
            | "arraylength"
            | "variableget"
    )
}

fn profile_flowscript_call(call: &FlowScriptCall, profile: &mut FlowScriptCandidateProfile) {
    profile.call_sites = profile.call_sites.saturating_add(1);
    profile.call_names.insert(normalize_candidate_symbol(
        if call.display.trim().is_empty() {
            &call.node_type
        } else {
            &call.display
        },
    ));
    for operand in flowscript_call_operands(call) {
        profile_flowscript_expr(operand, profile);
    }
}

fn profile_flowscript_expr(expression: &FlowScriptExpr, profile: &mut FlowScriptCandidateProfile) {
    match expression {
        FlowScriptExpr::Call(call) => profile_flowscript_call(call, profile),
        FlowScriptExpr::Field { base, .. } | FlowScriptExpr::Member { base, .. } => {
            profile_flowscript_expr(base, profile)
        }
        FlowScriptExpr::Object(fields) => {
            for field in fields {
                profile_flowscript_expr(&field.value, profile);
            }
        }
        FlowScriptExpr::Array(values) => {
            for value in values {
                profile_flowscript_expr(value, profile);
            }
        }
        FlowScriptExpr::Index { base, index } => {
            profile_flowscript_expr(base, profile);
            profile_flowscript_expr(index, profile);
        }
        FlowScriptExpr::Ternary {
            cond,
            then,
            otherwise,
        } => {
            profile_flowscript_expr(cond, profile);
            profile_flowscript_expr(then, profile);
            profile_flowscript_expr(otherwise, profile);
        }
        FlowScriptExpr::Binary { lhs, rhs, .. } => {
            profile_flowscript_expr(lhs, profile);
            profile_flowscript_expr(rhs, profile);
        }
        FlowScriptExpr::Template { parts } => {
            for part in parts {
                if let flow_like_ast::TemplatePart::Expr(expr) = part {
                    profile_flowscript_expr(expr, profile);
                }
            }
        }
        FlowScriptExpr::Ref(_) | FlowScriptExpr::Literal(_) => {}
    }
}

fn profile_flowscript_candidate_lexically(source: &str) -> FlowScriptCandidateProfile {
    let mut profile = FlowScriptCandidateProfile::default();
    for line in source.lines() {
        let trimmed = line.trim();
        // Every other block header names a board object this profile records separately.
        // `detached {` names nothing — it has no node and therefore no anchor — so it is
        // punctuation like the brace lines above it, and only the chain inside it counts.
        if trimmed.is_empty()
            || trimmed == "{"
            || trimmed == "}"
            || trimmed.starts_with("//")
            || trimmed.starts_with('@')
            || trimmed
                .strip_prefix("detached")
                .is_some_and(|rest| rest.trim() == "{")
        {
            continue;
        }
        profile.meaningful_statements = profile.meaningful_statements.saturating_add(1);

        if let Some(rest) = trimmed.strip_prefix("function ")
            && let Some(name) = rest.split(['(', ' ']).next()
            && !name.is_empty()
        {
            profile
                .helper_functions
                .insert(normalize_candidate_symbol(name));
        } else if !line.chars().next().is_some_and(char::is_whitespace)
            && (trimmed.starts_with("const ") || trimmed.starts_with("let "))
            && let Some(name) = trimmed
                .split_whitespace()
                .nth(1)
                .and_then(|value| value.split([':', '=']).next())
        {
            profile
                .top_level_variables
                .insert(normalize_candidate_symbol(name));
        }

        for name in lexical_call_names(trimmed) {
            if matches!(name.as_str(), "if" | "for" | "while") {
                continue;
            }
            if name.starts_with("events") {
                profile.event_entries = profile.event_entries.saturating_add(1);
                profile.event_names.insert(name);
            } else if !trimmed.starts_with("function ") {
                profile.call_sites = profile.call_sites.saturating_add(1);
                profile.call_names.insert(name);
            }
        }
    }
    profile
}

fn lexical_call_names(source: &str) -> Vec<String> {
    let chars = source.char_indices().collect::<Vec<_>>();
    let mut names = Vec::new();
    let mut index = 0usize;
    while index < chars.len() {
        let (_, ch) = chars[index];
        if !(ch.is_ascii_alphabetic() || ch == '_') {
            index += 1;
            continue;
        }
        let start = chars[index].0;
        index += 1;
        while index < chars.len()
            && (chars[index].1.is_ascii_alphanumeric() || chars[index].1 == '_')
        {
            index += 1;
        }
        let end = chars
            .get(index)
            .map(|(offset, _)| *offset)
            .unwrap_or(source.len());
        let mut lookahead = index;
        while lookahead < chars.len() && chars[lookahead].1.is_whitespace() {
            lookahead += 1;
        }
        if lookahead < chars.len() && chars[lookahead].1 == '(' {
            names.push(normalize_candidate_symbol(&source[start..end]));
        }
    }
    names
}

pub fn detect_flowscript_candidate_regression(
    previous: &FlowScriptCandidateProfile,
    candidate: &FlowScriptCandidateProfile,
) -> Option<FlowScriptCandidateRegression> {
    let regression = candidate_shrink_evidence(previous, candidate)?;
    (!is_deliberately_modular_partial(candidate)).then_some(regression)
}

fn is_deliberately_modular_partial(candidate: &FlowScriptCandidateProfile) -> bool {
    !candidate.non_empty_helper_functions.is_empty()
        && candidate.events_calling_helpers > 0
        && (candidate.helper_domain_call_sites > 0 || candidate.helper_non_helper_call_sites >= 2)
}

fn candidate_shrink_evidence(
    previous: &FlowScriptCandidateProfile,
    candidate: &FlowScriptCandidateProfile,
) -> Option<FlowScriptCandidateRegression> {
    // Only guard a clearly substantial prior draft and a dramatic collapse. Small workflows and
    // ordinary code cleanup must remain free to become genuinely concise.
    let previous_is_substantial = previous.call_sites >= 5
        && (previous.meaningful_statements >= 6
            || previous.helper_functions.len() + previous.event_entries >= 3);
    let severe_call_shrink = candidate.call_sites.saturating_mul(3) < previous.call_sites;
    let severe_statement_shrink =
        candidate.meaningful_statements.saturating_mul(2) < previous.meaningful_statements;
    if !(previous_is_substantial && severe_call_shrink && severe_statement_shrink) {
        return None;
    }

    let previous_symbols = previous.stable_scope_symbols();
    let candidate_symbols = candidate.stable_scope_symbols();
    let retained_scope_symbols = previous_symbols.intersection(&candidate_symbols).count();
    let identity_was_lost = if previous_symbols.len() >= 2 {
        retained_scope_symbols.saturating_mul(2) < previous_symbols.len()
    } else {
        let retained_calls = previous
            .call_names
            .intersection(&candidate.call_names)
            .count();
        previous.call_names.len() >= 5
            && candidate.call_names.len() <= 2
            && retained_calls.saturating_mul(3) < previous.call_names.len()
    };
    let multiple_event_scope_was_lost = previous.event_entries >= 2
        && candidate.event_entries.saturating_mul(2) < previous.event_entries;
    if !(identity_was_lost || multiple_event_scope_was_lost) {
        return None;
    }

    Some(FlowScriptCandidateRegression {
        previous_call_sites: previous.call_sites,
        candidate_call_sites: candidate.call_sites,
        previous_statements: previous.meaningful_statements,
        candidate_statements: candidate.meaningful_statements,
        previous_scope_symbols: previous_symbols.len(),
        retained_scope_symbols,
    })
}

pub fn render_flowscript_modular_partial_result(
    queued_result: &str,
    regression: &FlowScriptCandidateRegression,
) -> String {
    format!(
        "{queued_result}\n\n⚠ status: partial_working_slice. This valid modular candidate queues an independently runnable subset, not the complete requested application. It contains {} of the previous {} call sites and retains {}/{} application-scope identities. Report that remaining scope is incomplete; do not claim the whole app was built. The fuller failed draft remains retained for a later repair pass.",
        regression.candidate_call_sites,
        regression.previous_call_sites,
        regression.retained_scope_symbols,
        regression.previous_scope_symbols,
    )
}

pub fn render_flowscript_candidate_regression(
    retained_source: &str,
    regression: &FlowScriptCandidateRegression,
) -> String {
    format!(
        "{}\nFlowScript repair blocked before queueing (code: candidate_regression). Nothing was queued. The submitted candidate collapsed from {} to {} call sites and from {} to {} meaningful statements while retaining only {}/{} helper/Event/variable/interface identities. Restore the complete retained draft shown in the workspace, fix its diagnostics in place, and resubmit it. A small test-only Event must not replace the requested application. If only a working partial is currently possible, keep real logic in one or more non-empty named `function` helpers and add a separate thin Event entry that calls those helpers; that modular partial is accepted and can be tested or extended independently.",
        flowscript_workspace_tag(retained_source, "validation_errors"),
        regression.previous_call_sites,
        regression.candidate_call_sites,
        regression.previous_statements,
        regression.candidate_statements,
        regression.retained_scope_symbols,
        regression.previous_scope_symbols,
    )
}

pub fn render_edit_flowscript_result(
    flowscript: &str,
    result: &ReconcileResult,
    board_is_empty: bool,
    allow_deletions: bool,
) -> String {
    render_edit_flowscript_result_inner(flowscript, result, board_is_empty, allow_deletions, true)
}

/// Render a commit the draft store already validated at an exact revision. The pre-commit
/// actionability scan must not run here: a false positive (e.g. a stub marker inside a string
/// literal) would report "Nothing was queued." for a commit whose pending claim the store already
/// holds — a wedged state no agent effort can escape.
pub fn render_committed_flowscript_result(
    flowscript: &str,
    result: &ReconcileResult,
    board_is_empty: bool,
    allow_deletions: bool,
) -> String {
    render_edit_flowscript_result_inner(flowscript, result, board_is_empty, allow_deletions, false)
}

fn render_edit_flowscript_result_inner(
    flowscript: &str,
    result: &ReconcileResult,
    board_is_empty: bool,
    allow_deletions: bool,
    enforce_actionability: bool,
) -> String {
    let mut rendered = render_edit_flowscript_result_legacy(
        flowscript,
        result,
        board_is_empty,
        allow_deletions,
        enforce_actionability,
    );
    if !result.corrections.is_empty() {
        let payload = serde_json::to_string(&result.corrections).unwrap_or_else(|_| "[]".into());
        rendered.push_str(&format!(
            "\n<flowscript_corrections>{payload}</flowscript_corrections>\nApply these exact canonical rewrites to the retained FlowScript source on the next patch."
        ));
    }
    if result.diagnostics.is_empty() {
        return rendered;
    }
    let structured = result.structured_diagnostics_for_source(flowscript);
    let payload = serde_json::to_string(&structured).unwrap_or_else(|_| "[]".to_string());
    format!("{rendered}\n<structured_diagnostics>{payload}</structured_diagnostics>")
}

fn render_edit_flowscript_result_legacy(
    flowscript: &str,
    result: &ReconcileResult,
    board_is_empty: bool,
    allow_deletions: bool,
    enforce_actionability: bool,
) -> String {
    // Run this before inspecting commands: an empty Event entry itself reconciles to AddNode, but
    // accepting that shell on a new board lets a failed rich draft collapse into a one-node
    // "success" and stops the repair loop.
    if enforce_actionability
        && let Some(feedback) =
            edit_flowscript_actionability_feedback(flowscript, board_is_empty, &result.diagnostics)
    {
        let mut msg = format!("{feedback}\n\nNothing was queued.");
        if !result.diagnostics.is_empty() {
            msg.push_str("\nDiagnostics:\n");
            for diagnostic in &result.diagnostics {
                msg.push_str("- ");
                msg.push_str(diagnostic);
                msg.push('\n');
            }
        }
        return format!(
            "{}\n{}",
            flowscript_workspace_tag(flowscript, "validation_errors"),
            msg
        );
    }

    let blocking_diagnostics: Vec<&String> = result
        .diagnostics
        .iter()
        .filter(|diagnostic| is_blocking_flowscript_diagnostic(diagnostic))
        .collect();

    if result.commands.is_empty() {
        let status = if !result.diagnostics.is_empty() {
            "validation_errors"
        } else {
            "no_changes"
        };

        let mut msg = "No board changes were derived from the FlowScript.".to_string();
        if !result.diagnostics.is_empty() {
            msg.push_str("\nDiagnostics:\n");
            for d in &result.diagnostics {
                msg.push_str("- ");
                msg.push_str(d);
                msg.push('\n');
            }
        }
        return format!("{}\n{}", flowscript_workspace_tag(flowscript, status), msg);
    }

    if !blocking_diagnostics.is_empty() {
        let mut msg = String::from(
            "FlowScript validation failed before queueing board changes. The script produced partial commands, but at least one construct cannot be translated safely yet.",
        );
        msg.push_str("\nDiagnostics:\n");
        for d in blocking_diagnostics {
            msg.push_str("- ");
            msg.push_str(d);
            msg.push('\n');
        }
        msg.push_str(
            "\nRewrite new control flow as concrete catalog/control-node calls, or use straight-line SSA-style node calls without mutable branch/loop side effects.",
        );
        return format!(
            "{}\n{}",
            flowscript_workspace_tag(flowscript, "validation_errors"),
            msg
        );
    }

    if !allow_deletions {
        let destructive = destructive_flowscript_command_summaries(&result.commands);
        if !destructive.is_empty() {
            let mut msg = blocked_destructive_flowscript_message(&destructive);
            if !result.diagnostics.is_empty() {
                msg.push_str("\nDiagnostics:\n");
                for d in &result.diagnostics {
                    msg.push_str("- ");
                    msg.push_str(d);
                    msg.push('\n');
                }
            }
            return format!(
                "{}\n{}",
                flowscript_workspace_tag(flowscript, "validation_errors"),
                msg
            );
        }
    }

    let commands_json = serde_json::to_string(&result.commands).unwrap_or_default();
    let mut lines = vec![format!(
        "✓ Reconciled {} change(s) from FlowScript:",
        result.commands.len()
    )];
    for cmd in &result.commands {
        match cmd {
            BoardCommand::UpdateNodePin {
                node_id, pin_id, ..
            } => lines.push(format!("  - UpdatePin: {}.{}", node_id, pin_id)),
            BoardCommand::RemoveNode { node_id, .. } => {
                lines.push(format!("  - RemoveNode: {}", node_id))
            }
            BoardCommand::CreateVariable { name, .. } => {
                lines.push(format!("  - CreateVariable: {}", name))
            }
            BoardCommand::UpdateVariable { variable_id, .. } => {
                lines.push(format!("  - UpdateVariable: {}", variable_id))
            }
            BoardCommand::RemoveVariable { variable_id, .. } => {
                lines.push(format!("  - DeleteVariable: {}", variable_id))
            }
            _ => lines.push("  - (change)".to_string()),
        }
    }
    for d in &result.diagnostics {
        lines.push(format!("  - Note: {}", d));
    }
    lines.push(
        "\n⚠️ These changes are now queued. Do NOT submit the same FlowScript again.".to_string(),
    );

    format!(
        "{}\n<commands>{}</commands>\n\n{}",
        flowscript_workspace_tag(flowscript, "queued"),
        commands_json,
        lines.join("\n")
    )
}

pub fn is_blocking_flowscript_diagnostic(diagnostic: &str) -> bool {
    // Reconcile diagnostics always mean some requested behavior, input, connection, boundary or
    // identity could not be represented exactly. Applying the remaining commands creates a graph
    // that looks successful but is only a partial program, so agent-authored FlowScript is atomic.
    !diagnostic.trim().is_empty()
}

pub struct ExtendTimeBudgetTool;

impl Tool for ExtendTimeBudgetTool {
    const NAME: &'static str = "extend_time_budget";

    type Error = FlowScriptToolError;
    type Args = ExtendTimeBudgetArgs;
    type Output = String;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: Self::NAME.to_string(),
            description: "Ask for more wall clock on a long build. Call this when the run is genuinely still advancing and the remaining segments need more time than the current budget allows. The host decides from its own record of what actually moved — segments committed, revisions that checked valid, the retained document growing, new compiler states reached — not from what you write here, so an accurate account costs nothing and an optimistic one buys nothing. A run that is repairing the same diagnostics or rewriting the same document is refused and should stop and report instead. You do not have to call this to survive a deadline: the host also extends automatically at the boundary whenever the same evidence of progress is present. Use it when you already know the next segment is large."
                .to_string(),
            parameters: serde_json::to_value(schema_for!(ExtendTimeBudgetArgs))
                .unwrap_or_else(|_| json!({ "type": "object" })),
        }
    }

    async fn call(&self, _args: Self::Args) -> Result<Self::Output, Self::Error> {
        // The budget lives in the host run loop, which intercepts this call before dispatch. Only a
        // surface with no such loop (the in-process rig path) ever reaches this body.
        let payload = json!({
            "status": "time_budget_unavailable",
            "retryable": false,
            "next_action": "continue_building",
            "message": "This run has no extendable host time budget; continue within the budget you have.",
        });
        Ok(serde_json::to_string_pretty(&payload).unwrap_or_else(|_| payload.to_string()))
    }
}

pub struct PlanBoardScopeTool;

impl Tool for PlanBoardScopeTool {
    const NAME: &'static str = "plan_board_scope";

    type Error = FlowScriptToolError;
    type Args = PlanBoardScopeArgs;
    type Output = String;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: Self::NAME.to_string(),
            description: format!(
                "Declare how the requested behavior will be built, after the declaration lookup and before the first source write. Split the request into ordered segments that are each executable on their own, and pick how they reach the board: \"single\" for one segment (an ordinary edit — this is the common case and costs nothing extra), \"staged\" to grow one draft segment by segment and commit once atomically, \"incremental\" to commit each segment separately when the whole build is too large to reach one commit, or \"multi_board\" when the segments are independent entry points that each deserve their own board, the ordinary shape for a multi-page app where each page owns a board. Segments are NOT stubs: each must fully feed the required inputs of the nodes it adds, and describe concrete behavior rather than deferred work. Unfinished exec tails between segments are expected and do not block validation. At most {MAX_BOARD_SCOPE_SEGMENTS} segments; dependencies must point at earlier segments."
            ),
            parameters: serde_json::to_value(schema_for!(PlanBoardScopeArgs))
                .unwrap_or_else(|_| json!({ "type": "object" })),
        }
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        let payload = match accept_scope_plan(args) {
            Ok(plan) => plan.acceptance_payload(),
            Err(rejection) => rejection.payload(),
        };
        Ok(serde_json::to_string_pretty(&payload).unwrap_or_else(|_| payload.to_string()))
    }
}

impl Tool for WriteFlowScriptTool {
    const NAME: &'static str = "write_flowscript";

    type Error = FlowScriptToolError;
    type Args = WriteFlowScriptArgs;
    type Output = String;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: Self::NAME.to_string(),
            description: "Start a retained code-first FlowScript draft. Write the complete source document; the host binds it to the immutable user request, parses it into internal BoardAst, returns structured source diagnostics, and preserves the exact text for inline preview and later patches. When a retained draft already exists for this same request (a follow-up repair run), do NOT start a new draft: reuse the SAME draft_id and exact expected_revision and repair it in place. Preserve existing function cache decorators. To cache an input-determined function, place `@cache({ namespace: \"...\", ttlSeconds: 3600, scope: \"user\" })` immediately above it; bare `@cache` defaults to the `global` namespace, 300 seconds, and app scope, while `ttlSeconds: 0` is permanent. A cache hit skips the entire body and all side effects. Function returns accept node outputs, params, literals, and mutable `let` bindings (one return value per declared return pin). A `let` reassigned across if/for promotes to a board variable with its initializer preserved; never reassign a `const` inside a branch arm — declare it with `let`. Catalog-related diagnostics automatically include exact live signatures or bounded candidates in fix.catalog_declarations and structural context in fix.companion_declarations; use those before another lookup. Defaults to additive scope. Use replace mode only for an intentional complete-board document."
                .to_string(),
            parameters: serde_json::to_value(schema_for!(WriteFlowScriptArgs))
                .unwrap_or_else(|_| json!({ "type": "object" })),
        }
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        let catalog = self.provider.get_all_metadata().await;
        self.store.observe_board(&self.board);
        Ok(self
            .store
            .write_flowscript_with_acceptance_binding(
                &self.board,
                &catalog,
                args,
                &self.acceptance_binding,
            )
            .render_for_model(&self.board))
    }
}

impl Tool for PatchFlowScriptTool {
    const NAME: &'static str = "patch_flowscript";

    type Error = FlowScriptToolError;
    type Args = PatchFlowScriptArgs;
    type Output = String;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: Self::NAME.to_string(),
            description: "Patch one exact, uniquely occurring text range in a retained FlowScript draft using revision compare-and-swap. This is the way to resume a retained draft in a follow-up repair run: keep its SAME draft_id and exact expected_revision instead of rewriting from scratch. The full updated source and structured diagnostics are returned inline. Preserve an existing function `@cache` decorator during unrelated repairs; the canonical configured form is `@cache({ namespace: \"...\", ttlSeconds: 3600, scope: \"user\" })`. Bare `@cache` defaults to the `global` namespace, 300 seconds, and app scope; `ttlSeconds: 0` is permanent. A cache hit skips the body and its side effects. Function returns accept node outputs, params, literals, and mutable `let` bindings; a `let` reassigned across if/for promotes to a board variable — never reassign a `const` inside a branch arm. Catalog-related diagnostics automatically include repair signatures in fix.catalog_declarations and structural context in fix.companion_declarations; use those before another lookup. Ambiguous, stale, replayed, or scope-collapsing patches do not mutate the draft."
                .to_string(),
            parameters: serde_json::to_value(schema_for!(PatchFlowScriptArgs))
                .unwrap_or_else(|_| json!({ "type": "object" })),
        }
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        let catalog = self.provider.get_all_metadata().await;
        self.store.observe_board(&self.board);
        Ok(self
            .store
            .patch_flowscript_with_acceptance_binding(
                &self.board,
                &catalog,
                args,
                &self.acceptance_binding,
            )
            .render_for_model(&self.board))
    }
}

impl Tool for CheckFlowScriptTool {
    const NAME: &'static str = "check_flowscript";

    type Error = FlowScriptToolError;
    type Args = CheckFlowScriptArgs;
    type Output = String;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: Self::NAME.to_string(),
            description: "Parse, type/reconcile-check, and retain the exact BoardCommand batch for one FlowScript revision without queueing mutations. Every diagnostic is structured and source-located where possible; catalog-related fixes include exact live signatures or bounded candidates in fix.catalog_declarations and structural context in fix.companion_declarations, which should be used before another lookup. Commit is refused until this exact revision checks cleanly."
                .to_string(),
            parameters: serde_json::to_value(schema_for!(CheckFlowScriptArgs))
                .unwrap_or_else(|_| json!({ "type": "object" })),
        }
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        let catalog = self.provider.get_all_metadata().await;
        self.store.observe_board(&self.board);
        Ok(self
            .store
            .check_flowscript_with_acceptance_binding(
                &self.board,
                &catalog,
                args,
                &self.acceptance_binding,
            )
            .render_for_model(&self.board))
    }
}

impl Tool for CommitFlowScriptTool {
    const NAME: &'static str = "commit_flowscript";

    type Error = FlowScriptToolError;
    type Args = CommitFlowScriptArgs;
    type Output = String;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: Self::NAME.to_string(),
            description: "Idempotently claim the exact command batch retained by check_flowscript for this source revision. It refuses stale board, catalog, revision, or request state and requires exact per-entity removal ids for replacement drafts. If the live catalog changed, run check_flowscript again at the same revision before retrying. The existing Apply/Dismiss review boundary remains authoritative."
                .to_string(),
            parameters: serde_json::to_value(schema_for!(CommitFlowScriptArgs))
                .unwrap_or_else(|_| json!({ "type": "object" })),
        }
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        let catalog = self.provider.get_all_metadata().await;
        self.store.observe_board(&self.board);
        Ok(self
            .store
            .commit_flowscript_with_acceptance_binding(
                &self.board,
                &catalog,
                args,
                &self.acceptance_binding,
            )
            .render_for_model(&self.board))
    }
}

// ============================================================================
// Tool Execution Helpers
// ============================================================================

pub fn build_list_board_nodes_output(graph_context: &GraphContext) -> String {
    if graph_context.nodes.is_empty() && graph_context.layers.is_empty() {
        return "The board is empty - no nodes found. Use get_declarations to find FlowScript signatures, call plan_board_scope exactly once unless the host already accepted a plan, then write_flowscript, check_flowscript, and commit_flowscript. Use patch_flowscript for any diagnostic repairs."
            .to_string();
    }

    let mut lines = Vec::new();
    lines.push(format!("Board has {} nodes:", graph_context.nodes.len()));

    for node in &graph_context.nodes {
        let selected = if graph_context.selected_nodes.contains(&node.id) {
            " [SELECTED]"
        } else {
            ""
        };
        lines.push(format!(
            "- {} | {} | {} | pos:({},{}){}",
            node.id, node.node_type, node.friendly_name, node.position.0, node.position.1, selected
        ));
    }

    if !graph_context.layers.is_empty() {
        lines.push(format!("\nLayers ({}):", graph_context.layers.len()));
        for layer in &graph_context.layers {
            let parent = layer.parent_id.as_deref().unwrap_or("root");
            let cache = layer.cache.as_ref().map_or_else(String::new, |cache| {
                let ttl = cache
                    .ttl_seconds
                    .filter(|seconds| *seconds > 0)
                    .map(|seconds| format!("{seconds}s"))
                    .unwrap_or_else(|| "no-expiry".to_string());
                format!(
                    " | cache:{} namespace:{:?} ttl:{} scope:{}",
                    if cache.enabled { "on" } else { "off" },
                    cache.namespace,
                    ttl,
                    cache.scope,
                )
            });
            lines.push(format!(
                "- {} | {} | type:{} | parent:{} | nodes:{} | pos:({},{}){}",
                layer.id,
                layer.name,
                layer.layer_type,
                parent,
                layer.node_ids.len(),
                layer.position.0,
                layer.position.1,
                cache,
            ));
        }
    }

    if !graph_context.variables.is_empty() {
        lines.push(format!("\nVariables ({}):", graph_context.variables.len()));
        for variable in &graph_context.variables {
            lines.push(format!(
                "- {}: {} ({}/{})",
                variable.id, variable.name, variable.data_type, variable.value_type
            ));
        }
    }

    lines.push("\n→ Use get_node_details(node_id) to inspect exact pin names".to_string());
    lines.join("\n")
}

pub fn build_unconfigured_nodes_output(graph_context: &GraphContext) -> String {
    let connected_pins: std::collections::HashSet<(String, String)> = graph_context
        .edges
        .iter()
        .map(|edge| (edge.to_node_id.clone(), edge.to_pin_name.clone()))
        .collect();

    let mut unconfigured = Vec::new();

    for node in &graph_context.nodes {
        let missing_inputs: Vec<_> = node
            .inputs
            .iter()
            .filter(|input| input.type_name != "Execution")
            .filter(|input| {
                !connected_pins.contains(&(node.id.clone(), input.name.clone()))
                    && input.default_value.is_none()
            })
            .map(|input| {
                json!({
                    "pin": input.name,
                    "type": input.type_name,
                })
            })
            .collect();

        if !missing_inputs.is_empty() {
            unconfigured.push(json!({
                "node_id": node.id,
                "node_type": node.node_type,
                "name": node.friendly_name,
                "missing_inputs": missing_inputs,
            }));
        }
    }

    if unconfigured.is_empty() {
        "All nodes are configured - no missing non-execution inputs found.".to_string()
    } else {
        serde_json::to_string_pretty(&unconfigured).unwrap_or_default()
    }
}

pub async fn build_find_connectable_nodes_output(
    graph_context: &GraphContext,
    provider: &dyn CatalogProvider,
    args: FindConnectableNodesArgs,
) -> Result<String, BoardInspectionToolError> {
    let limit = args.limit.unwrap_or(8).clamp(1, 20);

    let mut pin_direction = None;
    let mut pin_type = None;

    if let Some(node) = graph_context
        .nodes
        .iter()
        .find(|node| node.id == args.node_id)
    {
        if let Some(pin) = node.inputs.iter().find(|pin| pin.name == args.pin_name) {
            pin_direction = Some("input");
            pin_type = Some(pin.type_name.clone());
        } else if let Some(pin) = node.outputs.iter().find(|pin| pin.name == args.pin_name) {
            pin_direction = Some("output");
            pin_type = Some(pin.type_name.clone());
        }
    }

    if pin_type.is_none()
        && let Some(layer) = graph_context
            .layers
            .iter()
            .find(|layer| layer.id == args.node_id)
    {
        if let Some(pin) = layer.inputs.iter().find(|pin| pin.name == args.pin_name) {
            pin_direction = Some("input");
            pin_type = Some(pin.type_name.clone());
        } else if let Some(pin) = layer.outputs.iter().find(|pin| pin.name == args.pin_name) {
            pin_direction = Some("output");
            pin_type = Some(pin.type_name.clone());
        }
    }

    let pin_type = pin_type.ok_or_else(|| {
        BoardInspectionToolError(format!(
            "Pin '{}' not found on node/layer '{}'",
            args.pin_name, args.node_id
        ))
    })?;

    let search_for_inputs = pin_direction == Some("output");
    let mut matches = provider
        .search_by_pin_type(&pin_type, search_for_inputs)
        .await;

    matches.retain(|metadata| metadata.name != args.node_id);

    if let Some(intent) = args.intent.as_ref() {
        matches.sort_by(|left, right| {
            score_catalog_metadata(right, intent).cmp(&score_catalog_metadata(left, intent))
        });
    }

    let payload = json!({
        "source": {
            "node_id": args.node_id,
            "pin_name": args.pin_name,
            "pin_type": pin_type,
            "pin_direction": pin_direction.unwrap_or("unknown"),
            "searching_for": if search_for_inputs { "input pins" } else { "output pins" },
        },
        "candidates": matches.into_iter().take(limit).collect::<Vec<_>>(),
    });

    Ok(serde_json::to_string_pretty(&payload).unwrap_or_default())
}

/// Get a human-readable description for a tool call
pub fn get_tool_description(name: &str, arguments: &serde_json::Value) -> String {
    match name {
        "think" => {
            if let Some(thought) = arguments.get("thought").and_then(|v| v.as_str()) {
                thought.to_string()
            } else {
                "Reasoning through the problem...".to_string()
            }
        }
        "get_node_details" => {
            if let Some(node_id) = arguments.get("node_id").and_then(|v| v.as_str()) {
                format!("Getting details for node {}", node_id)
            } else {
                "Getting node details...".to_string()
            }
        }
        "emit_commands" => {
            if let Some(commands) = arguments.get("commands").and_then(|v| v.as_array()) {
                format!("Preparing {} change(s)...", commands.len())
            } else {
                "Preparing changes...".to_string()
            }
        }
        "catalog_search" => {
            if let Some(query) = arguments.get("query").and_then(|v| v.as_str()) {
                format!("Searching catalog for \"{}\"", query)
            } else {
                "Searching the catalog...".to_string()
            }
        }
        "search_by_pin" => {
            if let Some(pin_type) = arguments.get("pin_type").and_then(|v| v.as_str()) {
                format!("Finding nodes with {} pins", pin_type)
            } else {
                "Finding compatible nodes...".to_string()
            }
        }
        "find_connectable_nodes" => {
            let node_id = arguments
                .get("node_id")
                .and_then(|v| v.as_str())
                .unwrap_or("node");
            let pin_name = arguments
                .get("pin_name")
                .and_then(|v| v.as_str())
                .unwrap_or("pin");
            format!("Finding connectable nodes for {}.{}", node_id, pin_name)
        }
        "list_board_nodes" => "Listing nodes in the current workflow...".to_string(),
        "get_unconfigured_nodes" => "Checking which nodes still need configuration...".to_string(),
        "filter_category" => {
            if let Some(category) = arguments.get("category_prefix").and_then(|v| v.as_str()) {
                format!("Browsing {} category", category)
            } else {
                "Browsing categories...".to_string()
            }
        }
        "search_templates" => {
            if let Some(query) = arguments.get("query").and_then(|v| v.as_str()) {
                format!("Searching templates for \"{}\"", query)
            } else {
                "Searching templates...".to_string()
            }
        }
        "query_logs" => {
            if let Some(query) = arguments.get("query").and_then(|v| v.as_str()) {
                format!("Searching logs for \"{}\"", query)
            } else {
                "Querying execution logs...".to_string()
            }
        }
        "execute_event" => arguments
            .get("event_id")
            .or_else(|| arguments.get("eventId"))
            .and_then(|value| value.as_str())
            .map(|event_id| format!("Executing Event {event_id} and collecting logs..."))
            .unwrap_or_else(|| "Executing workflow Event and collecting logs...".to_string()),
        "execute_node" => arguments
            .get("node_id")
            .or_else(|| arguments.get("nodeId"))
            .and_then(|value| value.as_str())
            .map(|node_id| format!("Executing workflow from node {node_id}..."))
            .unwrap_or_else(|| "Executing workflow from board node...".to_string()),
        "run_board_tests" => arguments
            .get("board_id")
            .or_else(|| arguments.get("boardId"))
            .and_then(|value| value.as_str())
            .map(|board_id| format!("Running test events on board {board_id}..."))
            .unwrap_or_else(|| "Running board test events...".to_string()),
        "query_execution_logs" => arguments
            .get("run_id")
            .or_else(|| arguments.get("runId"))
            .and_then(|value| value.as_str())
            .map(|run_id| format!("Reading execution logs for run {run_id}..."))
            .unwrap_or_else(|| "Reading persisted execution logs...".to_string()),
        "get_declarations" => {
            if let Some(query) = arguments.get("query").and_then(|v| v.as_str()) {
                format!("Looking up FlowScript declarations for \"{}\"", query)
            } else {
                "Looking up FlowScript declarations...".to_string()
            }
        }
        "get_current_flowscript" => "Reading current board FlowScript...".to_string(),
        "write_flowscript" => "Writing and previewing a retained FlowScript draft...".to_string(),
        "patch_flowscript" => "Patching the retained FlowScript source...".to_string(),
        "check_flowscript" => "Checking FlowScript and retaining its exact changes...".to_string(),
        "commit_flowscript" => "Queueing the exact checked FlowScript changes...".to_string(),
        "plan_flow_ir" => {
            "Checking required workflow capabilities and module budgets...".to_string()
        }
        "begin_flow_ir_draft" => "Starting a typed workflow draft...".to_string(),
        "update_flow_ir_draft" => "Repairing the retained typed workflow draft...".to_string(),
        "upsert_flow_ir_module" => arguments
            .get("module")
            .and_then(|module| module.get("name"))
            .and_then(|name| name.as_str())
            .map(|name| format!("Compiling typed workflow module {name}..."))
            .unwrap_or_else(|| "Compiling a typed workflow module...".to_string()),
        "validate_flow_ir_draft" => "Validating the complete typed workflow draft...".to_string(),
        "commit_flow_ir_draft" => {
            "Atomically compiling and queueing the typed workflow...".to_string()
        }
        "edit_flowscript" => "Applying FlowScript edits to the board...".to_string(),
        _ => format!("Running {}...", name),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use flow_like_types::tokio;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[derive(Default)]
    struct BatchDispatchProvider {
        batch_calls: AtomicUsize,
    }

    fn declaration_resolution_test_section(
        query: &str,
        status: &str,
        body: impl AsRef<str>,
    ) -> String {
        format!(
            "// flowpilot.declaration-resolution/v1 {}\n{}",
            json!({
                "query": query,
                "status": status,
                "top_score": if matches!(status, "exact" | "resolved") { Some(200) } else { None },
                "runner_up_score": null,
                "margin": if matches!(status, "exact" | "resolved") { Some(200) } else { None },
                "reason_codes": if matches!(status, "exact" | "resolved") {
                    vec!["test_confident_resolution"]
                } else {
                    vec!["test_resolver_abstained"]
                },
                "candidates": [],
            }),
            body.as_ref()
        )
    }

    #[async_trait::async_trait]
    impl CatalogProvider for BatchDispatchProvider {
        async fn search(&self, _query: &str) -> Vec<super::super::types::NodeMetadata> {
            Vec::new()
        }

        async fn search_by_pin_type(
            &self,
            _pin_type: &str,
            _is_input: bool,
        ) -> Vec<super::super::types::NodeMetadata> {
            Vec::new()
        }

        async fn filter_by_category(
            &self,
            _category_prefix: &str,
        ) -> Vec<super::super::types::NodeMetadata> {
            Vec::new()
        }

        async fn get_node_metadata(
            &self,
            _node_type: &str,
        ) -> Option<super::super::types::NodeMetadata> {
            None
        }

        async fn get_all_nodes(&self) -> Vec<String> {
            Vec::new()
        }

        async fn get_declarations(&self, _query: &str) -> String {
            panic!("multi-query dispatch must use get_declarations_batch")
        }

        async fn get_declarations_batch(&self, queries: &[String]) -> Vec<String> {
            self.batch_calls.fetch_add(1, Ordering::SeqCst);
            queries
                .iter()
                .map(|query| {
                    let function_name = query.replace(' ', "");
                    declaration_resolution_test_section(
                        query,
                        "resolved",
                        format!("declare function {function_name}(): void;"),
                    )
                })
                .collect()
        }
    }

    #[tokio::test]
    async fn active_flowscript_tools_document_function_cache_decorators() {
        let board = Arc::new(Board::new_detached(
            Some("cache-tool-docs".to_string()),
            flow_like_storage::Path::default(),
        ));
        let provider: Arc<dyn CatalogProvider> = Arc::new(BatchDispatchProvider::default());
        let store = Arc::new(FlowIrDraftStore::new());
        let acceptance_binding =
            store.bind_request_acceptance_contract(&board.id, "cache calculatePricing");

        let current_description = GetCurrentFlowScriptTool {
            board: board.clone(),
        }
        .definition(String::new())
        .await
        .description;
        let write_description = WriteFlowScriptTool {
            board: board.clone(),
            provider: provider.clone(),
            store: store.clone(),
            acceptance_binding: acceptance_binding.clone(),
        }
        .definition(String::new())
        .await
        .description;
        let patch_description = PatchFlowScriptTool {
            board,
            provider,
            store,
            acceptance_binding,
        }
        .definition(String::new())
        .await
        .description;

        for description in [
            current_description.as_str(),
            write_description.as_str(),
            patch_description.as_str(),
        ] {
            assert!(description.contains("@cache"));
            assert!(description.contains("namespace"));
            assert!(description.contains("ttlSeconds"));
            assert!(description.contains("scope"));
            assert!(description.contains("global"));
            assert!(description.contains("300"));
            assert!(description.contains("ttlSeconds: 0"));
        }
        assert!(current_description.contains("ttl_seconds: null"));
        assert!(write_description.contains("skips the entire body and all side effects"));
    }

    #[test]
    fn board_listing_exposes_default_function_cache_settings() {
        use super::super::context::{LayerCacheContext, LayerContext};

        let graph = GraphContext {
            nodes: vec![],
            edges: vec![],
            layers: vec![LayerContext {
                id: "pricing-layer".to_string(),
                name: "calculatePricing".to_string(),
                layer_type: "Function".to_string(),
                parent_id: None,
                node_ids: vec![],
                position: (10, 20),
                inputs: vec![],
                outputs: vec![],
                cache: Some(LayerCacheContext {
                    enabled: true,
                    namespace: "global".to_string(),
                    ttl_seconds: Some(300),
                    scope: "app".to_string(),
                }),
            }],
            variables: vec![],
            selected_nodes: vec![],
        };

        let listing = build_list_board_nodes_output(&graph);
        assert!(listing.contains("cache:on"));
        assert!(listing.contains("namespace:\"global\""));
        assert!(listing.contains("ttl:300s"));
        assert!(listing.contains("scope:app"));
    }

    #[test]
    fn board_listing_reports_zero_cache_ttl_as_no_expiry() {
        use super::super::context::{LayerCacheContext, LayerContext};

        let graph = GraphContext {
            nodes: vec![],
            edges: vec![],
            layers: vec![LayerContext {
                id: "pricing-layer".to_string(),
                name: "calculatePricing".to_string(),
                layer_type: "Function".to_string(),
                parent_id: None,
                node_ids: vec![],
                position: (10, 20),
                inputs: vec![],
                outputs: vec![],
                cache: Some(LayerCacheContext {
                    enabled: true,
                    namespace: "pricing".to_string(),
                    ttl_seconds: Some(0),
                    scope: "app".to_string(),
                }),
            }],
            variables: vec![],
            selected_nodes: vec![],
        };

        let listing = build_list_board_nodes_output(&graph);
        assert!(listing.contains("ttl:no-expiry"));
        assert!(!listing.contains("ttl:0s"));
    }

    #[test]
    fn model_facing_emit_schema_exposes_visual_commands_only() {
        let schema = model_facing_emit_commands_parameters();
        let variants = schema
            .pointer("/properties/commands/items/oneOf")
            .and_then(Value::as_array)
            .expect("emit_commands variants");
        let command_types = variants
            .iter()
            .filter_map(|variant| {
                variant
                    .pointer("/properties/command_type/const")
                    .and_then(Value::as_str)
            })
            .collect::<HashSet<_>>();

        assert_eq!(
            command_types,
            HashSet::from(["MoveNode", "CreateComment", "DeleteComment"])
        );

        let encoded = schema.to_string();
        for executable in [
            "AddNode",
            "AddPlaceholder",
            "RemoveNode",
            "ConnectPins",
            "DisconnectPins",
            "UpdateNodePin",
            "SetNodeFunctionRefs",
            "CreateVariable",
            "UpdateVariable",
            "DeleteVariable",
            "CreateLayer",
            "RemoveLayer",
            "RenameLayer",
            "MoveToLayer",
        ] {
            assert!(
                !encoded.contains(executable),
                "model-facing schema leaked {executable}"
            );
        }
        assert!(!encoded.contains("layer_type"));
        assert!(!encoded.contains("pins"));
        let move_node = variants
            .iter()
            .find(|variant| {
                variant
                    .pointer("/properties/command_type/const")
                    .and_then(Value::as_str)
                    == Some("MoveNode")
            })
            .expect("MoveNode schema");
        assert!(move_node.pointer("/properties/target_layer").is_none());
    }

    fn rich_support_repair_candidate() -> &'static str {
        r#"@secret
const IMAP_HOST: string = ""
@secret
const SMTP_HOST: string = ""

function fetchSupportMail() {
    const connection = emailImapConnect({ host: IMAP_HOST })
    const inbox = mailImapInbox({ connection: connection })
    const refs = mailImapList({ inbox: inbox })
    const mail = emailImapInboxFetchMail({ emailRef: refs })
    logInfo({ message: "mail fetched" })
}

function requestApproval() {
    const smtp = emailSmtpConnect({ host: SMTP_HOST })
    emailSmtpSend({ connection: smtp, body: "approve" })
    logInfo({ message: "approval requested" })
}

eventsSimple() {
    fetchSupportMail()
    requestApproval()
}

eventsGeneric(payload: Struct) {
    requestApproval()
}
"#
    }

    #[test]
    fn edit_flowscript_args_accept_common_source_aliases() {
        for key in ["flowscript", "script", "source", "content"] {
            let args: EditFlowScriptArgs =
                serde_json::from_value(json!({ key: "const db = openLocalDb({ name: \"x\" });" }))
                    .expect("alias should deserialize");
            assert!(args.flowscript.contains("openLocalDb"));
        }
    }

    #[test]
    fn runtime_verification_args_accept_provider_key_variants() {
        let execute: ExecuteNodeArgs = serde_json::from_value(json!({
            "appId": "app",
            "boardId": "board",
            "nodeId": "node",
            "streamState": true
        }))
        .unwrap();
        let serialized = runtime_tool_arguments(ExecuteNodeTool::NAME, execute).unwrap();
        assert_eq!(serialized["board_id"], "board");
        assert_eq!(serialized["node_id"], "node");

        let logs: QueryExecutionLogsArgs = serde_json::from_value(json!({
            "appId": "app",
            "boardId": "board",
            "runId": "run",
            "query": "log_level >= 3",
            "runMetadata": {"is_remote": false}
        }))
        .unwrap();
        let serialized = runtime_tool_arguments(QueryExecutionLogsTool::NAME, logs).unwrap();
        assert_eq!(serialized["filter"], "log_level >= 3");
        assert_eq!(serialized["run_metadata"]["is_remote"], false);
    }

    #[test]
    fn runtime_verification_args_reject_empty_required_ids() {
        let error = runtime_tool_arguments(
            ExecuteNodeTool::NAME,
            ExecuteNodeArgs {
                app_id: None,
                board_id: "board".to_string(),
                node_id: "  ".to_string(),
                payload: None,
                stream_state: None,
            },
        )
        .unwrap_err();
        assert!(error.to_string().contains("node_id"));
    }

    #[test]
    fn edit_flowscript_result_flags_comment_only_empty_board_drafts() {
        let result = ReconcileResult::default();
        let output = render_edit_flowscript_result(
            "// Implementation plan: call openLocalDb later",
            &result,
            true,
            false,
        );

        assert!(output.contains("\"status\":\"validation_errors\""));
        assert!(output.contains("plan/stub"));
        assert!(output.contains("Nothing was queued"));
    }

    #[test]
    fn edit_flowscript_result_includes_workspace_tag_for_preview() {
        let result = ReconcileResult::default();
        let output = render_edit_flowscript_result(
            "run() {\n    const db = openLocalDb({ name: \"gmail_vectors\" })\n}",
            &result,
            false,
            false,
        );

        assert!(output.starts_with("<flowscript_workspace>"));
        assert!(output.contains("\"source\""));
        assert!(output.contains("openLocalDb"));
    }

    #[test]
    fn edit_flowscript_result_flags_empty_function_shells() {
        let result = ReconcileResult::default();
        let output = render_edit_flowscript_result("run() {\n}", &result, true, false);

        assert!(output.contains("\"status\":\"validation_errors\""));
        assert!(output.contains("no executable catalog calls"));
    }

    #[test]
    fn edit_flowscript_result_explains_colon_parse_errors() {
        let result = ReconcileResult {
            commands: Vec::new(),
            corrections: Vec::new(),
            diagnostics: vec![
                "FlowScript parse error at line 31, col 21: expected `Colon`, found `Assign`"
                    .to_string(),
            ],
        };
        let output = render_edit_flowscript_result(
            "run() {\n    emailImapConnect({ host = \"imap.gmail.com\" })\n}",
            &result,
            true,
            false,
        );

        assert!(output.contains("\"status\":\"validation_errors\""));
        assert!(output.contains("colon syntax"));
        assert!(output.contains("not `{ host ="));
    }

    #[test]
    fn edit_flowscript_result_explains_const_binding_parse_errors() {
        let result = ReconcileResult {
            commands: Vec::new(),
            corrections: Vec::new(),
            diagnostics: vec![
                "FlowScript parse error at line 45, col 9: `const` binding requires a call expression"
                    .to_string(),
            ],
        };
        let output = render_edit_flowscript_result(
            "run() {\n    const row = { id: \"x\" }\n}",
            &result,
            true,
            false,
        );

        assert!(output.contains("\"status\":\"validation_errors\""));
        assert!(output.contains("can only bind the output of a node call"));
        assert!(output.contains("local alias syntax like `let rows = []`"));
    }

    #[test]
    fn edit_flowscript_result_identifies_helpers_missing_function_keyword() {
        let source = r#"connectImap() {
    emailImapConnect({ host: "imap.example.com", username: "user", password: "secret" })
}

eventsSimple() {
    connectImap()
}
"#;
        let result = ReconcileResult {
            commands: Vec::new(),
            corrections: Vec::new(),
            diagnostics: vec![
                "FlowScript call `connectImap` does not match a catalog declaration; call `get_declarations` and use the exact function name"
                    .to_string(),
            ],
        };

        let output = render_edit_flowscript_result(source, &result, true, false);

        assert!(output.contains("`connectImap`"));
        assert!(output.contains("missing the required `function` keyword"));
        assert!(output.contains("another declaration search will not fix"));
    }

    #[test]
    fn edit_flowscript_result_explains_missing_named_return_pin() {
        let result = ReconcileResult {
            commands: Vec::new(),
            corrections: Vec::new(),
            diagnostics: vec!["return value 1 has no matching function return pin".to_string()],
        };
        let output = render_edit_flowscript_result(
            "function classify() {\n    return result.value\n}",
            &result,
            true,
            false,
        );

        assert!(output.contains("declares no matching Function output pin"));
        assert!(output.contains(": (isSupport: bool)"));
    }

    #[test]
    fn edit_flowscript_result_blocks_partial_control_flow_commands() {
        let result = ReconcileResult {
            commands: vec![BoardCommand::AddNode {
                node_type: "control_for_each".to_string(),
                ref_id: Some("$0".to_string()),
                position: None,
                friendly_name: None,
                additional_pins: None,
                target_layer: None,
                summary: None,
            }],
            corrections: Vec::new(),
            diagnostics: vec![
                "new FlowScript loop statements are not yet converted automatically; repair the loop with supported FlowScript declarations because model-facing emit_commands cannot wire executable loop bodies"
                    .to_string(),
            ],
        };
        let output = render_edit_flowscript_result(
            "run() {\n    for (const item of controlForEach({ array: rows })) {\n        log({ text: item.value })\n    }\n}",
            &result,
            true,
            false,
        );

        assert!(output.contains("\"status\":\"validation_errors\""));
        assert!(output.contains("partial commands"));
        assert!(!output.contains("<commands>"));
    }

    #[test]
    fn edit_flowscript_result_blocks_commands_for_any_diagnostic() {
        let result = ReconcileResult {
            commands: vec![BoardCommand::AddNode {
                node_type: "log".to_string(),
                ref_id: Some("$0".to_string()),
                position: None,
                friendly_name: None,
                additional_pins: None,
                target_layer: None,
                summary: None,
            }],
            corrections: Vec::new(),
            diagnostics: vec!["future reconcile diagnostic wording".to_string()],
        };

        let output = render_edit_flowscript_result(
            "eventsSimple() {\n    log({ message: \"hi\" })\n}\n",
            &result,
            true,
            false,
        );

        assert!(output.contains("\"status\":\"validation_errors\""));
        assert!(output.contains("future reconcile diagnostic wording"));
        assert!(!output.contains("<commands>"));
    }

    #[test]
    fn stub_markers_ignore_string_literals_and_identifier_substrings() {
        let result = ReconcileResult {
            commands: vec![BoardCommand::AddNode {
                node_type: "log".to_string(),
                ref_id: Some("$0".to_string()),
                position: None,
                friendly_name: None,
                additional_pins: None,
                target_layer: None,
                summary: None,
            }],
            corrections: Vec::new(),
            diagnostics: Vec::new(),
        };
        // "Todo" lives in a user-facing label literal and "todoList" is an identifier; neither is
        // a stub. Before literal-stripping + word boundaries this rendered "Nothing was queued."
        let source = "eventsSimple() {\n    const todoList = listCreate({ label: \"Todo entries\" })\n    logInfo({ message: \"Replace with care\" })\n}\n";
        let output = render_edit_flowscript_result(source, &result, false, false);
        assert!(!output.contains("Nothing was queued."), "{output}");
        assert!(output.contains("<commands>"), "{output}");

        // A genuine stub comment must still be caught.
        let stub_source = "eventsSimple() {\n    // TODO: wire with the fetcher\n    logInfo({ message: \"x\" })\n}\n";
        let stub_output = render_edit_flowscript_result(stub_source, &result, false, false);
        assert!(stub_output.contains("Nothing was queued."), "{stub_output}");
    }

    #[test]
    fn committed_render_never_voids_a_queued_batch_over_stub_markers() {
        let result = ReconcileResult {
            commands: vec![BoardCommand::AddNode {
                node_type: "log".to_string(),
                ref_id: Some("$0".to_string()),
                position: None,
                friendly_name: None,
                additional_pins: None,
                target_layer: None,
                summary: None,
            }],
            corrections: Vec::new(),
            diagnostics: Vec::new(),
        };
        // Even a marker outside string literals must not void a commit the draft store already
        // validated: the store holds the pending claim, so "Nothing was queued." would wedge the
        // board with no agent-reachable recovery.
        let source =
            "eventsSimple() {\n    // todo\n    logInfo({ message: \"queued anyway\" })\n}\n";
        let output = render_committed_flowscript_result(source, &result, false, true);
        assert!(!output.contains("Nothing was queued."), "{output}");
        assert!(output.contains("<commands>"), "{output}");
    }

    #[test]
    fn event_only_flowscript_cannot_queue_on_an_empty_board() {
        let result = ReconcileResult {
            commands: vec![BoardCommand::AddNode {
                node_type: "events_simple".to_string(),
                ref_id: Some("$0".to_string()),
                position: None,
                friendly_name: None,
                additional_pins: None,
                target_layer: None,
                summary: None,
            }],
            corrections: Vec::new(),
            diagnostics: Vec::new(),
        };

        let output = render_edit_flowscript_result("eventsSimple() {\n}\n", &result, true, false);

        assert!(output.contains("\"status\":\"validation_errors\""));
        assert!(output.contains("only a future Event registration target"));
        assert!(!output.contains("<commands>"));
    }

    #[test]
    fn repair_tracker_blocks_tiny_direct_event_after_rich_failure() {
        let mut tracker = FlowScriptRepairTracker::default();
        tracker.record_failed(rich_support_repair_candidate());

        let tiny = r#"eventsSimple() {
    logInfo({ message: "test" })
}
"#;
        let regression = tracker
            .queued_candidate_regression(tiny)
            .expect("a direct smoke-test Event must not replace the support application");
        assert!(regression.previous_call_sites > regression.candidate_call_sites);

        let retained = tracker.best_failed_source().unwrap();
        let output = render_flowscript_candidate_regression(retained, &regression);
        assert!(output.contains("candidate_regression"));
        assert!(output.contains("Nothing was queued"));
        assert!(output.contains("fetchSupportMail"));
        assert!(output.contains("non-empty named `function` helpers"));
    }

    #[test]
    fn repair_tracker_rejects_log_only_helper_wrapped_as_modular_partial() {
        let mut tracker = FlowScriptRepairTracker::default();
        tracker.record_failed(rich_support_repair_candidate());
        let wrapped_smoke_test = r#"function smokeTest() {
    logInfo({ message: "test" })
}

eventsSimple() {
    smokeTest()
}
"#;

        assert!(
            tracker
                .queued_candidate_regression(wrapped_smoke_test)
                .is_some(),
            "wrapping the same one-node smoke test in a helper is not a real modular partial"
        );
    }

    #[test]
    fn repair_tracker_allows_working_domain_helper_with_thin_event() {
        let mut tracker = FlowScriptRepairTracker::default();
        tracker.record_failed(rich_support_repair_candidate());
        let modular_partial = r#"function pollInbox() {
    emailImapConnect({ host: "imap.example.com" })
}

eventsSimple() {
    pollInbox()
}
"#;
        let profile = profile_flowscript_candidate(modular_partial);
        assert_eq!(profile.helper_domain_call_sites, 1);
        assert_eq!(profile.events_calling_helpers, 1);
        assert!(
            tracker
                .queued_candidate_regression(modular_partial)
                .is_none()
        );
        let partial = tracker
            .queued_candidate_modular_fallback(modular_partial)
            .expect("major modular shrink is accepted but must be labeled partial");
        let output = render_flowscript_modular_partial_result("<commands>[]</commands>", &partial);
        assert!(output.contains("partial_working_slice"));
        assert!(output.contains("not the complete requested application"));
        assert!(output.contains("do not claim the whole app was built"));
    }

    #[test]
    fn repair_tracker_allows_concise_repair_that_keeps_application_scope() {
        let mut tracker = FlowScriptRepairTracker::default();
        tracker.record_failed(rich_support_repair_candidate());
        let concise = r#"@secret
const IMAP_HOST: string = ""
@secret
const SMTP_HOST: string = ""

function fetchSupportMail() {
    emailImapConnect({ host: IMAP_HOST })
}

function requestApproval() {
    emailSmtpConnect({ host: SMTP_HOST })
}

eventsSimple() {
    fetchSupportMail()
}

eventsGeneric(payload: Struct) {
    requestApproval()
}
"#;

        assert!(tracker.queued_candidate_regression(concise).is_none());
    }

    #[test]
    fn repair_tracker_retains_richest_failed_workspace_and_profiles_multiple_events() {
        let rich = rich_support_repair_candidate();
        let mut tracker = FlowScriptRepairTracker::default();
        tracker.record_failed(rich);
        tracker.record_failed("eventsSimple() {\n    logInfo({ message: \"test\" })\n}\n");
        assert_eq!(tracker.best_failed_source(), Some(rich));

        let profile = profile_flowscript_candidate(rich);
        assert_eq!(profile.helper_functions.len(), 2);
        assert_eq!(profile.event_entries, 2);
        assert_eq!(profile.top_level_variables.len(), 2);
        assert_eq!(profile.events_calling_helpers, 2);
    }

    #[test]
    fn detached_chains_count_as_workflow_scope_in_both_profile_paths() {
        let detached = r#"detached {
    logInfo({ message: "keep me" })
}

detached {
    emailSmtpConnect({ host: "smtp.example.com" })
}
"#;

        let profile = profile_flowscript_candidate(detached);
        assert_eq!(profile.call_sites, 2);
        assert_eq!(profile.meaningful_statements, 2);

        // The container is punctuation, not a statement, so a draft that fails to parse and falls
        // back to the lexical estimate measures the same shape.
        let lexical = profile_flowscript_candidate_lexically(detached);
        assert_eq!(lexical.call_sites, profile.call_sites);
        assert_eq!(lexical.meaningful_statements, profile.meaningful_statements);

        // `detached` only opens a container immediately before `{`; anywhere else it is an
        // ordinary identifier the lexical estimate must keep counting.
        assert_eq!(
            profile_flowscript_candidate_lexically("detachedFoo {\n}\n").meaningful_statements,
            1
        );
        assert_eq!(
            profile_flowscript_candidate_lexically("detached(payload: Struct) {\n}\n")
                .meaningful_statements,
            1
        );
    }

    #[test]
    fn repair_tracker_prefers_fewer_diagnostics_within_the_same_scope() {
        let older = rich_support_repair_candidate();
        let newer = older.replace("    logInfo({ message: \"mail fetched\" })\n", "");
        let mut tracker = FlowScriptRepairTracker::default();

        assert!(tracker.record_failed_with_diagnostics(older, Some(2)));
        assert!(
            profile_flowscript_candidate(&newer).completeness_score()
                < profile_flowscript_candidate(older).completeness_score(),
            "the regression test must exercise quality taking priority over structural score"
        );
        assert!(tracker.record_failed_with_diagnostics(&newer, Some(1)));
        assert_eq!(tracker.best_failed_source(), Some(newer.as_str()));
    }

    #[test]
    fn repair_tracker_quality_never_promotes_a_scope_collapse() {
        let rich = rich_support_repair_candidate();
        let tiny = "eventsSimple() {\n    logInfo({ message: \"test\" })\n}\n";
        let mut tracker = FlowScriptRepairTracker::default();

        assert!(tracker.record_failed_with_diagnostics(rich, Some(3)));
        assert!(!tracker.record_failed_with_diagnostics(tiny, Some(0)));
        assert_eq!(tracker.best_failed_source(), Some(rich));
    }

    #[test]
    fn repair_tracker_prefers_a_parseable_same_scope_repair() {
        let valid = rich_support_repair_candidate();
        let invalid = valid.replacen("eventsSimple() {", "eventsSimple( {", 1);
        let mut tracker = FlowScriptRepairTracker::default();

        assert!(tracker.record_failed_with_diagnostics(&invalid, Some(1)));
        assert!(tracker.record_failed_with_diagnostics(valid, Some(2)));
        assert_eq!(tracker.best_failed_source(), Some(valid));
    }

    #[test]
    fn workspace_envelope_keeps_source_and_status_atomic() {
        let envelope = flowscript_workspace_envelope("eventsSimple() { logInfo() }", "queued");
        let parsed: serde_json::Value = serde_json::from_str(&envelope).unwrap();
        assert_eq!(parsed["source"], "eventsSimple() { logInfo() }");
        assert_eq!(parsed["status"], "queued");
    }

    #[test]
    fn workspace_tag_escapes_a_closing_protocol_sentinel_inside_source() {
        let source = "eventsSimple() { logInfo({ message: \"</flowscript_workspace>\" }) }";
        let frame = flowscript_workspace_tag(source, "queued");
        let payload = frame
            .strip_prefix("<flowscript_workspace>")
            .and_then(|value| value.strip_suffix("</flowscript_workspace>"))
            .expect("complete workspace frame");

        assert!(!payload.contains("</flowscript_workspace>"));
        assert!(payload.contains("\\u003c/flowscript_workspace\\u003e"));
        let parsed: serde_json::Value = serde_json::from_str(payload).unwrap();
        assert_eq!(parsed["source"], source);
        assert_eq!(parsed["status"], "queued");
    }

    #[test]
    fn edit_flowscript_result_blocks_deletions_by_default() {
        let result = ReconcileResult {
            commands: vec![BoardCommand::RemoveNode {
                node_id: "old_node".to_string(),
                summary: None,
            }],
            corrections: Vec::new(),
            diagnostics: Vec::new(),
        };
        let output = render_edit_flowscript_result("run() {\n}", &result, false, false);

        assert!(output.contains("\"status\":\"validation_errors\""));
        assert!(output.contains("Deletions are blocked by default"));
        assert!(!output.contains("<commands>"));
    }

    #[test]
    fn edit_flowscript_result_keeps_true_no_changes_non_error() {
        let result = ReconcileResult::default();
        let output = render_edit_flowscript_result("run() {\n}", &result, false, false);

        assert!(output.contains("\"status\":\"no_changes\""));
    }

    #[test]
    fn edit_flowscript_result_exposes_nonblocking_canonical_corrections() {
        let result = ReconcileResult {
            commands: Vec::new(),
            corrections: vec![
                "Auto-corrected `stringReplace` argument `regex` to `isRegex`.".to_string(),
            ],
            diagnostics: Vec::new(),
        };
        let output = render_edit_flowscript_result("run() {\n}", &result, false, false);

        assert!(output.contains("<flowscript_corrections>"));
        assert!(output.contains("stringReplace"));
        assert!(output.contains("isRegex"));
        assert!(output.contains("retained FlowScript source"));
    }

    #[test]
    fn declaration_batch_normalizes_and_deduplicates_without_silent_eight_query_cap() {
        let args = GetDeclarationsArgs {
            query: "  SMTP   send EMAIL  ".to_string(),
            queries: vec![
                "smtp send email".to_string(),
                "IMAP fetch mail".to_string(),
                "imap   FETCH   mail".to_string(),
                "open database".to_string(),
                "invoke model".to_string(),
                "branch condition".to_string(),
                "format string".to_string(),
                "generate cuid".to_string(),
                "ninth query is capped".to_string(),
                "tenth query is capped".to_string(),
            ],
        };

        let queries = declaration_queries(&args);
        assert_eq!(queries.len(), 9);
        assert_eq!(queries[0], "SMTP send EMAIL");
        assert_eq!(queries[1], "IMAP fetch mail");
        assert_eq!(
            queries
                .iter()
                .filter(|query| declaration_query_key(query) == "imap fetch mail")
                .count(),
            1,
            "case/whitespace-equivalent declaration searches must run only once"
        );
        assert!(queries.iter().any(|query| query.contains("tenth")));
    }

    #[tokio::test]
    async fn declaration_tool_requires_one_scope_plan_then_an_early_draft() {
        let provider: Arc<dyn CatalogProvider> = Arc::new(BatchDispatchProvider::default());
        let tool = GetDeclarationsTool { provider };

        let definition = tool.definition(String::new()).await;
        assert!(
            definition
                .description
                .contains("ONE bounded, focused batch")
        );
        assert!(
            definition
                .description
                .contains("highest-leverage catalog calls")
        );
        assert!(definition.description.contains("After ANY usable response"));
        assert!(
            definition
                .description
                .contains("`plan_board_scope` exactly once")
        );
        assert!(definition.description.contains("ACTIVE SEGMENT"));
        assert!(definition.description.contains("omitted_queries"));
        assert!(definition.description.contains("compiler diagnostics"));
        assert!(!definition.description.contains("pass ALL the searches"));
        assert!(
            !definition
                .description
                .contains("list every node capability")
        );

        let queries_description = definition.parameters["properties"]["queries"]["description"]
            .as_str()
            .expect("queries description");
        assert!(queries_description.contains("do not enumerate every utility operation"));
        assert!(queries_description.contains("plan_board_scope exactly once"));
        assert!(queries_description.contains("write its active segment immediately"));
        assert!(queries_description.contains("defer omitted/unmatched searches"));
    }

    #[tokio::test]
    async fn declaration_query_runner_dispatches_one_provider_batch() {
        let concrete = Arc::new(BatchDispatchProvider::default());
        let provider: Arc<dyn CatalogProvider> = concrete.clone();
        let args = GetDeclarationsArgs {
            query: "smtp send".to_string(),
            queries: vec!["imap receive".to_string(), "boolean or".to_string()],
        };

        let result = run_declaration_queries(&provider, &args).await;

        assert_eq!(concrete.batch_calls.load(Ordering::SeqCst), 1);
        assert!(result.contains("\"processed_count\":3"));
        assert!(result.contains("\"matched_count\":3"));
        assert!(result.contains("\"complete\":true"));
    }

    #[test]
    fn declaration_batch_reports_queries_beyond_the_runtime_safety_bound() {
        let args = GetDeclarationsArgs {
            query: String::new(),
            queries: (0..MAX_DECLARATION_QUERIES + 3)
                .map(|index| format!("capability {index}"))
                .collect(),
        };

        let batch = declaration_query_batch(&args);
        assert_eq!(batch.processed.len(), MAX_DECLARATION_QUERIES);
        assert_eq!(batch.omitted.len(), 3);
        assert_eq!(batch.omitted_count, 3);

        let sections = batch
            .processed
            .iter()
            .map(|query| {
                declaration_resolution_test_section(
                    query,
                    "resolved",
                    format!("declare function {query}(): void;"),
                )
            })
            .collect::<Vec<_>>();
        let result = render_declaration_query_batch(&batch, &sections);
        assert!(result.contains("flowpilot.declaration-batch/v1"));
        assert!(result.contains("\"processed_count\":32"));
        assert!(result.contains("\"omitted_count\":3"));
        assert!(result.contains("capability 34"));
        assert!(result.len() <= MAX_DECLARATION_RESPONSE_BYTES);
    }

    #[test]
    fn declaration_batch_reports_match_coverage_and_completeness() {
        let args = GetDeclarationsArgs {
            query: String::new(),
            queries: vec![
                "boolean or".to_string(),
                "unknown package capability".to_string(),
            ],
        };
        let batch = declaration_query_batch(&args);
        let sections = vec![
            declaration_resolution_test_section(
                "boolean or",
                "resolved",
                "// result\n  declare function boolOr({ boolean?: bool }): bool;",
            ),
            declaration_resolution_test_section(
                "unknown package capability",
                "unresolved",
                "// No FlowScript declarations matched this query.",
            ),
        ];

        let result = render_declaration_query_batch(&batch, &sections);

        assert!(result.contains("\"matched_count\":1"));
        assert!(result.contains("\"matched_queries\":[\"boolean or\"]"));
        assert!(result.contains("\"unmatched_count\":1"));
        assert!(result.contains("\"unmatched_queries\":[\"unknown package capability\"]"));
        assert!(result.contains("\"complete\":false"));
    }

    #[test]
    fn declaration_batch_does_not_count_an_unclassified_signature_as_matched() {
        let args = GetDeclarationsArgs {
            query: "integer compare".to_string(),
            queries: Vec::new(),
        };
        let batch = declaration_query_batch(&args);
        let sections =
            vec!["declare function fakerInteger({ min?: int, max?: int }): int;".to_string()];

        let result = render_declaration_query_batch(&batch, &sections);

        assert!(result.contains("\"matched_count\":0"));
        assert!(result.contains("\"unmatched_count\":1"));
        assert!(result.contains("missing_resolution_metadata"));
        assert!(result.contains("\"complete\":false"));
    }

    #[test]
    fn declaration_batch_is_complete_only_when_every_requested_query_matches() {
        let args = GetDeclarationsArgs {
            query: "boolean or".to_string(),
            queries: vec!["smtp send email".to_string()],
        };
        let batch = declaration_query_batch(&args);
        let sections = vec![
            declaration_resolution_test_section(
                "boolean or",
                "resolved",
                "declare function boolOr({ boolean?: bool }): bool;",
            ),
            declaration_resolution_test_section(
                "smtp send email",
                "resolved",
                "declare function emailSmtpSend({ connection: Struct }): void;",
            ),
        ];

        let result = render_declaration_query_batch(&batch, &sections);

        assert!(result.contains("\"matched_count\":2"));
        assert!(result.contains("\"unmatched_count\":0"));
        assert!(result.contains("\"complete\":true"));
    }

    #[test]
    fn declaration_batch_retains_exact_signature_from_oversized_priority_section() {
        let args = GetDeclarationsArgs {
            query: "large live declaration".to_string(),
            queries: Vec::new(),
        };
        let batch = declaration_query_batch(&args);
        let signature =
            "declare function largeLiveDeclaration({ payload: Struct }): string;".to_string();
        let body = format!(
            "// declaration query: large live declaration\n{DECLARATION_PRIORITY_BEGIN}{signature}\n// {}\n{DECLARATION_PRIORITY_END}",
            "usage".repeat(MAX_DECLARATION_RESPONSE_BYTES)
        );
        let section =
            declaration_resolution_test_section("large live declaration", "resolved", body);

        let result = render_declaration_query_batch(&batch, &[section]);

        assert!(result.len() <= MAX_DECLARATION_RESPONSE_BYTES);
        assert!(result.contains(&signature));
        assert!(result.contains("\"output_omitted_count\":0"));
        assert!(result.contains("\"complete\":true"));
    }

    #[test]
    fn declaration_batch_marks_signature_larger_than_global_budget_output_omitted() {
        let args = GetDeclarationsArgs {
            query: "impossibly large declaration".to_string(),
            queries: Vec::new(),
        };
        let batch = declaration_query_batch(&args);
        let signature = format!(
            "declare function impossiblyLargeDeclaration({{ {} }}): void;",
            "payload: string, ".repeat(MAX_DECLARATION_RESPONSE_BYTES)
        );
        let body = format!(
            "// declaration query: impossibly large declaration\n{DECLARATION_PRIORITY_BEGIN}{signature}\n{DECLARATION_PRIORITY_END}"
        );
        let section =
            declaration_resolution_test_section("impossibly large declaration", "resolved", body);

        let result = render_declaration_query_batch(&batch, &[section]);

        assert!(result.len() <= MAX_DECLARATION_RESPONSE_BYTES);
        assert!(!result.contains("declare function impossiblyLargeDeclaration("));
        assert!(result.contains("\"output_omitted_count\":1"));
        assert!(result.contains("\"output_omitted_queries\":[\"impossibly large declaration\"]"));
        assert!(result.contains("\"matched_count\":0"));
        assert!(result.contains("\"complete\":false"));
        assert!(result.contains("Exact declaration omitted"));
        assert!(result.contains("Call plan_board_scope exactly once"));
        assert!(result.contains("retain its active segment now"));
        assert!(result.contains("only if a later compiler diagnostic still requires it"));
    }

    #[test]
    fn declaration_batch_bounds_oversized_queries_and_reports_that_fact() {
        let args = GetDeclarationsArgs {
            query: "x".repeat(MAX_DECLARATION_QUERY_BYTES + 100),
            queries: Vec::new(),
        };

        let batch = declaration_query_batch(&args);
        assert_eq!(batch.processed[0].len(), MAX_DECLARATION_QUERY_BYTES);
        assert_eq!(batch.truncated_query_count, 1);
        let result = render_declaration_query_batch(&batch, &["declaration".to_string()]);
        assert!(result.contains("\"truncated_query_count\":1"));
        assert!(result.contains("\"complete\":false"));
        assert!(result.len() <= MAX_DECLARATION_RESPONSE_BYTES);
    }

    #[test]
    fn declaration_batch_is_bounded_and_keeps_every_query_section() {
        let sections = (0..MAX_DECLARATION_QUERIES)
            .map(|index| format!("// query-{index}\n{}", "x".repeat(10_000)))
            .collect::<Vec<_>>();

        let result = bound_declaration_sections(&sections);

        assert!(result.len() <= MAX_DECLARATION_RESPONSE_BYTES);
        for index in 0..MAX_DECLARATION_QUERIES {
            assert!(result.contains(&format!("// query-{index}")));
        }
        assert!(result.contains("Additional matches omitted"));
        assert!(result.contains("comparison/type-conversion mismatch"));
    }

    #[test]
    fn declaration_batch_truncation_preserves_every_priority_usage_block() {
        let args = GetDeclarationsArgs {
            query: String::new(),
            queries: (0..MAX_DECLARATION_QUERIES)
                .map(|index| {
                    let prefix = format!("capability-{index:02}-");
                    format!(
                        "{prefix}{}",
                        "x".repeat(MAX_DECLARATION_QUERY_BYTES.saturating_sub(prefix.len()))
                    )
                })
                .collect(),
        };
        let batch = declaration_query_batch(&args);
        let priority_prefix = format!(
            "{DECLARATION_PRIORITY_BEGIN}// Exact top catalog signature:\n\
             declare function boolOr({{ boolean?: bool, boolean?: bool }}): bool;\n\
             // boolOr repeats input `boolean` twice; repeat the exact key in declaration order.\n"
        );
        let filler_bytes = MAX_DECLARATION_PRIORITY_BLOCK_BYTES.saturating_sub(
            priority_prefix
                .len()
                .saturating_add("// \n".len())
                .saturating_add(DECLARATION_PRIORITY_END.len()),
        );
        let priority = format!(
            "{priority_prefix}// {}\n{DECLARATION_PRIORITY_END}",
            "p".repeat(filler_bytes)
        );
        assert_eq!(priority.len(), MAX_DECLARATION_PRIORITY_BLOCK_BYTES);
        let sections = batch
            .processed
            .iter()
            .enumerate()
            .map(|(index, query)| {
                declaration_resolution_test_section(
                    query,
                    "resolved",
                    format!(
                        "// declaration query: {query}\n{priority}// query-{index}\n{}",
                        "x".repeat(10_000)
                    ),
                )
            })
            .collect::<Vec<_>>();

        let result = render_declaration_query_batch(&batch, &sections);

        assert!(result.len() <= MAX_DECLARATION_RESPONSE_BYTES);
        assert_eq!(
            result.matches(DECLARATION_PRIORITY_BEGIN).count(),
            MAX_DECLARATION_QUERIES
        );
        assert_eq!(
            result.matches(DECLARATION_PRIORITY_END).count(),
            MAX_DECLARATION_QUERIES
        );
        assert_eq!(
            result
                .matches("repeat the exact key in declaration order")
                .count(),
            MAX_DECLARATION_QUERIES
        );
        assert_eq!(
            result.matches("// declaration query: capability-").count(),
            MAX_DECLARATION_QUERIES
        );
    }
}
