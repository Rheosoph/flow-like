//! External databases as session schemas. Every connection goes through the catalog's connector
//! egress checks: on hosted deployments only public addresses are reachable. One pool per source
//! serves every table a request reads from it.

use crate::protocol::{ErrorCode, ExternalConnection, ExternalSource, SecretMaterial, WireError};
use async_trait::async_trait;
use flow_like_catalog_data::data::datafusion::connectors::{
    AthenaSource, BigquerySource, ClickhouseSource, ConnectPolicy, ConnectorSource, MysqlSource,
    PostgresSource,
};
use flow_like_catalog_data::data::datafusion::external_database::{
    ExternalDatabase, connect_database,
};
use flow_like_runtime::flow::execution::ExecutionEnvironment;
use flow_like_storage::databases::workbench::ExternalTableResolver;
use flow_like_storage::datafusion::catalog::TableProvider;
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::OnceCell;

/// Turns a source definition into table providers. Swappable so tests and future drivers do
/// not need a live database.
#[async_trait]
pub trait ExternalConnector: Send + Sync {
    /// `deadline` ends warehouse statements (BigQuery, Athena) that are still running, so they
    /// fail as timed out and stop billing before the request itself is cut.
    async fn resolver(
        &self,
        source: &ExternalSource,
        deadline: Option<Instant>,
    ) -> Result<Arc<dyn ExternalTableResolver>, WireError>;

    /// The tables a query can address under the source's schema; connecting proves the
    /// credentials work.
    async fn tables(&self, source: &ExternalSource) -> Result<Vec<String>, WireError> {
        Err(listing_unavailable(source))
    }
}

fn listing_unavailable(source: &ExternalSource) -> WireError {
    WireError::new(
        ErrorCode::Unavailable,
        format!(
            "Listing the tables of {} sources is not available yet",
            source.connection.driver()
        ),
    )
}

/// The built-in drivers.
pub struct DriverConnector {
    policy: ConnectPolicy,
    /// ClickHouse is reached over plain HTTP, so only hubs that admit private networks reach it.
    plaintext_allowed: bool,
}

impl DriverConnector {
    /// `allow_private_networks` admits private, shared and reserved addresses, for self-hosted
    /// deployments whose databases live on the same network.
    pub fn new(allow_private_networks: bool) -> Self {
        Self {
            policy: if allow_private_networks {
                ConnectPolicy::new(ExecutionEnvironment::Server)
            } else {
                ConnectPolicy::public_only(ExecutionEnvironment::Server)
            },
            plaintext_allowed: allow_private_networks,
        }
    }
}

/// The ClickHouse connector speaks plain HTTP: its credentials and rows would cross a public
/// network unencrypted, so hubs limited to public addresses refuse it.
fn refuse_plaintext(source: &ExternalSource, plaintext_allowed: bool) -> Result<(), WireError> {
    if plaintext_allowed || !matches!(source.connection, ExternalConnection::Clickhouse { .. }) {
        return Ok(());
    }
    Err(WireError::new(
        ErrorCode::InvalidRequest,
        format!(
            "External source '{}' is a ClickHouse database, which this query service reaches over \
             plain HTTP. That would send its credentials unencrypted, so only hubs that allow \
             private networks (QUERY_ALLOW_PRIVATE_NETWORKS) connect to ClickHouse",
            source.source_id
        ),
    ))
}

#[derive(Default)]
struct Credentials {
    username: String,
    password: String,
    service_account_json: String,
    access_key_id: String,
    secret_access_key: String,
    session_token: String,
}

fn credentials(source: &ExternalSource) -> Result<Credentials, WireError> {
    match &source.secret {
        SecretMaterial::Plain { fields } => {
            let field = |name: &str| fields.get(name).unwrap_or_default().to_string();
            Ok(Credentials {
                username: field("username"),
                password: field("password"),
                service_account_json: field("service_account_json"),
                access_key_id: field("access_key_id"),
                secret_access_key: field("secret_access_key"),
                session_token: field("session_token"),
            })
        }
        SecretMaterial::None => Ok(Credentials::default()),
        SecretMaterial::Sealed { sealed } => Err(WireError::new(
            ErrorCode::Unavailable,
            format!(
                "External source '{}' is sealed by '{}', which this query service cannot open",
                source.source_id, sealed.provider
            ),
        )),
    }
}

/// The tables of one source. The pool opens with the first table and serves the rest; a
/// failed open is kept, so later tables fail at once instead of reconnecting.
struct DriverTables {
    source_id: String,
    connection: ExternalConnection,
    credentials: Credentials,
    policy: ConnectPolicy,
    deadline: Option<Instant>,
    database: OnceCell<Result<ExternalDatabase, String>>,
}

impl DriverTables {
    fn new(
        source: &ExternalSource,
        policy: ConnectPolicy,
        deadline: Option<Instant>,
    ) -> Result<Self, WireError> {
        Ok(Self {
            source_id: source.source_id.clone(),
            connection: source.connection.clone(),
            credentials: credentials(source)?,
            policy,
            deadline,
            database: OnceCell::new(),
        })
    }

