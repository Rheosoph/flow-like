Use the supplied localhost mock, which is the same service used in API Integrations. Start it with `python3 mock_api.py`. Use local Desktop execution.

1. Create a Simple Event and build POST `http://127.0.0.1:8765/notes` with **Make Request**.
2. Add bearer token `practice-token`, header `Idempotency-Key: signup-001`, and struct body `{ "ticket_id": "T-001", "text": "Practice signup" }`.
3. Send through **API Call** and inspect the status before parsing the result. On success expect `stored: true`.
4. Repeat the same call with the same key/body, then GET `/notes`. Expect count 1.
5. Change the body while keeping the key. Expect 409; do not treat that conflict as successful creation.

The mock enforces operation-key uniqueness under a lock. That is why simultaneous equivalent requests can converge. Searching for an email and then creating a contact does not provide that guarantee by itself. For a real CRM, use its documented unique-key/idempotency mechanism or a suitable atomic claim; otherwise define manual reconciliation for uncertain writes.

To extend this into webhook glue, use the authenticated Generic Event from API Integrations. Preserve the caller's stable operation identity across retries, validate the payload, then inspect both the incoming Event and the stored record.

Stop the mock with Ctrl-C when finished. Restarting clears its in-memory records.
