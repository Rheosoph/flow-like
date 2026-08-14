Your first dashboard for the copilot looks great — until the support lead reads it. "Enterprise customers opened 214 tickets in July." She's certain it was 107. The query joined tickets to customers, and every ticket came back exactly twice. Nothing crashed, nothing warned you. By the end of this lesson you'll know why, and you'll know which of Flow-Like's two SQL surfaces to reach for in the first place.

## 1 · Two SQL surfaces

- **Data Studio → Queries** is the app-facing workbench: SQL over native tables, ontology overlays, and installed remote contracts. Local statements can be saved — together with their table, chart, graph, or JSON visualization — as stored queries, or as reusable views when they take no parameters. The July dashboard lives here.
- **A DataFusion session** is SQL inside one workflow run. The flow creates a session, registers sources under table names — CSV, JSON, Parquet, Lance tables, external engines like PostgreSQL, MySQL, DuckDB, ClickHouse, or BigQuery, and lake formats like Delta and Iceberg — queries across them, and ends. The session is a coordinator, not a database: results survive only if the flow writes them somewhere.

The rule: recurring views over app data go to Data Studio; one-run federation of mixed sources goes to a session. If a single run must join the archived-tickets CSV, the `customers` native table, and a billing Postgres, that's one session with three registered names — not three imports into native tables first.

## 2 · Register, describe, then query

The safe sequence in a session never changes: create the session, register each source under a stable SQL-safe name, list tables and describe schemas, then run a small `SELECT` of a few columns with a `LIMIT` before anything ambitious. Describing schemas isn't bureaucracy — it's how you (or an agent authoring the query) stop guessing column names, and it's where you learn each source's *grain*: what one row means.

Two safety habits while you're here: never concatenate user text into SQL — bind parameters or allow-list identifiers — and keep external database credentials in secrets or provider connections, preferably read-only accounts for analytics.

## 3 · Debug the doubled dashboard

Back to 214 versus 107. Here's the query:

```sql
SELECT
  DATE_TRUNC('month', t.created_at) AS month,
  c.plan,
  COUNT(*) AS tickets
FROM tickets AS t
JOIN customers AS c ON c.customer_id = t.customer_id
WHERE t.status = 'closed'
GROUP BY DATE_TRUNC('month', t.created_at), c.plan
ORDER BY month, c.plan;
```

The SQL is valid. The data broke the assumption underneath it: an early, non-idempotent import once left `customers` with two rows per `customer_id` — lesson 3's random-ID mistake, coming home to roost — so the join matched every ticket twice. A join at the wrong grain doesn't error. It multiplies.

Before trusting any aggregate: confirm join keys are unique at the grain you assume (`SELECT customer_id, COUNT(*) … GROUP BY customer_id HAVING COUNT(*) > 1`), filter early, aggregate before results leave the engine, and keep `LIMIT` on while exploring. Large results belong in a table, a file, or a chart — not pasted into a model's context.

**Watch out:** the session vanishes with the run. If the summary matters, write it to a durable destination before the flow ends — "the query worked" and "the result exists" are different claims.

## Recap

- Data Studio Queries is for saved, app-facing SQL; a DataFusion session federates registered sources for one run.
- Describe schemas and confirm grain before joining — a valid join at the wrong grain silently multiplies rows.
- Sessions persist nothing; write results you need to keep.
