pub mod buffered;
pub mod lancedb;
pub mod schema;
use flow_like_types::{Result, Value, async_trait};

#[allow(clippy::too_many_arguments)]
#[async_trait]
pub trait VectorStore: Send + Sync {
    /// Search for vectors similar to the given vector, potentially filtering results.
    ///
    /// # Arguments
    ///
    /// * `vector`: The vector to search for similar vectors.
    /// * `filter`: An optional filter to narrow down the search results.
    /// * `select`: Optional columns to return in the results.
    /// * `fields`: Column names to search. For vector search, searches each column.
    /// * `limit`: The maximum number of results to return.
    /// * `offset`: The number of results to skip.
    ///
    /// # Returns
    ///
    /// A result containing a vector of JSON-encoded search results or an error.
    async fn vector_search(
        &self,
        vector: Vec<f64>,
        filter: Option<&str>,
        select: Option<Vec<String>>,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<Value>>;

    /// Perform a full-text search using the given text input.
    ///
    /// # Arguments
    ///
    /// * `text`: The text to search for similar items.
    /// * `filter`: An optional filter to narrow down the search results.
    /// * `select`: Optional columns to return in the results.
    /// * `fields`: Column names to search with full-text search.
    /// * `limit`: The maximum number of results to return.
    /// * `offset`: The number of results to skip.
    ///
    /// # Returns
    ///
    /// A result containing a vector of JSON-encoded search results or an error.
    async fn fts_search(
        &self,
        text: &str,
        filter: Option<&str>,
        select: Option<Vec<String>>,
        fields: Option<Vec<String>>,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<Value>>;

    /// Perform a hybrid search using both vector and text input.
    ///
    /// # Arguments
    ///
    /// * `vector`: The vector to search for similar vectors.
    /// * `text`: The text to search for similar items.
    /// * `filter`: An optional filter to narrow down the search results.
    /// * `select`: Optional columns to return in the results.
    /// * `fields`: Column names for both vector and FTS search.
    /// * `limit`: The maximum number of results to return.
    /// * `offset`: The number of results to skip.
    /// * `rerank`: Whether to rerank results using RRF.
    ///
    /// # Returns
    ///
    /// A result containing a vector of JSON-encoded search results or an error.
    async fn hybrid_search(
        &self,
        vector: Vec<f64>,
        text: &str,
        filter: Option<&str>,
        select: Option<Vec<String>>,
        fields: Option<Vec<String>>,
        limit: usize,
        offset: usize,
        rerank: bool,
    ) -> Result<Vec<Value>>;

    /// Query the vector store based on a filter.
    ///
    /// # Arguments
    ///
    /// * `filter`: The filter to apply to the query.
    /// * `limit`: The maximum number of results to return.
    ///
    /// # Returns
    ///
    /// A result containing a vector of JSON-encoded query results or an error.
    async fn filter(
        &self,
        filter: &str,
        select: Option<Vec<String>>,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<Value>>;

    /// Upsert items into the vector store.
    ///
    /// # Arguments
    ///
    /// * `items`: A vector of JSON-encoded items to upsert.
    ///
    /// # Returns
    ///
    /// A result indicating success or an error.
    async fn upsert(&mut self, items: Vec<Value>, id_field: String) -> Result<()>;

    /// Upsert items into the vector store.
    ///
    /// # Arguments
    ///
    /// * `items`: A vector of JSON-encoded items to upsert.
    ///
    /// # Returns
    ///
    /// A result indicating success or an error.
    async fn insert(&mut self, items: Vec<Value>) -> Result<()>;

    /// Delete items from the vector store based on a filter.
    ///
    /// # Arguments
    ///
    /// * `filter`: The filter to select items for deletion.
    ///
    /// # Returns
    ///
    /// A result indicating success or an error.
    async fn delete(&self, filter: &str) -> Result<()>;

    /// Build a search index on the specified column.
    ///
    /// # Arguments
    ///
    /// * `column`: The column name to build the index on.
    /// * `fts`: Builds a full-text search index if true, otherwise determines the type of index automatically.
    ///
    /// # Returns
    ///
    /// A result indicating success or an error.
    async fn index(&self, column: &str, index_type: Option<&str>) -> Result<()>;

    /// Optimize the vector store (implementation-specific).
    ///
    /// # Returns
    ///
    /// A result indicating success or an error.
    async fn optimize(&self, keep_versions: bool) -> Result<()>;

    /// List all items in the vector store.
    ///
    /// # Returns
    ///
    /// A result containing a vector of JSON-encoded items or an error.
    async fn list(
        &self,
        select: Option<Vec<String>>,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<Value>>;

    /// Purge all data from the vector store.
    ///
    /// # Returns
    ///
    /// A result indicating success or an error.
    async fn purge(&self) -> Result<()>;

    /// Returns the total number of items in the vector store.
    ///
    /// # Returns
    ///
    /// * `Ok(usize)` - The total count of items in the vector store.
    /// * `Err(anyhow::Error)` - If the count operation fails.
    async fn count(&self, filter: Option<String>) -> Result<usize>;

    async fn schema(&self) -> Result<arrow_schema::Schema>;

    async fn flush(&mut self) -> Result<()> {
        Ok(())
    }

    fn is_dirty(&self) -> bool {
        false
    }
}
