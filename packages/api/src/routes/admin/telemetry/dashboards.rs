//! Saved telemetry dashboards: a name plus an ordered list of tiles, each tile
//! pointing at a stored analytics query by id.
//!
//! A tile never carries SQL. It references a `TelemetrySavedQuery` whose
//! definition was run through the `POST /admin/telemetry/query` planner when it
//! was saved, and the reference has to resolve to a row that exists. A tile that
//! still inlines a `query` object is planned here as well, so a persisted
//! dashboard can only ever describe allowlisted queries either way.

use super::query::{require_name, validate_query_definition};
use crate::entity::{telemetry_dashboard, telemetry_saved_query};
use crate::error::ApiError;
use crate::middleware::jwt::AppUser;
use crate::permission::global_permission::GlobalPermission;
use crate::state::AppState;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::{Extension, Json};
use chrono::Utc;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, EntityTrait, IntoActiveModel, QueryFilter, QueryOrder,
    QuerySelect, Set, TryIntoModel,
};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use utoipa::ToSchema;

/// Upper bound on the dashboards a single listing returns.
const MAX_DASHBOARDS: u64 = 100;
/// Upper bound on the tiles one dashboard may carry.
const MAX_TILES: usize = 24;
/// Upper bound on the tile and saved-query identifiers a tile may carry.
const MAX_TILE_ID_LEN: usize = 128;
/// Upper bound on a tile headline.
const MAX_TILE_TITLE_LEN: usize = 120;
/// Layout widths the dashboard grid renders.
const TILE_WIDTHS: [&str; 2] = ["half", "full"];
/// Presentations a tile may request for its result.
const TILE_VIEWS: [&str; 2] = ["chart", "table"];

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct TelemetryDashboardRecord {
    pub id: String,
    pub name: String,
    /// Ordered tiles, each `{ id, savedQueryId, title?, width?, view? }` with
    /// `savedQueryId` naming a stored analytics query, `width` one of "half" or
    /// "full" and `view` one of "chart" or "table".
    pub tiles: serde_json::Value,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ListTelemetryDashboardsResponse {
    pub dashboards: Vec<TelemetryDashboardRecord>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateTelemetryDashboardPayload {
    pub name: String,
    pub tiles: serde_json::Value,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct UpdateTelemetryDashboardPayload {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub tiles: Option<serde_json::Value>,
}

impl From<telemetry_dashboard::Model> for TelemetryDashboardRecord {
    fn from(model: telemetry_dashboard::Model) -> Self {
        Self {
            id: model.id,
            name: model.name,
            tiles: model.tiles,
            created_at: model.created_at.and_utc().to_rfc3339(),
            updated_at: model.updated_at.and_utc().to_rfc3339(),
        }
    }
}

type TileObject = serde_json::Map<String, serde_json::Value>;

/// Reads an optional string member. A member that is present but not a string is
/// a rejection rather than a silently ignored value.
fn tile_string<'a>(
    tile: &'a TileObject,
    key: &str,
    index: usize,
) -> Result<Option<&'a str>, ApiError> {
    match tile.get(key) {
        None | Some(serde_json::Value::Null) => Ok(None),
        Some(serde_json::Value::String(value)) => Ok(Some(value.as_str())),
        Some(_) => Err(ApiError::bad_request(format!(
            "Tile {} has a non-string '{}'",
            index + 1,
            key
        ))),
    }
}

/// Restricts a member to a closed vocabulary the dashboard grid can render.
fn tile_choice(
    tile: &TileObject,
    key: &str,
    allowed: &[&str],
    index: usize,
) -> Result<(), ApiError> {
    let Some(value) = tile_string(tile, key, index)? else {
        return Ok(());
    };

    if !allowed.contains(&value) {
        return Err(ApiError::bad_request(format!(
            "Tile {} has an unknown '{}', expected one of {}",
            index + 1,
            key,
            allowed.join(", ")
        )));
    }

    Ok(())
}

fn tile_identifier<'a>(
    tile: &'a TileObject,
    key: &str,
    index: usize,
) -> Result<Option<&'a str>, ApiError> {
    let Some(value) = tile_string(tile, key, index)?.map(str::trim) else {
        return Ok(None);
    };

    if value.is_empty() || value.len() > MAX_TILE_ID_LEN {
        return Err(ApiError::bad_request(format!(
            "Tile {} has a '{}' that is empty or longer than {} characters",
            index + 1,
            key,
            MAX_TILE_ID_LEN
        )));
    }

    Ok(Some(value))
}

