//! A bot's own mail address. It is a row of `InboundMailAddress` with `kind = 'bot'`, `appId =
//! 'bot'` and the bot's id in `eventId`, so the table's primary key makes it unique against every
//! event alias and primary address at once. Releasing it quarantines the address like a deleted
//! event's, and only the same bot may take it back until the quarantine ends.

use super::{
    address::{domain, normalize_alias},
    digest,
    limits::{self, Counter, Window},
    now, statement,
};
use crate::{
    entity::bot,
    error::ApiError,
    middleware::jwt::AppUser,
    routes::bots::{lock_bot, owned_bot},
    state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, State},
    http::StatusCode,
};
use sea_orm::{ConnectionTrait, FromQueryResult};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

const DAILY_ADDRESS_CHANGES: u32 = 10;
pub(crate) const BOT_KIND: &str = "bot";
pub(crate) const BOT_APP: &str = "bot";
pub(crate) const EMAIL_CAPABILITY: &str = "email";

#[derive(Serialize, ToSchema)]
pub struct BotAddressView {
    /// Whether this server receives email at all.
    configured: bool,
    domain: Option<String>,
    /// The bot's address, when it has claimed one.
    address: Option<String>,
    /// Local part of the address.
    alias: Option<String>,
    /// Whether the bot has the `email` capability switched on.
    enabled: bool,
}

#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct BotAddressClaim {
    /// Local part to claim: 3-64 lowercase letters, numbers or hyphens.
    alias: String,
}

#[derive(Serialize, ToSchema)]
pub struct BotAddressAvailability {
    available: bool,
    /// Why the name cannot be used.
    reason: Option<String>,
}

#[derive(FromQueryResult)]
struct Held {
    address: String,
}

pub(crate) fn has_email(bot: &bot::Model) -> bool {
    bot.capabilities
        .iter()
        .any(|c| c.as_str() == EMAIL_CAPABILITY)
}

pub(crate) async fn address_of<C: ConnectionTrait>(
    db: &C,
    bot_id: &str,
) -> Result<Option<String>, ApiError> {
    Ok(Held::find_by_statement(statement(
        r#"SELECT "address" FROM "InboundMailAddress" WHERE "kind"='bot' AND "appId"='bot' AND "eventId"=$1 AND "active"=TRUE"#,
        vec![bot_id.into()],
    ))
    .one(db)
    .await?
    .map(|held| held.address))
}

/// The bot an active address belongs to, deleted bots excluded.
pub(crate) async fn bot_of_address<C: ConnectionTrait>(
    db: &C,
    address: &str,
) -> Result<Option<String>, ApiError> {
    #[derive(FromQueryResult)]
    struct Owner {
        #[sea_orm(from_alias = "eventId")]
        event_id: String,
    }
    Ok(Owner::find_by_statement(statement(
        r#"SELECT a."eventId" FROM "InboundMailAddress" a JOIN "Bot" b ON b."id"=a."eventId" WHERE a."address"=$1 AND a."kind"='bot' AND a."appId"='bot' AND a."active"=TRUE AND b."deletedAt" IS NULL"#,
        vec![address.into()],
    ))
    .one(db)
    .await?
    .map(|owner| owner.event_id))
}

fn view(state: &AppState, bot: &bot::Model, address: Option<String>) -> BotAddressView {
    let domain = domain(state);
    let alias = address
        .as_deref()
        .and_then(|a| a.split_once('@'))
        .map(|(local, _)| local.to_owned());
    BotAddressView {
        configured: domain.is_some(),
        domain,
        address,
        alias,
        enabled: has_email(bot),
    }
}

async fn taken_by_other(
    state: &AppState,
    bot_id: &str,
    address: &str,
) -> Result<Option<String>, ApiError> {
    #[derive(FromQueryResult)]
    struct Row {
        kind: String,
        #[sea_orm(from_alias = "appId")]
        app_id: String,
        #[sea_orm(from_alias = "eventId")]
        event_id: String,
    }
    let row = Row::find_by_statement(statement(
        r#"SELECT "kind","appId","eventId" FROM "InboundMailAddress" WHERE "address"=$1"#,
        vec![address.into()],
    ))
    .one(&state.db)
    .await?;
    Ok(row.and_then(|row| {
        let ours = row.kind == "quarantined" && row.app_id == BOT_APP && row.event_id == bot_id;
        let mine = row.kind == BOT_KIND && row.app_id == BOT_APP && row.event_id == bot_id;
        if ours || mine {
            None
        } else if row.kind == "quarantined" {
            Some("This address was used recently and is reserved for a while".to_owned())
        } else {
            Some("This address is already taken".to_owned())
        }
    }))
}

