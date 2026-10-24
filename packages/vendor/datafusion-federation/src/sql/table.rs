use crate::sql::SQLFederationProvider;
use crate::FederatedTableSource;
use crate::FederationProvider;
use datafusion::arrow::datatypes::SchemaRef;
use datafusion::error::Result;
use datafusion::logical_expr::TableSource;
use datafusion::logical_expr::TableType;
use datafusion::sql::TableReference;
use std::any::Any;
use std::sync::Arc;

use super::ast_analyzer;
use super::executor::LogicalOptimizer;
use super::executor::SqlQueryRewriter;
use super::AstAnalyzer;
use super::RemoteTableRef;

/// Trait to represent a SQL remote table inside [`SQLTableSource`].
/// A remote table provides information such as schema, table reference, and
/// provides hooks for rewriting the logical plan and AST before execution.
/// This crate provides [`RemoteTable`] as a default ready-to-use type.
pub trait SQLTable: std::fmt::Debug + Send + Sync {
    /// Returns a reference as a trait object.
    fn as_any(&self) -> &dyn Any;
    /// Provides the [`TableReference`](`datafusion::sql::TableReference`) used to identify the table in SQL queries.
    /// This TableReference is used for registering the table with the [`SQLSchemaProvider`](`super::SQLSchemaProvider`).
    /// If the table provider is registered in the Datafusion context under a different name,
    /// the logical plan will be rewritten to use this table reference during execution.
    /// Therefore, any AST analyzer should match against this table reference.
    fn table_reference(&self) -> TableReference;
    /// Schema of the remote table
    fn schema(&self) -> SchemaRef;
    /// Returns a logical optimizer specific to this table, will be used to modify the logical plan before execution
    fn logical_optimizer(&self) -> Option<LogicalOptimizer> {
        None
    }
    /// Returns an AST analyzer specific to this table, will be used to modify the AST before execution
    fn ast_analyzer(&self) -> Option<AstAnalyzer> {
        None
    }
    /// Returns a SQL query rewriter specific to this table.
    /// This hook is applied after AST rewrites and can directly alter the final SQL string.
    fn sql_query_rewriter(&self) -> Option<SqlQueryRewriter> {
        None
    }
}

/// Represents a remote table with a reference and schema.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct RemoteTable {
    remote_table_ref: RemoteTableRef,
    schema: SchemaRef,
}

impl RemoteTable {
    /// Creates a new `RemoteTable` instance.
    ///
    /// Examples:
    /// ```ignore
    /// use datafusion::sql::TableReference;
    ///
    /// RemoteTable::new("myschema.table".try_into()?, schema);
    /// RemoteTable::new(r#"myschema."Table""#.try_into()?, schema);
    /// RemoteTable::new(TableReference::partial("myschema", "table").into(), schema);
    /// RemoteTable::new("myschema.view('obj')".try_into()?, schema);
    /// RemoteTable::new("myschema.view(name => 'obj')".try_into()?, schema);
    /// RemoteTable::new("myschema.view(name = 'obj')".try_into()?, schema);
    /// ```
    pub fn new(table_ref: RemoteTableRef, schema: SchemaRef) -> Self {
        Self {
            remote_table_ref: table_ref,
            schema,
        }
    }

    /// Return table reference of this remote table.
    /// Only returns the object name, ignoring functional params if any
    pub fn table_reference(&self) -> &TableReference {
        self.remote_table_ref.table_ref()
    }

    pub fn schema(&self) -> &SchemaRef {
        &self.schema
    }
}

impl SQLTable for RemoteTable {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn table_reference(&self) -> TableReference {
        Self::table_reference(self).clone()
    }

    fn schema(&self) -> SchemaRef {
        Arc::clone(&self.schema)
    }

    fn logical_optimizer(&self) -> Option<LogicalOptimizer> {
        None
    }

    /// Returns ast analyzer that modifies table that contains functional args after table ident
    fn ast_analyzer(&self) -> Option<AstAnalyzer> {
        if let Some(args) = self.remote_table_ref.args() {
            Some(
                ast_analyzer::TableArgReplace::default()
                    .with(self.remote_table_ref.table_ref().clone(), args.to_vec())
                    .into_analyzer(),
            )
        } else {
            None
        }
    }
}

#[derive(Debug, Clone)]
pub struct SQLTableSource {
    pub provider: Arc<SQLFederationProvider>,
    pub table: Arc<dyn SQLTable>,
}

impl SQLTableSource {
    // creates a SQLTableSource and infers the table schema
    pub async fn new(
        provider: Arc<SQLFederationProvider>,
        table_ref: RemoteTableRef,
    ) -> Result<Self> {
        let table_name = table_ref.to_quoted_string();
        let schema = provider.executor.get_table_schema(&table_name).await?;
        Ok(Self::new_with_schema(provider, table_ref, schema))
    }

    /// Create a SQLTableSource with a table reference and schema
    pub fn new_with_schema(
        provider: Arc<SQLFederationProvider>,
        table_ref: RemoteTableRef,
        schema: SchemaRef,
    ) -> Self {
        Self {
            provider,
            table: Arc::new(RemoteTable::new(table_ref, schema)),
        }
    }

    /// Create new with a custom SQLtable instance.
    pub fn new_with_table(provider: Arc<SQLFederationProvider>, table: Arc<dyn SQLTable>) -> Self {
        Self { provider, table }
    }

    /// Return associated table reference of stored remote table
    pub fn table_reference(&self) -> TableReference {
        self.table.table_reference()
    }
}

impl TableSource for SQLTableSource {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn schema(&self) -> SchemaRef {
        self.table.schema()
    }

    fn table_type(&self) -> TableType {
        TableType::Temporary
    }
}

impl FederatedTableSource for SQLTableSource {
    fn federation_provider(&self) -> Arc<dyn FederationProvider> {
        Arc::clone(&self.provider) as Arc<dyn FederationProvider>
    }
}
