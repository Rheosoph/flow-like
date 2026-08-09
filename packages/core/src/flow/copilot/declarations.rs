//! Fast lookup over generated FlowScript declaration files.
//!
//! FlowPilot uses this as the declaration equivalent of code search: the generated `.flow.d`
//! files are embedded into the binary, split into per-function snippets once, and ranked with a
//! small lexical scorer. This keeps `get_declarations` from depending on fragile catalog-name
//! matches alone.

use std::{
    collections::{BTreeSet, HashMap, HashSet},
    sync::LazyLock,
};

use super::search::{SearchQueryAnalysis, analyze_search_query, tokenize_query_text};

const MAX_DECLARATION_RESULTS: usize = 12;

struct DeclarationFileSource {
    path: &'static str,
    stem: &'static str,
    content: &'static str,
}

#[derive(Debug, Clone)]
pub struct DeclarationMatch {
    pub path: &'static str,
    pub category: String,
    pub function_name: String,
    pub signature_line: String,
    pub summary: String,
    pub impure: bool,
    pub score: i32,
}

/// Query-to-candidate evidence used by the live declaration resolver.
///
/// The embedded declaration index intentionally remains a recall-oriented candidate generator.
/// This second, precision-oriented pass is evaluated against live catalog metadata before a
/// declaration is presented as usable. In particular, operation words such as `register`,
/// `upsert`, or `compare` must occur in the candidate's name/friendly name/capability tags; a
/// coincidental mention in prose is not sufficient to authorize a FlowScript call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DeclarationSemanticEvidence {
    pub exact_symbol: bool,
    pub meaningful_token_count: usize,
    pub matched_token_count: usize,
    pub strong_matched_token_count: usize,
    pub coverage_basis_points: u16,
    pub strong_coverage_basis_points: u16,
    pub missing_strong_anchors: Vec<String>,
    pub reason_codes: Vec<String>,
}

impl DeclarationSemanticEvidence {
    pub(crate) fn accepts(&self) -> bool {
        self.exact_symbol
            || (self.meaningful_token_count > 0
                && self.missing_strong_anchors.is_empty()
                && self.coverage_basis_points >= 6_000
                && self.strong_coverage_basis_points >= 4_000)
    }
}

/// Evaluate the original query against one live declaration candidate without using the broad
/// synonym/recipe expansions that generated the candidate set.
#[allow(clippy::too_many_arguments)]
pub(crate) fn declaration_semantic_evidence(
    query: &str,
    function_name: &str,
    node_type: &str,
    friendly_name: &str,
    description: &str,
    category: Option<&str>,
    capability_tags: &[String],
) -> DeclarationSemanticEvidence {
    let exact_symbol = query
        .split(|character: char| !character.is_ascii_alphanumeric() && character != '_')
        .filter(|token| !token.is_empty())
        .any(|token| {
            token.eq_ignore_ascii_case(function_name) || token.eq_ignore_ascii_case(node_type)
        });

    let query_tokens = semantic_tokens(query)
        .into_iter()
        .filter(|token| !is_semantic_stop_word(token))
        .collect::<Vec<_>>();
    let strong_tokens = semantic_tokens(&format!(
        "{function_name} {node_type} {friendly_name} {}",
        capability_tags.join(" ")
    ))
    .into_iter()
    .collect::<HashSet<_>>();
    let all_tokens = strong_tokens
        .iter()
        .cloned()
        .chain(semantic_tokens(description))
        .chain(semantic_tokens(category.unwrap_or_default()))
        .collect::<HashSet<_>>();

    let matched_token_count = query_tokens
        .iter()
        .filter(|token| all_tokens.contains(*token))
        .count();
    let strong_matched_token_count = query_tokens
        .iter()
        .filter(|token| strong_tokens.contains(*token))
        .count();
    let required_strong_tokens = query_tokens
        .iter()
        .filter(|token| query_tokens.len() == 1 || is_strong_semantic_anchor(token))
        .cloned()
        .collect::<Vec<_>>();
    let missing_strong_anchors = required_strong_tokens
        .iter()
        .filter(|token| !strong_tokens.contains(*token))
        .cloned()
        .collect::<Vec<_>>();
    let denominator = query_tokens.len().max(1);
    let coverage_basis_points = ((matched_token_count * 10_000) / denominator) as u16;
    let strong_coverage_basis_points = ((strong_matched_token_count * 10_000) / denominator) as u16;

    let mut reason_codes = Vec::new();
    if exact_symbol {
        reason_codes.push("exact_function_symbol".to_string());
    }
    if query_tokens.is_empty() {
        reason_codes.push("no_meaningful_query_tokens".to_string());
    }
    if coverage_basis_points >= 6_000 {
        reason_codes.push("query_token_coverage".to_string());
    } else if !query_tokens.is_empty() {
        reason_codes.push("insufficient_query_token_coverage".to_string());
    }
    if strong_coverage_basis_points >= 4_000 {
        reason_codes.push("strong_name_or_tag_evidence".to_string());
    } else if !query_tokens.is_empty() {
        reason_codes.push("insufficient_strong_evidence".to_string());
    }
    for anchor in &missing_strong_anchors {
        reason_codes.push(format!("missing_strong_anchor:{anchor}"));
    }

    DeclarationSemanticEvidence {
        exact_symbol,
        meaningful_token_count: query_tokens.len(),
        matched_token_count,
        strong_matched_token_count,
        coverage_basis_points,
        strong_coverage_basis_points,
        missing_strong_anchors,
        reason_codes,
    }
}

