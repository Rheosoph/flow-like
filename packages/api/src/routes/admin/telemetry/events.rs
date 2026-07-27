//! Raw anonymous telemetry event list.

use crate::entity::telemetry_event;
use crate::error::ApiError;
use crate::middleware::jwt::AppUser;
use crate::permission::global_permission::GlobalPermission;
use crate::state::AppState;
use axum::extract::{Query, State};
use axum::{Extension, Json};
use sea_orm::{ColumnTrait, EntityTrait, PaginatorTrait, QueryFilter, QueryOrder};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

#[derive(Debug, Deserialize, IntoParams)]
pub struct ListTelemetryEventsQuery {
    #[serde(default)]
    pub page: Option<u64>,
    /// Page size, capped at 100. Default 50.
    #[serde(default)]
    pub page_size: Option<u64>,
    /// Filter by exact event name.
    #[serde(default)]
    pub name: Option<String>,
    /// Filter by source.
    #[serde(default)]
    pub source: Option<String>,
    /// Filter by anonymous install identifier.
    #[serde(default)]
    pub anon_id: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct TelemetryEventRecord {
    pub id: String,
    pub name: String,
    pub source: String,
    pub anon_id: String,
    pub props: Option<serde_json::Value>,
    pub app_version: Option<String>,
    pub platform: Option<String>,
    pub client_ts: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ListTelemetryEventsResponse {
    pub events: Vec<TelemetryEventRecord>,
    pub total: u64,
    pub page: u64,
    pub page_size: u64,
}

impl From<telemetry_event::Model> for TelemetryEventRecord {
    fn from(m: telemetry_event::Model) -> Self {
        Self {
            id: m.id,
            name: m.name,
            source: m.source,
            anon_id: m.anon_id,
            props: m.props,
            app_version: m.app_version,
            platform: m.platform,
            client_ts: m.client_ts.map(|ts| ts.and_utc().to_rfc3339()),
            created_at: m.created_at.and_utc().to_rfc3339(),
        }
    }
}

#[utoipa::path(
    get,
    path = "/admin/telemetry/events",
    tag = "admin",
    params(ListTelemetryEventsQuery),
    responses(
        (status = 200, description = "Paginated list of anonymous telemetry events, newest first", body = ListTelemetryEventsResponse),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden")
    ),
    description = "List and filter raw anonymous telemetry events. Requires Admin permission."
)]
#[tracing::instrument(name = "GET /admin/telemetry/events", skip_all)]
pub async fn list_telemetry_events(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Query(q): Query<ListTelemetryEventsQuery>,
) -> Result<Json<ListTelemetryEventsResponse>, ApiError> {
    user.check_global_permission(&state, GlobalPermission::Admin)
        .await?;

    let page = q.page.unwrap_or(0);
    let page_size = q.page_size.unwrap_or(50).clamp(1, 100);

    let mut select = telemetry_event::Entity::find();

    if let Some(name) = &q.name
        && !name.is_empty()
    {
        select = select.filter(telemetry_event::Column::Name.eq(name));
    }

    if let Some(source) = &q.source
        && !source.is_empty()
    {
        select = select.filter(telemetry_event::Column::Source.eq(source));
    }

    if let Some(anon_id) = &q.anon_id
        && !anon_id.is_empty()
    {
        select = select.filter(telemetry_event::Column::AnonId.eq(anon_id));
    }

    let total = select.clone().count(&state.db).await?;

    let records = select
        .order_by_desc(telemetry_event::Column::CreatedAt)
        .paginate(&state.db, page_size)
        .fetch_page(page)
        .await?;

    Ok(Json(ListTelemetryEventsResponse {
        events: records.into_iter().map(Into::into).collect(),
        total,
        page,
        page_size,
    }))
}
