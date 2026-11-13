//! Standing tool approvals for bots. A call that asks the owner is first matched against the
//! bot's grants (`(tool, scope)` pairs kept by the host's [`GrantStore`]): a grant runs the call
//! without asking, on every host. A headless turn can infer authorization for a concrete call
//! from the owner's instructions. Otherwise it posts an approval card and sleeps until the
//! owner answers; a one-time answer leaves a token for the resumed run.
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
//! | `email_send` | one `email:<domain>` per recipient domain |
//! | `message_person` | `person:<id>` |
//! | `data_sources` attach | `connection:<kind>:<host>:<port>:<database>` |
//! | `web_request` | `web:<site>:read` or `web:<site>:write`, plus `web:<site>:secret:<NAME>` per secret sent |
//! | `file_save` with `url` | `web:<site>:read` |
//! | `browser_open` | `browser:<site>`; in the owner's own browser `owner_browser:<site>` |
//! | `http_request`, `forecast_tool`, `flowpilot_board_review`, `app_build`, `apply_home_layout`, `bot_update_configuration`, `create_bot`, `update_bot`, `delete_bot`, `secret_request`, `room_invite` | none: asked every time |
//! | every other tool that asks | `*` |
//!
//! The document cost check is the grant `(documents, documents.expensive)`.
//!
//! Once a round begins with web content in the model's context, standing grants do not cover a
//! call whose scopes name a web destination ([`web_scoped`]): it asks again. A headless attempt
//! collects every call of its round that asks into one [`ParkedBatch`] card.

use async_trait::async_trait;
use flow_like_types::bot_message::{
    APPROVAL_BATCH_TOOL, ApprovalItem, ApprovalRequest, ApprovalResponse, MAX_APPROVAL_ITEMS,
    ROOM_INVITE_TOOL, SECRET_REQUEST_TOOL, SecretAsk,
};
use flow_like_types::tool_grant::{
    ApprovalAnswer, ApprovalDecision, DENY_PREFIX, DOCUMENTS_TOOL, ONCE_PREFIX, SCOPE_ALL,
    SCOPE_DOCUMENTS_EXPENSIVE, approval_message_id, args_hash, batch_args_hash,
};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::{Mutex, PoisonError};

use flow_like_bot_tools as tools;

use super::tool_authorization::AuthorizationInference;
use super::tool_spec::spec_arg_str;

const NOT_GRANTABLE: [&str; 11] = [
    "http_request",
    "forecast_tool",
    "flowpilot_board_review",
    "app_build",
    "apply_home_layout",
    SECRET_REQUEST_TOOL,
    ROOM_INVITE_TOOL,
    tools::configuration::BOT_UPDATE_CONFIGURATION,
    tools::crew::CREATE_BOT,
    tools::crew::UPDATE_BOT,
    tools::crew::DELETE_BOT,
];

const CONFIGURATION_TOOLS: [&str; 4] = [
    tools::configuration::BOT_UPDATE_CONFIGURATION,
    tools::crew::CREATE_BOT,
    tools::crew::UPDATE_BOT,
    tools::crew::DELETE_BOT,
];

const WEB_SCOPE_PREFIXES: [&str; 3] = ["web:", "browser:", "owner_browser:"];

/// The `asks_again` reason a live chat's approval prompt carries when [`grant_set_aside`] holds:
/// the prompt offers no "don't ask again".
pub const ASKS_AGAIN_WEB_CONTENT: &str = "web_content";

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

    /// Records that the bot just relied on these standing grants, which moves their expiry
    /// forward. It never creates a grant and never revives one that lapsed or was revoked.
    async fn touch(&self, _keys: &[GrantKey]) -> flow_like_types::Result<()> {
        Ok(())
    }

    async fn has(&self, tool: &str, scope: &str) -> flow_like_types::Result<bool> {
        let held = self
            .list()
            .await?
            .iter()
            .any(|key| key.tool == tool && key.scope == scope);
        if held {
            let _ = self.touch(&[GrantKey::new(tool, scope)]).await;
        }
        Ok(held)
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
        "connector_call" | "connector_request" => {
            one(flow_like_bot_tools::services::grant_scope(tool, args))
        }
        "browser_open" => one(clean(arg(args, "url", "url"))
            .and_then(|url| tools::browser::site_of(&url))
            .map(|site| format!("{}:{site}", tools::browser::grant_prefix(args)))),
        "computer_screenshot" | "computer_act" | "computer_window" => {
            Some(vec!["computer:session".to_string()])
        }
        "computer_launch" => one(clean(arg(args, "program", "program"))
            .and_then(|program| flow_like_bot_tools::computer::launch_scope(&program))
            .map(|name| format!("computer:launch:{name}"))),
        "email_send" => flow_like_bot_tools::email::grant_scopes(args),
        "message_person" => flow_like_bot_tools::people::grant_scopes(args),
        "shell_run" => one(flow_like_bot_tools::shell::grant_scope(args).and_then(|s| clean(&s))),
        "web_request" => flow_like_bot_tools::web::grant_scopes(args)
            .and_then(|scopes| scopes.iter().map(|scope| clean(scope)).collect()),
        "file_save" => super::tool_spec::download_grant_scopes(args)
            .and_then(|scopes| scopes.iter().map(|scope| clean(scope)).collect()),
        _ => Some(vec![SCOPE_ALL.to_string()]),
    }
}

fn token_scope(prefix: &str, hash: &str) -> String {
    format!("{prefix}{hash}")
}

/// Whether a call's grant scopes name a web destination (`web:`, `browser:`, `owner_browser:`).
pub fn web_scoped(tool: &str, args: &Value) -> bool {
    grant_scopes(tool, args, None).is_some_and(|scopes| {
        scopes.iter().any(|scope| {
            WEB_SCOPE_PREFIXES
                .iter()
                .any(|prefix| scope.starts_with(prefix))
        })
    })
}

/// Whether standing grants are set aside for this call: its round began with web content in the
/// model's context and it reaches a web destination.
pub fn grant_set_aside(tool: &str, args: &Value) -> bool {
    super::run_recovery::round_has_web_content() && web_scoped(tool, args)
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
    /// The round read web content first, so standing grants do not cover it: ask again.
    SetAside,
}

impl GrantCheck {
    pub fn runs(self) -> bool {
        matches!(self, Self::Granted | Self::Once)
    }
}

fn covering_grants(keys: &[GrantKey], tool: &str, scopes: &[String]) -> Option<Vec<GrantKey>> {
    let held = |scope: &str| {
        keys.iter()
            .any(|key| key.tool == tool && key.scope == scope)
    };
    let used: Vec<&str> = if held(SCOPE_ALL) {
        vec![SCOPE_ALL]
    } else if scopes.iter().all(|scope| held(scope)) {
        scopes.iter().map(String::as_str).collect()
    } else {
        return None;
    };
    Some(
        used.into_iter()
            .map(|scope| GrantKey::new(tool, scope))
            .collect(),
    )
}

