//! Bots: assistants that act on behalf of their owner. Every person has Assistant and Flux and
//! may create more. Only the owner manages a bot: someone else's bot and a deleted bot both
//! read as missing, and every query is scoped to the caller in SQL.

use crate::{entity::bot, error::ApiError, state::AppState};
use axum::{
    Router,
    extract::DefaultBodyLimit,
    routing::{delete, get, post, put},
};
use sea_orm::{ColumnTrait, ConnectionTrait, DatabaseTransaction, DbErr, EntityTrait, QueryFilter};

pub mod apps;
pub mod ask;
pub mod avatar;
pub mod clone;
pub mod crud;
pub mod fire;
pub mod import;
pub mod mail;
pub mod live_activities;
pub mod memory;
pub mod model;
pub mod notify;
pub mod people;
pub mod permissions;
pub mod presence;
mod provision;
pub(crate) mod runtime;
pub mod runtime_token;
mod secret_cipher;
pub mod secrets;
pub mod talk;
pub mod tasks;
pub mod tool_grants;
pub mod turns;
mod validation;
pub mod webhooks;
pub mod workspace;

#[cfg(test)]
mod import_tests;
#[cfg(test)]
mod permissions_tests;
#[cfg(test)]
mod presence_tests;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod webhooks_tests;
#[cfg(test)]
mod workspace_tests;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/", get(crud::list_bots).post(crud::create_bot))
        .route("/import", post(import::import_bot))
        .route("/import/check", post(import::check_bot_import))
        .route("/presence", get(presence::get_presence))
        .route("/mobile-status", get(live_activities::mobile_status))
        .route(
            "/hosts/{host_id}",
            put(presence::report_host).delete(presence::forget_host),
        )
        .route(
            "/hooks/{hook_id}",
            post(webhooks::call_bot_webhook)
                .layer(DefaultBodyLimit::max(webhooks::MAX_PAYLOAD_BYTES + 1)),
        )
        .route("/turns", get(turns::list_turns))
        .route("/turns/{turn_id}/claim", post(turns::claim_turn))
        .route("/turns/{turn_id}/heartbeat", post(turns::heartbeat_turn))
        .route("/turns/{turn_id}/silent", post(turns::silent_turn))
        .route("/turns/{turn_id}/hold", post(turns::hold_turn))
        .route("/turns/{turn_id}/park", post(tool_grants::park_turn))
        .route(
            "/turns/{turn_id}/secrets/{name}",
            post(secrets::read_turn_secret),
        )
        .route(
            "/{bot_id}",
            get(crud::get_bot)
                .patch(crud::update_bot)
                .delete(crud::delete_bot),
        )
        .route("/{bot_id}/clone", post(clone::clone_bot))
        .route(
            "/{bot_id}/avatar",
            get(avatar::get_bot_avatar).put(avatar::put_bot_avatar),
        )
        .route(
            "/{bot_id}/runtime-token",
            post(runtime_token::mint_runtime_token),
        )
        .route("/{bot_id}/memory", delete(memory::reset_memory))
        .route("/{bot_id}/memory/facts", get(memory::list_facts))
        .route(
            "/{bot_id}/memory/facts/{fact_id}",
            delete(memory::forget_fact),
        )
        .route(
            "/{bot_id}/email-address",
            get(crate::mail_ingress::bot_address::get_bot_address)
                .put(crate::mail_ingress::bot_address::claim_bot_address)
                .delete(crate::mail_ingress::bot_address::release_bot_address),
        )
        .route(
            "/{bot_id}/mail/send",
            post(mail::send_bot_mail).layer(axum::extract::DefaultBodyLimit::max(
                crate::routes::app::mail::MAX_REQUEST_BYTES,
            )),
        )
        .route(
            "/{bot_id}/mail/reply",
            post(mail::reply_bot_mail).layer(axum::extract::DefaultBodyLimit::max(
                crate::routes::app::mail::MAX_REQUEST_BYTES,
            )),
        )
        .route("/{bot_id}/mail/threads", get(mail::list_mail_threads))
        .route(
            "/{bot_id}/mail/threads/{thread_id}",
            get(mail::get_mail_thread),
        )
        .route(
            "/{bot_id}/email-address/check/{alias}",
            get(crate::mail_ingress::bot_address::check_bot_address),
        )
        .route("/{bot_id}/ask", post(ask::ask_bot))
        .route("/{bot_id}/talk", post(talk::talk))
        .route("/{bot_id}/talk/exchange", post(talk::talk_exchange))
        .route("/{bot_id}/talk/roster", get(talk::talk_roster))
        .route(
            "/{bot_id}/turns/{turn_id}/ask-result",
            get(talk::turn_ask_result),
        )
        .route(
            "/{bot_id}/webhooks",
            get(webhooks::list_bot_webhooks).post(webhooks::create_bot_webhook),
        )
        .route(
            "/{bot_id}/webhooks/{hook_id}",
            axum::routing::patch(webhooks::update_bot_webhook).delete(webhooks::delete_bot_webhook),
        )
        .route(
            "/{bot_id}/webhooks/{hook_id}/rotate",
            post(webhooks::rotate_bot_webhook),
        )
        .route(
            "/{bot_id}/tasks",
            get(tasks::list_bot_tasks).post(tasks::create_bot_task),
        )
        .route(
            "/{bot_id}/tasks/{task_id}",
            axum::routing::patch(tasks::update_bot_task).delete(tasks::delete_bot_task),
        )
        .route("/{bot_id}/tasks/{task_id}/fire", post(tasks::fire_bot_task))
        .route("/{bot_id}/turns/{turn_id}/retry", post(turns::retry_turn))
        .route("/{bot_id}/turns/{turn_id}/cancel", post(turns::cancel_turn))
        .route(
            "/{bot_id}/turns/{turn_id}/approval",
            post(tool_grants::answer_approval),
        )
        .route(
            "/{bot_id}/tool-grants",
            get(tool_grants::list_tool_grants)
                .put(tool_grants::put_tool_grant)
                .delete(tool_grants::delete_tool_grant),
        )
        .route(
            "/{bot_id}/permissions",
            get(permissions::list_bot_permissions),
        )
        .route(
            "/{bot_id}/permissions/{to_bot_id}",
            put(permissions::grant_bot_permission).delete(permissions::revoke_bot_permission),
        )
        .route("/{bot_id}/live-activities", post(live_activities::register))
        .route("/{bot_id}/live-activities/{activity_id}", delete(live_activities::unregister))
        .route("/{bot_id}/notify", post(notify::message_owner))
        .route("/{bot_id}/people", get(people::list_bot_people))
        .route("/{bot_id}/people/message", post(people::message_person))
        .route("/{bot_id}/apps", get(apps::list_bot_apps))
        .route(
            "/{bot_id}/apps/{app_id}",
            put(apps::attach_bot_app).delete(apps::detach_bot_app),
        )
        .route(
            "/{bot_id}/apps/{app_id}/restrictions",
            get(apps::get_bot_app_restrictions).put(apps::set_bot_app_restrictions),
        )
        .route("/{bot_id}/secrets", get(secrets::list_bot_secrets))
        .route(
            "/{bot_id}/secrets/{name}",
            put(secrets::upsert_bot_secret).delete(secrets::delete_bot_secret),
        )
        .route("/{bot_id}/workspace", post(workspace::ensure_bot_workspace))
        .route(
            "/{bot_id}/workspace/upload",
            post(workspace::upload_to_bot_workspace),
        )
}