    /// The source as the catalog connectors take it, without a table.
    fn connector_source(&self) -> ConnectorSource {
        let username = || self.credentials.username.clone();
        let password = || self.credentials.password.clone();
        match &self.connection {
            ExternalConnection::Postgres {
                host,
                port,
                database,
                schema,
                ssl_mode,
            } => ConnectorSource::Postgres(PostgresSource {
                host: host.clone(),
                port: *port,
                database: database.clone(),
                username: username(),
                password: password(),
                schema: schema.clone(),
                table: String::new(),
                ssl_mode: ssl_mode.clone(),
                readonly: true,
            }),
            ExternalConnection::Mysql {
                host,
                port,
                database,
                ssl_mode,
            } => ConnectorSource::Mysql(MysqlSource {
                host: host.clone(),
                port: *port,
                database: database.clone(),
                username: username(),
                password: password(),
                table: String::new(),
                ssl_mode: ssl_mode.clone(),
                readonly: true,
            }),
            ExternalConnection::Clickhouse {
                host,
                port,
                database,
            } => ConnectorSource::Clickhouse(ClickhouseSource {
                host: host.clone(),
                port: *port,
                database: database.clone(),
                username: username(),
                password: password(),
                table: String::new(),
            }),
            ExternalConnection::Bigquery {
                project_id,
                dataset,
                location,
            } => ConnectorSource::Bigquery(BigquerySource {
                project_id: project_id.clone(),
                dataset: dataset.clone(),
                location: location.clone(),
                service_account_json: self.credentials.service_account_json.clone(),
                table: String::new(),
                deadline: self.deadline,
            }),
            ExternalConnection::Athena {
                region,
                database,
                catalog,
                workgroup,
                output_location,
            } => ConnectorSource::Athena(AthenaSource {
                region: region.clone(),
                database: database.clone(),
                catalog: catalog.clone(),
                workgroup: workgroup.clone(),
                output_location: output_location.clone(),
                access_key_id: self.credentials.access_key_id.clone(),
                secret_access_key: self.credentials.secret_access_key.clone(),
                session_token: Some(self.credentials.session_token.clone())
                    .filter(|token| !token.is_empty()),
                table: String::new(),
                deadline: self.deadline,
            }),
        }
    }

    async fn database(&self) -> flow_like_types::Result<&ExternalDatabase> {
        let opened = self
            .database
            .get_or_init(|| async {
                let mut source = self.connector_source();
                let database = connect_database(&source, self.policy)
                    .await
                    .map_err(|error| format!("{error:#}"));
                wipe_password(&mut source);
                database
            })
            .await;
        opened.as_ref().map_err(|error| {
            flow_like_types::anyhow!("External source '{}': {error}", self.source_id)
        })
    }
}

fn wipe_password(source: &mut ConnectorSource) {
    use zeroize::Zeroize;
    match source {
        ConnectorSource::Postgres(source) => source.password.zeroize(),
        ConnectorSource::Mysql(source) => source.password.zeroize(),
        ConnectorSource::Clickhouse(source) => source.password.zeroize(),
        ConnectorSource::Bigquery(source) => source.service_account_json.zeroize(),
        ConnectorSource::Athena(source) => {
            source.secret_access_key.zeroize();
            if let Some(token) = source.session_token.as_mut() {
                token.zeroize();
            }
        }
        _ => {}
    }
}

#[async_trait]
impl ExternalTableResolver for DriverTables {
    async fn table(&self, table: &str) -> flow_like_types::Result<Arc<dyn TableProvider>> {
        self.database().await?.table(table).await
    }
}

#[async_trait]
impl ExternalConnector for DriverConnector {
    async fn resolver(
        &self,
        source: &ExternalSource,
        deadline: Option<Instant>,
    ) -> Result<Arc<dyn ExternalTableResolver>, WireError> {
        refuse_plaintext(source, self.plaintext_allowed)?;
        Ok(Arc::new(DriverTables::new(source, self.policy, deadline)?))
    }

    async fn tables(&self, source: &ExternalSource) -> Result<Vec<String>, WireError> {
        refuse_plaintext(source, self.plaintext_allowed)?;
        let tables = DriverTables::new(source, self.policy, None)?;
        let failed = |error: flow_like_types::Error| {
            WireError::new(ErrorCode::QueryFailed, format!("{error:#}"))
        };
        let database = tables.database().await.map_err(failed)?;
        database.tables().await.map_err(|error| {
            failed(error.context(format!("External source '{}'", source.source_id)))
        })
    }
}

