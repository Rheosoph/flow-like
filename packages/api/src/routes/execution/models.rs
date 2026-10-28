//! Executor-only relay for the private model Bits of a run's model owner.
//!
//! The executor holds no provider secret. It sends the hop (OpenAI Chat Completions, or a
//! SystemOne request, with `model` = the Bit id) with its run token, and the API calls the
//! provider with a client built for that one call from the owner's encrypted `UserBit` row.
//! `flow_like_model_provider::llm::relay` describes the wire format.
//!
//! A chat hop carries media as the run holds it: links unchanged, and base64 data only for media
//! that exists as bytes. Before the provider is called, [`media`] resolves each link the way a
//! direct call with the same Bit would, so file bytes cross the API only where the provider
//! needs them inline or cannot reach the link.

mod media;

use std::{collections::HashMap, convert::Infallible, sync::Arc, time::Duration};

use axum::{
    Extension, Json,
    body::Body,
    extract::{Path, State},
    http::{HeaderValue, header},
    response::{IntoResponse, Response},
};
use bytes::Bytes;
use chrono::{DateTime, Utc};
use flow_like::flow_like_model_provider::{
    egress::ProviderEgress,
    history::{History, HistoryThinking},
    llm::{
        LLMCallback, ModelLogic,
        custom::{self, AmbientCredentials},
        relay::{history_from_relay_request, relay_chunk, relay_completion, relay_error},
    },
    provider::{ModelApiSurface, ModelProvider},
    response_chunk::ResponseChunk,
    systemone::{SystemOneClient, SystemOneModelLogic, SystemOneRequest},
};
use flow_like_types::tokio::{
    self,
    sync::mpsc,
    time::{Instant, MissedTickBehavior},
};
use futures_util::Stream;
use sea_orm::{ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::{
    entity::{sea_orm_active_enums::BitType, user_bit},
    error::ApiError,
    execution::{live_run::live_run, state::ExecutionRunRecord, subject::account_user_id},
    middleware::{
        deadline::DeadlineClass,
        jwt::{AppUser, ExecutorUser, api_key_run_role, connection_holds_person},
    },
    permission::role_permission::RolePermissions,
    quota::{QuotaAmounts, QuotaRequest},
    routes::{
        chat::{
            UsageRequestContext,
            hosted_worker::{self, HostedAiJob, HostedWork},
            systemone::validate_payload,
            track_llm_usage,
        },
        sink::authority::{RunAsDenial, RunAsVerdict, run_as_authority},
        user::bits::{relayable_provider, user_bit_to_core},
    },
    state::AppState,
    usage_accounting::STATUS_COMPLETED,
};

/// The largest hop, inline media included. The executor refuses a larger call with its size.
pub(crate) const MAX_REQUEST_BYTES: usize =
    flow_like::flow_like_model_provider::llm::relay::MAX_REQUEST_BYTES;
/// How long the status waits for the call's first event, so an early failure keeps it.
const HEADER_HOLD: Duration = Duration::from_secs(10);
const KEEPALIVE: Duration = Duration::from_secs(10);
/// Leaves room to report a timeout in band before the route's deadline cuts the body.
const DEADLINE_MARGIN: Duration = Duration::from_secs(5);
const RETRY_AFTER_SECS: u64 = 30;
const EVENT_BUFFER: usize = 32;
const MIN_SECRET_LEN: usize = 4;
const SECRET_PARAMS: [&str; 5] = [
    "api_key",
    "access_token",
    "headers",
    "service_account_json",
    "service_account_key",
];
const SERVICE_ACCOUNT_SECRETS: [&str; 4] = [
    "private_key",
    "private_key_id",
    "client_secret",
    "refresh_token",
];
/// Their reasoning on a tool continuation needs a signature that the hop cannot carry.
const SIGNED_REASONING_PROVIDERS: [&str; 5] = [
    "custom:anthropic",
    "custom:bedrock",
    "custom:gemini",
    "custom:vertex",
    "custom:openrouter",
];
const OWNER_REFUSED: &str = "The model's owner can no longer use it in this app";
const SSE_DONE: &[u8] = b"data: [DONE]\n\n";
const SSE_KEEPALIVE: &[u8] = b": keepalive\n\n";
const JSON_KEEPALIVE: &[u8] = b" ";

#[utoipa::path(
    post,
    path = "/execution/apps/{app_id}/models/chat/completions",
    tag = "execution",
    description = "Call a private chat model of the run's model owner. The body is an OpenAI Chat Completions request with `model` set to the Bit id and an optional `thinking` of off, low, mid or high. Media in a user message are `image_url`, `document_url`, `audio_url` or `video_url` parts, each an http(s) link or a base64 data URL. The relay hands a link to the provider when the provider reads links and the link's host is public, signing a link into the platform's storage again for a few minutes; it fetches any other link itself and sends that media inline. With `stream: true` the answer is server-sent chunk frames with `: keepalive` comments and a closing `data: [DONE]`; otherwise it is a `chat.completion` object that may follow whitespace keepalives. A failure after the answer has started, including a media link that cannot be fetched, arrives in band as `{\"error\":{\"message\",\"type\"}}`. Executor only.",
    params(("app_id" = String, Path, description = "Application ID")),
    request_body = serde_json::Value,
    responses(
        (status = 200, description = "A chat.completion object, or text/event-stream chunk frames when stream is true", body = serde_json::Value),
        (status = 400, description = "Invalid request, or media that is neither an http(s) link nor a base64 data URL"),
        (status = 403, description = "Not a live run of this app that may use private models, or the model's owner can no longer use this app"),
        (status = 404, description = "The run's model owner has no private model with this id"),
        (status = 422, description = "Not a chat model, a provider that runs on desktop only, or an endpoint this server may not call"),
        (status = 429, description = "The provider is rate limiting the model", headers(("Retry-After" = u64, description = "Seconds to wait before retrying"))),
        (status = 413, description = "The request is larger than 6,000,000 bytes, inline media included"),
        (status = 502, description = "The provider failed, or a media link could not be fetched; the message names at most the link's host and carries no URL or credential"),
        (status = 504, description = "The provider did not answer before the run's deadline")
    ),
    security(("executor_jwt" = []))
)]
#[tracing::instrument(
    name = "POST /execution/apps/{app_id}/models/chat/completions",
    skip_all
)]
pub async fn chat_completions(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(app_id): Path<String>,
    Json(body): Json<Value>,
) -> Result<Response, ApiError> {
    relay(state, &user, &app_id, body, false).await
}

#[utoipa::path(
    post,
    path = "/execution/apps/{app_id}/models/systemone",
    tag = "execution",
    description = "Ask a private SystemOne model of the run's model owner typed questions. The body is a SystemOne request with `model` set to the Bit id; images travel only as base64 data URLs. The answer may follow whitespace keepalives, and a failure after that arrives as `{\"error\":{\"message\",\"type\"}}`. Executor only.",
    params(("app_id" = String, Path, description = "Application ID")),
    request_body = serde_json::Value,
    responses(
        (status = 200, description = "Typed answers, probabilities and usage", body = serde_json::Value),
        (status = 400, description = "Invalid SystemOne request"),
        (status = 403, description = "Not a live run of this app that may use private models, or the model's owner can no longer use this app"),
        (status = 404, description = "The run's model owner has no private model with this id"),
        (status = 422, description = "Not a SystemOne model, a provider that runs on desktop only, or an endpoint this server may not call"),
        (status = 429, description = "The provider is rate limiting the model", headers(("Retry-After" = u64, description = "Seconds to wait before retrying"))),
        (status = 502, description = "The provider failed or returned invalid answers; the message carries no URL or credential"),
        (status = 504, description = "The provider did not answer before the run's deadline")
    ),
    security(("executor_jwt" = []))
)]
#[tracing::instrument(name = "POST /execution/apps/{app_id}/models/systemone", skip_all)]
pub async fn systemone(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(app_id): Path<String>,
    Json(body): Json<Value>,
) -> Result<Response, ApiError> {
    relay(state, &user, &app_id, body, true).await
}

/// A relayed call queued for the hosted worker: ids and the executor's hop body, never a
/// secret. The body holds media links as the run sent them; the worker resolves them when it
/// runs the call.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct RelayCall {
    pub(crate) owner: String,
    pub(crate) run_id: String,
    pub(crate) bit_id: String,
    pub(crate) systemone: bool,
    pub(crate) body: Value,
    pub(crate) stream: bool,
    pub(crate) context: UsageRequestContext,
    /// How the owner, the person the run acts as, may stand in the app without being a member.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) person: Option<PersonRun>,
}

