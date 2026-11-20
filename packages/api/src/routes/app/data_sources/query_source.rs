//! Stored sources as the query service receives them, and the record of their last check.

use super::{model::DataSourceDriver, validation};
use crate::{
    data_source_secrets::{Sealers, SecretScope},
    entity::app_data_source,
    error::ApiError,
    state::AppState,
};
use flow_like_query::{ExternalSource, SecretMaterial};
use sea_orm::{ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter, sea_query::Expr};

const MAX_CHECK_STATUS_CHARS: usize = 1000;

pub(super) fn sealers(state: &AppState) -> Sealers {
    Sealers::new(&state.encryption_key)
}

pub(super) async fn load_source(
    db: &impl ConnectionTrait,
    app_id: &str,
    source_id: &str,
) -> Result<app_data_source::Model, ApiError> {
    app_data_source::Entity::find()
        .filter(app_data_source::Column::AppId.eq(app_id))
        .filter(app_data_source::Column::Id.eq(source_id))
        .one(db)
        .await?
        .ok_or_else(|| ApiError::not_found("Data source not found"))
}

pub(super) fn driver_of(model: &app_data_source::Model) -> Result<DataSourceDriver, ApiError> {
    DataSourceDriver::parse(&model.driver).ok_or_else(|| {
        ApiError::internal(format!(
            "Data source '{}' has the unknown driver '{}'",
            model.id, model.driver
        ))
    })
}

/// The source as the query service connects to it, with its credentials opened. The caller has
/// already authorized reading `app_id`'s data. The result holds plaintext credentials (wiped on
/// drop): it goes into a request body and nowhere else.
pub async fn external_source_for_query(
    state: &AppState,
    app_id: &str,
    source_id: &str,
) -> Result<ExternalSource, ApiError> {
    opened_source(&state.db, &sealers(state), app_id, source_id).await
}

pub(super) async fn opened_source(
    db: &impl ConnectionTrait,
    sealers: &Sealers,
    app_id: &str,
    source_id: &str,
) -> Result<ExternalSource, ApiError> {
    let model = load_source(db, app_id, source_id).await?;
    let driver = driver_of(&model)?;
    let config = model.config.as_object().cloned().unwrap_or_default();
    let connection = validation::connection(driver, &config).map_err(|error| {
        ApiError::internal(format!(
            "The stored settings of data source '{source_id}' are invalid: {error}"
        ))
    })?;
    let secret = match &model.secret {
        Some(stored) => SecretMaterial::Plain {
            fields: sealers
                .open_fields(SecretScope { app_id, source_id }, stored)
                .await
                .map_err(ApiError::internal_error)?,
        },
        None => SecretMaterial::None,
    };
    Ok(ExternalSource {
        source_id: model.id,
        app_id: model.app_id,
        connection,
        secret,
    })
}

/// Records a connection check: `ok`, or the error it ended with.
pub async fn record_check(
    state: &AppState,
    app_id: &str,
    source_id: &str,
    status: &str,
) -> Result<(), ApiError> {
    store_check(&state.db, app_id, source_id, status).await
}

pub(super) async fn store_check(
    db: &impl ConnectionTrait,
    app_id: &str,
    source_id: &str,
    status: &str,
) -> Result<(), ApiError> {
    let status: String = status.chars().take(MAX_CHECK_STATUS_CHARS).collect();
    let updated = app_data_source::Entity::update_many()
        .col_expr(
            app_data_source::Column::LastCheckedAt,
            Expr::value(chrono::Utc::now().fixed_offset()),
        )
        .col_expr(
            app_data_source::Column::LastCheckStatus,
            Expr::value(status),
        )
        .filter(app_data_source::Column::AppId.eq(app_id))
        .filter(app_data_source::Column::Id.eq(source_id))
        .exec(db)
        .await?;
    if updated.rows_affected == 0 {
        return Err(ApiError::not_found("Data source not found"));
    }
    Ok(())
}
