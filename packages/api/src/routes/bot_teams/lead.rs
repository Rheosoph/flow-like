//! A lead offered by an admin becomes the lead once the bot's owner agrees; the lead's owner may
//! step it down again.

use super::{
    access::{active, forbidden, is_active_in, load_team, org_of},
    effects::{Effects, settle},
    lock_org_scope, lock_scope,
    room::sync_room,
    teams::clear_offer,
    threads::sync_team_threads,
    view::team_view,
};
use crate::{
    audit,
    entity::{bot_team, bot_team_bot},
    error::ApiError,
    middleware::jwt::AppUser,
    routes::{bots::owned_bot, inbox::now_millis},
    state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, State},
    http::StatusCode,
};
use chrono::{DateTime, FixedOffset};
use flow_like_types::bot_team::Team;
use sea_orm::{ActiveModelTrait, ActiveValue::Set, ConnectionTrait, EntityTrait};

fn no_offer(team: &bot_team::Model, bot_id: &str) -> ApiError {
    ApiError::not_found(format!(
        "Team '{}' has no lead offer for bot '{bot_id}'",
        team.name
    ))
}

async fn offered_team<C: ConnectionTrait>(
    db: &C,
    sub: &str,
    team_id: &str,
    bot_id: &str,
) -> Result<bot_team::Model, ApiError> {
    owned_bot(db, sub, bot_id).await?;
    let team = load_team(db, team_id).await?;
    if team.lead_offer_bot_id.as_deref() != Some(bot_id) {
        return Err(no_offer(&team, bot_id));
    }
    Ok(team)
}

async fn store_lead<C: ConnectionTrait>(
    db: &C,
    team: &bot_team::Model,
    lead: Option<String>,
    now: DateTime<FixedOffset>,
) -> Result<bot_team::Model, ApiError> {
    let mut update = bot_team::ActiveModel {
        id: Set(team.id.clone()),
        lead_bot_id: Set(lead),
        updated_at: Set(now),
        ..Default::default()
    };
    clear_offer(&mut update);
    Ok(update.update(db).await?)
}

pub(crate) async fn accept_lead_in<C: ConnectionTrait>(
    db: &C,
    sub: &str,
    team_id: &str,
    bot_id: &str,
    now: DateTime<FixedOffset>,
) -> Result<Effects, ApiError> {
    let team = offered_team(db, sub, team_id, bot_id).await?;
    let row = bot_team_bot::Entity::find_by_id((team.id.clone(), bot_id.to_string()))
        .one(db)
        .await?;
    if row.is_none_or(|row| row.status != active()) {
        return Err(ApiError::bad_request(format!(
            "Bot '{bot_id}' is not an active bot of team '{}' and can't lead it",
            team.name
        )));
    }
    let org = org_of(db, &team).await?;
    if !is_active_in(db, &org.ids(), sub).await? {
        return Err(forbidden(format!(
            "You are no longer a person of '{}' and can't lead its team '{}' with your bot",
            org.root.name, team.name
        )));
    }
    let team = store_lead(db, &team, Some(bot_id.to_string()), now).await?;
    let mut effects = Effects::default();
    sync_room(db, &team, sub, now, &mut effects).await?;
    sync_team_threads(db, &team, sub, now, &mut effects).await?;
    Ok(effects)
}

pub(crate) async fn decline_lead_in<C: ConnectionTrait>(
    db: &C,
    sub: &str,
    team_id: &str,
    bot_id: &str,
    now: DateTime<FixedOffset>,
) -> Result<(), ApiError> {
    let team = offered_team(db, sub, team_id, bot_id).await?;
    store_lead(db, &team, team.lead_bot_id.clone(), now).await?;
    Ok(())
}

pub(crate) async fn resign_lead_in<C: ConnectionTrait>(
    db: &C,
    sub: &str,
    team_id: &str,
    bot_id: &str,
    now: DateTime<FixedOffset>,
) -> Result<Effects, ApiError> {
    owned_bot(db, sub, bot_id).await?;
    let team = load_team(db, team_id).await?;
    if team.lead_bot_id.as_deref() != Some(bot_id) {
        return Err(ApiError::not_found(format!(
            "Bot '{bot_id}' is not the lead of team '{}'",
            team.name
        )));
    }
    let team = bot_team::ActiveModel {
        id: Set(team.id.clone()),
        lead_bot_id: Set(None),
        updated_at: Set(now),
        ..Default::default()
    }
    .update(db)
    .await?;
    let mut effects = Effects::default();
    sync_room(db, &team, sub, now, &mut effects).await?;
    sync_team_threads(db, &team, sub, now, &mut effects).await?;
    Ok(effects)
}

