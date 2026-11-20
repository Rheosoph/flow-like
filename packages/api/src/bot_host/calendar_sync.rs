//! Planning the wake-ups of the owner's connected calendars. Every [`CALENDAR_SYNC_EVERY_SECS`] a
//! host reads each server bot's Google, Microsoft and CalDAV calendars, in one job on the executor
//! per bot (`bot_tools::calendar_jobs`), and keeps one event task per event that wakes the bot
//! (see `flow_like_bot_tools::calendar::sync_wakes`). Without an executor the connected calendars
//! read as unreadable and their rows stay as they are. A bot is claimed with a
//! compare-and-set on `calendarSyncedAt`, so several hosts and sweeps plan each bot once per
//! period. Desktop bots plan their own calendars on the desktop.

use super::calendar_tasks::holds_calendar;
use crate::{
    bot_tools::{connectors::ApiConnectors, schedule::ApiScheduler},
    entity::{bot, bot_connector},
    error::ApiError,
    routes::inbox::now_millis,
    state::AppState,
};
use chrono::{DateTime, Duration, FixedOffset, Utc};
use flow_like_types::{
    bot::{BotCapability, BotLocation},
    bot_calendar::{CALENDAR_PROVIDERS, CALENDAR_SYNC_EVERY_SECS},
    bot_connector::ConnectorKind,
    tokio,
};
use sea_orm::{
    ColumnTrait, Condition, ConnectionTrait, EntityTrait, QueryFilter, QueryOrder, QuerySelect,
    QueryTrait,
    sea_query::{Expr, ExprTrait},
};

const SYNC_BATCH: u64 = 10;
const SYNC_BUDGET: std::time::Duration = std::time::Duration::from_secs(45);

fn calendar_connectors() -> sea_orm::sea_query::SelectStatement {
    let mut providers = Condition::any();
    for provider in CALENDAR_PROVIDERS {
        providers = providers.add(bot_connector::Column::ProviderId.like(format!("%{provider}%")));
    }
    bot_connector::Entity::find()
        .select_only()
        .column(bot_connector::Column::BotId)
        .filter(bot_connector::Column::Kind.is_in([
            ConnectorKind::Oauth.column(),
            ConnectorKind::Caldav.column(),
        ]))
        .filter(providers)
        .into_query()
}

/// Bots whose connected calendars are due for planning: live, not desktop bots, holding
/// `calendar`, with a Google, Microsoft or CalDAV connector, planned never or before `stale`.
pub(crate) async fn due_in<C: ConnectionTrait>(
    db: &C,
    stale: DateTime<FixedOffset>,
) -> Result<Vec<bot::Model>, ApiError> {
    Ok(bot::Entity::find()
        .filter(bot::Column::DeletedAt.is_null())
        .filter(bot::Column::Location.ne(BotLocation::Desktop.as_str().to_ascii_uppercase()))
        .filter(
            Expr::col(bot::Column::Capabilities)
                .cast_as("text")
                .like(format!("%\"{}\"%", BotCapability::Calendar.as_str())),
        )
        .filter(bot::Column::Id.in_subquery(calendar_connectors()))
        .filter(
            Condition::any()
                .add(bot::Column::CalendarSyncedAt.is_null())
                .add(bot::Column::CalendarSyncedAt.lte(stale)),
        )
        .order_by_asc(bot::Column::Id)
        .limit(SYNC_BATCH)
        .all(db)
        .await?)
}

/// Marks the bot as planned at `now`, once: `false` when another host claimed it first.
async fn claim_in<C: ConnectionTrait>(
    db: &C,
    bot: &bot::Model,
    now: DateTime<FixedOffset>,
) -> Result<bool, ApiError> {
    let claimed = bot::Entity::update_many()
        .col_expr(bot::Column::CalendarSyncedAt, Expr::value(now))
        .filter(bot::Column::Id.eq(bot.id.as_str()))
        .filter(match bot.calendar_synced_at {
            Some(at) => bot::Column::CalendarSyncedAt.eq(at),
            None => bot::Column::CalendarSyncedAt.is_null(),
        })
        .exec(db)
        .await?;
    Ok(claimed.rows_affected == 1)
}

async fn sync_bot(state: &AppState, bot: &bot::Model) {
    let connectors = ApiConnectors::new(state, &bot.owner_id, &bot.id);
    let scheduler = ApiScheduler::new(state, &bot.owner_id, &bot.id, None);
    match flow_like_bot_tools::calendar::sync_wakes(&connectors, &scheduler, Utc::now()).await {
        Ok(report) => {
            tracing::debug!(bot_id = %bot.id, ?report, "Bot calendar planned")
        }
        Err(error) => {
            tracing::warn!(%error, bot_id = %bot.id, "Bot calendar not planned; the next sync tries again")
        }
    }
}

/// Plans the calendars of up to [`SYNC_BATCH`] bots that are due; returns how many this pass
/// planned. A bot whose settings keep connected calendars off is marked without reading them.
pub(crate) async fn sync_due(state: &AppState) -> Result<usize, ApiError> {
    let now = now_millis();
    let stale = now - Duration::seconds(CALENDAR_SYNC_EVERY_SECS);
    let mut planned = 0;
    for bot in due_in(&state.db, stale).await? {
        if !claim_in(&state.db, &bot, now).await? {
            continue;
        }
        if !holds_calendar(&bot) {
            continue;
        }
        sync_bot(state, &bot).await;
        planned += 1;
    }
    Ok(planned)
}

/// [`sync_due`] within a time budget; errors are logged, never returned.
pub(crate) async fn sync_due_logged(state: &AppState) {
    match tokio::time::timeout(SYNC_BUDGET, sync_due(state)).await {
        Ok(Ok(0)) | Err(_) => {}
        Ok(Ok(planned)) => tracing::info!(planned, "Bot calendars planned"),
        Ok(Err(error)) => tracing::warn!(%error, "Bot calendar sync incomplete"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bot_host::fixture::{database, insert_bot};

    #[tokio::test]
    async fn a_bot_is_planned_once_per_period() {
        let db = database().await;
        insert_bot(&db, "bot_a", "owner", "SERVER").await;
        let bot = bot::Entity::find_by_id("bot_a")
            .one(&db)
            .await
            .unwrap()
            .unwrap();
        let now = now_millis();
        assert!(claim_in(&db, &bot, now).await.unwrap());
        assert!(!claim_in(&db, &bot, now).await.unwrap());

        let claimed = bot::Entity::find_by_id("bot_a")
            .one(&db)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(claimed.calendar_synced_at, Some(now));
        assert!(
            claim_in(&db, &claimed, now + Duration::minutes(16))
                .await
                .unwrap()
        );
    }
}
