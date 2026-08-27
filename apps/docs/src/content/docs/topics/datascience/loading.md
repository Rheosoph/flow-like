---
title: Data Loading & Storage
description: Load files, spreadsheets, databases, and external sources into Flow-Like workflows
sidebar:
  order: 2
---

Choose the reader from the source format and expected data size. Preserve the source path and schema alongside the loaded data so downstream analysis remains traceable.

## Choose a loading path

| Input | Recommended starting point |
|-------|----------------------------|
| Small text file | [Read to String](/nodes/data/files/content/read-to-string/) |
| Large CSV | [Buffered CSV Reader](/nodes/utils/csv/csv-buffered-reader/) |
| Queryable CSV, JSON, or Parquet | Mount it in a DataFusion session |
| Excel workbook | Cell, worksheet, or table extraction nodes |
| JSON payload | Parse with a schema; repair only when the source is expected to be imperfect |
| App file | Use the app storage path abstraction |
| Local structured records | Open the local database and insert or upsert |
| External database or lake | Register it with DataFusion |
| Provider file store | Use the provider connection and its typed file nodes |

## CSV files

Use [Buffered CSV Reader](/nodes/utils/csv/csv-buffered-reader/) when the file should be processed in bounded batches. Validate the header before accepting the first batch and keep a rejection path for malformed rows.

For SQL analysis, use [Mount CSV](/nodes/data/datafusion/df-mount-csv/) to register the file in a [DataFusion session](/nodes/data/datafusion/df-create-session/).

Check:

- delimiter and quoting rules;
- text encoding;
- whether headers are present and unique;
- decimal, date, and timezone conventions;
- empty-string versus null behavior;
- expected row count and key uniqueness.

## Excel files

| Need | Node |
|------|------|
| Read or write one cell | [Excel Read Cell](/nodes/data/excel/excel-read-cell/), [Excel Write Cell](/nodes/data/excel/excel-write-cell/) |
| Discover worksheets | [Get Sheet Names](/nodes/data/excel/files-spreadsheet-get-sheet-names/) |
| Extract predictable tables | [Extract Tables (Excel)](/nodes/data/excel/data-excel-extract-tables/) |
| Extract irregular tables | [Extract Tables AI (Excel)](/nodes/data/excel/data-excel-extract-tables-ai/) |

Inspect sheet names first, then select the intended sheet and validate its expected columns. AI extraction is useful for unusual layouts, but it should still be followed by type, row-count, and business-rule checks.

## JSON

Use [Parse JSON with Schema](/nodes/utils/json/parse-with-schema/) when the expected shape is known. The schema makes required fields and types explicit and avoids spreading defensive field checks throughout the board.

[Repair Parse JSON](/nodes/utils/json/repair-parse/) is for input that may be almost, but not quite, valid JSON. Do not use repair to hide a broken contract from a system you control; fix the producer or reject the payload.

## Parquet and analytical files

[Mount Parquet](/nodes/data/datafusion/df-mount-parquet/) registers a Parquet file for SQL queries without converting it to rows first. Parquet is a good fit for repeated analytical scans because it is columnar and carries a schema.

```sql
SELECT
  region,
  SUM(revenue) AS revenue
FROM analytics
WHERE event_date >= DATE '2026-01-01'
GROUP BY region
ORDER BY revenue DESC;
```

Use [Mount JSON](/nodes/data/datafusion/df-mount-json/) for JSON or NDJSON and [Mount CSV](/nodes/data/datafusion/df-mount-csv/) for delimited files.

## App storage and paths

App storage gives workflows a provider-independent path to files owned by the app. See [App storage](/apps/storage/) for how files are organized and accessed.

The file catalog provides distinct path constructors:

| Source | Node |
|--------|------|
| App storage directory | [Storage Dir](/nodes/data/files/directories/path-from-storage-dir/) |
| Explicit raw path | [Raw Path](/nodes/data/files/path/raw-path/) |
| Convert a raw path into a Flow-Like path | [From Raw Path](/nodes/data/files/path/from-raw-path/) |
| Convert a local path value | [Local Path to Path](/nodes/data/files/pathbuf-to-path/) |

