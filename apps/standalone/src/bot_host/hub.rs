//! The device's side of `/devices/{id}/bots/...`: turn lifecycle calls are DPoP-authorized as the
//! device, everything the bot does during a turn (inbox reads, grants, memory) uses the bot token
//! the claim returned. Both token kinds are refreshed from heartbeats.

use async_trait::async_trait;
use flow_like_bot_host::hub::{
    CODE_SIGNED_OUT, Claimed, ENTRIES, HubFailure, TurnHub, TurnSession, WaitOutcome, answer,
    decode, entries_of,
};
use flow_like_local_bots::InboxEntry;
use flow_like_types::{
    bot_message::ApprovalRequest,
    bot_turn::BotTurn,
    device_bot::{
        DeviceHeartbeatRequest, DeviceHeartbeatResponse, DevicePollResponse, DeviceReleaseRequest,
        DeviceReplyRequest, DeviceSilentRequest, DeviceToken, DeviceTurnGrant, DeviceUsageGrant,
        DeviceUsageReport, MAX_POLL_MS,
    },
};
use reqwest::Method;
use serde::Serialize;
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex, PoisonError,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

const MAX_RESPONSE_BYTES: usize = 8 * 1024 * 1024;
const MIN_POLL_MS: u64 = 500;

#[async_trait]
pub trait DeviceApi: Send + Sync + 'static {
    fn base_url(&self) -> &str;
    fn device_id(&self) -> &str;
    async fn call(
        &self,
        method: Method,
        path: &str,
        body: Option<&Value>,
    ) -> anyhow::Result<(u16, Vec<u8>)>;
}

#[async_trait]
impl DeviceApi for crate::enrollment::DeviceSession {
    fn base_url(&self) -> &str {
        self.api_base_url()
    }

    fn device_id(&self) -> &str {
        crate::enrollment::DeviceSession::device_id(self)
    }

    async fn call(
        &self,
        method: Method,
        path: &str,
        body: Option<&Value>,
    ) -> anyhow::Result<(u16, Vec<u8>)> {
        self.api_call(method, path, body, MAX_RESPONSE_BYTES).await
    }
}

/// The credentials of one claimed turn; heartbeats replace them before they expire.
pub struct TurnCreds {
    delivery_only: bool,
    bot: Mutex<DeviceToken>,
    executor: Mutex<Option<DeviceToken>>,
    usage: Option<DeviceUsageGrant>,
    started: Instant,
    deadline: i64,
}

fn locked<T>(cell: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    cell.lock().unwrap_or_else(PoisonError::into_inner)
}

impl TurnCreds {
    pub fn new(grant: &DeviceTurnGrant) -> Self {
        Self {
            delivery_only: grant.delivery_only,
            bot: Mutex::new(grant.bot_token.clone()),
            executor: Mutex::new(
                (!grant.delivery_only)
                    .then(|| grant.executor_token.clone())
                    .flatten(),
            ),
            usage: (!grant.delivery_only)
                .then(|| grant.usage.clone())
                .flatten(),
            started: Instant::now(),
            deadline: grant.deadline,
        }
    }

    pub fn bot_token(&self) -> String {
        locked(&self.bot).token.clone()
    }

    pub fn bot_expires_at(&self) -> i64 {
        locked(&self.bot).expires_at
    }

    pub fn executor_expires_at(&self) -> Option<i64> {
        locked(&self.executor).as_ref().map(|t| t.expires_at)
    }

    pub fn executor_token(&self) -> Option<String> {
        locked(&self.executor).as_ref().map(|t| t.token.clone())
    }

    fn refresh(&self, beat: &DeviceHeartbeatResponse) {
        if let Some(token) = &beat.bot_token {
            *locked(&self.bot) = token.clone();
        }
        if let Some(token) = &beat.executor_token
            && !self.delivery_only
        {
            *locked(&self.executor) = Some(token.clone());
        }
    }

    fn usage_report(&self) -> Option<DeviceUsageReport> {
        self.usage.as_ref().map(|usage| DeviceUsageReport {
            operation_id: usage.operation_id.clone(),
            elapsed_ms: i64::try_from(self.started.elapsed().as_millis()).unwrap_or(i64::MAX),
        })
    }
}

