//! What a device receives when it claims a bot turn: the turn's credentials, the bot's run spec
//! and the hub-resolved models. It resolves the turn like the server's worker does, except that
//! only hosted models leave the server (a customer provider's Bit carries the owner's
//! credentials), and the model allowance is reserved here and settled when the device ends the
//! turn. The device pays no runtime compute: it computes.

use crate::{
    bot_host::{
        history,
        ledger::{BOT_TURN_LEASE_LOST, Claimed},
        recovery,
        worker::{self, MAX_TURN_RUNTIME_MS},
    },
    bot_permission::{SignedBotToken, sign_bot_token},
    entity::{bot, bot_turn, conversation},
    error::ApiError,
    routes::{
        ai::{
            copilot::master_flow_like_state,
            global_chat::{
                AssistantUsage, assistant_rate, reserve_assistant_usage_with_max_runtime,
                settle_assistant_usage,
            },
        },
        bots::{
            owned_bot,
            runtime::{ChatBot, chat_bot, hosted_bot_model},
            runtime_token::bot_user_for_turn,
        },
        inbox::active_membership,
    },
    state::AppState,
};
use axum::http::StatusCode;
use flow_like::{
    bit::BitModelPreference, flow::copilot::runnable_completion_model,
    flow_like_model_provider::provider::is_hosted_provider_name, profile::Profile,
    state::FlowLikeState,
};
use flow_like_types::{
    bot_turn::{STORED_RUNNING, TurnAudience},
    device_bot::{
        DeviceBotSpec, DeviceConversation, DeviceModels, DeviceRequester, DeviceToken,
        DeviceTurnGrant, DeviceUsageGrant, DeviceUsageReport, REFRESHED_TOKEN_SECS,
    },
    inbox::{ConversationEncryption, ConversationKind},
};
use sea_orm::{ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter, sea_query::Expr};
use std::sync::Arc;

pub(crate) const BOT_TURN_ENCRYPTED: &str = worker::CODE_ENCRYPTED;

pub(crate) const BOT_DEVICE_MODEL_NOT_HOSTED: &str = "BOT_DEVICE_MODEL_NOT_HOSTED";
pub(crate) const MODEL_NOT_HOSTED_TEXT: &str =
    "This bot's model can't run on a device; pick a hosted model in its settings.";
pub(crate) const DELIVERY_TTL_SECS: i64 = 60;
/// Posted after "I can't answer yet: " when the bot's device is shared for deploying.
pub(crate) const DEVICE_SHARED_TEXT: &str = "the device I run on lets other people deploy to it. My owner can end that access in the device's sharing settings or choose another place for me to run.";
/// How long past the end of its allowance a device may keep a turn to hand its answer in.
pub(crate) const HOLD_GRACE_SECS: i64 = 120;

/// The unix second after which a device may no longer keep the turn alive: the end of the
/// allowance recorded for its claim, plus the time to hand the answer in. `None` when the hub
/// recorded no allowance for the turn. An allowance that cannot be found has ended.
pub(crate) async fn hold_end<C: ConnectionTrait>(
    db: &C,
    turn: &bot_turn::Model,
) -> Result<Option<i64>, ApiError> {
    let Some(operation_id) = turn.operation_id.as_deref() else {
        return Ok(None);
    };
    let window = crate::quota::assistant_window(db, operation_id, &turn.owner_id).await?;
    Ok(Some(window.map_or(0, |(deadline_ms, _)| {
        deadline_ms / 1_000 + HOLD_GRACE_SECS
    })))
}

