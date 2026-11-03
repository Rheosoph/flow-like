//! Standing tool approvals for bots. A call that asks the owner is first matched against the
//! bot's grants (`(tool, scope)` pairs kept by the host's [`GrantStore`]): a grant runs the call
//! without asking, on every host. A headless turn with no grant parks: it posts an approval card
//! and sleeps until the owner answers; a one-time answer leaves a token for the resumed run.
//!
//! [`grant_scopes`] is the one scope rule. Interactive chats use it too, so the app's
//! "Always allow" and a headless turn's grant mean the same thing:
//!
//! | tool call | scope |
//! |---|---|
//! | `call_app_event`, `call_app_chat`, `database_tool` (not `delete_table`) | `app:<app_id>` (argument, else the chat's app) |
//! | `database_tool` `delete_table` | `table:<table_name>` |
//! | `knowledge_drop` | `table:<index>` |
//! | `interact_app_page` | `target:<app_id>:event:<id>` or `target:<app_id>:page:<id>` |
//! | `code_run` with `network_hosts` | one `host:<h>` per host |
//! | `shell_run` | `command:<program basename>`; `shell: true` is `command:sh` |
//! | `data_sources` attach | `connection:<kind>:<host>:<port>:<database>` |
//! | `http_request`, `forecast_tool`, `flowpilot_board_review`, `app_build`, `apply_home_layout` | none: asked every time |
//! | every other tool that asks | `*` |
//!
//! The document cost check is the grant `(documents, documents.expensive)`.

use async_trait::async_trait;
use flow_like_types::bot_message::ApprovalRequest;
use flow_like_types::tool_grant::{
    ApprovalDecision, DENY_PREFIX, DOCUMENTS_TOOL, ONCE_PREFIX, SCOPE_ALL,
    SCOPE_DOCUMENTS_EXPENSIVE, args_hash,
};
use serde_json::Value;

use flow_like_bot_tools as tools;

use super::tool_spec::spec_arg_str;

const NOT_GRANTABLE: [&str; 5] = [
    "http_request",
    "forecast_tool",
    "flowpilot_board_review",
    "app_build",
    "apply_home_layout",
];

/// One stored `(tool, scope)` pair.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct GrantKey {
    pub tool: String,
    pub scope: String,
}

impl GrantKey {
    pub fn new(tool: impl Into<String>, scope: impl Into<String>) -> Self {
        Self {
            tool: tool.into(),
            scope: scope.into(),
        }
    }
}

/// Where a bot's grants and answer tokens live: the account API on the server, the local store
/// on a desktop. `list` leaves out tokens older than a day.
#[async_trait]
pub trait GrantStore: Send + Sync {
    async fn list(&self) -> flow_like_types::Result<Vec<GrantKey>>;
    async fn put(&self, tool: &str, scope: &str) -> flow_like_types::Result<()>;
    /// Removes a pair; true when it existed.
    async fn delete(&self, tool: &str, scope: &str) -> flow_like_types::Result<bool>;

    async fn has(&self, tool: &str, scope: &str) -> flow_like_types::Result<bool> {
        Ok(self
            .list()
            .await?
            .iter()
            .any(|key| key.tool == tool && key.scope == scope))
    }
}

fn clean(value: &str) -> Option<String> {
    let value = value.trim();
    let usable = !value.is_empty()
        && value.chars().count() <= 200
        && !value.chars().any(|char| char.is_control() || char == ',');
    usable.then(|| value.to_string())
}

fn arg<'a>(args: &'a Value, snake: &str, camel: &str) -> &'a str {
    spec_arg_str(args, snake, camel)
}

fn app_id(args: &Value, context_app: Option<&str>) -> Option<String> {
    clean(arg(args, "app_id", "appId")).or_else(|| context_app.and_then(clean))
}

fn network_hosts(args: &Value) -> Vec<String> {
    let mut hosts: Vec<String> = args
        .get("network_hosts")
        .and_then(Value::as_array)
        .map(|hosts| {
            hosts
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_lowercase)
                .filter_map(|host| clean(&host))
                .collect()
        })
        .unwrap_or_default();
    hosts.sort();
    hosts.dedup();
    hosts
}