fn semantic_tokens(text: &str) -> Vec<String> {
    tokenize_query_text(text)
        .into_iter()
        .map(|token| canonical_semantic_token(&token))
        .filter(|token| !token.is_empty())
        .collect()
}

fn canonical_semantic_token(token: &str) -> String {
    match token.to_ascii_lowercase().as_str() {
        "bool" | "boolean" | "booleans" => "boolean".to_string(),
        "db" | "database" | "databases" => "database".to_string(),
        "df" | "datafusion" => "datafusion".to_string(),
        "lance" | "lancedb" => "lance".to_string(),
        "mail" | "mails" | "email" | "emails" => "email".to_string(),
        "int" | "integer" | "integers" => "integer".to_string(),
        "notification" | "notifications" | "notify" | "notifies" => "notification".to_string(),
        "chunk" | "chunks" | "chunked" | "chunking" => "chunk".to_string(),
        "compare" | "compares" | "compared" | "comparing" | "comparison" | "comparator" => {
            "compare".to_string()
        }
        "equal" | "equals" | "equality" | "unequal" | "greater" | "less" => "compare".to_string(),
        "register" | "registers" | "registered" | "registering" | "registration" => {
            "register".to_string()
        }
        "create" | "creates" | "created" | "creating" | "creation" => "create".to_string(),
        "upsert" | "upserts" | "upserted" | "upserting" => "upsert".to_string(),
        "search" | "searches" | "searched" | "searching" => "search".to_string(),
        "embed" | "embeds" | "embedded" | "embedding" | "embeddings" => "embed".to_string(),
        "insert" | "inserts" | "inserted" | "inserting" => "insert".to_string(),
        "index" | "indexes" | "indexed" | "indexing" => "index".to_string(),
        "query" | "queries" | "queried" | "querying" => "query".to_string(),
        "fetch" | "fetches" | "fetched" | "fetching" => "fetch".to_string(),
        "connect" | "connects" | "connected" | "connecting" => "connect".to_string(),
        "send" | "sends" | "sent" | "sending" => "send".to_string(),
        other => other.to_string(),
    }
}

fn is_semantic_stop_word(token: &str) -> bool {
    matches!(
        token,
        "a" | "an"
            | "the"
            | "for"
            | "to"
            | "of"
            | "in"
            | "on"
            | "with"
            | "and"
            | "node"
            | "function"
            | "flowscript"
            | "flow"
            | "operation"
            | "capability"
            | "use"
            | "using"
    )
}

fn is_strong_semantic_anchor(token: &str) -> bool {
    matches!(
        token,
        "create"
            | "register"
            | "upsert"
            | "chunk"
            | "compare"
            | "markdown"
            | "notification"
            | "hybrid"
            | "search"
            | "send"
            | "fetch"
            | "connect"
            | "disconnect"
            | "open"
            | "insert"
            | "update"
            | "delete"
            | "index"
            | "embed"
            | "split"
            | "parse"
            | "convert"
            | "clear"
            | "synthesize"
    )
}

#[derive(Debug, Clone)]
struct DeclarationEntry {
    path: &'static str,
    category: String,
    function_name: String,
    signature_line: String,
    summary: String,
    impure: bool,
    haystack: String,
    tokens: BTreeSet<String>,
    function_tokens: Vec<String>,
    function_joined: String,
}

static DECLARATION_INDEX: LazyLock<Vec<DeclarationEntry>> = LazyLock::new(build_declaration_index);