fn full_address(state: &AppState, alias: &str) -> Result<String, ApiError> {
    let domain = domain(state)
        .ok_or_else(|| ApiError::bad_request("Inbound email is not configured on this server"))?;
    Ok(format!("{alias}@{domain}"))
}

#[utoipa::path(
    get,
    path = "/bots/{bot_id}/email-address",
    tag = "Bots",
    description = "Show the bot's own email address, if it has claimed one.",
    params(("bot_id" = String, Path, description = "Your bot's id")),
    responses(
        (status = 200, description = "The bot's email address", body = BotAddressView),
        (status = 401, description = "Not signed in"),
        (status = 404, description = "You have no bot with this id")
    ),
    security(("bearer_auth" = []))
)]
pub async fn get_bot_address(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(bot_id): Path<String>,
) -> Result<Json<BotAddressView>, ApiError> {
    let sub = user.sub()?;
    let bot = owned_bot(&state.db, &sub, &bot_id).await?;
    let address = address_of(&state.db, &bot.id).await?;
    Ok(Json(view(&state, &bot, address)))
}

#[utoipa::path(
    get,
    path = "/bots/{bot_id}/email-address/check/{alias}",
    tag = "Bots",
    description = "Check whether a local part is free for this bot's address.",
    params(
        ("bot_id" = String, Path, description = "Your bot's id"),
        ("alias" = String, Path, description = "Local part to check")
    ),
    responses(
        (status = 200, description = "Whether the name can be claimed", body = BotAddressAvailability),
        (status = 401, description = "Not signed in"),
        (status = 404, description = "You have no bot with this id")
    ),
    security(("bearer_auth" = []))
)]
pub async fn check_bot_address(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((bot_id, alias)): Path<(String, String)>,
) -> Result<Json<BotAddressAvailability>, ApiError> {
    let sub = user.sub()?;
    let bot = owned_bot(&state.db, &sub, &bot_id).await?;
    let verdict = match normalize_alias(&alias).and_then(|alias| full_address(&state, &alias)) {
        Err(error) => Some(error.to_string()),
        Ok(address) => taken_by_other(&state, &bot.id, &address).await?,
    };
    Ok(Json(BotAddressAvailability {
        available: verdict.is_none(),
        reason: verdict,
    }))
}

struct Change {
    window: Window,
    subject: String,
}

impl Change {
    fn counters(&self) -> [Counter<'_>; 1] {
        [Counter {
            scope: "bot-address",
            subject: &self.subject,
            limit: DAILY_ADDRESS_CHANGES,
        }]
    }

    async fn reserve(state: &AppState, bot_id: &str) -> Result<Self, ApiError> {
        let change = Self {
            window: Window::at(now()),
            subject: digest(&["bot-address", bot_id]),
        };
        if limits::consume(state, &change.window, &change.counters(), 1)
            .await?
            .is_some()
        {
            return Err(ApiError::too_many_requests(format!(
                "A bot's email address can change at most {DAILY_ADDRESS_CHANGES} times per day"
            )));
        }
        Ok(change)
    }

    async fn refund(&self, state: &AppState) {
        limits::refund(state, &self.window, &self.counters(), 1).await;
    }
}

pub(crate) async fn release_in<C: ConnectionTrait>(db: &C, bot_id: &str) -> Result<u64, ApiError> {
    Ok(db
        .execute_raw(statement(
            r#"UPDATE "InboundMailAddress" SET "kind"='quarantined',"active"=FALSE,"updatedAt"=$1 WHERE "kind"='bot' AND "appId"='bot' AND "eventId"=$2"#,
            vec![now().into(), bot_id.into()],
        ))
        .await?
        .rows_affected())
}

/// Gives the bot `address` in place of its current one. Only a free address, or one this bot
/// released, can be taken; anything else is a conflict and nothing changes.
pub(crate) async fn claim_in<C: ConnectionTrait>(
    db: &C,
    bot_id: &str,
    address: &str,
) -> Result<(), ApiError> {
    release_in(db, bot_id).await?;
    let timestamp = now();
    let result = db
        .execute_raw(statement(
            r#"INSERT INTO "InboundMailAddress" ("address","appId","eventId","kind","active","createdAt","updatedAt") VALUES ($1,'bot',$2,'bot',TRUE,$3,$3) ON CONFLICT ("address") DO UPDATE SET "kind"='bot',"active"=TRUE,"updatedAt"=$3 WHERE "InboundMailAddress"."kind"='quarantined' AND "InboundMailAddress"."appId"='bot' AND "InboundMailAddress"."eventId"=$2"#,
            vec![address.into(), bot_id.into(), timestamp.into()],
        ))
        .await?;
    if result.rows_affected() != 1 {
        return Err(ApiError::conflict("This email address is already taken"));
    }
    Ok(())
}

