Use a table named `practice_tickets` in a scratch app. The exercise uses explicit fixture rows so file parsing does not hide the database behavior.

## Wire the write

1. Add **Simple Event → Open Database**. Set Table Name to `practice_tickets` and User Scoped to `false` for this shared synthetic table.
2. Add **Batch Upsert**. Connect Open Database's execution output and Database reference. Set **ID Column** to `ticket_id`.
3. Copy the array from `tickets.json` into Batch Upsert's **Value** input. Keep the pin as an array of structs.
4. Connect its Success output to **Flush Database**, passing the same Database reference. Connect Flush Success to **Count**, then print Count.
5. Route each write/flush Error output to **Log Error** with its Error Message. Do not print success on that branch.

Expected Count: **3**. Run again with the same array: **3**. Replace the array with `tickets-updated.json`: **3**, with T-002 now closed. Inspect the rows in Data Studio Sources to verify the changed value as well as the count.

**Insert can append duplicate items.** It is not a uniqueness check. Upsert uses the chosen ID column to replace or add the logical record. Random IDs defeat that behavior.

Open Database supports buffered writes. Flush exposes an explicit write completion boundary. Current Count and search nodes also ensure pending writes are flushed before reading; do not rely on the older claim that Count always sees stale buffered data. Route and inspect errors regardless of whether flushing is explicit or triggered by a read.

Adding an optional field is different from changing the meaning or type of a field used by queries and interfaces. Check those consumers before changing a real schema.
