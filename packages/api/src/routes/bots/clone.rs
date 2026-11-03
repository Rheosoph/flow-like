//! Duplicating a bot: a new bot of the caller's with the source's settings and the apps the
//! caller is still a member of. Secrets, the workspace, memory, history and the import key stay
//! with the source. Any of the caller's bots can be duplicated, Assistant and Flux included; the
//! copy is always a bot of their own and counts toward their limits.

use super::{
    apps::member_app_ids,
    crud::admit_new_bot,
    model::{Bot, BotSettings},
    owned_bot,
    provision::new_bot_row,
    validation::{self, NAME_MAX_CHARS},
};
use crate::{
    audit, audit_branch,
    db::coordination::coordinate,
    entity::{bot, bot_app},
    error::ApiError,
    middleware::jwt::AppUser,
    state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, State},
    http::StatusCode,
};
use chrono::{DateTime, FixedOffset};
use flow_like::flow::copilot::{FLUX_BOT_NAME, GENERAL_BOT_NAME};
use flow_like_types::bot::BotSystemKind;
use sea_orm::{ActiveModelTrait, ActiveValue::Set, ConnectionTrait, EntityTrait};

const COPY_MARKER: &str = " copy";

/// "<name> copy", the name shortened at its end so the marker always fits.
pub(super) fn copy_name(name: &str) -> String {
    let room = NAME_MAX_CHARS - COPY_MARKER.chars().count();
    let base: String = name.chars().take(room).collect();
    format!("{}{COPY_MARKER}", base.trim_end())
}

/// The stored name, or a built-in bot's default name while it was never renamed.
pub(super) fn source_name<'a>(source: &bot::Model, settings: &'a BotSettings) -> &'a str {
    settings.name.as_deref().unwrap_or(
        match source.system_key.as_deref().and_then(BotSystemKind::parse) {
            Some(BotSystemKind::General) => GENERAL_BOT_NAME,
            Some(BotSystemKind::Flux) => FLUX_BOT_NAME,
            None => "Bot",
        },
    )
}

/// A stored duplicate and the apps it took over.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct Cloned {
    pub bot: bot::Model,
    pub app_ids: Vec<String>,
}

/// Stores a duplicate of the caller's bot `bot_id`. The caller holds the owner's lock, so the
/// limits [`admit_new_bot`] checks are exact. The profile and device are taken over as they are:
/// like an update that leaves them alone, one deleted or revoked since doesn't block the copy.
pub(super) async fn clone_in<C: ConnectionTrait>(
    db: &C,
    owner_id: &str,
    bot_id: &str,
    now: DateTime<FixedOffset>,
) -> Result<Cloned, ApiError> {
    let source = owned_bot(db, owner_id, bot_id).await?;
    let mut settings = BotSettings::from_row(&source);
    settings.name = Some(copy_name(source_name(&source, &settings)));
    validation::check_rules(&settings)?;
    admit_new_bot(db, owner_id, settings.location).await?;
    let app_ids = member_app_ids(db, owner_id, bot_id).await?;
    let stored = new_bot_row(owner_id, None, source.preset_id, &settings, now)?
        .insert(db)
        .await?;
    if !app_ids.is_empty() {
        bot_app::Entity::insert_many(app_ids.iter().map(|app_id| bot_app::ActiveModel {
            bot_id: Set(stored.id.clone()),
            app_id: Set(app_id.clone()),
            added_at: Set(now),
        }))
        .exec_without_returning(db)
        .await?;
    }
    Ok(Cloned {
        bot: stored,
        app_ids,
    })
}

#[utoipa::path(
    post,
    path = "/bots/{bot_id}/clone",
    tag = "Bots",
    params(("bot_id" = String, Path, description = "Your bot's id; Assistant and Flux can be duplicated too")),
    responses(
        (status = 201, description = "A new bot of your own named \"<name> copy\", with the original's look, description, instructions, model, classification, capabilities, location, device, profile and preset, and the attached apps you are still a member of; a duplicate of Flux keeps FlowPilot. Secrets, workspace, memory and history stay with the original.", body = Bot),
        (status = 401, description = "Not signed in"),
        (status = 403, description = "Only people manage bots, not API keys or running flows"),
        (status = 404, description = "You have no bot with this id"),
        (status = 409, description = "BOT_LIMIT_REACHED: you already have 50 bots of your own; BOT_SERVER_LIMIT_REACHED: the copy would run on the server and your plan's bots on the server are used up (bots on a desktop or device don't count)"),
        (status = 422, description = "BOT_CAPABILITY_LOCATION: the original holds a capability its location no longer allows; change it first")
    ),
    security(("bearer_auth" = []))
)]
#[tracing::instrument(name = "POST /bots/{bot_id}/clone", skip(state, user))]
pub async fn clone_bot(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(bot_id): Path<String>,
) -> Result<(StatusCode, Json<Bot>), ApiError> {
    let sub = user.sub()?;
    let Cloned { bot, app_ids } = state
        .transaction(|txn| {
            let (sub, bot_id) = (sub.clone(), bot_id.clone());
            Box::pin(async move {
                coordinate(txn, "bot-owner", &[&sub]).await?;
                clone_in(txn, &sub, &bot_id, chrono::Utc::now().fixed_offset()).await
            })
        })
        .await?;
    let bot = Bot::from(bot);
    record_clone(&state, &user, &bot, &bot_id, &app_ids).await;
    Ok((StatusCode::CREATED, Json(bot)))
}

/// `bot.clone` on the person's chain and, like attaching, `bot.app.attach` on each app's.
async fn record_clone(
    state: &AppState,
    user: &AppUser,
    bot: &Bot,
    source_id: &str,
    app_ids: &[String],
) {
    audit!(
        state,
        user,
        "bot.clone",
        "Bot",
        bot.id,
        serde_json::json!({
            "source_id": source_id,
            "apps": app_ids,
            "capabilities": bot.capabilities,
            "location": bot.location,
            "preset_id": bot.preset_id,
        })
    );
    for app_id in app_ids {
        audit_branch!(state, user, app_id, "bot.app.attach", "Bot", bot.id);
    }
}