static DECLARATION_FILES: &[DeclarationFileSource] = &[
    DeclarationFileSource {
        path: "ai.flow.d",
        stem: "ai",
        content: include_str!("../../../../ast/flow.d/ai.flow.d"),
    },
    DeclarationFileSource {
        path: "automation.flow.d",
        stem: "automation",
        content: include_str!("../../../../ast/flow.d/automation.flow.d"),
    },
    DeclarationFileSource {
        path: "bit.flow.d",
        stem: "bit",
        content: include_str!("../../../../ast/flow.d/bit.flow.d"),
    },
    DeclarationFileSource {
        path: "control.flow.d",
        stem: "control",
        content: include_str!("../../../../ast/flow.d/control.flow.d"),
    },
    DeclarationFileSource {
        path: "data.flow.d",
        stem: "data",
        content: include_str!("../../../../ast/flow.d/data.flow.d"),
    },
    DeclarationFileSource {
        path: "document.flow.d",
        stem: "document",
        content: include_str!("../../../../ast/flow.d/document.flow.d"),
    },
    DeclarationFileSource {
        path: "email.flow.d",
        stem: "email",
        content: include_str!("../../../../ast/flow.d/email.flow.d"),
    },
    DeclarationFileSource {
        path: "events.flow.d",
        stem: "events",
        content: include_str!("../../../../ast/flow.d/events.flow.d"),
    },
    DeclarationFileSource {
        path: "image.flow.d",
        stem: "image",
        content: include_str!("../../../../ast/flow.d/image.flow.d"),
    },
    DeclarationFileSource {
        path: "logging.flow.d",
        stem: "logging",
        content: include_str!("../../../../ast/flow.d/logging.flow.d"),
    },
    DeclarationFileSource {
        path: "math.flow.d",
        stem: "math",
        content: include_str!("../../../../ast/flow.d/math.flow.d"),
    },
    DeclarationFileSource {
        path: "notifications.flow.d",
        stem: "notifications",
        content: include_str!("../../../../ast/flow.d/notifications.flow.d"),
    },
    DeclarationFileSource {
        path: "processing.flow.d",
        stem: "processing",
        content: include_str!("../../../../ast/flow.d/processing.flow.d"),
    },
    DeclarationFileSource {
        path: "structs.flow.d",
        stem: "structs",
        content: include_str!("../../../../ast/flow.d/structs.flow.d"),
    },
    DeclarationFileSource {
        path: "ui.flow.d",
        stem: "ui",
        content: include_str!("../../../../ast/flow.d/ui.flow.d"),
    },
    DeclarationFileSource {
        path: "utils.flow.d",
        stem: "utils",
        content: include_str!("../../../../ast/flow.d/utils.flow.d"),
    },
    DeclarationFileSource {
        path: "variable.flow.d",
        stem: "variable",
        content: include_str!("../../../../ast/flow.d/variable.flow.d"),
    },
    DeclarationFileSource {
        path: "web.flow.d",
        stem: "web",
        content: include_str!("../../../../ast/flow.d/web.flow.d"),
    },
    DeclarationFileSource {
        path: "packages/automation.flow.d",
        stem: "automation",
        content: include_str!("../../../../ast/flow.d/packages/automation.flow.d"),
    },
    DeclarationFileSource {
        path: "packages/data.flow.d",
        stem: "data",
        content: include_str!("../../../../ast/flow.d/packages/data.flow.d"),
    },
    DeclarationFileSource {
        path: "packages/geo.flow.d",
        stem: "geo",
        content: include_str!("../../../../ast/flow.d/packages/geo.flow.d"),
    },
    DeclarationFileSource {
        path: "packages/llm.flow.d",
        stem: "llm",
        content: include_str!("../../../../ast/flow.d/packages/llm.flow.d"),
    },
    DeclarationFileSource {
        path: "packages/media.flow.d",
        stem: "media",
        content: include_str!("../../../../ast/flow.d/packages/media.flow.d"),
    },
    DeclarationFileSource {
        path: "packages/ml.flow.d",
        stem: "ml",
        content: include_str!("../../../../ast/flow.d/packages/ml.flow.d"),
    },
    DeclarationFileSource {
        path: "packages/onnx.flow.d",
        stem: "onnx",
        content: include_str!("../../../../ast/flow.d/packages/onnx.flow.d"),
    },
    DeclarationFileSource {
        path: "packages/processing.flow.d",
        stem: "processing",
        content: include_str!("../../../../ast/flow.d/packages/processing.flow.d"),
    },
    DeclarationFileSource {
        path: "packages/std.flow.d",
        stem: "std",
        content: include_str!("../../../../ast/flow.d/packages/std.flow.d"),
    },
    DeclarationFileSource {
        path: "packages/web.flow.d",
        stem: "web",
        content: include_str!("../../../../ast/flow.d/packages/web.flow.d"),
    },
];

pub fn declaration_index_summary() -> String {
    let mut domains = DECLARATION_FILES
        .iter()
        .map(|file| file.stem)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    domains.sort_unstable();
    format!(
        "{} declarations across {} embedded .flow.d domains: {}",
        DECLARATION_INDEX.len(),
        domains.len(),
        domains.join(", ")
    )
}

pub fn search_declarations(query: &str) -> Vec<DeclarationMatch> {
    if query.trim().is_empty() {
        return Vec::new();
    }

    let analyses = declaration_query_plan(query);
    let normalized_query = query.trim().to_lowercase();
    // A query that names an exact FlowScript function is a symbol lookup first and a semantic
    // search second. Domain expansions (for example `mail` -> the whole IMAP/SMTP workflow) are
    // still useful companions, but must never displace the declaration the caller explicitly
    // requested.
    let exact_function_names = query
        .split(|character: char| !character.is_ascii_alphanumeric() && character != '_')
        .filter(|token| !token.is_empty())
        .map(str::to_ascii_lowercase)
        .collect::<HashSet<_>>();

    let mut scored = DECLARATION_INDEX
        .iter()
        .filter_map(|entry| {
            // Query-plan entries are alternate interpretations, not independent evidence.
            // Summing them rewards domains that happen to have more canned expansions and can
            // let a generic term such as `table` or `message` outrank the caller's actual query.
            let semantic_score = analyses
                .iter()
                .enumerate()
                .map(|(index, analysis)| {
                    let weight = 100_i32
                        .saturating_sub((index as i32).saturating_mul(12))
                        .max(30);
                    let normalized_joined = analysis.tokens.join("");
                    score_entry(entry, &analysis.expanded_tokens, &normalized_joined) * weight / 100
                })
                .max()
                .unwrap_or_default()
                + workflow_priority_score(entry, &normalized_query, false);
            let exact_symbol_score =
                if exact_function_names.contains(&entry.function_name.to_ascii_lowercase()) {
                    100_000
                } else {
                    Default::default()
                };
            let score = semantic_score.saturating_add(exact_symbol_score);

            (score > 0).then(|| DeclarationMatch {
                path: entry.path,
                category: entry.category.clone(),
                function_name: entry.function_name.clone(),
                signature_line: entry.signature_line.clone(),
                summary: entry.summary.clone(),
                impure: entry.impure,
                score,
            })
        })
        .collect::<Vec<_>>();

    scored.sort_by(|left, right| {
        right
            .score
            .cmp(&left.score)
            .then_with(|| left.function_name.cmp(&right.function_name))
    });
    diversify_declaration_matches(scored, &normalized_query)
}

