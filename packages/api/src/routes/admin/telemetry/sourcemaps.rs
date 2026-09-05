//! Source map uploads used to symbolicate minified crash stack traces.
//!
//! The map itself never enters the row. A build's source map runs to tens of
//! megabytes while Aurora DSQL rejects any text value over 1 MiB, so the bytes
//! go to the meta store and `TelemetrySourceMap.mapRef` keeps the reference —
//! the same claim check `ExecutionEvent.payloadRef` runs for oversized event
//! payloads. Rows written before this keep their inline `map` and still
//! symbolicate.
//!
//! The change is one-way for the API binary. An upload writes `map = NULL`, and
//! an older binary's model declares `map` as `NOT NULL`, so it fails to
//! deserialize any row this one wrote and 500s the whole issue detail endpoint
//! for that release - roll forward past this, never back.

use crate::entity::telemetry_source_map;
use crate::error::ApiError;
use crate::middleware::jwt::AppUser;
use crate::permission::global_permission::GlobalPermission;
use crate::state::AppState;
use axum::extract::State;
use axum::{Extension, Json};
use chrono::Utc;
use flow_like_storage::{
    files::store::FlowLikeStore,
    object_store::{Error as ObjectStoreError, path::Path},
};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, DatabaseConnection, DbErr, EntityTrait, QueryFilter, Set,
    UpdateResult,
};
use serde::{Deserialize, Serialize};
use sourcemap::{DecodedMap, decode_slice};
use utoipa::ToSchema;

