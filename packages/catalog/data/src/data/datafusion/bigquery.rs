use crate::data::datafusion::session::DataFusionSession;
use crate::data::providers::gcp::GcpProvider;
use crate::data::providers::util::get_pin_string_value;
use flow_like::flow::{
    board::Board,
    execution::context::ExecutionContext,
    node::{Node, NodeLogic, NodeScores, remove_pin_by_name},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_types::{JsonSchema, async_trait, json::json};
use serde::{Deserialize, Serialize};

#[allow(unused_imports)]
use std::sync::Arc;

const REG_TABLE: &str = "table";
const REG_QUERY: &str = "query";

#[derive(Default, Serialize, Deserialize, JsonSchema, Clone, Debug)]
pub struct BigQueryJobStats {
    pub job_id: Option<String>,
    pub location: Option<String>,
    pub bytes_processed: Option<i64>,
    pub total_rows: Option<i64>,
    pub cache_hit: Option<bool>,
}

#[crate::register_node]
#[derive(Default)]
pub struct RegisterBigQueryNode {}

impl RegisterBigQueryNode {
    pub fn new() -> Self {
        RegisterBigQueryNode {}
    }
}

#[async_trait]
impl NodeLogic for RegisterBigQueryNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "df_register_bigquery",
            "Register BigQuery",
            "Register a Google BigQuery table or query result into a DataFusion session. Takes a GcpProvider for authentication — pair it with the GCP Provider node.",
            "Data/DataFusion/Databases",
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
            "DataFusion session to register the table into",
            VariableType::Struct,
        )
        .set_schema::<DataFusionSession>();

        node.add_input_pin(
            "provider",
            "Provider",
            "GCP provider with authentication (from the GCP Provider node)",
            VariableType::Struct,
        )
        .set_schema::<GcpProvider>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_input_pin(
            "project_id",
            "Project ID",
            "GCP project ID for billing/job routing. Falls back to the provider's default_project_id when empty.",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        add_registration_mode_pin(&mut node);
        add_table_mode_pins(&mut node);

        node.add_input_pin(
            "table_name",
            "Table Name",
            "Name to register the result as in DataFusion",
            VariableType::String,
        );

        node.add_input_pin(
            "location",
            "Location",
            "BigQuery location for the job (e.g. 'US', 'EU', 'europe-west1')",
            VariableType::String,
        )
        .set_default_value(Some(json!("US")));

        node.add_input_pin(
            "page_size",
            "Page Size",
            "Max rows per page when paginating results. 0 lets BigQuery pick (10 MB cap).",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(0)));

        node.add_input_pin(
            "use_query_cache",
            "Use Query Cache",
            "Allow BigQuery to serve the result from its query cache when available",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(true)));

        node.add_input_pin(
            "max_bytes_billed",
            "Max Bytes Billed",
            "Cap on bytes billed for this query. 0 means use project default.",
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
            "registered_as",
            "Registered As",
            "Final table name registered in the DataFusion session",
            VariableType::String,
        );
        node.add_output_pin(
            "row_count",
            "Row Count",
            "Number of rows materialised into the DataFusion session",
            VariableType::Integer,
        );
        node.add_output_pin(
            "job_stats",
            "Job Stats",
            "BigQuery job statistics (job id, bytes processed, cache hit)",
            VariableType::Struct,
        )
        .set_schema::<BigQueryJobStats>();

        node.scores = Some(NodeScores {
            privacy: 5,
            security: 6,
            performance: 7,
            governance: 8,
            reliability: 8,
            cost: 6,
        });
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let session: DataFusionSession = context.evaluate_pin("session").await?;
        let provider: GcpProvider = context.evaluate_pin("provider").await?;
        let pin_project_id: String = context.evaluate_pin("project_id").await.unwrap_or_default();
        let registration_mode: String = context
            .evaluate_pin("registration_mode")
            .await
            .unwrap_or_else(|_| REG_TABLE.to_string());
        let dataset: String = context.evaluate_pin("dataset").await.unwrap_or_default();
        let source_table: String = context
            .evaluate_pin("source_table")
            .await
            .unwrap_or_default();
        let user_query: String = context.evaluate_pin("query").await.unwrap_or_default();
        let table_name: String = context.evaluate_pin("table_name").await?;
        let location: String = context
            .evaluate_pin("location")
            .await
            .unwrap_or_else(|_| "US".to_string());
        let row_limit: i64 = context.evaluate_pin("row_limit").await.unwrap_or(0);
        let page_size: i64 = context.evaluate_pin("page_size").await.unwrap_or(0);
        let use_query_cache: bool = context
            .evaluate_pin("use_query_cache")
            .await
            .unwrap_or(true);
        let max_bytes_billed: i64 = context.evaluate_pin("max_bytes_billed").await.unwrap_or(0);

        let project_id = if !pin_project_id.trim().is_empty() {
            pin_project_id
        } else {
            provider
                .default_project_id
                .clone()
                .unwrap_or_default()
                .trim()
                .to_string()
        };

        if project_id.is_empty() {
            return Err(flow_like_types::anyhow!(
                "BigQuery project_id is required (either on the node or as default_project_id on the provider)"
            ));
        }
        if table_name.trim().is_empty() {
            return Err(flow_like_types::anyhow!(
                "Target table_name (for DataFusion registration) is required"
            ));
        }

        let sql = build_sql(
            &registration_mode,
            &project_id,
            &dataset,
            &source_table,
            &user_query,
            row_limit,
        )?;

        let cached_session = session.load(context).await?;

        #[cfg(feature = "bigquery")]
        {
            let client = provider.build_bigquery_client(context).await?;

            let stats = run_bigquery_registration(
                &cached_session,
                &client,
                &project_id,
                &location,
                &sql,
                &table_name,
                page_size,
                use_query_cache,
                max_bytes_billed,
            )
            .await?;

            tracing::info!(
                "BigQuery result registered as '{}' (rows: {}, bytes_processed: {:?})",
                table_name,
                stats.total_rows.unwrap_or(0),
                stats.bytes_processed,
            );

            context.set_pin_value("session_out", json!(session)).await?;
            context
                .set_pin_value("registered_as", json!(table_name))
                .await?;
            context
                .set_pin_value("row_count", json!(stats.total_rows.unwrap_or(0)))
                .await?;
            context.set_pin_value("job_stats", json!(stats)).await?;
            context.activate_exec_pin("exec_out").await?;
            Ok(())
        }

        #[cfg(not(feature = "bigquery"))]
        {
            let _ = (
                provider,
                cached_session,
                sql,
                location,
                page_size,
                use_query_cache,
                max_bytes_billed,
                context,
            );
            Err(flow_like_types::anyhow!(
                "BigQuery support is not enabled. Rebuild with the 'bigquery' feature flag."
            ))
        }
    }

    async fn on_update(&self, node: &mut Node, _board: &Board) {
        let registration_mode = get_pin_string_value(node, "registration_mode");
        sync_registration_mode_pins(node, &registration_mode);
    }
}

