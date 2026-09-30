Use the same browser flow with three fixture URLs:

| Query | Expected behavior |
| --- | --- |
| `?delay=2500` | Wait, then read Ready to ship within the 5-second limit |
| `?missing=1` | Found=false after timeout; report unavailable; close resources |
| `?banner=1&delay=2500` | Same correct text despite layout movement and delay |

After every case, confirm no practice browser session remains open. Keep one small screenshot or the extracted value plus fixture URL and run ID. Do not capture unrelated windows.

**Wait For Selector** can finish with Found=false on timeout, so its execution continuation alone is not success evidence. Check the boolean. A fixed sleep can be too short or unnecessarily long; a condition tied to readiness is the relevant signal.

Retry a bounded read only when it can help. Before retrying a consequential click, determine whether the earlier action already took effect. This fixture’s Refresh button changes only local display state; a real order confirmation has a different contract.

For freshness, wait for a signal associated with the new request, rather than merely finding an old table left on the page. If the expected label or schema changes, stop and inspect instead of extracting an unrelated nearby value.
