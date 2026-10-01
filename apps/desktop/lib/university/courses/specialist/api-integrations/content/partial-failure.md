This lab uses the mock service's enforced idempotency key. That guarantee belongs to this fixture; confirm the receiver's real contract before using the same strategy elsewhere.

## Lose the response after the write

1. Restart `mock_api.py` to begin with no stored notes.
2. Build POST `http://127.0.0.1:8765/notes?drop_once=1` with the practice bearer token.
3. Add header `Idempotency-Key: operation-001` using **Set Header**, and use **Set Struct Body** for `{ "ticket_id": "T-001", "text": "Practice resolution" }`.
4. Execute. The fixture stores the note, then closes the first connection without a response. Observe the transport failure.
5. GET `/notes`. Expect count **1** despite the failed response.
6. Send the exact same POST with the same key. Expect a successful response and count still **1**.
7. Reuse the key with different text. Expect **409**, with the original note unchanged.

The fixture uses a lock around key lookup and write. A separate “search then create” sequence would leave a race between those operations.

## Add a second destination on paper

Suppose the CRM note succeeds but a local completion-record write fails. Retrying the first write with the same enforced key can recover; generating a new key creates a second logical operation. Record each destination outcome and keep the operation pending until required destinations are confirmed. A checkpoint written before all required effects succeed hides unfinished work.

Keep request_id separate from attempt number. Inspect `/notes` after recovery instead of relying on the last run color. The question assesses the recovery decision, not the local service state.
