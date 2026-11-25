//! A bot's remembered facts for its owner: list them with the history positions that back them,
//! forget one, reset the memory. The facts live in the bot's workspace database; the owner's
//! credentials open it, so nobody else's bot is reachable.

use super::{owned_bot, workspace::live_workspace};
use crate::{
    bot_tools::ApiAccess, error::ApiError, middleware::jwt::AppUser,
    routes::ai::copilot::master_flow_like_state, state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, Query, State},
};
use flow_like::flow::copilot::memory_open::workspace_connection;
use flow_like_bot_memory::host::{EntityUpdate, EntityView, FactAdmin, FactView};
use flow_like_bot_tools_host::AppAccess;
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct BotFactList {
    /// Newest first. Each fact: `id`, `text`, `kind`, `subject`, `source`, `confidence`,
    /// `recorded_at`, `valid_from`, `valid_to`, optional `geojson`, `time`, `expires_at`,
    /// `superseded_by` and `evidence` (`seq`,
    /// `message_id`, `quote`, `at`).
    #[schema(value_type = Vec<Object>)]
    pub facts: Vec<FactView>,
}

#[derive(Debug, Clone, Default, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct FactsQuery {
    /// Also the facts that were replaced by newer ones.
    pub include_superseded: Option<bool>,
}

pub(crate) async fn admin_for(
    state: &AppState,
    sub: &str,
    bot_id: &str,
) -> Result<Option<FactAdmin>, ApiError> {
    let bot = owned_bot(&state.db, sub, bot_id).await?;
    let Some(workspace) = live_workspace(&state.db, sub, &bot.id).await? else {
        return Ok(None);
    };
    let connection = ApiAccess::new(state.clone(), sub, &workspace)
        .connection(&workspace)
        .await
        .map_err(|error| ApiError::internal(format!("Opening the bot's memory failed: {error}")))?;
    let flow_like_state = master_flow_like_state(state).await?;
    let (connection, write_options) =
        workspace_connection(&flow_like_state, &workspace, Some(connection))
            .await
            .map_err(|error| {
                ApiError::internal(format!("Opening the bot's memory failed: {error}"))
            })?;
    Ok(Some(FactAdmin::open(connection, write_options).await))
}

#[utoipa::path(
    get,
    path = "/bots/{bot_id}/memory/facts",
    tag = "Bots",
    params(("bot_id" = String, Path, description = "Your bot's id"), FactsQuery),
    responses(
        (status = 200, description = "What the bot remembers, newest first, each with the chat positions and quotes it was learned from. A bot without a workspace remembers nothing.", body = BotFactList),
        (status = 401, description = "Not signed in"),
        (status = 404, description = "You have no bot with this id")
    ),
    security(("bearer_auth" = []))
)]
#[tracing::instrument(name = "GET /bots/{bot_id}/memory/facts", skip(state, user))]
pub async fn list_facts(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(bot_id): Path<String>,
    Query(query): Query<FactsQuery>,
) -> Result<Json<BotFactList>, ApiError> {
    let sub = user.sub()?;
    let Some(admin) = admin_for(&state, &sub, &bot_id).await? else {
        return Ok(Json(BotFactList { facts: Vec::new() }));
    };
    let facts = admin
        .list_views(query.include_superseded.unwrap_or(false))
        .await
        .map_err(|error| ApiError::internal(format!("Listing the bot's facts failed: {error}")))?;
    Ok(Json(BotFactList { facts }))
}

#[utoipa::path(
    delete,
    path = "/bots/{bot_id}/memory/facts/{fact_id}",
    tag = "Bots",
    params(
        ("bot_id" = String, Path, description = "Your bot's id"),
        ("fact_id" = String, Path, description = "The fact's id, or a unique start of it")
    ),
    responses(
        (status = 200, description = "The fact is deleted, old table versions included. The chat keeps what was said.", body = Object),
        (status = 401, description = "Not signed in"),
        (status = 404, description = "You have no bot with this id, or it has no such fact"),
        (status = 409, description = "The bot is working on its memory right now; try again shortly")
    ),
    security(("bearer_auth" = []))
)]
#[tracing::instrument(
    name = "DELETE /bots/{bot_id}/memory/facts/{fact_id}",
    skip(state, user)
)]
pub async fn forget_fact(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((bot_id, fact_id)): Path<(String, String)>,
) -> Result<Json<FactView>, ApiError> {
    let sub = user.sub()?;
    let admin = admin_for(&state, &sub, &bot_id).await?.ok_or_else(|| {
        ApiError::not_found(format!("Bot '{bot_id}' remembers no fact '{fact_id}'"))
    })?;
    match admin.forget(&fact_id).await {
        Ok(fact) => Ok(Json(FactView::from(fact))),
        Err(error)
            if error
                .downcast_ref::<flow_like_bot_memory::MemoryBusy>()
                .is_some() =>
        {
            Err(ApiError::conflict(error.to_string()))
        }
        Err(error) => Err(ApiError::not_found(error.to_string())),
    }
}

