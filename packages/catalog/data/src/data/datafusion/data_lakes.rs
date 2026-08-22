use crate::data::datafusion::session::DataFusionSession;
use crate::data::path::FlowPath;
use crate::data::query_params as params;
use flow_like::flow::{
    board::Board,
    execution::context::ExecutionContext,
    node::{Node, NodeLogic, NodeScores},
    pin::ValueType,
    variable::VariableType,
};
use flow_like_types::{Value as JsonValue, async_trait, json::json, reqwest::Url};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

fn build_store_url(store_ref: &str, path: &str) -> String {
    format!("flowlike://{}/{}", store_ref, path.trim_start_matches('/'))
}

#[cfg(feature = "delta")]
fn delta_table_provider(
    table: &flow_like_storage::deltalake::DeltaTable,
) -> flow_like_types::Result<
    std::sync::Arc<dyn flow_like_storage::datafusion::datasource::TableProvider>,
> {
    use flow_like_storage::deltalake::delta_datafusion::{
        DeltaScanConfigBuilder, DeltaTableProvider,
    };

    let snapshot = table
        .snapshot()
        .map_err(|e| flow_like_types::anyhow!("Failed to get Delta snapshot: {}", e))?;
    let scan_config = DeltaScanConfigBuilder::new()
        .build(snapshot.snapshot())
        .map_err(|e| flow_like_types::anyhow!("Failed to build Delta scan config: {}", e))?;

    Ok(std::sync::Arc::new(
        DeltaTableProvider::try_new(snapshot.snapshot().clone(), table.log_store(), scan_config)
            .map_err(|e| flow_like_types::anyhow!("Failed to create Delta provider: {}", e))?,
    ))
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct DeltaSchemaField {
    pub name: String,
    pub field_type: String,
    pub nullable: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct DeltaSchemaInfo {
    pub fields: Vec<DeltaSchemaField>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct DeltaHistoryEntry {
    pub timestamp: Option<String>,
    pub operation: Option<String>,
}

// ============================================================================
// Delta Lake Support
// ============================================================================

/// Register a Delta Lake table in DataFusion using FlowPath
/// A snapshot entry of an Iceberg table, as written to the `snapshots` pin.
#[derive(serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
pub struct IcebergSnapshot {
    pub snapshot_id: i64,
    /// When the snapshot was committed, in milliseconds since the epoch.
    pub timestamp_ms: i64,
    /// The snapshot this one was built on, absent for the first.
    pub parent_snapshot_id: Option<i64>,
}

/// The partitioning an Iceberg table is currently written with.
#[derive(serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
pub struct IcebergPartitionSpec {
    pub spec_id: i32,
    /// How many columns take part in the partitioning.
    pub fields_count: usize,
}

#[crate::register_node]
#[derive(Default)]
pub struct RegisterDeltaTableNode {}

impl RegisterDeltaTableNode {
    pub fn new() -> Self {
        RegisterDeltaTableNode {}
    }
}

#[async_trait]
impl NodeLogic for RegisterDeltaTableNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "df_register_delta",
            "Register Delta Table",
            "Register a Delta Lake table in DataFusion using a FlowPath. Requires the 'delta' feature.",
            "Data/DataFusion/Lakes",
        );
        node.add_icon("/flow/icons/database.svg");

        node.add_input_pin(
            "exec_in",
            "Input",
            "Trigger execution",
            VariableType::Execution,
        );

        node.add_input_pin(
            "session",
            "Session",
            "DataFusion session",
            VariableType::Struct,
        )
        .set_schema::<DataFusionSession>();

        node.add_input_pin(
            "path",
            "Path",
            "FlowPath to the Delta table directory",
            VariableType::Struct,
        )
        .set_schema::<FlowPath>();

        node.add_input_pin(
            "table_name",
            "Table Name",
            "Name to register in DataFusion",
            VariableType::String,
        );

        node.add_input_pin(
            "version",
            "Version",
            "Specific version to load (-1 for latest)",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(-1)));

        node.add_output_pin(
            "exec_out",
            "Done",
            "Table registered",
            VariableType::Execution,
        );

        node.add_output_pin(
            "session_out",
            "Session",
            "DataFusion session",
            VariableType::Struct,
        )
        .set_schema::<DataFusionSession>();

        node.add_output_pin(
            "table_version",
            "Table Version",
            "Actual version loaded",
            VariableType::Integer,
        );

        node.scores = Some(NodeScores {
            privacy: 8,
            security: 8,
            performance: 9,
            governance: 9,
            reliability: 9,
            cost: 9,
        });

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        #[cfg(feature = "delta")]
        {
            use flow_like_storage::deltalake::DeltaTableBuilder;

            context.deactivate_exec_pin("exec_out").await?;

            let session: DataFusionSession = context.evaluate_pin("session").await?;
            let path: FlowPath = context.evaluate_pin("path").await?;
            let table_name: String = context.evaluate_pin("table_name").await?;
            let version: i64 = context.evaluate_pin("version").await.unwrap_or(-1);

            let cached_session = session.load_lazy(context).await?;
            let store = path.to_store(context).await?;
            let object_store = store.as_generic();

            let url_str = build_store_url(&path.store_ref, &path.path);
            let url = Url::parse(&url_str)?;

            let mut builder =
                DeltaTableBuilder::from_url(url.clone())?.with_storage_backend(object_store, url);

            if version >= 0 {
                builder = builder.with_version(version);
            }

            let delta_table = builder
                .load()
                .await
                .map_err(|e| flow_like_types::anyhow!("Failed to open Delta table: {}", e))?;

            let actual_version = delta_table.version().unwrap_or(0);
            let table_provider = delta_table_provider(&delta_table)?;

            cached_session
                .ctx
                .register_table(&table_name, table_provider)?;

            context.set_pin_value("session_out", json!(session)).await?;
            context
                .set_pin_value("table_version", json!(actual_version))
                .await?;
            context.activate_exec_pin("exec_out").await?;
            Ok(())
        }

        #[cfg(not(feature = "delta"))]
        {
            Err(flow_like_types::anyhow!(
                "Delta Lake support not enabled. Rebuild with the 'delta' feature flag."
            ))
        }
    }
}

/// Time travel to a specific Delta table version using FlowPath
#[crate::register_node]
#[derive(Default)]
pub struct DeltaTimeTravelNode {}

impl DeltaTimeTravelNode {
    pub fn new() -> Self {
        DeltaTimeTravelNode {}
    }
}