fn diversify_declaration_matches(
    matches: Vec<DeclarationMatch>,
    normalized_query: &str,
) -> Vec<DeclarationMatch> {
    let mut selected = Vec::with_capacity(MAX_DECLARATION_RESULTS);
    let required_backbone = workflow_backbone_function_names(normalized_query)
        .into_iter()
        .collect::<HashSet<_>>();
    let mut remaining = Vec::new();
    let mut deferred = Vec::new();
    let mut group_counts: HashMap<&'static str, usize> = HashMap::new();

    for matched in matches {
        if required_backbone.contains(matched.function_name.as_str())
            && selected.len() < MAX_DECLARATION_RESULTS
        {
            *group_counts
                .entry(declaration_result_group(&matched.function_name))
                .or_default() += 1;
            selected.push(matched);
        } else {
            remaining.push(matched);
        }
    }

    for matched in remaining {
        let group = declaration_result_group(&matched.function_name);
        let count = group_counts.entry(group).or_default();
        let cap = if group == "other" { 3 } else { 4 };

        if *count < cap && selected.len() < MAX_DECLARATION_RESULTS {
            *count += 1;
            selected.push(matched);
        } else {
            deferred.push(matched);
        }
    }

    for matched in deferred {
        if selected.len() >= MAX_DECLARATION_RESULTS {
            break;
        }
        selected.push(matched);
    }

    selected
}

/// Broad workflow requests need the connecting operations, not merely the individually
/// highest-scoring leaf functions. Reserve the small backbone implied by explicit query intent
/// before applying normal score/group diversification.
fn workflow_backbone_function_names(normalized_query: &str) -> Vec<&'static str> {
    let mut names = Vec::new();
    let wants_email = contains_any(
        normalized_query,
        &["gmail", "email", "mail", "imap", "inbox"],
    );
    let wants_vector_db = normalized_query.contains("vector db")
        || normalized_query.contains("vector database")
        || (normalized_query.contains("vector") && normalized_query.contains("db"));

    if contains_any(normalized_query, &["gmail", "imap", "inbox"]) {
        names.push("emailImapConnect");
    }
    if normalized_query.contains("smtp") {
        names.push("emailSmtpConnect");
    }
    if wants_email && contains_any(normalized_query, &["fetch", "read", "receive"]) {
        names.push("emailImapInboxFetchMail");
    }
    if contains_any(
        normalized_query,
        &["embed", "embedding", "vector", "sentiment", "classif"],
    ) {
        names.push("embedDocument");
    }
    if wants_vector_db
        || contains_any(
            normalized_query,
            &["database", "lance", "lancedb", "local db"],
        )
    {
        names.push("openLocalDb");
    }
    if wants_vector_db
        || contains_any(
            normalized_query,
            &["store", "write", "persist", "insert", "upsert"],
        )
    {
        names.push("batchInsertLocalDb");
    }
    if wants_vector_db || contains_any(normalized_query, &["index", "search"]) {
        names.push("indexLocalDb");
    }

    names
}

fn declaration_result_group(function_name: &str) -> &'static str {
    workflow_priority_group(function_name)
        .map(|(_, group)| group)
        .unwrap_or("other")
}

