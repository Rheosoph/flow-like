//! Bot activity for the admin dashboard: the turns bots answered over the
//! period, the bots people use most, where turns ran, why they failed, and
//! what each bot spent in the current budget month.

use super::usage::{display_name, load_users, to_count, trend_label};
use crate::{
    entity::{bot, bot_turn},
    error::ApiError,
    middleware::jwt::AppUser,
    permission::global_permission::GlobalPermission,
    quota::{QuotaAmounts, monthly_period},
    state::AppState,
    usage_limits::{MONTHLY, YEARLY, normalize_period, period_start},
    utils::day_buckets::Buckets,
};
use axum::{
    Extension, Json,
    extract::{Query, State},
};
use chrono::{DateTime, FixedOffset, Utc};
use sea_orm::{
    ColumnTrait, ConnectionTrait, DatabaseConnection, DbErr, EntityTrait, FromQueryResult,
    QueryFilter, QuerySelect, Select,
    sea_query::{Alias, Expr, ExprTrait, Func, Query as SqlQuery, SimpleExpr},
};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use utoipa::{IntoParams, ToSchema};

const DONE: &str = "DONE";
const FAILED: &str = "FAILED";
const CANCELLED: &str = "CANCELLED";
const TOP_BOTS: usize = 10;

#[derive(Clone, Debug, Deserialize, IntoParams)]
pub struct BotActivityQuery {
    pub period: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct AdminBotTotals {
    pub turns: u64,
    pub completed: u64,
    pub failed: u64,
    pub cancelled: u64,
    pub active_bots: u64,
    pub people: u64,
    pub conversations: u64,
    pub live_bots: u64,
    pub new_bots: u64,
    /// Budget month to date across every bot, in micro-dollars.
    pub ai_cost_micros: i64,
    pub ai_calls: i64,
    pub runtime_ms: i64,
}

#[derive(Clone, Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct AdminBotTrendPoint {
    pub bucket: String,
    pub label: String,
    pub turns: u64,
    pub completed: u64,
    pub failed: u64,
    pub cancelled: u64,
    pub active_bots: u64,
}

#[derive(Clone, Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct AdminBotHost {
    /// `server`, `desktop` or `device`.
    pub host_kind: String,
    pub turns: u64,
}

#[derive(Clone, Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct AdminBotFailure {
    pub error_code: Option<String>,
    pub turns: u64,
}

#[derive(Clone, Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct AdminBotUsage {
    pub bot_id: String,
    pub name: Option<String>,
    pub owner_id: Option<String>,
    pub owner_name: Option<String>,
    pub deleted: bool,
    pub turns: u64,
    pub completed: u64,
    pub failed: u64,
    pub cancelled: u64,
    pub people: u64,
    pub conversations: u64,
    pub last_turn_at: Option<String>,
    /// Budget month to date, in micro-dollars.
    pub ai_cost_micros: i64,
    pub ai_calls: i64,
    pub runtime_ms: i64,
}

#[derive(Clone, Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct AdminBotActivity {
    pub period: String,
    pub started_at: String,
    /// Start of the budget month every spend figure covers.
    pub spend_since: String,
    pub totals: AdminBotTotals,
    pub trend: Vec<AdminBotTrendPoint>,
    pub hosts: Vec<AdminBotHost>,
    pub failures: Vec<AdminBotFailure>,
    /// The period's busiest bots.
    pub top_bots: Vec<AdminBotUsage>,
    /// The bots that spent the most this budget month.
    pub top_spenders: Vec<AdminBotUsage>,
}

#[derive(Debug, FromQueryResult)]
struct TurnGroup {
    bucket: i64,
    bot_id: String,
    status: String,
    host_kind: String,
    turns: i64,
    last_turn_at: DateTime<FixedOffset>,
}

#[derive(Debug, FromQueryResult)]
struct BotReach {
    bot_id: String,
    people: i64,
    conversations: i64,
}

#[derive(Debug, Default, FromQueryResult)]
struct Reach {
    people: i64,
    conversations: i64,
}

#[derive(Debug, FromQueryResult)]
struct FailureRow {
    error_code: Option<String>,
    turns: i64,
}

#[derive(Debug, Default, FromQueryResult)]
struct BotCounts {
    live_bots: i64,
    new_bots: i64,
}

#[derive(Clone, Debug, Default)]
struct Tally {
    turns: u64,
    completed: u64,
    failed: u64,
    cancelled: u64,
}

impl Tally {
    fn add(&mut self, status: &str, turns: u64) {
        self.turns += turns;
        match status {
            DONE => self.completed += turns,
            FAILED => self.failed += turns,
            CANCELLED => self.cancelled += turns,
            _ => {}
        }
    }
}