fn build_sql(
    registration_mode: &str,
    project_id: &str,
    dataset: &str,
    source_table: &str,
    user_query: &str,
    row_limit: i64,
) -> flow_like_types::Result<String> {
    match registration_mode {
        REG_TABLE => {
            if dataset.trim().is_empty() || source_table.trim().is_empty() {
                return Err(flow_like_types::anyhow!(
                    "'table' mode requires both 'dataset' and 'source_table'"
                ));
            }
            let mut sql = format!(
                "SELECT * FROM `{}.{}.{}`",
                project_id.replace('`', ""),
                dataset.replace('`', ""),
                source_table.replace('`', "")
            );
            if row_limit > 0 {
                sql.push_str(&format!(" LIMIT {}", row_limit));
            }
            Ok(sql)
        }
        REG_QUERY => {
            if user_query.trim().is_empty() {
                return Err(flow_like_types::anyhow!(
                    "'query' mode requires a non-empty 'query'"
                ));
            }
            Ok(user_query.to_string())
        }
        other => Err(flow_like_types::anyhow!(
            "Unknown registration_mode: '{}'. Expected 'table' or 'query'.",
            other
        )),
    }
}

fn add_registration_mode_pin(node: &mut Node) {
    node.add_input_pin(
        "registration_mode",
        "Registration Mode",
        "How to select the data: 'table' (register a full BigQuery table) or 'query' (register the result of a Standard SQL query)",
        VariableType::String,
    )
    .set_options(
        PinOptions::new()
            .set_valid_values(vec![REG_TABLE.to_string(), REG_QUERY.to_string()])
            .build(),
    )
    .set_default_value(Some(json!(REG_TABLE)));
}

