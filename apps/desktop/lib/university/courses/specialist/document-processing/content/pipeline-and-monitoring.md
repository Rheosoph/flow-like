Use **Diff Directory** to compare a practice input folder with a manifest. Process one file, then use **Write Directory Manifest** to commit only the paths whose required outputs succeeded.

The completion boundary includes the verified record and masked copy. If you also maintain an index, include that outcome. Commit after these writes; leave a failed document uncommitted for the next sweep.

Simulate a failure after writing the record but before writing the masked copy. Run again using the same source ID and upsert key. Expect one record, a completed masked copy and a committed path. An output path or content hash alone does not define how source updates and deletions should be reconciled; write that policy explicitly.

Record processed, skipped, held-for-review and failed counts with source IDs. If a document is held because extraction or masking is uncertain, keep it separate from completed work. It must not silently enter the shared index.

Scheduling is covered in Events. This exercise stays manual so you can inspect each transition.