/// Refuses to keep a turn alive past the end of its allowance. A turn whose answer is saved
/// stays: only publishing it is left.
pub(crate) async fn require_hold<C: ConnectionTrait>(
    db: &C,
    turn: &bot_turn::Model,
    host_id: &str,
) -> Result<(), ApiError> {
    let Some(end) = hold_end(db, turn).await? else {
        return Ok(());
    };
    if chrono::Utc::now().timestamp() <= end || has_finished_checkpoint(db, turn, host_id).await? {
        return Ok(());
    }
    Err(ApiError::coded(
        StatusCode::CONFLICT,
        BOT_TURN_LEASE_LOST,
        format!(
            "Turn '{}' ran past the end of its allowance; host '{host_id}' can't keep it",
            turn.id
        ),
    ))
}

/// How long a token refreshed for a running turn lives: never past the end of its hold.
pub(crate) fn refresh_ttl(hold_end: Option<i64>, now: i64) -> i64 {
    hold_end.map_or(REFRESHED_TOKEN_SECS, |end| {
        (end - now).clamp(DELIVERY_TTL_SECS, REFRESHED_TOKEN_SECS)
    })
}

pub(crate) async fn has_finished_checkpoint<C: ConnectionTrait>(
    db: &C,
    turn: &crate::entity::bot_turn::Model,
    host_id: &str,
) -> Result<bool, ApiError> {
    let checkpoint = recovery::load_in(
        db,
        &recovery::Claim {
            turn_id: turn.id.clone(),
            owner_id: turn.owner_id.clone(),
            host_id: host_id.to_owned(),
            attempt: u32::try_from(turn.attempts).unwrap_or(0),
        },
        crate::routes::inbox::now_millis(),
    )
    .await?;
    match checkpoint {
        Some(checkpoint) => flow_like::flow::copilot::run_recovery::finished_reply(&checkpoint)
            .map(|answer| answer.is_some())
            .map_err(ApiError::internal_error),
        None => Ok(false),
    }
}

/// Delivering a persisted answer needs current inbox access, but no model or usage allowance.
pub(crate) async fn completed_delivery_in<C: ConnectionTrait>(
    db: &C,
    claimed: &Claimed,
    host_id: &str,
    row: &bot::Model,
    conversation: &conversation::Model,
) -> Result<Option<DeviceTurnGrant>, ApiError> {
    if is_end_to_end(conversation) || !has_finished_checkpoint(db, &claimed.turn, host_id).await? {
        return Ok(None);
    }
    let turn = &claimed.turn;
    owned_bot(db, &turn.owner_id, &turn.bot_id).await?;
    active_membership(db, &conversation.id, &turn.bot_id).await?;
    let kind = ConversationKind::parse(&conversation.kind).unwrap_or(ConversationKind::Group);
    let bot_token = bot_token(
        db,
        &turn.owner_id,
        &turn.bot_id,
        &turn.id,
        DELIVERY_TTL_SECS,
    )
    .await?;
    // This is a delivery lease, separate from the immutable execution checkpoint deadline.
    let deadline = bot_token.expires_at;
    Ok(Some(DeviceTurnGrant {
        turn: crate::bot_host::ledger::turn_json(turn, claimed.batched.clone()),
        host_id: host_id.to_owned(),
        conversation: DeviceConversation {
            id: conversation.id.clone(),
            kind: kind.as_str().into(),
            title: conversation.title.clone(),
        },
        bot: DeviceBotSpec {
            bot_id: turn.bot_id.clone(),
            system: None,
            name: row.name.clone().unwrap_or_else(|| "Bot".into()),
            description: None,
            instructions: None,
            capabilities: Vec::new(),
            classification: Default::default(),
            selected_app_ids: Default::default(),
            profile_app_ids: Default::default(),
            workspace_app_id: None,
            embedding_model: None,
            stt_model: None,
            tts_model: None,
            image_model: None,
            vision_model: None,
            subtask_model: None,
            skills: Vec::new(),
            connectors: Vec::new(),
            model_policy: Default::default(),
        },
        requester: Some(DeviceRequester {
            requester_id: turn
                .requester_id
                .clone()
                .unwrap_or_else(|| turn.owner_id.clone()),
            requester_name: None,
            owner_id: turn.owner_id.clone(),
            owner_name: None,
            audience: TurnAudience::of(kind),
            room: conversation.title.clone(),
        }),
        models: DeviceModels {
            model_id: String::new(),
            completion: serde_json::Value::Null,
            fallbacks: Vec::new(),
            embedding: None,
            embedding_model_id: None,
            decision: None,
        },
        bot_token,
        executor_token: None,
        usage: None,
        deadline,
        delivery_only: true,
    }))
}