/// A run that acts as the person who owns its private models, and how that person may stand
/// in the app without being a member of it. A member stands on their membership whatever the
/// run; anyone else may use their private models only through one of these.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum PersonRun {
    /// A signed-in visitor of a hosted page, who may run the page's events.
    HostedVisitor,
    /// A person the app connection from `origin_app_id` passed through, while that connection
    /// grants them some standing in the app.
    Connection { origin_app_id: String },
}

impl PersonRun {
    /// The person run `executor` is when it acts as `owner`. A run nobody triggered, a run an
    /// API key started and a run whose models belong to someone else than its subject are none,
    /// and a pseudonym never owns models (see [`authorize_call`]).
    fn of(executor: &ExecutorUser, owner: &str) -> Option<Self> {
        if executor.unattended || executor.technical_user_id.is_some() || executor.sub != owner {
            return None;
        }
        match executor.app_chain.as_ref().and_then(|chain| chain.last()) {
            Some(origin_app_id) => Some(Self::Connection {
                origin_app_id: origin_app_id.clone(),
            }),
            None => executor.hosted_frontend.then_some(Self::HostedVisitor),
        }
    }

    /// Whether `person` still stands in `app_id` this way, read fresh like every callback.
    async fn stands_in<C: ConnectionTrait>(
        &self,
        db: &C,
        app_id: &str,
        person: &str,
    ) -> Result<bool, ApiError> {
        match self {
            Self::HostedVisitor => Ok(true),
            Self::Connection { origin_app_id } => {
                connection_holds_person(db, origin_app_id, app_id, person).await
            }
        }
    }
}

async fn relay(
    state: AppState,
    user: &AppUser,
    app_id: &str,
    body: Value,
    systemone: bool,
) -> Result<Response, ApiError> {
    let (executor, owner) = authorize_call(user, app_id)?;
    let person = PersonRun::of(&executor, &owner);
    let run = live_call(
        &state,
        app_id,
        &executor.run_id,
        &owner,
        person.as_ref(),
        executor.technical_user_id.as_deref(),
    )
    .await?;
    let request = RelayRequest::parse(&body, systemone)?;
    let bit_id = request.bit_id().to_owned();
    let (row, provider) = relayed_bit(&state.db, &owner, &bit_id, systemone).await?;
    let context = UsageRequestContext {
        app_id: Some(app_id.to_owned()),
        user_id: executor.sub,
        technical_user_id: executor.technical_user_id,
    };
    let deadline = call_deadline(Utc::now(), run.expires_at, DeadlineClass::Ai.budget());
    if hosted_worker::enabled() {
        let call = RelayCall {
            owner,
            run_id: executor.run_id,
            bit_id,
            systemone,
            stream: request.stream(),
            body,
            context,
            person,
        };
        return queue(&state, call, &provider.provider_name, deadline).await;
    }
    Ok(run_call(&state, context, row, request, deadline, None).await)
}

/// The executor of `app_id` and the user whose private Bits its run may call. The
/// dispatcher binds that owner only for the run's own principal, so a run reached through an
/// app connection keeps it; a placeholder or pseudonym never names an account.
fn authorize_call(user: &AppUser, app_id: &str) -> Result<(ExecutorUser, String), ApiError> {
    let AppUser::Executor(executor) = user else {
        return Err(ApiError::forbidden(
            "Only a running workflow can call its private models",
        ));
    };
    if executor.app_id != app_id {
        return Err(ApiError::forbidden(
            "The run token belongs to a different app",
        ));
    }
    let owner = executor
        .custom_bits_owner
        .as_deref()
        .and_then(account_user_id)
        .ok_or_else(|| ApiError::forbidden("This run may not call private models"))?
        .to_owned();
    Ok((executor.clone(), owner))
}

/// The run is live in `app_id`, its owner may still use the app and, for a run started with
/// an API key (`key_id`), that key still holds a current role, as on the run's other
/// callbacks.
async fn live_call(
    state: &AppState,
    app_id: &str,
    run_id: &str,
    owner: &str,
    person: Option<&PersonRun>,
    key_id: Option<&str>,
) -> Result<ExecutionRunRecord, ApiError> {
    let run = live_run(state, run_id).await?;
    if run.app_id != app_id {
        return Err(ApiError::forbidden("The run belongs to a different app"));
    }
    ensure_owner_may_use(&state.db, app_id, owner, person).await?;
    ensure_key_may_call(&state.db, app_id, run_id, key_id).await?;
    Ok(run)
}

/// A run started with an API key delegates the key creator's models only while the key
/// exists, holds a role and is unexpired.
async fn ensure_key_may_call<C: ConnectionTrait>(
    db: &C,
    app_id: &str,
    run_id: &str,
    key_id: Option<&str>,
) -> Result<(), ApiError> {
    if let Some(key_id) = key_id {
        api_key_run_role(db, app_id, key_id, run_id).await?;
    }
    Ok(())
}

/// The owner's account is active in an active app, and they are a member of it or the run
/// acts as them and they still stand in the app as its [`PersonRun`] says.
async fn ensure_owner_may_use<C: ConnectionTrait>(
    db: &C,
    app_id: &str,
    owner: &str,
    person: Option<&PersonRun>,
) -> Result<(), ApiError> {
    let verdict = run_as_authority(db, app_id, Some(owner), RolePermissions::empty()).await?;
    let person_stands = match (verdict, person) {
        (RunAsVerdict::Denied(RunAsDenial::NotMember), Some(person)) => {
            person.stands_in(db, app_id, owner).await?
        }
        _ => false,
    };
    owner_verdict(verdict, person_stands)
}

/// Only a refusal for not being a member yields to a person who stands in the app otherwise;
/// an inactive app or account refuses everyone.
fn owner_verdict(verdict: RunAsVerdict, person_stands: bool) -> Result<(), ApiError> {
    match verdict {
        RunAsVerdict::Allowed(_) => Ok(()),
        RunAsVerdict::Denied(RunAsDenial::NotMember) if person_stands => Ok(()),
        RunAsVerdict::Denied(_) => Err(ApiError::forbidden(OWNER_REFUSED)),
        RunAsVerdict::Transient(reason) => Err(ApiError::internal(reason)),
    }
}

enum RelayRequest {
    Chat {
        history: Box<History>,
        stream: bool,
        requested_thinking: Option<HistoryThinking>,
    },
    SystemOne {
        bit_id: String,
        request: SystemOneRequest,
    },
}

impl RelayRequest {
    fn parse(body: &Value, systemone: bool) -> Result<Self, ApiError> {
        if systemone {
            let request = validate_payload(body)?;
            let bit_id = body["model"].as_str().unwrap_or_default().to_owned();
            return Ok(Self::SystemOne { bit_id, request });
        }
        let (history, stream) = history_from_relay_request(body)
            .map_err(|error| ApiError::bad_request(error.to_string()))?;
        let requested_thinking = body
            .get("thinking")
            .and_then(Value::as_str)
            .and_then(|thinking| thinking.parse().ok());
        Ok(Self::Chat {
            history: Box::new(history),
            stream,
            requested_thinking,
        })
    }

    fn bit_id(&self) -> &str {
        match self {
            Self::Chat { history, .. } => &history.model,
            Self::SystemOne { bit_id, .. } => bit_id,
        }
    }

    fn stream(&self) -> bool {
        matches!(self, Self::Chat { stream: true, .. })
    }
}

/// The owner's Bit behind `bit_id`. A Bit of anyone else is as missing as a deleted one.
async fn relayed_bit<C: ConnectionTrait>(
    db: &C,
    owner: &str,
    bit_id: &str,
    systemone: bool,
) -> Result<(user_bit::Model, ModelProvider), ApiError> {
    let row = user_bit::Entity::find_by_id(bit_id)
        .filter(user_bit::Column::UserId.eq(owner))
        .one(db)
        .await?
        .ok_or_else(|| {
            ApiError::not_found(format!(
                "The run's model owner has no private model '{bit_id}'"
            ))
        })?;
    let provider = relay_provider(&row, systemone)?;
    Ok((row, provider))
}