impl Drop for Credentials {
    fn drop(&mut self) {
        use zeroize::Zeroize;
        self.password.zeroize();
        self.service_account_json.zeroize();
        self.secret_access_key.zeroize();
        self.session_token.zeroize();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::{SealedSecret, SecretFields};
    use std::collections::BTreeMap;

    const PASSWORD: &str = "pa55-word-value";

    fn source(connection: ExternalConnection, secret: SecretMaterial) -> ExternalSource {
        ExternalSource {
            source_id: "src_1".to_string(),
            app_id: "app_1".to_string(),
            connection,
            secret,
        }
    }

    fn plain() -> SecretMaterial {
        SecretMaterial::Plain {
            fields: SecretFields(BTreeMap::from([
                ("username".to_string(), "reader".to_string()),
                ("password".to_string(), PASSWORD.to_string()),
            ])),
        }
    }

    fn postgres(host: &str) -> ExternalConnection {
        ExternalConnection::Postgres {
            host: host.to_string(),
            port: 5432,
            database: "sales".to_string(),
            schema: "public".to_string(),
            ssl_mode: "require".to_string(),
        }
    }

    #[tokio::test]
    async fn hosted_deployments_refuse_private_addresses_before_connecting() {
        let connector = DriverConnector::new(false);
        let source = source(postgres("10.1.2.3"), plain());

        let error = connector.tables(&source).await.unwrap_err();
        assert_eq!(error.code, ErrorCode::QueryFailed);
        assert!(
            error.message.contains("not a public address") && error.message.contains("src_1"),
            "{}",
            error.message
        );
        assert!(!error.message.contains(PASSWORD));

        let resolver = connector.resolver(&source, None).await.unwrap();
        let error = format!("{:#}", resolver.table("orders").await.unwrap_err());
        assert!(error.contains("not a public address"), "{error}");
        let again = format!("{:#}", resolver.table("customers").await.unwrap_err());
        assert_eq!(error, again);
    }

    #[tokio::test]
    async fn the_executor_network_plane_stays_closed_on_private_networks() {
        let connector = DriverConnector::new(true);
        for connection in [
            postgres("169.254.169.254"),
            ExternalConnection::Mysql {
                host: "169.254.169.254".to_string(),
                port: 3306,
                database: "shop".to_string(),
                ssl_mode: "required".to_string(),
            },
            ExternalConnection::Clickhouse {
                host: "localhost".to_string(),
                port: 8123,
                database: "events".to_string(),
            },
        ] {
            let error = connector
                .tables(&source(connection, plain()))
                .await
                .unwrap_err();
            assert_eq!(error.code, ErrorCode::QueryFailed);
            assert!(error.message.contains("refused"), "{}", error.message);
        }
    }

    #[tokio::test]
    async fn hosted_deployments_refuse_plaintext_clickhouse() {
        let connector = DriverConnector::new(false);
        let clickhouse = source(
            ExternalConnection::Clickhouse {
                host: "clickhouse.example.com".to_string(),
                port: 8123,
                database: "events".to_string(),
            },
            plain(),
        );
        let error = connector.tables(&clickhouse).await.unwrap_err();
        assert_eq!(error.code, ErrorCode::InvalidRequest);
        assert!(
            error.message.contains("plain HTTP") && error.message.contains("src_1"),
            "{}",
            error.message
        );
        assert!(!error.message.contains(PASSWORD));
        let error = connector.resolver(&clickhouse, None).await.err().unwrap();
        assert_eq!(error.code, ErrorCode::InvalidRequest);
    }

    #[tokio::test]
    async fn sealed_secrets_are_refused_without_opening_them() {
        let connector = DriverConnector::new(false);
        let sealed = source(
            postgres("db.example.com"),
            SecretMaterial::Sealed {
                sealed: SealedSecret {
                    provider: "aws-kms".to_string(),
                    key_id: None,
                    ciphertext: "AAAA".to_string(),
                },
            },
        );
        let error = connector.tables(&sealed).await.unwrap_err();
        assert_eq!(error.code, ErrorCode::Unavailable);
        assert!(error.message.contains("aws-kms"));
        let error = connector.resolver(&sealed, None).await.err().unwrap();
        assert_eq!(error.code, ErrorCode::Unavailable);
    }

    #[tokio::test]
    async fn warehouse_sources_need_their_keys_before_connecting() {
        let connector = DriverConnector::new(false);
        let bigquery = source(
            ExternalConnection::Bigquery {
                project_id: "acme".to_string(),
                dataset: "sales".to_string(),
                location: None,
            },
            SecretMaterial::None,
        );
        let error = connector.tables(&bigquery).await.unwrap_err();
        assert_eq!(error.code, ErrorCode::QueryFailed);
        assert!(
            error.message.contains("service_account_json"),
            "{}",
            error.message
        );

        let athena = source(
            ExternalConnection::Athena {
                region: "eu-central-1".to_string(),
                database: "logs".to_string(),
                catalog: None,
                workgroup: None,
                output_location: None,
            },
            SecretMaterial::None,
        );
        let error = connector.tables(&athena).await.unwrap_err();
        assert_eq!(error.code, ErrorCode::QueryFailed);
        assert!(error.message.contains("access_key_id"), "{}", error.message);
    }
}
