use std::{any::Any, sync::Arc};

use async_trait::async_trait;
use datafusion::{catalog::SchemaProvider, datasource::TableProvider, error::Result};
use futures::future::join_all;

use super::{table::SQLTable, RemoteTableRef, SQLTableSource};
use crate::{sql::SQLFederationProvider, FederatedTableProviderAdaptor};

/// An in-memory schema provider for SQL tables.
#[derive(Debug)]
pub struct SQLSchemaProvider {
    tables: Vec<Arc<SQLTableSource>>,
}

impl SQLSchemaProvider {
    /// Creates a new SQLSchemaProvider from a [`SQLFederationProvider`].
    /// Initializes the schema provider by fetching table names and schema from the federation provider's executor,
    pub async fn new(provider: Arc<SQLFederationProvider>) -> Result<Self> {
        let tables = Arc::clone(&provider.executor)
            .table_names()
            .await?
            .iter()
            .map(RemoteTableRef::try_from)
            .collect::<Result<Vec<_>>>()?;

        Self::new_with_table_references(provider, tables).await
    }

    /// Creates a new SQLSchemaProvider from a SQLFederationProvider and a list of table references.
    /// Fetches the schema for each table using the executor's implementation.
    pub async fn new_with_tables<T: AsRef<str>>(
        provider: Arc<SQLFederationProvider>,
        tables: impl IntoIterator<Item = T>,
    ) -> Result<Self> {
        let tables = tables
            .into_iter()
            .map(|x| RemoteTableRef::try_from(x.as_ref()))
            .collect::<Result<Vec<_>>>()?;

        let futures: Vec<_> = tables
            .into_iter()
            .map(|t| SQLTableSource::new(Arc::clone(&provider), t))
            .collect();
        let results: Result<Vec<_>> = join_all(futures).await.into_iter().collect();
        let tables = results?.into_iter().map(Arc::new).collect();
        Ok(Self { tables })
    }

    /// Creates a new SQLSchemaProvider from a SQLFederationProvider and a list of custom table instances.
    pub fn new_with_custom_tables(
        provider: Arc<SQLFederationProvider>,
        tables: Vec<Arc<dyn SQLTable>>,
    ) -> Self {
        Self {
            tables: tables
                .into_iter()
                .map(|table| SQLTableSource::new_with_table(provider.clone(), table))
                .map(Arc::new)
                .collect(),
        }
    }

    pub async fn new_with_table_references(
        provider: Arc<SQLFederationProvider>,
        tables: Vec<RemoteTableRef>,
    ) -> Result<Self> {
        let futures: Vec<_> = tables
            .into_iter()
            .map(|t| SQLTableSource::new(Arc::clone(&provider), t))
            .collect();
        let results: Result<Vec<_>> = join_all(futures).await.into_iter().collect();
        let tables = results?.into_iter().map(Arc::new).collect();
        Ok(Self { tables })
    }
}

#[async_trait]
impl SchemaProvider for SQLSchemaProvider {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn table_names(&self) -> Vec<String> {
        self.tables
            .iter()
            .map(|source| source.table_reference().to_string())
            .collect()
    }

    async fn table(&self, name: &str) -> Result<Option<Arc<dyn TableProvider>>> {
        if let Some(source) = self
            .tables
            .iter()
            .find(|s| s.table_reference().to_string().eq(name))
        {
            let adaptor = FederatedTableProviderAdaptor::new(source.clone());
            return Ok(Some(Arc::new(adaptor)));
        }
        Ok(None)
    }

    fn table_exist(&self, name: &str) -> bool {
        self.tables
            .iter()
            .any(|source| source.table_reference().to_string().eq(name))
    }
}

#[derive(Debug)]
pub struct MultiSchemaProvider {
    children: Vec<Arc<dyn SchemaProvider>>,
}

impl MultiSchemaProvider {
    pub fn new(children: Vec<Arc<dyn SchemaProvider>>) -> Self {
        Self { children }
    }
}

#[async_trait]
impl SchemaProvider for MultiSchemaProvider {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn table_names(&self) -> Vec<String> {
        self.children.iter().flat_map(|p| p.table_names()).collect()
    }

    async fn table(&self, name: &str) -> Result<Option<Arc<dyn TableProvider>>> {
        for child in &self.children {
            if let Ok(Some(table)) = child.table(name).await {
                return Ok(Some(table));
            }
        }
        Ok(None)
    }

    fn table_exist(&self, name: &str) -> bool {
        self.children.iter().any(|p| p.table_exist(name))
    }
}
