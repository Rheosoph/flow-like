//! Apps in the org chart: the metadata and type everyone in an org sees, and the live numbers
//! only members of an app see.

use crate::{
    entity::{
        app, event, execution_usage_daily, execution_usage_tracking, membership, meta, page, role,
        sea_orm_active_enums::{AppType, ExecutionStatus, Status, Visibility},
    },
    execution_usage_rollup::{last_closed_day, rolled_days},
    permission::role_permission::RolePermissions,
    routes::{
        app::events::db::extract_cron_expression,
        explore::{
            audience::DEFAULT_LANGUAGE,
            hydrate::{app_media, best_meta, media_store},
        },
    },
    state::AppState,
    utils::{day_buckets::Buckets, time::utc_midnight},
};
use chrono::{DateTime, Duration, FixedOffset, NaiveDate};
use flow_like::bit::Metadata;
use flow_like_types::bot_team::{ORG_APP_DAYS, OrgApp, OrgAppStats};
use futures::future::join_all;
use sea_orm::{
    ColumnTrait, ConnectionTrait, DbErr, EntityTrait, FromQueryResult, QueryFilter, QueryOrder,
    QuerySelect,
    sea_query::{Alias, Expr, ExprTrait},
};
use std::collections::{HashMap, HashSet};

/// Everything the chart knows about one app for one reader.
pub(crate) struct AppFacts {
    pub app: app::Model,
    pub meta: Option<meta::Model>,
    pub reader_role: Option<i64>,
    pub owner_id: Option<String>,
}

impl AppFacts {
    pub fn can_open(&self) -> bool {
        self.reader_role.is_some()
    }

    pub fn reader_admin(&self) -> bool {
        self.reader_role.is_some_and(is_admin_role)
    }

    pub fn can_request(&self) -> bool {
        !self.can_open()
            && matches!(
                self.app.visibility,
                Visibility::Public | Visibility::PublicRequestAccess
            )
    }

    pub fn name(&self) -> String {
        self.meta
            .as_ref()
            .map_or_else(|| self.app.id.clone(), |meta| meta.name.clone())
    }

    pub fn description(&self) -> Option<String> {
        self.meta.as_ref().and_then(|meta| meta.description.clone())
    }

    pub fn app_type(&self) -> Option<String> {
        self.app.app_type.as_ref().map(app_type_name)
    }
}

pub(crate) fn is_admin_role(permissions: i64) -> bool {
    let flags = RolePermissions::from_bits_truncate(permissions);
    flags.contains(RolePermissions::Owner) || flags.contains(RolePermissions::Admin)
}

pub(crate) fn app_type_name(app_type: &AppType) -> String {
    match app_type {
        AppType::Agent => "Agent",
        AppType::CustomInterface => "CustomInterface",
        AppType::DataFocus => "DataFocus",
        AppType::DataPipeline => "DataPipeline",
        AppType::Analytics => "Analytics",
        AppType::Form => "Form",
    }
    .to_string()
}

