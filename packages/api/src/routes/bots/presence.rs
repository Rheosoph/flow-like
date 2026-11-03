//! Where a person's bots are reachable. A desktop or device host reports the bots it runs every
//! minute with `PUT /bots/hosts/{host_id}`; server bots are always online. A bot on a desktop or
//! device is online while one host saw it within [`ONLINE_WINDOW_SECS`]. Every query is scoped
//! to the caller in SQL.

use super::turns::{TurnHostRequest, checked_host_id};
use crate::{
    entity::{bot, bot_host_heartbeat},
    error::ApiError,
    middleware::jwt::AppUser,
    routes::inbox::now_millis,
    state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, State},
    http::StatusCode,
};
use chrono::{DateTime, Duration, FixedOffset};
use flow_like_types::{bot::BotLocation, bot_turn::TurnHostKind};
use sea_orm::{
    ActiveValue::Set,
    ColumnTrait, ConnectionTrait, EntityTrait, FromQueryResult, QueryFilter, QuerySelect,
    sea_query::{Expr, ExprTrait, OnConflict, Query},
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use utoipa::ToSchema;

/// A desktop or device bot is online this long after a host last saw it: two missed reports of
/// the 60 second rhythm and a margin.
pub(crate) const ONLINE_WINDOW_SECS: i64 = 150;
const MAX_BOTS_PER_REPORT: usize = 100;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum PresenceState {
    /// A host answers for the bot right now.
    Online,
    /// Its host was last seen longer ago than two and a half minutes.
    Offline,
    /// No host ever reported the bot.
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, ToSchema)]
pub struct BotPresence {
    pub state: PresenceState,
    /// When a host last reported the bot; absent for server bots and bots never seen.
    pub last_seen_at: Option<String>,
}

/// The hosts' report of the bots they run.
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct HostReport {
    /// `desktop` or `device`.
    #[schema(example = "desktop")]
    pub host_kind: String,
    /// The bots this host runs, up to 100.
    pub bot_ids: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, ToSchema)]
pub struct HostReportResult {
    /// Bots now reported as seen.
    pub accepted: Vec<String>,
    /// Bots that aren't yours, were deleted or don't run on this kind of host.
    pub refused: Vec<String>,
}

pub(crate) fn presence_of(
    location: BotLocation,
    last_seen: Option<DateTime<FixedOffset>>,
    now: DateTime<FixedOffset>,
) -> BotPresence {
    let state = match (location, last_seen) {
        (BotLocation::Server, _) => PresenceState::Online,
        (_, None) => PresenceState::Unknown,
        (_, Some(seen)) if now - seen <= Duration::seconds(ONLINE_WINDOW_SECS) => {
            PresenceState::Online
        }
        _ => PresenceState::Offline,
    };
    BotPresence {
        state,
        last_seen_at: match location {
            BotLocation::Server => None,
            _ => last_seen.map(|seen| seen.to_rfc3339()),
        },
    }
}

#[derive(Debug, FromQueryResult)]
struct PresenceRow {
    id: String,
    location: String,
    last_seen: Option<DateTime<FixedOffset>>,
}

pub(crate) async fn presence_in<C: ConnectionTrait>(
    db: &C,
    owner_id: &str,
    now: DateTime<FixedOffset>,
) -> Result<BTreeMap<String, BotPresence>, ApiError> {
    let rows = bot::Entity::find()
        .select_only()
        .column(bot::Column::Id)
        .column(bot::Column::Location)
        .column_as(
            Expr::col((
                bot_host_heartbeat::Entity,
                bot_host_heartbeat::Column::LastSeenAt,
            ))
            .max(),
            "last_seen",
        )
        .left_join(bot_host_heartbeat::Entity)
        .filter(bot::Column::OwnerId.eq(owner_id))
        .filter(bot::Column::DeletedAt.is_null())
        .group_by(bot::Column::Id)
        .group_by(bot::Column::Location)
        .into_model::<PresenceRow>()
        .all(db)
        .await?;
    Ok(rows
        .into_iter()
        .map(|row| {
            let location = BotLocation::parse(&row.location).unwrap_or_default();
            (row.id, presence_of(location, row.last_seen, now))
        })
        .collect())
}

fn host_kind(report: &HostReport) -> Result<TurnHostKind, ApiError> {
    match TurnHostKind::parse(&report.host_kind) {
        Some(kind @ (TurnHostKind::Desktop | TurnHostKind::Device)) => Ok(kind),
        _ => Err(ApiError::bad_request(
            "host_kind is desktop or device; server bots need no report",
        )),
    }
}

fn location_of(kind: TurnHostKind) -> BotLocation {
    match kind {
        TurnHostKind::Server => BotLocation::Server,
        TurnHostKind::Desktop => BotLocation::Desktop,
        TurnHostKind::Device => BotLocation::Device,
    }
}

