use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic, NodeScores},
    variable::VariableType,
};
use flow_like_storage::datafusion::prelude::{SessionConfig, SessionContext};
use flow_like_storage::num_cpus;
use flow_like_types::{Cacheable, JsonSchema, async_trait, json::json};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

#[derive(Default, Serialize, Deserialize, JsonSchema, Clone)]
pub struct DataFusionSession {
    pub cache_key: String,
}

#[derive(Clone)]
pub struct CachedDataFusionSession {
    pub ctx: Arc<SessionContext>,
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
    pub async fn load(
        &self,
        context: &mut ExecutionContext,
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

            // Note: Federation support (query push-down to external databases) requires
            // datafusion-federation 0.4.7+ which needs DataFusion 50+.
            // When upgrading to DataFusion 50+, enable federation feature and use:
            // let rules = datafusion_federation::default_optimizer_rules();
            // let state = SessionStateBuilder::new()
            //     .with_config(config)
            //     .with_optimizer_rules(rules)
            //     .with_query_planner(Arc::new(FederatedQueryPlanner::new()))
            //     .with_default_features()
            //     .build();
            let ctx = SessionContext::new_with_config(config);

            let cached = CachedDataFusionSession { ctx: Arc::new(ctx) };
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
}
