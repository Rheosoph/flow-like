//! Fast lookup over generated FlowScript declaration files.
//!
//! FlowPilot uses this as the declaration equivalent of code search: the generated `.flow.d`
//! files are embedded into the binary, split into per-function snippets once, and ranked with a
//! small lexical scorer. This keeps `get_declarations` from depending on fragile catalog-name
//! matches alone.
//!
//! The index reads the namespaced v2 format (`declare namespace ns { function alias(this: T,
//! { … }): R; }` with `@node` / `@receiver` / `@alias` tags) and the legacy flat
//! `declare function flat(…)` lines. Every entry is addressable by all of its accepted
//! spellings: the qualified `ns::alias`, the legacy camelCase name and the catalog node type.

use std::{
    collections::{BTreeSet, HashMap, HashSet},
    sync::LazyLock,
};

use flow_like_ast::{is_signature_line, to_camel_case};

use super::search::{SearchQueryAnalysis, analyze_search_query, tokenize_query_text};

const MAX_DECLARATION_RESULTS: usize = 12;
const EXACT_SYMBOL_SCORE: i32 = 100_000;

struct DeclarationFileSource {
    path: &'static str,
    stem: &'static str,
    content: &'static str,
}

#[derive(Debug, Clone)]
pub struct DeclarationMatch {
    pub path: &'static str,
    pub category: String,
    /// The qualified call spelling (`string::contains`), or the legacy flat name for a
    /// declaration without a namespace.
    pub function_name: String,
    /// Catalog node type (`string_contains`).
    pub node_type: String,
    /// `::`-joined namespace path (`email::imap`); empty for a legacy flat declaration.
    pub namespace: String,
    /// Member name inside the namespace (`contains`).
    pub alias: String,
    /// Legacy camelCase spelling (`stringContains`), accepted forever.
    pub flat: String,
    /// camelCase name of the input pin bound by `this` in method form (`x.alias(...)`).
    pub receiver: Option<String>,
    pub signature_line: String,
    pub summary: String,
    pub impure: bool,
    pub score: i32,
}

