//! What a device receives when it claims a bot turn: the turn's credentials, the bot's run spec
//! and the hub-resolved models. It resolves the turn like the server's worker does, except that
//! only hosted models leave the server (a customer provider's Bit carries the owner's
//! credentials), and the model allowance is reserved here and settled when the device ends the
//! turn. The device pays no runtime compute: it computes.

use crate::{
    bot_host::{
        history,
        ledger::Claimed,
        worker::{self, MAX_TURN_RUNTIME_MS},
    },
    bot_permission::{SignedBotToken, sign_bot_token},
    entity::{bot, conversation},
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
            runtime::{ChatBot, chat_bot, hosted_bot_model},
            runtime_token::bot_user_for_turn,
        },
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
    bot_turn::TurnAudience,
    device_bot::{
        DeviceBotSpec, DeviceConversation, DeviceModels, DeviceRequester, DeviceToken,
        DeviceTurnGrant, DeviceUsageGrant, DeviceUsageReport,
    },
    inbox::{ConversationEncryption, ConversationKind},
};
use sea_orm::EntityTrait;
use std::sync::Arc;

pub(crate) const BOT_TURN_ENCRYPTED: &str = worker::CODE_ENCRYPTED;

pub(crate) const BOT_DEVICE_MODEL_NOT_HOSTED: &str = "BOT_DEVICE_MODEL_NOT_HOSTED";
pub(crate) const MODEL_NOT_HOSTED_TEXT: &str =
    "This bot's model can't run on a device; pick a hosted model in its settings.";

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
    let usage = reserve(state, owner, &resolved).await?;
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
            settle_assistant_usage(state, &usage, 0, false).await.ok();
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