fn unrunnable(what: &str) -> ApiError {
    ApiError::coded(
        StatusCode::BAD_REQUEST,
        BOT_DEVICE_MODEL_NOT_HOSTED,
        format!("A device bot runs on hosted models only; {what} is not a hosted model"),
    )
}

async fn hosted_bit(
    fls: &Arc<FlowLikeState>,
    profile: &Profile,
    id: &str,
) -> Option<flow_like::bit::Bit> {
    let bit = runnable_completion_model(fls, profile, id).await?;
    let provider = bit.try_to_provider()?;
    is_hosted_provider_name(&provider.provider_name).then_some(bit)
}

async fn embedding_bit(
    fls: &Arc<FlowLikeState>,
    profile: &Profile,
    id: &str,
) -> Result<Option<flow_like::bit::Bit>, ApiError> {
    let bit = profile
        .find_bit(id, fls.http_client.clone())
        .await
        .map_err(|error| ApiError::bad_request(format!("Memory model '{id}': {error}")))?;
    let embedding = bit.try_to_embedding().ok_or_else(|| {
        ApiError::bad_request(format!("Memory model '{id}' is not an embedding model"))
    })?;
    let hosted = embedding
        .remote
        .as_ref()
        .is_some_and(|remote| remote.implementation.is_some())
        || is_hosted_provider_name(&embedding.provider.provider_name);
    if !hosted {
        return Err(unrunnable(&format!("memory model '{id}'")));
    }
    Ok(Some(bit))
}

/// An optional classifier becoming unavailable must not disable the bot's primary model.
async fn decision_bit(
    profile: &Profile,
    id: &str,
    http_client: Arc<flow_like::utils::http::HTTPClient>,
) -> Option<serde_json::Value> {
    let bit = match profile.find_bit(id, http_client).await {
        Ok(bit) => bit,
        Err(error) => {
            tracing::warn!(bit_id = id, %error, "Device bot decision model unavailable; using normal response handling");
            return None;
        }
    };
    if !crate::routes::bots::crud::is_hosted_decision_model(&bit) {
        tracing::warn!(
            bit_id = id,
            "Device bot decision model is not a hosted System One Bit; using normal response handling"
        );
        return None;
    }
    match serde_json::to_value(&bit) {
        Ok(value) => Some(value),
        Err(error) => {
            tracing::warn!(bit_id = id, %error, "Device bot decision model could not be encoded; using normal response handling");
            None
        }
    }
}

struct Resolved {
    models: DeviceModels,
    provider_name: String,
    funding: &'static str,
}

