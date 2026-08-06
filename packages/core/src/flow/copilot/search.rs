use std::collections::BTreeSet;

use super::types::NodeMetadata;

#[derive(Debug, Clone)]
pub struct SearchQueryAnalysis {
    pub normalized: String,
    pub tokens: Vec<String>,
    pub expanded_tokens: Vec<String>,
}

pub fn analyze_search_query(query: &str) -> SearchQueryAnalysis {
    let normalized = query.trim().to_lowercase();
    let tokens = tokenize_query_text(query);

    let mut expanded = BTreeSet::new();
    for token in &tokens {
        expanded.insert(token.clone());
        for synonym in token_synonyms(token) {
            expanded.insert(synonym.to_string());
        }
    }

    if normalized.contains("send email") || normalized.contains("email send") {
        expanded.insert("smtp".to_string());
        expanded.insert("send".to_string());
        expanded.insert("mail".to_string());
    }

    if normalized.contains("read email")
        || normalized.contains("check inbox")
        || normalized.contains("read inbox")
        || normalized.contains("fetch email")
        || normalized.contains("fetch mail")
        || normalized.contains("past 2 days")
    {
        expanded.insert("imap".to_string());
        expanded.insert("inbox".to_string());
        expanded.insert("fetch".to_string());
        expanded.insert("list".to_string());
        expanded.insert("mail".to_string());
        expanded.insert("message".to_string());
    }

    if normalized.contains("open database")
        || normalized.contains("local db")
        || normalized.contains("local database")
    {
        expanded.insert("open".to_string());
        expanded.insert("local".to_string());
        expanded.insert("db".to_string());
        expanded.insert("database".to_string());
        expanded.insert("lance".to_string());
        expanded.insert("lancedb".to_string());
    }

    if normalized.contains("store")
        || normalized.contains("write")
        || normalized.contains("persist")
        || normalized.contains("put")
        || (normalized.contains("vector") && normalized.contains("db"))
        || normalized.contains("vector database")
    {
        expanded.insert("open".to_string());
        expanded.insert("local".to_string());
        expanded.insert("db".to_string());
        expanded.insert("database".to_string());
        expanded.insert("table".to_string());
        expanded.insert("row".to_string());
        expanded.insert("record".to_string());
        expanded.insert("batch".to_string());
        expanded.insert("insert".to_string());
        expanded.insert("upsert".to_string());
    }

    if normalized.contains("datafusion sql") || normalized.contains("sql query") {
        expanded.insert("datafusion".to_string());
        expanded.insert("sql".to_string());
        expanded.insert("query".to_string());
        expanded.insert("session".to_string());
        expanded.insert("table".to_string());
    }

    if normalized.contains("hybrid search") {
        expanded.insert("hybrid".to_string());
        expanded.insert("vector".to_string());
        expanded.insert("full".to_string());
        expanded.insert("text".to_string());
        expanded.insert("fts".to_string());
        expanded.insert("search".to_string());
    }

    if normalized.contains("build index")
        || normalized.contains("create index")
        || normalized.contains("vector index")
        || normalized.contains("index build")
        || normalized.contains("vector db")
        || normalized.contains("vector database")
    {
        expanded.insert("build".to_string());
        expanded.insert("create".to_string());
        expanded.insert("index".to_string());
        expanded.insert("vector".to_string());
        expanded.insert("full".to_string());
        expanded.insert("text".to_string());
    }

    if normalized.contains("sentiment") {
        expanded.insert("classification".to_string());
        expanded.insert("classify".to_string());
        expanded.insert("label".to_string());
        expanded.insert("positive".to_string());
        expanded.insert("negative".to_string());
        expanded.insert("embedding".to_string());
    }

    SearchQueryAnalysis {
        normalized,
        tokens,
        expanded_tokens: expanded.into_iter().collect(),
    }
}

pub fn tokenize_query_text(text: &str) -> Vec<String> {
    let mut normalized = String::with_capacity(text.len());
    let mut previous_was_lower_or_digit = false;

    for ch in text.chars() {
        if ch.is_ascii_uppercase() {
            if previous_was_lower_or_digit {
                normalized.push(' ');
            }
            normalized.push(ch.to_ascii_lowercase());
            previous_was_lower_or_digit = false;
        } else if ch.is_ascii_alphanumeric() {
            normalized.push(ch.to_ascii_lowercase());
            previous_was_lower_or_digit = ch.is_ascii_lowercase() || ch.is_ascii_digit();
        } else {
            normalized.push(' ');
            previous_was_lower_or_digit = false;
        }
    }

    normalized
        .split_whitespace()
        .filter(|token| !token.is_empty())
        .map(ToString::to_string)
        .collect()
}

