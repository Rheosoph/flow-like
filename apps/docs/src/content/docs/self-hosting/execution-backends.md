---
title: Execution Backends
description: Configure how the Flow-Like API dispatches server-side runs
sidebar:
  order: 55
---

Compose and Kubernetes default to a Rust execution manager that assigns each
workflow to a clean gVisor sandbox. The API signs the dispatch, including its
artifact, credentials, callback destination and run identity. The runtime
verifies that binding before loading tenant code.

Execution mode determines the isolation boundary. Dispatch backend determines
how the request reaches that boundary. An HTTP response alone does not prove
that a workflow has started or completed.

## Dispatch model

The default self-hosted routes are:

```bash
# /invoke and streaming endpoints
EXECUTION_BACKEND=http

# /invoke/async endpoints
ASYNC_EXECUTION_BACKEND=redis
EXECUTION_ISOLATION_MODE=per_run
REDIS_EXECUTION_QUEUE=exec:jobs:v3
```

Both variables are parsed into the same backend enum, but not every transport
is appropriate for every endpoint. In particular, `lambda_stream` uses the
streaming dispatcher, while queue backends are normally selected for
asynchronous endpoints. Compose and Helm configure `EXECUTOR_URL` to reach the
manager and supply its private authentication token to trusted callers.

Requests containing runtime variables require direct HTTP or Lambda streaming
dispatch. The dispatcher rejects them for queues, asynchronous Lambda invocation,
and the legacy Kubernetes Job backend before publishing or staging the request.
This prevents per-run configuration from entering server-side transport storage.
Use a direct execution endpoint for these runs; the dispatcher does not switch
backends or discard their values automatically.

## Per-execution isolation

1. A manager prepares a runner and an external gateway before requests arrive.
   The runtime initializes its catalog and verification key, then waits without
   tenant code or credentials.
2. Admission reserves one ready slot and records a durable run claim. Compose
   managers share a local SQLite registry; Kubernetes managers share Redis
   claims. An empty pool returns HTTP 429 with `X-Execution-Admitted: false`
   before streaming success.
3. The gateway accepts one immutable run policy. The manager supplies the signed
   dispatch, and the runtime verifies it before fetching executable content.
4. The manager relays bounded results and retains capacity through cleanup.
   A client disconnect leaves accepted work running under its existing deadline.
   Completion, failure and cancellation destroy the used environment.

Compose disables external runner networking and exposes only that run's Unix
proxy socket. Kubernetes uses a separate gateway Pod and requires Cilium rules
denying node, Kubernetes API and metadata access. Workflow environments receive
no Docker socket, service-account token, manager token or storage root key.

The gateway permits the selected callbacks, configured storage buckets and
explicit HTTPS integration hosts. The object store enforces prefix permissions
through temporary STS credentials. HTTPS storage must terminate at a bucket-only
gateway because CONNECT hides the request path. Raw TCP/UDP and broad API/PAT
access have no fallback in this execution class.

Cancellation first records a shared marker, then confirms termination. A
Kubernetes API object disappearing does not prove that a partitioned node has
stopped its process. Unconfirmed cleanup closes admission and requires
reconciliation; it must not cause automatic replay of possible external effects.

See [Compose scaling](/self-hosting/docker-compose/scaling/) and the
[Kubernetes executor guide](/self-hosting/kubernetes/executor/) for capacity,
warm reserves, time budgets and operational checks.

## Supported backend values

| Value | Dispatch behavior | Required configuration |
| --- | --- | --- |
| `http` | Posts to a manager or compatible executor's `/execute` or `/execute/sse` endpoint | `EXECUTOR_URL`; manager authentication in `per_run` mode |
| `lambda_invoke` | Uses the AWS SDK with asynchronous `Event` invocation | `lambda` build feature, `LAMBDA_ASYNC_EXECUTOR_FUNCTION` for the native worker or `LAMBDA_EXECUTOR_FUNCTION` for the legacy HTTP worker, AWS region and credentials |
| `lambda_stream` | Uses the AWS SDK response-stream API | `lambda` build feature, function name, region and credentials |
| `kubernetes_job` | Legacy API Job dispatcher, rejected by the isolated Helm deployment | A separately reviewed deployment and runner contract |
| `redis` | Publishes a bounded, retained delivery consumed by the queue bridge | `redis` build feature, authenticated `REDIS_URL`, matching v3 queue settings and consumer |
| `sqs` | Sends the job to an AWS SQS queue | `sqs` build feature, `SQS_EXECUTION_QUEUE_URL` and AWS credentials |
| `kafka` | Posts a record to a Kafka-compatible REST proxy | `KAFKA_BROKERS` as the proxy base URL and `KAFKA_EXECUTION_TOPIC` |
| `sqs_event_bridge` | Stages the payload in object storage, then sends a compact SQS reference for an EventBridge-to-ECS path | `sqs` build feature, staging store, `SQS_EVENT_BRIDGE_EXECUTION_QUEUE_URL`, AWS credentials, and the external Pipe/ECS resources |