/// Validates the stored tile shape and returns the saved queries the tiles
/// reference, in tile order. An inline `query` is still accepted and still runs
/// through the planner, so an older client cannot persist an unplannable
/// definition.
fn validate_tiles(tiles: &serde_json::Value) -> Result<Vec<String>, ApiError> {
    let entries = tiles
        .as_array()
        .ok_or_else(|| ApiError::bad_request("'tiles' must be an array of dashboard tiles"))?;

    if entries.len() > MAX_TILES {
        return Err(ApiError::bad_request(format!(
            "A dashboard may carry at most {} tiles",
            MAX_TILES
        )));
    }

    let mut referenced = Vec::with_capacity(entries.len());

    for (index, tile) in entries.iter().enumerate() {
        let tile = tile.as_object().ok_or_else(|| {
            ApiError::bad_request(format!("Tile {} must be an object", index + 1))
        })?;

        tile_identifier(tile, "id", index)?;

        let saved_query_id = tile_identifier(tile, "savedQueryId", index)?.ok_or_else(|| {
            ApiError::bad_request(format!(
                "Tile {} must reference a saved query through 'savedQueryId'",
                index + 1
            ))
        })?;

        if let Some(title) = tile_string(tile, "title", index)?
            && title.chars().count() > MAX_TILE_TITLE_LEN
        {
            return Err(ApiError::bad_request(format!(
                "Tile {} has a title longer than {} characters",
                index + 1,
                MAX_TILE_TITLE_LEN
            )));
        }

        tile_choice(tile, "width", &TILE_WIDTHS, index)?;
        tile_choice(tile, "view", &TILE_VIEWS, index)?;

        if let Some(query) = tile.get("query").filter(|query| !query.is_null()) {
            validate_query_definition(query).inspect_err(|_| {
                tracing::warn!(tile = index + 1, "Dashboard tile carries an invalid query");
            })?;
        }

        referenced.push(saved_query_id.to_string());
    }

    Ok(referenced)
}

/// A tile may only point at a saved query that exists, so a dashboard can never
/// reference a definition the query store never validated.
fn ensure_queries_exist(referenced: &[String], known: &HashSet<String>) -> Result<(), ApiError> {
    if let Some(missing) = referenced.iter().find(|id| !known.contains(*id)) {
        return Err(ApiError::bad_request(format!(
            "A tile references the unknown saved query '{}'",
            missing
        )));
    }

    Ok(())
}

/// Full tile check: shape, vocabularies and the existence of every referenced
/// saved query, in one round trip.
async fn check_tiles(state: &AppState, tiles: &serde_json::Value) -> Result<(), ApiError> {
    let referenced = validate_tiles(tiles)?;
    if referenced.is_empty() {
        return Ok(());
    }

    let mut lookup: Vec<String> = referenced.clone();
    lookup.sort_unstable();
    lookup.dedup();

    let known: HashSet<String> = telemetry_saved_query::Entity::find()
        .select_only()
        .column(telemetry_saved_query::Column::Id)
        .filter(telemetry_saved_query::Column::Id.is_in(lookup))
        .into_tuple::<String>()
        .all(&state.db)
        .await?
        .into_iter()
        .collect();

    ensure_queries_exist(&referenced, &known)
}