#[async_trait]
impl NodeLogic for DeltaTimeTravelNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "df_delta_time_travel",
            "Delta Time Travel",
            "Load a specific version or timestamp of a Delta table for point-in-time queries.",
            "Data/DataFusion/Lakes",
        );
        node.add_icon("/flow/icons/clock.svg");

        node.add_input_pin(
            "exec_in",
            "Input",
            "Trigger execution",
            VariableType::Execution,
        );

        node.add_input_pin(
            "session",
            "Session",
            "DataFusion session",
            VariableType::Struct,
        )
        .set_schema::<DataFusionSession>();

        node.add_input_pin(
            "path",
            "Path",
            "FlowPath to the Delta table directory",
            VariableType::Struct,
        )
        .set_schema::<FlowPath>();

        node.add_input_pin(
            "table_name",
            "Table Name",
            "Name to register in DataFusion",
            VariableType::String,
        );

        node.add_input_pin(
            "travel_mode",
            "Travel Mode",
            "Mode: 'version' or 'timestamp'",
            VariableType::String,
        )
        .set_default_value(Some(json!("version")));

        node.add_input_pin(
            "version",
            "Version",
            "Version number (when mode is 'version')",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(0)));

        node.add_input_pin(
            "timestamp",
            "Timestamp",
            "ISO 8601 timestamp (when mode is 'timestamp')",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_output_pin(
            "exec_out",
            "Done",
            "Table registered",
            VariableType::Execution,
        );

        node.add_output_pin(
            "session_out",
            "Session",
            "DataFusion session",
            VariableType::Struct,
        )
        .set_schema::<DataFusionSession>();

        node.add_output_pin(
            "loaded_version",
            "Loaded Version",
            "Actual version loaded",
            VariableType::Integer,
        );

        node.scores = Some(NodeScores {
            privacy: 8,
            security: 8,
            performance: 8,
            governance: 10,
            reliability: 9,
            cost: 8,
        });

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        #[cfg(feature = "delta")]
        {
            use flow_like_storage::deltalake::DeltaTableBuilder;

            context.deactivate_exec_pin("exec_out").await?;

            let session: DataFusionSession = context.evaluate_pin("session").await?;
            let path: FlowPath = context.evaluate_pin("path").await?;
            let table_name: String = context.evaluate_pin("table_name").await?;
            let travel_mode: String = context.evaluate_pin("travel_mode").await?;
            let version: i64 = context.evaluate_pin("version").await.unwrap_or(0);
            let timestamp: String = context.evaluate_pin("timestamp").await.unwrap_or_default();

            let cached_session = session.load_lazy(context).await?;
            let store = path.to_store(context).await?;
            let object_store = store.as_generic();

            let url_str = build_store_url(&path.store_ref, &path.path);
            let url = Url::parse(&url_str)?;

            let builder = match travel_mode.to_lowercase().as_str() {
                "version" => DeltaTableBuilder::from_url(url.clone())?
                    .with_storage_backend(object_store, url)
                    .with_version(version),
                "timestamp" => {
                    let dt = chrono::DateTime::parse_from_rfc3339(&timestamp)
                        .map_err(|e| flow_like_types::anyhow!("Invalid timestamp format: {}", e))?;
                    DeltaTableBuilder::from_url(url.clone())?
                        .with_storage_backend(object_store, url)
                        .with_timestamp(dt.to_utc())
                }
                _ => {
                    return Err(flow_like_types::anyhow!(
                        "Invalid travel mode: {}. Use 'version' or 'timestamp'",
                        travel_mode
                    ));
                }
            };

            let delta_table = builder
                .load()
                .await
                .map_err(|e| flow_like_types::anyhow!("Failed to load Delta table: {}", e))?;

            let loaded_version = delta_table.version().unwrap_or(0);
            let table_provider = delta_table_provider(&delta_table)?;
            cached_session
                .ctx
                .register_table(&table_name, table_provider)?;

            context.set_pin_value("session_out", json!(session)).await?;
            context
                .set_pin_value("loaded_version", json!(loaded_version))
                .await?;
            context.activate_exec_pin("exec_out").await?;
            Ok(())
        }

        #[cfg(not(feature = "delta"))]
        {
            Err(flow_like_types::anyhow!(
                "Delta Lake support not enabled. Rebuild with the 'delta' feature flag."
            ))
        }
    }
}

/// Get Delta table history and metadata using FlowPath
#[crate::register_node]
#[derive(Default)]
pub struct DeltaTableInfoNode {}

impl DeltaTableInfoNode {
    pub fn new() -> Self {
        DeltaTableInfoNode {}
    }
}

#[async_trait]
impl NodeLogic for DeltaTableInfoNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "df_delta_info",
            "Delta Table Info",
            "Get metadata and history information about a Delta table.",
            "Data/DataFusion/Lakes",
        );
        node.add_icon("/flow/icons/info.svg");
        node.set_version(1);

        node.add_input_pin(
            "exec_in",
            "Input",
            "Trigger execution",
            VariableType::Execution,
        );

        node.add_input_pin(
            "path",
            "Path",
            "FlowPath to the Delta table directory",
            VariableType::Struct,
        )
        .set_schema::<FlowPath>();

        node.add_input_pin(
            "history_limit",
            "History Limit",
            "Max number of history entries to return",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(10)));

        node.add_output_pin(
            "exec_out",
            "Done",
            "Info retrieved",
            VariableType::Execution,
        );

        node.add_output_pin(
            "current_version",
            "Current Version",
            "Latest version number",
            VariableType::Integer,
        );

        node.add_output_pin(
            "schema",
            "Schema",
            "Table schema as typed field metadata",
            VariableType::Struct,
        )
        .set_schema::<DeltaSchemaInfo>();

        node.add_output_pin(
            "history",
            "History",
            "Version history as array of typed entries",
            VariableType::Struct,
        )
        .set_schema::<DeltaHistoryEntry>()
        .set_value_type(ValueType::Array);

        node.add_output_pin(
            "partitions",
            "Partitions",
            "Partition columns",
            VariableType::String,
        )
        .set_value_type(ValueType::Array);

        node.scores = Some(NodeScores {
            privacy: 10,
            security: 10,
            performance: 10,
            governance: 10,
            reliability: 10,
            cost: 10,
        });

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        #[cfg(feature = "delta")]
        {
            use flow_like_storage::deltalake::DeltaTableBuilder;

            context.deactivate_exec_pin("exec_out").await?;

            let path: FlowPath = context.evaluate_pin("path").await?;
            let history_limit: i64 = context.evaluate_pin("history_limit").await.unwrap_or(10);

            let store = path.to_store(context).await?;
            let object_store = store.as_generic();

            let url_str = build_store_url(&path.store_ref, &path.path);
            let url = Url::parse(&url_str)?;

            let delta_table = DeltaTableBuilder::from_url(url.clone())?
                .with_storage_backend(object_store, url)
                .load()
                .await
                .map_err(|e| flow_like_types::anyhow!("Failed to open Delta table: {}", e))?;

            let current_version = delta_table.version().unwrap_or(0);

            let snapshot = delta_table
                .snapshot()
                .map_err(|e| flow_like_types::anyhow!("Failed to get snapshot: {}", e))?;

            let schema_info = {
                let schema = snapshot.schema();
                let fields: Vec<DeltaSchemaField> = schema
                    .fields()
                    .map(|f| DeltaSchemaField {
                        name: f.name().to_string(),
                        field_type: format!("{:?}", f.data_type()),
                        nullable: f.is_nullable(),
                    })
                    .collect();
                DeltaSchemaInfo { fields }
            };

            let partitions: Vec<String> = snapshot.metadata().partition_columns().to_vec();

            let history: Vec<DeltaHistoryEntry> = delta_table
                .history(Some(history_limit as usize))
                .await
                .map(|h| {
                    h.map(|entry| DeltaHistoryEntry {
                        timestamp: entry.timestamp.map(|ts| ts.to_string()),
                        operation: entry.operation,
                    })
                    .collect()
                })
                .unwrap_or_default();

            context
                .set_pin_value("current_version", json!(current_version))
                .await?;
            context.set_pin_value("schema", json!(schema_info)).await?;
            context.set_pin_value("history", json!(history)).await?;
            context
                .set_pin_value("partitions", json!(partitions))
                .await?;
            context.activate_exec_pin("exec_out").await?;
            Ok(())
        }

        #[cfg(not(feature = "delta"))]
        {
            Err(flow_like_types::anyhow!(
                "Delta Lake support not enabled. Rebuild with the 'delta' feature flag."
            ))
        }
    }
}