async fn resolve_models(
    state: &AppState,
    owner: &str,
    bot: &ChatBot,
    profile: &Arc<Profile>,
    access: &crate::routes::ai::global_chat::ProfileModelAccess,
    fls: &Arc<FlowLikeState>,
) -> Result<Resolved, ApiError> {
    let model_id = hosted_bot_model(state, owner, fls, profile, &bot.model).await;
    if let Some(rejection) = access.rejection(model_id.as_deref()) {
        return Err(rejection);
    }
    let selected = profile
        .resolve_completion_model(
            model_id.as_deref(),
            &BitModelPreference {
                reasoning_weight: Some(1.0),
                ..Default::default()
            },
            false,
            FlowLikeState::completion_model_capabilities(fls).await,
            None,
            fls.http_client.clone(),
        )
        .await
        .map_err(|error| ApiError::bad_request(error.to_string()))?;
    let provider = selected
        .try_to_provider()
        .ok_or_else(|| ApiError::bad_request("The bot's model is not a completion provider"))?;
    if !is_hosted_provider_name(&provider.provider_name) {
        return Err(unrunnable(&format!("'{}'", selected.id)));
    }
    let mut fallbacks = Vec::new();
    for id in bot.model.fallbacks.iter().filter(|id| **id != selected.id) {
        if let Some(bit) = hosted_bit(fls, profile, id).await {
            fallbacks.push(serde_json::to_value(&bit)?);
        }
    }
    let memory_model = match bot.context.memory_model(None) {
        Some(id) => Some(id.to_string()),
        None => worker::stored_memory_model(fls, profile, bot, owner).await,
    };
    let embedding = match memory_model.as_deref() {
        Some(id) => embedding_bit(fls, profile, id).await?,
        None => None,
    };
    let decision = match bot.model.decision.as_deref() {
        Some(id) => decision_bit(profile, id, fls.http_client.clone()).await,
        None => None,
    };
    let funding = worker::funding_class(true, embedding.as_ref().map(|_| true));
    Ok(Resolved {
        provider_name: provider.provider_name.clone(),
        funding,
        models: DeviceModels {
            model_id: selected.id.clone(),
            completion: serde_json::to_value(&selected)?,
            fallbacks,
            embedding: embedding.as_ref().map(serde_json::to_value).transpose()?,
            embedding_model_id: memory_model,
            decision,
        },
    })
}

fn bot_spec(bot: ChatBot) -> DeviceBotSpec {
    let context = bot.context;
    DeviceBotSpec {
        bot_id: context.bot_id,
        system: context.system,
        name: context.name,
        description: context.description,
        instructions: context.instructions,
        capabilities: context.capabilities,
        classification: context.classification,
        selected_app_ids: context.selected_app_ids,
        profile_app_ids: context.profile_app_ids,
        workspace_app_id: context.workspace_app_id,
        embedding_model: context.embedding_model,
        stt_model: context.stt_model,
        tts_model: context.tts_model,
        image_model: context.image_model,
        vision_model: context.vision_model,
        subtask_model: context.subtask_model,
        skills: context.skills,
        connectors: context.connectors,
        model_policy: bot.model,
    }
}

async fn requester_of(
    state: &AppState,
    turn: &crate::entity::bot_turn::Model,
    kind: ConversationKind,
    room: Option<&str>,
) -> Result<DeviceRequester, ApiError> {
    let requester_id = turn
        .requester_id
        .clone()
        .unwrap_or_else(|| turn.owner_id.clone());
    let names = history::sender_names(&state.db, &[&requester_id, &turn.owner_id]).await?;
    Ok(DeviceRequester {
        requester_name: names.get(&requester_id).cloned(),
        owner_name: names.get(&turn.owner_id).cloned(),
        requester_id,
        owner_id: turn.owner_id.clone(),
        audience: TurnAudience::of(kind),
        room: room.map(str::to_string),
    })
}

fn token(signed: SignedBotToken) -> DeviceToken {
    DeviceToken {
        token: signed.token,
        expires_at: signed.expires_at,
    }
}

pub(crate) fn executor_token(
    owner: &str,
    bot_id: &str,
    conversation: Option<&str>,
    turn_id: &str,
    workspace_app_id: Option<&str>,
    ttl_secs: i64,
) -> Result<DeviceToken, ApiError> {
    let token = worker::executor_token(
        owner,
        bot_id,
        conversation,
        turn_id,
        workspace_app_id,
        ttl_secs,
    )
    .ok_or_else(|| {
        ApiError::internal("The executor token for a device turn could not be signed")
    })?;
    Ok(DeviceToken {
        token,
        expires_at: chrono::Utc::now().timestamp() + ttl_secs.clamp(1, 86_400),
    })
}