#[utoipa::path(
    get,
    path = "/admin/telemetry/dashboards",
    tag = "admin",
    responses(
        (status = 200, description = "Stored dashboards, most recently updated first", body = ListTelemetryDashboardsResponse),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden")
    ),
    description = "List the saved telemetry dashboards. Requires Admin permission."
)]
#[tracing::instrument(name = "GET /admin/telemetry/dashboards", skip(state, user))]
pub async fn list_telemetry_dashboards(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
) -> Result<Json<ListTelemetryDashboardsResponse>, ApiError> {
    user.check_global_permission(&state, GlobalPermission::Admin)
        .await?;

    let records = telemetry_dashboard::Entity::find()
        .order_by_desc(telemetry_dashboard::Column::UpdatedAt)
        .limit(MAX_DASHBOARDS)
        .all(&state.db)
        .await?;

    Ok(Json(ListTelemetryDashboardsResponse {
        dashboards: records.into_iter().map(Into::into).collect(),
    }))
}

#[utoipa::path(
    post,
    path = "/admin/telemetry/dashboards",
    tag = "admin",
    request_body = CreateTelemetryDashboardPayload,
    responses(
        (status = 200, description = "The stored dashboard", body = TelemetryDashboardRecord),
        (status = 400, description = "Missing name, malformed tiles or an unknown saved query"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden")
    ),
    description = "Store a telemetry dashboard. Every tile has to reference a saved analytics query that exists; inline tile queries are validated against the same allowlist the query endpoint uses. Requires Admin permission."
)]
#[tracing::instrument(name = "POST /admin/telemetry/dashboards", skip(state, user, payload))]
pub async fn create_telemetry_dashboard(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Json(payload): Json<CreateTelemetryDashboardPayload>,
) -> Result<Json<TelemetryDashboardRecord>, ApiError> {
    user.check_global_permission(&state, GlobalPermission::Admin)
        .await?;

    let name = require_name("name", &payload.name)?;
    check_tiles(&state, &payload.tiles).await?;

    let now = Utc::now().naive_utc();
    let model = telemetry_dashboard::ActiveModel {
        id: Set(flow_like_types::create_id()),
        name: Set(name),
        tiles: Set(payload.tiles),
        created_at: Set(now),
        updated_at: Set(now),
    }
    .insert(&state.db)
    .await?;

    Ok(Json(model.into()))
}

#[utoipa::path(
    patch,
    path = "/admin/telemetry/dashboards/{dashboard_id}",
    tag = "admin",
    params(("dashboard_id" = String, Path, description = "Dashboard identifier")),
    request_body = UpdateTelemetryDashboardPayload,
    responses(
        (status = 200, description = "The updated dashboard", body = TelemetryDashboardRecord),
        (status = 400, description = "Empty name, malformed tiles or an unknown saved query"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
        (status = 404, description = "Dashboard not found")
    ),
    description = "Rename a telemetry dashboard or replace its tiles. Every tile has to reference a saved analytics query that exists; inline tile queries are validated against the same allowlist the query endpoint uses. Requires Admin permission."
)]
#[tracing::instrument(
    name = "PATCH /admin/telemetry/dashboards/{dashboard_id}",
    skip(state, user, payload)
)]
pub async fn update_telemetry_dashboard(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(dashboard_id): Path<String>,
    Json(payload): Json<UpdateTelemetryDashboardPayload>,
) -> Result<Json<TelemetryDashboardRecord>, ApiError> {
    user.check_global_permission(&state, GlobalPermission::Admin)
        .await?;

    let model = telemetry_dashboard::Entity::find_by_id(&dashboard_id)
        .one(&state.db)
        .await?
        .ok_or(ApiError::NOT_FOUND)?;

    let mut active = model.into_active_model();

    if let Some(name) = &payload.name {
        active.name = Set(require_name("name", name)?);
    }

    if let Some(tiles) = payload.tiles {
        check_tiles(&state, &tiles).await?;
        active.tiles = Set(tiles);
    }

    if !active.is_changed() {
        return Ok(Json(active.try_into_model()?.into()));
    }

    active.updated_at = Set(Utc::now().naive_utc());
    let model = active.update(&state.db).await?;

    Ok(Json(model.into()))
}

