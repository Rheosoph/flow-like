Thursday, 11:40 p.m. The first full nightly run dies at document 178 of 214 — a timeout, already fixed. Friday morning you're holding the run button, and the only question that matters: if you press it, what happens? Do 36 documents process, or 214, or do you end up with 392 records for 214 contracts? If you can't answer without looking, the pipeline isn't finished yet.

> **Predict first:** What two properties would make that rerun completely boring — the good kind of boring?

## 1 · Assemble the chain

Everything you've built snaps together in a fixed order: list the folder → route each file by type → extract (deterministic, AI for the empties) → structure (keywords, sections, summary) → redact (regex, then AI) → deliver. Delivery is three writes: the structured record into a database table, the masked copy into a shared storage prefix, and the search index. Full-Text Search and Vector Search run over your tables — the Data course owns indexing internals, and the Events course owns wiring the schedule that triggers all this while you sleep.

@DataPipelinesOverview

That's the general shape, straight from the pipeline playbook: an Extract stage (APIs, databases, files, streams — scheduled, event-driven, or on demand), a Transform stage (clean, map, aggregate, enrich — typed nodes, SQL, AI-assisted), and a Load stage (database, data lake, API, files) that's repeatable, observable, and recoverable. Your document pipeline is this picture with files as the source.

## 2 · Make reruns boring

Two properties answer the hook: *only new work runs, and repeated writes converge.*

**Only new work.** **Diff Directory** compares your contracts/ folder against a manifest and emits exactly what changed — added, updated, and deleted files. Its partner, **Write Directory Manifest**, commits processed paths to that manifest, so the next diff only reports what's still outstanding. The discipline that makes this safe: commit *after* the file's writes succeed, never before. A checkpoint that advances before the work is done is how documents silently vanish from pipelines.

**Converging writes.** Insert is fast, but its own description warns it "might write duplicate items" on retries. Use upsert semantics instead, keyed on something stable — the source path, or a content hash from **Hash File**. Rerun the same file and the write lands on the same record.

Put together, Friday morning becomes: press run, Diff Directory reports the 36 uncommitted files, they process, their records upsert, the manifest commits. 214 contracts, 214 records, zero drama.

## 3 · Watch it run

A pipeline that runs at 2 a.m. needs to explain itself at 9 a.m. That's Runs and Logs.

@RunsAndLogs

That's a board grouped into three labeled stages (1 · Listen for requests, 2 · Draft with AI, 3 · Approve and send) with a Runs panel on the right — a run named "Incoming Support Request" completed in 1.85 s with a green check — and a log pane below with severity filters from Debug to Fatal. Each entry carries its own timing: one logs a received request at 120 ms, another a drafted reply at 730 ms with token counts. Your nightly run gets the same treatment: every execution listed, every step timed and inspectable.

Log like an auditor reads it: counts, document IDs, durations, and a final summary — processed, skipped, routed to review, failed. What never goes in: document content or masked values. The quality checklist says it directly — sensitive document content is not exposed in logs. A Debug line with a contract paragraph in it is a leak wearing a log level.

**Recap**

- Fixed order: list → route → extract → structure → redact → deliver.
- Diff Directory + manifest committed after success, plus upserts on a stable key = boring reruns.
- Log counts, IDs, and durations — never document content.
