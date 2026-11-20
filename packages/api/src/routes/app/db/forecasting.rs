use std::time::Duration;

use axum::{
    Extension, Json,
    extract::{Path, Query, State},
};
use flow_like::bit::{Bit, BitTypes, ForecastingAssetRef};
use flow_like::state::FlowLikeState;
use flow_like_catalog_ml::inspection::forecasting::studio::{
    self, StudioForecastRequest, StudioForecastResult,
};
use flow_like_catalog_ml::inspection::forecasting_bit::{self, ForecastCheckpoint};
use flow_like_query::forecast::{ForecastModelSpec, ForecastOperation, ModelFile};
use flow_like_query::{Operation, QueryShape, SurfaceSpec, ViewSpec};
use flow_like_storage::databases::workbench::saved_query::SavedQuerySurface;
use flow_like_storage::{files::store::FlowLikeStore, object_store::ObjectStoreExt};
use sea_orm::{ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter};
use serde::Deserialize;

use super::{ScopeParams, resolve_connection};
use crate::{
    ensure_any_permission,
    entity::{bit, profile, user_bit},
    error::ApiError,
    middleware::jwt::AppUser,
    permission::role_permission::RolePermissions,
    query::{self, LanceRead},
    state::AppState,
};

/// Signed checkpoint URLs only have to outlive one forecast request.
const CHECKPOINT_URL_LIFETIME: Duration = Duration::from_secs(5 * 60);

#[derive(Debug, Deserialize)]
pub struct ForecastQueryPayload {
    pub profile_id: String,
    #[serde(flatten)]
    pub request: StudioForecastRequest,
}

fn activated_reference(
    references: &[String],
    model_id: &str,
    default_hub: &str,
) -> Result<ForecastingAssetRef, ApiError> {
    let matching = references
        .iter()
        .filter(|reference| {
            reference
                .rsplit_once(':')
                .map_or(reference.as_str(), |(_, id)| id)
                == model_id
        })
        .collect::<Vec<_>>();
    let first = matching.first().ok_or_else(|| {
        ApiError::forbidden("Add this forecasting model to the selected profile before running it")
    })?;
    let selected = ForecastingAssetRef {
        bit: (*first).clone(),
        relative_path: String::new(),
    };
    let (hub, id) = selected
        .reference_parts(default_hub)
        .map_err(|_| ApiError::bad_request("The activated model has an invalid Bit reference"))?;
    let identity = Bit {
        id: id.into(),
        hub: hub.into(),
        ..Default::default()
    };
    for reference in matching {
        let candidate = ForecastingAssetRef {
            bit: reference.clone(),
            relative_path: String::new(),
        };
        if !candidate
            .matches_reference(&identity, default_hub)
            .unwrap_or(false)
        {
            return Err(ApiError::bad_request(
                "This model ID is activated from more than one hub; remove the duplicate before forecasting",
            ));
        }
    }
    Ok(selected)
}

async fn profile_model_reference(
    db: &DatabaseConnection,
    sub: &str,
    profile_id: &str,
    model_id: &str,
) -> Result<(ForecastingAssetRef, String), ApiError> {
    if profile_id.trim().is_empty() || profile_id.trim() != profile_id || profile_id.len() > 256 {
        return Err(ApiError::bad_request(
            "A valid profile_id is required for forecasting",
        ));
    }
    let profile = profile::Entity::find()
        .filter(profile::Column::Id.eq(profile_id))
        .filter(profile::Column::UserId.eq(sub))
        .filter(profile::Column::DeletedAt.is_null())
        .one(db)
        .await?
        .ok_or_else(|| ApiError::not_found("Profile not found"))?;
    let reference =
        activated_reference(&profile.bit_ids.unwrap_or_default(), model_id, &profile.hub)?;
    Ok((reference, profile.hub))
}

