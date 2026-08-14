A support agent presses Run in the browser. The request lands on one of your API pods.

> **Predict first:** which process actually executes the flow — the API pod that received the request, or something else?

## 1 · One request, two lanes

Something else, always. In these deployments the API never runs the board itself. It builds one normalized run request — flow, version, payload, scoped storage credentials, an execution JWT, callback details — and hands it to a configured dispatch backend. The backend controls transport and worker lifecycle; it never changes the flow graph.

Which backend gets the request depends on the door it came through:

```bash
# /invoke and streaming endpoints
EXECUTION_BACKEND=http

# /invoke/async endpoints
ASYNC_EXECUTION_BACKEND=redis
```

@ExecutionBackends

The diagram shows both lanes. Sync + streaming: the dispatcher builds one run request and posts it to an HTTP executor URL ("Pool · Compose · Function URL"), a private streaming Lambda, or the `kubernetes_job` path — labeled "Job spec only, job runner pending" — with `lambda_invoke` as the SDK handoff. Background: the API persists and enqueues, returning run metadata while a worker claims the job from a Redis queue, an SQS queue, a Kafka topic, or the SQS → Pipe → ECS chain that stages large payloads in object storage first. The footer line is the thesis of this lesson: backend names describe dispatch mechanisms, not automatic security guarantees.

## 2 · `http` is a protocol, not a platform

`EXECUTOR_URL` can point at the Compose runtime service, the chart's executor-pool Service (derived automatically when left empty), a Lambda Function URL, or any compatible HTTP execution service. That's why staging and production share one mental model — and it answers the agent's question: on `staging-01`, the run executes on one of the runtime replicas behind `http://runtime:9000`; on the cluster, on a warm executor-pool pod.

Warm means shared. Pool workers handle many runs over their lifetime, so treat them as a shared execution environment: verify cross-run cleanup, filesystem and credential behavior, and concurrency for your threat model before you call the deployment hardened.

## 3 · Three trap doors

**`kubernetes_job` creates Jobs, not results.** The API's dispatcher genuinely creates a Kubernetes Job per run — but the checked-in executor's one-job entrypoint is a placeholder that logs "unimplemented" and exits with status 1. It proves Job creation, not a working execution path. Don't select it with the stock images; use the warm pool, or supply and validate your own compatible one-job runner.

**A queue is half a system.** Pushing a job onto Redis, SQS, or Kafka is not execution. A compatible consumer must claim the message, run it, report state, and apply your retry and dead-letter policy. The Compose runtime consumes the Redis list (`QUEUE_WORKER_ENABLED=true`); the Kubernetes chart's executor pool does not — which is exactly why last lesson told you to set both lanes to `http` for a chart-only deployment.

**Typos don't fail closed.** Unknown backend values silently fall back to `http`. A misspelled backend name won't error at startup — it will quietly change where your runs execute. Validate the rendered configuration; don't rely on a typo to save you.

**Watch out:** picking a heavier backend never adds isolation by itself. A run inherits the isolation of whatever actually executes it — a shared pool pod is a shared pool pod, whatever the dispatcher is called.

**Recap**

- The API builds one run request and dispatches it; `EXECUTION_BACKEND` and `ASYNC_EXECUTION_BACKEND` select the sync and background lanes.
- `http` describes the protocol — pool, Compose runtime, or Function URL — and warm workers are shared across runs.
- `kubernetes_job` is not operational with stock images, queues need consumers, and unknown values fall back to `http`.