// ============================================================================
// Register Parquet with Hive Partitioning (works without extra deps)
// ============================================================================

/// Register partitioned Parquet files with Hive-style partitioning using FlowPath
#[crate::register_node]
#[derive(Default)]
pub struct RegisterHivePartitionedParquetNode {}

impl RegisterHivePartitionedParquetNode {
    pub fn new() -> Self {
        RegisterHivePartitionedParquetNode {}
    }
}

#[async_trait]
impl NodeLogic for RegisterHivePartitionedParquetNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "df_register_hive_parquet",
            "Register Hive Parquet",
            "Register Hive-partitioned Parquet files as a table in DataFusion using a FlowPath.",
            "Data/DataFusion/Lakes",
        );
        node.add_icon("/flow/icons/database.svg");

        node.add_input_pin(
            "exec_in",
            "Input",
            "Trigger execution",
            VariableType::Execution,
        );

        node.add_input_pin(
            "session",
            "Session",
            "DataFusion session",
            VariableType::Struct,
        )
        .set_schema::<DataFusionSession>();

        node.add_input_pin(
            "path",
            "Path",
            "FlowPath to root directory of partitioned Parquet files",
            VariableType::Struct,
        )
        .set_schema::<FlowPath>();

        node.add_input_pin(
            "table_name",
            "Table Name",
            "Name to register in DataFusion",
            VariableType::String,
        );

        node.add_output_pin(
            "exec_out",
            "Done",
            "Table registered",
            VariableType::Execution,
        );

        node.add_output_pin(
            "session_out",
            "Session",
            "DataFusion session",
            VariableType::Struct,
        )
        .set_schema::<DataFusionSession>();

        node.scores = Some(NodeScores {
            privacy: 9,
            security: 9,
            performance: 9,
            governance: 8,
            reliability: 9,
            cost: 10,
        });

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        use flow_like_storage::datafusion::datasource::file_format::parquet::ParquetFormat;
        use flow_like_storage::datafusion::datasource::listing::{
            ListingOptions, ListingTable, ListingTableConfig, ListingTableUrl,
        };
        use std::sync::Arc;

        context.deactivate_exec_pin("exec_out").await?;

        let session: DataFusionSession = context.evaluate_pin("session").await?;
        let path: FlowPath = context.evaluate_pin("path").await?;
        let table_name: String = context.evaluate_pin("table_name").await?;

        let cached_session = session.load_lazy(context).await?;
        let store = path.to_store(context).await?;
        let object_store = store.as_generic();

        let url_str = build_store_url(&path.store_ref, &path.path);
        let url = Url::parse(&url_str)?;
        let table_path = ListingTableUrl::parse(&url_str)?;

        cached_session.ctx.register_object_store(&url, object_store);

        let parquet_format = ParquetFormat::default();
        let listing_options = ListingOptions::new(Arc::new(parquet_format))
            .with_file_extension(".parquet")
            .with_table_partition_cols(vec![])
            .with_collect_stat(true);

        let config = ListingTableConfig::new(table_path)
            .with_listing_options(listing_options)
            .infer_schema(&cached_session.ctx.state())
            .await?;

        let listing_table = ListingTable::try_new(config)?;
        cached_session
            .ctx
            .register_table(&table_name, Arc::new(listing_table))?;

        context.set_pin_value("session_out", json!(session)).await?;
        context.activate_exec_pin("exec_out").await?;
        Ok(())
    }
}

/// Register partitioned JSON files as a table using FlowPath
#[crate::register_node]
#[derive(Default)]
pub struct RegisterPartitionedJsonNode {}

impl RegisterPartitionedJsonNode {
    pub fn new() -> Self {
        RegisterPartitionedJsonNode {}
    }
}

#[async_trait]
impl NodeLogic for RegisterPartitionedJsonNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "df_register_partitioned_json",
            "Register Partitioned JSON",
            "Register partitioned JSON/NDJSON files as a table in DataFusion using a FlowPath.",
            "Data/DataFusion/Lakes",
        );
        node.add_icon("/flow/icons/database.svg");

        node.add_input_pin(
            "exec_in",
            "Input",
            "Trigger execution",
            VariableType::Execution,
        );

        node.add_input_pin(
            "session",
            "Session",
            "DataFusion session",
            VariableType::Struct,
        )
        .set_schema::<DataFusionSession>();

        node.add_input_pin(
            "path",
            "Path",
            "FlowPath to JSON files",
            VariableType::Struct,
        )
        .set_schema::<FlowPath>();

        node.add_input_pin(
            "table_name",
            "Table Name",
            "Name to register",
            VariableType::String,
        );

        node.add_input_pin(
            "file_extension",
            "File Extension",
            "File extension to match",
            VariableType::String,
        )
        .set_default_value(Some(json!(".json")));

        node.add_output_pin(
            "exec_out",
            "Done",
            "Table registered",
            VariableType::Execution,
        );

        node.add_output_pin(
            "session_out",
            "Session",
            "DataFusion session",
            VariableType::Struct,
        )
        .set_schema::<DataFusionSession>();

        node.scores = Some(NodeScores {
            privacy: 9,
            security: 9,
            performance: 7,
            governance: 8,
            reliability: 9,
            cost: 10,
        });

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        use flow_like_storage::datafusion::datasource::file_format::json::JsonFormat;
        use flow_like_storage::datafusion::datasource::listing::{
            ListingOptions, ListingTable, ListingTableConfig, ListingTableUrl,
        };
        use std::sync::Arc;

        context.deactivate_exec_pin("exec_out").await?;

        let session: DataFusionSession = context.evaluate_pin("session").await?;
        let path: FlowPath = context.evaluate_pin("path").await?;
        let table_name: String = context.evaluate_pin("table_name").await?;
        let file_extension: String = context
            .evaluate_pin("file_extension")
            .await
            .unwrap_or_else(|_| ".json".to_string());

        let cached_session = session.load_lazy(context).await?;
        let store = path.to_store(context).await?;
        let object_store = store.as_generic();

        let url_str = build_store_url(&path.store_ref, &path.path);
        let url = Url::parse(&url_str)?;
        let table_path = ListingTableUrl::parse(&url_str)?;

        cached_session.ctx.register_object_store(&url, object_store);

        let json_format = JsonFormat::default();
        let listing_options = ListingOptions::new(Arc::new(json_format))
            .with_file_extension(&file_extension)
            .with_collect_stat(true);

        let config = ListingTableConfig::new(table_path)
            .with_listing_options(listing_options)
            .infer_schema(&cached_session.ctx.state())
            .await?;

        let listing_table = ListingTable::try_new(config)?;
        cached_session
            .ctx
            .register_table(&table_name, Arc::new(listing_table))?;

        context.set_pin_value("session_out", json!(session)).await?;
        context.activate_exec_pin("exec_out").await?;
        Ok(())
    }
}