pub fn render_declaration_matches(query: &str, matches: &[DeclarationMatch]) -> String {
    let query = query.trim();
    if query.is_empty() {
        return format!(
            "// get_declarations needs a concrete query; empty queries no longer return the full declaration bundle.\n// Search is backed by {}.\n// Try focused calls such as:\n// - get_declarations(\"gmail imap fetch mail\")\n// - get_declarations(\"smtp send email\")\n// - get_declarations(\"open local database batch insert\")\n// - get_declarations(\"lancedb vector index hybrid search\")\n// - get_declarations(\"datafusion sql register lance\")\n// - get_declarations(\"struct set cuid timestamp\")",
            declaration_index_summary()
        );
    }

    if matches.is_empty() {
        return format!(
            "// No FlowScript declarations matched {query:?} in the embedded .flow.d index.\n// Search is backed by {}.\n// Try function/domain words such as \"database\", \"email\", \"DataFusion\", \"embedding\", \"vector\", \"full text\", \"hybrid\", \"http\", or \"agent\".",
            declaration_index_summary()
        );
    }

    let mut out = format!(
        "// FlowScript declarations matched {query:?} from the embedded .flow.d index.\n// Showing {} compact signatures. Use these exact camelCase function and argument names in FlowScript.\n\n",
        matches.len()
    );

    for (idx, matched) in matches.iter().enumerate() {
        out.push_str(&format!(
            "{}. {} — {} [{} :: {}, score {}]\n",
            idx + 1,
            matched.function_name,
            matched.summary,
            matched.path,
            matched.category,
            matched.score
        ));
        out.push_str("   ");
        out.push_str(&compact_signature_line(matched));
        out.push_str("\n\n");
    }

    out
}

fn declaration_query_plan(query: &str) -> Vec<SearchQueryAnalysis> {
    let normalized = query.trim().to_lowercase();
    let mut queries = Vec::new();

    if !normalized.is_empty() {
        queries.push(query.to_string());
    }

    if normalized.is_empty()
        || normalized.contains("gmail")
        || normalized.contains("email")
        || normalized.contains("mail")
        || normalized.contains("imap")
        || normalized.contains("smtp")
    {
        queries.extend(
            [
                "imap connect",
                "list inboxes",
                "list mail filter",
                "fetch mail message",
                "smtp connect",
                "smtp send email",
            ]
            .into_iter()
            .map(ToString::to_string),
        );
    }

    if normalized.is_empty()
        || normalized.contains("database")
        || normalized.contains("db")
        || normalized.contains("lance")
        || normalized.contains("vector")
        || normalized.contains("table")
        || normalized.contains("store")
        || normalized.contains("insert")
    {
        queries.extend(
            [
                "open local database",
                "batch insert local database",
                "batch upsert local database",
                "index local database vector full text",
                "hybrid search local database",
                "vector search local database",
                "schema local database",
            ]
            .into_iter()
            .map(ToString::to_string),
        );
    }

    if normalized.is_empty()
        || normalized.contains("embed")
        || normalized.contains("embedding")
        || normalized.contains("vector")
        || normalized.contains("sentiment")
        || normalized.contains("classif")
    {
        queries.extend(
            [
                "embed document",
                "embed query",
                "load embedding model",
                "split text chunks embedding",
                "classification model label sentiment",
            ]
            .into_iter()
            .map(ToString::to_string),
        );
    }

    if normalized.is_empty()
        || normalized.contains("datafusion")
        || normalized.contains("sql")
        || normalized.contains("big data")
        || normalized.contains("analytics")
    {
        queries.extend(
            [
                "create datafusion session",
                "register lancedb datafusion table",
                "execute datafusion sql",
                "list datafusion tables",
                "datafusion schema",
            ]
            .into_iter()
            .map(ToString::to_string),
        );
    }

    if normalized.is_empty()
        || normalized.contains("json")
        || normalized.contains("metadata")
        || normalized.contains("cuid")
        || normalized.contains("timestamp")
        || normalized.contains("sender")
        || normalized.contains("subject")
    {
        queries.extend(
            [
                "struct set field",
                "struct get field",
                "make struct",
                "current date timestamp",
                "cuid unique id",
            ]
            .into_iter()
            .map(ToString::to_string),
        );
    }

    let mut seen = HashSet::new();
    queries
        .into_iter()
        .filter(|query| seen.insert(query.to_lowercase()))
        .map(|query| analyze_declaration_search_query(&query))
        .filter(|analysis| !analysis.tokens.is_empty())
        .collect()
}

fn analyze_declaration_search_query(query: &str) -> SearchQueryAnalysis {
    let mut analysis = analyze_search_query(query);
    let has_storage_intent = analysis.tokens.iter().any(|token| {
        matches!(
            token.as_str(),
            "put"
                | "store"
                | "write"
                | "persist"
                | "db"
                | "database"
                | "lance"
                | "lancedb"
                | "vector"
                | "table"
                | "row"
                | "record"
                | "batch"
                | "insert"
                | "upsert"
        )
    });

    if !has_storage_intent {
        let query_tokens = analysis.tokens.iter().cloned().collect::<HashSet<_>>();
        analysis.expanded_tokens.retain(|token| {
            query_tokens.contains(token)
                || !matches!(
                    token.as_str(),
                    "open"
                        | "local"
                        | "db"
                        | "database"
                        | "table"
                        | "row"
                        | "record"
                        | "batch"
                        | "insert"
                        | "upsert"
                )
        });
    }

    analysis
}

