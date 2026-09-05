---
title: Configuration
description: How Helm values become runtime configuration for the Kubernetes backend.
sidebar:
  order: 20
---

Helm values configure workloads and runtime environment variables. The API's
public hub and OIDC configuration is selected separately at image build time.

| Source | Purpose |
| --- | --- |
| `helm/values.yaml` | Chart defaults |
| `helm/values-production.yaml` | Example operator overrides |
| `.generated/values-generated.yaml` | Setup-generated endpoint settings and Secret references |
| `.generated/values-images.yaml` | Build-generated image references and required digests |
| `.generated/secrets.yaml` | Private credentials, applied separately |
| `FLOW_LIKE_CONFIG` | Repository-relative JSON embedded in the API image |

Paths above are relative to `apps/backend/kubernetes/`, except the path supplied
to `FLOW_LIKE_CONFIG`. Setup reads exported environment variables as data; it
does not source a `.env` file. Changing Helm values does not rebuild the embedded
hub configuration.

## Execution capacity

The default mode prepares clean, single-use Pod pairs:

```yaml
execution:
  isolationMode: per_run
  backend: http
  asyncBackend: redis
  queueName: exec:jobs:v3
  queueMaxWaitSeconds: 300
  credentialMarginSeconds: 120

executionManager:
  replicaCount: 2
  workerThreads: 2
  maxConcurrentExecutions: 20
  warmPoolSize: 4
  warmPoolCreationConcurrency: 2
  warmPoolMaxAgeSeconds: 600
  sandbox:
    memoryMb: 1024
    cpus: 1
    tmpMb: 256
    nodeSelector:
      flow-like.io/execution: "true"
  queueBridge:
    replicaCount: 2
    concurrency: 20
```

This example configures up to 40 active executions and eight additional clean
slots across the two managers, subject to resources and slot availability. Each
slot also has a separate gateway Pod. Provision execution nodes before raising
these values.

`workerThreads` sets each manager's Tokio worker count; it does not set workflow
concurrency. `queueBridge.concurrency` limits jobs a bridge can hold while waiting
or executing. A large queue-consumer count does not create runner capacity.

## Time budgets

| Helm value | Default | Runtime variable |
| --- | ---: | --- |
| `executor.timeout` | 3600 s | `EXECUTION_TIMEOUT_SECONDS` |
| `executionManager.startupGraceSeconds` | 30 s | `EXECUTION_STARTUP_GRACE_SECONDS` |
| `executionManager.terminalGraceSeconds` | 60 s | `EXECUTION_TERMINAL_GRACE_SECONDS` |
| `executionManager.cleanupTimeoutSeconds` | 30 s | `EXECUTION_CLEANUP_TIMEOUT_SECONDS` |
| `execution.queueMaxWaitSeconds` | 300 s | `EXECUTION_QUEUE_MAX_WAIT_SECONDS` |
| `execution.credentialMarginSeconds` | 120 s | `EXECUTION_CREDENTIAL_MARGIN_SECONDS` |
| `storage.s3.stsSessionTtlSeconds` | 7200 s | `STS_SESSION_TTL_SECONDS` |

The API checks actual remaining credential lifetime against the execution, queue,
supervisor and safety budgets. The defaults require 4,140 seconds at checkout.
Requesting a longer STS session does not help if the provider returns a shorter
one. Adjust ingress and load-balancer timeouts when changing execution duration.

## Storage and Secrets

`storage.provider=s3` and `rustfs.enabled=true` select the bundled RustFS setup.
Public, internal and STS origins are distinct settings:

- `storage.s3.publicEndpoint`: signed object URLs used by browsers and runtimes.
- `storage.s3.internalEndpoint`: API LanceDB access; ordinary signed object requests use the public origin.
- `storage.s3.stsEndpoint`: private temporary-credential issuance.

Setup generates separate root, API and STS identities. Keep Secret values out of
ordinary values files and command-line `--set` arguments. See
[Storage](/self-hosting/kubernetes/storage/) and
[Secret contracts](/self-hosting/kubernetes/helm/#existing-secret-contracts).

Bundled Redis requires both `REDIS_PASSWORD` and a complete, URL-encoded
`REDIS_URL` in `redis.auth.existingSecret`. External Redis uses
`redis.enabled=false` with `redis.externalExistingSecret`; that Secret contains
`REDIS_URL`. Use `rediss://` and a trusted certificate chain for TLS.

## API, database and integrations

Configure API replicas with `api.replicaCount`, or enable `api.autoscaling` and
let the HPA own the count. Size `database.pool.maxConnections` and
`database.pool.minConnections` per API process; include maximum replicas and
rollout surge in the database's total connection budget.

Add extra configuration through `api.env` and `api.envFrom`. Avoid duplicating
chart-owned names such as `DATABASE_URL`, `REDIS_URL` or `BACKEND_KEY`.

Hosted-model provider credentials belong to the API's authenticated model proxy.
The chart's `llm.*` Secret references configure that proxy. Runner Pods receive
execution capabilities, not installation-wide model-provider keys. Grant direct
HTTPS integrations with exact hostnames in
`executionManager.allowedHttpsHosts`. Integration destinations that resolve to
private or reserved addresses are rejected even if a NetworkPolicy permits them.
Use `networkPolicy.executionGatewayExtraEgress` for required private object-store
endpoints.

For multiple signaling replicas, set `signaling.fanoutMode=redis`, exact browser
origins in `signaling.allowedOrigins`, and the deployment's WSS endpoint in the
embedded hub configuration.

## Review effective configuration

```bash
helm get values flow-like -n flow-like
kubectl describe deployment flow-like-api -n flow-like
kubectl describe deployment flow-like-execution-manager -n flow-like
```

Use the [deploy helper](/self-hosting/kubernetes/scripts/#deploysh) to validate the
same ordered values files before applying an update. For trusted-only local
workflows, `execution.isolationMode=trusted_shared` and
`execution.asyncBackend=http` select the reusable executor pool. That mode does
not provide per-execution tenant isolation.
