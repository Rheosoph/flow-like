use crate::data::db::vector::NodeDBConnection;
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic, NodeScores},
    variable::VariableType,
};
use flow_like_storage::datafusion::common::TableReference;
#[cfg(feature = "federation")]
use flow_like_storage::datafusion::execution::session_state::SessionStateBuilder;
use flow_like_storage::datafusion::prelude::{SessionConfig, SessionContext};
#[cfg(feature = "federation")]
use flow_like_storage::datafusion_federation::{FederatedQueryPlanner, default_optimizer_rules};
use flow_like_storage::num_cpus;
use flow_like_types::{Cacheable, JsonSchema, async_trait, json::json, sync::Mutex};
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, sync::Arc};

#[derive(Default, Serialize, Deserialize, JsonSchema, Clone)]
pub struct DataFusionSession {
    pub cache_key: String,
}

/// A table registration whose expensive work — file downloads, parsing, schema
/// inference, connection setup — is postponed until a node actually queries the engine.
///
/// Mount nodes enqueue one of these via [`CachedDataFusionSession::defer_mount`] instead
/// of registering immediately; [`DataFusionSession::load`] applies the queue before any
/// consumer touches `ctx`. A cached query that never loads the session therefore never
/// pays for the mounts either.
#[async_trait]
pub trait DeferredMount: Send + Sync {
    /// Short human-readable description used in logs and error messages, e.g.
    /// "Excel workbook 'sales.xlsx'".
    fn describe(&self) -> String;

    /// Identity used to collapse repeats: deferring a mount evicts any queued mount
    /// with the same key (a mount node re-run in a loop replaces its earlier self
    /// instead of growing the queue and colliding at registration time). `None` never
    /// dedupes.
    fn dedupe_key(&self) -> Option<String> {
        None
    }

    /// Perform the registration. A failed materialization leaves the mount queued for
    /// the next consumer, so implementations must tolerate running again — deregister
    /// the target table name before registering it.
    async fn mount(
        &self,
        session: &CachedDataFusionSession,
        context: &mut ExecutionContext,
    ) -> flow_like_types::Result<()>;
}

#[derive(Clone)]
pub struct CachedDataFusionSession {
    pub ctx: Arc<SessionContext>,
    lance_tables: Arc<Mutex<HashMap<String, LanceTableRegistration>>>,
    pending_mounts: Arc<Mutex<Vec<Arc<dyn DeferredMount>>>>,
}

#[derive(Clone)]
struct LanceTableRegistration {
    database: NodeDBConnection,
    generation: u64,
}

impl Cacheable for CachedDataFusionSession {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}

impl DataFusionSession {
    /// Load the session and make it fully queryable: queued deferred mounts are applied
    /// and Lance registrations refreshed. Every node that reads from `ctx` must use
    /// this.
    pub async fn load(
        &self,
        context: &mut ExecutionContext,
    ) -> flow_like_types::Result<CachedDataFusionSession> {
        let session = self.load_lazy(context).await?;
        session.materialize(context).await?;
        session.refresh_lance_tables(context).await?;
        Ok(session)
    }

    /// Load the session handle without applying queued mounts. For mount nodes only:
    /// they add work to the session and must not force earlier deferred mounts to run.
    pub async fn load_lazy(
        &self,
        context: &ExecutionContext,
    ) -> flow_like_types::Result<CachedDataFusionSession> {
        let cached = context
            .cache
            .read()
            .await
            .get(self.cache_key.as_str())
            .cloned()
            .ok_or(flow_like_types::anyhow!(
                "DataFusion session not found in cache"
            ))?;
        let session = cached
            .as_any()
            .downcast_ref::<CachedDataFusionSession>()
            .ok_or(flow_like_types::anyhow!(
                "Could not downcast to DataFusion session"
            ))?;
        Ok(session.clone())
    }
}

impl CachedDataFusionSession {
    /// Queue a mount to run when the session is next materialized by a consumer. A
    /// queued mount with the same dedupe key is replaced, not duplicated.
    pub async fn defer_mount(&self, mount: Arc<dyn DeferredMount>) {
        let mut pending = self.pending_mounts.lock().await;
        if let Some(key) = mount.dedupe_key() {
            pending.retain(|queued| queued.dedupe_key().as_deref() != Some(key.as_str()));
        }
        pending.push(mount);
    }

    /// Apply queued mounts in the order they were deferred.
    ///
    /// The queue lock is held across the whole drain: two consumers materializing
    /// concurrently must not both run the same mount. A mount leaves the queue only
    /// once it has succeeded — a failed (or cancelled) one stays queued and is retried
    /// by the next consumer, so a transient error cannot silently cost the session a
    /// table. The failure itself propagates with the mount's description attached.
    pub async fn materialize(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let mut pending = self.pending_mounts.lock().await;
        while let Some(mount) = pending.first().cloned() {
            mount.mount(self, context).await.map_err(|err| {
                flow_like_types::anyhow!("Deferred mount of {} failed: {err}", mount.describe())
            })?;
            pending.remove(0);
        }
        Ok(())
    }

