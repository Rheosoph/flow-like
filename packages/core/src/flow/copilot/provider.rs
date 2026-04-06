use async_trait::async_trait;

use super::types::NodeMetadata;

/// Trait for providing catalog search functionality
#[async_trait]
pub trait CatalogProvider: Send + Sync {
    async fn search(&self, query: &str) -> Vec<NodeMetadata>;
    async fn search_by_pin_type(&self, pin_type: &str, is_input: bool) -> Vec<NodeMetadata>;
    async fn filter_by_category(&self, category_prefix: &str) -> Vec<NodeMetadata>;
    async fn get_node_metadata(&self, node_type: &str) -> Option<NodeMetadata>;
    async fn get_all_nodes(&self) -> Vec<String>;
}
