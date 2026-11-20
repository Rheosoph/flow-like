//! Checks a data source definition before it is stored. Errors name the setting that failed,
//! never a credential value.

use super::model::DataSourceDriver;
use flow_like_query::{ExternalConnection, SecretFields};
use flow_like_types::Value;
use serde_json::Map;

pub const MAX_NAME_CHARS: usize = 120;
pub const MAX_CONFIG_BYTES: usize = 8 * 1024;
pub const MAX_SECRET_VALUE_BYTES: usize = 16 * 1024;
pub const MAX_SECRET_BYTES: usize = 32 * 1024;
const MAX_ALIAS_CHARS: usize = 63;
const MAX_HOST_CHARS: usize = 253;
const MAX_TEXT_CHARS: usize = 255;
const RESERVED_ALIASES: [&str; 2] = ["public", "information_schema"];
const POSTGRES_SSL_MODES: [&str; 5] = ["disable", "prefer", "require", "verify-ca", "verify-full"];
const MYSQL_SSL_MODES: [&str; 3] = ["disabled", "preferred", "required"];

pub fn name(name: &str) -> Result<String, String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("Give the data source a name".to_string());
    }
    if name.chars().count() > MAX_NAME_CHARS {
        return Err(format!(
            "The name is longer than {MAX_NAME_CHARS} characters"
        ));
    }
    if name.chars().any(char::is_control) {
        return Err("The name must not contain control characters".to_string());
    }
    Ok(name.to_string())
}

/// The schema name queries address the source by: the same rule as a Home widget's app aliases.
pub fn is_valid_alias(alias: &str) -> bool {
    alias.len() <= MAX_ALIAS_CHARS
        && alias
            .chars()
            .next()
            .is_some_and(|first| first.is_ascii_lowercase() || first == '_')
        && alias
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
        && !RESERVED_ALIASES.contains(&alias)
}

pub fn alias(alias: &str) -> Result<(), String> {
    if is_valid_alias(alias) {
        Ok(())
    } else {
        Err(format!(
            "'{alias}' is not a valid schema name: use at most {MAX_ALIAS_CHARS} lowercase letters, \
             digits and _, starting with a letter or _, and not 'public' or 'information_schema'"
        ))
    }
}

/// The connection `config` describes for `driver`. Unknown settings are refused so a typo does
/// not silently fall back to a default.
pub fn connection(
    driver: DataSourceDriver,
    config: &Map<String, Value>,
) -> Result<ExternalConnection, String> {
    let size = serde_json::to_vec(config).map_or(usize::MAX, |bytes| bytes.len());
    if size > MAX_CONFIG_BYTES {
        return Err(format!(
            "The connection settings are larger than {MAX_CONFIG_BYTES} bytes"
        ));
    }
    if config.contains_key("driver") {
        return Err("Give the driver next to the settings, not inside them".to_string());
    }
    let mut tagged = config.clone();
    tagged.insert(
        "driver".to_string(),
        Value::String(driver.as_str().to_string()),
    );
    let connection: ExternalConnection = serde_json::from_value(Value::Object(tagged))
        .map_err(|error| format!("Invalid {} settings: {error}", driver.as_str()))?;
    let known = settings(&connection);
    let unknown: Vec<String> = config
        .keys()
        .filter(|key| !known.contains_key(*key))
        .map(|key| format!("'{key}'"))
        .collect();
    if !unknown.is_empty() {
        return Err(format!(
            "{} sources have no setting {}",
            driver.as_str(),
            unknown.join(", ")
        ));
    }
    check_connection(&connection)?;
    Ok(connection)
}

/// Every setting of `connection`, unset options as `null`.
fn settings(connection: &ExternalConnection) -> Map<String, Value> {
    let mut settings = match serde_json::to_value(connection) {
        Ok(Value::Object(settings)) => settings,
        _ => Map::new(),
    };
    settings.remove("driver");
    settings
}

/// The settings as stored and returned: defaults filled in, unset options left out.
pub fn stored_config(connection: &ExternalConnection) -> Value {
    let mut config = settings(connection);
    config.retain(|_, value| !value.is_null());
    Value::Object(config)
}

