#![allow(dead_code)]

use axum::{
    Json, Router,
    body::Body,
    extract::{Path, State},
    http::{HeaderMap, Request, StatusCode, header},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use flow_like::bit::{Bit, BitModelClassification, BitTypes, LLMParameters};
use flow_like::flow_like_model_provider::provider::ModelProvider;
use flow_like_bot_host::{
    account::{AccountConfig, AccountHost},
    host::{TurnEvents, TurnInfo},
};
use flow_like_device_protocol::*;
use flow_like_standalone::{
    bot_host::{
        env::Machines,
        hub::{DeviceApi, DeviceTurnHub},
        runner::DeviceRunner,
    },
    enrollment::{self, DeviceSession},
    supervisor,
};
use flow_like_types::{
    bot::{BotCapability, BotModelPolicy},
    bot_turn::{BotTurn, TurnAudience, TurnHostKind, TurnStatus, reply_message_id, turn_id},
    data_classification::DataSensitivity,
    device_bot::*,
};
use serde_json::{Value, json};
use std::{
    collections::{HashSet, VecDeque},
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::sync::Mutex;
use uuid::Uuid;

pub const BOT_ID: &str = "bot-1";
pub const CONVERSATION_ID: &str = "conv-1";
pub const TRIGGER_ID: &str = "msg-1";
pub const OWNER_ID: &str = "owner";
pub const MODEL_ID: &str = "hosted-test-model";
pub const BOT_TOKEN: &str = "bot-token-1";
pub const EXECUTOR_TOKEN: &str = "executor-token-1";
pub const OPERATION_ID: &str = "op-1";

#[derive(Clone, Debug)]
pub struct Rec {
    pub method: String,
    pub path: String,
    pub query: String,
    pub authorization: String,
    pub body: Value,
}

#[derive(Clone, Debug)]
pub enum Step {
    Text(String),
    Tool(&'static str, Value),
    Status(u16),
    Slow(Duration, Box<Step>),
    EchoLastTool(&'static str),
}

#[derive(Clone, Debug)]
pub enum Claim {
    Grant,
    Refuse(u16, &'static str),
}

#[derive(Clone, Debug, PartialEq)]
pub enum TurnState {
    Queued,
    Running,
    Replied,
    Parked,
    Released,
    Lost,
    Failed,
}

pub struct Scenario {
    pub claim: Claim,
    pub lease_lost: bool,
    pub grants: Vec<(String, String)>,
    pub model: VecDeque<Step>,
    pub capabilities: Vec<BotCapability>,
    pub prompt: String,
}

impl Default for Scenario {
    fn default() -> Self {
        Self {
            claim: Claim::Grant,
            lease_lost: false,
            grants: Vec::new(),
            model: VecDeque::new(),
            capabilities: Vec::new(),
            prompt: "Please say hello".into(),
        }
    }
}

pub struct Hub {
    pub base: String,
    pub scenario: Scenario,
    pub state: TurnState,
    pub requests: Vec<Rec>,
    pub model_requests: Vec<Value>,
    pub model_in_flight: usize,
    pub model_dropped: usize,
    /// What the device saved last for the turn; a fresh turn has none.
    pub checkpoint: Option<Value>,
    manifest: Option<OnboardingManifest>,
    challenge: Option<EnrollmentChallenge>,
    receipt: Option<DeviceReceipt>,
    token_requests: usize,
    replay: HashSet<String>,
    pub dpop_verified: usize,
}

pub type Shared = Arc<Mutex<Hub>>;

impl Hub {
    pub fn paths(&self, prefix: &str) -> Vec<String> {
        self.requests
            .iter()
            .filter(|rec| rec.path.starts_with(prefix))
            .map(|rec| format!("{} {}", rec.method, rec.path))
            .collect()
    }

    pub fn calls(&self, suffix: &str) -> Vec<&Rec> {
        self.requests
            .iter()
            .filter(|rec| rec.path.ends_with(suffix))
            .collect()
    }

    pub fn device_id(&self) -> String {
        self.manifest.as_ref().unwrap().device_id.clone()
    }
}

fn error(status: u16, code: &str, message: &str) -> Response {
    (
        StatusCode::from_u16(status).unwrap(),
        Json(json!({"error": {"code": code, "message": message}})),
    )
        .into_response()
}

fn now() -> i64 {
    enrollment::unix_time().unwrap()
}

fn rfc3339(offset: i64) -> String {
    chrono::DateTime::from_timestamp(now() + offset, 0)
        .unwrap()
        .to_rfc3339()
}

pub fn turn_of(state: &Hub, status: TurnStatus) -> BotTurn {
    BotTurn {
        id: turn_id(BOT_ID, TRIGGER_ID),
        bot_id: BOT_ID.into(),
        conversation_id: CONVERSATION_ID.into(),
        trigger_message_id: TRIGGER_ID.into(),
        host_kind: TurnHostKind::Device,
        status,
        attempts: if status == TurnStatus::Queued { 0 } else { 1 },
        host_id: (status == TurnStatus::Running).then(|| format!("device:{}", state.device_id())),
        lease_until: (status == TurnStatus::Running).then(|| rfc3339(60)),
        reply_message_id: None,
        error_code: None,
        hops: 0,
        owner_id: OWNER_ID.into(),
        requester_id: Some(OWNER_ID.into()),
        requester_name: Some("Owner".into()),
        audience: TurnAudience::Owner,
        created_at: rfc3339(-10),
        updated_at: rfc3339(0),
        batched: Vec::new(),
    }
}

pub fn hosted_bit() -> Value {
    let bit = Bit {
        id: MODEL_ID.into(),
        bit_type: BitTypes::Llm,
        parameters: serde_json::to_value(LLMParameters {
            context_length: 20_000,
            model_classification: BitModelClassification::default(),
            data_classification: None,
            provider: ModelProvider {
                api_surface: None,
                provider_name: "hosted:openrouter".into(),
                model_id: Some(MODEL_ID.into()),
                version: None,
                params: None,
            },
        })
        .unwrap(),
        ..Bit::default()
    };
    serde_json::to_value(bit).unwrap()
}

fn grant_of(state: &Hub) -> DeviceTurnGrant {
    DeviceTurnGrant {
        delivery_only: false,
        turn: turn_of(state, TurnStatus::Running),
        host_id: format!("device:{}", state.device_id()),
        conversation: DeviceConversation {
            id: CONVERSATION_ID.into(),
            kind: "direct".into(),
            title: None,
        },
        bot: DeviceBotSpec {
            bot_id: BOT_ID.into(),
            system: None,
            name: "Deviceling".into(),
            description: None,
            instructions: None,
            capabilities: state.scenario.capabilities.clone(),
            classification: DataSensitivity::default(),
            selected_app_ids: Default::default(),
            profile_app_ids: Default::default(),
            workspace_app_id: Some("workspace-app".into()),
            embedding_model: None,
            stt_model: None,
            tts_model: None,
            image_model: None,
            vision_model: None,
            subtask_model: None,
            skills: Vec::new(),
            connectors: Vec::new(),
            model_policy: BotModelPolicy::default(),
        },
        requester: Some(DeviceRequester {
            requester_id: OWNER_ID.into(),
            requester_name: Some("Owner".into()),
            owner_id: OWNER_ID.into(),
            owner_name: Some("Owner".into()),
            audience: TurnAudience::Owner,
            room: None,
        }),
        models: DeviceModels {
            decision: None,
            model_id: MODEL_ID.into(),
            completion: hosted_bit(),
            fallbacks: Vec::new(),
            embedding: None,
            embedding_model_id: None,
            image: None,
        },
        bot_token: DeviceToken {
            token: BOT_TOKEN.into(),
            expires_at: now() + 600,
        },
        executor_token: Some(DeviceToken {
            token: EXECUTOR_TOKEN.into(),
            expires_at: now() + 600,
        }),
        usage: Some(DeviceUsageGrant {
            operation_id: OPERATION_ID.into(),
            deadline_ms: (now() + 600) * 1000,
        }),
        deadline: now() + 600,
    }
}

async fn record(State(hub): State<Shared>, request: Request<Body>, next: Next) -> Response {
    let (parts, body) = request.into_parts();
    let bytes = axum::body::to_bytes(body, 8 * 1024 * 1024).await.unwrap();
    hub.lock().await.requests.push(Rec {
        method: parts.method.to_string(),
        path: parts.uri.path().to_owned(),
        query: parts.uri.query().unwrap_or_default().to_owned(),
        authorization: parts
            .headers
            .get("authorization")
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default()
            .to_owned(),
        body: serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    });
    next.run(Request::from_parts(parts, Body::from(bytes)))
        .await
}

async fn create(
    State(hub): State<Shared>,
    headers: HeaderMap,
    Json(body): Json<CreateEnrollmentRequest>,
) -> Json<CreateEnrollmentResponse> {
    assert_eq!(
        headers["authorization"],
        "Bearer enrollment-test-user-token"
    );
    let mut hub = hub.lock().await;
    let issued = now();
    let manifest = OnboardingManifest {
        version: 1,
        enrollment_id: Uuid::new_v4().to_string(),
        device_id: Uuid::new_v4().to_string(),
        owner_id: OWNER_ID.into(),
        name: body.name,
        api_base_url: body.api_base_url,
        bootstrap_key: body.bootstrap_key,
        controller_key: body.controller_key,
        owner_invitation_key: body.owner_invitation_key,
        issued_at: issued,
        expires_at: issued + 86400,
    };
    validate_manifest(&manifest).unwrap();
    hub.manifest = Some(manifest.clone());
    Json(CreateEnrollmentResponse {
        enrollment_token: "one-use-bootstrap-token".into(),
        manifest,
    })
}

async fn challenge(
    State(hub): State<Shared>,
    Json(_): Json<ChallengeRequest>,
) -> Json<EnrollmentChallenge> {
    let challenge = EnrollmentChallenge {
        challenge_id: Uuid::new_v4().to_string(),
        nonce: "VhF2QDp2QgpCIh2hsyIeDE_lWL4nGCPEyzutUfRkoes".into(),
        expires_at: now() + 60,
    };
    hub.lock().await.challenge = Some(challenge.clone());
    Json(challenge)
}

async fn redeem(
    State(hub): State<Shared>,
    Path(id): Path<String>,
    Json(body): Json<RedeemEnrollmentRequest>,
) -> StatusCode {
    let mut hub = hub.lock().await;
    let at = now();
    let manifest = hub.manifest.clone().unwrap();
    let binding = verify_binding(&body.binding_jws, &manifest.bootstrap_key, at).unwrap();
    let challenge = hub.challenge.clone().unwrap();
    let endpoint = endpoint_url(&hub.base, &format!("/devices/enrollments/{id}/redeem")).unwrap();
    verify_enrollment_proof(
        &body.proof_jws,
        &binding.identity.auth_key,
        &EnrollmentProofContext {
            device_id: &manifest.device_id,
            endpoint: &endpoint,
            challenge_id: &challenge.challenge_id,
            challenge_nonce: &challenge.nonce,
            binding_digest: &compact_digest(&body.binding_jws),
            manifest_digest: &compact_digest(&body.manifest_jws),
            now: at,
        },
    )
    .unwrap();
    hub.receipt = Some(DeviceReceipt {
        enrollment_id: id,
        device_id: manifest.device_id.clone(),
        owner_id: manifest.owner_id.clone(),
        name: manifest.name.clone(),
        identity: binding.identity,
        manifest_jws: body.manifest_jws,
        binding_jws: body.binding_jws,
        registered_at: at,
        auth_epoch: 1,
    });
    StatusCode::INTERNAL_SERVER_ERROR
}

async fn receipt(
    State(hub): State<Shared>,
    Path(id): Path<String>,
    Json(body): Json<ReceiptRequest>,
) -> std::result::Result<Json<DeviceReceipt>, StatusCode> {
    let hub = hub.lock().await;
    let receipt = hub.receipt.as_ref().ok_or(StatusCode::NOT_FOUND)?;
    let endpoint = endpoint_url(&hub.base, &format!("/devices/{id}/receipt")).unwrap();
    verify_client_assertion(
        &body.client_assertion,
        &receipt.identity.auth_key,
        &id,
        &endpoint,
        now(),
    )
    .unwrap();
    Ok(Json(receipt.clone()))
}

async fn token(
    State(hub): State<Shared>,
    Json(body): Json<DeviceTokenRequest>,
) -> Json<DeviceTokenResponse> {
    let mut hub = hub.lock().await;
    let receipt = hub.receipt.clone().unwrap();
    let endpoint = endpoint_url(&hub.base, "/devices/token").unwrap();
    let assertion = verify_client_assertion(
        &body.client_assertion,
        &receipt.identity.auth_key,
        &receipt.device_id,
        &endpoint,
        now(),
    )
    .unwrap();
    assert!(hub.replay.insert(assertion.jti));
    hub.token_requests += 1;
    Json(DeviceTokenResponse {
        access_token: format!("test-session-{}", hub.token_requests),
        token_type: "DPoP".into(),
        expires_in: 600,
        expires_at: now() + 600,
    })
}

fn device_session(
    hub: &mut Hub,
    id: &str,
    headers: &HeaderMap,
    path: &str,
) -> std::result::Result<(), Response> {
    device_request(hub, id, headers, "POST", path)
}

fn device_request(
    hub: &mut Hub,
    id: &str,
    headers: &HeaderMap,
    method: &str,
    path: &str,
) -> std::result::Result<(), Response> {
    let receipt = hub.receipt.clone().unwrap();
    if receipt.device_id != id {
        return Err(error(404, "DEVICE_NOT_FOUND", "unknown device"));
    }
    let token = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("DPoP "))
        .ok_or_else(|| error(401, "DEVICE_PROOF_INVALID", "no DPoP authorization"))?;
    let proof = headers
        .get("dpop")
        .and_then(|v| v.to_str().ok())
        .ok_or_else(|| error(401, "DEVICE_PROOF_INVALID", "no DPoP proof"))?;
    let endpoint = endpoint_url(&hub.base, path).unwrap();
    let claims = verify_dpop(
        proof,
        &receipt.identity.auth_key,
        &DpopContext {
            method,
            url: &endpoint,
            access_token: Some(token),
            nonce: None,
            key_thumbprint: &receipt.identity.auth_key.thumbprint().unwrap(),
            now: now(),
        },
    )
    .map_err(|e| error(401, "DEVICE_PROOF_INVALID", &format!("{e:?}")))?;
    if !hub.replay.insert(claims.jti) {
        return Err(error(401, "DEVICE_PROOF_INVALID", "replayed proof"));
    }
    hub.dpop_verified += 1;
    Ok(())
}

/// The proof of a checkpoint call names the path without its query, as the hub checks it.
async fn checkpoint_load(
    State(hub): State<Shared>,
    Path((id, turn)): Path<(String, String)>,
    headers: HeaderMap,
) -> Response {
    let mut hub = hub.lock().await;
    let path = format!("/devices/{id}/bots/turns/{turn}/checkpoint");
    if let Err(response) = device_request(&mut hub, &id, &headers, "GET", &path) {
        return response;
    }
    Json(json!({"checkpoint": hub.checkpoint, "monitor_result": null})).into_response()
}

async fn checkpoint_save(
    State(hub): State<Shared>,
    Path((id, turn)): Path<(String, String)>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    let mut hub = hub.lock().await;
    let path = format!("/devices/{id}/bots/turns/{turn}/checkpoint");
    if let Err(response) = device_request(&mut hub, &id, &headers, "PUT", &path) {
        return response;
    }
    hub.checkpoint = Some(body["checkpoint"].clone());
    Json(json!({"saved": true})).into_response()
}

async fn poll(
    State(hub): State<Shared>,
    Path(id): Path<String>,
    headers: HeaderMap,
    Json(_): Json<DevicePollRequest>,
) -> Response {
    let mut hub = hub.lock().await;
    if let Err(response) =
        device_session(&mut hub, &id, &headers, &format!("/devices/{id}/bots/poll"))
    {
        return response;
    }
    let queued = hub.state == TurnState::Queued;
    Json(DevicePollResponse {
        bots: vec![DeviceBotInfo {
            id: BOT_ID.into(),
            name: Some("Deviceling".into()),
            capabilities: hub.scenario.capabilities.clone(),
            updated_at: rfc3339(0),
        }],
        turns: if queued {
            vec![turn_of(&hub, TurnStatus::Queued)]
        } else {
            Vec::new()
        },
        next_poll_ms: 500,
    })
    .into_response()
}

async fn claim(
    State(hub): State<Shared>,
    Path((id, turn)): Path<(String, String)>,
    headers: HeaderMap,
    Json(_): Json<DeviceClaimRequest>,
) -> Response {
    let mut hub = hub.lock().await;
    let path = format!("/devices/{id}/bots/turns/{turn}/claim");
    if let Err(response) = device_session(&mut hub, &id, &headers, &path) {
        return response;
    }
    if hub.state != TurnState::Queued {
        return error(409, "BOT_TURN_NOT_QUEUED", "the turn is not queued");
    }
    match hub.scenario.claim.clone() {
        Claim::Refuse(status, code) => {
            hub.state = TurnState::Failed;
            error(status, code, "claim refused by the scenario")
        }
        Claim::Grant => {
            hub.state = TurnState::Running;
            Json(grant_of(&hub)).into_response()
        }
    }
}

async fn heartbeat(
    State(hub): State<Shared>,
    Path((id, turn)): Path<(String, String)>,
    headers: HeaderMap,
    Json(_): Json<DeviceHeartbeatRequest>,
) -> Response {
    let mut hub = hub.lock().await;
    let path = format!("/devices/{id}/bots/turns/{turn}/heartbeat");
    if let Err(response) = device_session(&mut hub, &id, &headers, &path) {
        return response;
    }
    if hub.scenario.lease_lost {
        hub.state = TurnState::Lost;
        return error(409, "BOT_TURN_LEASE_LOST", "the lease is gone");
    }
    Json(DeviceHeartbeatResponse {
        turn: turn_of(&hub, TurnStatus::Running),
        bot_token: None,
        executor_token: None,
    })
    .into_response()
}

async fn reply(
    State(hub): State<Shared>,
    Path((id, turn)): Path<(String, String)>,
    headers: HeaderMap,
    Json(body): Json<DeviceReplyRequest>,
) -> Response {
    let mut hub = hub.lock().await;
    let path = format!("/devices/{id}/bots/turns/{turn}/reply");
    if let Err(response) = device_session(&mut hub, &id, &headers, &path) {
        return response;
    }
    if body.message_id != reply_message_id(BOT_ID, TRIGGER_ID) {
        return error(400, "BOT_TURN_REPLY_REFUSED", "wrong message id");
    }
    hub.state = TurnState::Replied;
    let mut done = turn_of(&hub, TurnStatus::Done);
    done.reply_message_id = Some(body.message_id.clone());
    Json(DeviceReplyResponse {
        turn: done,
        message_id: body.message_id,
        posted: true,
    })
    .into_response()
}

async fn ended(
    hub: Shared,
    id: String,
    turn: String,
    action: &str,
    headers: HeaderMap,
    body: Value,
) -> Response {
    let mut hub = hub.lock().await;
    let path = format!("/devices/{id}/bots/turns/{turn}/{action}");
    if let Err(response) = device_session(&mut hub, &id, &headers, &path) {
        return response;
    }
    let (state, status) = match action {
        "park" => {
            serde_json::from_value::<DeviceParkRequest>(body).unwrap();
            (TurnState::Parked, TurnStatus::Waiting)
        }
        "release" => {
            serde_json::from_value::<DeviceReleaseRequest>(body).unwrap();
            (TurnState::Released, TurnStatus::Failed)
        }
        _ => {
            serde_json::from_value::<DeviceSilentRequest>(body).unwrap();
            (TurnState::Replied, TurnStatus::Done)
        }
    };
    hub.state = state;
    Json(DeviceTurnState {
        turn: turn_of(&hub, status),
    })
    .into_response()
}

async fn park(
    State(hub): State<Shared>,
    Path((id, turn)): Path<(String, String)>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    ended(hub, id, turn, "park", headers, body).await
}

async fn release(
    State(hub): State<Shared>,
    Path((id, turn)): Path<(String, String)>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    ended(hub, id, turn, "release", headers, body).await
}

async fn silent(
    State(hub): State<Shared>,
    Path((id, turn)): Path<(String, String)>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    ended(hub, id, turn, "silent", headers, body).await
}

async fn forget(
    State(hub): State<Shared>,
    Path(id): Path<String>,
    headers: HeaderMap,
    Json(_): Json<DeviceForgetRequest>,
) -> Response {
    let mut hub = hub.lock().await;
    if let Err(response) = device_session(
        &mut hub,
        &id,
        &headers,
        &format!("/devices/{id}/bots/hosts/forget"),
    ) {
        return response;
    }
    Json(DeviceForgetResponse { removed: 1 }).into_response()
}

fn bearer_ok(headers: &HeaderMap, token: &str) -> bool {
    headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v == format!("Bearer {token}"))
}

async fn entries(
    State(hub): State<Shared>,
    headers: HeaderMap,
    Path(conversation): Path<String>,
) -> Response {
    if !bearer_ok(&headers, BOT_TOKEN) {
        return error(401, "UNAUTHORIZED", "bot token expected");
    }
    let hub = hub.lock().await;
    let sent = chrono::DateTime::from_timestamp(now() - 5, 0)
        .unwrap()
        .to_rfc3339();
    Json(json!({
        "entries": [{
            "id": "entry-1",
            "message_id": TRIGGER_ID,
            "conversation_id": conversation,
            "sender_id": OWNER_ID,
            "kind": "message",
            "envelope": {"v": 1, "enc": "none", "body": {"text": hub.scenario.prompt}},
            "created_at": sent,
            "seq": 1,
            "cursor": "c1"
        }]
    }))
    .into_response()
}

async fn conversation(headers: HeaderMap, Path(conversation): Path<String>) -> Response {
    if !bearer_ok(&headers, BOT_TOKEN) {
        return error(401, "UNAUTHORIZED", "bot token expected");
    }
    Json(json!({
        "id": conversation,
        "kind": "direct",
        "members": [
            {"principal_id": BOT_ID, "kind": "bot"},
            {"principal_id": OWNER_ID, "kind": "user"}
        ]
    }))
    .into_response()
}

async fn tool_grants(State(hub): State<Shared>, headers: HeaderMap) -> Response {
    if !bearer_ok(&headers, BOT_TOKEN) {
        return error(401, "UNAUTHORIZED", "bot token expected");
    }
    let hub = hub.lock().await;
    let grants: Vec<Value> = hub
        .scenario
        .grants
        .iter()
        .map(|(tool, scope)| json!({"tool": tool, "scope": scope}))
        .collect();
    Json(json!({"grants": grants})).into_response()
}

async fn put_tool_grant(headers: HeaderMap) -> Response {
    if !bearer_ok(&headers, BOT_TOKEN) {
        return error(401, "UNAUTHORIZED", "bot token expected");
    }
    Json(json!({})).into_response()
}

async fn delete_tool_grant(headers: HeaderMap) -> Response {
    if !bearer_ok(&headers, BOT_TOKEN) {
        return error(401, "UNAUTHORIZED", "bot token expected");
    }
    Json(json!({"deleted": false})).into_response()
}

async fn secret(headers: HeaderMap) -> Response {
    if !bearer_ok(&headers, BOT_TOKEN) {
        return error(401, "UNAUTHORIZED", "bot token expected");
    }
    error(404, "NOT_FOUND", "no such secret")
}

async fn presign() -> Response {
    error(403, "FORBIDDEN", "this mock hub holds no object store")
}

struct InFlight(Shared);

impl Drop for InFlight {
    fn drop(&mut self) {
        let hub = self.0.clone();
        tokio::spawn(async move {
            let mut hub = hub.lock().await;
            hub.model_in_flight -= 1;
            hub.model_dropped += 1;
        });
    }
}

fn last_tool_result(request: &Value) -> String {
    request["messages"]
        .as_array()
        .into_iter()
        .flatten()
        .rev()
        .find(|message| message["role"] == "tool")
        .map(|message| message["content"].to_string())
        .unwrap_or_default()
}

fn completion(request: &Value, step: &Step) -> Response {
    let (content, tool) = match step {
        Step::Text(text) => (Some(text.clone()), None),
        Step::EchoLastTool(prefix) => {
            (Some(format!("{prefix}{}", last_tool_result(request))), None)
        }
        Step::Tool(name, args) => (None, Some((*name, args.to_string()))),
        Step::Status(_) | Step::Slow(..) => unreachable!("resolved before"),
    };
    let finish = if tool.is_some() { "tool_calls" } else { "stop" };
    let calls = tool.map(|(name, arguments)| {
        json!([{"index": 0, "id": "call_1", "type": "function", "function": {"name": name, "arguments": arguments}}])
    });
    let usage = json!({"prompt_tokens": 10, "completion_tokens": 5, "total_tokens": 15});
    if request["stream"] == true {
        let mut delta = json!({"role": "assistant", "content": content});
        if let Some(calls) = &calls {
            delta["tool_calls"] = calls.clone();
        }
        let chunk = |delta: Value, reason: Value, usage: Value| {
            let mut value = json!({
                "id": "chatcmpl-1", "object": "chat.completion.chunk", "created": 1,
                "model": MODEL_ID,
                "choices": [{"index": 0, "delta": delta, "finish_reason": reason}]
            });
            if !usage.is_null() {
                value["usage"] = usage;
            }
            format!("data: {value}\n\n")
        };
        let body = format!(
            "{}{}data: [DONE]\n\n",
            chunk(delta, Value::Null, Value::Null),
            chunk(json!({}), json!(finish), usage)
        );
        return ([(header::CONTENT_TYPE, "text/event-stream")], body).into_response();
    }
    let mut message = json!({"role": "assistant", "content": content});
    if let Some(calls) = calls {
        message["tool_calls"] = calls;
    }
    Json(json!({
        "id": "chatcmpl-1", "object": "chat.completion", "created": 1, "model": MODEL_ID,
        "choices": [{"index": 0, "message": message, "finish_reason": finish}],
        "usage": usage
    }))
    .into_response()
}

async fn chat_completions(
    State(hub): State<Shared>,
    headers: HeaderMap,
    Json(request): Json<Value>,
) -> Response {
    if !bearer_ok(&headers, EXECUTOR_TOKEN) {
        return error(401, "UNAUTHORIZED", "executor token expected");
    }
    let mut step = {
        let mut state = hub.lock().await;
        state.model_requests.push(request.clone());
        state.scenario.model.pop_front()
    };
    if let Some(Step::Slow(delay, next)) = step {
        hub.lock().await.model_in_flight += 1;
        let guard = InFlight(hub.clone());
        tokio::time::sleep(delay).await;
        std::mem::forget(guard);
        hub.lock().await.model_in_flight -= 1;
        step = Some(*next);
    }
    match step {
        None => error(
            500,
            "EMPTY_SCRIPT",
            "the scenario scripted no further model answer",
        ),
        Some(Step::Status(status)) => error(status, "SCRIPTED_FAILURE", "scripted model failure"),
        Some(step) => completion(&request, &step),
    }
}

pub fn router(hub: Shared) -> Router {
    let device = Router::new()
        .route("/devices/enrollments", post(create))
        .route("/devices/enrollments/{id}/challenge", post(challenge))
        .route("/devices/enrollments/{id}/redeem", post(redeem))
        .route("/devices/{id}/receipt", post(receipt))
        .route("/devices/token", post(token))
        .route("/devices/{id}/bots/poll", post(poll))
        .route("/devices/{id}/bots/turns/{turn}/claim", post(claim))
        .route("/devices/{id}/bots/turns/{turn}/heartbeat", post(heartbeat))
        .route(
            "/devices/{id}/bots/turns/{turn}/checkpoint",
            get(checkpoint_load).put(checkpoint_save),
        )
        .route("/devices/{id}/bots/turns/{turn}/reply", post(reply))
        .route("/devices/{id}/bots/turns/{turn}/silent", post(silent))
        .route("/devices/{id}/bots/turns/{turn}/park", post(park))
        .route("/devices/{id}/bots/turns/{turn}/release", post(release))
        .route("/devices/{id}/bots/hosts/forget", post(forget))
        .route("/inbox/conversations/{c}/entries", get(entries))
        .route("/inbox/conversations/{c}", get(conversation))
        .route(
            "/bots/{bot}/tool-grants",
            get(tool_grants)
                .put(put_tool_grant)
                .delete(delete_tool_grant),
        )
        .route("/bots/turns/{turn}/secrets/{name}", post(secret))
        .route("/apps/{app}/invoke/presign", get(presign))
        .route("/chat/completions", post(chat_completions));
    Router::new()
        .nest("/api/v1", device)
        .layer(middleware::from_fn_with_state(hub.clone(), record))
        .with_state(hub)
}

pub struct MockHub {
    pub hub: Shared,
    pub session: Arc<DeviceSession>,
    pub base: String,
    server: tokio::task::JoinHandle<()>,
    _temp: tempfile::TempDir,
}

impl Drop for MockHub {
    fn drop(&mut self) {
        self.server.abort();
    }
}

impl MockHub {
    pub async fn start(scenario: Scenario) -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}/api/v1", listener.local_addr().unwrap());
        let hub = Arc::new(Mutex::new(Hub {
            base: base.clone(),
            scenario,
            state: TurnState::Queued,
            requests: Vec::new(),
            model_requests: Vec::new(),
            model_in_flight: 0,
            model_dropped: 0,
            checkpoint: None,
            manifest: None,
            challenge: None,
            receipt: None,
            token_requests: 0,
            replay: HashSet::new(),
            dpop_verified: 0,
        }));
        let app = router(hub.clone());
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let temp = tempfile::tempdir().unwrap();
        let controller = temp.path().join("controller");
        let package = temp.path().join("package");
        let target = supervisor::prepare_state_dir(&temp.path().join("target")).unwrap();
        let binary = temp.path().join("binary");
        std::fs::write(&binary, b"test executable").unwrap();
        enrollment::create_package(
            &controller,
            &base,
            "device-bot-e2e",
            &package,
            "enrollment-test-user-token",
            b"correct password kept by owner",
            &binary,
        )
        .await
        .unwrap();
        assert!(enrollment::enroll(&target, &package).await.is_err());
        enrollment::enroll(&target, &package).await.unwrap();
        let session = Arc::new(DeviceSession::load(&target).unwrap().unwrap());
        Self {
            hub,
            session,
            base,
            server,
            _temp: temp,
        }
    }

    pub async fn wait<F: Fn(&Hub) -> bool>(&self, what: &str, within: Duration, ready: F) {
        let deadline = Instant::now() + within;
        loop {
            {
                let hub = self.hub.lock().await;
                if ready(&hub) {
                    return;
                }
                if Instant::now() > deadline {
                    panic!(
                        "timed out waiting for {what}; requests so far:\n{}",
                        hub.requests
                            .iter()
                            .map(|r| format!("{} {} {}", r.method, r.path, r.body))
                            .collect::<Vec<_>>()
                            .join("\n")
                    );
                }
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }
}

struct NoEvents;

impl TurnEvents for NoEvents {
    fn turn(&self, _: &TurnInfo) {}
}

pub struct HostHandle {
    task: tokio::task::JoinHandle<()>,
    _state: tempfile::TempDir,
}

impl Drop for HostHandle {
    fn drop(&mut self) {
        self.task.abort();
    }
}

pub fn start_host(session: Arc<DeviceSession>, machines: Option<Arc<dyn Machines>>) -> HostHandle {
    flow_like::flow_like_model_provider::embedding::proxy_config::set_api_base_url(
        DeviceApi::base_url(session.as_ref()),
    );
    let host_id = flow_like_standalone::bot_host::host_id(DeviceApi::device_id(session.as_ref()));
    let base_url = DeviceApi::base_url(session.as_ref()).to_owned();
    let api: Arc<dyn DeviceApi> = session;
    let hub = Arc::new(DeviceTurnHub::new(api, host_id.clone()));
    let state = Arc::new(flow_like::state::FlowLikeState::new(
        flow_like::state::FlowLikeConfig::new(),
        flow_like::utils::http::HTTPClient::new_without_refetch(),
    ));
    let state_dir = tempfile::tempdir().unwrap();
    let runner = Arc::new(DeviceRunner::new(
        state,
        base_url,
        machines,
        state_dir.path(),
    ));
    let mut config = AccountConfig::new(host_id);
    config.fire_tasks = false;
    config.heartbeat = Duration::from_millis(200);
    config.send_retry_delay = Duration::from_millis(100);
    let host = AccountHost::new(
        hub,
        runner,
        Arc::new(NoEvents),
        Arc::new(tokio::sync::Semaphore::new(2)),
        config,
    );
    HostHandle {
        task: tokio::spawn(host.run()),
        _state: state_dir,
    }
}

static SERIAL: Mutex<()> = Mutex::const_new(());

pub async fn serial() -> tokio::sync::MutexGuard<'static, ()> {
    SERIAL.lock().await
}