/// What the runner gets from the claim.
pub struct DeviceRunSpec {
    pub grant: DeviceTurnGrant,
    pub creds: Arc<TurnCreds>,
    /// The hub as the bot, for what the turn does besides the host's own calls.
    pub bot_hub: Arc<dyn TurnHub>,
}

struct Shared {
    api: Arc<dyn DeviceApi>,
    http: reqwest::Client,
    host_id: String,
    turns: Mutex<HashMap<String, Arc<TurnCreds>>>,
    next_poll_ms: AtomicU64,
}

#[derive(Clone)]
pub struct DeviceTurnHub {
    shared: Arc<Shared>,
    bearer: Option<Arc<TurnCreds>>,
}

fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| {
            i64::try_from(elapsed.as_secs()).unwrap_or(i64::MAX)
        })
}

impl DeviceTurnHub {
    pub fn new(api: Arc<dyn DeviceApi>, host_id: String) -> Self {
        let http = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(30))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .unwrap_or_default();
        Self {
            shared: Arc::new(Shared {
                api,
                http,
                host_id,
                turns: Mutex::default(),
                next_poll_ms: AtomicU64::new(flow_like_types::device_bot::IDLE_POLL_MS),
            }),
            bearer: None,
        }
    }

    pub fn host_id(&self) -> &str {
        &self.shared.host_id
    }

    fn device_path(&self, tail: &str) -> String {
        format!("/devices/{}/bots/{tail}", self.shared.api.device_id())
    }

    async fn device_call(
        &self,
        method: Method,
        tail: &str,
        body: Option<Value>,
    ) -> Result<Value, HubFailure> {
        let path = self.device_path(tail);
        let (status, bytes) = self
            .shared
            .api
            .call(method, &path, body.as_ref())
            .await
            .map_err(|error| {
                HubFailure::offline(format!(
                    "the hub could not be reached for {path}: {error:#}"
                ))
            })?;
        answer(status, &bytes)
    }

    async fn device_post<T: serde::de::DeserializeOwned>(
        &self,
        tail: &str,
        body: impl Serialize,
    ) -> Result<T, HubFailure> {
        let body = serde_json::to_value(body).map_err(|error| {
            HubFailure::offline(format!("the request for {tail} is not JSON: {error}"))
        })?;
        decode(self.device_call(Method::POST, tail, Some(body)).await?)
    }

    async fn bearer_call(
        &self,
        method: Method,
        path: &str,
        query: &[(&str, String)],
        body: Option<Value>,
    ) -> Result<Value, HubFailure> {
        let Some(creds) = &self.bearer else {
            return Err(HubFailure::answered(
                401,
                Some(CODE_SIGNED_OUT),
                format!("{path} needs the bot token of a claimed turn"),
            ));
        };
        let url = format!(
            "{}/{path}",
            self.shared.api.base_url().trim_end_matches('/')
        );
        let mut request = self
            .shared
            .http
            .request(method, &url)
            .query(query)
            .bearer_auth(creds.bot_token());
        if let Some(body) = &body {
            request = request.json(body);
        }
        let response = request
            .send()
            .await
            .map_err(|_| HubFailure::offline(format!("the hub could not be reached for {path}")))?;
        let status = response.status().as_u16();
        let bytes = response
            .bytes()
            .await
            .map_err(|_| HubFailure::offline(format!("the hub's answer for {path} broke off")))?;
        if bytes.len() > MAX_RESPONSE_BYTES {
            return Err(HubFailure::offline(format!(
                "the hub's answer for {path} is too large"
            )));
        }
        answer(status, &bytes)
    }

    fn creds_of(&self, turn_id: &str) -> Option<Arc<TurnCreds>> {
        locked(&self.shared.turns).get(turn_id).cloned()
    }

    fn usage_of(&self, turn_id: &str) -> Option<DeviceUsageReport> {
        self.creds_of(turn_id)
            .and_then(|creds| creds.usage_report())
    }

    fn end_turn(&self, turn_id: &str) {
        locked(&self.shared.turns).remove(turn_id);
    }

    fn remember_turn(&self, turn_id: &str, creds: Arc<TurnCreds>) {
        let now = unix_now();
        let mut turns = locked(&self.shared.turns);
        turns.retain(|_, held| held.deadline > now);
        turns.insert(turn_id.to_owned(), creds);
    }

    async fn give_up(
        &self,
        turn_id: &str,
        host_id: &str,
        code: &str,
        retry: bool,
    ) -> Result<(), HubFailure> {
        let request = DeviceReleaseRequest {
            host_id: host_id.to_owned(),
            error_code: code.to_owned(),
            retry,
            usage: self.usage_of(turn_id),
        };
        let _: Value = self
            .device_post(&format!("turns/{turn_id}/release"), request)
            .await?;
        self.end_turn(turn_id);
        Ok(())
    }

    /// Tells the hub this host is gone so the device's bots stop reading online.
    pub async fn forget(&self) -> Result<u64, HubFailure> {
        let answered = self
            .device_call(
                Method::POST,
                "hosts/forget",
                Some(json!({ "host_id": self.shared.host_id })),
            )
            .await?;
        Ok(answered.get("removed").and_then(Value::as_u64).unwrap_or(0))
    }

    /// Hands every held turn back for a retry; used on shutdown.
    pub async fn release_all(&self, code: &str) {
        let held: Vec<String> = locked(&self.shared.turns).keys().cloned().collect();
        for turn_id in held {
            if let Err(failure) = self.release(&turn_id, &self.shared.host_id, code).await {
                tracing::debug!(%failure, turn_id, "Bot host: releasing a turn on shutdown failed");
            }
        }
    }
}