Aliases accepted by the parser include `lambda_sdk`, `lambda_streaming`,
`k8s_job`, `isolated`, `redis_queue`, `aws_sqs`, `sqs_ecs`, and `ecs`.
Unknown values fall back to `http`, so validate rendered configuration rather
than relying on a typo to fail closed.

## HTTP executors

`EXECUTOR_URL` can point to:

- The Compose or Kubernetes execution manager, the self-hosted default
- A shared runtime or executor-pool Service in explicit `trusted_shared` mode
- A Lambda Function URL
- Another compatible HTTP execution service

This is the default synchronous backend in the checked-in Compose and Helm
configurations. It supports ordinary dispatch and an SSE endpoint for streamed
state.

For trusted internal workflows, Compose's trusted profile and Helm's
`execution.isolationMode=trusted_shared` with `execution.asyncBackend=http`
enable shared workers. These processes may handle multiple workflows over their
lifetime. That mode does not meet isolation per execution for hostile tenants.

## Kubernetes Job dispatcher

The Kubernetes runner supports `--once` with a signed dispatch on stdin. The
Rust slot adapter feeds that interface in a prepared Pod. This implementation
does not make the legacy API Job dispatcher's environment-based contract the
supported execution path. The Helm chart rejects `kubernetes_job` in `per_run`
mode; use its manager and queue bridge.

## Lambda modes

`lambda_invoke` and `lambda_stream` use AWS SDK clients compiled into the API:

- `lambda_invoke` sends an asynchronous event and returns dispatch metadata.
- `lambda_stream` uses `InvokeWithResponseStream` for a private Lambda.
- A Lambda Function URL can instead be used through the generic `http`
  backend.

The operational and isolation properties are those of the Lambda function and
AWS account configuration. Confirm concurrency, retry, timeout, networking,
and downstream callback behavior for the selected mode.

### Executor architecture

The dispatcher selects the function by name. Lambda's architecture setting and
the container build determine which CPU runs the workflow. The API's platform
settings select the matching compiled WASM artifacts:

| Architecture | Lambda setting | Docker build platform | WASM platform |
| --- | --- | --- | --- |
| ARM64 | `arm64` | `linux/arm64` | `linux-aarch64-wt48` |
| x86-64 | `x86_64` | `linux/amd64` | `linux-x86_64-wt48` |

Set `EXECUTOR_PLATFORM` on the API for the live executor and
`LAMBDA_ASYNC_EXECUTOR_PLATFORM` for the dedicated native background executor.
These settings describe the receiving executor, regardless of the API host's
own architecture. `wt48` identifies the compiled artifact's Wasmtime version.

Both AWS Lambda executor Dockerfiles support ARM64 and x86-64. Build each Lambda
image for one architecture. The SaaS dev Terraform root defaults the live
executor to `executor_architecture = "arm64"` and derives `EXECUTOR_PLATFORM`
from the function, keeping the build, Lambda and artifact settings together.
Changing that variable to `"x86_64"` also rebuilds the matching image. ARM
compiled WASM artifacts must exist for the app's installed packages.

### Native background execution

Use a dedicated function for background workflows so they have their own
concurrency budget and can use a different CPU architecture from live runs:

```bash
EXECUTION_BACKEND=lambda_stream
LAMBDA_EXECUTOR_FUNCTION=flow-executor-fn-dev
EXECUTOR_PLATFORM=linux-aarch64-wt48
ASYNC_EXECUTION_BACKEND=lambda_invoke
LAMBDA_ASYNC_EXECUTOR_FUNCTION=flow-executor-async-fn-dev
LAMBDA_ASYNC_EXECUTOR_PLATFORM=linux-aarch64-wt48
LAMBDA_TENANT_ISOLATION=sub
```

