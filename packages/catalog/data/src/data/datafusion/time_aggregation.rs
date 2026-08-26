#[cfg(feature = "execute")]
use crate::data::datafusion::query::batches_to_rows;
use crate::data::datafusion::session::DataFusionSession;
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic, NodeScores},
    pin::ValueType,
    variable::VariableType,
};
use flow_like_types::{async_trait, json::json};

/// Time interval units for aggregation
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimeInterval {
    Second,
    Minute,
    FiveMinutes,
    FifteenMinutes,
    ThirtyMinutes,
    Hour,
    Day,
    Week,
    Month,
    Quarter,
    Year,
}

impl TimeInterval {
    pub fn to_interval_string(&self) -> &'static str {
        match self {
            TimeInterval::Second => "1 second",
            TimeInterval::Minute => "1 minute",
            TimeInterval::FiveMinutes => "5 minutes",
            TimeInterval::FifteenMinutes => "15 minutes",
            TimeInterval::ThirtyMinutes => "30 minutes",
            TimeInterval::Hour => "1 hour",
            TimeInterval::Day => "1 day",
            TimeInterval::Week => "1 week",
            TimeInterval::Month => "1 month",
            TimeInterval::Quarter => "3 months",
            TimeInterval::Year => "1 year",
        }
    }

    pub fn to_trunc_precision(&self) -> &'static str {
        match self {
            TimeInterval::Second => "second",
            TimeInterval::Minute
            | TimeInterval::FiveMinutes
            | TimeInterval::FifteenMinutes
            | TimeInterval::ThirtyMinutes => "minute",
            TimeInterval::Hour => "hour",
            TimeInterval::Day => "day",
            TimeInterval::Week => "week",
            TimeInterval::Month => "month",
            TimeInterval::Quarter => "quarter",
            TimeInterval::Year => "year",
        }
    }

    pub fn from_string(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "second" | "1s" | "s" => Some(TimeInterval::Second),
            "minute" | "1m" | "m" => Some(TimeInterval::Minute),
            "5m" | "5min" | "5_minutes" | "five_minutes" => Some(TimeInterval::FiveMinutes),
            "15m" | "15min" | "15_minutes" | "fifteen_minutes" => {
                Some(TimeInterval::FifteenMinutes)
            }
            "30m" | "30min" | "30_minutes" | "thirty_minutes" => Some(TimeInterval::ThirtyMinutes),
            "hour" | "1h" | "h" => Some(TimeInterval::Hour),
            "day" | "1d" | "d" => Some(TimeInterval::Day),
            "week" | "1w" | "w" => Some(TimeInterval::Week),
            "month" | "1mo" | "mo" => Some(TimeInterval::Month),
            "quarter" | "q" | "3mo" => Some(TimeInterval::Quarter),
            "year" | "1y" | "y" => Some(TimeInterval::Year),
            _ => None,
        }
    }
}

/// Create a time-binned aggregation query using date_bin
/// This is the most efficient method for fixed-interval aggregations
#[crate::register_node]
#[derive(Default)]
pub struct TimeBinAggregationNode {}

impl TimeBinAggregationNode {
    pub fn new() -> Self {
        TimeBinAggregationNode {}
    }
}