Use [Path Exists?](/nodes/data/files/operations/path-exists/) before an optional read, and [List Paths](/nodes/data/files/operations/path-list-paths/) to enumerate a directory. Avoid relying on machine-specific paths in a board intended to run on different backends.

## Local database

[Open Database](/nodes/data/database/open-local-db/) opens the app-local database. Choose a write node by volume and retry behavior:

| Behavior | Node |
|----------|------|
| Insert one record | [Insert](/nodes/data/database/insert/insert-local-db/) |
| Insert a collection | [Batch Insert](/nodes/data/database/insert/batch-insert-local-db/) |
| Insert a CSV table | [Batch Insert (CSV)](/nodes/data/database/insert/csv-insert-local-db/) |
| Retry-safe single write | [Upsert](/nodes/data/database/insert/upsert-local-db/) |
| Retry-safe collection write | [Batch Upsert](/nodes/data/database/insert/batch-upsert-local-db/) |

Build a stable key before using upsert. After a large write, [Flush Database](/nodes/data/database/optimization/flush-local-db/) can make the persistence boundary explicit. Use [Build Index](/nodes/data/database/optimization/index-local-db/) for fields that support repeated filters or searches. [Choosing a Lance index](/topics/datascience/lance-indexes/) explains the current Flow-Like choices, the latest stable upstream options, and when a legacy vector index needs a one-time rebuild for cosine distance.

Query local records with [(SQL) Filter Database](/nodes/data/database/search/filter-local-db/). Keep result limits and selected fields bounded for interactive workflows.

## External sources

DataFusion can register PostgreSQL, MySQL, SQLite, DuckDB, ClickHouse, Oracle, BigQuery, Athena, FlightSQL, and other cataloged sources. Browse [DataFusion databases](/nodes/data/datafusion/databases/) and [DataFusion lakes](/nodes/data/datafusion/lakes/) for the current set.

Provider nodes cover service-specific file and data operations. The catalog includes [AWS, Azure, GCP, and Cloudflare provider builders](/nodes/data/providers/) plus typed integrations for services such as Microsoft 365, Google Workspace, GitHub, Notion, Atlassian, and Databricks.

Keep credentials in secrets or provider connections, not in path strings or examples.

## Writing files

Use:

- [Write String](/nodes/data/files/content/write-string/) for text;
- [Write Bytes](/nodes/data/files/content/write-bytes/) for binary content;
- format-specific writers when the destination has a structured contract.

Write to a temporary or versioned path first when replacing an important artifact. Confirm the result before moving consumers to the new file.

## Loading checklist

- [ ] Reader matches the actual format and expected size
- [ ] Source path, version, and ingestion time are retained
- [ ] Header or schema is validated before processing
- [ ] Large files are streamed, mounted, or batched
- [ ] Invalid rows have a rejection reason and source reference
- [ ] Destination writes are retry-safe
- [ ] Credentials come from secrets or provider connections
- [ ] Machine-specific paths are avoided in portable boards
- [ ] Row counts and key uniqueness are checked

## Troubleshooting

| Symptom | Check |
|---------|-------|
| File not found | Path constructor, storage scope, permissions, execution backend |
| Out of memory | Buffered reader, DataFusion mount, batch size, selected columns |
| CSV rows shift columns | Delimiter, quoting, embedded newlines, encoding |
| Spreadsheet table is missing | Worksheet selection, merged cells, table boundaries |
| JSON fields disappear | Schema optionality, field names, repair behavior |
| Duplicate database records | Stable key, upsert choice, checkpoint timing |

## Next steps

- [DataFusion and SQL](/topics/datascience/datafusion/)
- [Machine learning](/topics/datascience/ml/)
- [Data visualization](/topics/datascience/visualization/)
- [Data pipelines](/topics/data-pipelines/overview/)
