Use the local practice service. Keep its bearer token attached to every request.

## Classify responses

Call `/status/401`, `/status/404`, `/status/429` and `/status/503` through the same request flow. On Error, inspect Get Status Code before reading any success schema.

| Status | Practice decision |
| --- | --- |
| 401 | Stop and repair authentication |
| 404 | Return a deliberate missing-record result |
| 429 | Read Retry-After and delay before a bounded retry |
| 503 | Retry a safe operation with a bounded backoff |

Use a counter and **Delay** between retry attempts; stop after three attempts in this exercise. The fixture always returns the selected error, so the expected result is three recorded attempts followed by failure. No success branch should run.

## Follow pagination

GET `/customers?page=1` returns one item and `next_page: 2`. Request page 2 and expect the second item with `next_page: null`. Accumulate the two IDs and stop on null. Use the returned continuation value; an empty page and an absent continuation are not universally equivalent.

Add a delay between requests when the API requires it. Rate limits apply across callers, so a per-flow delay alone does not coordinate an entire team.

A timeout after a write leaves an unknown outcome. Automatically retry only when the operation has an enforced repeatable-write contract. Checking for an existing record and then creating it is vulnerable to concurrent requests unless the receiver or a suitable atomic claim enforces uniqueness. Practice this in the optional partial-failure lab.