#[async_trait]
impl NodeLogic for TimeBinAggregationNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "df_time_bin_aggregation",
            "Time Bin Aggregation",
            "Create time-based aggregations using DataFusion's date_bin function. Groups data by fixed time intervals (minute, hour, day, etc.) and applies aggregation functions.",
            "Data/DataFusion/Aggregation",
        );
        node.set_flowscript_name("df", "timeBinAggregation");
        node.set_receiver("session");
        node.add_icon("/flow/icons/clock.svg");
        node.set_version(1);

        node.add_input_pin(
            "exec_in",
            "Input",
            "Trigger execution",
            VariableType::Execution,
        );

        node.add_input_pin(
            "session",
            "Session",
            "DataFusion session to execute the query in",
            VariableType::Struct,
        )
        .set_schema::<DataFusionSession>();

        node.add_input_pin(
            "source_table",
            "Source Table",
            "Name of the table to aggregate",
            VariableType::String,
        );

        node.add_input_pin(
            "timestamp_column",
            "Timestamp Column",
            "Column containing timestamp/datetime values",
            VariableType::String,
        );

        node.add_input_pin(
            "interval",
            "Interval",
            "Time interval for binning: second, minute, 5m, 15m, 30m, hour, day, week, month, quarter, year",
            VariableType::String,
        )
        .set_default_value(Some(json!("hour")));

        node.add_input_pin(
            "value_columns",
            "Value Columns",
            "Columns to aggregate (comma-separated)",
            VariableType::String,
        );

        node.add_input_pin(
            "aggregations",
            "Aggregations",
            "Aggregation functions to apply (comma-separated): count, sum, avg, min, max, first, last",
            VariableType::String,
        )
        .set_default_value(Some(json!("count,sum,avg")));

        node.add_input_pin(
            "group_by",
            "Group By",
            "Additional columns to group by (comma-separated, optional)",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_input_pin(
            "filter",
            "Filter",
            "Optional WHERE clause filter (e.g., 'status = active')",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_input_pin(
            "output_table",
            "Output Table",
            "Name for the result table (optional, creates view if provided)",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_output_pin(
            "exec_out",
            "Output",
            "Continues after query execution",
            VariableType::Execution,
        );

        node.add_output_pin(
            "session_out",
            "Session",
            "DataFusion session (pass-through)",
            VariableType::Struct,
        )
        .set_schema::<DataFusionSession>();

        node.add_output_pin(
            "results",
            "Results",
            "Query results as array of row structs",
            VariableType::Struct,
        )
        .set_value_type(ValueType::Array)
        .set_open_schema();

        node.add_output_pin(
            "sql",
            "SQL",
            "Generated SQL query for debugging",
            VariableType::String,
        );

        node.add_output_pin(
            "row_count",
            "Row Count",
            "Number of result rows",
            VariableType::Integer,
        );

        node.scores = Some(NodeScores {
            privacy: 8,
            security: 8,
            performance: 9,
            governance: 8,
            reliability: 8,
            cost: 9,
        });

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let session: DataFusionSession = context.evaluate_pin("session").await?;
        let source_table: String = context.evaluate_pin("source_table").await?;
        let timestamp_column: String = context.evaluate_pin("timestamp_column").await?;
        let interval_str: String = context.evaluate_pin("interval").await?;
        let value_columns: String = context.evaluate_pin("value_columns").await?;
        let aggregations: String = context.evaluate_pin("aggregations").await?;
        let group_by: String = context.evaluate_pin("group_by").await.unwrap_or_default();
        let filter: String = context.evaluate_pin("filter").await.unwrap_or_default();
        let output_table: String = context
            .evaluate_pin("output_table")
            .await
            .unwrap_or_default();

        let interval = TimeInterval::from_string(&interval_str)
        .ok_or_else(|| flow_like_types::anyhow!("Invalid interval: {}. Use: second, minute, 5m, 15m, 30m, hour, day, week, month, quarter, year", interval_str))?;

        let cached_session = session.load(context).await?;

        // Parse value columns and aggregations
        let value_cols: Vec<&str> = value_columns
            .split(',')
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .collect();
        let aggs: Vec<&str> = aggregations
            .split(',')
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .collect();

        if value_cols.is_empty() {
            return Err(flow_like_types::anyhow!(
                "At least one value column is required"
            ));
        }

        // Build aggregation expressions
        let mut select_parts = vec![format!(
            "date_bin(INTERVAL '{}', {}, TIMESTAMP '1970-01-01') as time_bucket",
            interval.to_interval_string(),
            timestamp_column
        )];

        // Add group by columns to select
        let group_cols: Vec<&str> = group_by
            .split(',')
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .collect();
        for col in &group_cols {
            select_parts.push(col.to_string());
        }

        // Build aggregations
        for col in &value_cols {
            for agg in &aggs {
                let agg_fn = match *agg {
                    "count" => format!("COUNT({}) as {}_{}", col, col, agg),
                    "sum" => format!("SUM({}) as {}_{}", col, col, agg),
                    "avg" => format!("AVG({}) as {}_{}", col, col, agg),
                    "min" => format!("MIN({}) as {}_{}", col, col, agg),
                    "max" => format!("MAX({}) as {}_{}", col, col, agg),
                    "first" => format!("FIRST_VALUE({}) as {}_{}", col, col, agg),
                    "last" => format!("LAST_VALUE({}) as {}_{}", col, col, agg),
                    "stddev" => format!("STDDEV({}) as {}_{}", col, col, agg),
                    "variance" => format!("VARIANCE({}) as {}_{}", col, col, agg),
                    _ => continue,
                };
                select_parts.push(agg_fn);
            }
        }

        // Build GROUP BY clause
        let mut group_by_parts = vec!["time_bucket".to_string()];
        group_by_parts.extend(group_cols.iter().map(|s| s.to_string()));

        // Build WHERE clause
        let where_clause = if filter.is_empty() {
            String::new()
        } else {
            format!(" WHERE {}", filter)
        };

        // Build full query
        let sql = format!(
            "SELECT {} FROM {}{} GROUP BY {} ORDER BY time_bucket",
            select_parts.join(", "),
            source_table,
            where_clause,
            group_by_parts.join(", ")
        );

        context.set_pin_value("sql", json!(sql.clone())).await?;

        // Execute query
        let df = cached_session.ctx.sql(&sql).await?;
        let batches = df.collect().await?;

        // Convert to JSON
        let results = batches_to_rows(&batches)?;

        let row_count = results.len();

        // Register as view if output table specified
        if !output_table.is_empty() {
            let view_sql = format!("CREATE OR REPLACE VIEW {} AS {}", output_table, sql);
            cached_session.ctx.sql(&view_sql).await?.collect().await?;
        }

        context.set_pin_value("session_out", json!(session)).await?;
        context.set_pin_value("results", json!(results)).await?;
        context.set_pin_value("row_count", json!(row_count)).await?;
        context.activate_exec_pin("exec_out").await?;

        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "Node execution is not enabled. Rebuild with the execute feature flag."
        ))
    }
}

