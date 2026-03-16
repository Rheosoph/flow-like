use crate::generative::embedding::CachedEmbeddingModel;
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic, NodeScores},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_catalog_core::NodeDBConnection;
use flow_like_types::{
    JsonSchema,
    async_trait,
    json::{self, json, Deserialize, Serialize},
};

#[derive(Clone, Serialize, Deserialize, JsonSchema)]
pub enum RecallStrategy {
    RecentFirst,
    Relevance,
    Hybrid,
}

impl Default for RecallStrategy {
    fn default() -> Self {
        Self::Hybrid
    }
}

#[derive(Clone, Serialize, Deserialize, JsonSchema)]
pub struct MemoryConfig {
    pub database: NodeDBConnection,
    pub embedding_model: CachedEmbeddingModel,
    pub max_context_tokens: u32,
    pub recall_strategy: RecallStrategy,
    pub recall_top_k: u32,
    pub auto_compress: bool,
    pub compress_threshold: u32,
}

impl MemoryConfig {
    pub fn new(database: NodeDBConnection, embedding_model: CachedEmbeddingModel) -> Self {
        Self {
            database,
            embedding_model,
            max_context_tokens: 4000,
            recall_strategy: RecallStrategy::default(),
            recall_top_k: 20,
            auto_compress: true,
            compress_threshold: 30,
        }
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct CreateMemoryConfigNode {}

impl CreateMemoryConfigNode {
    pub fn new() -> Self {
        CreateMemoryConfigNode {}
    }
}

#[async_trait]
impl NodeLogic for CreateMemoryConfigNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "memory_create_config",
            "Create Memory Config",
            "Creates a MemoryConfig that bundles database, embedding model, and tuning parameters for all memory nodes",
            "AI/Memory",
        );
        node.add_icon("/flow/icons/bot-invoke.svg");

        node.set_scores(
            NodeScores::new()
                .set_privacy(8)
                .set_security(8)
                .set_performance(10)
                .set_governance(8)
                .set_reliability(9)
                .set_cost(10)
                .build(),
        );

        node.add_input_pin(
            "database",
            "Database",
            "LanceDB connection (from Open Database node). The table IS the scope boundary — use one table per user/session.",
            VariableType::Struct,
        )
        .set_schema::<NodeDBConnection>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_input_pin(
            "embedding_model",
            "Embedding Model",
            "Cached embedding model for vector search (from Load Embedding Model node)",
            VariableType::Struct,
        )
        .set_schema::<CachedEmbeddingModel>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_input_pin(
            "max_context_tokens",
            "Max Context Tokens",
            "Token budget for assembled memory context",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(4000)));

        node.add_input_pin(
            "recall_strategy",
            "Recall Strategy",
            "How to retrieve memories: recent_first (last N), relevance (vector similarity), hybrid (both)",
            VariableType::String,
        )
        .set_options(
            PinOptions::new()
                .set_valid_values(vec![
                    "recent_first".into(),
                    "relevance".into(),
                    "hybrid".into(),
                ])
                .build(),
        )
        .set_default_value(Some(json!("hybrid")));

        node.add_input_pin(
            "recall_top_k",
            "Recall Top K",
            "Max items returned from vector search",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(20)));

        node.add_input_pin(
            "auto_compress",
            "Auto Compress",
            "Automatically compress old observations when threshold is reached",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(true)));

        node.add_input_pin(
            "compress_threshold",
            "Compress Threshold",
            "Number of observations before triggering compression",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(30)));

        node.add_output_pin(
            "memory_config",
            "Memory Config",
            "Configured MemoryConfig — pass to any memory node",
            VariableType::Struct,
        )
        .set_schema::<MemoryConfig>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let database: NodeDBConnection = context.evaluate_pin("database").await?;
        let embedding_model: CachedEmbeddingModel =
            context.evaluate_pin("embedding_model").await?;
        let max_context_tokens: i64 = context
            .evaluate_pin("max_context_tokens")
            .await
            .unwrap_or(4000);
        let recall_strategy: String = context
            .evaluate_pin("recall_strategy")
            .await
            .unwrap_or_else(|_| "hybrid".to_string());
        let recall_top_k: i64 = context.evaluate_pin("recall_top_k").await.unwrap_or(20);
        let auto_compress: bool = context
            .evaluate_pin("auto_compress")
            .await
            .unwrap_or(true);
        let compress_threshold: i64 = context
            .evaluate_pin("compress_threshold")
            .await
            .unwrap_or(30);

        let strategy = match recall_strategy.as_str() {
            "recent_first" => RecallStrategy::RecentFirst,
            "relevance" => RecallStrategy::Relevance,
            _ => RecallStrategy::Hybrid,
        };

        let mut config = MemoryConfig::new(database, embedding_model);
        config.max_context_tokens = max_context_tokens.max(0) as u32;
        config.recall_strategy = strategy;
        config.recall_top_k = recall_top_k.max(1) as u32;
        config.auto_compress = auto_compress;
        config.compress_threshold = compress_threshold.max(1) as u32;

        context
            .set_pin_value("memory_config", json::json!(config))
            .await?;

        Ok(())
    }
}