fn connection_scope(args: &Value) -> Option<String> {
    let attach = args.get("attach")?;
    let text = |value: Option<&Value>| value.and_then(Value::as_str).and_then(clean);
    let connection = attach.get("connection")?;
    let port = match connection.get("port") {
        Some(Value::String(text)) => text.trim().to_string(),
        Some(Value::Null) | None => String::new(),
        Some(other) => other.to_string(),
    };
    Some(format!(
        "connection:{}:{}:{}:{}",
        text(attach.get("kind"))?,
        text(connection.get("host"))?,
        clean(&port).unwrap_or_default(),
        text(connection.get("database"))?,
    ))
}

/// The grant scopes that cover one call of `tool`, or `None` when the call is never remembered.
/// A call is covered when every scope is granted (or the tool is granted for `*`).
pub fn grant_scopes(tool: &str, args: &Value, context_app: Option<&str>) -> Option<Vec<String>> {
    if NOT_GRANTABLE.contains(&tool) {
        return None;
    }
    let one = |scope: Option<String>| scope.map(|scope| vec![scope]);
    match tool {
        "call_app_event" | "call_app_chat" => {
            one(app_id(args, context_app).map(|id| format!("app:{id}")))
        }
        "database_tool" if arg(args, "operation", "operation") == "delete_table" => {
            one(clean(arg(args, "table_name", "tableName")).map(|name| format!("table:{name}")))
        }
        "database_tool" => one(app_id(args, context_app).map(|id| format!("app:{id}"))),
        "knowledge_drop" => {
            one(clean(arg(args, "index", "index")).map(|name| format!("table:{name}")))
        }
        "interact_app_page" => {
            let app = app_id(args, context_app)?;
            let (kind, id) = match clean(arg(args, "event_id", "eventId")) {
                Some(event) => ("event", event),
                None => ("page", clean(arg(args, "page_id", "pageId"))?),
            };
            Some(vec![format!("target:{app}:{kind}:{id}")])
        }
        "code_run" => {
            let hosts = network_hosts(args);
            (!hosts.is_empty()).then(|| {
                hosts
                    .into_iter()
                    .map(|host| format!("host:{host}"))
                    .collect()
            })
        }
        "data_sources" => one(connection_scope(args)),
        "browser_open" => one(clean(arg(args, "url", "url"))
            .and_then(|url| tools::browser::site_of(&url))
            .map(|site| format!("browser:{site}"))),
        "computer_screenshot" | "computer_act" => Some(vec!["computer:session".to_string()]),
        "email_send" => flow_like_bot_tools::email::grant_scopes(args),
        "shell_run" => one(flow_like_bot_tools::shell::grant_scope(args).and_then(|s| clean(&s))),
        _ => Some(vec![SCOPE_ALL.to_string()]),
    }
}

fn token_scope(prefix: &str, hash: &str) -> String {
    format!("{prefix}{hash}")
}

/// What the stores say about one call that asks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GrantCheck {
    /// A standing grant covers it.
    Granted,
    /// The owner allowed this exact call once; the token is spent.
    Once,
    /// The owner refused this exact call; the token is spent.
    Denied,
    /// Nothing covers it: ask.
    Missing,
}

impl GrantCheck {
    pub fn runs(self) -> bool {
        matches!(self, Self::Granted | Self::Once)
    }
}

/// Matches one call against the bot's grants and answer tokens. A refusal token wins; a spent
/// token cannot be used twice.
pub async fn check_call(
    store: &dyn GrantStore,
    tool: &str,
    args: &Value,
    context_app: Option<&str>,
) -> flow_like_types::Result<GrantCheck> {
    let hash = args_hash(tool, args);
    if store.delete(tool, &token_scope(DENY_PREFIX, &hash)).await? {
        return Ok(GrantCheck::Denied);
    }
    if let Some(scopes) = grant_scopes(tool, args, context_app) {
        let keys = store.list().await?;
        let held = |scope: &str| {
            keys.iter()
                .any(|key| key.tool == tool && key.scope == scope)
        };
        if held(SCOPE_ALL) || scopes.iter().all(|scope| held(scope)) {
            return Ok(GrantCheck::Granted);
        }
    }
    if store.delete(tool, &token_scope(ONCE_PREFIX, &hash)).await? {
        return Ok(GrantCheck::Once);
    }
    Ok(GrantCheck::Missing)
}

/// Whether the owner gave standing approval for expensive document work.
pub async fn documents_expensive_granted(store: &dyn GrantStore) -> flow_like_types::Result<bool> {
    store.has(DOCUMENTS_TOOL, SCOPE_DOCUMENTS_EXPENSIVE).await
}