pub(crate) async fn bot_token<C: sea_orm::ConnectionTrait>(
    db: &C,
    owner: &str,
    bot_id: &str,
    turn_id: &str,
    ttl_secs: i64,
) -> Result<DeviceToken, ApiError> {
    let user = bot_user_for_turn(db, owner, bot_id, turn_id).await?;
    sign_bot_token(&user, Some(ttl_secs))
        .map(token)
        .map_err(|error| ApiError::internal_error(flow_like_types::anyhow!(error)))
}

/// Reserves and starts the model allowance of a device turn.
async fn reserve(
    state: &AppState,
    owner: &str,
    resolved: &Resolved,
    bot: &crate::middleware::jwt::BotUser,
) -> Result<AssistantUsage, ApiError> {
    let usage = reserve_assistant_usage_with_max_runtime(
        state,
        owner,
        &resolved.models.model_id,
        &resolved.provider_name,
        resolved.funding,
        None,
        MAX_TURN_RUNTIME_MS,
        false,
        Some(bot),
    )
    .await?;
    if !crate::quota::mark_started(state, &usage.operation_id).await? {
        return Err(ApiError::payment_required(
            "The bot can't answer right now: your usage allowance isn't available",
        ));
    }
    Ok(usage)
}

pub(crate) async fn conversation_of(
    state: &AppState,
    conversation_id: &str,
) -> Result<conversation::Model, ApiError> {
    conversation::Entity::find_by_id(conversation_id)
        .one(&state.db)
        .await?
        .ok_or_else(|| ApiError::not_found(format!("Conversation '{conversation_id}' is gone")))
}

pub(crate) fn is_end_to_end(conversation: &conversation::Model) -> bool {
    ConversationEncryption::parse(&conversation.encryption) == Some(ConversationEncryption::E2e)
}

/// The model allowance a claim runs on.
struct Allowance {
    operation_id: String,
    deadline: chrono::DateTime<chrono::Utc>,
    /// Set when this claim reserved it.
    reserved: Option<AssistantUsage>,
}

/// A turn keeps the allowance recorded for it for as long as that allowance runs, so claiming
/// it again neither reserves twice nor moves the end of the run. Otherwise a new one is
/// reserved and recorded on the turn, which the device must still hold.
async fn turn_allowance(
    state: &AppState,
    turn: &bot_turn::Model,
    host_id: &str,
    resolved: &Resolved,
    room: Option<&str>,
) -> Result<Allowance, ApiError> {
    let owner = turn.owner_id.as_str();
    if let Some(operation_id) = turn.operation_id.as_deref()
        && let Some((deadline_ms, true)) =
            crate::quota::assistant_window(&state.db, operation_id, owner).await?
        && let Some(deadline) = chrono::DateTime::from_timestamp_millis(deadline_ms)
        && deadline > chrono::Utc::now()
    {
        return Ok(Allowance {
            operation_id: operation_id.to_owned(),
            deadline,
            reserved: None,
        });
    }
    let usage = reserve(
        state,
        owner,
        resolved,
        &crate::middleware::jwt::BotUser {
            bot_id: turn.bot_id.clone(),
            turn_id: turn.id.clone(),
            conv_id: room.map(ToOwned::to_owned),
        },
    )
    .await?;
    let recorded = bot_turn::Entity::update_many()
        .col_expr(
            bot_turn::Column::OperationId,
            Expr::value(usage.operation_id.clone()),
        )
        .filter(bot_turn::Column::Id.eq(turn.id.as_str()))
        .filter(bot_turn::Column::HostId.eq(host_id))
        .filter(bot_turn::Column::Status.eq(STORED_RUNNING))
        .exec(&state.db)
        .await;
    match recorded {
        Ok(updated) if updated.rows_affected == 1 => Ok(Allowance {
            operation_id: usage.operation_id.clone(),
            deadline: usage.deadline,
            reserved: Some(usage),
        }),
        outcome => {
            settle_assistant_usage(state, &usage, 0, false).await.ok();
            outcome?;
            Err(ApiError::coded(
                StatusCode::CONFLICT,
                BOT_TURN_LEASE_LOST,
                format!(
                    "Host '{host_id}' no longer holds turn '{}'; its allowance was released",
                    turn.id
                ),
            ))
        }
    }
}

