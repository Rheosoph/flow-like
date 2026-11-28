//! What adding people or bots would change in a team's room, before anyone is invited: the bots
//! that would stop answering there because someone in it would be classified below them.

use super::{
    access::{active, admin_team, member_team},
    levels::person_levels,
    model::TeamPreviewRequest,
};
use crate::{
    bot_host::team_gate::refused_level,
    entity::{bot, bot_team_bot, bot_team_person},
    error::ApiError,
    middleware::jwt::AppUser,
    state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, State},
};
use flow_like_types::{
    bot_team::{CandidateEffect, MAX_TEAM_BOTS, MAX_TEAM_PEOPLE, SilencedBot, TeamPreview},
    data_classification::DataClassification,
};
use sea_orm::{ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter};
use std::collections::{HashMap, HashSet};

fn level_of(row: &bot::Model) -> Option<DataClassification> {
    row.classification
        .as_deref()
        .and_then(DataClassification::parse)
}

/// The level `row` is silenced at among `people`, when it is.
fn silenced(row: &bot::Model, people: &[(String, DataClassification)]) -> Option<SilencedBot> {
    refused_level(level_of(row), &row.owner_id, people).map(|classification| SilencedBot {
        bot_id: row.id.clone(),
        classification,
    })
}

async fn active_people<C: ConnectionTrait>(
    db: &C,
    team_ids: Vec<String>,
) -> Result<Vec<String>, ApiError> {
    let mut people: Vec<String> = bot_team_person::Entity::find()
        .filter(bot_team_person::Column::TeamId.is_in(team_ids))
        .filter(bot_team_person::Column::Status.eq(active()))
        .all(db)
        .await?
        .into_iter()
        .map(|row| row.user_id)
        .collect();
    people.sort();
    people.dedup();
    Ok(people)
}

/// For each person and bot in `request` on its own: the team's bots that answer in the room now
/// and wouldn't once that one is active in it, and for a bot whether it would answer there. Only
/// the team's admins preview people; any active person of the team previews bots. Bots whose
/// owner isn't in the org are left out, since they can't join.
pub(crate) async fn preview_in<C: ConnectionTrait>(
    db: &C,
    sub: &str,
    team_id: &str,
    request: &TeamPreviewRequest,
) -> Result<TeamPreview, ApiError> {
    let (team, org) = member_team(db, sub, team_id).await?;
    if !request.user_ids.is_empty() {
        admin_team(db, sub, team_id).await?;
    }
    if request.user_ids.len() > MAX_TEAM_PEOPLE || request.bot_ids.len() > MAX_TEAM_BOTS {
        return Err(ApiError::bad_request(format!(
            "A preview covers at most {MAX_TEAM_PEOPLE} people and {MAX_TEAM_BOTS} bots; got {} and {}",
            request.user_ids.len(),
            request.bot_ids.len()
        )));
    }
    let org_people: HashSet<String> = active_people(db, org.ids()).await?.into_iter().collect();
    let people = active_people(db, vec![team.id.clone()]).await?;
    let present: Vec<String> = bot_team_bot::Entity::find()
        .filter(bot_team_bot::Column::TeamId.eq(&team.id))
        .filter(bot_team_bot::Column::Status.eq(active()))
        .all(db)
        .await?
        .into_iter()
        .map(|row| row.bot_id)
        .collect();
    let mut wanted = present.clone();
    wanted.extend(request.bot_ids.iter().cloned());
    let bots: HashMap<String, bot::Model> = bot::Entity::find()
        .filter(bot::Column::Id.is_in(wanted))
        .filter(bot::Column::DeletedAt.is_null())
        .all(db)
        .await?
        .into_iter()
        .filter(|row| org_people.contains(&row.owner_id))
        .map(|row| (row.id.clone(), row))
        .collect();
    let mut everyone = people.clone();
    everyone.extend(request.user_ids.iter().cloned());
    let levels = person_levels(db, &org.root.id, &everyone).await?;
    let ranked = |ids: &[String]| -> Vec<(String, DataClassification)> {
        ids.iter()
            .filter_map(|id| levels.get(id).map(|level| (id.clone(), *level)))
            .collect()
    };
    let now = ranked(&people);
    let answering: Vec<&bot::Model> = present
        .iter()
        .filter_map(|id| bots.get(id))
        .filter(|row| silenced(row, &now).is_none())
        .collect();
    let people_effects = request
        .user_ids
        .iter()
        .filter(|user_id| !people.contains(user_id))
        .filter_map(|user_id| {
            let mut with = people.clone();
            with.push(user_id.clone());
            let with = ranked(&with);
            let silenced: Vec<SilencedBot> = answering
                .iter()
                .filter_map(|row| silenced(row, &with))
                .collect();
            (!silenced.is_empty()).then(|| CandidateEffect {
                id: user_id.clone(),
                silenced,
            })
        })
        .collect();
    let bot_effects = request
        .bot_ids
        .iter()
        .filter(|bot_id| !present.contains(bot_id))
        .filter_map(|bot_id| {
            let found = silenced(bots.get(bot_id)?, &now)?;
            Some(CandidateEffect {
                id: bot_id.clone(),
                silenced: vec![found],
            })
        })
        .collect();
    Ok(TeamPreview {
        people: people_effects,
        bots: bot_effects,
    })
}

#[utoipa::path(
    post,
    path = "/bot-teams/{team_id}/preview",
    tag = "Bot teams",
    params(("team_id" = String, Path, description = "The team")),
    request_body = TeamPreviewRequest,
    responses(
        (status = 200, description = "For each person or bot on its own, the bots that would stop answering in the team room if it joined, because someone in the room would be classified below them. Nothing is invited.", body = Object),
        (status = 400, description = "More people or bots than a team holds"),
        (status = 401, description = "Not signed in"),
        (status = 403, description = "TEAM_FORBIDDEN: only the team's admins preview people, and only its people preview bots"),
        (status = 404, description = "TEAM_NOT_FOUND: the team does not exist or you can't see it")
    ),
    security(("bearer_auth" = []))
)]
#[tracing::instrument(name = "POST /bot-teams/{team_id}/preview", skip(state, user, body))]
pub async fn preview(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(team_id): Path<String>,
    Json(body): Json<TeamPreviewRequest>,
) -> Result<Json<TeamPreview>, ApiError> {
    let sub = user.sub()?;
    Ok(Json(preview_in(&state.db, &sub, &team_id, &body).await?))
}