fn check_connection(connection: &ExternalConnection) -> Result<(), String> {
    match connection {
        ExternalConnection::Postgres {
            host,
            port,
            database,
            schema,
            ssl_mode,
        } => {
            check_host(host)?;
            check_port(*port)?;
            check_text("database", database)?;
            check_text("schema", schema)?;
            check_choice("ssl_mode", ssl_mode, &POSTGRES_SSL_MODES)
        }
        ExternalConnection::Mysql {
            host,
            port,
            database,
            ssl_mode,
        } => {
            check_host(host)?;
            check_port(*port)?;
            check_text("database", database)?;
            check_choice("ssl_mode", ssl_mode, &MYSQL_SSL_MODES)
        }
        ExternalConnection::Clickhouse {
            host,
            port,
            database,
        } => {
            check_host(host)?;
            check_port(*port)?;
            check_text("database", database)
        }
        ExternalConnection::Bigquery {
            project_id,
            dataset,
            location,
        } => {
            check_pattern(
                "project_id",
                project_id,
                128,
                |c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '-' | '.' | ':'),
                "lowercase letters, digits, '-', '.' and ':'",
            )?;
            check_pattern(
                "dataset",
                dataset,
                1024,
                |c| c.is_ascii_alphanumeric() || c == '_',
                "letters, digits and _",
            )?;
            location.as_deref().map_or(Ok(()), |location| {
                check_pattern(
                    "location",
                    location,
                    64,
                    |c| c.is_ascii_alphanumeric() || c == '-',
                    "letters, digits and '-'",
                )
            })
        }
        ExternalConnection::Athena {
            region,
            database,
            catalog,
            workgroup,
            output_location,
        } => {
            check_region(region)?;
            check_text("database", database)?;
            if let Some(catalog) = catalog {
                check_text("catalog", catalog)?;
            }
            if let Some(workgroup) = workgroup {
                check_pattern(
                    "workgroup",
                    workgroup,
                    128,
                    |c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'),
                    "letters, digits, '.', '_' and '-'",
                )?;
            }
            match output_location {
                Some(location) if !location.starts_with("s3://") || location.len() > 1024 => Err(
                    "output_location must be an s3:// location of at most 1024 characters"
                        .to_string(),
                ),
                _ => Ok(()),
            }
        }
    }
}

/// The rule the query service's connectors apply (`ensure_host_syntax`): drivers splice the
/// host into connection strings and URLs, where other characters would add parameters.
fn check_host(host: &str) -> Result<(), String> {
    let plain = !host.is_empty()
        && host.len() <= MAX_HOST_CHARS
        && host
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '.' | '_' | ':'));
    if plain {
        Ok(())
    } else {
        Err(format!(
            "host '{host}' must be a host name or IP address (letters, digits, '-', '.', '_' or ':')"
        ))
    }
}

fn check_port(port: u16) -> Result<(), String> {
    if port == 0 {
        Err("port must be between 1 and 65535".to_string())
    } else {
        Ok(())
    }
}

fn check_text(label: &str, value: &str) -> Result<(), String> {
    if value.trim().is_empty() {
        return Err(format!("{label} must not be empty"));
    }
    if value.trim() != value {
        return Err(format!("{label} must not start or end with spaces"));
    }
    if value.chars().count() > MAX_TEXT_CHARS {
        return Err(format!(
            "{label} is longer than {MAX_TEXT_CHARS} characters"
        ));
    }
    if value.chars().any(char::is_control) {
        return Err(format!("{label} must not contain control characters"));
    }
    Ok(())
}

fn check_choice(label: &str, value: &str, choices: &[&str]) -> Result<(), String> {
    if choices.contains(&value) {
        Ok(())
    } else {
        Err(format!(
            "{label} must be one of {}, not '{value}'",
            choices.join(", ")
        ))
    }
}

fn check_pattern(
    label: &str,
    value: &str,
    max_chars: usize,
    allowed: impl Fn(char) -> bool,
    allowed_text: &str,
) -> Result<(), String> {
    if value.is_empty() || value.chars().count() > max_chars || !value.chars().all(allowed) {
        return Err(format!(
            "{label} '{value}' must be 1 to {max_chars} characters of {allowed_text}"
        ));
    }
    Ok(())
}

fn check_region(region: &str) -> Result<(), String> {
    let mut parts = region.split('-');
    let valid = region.len() <= 32
        && parts
            .next()
            .is_some_and(|area| area.len() == 2 && area.chars().all(|c| c.is_ascii_lowercase()))
        && {
            let rest: Vec<&str> = parts.collect();
            rest.len() >= 2
                && rest[..rest.len() - 1]
                    .iter()
                    .all(|part| !part.is_empty() && part.chars().all(|c| c.is_ascii_lowercase()))
                && rest.last().is_some_and(|number| {
                    !number.is_empty() && number.chars().all(|c| c.is_ascii_digit())
                })
        };
    if valid {
        Ok(())
    } else {
        Err(format!(
            "region '{region}' is not an AWS region such as eu-west-1"
        ))
    }
}