async fn authorized_model(
    state: &AppState,
    sub: &str,
    payload: &ForecastQueryPayload,
) -> Result<Bit, ApiError> {
    let (reference, profile_hub) = profile_model_reference(
        &state.db,
        sub,
        &payload.profile_id,
        &payload.request.model_bit_id,
    )
    .await?;
    let private = user_bit::Entity::find_by_id(&payload.request.model_bit_id)
        .filter(user_bit::Column::UserId.eq(sub))
        .one(&state.db)
        .await?;
    let resolved = if let Some(private) = private {
        crate::routes::user::bits::user_bit_to_core(private, state, false)
    } else {
        let published = bit::Entity::find_by_id(&payload.request.model_bit_id)
            .one(&state.db)
            .await?
            .ok_or_else(|| {
                ApiError::not_found(
                    "Forecasting model is not available in your library or this catalog",
                )
            })?;
        let published: Bit = published.into();
        if state.platform_config.features.unauthorized_read {
            published
        } else {
            crate::routes::bit::get_bit::temporary_bit(published, &state.cdn_bucket).await?
        }
    };
    if resolved.bit_type != BitTypes::Forecasting {
        return Err(ApiError::bad_request("Select a Forecasting Bit"));
    }
    if !reference
        .matches_reference(&resolved, &profile_hub)
        .unwrap_or(false)
    {
        return Err(ApiError::forbidden(
            "The forecasting model does not match the hub activated in this profile",
        ));
    }
    Ok(resolved)
}

/// Same rule as the Bit downloader: legacy roots (`hash == id`) and source-derived identities
/// carry no content digest, so their bytes are trusted on first use.
fn verifiable(asset: &Bit) -> bool {
    asset.hash != asset.id && !asset.has_matching_user_source_artifact_identity()
}

/// A short-lived signed URL when the Bit store holds exactly this file, else the Bit's own
/// download link (already signed for published Bits on private CDNs).
async fn checkpoint_url(store: &FlowLikeStore, asset: &Bit, size: u64) -> Result<String, ApiError> {
    let file_name = asset.file_name.as_deref().ok_or_else(|| {
        ApiError::internal(format!("Forecast asset {} has no file name", asset.id))
    })?;
    let path = flow_like_storage::Path::from(asset.hash.as_str()).join(file_name);
    let installed = store
        .as_generic()
        .head(&path)
        .await
        .is_ok_and(|metadata| metadata.size == size);
    if installed {
        let url = store.sign("GET", &path, CHECKPOINT_URL_LIFETIME).await?;
        return Ok(url.to_string());
    }
    asset.download_link.clone().ok_or_else(|| {
        ApiError::internal(format!(
            "Forecast asset {} is neither in the Bit store nor downloadable",
            asset.id
        ))
    })
}

/// The authorized model as the query service fetches it: family, provenance and every
/// checkpoint file with its size, content hash and a URL the service may read.
async fn model_spec(state: &AppState, model: &Bit) -> Result<ForecastModelSpec, ApiError> {
    let master = state.master_state(state).await?;
    let ForecastCheckpoint {
        family,
        repository,
        revision,
        files,
    } = forecasting_bit::resolve_checkpoint(
        master.clone(),
        model,
        studio::MAXIMUM_CHECKPOINT_BYTES,
    )
    .await?;
    let store = FlowLikeState::bit_store(&master).await?;
    let mut checkpoint = Vec::with_capacity(files.len());
    for (name, asset) in files {
        let size = asset.size.ok_or_else(|| {
            ApiError::internal(format!("Forecast asset {} has no size", asset.id))
        })?;
        checkpoint.push(ModelFile {
            url: checkpoint_url(&store, &asset, size).await?,
            verify: verifiable(&asset),
            hash: asset.hash,
            name,
            size,
        });
    }
    Ok(ForecastModelSpec {
        bit_id: model.id.clone(),
        family: family_name(family)?,
        repository,
        revision,
        files: checkpoint,
    })
}

/// A forecast family under the name the engine (de)serializes it with.
fn family_name(family: impl serde::Serialize) -> Result<String, ApiError> {
    match serde_json::to_value(family)? {
        serde_json::Value::String(name) => Ok(name),
        other => Err(ApiError::internal(format!(
            "Forecast family serialized as {other}"
        ))),
    }
}

