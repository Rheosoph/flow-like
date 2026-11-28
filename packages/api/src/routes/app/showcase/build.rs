//! One builder per showcase section. Each reads only what its switches allow and degrades to
//! empty on failure: a store page never errors because one section could not be read.

use super::outline::page_outline;
use super::types::*;
use crate::entity::{
    app, app_board_score, app_data_source, app_package, bit, event, execution_usage_daily, meta,
    page, template, wasm_package, widget,
};
use crate::routes::app::board::scoring::BoardSummaryMeta;
use crate::routes::app::events::db as event_db;
use crate::state::AppState;
use flow_like::flow::board::summary::BoardSummaryMetrics;
use flow_like::flow::event::EventExposure;
use flow_like_storage::databases::showcase::{ShowcaseData, ShowcaseTableOptions, showcase_tables};
use futures::{StreamExt, stream};
use sea_orm::{
    ColumnTrait, EntityTrait, FromQueryResult, QueryFilter, QueryOrder, QuerySelect,
    sea_query::{Expr, ExprTrait},
};
use std::collections::{BTreeMap, BTreeSet, HashMap};

const MAX_OUTLINED_PAGES: usize = 24;
const PAGE_CONCURRENCY: usize = 6;
const ACTIVITY_DAYS: i64 = 30;

/// Board facts the flows, map and requirements sections share, read from the cached summaries.
pub struct BoardFacts {
    pub id: String,
    pub meta: BoardSummaryMeta,
    pub node_count: u32,
    pub scores: Option<ShowcaseScores>,
}

impl BoardFacts {
    fn metrics(&self) -> Option<&BoardSummaryMetrics> {
        self.meta.metrics.as_ref()
    }
}

pub async fn board_facts(state: &AppState, app_id: &str) -> Vec<BoardFacts> {
    let rows = match app_board_score::Entity::find()
        .filter(app_board_score::Column::AppId.eq(app_id))
        .all(&state.db)
        .await
    {
        Ok(rows) => rows,
        Err(error) => {
            tracing::warn!(%app_id, %error, "showcase: board summaries unavailable");
            return Vec::new();
        }
    };
    let mut facts: Vec<BoardFacts> = rows
        .into_iter()
        .filter_map(|row| {
            let meta: BoardSummaryMeta = serde_json::from_value(row.summary.clone()?).ok()?;
            let scores = (row.scored_node_count > 0).then(|| ShowcaseScores {
                security: Ord::clamp(row.security, 0, 10) as u8,
                privacy: Ord::clamp(row.privacy, 0, 10) as u8,
                performance: Ord::clamp(row.performance, 0, 10) as u8,
                governance: Ord::clamp(row.governance, 0, 10) as u8,
                reliability: Ord::clamp(row.reliability, 0, 10) as u8,
                cost: Ord::clamp(row.cost, 0, 10) as u8,
            });
            Some(BoardFacts {
                id: row.board_id,
                meta,
                node_count: Ord::max(row.node_count, 0) as u32,
                scores,
            })
        })
        .collect();
    facts.sort_by_key(|board| board.meta.name.to_lowercase());
    facts
}

pub async fn tables(
    state: &AppState,
    app_id: &str,
    settings: &ShowcaseDataSettings,
) -> ShowcaseData {
    if !settings.enabled || settings.tables.is_empty() {
        return ShowcaseData::default();
    }
    let connection = async {
        let credentials = state.master_credentials().await?;
        let builder = credentials.to_db(app_id).await?;
        Ok::<_, flow_like_types::Error>(builder.execute().await?)
    }
    .await;
    let connection = match connection {
        Ok(connection) => connection,
        Err(error) => {
            tracing::warn!(%app_id, %error, "showcase: database unavailable");
            return ShowcaseData::default();
        }
    };
    showcase_tables(
        &connection,
        settings.tables.clone(),
        ShowcaseTableOptions {
            row_counts: settings.row_counts,
            schema: settings.schema,
            samples: settings.samples,
            object_types: settings.ontology,
        },
    )
    .await
}

pub async fn sources(state: &AppState, app_id: &str) -> Vec<ShowcaseSource> {
    app_data_source::Entity::find()
        .filter(app_data_source::Column::AppId.eq(app_id))
        .order_by_asc(app_data_source::Column::Name)
        .all(&state.db)
        .await
        .unwrap_or_default()
        .into_iter()
        .map(|row| ShowcaseSource {
            id: row.id,
            name: row.name,
            alias: row.alias,
            driver: row.driver,
        })
        .collect()
}

