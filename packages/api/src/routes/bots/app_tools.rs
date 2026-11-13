//! One app tool call of a turn a device host runs: the device's own tools have no app access, so
//! it asks the hub, which runs the call as the bot exactly as for a server-run turn.

use axum::{
    Extension, Json,
    extract::{Path, Query, State},
    http::StatusCode,
};
use flow_like::flow::copilot::{BotToolHost, BotToolset};
use sea_orm::ConnectionTrait;
use serde::Deserialize;
use serde_json::Value;
use utoipa::IntoParams;

use super::{owned_bot, turns::checked_host_id};
use crate::{
    bot_caller::{self, Names},
    bot_host::ledger::{self, BOT_TURN_LEASE_LOST},
    bot_tools::apps::{AppTools, is_app_tool},
    entity::bot,
    error::ApiError,
    middleware::jwt::{AppUser, BotUser},
    routes::{bots::runtime::chat_bot, inbox::now_millis},
    state::AppState,
};

#[derive(Debug, Deserialize, IntoParams)]
pub struct AppToolQuery {
    /// The device host that holds the turn.
    #[param(example = "device:7f3c1a")]
    pub host_id: String,
}

/// The bot row of a turn that `host_id` holds right now, for a call that names `tool`. Only the
/// turn's own bot token passes, and only while the host's lease on the turn is live.
async fn held_by_host<C: ConnectionTrait>(
    db: &C,
    bot: &BotUser,
    bot_id: &str,
    turn_id: &str,
    host_id: &str,
    tool: &str,
) -> Result<bot::Model, ApiError> {
    bot_caller::check(
        bot,
        &Names {
            as_bot: Some(bot_id),
            turn: Some(turn_id),
            ..Default::default()
        },
    )?;
    if !is_app_tool(tool) {
        return Err(ApiError::bad_request(format!(
            "'{tool}' is not an app tool; the hub runs only app tools for a device turn"
        )));
    }
    let owner = bot_caller::live_owner(db, &bot.bot_id).await?;
    let held = ledger::holds_lease(db, turn_id, host_id, now_millis()).await?
        && ledger::find(db, turn_id, Some(&owner))
            .await?
            .is_some_and(|turn| turn.bot_id == bot.bot_id);
    if !held {
        return Err(ApiError::coded(
            StatusCode::CONFLICT,
            BOT_TURN_LEASE_LOST,
            format!(
                "Host '{host_id}' does not hold running turn '{turn_id}', so it runs no app tool for it"
            ),
        ));
    }
    owned_bot(db, &owner, &bot.bot_id).await
}

#[utoipa::path(
    post,
    path = "/bots/{bot_id}/turns/{turn_id}/app-tools/{tool}",
    tag = "Bots",
    params(
        ("bot_id" = String, Path, description = "Your bot's id"),
        ("turn_id" = String, Path, description = "The running turn the device host holds"),
        ("tool" = String, Path, description = "An app tool: list_apps, describe_app_interface, get_app_detail, call_app_event, call_app_chat, database_tool or graph_query_tool"),
        AppToolQuery
    ),
    request_body(content = Object, description = "The tool's arguments"),
    responses(
        (status = 200, description = "The tool's result as the bot reads it; a failure inside the tool is a result with status error", body = Object),
        (status = 400, description = "Not an app tool, or host_id is empty, too long or has spaces"),
        (status = 401, description = "Not signed in"),
        (status = 403, description = "Not this turn's bot token, or a bot whose tools the hub does not run"),
        (status = 409, description = "BOT_TURN_LEASE_LOST: the host does not hold this running turn")
    ),
    security(("bearer_auth" = []))
)]
#[tracing::instrument(
    name = "POST /bots/{bot_id}/turns/{turn_id}/app-tools/{tool}",
    skip(state, user, args)
)]
pub async fn run_app_tool(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((bot_id, turn_id, tool)): Path<(String, String, String)>,
    Query(query): Query<AppToolQuery>,
    Json(args): Json<Value>,
) -> Result<Json<Value>, ApiError> {
    let AppUser::Bot(principal) = user else {
        return Err(ApiError::forbidden(
            "Only a bot's runtime token for the turn runs its app tools",
        ));
    };
    let host_id = checked_host_id(super::turns::TurnHostRequest {
        host_id: query.host_id,
    })?;
    let row = held_by_host(&state.db, &principal, &bot_id, &turn_id, &host_id, &tool).await?;
    let owner = row.owner_id.as_str();
    let (profile, _) =
        crate::bot_host::worker::load_profile(&state, owner, row.profile_id.as_deref()).await?;
    let mut chat = chat_bot(&state, owner, Some(bot_id.as_str()), &profile)
        .await?
        .ok_or_else(|| ApiError::not_found(format!("Bot '{bot_id}' was deleted")))?;
    if chat.context.toolset != BotToolset::Capabilities {
        return Err(ApiError::forbidden(format!(
            "Bot '{bot_id}' runs FlowPilot's own toolset, which has no hosted app tools"
        )));
    }
    chat.context.tool_host = BotToolHost::Hosted;
    let result = AppTools::new(&state, principal, &chat.context)
        .run(&tool, args)
        .await;
    Ok(Json(serde_json::from_str(&result).map_err(|error| {
        ApiError::internal(format!(
            "App tool '{tool}' returned unreadable JSON: {error}"
        ))
    })?))
}

#[cfg(test)]
#[path = "app_tools_tests.rs"]
mod tests;
