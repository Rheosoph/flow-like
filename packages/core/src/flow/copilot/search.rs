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
    let tokens: Vec<String> = normalized
        .split(|c: char| !c.is_alphanumeric())
        .filter(|token| !token.is_empty())
        .map(ToString::to_string)
        .collect();

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
    {
        expanded.insert("imap".to_string());
        expanded.insert("inbox".to_string());
        expanded.insert("fetch".to_string());
        expanded.insert("list".to_string());
    }

    SearchQueryAnalysis {
        normalized,
        tokens,
        expanded_tokens: expanded.into_iter().collect(),
    }
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

fn token_synonyms(token: &str) -> &'static [&'static str] {
    match token {
        "email" | "mail" => &["smtp", "imap", "gmail", "outlook", "message"],
        "gmail" => &["smtp", "imap", "google", "email"],
        "inbox" => &["imap", "mailbox", "email", "message"],
        "unread" => &["new", "unseen", "imap", "inbox"],
        "notification" => &["email", "mail", "send"],
        "receipt" => &["email", "mail", "send"],
        "followup" | "follow-up" => &["email", "send", "notification"],
        "webhook" => &["trigger", "http", "event"],
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
            "email_get_headers".to_string(),
            "email_imap_mark_seen".to_string(),
            "email_imap_move_message".to_string(),
        ],
        _ => Vec::new(),
    }
}