#[utoipa::path(
    post,
    path = "/bot-teams/{team_id}/bots/{bot_id}/lead/accept",
    tag = "Bot teams",
    params(
        ("team_id" = String, Path, description = "The team"),
        ("bot_id" = String, Path, description = "Your bot, offered as the team's lead")
    ),
    responses(
        (status = 200, description = "Your bot leads the team now: it reads the team room along and people of the organization reach it through their thread with the team.", body = Object),
        (status = 400, description = "The bot is no longer an active bot of the team"),
        (status = 401, description = "Not signed in"),
        (status = 403, description = "You are no longer a person of the organization"),
        (status = 404, description = "The bot is not yours, or the team holds no lead offer for it")
    ),
    security(("bearer_auth" = []))
)]
#[tracing::instrument(
    name = "POST /bot-teams/{team_id}/bots/{bot_id}/lead/accept",
    skip(state, user)
)]
pub async fn accept_lead(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((team_id, bot_id)): Path<(String, String)>,
) -> Result<Json<Team>, ApiError> {
    let sub = user.sub()?;
    let effects = state
        .transaction(|txn| {
            let (sub, team_id, bot_id) = (sub.clone(), team_id.clone(), bot_id.clone());
            Box::pin(async move {
                lock_scope(txn, &[team_id.as_str()], &[bot_id.as_str()]).await?;
                accept_lead_in(txn, &sub, &team_id, &bot_id, now_millis()).await
            })
        })
        .await?;
    settle(&state, effects).await;
    audit_lead(&state, &user, "bot.team.lead.accept", &team_id, &bot_id).await;
    Ok(Json(team_view(&state, &sub, &team_id).await?))
}

#[utoipa::path(
    post,
    path = "/bot-teams/{team_id}/bots/{bot_id}/lead/decline",
    tag = "Bot teams",
    params(
        ("team_id" = String, Path, description = "The team"),
        ("bot_id" = String, Path, description = "Your bot, offered as the team's lead")
    ),
    responses(
        (status = 204, description = "You declined the offer; the team keeps its current lead."),
        (status = 401, description = "Not signed in"),
        (status = 404, description = "The bot is not yours, or the team holds no lead offer for it")
    ),
    security(("bearer_auth" = []))
)]
#[tracing::instrument(
    name = "POST /bot-teams/{team_id}/bots/{bot_id}/lead/decline",
    skip(state, user)
)]
pub async fn decline_lead(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((team_id, bot_id)): Path<(String, String)>,
) -> Result<StatusCode, ApiError> {
    let sub = user.sub()?;
    state
        .transaction(|txn| {
            let (sub, team_id, bot_id) = (sub.clone(), team_id.clone(), bot_id.clone());
            Box::pin(async move {
                lock_scope(txn, &[team_id.as_str()], &[bot_id.as_str()]).await?;
                decline_lead_in(txn, &sub, &team_id, &bot_id, now_millis()).await
            })
        })
        .await?;
    audit_lead(&state, &user, "bot.team.lead.decline", &team_id, &bot_id).await;
    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(
    post,
    path = "/bot-teams/{team_id}/bots/{bot_id}/lead/resign",
    tag = "Bot teams",
    params(
        ("team_id" = String, Path, description = "The team"),
        ("bot_id" = String, Path, description = "Your bot, the team's lead")
    ),
    responses(
        (status = 200, description = "Your bot no longer leads the team; the team has no lead until an admin offers one, and people can't talk to it as a team meanwhile.", body = Object),
        (status = 401, description = "Not signed in"),
        (status = 403, description = "TEAM_FORBIDDEN: only the lead's owner steps it down"),
        (status = 404, description = "The team has no lead, or the bot is not the lead")
    ),
    security(("bearer_auth" = []))
)]
#[tracing::instrument(
    name = "POST /bot-teams/{team_id}/bots/{bot_id}/lead/resign",
    skip(state, user)
)]
pub async fn resign_lead(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((team_id, bot_id)): Path<(String, String)>,
) -> Result<Json<Team>, ApiError> {
    let sub = user.sub()?;
    let effects = state
        .transaction(|txn| {
            let (sub, team_id, bot_id) = (sub.clone(), team_id.clone(), bot_id.clone());
            Box::pin(async move {
                lock_org_scope(txn, &[team_id.as_str()]).await?;
                resign_lead_in(txn, &sub, &team_id, &bot_id, now_millis()).await
            })
        })
        .await?;
    settle(&state, effects).await;
    audit_lead(&state, &user, "bot.team.lead.resign", &team_id, &bot_id).await;
    Ok(Json(team_view(&state, &sub, &team_id).await?))
}