#[utoipa::path(
    delete,
    path = "/admin/telemetry/dashboards/{dashboard_id}",
    tag = "admin",
    params(("dashboard_id" = String, Path, description = "Dashboard identifier")),
    responses(
        (status = 204, description = "Dashboard deleted"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
        (status = 404, description = "Dashboard not found")
    ),
    description = "Delete a telemetry dashboard. Requires Admin permission."
)]
#[tracing::instrument(
    name = "DELETE /admin/telemetry/dashboards/{dashboard_id}",
    skip(state, user)
)]
pub async fn delete_telemetry_dashboard(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(dashboard_id): Path<String>,
) -> Result<StatusCode, ApiError> {
    user.check_global_permission(&state, GlobalPermission::Admin)
        .await?;

    let result = telemetry_dashboard::Entity::delete_by_id(&dashboard_id)
        .exec(&state.db)
        .await?;

    if result.rows_affected == 0 {
        return Err(ApiError::NOT_FOUND);
    }

    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::response::IntoResponse;

    fn status(error: ApiError) -> StatusCode {
        error.into_response().status()
    }

    fn assert_bad_request<T: std::fmt::Debug>(result: Result<T, ApiError>) {
        match result {
            Ok(value) => panic!("expected a rejection, accepted {:?}", value),
            Err(error) => assert_eq!(status(error), StatusCode::BAD_REQUEST),
        }
    }

    /// The shape the dashboard page actually sends.
    fn tile(saved_query_id: &str) -> serde_json::Value {
        serde_json::json!({
            "id": "8f5a0d0e-9a3f-4a2b-9c11-6f2d0f0f2b7a",
            "savedQueryId": saved_query_id,
            "title": "Crashes per hour",
            "width": "half",
            "view": "chart"
        })
    }

    fn valid_query() -> serde_json::Value {
        serde_json::json!({
            "dataset": "events",
            "metric": { "type": "count" },
            "interval": "hour",
            "hours": 24
        })
    }

    fn known(ids: &[&str]) -> HashSet<String> {
        ids.iter().map(|id| (*id).to_string()).collect()
    }

    #[test]
    fn tiles_reference_a_saved_query_by_id() {
        assert_eq!(
            validate_tiles(&serde_json::json!([])).unwrap(),
            Vec::<String>::new()
        );
        assert_eq!(
            validate_tiles(&serde_json::json!([tile("query-1"), tile("query-2")])).unwrap(),
            vec!["query-1".to_string(), "query-2".to_string()]
        );

        assert!(
            validate_tiles(&serde_json::json!([{ "savedQueryId": "query-1" }])).is_ok(),
            "the optional members really are optional"
        );

        assert_bad_request(validate_tiles(&serde_json::json!({})));
        assert_bad_request(validate_tiles(&serde_json::json!("tiles")));
        assert_bad_request(validate_tiles(&serde_json::json!([1, 2, 3])));
        assert_bad_request(validate_tiles(
            &serde_json::json!([{ "id": "tile-1", "title": "No reference" }]),
        ));
        assert_bad_request(validate_tiles(&serde_json::json!([{ "savedQueryId": 7 }])));
        assert_bad_request(validate_tiles(
            &serde_json::json!([{ "savedQueryId": "  " }]),
        ));
        assert_bad_request(validate_tiles(
            &serde_json::json!([{ "savedQueryId": "q".repeat(MAX_TILE_ID_LEN + 1) }]),
        ));
        assert_bad_request(validate_tiles(
            &serde_json::json!([{ "savedQueryId": "query-1", "id": 7 }]),
        ));
    }

    #[test]
    fn tile_presentation_comes_from_a_closed_vocabulary() {
        for width in TILE_WIDTHS {
            assert!(
                validate_tiles(&serde_json::json!([{ "savedQueryId": "query-1", "width": width }]))
                    .is_ok(),
                "{width}"
            );
        }
        for view in TILE_VIEWS {
            assert!(
                validate_tiles(&serde_json::json!([{ "savedQueryId": "query-1", "view": view }]))
                    .is_ok(),
                "{view}"
            );
        }

        assert_bad_request(validate_tiles(
            &serde_json::json!([{ "savedQueryId": "query-1", "width": "third" }]),
        ));
        assert_bad_request(validate_tiles(
            &serde_json::json!([{ "savedQueryId": "query-1", "width": 2 }]),
        ));
        assert_bad_request(validate_tiles(
            &serde_json::json!([{ "savedQueryId": "query-1", "view": "flamegraph" }]),
        ));
        assert_bad_request(validate_tiles(
            &serde_json::json!([{ "savedQueryId": "query-1", "title": 7 }]),
        ));
        assert_bad_request(validate_tiles(
            &serde_json::json!([{ "savedQueryId": "query-1", "title": "t".repeat(MAX_TILE_TITLE_LEN + 1) }]),
        ));
    }

    #[test]
    fn unknown_saved_queries_are_rejected() {
        let referenced = validate_tiles(&serde_json::json!([tile("query-1"), tile("query-2")]))
            .expect("validated");

        assert!(ensure_queries_exist(&referenced, &known(&["query-1", "query-2"])).is_ok());
        assert_bad_request(ensure_queries_exist(
            &referenced,
            &known(&["query-1", "query-3"]),
        ));
        assert_bad_request(ensure_queries_exist(&referenced, &known(&[])));
        assert!(ensure_queries_exist(&[], &known(&[])).is_ok());
    }

    #[test]
    fn inline_tile_queries_still_go_through_the_query_planner() {
        assert!(
            validate_tiles(&serde_json::json!([{
                "savedQueryId": "query-1",
                "query": valid_query()
            }]))
            .is_ok()
        );

        for rejected in [
            serde_json::json!({ "dataset": "users", "metric": { "type": "count" } }),
            serde_json::json!({
                "dataset": "events",
                "metric": { "type": "count" },
                "breakdown": "platform\"; DROP TABLE \"TelemetryEvent\"; --"
            }),
            serde_json::json!("SELECT * FROM \"User\""),
            serde_json::json!({
                "dataset": "spans",
                "metric": { "type": "p95", "field": "name" }
            }),
        ] {
            assert_bad_request(validate_tiles(&serde_json::json!([{
                "savedQueryId": "query-1",
                "query": rejected
            }])));
        }
    }

    #[test]
    fn tile_count_is_capped() {
        let tiles: Vec<serde_json::Value> = (0..=MAX_TILES).map(|_| tile("query-1")).collect();
        assert_bad_request(validate_tiles(&serde_json::Value::Array(tiles)));

        let full: Vec<serde_json::Value> = (0..MAX_TILES).map(|_| tile("query-1")).collect();
        assert!(validate_tiles(&serde_json::Value::Array(full)).is_ok());
    }

    #[test]
    fn payloads_deserialize_with_optional_patch_fields() {
        let create: CreateTelemetryDashboardPayload =
            serde_json::from_str(r#"{"name":"Health","tiles":[]}"#).unwrap();
        assert_eq!(create.name, "Health");
        assert!(create.tiles.is_array());

        let rename: UpdateTelemetryDashboardPayload =
            serde_json::from_str(r#"{"name":"Renamed"}"#).unwrap();
        assert_eq!(rename.name.as_deref(), Some("Renamed"));
        assert!(rename.tiles.is_none());
    }
}