pub fn enrich_node_metadata(mut metadata: NodeMetadata) -> NodeMetadata {
    metadata.required_inputs = metadata
        .inputs
        .iter()
        .filter(|pin| pin.data_type != "Execution" && pin.default_value.is_none())
        .map(|pin| pin.name.clone())
        .collect();

    let mut capability_tags = BTreeSet::new();
    let haystack = format!(
        "{} {} {} {}",
        metadata.name.to_lowercase(),
        metadata.friendly_name.to_lowercase(),
        metadata.description.to_lowercase(),
        metadata.category.clone().unwrap_or_default().to_lowercase(),
    );

    if haystack.contains("email") || haystack.contains("mail") {
        capability_tags.insert("email".to_string());
    }
    if haystack.contains("smtp") {
        capability_tags.insert("smtp".to_string());
        capability_tags.insert("send-email".to_string());
    }
    if haystack.contains("imap") {
        capability_tags.insert("imap".to_string());
        capability_tags.insert("read-email".to_string());
    }
    if haystack.contains("gmail") {
        capability_tags.insert("gmail".to_string());
    }
    if haystack.contains("inbox") || haystack.contains("mailbox") {
        capability_tags.insert("inbox".to_string());
    }
    if haystack.contains("fetch") {
        capability_tags.insert("fetch".to_string());
    }
    if haystack.contains("webhook") || haystack.contains("http event") {
        capability_tags.insert("trigger".to_string());
        capability_tags.insert("webhook".to_string());
    }
    if haystack.contains("notification") {
        capability_tags.insert("notification".to_string());
    }
    // Category-gated so the tags stay strong evidence: without them the path accessors fall under
    // the declaration acceptance gate for the plain-language queries that should find them.
    if metadata
        .category
        .as_deref()
        .is_some_and(|category| category.starts_with("Data/Files/Path"))
    {
        capability_tags.insert("file".to_string());
        capability_tags.insert("path".to_string());
        if haystack.contains("filename") {
            capability_tags.insert("filename".to_string());
            capability_tags.insert("basename".to_string());
            capability_tags.insert("stem".to_string());
        }
        if haystack.contains("extension") {
            capability_tags.insert("extension".to_string());
        }
        if haystack.contains("parent") || haystack.contains("child") || haystack.contains("folder")
        {
            capability_tags.insert("folder".to_string());
        }
    }
    if haystack.contains("http") || haystack.contains("request") {
        capability_tags.insert("http".to_string());
    }

    metadata.capability_tags = capability_tags.into_iter().collect();
    metadata.companion_nodes = companion_nodes_for(&metadata.name);
    metadata
}

pub fn score_catalog_metadata(metadata: &NodeMetadata, query: &str) -> i32 {
    let analysis = analyze_search_query(query);
    let name_lower = metadata.name.to_lowercase();
    let friendly_lower = metadata.friendly_name.to_lowercase();
    let desc_lower = metadata.description.to_lowercase();
    let category = metadata.category.clone().unwrap_or_default().to_lowercase();
    let symbol_tokens = tokenize_query_text(&metadata.name);
    let friendly_tokens = tokenize_query_text(&metadata.friendly_name);

    let mut score = 0i32;

    if !analysis.normalized.is_empty() {
        if name_lower.contains(&analysis.normalized) {
            score += 100;
        }
        if friendly_lower.contains(&analysis.normalized) {
            score += 90;
        }
        if desc_lower.contains(&analysis.normalized) {
            score += 35;
        }
    }

    for token in &analysis.expanded_tokens {
        if name_lower.contains(token) {
            score += 30;
        }
        if friendly_lower.contains(token) {
            score += 25;
        }
        if category.contains(token) {
            score += 20;
        }
        if desc_lower.contains(token) {
            score += 10;
        }
        if metadata.capability_tags.iter().any(|tag| tag == token) {
            score += 18;
        }
        let fuzzy_symbol_match = symbol_tokens
            .iter()
            .chain(friendly_tokens.iter())
            .any(|candidate| candidate.len() > 3 && strsim::jaro_winkler(candidate, token) >= 0.9);
        if fuzzy_symbol_match {
            score += 16;
        }
    }

    let name_parts: Vec<&str> = name_lower.split([':', '_']).collect();
    for token in &analysis.tokens {
        if name_parts.iter().any(|part| part == token) {
            score += 15;
        }
    }

    if analysis.normalized.contains("send email") && name_lower.contains("email_smtp_send") {
        score += 120;
    }

    if (analysis.normalized.contains("smtp") || analysis.normalized.contains("gmail"))
        && name_lower.contains("email_smtp_connect")
    {
        score += 80;
    }

    if (analysis.normalized.contains("read email")
        || analysis.normalized.contains("check inbox")
        || analysis.normalized.contains("imap")
        || analysis.normalized.contains("unread"))
        && (name_lower.contains("email_imap_connect")
            || name_lower.contains("mail_imap_list")
            || name_lower.contains("email_imap_inbox_fetch_mail")
            || name_lower.contains("mail_imap_inbox"))
    {
        score += 70;
    }

    score
}