#[derive(Debug, Default)]
struct BotTally {
    tally: Tally,
    last_turn_at: Option<DateTime<FixedOffset>>,
}

fn turns_since(start: DateTime<FixedOffset>) -> Select<bot_turn::Entity> {
    bot_turn::Entity::find()
        .select_only()
        .filter(bot_turn::Column::CreatedAt.gte(start))
}

fn turn_groups(buckets: &Buckets, start: DateTime<FixedOffset>) -> Select<bot_turn::Entity> {
    use bot_turn::Column;

    buckets
        .group(turns_since(start), "bucket")
        .column_as(Column::BotId, "bot_id")
        .column_as(Column::Status, "status")
        .column_as(Column::HostKind, "host_kind")
        .expr_as(Expr::col(Column::Id).count(), "turns")
        .expr_as(Expr::col(Column::CreatedAt).max(), "last_turn_at")
        .group_by(Column::BotId)
        .group_by(Column::Status)
        .group_by(Column::HostKind)
}

fn count_distinct(column: bot_turn::Column) -> SimpleExpr {
    Func::count_distinct(Expr::col(column)).into()
}

fn reach(select: Select<bot_turn::Entity>) -> Select<bot_turn::Entity> {
    select
        .expr_as(count_distinct(bot_turn::Column::RequesterId), "people")
        .expr_as(
            count_distinct(bot_turn::Column::ConversationId),
            "conversations",
        )
}

fn bot_reach(start: DateTime<FixedOffset>) -> Select<bot_turn::Entity> {
    reach(turns_since(start).column_as(bot_turn::Column::BotId, "bot_id"))
        .group_by(bot_turn::Column::BotId)
}

fn failures(start: DateTime<FixedOffset>) -> Select<bot_turn::Entity> {
    use bot_turn::Column;

    turns_since(start)
        .column_as(Column::ErrorCode, "error_code")
        .expr_as(Expr::col(Column::Id).count(), "turns")
        .filter(Column::Status.eq(FAILED))
        .group_by(Column::ErrorCode)
}

fn bot_counts(start: DateTime<FixedOffset>) -> Select<bot::Entity> {
    use bot::Column;

    let count_where =
        |condition: SimpleExpr| Expr::expr(Expr::case(condition, Expr::col(Column::Id))).count();
    bot::Entity::find()
        .select_only()
        .expr_as(count_where(Column::DeletedAt.is_null()), "live_bots")
        .expr_as(
            count_where(
                Column::DeletedAt
                    .is_null()
                    .and(Column::CreatedAt.gte(start)),
            ),
            "new_bots",
        )
}

/// What every metered bot used in the budget month starting at `month_start`.
async fn month_spend(
    db: &DatabaseConnection,
    month_start: DateTime<Utc>,
) -> Result<HashMap<String, QuotaAmounts>, DbErr> {
    let query = SqlQuery::select()
        .column(Alias::new("subjectId"))
        .column(Alias::new("used"))
        .from(Alias::new("UsageBudgetPeriod"))
        .and_where(Expr::col(Alias::new("scope")).eq("bot"))
        .and_where(Expr::col(Alias::new("periodStart")).eq(month_start.timestamp_millis()))
        .to_owned();
    let rows = db
        .query_all_raw(db.get_database_backend().build(&query))
        .await?;
    let mut spend = HashMap::new();
    for row in rows {
        let bot_id: String = row.try_get("", "subjectId")?;
        let used: String = row.try_get("", "used")?;
        match flow_like_types::json::from_str::<QuotaAmounts>(&used) {
            Ok(amounts) => {
                spend.insert(bot_id, amounts);
            }
            Err(error) => tracing::warn!(%bot_id, %error, "Unreadable bot budget usage"),
        }
    }
    Ok(spend)
}

struct BotNames {
    names: HashMap<String, (Option<String>, String, bool)>,
    owners: HashMap<String, String>,
}