fn score_entry(entry: &DeclarationEntry, query_tokens: &[String], normalized_joined: &str) -> i32 {
    let function_lower = entry.function_name.to_lowercase();
    let category_lower = entry.category.to_lowercase();

    let mut score = 0i32;
    if !normalized_joined.is_empty() {
        if entry.function_joined == normalized_joined || function_lower == normalized_joined {
            score += 1000;
        } else if entry.function_joined.contains(normalized_joined)
            || normalized_joined.contains(&entry.function_joined)
        {
            score += 450;
        }
        if entry.haystack.contains(normalized_joined) {
            score += 160;
        }
        let joined_similarity = strsim::jaro_winkler(&entry.function_joined, normalized_joined);
        if joined_similarity >= 0.92 {
            score += 360;
        } else if joined_similarity >= 0.86 {
            score += 160;
        }
    }

    for token in query_tokens {
        if token.len() <= 1 {
            continue;
        }
        if entry.tokens.contains(token) {
            score += 80;
        }
        if function_lower.contains(token) {
            score += 70;
        }
        if category_lower.contains(token) {
            score += 35;
        }
        if entry.signature_line.to_lowercase().contains(token) {
            score += 25;
        }
        if entry.haystack.contains(token) {
            score += 8;
        }
        if entry
            .function_tokens
            .iter()
            .any(|candidate| candidate.len() > 3 && strsim::jaro_winkler(candidate, token) >= 0.9)
        {
            score += 35;
        }
    }

    if query_tokens.iter().any(|token| token == "batch")
        && (function_lower.contains("batch") || function_lower.contains("upsert"))
    {
        score += 45;
    }
    if query_tokens.iter().any(|token| token == "open")
        && (function_lower.contains("open") || function_lower.contains("connect"))
    {
        score += 45;
    }
    if query_tokens.iter().any(|token| token == "send")
        && (function_lower.contains("send") || function_lower.contains("smtp"))
    {
        score += 45;
    }
    if query_tokens
        .iter()
        .any(|token| token == "read" || token == "fetch")
        && (function_lower.contains("fetch") || function_lower.contains("imap"))
    {
        score += 45;
    }

    score
}

fn workflow_priority_score(
    entry: &DeclarationEntry,
    normalized_query: &str,
    include_defaults: bool,
) -> i32 {
    let name = entry.function_name.as_str();
    let Some((base, group)) = workflow_priority_group(name) else {
        return 0;
    };

    if include_defaults {
        return base;
    }

    let wants_vector_db_workflow = normalized_query.contains("vector db")
        || normalized_query.contains("vector database")
        || (normalized_query.contains("vector") && normalized_query.contains("db"));
    let wants_store_workflow = wants_vector_db_workflow
        || contains_any(
            normalized_query,
            &[
                "store", "write", "persist", "put", "insert", "upsert", "save",
            ],
        );
    let wants_index_workflow = wants_vector_db_workflow
        || contains_any(normalized_query, &["build index", "create index", "index"]);

    let group_matches_query = match group {
        "email" => contains_any(
            normalized_query,
            &["gmail", "email", "mail", "imap", "smtp", "inbox"],
        ),
        "embedding" => contains_any(
            normalized_query,
            &[
                "embed",
                "embedding",
                "vector",
                "sentiment",
                "classif",
                "chunk",
                "model",
            ],
        ),
        "database" => contains_any(
            normalized_query,
            &[
                "database", "db", "lance", "vector", "table", "store", "insert", "upsert", "index",
                "search",
            ],
        ),
        "datafusion" => contains_any(
            normalized_query,
            &["datafusion", "sql", "analytics", "big data", "query"],
        ),
        "struct" => contains_any(
            normalized_query,
            &[
                "json",
                "metadata",
                "cuid",
                "timestamp",
                "sender",
                "subject",
                "field",
            ],
        ),
        _ => false,
    };

    if group_matches_query {
        let mut score = base;
        if name == "emailImapInboxFetchMail"
            && contains_any(normalized_query, &["fetch", "mail", "message", "email"])
        {
            score += 360;
        }
        if name == "mailImapList"
            && contains_any(
                normalized_query,
                &["fetch", "mail", "message", "email", "inbox"],
            )
        {
            score += 180;
        }
        if name == "emailSmtpConnect"
            && contains_any(normalized_query, &["smtp", "send", "mail", "email"])
        {
            score += 420;
        }
        if name == "emailSmtpSend" && contains_any(normalized_query, &["smtp", "send"]) {
            score += 360;
        }
        if name == "mailImapListInboxes" && !normalized_query.contains("inboxes") {
            score = score.saturating_sub(220);
        }
        if matches!(
            name,
            "batchInsertLocalDb" | "batchUpsertLocalDb" | "insertLocalDb"
        ) && wants_store_workflow
        {
            score += 320;
        }
        if name == "indexLocalDb" && wants_index_workflow {
            score += 1_150;
        }
        if matches!(
            name,
            "hybridSearchLocalDb" | "vectorSearchLocalDb" | "ftsSearchLocalDb"
        ) && !contains_any(normalized_query, &["search", "query", "retrieve", "find"])
        {
            score = score.saturating_sub(260);
        }
        score
    } else {
        0
    }
}

fn contains_any(haystack: &str, needles: &[&str]) -> bool {
    needles.iter().any(|needle| haystack.contains(needle))
}

