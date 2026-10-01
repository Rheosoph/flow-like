Use the `practice_tickets` table from the core lab. This class first checks an aggregate, then chooses the right query environment.

In **Data Studio → Queries**, run:

```sql
SELECT status, COUNT(*) AS tickets
FROM practice_tickets
GROUP BY status
ORDER BY status;
```

After the updated fixture, expect `closed = 2` and `open = 1`. Then check identity:

```sql
SELECT ticket_id, COUNT(*) AS copies
FROM practice_tickets
GROUP BY ticket_id
HAVING COUNT(*) > 1;
```

Expect no rows. Save the useful aggregate in Data Studio if it is part of the app's recurring view.

## Inspect a join

The fixture includes `customers.csv`, where C-001 deliberately appears twice. Join ticket rows to those customers and T-001/T-002 multiply. Before any aggregate, establish each source's **grain**, the meaning of one row, and verify uniqueness of join keys. Fix the duplicate source or choose its intended version before aggregating. COUNT(DISTINCT ...) can hide the symptom while other measures remain wrong.

A **DataFusion session** is useful when one flow needs to query registered files, tables or external databases together. Create a session, register each source under a stable name, describe its schema and run a limited SELECT. The session does not make results durable: write any result you need after the run to a table or file.

Use parameters for input values and an allowlist for selectable identifiers. Do not concatenate untrusted text into SQL.