/// The chat route takes LLM and VLM Bits that do not speak SystemOne, the SystemOne route
/// only SystemOne Bits, and both only providers this server may call for a run.
fn relay_provider(row: &user_bit::Model, systemone: bool) -> Result<ModelProvider, ApiError> {
    let provider = row
        .parameters
        .as_ref()
        .and_then(provider_of)
        .ok_or_else(|| ApiError::unprocessable(format!("Model '{}' names no provider", row.id)))?;
    let fits_route = if systemone {
        row.r#type == BitType::SystemOne
    } else {
        matches!(row.r#type, BitType::Llm | BitType::Vlm)
            && provider.api_surface != Some(ModelApiSurface::SystemOne)
    };
    if !fits_route {
        let expected = if systemone {
            "a SystemOne model"
        } else {
            "a chat model"
        };
        return Err(ApiError::unprocessable(format!(
            "Model '{}' is not {expected}",
            row.id
        )));
    }
    if !relayable_provider(&provider.provider_name) {
        return Err(ApiError::unprocessable(format!(
            "Model '{}' runs on desktop only",
            row.id
        )));
    }
    Ok(provider)
}

pub(crate) fn provider_of(parameters: &Value) -> Option<ModelProvider> {
    ModelProvider::deserialize(parameters.get("provider")?).ok()
}

/// The earlier of the run's own expiry and the route's AI budget less [`DEADLINE_MARGIN`].
fn call_deadline(
    now: DateTime<Utc>,
    run_expires_at_ms: Option<i64>,
    budget: Duration,
) -> DateTime<Utc> {
    let by_budget = now
        + chrono::Duration::from_std(budget.saturating_sub(DEADLINE_MARGIN)).unwrap_or_default();
    run_expires_at_ms
        .and_then(DateTime::from_timestamp_millis)
        .map_or(by_budget, |expiry| expiry.min(by_budget))
}

/// On Lambda the call runs in the hosted worker. A zero-amount reservation drives the
/// worker's start and settle steps; the worker repeats every check and decrypts only when it
/// runs the call.
async fn queue(
    state: &AppState,
    call: RelayCall,
    provider: &str,
    deadline: DateTime<Utc>,
) -> Result<Response, ApiError> {
    let operation_id = flow_like_types::create_id();
    let actor_id = account_user_id(&call.context.user_id).map(ToOwned::to_owned);
    let app_id = call.context.app_id.clone();
    let payer_id =
        crate::quota::resolve_payer(state, actor_id.as_deref(), app_id.as_deref()).await?;
    crate::quota::reserve(
        state,
        QuotaRequest {
            operation_id: operation_id.clone(),
            payer_id,
            actor_id,
            app_id,
            model_id: Some(call.bit_id.clone()),
            provider: Some(provider.trim().to_ascii_lowercase()),
            kind: "assistant".into(),
            funding_class: "byok".into(),
            execution_mode: "cloud_model_relay".into(),
            amounts: QuotaAmounts::default(),
            deadline,
        },
    )
    .await?;
    hosted_worker::dispatch(
        state.clone(),
        HostedAiJob {
            operation_id,
            request: HostedWork::CustomBit(call),
            deadline,
        },
    )
    .await
}

/// The hosted worker's half of a queued call. Recovery may deliver it long after it was
/// queued, so the run, the owner's standing and the Bit are checked again before the owner's
/// key is decrypted.
pub(crate) async fn run_queued(
    state: &AppState,
    call: RelayCall,
    deadline: DateTime<Utc>,
    operation_id: &str,
) -> Response {
    match queued_request(state, &call).await {
        Ok((request, row)) => {
            let operation = Some(operation_id.to_owned());
            run_call(state, call.context, row, request, deadline, operation).await
        }
        Err(error) => {
            settle_operation(state, operation_id, false, Duration::ZERO).await;
            error.into_response()
        }
    }
}

async fn queued_request(
    state: &AppState,
    call: &RelayCall,
) -> Result<(RelayRequest, user_bit::Model), ApiError> {
    let app_id = call.context.app_id.as_deref().unwrap_or_default();
    live_call(
        state,
        app_id,
        &call.run_id,
        &call.owner,
        call.person.as_ref(),
        call.context.technical_user_id.as_deref(),
    )
    .await?;
    let request = RelayRequest::parse(&call.body, call.systemone)?;
    if request.bit_id() != call.bit_id {
        return Err(ApiError::bad_request(
            "The queued model call names another model",
        ));
    }
    let (row, _) = relayed_bit(&state.db, &call.owner, &call.bit_id, call.systemone).await?;
    Ok((request, row))
}

/// Finalizes the zero-amount reservation of a queued call. Repeating it changes nothing.
pub(crate) async fn settle_operation(
    state: &AppState,
    operation_id: &str,
    success: bool,
    elapsed: Duration,
) {
    let detail = json!({
        "status": if success { "completed" } else { "failed" },
        "elapsedMs": u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX),
        "fundingClass": "byok",
    });
    if let Err(error) = crate::quota::settle(
        state,
        operation_id,
        "relay-terminal",
        QuotaAmounts::default(),
        true,
        detail,
    )
    .await
    {
        tracing::warn!(operation_id, %error, "Failed to settle a private model relay operation");
    }
}

async fn run_call(
    state: &AppState,
    context: UsageRequestContext,
    row: user_bit::Model,
    request: RelayRequest,
    deadline: DateTime<Utc>,
    operation: Option<String>,
) -> Response {
    let stream = request.stream();
    match prepare(state, context, row, request).await {
        Ok(prepared) => {
            let (events, receiver) = mpsc::channel(EVENT_BUFFER);
            tokio::spawn(produce(
                state.clone(),
                prepared,
                deadline,
                operation,
                events,
            ));
            respond(receiver, stream).await
        }
        Err(error) => {
            if let Some(operation) = &operation {
                settle_operation(state, operation, false, Duration::ZERO).await;
            }
            error.into_response()
        }
    }
}

struct Prepared {
    invocation: Invocation,
    secrets: Vec<String>,
    usage: UsageLabels,
}

enum Invocation {
    Chat {
        model: Arc<dyn ModelLogic>,
        history: History,
        stream: bool,
    },
    SystemOne {
        client: SystemOneClient,
        request: SystemOneRequest,
    },
}

/// What the usage row records: the provider's model and name and, on the owner's own row, the
/// endpoint's host, never its URL.
struct UsageLabels {
    context: UsageRequestContext,
    model_id: String,
    provider: String,
    endpoint_host: Option<String>,
}

/// Decrypts the Bit and builds its provider's client for this call only.
async fn prepare(
    state: &AppState,
    context: UsageRequestContext,
    row: user_bit::Model,
    request: RelayRequest,
) -> Result<Prepared, ApiError> {
    let bit_id = row.id.clone();
    let owner = row.user_id.clone();
    let mut provider = provider_of(&user_bit_to_core(row, state, true).parameters)
        .ok_or_else(|| ApiError::unprocessable(format!("Model '{bit_id}' names no provider")))?;
    provider.provider_name = provider.provider_name.trim().to_ascii_lowercase();
    let secrets = secret_values(provider.params.as_ref());
    let endpoint = endpoint_param(&provider);
    let egress = ProviderEgress::for_server();
    if let Some(endpoint) = endpoint {
        egress
            .ensure_endpoint_allowed(endpoint)
            .await
            .map_err(|refused| ApiError::unprocessable(refused.to_string()))?;
    }
    let invocation = invocation(state, &provider, request, egress)
        .await
        .map_err(|error| ApiError::unprocessable(scrub(&format!("{error:#}"), &secrets)))?;
    let usage = UsageLabels {
        endpoint_host: owners_endpoint_host(endpoint, &context.user_id, &owner),
        context,
        model_id: provider
            .model_id
            .clone()
            .filter(|model_id| !model_id.trim().is_empty())
            .unwrap_or(bit_id),
        provider: provider.provider_name,
    };
    Ok(Prepared {
        invocation,
        secrets,
        usage,
    })
}

/// The provider's own client, confined to the server's egress policy and without the host's
/// own cloud identity. A chat model resolves the call's media links before each invocation.
async fn invocation(
    state: &AppState,
    provider: &ModelProvider,
    request: RelayRequest,
    egress: ProviderEgress,
) -> flow_like_types::Result<Invocation> {
    let name = provider.provider_name.as_str();
    Ok(match request {
        RelayRequest::Chat {
            mut history,
            stream,
            requested_thinking,
        } => {
            history.thinking = relayed_thinking(name, requested_thinking, history.thinking);
            let model =
                custom::build(name, provider, egress, AmbientCredentials::Forbidden).await?;
            Invocation::Chat {
                model: media::resolving(model, state, provider),
                history: *history,
                stream,
            }
        }
        RelayRequest::SystemOne { request, .. } => Invocation::SystemOne {
            client: custom::systemone(name, provider, egress)?,
            request,
        },
    })
}

