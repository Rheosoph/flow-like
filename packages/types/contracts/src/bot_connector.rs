//! Connectors: the MCP servers and OAuth accounts a bot's owner attached to it. The settings row
//! carries policies and a snapshot of the server's tools, never a token: grants stay sealed on the
//! host that resolves them.

use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub const MAX_CONNECTORS_PER_BOT: usize = 20;
pub const MAX_LABEL_CHARS: usize = 64;
pub const MAX_URL_CHARS: usize = 2_000;
pub const MAX_TOOLS_SNAPSHOT: usize = 500;
pub const MAX_TOOL_POLICIES: usize = 500;
pub const MAX_SCOPES: usize = 50;
/// Prefix of a connector call's scope in the bot's tool grants: `connector:<id>:<tool>`.
pub const SCOPE_PREFIX: &str = "connector:";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ConnectorKind {
    /// An OAuth account reached through REST.
    Oauth,
    /// An MCP server over Streamable HTTP.
    Mcp,
}

impl ConnectorKind {
    pub fn column(self) -> &'static str {
        match self {
            Self::Oauth => "OAUTH",
            Self::Mcp => "MCP",
        }
    }

    pub fn from_column(value: &str) -> Option<Self> {
        [Self::Oauth, Self::Mcp]
            .into_iter()
            .find(|kind| kind.column().eq_ignore_ascii_case(value))
    }
}

/// What a connector call may do without asking.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ConnectorPolicy {
    /// Runs without asking.
    Read,
    /// The owner approves each call (or grants it for good).
    Ask,
    /// Never listed, never run.
    Never,
}

impl ConnectorPolicy {
    pub fn column(self) -> &'static str {
        match self {
            Self::Read => "read",
            Self::Ask => "ask",
            Self::Never => "never",
        }
    }

    pub fn from_column(value: &str) -> Option<Self> {
        [Self::Read, Self::Ask, Self::Never]
            .into_iter()
            .find(|policy| policy.column().eq_ignore_ascii_case(value))
    }
}

/// How the connector signs in.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ConnectorAuth {
    #[default]
    None,
    /// The value of the bot secret `name` is sent as the `Authorization` header (`Bearer` is
    /// added when the value has no scheme).
    Secret { name: String },
    /// The owner's OAuth grant for this provider.
    Oauth { provider_id: String },
}

/// One tool of an MCP server as the last refresh saw it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ConnectorToolInfo {
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub read_only_hint: Option<bool>,
    #[serde(default)]
    pub destructive_hint: Option<bool>,
    /// blake3 of the input schema; a change tells the owner a tool was altered.
    #[serde(default)]
    pub schema_hash: String,
}

/// A connector as its owner sees it. `has_grant` says a sealed grant is stored; the grant itself
/// never leaves the host.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct BotConnector {
    pub id: String,
    pub bot_id: String,
    pub kind: ConnectorKind,
    pub label: String,
    #[serde(default)]
    pub provider_id: Option<String>,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub auth: ConnectorAuth,
    #[serde(default)]
    pub has_grant: bool,
    #[serde(default)]
    pub scopes: Vec<String>,
    pub default_policy: ConnectorPolicy,
    #[serde(default)]
    pub tool_policies: BTreeMap<String, ConnectorPolicy>,
    #[serde(default)]
    pub tools_snapshot: Vec<ConnectorToolInfo>,
    /// The `schema_hash` each tool had when the owner last set or confirmed its policy. A tool
    /// whose schema changed since then asks every time until the owner confirms it again.
    #[serde(default)]
    pub confirmed_hashes: BTreeMap<String, String>,
    /// RFC 3339.
    pub updated_at: String,
}

impl BotConnector {
    pub fn rule(&self) -> ConnectorRule {
        ConnectorRule {
            id: self.id.clone(),
            label: self.label.clone(),
            kind: Some(self.kind),
            default_policy: Some(self.default_policy),
            tool_policies: self.tool_policies.clone(),
            tools: self.tools_snapshot.clone(),
            confirmed_hashes: self.confirmed_hashes.clone(),
        }
    }
}