/// The grant of a claimed device turn. The model allowance is reserved last, and released
/// as a failed settlement when the tokens can't be signed.
pub(crate) async fn build(
    state: &AppState,
    claimed: &Claimed,
    host_id: &str,
    conversation: &conversation::Model,
) -> Result<DeviceTurnGrant, ApiError> {
    let turn = &claimed.turn;
    let owner = turn.owner_id.as_str();
    let row = bot::Entity::find_by_id(&turn.bot_id)
        .one(&state.db)
        .await?
        .filter(|row| row.deleted_at.is_none())
        .ok_or_else(|| ApiError::not_found(format!("Bot '{}' was deleted", turn.bot_id)))?;
    if let Some(device_id) = row.device_id.as_deref()
        && crate::devices::management::shared_for_deploy(
            &state.db,
            device_id,
            owner,
            chrono::Utc::now().timestamp(),
        )
        .await?
    {
        return Err(ApiError::coded(
            StatusCode::CONFLICT,
            crate::devices::management::DEVICE_SHARED_FOR_DEPLOY,
            DEVICE_SHARED_TEXT,
        ));
    }
    if let Some(delivery) =
        completed_delivery_in(&state.db, claimed, host_id, &row, conversation).await?
    {
        return Ok(delivery);
    }
    let kind = ConversationKind::parse(&conversation.kind).unwrap_or(ConversationKind::Group);
    let room = crate::bot_host::worker::room_of(kind, &turn.conversation_id);
    let (profile, access) = worker::load_profile(state, owner, row.profile_id.as_deref()).await?;
    let bot = chat_bot(state, owner, Some(turn.bot_id.as_str()), &profile)
        .await?
        .ok_or_else(|| ApiError::not_found(format!("Bot '{}' was deleted", turn.bot_id)))?;
    let fls = master_flow_like_state(state).await?;
    let resolved = resolve_models(state, owner, &bot, &profile, &access, &fls).await?;
    let requester = requester_of(state, turn, kind, conversation.title.as_deref()).await?;
    let workspace = bot.context.workspace_app_id.clone();
    let usage = turn_allowance(state, turn, host_id, &resolved, room).await?;
    let ttl = ((usage.deadline - chrono::Utc::now()).num_seconds()).clamp(60, 86_400);
    let tokens = async {
        Ok::<_, ApiError>((
            bot_token(&state.db, owner, &turn.bot_id, &turn.id, ttl).await?,
            executor_token(
                owner,
                &turn.bot_id,
                room,
                &turn.id,
                workspace.as_deref(),
                ttl,
            )?,
        ))
    }
    .await;
    let (bot_token, executor_token) = match tokens {
        Ok(tokens) => tokens,
        Err(error) => {
            if let Some(reserved) = &usage.reserved {
                settle_assistant_usage(state, reserved, 0, false).await.ok();
            }
            return Err(error);
        }
    };
    let mut turn_json = crate::bot_host::ledger::turn_json(turn, claimed.batched.clone());
    turn_json.requester_name = requester.requester_name.clone();
    Ok(DeviceTurnGrant {
        turn: turn_json,
        host_id: host_id.to_string(),
        conversation: DeviceConversation {
            id: conversation.id.clone(),
            kind: kind.as_str().to_string(),
            title: conversation.title.clone(),
        },
        bot: bot_spec(bot),
        requester: Some(requester),
        models: resolved.models,
        bot_token,
        executor_token: Some(executor_token),
        usage: Some(DeviceUsageGrant {
            operation_id: usage.operation_id,
            deadline_ms: usage.deadline.timestamp_millis(),
        }),
        deadline: usage.deadline.timestamp(),
        delivery_only: false,
    })
}