fn workflow_priority_group(function_name: &str) -> Option<(i32, &'static str)> {
    match function_name {
        "emailImapConnect" => Some((120, "email")),
        "mailImapListInboxes" => Some((80, "email")),
        "mailImapList" => Some((95, "email")),
        "emailImapInboxFetchMail" => Some((110, "email")),
        "emailGetHeaders" => Some((70, "email")),
        "emailSmtpConnect" => Some((100, "email")),
        "emailSmtpSend" => Some((105, "email")),
        "embedDocument" => Some((115, "embedding")),
        "embedQuery" => Some((90, "embedding")),
        "splitText" | "chunkText" => Some((70, "embedding")),
        "openLocalDb" => Some((120, "database")),
        "batchInsertLocalDb" => Some((110, "database")),
        "batchUpsertLocalDb" => Some((105, "database")),
        "insertLocalDb" => Some((80, "database")),
        "indexLocalDb" => Some((115, "database")),
        "hybridSearchLocalDb" => Some((105, "database")),
        "vectorSearchLocalDb" => Some((95, "database")),
        "ftsSearchLocalDb" => Some((90, "database")),
        "dfCreateSession" => Some((95, "datafusion")),
        "dfRegisterLance" => Some((85, "datafusion")),
        "dfSqlQuery" => Some((90, "datafusion")),
        "dfExecuteSql" => Some((95, "datafusion")),
        "dfListTables" => Some((65, "datafusion")),
        "structSet" => Some((100, "struct")),
        "structGet" => Some((70, "struct")),
        "structMake" => Some((60, "struct")),
        "cUIDV2" | "cuid" => Some((75, "struct")),
        "now" => Some((70, "struct")),
        _ => None,
    }
}

fn build_declaration_index() -> Vec<DeclarationEntry> {
    let mut seen_signatures = HashSet::new();
    let mut entries = Vec::new();

    for file in DECLARATION_FILES {
        for entry in parse_declaration_file(file) {
            if seen_signatures.insert(entry.signature_line.clone()) {
                entries.push(entry);
            }
        }
    }

    entries
}

fn parse_declaration_file(file: &DeclarationFileSource) -> Vec<DeclarationEntry> {
    let mut entries = Vec::new();
    let mut search_from = 0usize;
    let needle = "declare function ";

    while let Some(relative_idx) = file.content[search_from..].find(needle) {
        let declare_start = search_from + relative_idx;
        let Some(declare_end) = declaration_end(file.content, declare_start) else {
            break;
        };

        let comment_start = file.content[search_from..declare_start]
            .rfind("/**")
            .map(|idx| search_from + idx)
            .unwrap_or(declare_start);
        let declaration = file.content[comment_start..declare_end].trim().to_string();
        let signature_line = file.content[declare_start..declare_end].trim().to_string();
        let function_name = parse_function_name(&signature_line);
        let summary = declaration_summary(&declaration).unwrap_or_else(|| function_name.clone());
        let impure = declaration.contains("@impure");
        let category = find_category(file.content, declare_start).unwrap_or_else(|| {
            file.stem
                .split('_')
                .map(capitalize)
                .collect::<Vec<_>>()
                .join("/")
        });
        let haystack = format!(
            "{} {} {} {}",
            function_name, signature_line, declaration, category
        )
        .to_lowercase();
        let function_tokens = tokenize_query_text(&function_name);
        let function_joined = function_tokens.join("");
        let tokens = tokenize_query_text(&haystack).into_iter().collect();

        entries.push(DeclarationEntry {
            path: file.path,
            category,
            function_name,
            signature_line,
            summary,
            impure,
            haystack,
            tokens,
            function_tokens,
            function_joined,
        });

        search_from = declare_end;
    }

    entries
}

fn compact_signature_line(matched: &DeclarationMatch) -> String {
    if matched.impure {
        format!("{}  // impure", matched.signature_line)
    } else {
        matched.signature_line.clone()
    }
}

fn declaration_summary(declaration: &str) -> Option<String> {
    for line in declaration.lines() {
        let line = line
            .trim()
            .trim_start_matches("/**")
            .trim_start_matches('*')
            .trim_end_matches("*/")
            .trim();

        if line.is_empty() || line.starts_with('@') {
            continue;
        }

        return Some(truncate_summary(line));
    }

    None
}

fn truncate_summary(summary: &str) -> String {
    const MAX_SUMMARY_CHARS: usize = 120;
    let mut out = String::with_capacity(summary.len().min(MAX_SUMMARY_CHARS));
    for (idx, ch) in summary.chars().enumerate() {
        if idx >= MAX_SUMMARY_CHARS {
            out.push_str("...");
            return out;
        }
        out.push(ch);
    }
    out
}

fn declaration_end(content: &str, declare_start: usize) -> Option<usize> {
    content[declare_start..]
        .find(";\n")
        .map(|idx| declare_start + idx + 1)
        .or_else(|| {
            content[declare_start..]
                .find(';')
                .map(|idx| declare_start + idx + 1)
        })
}

fn parse_function_name(signature_line: &str) -> String {
    signature_line
        .strip_prefix("declare function ")
        .and_then(|rest| rest.split_once('(').map(|(name, _)| name.trim()))
        .filter(|name| !name.is_empty())
        .unwrap_or("<unknown>")
        .to_string()
}