#[utoipa::path(
    delete,
    path = "/bots/{bot_id}/memory",
    tag = "Bots",
    params(("bot_id" = String, Path, description = "Your bot's id")),
    responses(
        (status = 200, description = "Everything the bot remembered is deleted; the chat history stays."),
        (status = 401, description = "Not signed in"),
        (status = 404, description = "You have no bot with this id")
    ),
    security(("bearer_auth" = []))
)]
#[tracing::instrument(name = "DELETE /bots/{bot_id}/memory", skip(state, user))]
pub async fn reset_memory(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(bot_id): Path<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let sub = user.sub()?;
    if let Some(admin) = admin_for(&state, &sub, &bot_id).await? {
        admin.reset().await.map_err(|error| {
            ApiError::internal(format!("Resetting the bot's memory failed: {error}"))
        })?;
    }
    flow_like::flow::copilot::memory_open::forget_bot_memory(&bot_id);
    Ok(Json(serde_json::json!({ "status": "ok" })))
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct BotEntityList {
    /// The owner first, then by how much is remembered. Each entity: `id`, `name`, `kind`
    /// (person, place, organization, project, thing), `owner`, optional `participant`,
    /// `aliases`, `fact_count`, `created_at`, `updated_at` and, for a room, `conversation_scope`.
    #[schema(value_type = Vec<Object>)]
    pub entities: Vec<EntityView>,
}

#[derive(Debug, Clone, Default, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct EntityScopeQuery {
    /// The room the entity belongs to; leave out for the direct chat.
    pub scope: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize, ToSchema)]
pub struct EntityPatch {
    /// A new name; the old one stays an alias.
    pub name: Option<String>,
    /// person, place, organization, project or thing.
    pub kind: Option<String>,
}

#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct EntityMerge {
    /// The entity that stays; the merged one's facts move to it.
    pub into: String,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct BotEntityForgotten {
    /// The facts that were deleted with the entity.
    #[schema(value_type = Vec<Object>)]
    pub forgotten: Vec<FactView>,
}

fn entity_error(error: flow_like_types::Error) -> ApiError {
    if error
        .downcast_ref::<flow_like_bot_memory::MemoryBusy>()
        .is_some()
    {
        ApiError::conflict(error.to_string())
    } else if error
        .downcast_ref::<flow_like_bot_memory::NoEntity>()
        .is_some()
    {
        ApiError::not_found(error.to_string())
    } else {
        ApiError::bad_request(error.to_string())
    }
}

async fn entity_admin(
    state: &AppState,
    sub: &str,
    bot_id: &str,
    entity_id: &str,
) -> Result<FactAdmin, ApiError> {
    admin_for(state, sub, bot_id).await?.ok_or_else(|| {
        ApiError::not_found(format!("Bot '{bot_id}' remembers no entity '{entity_id}'"))
    })
}

#[utoipa::path(
    get,
    path = "/bots/{bot_id}/memory/entities",
    tag = "Bots",
    params(("bot_id" = String, Path, description = "Your bot's id")),
    responses(
        (status = 200, description = "The people, places, organizations, projects and things the bot remembers facts about, you first. A bot without a workspace remembers none.", body = BotEntityList),
        (status = 401, description = "Not signed in"),
        (status = 404, description = "You have no bot with this id")
    ),
    security(("bearer_auth" = []))
)]
#[tracing::instrument(name = "GET /bots/{bot_id}/memory/entities", skip(state, user))]
pub async fn list_entities(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(bot_id): Path<String>,
) -> Result<Json<BotEntityList>, ApiError> {
    let sub = user.sub()?;
    let Some(admin) = admin_for(&state, &sub, &bot_id).await? else {
        return Ok(Json(BotEntityList {
            entities: Vec::new(),
        }));
    };
    let entities = admin.list_entities().await.map_err(|error| {
        ApiError::internal(format!("Listing the bot's entities failed: {error}"))
    })?;
    Ok(Json(BotEntityList { entities }))
}

