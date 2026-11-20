//! Standing tool approvals. A bot's owner allows a tool for a scope once and is never asked again,
//! on any host. A grant is a `(tool, scope)` pair stored with the bot; the same pair format keys
//! the short-lived tokens a one-time answer leaves for the turn that resumes after it.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// The scope that covers every call of a tool.
pub const SCOPE_ALL: &str = "*";
/// The tool of the auto-approval grant `(*, *)`: the bot runs every call a grant could cover
/// without asking. Irreversible deletions, configuration changes, cost checks and web
/// destinations after web content still ask.
pub const AUTO_APPROVE_TOOL: &str = "*";
/// The scope of the document cost check; its grant belongs to [`DOCUMENTS_TOOL`].
pub const SCOPE_DOCUMENTS_EXPENSIVE: &str = "documents.expensive";
/// The tool name a `documents.expensive` grant is stored under.
pub const DOCUMENTS_TOOL: &str = "documents";
/// Scope prefix of a one-time token: `once:<args_hash>`.
pub const ONCE_PREFIX: &str = "once:";
/// Scope prefix of a refusal the owner gave: `deny:<args_hash>`.
pub const DENY_PREFIX: &str = "deny:";
/// Hours a one-time token or refusal lives when no resumed run consumes it.
pub const TOKEN_TTL_HOURS: i64 = 24;
/// Days a standing grant lasts after its last use before it lapses.
pub const GRANT_TTL_DAYS: i64 = 90;
/// A use moves the expiry forward at most this often, in hours.
pub const GRANT_SLIDE_HOURS: i64 = 24;
/// Days a turn waits for its owner's answer before it is cancelled.
pub const APPROVAL_WAIT_DAYS: i64 = 7;
/// The `error_code` of a turn cancelled because nobody answered.
pub const ERROR_APPROVAL_EXPIRED: &str = "BOT_APPROVAL_EXPIRED";
/// The `error_code` of an answer naming an item that is not on its card, or one item twice.
pub const ERROR_APPROVAL_ITEM_UNKNOWN: &str = "BOT_APPROVAL_ITEM_UNKNOWN";

/// Longest tool name or scope.
pub const MAX_KEY_CHARS: usize = 256;

/// Every prefix a tool's grant scopes start with (core `tool_grants::grant_scopes`, the bot-tools
/// scope helpers and the `ask_bot` gate).
pub const SCOPE_PREFIXES: [&str; 14] = [
    "app:",
    "host:",
    "connection:",
    "table:",
    "target:",
    "connector:",
    "browser:",
    "owner_browser:",
    "computer:",
    "email:",
    "person:",
    "command:",
    "web:",
    "bot:",
];

/// A grant as the owner lists it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ToolGrant {
    pub tool: String,
    pub scope: String,
    /// RFC 3339.
    pub granted_at: String,
    /// The person who granted it.
    pub granted_by: String,
    /// RFC 3339: when the grant lapses unless the bot uses it first.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<String>,
}

/// The grants of one bot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ToolGrantList {
    pub grants: Vec<ToolGrant>,
}

/// The body of a grant's `PUT` and `DELETE`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ToolGrantKey {
    pub tool: String,
    pub scope: String,
}

/// What the owner answers to an approval card.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalDecision {
    /// Run this call; ask again next time.
    Once,
    /// Run this call and store a grant: never ask again for the scope.
    Always,
    /// Do not run it; the model is told.
    Deny,
}

impl ApprovalDecision {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Once => "once",
            Self::Always => "always",
            Self::Deny => "deny",
        }
    }
}

/// The body of `POST /bots/{id}/turns/{turn}/approval`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ApprovalAnswer {
    pub approval_id: String,
    /// The answer to every item not listed in `items`; the whole answer to a card for one call.
    pub decision: ApprovalDecision,
    /// The items of a batch card answered differently from `decision`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub items: Vec<crate::bot_message::ItemDecision>,
}

/// Why a grant key was refused. Answer tokens (`once:…`, `deny:…`) pass: a host stores them for
/// the turn it resumes.
pub fn check_key(key: &ToolGrantKey) -> Result<(), String> {
    let tool = key.tool.trim();
    if tool.is_empty()
        || tool.chars().count() > MAX_KEY_CHARS
        || tool.chars().any(char::is_whitespace)
    {
        return Err(format!(
            "tool '{}' must be 1 to {MAX_KEY_CHARS} characters without spaces",
            key.tool.chars().take(80).collect::<String>()
        ));
    }
    if !is_grant_scope(&key.scope) && !is_token(&key.scope) {
        return Err(format!(
            "scope '{}' must be '*', 'documents.expensive' or start with one of {}",
            key.scope.chars().take(80).collect::<String>(),
            SCOPE_PREFIXES.join(", ")
        ));
    }
    Ok(())
}