/// `POST /bots/{bot_id}/connectors`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct CreateBotConnector {
    pub kind: ConnectorKind,
    pub label: String,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub auth: ConnectorAuth,
    /// The `fl_oauth_` handle (or token) of the owner's grant, for `auth: oauth`. Stored sealed;
    /// never returned.
    #[serde(default)]
    pub grant: Option<String>,
    #[serde(default)]
    pub scopes: Vec<String>,
    #[serde(default)]
    pub default_policy: Option<ConnectorPolicy>,
}

/// `PATCH /bots/{bot_id}/connectors/{connector_id}`; omitted fields stay.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct UpdateBotConnector {
    #[serde(default)]
    pub label: Option<String>,
    #[serde(default)]
    pub default_policy: Option<ConnectorPolicy>,
    #[serde(default)]
    pub tool_policies: Option<BTreeMap<String, ConnectorPolicy>>,
    #[serde(default)]
    pub grant: Option<String>,
    #[serde(default)]
    pub scopes: Option<Vec<String>>,
    /// Tools whose changed schema the owner has looked at and accepts.
    #[serde(default)]
    pub confirm_tools: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct BotConnectorList {
    pub connectors: Vec<BotConnector>,
}

/// One grant of the owner's OAuth accounts, as `GET /oauth/connections` lists it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct OAuthConnectionInfo {
    /// The handle a connector stores (`fl_oauth_…`); not a token.
    pub handle: String,
    #[serde(default)]
    pub provider_id: Option<String>,
    pub state: String,
    #[serde(default)]
    pub scopes: Vec<String>,
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct OAuthConnectionList {
    pub connections: Vec<OAuthConnectionInfo>,
}

/// What a bot run needs to know about one connector: enough for core to refuse and to decide
/// whether a call asks, nothing secret.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConnectorRule {
    pub id: String,
    pub label: String,
    pub kind: Option<ConnectorKind>,
    pub default_policy: Option<ConnectorPolicy>,
    pub tool_policies: BTreeMap<String, ConnectorPolicy>,
    pub tools: Vec<ConnectorToolInfo>,
    pub confirmed_hashes: BTreeMap<String, String>,
}

/// How one call is handled.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CallPolicy {
    Never,
    /// Runs without asking.
    Run,
    /// The owner approves the call; a standing grant may cover it.
    Ask,
    /// The owner approves each call; no standing grant covers it.
    AskEach,
}

/// The HTTP methods an OAuth REST connector may call without asking.
pub fn is_read_method(method: &str) -> bool {
    matches!(method.to_ascii_uppercase().as_str(), "GET" | "HEAD")
}

impl ConnectorRule {
    fn tool(&self, name: &str) -> Option<&ConnectorToolInfo> {
        self.tools.iter().find(|tool| tool.name == name)
    }

    /// The owner's choice for one MCP tool, else the hints, else the connector's default.
    pub fn tool_policy(&self, name: &str) -> ConnectorPolicy {
        if let Some(policy) = self.tool_policies.get(name) {
            return *policy;
        }
        match self.tool(name) {
            Some(tool) if tool.destructive_hint == Some(true) => ConnectorPolicy::Ask,
            Some(tool) if tool.read_only_hint == Some(true) => ConnectorPolicy::Read,
            _ => self.default_policy.unwrap_or(ConnectorPolicy::Ask),
        }
    }

    /// Whether the tool's schema changed since the owner set or confirmed its policy.
    pub fn drifted(&self, name: &str) -> bool {
        match (self.confirmed_hashes.get(name), self.tool(name)) {
            (Some(confirmed), Some(tool)) => {
                !tool.schema_hash.is_empty() && *confirmed != tool.schema_hash
            }
            _ => false,
        }
    }