// ============================================================================
// Write Delta Lake Table
// ============================================================================

/// Write query results to a Delta Lake table using FlowPath
#[crate::register_node]
#[derive(Default)]
pub struct WriteDeltaTableNode {}

impl WriteDeltaTableNode {
    pub fn new() -> Self {
        WriteDeltaTableNode {}
    }
}

#[async_trait]
impl NodeLogic for WriteDeltaTableNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "df_write_delta",
            "Write Delta Table",
            "Write query results to a new or existing Delta Lake table using FlowPath. Supports append, overwrite modes.",
            "Data/DataFusion/Lakes",
        );
        node.add_icon("/flow/icons/database.svg");

        node.add_input_pin(
            "exec_in",
            "Input",
            "Trigger execution",
            VariableType::Execution,
        );

        node.add_input_pin(
            "session",
            "Session",
            "DataFusion session",
            VariableType::Struct,
        )
        .set_schema::<DataFusionSession>();

        node.add_input_pin(
            "query",
            "Query",
            "SQL query to execute. Use $placeholders for values that come from the flow (SELECT * FROM events WHERE day = $day) — each one adds an input pin to wire the value into. Placeholders stand for values only; table and column names cannot be parameterized.",
            VariableType::String,
        );

        params::add_params_pin(&mut node, params::SqlFlavor::Query);

        node.add_input_pin(
            "path",
            "Path",
            "FlowPath for the Delta table directory",
            VariableType::Struct,
        )
        .set_schema::<FlowPath>();

        node.add_input_pin(
            "mode",
            "Mode",
            "Write mode: append, overwrite, error, ignore",
            VariableType::String,
        )
        .set_default_value(Some(json!("append")));

        node.add_input_pin(
            "partition_by",
            "Partition By",
            "Columns to partition by (comma-separated)",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_output_pin(
            "exec_out",
            "Done",
            "Write completed",
            VariableType::Execution,
        );

        node.add_output_pin(
            "session_out",
            "Session",
            "DataFusion session",
            VariableType::Struct,
        )
        .set_schema::<DataFusionSession>();

        node.add_output_pin(
            "rows_written",
            "Rows Written",
            "Number of rows written",
            VariableType::Integer,
        );

        node.add_output_pin(
            "new_version",
            "New Version",
            "Version number after write",
            VariableType::Integer,
        );

        node.scores = Some(NodeScores {
            privacy: 7,
            security: 7,
            performance: 8,
            governance: 9,
            reliability: 9,
            cost: 8,
        });

        node
    }

    async fn on_update(&self, node: &mut Node, board: &Board) {
        node.error = None;
        params::sync_param_pins(node, "query", board, params::SqlFlavor::Query);
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        #[cfg(feature = "delta")]
        {
            use flow_like_storage::deltalake::protocol::SaveMode;
            use flow_like_storage::deltalake::{DeltaTable, DeltaTableBuilder};

            context.deactivate_exec_pin("exec_out").await?;

            let session: DataFusionSession = context.evaluate_pin("session").await?;
            let query: String = context.evaluate_pin("query").await?;
            let query_params =
                params::resolve_params(context, &query, params::SqlFlavor::Query).await?;
            let path: FlowPath = context.evaluate_pin("path").await?;
            let mode: String = context
                .evaluate_pin("mode")
                .await
                .unwrap_or_else(|_| "append".to_string());
            let partition_by_str: String = context
                .evaluate_pin("partition_by")
                .await
                .unwrap_or_default();

            // df_write_delta consumes the session (it runs the query), so deferred
            // mounts must be materialized first — full load, not load_lazy.
            let cached_session = session.load(context).await?;

            let df = cached_session.ctx.sql(&query).await?;
            let df = params::bind(df, &query_params)?;
            let batches = df.collect().await?;

            if batches.is_empty() {
                context.set_pin_value("session_out", json!(session)).await?;
                context.set_pin_value("rows_written", json!(0)).await?;
                context.set_pin_value("new_version", json!(0)).await?;
                context.activate_exec_pin("exec_out").await?;
                return Ok(());
            }

            let total_rows: i64 = batches.iter().map(|b| b.num_rows() as i64).sum();

            let store = path.to_store(context).await?;
            let object_store = store.as_generic();

            let url_str = build_store_url(&path.store_ref, &path.path);
            let url = Url::parse(&url_str)?;

            let partition_cols: Vec<String> = partition_by_str
                .split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect();

            let write_mode = match mode.to_lowercase().as_str() {
                "append" => SaveMode::Append,
                "overwrite" => SaveMode::Overwrite,
                "error" | "errorifexists" => SaveMode::ErrorIfExists,
                "ignore" => SaveMode::Ignore,
                _ => return Err(flow_like_types::anyhow!("Invalid write mode: {}", mode)),
            };

            let table = match DeltaTableBuilder::from_url(url.clone())?
                .with_storage_backend(object_store.clone(), url.clone())
                .load()
                .await
            {
                Ok(table) => table,
                Err(_) => DeltaTableBuilder::from_url(url.clone())?
                    .with_storage_backend(object_store.clone(), url)
                    .build()
                    .map_err(|e| flow_like_types::anyhow!("Failed to create Delta table: {}", e))?,
            };

            let mut write_builder = table.write(batches).with_save_mode(write_mode);

            if !partition_cols.is_empty() {
                write_builder = write_builder.with_partition_columns(partition_cols);
            }

            let table: DeltaTable = write_builder
                .await
                .map_err(|e| flow_like_types::anyhow!("Failed to write to Delta table: {}", e))?;

            let new_version = table.version().unwrap_or(0);

            context.set_pin_value("session_out", json!(session)).await?;
            context
                .set_pin_value("rows_written", json!(total_rows))
                .await?;
            context
                .set_pin_value("new_version", json!(new_version))
                .await?;
            context.activate_exec_pin("exec_out").await?;
            Ok(())
        }

        #[cfg(not(feature = "delta"))]
        {
            Err(flow_like_types::anyhow!(
                "Delta Lake support not enabled. Rebuild with the 'delta' feature flag."
            ))
        }
    }
}

// ============================================================================
// Apache Iceberg Support
// ============================================================================

/// Register an Apache Iceberg table in DataFusion from a metadata file
#[crate::register_node]
#[derive(Default)]
pub struct RegisterIcebergTableNode {}

impl RegisterIcebergTableNode {
    pub fn new() -> Self {
        RegisterIcebergTableNode {}
    }
}