#[utoipa::path(
    put,
    path = "/bots/{bot_id}/email-address",
    tag = "Bots",
    description = "Claim an email address for the bot, or change it. The bot needs the email capability. A released address stays reserved for 30 days; a bot's address can change ten times per day.",
    params(("bot_id" = String, Path, description = "Your bot's id")),
    request_body = BotAddressClaim,
    responses(
        (status = 200, description = "The bot's email address", body = BotAddressView),
        (status = 400, description = "Invalid or reserved name, or the bot lacks the email capability"),
        (status = 401, description = "Not signed in"),
        (status = 404, description = "You have no bot with this id"),
        (status = 409, description = "The address is taken"),
        (status = 429, description = "The address changed too often today")
    ),
    security(("bearer_auth" = []))
)]
pub async fn claim_bot_address(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(bot_id): Path<String>,
    Json(input): Json<BotAddressClaim>,
) -> Result<Json<BotAddressView>, ApiError> {
    let sub = user.sub()?;
    let bot = owned_bot(&state.db, &sub, &bot_id).await?;
    if !has_email(&bot) {
        return Err(ApiError::bad_request(
            "Switch on the email ability before claiming an address",
        ));
    }
    let address = full_address(&state, &normalize_alias(&input.alias)?)?;
    let current = address_of(&state.db, &bot.id).await?;
    if current.as_deref() == Some(address.as_str()) {
        return Ok(Json(view(&state, &bot, current)));
    }
    let change = Change::reserve(&state, &bot.id).await?;
    let claimed = state
        .transaction(|txn| {
            let (owner, bot_id, address) = (sub.clone(), bot.id.clone(), address.clone());
            Box::pin(async move {
                lock_bot(txn, &bot_id).await?;
                owned_bot(txn, &owner, &bot_id).await?;
                claim_in(txn, &bot_id, &address).await?;
                Ok::<_, ApiError>(())
            })
        })
        .await;
    if let Err(error) = claimed {
        change.refund(&state).await;
        return Err(error);
    }
    crate::audit!(state, user, "bot.email-address.claim", "Bot", bot.id);
    Ok(Json(view(&state, &bot, Some(address))))
}

#[utoipa::path(
    delete,
    path = "/bots/{bot_id}/email-address",
    tag = "Bots",
    description = "Release the bot's email address. Mail to it is no longer delivered; the address stays reserved for 30 days so nobody else receives mail meant for the bot.",
    params(("bot_id" = String, Path, description = "Your bot's id")),
    responses(
        (status = 204, description = "The address was released"),
        (status = 401, description = "Not signed in"),
        (status = 404, description = "You have no bot with this id")
    ),
    security(("bearer_auth" = []))
)]
pub async fn release_bot_address(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(bot_id): Path<String>,
) -> Result<StatusCode, ApiError> {
    let sub = user.sub()?;
    let bot = owned_bot(&state.db, &sub, &bot_id).await?;
    release_in(&state.db, &bot.id).await?;
    crate::audit!(state, user, "bot.email-address.release", "Bot", bot.id);
    Ok(StatusCode::NO_CONTENT)
}

/// Marks addresses of bots that are gone; they stay reserved during the quarantine.
pub(super) async fn quarantine_gone_bots(
    state: &AppState,
    timestamp: i64,
    batch: u32,
) -> Result<u64, ApiError> {
    Ok(state
        .db
        .execute_raw(statement(
            &format!(
                r#"UPDATE "InboundMailAddress" SET "kind"='quarantined',"active"=FALSE,"updatedAt"=$1 WHERE "address" IN (SELECT a."address" FROM "InboundMailAddress" a WHERE a."kind"='bot' AND a."appId"='bot' AND NOT EXISTS (SELECT 1 FROM "Bot" b WHERE b."id"=a."eventId" AND b."deletedAt" IS NULL) LIMIT {batch})"#
            ),
            vec![timestamp.into()],
        ))
        .await?
        .rows_affected())
}
