//! Bot teams: people and bots in nested teams with one lead bot per team, and a room for each
//! team. The tables and their contract live in `flow_like_types::bot_team`; the coordinator that
//! reads them for the bot runtime is `bot_host::teams`.

use crate::{
    db::coordination::coordinate,
    entity::{bot_team, bot_team_bot, bot_team_chat, bot_team_thread},
    routes::{bots::lock_bot, inbox::team_room::lock_room},
    state::AppState,
};
use axum::{
    Router,
    routing::{delete, get, post, put},
};
use sea_orm::{ColumnTrait, DatabaseTransaction, DbErr, EntityTrait, QueryFilter};

pub mod apps;
pub(crate) mod access;
pub mod bots;
pub mod chats;
pub mod classification;
pub(crate) mod effects;
mod inputs;
pub mod lead;
pub(crate) mod levels;
pub mod model;
pub(crate) mod org_apps;
pub mod people;
pub mod preview;
pub(crate) mod reach;
pub(crate) mod room;
pub mod teams;
pub(crate) mod threads;
pub(crate) mod view;

#[cfg(test)]
mod apps_tests;
#[cfg(test)]
mod brief_tests;
#[cfg(test)]
mod chats_tests;
#[cfg(test)]
mod reach_tests;
#[cfg(test)]
mod request_tests;
#[cfg(test)]
mod tests;

/// Takes the locks a team change needs: the teams, then the bots and the teams those bots are
/// active in, then every room of them, each group in id order.
pub(crate) async fn lock_scope(
    txn: &DatabaseTransaction,
    team_ids: &[&str],
    bot_ids: &[&str],
) -> Result<(), DbErr> {
    let mut teams: Vec<String> = team_ids.iter().map(|id| id.to_string()).collect();
    let mut bots: Vec<String> = bot_ids.iter().map(|id| id.to_string()).collect();
    bots.sort();
    bots.dedup();
    for team_id in sorted(&mut teams) {
        coordinate(txn, "bot-team", &[team_id]).await?;
    }
    for bot_id in &bots {
        lock_bot(txn, bot_id).await?;
    }
    let active_rows = bot_team_bot::Entity::find()
        .filter(bot_team_bot::Column::BotId.is_in(bots.clone()))
        .filter(bot_team_bot::Column::Status.eq(access::active()))
        .all(txn)
        .await?;
    teams.extend(active_rows.into_iter().map(|row| row.team_id));
    for team_id in sorted(&mut teams) {
        coordinate(txn, "bot-team", &[team_id]).await?;
    }
    let mut rooms: Vec<String> = bot_team::Entity::find()
        .filter(bot_team::Column::Id.is_in(teams.clone()))
        .all(txn)
        .await?
        .into_iter()
        .filter_map(|team| team.conversation_id)
        .collect();
    for room_id in sorted(&mut rooms) {
        lock_room(txn, room_id).await?;
    }
    let mut threads: Vec<String> = bot_team_thread::Entity::find()
        .filter(bot_team_thread::Column::TeamId.is_in(teams))
        .all(txn)
        .await?
        .into_iter()
        .map(|thread| thread.conversation_id)
        .collect();
    for thread_id in sorted(&mut threads) {
        lock_room(txn, thread_id).await?;
    }
    let mut chats: Vec<String> = bot_team_chat::Entity::find()
        .filter(bot_team_chat::Column::BotId.is_in(bots))
        .all(txn)
        .await?
        .into_iter()
        .filter_map(|chat| chat.conversation_id)
        .collect();
    for chat_id in sorted(&mut chats) {
        lock_room(txn, chat_id).await?;
    }
    Ok(())
}

/// [`lock_scope`] for a change that may prune bots anywhere in the orgs of `team_ids`: every
/// team of those orgs and every bot in them.
pub(crate) async fn lock_org_scope(
    txn: &DatabaseTransaction,
    team_ids: &[&str],
) -> Result<(), DbErr> {
    let mut ids: Vec<String> = Vec::new();
    for team_id in team_ids {
        ids.push(team_id.to_string());
        if let Some(team) = bot_team::Entity::find_by_id(*team_id).one(txn).await? {
            let org = access::org_of(txn, &team).await?;
            ids.extend(org.ids());
        }
    }
    ids.sort();
    ids.dedup();
    let bots: Vec<String> = bot_team_bot::Entity::find()
        .filter(bot_team_bot::Column::TeamId.is_in(ids.clone()))
        .all(txn)
        .await?
        .into_iter()
        .map(|row| row.bot_id)
        .collect();
    let team_refs: Vec<&str> = ids.iter().map(String::as_str).collect();
    let bot_refs: Vec<&str> = bots.iter().map(String::as_str).collect();
    lock_scope(txn, &team_refs, &bot_refs).await
}

fn sorted(ids: &mut Vec<String>) -> &[String] {
    ids.sort();
    ids.dedup();
    ids
}

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/", get(teams::list_org).post(teams::create_team))
        .route("/bots/{bot_id}/briefing", get(bots::briefing))
        .route("/chats", post(chats::request_chat))
        .route("/chats/{bot_id}/{user_id}", delete(chats::end_chat))
        .route("/chats/{bot_id}/{user_id}/accept", post(chats::accept_chat))
        .route(
            "/chats/{bot_id}/{user_id}/decline",
            post(chats::decline_chat),
        )
        .route(
            "/{team_id}",
            axum::routing::patch(teams::patch_team).delete(teams::delete_team),
        )
        .route("/{team_id}/people", post(people::add_people))
        .route("/{team_id}/preview", post(preview::preview))
        .route("/{team_id}/thread", post(threads::open_thread))
        .route("/{team_id}/join", post(people::join_team))
        .route("/{team_id}/decline", post(people::decline_team))
        .route("/{team_id}/people/{user_id}/role", put(people::set_role))
        .route("/{team_id}/people/{user_id}", delete(people::remove_person))
        .route("/{team_id}/apps/pinnable", get(apps::list_pinnable))
        .route("/{team_id}/apps", post(apps::pin_app))
        .route("/{team_id}/apps/{app_id}", delete(apps::unpin_app))
        .route("/{team_id}/apps/{app_id}/approve", post(apps::approve_pin))
        .route("/{team_id}/apps/{app_id}/decline", post(apps::decline_pin))
        .route("/{team_id}/bots", post(bots::add_bot))
        .route("/{team_id}/bots/{bot_id}", delete(bots::remove_bot))
        .route("/{team_id}/bots/{bot_id}/accept", post(bots::accept_bot))
        .route("/{team_id}/bots/{bot_id}/decline", post(bots::decline_bot))
        .route("/{team_id}/bots/{bot_id}/direct", put(bots::set_direct))
        .route(
            "/{team_id}/bots/{bot_id}/lead/accept",
            post(lead::accept_lead),
        )
        .route(
            "/{team_id}/bots/{bot_id}/lead/decline",
            post(lead::decline_lead),
        )
        .route(
            "/{team_id}/bots/{bot_id}/lead/resign",
            post(lead::resign_lead),
        )
        .route(
            "/{team_id}/classifications/{user_id}",
            put(classification::set_classification),
        )
}