#[async_trait]
impl NodeLogic for RegisterIcebergTableNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "df_register_iceberg",
            "Register Iceberg Table",
            "Register an Apache Iceberg table in DataFusion from a metadata file. Supports schema evolution and partition pruning.",
            "Data/DataFusion/Lakes",
        );
        node.add_icon("/flow/icons/database.svg");

        node.add_input_pin(
            "exec_in",
            "Input",
            "Trigger execution",
            VariableType::Execution,
        );
        node.add_input_pin(
            "session",
            "Session",
            "DataFusion session",
            VariableType::Struct,
        )
        .set_schema::<DataFusionSession>();
        node.add_input_pin(
            "warehouse_path",
            "Warehouse Path",
            "FlowPath to the Iceberg table metadata directory",
            VariableType::Struct,
        )
        .set_schema::<FlowPath>();
        node.add_input_pin(
            "metadata_file",
            "Metadata File",
            "Relative path to metadata JSON file (e.g., 'metadata/v1.metadata.json')",
            VariableType::String,
        );
        node.add_input_pin(
            "table_name",
            "DataFusion Name",
            "Name to register in DataFusion",
            VariableType::String,
        );

        node.add_output_pin(
            "exec_out",
            "Done",
            "Table registered",
            VariableType::Execution,
        );
        node.add_output_pin(
            "session_out",
            "Session",
            "DataFusion session",
            VariableType::Struct,
        )
        .set_schema::<DataFusionSession>();
        node.add_output_pin(
            "current_snapshot",
            "Current Snapshot",
            "Current snapshot ID",
            VariableType::String,
        );
        node.add_output_pin(
            "schema_info",
            "Schema Info",
            "Table schema field count",
            VariableType::Integer,
        );

        node.scores = Some(NodeScores {
            privacy: 8,
            security: 8,
            performance: 9,
            governance: 10,
            reliability: 9,
            cost: 8,
        });
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        #[cfg(feature = "iceberg")]
        {
            use flow_like_storage::iceberg::TableIdent;
            use flow_like_storage::iceberg::io::FileIO;
            use flow_like_storage::iceberg::table::StaticTable;
            use flow_like_storage::iceberg_datafusion::table::IcebergStaticTableProvider;
            use std::sync::Arc;

            let session: DataFusionSession = context.evaluate_pin("session").await?;
            let warehouse_path: FlowPath = context.evaluate_pin("warehouse_path").await?;
            let metadata_file: String = context.evaluate_pin("metadata_file").await?;
            let table_name: String = context.evaluate_pin("table_name").await?;

            let cached_session = session.load_lazy(context).await?;
            let store = warehouse_path.to_store(context).await?;

            let url_str = build_store_url(&warehouse_path.store_ref, &warehouse_path.path);
            let url = Url::parse(&url_str)?;
            cached_session
                .ctx
                .register_object_store(&url, store.as_generic());

            let metadata_location = format!("{}/{}", warehouse_path.path, metadata_file);

            let file_io = FileIO::new_with_fs();

            let table_ident = TableIdent::from_strs(vec![&table_name])?;

            let static_table =
                StaticTable::from_metadata_file(&metadata_location, table_ident, file_io)
                    .await
                    .map_err(|e| flow_like_types::anyhow!("Failed to load Iceberg table: {}", e))?;

            let metadata = static_table.metadata();
            let current_snapshot = metadata
                .current_snapshot()
                .map(|s| s.snapshot_id().to_string())
                .unwrap_or_else(|| "none".to_string());

            let schema_fields = metadata.current_schema().as_struct().fields().len() as i64;

            let iceberg_table = static_table.into_table();
            let table_provider = IcebergStaticTableProvider::try_new_from_table(iceberg_table)
                .await
                .map_err(|e| flow_like_types::anyhow!("Failed to create table provider: {}", e))?;

            cached_session
                .ctx
                .register_table(&table_name, Arc::new(table_provider))?;

            context.set_pin_value("session_out", json!(session)).await?;
            context
                .set_pin_value("current_snapshot", json!(current_snapshot))
                .await?;
            context
                .set_pin_value("schema_info", json!(schema_fields))
                .await?;
            context.activate_exec_pin("exec_out").await?;
            Ok(())
        }

        #[cfg(not(feature = "iceberg"))]
        {
            Err(flow_like_types::anyhow!(
                "Iceberg support not enabled. Rebuild with the 'iceberg' feature flag."
            ))
        }
    }
}

/// Get metadata and history of an Iceberg table from a metadata file
#[crate::register_node]
#[derive(Default)]
pub struct IcebergTableInfoNode {}

impl IcebergTableInfoNode {
    pub fn new() -> Self {
        IcebergTableInfoNode {}
    }
}

#[async_trait]
impl NodeLogic for IcebergTableInfoNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "df_iceberg_info",
            "Iceberg Table Info",
            "Get metadata, snapshots, and history of an Apache Iceberg table from a metadata file.",
            "Data/DataFusion/Lakes",
        );
        node.add_icon("/flow/icons/info.svg");

        node.add_input_pin(
            "exec_in",
            "Input",
            "Trigger execution",
            VariableType::Execution,
        );
        node.add_input_pin(
            "warehouse_path",
            "Warehouse Path",
            "FlowPath to the Iceberg table directory",
            VariableType::Struct,
        )
        .set_schema::<FlowPath>();
        node.add_input_pin(
            "metadata_file",
            "Metadata File",
            "Relative path to metadata JSON file",
            VariableType::String,
        );

        node.add_output_pin(
            "exec_out",
            "Done",
            "Info retrieved",
            VariableType::Execution,
        );
        node.add_output_pin(
            "current_snapshot",
            "Current Snapshot",
            "Current snapshot ID",
            VariableType::String,
        );
        node.add_output_pin(
            "schema",
            "Schema",
            "Table schema as JSON",
            VariableType::Struct,
        )
        .set_open_schema();
        node.add_output_pin(
            "snapshots",
            "Snapshots",
            "List of all snapshots",
            VariableType::Struct,
        )
        .set_schema::<crate::data::datafusion::data_lakes::IcebergSnapshot>();
        node.add_output_pin(
            "partition_spec",
            "Partition Spec",
            "Current partition specification",
            VariableType::Struct,
        )
        .set_schema::<crate::data::datafusion::data_lakes::IcebergPartitionSpec>();
        node.add_output_pin(
            "properties",
            "Properties",
            "Table properties",
            VariableType::Struct,
        )
        .set_open_schema();

        node.scores = Some(NodeScores {
            privacy: 9,
            security: 9,
            performance: 10,
            governance: 10,
            reliability: 10,
            cost: 10,
        });
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        #[cfg(feature = "iceberg")]
        {
            use flow_like_storage::iceberg::TableIdent;
            use flow_like_storage::iceberg::io::FileIO;
            use flow_like_storage::iceberg::table::StaticTable;

            let warehouse_path: FlowPath = context.evaluate_pin("warehouse_path").await?;
            let metadata_file: String = context.evaluate_pin("metadata_file").await?;

            let metadata_location = format!("{}/{}", warehouse_path.path, metadata_file);

            let file_io = FileIO::new_with_fs();

            let table_ident = TableIdent::from_strs(vec!["info_table"])?;

            let static_table =
                StaticTable::from_metadata_file(&metadata_location, table_ident, file_io)
                    .await
                    .map_err(|e| flow_like_types::anyhow!("Failed to load Iceberg table: {}", e))?;

            let metadata = static_table.metadata();

            let current_snapshot = metadata
                .current_snapshot()
                .map(|s| s.snapshot_id().to_string())
                .unwrap_or_else(|| "none".to_string());

            let schema_json = json!({
                "schema_id": metadata.current_schema().schema_id(),
                "fields": metadata.current_schema().as_struct().fields().iter().map(|f| json!({
                    "id": f.id,
                    "name": f.name.clone(),
                    "required": f.required,
                })).collect::<Vec<_>>()
            });

            let snapshots: Vec<JsonValue> = metadata
                .snapshots()
                .map(|s| {
                    json!({
                        "snapshot_id": s.snapshot_id(),
                        "timestamp_ms": s.timestamp_ms(),
                        "parent_snapshot_id": s.parent_snapshot_id(),
                    })
                })
                .collect();

            let partition_spec = json!({
                "spec_id": metadata.default_partition_spec().spec_id(),
                "fields_count": metadata.default_partition_spec().fields().len()
            });

            let properties: std::collections::HashMap<&String, &String> =
                metadata.properties().iter().collect();

            context
                .set_pin_value("current_snapshot", json!(current_snapshot))
                .await?;
            context.set_pin_value("schema", schema_json).await?;
            context.set_pin_value("snapshots", json!(snapshots)).await?;
            context
                .set_pin_value("partition_spec", partition_spec)
                .await?;
            context
                .set_pin_value("properties", json!(properties))
                .await?;
            context.activate_exec_pin("exec_out").await?;
            Ok(())
        }

        #[cfg(not(feature = "iceberg"))]
        {
            Err(flow_like_types::anyhow!(
                "Iceberg support not enabled. Rebuild with the 'iceberg' feature flag."
            ))
        }
    }
}

