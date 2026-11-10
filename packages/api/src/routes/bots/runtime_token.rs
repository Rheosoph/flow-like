//! A desktop host signs in as the owner and obtains a token for its bot. A supplied turn pins
//! its group. Direct app tokens carry no group and cannot access inbox conversations.

use super::owned_bot;
use crate::{
    bot_permission::{MAX_BOT_TOKEN_TTL_SECONDS, SignedBotToken, sign_bot_token},
    entity::{bot_turn, conversation},
    error::ApiError,
    middleware::jwt::{AppUser, BotUser},
    state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, State},
};
use flow_like_types::{
    bot_turn::{TurnAudience, TurnStatus},
    inbox::ConversationKind,
};
use sea_orm::{ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct BotRuntimeTokenRequest {
    /// A turn of this bot that you own. Omit for an owner-initiated app call.
    pub turn_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct BotRuntimeToken {
    /// Send as `Authorization: Bearer <token>` on app calls the bot makes during the turn.
    pub token: String,
    /// Unix seconds; ask again before it passes.
    pub expires_at: i64,
}

/// The identity a token for `turn_id` carries: the turn must be the bot's and the caller's.
/// A room turn pins the room so its invitation limits apply. Late answers retain the original
/// conversation's scope rather than the exchange that delivered the answer.
pub(crate) async fn bot_user_for_turn<C: ConnectionTrait>(
    db: &C,
    owner_id: &str,
    bot_id: &str,
    turn_id: &str,
) -> Result<BotUser, ApiError> {
    owned_bot(db, owner_id, bot_id).await?;
    let turn = bot_turn::Entity::find_by_id(turn_id)
        .filter(bot_turn::Column::BotId.eq(bot_id))
        .filter(bot_turn::Column::OwnerId.eq(owner_id))
        .one(db)
        .await?
        .ok_or_else(|| ApiError::not_found(format!("Turn '{turn_id}' not found")))?;
    let origin = crate::bot_host::talk::turn_origin_conversation(db, &turn).await?;
    let conv_id = if origin == turn.conversation_id {
        (TurnAudience::parse(&turn.audience) == Some(TurnAudience::Room))
            .then_some(turn.conversation_id)
    } else {
        let kind = conversation::Entity::find_by_id(&origin)
            .one(db)
            .await?
            .and_then(|row| ConversationKind::parse(&row.kind))
            .ok_or_else(|| ApiError::forbidden("The original conversation is unavailable"))?;
        match kind {
            ConversationKind::Group => {
                crate::bot_permission::room_limits(db, bot_id, &origin).await?;
                Some(origin)
            }
            ConversationKind::Direct => {
                if !crate::bot_host::memory::is_owner_direct(db, &origin, owner_id, bot_id).await? {
                    return Err(ApiError::forbidden(
                        "The original conversation is not the bot's active conversation with its owner",
                    ));
                }
                None
            }
        }
    };
    Ok(BotUser {
        bot_id: bot_id.to_string(),
        turn_id: turn.id,
        conv_id,
    })
}

/// Tokens that outlive a request are only for turns a host still works on.
pub(crate) async fn ensure_live_turn<C: ConnectionTrait>(
    db: &C,
    owner_id: &str,
    bot_id: &str,
    turn_id: &str,
) -> Result<(), ApiError> {
    let turn = bot_turn::Entity::find_by_id(turn_id)
        .filter(bot_turn::Column::BotId.eq(bot_id))
        .filter(bot_turn::Column::OwnerId.eq(owner_id))
        .one(db)
        .await?
        .ok_or_else(|| ApiError::not_found(format!("Turn '{turn_id}' not found")))?;
    match TurnStatus::parse(&turn.status) {
        Some(TurnStatus::Queued | TurnStatus::Running | TurnStatus::Waiting) => Ok(()),
        _ => Err(ApiError::conflict(format!(
            "Turn '{turn_id}' is {}; a token is only issued for a turn that is queued, running or waiting",
            turn.status.to_ascii_lowercase()
        ))),
    }
}

#[utoipa::path(
    post,
    path = "/bots/{bot_id}/runtime-token",
    tag = "Bots",
    params(("bot_id" = String, Path, description = "Your bot")),
    request_body = BotRuntimeTokenRequest,
    responses(
        (status = 200, description = "A short-lived token that acts as the bot for this turn.", body = BotRuntimeToken),
        (status = 401, description = "Not signed in"),
        (status = 404, description = "You have no such bot or turn"),
        (status = 409, description = "The turn is already done, failed or cancelled")
    ),
    security(("bearer_auth" = []))
)]
#[tracing::instrument(name = "POST /bots/{bot_id}/runtime-token", skip(state, user, body))]
pub async fn mint_runtime_token(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(bot_id): Path<String>,
    Json(body): Json<BotRuntimeTokenRequest>,
) -> Result<Json<BotRuntimeToken>, ApiError> {
    let owner_id = user.sub()?;
    let bot = bot_user_for_request(&state, &owner_id, bot_id, body.turn_id.as_deref()).await?;
    let ttl = body.turn_id.as_ref().map(|_| MAX_BOT_TOKEN_TTL_SECONDS);
    let SignedBotToken { token, expires_at } = sign_bot_token(&bot, ttl)
        .map_err(|error| ApiError::internal_error(flow_like_types::anyhow!(error)))?;
    Ok(Json(BotRuntimeToken { token, expires_at }))
}

/// A model request may start a direct chat, or answer a validated inbox turn.
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct BotModelTokenRequest {
    pub turn_id: Option<String>,
    pub app_id: Option<String>,
}