/// Truncate timestamps to a specific precision for grouping
/// Uses date_trunc which is simpler for standard intervals
#[crate::register_node]
#[derive(Default)]
pub struct DateTruncAggregationNode {}

impl DateTruncAggregationNode {
    pub fn new() -> Self {
        DateTruncAggregationNode {}
    }
}

#[async_trait]
impl NodeLogic for DateTruncAggregationNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "df_date_trunc_aggregation",
            "Date Truncate Aggregation",
            "Truncate timestamps to a specific precision (hour, day, month, etc.) and aggregate. Simpler alternative to date_bin for standard intervals.",
            "Data/DataFusion/Aggregation",
        );
        node.set_flowscript_name("df", "dateTruncAggregation");
        node.set_receiver("session");
        node.add_icon("/flow/icons/clock.svg");
        node.set_version(1);

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
            "source_table",
            "Source Table",
            "Table to aggregate",
            VariableType::String,
        );

        node.add_input_pin(
            "timestamp_column",
            "Timestamp Column",
            "Timestamp column name",
            VariableType::String,
        );

        node.add_input_pin(
            "precision",
            "Precision",
            "Truncation precision: second, minute, hour, day, week, month, quarter, year",
            VariableType::String,
        )
        .set_default_value(Some(json!("hour")));

        node.add_input_pin(
            "aggregation_sql",
            "Aggregation SQL",
            "SQL aggregation expressions (e.g., 'COUNT(*) as cnt, SUM(amount) as total')",
            VariableType::String,
        );

        node.add_input_pin(
            "filter",
            "Filter",
            "Optional WHERE clause",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_output_pin(
            "exec_out",
            "Output",
            "Execution continues",
            VariableType::Execution,
        );

        node.add_output_pin(
            "session_out",
            "Session",
            "Session pass-through",
            VariableType::Struct,
        )
        .set_schema::<DataFusionSession>();

        node.add_output_pin(
            "results",
            "Results",
            "Aggregation results",
            VariableType::Struct,
        )
        .set_value_type(ValueType::Array)
        .set_open_schema();

        node.add_output_pin("sql", "SQL", "Generated SQL", VariableType::String);

        node.scores = Some(NodeScores {
            privacy: 8,
            security: 8,
            performance: 9,
            governance: 8,
            reliability: 8,
            cost: 9,
        });

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let session: DataFusionSession = context.evaluate_pin("session").await?;
        let source_table: String = context.evaluate_pin("source_table").await?;
        let timestamp_column: String = context.evaluate_pin("timestamp_column").await?;
        let precision: String = context.evaluate_pin("precision").await?;
        let aggregation_sql: String = context.evaluate_pin("aggregation_sql").await?;
        let filter: String = context.evaluate_pin("filter").await.unwrap_or_default();

        let valid_precisions = [
            "second", "minute", "hour", "day", "week", "month", "quarter", "year",
        ];
        let precision_lower = precision.to_lowercase();
        if !valid_precisions.contains(&precision_lower.as_str()) {
            return Err(flow_like_types::anyhow!(
                "Invalid precision: {}. Use: {}",
                precision,
                valid_precisions.join(", ")
            ));
        }

        let cached_session = session.load(context).await?;

        let where_clause = if filter.is_empty() {
            String::new()
        } else {
            format!(" WHERE {}", filter)
        };

        let sql = format!(
            "SELECT date_trunc('{}', {}) as time_period, {} FROM {}{} GROUP BY time_period ORDER BY time_period",
            precision_lower, timestamp_column, aggregation_sql, source_table, where_clause
        );

        context.set_pin_value("sql", json!(sql.clone())).await?;

        let df = cached_session.ctx.sql(&sql).await?;
        let batches = df.collect().await?;

        let results = batches_to_rows(&batches)?;

        context.set_pin_value("session_out", json!(session)).await?;
        context.set_pin_value("results", json!(results)).await?;
        context.activate_exec_pin("exec_out").await?;

        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "Node execution is not enabled. Rebuild with the execute feature flag."
        ))
    }
}

