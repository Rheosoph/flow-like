use super::config::{MemoryConfig, RecallStrategy};
use crate::generative::embedding::CachedEmbeddingModelObject;
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic, NodeScores},
    pin::{PinOptions, ValueType},
    variable::VariableType,
};
use flow_like_storage::databases::vector::VectorStore;
use flow_like_types::{
    Value, async_trait, bail,
    json::{self, json},
};

#[crate::register_node]
#[derive(Default)]
pub struct SearchMemoryNode {}

impl SearchMemoryNode {
    pub fn new() -> Self {
        SearchMemoryNode {}
    }
}

#[async_trait]
impl NodeLogic for SearchMemoryNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "memory_search",
            "Search Memory",
            "Searches the memory store using the configured recall strategy (recent, relevance, or hybrid)",
            "AI/Memory",
        );
        node.add_icon("/flow/icons/bot-invoke.svg");
        node.set_long_running(true);

        node.set_scores(
            NodeScores::new()
                .set_privacy(6)
                .set_security(7)
                .set_performance(6)
                .set_governance(7)
                .set_reliability(8)
                .set_cost(5)
                .build(),
        );

        node.add_input_pin("exec_in", "Input", "Trigger", VariableType::Execution);

        node.add_input_pin(
            "memory_config",
            "Memory Config",
            "MemoryConfig from Create Memory Config node",
            VariableType::Struct,
        )
        .set_schema::<MemoryConfig>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_input_pin(
            "query",
            "Query",
            "Search query text — used for vector similarity and/or full-text search",
            VariableType::String,
        );

        node.add_input_pin(
            "filter",
            "SQL Filter",
            "Optional SQL filter (e.g. \"role = 'user'\")",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_output_pin(
            "exec_out",
            "Done",
            "Fires when search completes",
            VariableType::Execution,
        );

        node.add_output_pin(
            "results",
            "Results",
            "Array of matching memory records (sorted by relevance/recency)",
            VariableType::Struct,
        )
        .set_value_type(ValueType::Array);

        node.add_output_pin(
            "result_count",
            "Result Count",
            "Number of results returned",
            VariableType::Integer,
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let config: MemoryConfig = context.evaluate_pin("memory_config").await?;
        let query: String = context.evaluate_pin("query").await?;
        let filter: String = context.evaluate_pin("filter").await.unwrap_or_default();
        let filter_opt: Option<&str> = if filter.is_empty() {
            None
        } else {
            Some(&filter)
        };

        let top_k = config.recall_top_k as usize;

        let cached_db = config.database.load(context).await?;
        cached_db.ensure_flushed().await?;
        let db = cached_db.db.read().await;

        let results: Vec<Value> = match config.recall_strategy {
            RecallStrategy::RecentFirst => {
                db.filter(
                    filter_opt.unwrap_or("1=1"),
                    Some(vec![
                        "id".to_string(),
                        "content".to_string(),
                        "role".to_string(),
                        "timestamp".to_string(),
                    ]),
                    top_k,
                    0,
                )
                .await?
            }
            RecallStrategy::Relevance => {
                let vector = embed_query(context, &config, &query).await?;
                db.vector_search(vector, filter_opt, None, top_k, 0).await?
            }
            RecallStrategy::Hybrid => {
                let vector = embed_query(context, &config, &query).await?;
                db.hybrid_search(
                    vector,
                    &query,
                    filter_opt,
                    None,
                    Some(vec!["content".to_string()]),
                    top_k,
                    0,
                    true,
                )
                .await?
            }
        };

        let count = results.len();
        context
            .set_pin_value("results", json::json!(results))
            .await?;
        context
            .set_pin_value("result_count", json!(count as i64))
            .await?;
        context.activate_exec_pin("exec_out").await?;

        Ok(())
    }
}

#[allow(dead_code)]
async fn embed_query(
    context: &mut ExecutionContext,
    config: &MemoryConfig,
    query: &str,
) -> flow_like_types::Result<Vec<f64>> {
    let cached_model = context
        .get_cache(&config.embedding_model.cache_key)
        .await
        .ok_or_else(|| flow_like_types::anyhow!("Embedding model not found in cache"))?;
    let embedding_obj = cached_model
        .as_any()
        .downcast_ref::<CachedEmbeddingModelObject>()
        .ok_or_else(|| flow_like_types::anyhow!("Failed to downcast embedding model"))?;

    let embeddings = if let Some(model) = &embedding_obj.text_model {
        model.text_embed_query(&vec![query.to_string()]).await?
    } else {
        bail!("No text embedding model available");
    };

    if embeddings.is_empty() {
        bail!("Embedding returned empty vector");
    }

    Ok(embeddings[0].iter().map(|x| *x as f64).collect())
}