/// `use_call` spends a one-time token and touches the grant it relies on; a peek leaves both.
async fn match_call(
    store: &dyn GrantStore,
    tool: &str,
    args: &Value,
    context_app: Option<&str>,
    use_call: bool,
) -> flow_like_types::Result<GrantCheck> {
    let hash = args_hash(tool, args);
    if store.delete(tool, &token_scope(DENY_PREFIX, &hash)).await? {
        return Ok(GrantCheck::Denied);
    }
    let set_aside = grant_set_aside(tool, args);
    let mut keys = None;
    if !set_aside && let Some(scopes) = grant_scopes(tool, args, context_app) {
        let listed = store.list().await?;
        if let Some(used) = covering_grants(&listed, tool, &scopes) {
            if use_call {
                let _ = store.touch(&used).await;
            }
            return Ok(GrantCheck::Granted);
        }
        keys = Some(listed);
    }
    let once = token_scope(ONCE_PREFIX, &hash);
    let has_once = if use_call {
        store.delete(tool, &once).await?
    } else {
        let keys = match keys {
            Some(keys) => keys,
            None => store.list().await?,
        };
        keys.iter().any(|key| key.tool == tool && key.scope == once)
    };
    Ok(match (has_once, set_aside) {
        (true, _) => GrantCheck::Once,
        (false, true) => GrantCheck::SetAside,
        (false, false) => GrantCheck::Missing,
    })
}

/// Matches one call against the bot's grants and answer tokens. A refusal token wins; a spent
/// token cannot be used twice. Under web content a web-scoped call skips the standing grants
/// ([`GrantCheck::SetAside`]); its tokens still apply.
pub async fn check_call(
    store: &dyn GrantStore,
    tool: &str,
    args: &Value,
    context_app: Option<&str>,
) -> flow_like_types::Result<GrantCheck> {
    match_call(store, tool, args, context_app, true).await
}

/// [`check_call`] for a call that does not run now: a one-time token stays for the run that
/// spends it and no grant is touched. A refusal token is spent, since the refusal becomes the
/// call's result.
pub async fn peek_call(
    store: &dyn GrantStore,
    tool: &str,
    args: &Value,
    context_app: Option<&str>,
) -> flow_like_types::Result<GrantCheck> {
    match_call(store, tool, args, context_app, false).await
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
/// but not its arguments. Every item of a batch card gets `decision`.
pub async fn record_card_answer(
    store: &dyn GrantStore,
    card: &ApprovalRequest,
    decision: ApprovalDecision,
) -> flow_like_types::Result<()> {
    let answer = ApprovalAnswer {
        approval_id: card.id.clone(),
        decision,
        items: Vec::new(),
    };
    record_batch_answer(store, card, &answer).await.map(drop)
}

/// Stores the owner's answer to every item of a card (an unlisted item takes the answer's
/// `decision`; "always" on a call that is never remembered stays one-time) and returns the answer
/// entry to post. Refuses an answer for another card or naming an item the card does not hold.
pub async fn record_batch_answer(
    store: &dyn GrantStore,
    card: &ApprovalRequest,
    answer: &ApprovalAnswer,
) -> flow_like_types::Result<ApprovalResponse> {
    if answer.approval_id != card.id {
        return Err(flow_like_types::anyhow!(
            "Answer for approval '{}' cannot settle card '{}'",
            answer.approval_id,
            card.id
        ));
    }
    let decisions = card
        .decisions(answer.decision, &answer.items)
        .map_err(|error| flow_like_types::anyhow!(error))?;
    for (item, decision) in &decisions {
        let scopes: Vec<String> = item
            .scope
            .split(',')
            .filter(|_| !NOT_GRANTABLE.contains(&item.tool.as_str()))
            .filter(|scope| !scope.is_empty())
            .map(str::to_string)
            .collect();
        store_decision(store, &item.tool, &item.args_hash, &scopes, *decision).await?;
    }
    Ok(ApprovalResponse::for_decisions(card.id.clone(), &decisions))
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
        secret: secret_ask(tool, args),
        invite: (tool == ROOM_INVITE_TOOL)
            .then(|| tools::rooms::invite_ask(args))
            .flatten(),
        configuration_changes: tools::configuration::proposed_changes(tool, args),
        items: Vec::new(),
    }
}

/// What a `secret_request` card asks for; `None` for every other tool or invalid arguments.
fn secret_ask(tool: &str, args: &Value) -> Option<SecretAsk> {
    if tool != SECRET_REQUEST_TOOL {
        return None;
    }
    let args = serde_json::from_value(args.clone()).ok()?;
    let (name, description) = tools::secrets::checked_request(&args).ok()?;
    Some(SecretAsk {
        name,
        description,
        replace: args.replace,
    })
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
    headless_answer_with_inference(store, turn_id, tool, args, context_app, description, None).await
}

/// Irreversible deletion and explicit review boundaries always require an owner's answer.
/// Document cost approvals use their separate gate and never reach inference.
pub fn permits_authorization_inference(tool: &str, args: &Value) -> bool {
    !NOT_GRANTABLE.contains(&tool)
        && tool != DOCUMENTS_TOOL
        && tool != "knowledge_drop"
        && !(tool == "database_tool" && arg(args, "operation", "operation") == "delete_table")
        && !(tool == "storage_tool" && arg(args, "operation", "operation") == "delete_files")
}

/// An inferred answer covers only this concrete call; it never writes a standing grant.
pub async fn headless_answer_with_inference(
    store: &dyn GrantStore,
    turn_id: &str,
    tool: &str,
    args: &Value,
    context_app: Option<&str>,
    description: &str,
    inference: Option<&dyn AuthorizationInference>,
) -> HeadlessAnswer {
    let check = check_call(store, tool, args, context_app).await;
    answer_check(
        check,
        turn_id,
        tool,
        args,
        context_app,
        description,
        inference,
    )
    .await
}

async fn answer_check(
    check: flow_like_types::Result<GrantCheck>,
    turn_id: &str,
    tool: &str,
    args: &Value,
    context_app: Option<&str>,
    description: &str,
    inference: Option<&dyn AuthorizationInference>,
) -> HeadlessAnswer {
    match check {
        Ok(check) if check.runs() => HeadlessAnswer::Run,
        Ok(GrantCheck::Denied) => {
            if let Some(inference) = inference {
                inference.record_refusal(tool, args, context_app);
            }
            HeadlessAnswer::Refuse(format!(
                "The owner refused this {tool} call. Do not try it again; tell them what you could not do."
            ))
        }
        Ok(_) => {
            if permits_authorization_inference(tool, args)
                && let Some(inference) = inference
                && flow_like_types::tokio::time::timeout(
                    super::tool_authorization::DECISION_TIMEOUT,
                    inference.infer(tool, args, context_app, description),
                )
                .await
                .unwrap_or(false)
            {
                return HeadlessAnswer::Run;
            }
            HeadlessAnswer::Park(approval_request(
                turn_id,
                tool,
                args,
                context_app,
                description,
            ))
        }
        Err(error) => HeadlessAnswer::Refuse(format!(
            "The owner's standing approvals could not be read ({error}). Nothing was run."
        )),
    }
}