pub async fn events(
    state: &AppState,
    app_id: &str,
    settings: &ShowcaseTriggerSettings,
) -> Vec<ShowcaseEvent> {
    if !settings.enabled || settings.events.is_empty() {
        return Vec::new();
    }
    let rows = event::Entity::find()
        .filter(event::Column::AppId.eq(app_id))
        .filter(event::Column::Id.is_in(settings.events.clone()))
        .filter(event::Column::Active.eq(true))
        .all(&state.db)
        .await
        .unwrap_or_default();
    let mut events: Vec<ShowcaseEvent> = rows
        .into_iter()
        .filter_map(|row| event_db::db_model_to_event(row).ok())
        .map(|event| {
            let public = event.exposure == EventExposure::Public;
            let technical = (settings.technical && public)
                .then(|| technical(&event.event_type, &event.config))
                .flatten();
            ShowcaseEvent {
                description: Some(event.description.trim().to_string()).filter(|d| !d.is_empty()),
                board_id: Some(event.board_id).filter(|board| !board.is_empty()),
                page_id: event.default_page_id,
                tables: Vec::new(),
                pages: Vec::new(),
                technical,
                id: event.id,
                name: event.name,
                event_type: event.event_type,
            }
        })
        .collect();
    let order: HashMap<&str, usize> = settings
        .events
        .iter()
        .enumerate()
        .map(|(index, id)| (id.as_str(), index))
        .collect();
    events.sort_by_key(|event| order.get(event.id.as_str()).copied().unwrap_or(usize::MAX));
    events
}

/// Only these config fields ever leave the API; secrets and everything else stay put.
fn technical(event_type: &str, config: &[u8]) -> Option<ShowcaseEventTechnical> {
    let technical = match event_type {
        "api" | "http" | "webhook" | "rest" => ShowcaseEventTechnical {
            method: event_db::extract_http_method(config).or_else(|| Some("POST".to_string())),
            path: event_db::extract_http_path(config),
            ..Default::default()
        },
        "cron" => ShowcaseEventTechnical {
            cron: event_db::extract_cron_expression(config),
            timezone: event_db::extract_cron_timezone(config),
            ..Default::default()
        },
        _ => return None,
    };
    (technical.path.is_some() || technical.cron.is_some()).then_some(technical)
}

pub fn flows(
    facts: &[BoardFacts],
    shown_tables: &[String],
    pages: &[ShowcasePage],
) -> Vec<ShowcaseFlow> {
    facts
        .iter()
        .map(|board| ShowcaseFlow {
            id: board.id.clone(),
            name: board.meta.name.clone(),
            description: board.meta.description.clone(),
            node_count: board.node_count,
            scores: board.scores.clone(),
            tables: board
                .metrics()
                .map(|metrics| {
                    metrics
                        .table_refs
                        .iter()
                        .filter(|table| shown_tables.contains(table))
                        .cloned()
                        .collect()
                })
                .unwrap_or_default(),
            pages: pages
                .iter()
                .filter(|page| page.board_id.as_deref() == Some(board.id.as_str()))
                .map(|page| page.id.clone())
                .collect(),
        })
        .collect()
}

/// Each event carries the shown tables and pages its own flow touches, so the map can draw
/// trigger → result even while flow names stay hidden. The flow id leaves only when flows show.
pub fn link_events(
    events: Vec<ShowcaseEvent>,
    flows: &[ShowcaseFlow],
    pages: &[ShowcasePage],
    show_flows: bool,
) -> Vec<ShowcaseEvent> {
    events
        .into_iter()
        .map(|mut event| {
            let flow = event
                .board_id
                .as_deref()
                .and_then(|board| flows.iter().find(|flow| flow.id == board));
            event.tables = flow.map(|flow| flow.tables.clone()).unwrap_or_default();
            event.pages = flow.map(|flow| flow.pages.clone()).unwrap_or_default();
            event.page_id = event
                .page_id
                .filter(|page_id| pages.iter().any(|page| &page.id == page_id));
            if !show_flows {
                event.board_id = None;
            }
            event
        })
        .collect()
}

