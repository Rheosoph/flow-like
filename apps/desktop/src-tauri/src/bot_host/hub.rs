//! The hub calls an account bot's turn needs. The real client uses the active profile's session
//! (token refreshed on a 401); tests answer from a fake.

use super::runner::identity;
use async_trait::async_trait;
use chrono::{DateTime, FixedOffset};
use flow_like_local_bots::InboxEntry;
use flow_like::flow::copilot::tool_grants::{GrantKey, GrantStore};
use flow_like_types::{bot_message::ApprovalRequest, bot_turn::BotTurn, inbox::InboxEntryKind};
use std::sync::Arc;
use reqwest::Method;
use serde::Deserialize;
use serde_json::{Value, json};
use std::time::Duration;
use tauri::AppHandle;

pub const CODE_LEASE_LOST: &str = "BOT_TURN_LEASE_LOST";
pub const CODE_NOT_CLAIMED: &str = "BOT_TURN_NOT_CLAIMED";
const CODE_SIGNED_OUT: &str = "ACCOUNT_REQUIRED";
const ENTRIES: u64 = 100;
pub const MAX_HOST_BOTS: usize = 100;
const MAX_RESPONSE_BYTES: usize = 8 * 1024 * 1024;
const MESSAGE_CHARS: usize = 300;

#[derive(Debug, Clone)]
pub struct HubFailure {
    /// `None`: the hub wasn't reached, or nobody is signed in.
    pub status: Option<u16>,
    pub code: Option<String>,
    pub message: String,
}

impl HubFailure {
    pub fn offline(message: impl Into<String>) -> Self {
        Self {
            status: None,
            code: None,
            message: message.into(),
        }
    }

    pub fn answered(status: u16, code: Option<&str>, message: impl Into<String>) -> Self {
        Self {
            status: Some(status),
            code: code.map(str::to_owned),
            message: message.into(),
        }
    }

    /// The host no longer holds the turn: another host took it over or the lease ran out.
    /// Nobody is signed in, or the session was refused.
    pub fn signed_out(&self) -> bool {
        self.status == Some(401) || self.code.as_deref() == Some(CODE_SIGNED_OUT)
    }

    pub fn lease_lost(&self) -> bool {
        self.status == Some(409)
            && matches!(
                self.code.as_deref(),
                Some(CODE_LEASE_LOST | CODE_NOT_CLAIMED)
            )
    }

    /// Another host (or an earlier claim) has the turn, or it is no longer ours to run.
    pub fn taken(&self) -> bool {
        matches!(self.status, Some(404 | 409))
    }
}

impl std::fmt::Display for HubFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match (&self.status, &self.code) {
            (Some(status), Some(code)) => write!(f, "{status} {code}: {}", self.message),
            (Some(status), None) => write!(f, "{status}: {}", self.message),
            _ => f.write_str(&self.message),
        }
    }
}