impl DeclarationMatch {
    /// How to spell a call to this declaration in FlowScript: the qualified static form and, for
    /// nodes with a receiver, the method form.
    pub fn call_hint(&self) -> String {
        let params = self
            .signature_line
            .split_once('(')
            .and_then(|(_, rest)| rest.rsplit_once(')'))
            .map(|(params, _)| params.trim())
            .unwrap_or_default();
        let object = params
            .find('{')
            .and_then(|start| params.rfind('}').map(|end| &params[start..=end]));
        let pins: Vec<String> = object
            .map(|object| {
                object
                    .trim_matches(|c| c == '{' || c == '}')
                    .split(',')
                    .filter_map(|part| {
                        part.split_once(':')
                            .map(|(name, _)| name.trim().trim_end_matches('?').to_string())
                    })
                    .filter(|name| !name.is_empty())
                    .collect()
            })
            .unwrap_or_default();
        let static_args = pins
            .iter()
            .map(|pin| format!("{pin}: {pin}"))
            .collect::<Vec<_>>()
            .join(", ");
        let static_form = if static_args.is_empty() {
            format!("{}()", self.function_name)
        } else {
            format!("{}({{ {static_args} }})", self.function_name)
        };
        let Some(receiver) = self.receiver.as_deref() else {
            return static_form;
        };
        let rest = pins
            .iter()
            .filter(|pin| pin.as_str() != receiver)
            .map(|pin| format!("{pin}: {pin}"))
            .collect::<Vec<_>>()
            .join(", ");
        let method_form = if rest.is_empty() {
            format!("{receiver}.{}()", self.alias)
        } else {
            format!("{receiver}.{}({{ {rest} }})", self.alias)
        };
        format!("{static_form}  or  {method_form}")
    }
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
    let flat = to_camel_case(node_type);
    let exact_symbol = symbol_tokens(query).any(|token| {
        token.eq_ignore_ascii_case(function_name)
            || token.eq_ignore_ascii_case(node_type)
            || token.eq_ignore_ascii_case(&flat)
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
    node_type: String,
    namespace: String,
    alias: String,
    flat: String,
    receiver: Option<String>,
    signature_line: String,
    summary: String,
    impure: bool,
    haystack: String,
    tokens: BTreeSet<String>,
    function_tokens: Vec<String>,
    function_joined: String,
}

impl DeclarationEntry {
    /// Every spelling that names this declaration exactly, lower-cased.
    fn exact_spellings(&self) -> impl Iterator<Item = String> + '_ {
        [
            self.function_name.as_str(),
            self.node_type.as_str(),
            self.flat.as_str(),
        ]
        .into_iter()
        .map(str::to_ascii_lowercase)
    }

    fn to_match(&self, score: i32) -> DeclarationMatch {
        DeclarationMatch {
            path: self.path,
            category: self.category.clone(),
            function_name: self.function_name.clone(),
            node_type: self.node_type.clone(),
            namespace: self.namespace.clone(),
            alias: self.alias.clone(),
            flat: self.flat.clone(),
            receiver: self.receiver.clone(),
            signature_line: self.signature_line.clone(),
            summary: self.summary.clone(),
            impure: self.impure,
            score,
        }
    }
}

static DECLARATION_INDEX: LazyLock<Vec<DeclarationEntry>> = LazyLock::new(build_declaration_index);

/// Every generated `.flow.d` file, embedded. `embeds_every_generated_declaration_file` fails
/// when the generator writes a file that is missing here.
static DECLARATION_FILES: &[DeclarationFileSource] = &[
    DeclarationFileSource {
        path: "ai.flow.d",
        stem: "ai",
        content: include_str!("../../../../../ast/flow.d/ai.flow.d"),
    },
    DeclarationFileSource {
        path: "audio.flow.d",
        stem: "audio",
        content: include_str!("../../../../../ast/flow.d/audio.flow.d"),
    },
    DeclarationFileSource {
        path: "automation.flow.d",
        stem: "automation",
        content: include_str!("../../../../../ast/flow.d/automation.flow.d"),
    },
    DeclarationFileSource {
        path: "bit.flow.d",
        stem: "bit",
        content: include_str!("../../../../../ast/flow.d/bit.flow.d"),
    },
    DeclarationFileSource {
        path: "control.flow.d",
        stem: "control",
        content: include_str!("../../../../../ast/flow.d/control.flow.d"),
    },
    DeclarationFileSource {
        path: "data-studio.flow.d",
        stem: "data-studio",
        content: include_str!("../../../../../ast/flow.d/data-studio.flow.d"),
    },
    DeclarationFileSource {
        path: "data.flow.d",
        stem: "data",
        content: include_str!("../../../../../ast/flow.d/data.flow.d"),
    },
    DeclarationFileSource {
        path: "diagnostics.flow.d",
        stem: "diagnostics",
        content: include_str!("../../../../../ast/flow.d/diagnostics.flow.d"),
    },
    DeclarationFileSource {
        path: "document.flow.d",
        stem: "document",
        content: include_str!("../../../../../ast/flow.d/document.flow.d"),
    },
    DeclarationFileSource {
        path: "email.flow.d",
        stem: "email",
        content: include_str!("../../../../../ast/flow.d/email.flow.d"),
    },
    DeclarationFileSource {
        path: "events.flow.d",
        stem: "events",
        content: include_str!("../../../../../ast/flow.d/events.flow.d"),
    },
    DeclarationFileSource {
        path: "image.flow.d",
        stem: "image",
        content: include_str!("../../../../../ast/flow.d/image.flow.d"),
    },
    DeclarationFileSource {
        path: "index.flow.d",
        stem: "index",
        content: include_str!("../../../../../ast/flow.d/index.flow.d"),
    },
    DeclarationFileSource {
        path: "logging.flow.d",
        stem: "logging",
        content: include_str!("../../../../../ast/flow.d/logging.flow.d"),
    },
    DeclarationFileSource {
        path: "math.flow.d",
        stem: "math",
        content: include_str!("../../../../../ast/flow.d/math.flow.d"),
    },
    DeclarationFileSource {
        path: "notifications.flow.d",
        stem: "notifications",
        content: include_str!("../../../../../ast/flow.d/notifications.flow.d"),
    },
    DeclarationFileSource {
        path: "processing.flow.d",
        stem: "processing",
        content: include_str!("../../../../../ast/flow.d/processing.flow.d"),
    },
    DeclarationFileSource {
        path: "streaming.flow.d",
        stem: "streaming",
        content: include_str!("../../../../../ast/flow.d/streaming.flow.d"),
    },
    DeclarationFileSource {
        path: "structs.flow.d",
        stem: "structs",
        content: include_str!("../../../../../ast/flow.d/structs.flow.d"),
    },
    DeclarationFileSource {
        path: "subtitles.flow.d",
        stem: "subtitles",
        content: include_str!("../../../../../ast/flow.d/subtitles.flow.d"),
    },
    DeclarationFileSource {
        path: "ui.flow.d",
        stem: "ui",
        content: include_str!("../../../../../ast/flow.d/ui.flow.d"),
    },
    DeclarationFileSource {
        path: "utils.flow.d",
        stem: "utils",
        content: include_str!("../../../../../ast/flow.d/utils.flow.d"),
    },
    DeclarationFileSource {
        path: "variable.flow.d",
        stem: "variable",
        content: include_str!("../../../../../ast/flow.d/variable.flow.d"),
    },
    DeclarationFileSource {
        path: "video.flow.d",
        stem: "video",
        content: include_str!("../../../../../ast/flow.d/video.flow.d"),
    },
    DeclarationFileSource {
        path: "web.flow.d",
        stem: "web",
        content: include_str!("../../../../../ast/flow.d/web.flow.d"),
    },
    DeclarationFileSource {
        path: "packages/automation.flow.d",
        stem: "automation",
        content: include_str!("../../../../../ast/flow.d/packages/automation.flow.d"),
    },
    DeclarationFileSource {
        path: "packages/data.flow.d",
        stem: "data",
        content: include_str!("../../../../../ast/flow.d/packages/data.flow.d"),
    },
    DeclarationFileSource {
        path: "packages/geo.flow.d",
        stem: "geo",
        content: include_str!("../../../../../ast/flow.d/packages/geo.flow.d"),
    },
    DeclarationFileSource {
        path: "packages/index.flow.d",
        stem: "index",
        content: include_str!("../../../../../ast/flow.d/packages/index.flow.d"),
    },
    DeclarationFileSource {
        path: "packages/llm.flow.d",
        stem: "llm",
        content: include_str!("../../../../../ast/flow.d/packages/llm.flow.d"),
    },
    DeclarationFileSource {
        path: "packages/media.flow.d",
        stem: "media",
        content: include_str!("../../../../../ast/flow.d/packages/media.flow.d"),
    },
    DeclarationFileSource {
        path: "packages/ml.flow.d",
        stem: "ml",
        content: include_str!("../../../../../ast/flow.d/packages/ml.flow.d"),
    },
    DeclarationFileSource {
        path: "packages/onnx.flow.d",
        stem: "onnx",
        content: include_str!("../../../../../ast/flow.d/packages/onnx.flow.d"),
    },
    DeclarationFileSource {
        path: "packages/processing.flow.d",
        stem: "processing",
        content: include_str!("../../../../../ast/flow.d/packages/processing.flow.d"),
    },
    DeclarationFileSource {
        path: "packages/std.flow.d",
        stem: "std",
        content: include_str!("../../../../../ast/flow.d/packages/std.flow.d"),
    },
    DeclarationFileSource {
        path: "packages/web.flow.d",
        stem: "web",
        content: include_str!("../../../../../ast/flow.d/packages/web.flow.d"),
    },
];

pub fn declaration_index_summary() -> String {
    let domains = DECLARATION_INDEX
        .iter()
        .map(|entry| entry.path.rsplit('/').next().unwrap_or(entry.path))
        .map(|file| file.trim_end_matches(".flow.d"))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    format!(
        "{} declarations across {} embedded .flow.d domains: {}",
        DECLARATION_INDEX.len(),
        domains.len(),
        domains.join(", ")
    )
}

/// Every accepted spelling of an indexed declaration, lower-cased, mapped to its node type:
/// the qualified `ns::alias`, the legacy flat name, the node type itself and — when unique across
/// the index and visibly a compound symbol — the bare alias.
static DECLARATION_NAME_MAP: LazyLock<HashMap<String, String>> = LazyLock::new(|| {
    let mut map: HashMap<String, String> = HashMap::new();
    let mut alias_owners: HashMap<String, BTreeSet<String>> = HashMap::new();
    for entry in DECLARATION_INDEX.iter() {
        for spelling in entry.exact_spellings() {
            map.entry(spelling)
                .or_insert_with(|| entry.node_type.clone());
        }
        alias_owners
            .entry(entry.alias.to_ascii_lowercase())
            .or_default()
            .insert(entry.node_type.clone());
    }
    for entry in DECLARATION_INDEX.iter() {
        let alias = entry.alias.to_ascii_lowercase();
        let unique = alias_owners
            .get(&alias)
            .is_some_and(|owners| owners.len() == 1);
        let compound = entry.alias.chars().any(|c| c.is_ascii_uppercase());
        if unique && compound {
            map.entry(alias).or_insert_with(|| entry.node_type.clone());
        }
    }
    map
});

/// Resolve any accepted spelling (`string::contains`, `stringContains`, `string_contains`, a
/// unique compound alias) to its catalog node type. Unknown spellings pass through unchanged so
/// callers can keep matching on them.
pub(crate) fn canonical_node_type(spelling: &str) -> String {
    let key = spelling.trim().to_ascii_lowercase();
    DECLARATION_NAME_MAP
        .get(&key)
        .cloned()
        .unwrap_or_else(|| key.replace("::", "_"))
}

/// Split a query into symbol-shaped tokens, keeping `::` so a qualified `ns::alias` survives as
/// one token.
fn symbol_tokens(query: &str) -> impl Iterator<Item = &str> {
    query
        .split(|character: char| {
            !character.is_ascii_alphanumeric() && character != '_' && character != ':'
        })
        .map(|token| token.trim_matches(':'))
        .filter(|token| !token.is_empty())
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
    // requested. Any accepted spelling counts: `string::contains`, `stringContains`,
    // `string_contains` or a unique compound alias.
    let exact_node_types = symbol_tokens(query)
        .filter_map(|token| DECLARATION_NAME_MAP.get(&token.to_ascii_lowercase()))
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
            let exact_symbol_score = if exact_node_types.contains(&entry.node_type) {
                EXACT_SYMBOL_SCORE
            } else {
                Default::default()
            };
            let score = semantic_score.saturating_add(exact_symbol_score);

            (score > 0).then(|| entry.to_match(score))
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
    let required_backbone = workflow_backbone_node_types(normalized_query)
        .into_iter()
        .collect::<HashSet<_>>();
    let mut remaining = Vec::new();
    let mut deferred = Vec::new();
    let mut group_counts: HashMap<&'static str, usize> = HashMap::new();

    // An explicitly named symbol is the answer to the question; it goes ahead of the backbone
    // the query's domain words imply.
    for matched in matches {
        let exact_symbol = matched.score >= EXACT_SYMBOL_SCORE;
        if (exact_symbol || required_backbone.contains(matched.node_type.as_str()))
            && selected.len() < MAX_DECLARATION_RESULTS
        {
            *group_counts
                .entry(declaration_result_group(&matched.node_type))
                .or_default() += 1;
            selected.push(matched);
        } else {
            remaining.push(matched);
        }
    }

    for matched in remaining {
        let group = declaration_result_group(&matched.node_type);
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
/// highest-scoring leaf functions. Reserve the small backbone (by node type) implied by explicit
/// query intent before applying normal score/group diversification.
fn workflow_backbone_node_types(normalized_query: &str) -> Vec<&'static str> {
    let mut names = Vec::new();
    let wants_email = contains_any(
        normalized_query,
        &["gmail", "email", "mail", "imap", "inbox"],
    );
    let wants_vector_db = normalized_query.contains("vector db")
        || normalized_query.contains("vector database")
        || (normalized_query.contains("vector") && normalized_query.contains("db"));

    if contains_any(normalized_query, &["gmail", "imap", "inbox"]) {
        names.push("email_imap_connect");
    }
    if normalized_query.contains("smtp") {
        names.push("email_smtp_connect");
    }
    if wants_email && contains_any(normalized_query, &["fetch", "read", "receive"]) {
        names.push("email_imap_inbox_fetch_mail");
    }
    if contains_any(
        normalized_query,
        &["embed", "embedding", "vector", "sentiment", "classif"],
    ) {
        names.push("embed_document");
    }
    if wants_vector_db
        || contains_any(
            normalized_query,
            &["database", "lance", "lancedb", "local db"],
        )
    {
        names.push("open_local_db");
    }
    if wants_vector_db
        || contains_any(
            normalized_query,
            &["store", "write", "persist", "insert", "upsert"],
        )
    {
        names.push("batch_insert_local_db");
    }
    if wants_vector_db || contains_any(normalized_query, &["index", "search"]) {
        names.push("index_local_db");
    }

    names
}

fn declaration_result_group(node_type: &str) -> &'static str {
    workflow_priority_group(node_type)
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
        "// FlowScript declarations matched {query:?} from the embedded .flow.d index.\n// Showing {} compact signatures. Call a node as `ns::alias({{ pin: value }})`, or open its namespace once at the top of the file with `use ns::*` and call `alias({{ pin: value }})`; a `this:` parameter marks the receiver pin, so that node is also a method on the value (`x.alias(...)`). Argument names are exact; the legacy camelCase name (`@alias`) is still accepted.\n",
        matches.len()
    );
    for line in use_lines(matches) {
        out.push_str(&line);
        out.push('\n');
    }
    out.push('\n');

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
        out.push('\n');
        if matched.receiver.is_some() {
            out.push_str("   // ");
            out.push_str(&matched.call_hint());
            out.push('\n');
        }
        out.push('\n');
    }

    out
}