/// Settles the model allowance a device turn used. Only a running assistant operation that the
/// owner started settles; anything else is ignored, so a repeat or a forged id changes nothing.
pub(crate) async fn settle(
    state: &AppState,
    owner: &str,
    report: Option<&DeviceUsageReport>,
    success: bool,
) {
    let Some(report) = report else {
        return;
    };
    let funding = match crate::quota::running_assistant_funding(state, &report.operation_id, owner)
        .await
    {
        Ok(Some(funding)) => funding,
        Ok(None) => return,
        Err(error) => {
            tracing::warn!(%error, operation_id = %report.operation_id, "Device bot turn: usage lookup failed");
            return;
        }
    };
    let usage = AssistantUsage {
        operation_id: report.operation_id.clone(),
        funding_class: funding,
        deadline: chrono::Utc::now(),
        max_runtime_ms: MAX_TURN_RUNTIME_MS,
        rate: assistant_rate(MAX_TURN_RUNTIME_MS, false),
        worker: false,
    };
    let elapsed = report.elapsed_ms.clamp(0, MAX_TURN_RUNTIME_MS);
    if let Err(error) = settle_assistant_usage(state, &usage, elapsed, success).await {
        tracing::error!(%error, operation_id = %report.operation_id, "Device bot turn usage settlement remains pending");
    }
}

#[cfg(test)]
mod decision_tests {
    use super::*;
    use flow_like::{
        bit::{Bit, BitTypes},
        profile::ProfileCustomBit,
        utils::http::HTTPClient,
    };
    use serde_json::json;

    #[tokio::test]
    async fn unusable_optional_decisions_are_omitted_without_releasing_customer_models() {
        let bits: Vec<_> = [
            ("hosted", BitTypes::SystemOne, "hosted:typesafe", 4096),
            ("local", BitTypes::SystemOne, "Local", 4096),
            ("custom", BitTypes::SystemOne, "custom:systemone", 4096),
            ("device", BitTypes::SystemOne, "device", 4096),
            ("chat", BitTypes::Llm, "hosted:openrouter", 4096),
            ("malformed", BitTypes::SystemOne, "hosted:typesafe", 0),
        ]
        .into_iter()
        .map(|(id, bit_type, provider, context_length)| {
            ProfileCustomBit(Bit {
                id: id.into(),
                bit_type,
                parameters: json!({ "context_length": context_length, "provider": {
                "provider_name": provider, "model_id": "upstream-model"
            } }),
                ..Default::default()
            })
        })
        .collect();
        let profile = Profile {
            custom_bits: bits,
            ..Default::default()
        };
        let http = Arc::new(HTTPClient::new_without_refetch());
        for id in ["missing", "local", "custom", "device", "chat", "malformed"] {
            assert!(
                decision_bit(&profile, id, http.clone()).await.is_none(),
                "{id}"
            );
        }
        let usable = decision_bit(&profile, "hosted", http).await.unwrap();
        assert_eq!(usable["id"], "hosted");
        assert_eq!(
            usable["parameters"]["provider"]["provider_name"],
            "hosted:typesafe"
        );
    }
}

#[cfg(test)]
mod hold_tests {
    use super::*;

    #[test]
    fn a_refreshed_token_ends_with_the_hold_of_its_turn() {
        let now = 1_790_000_000;
        for (hold_end, ttl) in [
            (None, REFRESHED_TOKEN_SECS),
            (Some(now + 3_600), REFRESHED_TOKEN_SECS),
            (Some(now + 400), 400),
            (Some(now + 5), DELIVERY_TTL_SECS),
            (Some(now - 500), DELIVERY_TTL_SECS),
        ] {
            assert_eq!(refresh_ttl(hold_end, now), ttl, "{hold_end:?}");
        }
    }
}