/// Create a sliding/tumbling window aggregation
#[crate::register_node]
#[derive(Default)]
pub struct WindowAggregationNode {}

impl WindowAggregationNode {
    pub fn new() -> Self {
        WindowAggregationNode {}
    }
}

#[async_trait]
impl NodeLogic for WindowAggregationNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "df_window_aggregation",
            "Window Aggregation",
            "Apply window functions for rolling/moving aggregations over time series data.",
            "Data/DataFusion/Aggregation",
        );
        node.set_flowscript_name("df", "windowAggregation");
        node.set_receiver("session");
        node.add_icon("/flow/icons/chart-network.svg");
        node.set_version(1);

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
            "source_table",
            "Source Table",
            "Table to query",
            VariableType::String,
        );

        node.add_input_pin(
            "timestamp_column",
            "Timestamp Column",
            "Column for ordering",
            VariableType::String,
        );

        node.add_input_pin(
            "value_column",
            "Value Column",
            "Column to aggregate",
            VariableType::String,
        );

        node.add_input_pin(
            "window_function",
            "Window Function",
            "Function: avg, sum, min, max, count, row_number, rank, lag, lead",
            VariableType::String,
        )
        .set_default_value(Some(json!("avg")));

        node.add_input_pin(
            "window_size",
            "Window Size",
            "Number of preceding rows (for rolling window), use 0 for cumulative",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(10)));

        node.add_input_pin(
            "partition_by",
            "Partition By",
            "Columns to partition by (comma-separated, optional)",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_input_pin(
            "select_columns",
            "Select Columns",
            "Additional columns to include (comma-separated)",
            VariableType::String,
        )
        .set_default_value(Some(json!("*")));

        node.add_output_pin(
            "exec_out",
            "Output",
            "Execution continues",
            VariableType::Execution,
        );

        node.add_output_pin(
            "session_out",
            "Session",
            "Session pass-through",
            VariableType::Struct,
        )
        .set_schema::<DataFusionSession>();

        node.add_output_pin("results", "Results", "Query results", VariableType::Struct)
            .set_value_type(ValueType::Array)
            .set_open_schema();

        node.add_output_pin("sql", "SQL", "Generated SQL", VariableType::String);

        node.scores = Some(NodeScores {
            privacy: 8,
            security: 8,
            performance: 8,
            governance: 8,
            reliability: 8,
            cost: 9,
        });

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let session: DataFusionSession = context.evaluate_pin("session").await?;
        let source_table: String = context.evaluate_pin("source_table").await?;
        let timestamp_column: String = context.evaluate_pin("timestamp_column").await?;
        let value_column: String = context.evaluate_pin("value_column").await?;
        let window_function: String = context.evaluate_pin("window_function").await?;
        let window_size: i64 = context.evaluate_pin("window_size").await?;
        let partition_by: String = context
            .evaluate_pin("partition_by")
            .await
            .unwrap_or_default();
        let select_columns: String = context
            .evaluate_pin("select_columns")
            .await
            .unwrap_or_else(|_| "*".to_string());

        let cached_session = session.load(context).await?;

        let partition_clause = if partition_by.is_empty() {
            String::new()
        } else {
            format!("PARTITION BY {} ", partition_by)
        };

        let frame_clause = if window_size == 0 {
            "ROWS UNBOUNDED PRECEDING".to_string()
        } else {
            format!("ROWS BETWEEN {} PRECEDING AND CURRENT ROW", window_size)
        };

        let window_expr = match window_function.to_lowercase().as_str() {
            "avg" => format!(
                "AVG({}) OVER ({}ORDER BY {} {})",
                value_column, partition_clause, timestamp_column, frame_clause
            ),
            "sum" => format!(
                "SUM({}) OVER ({}ORDER BY {} {})",
                value_column, partition_clause, timestamp_column, frame_clause
            ),
            "min" => format!(
                "MIN({}) OVER ({}ORDER BY {} {})",
                value_column, partition_clause, timestamp_column, frame_clause
            ),
            "max" => format!(
                "MAX({}) OVER ({}ORDER BY {} {})",
                value_column, partition_clause, timestamp_column, frame_clause
            ),
            "count" => format!(
                "COUNT({}) OVER ({}ORDER BY {} {})",
                value_column, partition_clause, timestamp_column, frame_clause
            ),
            "row_number" => format!(
                "ROW_NUMBER() OVER ({}ORDER BY {})",
                partition_clause, timestamp_column
            ),
            "rank" => format!(
                "RANK() OVER ({}ORDER BY {})",
                partition_clause, timestamp_column
            ),
            "lag" => format!(
                "LAG({}, 1) OVER ({}ORDER BY {})",
                value_column, partition_clause, timestamp_column
            ),
            "lead" => format!(
                "LEAD({}, 1) OVER ({}ORDER BY {})",
                value_column, partition_clause, timestamp_column
            ),
            _ => {
                return Err(flow_like_types::anyhow!(
                    "Unknown window function: {}",
                    window_function
                ));
            }
        };

        let sql = format!(
            "SELECT {}, {} as window_result FROM {} ORDER BY {}",
            select_columns, window_expr, source_table, timestamp_column
        );

        context.set_pin_value("sql", json!(sql.clone())).await?;

        let df = cached_session.ctx.sql(&sql).await?;
        let batches = df.collect().await?;

        let results = batches_to_rows(&batches)?;

        context.set_pin_value("session_out", json!(session)).await?;
        context.set_pin_value("results", json!(results)).await?;
        context.activate_exec_pin("exec_out").await?;

        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "Node execution is not enabled. Rebuild with the execute feature flag."
        ))
    }
}