/// The lead and lead-offer changes from `before` to `after`, as audit actions with their bot.
pub(crate) fn lead_events(
    before: &bot_team::Model,
    after: &bot_team::Model,
) -> Vec<(&'static str, String)> {
    let mut events = Vec::new();
    if before.lead_bot_id != after.lead_bot_id {
        match (&after.lead_bot_id, &before.lead_bot_id) {
            (Some(bot_id), _) => events.push(("bot.team.lead.set", bot_id.clone())),
            (None, Some(previous)) => events.push(("bot.team.lead.clear", previous.clone())),
            (None, None) => {}
        }
    }
    if before.lead_offer_bot_id != after.lead_offer_bot_id {
        match (&after.lead_offer_bot_id, &before.lead_offer_bot_id) {
            (Some(bot_id), _) => events.push(("bot.team.lead.offer", bot_id.clone())),
            (None, Some(previous)) if after.lead_bot_id.as_ref() != Some(previous) => {
                events.push(("bot.team.lead.withdraw", previous.clone()));
            }
            _ => {}
        }
    }
    events
}

/// Audits a change of the team's lead or lead offer; the bot is the one led or offered.
pub(crate) async fn audit_lead(
    state: &AppState,
    user: &AppUser,
    action: &str,
    team_id: &str,
    bot_id: &str,
) {
    audit!(
        state,
        user,
        action,
        "BotTeam",
        team_id,
        serde_json::json!({ "bot_id": bot_id })
    );
}

#[cfg(test)]
mod tests {
    use super::lead_events;
    use crate::entity::bot_team;

    fn team(lead: Option<&str>, offer: Option<&str>) -> bot_team::Model {
        let at = chrono::DateTime::from_timestamp(1_700_000_000, 0)
            .unwrap()
            .fixed_offset();
        bot_team::Model {
            id: "security".into(),
            name: "Security".into(),
            purpose: None,
            parent_id: None,
            lead_bot_id: lead.map(str::to_string),
            lead_offer_bot_id: offer.map(str::to_string),
            lead_offer_by: offer.map(|_| "mara".to_string()),
            lead_offer_at: offer.map(|_| at),
            direct_asks: false,
            conversation_id: None,
            created_by: "mara".into(),
            created_at: at,
            updated_at: at,
        }
    }

    fn events(
        before: (Option<&str>, Option<&str>),
        after: (Option<&str>, Option<&str>),
    ) -> Vec<(&'static str, String)> {
        lead_events(&team(before.0, before.1), &team(after.0, after.1))
    }

    #[test]
    fn lead_changes_name_the_bot_they_concern() {
        assert_eq!(
            events((None, None), (None, Some("vault"))),
            [("bot.team.lead.offer", "vault".to_string())]
        );
        assert_eq!(
            events((None, None), (Some("patch"), None)),
            [("bot.team.lead.set", "patch".to_string())]
        );
        assert_eq!(
            events((Some("patch"), None), (None, None)),
            [("bot.team.lead.clear", "patch".to_string())]
        );
        assert_eq!(
            events((Some("patch"), Some("vault")), (None, None)),
            [
                ("bot.team.lead.clear", "patch".to_string()),
                ("bot.team.lead.withdraw", "vault".to_string())
            ]
        );
        assert_eq!(
            events((None, Some("vault")), (Some("vault"), None)),
            [("bot.team.lead.set", "vault".to_string())],
            "an accepted offer is no withdrawal"
        );
        assert!(events((Some("patch"), None), (Some("patch"), None)).is_empty());
    }
}
