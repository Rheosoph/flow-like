---
title: DataFusion & SQL Analytics
description: Query files, databases, and data-lake tables through one Flow-Like SQL session
sidebar:
  order: 3
---

Flow-Like embeds Apache DataFusion as a query layer. A workflow creates a session, registers one or more sources as named tables, then runs SQL across those tables.

## Session model

| Step | Purpose |
|------|---------|
| Create session | Allocate one query context for the run |
| Register sources | Give files, databases, or lake tables stable SQL names |
| Inspect | List tables and confirm schemas |
| Query | Filter, join, aggregate, or window the registered data |
| Deliver | Send the result to another node, table, chart, file, or model tool |

Start with [Create DataFusion Session](/nodes/data/datafusion/df-create-session/) and pass the same session value to every registration and query node that should share tables.

## Register files and in-memory data

| Source | Node |
|--------|------|
| CSV file | [Mount CSV](/nodes/data/datafusion/df-mount-csv/) |
| JSON or NDJSON file | [Mount JSON](/nodes/data/datafusion/df-mount-json/) |
| Parquet file | [Mount Parquet](/nodes/data/datafusion/df-mount-parquet/) |
| Lance table | [Register Lance Table](/nodes/data/datafusion/df-register-lance/) |
| CSVTable value already in the workflow | [Register Table](/nodes/data/datafusion/df-register-csv-table/) |

Choose a SQL-safe table name and keep it stable across the query. Validate file schemas before assuming a column type.

## Register databases

The generated catalog includes:

| Database | Node |
|----------|------|
| PostgreSQL | [Register PostgreSQL](/nodes/data/datafusion/databases/df-register-postgres/) |
| MySQL | [Register MySQL](/nodes/data/datafusion/databases/df-register-mysql/) |
| SQLite | [Register SQLite](/nodes/data/datafusion/databases/df-register-sqlite/) |
| DuckDB | [Register DuckDB](/nodes/data/datafusion/databases/df-register-duckdb/) |
| ClickHouse | [Register ClickHouse](/nodes/data/datafusion/databases/df-register-clickhouse/) |
| Oracle | [Register Oracle](/nodes/data/datafusion/databases/df-register-oracle/) |
| BigQuery | [Register BigQuery](/nodes/data/datafusion/databases/df-register-bigquery/) |
| FlightSQL | [Register FlightSQL](/nodes/data/datafusion/databases/df-register-flightsql/) |
| Athena | [Register Athena Table](/nodes/data/datafusion/databases/df-register-athena/) |

Store credentials in secrets or provider connections. Use a read-only account for analytical workflows unless the board explicitly requires writes elsewhere.

## Register data-lake tables

| Format | Nodes |
|--------|-------|
| Delta Lake | [Register Delta Table](/nodes/data/datafusion/lakes/df-register-delta/), [Delta Table Info](/nodes/data/datafusion/lakes/df-delta-info/), [Delta Time Travel](/nodes/data/datafusion/lakes/df-delta-time-travel/) |
| Apache Iceberg | [Register Iceberg Table](/nodes/data/datafusion/lakes/df-register-iceberg/), [Iceberg Table Info](/nodes/data/datafusion/lakes/df-iceberg-info/), [Iceberg Time Travel](/nodes/data/datafusion/lakes/df-iceberg-time-travel/) |
| Hive-partitioned Parquet | [Register Hive Parquet](/nodes/data/datafusion/lakes/df-register-hive-parquet/) |
| Partitioned JSON | [Register Partitioned JSON](/nodes/data/datafusion/lakes/df-register-partitioned-json/) |

Time-travel nodes are useful for reproducible analysis. Record the selected table version or snapshot with the analysis result.

For Athena results stored in S3, [Mount Athena S3 Results](/nodes/data/datafusion/databases/df-mount-athena-query/) can make the result available to the session.

## Inspect the session

Use [List Tables](/nodes/data/datafusion/tools/df-list-tables/) to confirm registration and [Describe Table](/nodes/data/datafusion/tools/df-describe-table/) to inspect the schema.