/// Serialized cards stay below this size; the inbox envelope holds 64 KiB.
const APPROVAL_CARD_BUDGET_BYTES: usize = 56 * 1024;
/// An item's summary on a batch card has at most this many characters.
const ITEM_SUMMARY_CHARS: usize = 300;
/// A batch card's own summary has at most as many characters as a card for one call.
const CARD_SUMMARY_CHARS: usize = 480;
/// Clearance asks the decision model about at most this many calls of a parked round.
const CLEARANCE_INFERENCES: usize = 8;

fn clip(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let mut clipped: String = text.chars().take(max.saturating_sub(1)).collect();
    clipped.push('…');
    clipped
}

fn is_configuration(tool: &str) -> bool {
    CONFIGURATION_TOOLS.contains(&tool)
}

struct BatchEntry {
    card: ApprovalRequest,
    calls: u32,
    after_web_content: bool,
}

impl BatchEntry {
    fn item(&self) -> ApprovalItem {
        ApprovalItem {
            args_hash: self.card.args_hash.clone(),
            tool: self.card.tool.clone(),
            summary: clip(&self.card.summary, ITEM_SUMMARY_CHARS),
            scope: self.card.scope.clone(),
            calls: self.calls,
            after_web_content: self.after_web_content,
            secret: self.card.secret.clone(),
            invite: self.card.invite.clone(),
        }
    }

    fn rememberable(&self) -> bool {
        !self.after_web_content
            && !self.card.scope.is_empty()
            && !NOT_GRANTABLE.contains(&self.card.tool.as_str())
    }

    /// What the call does without its subject: "send a GET request to …".
    fn action(&self) -> &str {
        let summary = self.card.summary.as_str();
        summary
            .split_once(" wants to ")
            .filter(|(subject, _)| subject.chars().count() <= 40)
            .map_or(summary, |(_, action)| action)
            .trim_end_matches('.')
    }
}

/// Old clients offer "Always allow" for the card's top-level scope; it is set only when every
/// item may be remembered.
fn union_scope(entries: &[BatchEntry]) -> String {
    if !entries.iter().all(BatchEntry::rememberable) {
        return String::new();
    }
    let mut scopes: Vec<&str> = Vec::new();
    for scope in entries.iter().flat_map(|entry| entry.card.scope.split(',')) {
        if !scopes.contains(&scope) {
            scopes.push(scope);
        }
    }
    let union = scopes.join(",");
    if union.chars().count() <= flow_like_types::tool_grant::MAX_KEY_CHARS * 4 {
        union
    } else {
        String::new()
    }
}

fn batch_summary(entries: &[BatchEntry]) -> String {
    let calls: u32 = entries.iter().map(|entry| entry.calls).sum();
    let actions: Vec<&str> = entries.iter().map(BatchEntry::action).collect();
    clip(
        &format!(
            "The bot wants to make {calls} calls: {}",
            actions.join("; ")
        ),
        CARD_SUMMARY_CHARS,
    )
}

/// One plain call is the card [`approval_request`] built. A flagged call or several identical
/// ones add their item; several items make a batch card whose top-level fields summarize them.
fn batch_card(turn_id: &str, entries: &[BatchEntry]) -> Option<ApprovalRequest> {
    let first = entries.first()?;
    if entries.len() == 1 {
        let mut card = first.card.clone();
        if first.calls > 1 || first.after_web_content {
            if first.after_web_content {
                card.scope.clear();
            }
            card.items = vec![first.item()];
        }
        return Some(card);
    }
    let items: Vec<ApprovalItem> = entries.iter().map(BatchEntry::item).collect();
    let args_hash = batch_args_hash(items.iter().map(|item| item.args_hash.as_str()));
    Some(ApprovalRequest {
        id: approval_message_id(turn_id, &args_hash),
        turn_id: turn_id.to_string(),
        tool: APPROVAL_BATCH_TOOL.to_string(),
        summary: batch_summary(entries),
        scope: union_scope(entries),
        args_hash,
        requester_id: None,
        requester_name: None,
        room: None,
        secret: None,
        invite: None,
        configuration_changes: None,
        items,
    })
}

#[derive(Default)]
struct BatchState {
    entries: Vec<BatchEntry>,
    held: Vec<(String, String)>,
    answered: HashMap<String, GrantCheck>,
    inferences: usize,
}

/// Every call of a headless attempt's round that waits for the owner, on one card. A host keeps
/// one per attempt and shares it between its gates. Identical calls share an item; a card holds
/// at most [`MAX_APPROVAL_ITEMS`] items and stays below the envelope size; a configuration change
/// has the card to itself. A call that does not fit stays held and asks on the next card.
pub struct ParkedBatch {
    turn_id: String,
    state: Mutex<BatchState>,
}

impl std::fmt::Debug for ParkedBatch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let state = self.state();
        f.debug_struct("ParkedBatch")
            .field("turn_id", &self.turn_id)
            .field("items", &state.entries.len())
            .field("held", &state.held.len())
            .finish()
    }
}

impl ParkedBatch {
    pub fn new(turn_id: &str) -> Self {
        Self {
            turn_id: turn_id.to_string(),
            state: Mutex::new(BatchState::default()),
        }
    }

    pub fn turn_id(&self) -> &str {
        &self.turn_id
    }

    fn state(&self) -> std::sync::MutexGuard<'_, BatchState> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Puts a call that waits for the owner on the card. The call the running
    /// [`super::run_recovery::CallScope`] names is held: the gate confirms it did not run.
    pub fn park(&self, card: ApprovalRequest, after_web_content: bool) {
        let call =
            super::run_recovery::current_recovery_call_id().map(|id| (id, card.tool.clone()));
        let mut state = self.state();
        if let Some(call) = call {
            if state.held.contains(&call) {
                return;
            }
            state.held.push(call);
        }
        if let Some(entry) = state
            .entries
            .iter_mut()
            .find(|entry| entry.card.args_hash == card.args_hash)
        {
            entry.calls = entry.calls.saturating_add(1);
            entry.after_web_content |= after_web_content;
            return;
        }
        if state
            .entries
            .first()
            .is_some_and(|entry| is_configuration(&entry.card.tool))
        {
            return;
        }
        let entry = BatchEntry {
            card,
            calls: 1,
            after_web_content,
        };
        if is_configuration(&entry.card.tool) {
            state.entries = vec![entry];
            return;
        }
        if state.entries.len() >= MAX_APPROVAL_ITEMS {
            return;
        }
        state.entries.push(entry);
        let fits = batch_card(&self.turn_id, &state.entries)
            .and_then(|card| serde_json::to_vec(&card).ok())
            .is_some_and(|bytes| bytes.len() <= APPROVAL_CARD_BUDGET_BYTES);
        if state.entries.len() > 1 && !fits {
            state.entries.pop();
        }
    }