#[async_trait]
pub trait TurnHub: Send + Sync + 'static {
    async fn list(&self) -> Result<Vec<BotTurn>, HubFailure>;
    async fn claim(&self, turn_id: &str, host_id: &str) -> Result<BotTurn, HubFailure>;
    async fn heartbeat(&self, turn_id: &str, host_id: &str) -> Result<BotTurn, HubFailure>;
    /// The newest entries of the bot's copy of the conversation, newest first.
    async fn entries(
        &self,
        bot_id: &str,
        conversation_id: &str,
    ) -> Result<Vec<InboxEntry>, HubFailure>;
    async fn send(
        &self,
        bot_id: &str,
        conversation_id: &str,
        message: Value,
        turn_id: &str,
        host_id: &str,
    ) -> Result<(), HubFailure>;
    /// The ids of the signed-in account's bots that run on a desktop, at most [`MAX_HOST_BOTS`].
    async fn desktop_bots(&self) -> Result<Vec<String>, HubFailure>;
    /// Reports the bots this host runs; answers the ids the hub refused.
    async fn report_host(
        &self,
        host_id: &str,
        bot_ids: &[String],
    ) -> Result<Vec<String>, HubFailure>;
    async fn delete_host(&self, host_id: &str) -> Result<(), HubFailure>;
    /// The owner's cancel of a turn, whoever holds it.
    async fn cancel_turn(&self, bot_id: &str, turn_id: &str) -> Result<(), HubFailure>;
    /// A token that acts as the bot during one of its turns, minted from the owner's session; the
    /// hub reads the bot's rights, not the owner's, from it. `None` for a hub that mints none.
    async fn runtime_token(
        &self,
        _bot_id: &str,
        _turn_id: &str,
    ) -> Result<Option<String>, HubFailure> {
        Ok(None)
    }
    /// The same hub with every call made as the bot by `token`, for what the bot does during its
    /// turn; `None` for a hub that has no such identity. Owner-only calls keep `self`.
    fn with_runtime_token(&self, _token: &str) -> Option<Arc<dyn TurnHub>> {
        None
    }
    /// Every grant and answer token of the account bot as `(tool, scope)`.
    async fn tool_keys(&self, _bot_id: &str) -> Result<Vec<(String, String)>, HubFailure> {
        Ok(Vec::new())
    }
    async fn put_tool_key(&self, _bot_id: &str, _tool: &str, _scope: &str) -> Result<(), HubFailure> {
        Err(HubFailure::offline("This hub keeps no tool grants"))
    }
    async fn delete_tool_key(
        &self,
        _bot_id: &str,
        _tool: &str,
        _scope: &str,
    ) -> Result<bool, HubFailure> {
        Ok(false)
    }
    /// Parks the turn this host holds behind an approval card the hub posts as the bot.
    async fn park(
        &self,
        _turn_id: &str,
        _host_id: &str,
        _card: &ApprovalRequest,
    ) -> Result<(), HubFailure> {
        Err(HubFailure::offline("This hub cannot park turns"))
    }
    /// For a turn a late answer woke: the question and the answer. `None` for any other turn.
    async fn ask_result(
        &self,
        _bot_id: &str,
        _turn_id: &str,
    ) -> Result<Option<AskResult>, HubFailure> {
        Ok(None)
    }
    /// A GET with the signed-in session: how an account bot's memory reads its conversation and
    /// summaries by position.
    async fn read(
        &self,
        _path: &str,
        _query: Vec<(&'static str, String)>,
    ) -> Result<Value, HubFailure> {
        Err(HubFailure::offline("This hub offers no inbox reads"))
    }
    /// A POST with the signed-in session: how an account bot's memory writes its summaries.
    async fn write(
        &self,
        _path: &str,
        _query: Vec<(&'static str, String)>,
        _body: Value,
    ) -> Result<Value, HubFailure> {
        Err(HubFailure::offline("This hub offers no inbox writes"))
    }
    /// A PUT with the signed-in session: how an encryption bot publishes its inbox key.
    async fn put(
        &self,
        _path: &str,
        _query: Vec<(&'static str, String)>,
        _body: Value,
    ) -> Result<Value, HubFailure> {
        Err(HubFailure::offline("This hub offers no key writes"))
    }
    /// A DELETE with the signed-in session: how a rotated inbox key is revoked.
    async fn delete(
        &self,
        _path: &str,
        _query: Vec<(&'static str, String)>,
    ) -> Result<Value, HubFailure> {
        Err(HubFailure::offline("This hub offers no key revocation"))
    }
}

#[derive(Deserialize)]
struct HubEntry {
    id: String,
    message_id: String,
    conversation_id: String,
    sender_id: String,
    kind: InboxEntryKind,
    envelope: Value,
    created_at: DateTime<FixedOffset>,
    seq: Option<i64>,
    #[serde(default)]
    cursor: String,
}

#[derive(Deserialize)]
struct HubEntries {
    entries: Vec<HubEntry>,
}

impl From<HubEntry> for InboxEntry {
    fn from(entry: HubEntry) -> Self {
        InboxEntry {
            id: entry.id,
            message_id: entry.message_id,
            conversation_id: entry.conversation_id,
            sender_id: entry.sender_id,
            kind: entry.kind,
            envelope: entry.envelope,
            created_at: entry.created_at,
            seq: entry.seq,
            cursor: entry.cursor,
        }
    }
}

pub fn entries_of(value: Value) -> Result<Vec<InboxEntry>, HubFailure> {
    serde_json::from_value::<HubEntries>(value)
        .map(|page| page.entries.into_iter().map(InboxEntry::from).collect())
        .map_err(|_| HubFailure::offline("the hub's entries are not in the expected shape"))
}

/// The question a late answer belongs to, as `GET /bots/{id}/turns/{turn}/ask-result` says.
#[derive(Debug, Clone, Deserialize)]
pub struct AskResult {
    pub origin_conversation: String,
    pub callee_name: String,
    pub question: String,
    pub answer: String,
    pub failed: bool,
}

pub struct ProfileHub {
    app: AppHandle,
    client: reqwest::Client,
    bearer: Option<String>,
}

impl ProfileHub {
    pub fn new(app: AppHandle) -> Self {
        let client = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(30))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .unwrap_or_default();
        Self {
            app,
            client,
            bearer: None,
        }
    }

    /// Asks the hub to notify the account's owner as `bot_id`; it groups, mutes and limits.
    pub(crate) async fn notify_owner(
        &self,
        bot_id: &str,
        notice: &flow_like_bot_tools::OwnerNotice,
    ) -> Result<flow_like_bot_tools::NoticeReceipt, HubFailure> {
        let answer = self
            .call(
                Method::POST,
                &format!("bots/{bot_id}/notify"),
                &[],
                Some(json!({ "title": notice.title, "body": notice.body, "link": notice.link })),
            )
            .await?;
        Ok(flow_like_bot_tools::NoticeReceipt {
            id: answer["id"].as_str().unwrap_or_default().to_owned(),
            muted: answer["muted"].as_bool().unwrap_or(false),
        })
    }

    async fn call(
        &self,
        method: Method,
        path: &str,
        query: &[(&str, String)],
        body: Option<Value>,
    ) -> Result<Value, HubFailure> {
        let identity = identity(&self.app, None)
            .await
            .map_err(HubFailure::offline)?;
        let url = format!("{}/api/v1/{path}", identity.hub.trim_end_matches('/'));
        let mut token = match (&self.bearer, identity.auth.as_ref()) {
            (Some(bearer), _) => bearer.clone(),
            (None, Some(auth)) => auth
                .cached_access_token()
                .await
                .map_err(HubFailure::offline)?,
            (None, None) => {
                return Err(HubFailure::answered(
                    401,
                    Some(CODE_SIGNED_OUT),
                    "Sign in to run account bots here",
                ));
            }
        };
        let session = identity.auth.as_ref().filter(|_| self.bearer.is_none());
        for attempt in 0..2 {
            let mut request = self
                .client
                .request(method.clone(), &url)
                .query(query)
                .bearer_auth(&token);
            if let Some(body) = &body {
                request = request.json(body);
            }
            let response = request
                .send()
                .await
                .map_err(|_| HubFailure::offline("the hub could not be reached"))?;
            let status = response.status().as_u16();
            if status == 401
                && attempt == 0
                && let Some(auth) = session
            {
                token = auth
                    .access_token(Some(&token))
                    .await
                    .map_err(HubFailure::offline)?;
                continue;
            }
            let bytes = response
                .bytes()
                .await
                .map_err(|_| HubFailure::offline("the hub's answer broke off"))?;
            if bytes.len() > MAX_RESPONSE_BYTES {
                return Err(HubFailure::offline("the hub's answer is too large"));
            }
            return answer(status, &bytes);
        }
        Err(HubFailure::offline("the hub refused the session"))
    }
}

fn answer(status: u16, body: &[u8]) -> Result<Value, HubFailure> {
    if (200..300).contains(&status) {
        if body.iter().all(u8::is_ascii_whitespace) {
            return Ok(Value::Null);
        }
        return serde_json::from_slice(body)
            .map_err(|_| HubFailure::offline("the hub's answer is not JSON"));
    }
    let error: Option<Value> = serde_json::from_slice::<Value>(body)
        .ok()
        .and_then(|value| value.get("error").cloned());
    let text = |field: &str| {
        error
            .as_ref()
            .and_then(|error| error.get(field))
            .and_then(Value::as_str)
    };
    let message: String = text("message")
        .map(|message| message.chars().take(MESSAGE_CHARS).collect())
        .unwrap_or_else(|| format!("the hub answered HTTP {status}"));
    Err(HubFailure::answered(status, text("code"), message))
}

fn decode<T: serde::de::DeserializeOwned>(value: Value) -> Result<T, HubFailure> {
    serde_json::from_value(value)
        .map_err(|_| HubFailure::offline("the hub's answer is not in the expected shape"))
}

#[async_trait]
impl TurnHub for ProfileHub {
    fn with_runtime_token(&self, token: &str) -> Option<Arc<dyn TurnHub>> {
        Some(Arc::new(ProfileHub {
            app: self.app.clone(),
            client: self.client.clone(),
            bearer: Some(token.to_owned()),
        }))
    }

    async fn runtime_token(
        &self,
        bot_id: &str,
        turn_id: &str,
    ) -> Result<Option<String>, HubFailure> {
        let minted = self
            .call(
                Method::POST,
                &format!("bots/{bot_id}/runtime-token"),
                &[],
                Some(json!({ "turn_id": turn_id })),
            )
            .await?;
        minted["token"]
            .as_str()
            .map(|token| Some(token.to_owned()))
            .ok_or_else(|| HubFailure::offline("the hub's runtime token is not in the expected shape"))
    }

    async fn list(&self) -> Result<Vec<BotTurn>, HubFailure> {
        let list = self
            .call(
                Method::GET,
                "bots/turns",
                &[("host_kind", "desktop".into())],
                None,
            )
            .await?;
        #[derive(Deserialize)]
        struct Turns {
            turns: Vec<BotTurn>,
        }
        decode::<Turns>(list).map(|list| list.turns)
    }

    async fn claim(&self, turn_id: &str, host_id: &str) -> Result<BotTurn, HubFailure> {
        let path = format!("bots/turns/{turn_id}/claim");
        let turn = self
            .call(
                Method::POST,
                &path,
                &[],
                Some(json!({ "host_id": host_id })),
            )
            .await?;
        decode(turn)
    }

    async fn heartbeat(&self, turn_id: &str, host_id: &str) -> Result<BotTurn, HubFailure> {
        let path = format!("bots/turns/{turn_id}/heartbeat");
        let turn = self
            .call(
                Method::POST,
                &path,
                &[],
                Some(json!({ "host_id": host_id })),
            )
            .await?;
        decode(turn)
    }

    async fn entries(
        &self,
        bot_id: &str,
        conversation_id: &str,
    ) -> Result<Vec<InboxEntry>, HubFailure> {
        let path = format!("inbox/conversations/{conversation_id}/entries");
        let page = self
            .call(
                Method::GET,
                &path,
                &[("as", bot_id.to_owned()), ("limit", ENTRIES.to_string())],
                None,
            )
            .await?;
        entries_of(page)
    }

    async fn send(
        &self,
        bot_id: &str,
        conversation_id: &str,
        message: Value,
        turn_id: &str,
        host_id: &str,
    ) -> Result<(), HubFailure> {
        let path = format!("inbox/conversations/{conversation_id}/entries");
        let mut body = message;
        body["turn"] = json!({ "id": turn_id, "host_id": host_id });
        self.call(
            Method::POST,
            &path,
            &[("as", bot_id.to_owned())],
            Some(body),
        )
        .await
        .map(drop)
    }

    async fn desktop_bots(&self) -> Result<Vec<String>, HubFailure> {
        let list = self.call(Method::GET, "bots", &[], None).await?;
        Ok(list
            .get("bots")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter(|bot| bot.get("location").and_then(Value::as_str) == Some("desktop"))
            .filter_map(|bot| bot.get("id").and_then(Value::as_str))
            .take(MAX_HOST_BOTS)
            .map(str::to_owned)
            .collect())
    }

    async fn report_host(
        &self,
        host_id: &str,
        bot_ids: &[String],
    ) -> Result<Vec<String>, HubFailure> {
        let result = self
            .call(
                Method::PUT,
                &format!("bots/hosts/{host_id}"),
                &[],
                Some(json!({ "host_kind": "desktop", "bot_ids": bot_ids })),
            )
            .await?;
        Ok(result
            .get("refused")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|id| id.as_str().map(str::to_owned))
            .collect())
    }

    async fn delete_host(&self, host_id: &str) -> Result<(), HubFailure> {
        self.call(Method::DELETE, &format!("bots/hosts/{host_id}"), &[], None)
            .await
            .map(drop)
    }

    async fn cancel_turn(&self, bot_id: &str, turn_id: &str) -> Result<(), HubFailure> {
        self.call(
            Method::POST,
            &format!("bots/{bot_id}/turns/{turn_id}/cancel"),
            &[],
            None,
        )
        .await
        .map(drop)
    }

    async fn tool_keys(&self, bot_id: &str) -> Result<Vec<(String, String)>, HubFailure> {
        let list = self
            .call(
                Method::GET,
                &format!("bots/{bot_id}/tool-grants"),
                &[("tokens", "true".into())],
                None,
            )
            .await?;
        Ok(list
            .get("grants")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|grant| {
                Some((
                    grant.get("tool")?.as_str()?.to_owned(),
                    grant.get("scope")?.as_str()?.to_owned(),
                ))
            })
            .collect())
    }

    async fn put_tool_key(&self, bot_id: &str, tool: &str, scope: &str) -> Result<(), HubFailure> {
        self.call(
            Method::PUT,
            &format!("bots/{bot_id}/tool-grants"),
            &[],
            Some(json!({ "tool": tool, "scope": scope })),
        )
        .await
        .map(drop)
    }

    async fn delete_tool_key(
        &self,
        bot_id: &str,
        tool: &str,
        scope: &str,
    ) -> Result<bool, HubFailure> {
        let removed = self
            .call(
                Method::DELETE,
                &format!("bots/{bot_id}/tool-grants"),
                &[],
                Some(json!({ "tool": tool, "scope": scope })),
            )
            .await?;
        Ok(removed.get("deleted").and_then(Value::as_bool) == Some(true))
    }

    async fn ask_result(
        &self,
        bot_id: &str,
        turn_id: &str,
    ) -> Result<Option<AskResult>, HubFailure> {
        let path = format!("bots/{bot_id}/turns/{turn_id}/ask-result");
        match self.call(Method::GET, &path, &[], None).await {
            Ok(value) => decode(value).map(Some),
            Err(failure) if failure.status == Some(404) => Ok(None),
            Err(failure) => Err(failure),
        }
    }

    async fn park(
        &self,
        turn_id: &str,
        host_id: &str,
        card: &ApprovalRequest,
    ) -> Result<(), HubFailure> {
        self.call(
            Method::POST,
            &format!("bots/turns/{turn_id}/park"),
            &[],
            Some(json!({ "host_id": host_id, "approval": card })),
        )
        .await
        .map(drop)
    }

    async fn read(
        &self,
        path: &str,
        query: Vec<(&'static str, String)>,
    ) -> Result<Value, HubFailure> {
        self.call(Method::GET, path, &query, None).await
    }

    async fn write(
        &self,
        path: &str,
        query: Vec<(&'static str, String)>,
        body: Value,
    ) -> Result<Value, HubFailure> {
        self.call(Method::POST, path, &query, Some(body)).await
    }

    async fn put(
        &self,
        path: &str,
        query: Vec<(&'static str, String)>,
        body: Value,
    ) -> Result<Value, HubFailure> {
        self.call(Method::PUT, path, &query, Some(body)).await
    }

    async fn delete(
        &self,
        path: &str,
        query: Vec<(&'static str, String)>,
    ) -> Result<Value, HubFailure> {
        self.call(Method::DELETE, path, &query, None).await
    }
}

/// An account bot's grants on the hub.
pub struct HubGrants {
    pub hub: Arc<dyn TurnHub>,
    pub bot_id: String,
}

fn failed(failure: HubFailure) -> flow_like_types::Error {
    flow_like_types::anyhow!("{failure}")
}

#[async_trait]
impl GrantStore for HubGrants {
    async fn list(&self) -> flow_like_types::Result<Vec<GrantKey>> {
        let keys = self.hub.tool_keys(&self.bot_id).await.map_err(failed)?;
        Ok(keys
            .into_iter()
            .map(|(tool, scope)| GrantKey::new(tool, scope))
            .collect())
    }

    async fn put(&self, tool: &str, scope: &str) -> flow_like_types::Result<()> {
        self.hub
            .put_tool_key(&self.bot_id, tool, scope)
            .await
            .map_err(failed)
    }

    async fn delete(&self, tool: &str, scope: &str) -> flow_like_types::Result<bool> {
        self.hub
            .delete_tool_key(&self.bot_id, tool, scope)
            .await
            .map_err(failed)
    }
}

impl ProfileHub {
    /// One call to the signed-in account's hub for the bot-talk host.
    pub(crate) async fn request(
        &self,
        method: Method,
        path: &str,
        body: Option<Value>,
    ) -> Result<Value, HubFailure> {
        self.call(method, path, &[], body).await
    }
}