fn add_table_mode_pins(node: &mut Node) {
    node.add_input_pin(
        "dataset",
        "Dataset",
        "BigQuery dataset (only used when registration_mode is 'table')",
        VariableType::String,
    )
    .set_default_value(Some(json!("")));

    node.add_input_pin(
        "source_table",
        "Source Table",
        "BigQuery table name (only used when registration_mode is 'table')",
        VariableType::String,
    )
    .set_default_value(Some(json!("")));

    node.add_input_pin(
        "row_limit",
        "Row Limit",
        "Optional LIMIT applied in 'table' mode. 0 means no limit.",
        VariableType::Integer,
    )
    .set_default_value(Some(json!(0)));
}

fn add_query_mode_pin(node: &mut Node) {
    node.add_input_pin(
        "query",
        "Query",
        "Standard SQL query (only used when registration_mode is 'query')",
        VariableType::String,
    )
    .set_default_value(Some(json!("")));
}

fn sync_registration_mode_pins(node: &mut Node, registration_mode: &str) {
    let mode = if registration_mode.is_empty() {
        REG_TABLE
    } else {
        registration_mode
    };

    match mode {
        REG_QUERY => {
            remove_pin_by_name(node, "dataset");
            remove_pin_by_name(node, "source_table");
            remove_pin_by_name(node, "row_limit");
            if node.get_pin_by_name("query").is_none() {
                add_query_mode_pin(node);
            }
        }
        _ => {
            remove_pin_by_name(node, "query");
            if node.get_pin_by_name("dataset").is_none() {
                node.add_input_pin(
                    "dataset",
                    "Dataset",
                    "BigQuery dataset (only used when registration_mode is 'table')",
                    VariableType::String,
                )
                .set_default_value(Some(json!("")));
            }
            if node.get_pin_by_name("source_table").is_none() {
                node.add_input_pin(
                    "source_table",
                    "Source Table",
                    "BigQuery table name (only used when registration_mode is 'table')",
                    VariableType::String,
                )
                .set_default_value(Some(json!("")));
            }
            if node.get_pin_by_name("row_limit").is_none() {
                node.add_input_pin(
                    "row_limit",
                    "Row Limit",
                    "Optional LIMIT applied in 'table' mode. 0 means no limit.",
                    VariableType::Integer,
                )
                .set_default_value(Some(json!(0)));
            }
        }
    }
}