    /// The gate of a call about to run: [`headless_answer_with_inference`], except that a call
    /// identical to one whose answer this attempt already spent gets that answer again, and a call
    /// that asks joins the card (`Park` carries its own one-call card).
    pub async fn answer(
        &self,
        store: &dyn GrantStore,
        tool: &str,
        args: &Value,
        context_app: Option<&str>,
        description: &str,
        inference: Option<&dyn AuthorizationInference>,
    ) -> HeadlessAnswer {
        let check = check_call(store, tool, args, context_app).await;
        let check = self.recall(tool, args, check, true);
        self.settle(check, tool, args, context_app, description, inference)
            .await
    }

    /// The gate of a call held back in a parked round, asked without running it ([`peek_call`]):
    /// `Run` means nothing needs asking now, `Refuse` is the call's result, `Park` put it on the
    /// card. The decision model is asked about at most 8 such calls per attempt.
    pub async fn clearance(
        &self,
        store: &dyn GrantStore,
        tool: &str,
        args: &Value,
        context_app: Option<&str>,
        description: &str,
        inference: Option<&dyn AuthorizationInference>,
    ) -> HeadlessAnswer {
        let check = peek_call(store, tool, args, context_app).await;
        let check = self.recall(tool, args, check, false);
        let asks = matches!(check, Ok(GrantCheck::Missing | GrantCheck::SetAside));
        let inference = inference.filter(|_| {
            !asks || (permits_authorization_inference(tool, args) && self.reserve_inference())
        });
        self.settle(check, tool, args, context_app, description, inference)
            .await
    }

    fn reserve_inference(&self) -> bool {
        let mut state = self.state();
        let free = state.inferences < CLEARANCE_INFERENCES;
        state.inferences += usize::from(free);
        free
    }

    /// One item on the card covers every identical call of its round. A spent refusal therefore
    /// answers every identical call of the attempt. A spent one-time answer (`spent_once`) answers
    /// the identical calls of the held round it was given for, and none after it: a later call
    /// with the same arguments asks again.
    fn recall(
        &self,
        tool: &str,
        args: &Value,
        check: flow_like_types::Result<GrantCheck>,
        spent_once: bool,
    ) -> flow_like_types::Result<GrantCheck> {
        let hash = args_hash(tool, args);
        let mut state = self.state();
        match check {
            Ok(GrantCheck::Denied) => {
                state.answered.insert(hash, GrantCheck::Denied);
                Ok(GrantCheck::Denied)
            }
            Ok(GrantCheck::Once) if spent_once => {
                state.answered.insert(hash, GrantCheck::Once);
                Ok(GrantCheck::Once)
            }
            Ok(open @ (GrantCheck::Missing | GrantCheck::SetAside)) => {
                Ok(match state.answered.get(&hash).copied() {
                    Some(GrantCheck::Denied) => GrantCheck::Denied,
                    Some(GrantCheck::Once) if super::run_recovery::replays_held_call() => {
                        GrantCheck::Once
                    }
                    _ => open,
                })
            }
            other => other,
        }
    }

    async fn settle(
        &self,
        check: flow_like_types::Result<GrantCheck>,
        tool: &str,
        args: &Value,
        context_app: Option<&str>,
        description: &str,
        inference: Option<&dyn AuthorizationInference>,
    ) -> HeadlessAnswer {
        let set_aside = matches!(check, Ok(GrantCheck::SetAside));
        let answer = answer_check(
            check,
            &self.turn_id,
            tool,
            args,
            context_app,
            description,
            inference,
        )
        .await;
        if let HeadlessAnswer::Park(card) = &answer {
            self.park(card.clone(), set_aside);
        }
        answer
    }

    /// Whether any call of the attempt waits for the owner.
    pub fn is_parked(&self) -> bool {
        !self.state().entries.is_empty()
    }

    /// The card to post: `None` while nothing waits. One plain call is today's card.
    pub fn card(&self) -> Option<ApprovalRequest> {
        batch_card(&self.turn_id, &self.state().entries)
    }