Build `apps/backend/aws/executor-async/Dockerfile` for `linux/arm64` and create
the function with `architectures = ["arm64"]` and `PER_TENANT` isolation.
The background platform setting selects matching compiled WASM packages
without changing the live executor's artifacts.

The API sends one signed workflow payload with `InvocationType=Event` and its
tenant ID. Lambda buffers the event internally and returns HTTP 202 when it
accepts delivery. That response does not mean the workflow has started.
The worker consumes native `DispatchPayloadRef` events and validates the AWS
tenant against the signed subject before executing the workflow. It does not
accept SQS batches or API Gateway events.

Events above 256 KiB are staged in the configured object store. Lambda receives
a presigned reference, keeping both the invocation and its failure record
within their transport limits. Staged payloads are limited to 64 MiB. Without
`LAMBDA_ASYNC_EXECUTOR_FUNCTION`, `lambda_invoke` preserves the existing
API Gateway envelope sent to `LAMBDA_EXECUTOR_FUNCTION`.

The worker requires a durable execution lease and acknowledgement of terminal
API status. A persisted workflow failure completes delivery; transport,
authentication and unacknowledged callback failures return a Lambda error.
Duplicate deliveries still require idempotent external effects, especially
after a process is killed before it can record completion. Existing callbacks
continue to carry run events to live observers; queue time delays their start.

The AWS deployment module defaults to a 900-second hard timeout, an
840-second execution deadline, a 900-second maximum event age and two handler
retries. Setup counts against the execution deadline. The worker allows
30 seconds for cleanup, then retires the runtime if work cannot stop.
Keep queue age plus execution within credential and WASM URL lifetimes.
Retain the default `STS_SESSION_TTL_SECONDS=3600` for this queue window;
the API refreshes cached execution grants with less than 31 minutes remaining
and rejects freshly issued grants that cannot cover that window. WASM URLs
currently expire after one hour. The configured 25 reserved
executions cap background concurrency without provisioning warm capacity.
This cap does not provide scheduling fairness between tenants.

Discarded events and exhausted failures go to an SQS failure destination with
delivery, event-age and queue alarms. Operators must inspect and reconcile
these records. The failure destination is not part of normal dispatch.
Keep the previous ECS queue and Pipe until accepted messages and running tasks
have drained. Switching the API backend does not migrate queued jobs.

