use crate::{entity::app_data_source, error::ApiError};
use flow_like_query::SecretFields;
use flow_like_types::Value;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum DataSourceDriver {
    Postgres,
    Mysql,
    Clickhouse,
    Bigquery,
    Athena,
}

impl DataSourceDriver {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Postgres => "postgres",
            Self::Mysql => "mysql",
            Self::Clickhouse => "clickhouse",
            Self::Bigquery => "bigquery",
            Self::Athena => "athena",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        [
            Self::Postgres,
            Self::Mysql,
            Self::Clickhouse,
            Self::Bigquery,
            Self::Athena,
        ]
        .into_iter()
        .find(|driver| driver.as_str() == value)
    }

    /// The credential fields a source of this driver takes.
    pub fn secret_fields(self) -> &'static [&'static str] {
        match self {
            Self::Postgres | Self::Mysql | Self::Clickhouse => &["username", "password"],
            Self::Bigquery => &["service_account_json"],
            Self::Athena => &["access_key_id", "secret_access_key", "session_token"],
        }
    }
}

/// An external database an app's queries read as `<alias>.<table>`.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct ExternalDataSource {
    pub id: String,
    pub app_id: String,
    pub name: String,
    /// Schema name in SQL: lowercase letters, digits and _, unique per app.
    pub alias: String,
    pub driver: DataSourceDriver,
    /// Non-secret connection settings (host, port, database, schema, ssl_mode, project_id, …).
    #[schema(value_type = Object)]
    pub config: Value,
    /// Names of the stored credential fields, never their values.
    pub secret_fields: Vec<String>,
    pub created_by: String,
    /// RFC 3339.
    pub created_at: String,
    /// RFC 3339.
    pub updated_at: String,
    /// RFC 3339.
    pub last_checked_at: Option<String>,
    /// `ok`, or the error of the last connection check.
    pub last_check_status: Option<String>,
}

impl TryFrom<app_data_source::Model> for ExternalDataSource {
    type Error = ApiError;

    fn try_from(model: app_data_source::Model) -> Result<Self, ApiError> {
        let driver = DataSourceDriver::parse(&model.driver).ok_or_else(|| {
            ApiError::internal(format!(
                "Data source '{}' has the unknown driver '{}'",
                model.id, model.driver
            ))
        })?;
        Ok(Self {
            id: model.id,
            app_id: model.app_id,
            name: model.name,
            alias: model.alias,
            driver,
            config: model.config,
            secret_fields: model.secret_fields.into_inner(),
            created_by: model.created_by,
            created_at: model.created_at.to_rfc3339(),
            updated_at: model.updated_at.to_rfc3339(),
            last_checked_at: model.last_checked_at.map(|at| at.to_rfc3339()),
            last_check_status: model.last_check_status,
        })
    }
}

#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct CreateDataSourceRequest {
    pub name: String,
    pub alias: String,
    pub driver: DataSourceDriver,
    /// The driver's connection settings; credentials go into `secret`.
    #[serde(default)]
    #[schema(value_type = Object)]
    pub config: serde_json::Map<String, Value>,
    /// Write-only credentials such as `username` and `password`.
    #[serde(default)]
    #[schema(value_type = Option<std::collections::HashMap<String, String>>)]
    pub secret: Option<SecretFields>,
}

#[derive(Debug, Default, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct UpdateDataSourceRequest {
    pub name: Option<String>,
    pub alias: Option<String>,
    pub driver: Option<DataSourceDriver>,
    /// Replaces every connection setting when present.
    #[schema(value_type = Option<Object>)]
    pub config: Option<serde_json::Map<String, Value>>,
    /// Omit to keep the stored credentials; `{}` clears them.
    #[schema(value_type = Option<std::collections::HashMap<String, String>>)]
    pub secret: Option<SecretFields>,
}

/// The outcome of connecting with the stored credentials.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct DataSourceCheck {
    pub ok: bool,
    /// Tables queries can address as `<alias>.<table>`.
    pub tables: Vec<String>,
    pub error: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drivers_round_trip_through_their_wire_names() {
        for driver in [
            DataSourceDriver::Postgres,
            DataSourceDriver::Mysql,
            DataSourceDriver::Clickhouse,
            DataSourceDriver::Bigquery,
            DataSourceDriver::Athena,
        ] {
            assert_eq!(DataSourceDriver::parse(driver.as_str()), Some(driver));
            assert_eq!(
                serde_json::to_value(driver).unwrap(),
                Value::String(driver.as_str().to_string())
            );
        }
        assert_eq!(DataSourceDriver::parse("oracle"), None);
    }

    #[test]
    fn requests_reject_unknown_fields_and_keep_secret_values_out_of_debug() {
        let request: CreateDataSourceRequest = serde_json::from_value(serde_json::json!({
            "name": "Warehouse",
            "alias": "warehouse",
            "driver": "postgres",
            "config": { "host": "db.example.com", "port": 5432, "database": "sales" },
            "secret": { "username": "reader", "password": "hunter2" }
        }))
        .unwrap();
        let debug = format!("{request:?}");
        assert!(debug.contains("password") && !debug.contains("hunter2"));

        let update: UpdateDataSourceRequest =
            serde_json::from_value(serde_json::json!({ "secret": {} })).unwrap();
        assert!(update.secret.is_some_and(|secret| secret.0.is_empty()));
        assert!(
            serde_json::from_value::<UpdateDataSourceRequest>(serde_json::json!({ "secrets": {} }))
                .is_err()
        );
    }
}