/// [`history_from_relay_request`] turns thinking off once an assistant turn has tool calls,
/// because the hop carries no reasoning signatures. Providers that need none get the
/// requested level back.
fn relayed_thinking(
    provider_name: &str,
    requested: Option<HistoryThinking>,
    parsed: Option<HistoryThinking>,
) -> Option<HistoryThinking> {
    if SIGNED_REASONING_PROVIDERS.contains(&provider_name) {
        parsed
    } else {
        requested
    }
}

fn endpoint_param(provider: &ModelProvider) -> Option<&str> {
    provider
        .params
        .as_ref()?
        .get("endpoint")?
        .as_str()
        .map(str::trim)
        .filter(|endpoint| !endpoint.is_empty())
}

fn endpoint_host(endpoint: &str) -> Option<String> {
    reqwest::Url::parse(endpoint)
        .ok()?
        .host_str()
        .map(ToOwned::to_owned)
}

/// A hosted visitor runs as themself on the publisher's model, so their usage history never
/// names where that model runs.
fn owners_endpoint_host(endpoint: Option<&str>, subject: &str, owner: &str) -> Option<String> {
    endpoint
        .filter(|_| account_user_id(subject) == Some(owner))
        .and_then(endpoint_host)
}

enum Event {
    Chunk(Value),
    Done(Result<Value, Failure>),
}

struct Answer {
    body: Value,
    tokens: (i64, i64),
}

/// Runs the call until it ends, its deadline passes or the reader goes away, which drops
/// the provider request.
async fn produce(
    state: AppState,
    prepared: Prepared,
    deadline: DateTime<Utc>,
    operation: Option<String>,
    events: mpsc::Sender<Event>,
) {
    let Prepared {
        invocation,
        secrets,
        usage,
    } = prepared;
    let started = std::time::Instant::now();
    let budget = (deadline - Utc::now()).to_std().unwrap_or_default();
    let outcome = tokio::select! {
        outcome = tokio::time::timeout(budget, invoke(invocation, &events)) => match outcome {
            Ok(Ok(answer)) => Ok(answer),
            Ok(Err(error)) => Err(Failure::provider(&error, &secrets)),
            Err(_) => Err(Failure::timeout()),
        },
        () = events.closed() => return,
    };
    let elapsed = started.elapsed();
    match &outcome {
        Ok(answer) => record_usage(&state, &usage, answer.tokens, elapsed).await,
        Err(failure) => tracing::warn!(
            provider = %usage.provider,
            error = %failure.message,
            "Private model call failed"
        ),
    }
    if let Some(operation) = &operation {
        settle_operation(&state, operation, outcome.is_ok(), elapsed).await;
    }
    let _ = events
        .send(Event::Done(outcome.map(|answer| answer.body)))
        .await;
}

async fn invoke(
    invocation: Invocation,
    events: &mpsc::Sender<Event>,
) -> flow_like_types::Result<Answer> {
    match invocation {
        Invocation::Chat {
            model,
            history,
            stream,
        } => {
            let callback = stream.then(|| forward_chunks(events.clone()));
            let response = model.invoke(&history, callback).await?;
            Ok(Answer {
                tokens: (
                    i64::from(response.usage.prompt_tokens),
                    i64::from(response.usage.completion_tokens),
                ),
                body: relay_completion(&response),
            })
        }
        Invocation::SystemOne { client, request } => {
            let response = client.invoke(&request).await?;
            response.validate_for(&request)?;
            let tokens = response.usage.as_ref().map_or((0, 0), |usage| {
                (
                    i64::try_from(usage.input_tokens).unwrap_or(i64::MAX),
                    i64::try_from(usage.output_tokens).unwrap_or(i64::MAX),
                )
            });
            Ok(Answer {
                tokens,
                body: serde_json::to_value(&response)?,
            })
        }
    }
}

fn forward_chunks(events: mpsc::Sender<Event>) -> LLMCallback {
    Arc::new(move |chunk: ResponseChunk| {
        let events = events.clone();
        Box::pin(async move {
            events
                .send(Event::Chunk(relay_chunk(&chunk)))
                .await
                .map_err(|_| {
                    flow_like_types::anyhow!("The executor stopped reading the model relay")
                })
        })
    })
}