Direct SQS event-source mappings cannot supply the per-message Lambda tenant
parameter. Using SQS with `PER_TENANT` execution requires a separate trusted
router that invokes each tenant's work. See the
[AWS tenant-isolated invocation documentation](https://docs.aws.amazon.com/lambda/latest/dg/tenant-isolation-invoke.html).

### Tenant isolation

Lambda runs every execution in a Firecracker microVM, so a run is isolated from
the host and from the runs beside it. It is not isolated from the runs before
it: by default a warm execution environment is reused across invocations of the
same function whoever triggered them, carrying process memory and its `/tmp`
scratch directory across that reuse.

`LAMBDA_TENANT_ISOLATION=sub` makes the API send a per-subject tenant id with
each `lambda_invoke` and `lambda_stream` dispatch, and AWS then binds an
execution environment to a single tenant instead of reusing it for another.

```bash
LAMBDA_TENANT_ISOLATION=sub
```

The subject is not transmitted. The tenant id is a domain-separated BLAKE3
digest of it, so federated subjects containing characters AWS rejects still
produce a valid id, and no user identifier reaches CloudWatch. The mapping is
logged by the API at `debug` level, which is the only place a tenant id can be
traced back to a run.

Accepted values are `sub` (equivalently `user`, `user_id`, `true`, `1`, `on`,
`enabled`) and `off` (equivalently `false`, `0`, `none`, `disabled`, or unset).
Unlike `EXECUTION_BACKEND`, an unrecognized value is rejected rather than
treated as `off`. A typo that silently disabled isolation would leave the
deployment looking correctly configured.

Before enabling it, confirm all of the following:

- The executor function was **created** with
  `TenancyConfig.TenantIsolationMode=PER_TENANT`. The property is create-only:
  it cannot be added to an existing function, so adopting tenant isolation means
  replacing the function. Enabling the flag against a function without it makes
  every dispatch fail with `InvalidParameterValueException`.
- The backend is `lambda_invoke` or `lambda_stream`. The flag has no effect on
  `http`, and Lambda Function URLs do not support tenant isolation at all.
- Your run volume fits the quota. AWS caps tenant-bound execution environments
  at 2,500 per 1,000 configured concurrency and returns `TooManyRequestsException`
  beyond it. Cardinality follows the subject: runs triggered by sinks or inbound
  events carry a per-sink or per-event identity rather than a user, and API keys
  share their creator's identity.
- You accept the cost profile. Warm capacity is no longer shared, so cold starts
  rise, each environment creation is billed, and neither provisioned concurrency
  nor SnapStart can be used to mitigate.

Tenant isolation is defence in depth, not an authorization boundary: AWS
publishes no IAM condition key for the tenant id, and all tenants share the
function's execution role. Per-run authorization remains the executor JWT's job.

## Queue backends

Queue transports decouple API response time from worker execution:

- **Redis** atomically publishes and claims retained v3 deliveries. The queue
  bridge forwards a signed dispatch to the manager and settles it only after
  independently confirming durable terminal API state.
- **SQS** sends a complete serialized request to the configured queue.
- **Kafka** uses an HTTP REST proxy rather than an embedded Kafka client.
- **SQS + EventBridge + ECS** stores the full payload first and queues a signed
  reference, avoiding ECS container-override payload limits.

The Compose and Kubernetes deployments include the Redis consumer. Explicit
non-admission can requeue with backoff and the original publication time.
Expired or ambiguous work remains retained for reconciliation. SQL run creation
and Redis publication do not share a transactional outbox; retained delivery
does not promise exactly-once external effects. Preserve queue and replay state
across upgrades, and drain or reconcile older queues before switching every
producer and consumer to v3.

## Choosing a backend

| Need | Start with | Verify before production |
| --- | --- | --- |
| Untrusted workflows on one Linux Docker host | Default `per_run` manager over `http`, background work over `redis` | gVisor Unix proxy access, storage-prefix denial, replay state and host resources |
| Untrusted workflows across Kubernetes nodes | Default `per_run` manager and Redis queue bridge | Installed gVisor, Cilium enforcement, Pod termination, warm replacement rate and node resources |
| Trusted internal workflows | Explicit `trusted_shared` HTTP pool | Shared-process behavior, worker concurrency and integration access |
| Private streaming Lambda | `lambda_stream` | AWS feature build, response streaming, timeouts, concurrency |
| Tenant-isolated AWS background Lambda | `lambda_invoke` with a dedicated native worker | `PER_TENANT`, matching artifact architecture, concurrency, retries, failure destination and callback reachability |
| Long AWS container task | `sqs_event_bridge` | Staging-store lifetime, signed URL scope, Pipe and ECS task configuration |
| Existing Kafka platform | `kafka` | REST proxy compatibility, authentication, partitions, consumer contract |

Benchmark with your own Flow, image size, region, cluster, and concurrency.
The repository does not define universal latency or cost numbers for these
backends.

## Common configuration

```bash
# HTTP
EXECUTION_BACKEND=http
EXECUTOR_URL=http://execution-gateway:9000
EXECUTION_ISOLATION_MODE=per_run
# The deployment generator supplies the private manager token.

# Redis for background runs
ASYNC_EXECUTION_BACKEND=redis
# Use the authenticated REDIS_URL generated for the API.
REDIS_EXECUTION_QUEUE=exec:jobs:v3

# AWS Lambda SDK
LAMBDA_EXECUTOR_FUNCTION=arn:aws:lambda:eu-central-1:123456789012:function:flow-like-executor
AWS_REGION=eu-central-1

# Optional: one execution environment per subject. Requires a function created
# with TenancyConfig.TenantIsolationMode=PER_TENANT.
LAMBDA_TENANT_ISOLATION=sub
```

The HTTP URL above is the Compose gateway. Helm supplies its release-specific
manager Service address. Keep credentials in your platform's secret store. Environment-variable names
belong in documentation and configuration; their secret values do not.
