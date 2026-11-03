use std::sync::Arc;

use async_trait::async_trait;
use flow_like_storage::arrow_array::RecordBatch;
use flow_like_types::Bytes;
use flow_like_types::Result;

/// A table read from a file or a database, ready to be stored in the workspace.
#[derive(Debug, Clone)]
pub struct SourceTable {
    /// Empty for a file with one table; otherwise the sheet or table name.
    pub name: String,
    pub batches: Vec<RecordBatch>,
}

#[derive(Debug, Clone)]
pub struct NetworkTable {
    pub kind: String,
    pub host: String,
    pub port: u16,
    pub database: String,
    pub schema: Option<String>,
    pub table: String,
    pub username: String,
    pub password: String,
    pub ssl_mode: Option<String>,
}

/// Parsers and drivers behind `data_sources`. The catalog implements it (feature `connectors`).
#[async_trait]
pub trait DataBackend: Send + Sync {
    async fn csv(&self, bytes: Bytes, delimiter: u8, has_header: bool) -> Result<SourceTable>;

    /// The tables of a workbook, one per sheet or per detected table, and the detector's warnings.
    async fn workbook(
        &self,
        bytes: Bytes,
        sheet: Option<&str>,
        detect_tables: bool,
    ) -> Result<(Vec<SourceTable>, Vec<String>)>;

    /// Reads up to `max_rows` rows of a database table. `public_only` refuses private addresses.
    async fn network(
        &self,
        source: &NetworkTable,
        server: bool,
        public_only: bool,
        max_rows: usize,
    ) -> Result<SourceTable>;
}

pub fn default_backend() -> Option<Arc<dyn DataBackend>> {
    #[cfg(feature = "connectors")]
    return Some(Arc::new(crate::data_catalog::CatalogDataBackend));
    #[cfg(not(feature = "connectors"))]
    None
}
