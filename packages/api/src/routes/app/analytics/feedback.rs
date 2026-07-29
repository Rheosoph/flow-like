use crate::{
    ensure_permission,
    entity::{event, feedback},
    error::ApiError,
    middleware::jwt::AppUser,
    permission::role_permission::RolePermissions,
    routes::app::events::db::get_event_with_fallback_opt,
    state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, Query, State},
};
use flow_like::flow::event::Event as CoreEvent;
use sea_orm::{
    ColumnTrait, Condition, EntityTrait, PaginatorTrait, QueryFilter, QueryOrder, QuerySelect,
};
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;
use std::collections::{HashMap, HashSet};
use utoipa::ToSchema;

#[derive(Debug, Deserialize, ToSchema)]
pub struct FeedbackQuery {
    /// Pagination offset
    #[serde(default)]
    pub offset: u64,
    /// Items per page (max 100)
    #[serde(default = "default_limit")]
    pub limit: u64,
    /// Minimum rating filter
    pub min_rating: Option<i64>,
    /// Maximum rating filter
    pub max_rating: Option<i64>,
    /// Optional event ID filter
    pub event_id: Option<String>,
}

fn default_limit() -> u64 {
    50
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct FeedbackItem {
    pub id: String,
    pub user_id: Option<String>,
    pub event_id: Option<String>,
    pub event_name: Option<String>,
    pub event_route: Option<String>,
    pub event_page_id: Option<String>,
    pub page_path: Option<String>,
    pub page_search: Option<String>,
    pub page_hash: Option<String>,
    pub route_pathname: Option<String>,
    pub rating: i64,
    pub comment: String,
    pub created_at: String,
}

#[derive(Debug, Default)]
struct FeedbackPageContext {
    page_path: Option<String>,
    page_search: Option<String>,
    page_hash: Option<String>,
    route_pathname: Option<String>,
}

#[derive(Clone, Debug, Default)]
struct FeedbackEventDetails {
    event_name: Option<String>,
    event_route: Option<String>,
    event_page_id: Option<String>,
}

impl FeedbackEventDetails {
    fn fill_missing_from(&mut self, other: &FeedbackEventDetails) {
        if self.event_name.is_none() {
            self.event_name = other.event_name.clone();
        }
        if self.event_route.is_none() {
            self.event_route = other.event_route.clone();
        }
        if self.event_page_id.is_none() {
            self.event_page_id = other.event_page_id.clone();
        }
    }
}

fn clean_json_string(value: &JsonValue) -> Option<String> {
    value
        .as_str()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn extract_event_context(context: Option<&JsonValue>) -> FeedbackEventDetails {
    let Some(context) = context else {
        return FeedbackEventDetails::default();
    };

    let local_state_event_context = context
        .get("local_state")
        .or_else(|| context.get("localState"))
        .and_then(|local_state| {
            local_state
                .get("eventContext")
                .or_else(|| local_state.get("event_context"))
        });

    let event_context = local_state_event_context
        .or_else(|| context.get("eventContext"))
        .or_else(|| context.get("event_context"))
        .or_else(|| context.get("event"));

    let Some(event_context) = event_context else {
        return FeedbackEventDetails::default();
    };

    FeedbackEventDetails {
        event_name: json_path_string(event_context, &["name"]),
        event_route: json_path_string(event_context, &["route"]),
        event_page_id: json_path_string(event_context, &["pageId"])
            .or_else(|| json_path_string(event_context, &["page_id"]))
            .or_else(|| json_path_string(event_context, &["defaultPageId"]))
            .or_else(|| json_path_string(event_context, &["default_page_id"])),
    }
}

fn db_event_details(event: event::Model) -> FeedbackEventDetails {
    FeedbackEventDetails {
        event_name: Some(event.name),
        event_route: event.route,
        event_page_id: event.page_id,
    }
}

fn core_event_details(event: &CoreEvent) -> FeedbackEventDetails {
    FeedbackEventDetails {
        event_name: Some(event.name.clone()),
        event_route: event.route.clone(),
        event_page_id: event.default_page_id.clone(),
    }
}

fn json_path_string(value: &JsonValue, path: &[&str]) -> Option<String> {
    let mut current = value;
    for segment in path {
        current = current.get(*segment)?;
    }
    clean_json_string(current)
}

fn extract_page_context(context: Option<&JsonValue>) -> FeedbackPageContext {
    let Some(context) = context else {
        return FeedbackPageContext::default();
    };

    let page_context = context
        .get("local_state")
        .or_else(|| context.get("localState"))
        .and_then(|local_state| {
            local_state
                .get("pageContext")
                .or_else(|| local_state.get("page_context"))
        });

    let Some(page_context) = page_context else {
        return FeedbackPageContext::default();
    };

    FeedbackPageContext {
        page_path: json_path_string(page_context, &["pathname"]),
        page_search: json_path_string(page_context, &["search"]),
        page_hash: json_path_string(page_context, &["hash"]),
        route_pathname: json_path_string(page_context, &["routePathname"])
            .or_else(|| json_path_string(page_context, &["route_pathname"])),
    }
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct PaginatedFeedback {
    pub items: Vec<FeedbackItem>,
    pub total: u64,
    pub offset: u64,
    pub limit: u64,
}

/// GET /apps/{app_id}/analytics/feedback - List feedback entries
#[utoipa::path(
    get,
    path = "/apps/{app_id}/analytics/feedback",
    tag = "analytics",
    description = "List feedback entries for an app.",
    params(
        ("app_id" = String, Path, description = "Application ID"),
        ("offset" = Option<u64>, Query, description = "Pagination offset"),
        ("limit" = Option<u64>, Query, description = "Items per page (max 100)"),
        ("min_rating" = Option<i64>, Query, description = "Minimum rating filter"),
        ("max_rating" = Option<i64>, Query, description = "Maximum rating filter"),
        ("event_id" = Option<String>, Query, description = "Optional event ID filter")
    ),
    responses(
        (status = 200, description = "Feedback list", body = PaginatedFeedback),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden")
    ),
    security(
        ("bearer_auth" = []),
        ("api_key" = []),
        ("pat" = [])
    )
)]
#[tracing::instrument(
    name = "GET /apps/{app_id}/analytics/feedback",
    skip(state, user, query)
)]
pub async fn list_feedback(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(app_id): Path<String>,
    Query(query): Query<FeedbackQuery>,
) -> Result<Json<PaginatedFeedback>, ApiError> {
    ensure_permission!(user, &app_id, &state, RolePermissions::ReadAnalytics);

    let limit = query.limit.min(100);
    let offset = query.offset;

    let mut condition = Condition::all().add(feedback::Column::AppId.eq(&app_id));

    if let Some(min) = query.min_rating {
        condition = condition.add(feedback::Column::Rating.gte(min));
    }
    if let Some(max) = query.max_rating {
        condition = condition.add(feedback::Column::Rating.lte(max));
    }
    if let Some(event_id) = query
        .event_id
        .as_deref()
        .map(str::trim)
        .filter(|event_id| !event_id.is_empty())
        .filter(|event_id| !event_id.eq_ignore_ascii_case("all"))
    {
        condition = condition.add(feedback::Column::EventId.eq(event_id));
    }

    let total = feedback::Entity::find()
        .filter(condition.clone())
        .count(&state.db)
        .await?;

    let records = feedback::Entity::find()
        .filter(condition)
        .order_by_desc(feedback::Column::CreatedAt)
        .offset(Some(offset))
        .limit(Some(limit))
        .all(&state.db)
        .await?;

    let event_ids: Vec<String> = records
        .iter()
        .filter_map(|record| record.event_id.clone())
        .collect::<HashSet<_>>()
        .into_iter()
        .collect();
    let mut event_lookup: HashMap<String, FeedbackEventDetails> = if event_ids.is_empty() {
        HashMap::new()
    } else {
        event::Entity::find()
            .filter(event::Column::AppId.eq(&app_id))
            .filter(event::Column::Id.is_in(event_ids.clone()))
            .all(&state.db)
            .await?
            .into_iter()
            .map(|event| (event.id.clone(), db_event_details(event)))
            .collect()
    };

    let missing_event_ids: Vec<String> = event_ids
        .iter()
        .filter(|event_id| !event_lookup.contains_key(*event_id))
        .cloned()
        .collect();
    if !missing_event_ids.is_empty() {
        match state.master_app("", &app_id, &state).await {
            Ok(app) => {
                for event_id in missing_event_ids {
                    match get_event_with_fallback_opt(&state.db, &app, &event_id).await {
                        Ok(Some(event)) => {
                            event_lookup.insert(event_id, core_event_details(&event));
                        }
                        Ok(None) => {}
                        Err(error) => {
                            tracing::warn!(
                                event_id = %event_id,
                                error = %error,
                                "failed to load event metadata for feedback analytics"
                            );
                        }
                    }
                }
            }
            Err(error) => {
                tracing::warn!(
                    app_id = %app_id,
                    error = %error,
                    "failed to load app metadata for feedback analytics"
                );
            }
        }
    }

    let items = records
        .into_iter()
        .map(|r| {
            let page_context = extract_page_context(r.context.as_ref());
            let mut event_details = extract_event_context(r.context.as_ref());
            if let Some(lookup_details) = r
                .event_id
                .as_ref()
                .and_then(|event_id| event_lookup.get(event_id))
            {
                event_details.fill_missing_from(lookup_details);
            }

            FeedbackItem {
                id: r.id,
                user_id: r.user_id,
                event_id: r.event_id,
                event_name: event_details.event_name,
                event_route: event_details.event_route,
                event_page_id: event_details.event_page_id,
                page_path: page_context.page_path,
                page_search: page_context.page_search,
                page_hash: page_context.page_hash,
                route_pathname: page_context.route_pathname,
                rating: r.rating,
                comment: r.comment,
                created_at: r.created_at.format("%Y-%m-%dT%H:%M:%SZ").to_string(),
            }
        })
        .collect();

    Ok(Json(PaginatedFeedback {
        items,
        total,
        offset,
        limit,
    }))
}