#[utoipa::path(
    post,
    path = "/bots/{bot_id}/model-token",
    tag = "Bots",
    params(("bot_id" = String, Path, description = "Your bot")),
    request_body = BotModelTokenRequest,
    responses((status = 200, description = "A model token attributed to this bot and its turn's group", body = BotRuntimeToken)),
    security(("bearer_auth" = []))
)]
pub async fn mint_model_token(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(bot_id): Path<String>,
    Json(body): Json<BotModelTokenRequest>,
) -> Result<Json<BotRuntimeToken>, ApiError> {
    let owner_id = user.sub()?;
    let bot = bot_user_for_request(&state, &owner_id, bot_id, body.turn_id.as_deref()).await?;
    if let Some(app_id) = &body.app_id {
        crate::bot_permission::bot_app_permission(&bot, app_id, &state).await?;
    }
    const TTL_SECONDS: i64 = 900;
    // A project is attached only after checking the bot's current app permission.
    let token = crate::bot_host::worker::executor_token(
        &owner_id,
        &bot.bot_id,
        bot.conv_id.as_deref(),
        &bot.turn_id,
        body.app_id.as_deref(),
        TTL_SECONDS,
    )
    .ok_or_else(|| ApiError::internal("The bot model token could not be signed"))?;
    Ok(Json(BotRuntimeToken {
        token,
        expires_at: chrono::Utc::now().timestamp() + TTL_SECONDS,
    }))
}

async fn bot_user_for_request(
    state: &AppState,
    owner_id: &str,
    bot_id: String,
    turn_id: Option<&str>,
) -> Result<BotUser, ApiError> {
    let bot_id = match bot_id.as_str() {
        "general" | "flux" => {
            let rows = super::provision::bots_with_system(state, owner_id).await?;
            rows.into_iter()
                .find(|row| row.system_key.as_deref() == Some(bot_id.as_str()))
                .map(|row| row.id)
                .ok_or_else(|| ApiError::not_found("The account's built-in bot is unavailable"))?
        }
        _ => bot_id,
    };
    let bot = match turn_id {
        Some(turn_id) => {
            let turn_id = turn_id.trim();
            ensure_live_turn(&state.db, owner_id, &bot_id, turn_id).await?;
            bot_user_for_turn(&state.db, owner_id, &bot_id, turn_id).await?
        }
        None => bot_user_for_direct(&state.db, owner_id, &bot_id).await?,
    };
    crate::bot_caller::usage_identity(&state.db, &bot).await?;
    Ok(bot)
}

/// Direct app calls carry no room and cannot use inbox conversation endpoints.
pub(crate) async fn bot_user_for_direct<C: ConnectionTrait>(
    db: &C,
    owner_id: &str,
    bot_id: &str,
) -> Result<BotUser, ApiError> {
    owned_bot(db, owner_id, bot_id).await?;
    Ok(BotUser {
        bot_id: bot_id.to_owned(),
        turn_id: format!("direct:{}", flow_like_types::create_id()),
        conv_id: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::routes::inbox::fixture;
    use sea_orm::{ActiveModelTrait, Set};

    #[tokio::test]
    async fn tokens_are_only_issued_for_live_turns() {
        let db = fixture::database().await;
        fixture::insert_bot(&db, "bot_a", "user-a", "DESKTOP").await;
        let lease = fixture::at(60);
        fixture::running_turn(&db, "t-run", "bot_a", "room", "host", lease).await;
        assert!(
            ensure_live_turn(&db, "user-a", "bot_a", "t-run")
                .await
                .is_ok()
        );
        for (stored, expected) in [
            ("QUEUED", true),
            ("WAITING", true),
            ("DONE", false),
            ("FAILED", false),
            ("CANCELLED", false),
        ] {
            let mut turn: bot_turn::ActiveModel = fixture::turn(&db, "t-run").await.into();
            turn.status = Set(stored.into());
            turn.update(&db).await.unwrap();
            assert_eq!(
                ensure_live_turn(&db, "user-a", "bot_a", "t-run")
                    .await
                    .is_ok(),
                expected,
                "{stored}"
            );
        }
        assert!(
            ensure_live_turn(&db, "user-b", "bot_a", "t-run")
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn tokens_expire_after_the_grace_following_their_turn() {
        use crate::bot_caller::{TOKEN_GRACE_SECONDS, ensure_token_live};
        let db = fixture::database().await;
        fixture::insert_bot(&db, "bot_a", "user-a", "DESKTOP").await;
        fixture::running_turn(&db, "t-1", "bot_a", "room", "host", fixture::at(60)).await;
        let token = BotUser {
            bot_id: "bot_a".into(),
            turn_id: "t-1".into(),
            conv_id: None,
        };
        let set = |status: &'static str, age: i64| {
            let db = db.clone();
            async move {
                let mut turn: bot_turn::ActiveModel = fixture::turn(&db, "t-1").await.into();
                turn.status = Set(status.into());
                let at = chrono::Utc::now() - chrono::Duration::seconds(age);
                turn.updated_at = Set(at.fixed_offset());
                turn.update(&db).await.unwrap();
            }
        };
        set("RUNNING", 3600).await;
        assert!(ensure_token_live(&db, &token).await.is_ok());
        set("DONE", 60).await;
        assert!(ensure_token_live(&db, &token).await.is_ok());
        set("DONE", 600).await;
        assert!(600 > TOKEN_GRACE_SECONDS);
        assert!(ensure_token_live(&db, &token).await.is_err());
        let direct = BotUser {
            turn_id: "direct:x".into(),
            ..token
        };
        assert!(ensure_token_live(&db, &direct).await.is_ok());
    }
}