fn failure_code_of(message: &Value) -> Option<String> {
    message["envelope"]["body"]["error"]["code"]
        .as_str()
        .map(str::to_owned)
}

#[async_trait]
impl TurnHub for DeviceTurnHub {
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
        let query = {
            let mut query = url::form_urlencoded::Serializer::new(String::new());
            query
                .append_pair("host_id", host_id)
                .append_pair("attempt", &attempt.to_string());
            query.finish()
        };
        let value = self
            .device_call(
                Method::GET,
                &format!("turns/{turn_id}/checkpoint?{query}"),
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
        let query = {
            let mut query = url::form_urlencoded::Serializer::new(String::new());
            query
                .append_pair("host_id", host_id)
                .append_pair("attempt", &attempt.to_string())
                .append_pair("monitor_baseline", "true");
            query.finish()
        };
        let value = self
            .device_call(
                Method::GET,
                &format!("turns/{turn_id}/checkpoint?{query}"),
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
        self.device_call(
            Method::PUT,
            &format!("turns/{turn_id}/checkpoint"),
            Some(json!({"host_id": host_id, "attempt": attempt, "checkpoint": checkpoint})),
        )
        .await
        .map(|_| ())
    }
    async fn list(&self) -> Result<Vec<BotTurn>, HubFailure> {
        let polled: DevicePollResponse = self
            .device_post("poll", json!({ "host_id": self.shared.host_id }))
            .await?;
        self.shared.next_poll_ms.store(
            polled.next_poll_ms.clamp(MIN_POLL_MS, MAX_POLL_MS),
            Ordering::Relaxed,
        );
        Ok(polled.turns)
    }

    fn next_poll(&self) -> Option<Duration> {
        Some(Duration::from_millis(
            self.shared.next_poll_ms.load(Ordering::Relaxed),
        ))
    }

    async fn claim(&self, turn_id: &str, host_id: &str) -> Result<BotTurn, HubFailure> {
        Ok(self.claim_turn(turn_id, host_id).await?.turn)
    }

    async fn claim_turn(&self, turn_id: &str, host_id: &str) -> Result<Claimed, HubFailure> {
        let grant: DeviceTurnGrant = self
            .device_post(
                &format!("turns/{turn_id}/claim"),
                json!({ "host_id": host_id }),
            )
            .await?;
        let creds = Arc::new(TurnCreds::new(&grant));
        self.remember_turn(&grant.turn.id, creds.clone());
        let turn = grant.turn.clone();
        let bearer: Arc<dyn TurnHub> = Arc::new(Self {
            shared: self.shared.clone(),
            bearer: Some(creds.clone()),
        });
        Ok(Claimed {
            turn,
            session: Some(TurnSession {
                token: Some(creds.bot_token()),
                hub: Some(bearer.clone()),
                run: Some(Arc::new(DeviceRunSpec {
                    grant,
                    creds,
                    bot_hub: bearer.clone(),
                })),
            }),
        })
    }

    async fn heartbeat(&self, turn_id: &str, host_id: &str) -> Result<BotTurn, HubFailure> {
        let request = DeviceHeartbeatRequest {
            host_id: host_id.to_owned(),
            bot_token_expires_at: self.creds_of(turn_id).map(|creds| creds.bot_expires_at()),
            executor_token_expires_at: self
                .creds_of(turn_id)
                .and_then(|creds| creds.executor_expires_at()),
        };
        let beat: DeviceHeartbeatResponse = self
            .device_post(&format!("turns/{turn_id}/heartbeat"), request)
            .await?;
        if let Some(creds) = self.creds_of(turn_id) {
            creds.refresh(&beat);
        }
        Ok(beat.turn)
    }

    async fn release(&self, turn_id: &str, host_id: &str, code: &str) -> Result<(), HubFailure> {
        self.give_up(turn_id, host_id, code, true).await
    }

    async fn abandon(&self, turn_id: &str, host_id: &str, code: &str) -> Result<(), HubFailure> {
        self.give_up(turn_id, host_id, code, false).await
    }

    async fn silent(&self, turn_id: &str, host_id: &str) -> Result<(), HubFailure> {
        let request = DeviceSilentRequest {
            host_id: host_id.to_owned(),
            usage: self.usage_of(turn_id),
        };
        let _: Value = self
            .device_post(&format!("turns/{turn_id}/silent"), request)
            .await?;
        self.end_turn(turn_id);
        Ok(())
    }

    async fn wait_for(
        &self,
        turn_id: &str,
        host_id: &str,
        participant_id: &str,
    ) -> Result<WaitOutcome, HubFailure> {
        let result: Value = self
            .device_post(
                &format!("turns/{turn_id}/wait"),
                json!({
                    "host_id": host_id,
                    "participant_id": participant_id,
                    "usage": self.usage_of(turn_id),
                }),
            )
            .await?;
        let outcome: WaitOutcome = decode(result.get("status").cloned().unwrap_or(Value::Null))?;
        if outcome == WaitOutcome::Parked {
            self.end_turn(turn_id);
        }
        Ok(outcome)
    }

    async fn park(
        &self,
        turn_id: &str,
        host_id: &str,
        card: &ApprovalRequest,
    ) -> Result<(), HubFailure> {
        let request = flow_like_types::device_bot::DeviceParkRequest {
            host_id: host_id.to_owned(),
            approval: card.clone(),
            usage: self.usage_of(turn_id),
        };
        let _: Value = self
            .device_post(&format!("turns/{turn_id}/park"), request)
            .await?;
        self.end_turn(turn_id);
        Ok(())
    }

    async fn send(
        &self,
        _bot_id: &str,
        conversation_id: &str,
        message: Value,
        turn_id: &str,
        host_id: &str,
    ) -> Result<(), HubFailure> {
        let (Some(message_id), Some(envelope)) = (
            message["message_id"].as_str(),
            message
                .get("envelope")
                .filter(|envelope| !envelope.is_null()),
        ) else {
            return Err(HubFailure::offline(format!(
                "a reply to conversation {conversation_id} has no plain envelope; sealed replies are not supported on devices"
            )));
        };
        let request = DeviceReplyRequest {
            host_id: host_id.to_owned(),
            message_id: message_id.to_owned(),
            envelope: envelope.clone(),
            failure_code: failure_code_of(&message),
            usage: self.usage_of(turn_id),
        };
        let _: Value = self
            .device_post(&format!("turns/{turn_id}/reply"), request)
            .await?;
        self.end_turn(turn_id);
        Ok(())
    }

    async fn entries(
        &self,
        bot_id: &str,
        conversation_id: &str,
    ) -> Result<Vec<InboxEntry>, HubFailure> {
        let path = format!("inbox/conversations/{conversation_id}/entries");
        let page = self
            .bearer_call(
                Method::GET,
                &path,
                &[("as", bot_id.to_owned()), ("limit", ENTRIES.to_string())],
                None,
            )
            .await?;
        entries_of(page)
    }

    async fn desktop_bots(&self) -> Result<Vec<String>, HubFailure> {
        Ok(Vec::new())
    }

    async fn report_host(
        &self,
        _host_id: &str,
        _bot_ids: &[String],
    ) -> Result<Vec<String>, HubFailure> {
        Ok(Vec::new())
    }

    async fn delete_host(&self, _host_id: &str) -> Result<(), HubFailure> {
        self.forget().await.map(drop)
    }

    async fn cancel_turn(&self, bot_id: &str, turn_id: &str) -> Result<(), HubFailure> {
        Err(HubFailure::offline(format!(
            "a device cannot cancel turn {turn_id} of bot {bot_id}; its lease will lapse"
        )))
    }

    async fn tool_keys(&self, bot_id: &str) -> Result<Vec<(String, String)>, HubFailure> {
        let list = self
            .bearer_call(
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
        self.bearer_call(
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
            .bearer_call(
                Method::DELETE,
                &format!("bots/{bot_id}/tool-grants"),
                &[],
                Some(json!({ "tool": tool, "scope": scope })),
            )
            .await?;
        Ok(removed.get("deleted").and_then(Value::as_bool) == Some(true))
    }

    async fn read(
        &self,
        path: &str,
        query: Vec<(&'static str, String)>,
    ) -> Result<Value, HubFailure> {
        self.bearer_call(Method::GET, path, &query, None).await
    }

    async fn write(
        &self,
        path: &str,
        query: Vec<(&'static str, String)>,
        body: Value,
    ) -> Result<Value, HubFailure> {
        self.bearer_call(Method::POST, path, &query, Some(body))
            .await
    }

    async fn put(
        &self,
        path: &str,
        query: Vec<(&'static str, String)>,
        body: Value,
    ) -> Result<Value, HubFailure> {
        self.bearer_call(Method::PUT, path, &query, Some(body))
            .await
    }

    async fn delete(
        &self,
        path: &str,
        query: Vec<(&'static str, String)>,
    ) -> Result<Value, HubFailure> {
        self.bearer_call(Method::DELETE, path, &query, None).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use flow_like_types::bot_turn::{TurnAudience, TurnHostKind, TurnStatus};
    use std::collections::VecDeque;

    type Recorded = Vec<(Method, String, Option<Value>)>;

    #[derive(Default)]
    struct FakeApi {
        answers: Mutex<VecDeque<(u16, Value)>>,
        requests: Mutex<Recorded>,
    }

    impl FakeApi {
        fn answering(answers: impl IntoIterator<Item = (u16, Value)>) -> Arc<Self> {
            Arc::new(Self {
                answers: Mutex::new(answers.into_iter().collect()),
                requests: Mutex::default(),
            })
        }

        fn requests(&self) -> Recorded {
            self.requests.lock().unwrap().clone()
        }
    }

    #[async_trait]
    impl DeviceApi for FakeApi {
        fn base_url(&self) -> &str {
            "https://hub.test/api/v1"
        }

        fn device_id(&self) -> &str {
            "dev1"
        }

        async fn call(
            &self,
            method: Method,
            path: &str,
            body: Option<&Value>,
        ) -> anyhow::Result<(u16, Vec<u8>)> {
            self.requests
                .lock()
                .unwrap()
                .push((method, path.to_owned(), body.cloned()));
            let (status, body) = self
                .answers
                .lock()
                .unwrap()
                .pop_front()
                .ok_or_else(|| anyhow::anyhow!("no scripted answer for {path}"))?;
            Ok((status, serde_json::to_vec(&body)?))
        }
    }

    fn turn(id: &str) -> BotTurn {
        BotTurn {
            id: id.into(),
            bot_id: "bot".into(),
            conversation_id: "conv".into(),
            trigger_message_id: "m1".into(),
            host_kind: TurnHostKind::Device,
            status: TurnStatus::Running,
            attempts: 1,
            host_id: Some("device:dev1".into()),
            lease_until: None,
            reply_message_id: None,
            error_code: None,
            hops: 0,
            owner_id: "owner".into(),
            requester_id: None,
            requester_name: None,
            audience: TurnAudience::Owner,
            created_at: Utc::now().to_rfc3339(),
            updated_at: Utc::now().to_rfc3339(),
            batched: Vec::new(),
        }
    }

    fn grant(id: &str) -> Value {
        json!({
            "turn": turn(id),
            "host_id": "device:dev1",
            "conversation": { "id": "conv", "kind": "direct" },
            "bot": { "bot_id": "bot", "name": "Ada" },
            "models": { "model_id": "m", "completion": {} },
            "bot_token": { "token": "bot-1", "expires_at": 100 },
            "executor_token": { "token": "exec-1", "expires_at": 100 },
            "usage": { "operation_id": "op-1", "deadline_ms": 1 },
            "deadline": unix_now() + 600,
        })
    }

    fn hub(api: &Arc<FakeApi>) -> DeviceTurnHub {
        DeviceTurnHub::new(api.clone(), "device:dev1".into())
    }

    #[test]
    fn delivery_credentials_cannot_acquire_execution_tokens_or_usage() {
        let mut value = grant("bt_delivery");
        value["delivery_only"] = json!(true);
        let grant: DeviceTurnGrant = serde_json::from_value(value).unwrap();
        let creds = TurnCreds::new(&grant);
        assert!(creds.executor_token().is_none());
        assert!(creds.executor_expires_at().is_none());
        assert!(creds.usage_report().is_none());
        creds.refresh(&DeviceHeartbeatResponse {
            turn: grant.turn.clone(),
            bot_token: Some(DeviceToken {
                token: "refreshed-bot".into(),
                expires_at: 200,
            }),
            executor_token: Some(DeviceToken {
                token: "unexpected-executor".into(),
                expires_at: 200,
            }),
        });
        assert_eq!(creds.bot_token(), "refreshed-bot");
        assert!(creds.executor_token().is_none());
    }

    #[tokio::test]
    async fn a_poll_lists_turns_and_sets_the_next_poll_within_bounds() {
        let api = FakeApi::answering([(
            200,
            json!({ "bots": [], "turns": [turn("bt_1")], "next_poll_ms": 1 }),
        )]);
        let hub = hub(&api);
        let turns = hub.list().await.unwrap();
        assert_eq!(turns.len(), 1);
        assert_eq!(hub.next_poll(), Some(Duration::from_millis(MIN_POLL_MS)));
        let (method, path, body) = api.requests().remove(0);
        assert_eq!(
            (method, path.as_str()),
            (Method::POST, "/devices/dev1/bots/poll")
        );
        assert_eq!(body, Some(json!({ "host_id": "device:dev1" })));
    }

    #[tokio::test]
    async fn a_claim_returns_the_bot_token_and_the_run_spec() {
        let api = FakeApi::answering([(200, grant("bt_1"))]);
        let claimed = hub(&api).claim_turn("bt_1", "device:dev1").await.unwrap();
        let session = claimed.session.unwrap();
        assert_eq!(session.token.as_deref(), Some("bot-1"));
        let spec = session
            .run
            .unwrap()
            .downcast::<DeviceRunSpec>()
            .ok()
            .unwrap();
        assert_eq!(spec.creds.executor_token().as_deref(), Some("exec-1"));
        assert_eq!(api.requests()[0].1, "/devices/dev1/bots/turns/bt_1/claim");
    }

    #[tokio::test]
    async fn a_heartbeat_replaces_the_tokens_the_turn_uses() {
        let api = FakeApi::answering([
            (200, grant("bt_1")),
            (
                200,
                json!({ "turn": turn("bt_1"), "bot_token": { "token": "bot-2", "expires_at": 900 } }),
            ),
        ]);
        let hub = hub(&api);
        let claimed = hub.claim_turn("bt_1", "device:dev1").await.unwrap();
        hub.heartbeat("bt_1", "device:dev1").await.unwrap();
        let creds = hub.creds_of("bt_1").unwrap();
        assert_eq!(creds.bot_token(), "bot-2");
        assert_eq!(creds.executor_token().as_deref(), Some("exec-1"));
        assert!(claimed.session.is_some());
    }

    #[tokio::test]
    async fn a_reply_carries_the_envelope_the_failure_code_and_the_usage_report() {
        let api = FakeApi::answering([
            (200, grant("bt_1")),
            (
                200,
                json!({ "turn": turn("bt_1"), "message_id": "rf-1", "posted": true }),
            ),
        ]);
        let hub = hub(&api);
        hub.claim_turn("bt_1", "device:dev1").await.unwrap();
        let message = json!({
            "message_id": "rf-1",
            "kind": "message",
            "envelope": flow_like_types::inbox::plain_envelope(
                json!({ "text": "no", "error": { "code": "BOT_TURN_MODEL_ERROR" } })
            ),
        });
        hub.send("bot", "conv", message, "bt_1", "device:dev1")
            .await
            .unwrap();
        let (_, path, body) = api.requests().remove(1);
        assert_eq!(path, "/devices/dev1/bots/turns/bt_1/reply");
        let body = body.unwrap();
        assert_eq!(body["message_id"], "rf-1");
        assert_eq!(body["failure_code"], "BOT_TURN_MODEL_ERROR");
        assert_eq!(body["usage"]["operation_id"], "op-1");
        assert!(hub.creds_of("bt_1").is_none());
    }

    #[tokio::test]
    async fn a_sealed_reply_is_refused_before_any_request() {
        let api = FakeApi::answering([]);
        let message = json!({ "message_id": "re-1", "kind": "message", "envelopes": [] });
        let failure = hub(&api)
            .send("bot", "conv", message, "bt_1", "device:dev1")
            .await
            .unwrap_err();
        assert!(failure.message.contains("sealed replies"));
        assert!(api.requests().is_empty());
    }

    #[tokio::test]
    async fn release_retries_and_abandon_fails_the_turn() {
        let api = FakeApi::answering([
            (200, json!({ "turn": turn("bt_1") })),
            (200, json!({ "turn": turn("bt_2") })),
        ]);
        let hub = hub(&api);
        hub.release("bt_1", "device:dev1", "BOT_TURN_FAILED")
            .await
            .unwrap();
        hub.abandon("bt_2", "device:dev1", "BOT_ENCRYPTED")
            .await
            .unwrap();
        let requests = api.requests();
        assert_eq!(requests[0].2.as_ref().unwrap()["retry"], true);
        assert_eq!(requests[1].2.as_ref().unwrap()["retry"], false);
        assert_eq!(
            requests[1].2.as_ref().unwrap()["error_code"],
            "BOT_ENCRYPTED"
        );
    }

    #[tokio::test]
    async fn every_call_that_ends_a_turn_echoes_the_usage_operation() {
        let ended = || json!({ "turn": turn("bt_1") });
        let card: flow_like_types::bot_message::ApprovalRequest = serde_json::from_value(json!({
            "id": "a1", "turn_id": "bt_1", "tool": "file_save", "summary": "s",
            "scope": "", "args_hash": "h",
        }))
        .unwrap();
        let plain = flow_like_types::inbox::plain_envelope(json!({ "text": "hi" }));
        let reply = json!({ "message_id": "re-1", "kind": "message", "envelope": plain });
        for (tail, end) in [("reply", 0), ("silent", 1), ("park", 2), ("release", 3)] {
            let api = FakeApi::answering([
                (200, grant("bt_1")),
                (
                    200,
                    if end == 0 {
                        json!({ "turn": turn("bt_1"), "message_id": "re-1", "posted": true })
                    } else {
                        ended()
                    },
                ),
            ]);
            let hub = hub(&api);
            hub.claim_turn("bt_1", "device:dev1").await.unwrap();
            match end {
                0 => {
                    hub.send("bot", "conv", reply.clone(), "bt_1", "device:dev1")
                        .await
                }
                1 => hub.silent("bt_1", "device:dev1").await,
                2 => hub.park("bt_1", "device:dev1", &card).await,
                _ => hub.release("bt_1", "device:dev1", "BOT_TURN_FAILED").await,
            }
            .unwrap();
            let (_, path, body) = api.requests().remove(1);
            assert_eq!(path, format!("/devices/dev1/bots/turns/bt_1/{tail}"));
            assert_eq!(body.unwrap()["usage"]["operation_id"], "op-1", "{tail}");
        }
    }

    #[tokio::test]
    async fn participant_wait_uses_device_auth_and_releases_usage_only_when_parked() {
        for (status, expected) in [
            ("parked", WaitOutcome::Parked),
            ("already_answered", WaitOutcome::AlreadyAnswered),
            ("unavailable", WaitOutcome::Unavailable),
            ("superseded", WaitOutcome::Superseded),
        ] {
            let api =
                FakeApi::answering([(200, grant("bt_1")), (200, json!({ "status": status }))]);
            let hub = hub(&api);
            hub.claim_turn("bt_1", "device:dev1").await.unwrap();
            assert_eq!(
                hub.wait_for("bt_1", "device:dev1", "peer").await.unwrap(),
                expected
            );
            let (_, path, body) = api.requests().remove(1);
            assert_eq!(path, "/devices/dev1/bots/turns/bt_1/wait");
            let body = body.unwrap();
            assert_eq!(body["participant_id"], "peer");
            assert_eq!(body["usage"]["operation_id"], "op-1");
            assert_eq!(
                hub.usage_of("bt_1").is_none(),
                expected == WaitOutcome::Parked
            );
        }
    }

    #[tokio::test]
    async fn heartbeats_report_the_token_expiries() {
        let api =
            FakeApi::answering([(200, grant("bt_1")), (200, json!({ "turn": turn("bt_1") }))]);
        let hub = hub(&api);
        hub.claim_turn("bt_1", "device:dev1").await.unwrap();
        hub.heartbeat("bt_1", "device:dev1").await.unwrap();
        let body = api.requests().remove(1).2.unwrap();
        assert_eq!(body["bot_token_expires_at"], 100);
        assert_eq!(body["executor_token_expires_at"], 100);
    }

    #[tokio::test]
    async fn recovery_uses_device_identity_and_includes_the_claim_fence() {
        let checkpoint = flow_like_types::bot_turn::BotRunCheckpoint {
            revision: 1,
            started_at_ms: 1000,
            deadline_at_ms: 31_000,
            state: json!({"phase":"ready"}),
        };
        let api = FakeApi::answering([
            (200, json!({"checkpoint": checkpoint})),
            (200, json!({"ok":true})),
        ]);
        let hub = hub(&api);
        assert_eq!(
            hub.checkpoint_load("bot", "bt_1", "device:dev1", 2)
                .await
                .unwrap(),
            Some(checkpoint.clone())
        );
        hub.checkpoint_save("bot", "bt_1", "device:dev1", 2, &checkpoint)
            .await
            .unwrap();
        let requests = api.requests();
        assert_eq!(requests[0].0, Method::GET);
        assert_eq!(
            requests[0].1,
            "/devices/dev1/bots/turns/bt_1/checkpoint?host_id=device%3Adev1&attempt=2"
        );
        assert_eq!(requests[1].0, Method::PUT);
        assert_eq!(requests[1].1, "/devices/dev1/bots/turns/bt_1/checkpoint");
        assert_eq!(
            requests[1].2.as_ref().unwrap(),
            &json!({"host_id":"device:dev1","attempt":2,"checkpoint":checkpoint})
        );
    }

    #[tokio::test]
    async fn monitor_baselines_use_the_current_device_claim_and_cap_the_result() {
        let api = FakeApi::answering([(
            200,
            json!({"checkpoint":null,"monitor_result":"🙂".repeat(5_000)}),
        )]);
        let result = hub(&api)
            .previous_monitor_result("bot", "bt_1", "device:dev1", 2)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(result.chars().count(), 4_000);
        let requests = api.requests();
        assert_eq!(requests[0].0, Method::GET);
        assert_eq!(
            requests[0].1,
            "/devices/dev1/bots/turns/bt_1/checkpoint?host_id=device%3Adev1&attempt=2&monitor_baseline=true"
        );
        assert!(requests[0].2.is_none());
    }

    #[tokio::test]
    async fn a_lost_lease_is_recognised_from_the_hubs_error() {
        let api = FakeApi::answering([(
            409,
            json!({ "error": { "code": CODE_LEASE_LOST_CODE, "message": "taken" } }),
        )]);
        let failure = hub(&api)
            .heartbeat("bt_1", "device:dev1")
            .await
            .unwrap_err();
        assert!(failure.lease_lost());
    }

    #[tokio::test]
    async fn reads_as_the_bot_need_a_claimed_turn() {
        let api = FakeApi::answering([]);
        let failure = hub(&api).entries("bot", "conv").await.unwrap_err();
        assert!(failure.signed_out());
    }

    const CODE_LEASE_LOST_CODE: &str = flow_like_bot_host::hub::CODE_LEASE_LOST;
}