/// The pinnable subset of `ids`: existing apps that are neither a bot's workspace nor archived,
/// with metadata, the reader's role and the owner. Five queries however many apps.
pub(crate) async fn load_facts<C: ConnectionTrait>(
    db: &C,
    viewer: &str,
    ids: &[String],
) -> Result<HashMap<String, AppFacts>, DbErr> {
    if ids.is_empty() {
        return Ok(HashMap::new());
    }
    let apps = app::Entity::find()
        .filter(app::Column::Id.is_in(ids.to_vec()))
        .filter(app::Column::BotId.is_null())
        .filter(app::Column::Status.ne(Status::Archived))
        .all(db)
        .await?;
    let found: Vec<String> = apps.iter().map(|app| app.id.clone()).collect();
    let metas = meta::Entity::find()
        .filter(meta::Column::AppId.is_in(found.clone()))
        .order_by_asc(meta::Column::Lang)
        .all(db)
        .await?;
    let mut best = best_meta(metas, DEFAULT_LANGUAGE, |meta| meta.app_id.as_ref());
    let mine: HashMap<String, String> = membership::Entity::find()
        .filter(membership::Column::UserId.eq(viewer))
        .filter(membership::Column::AppId.is_in(found))
        .all(db)
        .await?
        .into_iter()
        .map(|row| (row.app_id, row.role_id))
        .collect();
    let permissions: HashMap<String, i64> = role::Entity::find()
        .filter(role::Column::Id.is_in(mine.values().cloned()))
        .all(db)
        .await?
        .into_iter()
        .map(|row| (row.id, row.permissions))
        .collect();
    let owner_roles: Vec<String> = apps
        .iter()
        .filter_map(|app| app.owner_role_id.clone())
        .collect();
    let owners: HashMap<String, String> = membership::Entity::find()
        .filter(membership::Column::RoleId.is_in(owner_roles))
        .order_by_asc(membership::Column::CreatedAt)
        .all(db)
        .await?
        .into_iter()
        .rev()
        .map(|row| (row.role_id, row.user_id))
        .collect();
    Ok(apps
        .into_iter()
        .map(|app| {
            let facts = AppFacts {
                meta: best.remove(&app.id),
                reader_role: mine
                    .get(&app.id)
                    .map(|role_id| permissions.get(role_id).copied().unwrap_or(0)),
                owner_id: app
                    .owner_role_id
                    .as_ref()
                    .and_then(|role_id| owners.get(role_id).cloned()),
                app,
            };
            (facts.app.id.clone(), facts)
        })
        .collect())
}

/// The org-chart card of an app; icon and thumbnail still name the stored media, which
/// [`sign_media`] turns into links.
pub(crate) fn org_app(facts: &AppFacts, stats: Option<OrgAppStats>) -> OrgApp {
    OrgApp {
        id: facts.app.id.clone(),
        name: facts.name(),
        description: facts.description(),
        icon: facts.meta.as_ref().and_then(|meta| meta.icon.clone()),
        thumbnail: facts.meta.as_ref().and_then(|meta| meta.thumbnail.clone()),
        app_type: facts.app_type(),
        can_open: facts.can_open(),
        can_request: facts.can_request(),
        reader_admin: facts.reader_admin(),
        owner_id: facts.owner_id.clone(),
        updated_at: facts.app.updated_at.to_rfc3339(),
        stats,
    }
}

/// Replaces stored media names with signed links, in place. Without a media store the names stay.
pub(crate) async fn sign_media(
    state: &AppState,
    items: Vec<(&str, &mut Option<String>, Option<&mut Option<String>>)>,
) {
    let Some(store) = media_store(state).await else {
        return;
    };
    let tasks = items.into_iter().map(|(app_id, icon, thumbnail)| {
        let store = &store;
        async move {
            let mut metadata = Metadata {
                icon: icon.clone(),
                thumbnail: thumbnail.as_ref().and_then(|slot| (**slot).clone()),
                ..Default::default()
            };
            metadata.presign(app_media(app_id), store).await;
            *icon = metadata.icon;
            if let Some(slot) = thumbnail {
                *slot = metadata.thumbnail;
            }
        }
    });
    join_all(tasks).await;
}

pub(crate) async fn sign_org_apps(state: &AppState, apps: &mut [OrgApp]) {
    let items = apps
        .iter_mut()
        .map(|app| (app.id.as_str(), &mut app.icon, Some(&mut app.thumbnail)))
        .collect();
    sign_media(state, items).await;
}

#[derive(Debug, FromQueryResult)]
struct RolledRow {
    day: NaiveDate,
    app_id: Option<String>,
    status: ExecutionStatus,
    executions: i64,
}

#[derive(Debug, FromQueryResult)]
struct LiveRow {
    bucket: i64,
    app_id: Option<String>,
    status: ExecutionStatus,
    cnt: i64,
}

#[derive(Default)]
struct Counts {
    runs: Vec<u32>,
    failed: Vec<u32>,
}

impl Counts {
    fn new() -> Self {
        Self {
            runs: vec![0; ORG_APP_DAYS],
            failed: vec![0; ORG_APP_DAYS],
        }
    }