pub fn search_result_hint_lines(metadata: &NodeMetadata) -> Vec<String> {
    let mut hints = Vec::new();

    if !metadata.required_inputs.is_empty() {
        hints.push(format!("requires: {}", metadata.required_inputs.join(", ")));
    }

    if !metadata.companion_nodes.is_empty() {
        hints.push(format!(
            "pairs with: {}",
            metadata.companion_nodes.join(", ")
        ));
    }

    if !metadata.capability_tags.is_empty() {
        hints.push(format!("tags: {}", metadata.capability_tags.join(", ")));
    }

    hints
}

/// Compact model-facing rendering of catalog search results (one line per node + usage hints).
/// Single source for the `catalog_search` tool output across every backend executor.
pub fn render_catalog_search_results(results: &[NodeMetadata]) -> String {
    if results.is_empty() {
        return "No nodes found matching your query. Try different keywords.".to_string();
    }

    results
        .iter()
        .map(|meta| {
            let hints = search_result_hint_lines(meta);
            if hints.is_empty() {
                meta.to_compact()
            } else {
                format!("{} [{}]", meta.to_compact(), hints.join("; "))
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn token_synonyms(token: &str) -> &'static [&'static str] {
    match token {
        "email" | "mail" => &["smtp", "imap", "gmail", "outlook", "message"],
        "gmail" => &["smtp", "imap", "google", "email"],
        "inbox" => &["imap", "mailbox", "email", "message"],
        "unread" => &["new", "unseen", "imap", "inbox"],
        "db" => &["database", "lance", "lancedb", "local", "table"],
        "database" => &["db", "lance", "lancedb", "local", "table"],
        "lance" | "lancedb" => &["database", "db", "vector", "table"],
        "sql" => &["datafusion", "query", "table"],
        "datafusion" => &["sql", "query", "session", "table"],
        "embed" | "embedding" => &["vector", "model", "document", "query"],
        "vector" => &["embedding", "search", "index", "lancedb"],
        "fts" => &["full", "text", "search", "index"],
        "hybrid" => &["vector", "full", "text", "search", "rerank"],
        "index" => &["build", "create", "optimize", "vector", "full", "text"],
        "batch" => &["many", "bulk", "array", "insert", "upsert"],
        "put" | "store" | "write" | "persist" => {
            &["database", "insert", "upsert", "local", "table", "record"]
        }
        "row" | "record" => &["database", "insert", "table", "struct"],
        "notification" => &["email", "mail", "send"],
        "receipt" => &["email", "mail", "send"],
        "followup" | "follow-up" => &["email", "send", "notification"],
        "webhook" => &["trigger", "http", "event"],
        "file" => &["path", "flowpath", "filename", "extension", "storage"],
        "filename" | "basename" => &["file", "path", "name", "extension", "stem"],
        "extension" | "ext" => &["file", "path", "filename", "suffix"],
        "folder" | "directory" | "dir" => &["path", "parent", "child", "list", "storage"],
        "stem" => &["filename", "file", "path", "extension"],
        _ => &[],
    }
}

fn companion_nodes_for(node_name: &str) -> Vec<String> {
    match node_name {
        "email_smtp_connect" => vec!["email_smtp_send".to_string()],
        "email_smtp_send" => vec!["email_smtp_connect".to_string()],
        "email_imap_connect" => vec![
            "mail_imap_list_inboxes".to_string(),
            "mail_imap_inbox".to_string(),
            "mail_imap_list".to_string(),
        ],
        "mail_imap_inbox" => vec!["mail_imap_list".to_string()],
        "mail_imap_list" => vec![
            "email_imap_inbox_fetch_mail".to_string(),
            "email_imap_mark_seen".to_string(),
        ],
        "email_imap_inbox_fetch_mail" => vec![
            "email_imap_connect".to_string(),
            "mail_imap_inbox".to_string(),
            "mail_imap_list".to_string(),
            "email_get_headers".to_string(),
            "email_get_content".to_string(),
            "mail_address_fields".to_string(),
            "email_imap_mark_seen".to_string(),
            "email_imap_move_message".to_string(),
        ],
        // A FlowPath carries no file attributes, so every producer of one advertises the accessors.
        // Without this the model reaches for a generic struct-field read and silently gets null.
        "path_from_upload_dir"
        | "path_from_storage_dir"
        | "path_from_cache_dir"
        | "path_from_user_dir"
        | "path_virtual_dir"
        | "path_list_paths"
        | "path_list_folders"
        | "pathbuf_to_path"
        | "a2ui_get_file_input_files"
        | "events_chat_attachment_from_signed_url" => vec![
            "filename".to_string(),
            "extension".to_string(),
            "raw_path".to_string(),
            "parent".to_string(),
            "child".to_string(),
        ],
        _ => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn imap_fetch_companions_include_the_required_upstream_chain() {
        let companions = companion_nodes_for("email_imap_inbox_fetch_mail");
        for required in [
            "email_imap_connect",
            "mail_imap_inbox",
            "mail_imap_list",
            "email_get_headers",
            "email_get_content",
            "mail_address_fields",
            "email_imap_mark_seen",
        ] {
            assert!(
                companions.iter().any(|companion| companion == required),
                "missing {required} in {companions:?}"
            );
        }
    }
}