/// Records that `host_id` saw the caller's live bots among `bot_ids` that run on `kind`.
pub(crate) async fn report_in<C: ConnectionTrait>(
    db: &C,
    owner_id: &str,
    host_id: &str,
    kind: TurnHostKind,
    bot_ids: &[String],
    now: DateTime<FixedOffset>,
) -> Result<HostReportResult, ApiError> {
    let wanted: BTreeSet<&String> = bot_ids.iter().collect();
    let matching: Vec<String> = if wanted.is_empty() {
        Vec::new()
    } else {
        bot::Entity::find()
            .select_only()
            .column(bot::Column::Id)
            .filter(bot::Column::Id.is_in(wanted.iter().map(|id| id.as_str())))
            .filter(bot::Column::OwnerId.eq(owner_id))
            .filter(bot::Column::DeletedAt.is_null())
            .filter(bot::Column::Location.eq(location_of(kind).as_str().to_ascii_uppercase()))
            .into_tuple()
            .all(db)
            .await?
    };
    if !matching.is_empty() {
        bot_host_heartbeat::Entity::insert_many(matching.iter().map(|bot_id| {
            bot_host_heartbeat::ActiveModel {
                bot_id: Set(bot_id.clone()),
                host_id: Set(host_id.to_string()),
                host_kind: Set(kind.stored().to_string()),
                last_seen_at: Set(now),
            }
        }))
        .on_conflict(
            OnConflict::columns([
                bot_host_heartbeat::Column::BotId,
                bot_host_heartbeat::Column::HostId,
            ])
            .update_columns([
                bot_host_heartbeat::Column::HostKind,
                bot_host_heartbeat::Column::LastSeenAt,
            ])
            .to_owned(),
        )
        .exec_without_returning(db)
        .await?;
    }
    let accepted: BTreeSet<&String> = matching.iter().collect();
    Ok(HostReportResult {
        refused: wanted
            .iter()
            .filter(|id| !accepted.contains(**id))
            .map(|id| (*id).clone())
            .collect(),
        accepted: matching,
    })
}

/// Forgets the reports of `host_id` for the caller's bots; the number of rows removed.
pub(crate) async fn forget_in<C: ConnectionTrait>(
    db: &C,
    owner_id: &str,
    host_id: &str,
) -> Result<u64, ApiError> {
    let mine = Query::select()
        .column(bot::Column::Id)
        .from(bot::Entity)
        .and_where(Expr::col(bot::Column::OwnerId).eq(owner_id))
        .to_owned();
    Ok(bot_host_heartbeat::Entity::delete_many()
        .filter(bot_host_heartbeat::Column::HostId.eq(host_id))
        .filter(bot_host_heartbeat::Column::BotId.in_subquery(mine))
        .exec(db)
        .await?
        .rows_affected)
}

#[utoipa::path(
    put,
    path = "/bots/hosts/{host_id}",
    tag = "Bots",
    params(("host_id" = String, Path, description = "Names this host: 1 to 128 characters without spaces; the same id the host uses to claim turns")),
    request_body = HostReport,
    responses(
        (status = 200, description = "The bots in `accepted` count as seen now. Send the report every 60 seconds while the host runs; a bot a host stops reporting reads offline after 150 seconds. Bots in `refused` aren't yours, were deleted or run on the other kind of host.", body = HostReportResult),
        (status = 400, description = "host_id is empty, too long or has spaces; host_kind is not desktop or device; more than 100 bots"),
        (status = 401, description = "Not signed in")
    ),
    security(("bearer_auth" = []))
)]
#[tracing::instrument(name = "PUT /bots/hosts/{host_id}", skip(state, user, report))]
pub async fn report_host(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(host_id): Path<String>,
    Json(report): Json<HostReport>,
) -> Result<Json<HostReportResult>, ApiError> {
    let sub = user.sub()?;
    let host_id = checked_host_id(TurnHostRequest { host_id })?;
    let kind = host_kind(&report)?;
    if report.bot_ids.len() > MAX_BOTS_PER_REPORT {
        return Err(ApiError::bad_request(format!(
            "A report names at most {MAX_BOTS_PER_REPORT} bots"
        )));
    }
    Ok(Json(
        report_in(
            &state.db,
            &sub,
            &host_id,
            kind,
            &report.bot_ids,
            now_millis(),
        )
        .await?,
    ))
}

#[utoipa::path(
    delete,
    path = "/bots/hosts/{host_id}",
    tag = "Bots",
    params(("host_id" = String, Path, description = "The host that signs out or shuts down")),
    responses(
        (status = 204, description = "Your bots stop reading online because of this host (or it never reported)"),
        (status = 400, description = "host_id is empty, too long or has spaces"),
        (status = 401, description = "Not signed in")
    ),
    security(("bearer_auth" = []))
)]
#[tracing::instrument(name = "DELETE /bots/hosts/{host_id}", skip(state, user))]
pub async fn forget_host(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(host_id): Path<String>,
) -> Result<StatusCode, ApiError> {
    let sub = user.sub()?;
    let host_id = checked_host_id(TurnHostRequest { host_id })?;
    forget_in(&state.db, &sub, &host_id).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(
    get,
    path = "/bots/presence",
    tag = "Bots",
    responses(
        (status = 200, description = "For each of your bots: `online` (a server bot, or a desktop or device bot a host reported within the last 150 seconds), `offline` (reported earlier) or `unknown` (never reported), and when a host last reported it.", body = HashMap<String, BotPresence>),
        (status = 401, description = "Not signed in")
    ),
    security(("bearer_auth" = []))
)]
#[tracing::instrument(name = "GET /bots/presence", skip(state, user))]
pub async fn get_presence(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
) -> Result<Json<BTreeMap<String, BotPresence>>, ApiError> {
    let sub = user.sub()?;
    Ok(Json(presence_in(&state.db, &sub, now_millis()).await?))
}
