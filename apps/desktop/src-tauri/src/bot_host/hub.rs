//! The desktop's client for the hub calls a bot's turn needs: the active profile's session (token
//! refreshed on a 401). The [`TurnHub`] trait and its helpers live in `flow-like-bot-host`.

use super::runner::identity;
use async_trait::async_trait;
use flow_like_local_bots::InboxEntry;
use flow_like_types::{
    bot::{BotCapability, BotLocation},
    bot_message::ApprovalRequest,
    bot_turn::BotTurn,
};
use reqwest::Method;
use serde::Deserialize;
use serde_json::{Value, json};
use std::sync::Arc;
use std::time::Duration;
use tauri::AppHandle;

pub use flow_like_bot_host::hub::*;

const MAX_RESPONSE_BYTES: usize = 8 * 1024 * 1024;

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

    /// Asks the hub to write a message from `bot_id` into the account owner's chat with it; it
    /// posts, groups, mutes and limits.
    pub(crate) async fn message_owner(
        &self,
        bot_id: &str,
        conversation_id: Option<&str>,
        notice: &flow_like_bot_tools::OwnerNotice,
    ) -> Result<flow_like_bot_tools::NoticeReceipt, HubFailure> {
        let answer = self
            .call(
                Method::POST,
                &format!("bots/{bot_id}/notify"),
                &[],
                Some(json!({
                    "text": notice.text,
                    "title": notice.title,
                    "conversation_id": conversation_id,
                })),
            )
            .await?;
        Ok(flow_like_bot_tools::NoticeReceipt {
            id: answer["id"].as_str().unwrap_or_default().to_owned(),
            muted: answer["muted"].as_bool().unwrap_or(false),
            posted: answer["posted"].as_bool().unwrap_or(false),
            in_owner_chat: answer["in_owner_chat"].as_bool().unwrap_or(false),
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

    /// The ids of the signed-in account's bots whose location and capabilities `keep` accepts, at
    /// most [`MAX_HOST_BOTS`].
    async fn bots_where(
        &self,
        keep: impl Fn(BotLocation, &[BotCapability]) -> bool,
    ) -> Result<Vec<String>, HubFailure> {
        let list = self.call(Method::GET, "bots", &[], None).await?;
        Ok(list
            .get("bots")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter(|bot| {
                let capabilities: Vec<BotCapability> = bot
                    .get("capabilities")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(Value::as_str)
                    .filter_map(BotCapability::parse)
                    .collect();
                bot.get("location")
                    .and_then(Value::as_str)
                    .and_then(BotLocation::parse)
                    .is_some_and(|location| keep(location, &capabilities))
            })
            .filter_map(|bot| bot.get("id").and_then(Value::as_str))
            .take(MAX_HOST_BOTS)
            .map(str::to_owned)
            .collect())
    }
}

#[async_trait]
impl TurnHub for ProfileHub {
    fn supports_recovery(&self) -> bool {
        true
    }

    async fn checkpoint_load(
        &self,
        _bot_id: &str,
        turn_id: &str,
        host_id: &str,
        attempt: u32,
    ) -> Result<Option<flow_like_types::bot_turn::BotRunCheckpoint>, HubFailure> {
        let value = self
            .call(
                Method::GET,
                &format!("bots/turns/{turn_id}/checkpoint"),
                &[
                    ("host_id", host_id.into()),
                    ("attempt", attempt.to_string()),
                ],
                None,
            )
            .await?;
        decode(value.get("checkpoint").cloned().unwrap_or(Value::Null))
    }

    async fn previous_monitor_result(
        &self,
        _bot_id: &str,
        turn_id: &str,
        host_id: &str,
        attempt: u32,
    ) -> Result<Option<String>, HubFailure> {
        let value = self
            .call(
                Method::GET,
                &format!("bots/turns/{turn_id}/checkpoint"),
                &[
                    ("host_id", host_id.into()),
                    ("attempt", attempt.to_string()),
                    ("monitor_baseline", "true".into()),
                ],
                None,
            )
            .await?;
        let result: Option<String> =
            decode(value.get("monitor_result").cloned().unwrap_or(Value::Null))?;
        Ok(result.map(|text| text.chars().take(4_000).collect()))
    }

    async fn checkpoint_save(
        &self,
        _bot_id: &str,
        turn_id: &str,
        host_id: &str,
        attempt: u32,
        checkpoint: &flow_like_types::bot_turn::BotRunCheckpoint,
    ) -> Result<(), HubFailure> {
        self.call(
            Method::PUT,
            &format!("bots/turns/{turn_id}/checkpoint"),
            &[],
            Some(json!({"host_id": host_id, "attempt": attempt, "checkpoint": checkpoint})),
        )
        .await
        .map(|_| ())
    }
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
            .ok_or_else(|| {
                HubFailure::offline("the hub's runtime token is not in the expected shape")
            })
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
        self.bots_where(|location, _| location == BotLocation::Desktop)
            .await
    }

    async fn reported_bots(&self) -> Result<Vec<String>, HubFailure> {
        self.bots_where(|location, capabilities| {
            location == BotLocation::Desktop || location.hands_off_to_desktop(capabilities)
        })
        .await
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

    async fn touch_tool_keys(
        &self,
        bot_id: &str,
        keys: &[(String, String)],
    ) -> Result<(), HubFailure> {
        let grants: Vec<Value> = keys
            .iter()
            .map(|(tool, scope)| json!({ "tool": tool, "scope": scope }))
            .collect();
        self.call(
            Method::PATCH,
            &format!("bots/{bot_id}/tool-grants"),
            &[],
            Some(json!({ "grants": grants })),
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

    async fn silent(&self, turn_id: &str, host_id: &str) -> Result<(), HubFailure> {
        self.call(
            Method::POST,
            &format!("bots/turns/{turn_id}/silent"),
            &[],
            Some(json!({ "host_id": host_id })),
        )
        .await
        .map(drop)
    }

    async fn wait_for(
        &self,
        turn_id: &str,
        host_id: &str,
        participant_id: &str,
    ) -> Result<WaitOutcome, HubFailure> {
        let value = self
            .call(
                Method::POST,
                &format!("bots/turns/{turn_id}/wait"),
                &[],
                Some(json!({ "host_id": host_id, "participant_id": participant_id })),
            )
            .await?;
        decode(value["status"].clone())
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

    async fn patch(
        &self,
        path: &str,
        query: Vec<(&'static str, String)>,
        body: Value,
    ) -> Result<Value, HubFailure> {
        self.call(Method::PATCH, path, &query, Some(body)).await
    }

    async fn delete(
        &self,
        path: &str,
        query: Vec<(&'static str, String)>,
    ) -> Result<Value, HubFailure> {
        self.call(Method::DELETE, path, &query, None).await
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