/// Convert DateTime<Utc> to/from DataFusion timestamps
#[crate::register_node]
#[derive(Default)]
pub struct DateTimeToTimestampNode {}

impl DateTimeToTimestampNode {
    pub fn new() -> Self {
        DateTimeToTimestampNode {}
    }
}

#[async_trait]
impl NodeLogic for DateTimeToTimestampNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "df_datetime_to_timestamp",
            "DateTime to SQL Timestamp",
            "Convert a DateTime (ISO 8601 string) to SQL timestamp literal for use in DataFusion queries.",
            "Data/DataFusion/Time",
        );
        node.set_flowscript_name("df", "toSqlTimestamp");
        node.set_receiver("datetime");
        node.add_icon("/flow/icons/clock.svg");

        node.add_input_pin(
            "datetime",
            "DateTime",
            "DateTime value (ISO 8601 string format)",
            VariableType::Date,
        );

        node.add_output_pin(
            "timestamp_literal",
            "Timestamp Literal",
            "SQL timestamp literal (e.g., TIMESTAMP '2024-01-15 10:30:00')",
            VariableType::String,
        );

        node.add_output_pin(
            "epoch_micros",
            "Epoch Microseconds",
            "Timestamp as microseconds since Unix epoch",
            VariableType::Integer,
        );

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
        let datetime_str: String = context.evaluate_pin("datetime").await?;

        // Parse ISO 8601 datetime
        let dt = chrono::DateTime::parse_from_rfc3339(&datetime_str)
            .or_else(|_| {
                // Try without timezone
                chrono::NaiveDateTime::parse_from_str(&datetime_str, "%Y-%m-%dT%H:%M:%S")
                    .or_else(|_| {
                        chrono::NaiveDateTime::parse_from_str(&datetime_str, "%Y-%m-%d %H:%M:%S")
                    })
                    .map(|ndt| ndt.and_utc().fixed_offset())
            })
            .map_err(|e| {
                flow_like_types::anyhow!("Failed to parse datetime '{}': {}", datetime_str, e)
            })?;

        let timestamp_literal = format!("TIMESTAMP '{}'", dt.format("%Y-%m-%d %H:%M:%S%.6f"));
        let epoch_micros = dt.timestamp_micros();

        context
            .set_pin_value("timestamp_literal", json!(timestamp_literal))
            .await?;
        context
            .set_pin_value("epoch_micros", json!(epoch_micros))
            .await?;

        Ok(())
    }
}