    pub fn tool_is_destructive(&self, name: &str) -> bool {
        self.tool(name)
            .is_some_and(|tool| tool.destructive_hint == Some(true))
    }

    /// REST: the owner's choice for the pseudo-tools `read` (GET, HEAD) and `write` (the rest).
    pub fn request_policy(&self, method: &str) -> ConnectorPolicy {
        let read = is_read_method(method);
        let pseudo = if read { "read" } else { "write" };
        if let Some(policy) = self.tool_policies.get(pseudo) {
            return *policy;
        }
        if read {
            ConnectorPolicy::Read
        } else {
            ConnectorPolicy::Ask
        }
    }

    pub fn call_policy(&self, tool: &str) -> CallPolicy {
        match self.tool_policy(tool) {
            ConnectorPolicy::Never => CallPolicy::Never,
            _ if self.drifted(tool) => CallPolicy::AskEach,
            ConnectorPolicy::Ask if self.tool_is_destructive(tool) => CallPolicy::AskEach,
            ConnectorPolicy::Ask => CallPolicy::Ask,
            ConnectorPolicy::Read if self.tool_is_destructive(tool) => CallPolicy::Ask,
            ConnectorPolicy::Read => CallPolicy::Run,
        }
    }

    pub fn request_call_policy(&self, method: &str) -> CallPolicy {
        match self.request_policy(method) {
            ConnectorPolicy::Never => CallPolicy::Never,
            ConnectorPolicy::Ask => CallPolicy::Ask,
            ConnectorPolicy::Read if is_read_method(method) => CallPolicy::Run,
            ConnectorPolicy::Read => CallPolicy::Ask,
        }
    }
}

/// The hosts a provider's token may go to when the hub configures none. Exact names or `*.suffix`.
pub fn default_api_hosts(provider_id: &str) -> &'static [&'static str] {
    match provider_id {
        "google" => &[
            "www.googleapis.com",
            "gmail.googleapis.com",
            "people.googleapis.com",
            "sheets.googleapis.com",
            "docs.googleapis.com",
            "slides.googleapis.com",
            "forms.googleapis.com",
            "tasks.googleapis.com",
            "meet.googleapis.com",
            "youtube.googleapis.com",
        ],
        "microsoft" => &["graph.microsoft.com"],
        "github" => &["api.github.com", "uploads.github.com"],
        "notion" => &["api.notion.com"],
        "atlassian" => &["api.atlassian.com"],
        "linkedin" => &["api.linkedin.com"],
        _ => &[],
    }
}

/// The hub's configured hosts, else the built-in list for the provider.
pub fn api_hosts_for(provider_id: &str, configured: &[String]) -> Vec<String> {
    if !configured.is_empty() {
        return configured.to_vec();
    }
    default_api_hosts(provider_id)
        .iter()
        .map(|host| host.to_string())
        .collect()
}

/// Checks a label; the message is for the owner.
pub fn validate_label(label: &str) -> Result<String, String> {
    let label = label.trim();
    if label.is_empty() || label.chars().count() > MAX_LABEL_CHARS {
        return Err(format!("A label has 1 to {MAX_LABEL_CHARS} characters"));
    }
    if label.chars().any(char::is_control) {
        return Err("A label has no control characters".to_string());
    }
    Ok(label.to_string())
}

