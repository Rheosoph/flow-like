use std::sync::Arc;

use async_trait::async_trait;
use datafusion::{
    arrow::datatypes::SchemaRef,
    catalog::SchemaProvider,
    error::{DataFusionError, Result},
    execution::context::{SessionContext, SessionState},
    physical_plan::{stream::RecordBatchStreamAdapter, PhysicalExpr, SendableRecordBatchStream},
    sql::unparser::dialect::{DefaultDialect, Dialect},
};
use futures::TryStreamExt;

use datafusion_federation::sql::SQLExecutor;

pub fn overwrite_default_schema(
    state: &SessionState,
    schema: Arc<dyn SchemaProvider>,
) -> Result<()> {
    let options = &state.config().options().catalog;
    let catalog = state
        .catalog_list()
        .catalog(options.default_catalog.as_str())
        .unwrap();

    catalog.register_schema(options.default_schema.as_str(), schema)?;
    Ok(())
}

pub struct MockSqliteExecutor {
    session: Arc<SessionContext>,
}

impl MockSqliteExecutor {
    pub fn new(session: Arc<SessionContext>) -> Self {
        Self { session }
    }
}

#[async_trait]
impl SQLExecutor for MockSqliteExecutor {
    fn name(&self) -> &str {
        "mock_sqlite_executor"
    }

    fn compute_context(&self) -> Option<String> {
        // Don't return None here - it will cause incorrect federation with other providers of the
        // same name that also have a compute_context of None.
        // Instead return a random string that will never match any other provider's context.
        Some("sqlite_exec".to_string())
    }

    fn execute(
        &self,
        sql: &str,
        schema: SchemaRef,
        _filters: &[Arc<dyn PhysicalExpr>],
    ) -> Result<SendableRecordBatchStream> {
        // Execute it using the remote datafusion session context
        let future_stream = _execute(self.session.clone(), sql.to_string());
        let stream = futures::stream::once(future_stream).try_flatten();
        Ok(Box::pin(RecordBatchStreamAdapter::new(
            schema.clone(),
            stream,
        )))
    }

    async fn table_names(&self) -> Result<Vec<String>> {
        Err(DataFusionError::NotImplemented(
            "table inference not implemented".to_string(),
        ))
    }

    async fn get_table_schema(&self, table_name: &str) -> Result<SchemaRef> {
        let sql = format!("select * from {table_name} limit 1");
        let df = self.session.sql(&sql).await?;
        let schema = df.schema().as_arrow().clone();
        Ok(Arc::new(schema))
    }

    fn dialect(&self) -> Arc<dyn Dialect> {
        Arc::new(DefaultDialect {})
    }
}

#[allow(dead_code)]
pub struct MockPostgresExecutor {
    session: Arc<SessionContext>,
}

#[allow(dead_code)]
impl MockPostgresExecutor {
    pub fn new(session: Arc<SessionContext>) -> Self {
        Self { session }
    }
}

#[async_trait]
impl SQLExecutor for MockPostgresExecutor {
    fn name(&self) -> &str {
        "mock_postgres_executor"
    }

    fn compute_context(&self) -> Option<String> {
        // Don't return None here - it will cause incorrect federation with other providers of the
        // same name that also have a compute_context of None.
        // Instead return a random string that will never match any other provider's context.
        Some("postgres_exec".to_string())
    }

    fn execute(
        &self,
        sql: &str,
        schema: SchemaRef,
        _filters: &[Arc<dyn PhysicalExpr>],
    ) -> Result<SendableRecordBatchStream> {
        // Execute it using the remote datafusion session context
        let future_stream = _execute(self.session.clone(), sql.to_string());
        let stream = futures::stream::once(future_stream).try_flatten();
        Ok(Box::pin(RecordBatchStreamAdapter::new(
            schema.clone(),
            stream,
        )))
    }

    async fn table_names(&self) -> Result<Vec<String>> {
        Err(DataFusionError::NotImplemented(
            "table inference not implemented".to_string(),
        ))
    }

    async fn get_table_schema(&self, table_name: &str) -> Result<SchemaRef> {
        let sql = format!("select * from {table_name} limit 1");
        let df = self.session.sql(&sql).await?;
        let schema = df.schema().as_arrow().clone();
        Ok(Arc::new(schema))
    }

    fn dialect(&self) -> Arc<dyn Dialect> {
        Arc::new(DefaultDialect {})
    }
}

async fn _execute(ctx: Arc<SessionContext>, sql: String) -> Result<SendableRecordBatchStream> {
    ctx.sql(&sql).await?.execute_stream().await
}