    /// Remembers a Lance registration so derived DataFusion providers can be
    /// replaced when a remote database rotates its scoped credentials.
    pub async fn track_lance_table(
        &self,
        table_name: String,
        database: NodeDBConnection,
        generation: u64,
    ) {
        self.lance_tables.lock().await.insert(
            table_name,
            LanceTableRegistration {
                database,
                generation,
            },
        );
    }

    async fn refresh_lance_tables(
        &self,
        context: &mut ExecutionContext,
    ) -> flow_like_types::Result<()> {
        let mut registrations = self.lance_tables.lock().await;
        for (table_name, registration) in registrations.iter_mut() {
            let (cached_db, generation) =
                registration.database.load_with_generation(context).await?;
            if generation == registration.generation {
                continue;
            }

            cached_db.ensure_flushed().await?;
            let db_guard = cached_db.db.read().await;
            let adapter = db_guard.inner().to_datafusion().await?;
            drop(db_guard);
            // The catalog rejects registering an existing name, so the stale provider
            // must be dropped before the rotated one can take its place.
            self.ctx
                .deregister_table(TableReference::bare(table_name.clone()))?;
            self.ctx
                .register_table(TableReference::bare(table_name.clone()), adapter)?;
            registration.generation = generation;
        }
        Ok(())
    }
}

fn build_session_config(
    target_partitions: i64,
    batch_size: i64,
    repartition_joins: bool,
    repartition_aggregations: bool,
    repartition_sorts: bool,
    coalesce_batches: bool,
    parquet_pruning: bool,
    collect_statistics: bool,
) -> SessionConfig {
    let target_partitions = if target_partitions <= 0 {
        num_cpus::get()
    } else {
        target_partitions as usize
    };

    let batch_size = batch_size.max(1) as usize;

    SessionConfig::new()
        .with_target_partitions(target_partitions)
        .with_batch_size(batch_size)
        .with_repartition_joins(repartition_joins)
        .with_repartition_aggregations(repartition_aggregations)
        .with_repartition_sorts(repartition_sorts)
        .with_coalesce_batches(coalesce_batches)
        .with_collect_statistics(collect_statistics)
        .with_parquet_pruning(parquet_pruning)
        .with_parquet_bloom_filter_pruning(parquet_pruning)
        .with_parquet_page_index_pruning(parquet_pruning)
}

#[cfg(feature = "federation")]
fn create_session_context(config: SessionConfig) -> SessionContext {
    let state = SessionStateBuilder::new()
        .with_config(config)
        .with_optimizer_rules(default_optimizer_rules())
        .with_query_planner(Arc::new(FederatedQueryPlanner::new()))
        .with_default_features()
        .build();

    SessionContext::new_with_state(state)
}

#[cfg(not(feature = "federation"))]
fn create_session_context(config: SessionConfig) -> SessionContext {
    SessionContext::new_with_config(config)
}

#[crate::register_node]
#[derive(Default)]
pub struct CreateDataFusionSessionNode {}

impl CreateDataFusionSessionNode {
    pub fn new() -> Self {
        CreateDataFusionSessionNode {}
    }
}