#[utoipa::path(
    post,
    path = "/apps/{app_id}/db/queries/forecast",
    tag = "data-studio",
    description = "Forecast the result of read-only SQL over native app tables and saved views. Requires an activated Forecasting Bit in the caller's profile. Returns history, future points, quantiles, covariates and optional holdout metrics without writing tables.",
    params(
        ("app_id" = String, Path, description = "Application ID"),
        ("scope" = Option<String>, Query, description = "Scope: 'user' or omit for project")
    ),
    request_body = serde_json::Value,
    responses(
        (status = 200, description = "Forecast points and metadata"),
        (status = 400, description = "Invalid mapping, SQL or forecast request"),
        (status = 403, description = "App database or model access denied"),
        (status = 413, description = "The model's checkpoint is larger than the query service can stage"),
        (status = 503, description = "The query service is unavailable, busy with another forecast, or built without forecasting"),
        (status = 504, description = "The forecast ran out of time")
    ),
    security(("bearer_auth" = []), ("api_key" = []), ("pat" = []))
)]
#[tracing::instrument(name = "POST /apps/{app_id}/db/queries/forecast", skip_all)]
pub async fn forecast_query(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(app_id): Path<String>,
    Query(scope): Query<ScopeParams>,
    Json(payload): Json<ForecastQueryPayload>,
) -> Result<Json<StudioForecastResult>, ApiError> {
    let permission = ensure_any_permission!(
        user,
        &app_id,
        &state,
        RolePermissions::ReadFiles,
        RolePermissions::ReadDatabase
    );
    if scope
        .scope
        .as_deref()
        .is_some_and(|scope| scope != "user" && scope != "project")
    {
        return Err(ApiError::bad_request("scope must be 'user' or 'project'"));
    }
    let model = authorized_model(&state, &user.sub()?, &payload).await?;
    let model = model_spec(&state, &model).await?;
    let connection = resolve_connection(&state, &user, &app_id, &scope).await?;
    let views =
        super::saved_queries::collect_views(&connection, SavedQuerySurface::Native, None).await?;
    let read = LanceRead::scoped(&user, &permission, &app_id, &scope)?;
    let source = state.query.lance_source(&state, &read, None).await?;
    let operation = Operation::Forecast {
        forecast: ForecastOperation::Run {
            request: serde_json::to_value(&payload.request)?,
            shape: QueryShape::Workbench {
                surface: SurfaceSpec::Native,
                views: views
                    .into_iter()
                    .map(|view| ViewSpec {
                        name: view.name,
                        sql: view.sql,
                    })
                    .collect(),
            },
            model,
        },
    };
    let response = state
        .query
        .run(
            &query::subject(&user),
            &query::request(vec![source], 1, operation),
        )
        .await?;
    let result = serde_json::from_value(query::json_answer(response)?).map_err(|error| {
        tracing::warn!(%error, "The query service sent a forecast the API cannot read");
        ApiError::bad_gateway("The query service sent an invalid forecast")
    })?;
    Ok(Json(result))
}

#[cfg(test)]
mod tests {
    use super::*;
    use flow_like_storage::object_store::memory::InMemory;
    use sea_orm::{
        ActiveValue::Set, ConnectOptions, ConnectionTrait, Database, Schema, sea_query::Table,
    };
    use std::sync::Arc;

    #[test]
    fn model_activation_checks_ids_and_hubs_without_accepting_client_bits() {
        let references = vec!["hub.example:model".into()];
        assert_eq!(
            activated_reference(&references, "other", "hub.example")
                .unwrap_err()
                .status(),
            axum::http::StatusCode::FORBIDDEN
        );
        assert!(
            activated_reference(
                &["hub.example:model".into(), "other.example:model".into()],
                "model",
                "hub.example"
            )
            .is_err()
        );
        let reference = activated_reference(
            &[
                "hub.example:model".into(),
                "https://hub.example:model".into(),
            ],
            "model",
            "hub.example",
        )
        .unwrap();
        assert!(
            reference
                .matches_reference(
                    &Bit {
                        id: "model".into(),
                        hub: "hub.example".into(),
                        ..Default::default()
                    },
                    "hub.example"
                )
                .unwrap()
        );
    }

