use super::{build, cache_key, language, types::*};
use crate::{
    entity::app, error::ApiError, middleware::jwt::AppUser,
    routes::app::ensure_app_publicly_visible, state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, Query, State},
};
use sea_orm::EntityTrait;

#[derive(Debug, serde::Deserialize, utoipa::IntoParams)]
pub struct ShowcaseQuery {
    /// Locale for widget, template and model names. Unknown locales read English.
    pub language: Option<String>,
}

#[utoipa::path(
    get,
    path = "/apps/{app_id}/showcase",
    tag = "apps",
    description = "What the app's store page shows beyond its description: the data, ways to use it, pages, widgets, templates, requirements and activity its owner chose to show.",
    params(("app_id" = String, Path, description = "Application ID"), ShowcaseQuery),
    responses(
        (status = 200, description = "The parts of the app its owner shows on the store page", body = AppShowcase),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "The app is private and you are not a member"),
        (status = 404, description = "Application not found")
    )
)]
#[tracing::instrument(name = "GET /apps/{app_id}/showcase", skip(state, user, query))]
pub async fn get_showcase(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(app_id): Path<String>,
    Query(query): Query<ShowcaseQuery>,
) -> Result<Json<AppShowcase>, ApiError> {
    let row = if user.app_permission(&app_id, &state).await.is_ok() {
        app::Entity::find_by_id(&app_id)
            .one(&state.db)
            .await?
            .ok_or(ApiError::NOT_FOUND)?
    } else {
        if !state.platform_config.features.unauthorized_read {
            user.sub()?;
        }
        ensure_app_publicly_visible(&app_id, &state).await?
    };

    let language = language(query.language.as_deref());
    let key = cache_key(&app_id, language);
    if let Some(cached) = state.get_cache::<AppShowcase>(&key) {
        return Ok(Json(cached));
    }

    let showcase = assemble(&state, &row, language).await;
    state.set_cache(key, &showcase);
    Ok(Json(showcase))
}

/// Every section reads in parallel; each one checks its own switches.
async fn assemble(state: &AppState, row: &app::Model, language: &str) -> AppShowcase {
    let mut showcase = AppShowcase {
        app_id: row.id.clone(),
        ..Default::default()
    };
    if row.bot_id.is_some() {
        return showcase;
    }
    let settings = ShowcaseSettings::from_column(row.showcase.as_ref()).normalized();
    let app_id = row.id.as_str();
    let show_flows = settings.flows.enabled;
    let needs_boards = show_flows || settings.hood.enabled || settings.map;

    let (data, sources, events, facts, widgets, templates) = futures::join!(
        build::tables(state, app_id, &settings.data),
        async {
            if settings.data.enabled && settings.data.sources {
                build::sources(state, app_id).await
            } else {
                Vec::new()
            }
        },
        build::events(state, app_id, &settings.triggers),
        async {
            let facts: Vec<build::BoardFacts> = if needs_boards {
                build::board_facts(state, app_id).await
            } else {
                Vec::new()
            };
            facts
        },
        async {
            if settings.widgets.enabled {
                build::widgets(state, app_id, language).await
            } else {
                Vec::new()
            }
        },
        async {
            if settings.templates.enabled {
                build::templates(state, app_id, language).await
            } else {
                Vec::new()
            }
        },
    );

    let (pages, hood, activity) = futures::join!(
        build::pages(state, app_id, &settings.pages),
        build::hood(state, row, &settings.hood, &facts, language),
        build::activity(state, row, &settings.activity, language),
    );

    let shown_tables: Vec<String> = data.tables.iter().map(|table| table.name.clone()).collect();
    let flows = build::flows(&facts, &shown_tables, &pages);
    showcase.events = build::link_events(events, &flows, &pages, show_flows);
    if show_flows {
        showcase.flows = flows;
    }
    showcase.map = settings.map;
    showcase.tables = data.tables;
    showcase.links = data.links;
    showcase.sources = sources;
    showcase.pages = pages
        .into_iter()
        .map(|mut page| {
            page.board_id = page.board_id.filter(|_| show_flows);
            page
        })
        .collect();
    showcase.widgets = widgets;
    showcase.templates = templates;
    showcase.hood = hood;
    showcase.activity = activity;
    showcase
}