Inspecting first is especially important for agent-driven analysis and sources whose schema can evolve. Do not let a model guess table or column names when the workflow can retrieve them.

## Execute SQL

### Structured workflow output

[SQL Query](/nodes/data/datafusion/df-sql-query/) returns:

- a CSVTable for analytics and visualization;
- an array of row objects for workflow iteration;
- the row count.

Use it when downstream nodes need structured values.

### Agent-readable output

[Execute SQL](/nodes/data/datafusion/tools/df-execute-sql/) returns a Markdown table, a CSVTable, and the row count. Its formatted text output is convenient for a controlled data-analysis tool, but large results should remain in structured storage rather than being copied into a model context.

## SQL examples

### Aggregate by period

```sql
SELECT
  DATE_TRUNC('month', order_date) AS month,
  SUM(revenue) AS revenue
FROM orders
WHERE order_date >= DATE '2026-01-01'
GROUP BY DATE_TRUNC('month', order_date)
ORDER BY month;
```

### Join registered sources

```sql
SELECT
  o.order_id,
  c.customer_name,
  o.revenue
FROM orders AS o
JOIN customers AS c
  ON o.customer_id = c.customer_id
WHERE o.status = 'complete';
```

### Window calculation

```sql
SELECT
  order_date,
  revenue,
  SUM(revenue) OVER (
    ORDER BY order_date
    ROWS BETWEEN 6 PRECEDING AND CURRENT ROW
  ) AS seven_row_revenue
FROM daily_sales;
```

### Common table expression

```sql
WITH monthly_sales AS (
  SELECT
    DATE_TRUNC('month', order_date) AS month,
    SUM(revenue) AS revenue
  FROM orders
  GROUP BY DATE_TRUNC('month', order_date)
)
SELECT *
FROM monthly_sales
ORDER BY month;
```

Dynamic query text should be constructed only from strictly parsed or allow-listed values. The SQL Query node accepts a query string; do not concatenate arbitrary user input into it.

## Time-series helper nodes

The catalog includes workflow-oriented helpers for common time operations:

- [Time Bin Aggregation](/nodes/data/datafusion/aggregation/df-time-bin-aggregation/)
- [Date Truncate Aggregation](/nodes/data/datafusion/aggregation/df-date-trunc-aggregation/)
- [Window Aggregation](/nodes/data/datafusion/aggregation/df-window-aggregation/)
- [Time Range Filter](/nodes/data/datafusion/time/df-time-range-filter/)
- [DateTime to SQL Timestamp](/nodes/data/datafusion/time/df-datetime-to-timestamp/)

Use SQL when the calculation is already clear there. Use helper nodes when their typed inputs make a reusable board easier to configure safely.

## Write results

[Write Delta Table](/nodes/data/datafusion/lakes/df-write-delta/) writes a result into Delta Lake. For other destinations, pass the CSVTable or row output to the corresponding file, database, API, or A2UI node.

Before publishing a derived table, record its source window, query or board version, and row count.

## Performance guidance

- Filter early and select only required columns.
- Prefer Parquet or a lake table for repeated analytical scans.
- Aggregate before sending data to an A2UI page or model.
- Add `LIMIT` while exploring an unfamiliar table.
- Avoid per-row workflow loops for operations SQL can perform as a set.
- Inspect whether source filters are pushed down before assuming a federated query is cheap.
- Separate a fast summary query from slower drill-down queries.

## Troubleshooting

| Symptom | Check |
|---------|-------|
| Table not found | Same session value, registration execution path, exact table name |
| Column not found | Describe Table output, casing, schema evolution |
| Query is slow | Selected columns, filters, join size, source pushdown |
| Memory pressure | Result size, early aggregation, Parquet, batch boundaries |
| Unexpected duplicate rows | Join keys and source grain |
| Agent produces invalid SQL | List/describe tools, read-only tool, row limits, retry policy |

## Next steps

- [Data loading and storage](/topics/datascience/loading/)
- [Data visualization](/topics/datascience/visualization/)
- [AI-powered analysis](/topics/datascience/ai-analysis/)
- [Data pipelines](/topics/data-pipelines/overview/)
