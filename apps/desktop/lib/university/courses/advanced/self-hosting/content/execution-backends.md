Trace the practice run through the actual deployment.

The API authorizes and builds a run request. Default Compose synchronous invocation uses `EXECUTION_BACKEND=http` and `EXECUTOR_URL=http://execution-gateway:9000`. Background invocation uses `ASYNC_EXECUTION_BACKEND=redis`; the queue bridge hands work to execution managers.

The manager owns the per-run sandbox lifecycle. A warm reserve contains unused slots prepared to reduce startup delay. A trusted-shared configuration uses a different path for repeated trusted runs.

Run the harmless fixture, note its run ID and inspect logs around that time:

```bash
docker compose logs --since=5m api execution-manager queue-bridge
```

Identify admission, execution and terminal outcome where the logs expose them. Avoid adding payloads or credentials merely to correlate a request.

Completion: explain which component executed the request and whether it used the intended mode. An HTTP transport or Kubernetes Job name does not establish isolation. Alternative dispatch backends require a compatible worker and their own verification. Reference: [execution manager](https://docs.flow-like.com/self-hosting/docker-compose/execution-manager/).