async fn store_decision(
    store: &dyn GrantStore,
    tool: &str,
    hash: &str,
    scopes: &[String],
    decision: ApprovalDecision,
) -> flow_like_types::Result<()> {
    match decision {
        ApprovalDecision::Deny => store.put(tool, &token_scope(DENY_PREFIX, hash)).await,
        ApprovalDecision::Once => store.put(tool, &token_scope(ONCE_PREFIX, hash)).await,
        ApprovalDecision::Always => {
            for scope in scopes {
                let grant_tool = if scope == SCOPE_DOCUMENTS_EXPENSIVE {
                    DOCUMENTS_TOOL
                } else {
                    tool
                };
                store.put(grant_tool, scope).await?;
            }
            store.put(tool, &token_scope(ONCE_PREFIX, hash)).await
        }
    }
}

/// Stores what the owner answered to a call: a grant for "always" (a call that is never
/// remembered stays a one-time answer), and the token the resumed run spends.
pub async fn record_answer(
    store: &dyn GrantStore,
    tool: &str,
    args: &Value,
    context_app: Option<&str>,
    decision: ApprovalDecision,
) -> flow_like_types::Result<()> {
    let scopes = grant_scopes(tool, args, context_app).unwrap_or_default();
    store_decision(store, tool, &args_hash(tool, args), &scopes, decision).await
}

/// [`record_answer`] for an answer to a posted card, which carries the scopes and the call's hash
/// but not its arguments.
pub async fn record_card_answer(
    store: &dyn GrantStore,
    card: &ApprovalRequest,
    decision: ApprovalDecision,
) -> flow_like_types::Result<()> {
    let scopes: Vec<String> = card
        .scope
        .split(',')
        .filter(|scope| !scope.is_empty())
        .map(str::to_string)
        .collect();
    store_decision(store, &card.tool, &card.args_hash, &scopes, decision).await
}

/// Spends the one-time token for exactly this call, if the owner left one.
pub async fn take_once(
    store: &dyn GrantStore,
    tool: &str,
    args: &Value,
) -> flow_like_types::Result<bool> {
    store
        .delete(tool, &token_scope(ONCE_PREFIX, &args_hash(tool, args)))
        .await
}

/// The card for a document call above the cost limits.
pub fn expensive_documents_request(
    turn_id: &str,
    tool: &str,
    args: &Value,
    message: &str,
) -> ApprovalRequest {
    let mut card = approval_request(turn_id, tool, args, None, message);
    card.scope = SCOPE_DOCUMENTS_EXPENSIVE.to_string();
    card
}

/// The card a parked turn posts for one call.
pub fn approval_request(
    turn_id: &str,
    tool: &str,
    args: &Value,
    context_app: Option<&str>,
    description: &str,
) -> ApprovalRequest {
    let hash = args_hash(tool, args);
    let summary: String = description
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or(tool)
        .chars()
        .take(480)
        .collect();
    ApprovalRequest {
        id: flow_like_types::tool_grant::approval_message_id(turn_id, &hash),
        turn_id: turn_id.to_string(),
        tool: tool.to_string(),
        summary,
        scope: grant_scopes(tool, args, context_app)
            .map(|scopes| scopes.join(","))
            .unwrap_or_default(),
        args_hash: hash,
        requester_id: None,
        requester_name: None,
        room: None,
    }
}

/// What a headless host does with a call that asks the owner.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HeadlessAnswer {
    Run,
    Refuse(String),
    Park(ApprovalRequest),
}

pub async fn headless_answer(
    store: &dyn GrantStore,
    turn_id: &str,
    tool: &str,
    args: &Value,
    context_app: Option<&str>,
    description: &str,
) -> HeadlessAnswer {
    match check_call(store, tool, args, context_app).await {
        Ok(check) if check.runs() => HeadlessAnswer::Run,
        Ok(GrantCheck::Denied) => HeadlessAnswer::Refuse(format!(
            "The owner refused this {tool} call. Do not try it again; tell them what you could not do."
        )),
        Ok(_) => HeadlessAnswer::Park(approval_request(
            turn_id,
            tool,
            args,
            context_app,
            description,
        )),
        Err(error) => HeadlessAnswer::Refuse(format!(
            "The owner's standing approvals could not be read ({error}). Nothing was run."
        )),
    }
}