/// Parse a time range for filtering queries
#[crate::register_node]
#[derive(Default)]
pub struct TimeRangeFilterNode {}

impl TimeRangeFilterNode {
    pub fn new() -> Self {
        TimeRangeFilterNode {}
    }
}

#[async_trait]
impl NodeLogic for TimeRangeFilterNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "df_time_range_filter",
            "Time Range Filter",
            "Generate a SQL WHERE clause for filtering by time range. Supports relative time expressions.",
            "Data/DataFusion/Time",
        );
        node.set_flowscript_name("df", "timeRangeFilter");
        node.add_icon("/flow/icons/filter.svg");

        node.add_input_pin(
            "timestamp_column",
            "Timestamp Column",
            "Name of the timestamp column to filter",
            VariableType::String,
        );

        node.add_input_pin(
            "start_time",
            "Start Time",
            "Start of range (ISO 8601 or relative: '-1d', '-24h', '-30m')",
            VariableType::String,
        )
        .set_default_value(Some(json!("-24h")));

        node.add_input_pin(
            "end_time",
            "End Time",
            "End of range (ISO 8601, 'now', or relative)",
            VariableType::String,
        )
        .set_default_value(Some(json!("now")));

        node.add_output_pin(
            "where_clause",
            "WHERE Clause",
            "SQL WHERE clause fragment",
            VariableType::String,
        );

        node.add_output_pin(
            "start_timestamp",
            "Start Timestamp",
            "Resolved start timestamp literal",
            VariableType::String,
        );

        node.add_output_pin(
            "end_timestamp",
            "End Timestamp",
            "Resolved end timestamp literal",
            VariableType::String,
        );

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
        let timestamp_column: String = context.evaluate_pin("timestamp_column").await?;
        let start_time: String = context.evaluate_pin("start_time").await?;
        let end_time: String = context.evaluate_pin("end_time").await?;

        let now = chrono::Utc::now();

        let parse_time = |s: &str| -> flow_like_types::Result<chrono::DateTime<chrono::Utc>> {
            if s == "now" {
                return Ok(now);
            }

            // Try relative time
            if s.starts_with('-') || s.starts_with('+') {
                let (sign, rest) = match s.strip_prefix('-') {
                    Some(rest) => (-1i64, rest),
                    None => (1i64, s.strip_prefix('+').unwrap_or(s)),
                };

                let (num_str, unit) = rest.split_at(rest.len() - 1);
                let num: i64 = num_str
                    .parse()
                    .map_err(|_| flow_like_types::anyhow!("Invalid relative time: {}", s))?;

                let duration = match unit {
                    "s" => chrono::Duration::seconds(num * sign),
                    "m" => chrono::Duration::minutes(num * sign),
                    "h" => chrono::Duration::hours(num * sign),
                    "d" => chrono::Duration::days(num * sign),
                    "w" => chrono::Duration::weeks(num * sign),
                    _ => {
                        return Err(flow_like_types::anyhow!(
                            "Invalid time unit in '{}'. Use s, m, h, d, w",
                            s
                        ));
                    }
                };

                return Ok(now + duration);
            }

            // Try ISO 8601
            chrono::DateTime::parse_from_rfc3339(s)
                .map(|dt| dt.with_timezone(&chrono::Utc))
                .or_else(|_| {
                    chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%S")
                        .or_else(|_| chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S"))
                        .or_else(|_| {
                            chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d")
                                .map(|d| d.and_hms_opt(0, 0, 0).unwrap())
                        })
                        .map(|ndt| ndt.and_utc())
                })
                .map_err(|e| flow_like_types::anyhow!("Failed to parse time '{}': {}", s, e))
        };

        let start_dt = parse_time(&start_time)?;
        let end_dt = parse_time(&end_time)?;

        let start_literal = format!("TIMESTAMP '{}'", start_dt.format("%Y-%m-%d %H:%M:%S%.6f"));
        let end_literal = format!("TIMESTAMP '{}'", end_dt.format("%Y-%m-%d %H:%M:%S%.6f"));

        let where_clause = format!(
            "{} >= {} AND {} < {}",
            timestamp_column, start_literal, timestamp_column, end_literal
        );

        context
            .set_pin_value("where_clause", json!(where_clause))
            .await?;
        context
            .set_pin_value("start_timestamp", json!(start_literal))
            .await?;
        context
            .set_pin_value("end_timestamp", json!(end_literal))
            .await?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use flow_like::flow::pin::PinType;

    #[test]
    fn test_time_interval_from_string_standard() {
        assert_eq!(
            TimeInterval::from_string("second"),
            Some(TimeInterval::Second)
        );
        assert_eq!(
            TimeInterval::from_string("minute"),
            Some(TimeInterval::Minute)
        );
        assert_eq!(TimeInterval::from_string("hour"), Some(TimeInterval::Hour));
        assert_eq!(TimeInterval::from_string("day"), Some(TimeInterval::Day));
        assert_eq!(TimeInterval::from_string("week"), Some(TimeInterval::Week));
        assert_eq!(
            TimeInterval::from_string("month"),
            Some(TimeInterval::Month)
        );
        assert_eq!(
            TimeInterval::from_string("quarter"),
            Some(TimeInterval::Quarter)
        );
        assert_eq!(TimeInterval::from_string("year"), Some(TimeInterval::Year));
    }

    #[test]
    fn test_time_interval_from_string_shortcuts() {
        assert_eq!(TimeInterval::from_string("1s"), Some(TimeInterval::Second));
        assert_eq!(TimeInterval::from_string("s"), Some(TimeInterval::Second));
        assert_eq!(TimeInterval::from_string("1m"), Some(TimeInterval::Minute));
        assert_eq!(TimeInterval::from_string("m"), Some(TimeInterval::Minute));
        assert_eq!(
            TimeInterval::from_string("5m"),
            Some(TimeInterval::FiveMinutes)
        );
        assert_eq!(
            TimeInterval::from_string("15m"),
            Some(TimeInterval::FifteenMinutes)
        );
        assert_eq!(
            TimeInterval::from_string("30m"),
            Some(TimeInterval::ThirtyMinutes)
        );
        assert_eq!(TimeInterval::from_string("1h"), Some(TimeInterval::Hour));
        assert_eq!(TimeInterval::from_string("h"), Some(TimeInterval::Hour));
        assert_eq!(TimeInterval::from_string("1d"), Some(TimeInterval::Day));
        assert_eq!(TimeInterval::from_string("d"), Some(TimeInterval::Day));
        assert_eq!(TimeInterval::from_string("1w"), Some(TimeInterval::Week));
        assert_eq!(TimeInterval::from_string("w"), Some(TimeInterval::Week));
        assert_eq!(TimeInterval::from_string("1mo"), Some(TimeInterval::Month));
        assert_eq!(TimeInterval::from_string("mo"), Some(TimeInterval::Month));
        assert_eq!(TimeInterval::from_string("q"), Some(TimeInterval::Quarter));
        assert_eq!(
            TimeInterval::from_string("3mo"),
            Some(TimeInterval::Quarter)
        );
        assert_eq!(TimeInterval::from_string("1y"), Some(TimeInterval::Year));
        assert_eq!(TimeInterval::from_string("y"), Some(TimeInterval::Year));
    }

    #[test]
    fn test_time_interval_from_string_case_insensitive() {
        assert_eq!(
            TimeInterval::from_string("SECOND"),
            Some(TimeInterval::Second)
        );
        assert_eq!(
            TimeInterval::from_string("Minute"),
            Some(TimeInterval::Minute)
        );
        assert_eq!(TimeInterval::from_string("HOUR"), Some(TimeInterval::Hour));
    }

    #[test]
    fn test_time_interval_from_string_invalid() {
        assert_eq!(TimeInterval::from_string("invalid"), None);
        assert_eq!(TimeInterval::from_string(""), None);
        assert_eq!(TimeInterval::from_string("10m"), None);
        assert_eq!(TimeInterval::from_string("2h"), None);
    }

    #[test]
    fn test_time_interval_to_interval_string() {
        assert_eq!(TimeInterval::Second.to_interval_string(), "1 second");
        assert_eq!(TimeInterval::Minute.to_interval_string(), "1 minute");
        assert_eq!(TimeInterval::FiveMinutes.to_interval_string(), "5 minutes");
        assert_eq!(
            TimeInterval::FifteenMinutes.to_interval_string(),
            "15 minutes"
        );
        assert_eq!(
            TimeInterval::ThirtyMinutes.to_interval_string(),
            "30 minutes"
        );
        assert_eq!(TimeInterval::Hour.to_interval_string(), "1 hour");
        assert_eq!(TimeInterval::Day.to_interval_string(), "1 day");
        assert_eq!(TimeInterval::Week.to_interval_string(), "1 week");
        assert_eq!(TimeInterval::Month.to_interval_string(), "1 month");
        assert_eq!(TimeInterval::Quarter.to_interval_string(), "3 months");
        assert_eq!(TimeInterval::Year.to_interval_string(), "1 year");
    }

    #[test]
    fn test_time_interval_to_trunc_precision() {
        assert_eq!(TimeInterval::Second.to_trunc_precision(), "second");
        assert_eq!(TimeInterval::Minute.to_trunc_precision(), "minute");
        assert_eq!(TimeInterval::FiveMinutes.to_trunc_precision(), "minute");
        assert_eq!(TimeInterval::FifteenMinutes.to_trunc_precision(), "minute");
        assert_eq!(TimeInterval::ThirtyMinutes.to_trunc_precision(), "minute");
        assert_eq!(TimeInterval::Hour.to_trunc_precision(), "hour");
        assert_eq!(TimeInterval::Day.to_trunc_precision(), "day");
        assert_eq!(TimeInterval::Week.to_trunc_precision(), "week");
        assert_eq!(TimeInterval::Month.to_trunc_precision(), "month");
        assert_eq!(TimeInterval::Quarter.to_trunc_precision(), "quarter");
        assert_eq!(TimeInterval::Year.to_trunc_precision(), "year");
    }

    #[test]
    fn test_time_bin_aggregation_node_structure() {
        let node_logic = TimeBinAggregationNode::new();
        let node = node_logic.get_node();

        assert_eq!(node.name, "df_time_bin_aggregation");
        assert_eq!(node.friendly_name, "Time Bin Aggregation");
        assert_eq!(node.category, "Data/DataFusion/Aggregation");

        let input_pins: Vec<_> = node
            .pins
            .values()
            .filter(|p| p.pin_type == PinType::Input)
            .collect();
        let output_pins: Vec<_> = node
            .pins
            .values()
            .filter(|p| p.pin_type == PinType::Output)
            .collect();

        assert!(input_pins.iter().any(|p| p.name == "exec_in"));
        assert!(input_pins.iter().any(|p| p.name == "session"));
        assert!(input_pins.iter().any(|p| p.name == "source_table"));
        assert!(input_pins.iter().any(|p| p.name == "timestamp_column"));
        assert!(input_pins.iter().any(|p| p.name == "interval"));
        assert!(output_pins.iter().any(|p| p.name == "exec_out"));
    }

    #[test]
    fn test_window_aggregation_node_structure() {
        let node_logic = WindowAggregationNode::new();
        let node = node_logic.get_node();

        assert_eq!(node.name, "df_window_aggregation");
        assert_eq!(node.friendly_name, "Window Aggregation");

        let input_pins: Vec<_> = node
            .pins
            .values()
            .filter(|p| p.pin_type == PinType::Input)
            .collect();

        assert!(input_pins.iter().any(|p| p.name == "window_size"));
        assert!(input_pins.iter().any(|p| p.name == "window_function"));
    }

    #[test]
    fn test_time_range_filter_node_structure() {
        let node_logic = TimeRangeFilterNode::new();
        let node = node_logic.get_node();

        assert_eq!(node.name, "df_time_range_filter");
        assert_eq!(node.friendly_name, "Time Range Filter");

        let input_pins: Vec<_> = node
            .pins
            .values()
            .filter(|p| p.pin_type == PinType::Input)
            .collect();
        let output_pins: Vec<_> = node
            .pins
            .values()
            .filter(|p| p.pin_type == PinType::Output)
            .collect();

        assert!(input_pins.iter().any(|p| p.name == "start_time"));
        assert!(input_pins.iter().any(|p| p.name == "end_time"));
        assert!(output_pins.iter().any(|p| p.name == "where_clause"));
    }
}