#[cfg(feature = "bigquery")]
async fn run_bigquery_registration(
    cached_session: &crate::data::datafusion::session::CachedDataFusionSession,
    client: &gcp_bigquery_client::Client,
    project_id: &str,
    location: &str,
    sql: &str,
    table_name: &str,
    page_size: i64,
    use_query_cache: bool,
    max_bytes_billed: i64,
) -> flow_like_types::Result<BigQueryJobStats> {
    use flow_like_storage::arrow::array::RecordBatch;
    use flow_like_storage::arrow::datatypes::Schema as ArrowSchema;
    use flow_like_storage::datafusion::common::TableReference;
    use flow_like_storage::datafusion::datasource::memory::MemTable;
    use gcp_bigquery_client::model::get_query_results_parameters::GetQueryResultsParameters;
    use gcp_bigquery_client::model::query_request::QueryRequest;
    use gcp_bigquery_client::model::table_field_schema::TableFieldSchema;

    let mut request = QueryRequest::new(sql.to_string());
    request.use_legacy_sql = false;
    request.location = Some(location.to_string());
    request.use_query_cache = Some(use_query_cache);
    if page_size > 0 {
        request.max_results = Some(page_size.min(i32::MAX as i64) as i32);
    }
    if max_bytes_billed > 0 {
        request.maximum_bytes_billed = Some(max_bytes_billed.to_string());
    }

    let initial = client
        .job()
        .query(project_id, request)
        .await
        .map_err(|e| flow_like_types::anyhow!("BigQuery query failed: {}", e))?;

    let schema_fields: Vec<TableFieldSchema> = initial
        .schema
        .as_ref()
        .and_then(|s| s.fields.clone())
        .ok_or_else(|| {
            flow_like_types::anyhow!(
                "BigQuery query returned no schema (job may be incomplete or failed)"
            )
        })?;

    let arrow_schema = Arc::new(ArrowSchema::new(
        schema_fields
            .iter()
            .map(bq_field_to_arrow)
            .collect::<Vec<_>>(),
    ));

    let mut all_rows: Vec<gcp_bigquery_client::model::table_row::TableRow> =
        initial.rows.clone().unwrap_or_default();

    let job_id_opt = initial
        .job_reference
        .as_ref()
        .and_then(|r| r.job_id.clone());
    let mut page_token = initial.page_token.clone();
    let mut job_complete = initial.job_complete.unwrap_or(false);

    if let Some(job_id) = job_id_opt.as_ref() {
        loop {
            let need_more = page_token.is_some() || !job_complete;
            if !need_more {
                break;
            }

            let mut params = GetQueryResultsParameters {
                page_token: page_token.clone(),
                location: Some(location.to_string()),
                ..Default::default()
            };
            if page_size > 0 {
                params.max_results = Some(page_size.min(i32::MAX as i64) as i32);
            }

            let next = client
                .job()
                .get_query_results(project_id, job_id, params)
                .await
                .map_err(|e| flow_like_types::anyhow!("BigQuery pagination failed: {}", e))?;

            if !next.job_complete.unwrap_or(false) {
                tokio::time::sleep(std::time::Duration::from_millis(250)).await;
                job_complete = false;
                continue;
            }
            job_complete = true;

            if let Some(rows) = next.rows {
                all_rows.extend(rows);
            }
            page_token = next.page_token;
            if page_token.is_none() {
                break;
            }
        }
    }

    let batch: RecordBatch = rows_to_record_batch(&arrow_schema, &schema_fields, &all_rows)?;

    let mem_table = Arc::new(MemTable::try_new(arrow_schema, vec![vec![batch]]).map_err(|e| {
        flow_like_types::anyhow!("Failed to build DataFusion MemTable for BigQuery result: {}", e)
    })?);

    cached_session
        .ctx
        .register_table(TableReference::bare(table_name.to_string()), mem_table)
        .map_err(|e| flow_like_types::anyhow!("Failed to register BigQuery table: {}", e))?;

    let stats = BigQueryJobStats {
        job_id: job_id_opt,
        location: Some(location.to_string()),
        bytes_processed: initial
            .total_bytes_processed
            .as_deref()
            .and_then(|s| s.parse::<i64>().ok()),
        total_rows: Some(all_rows.len() as i64),
        cache_hit: initial.cache_hit,
    };
    Ok(stats)
}