/// Tool names an owner sets a policy for: `read` and `write` for REST accounts, any MCP tool name.
pub fn validate_tool_policies(policies: &BTreeMap<String, ConnectorPolicy>) -> Result<(), String> {
    if policies.len() > MAX_TOOL_POLICIES {
        return Err(format!("At most {MAX_TOOL_POLICIES} tool policies"));
    }
    let bad = |name: &String| {
        name.is_empty() || name.chars().count() > 200 || name.chars().any(char::is_control)
    };
    if policies.keys().any(bad) {
        return Err("A tool name has 1 to 200 characters and no control characters".to_string());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tool(name: &str, read_only: Option<bool>, destructive: Option<bool>) -> ConnectorToolInfo {
        ConnectorToolInfo {
            name: name.to_string(),
            description: String::new(),
            read_only_hint: read_only,
            destructive_hint: destructive,
            schema_hash: String::new(),
        }
    }

    fn rule() -> ConnectorRule {
        ConnectorRule {
            id: "c1".into(),
            label: "Docs".into(),
            kind: Some(ConnectorKind::Mcp),
            default_policy: None,
            tool_policies: BTreeMap::new(),
            confirmed_hashes: BTreeMap::new(),
            tools: vec![
                tool("search", Some(true), None),
                tool("drop", None, Some(true)),
                tool("create", None, None),
                tool("odd", Some(true), Some(true)),
            ],
        }
    }

    #[test]
    fn a_changed_schema_asks_every_time_until_confirmed() {
        let mut rule = rule();
        rule.tools[0].schema_hash = "v2".into();
        rule.tool_policies
            .insert("search".into(), ConnectorPolicy::Read);
        rule.confirmed_hashes.insert("search".into(), "v1".into());
        assert_eq!(rule.call_policy("search"), CallPolicy::AskEach);
        rule.confirmed_hashes.insert("search".into(), "v2".into());
        assert_eq!(rule.call_policy("search"), CallPolicy::Run);
        rule.confirmed_hashes.insert("search".into(), "v1".into());
        rule.tool_policies
            .insert("search".into(), ConnectorPolicy::Never);
        assert_eq!(rule.call_policy("search"), CallPolicy::Never);
    }

    #[test]
    fn hosts_come_from_the_hub_else_from_the_built_in_list() {
        assert_eq!(
            api_hosts_for("github", &[]),
            ["api.github.com", "uploads.github.com"]
        );
        assert_eq!(
            api_hosts_for("github", &["ghe.corp".to_string()]),
            ["ghe.corp"]
        );
        assert!(api_hosts_for("unknown", &[]).is_empty());
    }

    #[test]
    fn hints_pick_the_default_policy() {
        let rule = rule();
        assert_eq!(rule.call_policy("search"), CallPolicy::Run);
        assert_eq!(rule.call_policy("create"), CallPolicy::Ask);
        assert_eq!(rule.call_policy("unlisted"), CallPolicy::Ask);
        assert_eq!(rule.call_policy("drop"), CallPolicy::AskEach);
        assert_eq!(rule.call_policy("odd"), CallPolicy::AskEach);
    }

    #[test]
    fn an_owner_choice_wins_but_destructive_tools_still_ask() {
        let mut rule = rule();
        rule.tool_policies
            .insert("create".into(), ConnectorPolicy::Read);
        rule.tool_policies
            .insert("search".into(), ConnectorPolicy::Never);
        rule.tool_policies
            .insert("drop".into(), ConnectorPolicy::Read);
        assert_eq!(rule.call_policy("create"), CallPolicy::Run);
        assert_eq!(rule.call_policy("search"), CallPolicy::Never);
        assert_eq!(rule.call_policy("drop"), CallPolicy::Ask);
    }

    #[test]
    fn rest_reads_run_and_writes_ask() {
        let mut rule = rule();
        assert_eq!(rule.request_call_policy("get"), CallPolicy::Run);
        assert_eq!(rule.request_call_policy("HEAD"), CallPolicy::Run);
        assert_eq!(rule.request_call_policy("POST"), CallPolicy::Ask);
        rule.tool_policies
            .insert("write".into(), ConnectorPolicy::Never);
        assert_eq!(rule.request_call_policy("DELETE"), CallPolicy::Never);
        rule.tool_policies
            .insert("write".into(), ConnectorPolicy::Read);
        assert_eq!(rule.request_call_policy("DELETE"), CallPolicy::Ask);
        rule.tool_policies
            .insert("read".into(), ConnectorPolicy::Never);
        assert_eq!(rule.request_call_policy("GET"), CallPolicy::Never);
    }
}
