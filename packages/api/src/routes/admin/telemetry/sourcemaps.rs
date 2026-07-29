//! Source map uploads used to symbolicate minified crash stack traces.

use crate::entity::telemetry_source_map;
use crate::error::ApiError;
use crate::middleware::jwt::AppUser;
use crate::permission::global_permission::GlobalPermission;
use crate::state::AppState;
use axum::extract::State;
use axum::{Extension, Json};
use chrono::Utc;
use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, IntoActiveModel, QueryFilter, Set};
use serde::{Deserialize, Serialize};
use sourcemap::{DecodedMap, decode_slice};
use utoipa::ToSchema;

const MAX_SOURCE_MAP_BYTES: usize = 20 * 1024 * 1024;
/// Source maps exceed axum's 2MB default body limit; the route has to raise it.
pub const SOURCE_MAP_BODY_LIMIT_BYTES: usize = 24 * 1024 * 1024;

#[derive(Debug, Deserialize, ToSchema)]
pub struct UploadSourceMapPayload {
    /// Release the map belongs to, matching the release reported by clients.
    pub release: String,
    /// Origin of the build: "desktop", "web", "desktop_native" or "backend".
    pub source: String,
    /// Minified file the map belongs to, e.g. "main-abc123.js".
    pub file_name: String,
    /// Raw source map JSON.
    pub map: String,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct UploadSourceMapResponse {
    pub id: String,
}

fn require_field(name: &str, value: &str) -> Result<String, ApiError> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(ApiError::bad_request(format!(
            "'{}' must not be empty",
            name
        )));
    }
    Ok(trimmed.to_string())
}

/// A map that cannot be decoded, or carries no mappings, can never symbolicate
/// a frame — reject it at upload time instead of failing silently later.
fn validate_source_map(map: &str) -> Result<(), ApiError> {
    let decoded = decode_slice(map.as_bytes()).map_err(|err| {
        ApiError::bad_request(format!("'map' is not a readable source map: {}", err))
    })?;
    let has_mappings = match &decoded {
        DecodedMap::Regular(map) => map.get_token_count() > 0,
        _ => true,
    };
    if !has_mappings {
        return Err(ApiError::bad_request("'map' contains no mappings"));
    }
    Ok(())
}

#[utoipa::path(
    post,
    path = "/admin/telemetry/sourcemaps",
    tag = "admin",
    request_body = UploadSourceMapPayload,
    responses(
        (status = 200, description = "Identifier of the stored source map", body = UploadSourceMapResponse),
        (status = 400, description = "Missing field, oversized or unreadable source map"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden")
    ),
    description = "Upload a build source map so minified crash reports of that release show original file names and line numbers. Uploading the same file again replaces the stored map. Requires Admin permission."
)]
#[tracing::instrument(name = "POST /admin/telemetry/sourcemaps", skip(state, user, payload))]
pub async fn upload_telemetry_sourcemap(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Json(payload): Json<UploadSourceMapPayload>,
) -> Result<Json<UploadSourceMapResponse>, ApiError> {
    user.check_global_permission(&state, GlobalPermission::Admin)
        .await?;

    if payload.map.len() > MAX_SOURCE_MAP_BYTES {
        return Err(ApiError::bad_request(format!(
            "A source map may be at most {} bytes",
            MAX_SOURCE_MAP_BYTES
        )));
    }

    let release = require_field("release", &payload.release)?;
    let source = require_field("source", &payload.source)?;
    let file_name = require_field("file_name", &payload.file_name)?;

    let map = payload.map;
    validate_source_map(&map)?;

    let existing = telemetry_source_map::Entity::find()
        .filter(telemetry_source_map::Column::Release.eq(&release))
        .filter(telemetry_source_map::Column::Source.eq(&source))
        .filter(telemetry_source_map::Column::FileName.eq(&file_name))
        .one(&state.db)
        .await?;

    let id = match existing {
        Some(model) => {
            let id = model.id.clone();
            let mut active = model.into_active_model();
            active.map = Set(map);
            active.update(&state.db).await?;
            id
        }
        None => {
            let id = flow_like_types::create_id();
            telemetry_source_map::ActiveModel {
                id: Set(id.clone()),
                release: Set(release),
                source: Set(source),
                file_name: Set(file_name),
                map: Set(map),
                created_at: Set(Utc::now().naive_utc()),
            }
            .insert(&state.db)
            .await?;
            id
        }
    };

    Ok(Json(UploadSourceMapResponse { id }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn required_fields_are_trimmed_and_must_not_be_blank() {
        assert_eq!(require_field("release", "  1.2.3 ").unwrap(), "1.2.3");
        assert!(require_field("release", "   ").is_err());
        assert!(require_field("release", "").is_err());
    }

    #[test]
    fn uploads_are_rejected_unless_they_carry_usable_mappings() {
        let mut builder = sourcemap::SourceMapBuilder::new(Some("main.js"));
        builder.add(0, 0, 1, 0, Some("src/index.ts"), None, false);
        let mut buffer: Vec<u8> = Vec::new();
        builder.into_sourcemap().to_writer(&mut buffer).unwrap();
        let valid = String::from_utf8(buffer).unwrap();

        assert!(validate_source_map(&valid).is_ok());
        assert!(validate_source_map("{ not json").is_err());
        assert!(validate_source_map(r#"{"not":"a map"}"#).is_err());
        assert!(validate_source_map("").is_err());
    }
}
