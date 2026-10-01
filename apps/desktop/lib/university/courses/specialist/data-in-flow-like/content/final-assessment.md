Add `{ "ticket_id": "T-004", "customer_id": "C-002", "status": "open" }` to the updated fixture. Run the import twice. Expect four records after both runs, with T-002 still closed.

Record the input hash or fixture version, the ID column, both counts and the inspected changed row. Trace the write and flush Error outputs. Confirm that each reaches Log Error and cannot reach the successful Count report. The questions check your decisions; completion does not automatically inspect the table.