/// One `// use ns::*` line per namespace in the result set, so the caller can copy the idiom
/// that makes the bare `alias({ … })` spelling resolve.
pub fn use_lines(matches: &[DeclarationMatch]) -> Vec<String> {
    matches
        .iter()
        .map(|matched| matched.namespace.as_str())
        .filter(|namespace| !namespace.is_empty())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .map(|namespace| format!("// use {namespace}::*"))
        .collect()
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
    let name = entry.node_type.as_str();
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
        if name == "email_imap_inbox_fetch_mail"
            && contains_any(normalized_query, &["fetch", "mail", "message", "email"])
        {
            score += 360;
        }
        if name == "mail_imap_list"
            && contains_any(
                normalized_query,
                &["fetch", "mail", "message", "email", "inbox"],
            )
        {
            score += 180;
        }
        if name == "email_smtp_connect"
            && contains_any(normalized_query, &["smtp", "send", "mail", "email"])
        {
            score += 420;
        }
        if name == "email_smtp_send" && contains_any(normalized_query, &["smtp", "send"]) {
            score += 360;
        }
        if name == "mail_imap_list_inboxes" && !normalized_query.contains("inboxes") {
            score = score.saturating_sub(220);
        }
        if matches!(
            name,
            "batch_insert_local_db" | "batch_upsert_local_db" | "insert_local_db"
        ) && wants_store_workflow
        {
            score += 320;
        }
        if name == "index_local_db" && wants_index_workflow {
            score += 1_150;
        }
        if matches!(
            name,
            "hybrid_search_local_db" | "vector_search_local_db" | "fts_search_local_db"
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

/// Workflow backbone weights, keyed by catalog node type. Any accepted spelling resolves here
/// through [`canonical_node_type`].
fn workflow_priority_group(spelling: &str) -> Option<(i32, &'static str)> {
    match canonical_node_type(spelling).as_str() {
        "email_imap_connect" => Some((120, "email")),
        "mail_imap_list_inboxes" => Some((80, "email")),
        "mail_imap_list" => Some((95, "email")),
        "email_imap_inbox_fetch_mail" => Some((110, "email")),
        "email_get_headers" => Some((70, "email")),
        "email_smtp_connect" => Some((100, "email")),
        "email_smtp_send" => Some((105, "email")),
        "embed_document" => Some((115, "embedding")),
        "embed_query" => Some((90, "embedding")),
        "split_text" | "chunk_text" => Some((70, "embedding")),
        "open_local_db" => Some((120, "database")),
        "batch_insert_local_db" => Some((110, "database")),
        "batch_upsert_local_db" => Some((105, "database")),
        "insert_local_db" => Some((80, "database")),
        "index_local_db" => Some((115, "database")),
        "hybrid_search_local_db" => Some((105, "database")),
        "vector_search_local_db" => Some((95, "database")),
        "fts_search_local_db" => Some((90, "database")),
        "df_create_session" => Some((95, "datafusion")),
        "df_register_lance" => Some((85, "datafusion")),
        "df_sql_query" => Some((90, "datafusion")),
        "df_execute_sql" => Some((95, "datafusion")),
        "df_list_tables" => Some((65, "datafusion")),
        "struct_set" => Some((100, "struct")),
        "struct_get" => Some((70, "struct")),
        "struct_make" => Some((60, "struct")),
        "cuid" => Some((75, "struct")),
        "utils_datetime_now" => Some((70, "struct")),
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

/// One parsed `function …;` / `declare function …;` line plus its JSDoc block and the namespace
/// block it sits in.
struct ParsedDeclaration {
    namespace: Vec<String>,
    name: String,
    params: String,
    return_type: String,
    jsdoc: String,
    category: Option<String>,
}

/// Walk a `.flow.d` file line by line. `declare namespace a {` / `namespace b {` heads push a
/// namespace segment, a bare `}` pops it, `// === Category ===` markers name the category of the
/// following declarations, and `/** … */` blocks attach to the next signature line.
fn parse_declaration_file(file: &DeclarationFileSource) -> Vec<DeclarationEntry> {
    let mut entries = Vec::new();
    let mut namespace: Vec<String> = Vec::new();
    let mut block_depths: Vec<usize> = Vec::new();
    let mut category: Option<String> = None;
    let mut jsdoc = String::new();
    let mut in_jsdoc = false;
    let mut pending: Option<String> = None;

    for raw_line in file.content.lines() {
        let line = raw_line.trim();
        if let Some(signature) = pending.as_mut() {
            signature.push(' ');
            signature.push_str(line);
            if line.contains(';') {
                let signature = pending.take().unwrap_or_default();
                push_declaration(
                    file,
                    &namespace,
                    category.as_deref(),
                    &jsdoc,
                    &signature,
                    &mut entries,
                );
                jsdoc.clear();
            }
            continue;
        }
        if in_jsdoc {
            jsdoc.push_str(raw_line);
            jsdoc.push('\n');
            if line.ends_with("*/") {
                in_jsdoc = false;
            }
            continue;
        }
        if line.starts_with("/**") {
            jsdoc.clear();
            jsdoc.push_str(raw_line);
            jsdoc.push('\n');
            in_jsdoc = !line.ends_with("*/");
            continue;
        }
        if let Some(section) = line
            .strip_prefix("// === ")
            .and_then(|rest| rest.strip_suffix(" ==="))
            .map(str::trim)
            .filter(|section| !section.is_empty())
        {
            category = Some(section.to_string());
            continue;
        }
        if let Some(head) = namespace_head(line) {
            block_depths.push(head.len());
            namespace.extend(head);
            category = None;
            jsdoc.clear();
            continue;
        }
        if line == "}" {
            let segments = block_depths.pop().unwrap_or(1);
            namespace.truncate(namespace.len().saturating_sub(segments));
            category = None;
            jsdoc.clear();
            continue;
        }
        if is_signature_line(line) {
            if line.contains(';') {
                push_declaration(
                    file,
                    &namespace,
                    category.as_deref(),
                    &jsdoc,
                    line,
                    &mut entries,
                );
                jsdoc.clear();
            } else {
                pending = Some(line.to_string());
            }
        }
    }

    entries
}

/// The namespace segments opened by a `declare namespace a.b {` / `namespace a::b {` line.
fn namespace_head(line: &str) -> Option<Vec<String>> {
    let rest = line.strip_prefix("declare ").unwrap_or(line);
    let rest = rest.strip_prefix("namespace ")?;
    let rest = rest.strip_suffix('{')?.trim();
    let segments = rest
        .split("::")
        .flat_map(|segment| segment.split('.'))
        .map(str::trim)
        .filter(|segment| !segment.is_empty())
        .map(ToString::to_string)
        .collect::<Vec<_>>();
    (!segments.is_empty()).then_some(segments)
}

fn push_declaration(
    file: &DeclarationFileSource,
    namespace: &[String],
    category: Option<&str>,
    jsdoc: &str,
    signature: &str,
    entries: &mut Vec<DeclarationEntry>,
) {
    let Some(parsed) = parse_signature_line(namespace, category, jsdoc, signature) else {
        return;
    };
    let (namespace, name) = split_qualified(&parsed.namespace, &parsed.name);
    let tags = jsdoc_tags(&parsed.jsdoc);
    let legacy = namespace.is_empty();
    let flat = tags
        .alias
        .clone()
        .or_else(|| legacy.then(|| name.clone()))
        .or_else(|| tags.node.as_deref().map(to_camel_case))
        .unwrap_or_else(|| name.clone());
    let node_type = tags.node.clone().unwrap_or_else(|| legacy_node_type(&flat));
    let function_name = if legacy {
        name.clone()
    } else {
        format!("{}::{name}", namespace.join("::"))
    };
    let receiver = tags.receiver.as_deref().map(to_camel_case).or_else(|| {
        parsed
            .params
            .trim_start()
            .starts_with("this:")
            .then(|| first_object_key(&parsed.params))
            .flatten()
    });
    let signature_line = if legacy {
        format!(
            "declare function {name}({}): {};",
            parsed.params, parsed.return_type
        )
    } else {
        format!(
            "function {function_name}({}): {};",
            parsed.params, parsed.return_type
        )
    };
    let declaration = format!("{}{signature_line}", parsed.jsdoc);
    let summary = declaration_summary(&parsed.jsdoc).unwrap_or_else(|| function_name.clone());
    let impure = tags.impure;
    let category = parsed.category.unwrap_or_else(|| {
        file.stem
            .split(['_', '-'])
            .map(capitalize)
            .collect::<Vec<_>>()
            .join("/")
    });
    let haystack =
        format!("{function_name} {flat} {node_type} {signature_line} {declaration} {category}")
            .to_lowercase();
    let function_tokens = tokenize_query_text(&function_name);
    let function_joined = function_tokens.join("");
    let tokens = tokenize_query_text(&haystack).into_iter().collect();

    entries.push(DeclarationEntry {
        path: file.path,
        category,
        function_name,
        node_type,
        namespace: namespace.join("::"),
        alias: name,
        flat,
        receiver,
        signature_line,
        summary,
        impure,
        haystack,
        tokens,
        function_tokens,
        function_joined,
    });
}

/// `function NAME(PARAMS): RET;` / `declare function NAME(PARAMS): RET;` → its parts.
fn parse_signature_line(
    namespace: &[String],
    category: Option<&str>,
    jsdoc: &str,
    signature: &str,
) -> Option<ParsedDeclaration> {
    let rest = signature.trim();
    let rest = rest.strip_prefix("declare ").unwrap_or(rest);
    let rest = rest.strip_prefix("function ")?.trim_start();
    let open = rest.find('(')?;
    let name = rest[..open].trim().to_string();
    if name.is_empty() {
        return None;
    }
    let after_name = &rest[open + 1..];
    let close = matching_paren(after_name)?;
    let params = after_name[..close].trim().to_string();
    let tail = after_name[close + 1..].trim();
    let return_type = tail
        .strip_prefix(':')
        .map(str::trim)
        .unwrap_or("void")
        .trim_end_matches(';')
        .trim()
        .to_string();
    Some(ParsedDeclaration {
        namespace: namespace.to_vec(),
        name,
        params,
        return_type,
        jsdoc: jsdoc.to_string(),
        category: category.map(ToString::to_string),
    })
}

/// Offset of the `)` closing the parameter list that starts right after `(`.
fn matching_paren(text: &str) -> Option<usize> {
    let mut depth = 1usize;
    for (index, character) in text.char_indices() {
        match character {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    return Some(index);
                }
            }
            _ => {}
        }
    }
    None
}

/// A name may itself be qualified (`string::contains`) when a standalone line carries the
/// namespace; fold those segments into the block namespace.
fn split_qualified(block: &[String], name: &str) -> (Vec<String>, String) {
    let mut segments = name
        .split("::")
        .map(str::trim)
        .filter(|segment| !segment.is_empty())
        .map(ToString::to_string)
        .collect::<Vec<_>>();
    let Some(last) = segments.pop() else {
        return (block.to_vec(), name.to_string());
    };
    let mut namespace = block.to_vec();
    namespace.extend(segments);
    (namespace, last)
}

#[derive(Default)]
struct JsDocTags {
    node: Option<String>,
    receiver: Option<String>,
    alias: Option<String>,
    impure: bool,
}

fn jsdoc_tags(jsdoc: &str) -> JsDocTags {
    let mut tags = JsDocTags::default();
    let mut words = jsdoc.split_whitespace().peekable();
    while let Some(word) = words.next() {
        match word {
            "@node" => tags.node = words.next().map(ToString::to_string),
            "@receiver" => tags.receiver = words.next().map(ToString::to_string),
            "@alias" => tags.alias = words.next().map(ToString::to_string),
            "@impure" => tags.impure = true,
            _ => {}
        }
    }
    tags
}

/// Best-effort node type for a legacy flat declaration without an `@node` tag.
fn legacy_node_type(flat: &str) -> String {
    let mut out = String::with_capacity(flat.len() + 4);
    for character in flat.chars() {
        if character.is_ascii_uppercase() {
            if !out.is_empty() {
                out.push('_');
            }
            out.push(character.to_ascii_lowercase());
        } else {
            out.push(character);
        }
    }
    out
}

fn first_object_key(params: &str) -> Option<String> {
    let object = &params[params.find('{')?..];
    let key = object
        .trim_start_matches('{')
        .split([',', '}'])
        .next()?
        .split_once(':')?
        .0
        .trim()
        .trim_end_matches('?');
    (!key.is_empty()).then(|| key.to_string())
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

    fn node_types(query: &str) -> Vec<String> {
        search_declarations(query)
            .into_iter()
            .map(|matched| matched.node_type)
            .collect()
    }

    fn first_node_type(query: &str) -> Option<String> {
        search_declarations(query)
            .into_iter()
            .next()
            .map(|matched| matched.node_type)
    }

    const V2_FIXTURE: &str = r#"// Utils — FlowScript node declarations (generated, do not edit).

declare namespace string {
    // === Utils/String ===

    /**
     * Checks whether a string contains a substring
     * @node string_contains @receiver string @alias stringContains
     * @param string — Input String (receiver: `this` in `x.contains(...)`)
     * @param substring — Needle
     * @param ignoreCase (optional) — Compare without regard to case
     * @returns contains — Does the string contain the substring?
     */
    function contains(this: string, { string: string, substring: string, ignoreCase?: bool }): bool;

    /** @node string_length @receiver string @alias stringLength */
    function length(this: string, { string: string }): int;
}

declare namespace ai {
    namespace ml {
        // === AI/ML ===

        /**
         * Reads a model
         * @node ai_ml_model_read @alias aiMlModelRead
         * @param path — Model path
         * @impure has side effects / drives control flow
         */
        function read({ path: string }): Struct;
    }
}

declare namespace utils.markdown {
    /** @node utils_md_html_to_md @alias utilsMdHtmlToMd */
    function mdHtmlToMd({ html: string, skippedTags?: string[] }): string;
}

// === Legacy ===

/**
 * Hashes a string
 * @param input — Input
 * @returns hash — Digest
 */
declare function utilsHashMd5({ input: string }): string;
"#;

    fn fixture_entries() -> Vec<DeclarationEntry> {
        parse_declaration_file(&DeclarationFileSource {
            path: "fixture.flow.d",
            stem: "fixture",
            content: V2_FIXTURE,
        })
    }

    #[test]
    fn parses_namespaced_declarations_with_tags() {
        let entries = fixture_entries();
        assert_eq!(
            entries
                .iter()
                .map(|entry| entry.function_name.as_str())
                .collect::<Vec<_>>(),
            [
                "string::contains",
                "string::length",
                "ai::ml::read",
                "utils::markdown::mdHtmlToMd",
                "utilsHashMd5"
            ]
        );

        let contains = &entries[0];
        assert_eq!(contains.node_type, "string_contains");
        assert_eq!(contains.namespace, "string");
        assert_eq!(contains.alias, "contains");
        assert_eq!(contains.flat, "stringContains");
        assert_eq!(contains.receiver.as_deref(), Some("string"));
        assert_eq!(contains.category, "Utils/String");
        assert_eq!(
            contains.summary,
            "Checks whether a string contains a substring"
        );
        assert!(!contains.impure);
        assert_eq!(
            contains.signature_line,
            "function string::contains(this: string, { string: string, substring: string, ignoreCase?: bool }): bool;"
        );

        let length = &entries[1];
        assert_eq!(length.summary, "string::length");
        assert_eq!(length.receiver.as_deref(), Some("string"));

        let read = &entries[2];
        assert_eq!(read.node_type, "ai_ml_model_read");
        assert_eq!(read.namespace, "ai::ml");
        assert_eq!(read.flat, "aiMlModelRead");
        assert_eq!(read.category, "AI/ML");
        assert!(read.impure);
        assert_eq!(read.receiver, None);
        assert_eq!(
            read.signature_line,
            "function ai::ml::read({ path: string }): Struct;"
        );

        let dotted = &entries[3];
        assert_eq!(dotted.namespace, "utils::markdown");
        assert_eq!(dotted.category, "Fixture");

        let legacy = &entries[4];
        assert_eq!(legacy.node_type, "utils_hash_md5");
        assert_eq!(legacy.namespace, "");
        assert_eq!(legacy.flat, "utilsHashMd5");
        assert_eq!(legacy.category, "Legacy");
        assert_eq!(
            legacy.signature_line,
            "declare function utilsHashMd5({ input: string }): string;"
        );
    }

    #[test]
    fn call_hints_show_static_and_method_forms() {
        let entries = fixture_entries();
        assert_eq!(
            entries[0].to_match(0).call_hint(),
            "string::contains({ string: string, substring: substring, ignoreCase: ignoreCase })  or  string.contains({ substring: substring, ignoreCase: ignoreCase })"
        );
        assert_eq!(
            entries[1].to_match(0).call_hint(),
            "string::length({ string: string })  or  string.length()"
        );
        assert_eq!(
            entries[2].to_match(0).call_hint(),
            "ai::ml::read({ path: path })"
        );
    }

    #[test]
    fn embeds_every_generated_declaration_file() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../ast/flow.d");
        let mut expected = Vec::new();
        for (dir, prefix) in [(root.clone(), ""), (root.join("packages"), "packages/")] {
            for entry in std::fs::read_dir(&dir).expect("flow.d directory") {
                let name = entry.expect("dir entry").file_name();
                let name = name.to_string_lossy().to_string();
                if name.ends_with(".flow.d") {
                    expected.push(format!("{prefix}{name}"));
                }
            }
        }
        expected.sort();
        let mut embedded = DECLARATION_FILES
            .iter()
            .map(|file| file.path.to_string())
            .collect::<Vec<_>>();
        embedded.sort();
        assert_eq!(
            embedded, expected,
            "DECLARATION_FILES must embed every generated .flow.d file"
        );
    }

    #[test]
    fn every_embedded_declaration_resolves_to_a_node_type() {
        assert!(DECLARATION_INDEX.len() > 1_000);
        for entry in DECLARATION_INDEX.iter() {
            assert!(
                !entry.node_type.is_empty() && !entry.flat.is_empty(),
                "{} lacks a node type or flat name",
                entry.function_name
            );
            assert!(
                is_signature_line(&entry.signature_line),
                "{} has no signature line",
                entry.function_name
            );
        }
        assert_eq!(
            canonical_node_type("email::imap::connect"),
            "email_imap_connect"
        );
        assert_eq!(
            canonical_node_type("emailImapConnect"),
            "email_imap_connect"
        );
        assert_eq!(
            canonical_node_type("EMAIL_IMAP_CONNECT"),
            "email_imap_connect"
        );
    }

    #[test]
    fn finds_email_declarations_from_camelish_query() {
        let names = node_types("imapConnect");
        assert!(names.iter().any(|name| name == "email_imap_connect"));
    }

    #[test]
    fn finds_database_and_search_declarations() {
        let open_names = node_types("open database");
        assert!(open_names.iter().any(|name| name == "open_local_db"));

        let hybrid_names = node_types("hybrid vector search");
        assert!(
            hybrid_names
                .iter()
                .any(|name| name == "hybrid_search_local_db")
        );
    }

    #[test]
    fn fuzzy_missing_batch_embedding_still_finds_document_embedding() {
        let names = node_types("batchEmbedDocument");
        assert!(names.iter().any(|name| name == "embed_document"));
    }

    #[test]
    fn exact_function_symbol_outranks_workflow_expansion() {
        assert_eq!(
            first_node_type("mailAddressFields address struct input schema").as_deref(),
            Some("mail_address_fields")
        );
        assert_eq!(first_node_type("cuid generate id").as_deref(), Some("cuid"));
    }

    #[test]
    fn every_accepted_spelling_is_an_exact_symbol() {
        for query in [
            "hybridSearchLocalDb with these pins",
            "hybrid_search_local_db with these pins",
            "db::hybridSearch with these pins",
            "use db::hybridSearch({ database })",
        ] {
            let matches = search_declarations(query);
            let top = matches.first().expect("a match");
            assert_eq!(
                top.node_type, "hybrid_search_local_db",
                "{query:?}: {matches:?}"
            );
            assert!(
                top.score >= EXACT_SYMBOL_SCORE,
                "{query:?} was not an exact symbol"
            );
        }
    }

    #[test]
    fn generic_terms_do_not_promote_unrelated_workflow_declarations() {
        for (query, unrelated) in [
            (
                "chat send response message citation source attachment file",
                "email_smtp_send",
            ),
            ("a2ui write CSV to table", "open_local_db"),
            (
                "agent tool structured output schema",
                "batch_insert_local_db",
            ),
        ] {
            let matches = search_declarations(query);
            assert_ne!(
                matches.first().map(|matched| matched.node_type.as_str()),
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
            "faker::integer",
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
        for query in [
            "Use hybridSearchLocalDb with these pins",
            "Use db::hybridSearch with these pins",
            "Use hybrid_search_local_db with these pins",
        ] {
            let evidence = declaration_semantic_evidence(
                query,
                "db::hybridSearch",
                "hybrid_search_local_db",
                "Hybrid Search Local DB",
                "Runs hybrid vector and full text search.",
                Some("data/search"),
                &[],
            );

            assert!(evidence.exact_symbol, "{query:?}");
            assert!(evidence.accepts());
            assert!(
                evidence
                    .reason_codes
                    .contains(&"exact_function_symbol".to_string())
            );
        }
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

        assert!(rendered.contains("function "));
        assert!(!rendered.contains("@param"));
        assert!(!rendered.contains("/**"));
        assert!(
            rendered.len() < 8_000,
            "declaration output should stay compact, got {} bytes",
            rendered.len()
        );
    }

    #[test]
    fn rendered_matches_teach_the_use_idiom_and_method_form() {
        let entries = fixture_entries();
        let matches = entries
            .iter()
            .map(|entry| entry.to_match(10))
            .collect::<Vec<_>>();
        let rendered = render_declaration_matches("contains", &matches);
        assert!(
            rendered.contains("// use ai::ml::*\n// use string::*\n// use utils::markdown::*\n")
        );
        assert!(rendered.contains("`use ns::*`"));
        assert!(rendered.contains(
            "1. string::contains — Checks whether a string contains a substring [fixture.flow.d :: Utils/String, score 10]\n   function string::contains(this: string, { string: string, substring: string, ignoreCase?: bool }): bool;\n   // string::contains({ string: string, substring: substring, ignoreCase: ignoreCase })  or  string.contains({ substring: substring, ignoreCase: ignoreCase })\n"
        ));
        assert!(
            rendered.contains("   function ai::ml::read({ path: string }): Struct;  // impure\n\n")
        );
        assert!(!rendered.contains("// ai::ml::read("));
    }

    #[test]
    fn broad_email_vector_workflow_query_finds_core_nodes() {
        let names = node_types(
            "gmail imap smtp fetch mails last 2 days embed sentiment vector db cuids timestamp sender",
        );
        for expected in [
            "email_imap_connect",
            "email_smtp_connect",
            "email_imap_inbox_fetch_mail",
            "embed_document",
            "open_local_db",
            "batch_insert_local_db",
            "index_local_db",
        ] {
            assert!(
                names.iter().any(|name| name == expected),
                "missing {expected} in {names:?}"
            );
        }
    }
}