    fn add(&mut self, index: usize, status: &ExecutionStatus, count: i64) {
        let count = u32::try_from(Ord::max(count, 0)).unwrap_or(u32::MAX);
        if let Some(slot) = self.runs.get_mut(index) {
            *slot = slot.saturating_add(count);
        }
        if matches!(status, ExecutionStatus::Error | ExecutionStatus::Fatal)
            && let Some(slot) = self.failed.get_mut(index)
        {
            *slot = slot.saturating_add(count);
        }
    }
}

fn window(now: DateTime<FixedOffset>) -> (NaiveDate, NaiveDate) {
    let today = now.naive_utc().date();
    (today - Duration::days(ORG_APP_DAYS as i64 - 1), today)
}

/// Runs per app and window day: closed, rolled-up days from `ExecutionUsageDaily`, the rest
/// from raw `ExecutionUsageTracking` rows grouped by day.
async fn run_counts<C: ConnectionTrait>(
    db: &C,
    ids: &[String],
    now: DateTime<FixedOffset>,
) -> Result<HashMap<String, Counts>, DbErr> {
    let (first, today) = window(now);
    let closed = last_closed_day(now);
    let rolled = rolled_days(db, first, closed).await?;
    let live_from = first
        .iter_days()
        .take_while(|day| *day <= closed)
        .find(|day| !rolled.contains(day))
        .unwrap_or(closed + Duration::days(1));
    let mut counts: HashMap<String, Counts> = HashMap::new();
    let index_of = |day: NaiveDate| usize::try_from((day - first).num_days()).ok();
    if live_from > first {
        use execution_usage_daily::Column;
        let rows = execution_usage_daily::Entity::find()
            .select_only()
            .column(Column::Day)
            .column_as(Column::AppId, "app_id")
            .column(Column::Status)
            .column(Column::Executions)
            .filter(Column::AppId.is_in(ids.to_vec()))
            .filter(Column::Day.gte(first))
            .filter(Column::Day.lt(live_from))
            .into_model::<RolledRow>()
            .all(db)
            .await?;
        for row in rolled_rows(rows) {
            if let Some(index) = index_of(row.0) {
                counts
                    .entry(row.1)
                    .or_insert_with(Counts::new)
                    .add(index, &row.2, row.3);
            }
        }
    }
    let buckets = Buckets::new(db.get_database_backend(), live_from, today, false);
    use execution_usage_tracking::Column;
    let live = buckets
        .group(
            execution_usage_tracking::Entity::find().select_only(),
            "bucket",
        )
        .column_as(Column::AppId, "app_id")
        .column(Column::Status)
        .expr_as(Expr::col(Column::Id).count(), "cnt")
        .filter(Column::AppId.is_in(ids.to_vec()))
        .filter(Column::CreatedAt.gte(utc_midnight(live_from)))
        .group_by(Column::AppId)
        .group_by(Column::Status)
        .into_model::<LiveRow>()
        .all(db)
        .await?;
    for row in live {
        let (Some(app_id), Ok(offset)) = (row.app_id, usize::try_from(row.bucket)) else {
            continue;
        };
        let Some(index) = index_of(live_from).map(|start| start + offset) else {
            continue;
        };
        counts
            .entry(app_id)
            .or_insert_with(Counts::new)
            .add(index, &row.status, row.cnt);
    }
    Ok(counts)
}

fn rolled_rows(rows: Vec<RolledRow>) -> Vec<(NaiveDate, String, ExecutionStatus, i64)> {
    rows.into_iter()
        .filter_map(|row| Some((row.day, row.app_id?, row.status, row.executions)))
        .collect()
}

async fn last_runs<C: ConnectionTrait>(
    db: &C,
    ids: &[String],
    now: DateTime<FixedOffset>,
) -> Result<HashMap<String, String>, DbErr> {
    use execution_usage_tracking::Column;
    let (first, _) = window(now);
    Ok(execution_usage_tracking::Entity::find()
        .select_only()
        .column(Column::AppId)
        .column_as(Column::CreatedAt.max(), "last_run")
        .filter(Column::AppId.is_in(ids.to_vec()))
        .filter(Column::CreatedAt.gte(utc_midnight(first)))
        .group_by(Column::AppId)
        .into_tuple::<(Option<String>, Option<DateTime<FixedOffset>>)>()
        .all(db)
        .await?
        .into_iter()
        .filter_map(|(app_id, at)| Some((app_id?, at?.to_rfc3339())))
        .collect())
}