pub async fn pages(
    state: &AppState,
    app_id: &str,
    settings: &ShowcasePageSettings,
) -> Vec<ShowcasePage> {
    if !settings.enabled {
        return Vec::new();
    }
    let routes: HashMap<String, String> = event::Entity::find()
        .filter(event::Column::AppId.eq(app_id))
        .filter(event::Column::PageId.is_not_null())
        .filter(event::Column::Route.is_not_null())
        .all(&state.db)
        .await
        .unwrap_or_default()
        .into_iter()
        .filter_map(|row| Some((row.page_id?, row.route?)))
        .collect();
    let rows = page::Entity::find()
        .filter(page::Column::AppId.eq(app_id))
        .order_by_asc(page::Column::Name)
        .all(&state.db)
        .await
        .unwrap_or_default();

    let outlines: HashMap<String, (Option<String>, ShowcaseOutlineNode, u32)> = if settings.previews
    {
        load_outlines(state, app_id, &rows).await
    } else {
        HashMap::new()
    };

    rows.into_iter()
        .map(|row| {
            let outlined = outlines.get(&row.id);
            ShowcasePage {
                route: outlined
                    .and_then(|(route, _, _)| route.clone())
                    .or_else(|| routes.get(&row.id).cloned()),
                outline: outlined.map(|(_, outline, _)| outline.clone()),
                component_count: outlined.map(|(_, _, count)| *count),
                board_id: row.board_id,
                description: row.description.filter(|d| !d.trim().is_empty()),
                name: row.name,
                id: row.id,
            }
        })
        .collect()
}

fn component_total(page: &flow_like::a2ui::widget::Page) -> u32 {
    let widget_components: usize = page
        .widget_refs
        .values()
        .map(|widget| widget.components.len())
        .sum();
    (page.components.len() + widget_components) as u32
}

async fn load_outlines(
    state: &AppState,
    app_id: &str,
    rows: &[page::Model],
) -> HashMap<String, (Option<String>, ShowcaseOutlineNode, u32)> {
    let Ok(app_state) = state.master_state(state).await else {
        return HashMap::new();
    };
    let Ok(store) = flow_like::flow::board::Board::meta_store(&app_state).await else {
        return HashMap::new();
    };
    let storage_scope = state.storage_identity.meta.cache_scope();
    let storage_root = flow_like_storage::Path::from("apps").join(app_id);
    let cache = &state.cache;
    let app_id = app_id.to_string();
    let jobs: Vec<(String, String)> = rows
        .iter()
        .filter_map(|row| Some((row.id.clone(), row.board_id.clone()?)))
        .take(MAX_OUTLINED_PAGES)
        .collect();

    stream::iter(jobs)
        .map(move |(page_id, board_id)| {
            let store = store.clone();
            let storage_root = storage_root.clone();
            let storage_scope = storage_scope.clone();
            let app_id = app_id.clone();
            async move {
                let board = crate::routes::app::page::cached_page::page_owner(
                    &store,
                    &storage_root,
                    board_id,
                    None,
                )
                .await?;
                let page = crate::routes::app::page::cached_page::load(
                    cache,
                    store,
                    storage_scope.as_deref(),
                    &app_id,
                    &board,
                    &page_id,
                    None,
                )
                .await
                .ok()?;
                let route = Some(page.route.clone()).filter(|route| !route.trim().is_empty());
                Some((
                    page_id,
                    (route, page_outline(&page), component_total(&page)),
                ))
            }
        })
        .buffer_unordered(PAGE_CONCURRENCY)
        .filter_map(|entry| async move { entry })
        .collect()
        .await
}

/// Meta rows for the given owner column, preferring `language`, falling back to English.
async fn localized_meta(
    state: &AppState,
    owner: meta::Column,
    ids: Vec<String>,
    language: &str,
) -> HashMap<String, meta::Model> {
    if ids.is_empty() {
        return HashMap::new();
    }
    let rows = meta::Entity::find()
        .filter(owner.is_in(ids))
        .filter(
            meta::Column::Lang
                .eq(language)
                .or(meta::Column::Lang.eq("en")),
        )
        .all(&state.db)
        .await
        .unwrap_or_default();
    let owner_of = |row: &meta::Model| match owner {
        meta::Column::WidgetId => row.widget_id.clone(),
        meta::Column::TemplateId => row.template_id.clone(),
        meta::Column::BitId => row.bit_id.clone(),
        meta::Column::WasmPackageId => row.wasm_package_id.clone(),
        _ => row.app_id.clone(),
    };
    let mut by_owner: HashMap<String, meta::Model> = HashMap::new();
    for row in rows {
        let Some(id) = owner_of(&row) else { continue };
        let preferred = row.lang == language;
        match by_owner.get(&id) {
            Some(existing) if existing.lang == language || !preferred => {}
            _ => {
                by_owner.insert(id, row);
            }
        }
    }
    by_owner
}