/// Credential fields must be ones the driver takes, within the size caps. Errors name fields,
/// never values.
pub fn secret(driver: DataSourceDriver, secret: &SecretFields) -> Result<(), String> {
    let allowed = driver.secret_fields();
    let mut total = 0usize;
    for (field, value) in &secret.0 {
        if !allowed.contains(&field.as_str()) {
            return Err(format!(
                "'{field}' is not a credential of {} sources; use {}",
                driver.as_str(),
                allowed.join(", ")
            ));
        }
        if value.len() > MAX_SECRET_VALUE_BYTES {
            return Err(format!(
                "The credential '{field}' is larger than {MAX_SECRET_VALUE_BYTES} bytes"
            ));
        }
        total += field.len() + value.len();
    }
    if total > MAX_SECRET_BYTES {
        return Err(format!(
            "The credentials are larger than {MAX_SECRET_BYTES} bytes together"
        ));
    }
    if let Some(key) = secret.get("service_account_json") {
        let is_object = serde_json::from_str::<Value>(key).is_ok_and(|key| key.is_object());
        if !is_object {
            return Err(
                "service_account_json must be the service account's JSON key file".to_string(),
            );
        }
    }
    Ok(())
}

/// Credentials stored for one driver that another driver would not read.
pub fn foreign_secret_fields<'a>(driver: DataSourceDriver, stored: &'a [String]) -> Vec<&'a str> {
    stored
        .iter()
        .map(String::as_str)
        .filter(|field| !driver.secret_fields().contains(field))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::collections::BTreeMap;

    fn config(value: Value) -> Map<String, Value> {
        match value {
            Value::Object(map) => map,
            _ => unreachable!(),
        }
    }

    fn secret(fields: &[(&str, &str)]) -> SecretFields {
        SecretFields(
            fields
                .iter()
                .map(|(name, value)| (name.to_string(), value.to_string()))
                .collect::<BTreeMap<_, _>>(),
        )
    }

    #[test]
    fn aliases_follow_the_widget_alias_rule() {
        for valid in ["crm", "_staging", "warehouse_2", &"a".repeat(63)] {
            assert!(alias(valid).is_ok(), "{valid}");
        }
        for invalid in [
            "",
            "CRM",
            "1crm",
            "crm-eu",
            "crm.eu",
            "public",
            "information_schema",
            &"a".repeat(64),
        ] {
            assert!(alias(invalid).is_err(), "{invalid}");
        }
    }

    #[test]
    fn names_are_trimmed_and_bounded() {
        assert_eq!(name("  Warehouse ").unwrap(), "Warehouse");
        assert!(name("   ").is_err());
        assert!(name(&"x".repeat(MAX_NAME_CHARS + 1)).is_err());
        assert!(name("a\nb").is_err());
    }

    #[test]
    fn postgres_settings_fill_defaults_and_store_without_the_driver() {
        let connection = connection(
            DataSourceDriver::Postgres,
            &config(json!({ "host": "db.example.com", "port": 5432, "database": "sales" })),
        )
        .unwrap();
        assert_eq!(
            connection,
            ExternalConnection::Postgres {
                host: "db.example.com".into(),
                port: 5432,
                database: "sales".into(),
                schema: "public".into(),
                ssl_mode: "require".into(),
            }
        );
        assert_eq!(
            stored_config(&connection),
            json!({
                "host": "db.example.com",
                "port": 5432,
                "database": "sales",
                "schema": "public",
                "ssl_mode": "require"
            })
        );
    }

    #[test]
    fn unset_options_are_left_out_of_the_stored_settings() {
        let connection = connection(
            DataSourceDriver::Athena,
            &config(json!({ "region": "eu-west-1", "database": "lake" })),
        )
        .unwrap();
        assert_eq!(
            stored_config(&connection),
            json!({ "region": "eu-west-1", "database": "lake" })
        );
    }

    #[test]
    fn settings_are_refused_with_the_reason() {
        let cases = [
            (
                DataSourceDriver::Postgres,
                json!({ "host": "db", "port": 5432, "database": "x", "pasword": "p" }),
                "no setting 'pasword'",
            ),
            (
                DataSourceDriver::Postgres,
                json!({ "host": "db", "port": 0, "database": "x" }),
                "port",
            ),
            (
                DataSourceDriver::Postgres,
                json!({ "host": "db", "port": 70000, "database": "x" }),
                "Invalid postgres settings",
            ),
            (
                DataSourceDriver::Postgres,
                json!({ "host": "db host", "port": 5432, "database": "x" }),
                "host",
            ),
            (
                DataSourceDriver::Postgres,
                json!({ "host": "user@db", "port": 5432, "database": "x" }),
                "host",
            ),
            (
                DataSourceDriver::Postgres,
                json!({ "host": "db", "port": 5432, "database": "x", "ssl_mode": "allow-all" }),
                "ssl_mode",
            ),
            (
                DataSourceDriver::Postgres,
                json!({ "port": 5432, "database": "x" }),
                "host",
            ),
            (
                DataSourceDriver::Mysql,
                json!({ "host": "db", "port": 3306, "database": " x" }),
                "spaces",
            ),
            (
                DataSourceDriver::Mysql,
                json!({ "host": "db", "port": 3306, "database": "x", "ssl_mode": "require" }),
                "ssl_mode",
            ),
            (
                DataSourceDriver::Clickhouse,
                json!({ "host": "db", "port": 8123, "database": "", "driver": "x" }),
                "driver",
            ),
            (
                DataSourceDriver::Clickhouse,
                json!({ "host": "db", "port": 8123, "database": "" }),
                "database",
            ),
            (
                DataSourceDriver::Bigquery,
                json!({ "project_id": "My Project", "dataset": "sales" }),
                "project_id",
            ),
            (
                DataSourceDriver::Bigquery,
                json!({ "project_id": "acme-prod", "dataset": "sales-eu" }),
                "dataset",
            ),
            (
                DataSourceDriver::Athena,
                json!({ "region": "europe", "database": "lake" }),
                "region",
            ),
            (
                DataSourceDriver::Athena,
                json!({ "region": "eu-west-1", "database": "lake", "output_location": "https://x" }),
                "output_location",
            ),
            (
                DataSourceDriver::Athena,
                json!({ "region": "eu-west-1", "database": "lake", "padding": "x".repeat(MAX_CONFIG_BYTES) }),
                "larger than",
            ),
        ];
        for (driver, value, expected) in cases {
            let error = connection(driver, &config(value.clone())).unwrap_err();
            assert!(error.contains(expected), "{value}: {error}");
        }
    }

    #[test]
    fn every_driver_accepts_a_typical_definition() {
        let cases = [
            (
                DataSourceDriver::Postgres,
                json!({ "host": "10.0.0.5", "port": 26257, "database": "defaultdb", "schema": "analytics", "ssl_mode": "verify-full" }),
            ),
            (
                DataSourceDriver::Mysql,
                json!({ "host": "mysql.internal", "port": 3306, "database": "shop", "ssl_mode": "preferred" }),
            ),
            (
                DataSourceDriver::Clickhouse,
                json!({ "host": "ch.example.com", "port": 8123, "database": "events" }),
            ),
            (
                DataSourceDriver::Bigquery,
                json!({ "project_id": "example.com:acme-prod", "dataset": "sales_eu", "location": "EU" }),
            ),
            (
                DataSourceDriver::Athena,
                json!({ "region": "us-gov-west-1", "database": "lake", "catalog": "AwsDataCatalog", "workgroup": "primary", "output_location": "s3://bucket/results/" }),
            ),
        ];
        for (driver, value) in cases {
            let connection = connection(driver, &config(value.clone()))
                .unwrap_or_else(|error| panic!("{value}: {error}"));
            assert_eq!(connection.driver(), driver.as_str());
            assert_eq!(stored_config(&connection), value);
        }
    }

    #[test]
    fn credentials_must_belong_to_the_driver_and_errors_hide_values() {
        assert!(
            super::secret(
                DataSourceDriver::Postgres,
                &secret(&[("username", "reader"), ("password", "")])
            )
            .is_ok()
        );
        assert!(super::secret(DataSourceDriver::Postgres, &secret(&[])).is_ok());
        let error = super::secret(
            DataSourceDriver::Postgres,
            &secret(&[("token", "tok-value-123")]),
        )
        .unwrap_err();
        assert!(error.contains("'token'") && !error.contains("tok-value-123"));
        let big = "x".repeat(MAX_SECRET_VALUE_BYTES + 1);
        let error =
            super::secret(DataSourceDriver::Postgres, &secret(&[("password", &big)])).unwrap_err();
        assert!(error.contains("password") && !error.contains("xxxx"));
        let half = "x".repeat(MAX_SECRET_VALUE_BYTES);
        assert!(
            super::secret(
                DataSourceDriver::Athena,
                &secret(&[
                    ("access_key_id", &half),
                    ("secret_access_key", &half),
                    ("session_token", "x")
                ])
            )
            .is_err()
        );
        let error = super::secret(
            DataSourceDriver::Bigquery,
            &secret(&[("service_account_json", "not-json-secret")]),
        )
        .unwrap_err();
        assert!(!error.contains("not-json-secret"));
        assert!(
            super::secret(
                DataSourceDriver::Bigquery,
                &secret(&[("service_account_json", r#"{"type":"service_account"}"#)])
            )
            .is_ok()
        );
    }

    #[test]
    fn a_driver_change_finds_credentials_the_new_driver_would_not_read() {
        let stored = vec!["username".to_string(), "password".to_string()];
        assert!(foreign_secret_fields(DataSourceDriver::Mysql, &stored).is_empty());
        assert_eq!(
            foreign_secret_fields(DataSourceDriver::Bigquery, &stored),
            vec!["username", "password"]
        );
    }
}
