#[cfg(feature = "execute")]
use crate::generative::embedding::CachedEmbeddingModelObject;
/// # Lazy Register Function Tools Node
/// Indexes referenced Flow-Like functions into a local LanceDB so that agents can
/// perform hybrid (FTS + vector) searches to discover tools at runtime instead of
/// loading every tool schema into the context window up front.
use crate::generative::{
    agent::{Agent, LazyFunctionRef},
    embedding::CachedEmbeddingModel,
};
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic, NodeScores},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_storage::databases::vector::lancedb::LanceDBVectorStore;
use flow_like_types::sync::RwLock;
use flow_like_types::{Cacheable, async_trait, json};
use std::sync::Arc;

/// Minimum number of rows before a vector (ANN) index is built.
/// Below this threshold only FTS is available; lancedb auto-indexing works best
/// with at least a few hundred rows, but 50 is a safe lower bound.
const VECTOR_INDEX_THRESHOLD: usize = 50;

/// Cached handle to the LanceDB used for lazy tool indexing.
pub struct CachedLazyToolDB {
    pub db: Arc<RwLock<LanceDBVectorStore>>,
}

impl Cacheable for CachedLazyToolDB {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct LazyRegisterFunctionToolsNode {}

impl LazyRegisterFunctionToolsNode {
    pub fn new() -> Self {
        LazyRegisterFunctionToolsNode {}
    }
}

#[async_trait]
impl NodeLogic for LazyRegisterFunctionToolsNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "agent_lazy_register_function_tools",
            "Lazy Register Function Tools",
            "Indexes referenced Flow-Like functions into a vector DB so agents can discover tools via semantic search at runtime, keeping the context window lean.",
            "AI/Agents/Builder",
        );
        node.set_version(2);
        node.add_icon("/flow/icons/bot-invoke.svg");
        node.set_can_reference_fns(true);
        node.set_long_running(true);

        node.set_scores(
            NodeScores::new()
                .set_privacy(5)
                .set_security(5)
                .set_performance(6)
                .set_governance(6)
                .set_reliability(7)
                .set_cost(3)
                .build(),
        );

        node.add_input_pin(
            "exec_in",
            "Input",
            "Execution trigger",
            VariableType::Execution,
        );

        node.add_input_pin(
            "agent_in",
            "Agent",
            "Agent object to register lazy function tools on",
            VariableType::Struct,
        )
        .set_schema::<Agent>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_input_pin(
            "model",
            "Embedding Model",
            "Embedding model used to index functions for semantic search",
            VariableType::Struct,
        )
        .set_schema::<CachedEmbeddingModel>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_output_pin(
            "exec_out",
            "Done",
            "Fires when function indexing is complete",
            VariableType::Execution,
        );

        node.add_output_pin(
            "agent_out",
            "Agent",
            "Agent with lazy function tool references attached",
            VariableType::Struct,
        )
        .set_schema::<Agent>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        use flow_like::flow::{pin::PinType, variable::VariableType as VT};
        use flow_like_storage::databases::vector::VectorStore;

        context.deactivate_exec_pin("exec_out").await?;

        let mut agent: Agent = context.evaluate_pin("agent_in").await?;
        let model: CachedEmbeddingModel = context.evaluate_pin("model").await?;

        // The LanceDB table name encodes both the node ID and the model key so
        // that swapping the embedding model automatically uses a fresh table
        // (old embeddings are silently abandoned – no explicit purge needed).
        let db_cache_key = format!("lazy_fn_db_{}", model.cache_key);

        // ── Open / reuse the LanceDB connection ──────────────────────────────
        if !context.cache.read().await.contains_key(&db_cache_key) {
            let context_cache = context
                .execution_cache
                .clone()
                .ok_or_else(|| flow_like_types::anyhow!("No execution cache found"))?;

            let db_builder = if let Some(credentials) = &context.credentials {
                credentials.to_db(&context_cache.app_id).await?
            } else {
                let board_dir = context_cache.get_storage(false)?;
                let agents_dir = board_dir.child(".agents");
                context
                    .app_state
                    .config
                    .read()
                    .await
                    .callbacks
                    .build_project_database
                    .clone()
                    .ok_or_else(|| flow_like_types::anyhow!("No database builder found"))?(
                    agents_dir,
                )
            };

            let connection = db_builder.execute().await?;
            let mut lance_db =
                LanceDBVectorStore::from_connection(connection, model.cache_key.clone()).await;

            if let Some(opts) = &context
                .app_state
                .config
                .read()
                .await
                .callbacks
                .lance_write_options
            {
                lance_db.set_write_options(opts.clone());
            }

            let cached: Arc<dyn Cacheable> = Arc::new(CachedLazyToolDB {
                db: Arc::new(RwLock::new(lance_db)),
            });
            context
                .cache
                .write()
                .await
                .insert(db_cache_key.clone(), cached);
        }

        // ── Resolve the cached DB ─────────────────────────────────────────────
        let db_arc = {
            let cache = context.cache.read().await;
            let entry = cache
                .get(&db_cache_key)
                .ok_or_else(|| flow_like_types::anyhow!("DB not found in cache after init"))?;
            entry
                .as_any()
                .downcast_ref::<CachedLazyToolDB>()
                .ok_or_else(|| flow_like_types::anyhow!("Failed to downcast CachedLazyToolDB"))?
                .db
                .clone()
        };

        // ── Resolve the embedding model ───────────────────────────────────────
        let text_model = {
            let cache = context.cache.read().await;
            let entry = cache
                .get(&model.cache_key)
                .ok_or_else(|| flow_like_types::anyhow!("Embedding model not in cache"))?;
            entry
                .as_any()
                .downcast_ref::<CachedEmbeddingModelObject>()
                .ok_or_else(|| {
                    flow_like_types::anyhow!("Failed to downcast CachedEmbeddingModelObject")
                })?
                .text_model
                .clone()
                .ok_or_else(|| flow_like_types::anyhow!("No text embedding model available"))?
        };

        // ── Sync referenced functions into the DB ────────────────────────────
        let referenced_functions = context.get_referenced_functions().await?;
        let mut current_node_ids: std::collections::HashSet<String> =
            std::collections::HashSet::new();

        for referenced_node in &referenced_functions {
            let node_guard = referenced_node.node.lock().await;
            let node_id = node_guard.id.clone();

            // Canvas placement and graph wiring do not affect the indexed text.
            let node_hash = node_guard.semantic_hash().to_string();
            current_node_ids.insert(node_id.clone());

            // Build the searchable text blob (used for both FTS and embedding).
            let content = {
                let mut parts = vec![
                    node_guard.friendly_name.clone(),
                    node_guard.description.clone(),
                ];
                let mut sorted_pins: Vec<_> = node_guard.pins.values().collect();
                sorted_pins.sort_by_key(|p| (p.pin_type.clone() as u8, p.index));
                for pin in sorted_pins {
                    if pin.data_type == VT::Execution {
                        continue;
                    }
                    let prefix = match pin.pin_type {
                        PinType::Input => "Input",
                        PinType::Output => "Output",
                    };
                    if !pin.description.is_empty() {
                        parts.push(format!(
                            "{} {}: {}",
                            prefix, pin.friendly_name, pin.description
                        ));
                    } else {
                        parts.push(format!("{} {}", prefix, pin.friendly_name));
                    }
                }
                parts.join(". ")
            };

            drop(node_guard);

            // Check whether this node already has an up-to-date record.
            let existing = {
                let db = db_arc.read().await;
                db.filter(&format!("node_id = '{}'", node_id), None, 1, 0)
                    .await
                    .unwrap_or_default()
            };

            if !existing.is_empty() {
                let stored_hash = existing[0]
                    .get("hash")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                if stored_hash == node_hash {
                    // Nothing changed – skip re-embedding.
                    continue;
                }
                // Hash changed: remove the stale record before inserting the new one.
                let db = db_arc.read().await;
                let _ = db.delete(&format!("node_id = '{}'", node_id)).await;
            }

            // Embed the content string (document mode for indexing).
            let embeddings = text_model
                .text_embed_document(&vec![content.clone()])
                .await?;
            if embeddings.is_empty() {
                continue;
            }
            // Store as f64 to match the VectorStore search API.
            let vector: Vec<f64> = embeddings[0].iter().map(|&v| v as f64).collect();

            let record = json::json!({
                "node_id": node_id,
                "hash": node_hash,
                "content": content,
                "vector": vector,
            });

            let mut db = db_arc.write().await;
            db.insert(vec![record]).await?;
        }

        // ── Remove entries for functions no longer referenced ─────────────────
        {
            let existing_entries = db_arc
                .read()
                .await
                .list(Some(vec!["node_id".to_string()]), 10_000, 0)
                .await
                .unwrap_or_default();

            let stale: Vec<String> = existing_entries
                .iter()
                .filter_map(|e| {
                    e.get("node_id")
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string())
                })
                .filter(|id| !current_node_ids.contains(id))
                .collect();

            if !stale.is_empty() {
                let db = db_arc.read().await;
                for node_id in stale {
                    let _ = db.delete(&format!("node_id = '{}'", node_id)).await;
                }
            }
        }

        // ── Build / refresh indexes ───────────────────────────────────────────
        // FTS is always useful; ignore errors (e.g. table still empty on first run).
        let _ = db_arc
            .read()
            .await
            .index("content", Some("FULL TEXT"))
            .await;

        // Only build the vector index once we have enough rows for it to help.
        let row_count = db_arc.read().await.count(None).await.unwrap_or(0);
        if row_count >= VECTOR_INDEX_THRESHOLD {
            let _ = db_arc.read().await.index("vector", None).await;
        }

        // ── Register the lazy ref and shared embedding model on the agent ─────
        agent.set_lazy_embedding_model(model.clone());
        agent.add_lazy_function_ref(LazyFunctionRef {
            db_cache_key: db_cache_key.clone(),
        });

        context
            .set_pin_value("agent_out", json::json!(agent))
            .await?;
        context.activate_exec_pin("exec_out").await?;

        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "LLM processing requires the 'execute' feature"
        ))
    }
}
