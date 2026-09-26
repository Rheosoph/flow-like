//! Direct Line 3.0 for Copilot Studio agents that do not use Microsoft authentication (anonymous,
//! manual auth, B2C or cross-tenant). Direct Line has no end-of-turn signal, so a turn ends once
//! the reply to our message arrived and the channel stayed quiet.

use super::{
    CATEGORY, FLOWSCRIPT_NAMESPACE,
    agent::{D2E_API_VERSION, validate_environment_host},
    chat::{add_locale_pin, evaluate_locale},
    http_failure, scores,
    turn::{Turn, add_turn_output_pins},
};
use flow_like::flow::{
    execution::{LogLevel, context::ExecutionContext},
    node::{Node, NodeLogic},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_types::{
    JsonSchema, Result, Value, anyhow, async_trait,
    json::{self, json},
    reqwest::{self, Url},
    tokio::time::{Instant, sleep},
};
use serde::{Deserialize, Serialize};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const DIRECT_LINE_HOST: &str = "directline.botframework.com";
const DIRECT_LINE_PATH: &str = "/v3/directline";
const REGIONS: [&str; 4] = ["Auto", "Global", "Europe", "India"];
const DEFAULT_USER_ID: &str = "flow-like-user";
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
const POLL_INTERVAL: Duration = Duration::from_millis(900);
const QUIET_PERIOD: Duration = Duration::from_millis(2500);
const DEFAULT_TOKEN_LIFETIME_SECS: u64 = 1800;
const TOKEN_REFRESH_MARGIN_SECS: u64 = 300;
const TOKEN_EXPIRY_SLACK_SECS: u64 = 60;

/// Everything needed to continue a Direct Line conversation. The token is scoped to this one
/// conversation and expires; the channel secret is never stored here.
#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone, PartialEq)]
pub struct DirectLineSession {
    pub conversation_id: String,
    pub token: String,
    /// Unix seconds.
    pub expires_at: u64,
    #[serde(default)]
    pub watermark: Option<String>,
    /// `https://{region.}directline.botframework.com/v3/directline`
    pub endpoint: String,
    pub user_id: String,
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

fn is_direct_line_host(host: &str) -> bool {
    match host.strip_suffix(DIRECT_LINE_HOST) {
        Some("") => true,
        Some(prefix) => prefix.strip_suffix('.').is_some_and(|region| {
            !region.is_empty()
                && region
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '-')
        }),
        None => false,
    }
}

/// The channel secret and conversation tokens are only ever sent to Bot Framework's clusters.
fn validate_direct_line_endpoint(url: &str) -> Result<String> {
    let parsed = Url::parse(url.trim())
        .map_err(|error| anyhow!("'{url}' is not a Direct Line URL: {error}"))?;
    let host = parsed.host_str().unwrap_or_default().to_ascii_lowercase();
    if parsed.scheme() != "https" || parsed.port().is_some() || !is_direct_line_host(&host) {
        return Err(anyhow!(
            "'{url}' is not a Direct Line endpoint (https://[region.]{DIRECT_LINE_HOST})"
        ));
    }
    Ok(format!("https://{host}{DIRECT_LINE_PATH}"))
}

fn validate_token_endpoint(url: &str) -> Result<Url> {
    let parsed = Url::parse(url.trim()).map_err(|error| {
        anyhow!("Token endpoint '{url}' is not a valid URL ({error}); copy it from Copilot Studio → Channels → Mobile app")
    })?;
    if parsed.scheme() != "https" || parsed.port().is_some() {
        return Err(anyhow!("Token endpoint '{url}' must be a plain https URL"));
    }
    validate_environment_host(&parsed.host_str().unwrap_or_default().to_ascii_lowercase())?;
    if !parsed.path().starts_with("/powervirtualagents/") {
        return Err(anyhow!(
            "Token endpoint path '{}' is not a Copilot Studio Direct Line token endpoint",
            parsed.path()
        ));
    }
    Ok(parsed)
}

fn region_endpoint(region: &str) -> Option<String> {
    let host = match region {
        "Global" => DIRECT_LINE_HOST.to_string(),
        "Europe" => format!("europe.{DIRECT_LINE_HOST}"),
        "India" => format!("india.{DIRECT_LINE_HOST}"),
        _ => return None,
    };
    Some(format!("https://{host}{DIRECT_LINE_PATH}"))
}

fn global_endpoint() -> String {
    format!("https://{DIRECT_LINE_HOST}{DIRECT_LINE_PATH}")
}

#[derive(Deserialize)]
struct TokenResponse {
    token: Option<String>,
    expires_in: Option<u64>,
    #[serde(rename = "conversationId")]
    conversation_id: Option<String>,
}

struct DirectLineClient {
    http: reqwest::Client,
}

impl DirectLineClient {
    fn new() -> Result<Self> {
        Ok(Self {
            http: reqwest::Client::builder()
                .timeout(REQUEST_TIMEOUT)
                .build()?,
        })
    }

    async fn send_json(&self, operation: &str, request: reqwest::RequestBuilder) -> Result<Value> {
        let response = request
            .send()
            .await
            .map_err(|error| anyhow!("{operation} failed: {error}"))?;
        if !response.status().is_success() {
            return Err(anyhow!(http_failure(operation, response).await));
        }
        let text = response.text().await?;
        if text.trim().is_empty() {
            return Ok(Value::Null);
        }
        json::from_str(&text).map_err(|error| anyhow!("{operation} returned invalid JSON: {error}"))
    }

    async fn token(
        &self,
        operation: &str,
        request: reqwest::RequestBuilder,
    ) -> Result<TokenResponse> {
        let body = self.send_json(operation, request).await?;
        json::from_value(body)
            .map_err(|error| anyhow!("{operation} returned an unexpected body: {error}"))
    }

    /// The agent's environment knows which regional Direct Line cluster it must use; the global
    /// endpoint answers 403 for agents with a data boundary.
    async fn regional_endpoint(&self, token_endpoint: &Url) -> Result<String> {
        let host = token_endpoint.host_str().unwrap_or_default();
        let url = format!(
            "https://{host}/powervirtualagents/regionalchannelsettings?api-version={D2E_API_VERSION}"
        );
        let settings = self
            .send_json(
                "Reading the agent's regional channel settings",
                self.http.get(url),
            )
            .await?;
        let direct_line = settings["channelUrlsById"]["directline"]
            .as_str()
            .ok_or_else(|| anyhow!("Regional channel settings contain no Direct Line URL"))?;
        validate_direct_line_endpoint(direct_line)
    }

    async fn endpoint_for(
        &self,
        context: &mut ExecutionContext,
        request: &DirectLineRequest,
    ) -> String {
        if let Some(endpoint) = region_endpoint(&request.region) {
            return endpoint;
        }
        let Some(token_endpoint) = &request.token_endpoint else {
            return global_endpoint();
        };
        match self.regional_endpoint(token_endpoint).await {
            Ok(endpoint) => endpoint,
            Err(error) => {
                context.log_message(
                    &format!("Falling back to the global Direct Line endpoint: {error:#}"),
                    LogLevel::Warn,
                );
                global_endpoint()
            }
        }
    }

    async fn issue_token(
        &self,
        endpoint: &str,
        request: &DirectLineRequest,
    ) -> Result<TokenResponse> {
        if let Some(token_endpoint) = &request.token_endpoint {
            return self
                .token(
                    "Fetching a Direct Line token from the agent's token endpoint",
                    self.http.get(token_endpoint.clone()),
                )
                .await;
        }
        let secret = request.secret.as_deref().ok_or_else(|| {
            anyhow!("Provide the agent's token endpoint (Channels → Mobile app) or its Web channel security secret")
        })?;
        self.token(
            "Generating a Direct Line token from the channel secret",
            self.http
                .post(format!("{endpoint}/tokens/generate"))
                .bearer_auth(secret)
                .json(&json!({ "user": { "id": request.user_id } })),
        )
        .await
    }

    async fn open(
        &self,
        context: &mut ExecutionContext,
        request: &DirectLineRequest,
    ) -> Result<DirectLineSession> {
        let endpoint = self.endpoint_for(context, request).await;
        let issued = self.issue_token(&endpoint, request).await?;
        let token = issued
            .token
            .ok_or_else(|| anyhow!("Direct Line issued no token"))?;
        let started = self
            .token(
                "Starting the Direct Line conversation",
                self.http
                    .post(format!("{endpoint}/conversations"))
                    .bearer_auth(&token),
            )
            .await?;
        let conversation_id = started
            .conversation_id
            .or(issued.conversation_id)
            .ok_or_else(|| anyhow!("Direct Line started a conversation without an id"))?;
        let lifetime = started
            .expires_in
            .or(issued.expires_in)
            .unwrap_or(DEFAULT_TOKEN_LIFETIME_SECS);
        Ok(DirectLineSession {
            conversation_id,
            token: started.token.unwrap_or(token),
            expires_at: unix_now() + lifetime,
            watermark: None,
            endpoint,
            user_id: request.user_id.clone(),
        })
    }

    async fn resume(
        &self,
        mut session: DirectLineSession,
        secret: Option<&str>,
    ) -> Result<DirectLineSession> {
        session.endpoint = validate_direct_line_endpoint(&session.endpoint)?;
        let now = unix_now();
        if session.expires_at <= now + TOKEN_EXPIRY_SLACK_SECS {
            return self.reconnect(session, secret).await;
        }
        if session.expires_at < now + TOKEN_REFRESH_MARGIN_SECS {
            let refreshed = self
                .token(
                    "Refreshing the Direct Line token",
                    self.http
                        .post(format!("{}/tokens/refresh", session.endpoint))
                        .bearer_auth(&session.token),
                )
                .await?;
            apply_token(&mut session, refreshed);
        }
        Ok(session)
    }

    /// An expired conversation token can only be replaced with the channel secret.
    async fn reconnect(
        &self,
        mut session: DirectLineSession,
        secret: Option<&str>,
    ) -> Result<DirectLineSession> {
        let secret = secret.ok_or_else(|| {
            anyhow!(
                "The Direct Line token for conversation {} expired; without the channel secret it cannot be resumed, so start a new conversation",
                session.conversation_id
            )
        })?;
        let mut url = conversation_url(&session, "")?;
        if let Some(watermark) = &session.watermark {
            url.query_pairs_mut().append_pair("watermark", watermark);
        }
        let reconnected = self
            .token(
                "Reconnecting to the Direct Line conversation",
                self.http.get(url).bearer_auth(secret),
            )
            .await?;
        if reconnected.token.is_none() {
            return Err(anyhow!(
                "Direct Line reconnected without issuing a conversation token"
            ));
        }
        apply_token(&mut session, reconnected);
        Ok(session)
    }

    async fn post_activity(&self, session: &DirectLineSession, activity: Value) -> Result<String> {
        let body = self
            .send_json(
                "Sending the activity over Direct Line",
                self.http
                    .post(conversation_url(session, "/activities")?)
                    .bearer_auth(&session.token)
                    .json(&activity),
            )
            .await?;
        Ok(body["id"].as_str().unwrap_or_default().to_string())
    }

    async fn activities(&self, session: &mut DirectLineSession) -> Result<Vec<Value>> {
        let mut url = conversation_url(session, "/activities")?;
        if let Some(watermark) = &session.watermark {
            url.query_pairs_mut().append_pair("watermark", watermark);
        }
        let mut body = self
            .send_json(
                "Reading the agent's replies over Direct Line",
                self.http.get(url).bearer_auth(&session.token),
            )
            .await?;
        if let Some(watermark) = body["watermark"].as_str() {
            session.watermark = Some(watermark.to_string());
        }
        Ok(match body.get_mut("activities").map(Value::take) {
            Some(Value::Array(activities)) => activities,
            _ => Vec::new(),
        })
    }
}

fn conversation_url(session: &DirectLineSession, suffix: &str) -> Result<Url> {
    let url = format!(
        "{}/conversations/{}{suffix}",
        session.endpoint,
        urlencoding::encode(&session.conversation_id)
    );
    Url::parse(&url).map_err(|error| anyhow!("Direct Line URL '{url}' is invalid: {error}"))
}

fn apply_token(session: &mut DirectLineSession, issued: TokenResponse) {
    if let Some(token) = issued.token {
        session.token = token;
        session.expires_at = unix_now() + issued.expires_in.unwrap_or(DEFAULT_TOKEN_LIFETIME_SECS);
    }
}

/// Tracks when a Direct Line turn is over.
struct TurnProgress {
    user_id: String,
    sent_id: String,
    replied: bool,
    any_reply_to: bool,
    last_bot_activity: Instant,
    last_bot_type: String,
}

impl TurnProgress {
    fn new(user_id: &str, sent_id: String) -> Self {
        Self {
            user_id: user_id.to_string(),
            sent_id,
            replied: false,
            any_reply_to: false,
            last_bot_activity: Instant::now(),
            last_bot_type: String::new(),
        }
    }

    fn observe(&mut self, activity: &Value) {
        if activity["from"]["id"].as_str() == Some(self.user_id.as_str()) {
            return;
        }
        if let Some(reply_to) = activity["replyToId"].as_str() {
            self.any_reply_to = true;
            self.replied |= !self.sent_id.is_empty() && reply_to == self.sent_id;
        }
        self.last_bot_activity = Instant::now();
        self.last_bot_type = activity["type"].as_str().unwrap_or_default().to_string();
    }

    /// Agents that never set `replyToId` fall back to "any bot message arrived".
    fn answered(&self, has_bot_message: bool) -> bool {
        self.replied || (!self.any_reply_to && has_bot_message)
    }

    fn settled(&self, expecting_input: bool) -> bool {
        self.last_bot_type != "typing"
            && (expecting_input || self.last_bot_activity.elapsed() >= QUIET_PERIOD)
    }
}

/// The node's inputs, read before the turn starts so the collector knows the user id to skip.
struct DirectLineRequest {
    session: Result<Option<DirectLineSession>>,
    user_id: String,
    prompt: String,
    secret: Option<String>,
    token_endpoint: Option<Url>,
    region: String,
    locale: String,
    run_greeting: bool,
    timeout: Duration,
}

async fn string_pin(context: &mut ExecutionContext, pin: &str) -> String {
    let value: String = context.evaluate_pin(pin).await.unwrap_or_default();
    value.trim().to_string()
}

impl DirectLineRequest {
    async fn read(context: &mut ExecutionContext) -> Result<Self> {
        let raw: Value = context.evaluate_pin("session").await.unwrap_or(Value::Null);
        let session = match raw {
            Value::Null => Ok(None),
            value => json::from_value::<DirectLineSession>(value)
                .map(|session| Some(session).filter(|s| !s.conversation_id.is_empty()))
                .map_err(|error| anyhow!("Session is not a Direct Line session: {error}")),
        };
        let user_id = match (&session, string_pin(context, "user_id").await) {
            (Ok(Some(session)), _) => session.user_id.clone(),
            (_, pin) if pin.is_empty() => DEFAULT_USER_ID.to_string(),
            (_, pin) => pin,
        };
        let token_endpoint = match string_pin(context, "token_endpoint").await.as_str() {
            "" => None,
            url => Some(validate_token_endpoint(url)?),
        };
        let timeout_seconds: i64 = context.evaluate_pin("timeout_seconds").await.unwrap_or(120);
        Ok(Self {
            session,
            user_id,
            prompt: context.evaluate_pin("prompt").await.unwrap_or_default(),
            secret: Some(string_pin(context, "secret").await).filter(|secret| !secret.is_empty()),
            token_endpoint,
            region: string_pin(context, "region").await,
            locale: evaluate_locale(context).await,
            run_greeting: context.evaluate_pin("run_greeting").await.unwrap_or(false),
            timeout: Duration::from_secs(timeout_seconds.clamp(5, 900) as u64),
        })
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct CopilotStudioDirectLineChatNode {}

impl CopilotStudioDirectLineChatNode {
    async fn start_or_resume(
        context: &mut ExecutionContext,
        client: &DirectLineClient,
        request: &mut DirectLineRequest,
    ) -> Result<DirectLineSession> {
        let existing = std::mem::replace(&mut request.session, Ok(None))?;
        if let Some(session) = existing {
            return client.resume(session, request.secret.as_deref()).await;
        }
        let session = client.open(context, request).await?;
        if request.run_greeting {
            let greeting = json!({
                "type": "event",
                "name": "startConversation",
                "from": { "id": request.user_id },
                "locale": request.locale,
            });
            client.post_activity(&session, greeting).await?;
        }
        Ok(session)
    }

    async fn await_reply(
        context: &mut ExecutionContext,
        turn: &mut Turn,
        client: &DirectLineClient,
        session: &mut DirectLineSession,
        mut progress: TurnProgress,
        timeout: Duration,
    ) -> Result<()> {
        let deadline = Instant::now() + timeout;
        loop {
            for activity in client.activities(session).await? {
                progress.observe(&activity);
                let updates = turn.collector.ingest(activity);
                turn.emitter.emit_all(context, updates).await?;
            }
            let collector = &turn.collector;
            let finished = collector.ended()
                || (progress.answered(collector.has_bot_message())
                    && progress.settled(collector.expecting_input()));
            if finished {
                return Ok(());
            }
            if Instant::now() >= deadline {
                return Self::timed_out(context, collector.has_bot_message(), timeout);
            }
            sleep(POLL_INTERVAL).await;
        }
    }

    fn timed_out(context: &mut ExecutionContext, has_reply: bool, timeout: Duration) -> Result<()> {
        if !has_reply {
            return Err(anyhow!(
                "The agent did not reply within {} seconds",
                timeout.as_secs()
            ));
        }
        context.log_message(
            &format!(
                "Direct Line turn still active after {} s; returning the replies received so far",
                timeout.as_secs()
            ),
            LogLevel::Warn,
        );
        Ok(())
    }

    async fn execute(
        context: &mut ExecutionContext,
        turn: &mut Turn,
        mut request: DirectLineRequest,
    ) -> Result<()> {
        if request.prompt.trim().is_empty() {
            return Err(anyhow!(
                "Prompt is empty; the agent needs a message to answer"
            ));
        }
        let client = DirectLineClient::new()?;
        let mut session = Self::start_or_resume(context, &client, &mut request).await?;
        turn.collector.set_conversation_id(&session.conversation_id);

        let message = json!({
            "type": "message",
            "from": { "id": request.user_id },
            "text": request.prompt,
            "textFormat": "plain",
            "locale": request.locale,
        });
        let sent_id = client.post_activity(&session, message).await?;
        let progress = TurnProgress::new(&request.user_id, sent_id);
        Self::await_reply(
            context,
            turn,
            &client,
            &mut session,
            progress,
            request.timeout,
        )
        .await?;

        context.set_pin_value("new_session", json!(session)).await?;
        Ok(())
    }
}

fn add_connection_pins(node: &mut Node) {
    node.add_input_pin(
        "token_endpoint",
        "Token Endpoint",
        "The agent's token endpoint from Channels → Mobile app (when secured access is off)",
        VariableType::String,
    )
    .set_default_value(Some(json!("")));
    node.add_input_pin(
        "secret",
        "Channel Secret",
        "Web channel security secret (when secured access is on); never stored in the session",
        VariableType::String,
    )
    .set_options(PinOptions::new().set_sensitive(true).build())
    .set_default_value(Some(json!("")));
    node.add_input_pin(
        "region",
        "Region",
        "Direct Line cluster; Auto asks the agent's environment (needs the token endpoint)",
        VariableType::String,
    )
    .set_options(
        PinOptions::new()
            .set_valid_values(REGIONS.iter().map(|region| region.to_string()).collect())
            .build(),
    )
    .set_default_value(Some(json!("Auto")));
    node.add_input_pin(
        "timeout_seconds",
        "Timeout (s)",
        "How long to wait for the agent's reply",
        VariableType::Integer,
    )
    .set_options(PinOptions::new().set_range((5.0, 900.0)).build())
    .set_default_value(Some(json!(120)));
}

fn add_conversation_pins(node: &mut Node) {
    node.add_input_pin(
        "session",
        "Session",
        "Session returned by a previous call; leave unconnected to start a new conversation",
        VariableType::Struct,
    )
    .set_schema::<DirectLineSession>()
    .set_default_value(Some(Value::Null));
    node.add_input_pin(
        "prompt",
        "Prompt",
        "The user's message",
        VariableType::String,
    );
    node.add_input_pin(
        "user_id",
        "User ID",
        "Stable id for the user in this conversation",
        VariableType::String,
    )
    .set_default_value(Some(json!(DEFAULT_USER_ID)));
    add_locale_pin(node);
    node.add_input_pin(
        "run_greeting",
        "Run Greeting",
        "On a new conversation, run the agent's Conversation Start topic first",
        VariableType::Boolean,
    )
    .set_default_value(Some(json!(false)));
}

#[async_trait]
impl NodeLogic for CopilotStudioDirectLineChatNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_microsoft_copilot_studio_direct_line_chat",
            "Copilot Studio Direct Line Chat",
            "Chat with a Copilot Studio agent over Direct Line, for agents without Microsoft authentication (anonymous or manual auth). Uses the token endpoint (Channels → Mobile app) or the Web channel security secret; no Microsoft sign-in. Pass the returned session back in to continue the conversation.",
            CATEGORY,
        );
        node.set_flowscript_name(FLOWSCRIPT_NAMESPACE, "directLineChat");
        node.add_icon("/flow/icons/copilot.svg");

        node.add_input_pin("exec_in", "Input", "Trigger", VariableType::Execution);
        add_connection_pins(&mut node);
        add_conversation_pins(&mut node);

        add_turn_output_pins(&mut node);
        node.add_output_pin(
            "new_session",
            "Session",
            "Pass this into the next call to continue the conversation",
            VariableType::Struct,
        )
        .set_schema::<DirectLineSession>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.set_long_running(true);
        node.set_scores(scores(5, 4, 3));
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let request = DirectLineRequest::read(context).await;
        let user_id = request
            .as_ref()
            .map(|request| request.user_id.clone())
            .unwrap_or_else(|_| DEFAULT_USER_ID.to_string());
        let mut turn = Turn::begin(context, Some(user_id)).await?;
        let outcome = match request {
            Ok(request) => Self::execute(context, &mut turn, request).await,
            Err(error) => Err(error),
        };
        turn.complete(context, outcome).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn direct_line_endpoints_are_limited_to_botframework_clusters() {
        assert_eq!(
            validate_direct_line_endpoint("https://europe.directline.botframework.com/").unwrap(),
            "https://europe.directline.botframework.com/v3/directline"
        );
        assert_eq!(
            validate_direct_line_endpoint("https://directline.botframework.com/v3/directline")
                .unwrap(),
            "https://directline.botframework.com/v3/directline"
        );
        for url in [
            "http://directline.botframework.com",
            "https://evildirectline.botframework.com",
            "https://directline.botframework.com.evil.com",
            "https://directline.botframework.com:8443",
            "https://a.b/directline.botframework.com",
        ] {
            assert!(
                validate_direct_line_endpoint(url).is_err(),
                "{url} must be rejected"
            );
        }
    }

    #[test]
    fn token_endpoints_must_be_copilot_studio_environment_urls() {
        let valid = "https://aaaa.ee.environment.api.powerplatform.com/powervirtualagents/botsbyschema/cr3e1_x/directline/token?api-version=2022-03-01-preview";
        assert!(validate_token_endpoint(valid).is_ok());
        assert!(validate_token_endpoint("https://example.com/powervirtualagents/x").is_err());
        assert!(
            validate_token_endpoint(
                "https://aaaa.ee.environment.api.powerplatform.com/copilotstudio/x"
            )
            .is_err()
        );
    }

    #[test]
    fn a_turn_waits_for_the_reply_to_our_message_not_the_greeting() {
        let mut progress = TurnProgress::new("u1", "conv|0002".to_string());
        progress.observe(
            &json!({ "type": "message", "from": { "id": "bot" }, "replyToId": "conv|0001" }),
        );
        assert!(!progress.answered(true));
        progress.observe(
            &json!({ "type": "typing", "from": { "id": "bot" }, "replyToId": "conv|0002" }),
        );
        assert!(progress.answered(true));
        assert!(!progress.settled(true), "still typing");
        progress.observe(
            &json!({ "type": "message", "from": { "id": "bot" }, "replyToId": "conv|0002" }),
        );
        assert!(progress.settled(true));
    }

    #[test]
    fn agents_without_reply_ids_are_answered_by_any_bot_message() {
        let mut progress = TurnProgress::new("u1", "conv|0002".to_string());
        progress.observe(&json!({ "type": "message", "from": { "id": "u1" }, "replyToId": "x" }));
        assert!(!progress.any_reply_to, "user echoes are ignored");
        progress.observe(&json!({ "type": "message", "from": { "id": "bot" } }));
        assert!(progress.answered(true));
    }

    #[test]
    fn sessions_round_trip_and_regions_resolve() {
        let session = DirectLineSession {
            conversation_id: "c1".to_string(),
            token: "t".to_string(),
            expires_at: 1,
            watermark: Some("3".to_string()),
            endpoint: "https://directline.botframework.com/v3/directline".to_string(),
            user_id: "u1".to_string(),
        };
        let parsed: DirectLineSession = json::from_value(json!(session)).unwrap();
        assert_eq!(parsed, session);
        assert_eq!(
            conversation_url(&session, "/activities").unwrap().as_str(),
            "https://directline.botframework.com/v3/directline/conversations/c1/activities"
        );
        assert_eq!(
            region_endpoint("India").as_deref(),
            Some("https://india.directline.botframework.com/v3/directline")
        );
        assert!(region_endpoint("Auto").is_none());
    }
}