#[cfg(feature = "bigquery")]
fn bq_field_to_arrow(
    field: &gcp_bigquery_client::model::table_field_schema::TableFieldSchema,
) -> flow_like_storage::arrow::datatypes::Field {
    use flow_like_storage::arrow::datatypes::{DataType, Field, TimeUnit};
    use gcp_bigquery_client::model::field_type::FieldType;

    let is_repeated = field.mode.as_deref() == Some("REPEATED");
    let is_nullable = !matches!(field.mode.as_deref(), Some("REQUIRED"));

    let data_type = if is_repeated {
        DataType::Utf8
    } else {
        match field.r#type {
            FieldType::String | FieldType::Bytes => DataType::Utf8,
            FieldType::Integer | FieldType::Int64 => DataType::Int64,
            FieldType::Float | FieldType::Float64 => DataType::Float64,
            FieldType::Numeric | FieldType::Bignumeric => DataType::Float64,
            FieldType::Boolean | FieldType::Bool => DataType::Boolean,
            FieldType::Timestamp => DataType::Timestamp(TimeUnit::Microsecond, None),
            FieldType::Date => DataType::Date32,
            FieldType::Time
            | FieldType::Datetime
            | FieldType::Geography
            | FieldType::Json
            | FieldType::Interval
            | FieldType::Record
            | FieldType::Struct => DataType::Utf8,
        }
    };

    Field::new(&field.name, data_type, is_nullable)
}

#[cfg(feature = "bigquery")]
fn rows_to_record_batch(
    schema: &Arc<flow_like_storage::arrow::datatypes::Schema>,
    fields: &[gcp_bigquery_client::model::table_field_schema::TableFieldSchema],
    rows: &[gcp_bigquery_client::model::table_row::TableRow],
) -> flow_like_types::Result<flow_like_storage::arrow::array::RecordBatch> {
    use flow_like_storage::arrow::array::{
        ArrayRef, BooleanBuilder, Date32Builder, Float64Builder, Int64Builder, RecordBatch,
        StringBuilder, TimestampMicrosecondBuilder,
    };
    use flow_like_storage::arrow::datatypes::DataType;
    use flow_like_types::Value;

    let n_cols = fields.len();
    let n_rows = rows.len();
    let mut columns: Vec<ArrayRef> = Vec::with_capacity(n_cols);

    for (col_idx, field) in fields.iter().enumerate() {
        let arrow_field = schema.field(col_idx);
        let is_repeated = field.mode.as_deref() == Some("REPEATED");

        let array: ArrayRef = match arrow_field.data_type() {
            DataType::Utf8 => {
                let mut b = StringBuilder::with_capacity(n_rows, n_rows * 16);
                for row in rows {
                    let raw = cell_value(row, col_idx);
                    match raw {
                        None => b.append_null(),
                        Some(v) => {
                            if is_repeated {
                                b.append_value(v.to_string());
                            } else {
                                match v {
                                    Value::String(s) => b.append_value(s),
                                    Value::Null => b.append_null(),
                                    other => b.append_value(other.to_string()),
                                }
                            }
                        }
                    }
                }
                Arc::new(b.finish()) as ArrayRef
            }
            DataType::Int64 => {
                let mut b = Int64Builder::with_capacity(n_rows);
                for row in rows {
                    match cell_scalar_str(row, col_idx) {
                        None => b.append_null(),
                        Some(s) => match s.parse::<i64>() {
                            Ok(v) => b.append_value(v),
                            Err(_) => b.append_null(),
                        },
                    }
                }
                Arc::new(b.finish()) as ArrayRef
            }
            DataType::Float64 => {
                let mut b = Float64Builder::with_capacity(n_rows);
                for row in rows {
                    match cell_scalar_str(row, col_idx) {
                        None => b.append_null(),
                        Some(s) => match s.parse::<f64>() {
                            Ok(v) => b.append_value(v),
                            Err(_) => b.append_null(),
                        },
                    }
                }
                Arc::new(b.finish()) as ArrayRef
            }
            DataType::Boolean => {
                let mut b = BooleanBuilder::with_capacity(n_rows);
                for row in rows {
                    match cell_scalar_str(row, col_idx) {
                        None => b.append_null(),
                        Some(s) => match s.to_ascii_lowercase().as_str() {
                            "true" | "t" | "1" => b.append_value(true),
                            "false" | "f" | "0" => b.append_value(false),
                            _ => b.append_null(),
                        },
                    }
                }
                Arc::new(b.finish()) as ArrayRef
            }
            DataType::Date32 => {
                let mut b = Date32Builder::with_capacity(n_rows);
                for row in rows {
                    match cell_scalar_str(row, col_idx) {
                        None => b.append_null(),
                        Some(s) => match parse_date32(&s) {
                            Some(v) => b.append_value(v),
                            None => b.append_null(),
                        },
                    }
                }
                Arc::new(b.finish()) as ArrayRef
            }
            DataType::Timestamp(_, _) => {
                let mut b = TimestampMicrosecondBuilder::with_capacity(n_rows);
                for row in rows {
                    match cell_scalar_str(row, col_idx) {
                        None => b.append_null(),
                        Some(s) => match parse_timestamp_micros(&s) {
                            Some(v) => b.append_value(v),
                            None => b.append_null(),
                        },
                    }
                }
                Arc::new(b.finish()) as ArrayRef
            }
            other => {
                return Err(flow_like_types::anyhow!(
                    "Unsupported Arrow type built for BigQuery field '{}': {:?}",
                    field.name,
                    other
                ));
            }
        };

        columns.push(array);
    }

    RecordBatch::try_new(schema.clone(), columns)
        .map_err(|e| flow_like_types::anyhow!("Failed to build RecordBatch: {}", e))
}

