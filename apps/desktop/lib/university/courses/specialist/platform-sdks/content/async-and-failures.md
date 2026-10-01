A background invocation acknowledges a request before the run finishes. Adapt your chosen client to use `trigger_event_async` in Python or `triggerEventAsync` in TypeScript. Keep the returned run ID and poll token privately.

```python
result = client.trigger_event_async(app_id, event_id, payload)
status = client.get_run_status(result.run_id)
print(status.status)
```

```typescript
const result = await client.triggerEventAsync(appId, eventId, payload);
const status = await client.getRunStatus(result.run_id);
console.log(status.status);
```

Replace those variable names with your client's existing app/Event/payload values. Query status until the run reaches a terminal outcome, with a bounded wait and delay between requests. Alternatively use `poll_execution`/`pollExecution` with the poll token; advance its sequence cursor as entries arrive. A run ID means accepted work, not a successful business result.

Test the original streamed call with a revoked test key. Expect authorization failure before the Event executes. Do not respond by granting broad administrative rights. Restore a properly scoped test credential for the final exercise.

A client timeout can leave the server run active. Reconcile the run/result before retrying an action with effects. The synthetic normalization Event is safe to repeat, but that property does not transfer automatically to payments or sends.