#[async_trait]
impl NodeLogic for CreateDataFusionSessionNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "df_create_session",
            "Create DataFusion Session",
            "Creates a new DataFusion session for SQL analytics. Configure optimization settings for production workloads.",
            "Data/DataFusion",
        );
        node.add_icon("/flow/icons/database.svg");

        node.add_input_pin(
            "exec_in",
            "Input",
            "Trigger execution",
            VariableType::Execution,
        );

        node.add_input_pin(
            "session_name",
            "Session Name",
            "Unique name for this session (used for caching)",
            VariableType::String,
        )
        .set_default_value(Some(json!("default")));

        node.add_input_pin(
            "target_partitions",
            "Target Partitions",
            "Number of partitions for parallel query execution. Higher values increase parallelism but add overhead. 0 = auto (uses CPU count).",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(0)));

        node.add_input_pin(
            "batch_size",
            "Batch Size",
            "Number of rows processed per batch. Larger batches improve throughput but use more memory.",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(8192)));

        node.add_input_pin(
            "repartition_joins",
            "Repartition Joins",
            "Enable automatic repartitioning before joins for better parallelism",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(true)));

        node.add_input_pin(
            "repartition_aggregations",
            "Repartition Aggregations",
            "Enable automatic repartitioning before aggregations",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(true)));

        node.add_input_pin(
            "repartition_sorts",
            "Repartition Sorts",
            "Enable automatic repartitioning for parallel sorting",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(true)));

        node.add_input_pin(
            "coalesce_batches",
            "Coalesce Batches",
            "Combine small batches into larger ones to reduce overhead",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(true)));

        node.add_input_pin(
            "parquet_pruning",
            "Parquet Pruning",
            "Enable predicate pushdown and column pruning for Parquet files",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(true)));

        node.add_input_pin(
            "collect_statistics",
            "Collect Statistics",
            "Collect statistics from data sources for query optimization",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(true)));

        node.add_output_pin(
            "exec_out",
            "Done",
            "Session created successfully",
            VariableType::Execution,
        );

        node.add_output_pin(
            "session",
            "Session",
            "DataFusion session reference for use with other DataFusion nodes",
            VariableType::Struct,
        )
        .set_schema::<DataFusionSession>();

        node.scores = Some(NodeScores {
            privacy: 10,
            security: 10,
            performance: 9,
            governance: 9,
            reliability: 9,
            cost: 10,
        });

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let session_name: String = context.evaluate_pin("session_name").await?;
        let cache_key = format!("df_session_{}", session_name);

        let cache_exists = context.cache.read().await.contains_key(&cache_key);
        if !cache_exists {
            let target_partitions: i64 = context.evaluate_pin("target_partitions").await?;
            let batch_size: i64 = context.evaluate_pin("batch_size").await?;
            let repartition_joins: bool = context.evaluate_pin("repartition_joins").await?;
            let repartition_aggregations: bool =
                context.evaluate_pin("repartition_aggregations").await?;
            let repartition_sorts: bool = context.evaluate_pin("repartition_sorts").await?;
            let coalesce_batches: bool = context.evaluate_pin("coalesce_batches").await?;
            let parquet_pruning: bool = context.evaluate_pin("parquet_pruning").await?;
            let collect_statistics: bool = context.evaluate_pin("collect_statistics").await?;

            let config = build_session_config(
                target_partitions,
                batch_size,
                repartition_joins,
                repartition_aggregations,
                repartition_sorts,
                coalesce_batches,
                parquet_pruning,
                collect_statistics,
            );

            let ctx = create_session_context(config);

            let cached = CachedDataFusionSession {
                ctx: Arc::new(ctx),
                lance_tables: Arc::new(Mutex::new(HashMap::new())),
                pending_mounts: Arc::new(Mutex::new(Vec::new())),
            };
            let cacheable: Arc<dyn Cacheable> = Arc::new(cached);
            context
                .cache
                .write()
                .await
                .insert(cache_key.clone(), cacheable);
        }

        let session = DataFusionSession { cache_key };
        context.set_pin_value("session", json!(session)).await?;
        context.activate_exec_pin("exec_out").await?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use flow_like::flow::pin::PinType;
    use flow_like::flow::variable::VariableType;
    use flow_like_types::json::to_value;
    #[cfg(feature = "sqlite-federation")]
    use std::{sync::Arc, time::Duration};

    #[cfg(feature = "sqlite-federation")]
    use datafusion_table_providers::{
        sql::{
            db_connection_pool::{DbConnectionPool, Mode, sqlitepool::SqliteConnectionPoolFactory},
            sql_provider_datafusion::SqlTable,
        },
        sqlite::DynSqliteConnectionPool,
    };

    #[test]
    fn test_datafusion_session_serialization() {
        let session = DataFusionSession {
            cache_key: "test_cache_key".to_string(),
        };

        let serialized = to_value(&session).unwrap();
        assert_eq!(serialized["cache_key"], "test_cache_key");
    }

    #[test]
    fn test_datafusion_session_default() {
        let session = DataFusionSession::default();
        assert!(session.cache_key.is_empty());
    }

    #[test]
    fn test_create_datafusion_session_node_structure() {
        let node_logic = CreateDataFusionSessionNode::new();
        let node = node_logic.get_node();

        assert_eq!(node.name, "df_create_session");
        assert_eq!(node.friendly_name, "Create DataFusion Session");
        assert_eq!(node.category, "Data/DataFusion");
    }

    #[test]
    fn test_create_datafusion_session_node_input_pins() {
        let node_logic = CreateDataFusionSessionNode::new();
        let node = node_logic.get_node();

        let input_pins: Vec<_> = node
            .pins
            .values()
            .filter(|p| p.pin_type == PinType::Input)
            .collect();

        let exec_pin = input_pins.iter().find(|p| p.name == "exec_in");
        assert!(exec_pin.is_some());
        assert_eq!(exec_pin.unwrap().data_type, VariableType::Execution);

        let session_name_pin = input_pins.iter().find(|p| p.name == "session_name");
        assert!(session_name_pin.is_some());
        assert_eq!(session_name_pin.unwrap().data_type, VariableType::String);
        assert!(session_name_pin.unwrap().default_value.is_some());

        let partitions_pin = input_pins.iter().find(|p| p.name == "target_partitions");
        assert!(partitions_pin.is_some());
        assert_eq!(partitions_pin.unwrap().data_type, VariableType::Integer);

        let batch_size_pin = input_pins.iter().find(|p| p.name == "batch_size");
        assert!(batch_size_pin.is_some());
        assert_eq!(batch_size_pin.unwrap().data_type, VariableType::Integer);

        let boolean_pins = [
            "repartition_joins",
            "repartition_aggregations",
            "repartition_sorts",
            "coalesce_batches",
            "parquet_pruning",
            "collect_statistics",
        ];
        for pin_name in boolean_pins {
            let pin = input_pins.iter().find(|p| p.name == pin_name);
            assert!(pin.is_some(), "Missing pin: {}", pin_name);
            assert_eq!(pin.unwrap().data_type, VariableType::Boolean);
        }
    }

    #[test]
    fn test_create_datafusion_session_node_output_pins() {
        let node_logic = CreateDataFusionSessionNode::new();
        let node = node_logic.get_node();

        let output_pins: Vec<_> = node
            .pins
            .values()
            .filter(|p| p.pin_type == PinType::Output)
            .collect();

        let exec_out = output_pins.iter().find(|p| p.name == "exec_out");
        assert!(exec_out.is_some());
        assert_eq!(exec_out.unwrap().data_type, VariableType::Execution);

        let session_pin = output_pins.iter().find(|p| p.name == "session");
        assert!(session_pin.is_some());
        assert_eq!(session_pin.unwrap().data_type, VariableType::Struct);
    }

    #[test]
    fn test_create_datafusion_session_node_has_scores() {
        let node_logic = CreateDataFusionSessionNode::new();
        let node = node_logic.get_node();

        assert!(node.scores.is_some());
        let scores = node.scores.unwrap();
        assert!(scores.privacy > 0);
        assert!(scores.security > 0);
        assert!(scores.performance > 0);
    }

    #[test]
    fn test_build_session_config_disables_parquet_pruning() {
        let config = build_session_config(4, 1024, true, true, true, true, false, true);

        assert!(!config.parquet_pruning());
        assert!(!config.parquet_bloom_filter_pruning());
        assert!(!config.parquet_page_index_pruning());
    }

    #[cfg(feature = "federation")]
    #[test]
    fn test_create_session_context_uses_federated_query_planner() {
        let ctx = create_session_context(SessionConfig::new());
        let planner = format!("{:?}", ctx.state().query_planner());

        assert!(
            planner.contains("FederatedQueryPlanner"),
            "unexpected planner: {planner}"
        );
    }

    #[cfg(feature = "sqlite-federation")]
    #[tokio::test]
    async fn test_create_session_context_pushes_down_sqlite_queries() {
        let pool = SqliteConnectionPoolFactory::new(
            ":memory:",
            Mode::Memory,
            Duration::from_millis(5_000),
        )
        .build()
        .await
        .unwrap();

        let conn = pool.connect().await.unwrap();
        let conn = conn.as_async().unwrap();
        conn.execute(
            "CREATE TABLE customers (id INTEGER PRIMARY KEY, name TEXT, region TEXT)",
            &[],
        )
        .await
        .unwrap();
        conn.execute(
            "INSERT INTO customers (id, name, region) VALUES
             (1, 'Acme Corp', 'West'),
             (2, 'TechStart', 'East'),
             (3, 'Global Inc', 'West')",
            &[],
        )
        .await
        .unwrap();

        let sqltable_pool: Arc<DynSqliteConnectionPool> = Arc::new(pool);
        let table = Arc::new(
            SqlTable::new("sqlite", &sqltable_pool, "customers")
                .await
                .unwrap(),
        );
        let table_provider = table.create_federated_table_provider().unwrap();

        let ctx = create_session_context(SessionConfig::new());
        ctx.register_table("customers", Arc::new(table_provider))
            .unwrap();

        let query = "SELECT name FROM customers WHERE region = 'West' ORDER BY name";
        let optimized_plan = ctx.sql(query).await.unwrap().into_optimized_plan().unwrap();
        let optimized_plan_text = format!("{optimized_plan:?}");

        assert!(
            optimized_plan_text.contains("Federated"),
            "unexpected optimized plan: {optimized_plan_text}"
        );

        let batches = ctx.sql(query).await.unwrap().collect().await.unwrap();
        let total_rows: usize = batches.iter().map(|batch| batch.num_rows()).sum();

        assert_eq!(total_rows, 2);
    }
}