async fn bot_names(state: &AppState, bot_ids: HashSet<String>) -> Result<BotNames, ApiError> {
    if bot_ids.is_empty() {
        return Ok(BotNames {
            names: HashMap::new(),
            owners: HashMap::new(),
        });
    }
    let rows: Vec<(
        String,
        Option<String>,
        String,
        Option<DateTime<FixedOffset>>,
    )> = bot::Entity::find()
        .select_only()
        .column(bot::Column::Id)
        .column(bot::Column::Name)
        .column(bot::Column::OwnerId)
        .column(bot::Column::DeletedAt)
        .filter(bot::Column::Id.is_in(bot_ids))
        .into_tuple()
        .all(&state.db)
        .await?;
    let owner_ids = rows.iter().map(|(_, _, owner, _)| owner.clone()).collect();
    let owners = load_users(state, owner_ids)
        .await?
        .into_iter()
        .filter_map(|(id, user)| Some((id, display_name(&user)?)))
        .collect();
    let names = rows
        .into_iter()
        .map(|(id, name, owner, deleted_at)| (id, (name, owner, deleted_at.is_some())))
        .collect();
    Ok(BotNames { names, owners })
}

fn ranked<'a, K: Ord>(entries: impl Iterator<Item = (&'a String, K)>, limit: usize) -> Vec<String> {
    let mut entries: Vec<_> = entries.collect();
    entries.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(b.0)));
    entries
        .into_iter()
        .take(limit)
        .map(|(id, _)| id.clone())
        .collect()
}