const MAX_SOURCE_MAP_BYTES: usize = 20 * 1024 * 1024;
/// `release`, `source` and `file_name` are stored verbatim in `TEXT NOT NULL`
/// columns that also form a unique index. Only `map` was ever bounded, so
/// without this the 24 MB body limit is their real cap and a large one trips
/// exactly the 1 MiB Aurora DSQL limit this module exists to avoid.
const MAX_SOURCE_MAP_KEY_BYTES: usize = 512;
/// Source maps exceed axum's 2MB default body limit; the route has to raise it.
pub const SOURCE_MAP_BODY_LIMIT_BYTES: usize = 24 * 1024 * 1024;
/// Prefix of the stored maps on the meta store. Unlike the staged execution
/// payloads this is not under `tmp/`: a map lives exactly as long as its row,
/// so no lifecycle sweep may reclaim it.
const SOURCE_MAP_PREFIX: &str = "telemetry/sourcemaps";

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
    if trimmed.len() > MAX_SOURCE_MAP_KEY_BYTES {
        return Err(ApiError::bad_request(format!(
            "'{}' must be at most {} bytes",
            name, MAX_SOURCE_MAP_KEY_BYTES
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
        (status = 403, description = "Forbidden"),
        (status = 409, description = "A concurrent upload replaced the source map; retry the upload")
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

    // The object goes first: a row that named an object the store never took
    // would symbolicate nothing, while an object no row ends up naming is only
    // dead weight the next upload of the same bytes reuses.
    let reference = store_source_map(
        &state.meta_bucket,
        &release,
        &source,
        &file_name,
        map.into_bytes(),
    )
    .await?;

    let (id, superseded) = match write_source_map_row(
        &state.db, existing, &release, &source, &file_name, &reference,
    )
    .await
    {
        Ok(written) => written,
        Err(error) => {
            cleanup_failed_source_map_upload(
                &state.db,
                &state.meta_bucket,
                &error,
                &release,
                &source,
                &file_name,
                &reference,
            )
            .await;
            return Err(error);
        }
    };

    // Only after the row stopped naming it: an object deleted before the update
    // commits would leave a live row pointing at nothing.
    if let Some(superseded) = superseded.filter(|old| *old != reference) {
        delete_source_map(&state.meta_bucket, &superseded).await;
    }

    Ok(Json(UploadSourceMapResponse { id }))
}

/// Point the row for `(release, source, file_name)` at `reference`, returning
/// its id and the reference it displaced.
///
/// The update is a compare-and-set on that displaced value. Two uploads of the
/// same file race on one row, and both snapshot the same predecessor: without
/// the guard the loser deletes the object the winner's committed row now names,
/// which is silently lossy rather than merely wasteful - the frames of that
/// release stop symbolicating until someone re-uploads. `rows_affected == 0`
/// means another writer repointed the row first, and what it displaced is that
/// writer's to delete. Return a conflict so the caller can retry its upload.
async fn write_source_map_row(
    db: &DatabaseConnection,
    existing: Option<telemetry_source_map::Model>,
    release: &str,
    source: &str,
    file_name: &str,
    reference: &str,
) -> Result<(String, Option<String>), ApiError> {
    let Some(model) = existing else {
        let id = flow_like_types::create_id();
        new_source_map_model(
            id.clone(),
            release.to_string(),
            source.to_string(),
            file_name.to_string(),
            reference.to_string(),
        )
        .insert(db)
        .await?;
        return Ok((id, None));
    };

    let id = model.id.clone();
    let superseded = model.map_ref.clone();
    let update = telemetry_source_map::Entity::update_many()
        .col_expr(
            telemetry_source_map::Column::Map,
            sea_orm::sea_query::Expr::value(Option::<String>::None),
        )
        .col_expr(
            telemetry_source_map::Column::MapRef,
            sea_orm::sea_query::Expr::value(Some(reference.to_string())),
        )
        .filter(telemetry_source_map::Column::Id.eq(id.clone()));
    let update = match superseded.as_deref() {
        Some(old) => update.filter(telemetry_source_map::Column::MapRef.eq(old)),
        None => update.filter(telemetry_source_map::Column::MapRef.is_null()),
    };
    let result = update.exec(db).await?;
    require_source_map_update(result)?;

    Ok((id, superseded))
}

fn require_source_map_update(result: UpdateResult) -> Result<(), ApiError> {
    if result.rows_affected == 0 {
        return Err(ApiError::conflict(
            "A concurrent upload replaced the source map; retry the upload",
        ));
    }
    Ok(())
}

async fn cleanup_failed_source_map_upload(
    db: &DatabaseConnection,
    store: &FlowLikeStore,
    error: &ApiError,
    release: &str,
    source: &str,
    file_name: &str,
    reference: &str,
) {
    // A competing upload may still commit this same reference. A conflict
    // does not establish ownership of the object, so leave it in place.
    if error.status() == axum::http::StatusCode::CONFLICT {
        return;
    }
    // A failed write may have committed, or this may be an identical re-upload
    // of the live object. Preserve it whenever verification is unavailable.
    if matches!(
        reference_is_committed(db, release, source, file_name, reference).await,
        Ok(false)
    ) {
        delete_source_map(store, reference).await;
    }
}

/// Whether a committed row already names `reference`. A query failure leaves
/// ownership unknown and must not be treated as an unreferenced object.
async fn reference_is_committed(
    db: &DatabaseConnection,
    release: &str,
    source: &str,
    file_name: &str,
    reference: &str,
) -> Result<bool, DbErr> {
    let committed = telemetry_source_map::Entity::find()
        .filter(telemetry_source_map::Column::Release.eq(release))
        .filter(telemetry_source_map::Column::Source.eq(source))
        .filter(telemetry_source_map::Column::FileName.eq(file_name))
        .one(db)
        .await?
        .and_then(|model| model.map_ref);
    Ok(committed.is_some_and(|current| current == reference))
}

/// The row a first upload of `file_name` writes. `map` stays null: the bytes
/// live on the meta store and only rows written before that carry them inline.
fn new_source_map_model(
    id: String,
    release: String,
    source: String,
    file_name: String,
    reference: String,
) -> telemetry_source_map::ActiveModel {
    telemetry_source_map::ActiveModel {
        id: Set(id),
        release: Set(release),
        source: Set(source),
        file_name: Set(file_name),
        map: Set(None),
        map_ref: Set(Some(reference)),
        created_at: Set(Utc::now().fixed_offset()),
    }
}

/// Object holding the map of one row.
///
/// The release, and the source and file name together, are hashed rather than
/// spelled out: both are caller-supplied strings that must not be able to shape
/// the key, and a hash is fixed width. The release stays its own segment so
/// every map of one build shares a prefix a future release cleanup can list.
///
/// The body hash makes the path content addressed, so re-uploading identical
/// bytes rewrites the same object instead of stranding a second one, and a
/// changed map lands beside the old one until the row is repointed at it.
fn source_map_path(release: &str, source: &str, file_name: &str, body: &[u8]) -> Path {
    let release_hash = blake3::hash(release.as_bytes()).to_hex();
    let mut file_key = Vec::with_capacity(source.len() + file_name.len() + 1);
    file_key.extend_from_slice(source.as_bytes());
    file_key.push(0);
    file_key.extend_from_slice(file_name.as_bytes());
    let file_hash = blake3::hash(&file_key).to_hex();
    let body_hash = blake3::hash(body).to_hex();
    Path::from(format!(
        "{SOURCE_MAP_PREFIX}/{release_hash}/{file_hash}-{body_hash}.map"
    ))
}

/// The path a stored `mapRef` names, tolerating the `s3://bucket/` form the
/// execution claim check also accepts.
pub(crate) fn source_map_reference_path(reference: &str) -> Path {
    let path = reference
        .strip_prefix("store://")
        .or_else(|| {
            reference
                .strip_prefix("s3://")
                .and_then(|rest| rest.split_once('/').map(|(_, path)| path))
        })
        .unwrap_or(reference);
    Path::from(path)
}

/// Write the map to the meta store and return the reference the row keeps.
async fn store_source_map(
    store: &FlowLikeStore,
    release: &str,
    source: &str,
    file_name: &str,
    body: Vec<u8>,
) -> Result<String, ApiError> {
    let path = source_map_path(release, source, file_name, &body);
    store
        .as_generic()
        .put(&path, body.into())
        .await
        .map_err(|error| {
            ApiError::internal(format!(
                "Storing the source map of '{file_name}' at '{path}' failed: {error}"
            ))
        })?;
    Ok(format!("store://{path}"))
}

/// Read a stored map back. Errors are the caller's to degrade on: symbolication
/// is best effort and a map it cannot load is a map it does without.
pub(crate) async fn load_source_map(
    store: &FlowLikeStore,
    reference: &str,
) -> Result<Vec<u8>, ObjectStoreError> {
    let path = source_map_reference_path(reference);
    let object = store.as_generic().get(&path).await?;
    Ok(object.bytes().await?.to_vec())
}

/// Delete the object a `mapRef` names. An object that is already gone is in the
/// state the caller wants, so only a real failure is worth a line — and it must
/// never fail the upload that superseded it.
pub(crate) async fn delete_source_map(store: &FlowLikeStore, reference: &str) -> bool {
    let path = source_map_reference_path(reference);
    match store.as_generic().delete(&path).await {
        Ok(()) | Err(ObjectStoreError::NotFound { .. }) => true,
        Err(error) => {
            tracing::warn!(
                %path,
                %error,
                "Stored source map could not be deleted; the object is orphaned"
            );
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use flow_like_storage::object_store::{ObjectStore, memory::InMemory};
    use std::sync::Arc;

    fn memory_store() -> FlowLikeStore {
        FlowLikeStore::Memory(Arc::new(InMemory::new()))
    }

    async fn listed(store: &FlowLikeStore) -> Vec<String> {
        use futures::TryStreamExt;
        let mut paths: Vec<String> = store
            .as_generic()
            .list(None)
            .map_ok(|meta| meta.location.to_string())
            .try_collect()
            .await
            .unwrap();
        paths.sort();
        paths
    }

    #[test]
    fn required_fields_are_trimmed_and_must_not_be_blank() {
        assert_eq!(require_field("release", "  1.2.3 ").unwrap(), "1.2.3");
        assert!(require_field("release", "   ").is_err());
        assert!(require_field("release", "").is_err());
    }

    /// The key fields land in `TEXT NOT NULL` columns of the row itself, so
    /// unlike `map` they are not covered by the claim check - only the body
    /// limit ever bounded them, and Aurora DSQL rejects any text over 1 MiB.
    #[test]
    fn key_fields_are_bounded_so_they_cannot_reach_the_column_limit() {
        let at_limit = "a".repeat(MAX_SOURCE_MAP_KEY_BYTES);
        assert_eq!(require_field("release", &at_limit).unwrap(), at_limit);

        let over = "a".repeat(MAX_SOURCE_MAP_KEY_BYTES + 1);
        assert!(require_field("release", &over).is_err());
        assert!(require_field("source", &over).is_err());
        assert!(require_field("file_name", &over).is_err());

        // Trimmed first, so surrounding whitespace does not count against it.
        let padded = format!("  {at_limit}  ");
        assert_eq!(require_field("release", &padded).unwrap(), at_limit);
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

    /// The row must never carry the bytes: that column is what the 1 MiB
    /// Aurora DSQL text limit applies to.
    #[test]
    fn a_new_row_carries_the_reference_and_no_inline_map() {
        let model = new_source_map_model(
            "id".to_string(),
            "1.2.3".to_string(),
            "web".to_string(),
            "main-abc123.js".to_string(),
            "store://telemetry/sourcemaps/a/b-c.map".to_string(),
        );

        assert_eq!(model.map.try_as_ref(), Some(&None));
        assert_eq!(
            model.map_ref.try_as_ref().unwrap().as_deref(),
            Some("store://telemetry/sourcemaps/a/b-c.map")
        );
    }

    #[tokio::test]
    async fn storing_a_map_writes_one_object_and_returns_its_reference() {
        let store = memory_store();

        let reference = store_source_map(&store, "1.2.3", "web", "main.js", b"map bytes".to_vec())
            .await
            .unwrap();

        let path = reference.strip_prefix("store://").unwrap();
        assert_eq!(listed(&store).await, vec![path.to_string()]);
        assert_eq!(
            load_source_map(&store, &reference).await.unwrap(),
            b"map bytes"
        );
    }

    /// Re-uploading the same bytes must not strand a second object, so the
    /// path is derived from the bytes rather than from the upload.
    #[tokio::test]
    async fn identical_bytes_rewrite_the_same_object() {
        let store = memory_store();

        let first = store_source_map(&store, "1.2.3", "web", "main.js", b"same".to_vec())
            .await
            .unwrap();
        let second = store_source_map(&store, "1.2.3", "web", "main.js", b"same".to_vec())
            .await
            .unwrap();

        assert_eq!(first, second);
        assert_eq!(listed(&store).await.len(), 1);
    }

    #[tokio::test]
    async fn failed_identical_reupload_preserves_the_object_when_verification_fails() {
        // A closed lazy pool fails both UPDATE and SELECT without connecting
        // to a database, reproducing an outage during an identical re-upload.
        let mut options = sea_orm::ConnectOptions::new("postgres://localhost/sourcemap_test");
        options.connect_lazy(true).min_connections(0);
        let db = sea_orm::Database::connect(options).await.unwrap();
        db.close_by_ref().await.unwrap();

        let store = memory_store();
        let live_reference = store_source_map(&store, "1.2.3", "web", "main.js", b"same".to_vec())
            .await
            .unwrap();
        let reference = store_source_map(&store, "1.2.3", "web", "main.js", b"same".to_vec())
            .await
            .unwrap();
        assert_eq!(reference, live_reference);
        let existing = telemetry_source_map::Model {
            id: "existing-map".to_string(),
            release: "1.2.3".to_string(),
            source: "web".to_string(),
            file_name: "main.js".to_string(),
            map: None,
            map_ref: Some(live_reference.clone()),
            created_at: Utc::now().fixed_offset(),
        };

        let error =
            write_source_map_row(&db, Some(existing), "1.2.3", "web", "main.js", &reference)
                .await
                .unwrap_err();
        assert!(
            reference_is_committed(&db, "1.2.3", "web", "main.js", &reference)
                .await
                .is_err()
        );
        cleanup_failed_source_map_upload(
            &db, &store, &error, "1.2.3", "web", "main.js", &reference,
        )
        .await;

        assert_eq!(
            load_source_map(&store, &live_reference).await.unwrap(),
            b"same"
        );
        assert_eq!(listed(&store).await.len(), 1);
    }

    #[test]
    fn a_lost_compare_and_set_returns_a_conflict() {
        let mut result = UpdateResult::default();
        let error = require_source_map_update(result.clone()).unwrap_err();
        assert_eq!(error.status(), axum::http::StatusCode::CONFLICT);

        result.rows_affected = 1;
        assert!(require_source_map_update(result).is_ok());
    }

    #[tokio::test]
    async fn a_conflicting_upload_keeps_its_object_without_querying_the_database() {
        let store = memory_store();
        let reference = store_source_map(&store, "1.2.3", "web", "main.js", b"pending".to_vec())
            .await
            .unwrap();
        let error = require_source_map_update(UpdateResult::default()).unwrap_err();

        // A disconnected connection panics if SeaORM tries to build a query.
        // Conflict cleanup must not query and then delete a candidate that
        // another in-flight upload could commit immediately after that read.
        cleanup_failed_source_map_upload(
            &DatabaseConnection::default(),
            &store,
            &error,
            "1.2.3",
            "web",
            "main.js",
            &reference,
        )
        .await;

        assert_eq!(
            load_source_map(&store, &reference).await.unwrap(),
            b"pending"
        );
    }

    /// The natural key is part of the path, so two files of one release never
    /// collide even when their maps differ only in size.
    #[test]
    fn the_path_separates_releases_and_files() {
        let a = source_map_path("1.2.3", "web", "main.js", b"x");
        let b = source_map_path("1.2.4", "web", "main.js", b"x");
        let c = source_map_path("1.2.3", "web", "other.js", b"x");
        let d = source_map_path("1.2.3", "web", "main.js", b"y");

        assert_ne!(a, b);
        assert_ne!(a, c);
        assert_ne!(a, d);
        assert!(a.as_ref().starts_with(SOURCE_MAP_PREFIX));
    }

    /// What the upload does after it repoints a row at new bytes.
    #[tokio::test]
    async fn replacing_a_map_removes_only_the_superseded_object() {
        let store = memory_store();
        let old = store_source_map(&store, "1.2.3", "web", "main.js", b"old".to_vec())
            .await
            .unwrap();
        let new = store_source_map(&store, "1.2.3", "web", "main.js", b"new".to_vec())
            .await
            .unwrap();
        assert_eq!(listed(&store).await.len(), 2);

        assert!(delete_source_map(&store, &old).await);

        assert_eq!(
            listed(&store).await,
            vec![new.strip_prefix("store://").unwrap().to_string()]
        );
        assert!(load_source_map(&store, &old).await.is_err());
    }

    /// A cleanup pass that runs twice, or after a failed upload, must not fail
    /// on an object that is already gone.
    #[tokio::test]
    async fn deleting_a_missing_object_succeeds() {
        let store = memory_store();

        assert!(delete_source_map(&store, "store://telemetry/sourcemaps/a/b-c.map").await);
    }

    #[test]
    fn a_reference_resolves_to_a_path_in_either_form() {
        assert_eq!(
            source_map_reference_path("store://telemetry/sourcemaps/a/b-c.map").as_ref(),
            "telemetry/sourcemaps/a/b-c.map"
        );
        assert_eq!(
            source_map_reference_path("s3://bucket/telemetry/sourcemaps/a/b-c.map").as_ref(),
            "telemetry/sourcemaps/a/b-c.map"
        );
    }
}