/// The caller's bot `bot_id` if it is not deleted. Someone else's bot reads as 404 too, so the
/// answer never tells whether a bot exists. Never trust the `bot_` prefix; use this.
pub(crate) async fn owned_bot<C: ConnectionTrait>(
    db: &C,
    owner_id: &str,
    bot_id: &str,
) -> Result<bot::Model, ApiError> {
    bot::Entity::find_by_id(bot_id)
        .filter(bot::Column::OwnerId.eq(owner_id))
        .filter(bot::Column::DeletedAt.is_null())
        .one(db)
        .await?
        .ok_or_else(|| ApiError::not_found(format!("Bot '{bot_id}' not found")))
}

/// Bots with these ids, deleted ones included, so past actions keep rendering as their bot.
pub(crate) async fn bots_by_id<C: ConnectionTrait>(
    db: &C,
    ids: Vec<String>,
) -> Result<Vec<bot::Model>, DbErr> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    bot::Entity::find()
        .filter(bot::Column::Id.is_in(ids))
        .all(db)
        .await
}

/// Serializes the writes on one bot (deletion, app attachments, secrets) across replicas.
/// Call it first in the transaction, then re-read the bot with [`owned_bot`].
pub(crate) async fn lock_bot(txn: &DatabaseTransaction, bot_id: &str) -> Result<(), DbErr> {
    crate::db::coordination::coordinate(txn, "bot", &[bot_id]).await
}