#[tracing::instrument(name = "GET /admin/usage/bot-activity", skip_all)]
pub async fn activity(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Query(query): Query<BotActivityQuery>,
) -> Result<Json<AdminBotActivity>, ApiError> {
    user.check_global_permission(&state, GlobalPermission::Admin)
        .await?;

    let period = query
        .period
        .as_deref()
        .and_then(normalize_period)
        .unwrap_or_else(|| MONTHLY.to_string());
    let started_at =
        period_start(&period).ok_or_else(|| ApiError::bad_request("Invalid period"))?;
    let now = Utc::now();
    let buckets = Buckets::new(
        state.db.get_database_backend(),
        started_at.date_naive(),
        now.date_naive(),
        period == YEARLY,
    );
    let (month_start, _) = monthly_period(now, None);
    let db = &state.db;

    let (groups, bot_reach, reach, failures, counts, spend) = flow_like_types::tokio::try_join!(
        turn_groups(&buckets, started_at)
            .into_model::<TurnGroup>()
            .all(db),
        bot_reach(started_at).into_model::<BotReach>().all(db),
        reach(turns_since(started_at)).into_model::<Reach>().one(db),
        failures(started_at).into_model::<FailureRow>().all(db),
        bot_counts(started_at).into_model::<BotCounts>().one(db),
        month_spend(db, month_start),
    )?;

    let mut total = Tally::default();
    let mut trend = vec![(Tally::default(), HashSet::new()); buckets.keys.len()];
    let mut hosts: HashMap<String, u64> = HashMap::new();
    let mut bots: HashMap<String, BotTally> = HashMap::new();
    for group in &groups {
        let turns = to_count(group.turns);
        total.add(&group.status, turns);
        if let Some((tally, active)) = usize::try_from(group.bucket)
            .ok()
            .and_then(|index| trend.get_mut(index))
        {
            tally.add(&group.status, turns);
            active.insert(group.bot_id.as_str());
        }
        *hosts
            .entry(group.host_kind.to_ascii_lowercase())
            .or_default() += turns;
        let bot = bots.entry(group.bot_id.clone()).or_default();
        bot.tally.add(&group.status, turns);
        bot.last_turn_at = Ord::max(bot.last_turn_at, Some(group.last_turn_at));
    }

    let reach_by_bot: HashMap<&str, &BotReach> = bot_reach
        .iter()
        .map(|row| (row.bot_id.as_str(), row))
        .collect();
    let top_bot_ids = ranked(
        bots.iter()
            .map(|(id, bot)| (id, (bot.tally.turns, bot.last_turn_at))),
        TOP_BOTS,
    );
    let top_spender_ids = ranked(
        spend
            .iter()
            .filter(|(_, used)| used.ai_cost_micros > 0 || used.runtime_ms > 0)
            .map(|(id, used)| (id, (used.ai_cost_micros, used.runtime_ms, used.ai_calls))),
        TOP_BOTS,
    );
    let names = bot_names(
        &state,
        top_bot_ids
            .iter()
            .chain(&top_spender_ids)
            .cloned()
            .collect(),
    )
    .await?;

    let usage = |bot_id: &String| {
        let tally = bots.get(bot_id);
        let used = spend.get(bot_id).copied().unwrap_or_default();
        let reach = reach_by_bot.get(bot_id.as_str());
        let named = names.names.get(bot_id);
        let owner_id = named.map(|(_, owner, _)| owner.clone());
        AdminBotUsage {
            bot_id: bot_id.clone(),
            name: named.and_then(|(name, _, _)| name.clone()),
            owner_name: owner_id
                .as_ref()
                .and_then(|owner| names.owners.get(owner).cloned()),
            owner_id,
            deleted: named.is_some_and(|(_, _, deleted)| *deleted),
            turns: tally.map_or(0, |bot| bot.tally.turns),
            completed: tally.map_or(0, |bot| bot.tally.completed),
            failed: tally.map_or(0, |bot| bot.tally.failed),
            cancelled: tally.map_or(0, |bot| bot.tally.cancelled),
            people: reach.map_or(0, |row| to_count(row.people)),
            conversations: reach.map_or(0, |row| to_count(row.conversations)),
            last_turn_at: tally
                .and_then(|bot| bot.last_turn_at)
                .map(|at| at.to_rfc3339()),
            ai_cost_micros: used.ai_cost_micros,
            ai_calls: used.ai_calls,
            runtime_ms: used.runtime_ms,
        }
    };

    let reach = reach.unwrap_or_default();
    let counts = counts.unwrap_or_default();
    let mut hosts: Vec<AdminBotHost> = hosts
        .into_iter()
        .map(|(host_kind, turns)| AdminBotHost { host_kind, turns })
        .collect();
    hosts.sort_by(|a, b| {
        b.turns
            .cmp(&a.turns)
            .then_with(|| a.host_kind.cmp(&b.host_kind))
    });
    let mut failures: Vec<AdminBotFailure> = failures
        .into_iter()
        .map(|row| AdminBotFailure {
            error_code: row.error_code,
            turns: to_count(row.turns),
        })
        .collect();
    failures.sort_by(|a, b| {
        b.turns
            .cmp(&a.turns)
            .then_with(|| a.error_code.cmp(&b.error_code))
    });

    Ok(Json(AdminBotActivity {
        started_at: started_at.to_rfc3339(),
        spend_since: month_start.to_rfc3339(),
        totals: AdminBotTotals {
            turns: total.turns,
            completed: total.completed,
            failed: total.failed,
            cancelled: total.cancelled,
            active_bots: bots.len() as u64,
            people: to_count(reach.people),
            conversations: to_count(reach.conversations),
            live_bots: to_count(counts.live_bots),
            new_bots: to_count(counts.new_bots),
            ai_cost_micros: spend.values().map(|used| used.ai_cost_micros).sum(),
            ai_calls: spend.values().map(|used| used.ai_calls).sum(),
            runtime_ms: spend.values().map(|used| used.runtime_ms).sum(),
        },
        trend: buckets
            .keys
            .iter()
            .zip(trend)
            .map(|(key, (tally, active))| AdminBotTrendPoint {
                bucket: key.clone(),
                label: trend_label(key, buckets.monthly),
                turns: tally.turns,
                completed: tally.completed,
                failed: tally.failed,
                cancelled: tally.cancelled,
                active_bots: active.len() as u64,
            })
            .collect(),
        hosts,
        failures,
        top_bots: top_bot_ids.iter().map(usage).collect(),
        top_spenders: top_spender_ids.iter().map(usage).collect(),
        period,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use sea_orm::{DbBackend, QueryTrait};

    #[test]
    fn tally_splits_turns_by_outcome() {
        let mut tally = Tally::default();
        tally.add(DONE, 5);
        tally.add(FAILED, 2);
        tally.add(CANCELLED, 1);
        tally.add("RUNNING", 3);
        assert_eq!(
            (tally.turns, tally.completed, tally.failed, tally.cancelled),
            (11, 5, 2, 1)
        );
    }

    #[test]
    fn ranked_orders_by_key_then_id_and_truncates() {
        let ids = ["b".to_string(), "a".to_string(), "c".to_string()];
        let ranked = ranked([(&ids[0], 3), (&ids[1], 3), (&ids[2], 9)].into_iter(), 2);
        assert_eq!(ranked, ["c", "a"]);
    }

    /// See `utils::day_buckets`: turns must take their bucket from the joined
    /// series, or DSQL sorts the whole window.
    #[test]
    fn turn_groups_join_the_bucket_series() {
        let start = Utc::now().fixed_offset() - chrono::Duration::days(2);
        let buckets = Buckets::new(
            DbBackend::Postgres,
            start.date_naive(),
            Utc::now().date_naive(),
            false,
        );
        let sql = turn_groups(&buckets, start)
            .build(DbBackend::Postgres)
            .to_string();

        assert!(sql.contains(r#"AS "bucket_series" ON"#), "{sql}");
        assert!(
            sql.contains(r#"GROUP BY "bucket_series"."i", "BotTurn"."botId""#),
            "{sql}"
        );
    }
}