pub async fn widgets(state: &AppState, app_id: &str, language: &str) -> Vec<ShowcaseWidget> {
    let ids: Vec<String> = widget::Entity::find()
        .filter(widget::Column::AppId.eq(app_id))
        .all(&state.db)
        .await
        .unwrap_or_default()
        .into_iter()
        .map(|row| row.id)
        .collect();
    let metas = localized_meta(state, meta::Column::WidgetId, ids.clone(), language).await;
    let store = match state.master_credentials().await {
        Ok(credentials) => credentials.to_store(false).await.ok(),
        Err(_) => None,
    };
    let prefix = flow_like_storage::Path::from("media")
        .join("apps")
        .join(app_id);
    let mut widgets = Vec::new();
    for id in ids {
        let Some(row) = metas.get(&id) else { continue };
        let mut metadata = flow_like::bit::Metadata::from(row.clone());
        if let Some(store) = &store {
            metadata.presign(prefix.clone(), store).await;
        }
        widgets.push(ShowcaseWidget {
            id,
            name: metadata.name,
            description: Some(metadata.description).filter(|d| !d.trim().is_empty()),
            thumbnail: metadata.thumbnail,
        });
    }
    widgets.sort_by_key(|item| item.name.to_lowercase());
    widgets
}

pub async fn templates(state: &AppState, app_id: &str, language: &str) -> Vec<ShowcaseTemplate> {
    let ids: Vec<String> = template::Entity::find()
        .filter(template::Column::AppId.eq(app_id))
        .all(&state.db)
        .await
        .unwrap_or_default()
        .into_iter()
        .map(|row| row.id)
        .collect();
    let metas = localized_meta(state, meta::Column::TemplateId, ids.clone(), language).await;
    let mut templates: Vec<ShowcaseTemplate> = ids
        .into_iter()
        .filter_map(|id| {
            let row = metas.get(&id)?;
            Some(ShowcaseTemplate {
                id,
                name: row.name.clone(),
                description: row.description.clone().filter(|d| !d.trim().is_empty()),
            })
        })
        .collect();
    templates.sort_by_key(|item| item.name.to_lowercase());
    templates
}

pub async fn hood(
    state: &AppState,
    row: &app::Model,
    settings: &ShowcaseHoodSettings,
    facts: &[BoardFacts],
    language: &str,
) -> Option<ShowcaseHood> {
    if !settings.enabled {
        return None;
    }
    let metrics: Vec<&BoardSummaryMetrics> = facts.iter().filter_map(BoardFacts::metrics).collect();
    let mut hood = ShowcaseHood {
        refuse_bots: row.refuse_bots.unwrap_or(false),
        allow_forking: row.allow_forking,
        ..Default::default()
    };
    if settings.runtime {
        hood.execution_mode = Some(format!("{:?}", row.execution_mode));
        hood.requires_local = Some(metrics.iter().any(|metrics| metrics.requires_local));
    }
    if settings.models {
        hood.models = Some(models(state, row, language).await);
    }
    if settings.packages {
        hood.packages = Some(packages(state, &row.id).await);
    }
    if settings.accounts {
        let mut providers: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        for metrics in &metrics {
            for (provider, scopes) in &metrics.oauth_providers {
                providers
                    .entry(provider.clone())
                    .or_default()
                    .extend(scopes.iter().cloned());
            }
        }
        hood.oauth = Some(
            providers
                .into_iter()
                .map(|(provider, scopes)| ShowcaseOAuth {
                    provider,
                    scopes: scopes.into_iter().collect(),
                })
                .collect(),
        );
    }
    Some(hood)
}