fn find_category(content: &str, declare_start: usize) -> Option<String> {
    let prefix = &content[..declare_start];
    let marker_start = prefix.rfind("// === ")?;
    let section = prefix[marker_start..].lines().next()?;
    section
        .strip_prefix("// === ")
        .and_then(|line| line.strip_suffix(" ==="))
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(ToString::to_string)
}

fn capitalize(value: &str) -> String {
    let mut chars = value.chars();
    match chars.next() {
        Some(first) => format!("{}{}", first.to_uppercase(), chars.as_str()),
        None => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(query: &str) -> Vec<String> {
        search_declarations(query)
            .into_iter()
            .map(|matched| matched.function_name)
            .collect()
    }

    #[test]
    fn finds_email_declarations_from_camelish_query() {
        let names = names("imapConnect");
        assert!(names.iter().any(|name| name == "emailImapConnect"));
    }

    #[test]
    fn finds_database_and_search_declarations() {
        let open_names = names("open database");
        assert!(open_names.iter().any(|name| name == "openLocalDb"));

        let hybrid_names = names("hybrid vector search");
        assert!(
            hybrid_names
                .iter()
                .any(|name| name == "hybridSearchLocalDb")
        );
    }

    #[test]
    fn fuzzy_missing_batch_embedding_still_finds_document_embedding() {
        let names = names("batchEmbedDocument");
        assert!(names.iter().any(|name| name == "embedDocument"));
    }

    #[test]
    fn exact_function_symbol_outranks_workflow_expansion() {
        let matches = search_declarations("mailAddressFields address struct input schema");
        assert_eq!(
            matches
                .first()
                .map(|matched| matched.function_name.as_str()),
            Some("mailAddressFields")
        );

        let matches = search_declarations("cuid generate id");
        assert_eq!(
            matches
                .first()
                .map(|matched| matched.function_name.as_str()),
            Some("cuid")
        );
    }

    #[test]
    fn generic_terms_do_not_promote_unrelated_workflow_declarations() {
        for (query, unrelated) in [
            (
                "chat send response message citation source attachment file",
                "emailSmtpSend",
            ),
            ("a2ui write CSV to table", "openLocalDb"),
            ("agent tool structured output schema", "batchInsertLocalDb"),
        ] {
            let matches = search_declarations(query);
            assert_ne!(
                matches
                    .first()
                    .map(|matched| matched.function_name.as_str()),
                Some(unrelated),
                "{query:?} incorrectly preferred {unrelated}; matches: {matches:?}"
            );
        }
    }

    #[test]
    fn semantic_evidence_requires_operation_words_on_the_candidate_surface() {
        let tags = Vec::new();
        let evidence = declaration_semantic_evidence(
            "integer compare",
            "fakerInteger",
            "faker_integer",
            "Faker Integer",
            "Generates integer values for comparison tests.",
            Some("utils/faker"),
            &tags,
        );

        assert!(!evidence.accepts());
        assert_eq!(evidence.coverage_basis_points, 10_000);
        assert!(
            evidence
                .reason_codes
                .contains(&"missing_strong_anchor:compare".to_string())
        );
    }

    #[test]
    fn semantic_evidence_accepts_a_unique_exact_function_symbol() {
        let evidence = declaration_semantic_evidence(
            "Use hybridSearchLocalDb with these pins",
            "hybridSearchLocalDb",
            "hybrid_search_local_db",
            "Hybrid Search Local DB",
            "Runs hybrid vector and full text search.",
            Some("data/search"),
            &[],
        );

        assert!(evidence.exact_symbol);
        assert!(evidence.accepts());
        assert!(
            evidence
                .reason_codes
                .contains(&"exact_function_symbol".to_string())
        );
    }

    #[test]
    fn blank_query_renders_actionable_hint() {
        let matches = search_declarations("");
        assert!(matches.is_empty());

        let rendered = render_declaration_matches("", &[]);
        assert!(rendered.contains("needs a concrete query"));
        assert!(rendered.contains("gmail imap"));
        assert!(rendered.len() < 1_200);
    }

    #[test]
    fn focused_query_renders_compact_declarations() {
        let matches = search_declarations("gmail imap fetch mail");
        let rendered = render_declaration_matches("gmail imap fetch mail", &matches);

        assert!(rendered.contains("declare function"));
        assert!(!rendered.contains("@param"));
        assert!(!rendered.contains("/**"));
        assert!(
            rendered.len() < 8_000,
            "declaration output should stay compact, got {} bytes",
            rendered.len()
        );
    }

    #[test]
    fn broad_email_vector_workflow_query_finds_core_nodes() {
        let names = names(
            "gmail imap smtp fetch mails last 2 days embed sentiment vector db cuids timestamp sender",
        );
        for expected in [
            "emailImapConnect",
            "emailSmtpConnect",
            "emailImapInboxFetchMail",
            "embedDocument",
            "openLocalDb",
            "batchInsertLocalDb",
            "indexLocalDb",
        ] {
            assert!(
                names.iter().any(|name| name == expected),
                "missing {expected} in {names:?}"
            );
        }
    }
}