#[utoipa::path(
    patch,
    path = "/bots/{bot_id}/memory/entities/{entity_id}",
    tag = "Bots",
    params(
        ("bot_id" = String, Path, description = "Your bot's id"),
        ("entity_id" = String, Path, description = "The entity's id"),
        EntityScopeQuery
    ),
    request_body = EntityPatch,
    responses(
        (status = 200, description = "The entity is renamed or re-typed; the old name stays one of its names.", body = Object),
        (status = 400, description = "The change is not allowed, for example renaming you or an unknown kind"),
        (status = 401, description = "Not signed in"),
        (status = 404, description = "You have no bot with this id, or it has no such entity"),
        (status = 409, description = "The bot is working on its memory right now; try again shortly")
    ),
    security(("bearer_auth" = []))
)]
#[tracing::instrument(
    name = "PATCH /bots/{bot_id}/memory/entities/{entity_id}",
    skip(state, user, patch)
)]
pub async fn update_entity(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((bot_id, entity_id)): Path<(String, String)>,
    Query(query): Query<EntityScopeQuery>,
    Json(patch): Json<EntityPatch>,
) -> Result<Json<EntityView>, ApiError> {
    let sub = user.sub()?;
    let admin = entity_admin(&state, &sub, &bot_id, &entity_id).await?;
    let update = EntityUpdate {
        name: patch.name,
        kind: patch.kind,
    };
    admin
        .update_entity(query.scope.as_deref(), &entity_id, &update)
        .await
        .map(Json)
        .map_err(entity_error)
}

#[utoipa::path(
    post,
    path = "/bots/{bot_id}/memory/entities/{entity_id}/merge",
    tag = "Bots",
    params(
        ("bot_id" = String, Path, description = "Your bot's id"),
        ("entity_id" = String, Path, description = "The entity to merge away"),
        EntityScopeQuery
    ),
    request_body = EntityMerge,
    responses(
        (status = 200, description = "The entity is merged: its facts and names move to the other one.", body = Object),
        (status = 400, description = "The merge is not allowed, for example merging yourself away"),
        (status = 401, description = "Not signed in"),
        (status = 404, description = "You have no bot with this id, or it has no such entity"),
        (status = 409, description = "The bot is working on its memory right now; try again shortly")
    ),
    security(("bearer_auth" = []))
)]
#[tracing::instrument(
    name = "POST /bots/{bot_id}/memory/entities/{entity_id}/merge",
    skip(state, user, merge)
)]
pub async fn merge_entity(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((bot_id, entity_id)): Path<(String, String)>,
    Query(query): Query<EntityScopeQuery>,
    Json(merge): Json<EntityMerge>,
) -> Result<Json<EntityView>, ApiError> {
    let sub = user.sub()?;
    let admin = entity_admin(&state, &sub, &bot_id, &entity_id).await?;
    admin
        .merge_entities(query.scope.as_deref(), &entity_id, &merge.into)
        .await
        .map(Json)
        .map_err(entity_error)
}

#[utoipa::path(
    delete,
    path = "/bots/{bot_id}/memory/entities/{entity_id}",
    tag = "Bots",
    params(
        ("bot_id" = String, Path, description = "Your bot's id"),
        ("entity_id" = String, Path, description = "The entity's id"),
        EntityScopeQuery
    ),
    responses(
        (status = 200, description = "The entity and every fact about it are deleted. The chat keeps what was said.", body = BotEntityForgotten),
        (status = 400, description = "You cannot be forgotten as an entity"),
        (status = 401, description = "Not signed in"),
        (status = 404, description = "You have no bot with this id, or it has no such entity"),
        (status = 409, description = "The bot is working on its memory right now; try again shortly")
    ),
    security(("bearer_auth" = []))
)]
#[tracing::instrument(
    name = "DELETE /bots/{bot_id}/memory/entities/{entity_id}",
    skip(state, user)
)]
pub async fn forget_entity(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((bot_id, entity_id)): Path<(String, String)>,
    Query(query): Query<EntityScopeQuery>,
) -> Result<Json<BotEntityForgotten>, ApiError> {
    let sub = user.sub()?;
    let admin = entity_admin(&state, &sub, &bot_id, &entity_id).await?;
    let forgotten = admin
        .forget_entity(query.scope.as_deref(), &entity_id)
        .await
        .map_err(entity_error)?;
    Ok(Json(BotEntityForgotten { forgotten }))
}