#[cfg(feature = "bigquery")]
fn cell_value(
    row: &gcp_bigquery_client::model::table_row::TableRow,
    col_idx: usize,
) -> Option<flow_like_types::Value> {
    row.columns
        .as_ref()
        .and_then(|c| c.get(col_idx))
        .and_then(|cell| cell.value.clone())
}

#[cfg(feature = "bigquery")]
fn cell_scalar_str(
    row: &gcp_bigquery_client::model::table_row::TableRow,
    col_idx: usize,
) -> Option<String> {
    use flow_like_types::Value;
    match cell_value(row, col_idx)? {
        Value::Null => None,
        Value::String(s) => Some(s),
        Value::Number(n) => Some(n.to_string()),
        Value::Bool(b) => Some(b.to_string()),
        other => Some(other.to_string()),
    }
}

#[cfg(feature = "bigquery")]
fn parse_date32(s: &str) -> Option<i32> {
    let date = chrono::NaiveDate::parse_from_str(s.trim(), "%Y-%m-%d").ok()?;
    let epoch = chrono::NaiveDate::from_ymd_opt(1970, 1, 1)?;
    Some((date - epoch).num_days() as i32)
}

#[cfg(feature = "bigquery")]
fn parse_timestamp_micros(s: &str) -> Option<i64> {
    let trimmed = s.trim();
    if let Ok(f) = trimmed.parse::<f64>() {
        return Some((f * 1_000_000.0) as i64);
    }
    if let Ok(ts) = chrono::DateTime::parse_from_rfc3339(trimmed) {
        return Some(ts.timestamp_micros());
    }
    if let Ok(ts) = chrono::NaiveDateTime::parse_from_str(trimmed, "%Y-%m-%d %H:%M:%S%.f") {
        return Some(ts.and_utc().timestamp_micros());
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use flow_like::flow::pin::PinType;

    #[test]
    fn test_node_structure() {
        let node = RegisterBigQueryNode::new().get_node();
        assert_eq!(node.name, "df_register_bigquery");
        assert_eq!(node.friendly_name, "Register BigQuery");
        assert_eq!(node.category, "Data/DataFusion/Databases");
    }

    #[test]
    fn test_takes_gcp_provider_pin() {
        let node = RegisterBigQueryNode::new().get_node();
        let provider = node
            .pins
            .values()
            .find(|p| p.name == "provider" && p.pin_type == PinType::Input)
            .expect("provider input pin");
        assert_eq!(provider.data_type, VariableType::Struct);
        assert!(provider.schema.is_some(), "provider pin must be schema-typed GcpProvider");
    }

    #[test]
    fn test_no_raw_credential_pins_on_bigquery() {
        // Credentials live on the provider; the BigQuery node must not duplicate them.
        let node = RegisterBigQueryNode::new().get_node();
        assert!(node.get_pin_by_name("credential_mode").is_none());
        assert!(node.get_pin_by_name("service_account_json").is_none());
        assert!(node.get_pin_by_name("service_account_file").is_none());
    }

    #[test]
    fn test_session_pins_are_schema_typed() {
        let node = RegisterBigQueryNode::new().get_node();
        let session_in = node
            .pins
            .values()
            .find(|p| p.name == "session" && p.pin_type == PinType::Input)
            .expect("session input pin");
        assert_eq!(session_in.data_type, VariableType::Struct);
        assert!(session_in.schema.is_some());

        let session_out = node
            .pins
            .values()
            .find(|p| p.name == "session_out" && p.pin_type == PinType::Output)
            .expect("session_out output pin");
        assert_eq!(session_out.data_type, VariableType::Struct);
        assert!(session_out.schema.is_some());
    }

    #[test]
    fn test_default_shape_is_table_mode() {
        let node = RegisterBigQueryNode::new().get_node();
        assert!(node.get_pin_by_name("dataset").is_some());
        assert!(node.get_pin_by_name("source_table").is_some());
        assert!(node.get_pin_by_name("row_limit").is_some());
        assert!(node.get_pin_by_name("query").is_none());
    }

    #[test]
    fn test_sync_registration_mode_switches_and_is_diff_only() {
        let mut node = RegisterBigQueryNode::new().get_node();

        sync_registration_mode_pins(&mut node, REG_QUERY);
        assert!(node.get_pin_by_name("query").is_some());
        assert!(node.get_pin_by_name("dataset").is_none());
        let id_before = node.get_pin_by_name("query").unwrap().id.clone();

        sync_registration_mode_pins(&mut node, REG_QUERY);
        let id_after = node.get_pin_by_name("query").unwrap().id.clone();
        assert_eq!(id_before, id_after);

        sync_registration_mode_pins(&mut node, REG_TABLE);
        assert!(node.get_pin_by_name("query").is_none());
        assert!(node.get_pin_by_name("dataset").is_some());
    }

    #[test]
    fn test_build_sql_table_mode() {
        let sql = build_sql("table", "my-proj", "ds", "tbl", "", 0).unwrap();
        assert_eq!(sql, "SELECT * FROM `my-proj.ds.tbl`");
    }

    #[test]
    fn test_build_sql_table_mode_with_limit() {
        let sql = build_sql("table", "p", "d", "t", "", 500).unwrap();
        assert_eq!(sql, "SELECT * FROM `p.d.t` LIMIT 500");
    }

    #[test]
    fn test_build_sql_table_mode_strips_backticks() {
        let sql = build_sql("table", "`p`", "`d`", "`t`", "", 0).unwrap();
        assert_eq!(sql, "SELECT * FROM `p.d.t`");
    }

    #[test]
    fn test_build_sql_table_mode_missing_parts() {
        assert!(build_sql("table", "p", "", "t", "", 0).is_err());
        assert!(build_sql("table", "p", "d", "", "", 0).is_err());
    }

    #[test]
    fn test_build_sql_query_mode() {
        let sql = build_sql("query", "p", "", "", "SELECT 1 AS x", 0).unwrap();
        assert_eq!(sql, "SELECT 1 AS x");
    }

    #[test]
    fn test_build_sql_query_mode_empty() {
        assert!(build_sql("query", "p", "", "", "", 0).is_err());
    }

    #[test]
    fn test_build_sql_unknown_mode() {
        assert!(build_sql("bogus", "p", "d", "t", "", 0).is_err());
    }
}