async fn record_usage(
    state: &AppState,
    usage: &UsageLabels,
    (token_in, token_out): (i64, i64),
    elapsed: Duration,
) {
    if let Err(error) = track_llm_usage(
        state,
        &usage.context.user_id,
        &usage.model_id,
        token_in,
        token_out,
        0,
        elapsed.as_secs_f64() * 1000.0,
        usage.context.app_id.as_deref(),
        usage.context.technical_user_id.as_deref(),
        Some(&usage.provider),
        usage.endpoint_host.as_deref(),
        None,
        None,
        None,
        STATUS_COMPLETED,
    )
    .await
    {
        tracing::warn!(%error, "Failed to record private model usage");
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FailureKind {
    RateLimited,
    Timeout,
    Provider,
}

#[derive(Debug)]
struct Failure {
    kind: FailureKind,
    message: String,
}

impl Failure {
    fn provider(error: &flow_like_types::Error, secrets: &[String]) -> Self {
        let message = scrub(&format!("{error:#}"), secrets);
        let kind = if mentions_rate_limit(&message) {
            FailureKind::RateLimited
        } else {
            FailureKind::Provider
        };
        Self { kind, message }
    }

    fn timeout() -> Self {
        Self {
            kind: FailureKind::Timeout,
            message: "The private model did not answer before the run's deadline".to_owned(),
        }
    }

    fn lost() -> Self {
        Self {
            kind: FailureKind::Provider,
            message: "The private model call ended without an answer".to_owned(),
        }
    }

    fn wire_type(&self) -> &str {
        match self.kind {
            FailureKind::RateLimited => "rate_limited",
            FailureKind::Timeout => "timeout",
            FailureKind::Provider => "provider_error",
        }
    }

    fn into_response(self) -> Response {
        let kind = self.kind;
        let mut response = match kind {
            FailureKind::RateLimited => ApiError::too_many_requests(self.message),
            FailureKind::Timeout => ApiError::gateway_timeout(self.message),
            FailureKind::Provider => ApiError::bad_gateway(self.message),
        }
        .into_response();
        if kind == FailureKind::RateLimited {
            response
                .headers_mut()
                .insert(header::RETRY_AFTER, HeaderValue::from(RETRY_AFTER_SECS));
        }
        response
    }

    /// The failure once the answer has started: an error frame and the closing `[DONE]` on
    /// a stream, the error body otherwise.
    fn in_band(&self, stream: bool) -> Bytes {
        let error = relay_error(self.wire_type(), &self.message);
        Bytes::from(if stream {
            format!("data: {error}\n\ndata: [DONE]\n\n")
        } else {
            error.to_string()
        })
    }
}

/// Waits up to [`HEADER_HOLD`] for the call's first event, then commits 200 and keeps the
/// body alive until the call ends. A producer that is gone just closes the body, which the
/// executor reports as a cut answer.
async fn respond(mut events: mpsc::Receiver<Event>, stream: bool) -> Response {
    let first = match tokio::time::timeout(HEADER_HOLD, events.recv()).await {
        Ok(Some(Event::Done(Err(failure)))) => return failure.into_response(),
        Ok(Some(Event::Done(Ok(body)))) if !stream => return Json(body).into_response(),
        Ok(Some(event)) => Some(event),
        Ok(None) => return Failure::lost().into_response(),
        Err(_) => None,
    };
    committed(first, events, stream)
}

enum Step {
    Event(Option<Event>),
    Keepalive,
}

impl Step {
    /// What a committed body sends for this step, and whether the body ends with it. A
    /// producer that is gone ends the body without a closing frame.
    fn frame(self, stream: bool) -> (Option<Bytes>, bool) {
        match self {
            Self::Keepalive => (
                Some(Bytes::from_static(if stream {
                    SSE_KEEPALIVE
                } else {
                    JSON_KEEPALIVE
                })),
                false,
            ),
            Self::Event(Some(Event::Chunk(frame))) => {
                (Some(Bytes::from(format!("data: {frame}\n\n"))), false)
            }
            Self::Event(Some(Event::Done(Ok(body)))) => (
                Some(if stream {
                    Bytes::from_static(SSE_DONE)
                } else {
                    Bytes::from(body.to_string())
                }),
                true,
            ),
            Self::Event(Some(Event::Done(Err(failure)))) => (Some(failure.in_band(stream)), true),
            Self::Event(None) => (None, true),
        }
    }
}

fn committed(first: Option<Event>, events: mpsc::Receiver<Event>, stream: bool) -> Response {
    let content_type = if stream {
        "text/event-stream"
    } else {
        "application/json"
    };
    let mut response = Body::from_stream(committed_frames(first, events, stream)).into_response();
    let headers = response.headers_mut();
    headers.insert(header::CONTENT_TYPE, HeaderValue::from_static(content_type));
    headers.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("no-cache, no-transform"),
    );
    headers.insert("x-accel-buffering", HeaderValue::from_static("no"));
    response
}

/// Keepalives come every [`KEEPALIVE`] that passes without a chunk.
fn committed_frames(
    first: Option<Event>,
    mut events: mpsc::Receiver<Event>,
    stream: bool,
) -> impl Stream<Item = Result<Bytes, Infallible>> {
    async_stream::stream! {
        let mut keepalive = tokio::time::interval_at(Instant::now() + KEEPALIVE, KEEPALIVE);
        keepalive.set_missed_tick_behavior(MissedTickBehavior::Delay);
        let mut pending = first;
        loop {
            let step = match pending.take() {
                Some(event) => Step::Event(Some(event)),
                None => tokio::select! {
                    event = events.recv() => Step::Event(event),
                    _ = keepalive.tick() => Step::Keepalive,
                },
            };
            if matches!(step, Step::Event(Some(Event::Chunk(_)))) {
                keepalive.reset();
            }
            let (frame, last) = step.frame(stream);
            if let Some(frame) = frame {
                yield Ok::<_, Infallible>(frame);
            }
            if last {
                break;
            }
        }
    }
}

/// The decrypted credential values of a provider, plus the private parts of a service
/// account key, longest first so no value survives inside another.
pub(crate) fn secret_values(params: Option<&HashMap<String, Value>>) -> Vec<String> {
    let mut values = Vec::new();
    for value in SECRET_PARAMS.iter().filter_map(|key| params?.get(*key)) {
        match value {
            Value::String(text) => {
                values.push(text.clone());
                if let Ok(Value::Object(account)) = serde_json::from_str::<Value>(text) {
                    values.extend(
                        SERVICE_ACCOUNT_SECRETS.iter().filter_map(|field| {
                            account.get(*field)?.as_str().map(ToOwned::to_owned)
                        }),
                    );
                }
            }
            Value::Object(headers) => values.extend(
                headers
                    .values()
                    .filter_map(|header| header.as_str().map(ToOwned::to_owned)),
            ),
            _ => {}
        }
    }
    values.retain(|value| value.trim().len() >= MIN_SECRET_LEN);
    values.sort_by(|a, b| b.len().cmp(&a.len()).then_with(|| a.cmp(b)));
    values.dedup();
    values
}

/// Error text without the provider's credentials or any URL: rig and reqwest errors carry
/// whole request URLs, and Gemini sends its key as `?key=`. A credential is also removed in
/// its JSON-escaped and percent-encoded forms.
pub(crate) fn scrub(text: &str, secrets: &[String]) -> String {
    let mut text = text.to_owned();
    for secret in secrets {
        let quoted = serde_json::to_string(secret).unwrap_or_default();
        let escaped = quoted
            .get(1..quoted.len().saturating_sub(1))
            .unwrap_or_default();
        let encoded = urlencoding::encode(secret);
        for form in [secret.as_str(), escaped, encoded.as_ref()] {
            if !form.is_empty() {
                text = text.replace(form, "[REDACTED]");
            }
        }
    }
    strip_urls(&text)
}

/// Replaces each `scheme://…` up to whitespace, a quote, or a closing parenthesis, angle
/// bracket or brace. A square bracket belongs to the URL: IPv6 hosts and the redaction
/// marker contain one.
fn strip_urls(text: &str) -> String {
    let mut stripped = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(separator) = rest.find("://") {
        let start = rest[..separator]
            .char_indices()
            .rev()
            .take_while(|(_, c)| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
            .last()
            .map_or(separator, |(index, _)| index);
        let tail = &rest[separator + 3..];
        let end = tail
            .find(|c: char| {
                c.is_whitespace() || matches!(c, '"' | '\'' | '`' | '<' | '>' | ')' | '}')
            })
            .unwrap_or(tail.len());
        stripped.push_str(&rest[..start]);
        stripped.push_str("[url]");
        rest = &tail[end..];
    }
    stripped.push_str(rest);
    stripped
}

fn mentions_rate_limit(message: &str) -> bool {
    let message = message.to_ascii_lowercase();
    let digit_at = |index: Option<usize>| {
        index
            .and_then(|index| message.as_bytes().get(index))
            .is_some_and(u8::is_ascii_digit)
    };
    [
        "rate limit",
        "rate_limit",
        "ratelimit",
        "too many requests",
        "resource_exhausted",
    ]
    .iter()
    .any(|needle| message.contains(needle))
        || message
            .match_indices("429")
            .any(|(index, _)| !digit_at(index.checked_sub(1)) && !digit_at(Some(index + 3)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::middleware::jwt::{ApiKey, ConnectedAppUser, OpenIDUser, PATUser};
    use axum::http::StatusCode;
    use futures_util::StreamExt;
    use sea_orm::{
        ActiveValue::Set, ConnectOptions, Database, DatabaseConnection, Schema, sea_query::Table,
    };

    fn executor(app_id: &str, owner: Option<&str>, app_chain: Option<Vec<String>>) -> AppUser {
        AppUser::Executor(ExecutorUser {
            sub: "runner".into(),
            app_id: app_id.into(),
            run_id: "run-1".into(),
            technical_user_id: None,
            app_chain,
            correlation: None,
            shadow: false,
            hosted_frontend: false,
            event_id: None,
            custom_bits_owner: owner.map(ToOwned::to_owned),
            unattended: false,
        })
    }

    #[test]
    fn only_an_executor_of_the_app_with_an_account_owner_may_call() {
        let (executor_user, owner) =
            authorize_call(&executor("app-1", Some("owner-1"), None), "app-1").unwrap();
        assert_eq!(
            (executor_user.run_id.as_str(), owner.as_str()),
            ("run-1", "owner-1")
        );
        let chained = executor("app-1", Some("owner-1"), Some(vec!["app-0".into()]));
        assert_eq!(authorize_call(&chained, "app-1").unwrap().1, "owner-1");

        let refused = [
            AppUser::OpenID(OpenIDUser {
                sub: "owner-1".into(),
                access_token: "token".into(),
            }),
            AppUser::PAT(PATUser {
                pat: "pat".into(),
                sub: "owner-1".into(),
            }),
            AppUser::APIKey(ApiKey {
                key_id: "key-1".into(),
                api_key: "secret".into(),
                app_id: "app-1".into(),
                creator_user_id: Some("owner-1".into()),
            }),
            AppUser::ConnectedApp(ConnectedAppUser {
                sub: Some("owner-1".into()),
                origin_app_id: "app-0".into(),
                target_app_id: "app-1".into(),
                app_chain: vec!["app-0".into()],
                technical_user_id: None,
                run_id: Some("run-0".into()),
                correlation: None,
            }),
            AppUser::Unauthorized,
            executor("app-2", Some("owner-1"), None),
            executor("app-1", None, None),
            executor("app-1", Some(""), None),
            executor("app-1", Some("frontend:event-1:run-run-1"), None),
            executor("app-1", Some("sink:sink-1"), None),
            executor("app-1", Some("regression:suite-1"), None),
        ];
        for user in refused {
            let status = authorize_call(&user, "app-1")
                .err()
                .map(|error| error.status());
            assert_eq!(status, Some(StatusCode::FORBIDDEN), "{user:?}");
        }
    }

    /// The run a person triggered that acts as the owner of its private models: a signed-in
    /// hosted visitor, or a person an app connection passed through. A member's own run needs
    /// no such standing, and a run nobody triggered, an API key's run, an anonymous visit that
    /// borrows the run-as user's models, or a run whose models are not its subject's never
    /// counts as one.
    #[test]
    fn only_a_run_that_acts_as_the_models_owner_is_a_person_run() {
        let run = |sub: &str| ExecutorUser {
            sub: sub.into(),
            app_id: "app-1".into(),
            run_id: "run-1".into(),
            technical_user_id: None,
            app_chain: None,
            correlation: None,
            shadow: false,
            hosted_frontend: false,
            event_id: Some("event-1".into()),
            custom_bits_owner: Some(sub.into()),
            unattended: false,
        };
        let hosted = |sub: &str| ExecutorUser {
            hosted_frontend: true,
            ..run(sub)
        };
        let chained = |sub: &str| ExecutorUser {
            app_chain: Some(vec!["app-0".into(), "app-9".into()]),
            ..run(sub)
        };
        let connection = Some(PersonRun::Connection {
            origin_app_id: "app-9".into(),
        });
        for (case, executor, owner, expected) in [
            ("member in the app", run("person"), "person", None),
            (
                "signed-in visitor",
                hosted("visitor"),
                "visitor",
                Some(PersonRun::HostedVisitor),
            ),
            (
                "anonymous visitor on the run-as user's models",
                hosted("frontend:event-1:anonymous-session"),
                "run-as",
                None,
            ),
            (
                "person through a connection",
                chained("person"),
                "person",
                connection.clone(),
            ),
            (
                "run nobody triggered",
                ExecutorUser {
                    unattended: true,
                    ..run("run-as")
                },
                "run-as",
                None,
            ),
            (
                "unattended run through a connection",
                ExecutorUser {
                    unattended: true,
                    ..chained("run-as")
                },
                "run-as",
                None,
            ),
            (
                "API key run",
                ExecutorUser {
                    technical_user_id: Some("key-1".into()),
                    ..chained("creator")
                },
                "creator",
                None,
            ),
            ("models of someone else", hosted("visitor"), "other", None),
        ] {
            assert_eq!(PersonRun::of(&executor, owner), expected, "{case}");
        }
    }

    /// A member may use their private models; a person who is not a member may only while the
    /// run acts as them and they still stand in the app. An inactive app or account, and any
    /// other refusal, refuse everyone; a fault on the server stays a fault.
    #[test]
    fn non_members_use_their_models_only_while_they_stand_in_the_app() {
        use RunAsDenial::{AccountInactive, AppInactive, CannotTrigger, NotMember};
        let allowed = RunAsVerdict::Allowed(RolePermissions::ExecuteEvents);
        for (case, verdict, person_stands, expected) in [
            ("member", allowed, false, None),
            ("member who also stands", allowed, true, None),
            (
                "non-member who stands",
                RunAsVerdict::Denied(NotMember),
                true,
                None,
            ),
            (
                "non-member who no longer stands",
                RunAsVerdict::Denied(NotMember),
                false,
                Some(StatusCode::FORBIDDEN),
            ),
            (
                "inactive account",
                RunAsVerdict::Denied(AccountInactive),
                true,
                Some(StatusCode::FORBIDDEN),
            ),
            (
                "inactive app",
                RunAsVerdict::Denied(AppInactive),
                true,
                Some(StatusCode::FORBIDDEN),
            ),
            (
                "role refused",
                RunAsVerdict::Denied(CannotTrigger),
                true,
                Some(StatusCode::FORBIDDEN),
            ),
            (
                "fault",
                RunAsVerdict::Transient("Invalid role permission bits"),
                true,
                Some(StatusCode::INTERNAL_SERVER_ERROR),
            ),
        ] {
            assert_eq!(
                owner_verdict(verdict, person_stands)
                    .err()
                    .map(|error| error.status()),
                expected,
                "{case}"
            );
        }
    }

    /// A queued call keeps how its owner stands, so the hosted worker repeats the same check,
    /// and a call queued before this field existed reads as a member's.
    #[test]
    fn a_queued_call_keeps_how_its_owner_stands() {
        let call = |person: Option<PersonRun>| RelayCall {
            owner: "person".into(),
            run_id: "run-1".into(),
            bit_id: "person-gpt".into(),
            systemone: false,
            body: json!({"model": "person-gpt"}),
            stream: false,
            context: UsageRequestContext {
                app_id: Some("app-1".into()),
                user_id: "person".into(),
                technical_user_id: None,
            },
            person,
        };
        for person in [
            None,
            Some(PersonRun::HostedVisitor),
            Some(PersonRun::Connection {
                origin_app_id: "app-0".into(),
            }),
        ] {
            let queued = serde_json::to_value(call(person.clone())).unwrap();
            assert_eq!(queued.get("person").is_some(), person.is_some());
            let restored: RelayCall = serde_json::from_value(queued).unwrap();
            assert_eq!(restored.person, person);
        }
    }

    /// The relay and the hosted worker both judge the owner with the run's person standing.
    #[test]
    fn every_owner_check_carries_the_runs_person_standing() {
        let source = include_str!("models.rs");
        let body = |signature: &str| {
            let body = source
                .split_once(signature)
                .unwrap_or_else(|| panic!("{signature} exists"))
                .1;
            body.split_once("\n}\n").map_or(body, |(body, _)| body)
        };
        let relay = body("\nasync fn relay(");
        let person = relay.find("PersonRun::of(&executor, &owner)").unwrap();
        let live = relay.find("person.as_ref(),").unwrap();
        assert!(person < live && live < relay.find("relayed_bit(").unwrap());
        assert!(relay.contains("            person,\n"));
        assert!(body("\nasync fn queued_request(").contains("call.person.as_ref(),"));
        assert!(
            body("\nasync fn live_call(")
                .contains("ensure_owner_may_use(&state.db, app_id, owner, person)")
        );
    }

    /// Id, owner, type, provider and API surface.
    const LIBRARY: [(&str, &str, BitType, &str, Option<&str>); 9] = [
        ("alice-gpt", "alice", BitType::Llm, "custom:openai", None),
        ("alice-vision", "alice", BitType::Vlm, "Custom:Gemini", None),
        (
            "alice-decide",
            "alice",
            BitType::SystemOne,
            "custom:typesafe",
            None,
        ),
        ("alice-old", "alice", BitType::Llm, "custom:groq", None),
        (
            "alice-chat-decide",
            "alice",
            BitType::Llm,
            "custom:openrouter",
            Some("SystemOne"),
        ),
        ("alice-codex", "alice", BitType::Llm, "custom:codex", None),
        ("alice-local", "alice", BitType::Llm, "Local", None),
        (
            "alice-embed",
            "alice",
            BitType::Embedding,
            "custom:openai",
            None,
        ),
        ("bob-claude", "bob", BitType::Llm, "custom:anthropic", None),
    ];

    const NOT_FOUND: (StatusCode, &str) = (StatusCode::NOT_FOUND, "no private model");
    const NOT_CHAT: (StatusCode, &str) = (StatusCode::UNPROCESSABLE_ENTITY, "not a chat model");
    const DESKTOP_ONLY: (StatusCode, &str) =
        (StatusCode::UNPROCESSABLE_ENTITY, "runs on desktop only");

    /// Bit id, route and the refusal alice's run gets.
    const REFUSALS: [(&str, bool, (StatusCode, &str)); 8] = [
        ("bob-claude", false, NOT_FOUND),
        ("alice-old", false, NOT_FOUND),
        ("alice-decide", false, NOT_CHAT),
        ("alice-chat-decide", false, NOT_CHAT),
        ("alice-embed", false, NOT_CHAT),
        (
            "alice-gpt",
            true,
            (StatusCode::UNPROCESSABLE_ENTITY, "not a SystemOne model"),
        ),
        ("alice-codex", false, DESKTOP_ONLY),
        ("alice-local", false, DESKTOP_ONLY),
    ];

    async fn user_bit_table() -> DatabaseConnection {
        let mut options = ConnectOptions::new("sqlite::memory:");
        options.max_connections(1);
        let db = Database::connect(options).await.unwrap();
        db.execute_unprepared("ATTACH DATABASE ':memory:' AS public")
            .await
            .unwrap();
        let source =
            Schema::new(db.get_database_backend()).create_table_from_entity(user_bit::Entity);
        let mut table = Table::create();
        table.table(source.get_table_name().unwrap().clone());
        for column in source.get_columns() {
            table.col(column.clone());
        }
        db.execute(&table).await.unwrap();
        db
    }

    async fn library() -> DatabaseConnection {
        let db = user_bit_table().await;
        let now = Utc::now().fixed_offset();
        for (id, owner, bit_type, provider, surface) in LIBRARY {
            user_bit::Entity::insert(user_bit::ActiveModel {
                id: Set(id.into()),
                user_id: Set(owner.into()),
                r#type: Set(bit_type),
                parameters: Set(Some(json!({"provider": {
                    "provider_name": provider,
                    "model_id": "owner/model",
                    "api_surface": surface,
                    "params": {"model_id": "owner/model"},
                }}))),
                created_at: Set(now),
                updated_at: Set(now),
                ..Default::default()
            })
            .exec(&db)
            .await
            .unwrap();
        }
        user_bit::Entity::delete_by_id("alice-old")
            .exec(&db)
            .await
            .unwrap();
        db
    }

    #[tokio::test]
    async fn the_owners_bits_are_served_on_their_own_route() {
        let db = library().await;
        for (bit, systemone) in [
            ("alice-gpt", false),
            ("alice-vision", false),
            ("alice-decide", true),
        ] {
            let (row, _) = relayed_bit(&db, "alice", bit, systemone).await.unwrap();
            assert_eq!(row.id, bit);
        }
    }

    #[tokio::test]
    async fn another_owners_a_deleted_or_an_unfit_bit_is_refused() {
        let db = library().await;
        for (bit, systemone, (status, message)) in REFUSALS {
            let error = relayed_bit(&db, "alice", bit, systemone).await.unwrap_err();
            assert_eq!(error.status(), status, "{bit}");
            let public = error.public_message().unwrap_or_default();
            assert!(public.contains(message), "{bit}: {public}");
        }
    }

    #[test]
    fn errors_lose_every_url_and_the_providers_credentials() {
        let key = "AIzaSyD-relay-test-key-0123456789";
        let header_secret = "hk-relay-header-secret";
        let token = "ya29.relay+access/token";
        let private_key = "-----BEGIN PRIVATE KEY-----\nMIIEvQrelay\n-----END PRIVATE KEY-----\n";
        let params: HashMap<String, Value> = serde_json::from_value(json!({
            "api_key": key,
            "access_token": token,
            "headers": {"x-api-key": header_secret},
            "service_account_json": json!({
                "type": "service_account",
                "private_key": private_key,
            })
            .to_string(),
            "endpoint": "https://models.example.com/v1",
            "model_id": "gemini-2.5-pro",
        }))
        .unwrap();
        let secrets = secret_values(Some(&params));
        assert!(!secrets.iter().any(|secret| secret == "gemini-2.5-pro"));

        let message = format!(
            "Rig streaming error: HttpError: error sending request for url (https://generativelanguage.googleapis.com/v1beta/models/gemini:streamGenerateContent?alt=sse&key={key}) | Details: Request {{ url: \"https://proxy.example/v1?key={key}\" }}; redirected to https://[fd00::1]:8443/v1?sig=signature-1 then provider echoed the literal api_key {key}, header {header_secret}, token {}, key {}",
            urlencoding::encode(token),
            serde_json::to_string(private_key).unwrap()
        );
        let scrubbed = scrub(&message, &secrets);
        for leaked in [
            key,
            header_secret,
            "ya29",
            "?key=",
            "://",
            "googleapis",
            "signature-1",
            "BEGIN PRIVATE KEY",
        ] {
            assert!(!scrubbed.contains(leaked), "{leaked} survived: {scrubbed}");
        }
        for kept in [
            "error sending request for url ([url])",
            "url: \"[url]\"",
            "redirected to [url] then",
            "the literal api_key [REDACTED], header [REDACTED]",
        ] {
            assert!(scrubbed.contains(kept), "{kept} is missing: {scrubbed}");
        }
    }

    #[test]
    fn usage_names_the_endpoint_host_only_on_the_owners_row() {
        assert_eq!(
            endpoint_host("https://user:pass@models.example.com:8443/v1?key=secret").as_deref(),
            Some("models.example.com")
        );
        assert_eq!(endpoint_host("not a url"), None);
        let provider: ModelProvider = serde_json::from_value(json!({
            "provider_name": "custom:openai",
            "model_id": "gpt",
            "version": null,
            "params": {"endpoint": "  https://models.example.com/v1  "},
        }))
        .unwrap();
        let endpoint = endpoint_param(&provider);
        assert_eq!(
            owners_endpoint_host(endpoint, "owner-1", "owner-1").as_deref(),
            Some("models.example.com")
        );
        for subject in [
            "visitor-1",
            "frontend:event-1:anonymous-session-1",
            "regression:suite-1",
            "",
        ] {
            assert_eq!(
                owners_endpoint_host(endpoint, subject, "owner-1"),
                None,
                "{subject}"
            );
        }
        assert_eq!(owners_endpoint_host(None, "owner-1", "owner-1"), None);
    }

    #[test]
    fn a_queued_call_checks_its_run_and_owner_again_before_the_bit_is_loaded() {
        let source = include_str!("models.rs");
        let order = |signature: &str, steps: [&str; 2]| {
            let body = source
                .split_once(signature)
                .unwrap_or_else(|| panic!("{signature} exists"))
                .1;
            let body = body.split_once("\n}\n").map_or(body, |(body, _)| body);
            let [first, then] = steps.map(|step| {
                body.find(step)
                    .unwrap_or_else(|| panic!("{signature} calls {step}"))
            });
            assert!(first < then, "{signature} calls {steps:?} out of order");
            body
        };
        order("\nasync fn relay(", ["live_call(", "relayed_bit("]);
        order(
            "\nasync fn relay(",
            ["executor.technical_user_id.as_deref(),", "relayed_bit("],
        );
        order("\nasync fn queued_request(", ["live_call(", "relayed_bit("]);
        order(
            "\nasync fn queued_request(",
            ["call.context.technical_user_id.as_deref(),", "relayed_bit("],
        );
        order(
            "\nasync fn live_call(",
            ["ensure_owner_may_use(", "ensure_key_may_call("],
        );
        let worker = order(
            "\npub(crate) async fn run_queued(",
            ["queued_request(", "run_call("],
        );
        assert!(!worker.contains("relayed_bit(") && !worker.contains("prepare("));
    }

    /// A run started with an API key calls its creator's models only while the key exists,
    /// holds a role and is unexpired, like its other callbacks; other runs have no key.
    #[tokio::test]
    async fn an_api_key_run_calls_private_models_only_while_its_key_is_current() {
        let db = crate::middleware::jwt::test_support::api_keys().await;
        for key in [None, Some("current"), Some("unexpired")] {
            assert!(
                ensure_key_may_call(&db, "app-1", "run-1", key)
                    .await
                    .is_ok(),
                "{key:?}"
            );
        }
        for (app, key) in [
            ("app-1", "deleted"),
            ("app-1", "expired"),
            ("app-1", "roleless"),
            ("app-2", "current"),
        ] {
            let refusal = ensure_key_may_call(&db, app, "run-1", Some(key))
                .await
                .unwrap_err();
            assert_eq!(refusal.status(), StatusCode::FORBIDDEN, "{key} in {app}");
        }
    }

    #[test]
    fn tool_continuations_keep_thinking_only_without_signatures() {
        use HistoryThinking::{High, Off};
        for (provider, expected) in [
            ("custom:anthropic", Some(Off)),
            ("custom:bedrock", Some(Off)),
            ("custom:gemini", Some(Off)),
            ("custom:vertex", Some(Off)),
            ("custom:openrouter", Some(Off)),
            ("custom:openai", Some(High)),
            ("custom:deepseek", Some(High)),
            ("custom:groq", Some(High)),
        ] {
            assert_eq!(
                relayed_thinking(provider, Some(High), Some(Off)),
                expected,
                "{provider}"
            );
            assert_eq!(
                relayed_thinking(provider, Some(High), Some(High)),
                Some(High)
            );
            assert_eq!(relayed_thinking(provider, None, None), None);
        }
    }

    #[test]
    fn requested_thinking_survives_parsing_for_the_relay_to_restore() {
        let body = json!({
            "model": "alice-gpt",
            "thinking": "high",
            "messages": [
                {"role": "user", "content": "Weather?"},
                {"role": "assistant", "tool_calls": [{
                    "id": "call-1", "type": "function",
                    "function": {"name": "weather", "arguments": "{}"},
                }]},
                {"role": "tool", "tool_call_id": "call-1", "content": "Sunny"},
            ],
        });
        let RelayRequest::Chat {
            history,
            requested_thinking,
            ..
        } = RelayRequest::parse(&body, false).unwrap()
        else {
            panic!("a chat request");
        };
        assert_eq!(history.thinking, Some(HistoryThinking::Off));
        assert_eq!(requested_thinking, Some(HistoryThinking::High));
        assert_eq!(
            relayed_thinking("custom:openai", requested_thinking, history.thinking),
            Some(HistoryThinking::High)
        );
    }

    #[test]
    fn requests_are_checked_before_any_bit_is_loaded() {
        let invalid = [
            (json!({"model": "bit", "messages": []}), false),
            (
                json!({"model": "bit", "messages": [{"role": "user", "content": [
                    {"type": "image_url", "image_url": {"url": "file:///etc/passwd"}}
                ]}]}),
                false,
            ),
            (
                json!({"model": "bit", "messages": [{"role": "user", "content": [
                    {"type": "document_url", "document_url": "file_id:file-123"}
                ]}]}),
                false,
            ),
            (
                json!({"model": "bit", "state": "text", "questions": {}, "stream": true}),
                true,
            ),
        ];
        for (body, systemone) in invalid {
            let error = RelayRequest::parse(&body, systemone).err().unwrap();
            assert_eq!(error.status(), StatusCode::BAD_REQUEST, "{body}");
        }
        let decision = json!({"model": "alice-decide", "state": {"text": "charged twice"},
            "questions": {"refund": {"type": "noul", "instructions": "Refund?"}}});
        let request = RelayRequest::parse(&decision, true).unwrap();
        assert_eq!(
            (request.bit_id(), request.stream()),
            ("alice-decide", false)
        );
    }

    #[test]
    fn a_call_ends_before_its_run_and_its_route() {
        let now = DateTime::from_timestamp(1_000_000, 0).unwrap();
        let budget = Duration::from_secs(870);
        let by_budget = now + chrono::Duration::seconds(865);
        assert_eq!(call_deadline(now, None, budget), by_budget);
        let soon = now + chrono::Duration::seconds(60);
        assert_eq!(
            call_deadline(now, Some(soon.timestamp_millis()), budget),
            soon
        );
        let late = now + chrono::Duration::hours(2);
        assert_eq!(
            call_deadline(now, Some(late.timestamp_millis()), budget),
            by_budget
        );
    }

    #[test]
    fn rate_limits_are_told_apart_from_other_provider_failures() {
        for message in [
            "HTTP status client error (429 Too Many Requests) for url ([url])",
            "Rate limit reached for gpt-4o",
            "RESOURCE_EXHAUSTED: quota exceeded",
            "status: 429",
        ] {
            assert!(mentions_rate_limit(message), "{message}");
        }
        for message in [
            "invalid api key",
            "context has 4290 tokens",
            "HTTP status server error (500 Internal Server Error)",
        ] {
            assert!(!mentions_rate_limit(message), "{message}");
        }
    }

    fn failure(kind: FailureKind) -> Failure {
        Failure {
            kind,
            message: "upstream broke".into(),
        }
    }

    async fn next(body: &mut axum::body::BodyDataStream) -> Option<Bytes> {
        body.next().await.map(Result::unwrap)
    }

    #[tokio::test(start_paused = true)]
    async fn a_silent_stream_commits_after_the_hold_and_keeps_alive() {
        let (events, receiver) = mpsc::channel(4);
        let started = Instant::now();
        let response = respond(receiver, true).await;
        assert_eq!(started.elapsed(), HEADER_HOLD);
        assert!(HEADER_HOLD <= Duration::from_secs(10));
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response.headers()[header::CONTENT_TYPE],
            "text/event-stream"
        );
        let mut body = response.into_body().into_data_stream();
        let mut previous = Instant::now();
        for _ in 0..3 {
            assert_eq!(next(&mut body).await.unwrap(), SSE_KEEPALIVE);
            assert!(previous.elapsed() <= Duration::from_secs(15));
            previous = Instant::now();
        }
        events
            .send(Event::Chunk(json!({"id": "chunk-1", "choices": []})))
            .await
            .unwrap();
        let frame = next(&mut body).await.unwrap();
        let frame = std::str::from_utf8(&frame).unwrap();
        let data: Value = serde_json::from_str(
            frame
                .strip_prefix("data: ")
                .and_then(|frame| frame.strip_suffix("\n\n"))
                .unwrap(),
        )
        .unwrap();
        assert_eq!(data["id"], "chunk-1");
        events.send(Event::Done(Ok(json!({})))).await.unwrap();
        assert_eq!(next(&mut body).await.unwrap(), SSE_DONE);
        assert!(next(&mut body).await.is_none());
    }

    #[tokio::test(start_paused = true)]
    async fn a_slow_answer_follows_whitespace_keepalives() {
        let (events, receiver) = mpsc::channel(4);
        let started = Instant::now();
        let response = respond(receiver, false).await;
        assert_eq!(started.elapsed(), HEADER_HOLD);
        assert_eq!(response.headers()[header::CONTENT_TYPE], "application/json");
        let mut body = response.into_body().into_data_stream();
        let mut previous = Instant::now();
        let mut received = Vec::new();
        for _ in 0..2 {
            let keepalive = next(&mut body).await.unwrap();
            assert_eq!(keepalive, JSON_KEEPALIVE);
            assert!(previous.elapsed() <= Duration::from_secs(15));
            previous = Instant::now();
            received.extend_from_slice(&keepalive);
        }
        events
            .send(Event::Done(Ok(json!({"object": "chat.completion"}))))
            .await
            .unwrap();
        while let Some(frame) = next(&mut body).await {
            received.extend_from_slice(&frame);
        }
        let answer: Value = serde_json::from_slice(&received).unwrap();
        assert_eq!(answer["object"], "chat.completion");
    }

    #[tokio::test(start_paused = true)]
    async fn early_outcomes_keep_their_status() {
        let (events, receiver) = mpsc::channel(4);
        events
            .send(Event::Done(Err(failure(FailureKind::RateLimited))))
            .await
            .unwrap();
        let response = respond(receiver, true).await;
        assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(response.headers()[header::RETRY_AFTER], "30");

        for (kind, status) in [
            (FailureKind::Provider, StatusCode::BAD_GATEWAY),
            (FailureKind::Timeout, StatusCode::GATEWAY_TIMEOUT),
        ] {
            let (events, receiver) = mpsc::channel(4);
            events.send(Event::Done(Err(failure(kind)))).await.unwrap();
            assert_eq!(respond(receiver, false).await.status(), status);
        }

        let (events, receiver) = mpsc::channel(4);
        events
            .send(Event::Done(Ok(json!({"object": "chat.completion"}))))
            .await
            .unwrap();
        let started = Instant::now();
        let response = respond(receiver, false).await;
        assert_eq!(started.elapsed(), Duration::ZERO);
        assert_eq!(response.status(), StatusCode::OK);
        let answer = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        assert_eq!(
            serde_json::from_slice::<Value>(&answer).unwrap()["object"],
            "chat.completion"
        );

        let (events, receiver) = mpsc::channel::<Event>(4);
        drop(events);
        assert_eq!(
            respond(receiver, true).await.status(),
            StatusCode::BAD_GATEWAY
        );
    }

    #[tokio::test(start_paused = true)]
    async fn late_failures_arrive_in_band_and_a_lost_call_cuts_the_body() {
        let (events, receiver) = mpsc::channel(4);
        events
            .send(Event::Chunk(json!({"id": "chunk-1", "choices": []})))
            .await
            .unwrap();
        let started = Instant::now();
        let response = respond(receiver, true).await;
        assert_eq!(started.elapsed(), Duration::ZERO);
        let mut body = response.into_body().into_data_stream();
        assert!(next(&mut body).await.unwrap().starts_with(b"data: {"));
        events
            .send(Event::Done(Err(failure(FailureKind::Provider))))
            .await
            .unwrap();
        let frame = next(&mut body).await.unwrap();
        let frame = std::str::from_utf8(&frame).unwrap();
        let (error, done) = frame.split_once("\n\n").unwrap();
        let error: Value = serde_json::from_str(error.strip_prefix("data: ").unwrap()).unwrap();
        assert_eq!(error["error"]["type"], "provider_error");
        assert_eq!(error["error"]["message"], "upstream broke");
        assert_eq!(done, "data: [DONE]\n\n");
        assert!(next(&mut body).await.is_none());

        let (events, receiver) = mpsc::channel(4);
        let response = respond(receiver, false).await;
        let mut body = response.into_body().into_data_stream();
        assert_eq!(next(&mut body).await.unwrap(), JSON_KEEPALIVE);
        events
            .send(Event::Done(Err(failure(FailureKind::Timeout))))
            .await
            .unwrap();
        let error: Value = serde_json::from_slice(&next(&mut body).await.unwrap()).unwrap();
        assert_eq!(error["error"]["type"], "timeout");

        let (events, receiver) = mpsc::channel::<Event>(4);
        let response = respond(receiver, true).await;
        drop(events);
        let mut body = response.into_body().into_data_stream();
        assert!(next(&mut body).await.is_none());
    }
}
