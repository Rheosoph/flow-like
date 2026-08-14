The archived-tickets import you designed last lesson just ran: 1,200 rows written to a new `tickets` table. Then the network hiccuped, and — nervous — you ran the whole thing again.

> **Predict first:** how many rows does the table hold now, 1,200 or 2,400? The answer was decided before the first run, by two choices: how rows get their IDs, and whether you write with insert or upsert.

## 1 · Scope and schema first

Open **Data → Data Studio → Sources** to see the copilot's tables. Here's one opened:

@AppDatabases

That's the `customers` table in Data Studio's grid view: five rows, columns `customer_id` (CUS-1004 through CUS-1042), `name`, `plan`, `status`, `open_tickets`, and `last_contact`, with a **Schema** button in the toolbar and a search box with filters above the grid. Project tables like this one are shared app data; personal tables isolate rows per user. Decide scope when you create the table — moving data later means a migration plus an access review.

A schema is an interface: accurate types, explicit optionality. Adding an optional column is cheap; changing what an existing column *means* silently breaks every consumer.

## 2 · Open, write, flush

The **Open Database** node takes a table name and a `user_scoped` flag and returns a connection reference. Pass that reference between nodes — not table contents.

Writes are buffered for throughput: rows become durable in batches, not one by one. That buys speed and costs visibility — a Count right after a write can see stale data. **Flush Database** is the boundary: flush before anything verifies, reads, or reports success. Treat a successful flush as part of the transaction.

Write semantics are your retry contract:

- **Insert** — duplicate identity is an error. Right when "this must be new."
- **Upsert** — a deterministic key creates or updates the same logical record. Right for imports, where a retry means "the same data again," not "more data."
- Batch variants — always, for sets of rows. Per-row loops make overhead the main workload.

The key word is *deterministic*. Derive the ID from the source: the old helpdesk's own ticket number, or the hash-plus-version from your lesson-2 manifest. Random IDs turn every retry into brand-new business records — that's the 2,400 answer. Deterministic IDs plus upsert give you 1,200 no matter how many times the import runs.

## 3 · Prove it, don't hope it

Idempotence isn't a vibe; it's a test you can run:

1. Create `tickets` with `ticket_id`, `status`, `customer_id`, `source_id`, `source_version`, `updated_at`.
2. Batch-upsert the archived CSV, keyed by `ticket_id`. Flush.
3. Count: 1,200. Now run the entire import again. Count: still 1,200.
4. Change one ticket's status in the source, bump `source_version`, re-run: that row updates in place, the count doesn't move.
5. Add an index only for a filter you actually run (say, `status`) — indexes speed reads and tax every write.

Two runs, same count, one updated row: that's an import you can put on a schedule and forget.

**Watch out:** before dropping or renaming a column, check what depends on it. Saved queries and ontology overlays reference your schema, and they aren't rewritten automatically.

## Recap

- Scope and schema are design-time choices; migrations and access reviews are the price of changing your mind.
- Deterministic IDs + upsert + flush make imports safe to re-run; random IDs + insert make retries multiply records.
- Run the import twice before you trust it once.