/// Whether `scope` is one an owner may grant: the whole tool, the cost check or a prefixed
/// target of at most [`MAX_KEY_CHARS`] characters.
pub fn is_grant_scope(scope: &str) -> bool {
    if scope == SCOPE_ALL || scope == SCOPE_DOCUMENTS_EXPENSIVE {
        return true;
    }
    scope.chars().count() <= MAX_KEY_CHARS
        && !scope.chars().any(|char| char.is_control() || char == ',')
        && SCOPE_PREFIXES
            .iter()
            .any(|prefix| scope.len() > prefix.len() && scope.starts_with(prefix))
}

fn is_token(scope: &str) -> bool {
    is_token_scope(scope)
        && scope.chars().count() <= MAX_KEY_CHARS
        && !scope
            .chars()
            .any(|char| char.is_control() || char.is_whitespace())
}

/// Tokens are internal rows beside the grants; they are never listed or granted by hand.
pub fn is_token_scope(scope: &str) -> bool {
    scope.starts_with(ONCE_PREFIX) || scope.starts_with(DENY_PREFIX)
}

fn canonical(value: &Value, out: &mut String) {
    match value {
        Value::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            out.push('{');
            for key in keys {
                out.push_str(&Value::String(key.clone()).to_string());
                out.push(':');
                canonical(&map[key], out);
                out.push(',');
            }
            out.push('}');
        }
        Value::Array(items) => {
            out.push('[');
            for item in items {
                canonical(item, out);
                out.push(',');
            }
            out.push(']');
        }
        other => out.push_str(&other.to_string()),
    }
}

/// A stable hash of one call: the tool and its arguments with object keys sorted, so the same
/// call hashes the same on every host and run.
pub fn args_hash(tool: &str, args: &Value) -> String {
    let mut text = String::new();
    canonical(args, &mut text);
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"flow-like.tool-call/v1\0");
    hasher.update(tool.as_bytes());
    hasher.update(b"\0");
    hasher.update(text.as_bytes());
    hasher.finalize().to_hex().as_str()[..32].to_string()
}

fn digest(prefix: &str, parts: &[&str]) -> String {
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"flow-like.approval/v1\0");
    for part in parts {
        hasher.update(&(part.len() as u64).to_be_bytes());
        hasher.update(part.as_bytes());
    }
    format!("{prefix}{}", &hasher.finalize().to_hex().as_str()[..32])
}

/// The message id (and approval id) of the card a turn posts for one call: asking twice for the
/// same call posts once.
pub fn approval_message_id(turn_id: &str, args_hash: &str) -> String {
    digest("ap-", &[turn_id, args_hash])
}

/// The message id of the owner's answer to an approval card.
pub fn approval_response_message_id(approval_id: &str) -> String {
    digest("ar-", &[approval_id])
}

/// The message id of the owner's answer to a card in an end-to-end encrypted chat. The owner's
/// device seals the answer under it, so it is derived without a hash the browser lacks: the
/// card id with the answer prefix.
pub fn sealed_approval_response_message_id(approval_id: &str) -> String {
    format!(
        "ar-{}",
        approval_id.strip_prefix("ap-").unwrap_or(approval_id)
    )
}

/// Whether `card` is the one its id names: `turn_id` and the hash of its calls give the id, and a
/// batch card's hash is its items' hashes. The server checks a card the owner's device opened
/// from an encrypted chat with this before settling an answer from it.
pub fn card_matches_id(card: &crate::bot_message::ApprovalRequest, turn_id: &str) -> bool {
    let calls_match = card.items.is_empty()
        || card.args_hash == batch_args_hash(card.items.iter().map(|item| item.args_hash.as_str()));
    card.turn_id == turn_id
        && calls_match
        && approval_message_id(turn_id, &card.args_hash) == card.id
}

/// An approval card a host sealed for the owner's end-to-end encrypted chat with the bot. The
/// server parks the turn behind `approval_id` and posts the envelopes as the bot's message
/// without reading them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SealedApprovalCard {
    /// The card's id (`ap-…`), also its message id.
    pub approval_id: String,
    /// The card message sealed to every member's keys, by member id.
    pub envelopes: std::collections::BTreeMap<String, Value>,
}

