use super::config::MemoryConfig;
#[cfg(feature = "execute")]
use super::config::RecallStrategy;
#[cfg(feature = "execute")]
use crate::generative::embedding::CachedEmbeddingModelObject;
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic, NodeScores},
    pin::{PinOptions, ValueType},
    variable::VariableType,
};
#[cfg(feature = "execute")]
use flow_like_storage::databases::vector::VectorStore;
#[cfg(feature = "execute")]
use flow_like_types::{Value, bail, json};
use flow_like_types::{async_trait, json::json};

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
        node.set_flowscript_name("ai.memory", "search");
        node.set_receiver("memory_config");
        node.set_version(2);
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
            "role_filter",
            "Role Filter",
            "Optional role filter (one of: user, assistant, observation, summary, context)",
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
        .set_value_type(ValueType::Array)
        .set_open_schema();

        node.add_output_pin(
            "result_count",
            "Result Count",
            "Number of results returned",
            VariableType::Integer,
        );

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let config: MemoryConfig = context.evaluate_pin("memory_config").await?;
        let query: String = context.evaluate_pin("query").await?;
        let role_filter: String = context
            .evaluate_pin("role_filter")
            .await
            .unwrap_or_default();
        let allowed_roles = ["user", "assistant", "observation", "summary", "context"];
        let filter_expr: Option<String> =
            if !role_filter.is_empty() && allowed_roles.contains(&role_filter.as_str()) {
                Some(format!("role = '{}'", role_filter))
            } else {
                None
            };
        let filter_opt: Option<&str> = filter_expr.as_deref();

        let top_k = config.recall_top_k as usize;

        let cached_db = config.database.load(context).await?;
        cached_db.ensure_flushed().await?;
        let db = cached_db.db.read().await;

        let mut results: Vec<Value> = match config.recall_strategy {
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

        // Client-side sort by timestamp descending for RecentFirst
        if matches!(config.recall_strategy, RecallStrategy::RecentFirst) {
            results.sort_by(|a, b| {
                let ts_a = a.get("timestamp").and_then(|v| v.as_i64()).unwrap_or(0);
                let ts_b = b.get("timestamp").and_then(|v| v.as_i64()).unwrap_or(0);
                ts_b.cmp(&ts_a)
            });
        }

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

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "Memory search requires the 'execute' feature"
        ))
    }
}

#[cfg(feature = "execute")]
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
