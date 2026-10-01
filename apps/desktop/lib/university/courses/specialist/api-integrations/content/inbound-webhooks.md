Expose a harmless local flow as one API Event. Prerequisite: the Events course's Generic Event lab.

1. Add a Generic Event with String output fields `event` and `request_id`. Connect it to Print Info using the synthetic request ID.
2. Create an **API** Event on that node. Set method POST, path `/practice-webhook` and a practice bearer token `webhook-practice-token`. Use local execution.
3. Activate it. Replace `<app-id>` below with your scratch app's actual ID.

```bash
curl -X POST 'http://localhost:9657/<app-id>/practice-webhook'   -H 'Content-Type: application/json'   -H 'Authorization: Bearer webhook-practice-token'   -d '{"event":"practice.created","request_id":"request-001"}'
```

Inspect the streamed response and run. Confirm `request-001` reached the entry. Repeat without Authorization and expect 401 before the flow is invoked. The path is an HTTP sink path, independent of a Page route.

Before any real write, validate required fields, expected types and allowed operations inside the flow. Authentication alone does not make a payload valid. Add a branch that rejects a blank request_id and verify it reaches no write node.

Remote delivery uses the configured hub endpoint and supported sink. Copy that endpoint from your deployment, provision its credentials there and test it separately. Do not substitute a laptop URL for an always-on service.