/// The `args_hash` of a batch card: its items' hashes in any order, so parking the same calls
/// again names the same card.
pub fn batch_args_hash<'a>(hashes: impl IntoIterator<Item = &'a str>) -> String {
    let mut sorted: Vec<&str> = hashes.into_iter().collect();
    sorted.sort_unstable();
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"flow-like.approval-batch/v1\0");
    for hash in sorted {
        hasher.update(&(hash.len() as u64).to_be_bytes());
        hasher.update(hash.as_bytes());
    }
    hasher.finalize().to_hex().as_str()[..32].to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn the_hash_ignores_key_order_and_tells_calls_apart() {
        let a = args_hash("code_run", &json!({"a": 1, "b": {"x": [1, 2], "y": null}}));
        let b = args_hash("code_run", &json!({"b": {"y": null, "x": [1, 2]}, "a": 1}));
        assert_eq!(a, b);
        assert_ne!(
            a,
            args_hash("code_run", &json!({"a": 2, "b": {"x": [1, 2], "y": null}}))
        );
        assert_ne!(
            a,
            args_hash("file_read", &json!({"a": 1, "b": {"x": [1, 2], "y": null}}))
        );
        assert_eq!(a.len(), 32);
    }

    #[test]
    fn scopes_are_whole_tool_cost_check_or_prefixed() {
        for ok in [
            "*",
            "documents.expensive",
            "app:a1",
            "host:example.com",
            "connection:postgres:h:5432:db",
            "table:orders",
            "target:a:event:e",
            "computer:session",
            "computer:launch:finder",
            "command:git",
            "email:example.com",
            "person:u1",
            "web:example.com:read",
            "browser:example.com",
            "owner_browser:example.com",
            "bot:b1",
        ] {
            assert!(is_grant_scope(ok), "{ok}");
        }
        for bad in ["", "app:", "once:abc", "host:a,b", "other", "table:\n"] {
            assert!(!is_grant_scope(bad), "{bad:?}");
        }
        assert!(is_token_scope("once:x") && is_token_scope("deny:x") && !is_token_scope("app:x"));
    }

    #[test]
    fn a_key_with_spaces_in_the_tool_is_refused() {
        let key = |tool: &str, scope: &str| ToolGrantKey {
            tool: tool.into(),
            scope: scope.into(),
        };
        assert!(check_key(&key("code_run", "host:a.com")).is_ok());
        assert!(check_key(&key("code run", "*")).is_err());
        assert!(check_key(&key("code_run", "bogus")).is_err());
    }

    #[test]
    fn a_sealed_answer_id_is_the_card_hash_with_the_answer_prefix() {
        let id = approval_message_id("bt_1", "h");
        let answer = sealed_approval_response_message_id(&id);
        assert_eq!(answer, format!("ar-{}", &id[3..]));
        assert_ne!(answer, approval_response_message_id(&id));
        assert!(crate::inbox::is_valid_message_id(&answer));
    }

    #[test]
    fn a_card_matches_its_id_only_with_its_own_turn_and_calls() {
        let item = |hash: &str| crate::bot_message::ApprovalItem {
            args_hash: hash.into(),
            tool: "computer_screenshot".into(),
            summary: "Look".into(),
            scope: "computer:session".into(),
            calls: 1,
            after_web_content: false,
            secret: None,
            invite: None,
            picture: None,
        };
        let single = crate::bot_message::ApprovalRequest {
            id: approval_message_id("bt_1", "h1"),
            turn_id: "bt_1".into(),
            tool: "computer_screenshot".into(),
            summary: "Look".into(),
            scope: "computer:session".into(),
            args_hash: "h1".into(),
            requester_id: None,
            requester_name: None,
            room: None,
            secret: None,
            invite: None,
            picture: None,
            configuration_changes: None,
            items: Vec::new(),
        };
        assert!(card_matches_id(&single, "bt_1"));
        assert!(!card_matches_id(&single, "bt_2"));
        let mut forged = single.clone();
        forged.args_hash = "h2".into();
        assert!(!card_matches_id(&forged, "bt_1"));

        let hash = batch_args_hash(["a", "b"]);
        let mut batch = single.clone();
        batch.args_hash = hash.clone();
        batch.id = approval_message_id("bt_1", &hash);
        batch.items = vec![item("a"), item("b")];
        assert!(card_matches_id(&batch, "bt_1"));
        batch.items.push(item("c"));
        assert!(!card_matches_id(&batch, "bt_1"));
    }

    #[test]
    fn approval_ids_are_stable_message_ids() {
        let id = approval_message_id("bt_1", "h");
        assert_eq!(id, approval_message_id("bt_1", "h"));
        assert_ne!(id, approval_message_id("bt_2", "h"));
        assert!(id.starts_with("ap-") && id.len() == 35);
        assert!(approval_response_message_id(&id).starts_with("ar-"));
    }
}