/// Time travel to a specific Iceberg snapshot
#[crate::register_node]
#[derive(Default)]
pub struct IcebergTimeTravelNode {}

impl IcebergTimeTravelNode {
    pub fn new() -> Self {
        IcebergTimeTravelNode {}
    }
}

#[async_trait]
impl NodeLogic for IcebergTimeTravelNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "df_iceberg_time_travel",
            "Iceberg Time Travel",
            "Load a specific snapshot of an Iceberg table for point-in-time queries.",
            "Data/DataFusion/Lakes",
        );
        node.add_icon("/flow/icons/clock.svg");

        node.add_input_pin(
            "exec_in",
            "Input",
            "Trigger execution",
            VariableType::Execution,
        );
        node.add_input_pin(
            "session",
            "Session",
            "DataFusion session",
            VariableType::Struct,
        )
        .set_schema::<DataFusionSession>();
        node.add_input_pin(
            "warehouse_path",
            "Warehouse Path",
            "FlowPath to the Iceberg table directory",
            VariableType::Struct,
        )
        .set_schema::<FlowPath>();
        node.add_input_pin(
            "metadata_file",
            "Metadata File",
            "Relative path to metadata JSON file",
            VariableType::String,
        );
        node.add_input_pin(
            "table_name",
            "DataFusion Name",
            "Name to register in DataFusion",
            VariableType::String,
        );
        node.add_input_pin(
            "travel_mode",
            "Travel Mode",
            "Mode: 'snapshot' or 'timestamp'",
            VariableType::String,
        )
        .set_default_value(Some(json!("snapshot")));
        node.add_input_pin(
            "snapshot_id",
            "Snapshot ID",
            "Snapshot ID (when mode is 'snapshot')",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));
        node.add_input_pin(
            "timestamp_ms",
            "Timestamp",
            "Unix timestamp in milliseconds (when mode is 'timestamp')",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(0)));

        node.add_output_pin(
            "exec_out",
            "Done",
            "Table registered",
            VariableType::Execution,
        );
        node.add_output_pin(
            "session_out",
            "Session",
            "DataFusion session",
            VariableType::Struct,
        )
        .set_schema::<DataFusionSession>();
        node.add_output_pin(
            "loaded_snapshot",
            "Loaded Snapshot",
            "Actual snapshot ID that was loaded",
            VariableType::String,
        );

        node.scores = Some(NodeScores {
            privacy: 8,
            security: 8,
            performance: 8,
            governance: 10,
            reliability: 9,
            cost: 8,
        });
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        #[cfg(feature = "iceberg")]
        {
            use flow_like_storage::iceberg::TableIdent;
            use flow_like_storage::iceberg::io::FileIO;
            use flow_like_storage::iceberg::table::StaticTable;
            use flow_like_storage::iceberg_datafusion::table::IcebergStaticTableProvider;
            use std::sync::Arc;

            let session: DataFusionSession = context.evaluate_pin("session").await?;
            let warehouse_path: FlowPath = context.evaluate_pin("warehouse_path").await?;
            let metadata_file: String = context.evaluate_pin("metadata_file").await?;
            let table_name: String = context.evaluate_pin("table_name").await?;
            let travel_mode: String = context
                .evaluate_pin("travel_mode")
                .await
                .unwrap_or_else(|_| "snapshot".to_string());
            let snapshot_id_str: String = context
                .evaluate_pin("snapshot_id")
                .await
                .unwrap_or_default();
            let timestamp_ms: i64 = context.evaluate_pin("timestamp_ms").await.unwrap_or(0);

            let cached_session = session.load_lazy(context).await?;
            let store = warehouse_path.to_store(context).await?;

            let url_str = build_store_url(&warehouse_path.store_ref, &warehouse_path.path);
            let url = Url::parse(&url_str)?;
            cached_session
                .ctx
                .register_object_store(&url, store.as_generic());

            let metadata_location = format!("{}/{}", warehouse_path.path, metadata_file);

            let file_io = FileIO::new_with_fs();

            let table_ident = TableIdent::from_strs(vec![&table_name])?;

            let static_table =
                StaticTable::from_metadata_file(&metadata_location, table_ident, file_io)
                    .await
                    .map_err(|e| flow_like_types::anyhow!("Failed to load Iceberg table: {}", e))?;

            let metadata = static_table.metadata();

            let target_snapshot_id = match travel_mode.as_str() {
                "timestamp" => {
                    let snapshot = metadata
                        .snapshots()
                        .filter(|s| s.timestamp_ms() <= timestamp_ms)
                        .max_by_key(|s| s.timestamp_ms())
                        .ok_or_else(|| {
                            flow_like_types::anyhow!(
                                "No snapshot found at or before timestamp {}",
                                timestamp_ms
                            )
                        })?;
                    snapshot.snapshot_id()
                }
                _ => {
                    if snapshot_id_str.is_empty() {
                        metadata
                            .current_snapshot()
                            .map(|s| s.snapshot_id())
                            .ok_or_else(|| {
                                flow_like_types::anyhow!("No current snapshot available")
                            })?
                    } else {
                        snapshot_id_str
                            .parse()
                            .map_err(|e| flow_like_types::anyhow!("Invalid snapshot ID: {}", e))?
                    }
                }
            };

            let iceberg_table = static_table.into_table();
            let table_provider = IcebergStaticTableProvider::try_new_from_table_snapshot(
                iceberg_table,
                target_snapshot_id,
            )
            .await
            .map_err(|e| flow_like_types::anyhow!("Failed to create table provider: {}", e))?;

            cached_session
                .ctx
                .register_table(&table_name, Arc::new(table_provider))?;

            context.set_pin_value("session_out", json!(session)).await?;
            context
                .set_pin_value("loaded_snapshot", json!(target_snapshot_id.to_string()))
                .await?;
            context.activate_exec_pin("exec_out").await?;
            Ok(())
        }

        #[cfg(not(feature = "iceberg"))]
        {
            Err(flow_like_types::anyhow!(
                "Iceberg support not enabled. Rebuild with the 'iceberg' feature flag."
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use flow_like::flow::pin::PinType;
    use flow_like::flow::variable::VariableType;

    #[test]
    fn test_build_store_url_simple() {
        let url = build_store_url("delta_store", "tables/sales");
        assert_eq!(url, "flowlike://delta_store/tables/sales");
    }

    #[test]
    fn test_build_store_url_with_leading_slash() {
        let url = build_store_url("delta_store", "/tables/sales");
        assert_eq!(url, "flowlike://delta_store/tables/sales");
    }

    #[test]
    fn test_build_store_url_deep_path() {
        let url = build_store_url("s3_bucket", "data/warehouse/tables/customers/_delta_log");
        assert_eq!(
            url,
            "flowlike://s3_bucket/data/warehouse/tables/customers/_delta_log"
        );
    }

    #[test]
    fn test_register_delta_table_node_structure() {
        let node_logic = RegisterDeltaTableNode::new();
        let node = node_logic.get_node();

        assert_eq!(node.name, "df_register_delta");
        assert_eq!(node.friendly_name, "Register Delta Table");
        assert_eq!(node.category, "Data/DataFusion/Lakes");
    }

    #[test]
    fn test_register_delta_table_node_input_pins() {
        let node_logic = RegisterDeltaTableNode::new();
        let node = node_logic.get_node();

        let input_pins: Vec<_> = node
            .pins
            .values()
            .filter(|p| p.pin_type == PinType::Input)
            .collect();

        assert!(
            input_pins
                .iter()
                .any(|p| p.name == "exec_in" && p.data_type == VariableType::Execution)
        );
        assert!(
            input_pins
                .iter()
                .any(|p| p.name == "session" && p.data_type == VariableType::Struct)
        );
        assert!(
            input_pins
                .iter()
                .any(|p| p.name == "path" && p.data_type == VariableType::Struct)
        );
        assert!(
            input_pins
                .iter()
                .any(|p| p.name == "table_name" && p.data_type == VariableType::String)
        );
        assert!(
            input_pins
                .iter()
                .any(|p| p.name == "version" && p.data_type == VariableType::Integer)
        );
    }

    #[test]
    fn test_register_delta_table_node_output_pins() {
        let node_logic = RegisterDeltaTableNode::new();
        let node = node_logic.get_node();

        let output_pins: Vec<_> = node
            .pins
            .values()
            .filter(|p| p.pin_type == PinType::Output)
            .collect();

        assert!(output_pins.iter().any(|p| p.name == "exec_out"));
        assert!(output_pins.iter().any(|p| p.name == "session_out"));
        assert!(output_pins.iter().any(|p| p.name == "table_version"));
    }

    #[test]
    fn test_delta_time_travel_node_structure() {
        let node_logic = DeltaTimeTravelNode::new();
        let node = node_logic.get_node();

        assert_eq!(node.name, "df_delta_time_travel");
        assert_eq!(node.friendly_name, "Delta Time Travel");
    }

    #[test]
    fn test_delta_time_travel_node_travel_pins() {
        let node_logic = DeltaTimeTravelNode::new();
        let node = node_logic.get_node();

        let input_pins: Vec<_> = node
            .pins
            .values()
            .filter(|p| p.pin_type == PinType::Input)
            .collect();

        let travel_mode_pin = input_pins.iter().find(|p| p.name == "travel_mode");
        assert!(travel_mode_pin.is_some());
        assert_eq!(travel_mode_pin.unwrap().data_type, VariableType::String);

        let version_pin = input_pins.iter().find(|p| p.name == "version");
        assert!(version_pin.is_some());
        assert_eq!(version_pin.unwrap().data_type, VariableType::Integer);

        let timestamp_pin = input_pins.iter().find(|p| p.name == "timestamp");
        assert!(timestamp_pin.is_some());
        assert_eq!(timestamp_pin.unwrap().data_type, VariableType::String);
    }

    #[test]
    fn test_delta_table_info_node_structure() {
        let node_logic = DeltaTableInfoNode::new();
        let node = node_logic.get_node();

        assert_eq!(node.name, "df_delta_info");
        assert_eq!(node.friendly_name, "Delta Table Info");
    }

    #[test]
    fn test_delta_table_info_node_output_pins() {
        let node_logic = DeltaTableInfoNode::new();
        let node = node_logic.get_node();

        let output_pins: Vec<_> = node
            .pins
            .values()
            .filter(|p| p.pin_type == PinType::Output)
            .collect();

        assert!(output_pins.iter().any(|p| p.name == "current_version"));
        assert!(output_pins.iter().any(|p| p.name == "schema"));
        assert!(output_pins.iter().any(|p| p.name == "partitions"));
        assert!(output_pins.iter().any(|p| p.name == "history"));
    }

    #[test]
    fn test_register_hive_partitioned_parquet_node_structure() {
        let node_logic = RegisterHivePartitionedParquetNode::new();
        let node = node_logic.get_node();

        assert_eq!(node.name, "df_register_hive_parquet");
        assert_eq!(node.friendly_name, "Register Hive Parquet");
    }

    #[test]
    fn test_register_partitioned_json_node_structure() {
        let node_logic = RegisterPartitionedJsonNode::new();
        let node = node_logic.get_node();

        assert_eq!(node.name, "df_register_partitioned_json");
        assert_eq!(node.friendly_name, "Register Partitioned JSON");
    }

    #[test]
    fn test_write_delta_table_node_structure() {
        let node_logic = WriteDeltaTableNode::new();
        let node = node_logic.get_node();

        assert_eq!(node.name, "df_write_delta");
        assert_eq!(node.friendly_name, "Write Delta Table");
        assert_eq!(node.category, "Data/DataFusion/Lakes");
    }

    #[test]
    fn test_write_delta_table_node_input_pins() {
        let node_logic = WriteDeltaTableNode::new();
        let node = node_logic.get_node();

        let input_pins: Vec<_> = node
            .pins
            .values()
            .filter(|p| p.pin_type == PinType::Input)
            .collect();

        assert!(input_pins.iter().any(|p| p.name == "query"));
        assert!(input_pins.iter().any(|p| p.name == "path"));
        assert!(input_pins.iter().any(|p| p.name == "mode"));
        assert!(input_pins.iter().any(|p| p.name == "partition_by"));
    }

    #[test]
    fn test_write_delta_table_node_output_pins() {
        let node_logic = WriteDeltaTableNode::new();
        let node = node_logic.get_node();

        let output_pins: Vec<_> = node
            .pins
            .values()
            .filter(|p| p.pin_type == PinType::Output)
            .collect();

        assert!(output_pins.iter().any(|p| p.name == "rows_written"));
        assert!(output_pins.iter().any(|p| p.name == "new_version"));
    }

    #[test]
    fn test_all_delta_nodes_have_scores() {
        let register_node = RegisterDeltaTableNode::new().get_node();
        let time_travel_node = DeltaTimeTravelNode::new().get_node();
        let info_node = DeltaTableInfoNode::new().get_node();
        let write_node = WriteDeltaTableNode::new().get_node();
        let hive_node = RegisterHivePartitionedParquetNode::new().get_node();
        let json_node = RegisterPartitionedJsonNode::new().get_node();

        assert!(register_node.scores.is_some());
        assert!(time_travel_node.scores.is_some());
        assert!(info_node.scores.is_some());
        assert!(write_node.scores.is_some());
        assert!(hive_node.scores.is_some());
        assert!(json_node.scores.is_some());
    }

    #[test]
    fn test_all_delta_nodes_use_flowpath() {
        let nodes = vec![
            RegisterDeltaTableNode::new().get_node(),
            DeltaTimeTravelNode::new().get_node(),
            DeltaTableInfoNode::new().get_node(),
            WriteDeltaTableNode::new().get_node(),
            RegisterHivePartitionedParquetNode::new().get_node(),
            RegisterPartitionedJsonNode::new().get_node(),
        ];

        for node in nodes {
            let path_pin = node.pins.values().find(|p| p.name == "path");
            assert!(path_pin.is_some(), "Node {} missing path pin", node.name);
            assert_eq!(
                path_pin.unwrap().data_type,
                VariableType::Struct,
                "Node {} path pin should be Struct type",
                node.name
            );
        }
    }

    // ========================================================================
    // Iceberg Node Tests
    // ========================================================================

    #[test]
    fn test_register_iceberg_table_node_structure() {
        let node_logic = RegisterIcebergTableNode::new();
        let node = node_logic.get_node();

        assert_eq!(node.name, "df_register_iceberg");
        assert_eq!(node.friendly_name, "Register Iceberg Table");
        assert_eq!(node.category, "Data/DataFusion/Lakes");
    }

    #[test]
    fn test_register_iceberg_table_node_input_pins() {
        let node_logic = RegisterIcebergTableNode::new();
        let node = node_logic.get_node();

        let input_pins: Vec<_> = node
            .pins
            .values()
            .filter(|p| p.pin_type == PinType::Input)
            .collect();

        assert!(input_pins.iter().any(|p| p.name == "exec_in"));
        assert!(input_pins.iter().any(|p| p.name == "session"));
        assert!(input_pins.iter().any(|p| p.name == "warehouse_path"));
        assert!(input_pins.iter().any(|p| p.name == "metadata_file"));
        assert!(input_pins.iter().any(|p| p.name == "table_name"));
    }

    #[test]
    fn test_register_iceberg_table_node_output_pins() {
        let node_logic = RegisterIcebergTableNode::new();
        let node = node_logic.get_node();

        let output_pins: Vec<_> = node
            .pins
            .values()
            .filter(|p| p.pin_type == PinType::Output)
            .collect();

        assert!(output_pins.iter().any(|p| p.name == "exec_out"));
        assert!(output_pins.iter().any(|p| p.name == "session_out"));
        assert!(output_pins.iter().any(|p| p.name == "current_snapshot"));
        assert!(output_pins.iter().any(|p| p.name == "schema_info"));
    }

    #[test]
    fn test_iceberg_table_info_node_structure() {
        let node_logic = IcebergTableInfoNode::new();
        let node = node_logic.get_node();

        assert_eq!(node.name, "df_iceberg_info");
        assert_eq!(node.friendly_name, "Iceberg Table Info");
        assert_eq!(node.category, "Data/DataFusion/Lakes");
    }

    #[test]
    fn test_iceberg_table_info_node_output_pins() {
        let node_logic = IcebergTableInfoNode::new();
        let node = node_logic.get_node();

        let output_pins: Vec<_> = node
            .pins
            .values()
            .filter(|p| p.pin_type == PinType::Output)
            .collect();

        assert!(output_pins.iter().any(|p| p.name == "current_snapshot"));
        assert!(output_pins.iter().any(|p| p.name == "schema"));
        assert!(output_pins.iter().any(|p| p.name == "snapshots"));
        assert!(output_pins.iter().any(|p| p.name == "partition_spec"));
        assert!(output_pins.iter().any(|p| p.name == "properties"));
    }

    #[test]
    fn test_iceberg_time_travel_node_structure() {
        let node_logic = IcebergTimeTravelNode::new();
        let node = node_logic.get_node();

        assert_eq!(node.name, "df_iceberg_time_travel");
        assert_eq!(node.friendly_name, "Iceberg Time Travel");
        assert_eq!(node.category, "Data/DataFusion/Lakes");
    }

    #[test]
    fn test_iceberg_time_travel_node_travel_pins() {
        let node_logic = IcebergTimeTravelNode::new();
        let node = node_logic.get_node();

        let input_pins: Vec<_> = node
            .pins
            .values()
            .filter(|p| p.pin_type == PinType::Input)
            .collect();

        let travel_mode_pin = input_pins.iter().find(|p| p.name == "travel_mode");
        assert!(travel_mode_pin.is_some());
        assert_eq!(travel_mode_pin.unwrap().data_type, VariableType::String);

        let snapshot_id_pin = input_pins.iter().find(|p| p.name == "snapshot_id");
        assert!(snapshot_id_pin.is_some());
        assert_eq!(snapshot_id_pin.unwrap().data_type, VariableType::String);

        let timestamp_pin = input_pins.iter().find(|p| p.name == "timestamp_ms");
        assert!(timestamp_pin.is_some());
        assert_eq!(timestamp_pin.unwrap().data_type, VariableType::Integer);
    }

    #[test]
    fn test_all_iceberg_nodes_have_scores() {
        let register_node = RegisterIcebergTableNode::new().get_node();
        let info_node = IcebergTableInfoNode::new().get_node();
        let time_travel_node = IcebergTimeTravelNode::new().get_node();

        assert!(register_node.scores.is_some());
        assert!(info_node.scores.is_some());
        assert!(time_travel_node.scores.is_some());
    }

    #[test]
    fn test_all_iceberg_nodes_use_warehouse_path() {
        let nodes = vec![
            RegisterIcebergTableNode::new().get_node(),
            IcebergTableInfoNode::new().get_node(),
            IcebergTimeTravelNode::new().get_node(),
        ];

        for node in nodes {
            let path_pin = node.pins.values().find(|p| p.name == "warehouse_path");
            assert!(
                path_pin.is_some(),
                "Node {} missing warehouse_path pin",
                node.name
            );
            assert_eq!(
                path_pin.unwrap().data_type,
                VariableType::Struct,
                "Node {} warehouse_path pin should be Struct type",
                node.name
            );
        }
    }
}