async fn models(state: &AppState, row: &app::Model, language: &str) -> Vec<ShowcaseModel> {
    let ids: Vec<String> = row.bits.clone().map(|bits| bits.0).unwrap_or_default();
    if ids.is_empty() {
        return Vec::new();
    }
    let bits = bit::Entity::find()
        .filter(bit::Column::Id.is_in(ids.clone()))
        .all(&state.db)
        .await
        .unwrap_or_default();
    let metas = localized_meta(state, meta::Column::BitId, ids, language).await;
    let mut models: Vec<ShowcaseModel> = bits
        .into_iter()
        .map(|bit| ShowcaseModel {
            name: metas
                .get(&bit.id)
                .map(|meta| meta.name.clone())
                .or_else(|| bit.model_slug.clone())
                .unwrap_or_else(|| bit.id.clone()),
            kind: format!("{:?}", bit.r#type),
            id: bit.id,
        })
        .collect();
    models.sort_by_key(|item| item.name.to_lowercase());
    models
}

async fn packages(state: &AppState, app_id: &str) -> Vec<ShowcasePackage> {
    let installed = app_package::Entity::find()
        .filter(app_package::Column::AppId.eq(app_id))
        .filter(app_package::Column::MembershipId.is_null())
        .all(&state.db)
        .await
        .unwrap_or_default();
    if installed.is_empty() {
        return Vec::new();
    }
    let ids: Vec<String> = installed.iter().map(|row| row.package_id.clone()).collect();
    let packages: HashMap<String, wasm_package::Model> = wasm_package::Entity::find()
        .filter(wasm_package::Column::Id.is_in(ids))
        .all(&state.db)
        .await
        .unwrap_or_default()
        .into_iter()
        .map(|row| (row.id.clone(), row))
        .collect();
    let mut out: Vec<ShowcasePackage> = installed
        .into_iter()
        .filter_map(|row| {
            let package = packages.get(&row.package_id)?;
            Some(ShowcasePackage {
                id: row.package_id,
                name: package.name.clone(),
                version: row.version,
                permissions: serde_json::from_value(package.permissions.clone())
                    .unwrap_or_default(),
            })
        })
        .collect();
    out.sort_by_key(|item| item.name.to_lowercase());
    out
}

#[derive(FromQueryResult)]
struct DayRuns {
    day: chrono::NaiveDate,
    runs: i64,
}

pub async fn activity(
    state: &AppState,
    row: &app::Model,
    settings: &ShowcaseActivitySettings,
    language: &str,
) -> Option<ShowcaseActivity> {
    if !settings.enabled || (!settings.runs && !settings.versions) {
        return None;
    }
    let mut activity = ShowcaseActivity::default();
    if settings.runs {
        let today = chrono::Utc::now().date_naive();
        let start = today - chrono::Duration::days(ACTIVITY_DAYS - 1);
        let sums: HashMap<chrono::NaiveDate, i64> = execution_usage_daily::Entity::find()
            .select_only()
            .column(execution_usage_daily::Column::Day)
            .column_as(
                Expr::col(execution_usage_daily::Column::Executions).sum(),
                "runs",
            )
            .filter(execution_usage_daily::Column::AppId.eq(&row.id))
            .filter(execution_usage_daily::Column::Day.gte(start))
            .group_by(execution_usage_daily::Column::Day)
            .into_model::<DayRuns>()
            .all(&state.db)
            .await
            .unwrap_or_default()
            .into_iter()
            .map(|day| (day.day, day.runs))
            .collect();
        activity.runs = Some(
            (0..ACTIVITY_DAYS)
                .map(|offset| {
                    let day = start + chrono::Duration::days(offset);
                    ShowcaseRunDay {
                        day: day.format("%Y-%m-%d").to_string(),
                        runs: sums.get(&day).copied().unwrap_or(0),
                    }
                })
                .collect(),
        );
    }
    if settings.versions {
        let notes = localized_meta(state, meta::Column::AppId, vec![row.id.clone()], language)
            .await
            .remove(&row.id)
            .and_then(|meta| meta.release_notes)
            .or_else(|| row.changelog.clone())
            .filter(|notes| !notes.trim().is_empty());
        activity.versions = Some(
            row.version
                .clone()
                .filter(|version| !version.trim().is_empty())
                .map(|version| ShowcaseVersion {
                    version,
                    date: Some(row.updated_at.to_rfc3339()),
                    notes,
                })
                .into_iter()
                .collect(),
        );
    }
    Some(activity)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn technical_details_copy_only_route_and_schedule() {
        let config = serde_json::to_vec(&serde_json::json!({
            "path": "/orders",
            "method": "get",
            "auth_token": "secret",
            "webhook_secret": "secret"
        }))
        .unwrap();
        let api = technical("api", &config).unwrap();
        assert_eq!(api.path.as_deref(), Some("/orders"));
        assert_eq!(api.method.as_deref(), Some("GET"));
        assert!(!serde_json::to_string(&api).unwrap().contains("secret"));

        let cron = serde_json::to_vec(
            &serde_json::json!({"cron": "0 9 * * 1-5", "timezone": "Europe/Berlin"}),
        )
        .unwrap();
        let schedule = technical("cron", &cron).unwrap();
        assert_eq!(schedule.cron.as_deref(), Some("0 9 * * 1-5"));
        assert_eq!(schedule.timezone.as_deref(), Some("Europe/Berlin"));

        assert!(technical("simple_chat", &config).is_none());
        assert!(technical("api", b"{}").is_none());
    }
}