    #[tokio::test]
    async fn profile_activation_is_owned_current_and_required() {
        let mut options = ConnectOptions::new("sqlite::memory:");
        options.max_connections(1);
        let db = Database::connect(options).await.unwrap();
        db.execute_unprepared("ATTACH DATABASE ':memory:' AS public")
            .await
            .unwrap();
        let schema = Schema::new(db.get_database_backend());
        let source = schema.create_table_from_entity(profile::Entity);
        let mut table = Table::create();
        table.table(source.get_table_name().unwrap().clone());
        for column in source.get_columns() {
            table.col(column.clone());
        }
        db.execute(&table).await.unwrap();
        let now = chrono::Utc::now().fixed_offset();
        for (id, owner, deleted) in [
            ("active", "alice", false),
            ("deleted", "alice", true),
            ("other-owner", "bob", false),
        ] {
            profile::Entity::insert(profile::ActiveModel {
                id: Set(id.into()),
                user_id: Set(owner.into()),
                name: Set(id.into()),
                hub: Set("hub.example".into()),
                bit_ids: Set(Some(vec!["hub.example:model".into()].into())),
                created_at: Set(now),
                updated_at: Set(now),
                deleted_at: Set(deleted.then_some(now)),
                ..Default::default()
            })
            .exec(&db)
            .await
            .unwrap();
        }
        profile_model_reference(&db, "alice", "active", "model")
            .await
            .unwrap();
        for id in ["deleted", "other-owner", "missing"] {
            let error = profile_model_reference(&db, "alice", id, "model")
                .await
                .unwrap_err();
            assert_eq!(error.status(), axum::http::StatusCode::NOT_FOUND);
        }
        let error = profile_model_reference(&db, "alice", "active", "unactivated")
            .await
            .unwrap_err();
        assert_eq!(error.status(), axum::http::StatusCode::FORBIDDEN);
    }

    fn asset(hash: &str) -> Bit {
        Bit {
            id: "weights".into(),
            hash: hash.into(),
            file_name: Some("model.safetensors".into()),
            size: Some(4),
            download_link: Some("https://hub.example/model.safetensors".into()),
            ..Default::default()
        }
    }

    #[test]
    fn only_content_hashes_are_verified() {
        assert!(verifiable(&asset(&"a".repeat(64))));
        assert!(!verifiable(&asset("weights")));
    }

    #[test]
    fn families_travel_under_their_engine_names() {
        for name in ["chronos2", "timesfm2_5", "tiny_time_mixer"] {
            let model: studio::StudioForecastModel = serde_json::from_value(serde_json::json!({
                "bit_id": "model",
                "family": name,
                "repository": null,
                "revision": null,
            }))
            .unwrap();
            assert_eq!(family_name(model.family).unwrap(), name);
        }
    }

    #[tokio::test]
    async fn installed_files_are_signed_and_others_use_their_download_link() {
        let store = FlowLikeStore::Memory(Arc::new(InMemory::new()));
        let missing = asset(&"b".repeat(64));
        assert_eq!(
            checkpoint_url(&store, &missing, 4).await.unwrap(),
            "https://hub.example/model.safetensors"
        );

        let installed = asset(&"c".repeat(64));
        store
            .as_generic()
            .put(
                &flow_like_storage::Path::from(installed.hash.as_str()).join("model.safetensors"),
                vec![1_u8; 4].into(),
            )
            .await
            .unwrap();
        let signed = checkpoint_url(&store, &installed, 4).await.unwrap();
        assert_ne!(signed, "https://hub.example/model.safetensors");
        assert_eq!(
            checkpoint_url(&store, &installed, 5).await.unwrap(),
            "https://hub.example/model.safetensors",
            "a file of another size is not the checkpoint file"
        );

        let unreachable = Bit {
            download_link: None,
            ..asset(&"d".repeat(64))
        };
        assert_eq!(
            checkpoint_url(&store, &unreachable, 4)
                .await
                .unwrap_err()
                .status(),
            axum::http::StatusCode::INTERNAL_SERVER_ERROR
        );
    }
}