fn cron_of(config: &flow_like_types::Value) -> Option<String> {
    let encoded = config.get("base64")?.as_str()?;
    let bytes = base64::Engine::decode(&base64::engine::general_purpose::STANDARD, encoded).ok()?;
    extract_cron_expression(&bytes)
}

#[derive(Default)]
struct Wiring {
    triggers: Vec<String>,
    schedule: Option<String>,
}

async fn wirings<C: ConnectionTrait>(
    db: &C,
    ids: &[String],
) -> Result<HashMap<String, Wiring>, DbErr> {
    let events = event::Entity::find()
        .filter(event::Column::AppId.is_in(ids.to_vec()))
        .filter(event::Column::Active.eq(true))
        .order_by_asc(event::Column::CreatedAt)
        .order_by_asc(event::Column::Id)
        .all(db)
        .await?;
    let mut wirings: HashMap<String, Wiring> = HashMap::new();
    for row in events {
        let wiring = wirings.entry(row.app_id).or_default();
        if !wiring.triggers.contains(&row.event_type) {
            wiring.triggers.push(row.event_type.clone());
        }
        if row.event_type == "cron" && wiring.schedule.is_none() {
            wiring.schedule = row.config.as_ref().and_then(cron_of);
        }
    }
    Ok(wirings)
}

async fn page_counts<C: ConnectionTrait>(
    db: &C,
    ids: &[String],
) -> Result<HashMap<String, u32>, DbErr> {
    Ok(page::Entity::find()
        .select_only()
        .column(page::Column::AppId)
        .column_as(
            Expr::col(page::Column::Id)
                .count()
                .cast_as(Alias::new("BIGINT")),
            "n",
        )
        .filter(page::Column::AppId.is_in(ids.to_vec()))
        .group_by(page::Column::AppId)
        .into_tuple::<(String, i64)>()
        .all(db)
        .await?
        .into_iter()
        .map(|(app_id, n)| (app_id, u32::try_from(n).unwrap_or(u32::MAX)))
        .collect())
}

/// What the apps in `ids` did lately: one query per table for all of them together.
pub(crate) async fn app_stats<C: ConnectionTrait>(
    db: &C,
    ids: &[String],
    now: DateTime<FixedOffset>,
) -> Result<HashMap<String, OrgAppStats>, DbErr> {
    if ids.is_empty() {
        return Ok(HashMap::new());
    }
    let mut counts = run_counts(db, ids, now).await?;
    let mut last = last_runs(db, ids, now).await?;
    let mut wiring = wirings(db, ids).await?;
    let pages = page_counts(db, ids).await?;
    let unique: HashSet<&String> = ids.iter().collect();
    Ok(unique
        .into_iter()
        .map(|id| {
            let counted = counts.remove(id).unwrap_or_else(Counts::new);
            let wired = wiring.remove(id).unwrap_or_default();
            let stats = OrgAppStats {
                runs: counted.runs,
                failed: counted.failed,
                last_run_at: last.remove(id),
                triggers: wired.triggers,
                schedule: wired.schedule,
                pages: pages.get(id).copied().unwrap_or(0),
            };
            (id.clone(), stats)
        })
        .collect())
}

/// The org-chart cards of `ids` as `viewer` sees them, stats only for apps they can open.
pub(crate) async fn org_apps<C: ConnectionTrait>(
    db: &C,
    viewer: &str,
    ids: &[String],
    now: DateTime<FixedOffset>,
) -> Result<Vec<OrgApp>, DbErr> {
    let facts = load_facts(db, viewer, ids).await?;
    let openable: Vec<String> = facts
        .values()
        .filter(|facts| facts.can_open())
        .map(|facts| facts.app.id.clone())
        .collect();
    let mut stats = app_stats(db, &openable, now).await?;
    let mut cards: Vec<OrgApp> = facts
        .values()
        .map(|facts| org_app(facts, stats.remove(&facts.app.id)))
        .collect();
    cards.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(cards)
}
