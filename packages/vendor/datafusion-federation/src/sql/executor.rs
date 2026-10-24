use async_trait::async_trait;
use core::fmt;
use datafusion::{
    arrow::datatypes::SchemaRef,
    common::Statistics,
    error::Result,
    logical_expr::LogicalPlan,
    physical_plan::{metrics::MetricsSet, PhysicalExpr, SendableRecordBatchStream},
    sql::{sqlparser::ast, unparser::dialect::Dialect},
};
use std::sync::Arc;

pub type SQLExecutorRef = Arc<dyn SQLExecutor>;
pub type AstAnalyzer = Box<dyn FnMut(ast::Statement) -> Result<ast::Statement>>;
pub type LogicalOptimizer = Box<dyn FnMut(LogicalPlan) -> Result<LogicalPlan>>;
pub type SqlQueryRewriter = Box<dyn FnMut(String) -> Result<String>>;

#[async_trait]
pub trait SQLExecutor: Sync + Send {
    /// Executor name
    fn name(&self) -> &str;

    /// Executor compute context allows differentiating the remote compute context
    /// such as authorization or active database.
    ///
    /// Note: returning None here may cause incorrect federation with other providers of the
    /// same name that also have a compute_context of None.
    /// Instead try to return a unique string that will never match any other
    /// provider's context.
    fn compute_context(&self) -> Option<String>;

    /// The specific SQL dialect (currently supports 'sqlite', 'postgres', 'flight')
    fn dialect(&self) -> Arc<dyn Dialect>;

    /// Returns the analyzer rule specific for this engine to modify the logical plan before execution
    fn logical_optimizer(&self) -> Option<LogicalOptimizer> {
        None
    }

    /// Returns an AST analyzer specific for this engine to modify the AST before execution
    fn ast_analyzer(&self) -> Option<AstAnalyzer> {
        None
    }

    /// Execute a SQL query.
    ///
    /// `filters` contain physical expressions generated at runtime, like
    /// `DynamicFilterPhysicalExpr`. Since the concrete expression values only become available when
    /// the `SendableRecordBatchStream` is executed, they must be manually added to the SQL query,
    /// if necessary. However, they can be safely ignored.
    fn execute(
        &self,
        query: &str,
        schema: SchemaRef,
        filters: &[Arc<dyn PhysicalExpr>],
    ) -> Result<SendableRecordBatchStream>;

    /// Returns statistics for this `SQLExecutor` node. If statistics are not available, it should
    /// return [`Statistics::new_unknown`] (the default), not an error. See the `ExecutionPlan`
    /// trait.
    async fn statistics(&self, plan: &LogicalPlan) -> Result<Statistics> {
        Ok(Statistics::new_unknown(plan.schema().as_arrow()))
    }

    /// Returns the tables provided by the remote
    async fn table_names(&self) -> Result<Vec<String>>;

    /// Returns the schema of table_name within this [`SQLExecutor`]
    async fn get_table_schema(&self, table_name: &str) -> Result<SchemaRef>;

    /// Returns the execution metrics, if available.
    fn metrics(&self) -> Option<MetricsSet> {
        None
    }
}

impl fmt::Debug for dyn SQLExecutor {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{} {:?}", self.name(), self.compute_context())
    }
}

impl fmt::Display for dyn SQLExecutor {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{} {:?}", self.name(), self.compute_context())
    }
}