/// The note a resumed run starts with: which calls the owner answered while the turn slept.
pub async fn resume_note(store: &dyn GrantStore) -> Option<String> {
    let keys = store.list().await.ok()?;
    let mut allowed = Vec::new();
    let mut refused = Vec::new();
    for key in &keys {
        if key.scope.starts_with(ONCE_PREFIX) {
            allowed.push(key.tool.as_str());
        } else if key.scope.starts_with(DENY_PREFIX) {
            refused.push(key.tool.as_str());
        }
    }
    allowed.dedup();
    refused.dedup();
    if allowed.is_empty() && refused.is_empty() {
        return None;
    }
    let mut note = String::from(
        "Earlier in this turn you asked your owner to approve a call and the turn paused for the answer.",
    );
    if !allowed.is_empty() {
        note.push_str(&format!(
            " They approved: {}. Make the same call again with identical arguments; it runs without asking.",
            allowed.join(", ")
        ));
    }
    if !refused.is_empty() {
        note.push_str(&format!(
            " They refused: {}. Do not call it again; tell them what you could not do.",
            refused.join(", ")
        ));
    }
    Some(note)
}

/// Removes the answer tokens a resumed run started with, whether or not it spent them.
pub async fn clear_tokens(store: &dyn GrantStore, seen: &[GrantKey]) {
    for key in seen {
        if key.scope.starts_with(ONCE_PREFIX) || key.scope.starts_with(DENY_PREFIX) {
            let _ = store.delete(&key.tool, &key.scope).await;
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn email_send_is_granted_per_recipient_domain() {
        let args = json!({"to": ["a@b.com"], "cc": ["c@d.org"], "subject": "s", "body": "x"});
        assert_eq!(
            grant_scopes("email_send", &args, None),
            Some(vec!["email:b.com".to_string(), "email:d.org".to_string()])
        );
        assert_eq!(
            grant_scopes("email_send", &json!({"to": ["nope"]}), None),
            None
        );
    }

    use super::*;
    use serde_json::json;
    use std::sync::Mutex;

    #[derive(Default)]
    struct Memory(Mutex<Vec<GrantKey>>);

    #[async_trait]
    impl GrantStore for Memory {
        async fn list(&self) -> flow_like_types::Result<Vec<GrantKey>> {
            Ok(self.0.lock().unwrap().clone())
        }
        async fn put(&self, tool: &str, scope: &str) -> flow_like_types::Result<()> {
            let key = GrantKey::new(tool, scope);
            let mut keys = self.0.lock().unwrap();
            if !keys.contains(&key) {
                keys.push(key);
            }
            Ok(())
        }
        async fn delete(&self, tool: &str, scope: &str) -> flow_like_types::Result<bool> {
            let mut keys = self.0.lock().unwrap();
            let before = keys.len();
            keys.retain(|key| !(key.tool == tool && key.scope == scope));
            Ok(keys.len() != before)
        }
    }

    #[test]
    fn app_tools_scope_to_their_app_and_fall_back_to_the_chats_app() {
        let scopes = |tool, args: Value, ctx| grant_scopes(tool, &args, ctx);
        assert_eq!(
            scopes("call_app_event", json!({"app_id": "a1"}), None),
            Some(vec!["app:a1".into()])
        );
        assert_eq!(
            scopes("call_app_chat", json!({"appId": "a2"}), Some("ctx")),
            Some(vec!["app:a2".into()])
        );
        assert_eq!(
            scopes("call_app_event", json!({}), Some("ctx")),
            Some(vec!["app:ctx".into()])
        );
        assert_eq!(scopes("call_app_event", json!({}), None), None);
        assert_eq!(
            scopes(
                "database_tool",
                json!({"operation": "insert", "app_id": "w"}),
                None
            ),
            Some(vec!["app:w".into()])
        );
    }

    #[test]
    fn drops_scope_to_the_table_or_index() {
        assert_eq!(
            grant_scopes(
                "database_tool",
                &json!({"operation": "delete_table", "table_name": "orders"}),
                Some("w")
            ),
            Some(vec!["table:orders".into()])
        );
        assert_eq!(
            grant_scopes("knowledge_drop", &json!({"index": "notes"}), None),
            Some(vec!["table:notes".into()])
        );
        assert_eq!(grant_scopes("knowledge_drop", &json!({}), None), None);
    }

    #[test]
    fn outside_access_scopes_to_each_destination() {
        assert_eq!(
            grant_scopes(
                "code_run",
                &json!({"network_hosts": ["B.com", "a.com", "b.com"]}),
                None
            ),
            Some(vec!["host:a.com".into(), "host:b.com".into()])
        );
        assert_eq!(
            grant_scopes("code_run", &json!({"network_hosts": []}), None),
            None
        );
        let attach = json!({"attach": {"kind": "postgres", "connection": {"host": "db", "port": 5432, "database": "shop"}}});
        assert_eq!(
            grant_scopes("data_sources", &attach, None),
            Some(vec!["connection:postgres:db:5432:shop".into()])
        );
        assert_eq!(
            grant_scopes(
                "data_sources",
                &json!({"attach": {"kind": "postgres"}}),
                None
            ),
            None
        );
    }

    #[test]
    fn a_browser_grant_is_one_site_and_the_computer_grant_is_the_session() {
        assert_eq!(
            grant_scopes(
                "browser_open",
                &json!({"url": "https://www.Shop.example/cart?x=1"}),
                None
            ),
            Some(vec!["browser:shop.example".into()])
        );
        assert_eq!(
            grant_scopes("browser_open", &json!({"url": "javascript:alert(1)"}), None),
            None
        );
        assert_eq!(grant_scopes("browser_open", &json!({}), None), None);
        for tool in ["computer_act", "computer_screenshot"] {
            assert_eq!(
                grant_scopes(tool, &json!({"action": "click"}), None),
                Some(vec!["computer:session".into()])
            );
        }
    }

    #[test]
    fn page_targets_need_an_app_and_a_target_and_some_tools_are_never_remembered() {
        assert_eq!(
            grant_scopes(
                "interact_app_page",
                &json!({"app_id": "a", "event_id": "e"}),
                None
            ),
            Some(vec!["target:a:event:e".into()])
        );
        assert_eq!(
            grant_scopes("interact_app_page", &json!({"page_id": "p"}), Some("a")),
            Some(vec!["target:a:page:p".into()])
        );
        assert_eq!(
            grant_scopes("interact_app_page", &json!({"app_id": "a"}), None),
            None
        );
        for tool in NOT_GRANTABLE {
            assert_eq!(grant_scopes(tool, &json!({}), Some("a")), None, "{tool}");
        }
        assert_eq!(
            grant_scopes("file_write", &json!({}), None),
            Some(vec!["*".into()])
        );
    }

    #[tokio::test]
    async fn always_stores_a_grant_and_the_next_call_does_not_ask() {
        let store = Memory::default();
        let args = json!({"app_id": "a1", "event_id": "x"});
        assert_eq!(
            check_call(&store, "call_app_event", &args, None)
                .await
                .unwrap(),
            GrantCheck::Missing
        );
        record_answer(
            &store,
            "call_app_event",
            &args,
            None,
            ApprovalDecision::Always,
        )
        .await
        .unwrap();
        let other = json!({"app_id": "a1", "event_id": "y"});
        assert_eq!(
            check_call(&store, "call_app_event", &other, None)
                .await
                .unwrap(),
            GrantCheck::Granted
        );
        let elsewhere = json!({"app_id": "a2"});
        assert_eq!(
            check_call(&store, "call_app_event", &elsewhere, None)
                .await
                .unwrap(),
            GrantCheck::Missing
        );
    }

    #[tokio::test]
    async fn a_grant_for_every_host_is_needed_to_cover_a_call_with_two() {
        let store = Memory::default();
        store.put("code_run", "host:a.com").await.unwrap();
        let both = json!({"network_hosts": ["a.com", "b.com"]});
        assert_eq!(
            check_call(&store, "code_run", &both, None).await.unwrap(),
            GrantCheck::Missing
        );
        store.put("code_run", "host:b.com").await.unwrap();
        assert_eq!(
            check_call(&store, "code_run", &both, None).await.unwrap(),
            GrantCheck::Granted
        );
    }

    #[tokio::test]
    async fn a_one_time_answer_runs_one_call_and_a_refusal_blocks_one() {
        let store = Memory::default();
        let args = json!({"index": "notes"});
        record_answer(
            &store,
            "knowledge_drop",
            &args,
            None,
            ApprovalDecision::Once,
        )
        .await
        .unwrap();
        assert_eq!(
            check_call(&store, "knowledge_drop", &args, None)
                .await
                .unwrap(),
            GrantCheck::Once
        );
        assert_eq!(
            check_call(&store, "knowledge_drop", &args, None)
                .await
                .unwrap(),
            GrantCheck::Missing
        );
        record_answer(
            &store,
            "knowledge_drop",
            &args,
            None,
            ApprovalDecision::Deny,
        )
        .await
        .unwrap();
        assert_eq!(
            check_call(&store, "knowledge_drop", &args, None)
                .await
                .unwrap(),
            GrantCheck::Denied
        );
        assert_eq!(
            check_call(&store, "knowledge_drop", &args, None)
                .await
                .unwrap(),
            GrantCheck::Missing
        );
    }

    #[tokio::test]
    async fn headless_runs_with_a_grant_and_parks_without() {
        let store = Memory::default();
        let args = json!({"index": "notes"});
        let HeadlessAnswer::Park(card) = headless_answer(
            &store,
            "bt_1",
            "knowledge_drop",
            &args,
            None,
            "Drops 'notes'.\nMore",
        )
        .await
        else {
            panic!("expected a park");
        };
        assert_eq!(
            (
                card.tool.as_str(),
                card.scope.as_str(),
                card.summary.as_str()
            ),
            ("knowledge_drop", "table:notes", "Drops 'notes'.")
        );
        assert_eq!(card.turn_id, "bt_1");
        assert_eq!(
            card.id,
            flow_like_types::tool_grant::approval_message_id("bt_1", &card.args_hash)
        );
        store.put("knowledge_drop", "table:notes").await.unwrap();
        assert_eq!(
            headless_answer(&store, "bt_1", "knowledge_drop", &args, None, "").await,
            HeadlessAnswer::Run
        );
    }

    #[tokio::test]
    async fn shell_grants_are_per_program_and_a_headless_turn_parks_for_a_new_one() {
        let store = Memory::default();
        let git = json!({"command": "/usr/bin/git", "args": ["status"]});
        let curl = json!({"command": "curl", "args": ["https://example.com"]});
        let script = json!({"command": "git status | wc -l", "shell": true});
        assert_eq!(
            grant_scopes("shell_run", &git, None),
            Some(vec!["command:git".into()])
        );
        assert_eq!(
            grant_scopes("shell_run", &script, None),
            Some(vec!["command:sh".into()])
        );
        let HeadlessAnswer::Park(card) =
            headless_answer(&store, "bt_1", "shell_run", &git, None, "Runs git.").await
        else {
            panic!("expected a park");
        };
        assert_eq!(
            (card.tool.as_str(), card.scope.as_str()),
            ("shell_run", "command:git")
        );
        store.put("shell_run", "command:git").await.unwrap();
        assert_eq!(
            headless_answer(&store, "bt_1", "shell_run", &git, None, "").await,
            HeadlessAnswer::Run
        );
        for other in [&curl, &script] {
            assert!(matches!(
                headless_answer(&store, "bt_1", "shell_run", other, None, "").await,
                HeadlessAnswer::Park(_)
            ));
        }
    }

    #[tokio::test]
    async fn headless_tells_the_model_about_a_refusal() {
        let store = Memory::default();
        let args = json!({"index": "notes"});
        record_answer(
            &store,
            "knowledge_drop",
            &args,
            None,
            ApprovalDecision::Deny,
        )
        .await
        .unwrap();
        let HeadlessAnswer::Refuse(message) =
            headless_answer(&store, "bt_1", "knowledge_drop", &args, None, "").await
        else {
            panic!("expected a refusal");
        };
        assert!(message.contains("refused"));
    }

    #[tokio::test]
    async fn the_resume_note_names_answered_calls_and_tokens_are_cleared() {
        let store = Memory::default();
        assert_eq!(resume_note(&store).await, None);
        record_answer(
            &store,
            "code_run",
            &json!({"network_hosts": ["a.com"]}),
            None,
            ApprovalDecision::Once,
        )
        .await
        .unwrap();
        record_answer(
            &store,
            "knowledge_drop",
            &json!({"index": "n"}),
            None,
            ApprovalDecision::Deny,
        )
        .await
        .unwrap();
        let note = resume_note(&store).await.unwrap();
        assert!(note.contains("approved: code_run") && note.contains("refused: knowledge_drop"));
        let seen = store.list().await.unwrap();
        clear_tokens(&store, &seen).await;
        assert!(store.list().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn the_cost_check_is_its_own_grant() {
        let store = Memory::default();
        assert!(!documents_expensive_granted(&store).await.unwrap());
        store.put("documents", "documents.expensive").await.unwrap();
        assert!(documents_expensive_granted(&store).await.unwrap());
    }
}