    /// `(call id, tool)` of every call a gate held back, for [`super::run_recovery::record_held_batch`].
    pub fn held_calls(&self) -> Vec<(String, String)> {
        self.state().held.clone()
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

    #[test]
    fn one_standing_grant_lets_the_bot_answer_every_mail() {
        assert_eq!(
            grant_scopes("email_reply", &json!({"thread": "t1", "body": "x"}), None),
            Some(vec![SCOPE_ALL.to_string()])
        );
    }

    use super::*;
    use serde_json::json;
    use std::sync::Mutex;

    /// Grants and tokens, and the grants touched by use.
    #[derive(Default)]
    struct Memory(Mutex<Vec<GrantKey>>, Mutex<Vec<GrantKey>>);

    #[derive(Default)]
    struct Infer {
        calls: std::sync::atomic::AtomicUsize,
        refused: Mutex<Vec<String>>,
        allow: bool,
    }

    #[async_trait]
    impl AuthorizationInference for Infer {
        async fn infer(&self, tool: &str, args: &Value, _: Option<&str>, _: &str) -> bool {
            self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            self.allow
                && !self
                    .refused
                    .lock()
                    .unwrap()
                    .contains(&args_hash(tool, args))
        }

        fn record_refusal(&self, tool: &str, args: &Value, _: Option<&str>) {
            self.refused.lock().unwrap().push(args_hash(tool, args));
        }
    }

    #[tokio::test]
    async fn inference_authorizes_one_call_without_storing_a_grant() {
        let store = Memory::default();
        let inference = Infer {
            allow: true,
            ..Default::default()
        };
        let args = json!({"app_id":"a", "event_id":"e"});
        assert_eq!(
            headless_answer_with_inference(
                &store,
                "t",
                "call_app_event",
                &args,
                None,
                "Run event",
                Some(&inference)
            )
            .await,
            HeadlessAnswer::Run
        );
        assert!(store.list().await.unwrap().is_empty());
        assert!(matches!(
            headless_answer(&store, "t", "call_app_event", &args, None, "").await,
            HeadlessAnswer::Park(_)
        ));
        let uncertain = Infer::default();
        assert!(matches!(
            headless_answer_with_inference(
                &store,
                "t",
                "call_app_event",
                &args,
                None,
                "",
                Some(&uncertain)
            )
            .await,
            HeadlessAnswer::Park(_)
        ));
    }

    #[tokio::test]
    async fn explicit_approvals_and_refusals_take_precedence_over_inference() {
        let store = Memory::default();
        let inference = Infer {
            allow: true,
            ..Default::default()
        };
        let args = json!({"app_id":"a", "event_id":"e"});
        record_answer(
            &store,
            "call_app_event",
            &args,
            None,
            ApprovalDecision::Always,
        )
        .await
        .unwrap();
        assert_eq!(
            headless_answer_with_inference(
                &store,
                "t",
                "call_app_event",
                &args,
                None,
                "",
                Some(&inference)
            )
            .await,
            HeadlessAnswer::Run
        );
        record_answer(
            &store,
            "call_app_event",
            &args,
            None,
            ApprovalDecision::Deny,
        )
        .await
        .unwrap();
        assert!(matches!(
            headless_answer_with_inference(
                &store,
                "t",
                "call_app_event",
                &args,
                None,
                "",
                Some(&inference)
            )
            .await,
            HeadlessAnswer::Refuse(_)
        ));
        assert_eq!(inference.calls.load(std::sync::atomic::Ordering::SeqCst), 0);
        assert_eq!(
            *inference.refused.lock().unwrap(),
            vec![args_hash("call_app_event", &args)]
        );
        let once_store = Memory::default();
        record_answer(
            &once_store,
            "call_app_event",
            &args,
            None,
            ApprovalDecision::Once,
        )
        .await
        .unwrap();
        assert_eq!(
            headless_answer_with_inference(
                &once_store,
                "t",
                "call_app_event",
                &args,
                None,
                "",
                Some(&inference)
            )
            .await,
            HeadlessAnswer::Run
        );
        assert_eq!(inference.calls.load(std::sync::atomic::Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn inference_cannot_waive_explicit_review_or_destructive_deletion() {
        let store = Memory::default();
        let inference = Infer {
            allow: true,
            ..Default::default()
        };
        let mut cases: Vec<_> = NOT_GRANTABLE
            .iter()
            .map(|tool| (*tool, json!({})))
            .collect();
        cases.extend([
            (
                "database_tool",
                json!({"operation":"delete_table", "table_name":"records"}),
            ),
            ("storage_tool", json!({"operation":"delete_files"})),
            ("knowledge_drop", json!({"index":"records"})),
            (DOCUMENTS_TOOL, json!({})),
        ]);
        for (tool, args) in cases {
            assert!(
                matches!(
                    headless_answer_with_inference(
                        &store,
                        "t",
                        tool,
                        &args,
                        None,
                        "",
                        Some(&inference)
                    )
                    .await,
                    HeadlessAnswer::Park(_)
                ),
                "{tool}"
            );
        }
        assert_eq!(inference.calls.load(std::sync::atomic::Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn unavailable_grants_never_fall_back_to_inference() {
        struct Unavailable;
        #[async_trait]
        impl GrantStore for Unavailable {
            async fn list(&self) -> flow_like_types::Result<Vec<GrantKey>> {
                Err(flow_like_types::anyhow!("store unavailable"))
            }
            async fn put(&self, _: &str, _: &str) -> flow_like_types::Result<()> {
                unreachable!()
            }
            async fn delete(&self, _: &str, _: &str) -> flow_like_types::Result<bool> {
                Ok(false)
            }
        }
        let inference = Infer {
            allow: true,
            ..Default::default()
        };
        assert!(matches!(
            headless_answer_with_inference(
                &Unavailable,
                "t",
                "email_reply",
                &json!({}),
                None,
                "",
                Some(&inference)
            )
            .await,
            HeadlessAnswer::Refuse(_)
        ));
        assert_eq!(inference.calls.load(std::sync::atomic::Ordering::SeqCst), 0);
        assert!(inference.refused.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn configuration_changes_require_one_approval_for_each_exact_call() {
        let tool = tools::configuration::BOT_UPDATE_CONFIGURATION;
        let store = Memory::default();
        let args =
            json!({"changes":{"instructions":"a".repeat(8_000),"capabilities":["internet"]}});
        store.put(tool, SCOPE_ALL).await.unwrap();
        assert_eq!(
            check_call(&store, tool, &args, None).await.unwrap(),
            GrantCheck::Missing
        );
        let HeadlessAnswer::Park(mut card) =
            headless_answer(&store, "turn", tool, &args, None, "Change my settings").await
        else {
            panic!("settings must ask");
        };
        assert!(card.scope.is_empty());
        assert_eq!(card.configuration_changes, Some(args["changes"].clone()));
        // A stored or stale card cannot turn this tool into a standing permission.
        card.scope = "*".into();
        record_card_answer(&store, &card, ApprovalDecision::Always)
            .await
            .unwrap();
        let different = json!({"changes":{"capabilities":["command_line"]}});
        assert_eq!(
            check_call(&store, tool, &different, None).await.unwrap(),
            GrantCheck::Missing
        );
        assert_eq!(
            check_call(&store, tool, &args, None).await.unwrap(),
            GrantCheck::Once
        );
        assert_eq!(
            check_call(&store, tool, &args, None).await.unwrap(),
            GrantCheck::Missing
        );
        record_answer(&store, tool, &args, None, ApprovalDecision::Deny)
            .await
            .unwrap();
        assert!(matches!(
            headless_answer(&store, "turn", tool, &args, None, "").await,
            HeadlessAnswer::Refuse(_)
        ));
    }

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
        async fn touch(&self, keys: &[GrantKey]) -> flow_like_types::Result<()> {
            self.1.lock().unwrap().extend_from_slice(keys);
            Ok(())
        }
    }

    use crate::flow::copilot::run_recovery::{CallScope, scope_call};
    use flow_like_types::bot_message::{ItemDecision, MessageBody};

    fn call_scope(id: &str, round_web_content: bool) -> CallScope {
        CallScope {
            id: id.into(),
            round_web_content,
            held_replay: false,
        }
    }

    fn held_call(id: &str) -> CallScope {
        CallScope {
            id: id.into(),
            round_web_content: false,
            held_replay: true,
        }
    }

    fn fetch(url: &str) -> Value {
        json!({ "url": url })
    }

    fn fetch_card(turn_id: &str, url: &str) -> ApprovalRequest {
        let site = url.trim_start_matches("https://");
        approval_request(
            turn_id,
            "web_request",
            &fetch(url),
            None,
            &format!("The bot wants to send a GET request to {site}."),
        )
    }

    async fn park_in(batch: &ParkedBatch, call_id: &str, card: ApprovalRequest) {
        scope_call(call_scope(call_id, false), async {
            batch.park(card, false)
        })
        .await;
    }

    fn token(prefix: &str, tool: &str, args: &Value) -> GrantKey {
        GrantKey::new(tool, format!("{prefix}{}", args_hash(tool, args)))
    }

    #[tokio::test]
    async fn twelve_parks_make_one_card() {
        let store = Memory::default();
        let batch = ParkedBatch::new("bt_1");
        let url = |n: usize| format!("https://cisa.gov/news-events/alerts?p={n}");
        let description = |n: usize| {
            format!(
                "The bot wants to send a GET request to {}.",
                url(n).trim_start_matches("https://")
            )
        };
        let first = scope_call(
            call_scope("c0", false),
            batch.answer(
                &store,
                "web_request",
                &fetch(&url(0)),
                None,
                &description(0),
                None,
            ),
        )
        .await;
        let expected = approval_request(
            "bt_1",
            "web_request",
            &fetch(&url(0)),
            None,
            &description(0),
        );
        assert_eq!(first, HeadlessAnswer::Park(expected.clone()));
        assert_eq!(batch.card(), Some(expected));
        for n in 1..12 {
            scope_call(
                call_scope(&format!("c{n}"), false),
                batch.answer(
                    &store,
                    "web_request",
                    &fetch(&url(n)),
                    None,
                    &description(n),
                    None,
                ),
            )
            .await;
        }
        let card = batch.card().unwrap();
        assert_eq!(card.tool, APPROVAL_BATCH_TOOL);
        assert_eq!(card.items.len(), 12);
        assert!(
            card.summary
                .starts_with("The bot wants to make 12 calls: send a GET request to cisa.gov/"),
            "{}",
            card.summary
        );
        assert!(card.summary.chars().count() <= 480);
        assert_eq!(
            card.scope,
            grant_scopes("web_request", &fetch(&url(0)), None)
                .unwrap()
                .join(",")
        );
        assert_eq!(
            card.args_hash,
            batch_args_hash(card.items.iter().rev().map(|item| item.args_hash.as_str()))
        );
        assert_eq!(card.id, approval_message_id("bt_1", &card.args_hash));
        assert_eq!(batch.held_calls().len(), 12);
        assert!(
            MessageBody {
                approval: Some(card.clone()),
                ..Default::default()
            }
            .check()
            .is_ok()
        );
        park_in(&batch, "c12", fetch_card("bt_1", &url(0))).await;
        park_in(&batch, "c12", fetch_card("bt_1", &url(0))).await;
        let card = batch.card().unwrap();
        assert_eq!((card.items.len(), card.items[0].calls), (12, 2));
        assert!(card.summary.starts_with("The bot wants to make 13 calls"));
        assert_eq!(batch.held_calls().len(), 13);
    }

    #[tokio::test]
    async fn items_past_the_budget_wait_for_the_next_card() {
        let batch = ParkedBatch::new("bt_1");
        for n in 0..40 {
            let card = fetch_card("bt_1", &format!("https://site{n}.example/feed"));
            park_in(&batch, &format!("c{n}"), card).await;
        }
        assert_eq!(batch.card().unwrap().items.len(), MAX_APPROVAL_ITEMS);
        assert_eq!(batch.held_calls().len(), 40);

        let batch = ParkedBatch::new("bt_2");
        for n in 0..MAX_APPROVAL_ITEMS {
            let mut card = fetch_card("bt_2", &format!("https://site{n}.example/feed"));
            card.summary = "s".repeat(480);
            card.scope = format!("web:site{n}.example:secret:{}", "K".repeat(960));
            card.secret = Some(SecretAsk {
                name: "API_KEY".into(),
                description: "d".repeat(500),
                replace: false,
            });
            park_in(&batch, &format!("c{n}"), card).await;
        }
        let card = batch.card().unwrap();
        assert!(
            (2..MAX_APPROVAL_ITEMS).contains(&card.items.len()),
            "{}",
            card.items.len()
        );
        assert!(serde_json::to_vec(&card).unwrap().len() <= APPROVAL_CARD_BUDGET_BYTES);
        assert_eq!(batch.held_calls().len(), MAX_APPROVAL_ITEMS);
    }

    #[tokio::test]
    async fn a_configuration_change_is_a_card_of_its_own() {
        let batch = ParkedBatch::new("bt_1");
        park_in(&batch, "c1", fetch_card("bt_1", "https://a.example/")).await;
        let config = approval_request(
            "bt_1",
            tools::configuration::BOT_UPDATE_CONFIGURATION,
            &json!({"changes": {"name": "Atlas"}}),
            None,
            "Change my settings",
        );
        park_in(&batch, "c2", config.clone()).await;
        park_in(&batch, "c3", fetch_card("bt_1", "https://b.example/")).await;
        assert_eq!(batch.card(), Some(config));
        assert_eq!(
            batch.held_calls(),
            vec![
                ("c1".to_string(), "web_request".to_string()),
                (
                    "c2".into(),
                    tools::configuration::BOT_UPDATE_CONFIGURATION.into()
                ),
                ("c3".into(), "web_request".into()),
            ]
        );
    }

    #[tokio::test]
    async fn one_answer_covers_the_identical_calls_of_its_round_and_none_after() {
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
        let batch = ParkedBatch::new("turn-1");
        let answer = || batch.answer(&store, "knowledge_drop", &args, None, "drop it", None);

        assert!(matches!(
            scope_call(held_call("call-1"), answer()).await,
            HeadlessAnswer::Run
        ));
        assert!(matches!(
            scope_call(held_call("call-2"), answer()).await,
            HeadlessAnswer::Run
        ));
        assert!(matches!(
            scope_call(call_scope("call-3", false), answer()).await,
            HeadlessAnswer::Park(_)
        ));
        assert!(matches!(answer().await, HeadlessAnswer::Park(_)));
    }

    #[tokio::test]
    async fn peek_spends_no_once_token() {
        let store = Memory::default();
        let args = json!({"index": "notes"});
        let peek = || peek_call(&store, "knowledge_drop", &args, None);
        record_answer(
            &store,
            "knowledge_drop",
            &args,
            None,
            ApprovalDecision::Once,
        )
        .await
        .unwrap();
        assert_eq!(peek().await.unwrap(), GrantCheck::Once);
        assert_eq!(peek().await.unwrap(), GrantCheck::Once);
        assert_eq!(
            check_call(&store, "knowledge_drop", &args, None)
                .await
                .unwrap(),
            GrantCheck::Once
        );
        assert_eq!(peek().await.unwrap(), GrantCheck::Missing);
        record_answer(
            &store,
            "knowledge_drop",
            &args,
            None,
            ApprovalDecision::Deny,
        )
        .await
        .unwrap();
        assert_eq!(peek().await.unwrap(), GrantCheck::Denied);
        assert_eq!(peek().await.unwrap(), GrantCheck::Missing);
        store.put("knowledge_drop", "table:notes").await.unwrap();
        assert_eq!(peek().await.unwrap(), GrantCheck::Granted);
        assert!(store.1.lock().unwrap().is_empty());

        let batch = ParkedBatch::new("bt_1");
        let other = json!({"index": "other"});
        record_answer(
            &store,
            "knowledge_drop",
            &other,
            None,
            ApprovalDecision::Once,
        )
        .await
        .unwrap();
        assert_eq!(
            batch
                .clearance(&store, "knowledge_drop", &other, None, "", None)
                .await,
            HeadlessAnswer::Run
        );
        assert!(store.list().await.unwrap().contains(&token(
            ONCE_PREFIX,
            "knowledge_drop",
            &other
        )));
        let fresh = json!({"index": "fresh"});
        assert!(matches!(
            batch
                .clearance(&store, "knowledge_drop", &fresh, None, "Drop 'fresh'", None)
                .await,
            HeadlessAnswer::Park(_)
        ));
        assert!(batch.is_parked());
    }

    #[tokio::test]
    async fn record_batch_answer_stores_each_item() {
        let batch = ParkedBatch::new("bt_1");
        let a = fetch("https://a.example/feed");
        let b = fetch("https://b.example/feed");
        let notes = json!({"index": "notes"});
        let secret = json!({"name": "API_TOKEN", "description": "Read the API"});
        park_in(&batch, "c1", fetch_card("bt_1", "https://a.example/feed")).await;
        park_in(&batch, "c2", fetch_card("bt_1", "https://b.example/feed")).await;
        let drop_card = approval_request("bt_1", "knowledge_drop", &notes, None, "Drop notes");
        park_in(&batch, "c3", drop_card).await;
        let secret_card = approval_request("bt_1", SECRET_REQUEST_TOOL, &secret, None, "Asks");
        park_in(&batch, "c4", secret_card).await;
        let card = batch.card().unwrap();
        assert_eq!(card.items.len(), 4);
        assert_eq!(card.scope, "");

        let store = Memory::default();
        let item = |tool: &str, args: &Value, decision| ItemDecision {
            args_hash: args_hash(tool, args),
            decision,
        };
        let answer = ApprovalAnswer {
            approval_id: card.id.clone(),
            decision: ApprovalDecision::Once,
            items: vec![
                item("web_request", &b, ApprovalDecision::Always),
                item("knowledge_drop", &notes, ApprovalDecision::Deny),
            ],
        };
        let response = record_batch_answer(&store, &card, &answer).await.unwrap();
        assert_eq!(response.decision, ApprovalDecision::Once);
        assert_eq!(response.items, answer.items);
        let keys = store.list().await.unwrap();
        let b_scope = grant_scopes("web_request", &b, None).unwrap()[0].clone();
        for key in [
            token(ONCE_PREFIX, "web_request", &a),
            token(ONCE_PREFIX, "web_request", &b),
            GrantKey::new("web_request", b_scope),
            token(DENY_PREFIX, "knowledge_drop", &notes),
            token(ONCE_PREFIX, SECRET_REQUEST_TOOL, &secret),
        ] {
            assert!(keys.contains(&key), "{key:?} in {keys:?}");
        }
        assert_eq!(keys.len(), 5);

        let untouched = Memory::default();
        let wrong = ApprovalAnswer {
            items: vec![ItemDecision {
                args_hash: "nope".into(),
                decision: ApprovalDecision::Deny,
            }],
            ..answer
        };
        assert!(
            record_batch_answer(&untouched, &card, &wrong)
                .await
                .is_err()
        );
        assert!(untouched.list().await.unwrap().is_empty());

        let old_client = Memory::default();
        record_card_answer(&old_client, &card, ApprovalDecision::Always)
            .await
            .unwrap();
        let keys = old_client.list().await.unwrap();
        assert!(keys.contains(&GrantKey::new("knowledge_drop", "table:notes")));
        assert!(
            !keys
                .iter()
                .any(|key| key.tool == SECRET_REQUEST_TOOL && !key.scope.starts_with(ONCE_PREFIX))
        );
        assert_eq!(keys.len(), 4 + 3);
    }

    #[tokio::test]
    async fn a_web_grant_is_set_aside_after_web_content() {
        let store = Memory::default();
        let feed = fetch("https://www.cisa.gov/feeds/kev.xml");
        let site = grant_scopes("web_request", &feed, None).unwrap()[0].clone();
        let browse = json!({"url": "https://shop.example/cart"});
        let app = json!({"app_id": "a1", "event_id": "e"});
        store.put("web_request", &site).await.unwrap();
        store.put("browser_open", SCOPE_ALL).await.unwrap();
        store.put("call_app_event", "app:a1").await.unwrap();
        let marked = |id: &'static str| call_scope(id, true);
        let check = |tool: &'static str, args: &Value| {
            let args = args.clone();
            let store = &store;
            async move { check_call(store, tool, &args, None).await.unwrap() }
        };

        assert_eq!(check("web_request", &feed).await, GrantCheck::Granted);
        assert!(!grant_set_aside("web_request", &feed));
        store.1.lock().unwrap().clear();
        assert_eq!(
            scope_call(marked("c1"), check("web_request", &feed)).await,
            GrantCheck::SetAside
        );
        assert_eq!(
            scope_call(marked("c1"), check("browser_open", &browse)).await,
            GrantCheck::SetAside
        );
        assert!(store.1.lock().unwrap().is_empty());
        assert_eq!(
            scope_call(marked("c1"), check("call_app_event", &app)).await,
            GrantCheck::Granted
        );
        assert!(
            scope_call(marked("c1"), async {
                grant_set_aside("web_request", &feed)
            })
            .await
        );
        assert!(
            !scope_call(marked("c1"), async {
                grant_set_aside("call_app_event", &app)
            })
            .await
        );
        assert!(web_scoped("browser_open", &browse) && !web_scoped("call_app_event", &app));

        record_answer(&store, "web_request", &feed, None, ApprovalDecision::Once)
            .await
            .unwrap();
        assert_eq!(
            scope_call(marked("c1"), check("web_request", &feed)).await,
            GrantCheck::Once
        );
        record_answer(&store, "web_request", &feed, None, ApprovalDecision::Once)
            .await
            .unwrap();
        record_answer(&store, "web_request", &feed, None, ApprovalDecision::Deny)
            .await
            .unwrap();
        assert_eq!(
            scope_call(marked("c1"), check("web_request", &feed)).await,
            GrantCheck::Denied
        );
        store
            .delete(
                "web_request",
                &token(ONCE_PREFIX, "web_request", &feed).scope,
            )
            .await
            .unwrap();

        let batch = ParkedBatch::new("bt_1");
        let description = "The bot wants to send a GET request to cisa.gov/feeds/kev.xml.";
        let parked = scope_call(
            marked("c2"),
            batch.answer(&store, "web_request", &feed, None, description, None),
        )
        .await;
        assert!(matches!(parked, HeadlessAnswer::Park(_)));
        let card = batch.card().unwrap();
        assert_eq!(
            (card.tool.as_str(), card.scope.as_str()),
            ("web_request", "")
        );
        assert_eq!(
            card.id,
            approval_message_id("bt_1", &args_hash("web_request", &feed))
        );
        assert_eq!(card.items.len(), 1);
        assert!(card.items[0].after_web_content);
        assert_eq!(card.items[0].scope, site);
        assert_eq!(
            batch.held_calls(),
            vec![("c2".to_string(), "web_request".to_string())]
        );

        let inference = Infer {
            allow: true,
            ..Default::default()
        };
        let inferred = scope_call(
            marked("c3"),
            batch.answer(
                &store,
                "web_request",
                &feed,
                None,
                description,
                Some(&inference),
            ),
        )
        .await;
        assert_eq!(inferred, HeadlessAnswer::Run);
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
            grant_scopes(
                "browser_open",
                &json!({"url": "https://www.Shop.example/cart", "browser": "owner"}),
                None
            ),
            Some(vec!["owner_browser:shop.example".into()])
        );
        assert_eq!(
            grant_scopes(
                "browser_open",
                &json!({"url": "https://shop.example/", "browser": "own"}),
                None
            ),
            Some(vec!["browser:shop.example".into()])
        );
        assert_eq!(
            grant_scopes("browser_open", &json!({"url": "javascript:alert(1)"}), None),
            None
        );
        assert_eq!(grant_scopes("browser_open", &json!({}), None), None);
        for tool in ["computer_act", "computer_screenshot", "computer_window"] {
            assert_eq!(
                grant_scopes(tool, &json!({"action": "click"}), None),
                Some(vec!["computer:session".into()])
            );
        }
        assert_eq!(
            grant_scopes(
                "computer_launch",
                &json!({"program": "/usr/bin/Firefox", "args": ["--private"]}),
                None
            ),
            Some(vec!["computer:launch:firefox".into()])
        );
        assert_eq!(grant_scopes("computer_launch", &json!({}), None), None);
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
    async fn always_for_one_person_never_covers_another() {
        let store = Memory::default();
        let anna = json!({"person": "user-anna", "text": "Does Friday work?"});
        record_answer(
            &store,
            "message_person",
            &anna,
            None,
            ApprovalDecision::Always,
        )
        .await
        .unwrap();
        assert!(
            store
                .has("message_person", "person:user-anna")
                .await
                .unwrap()
        );
        let later = json!({"person": "user-anna", "text": "Thanks!"});
        let ben = json!({"person": "user-ben", "text": "Does Friday work?"});
        assert_eq!(
            check_call(&store, "message_person", &later, None)
                .await
                .unwrap(),
            GrantCheck::Granted
        );
        assert_eq!(
            check_call(&store, "message_person", &ben, None)
                .await
                .unwrap(),
            GrantCheck::Missing
        );
        assert_eq!(
            grant_scopes("message_person", &json!({"person": "Anna Schmidt"}), None),
            None
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
    async fn a_web_grant_covers_one_site_its_kind_of_call_and_the_secrets_approved_there() {
        let store = Memory::default();
        let feed = json!({"url": "https://www.cisa.gov/feeds/kev.xml"});
        record_answer(&store, "web_request", &feed, None, ApprovalDecision::Always)
            .await
            .unwrap();
        let check = |args: Value| {
            let store = &store;
            async move { check_call(store, "web_request", &args, None).await.unwrap() }
        };
        assert_eq!(
            check(json!({"url": "https://www.cisa.gov/other.json?page=2"})).await,
            GrantCheck::Granted
        );
        for asks in [
            json!({"url": "https://www.cisa.gov/x", "method": "POST", "json": {}}),
            json!({"url": "https://www.cisa.gov/x", "headers": {"Key": "{{secret:CISA}}"}}),
            json!({"url": "https://evil.example/kev.xml"}),
            json!({"url": "https://www.cisa.gov:8443/kev.xml"}),
        ] {
            assert_eq!(check(asks.clone()).await, GrantCheck::Missing, "{asks}");
        }

        let keyed = json!({"url": "https://api.github.com/user", "headers": {"Authorization": "Bearer {{secret:GH}}"}});
        record_answer(
            &store,
            "web_request",
            &keyed,
            None,
            ApprovalDecision::Always,
        )
        .await
        .unwrap();
        assert_eq!(
            check(json!({"url": "https://api.github.com/repos", "headers": {"Authorization": "Bearer {{secret:GH}}"}})).await,
            GrantCheck::Granted
        );
        assert_eq!(
            check(json!({"url": "https://www.cisa.gov/kev.xml", "headers": {"Authorization": "Bearer {{secret:GH}}"}})).await,
            GrantCheck::Missing
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
    async fn a_secret_request_parks_with_the_ask_and_is_never_granted_for_good() {
        let store = Memory::default();
        let args =
            json!({"name": "GITHUB_TOKEN", "description": "Open pull requests", "replace": true});
        let HeadlessAnswer::Park(card) = headless_answer(
            &store,
            "bt_1",
            SECRET_REQUEST_TOOL,
            &args,
            None,
            "The bot asks you for the secret GITHUB_TOKEN",
        )
        .await
        else {
            panic!("expected a park");
        };
        assert_eq!(card.scope, "");
        assert_eq!(
            card.secret,
            Some(SecretAsk {
                name: "GITHUB_TOKEN".into(),
                description: "Open pull requests".into(),
                replace: true,
            })
        );
        record_card_answer(&store, &card, ApprovalDecision::Always)
            .await
            .unwrap();
        assert_eq!(
            headless_answer(&store, "bt_1", SECRET_REQUEST_TOOL, &args, None, "").await,
            HeadlessAnswer::Run
        );
        assert!(matches!(
            headless_answer(&store, "bt_2", SECRET_REQUEST_TOOL, &args, None, "").await,
            HeadlessAnswer::Park(_)
        ));
        let bad = json!({"name": "lower", "description": "x"});
        assert_eq!(
            approval_request("bt_1", SECRET_REQUEST_TOOL, &bad, None, "x").secret,
            None
        );
        assert_eq!(
            approval_request("bt_1", "code_run", &args, None, "x").secret,
            None
        );
    }

    #[tokio::test]
    async fn a_room_invite_shows_who_and_why_and_asks_every_time() {
        let store = Memory::default();
        let args = json!({"person": "@ana.lima", "reason": "She owns the budget sheet."});
        let HeadlessAnswer::Park(card) = headless_answer(
            &store,
            "bt_1",
            ROOM_INVITE_TOOL,
            &args,
            None,
            "The bot wants to invite @ana.lima to this group chat.",
        )
        .await
        else {
            panic!("expected a park");
        };
        assert_eq!(card.scope, "");
        assert_eq!(
            card.invite,
            Some(flow_like_types::bot_message::RoomInviteAsk {
                person: "ana.lima".into(),
                reason: "She owns the budget sheet.".into(),
            })
        );
        assert!(!permits_authorization_inference(ROOM_INVITE_TOOL, &args));
        record_card_answer(&store, &card, ApprovalDecision::Always)
            .await
            .unwrap();
        assert_eq!(
            headless_answer(&store, "bt_1", ROOM_INVITE_TOOL, &args, None, "").await,
            HeadlessAnswer::Run
        );
        assert!(matches!(
            headless_answer(&store, "bt_2", ROOM_INVITE_TOOL, &args, None, "").await,
            HeadlessAnswer::Park(_)
        ));
        assert_eq!(
            approval_request("bt_1", "code_run", &args, None, "x").invite,
            None
        );
    }

    #[tokio::test]
    async fn the_cost_check_is_its_own_grant() {
        let store = Memory::default();
        assert!(!documents_expensive_granted(&store).await.unwrap());
        store.put("documents", "documents.expensive").await.unwrap();
        assert!(documents_expensive_granted(&store).await.unwrap());
    }
}
