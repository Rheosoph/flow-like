use async_trait::async_trait;
use flow_like_catalog_data::data::datafusion::connectors::{
    ClickhouseSource, ConnectPolicy, ConnectorSource, CsvSource, MysqlSource, PostgresSource,
    connect_table, csv_provider, workbook_providers,
};
use flow_like_catalog_data::data::excel::table_detect::{ExtractConfig, SheetTableMode};
use flow_like_runtime::flow::execution::ExecutionEnvironment;
use flow_like_storage::datafusion::catalog::TableProvider;
use flow_like_storage::datafusion::prelude::SessionContext;
use flow_like_types::{Bytes, Result, anyhow};
use std::sync::Arc;

use crate::data_backend::{DataBackend, NetworkTable, SourceTable};

pub struct CatalogDataBackend;

async fn read_provider(
    provider: Arc<dyn TableProvider>,
    name: String,
    max_rows: usize,
) -> Result<SourceTable> {
    let context = SessionContext::new();
    let batches = context
        .read_table(provider)?
        .limit(0, Some(max_rows))?
        .collect()
        .await?;
    Ok(SourceTable { name, batches })
}

fn connector(source: &NetworkTable) -> Result<ConnectorSource> {
    let schema = source
        .schema
        .clone()
        .unwrap_or_else(|| "public".to_string());
    Ok(match source.kind.as_str() {
        "postgres" => ConnectorSource::Postgres(PostgresSource {
            host: source.host.clone(),
            port: source.port,
            database: source.database.clone(),
            username: source.username.clone(),
            password: source.password.clone(),
            schema,
            table: source.table.clone(),
            ssl_mode: source
                .ssl_mode
                .clone()
                .unwrap_or_else(|| "require".to_string()),
            readonly: true,
        }),
        "mysql" => ConnectorSource::Mysql(MysqlSource {
            host: source.host.clone(),
            port: source.port,
            database: source.database.clone(),
            username: source.username.clone(),
            password: source.password.clone(),
            table: source.table.clone(),
            ssl_mode: source
                .ssl_mode
                .clone()
                .unwrap_or_else(|| "required".to_string()),
            readonly: true,
        }),
        "clickhouse" => ConnectorSource::Clickhouse(ClickhouseSource {
            host: source.host.clone(),
            port: source.port,
            database: source.database.clone(),
            username: source.username.clone(),
            password: source.password.clone(),
            table: source.table.clone(),
        }),
        other => return Err(anyhow!("Database kind '{other}' is not supported")),
    })
}

#[async_trait]
impl DataBackend for CatalogDataBackend {
    async fn csv(&self, bytes: Bytes, delimiter: u8, has_header: bool) -> Result<SourceTable> {
        let provider = csv_provider(
            &bytes,
            &CsvSource {
                delimiter,
                has_header,
            },
        )?;
        read_provider(provider, String::new(), usize::MAX / 2).await
    }

    async fn workbook(
        &self,
        bytes: Bytes,
        sheet: Option<&str>,
        detect_tables: bool,
    ) -> Result<(Vec<SourceTable>, Vec<String>)> {
        let mode = if detect_tables {
            SheetTableMode::DetectTables
        } else {
            SheetTableMode::WholeSheet
        };
        let (providers, warnings) =
            workbook_providers(bytes.to_vec(), sheet, &ExtractConfig::default(), mode, "")?;
        let mut tables = Vec::new();
        for (name, provider) in providers {
            tables.push(read_provider(provider, name, usize::MAX / 2).await?);
        }
        Ok((tables, warnings))
    }

    async fn network(
        &self,
        source: &NetworkTable,
        server: bool,
        public_only: bool,
        max_rows: usize,
    ) -> Result<SourceTable> {
        let environment = if server {
            ExecutionEnvironment::Server
        } else {
            ExecutionEnvironment::Local
        };
        let policy = if public_only {
            ConnectPolicy::public_only(environment)
        } else {
            ConnectPolicy::new(environment)
        };
        let provider = connect_table(&connector(source)?, policy).await?;
        read_provider(provider, source.table.clone(), max_rows).await
    }
}
