---
title: Configuration
description: How Helm values become runtime configuration for the Kubernetes backend.
sidebar:
  order: 20
---

Helm values configure workloads and select the API's complete runtime hub/OIDC
document. Installation-specific API and web settings can change without rebuilding
the images.

| Source | Purpose |
| --- | --- |
| `helm/values.yaml` | Chart defaults |
| `helm/values-production.yaml` | Example operator overrides |
| `.generated/values-generated.yaml` | Setup-generated endpoint settings and Secret references |
| `.generated/values-images.yaml` | Image digests resolved from the published packages or recorded by a local build |
| `.generated/secrets.yaml` | Private credentials, applied separately |
| `FLOW_LIKE_CONFIG_FILE` | Host-side JSON file read by setup into a generated Kubernetes Secret |

Paths above are relative to `apps/backend/kubernetes/`. Setup reads exported
environment variables as data; it does not source a `.env` file.

## Images

Each first-party `*.image` map has `repository`, `tag`, `digest` and
`pullPolicy`. Defaults point at the public `ghcr.io/rheosoph` packages at the
`dev` tag; a digest pins one manifest and wins over the tag. Run
`scripts/resolve-images.py --tag <release>` to pin all images, or
`scripts/build-images.sh` to build them, and let `deploy.sh` apply the
resulting file. `global.imageRegistry` prefixes every repository for mirrors;
`global.imagePullSecrets` lists the `docker-registry` Secrets for private
packages and reaches the API-created execution Jobs and sink CronJobs as well.
See [Helm chart images](/self-hosting/kubernetes/helm/#images).

## Runtime API and web configuration

`setup-config.sh` stores the selected JSON in a separate `hub-config` Secret
inside its private output and points generated Helm values at it. The API mounts
the Secret read-only at `/etc/flow-like/flow-like.config.json`. Setup also accepts
`FLOW_LIKE_RUNTIME_CONFIG_FILE`, the older repository-relative
`FLOW_LIKE_CONFIG`, or raw `FLOW_LIKE_CONFIG_JSON`. With no input it uses the
self-hosting example; replace its OIDC placeholders before deployment.

Select at most one source under `api.runtimeConfig`:

| Value | Source |
| --- | --- |
| `existingSecret` | Read-only Secret volume |
| `existingConfigMap` | Read-only ConfigMap volume for public settings |
| `secretKeyRef.name` and `.key` | JSON from an existing Secret key in `FLOW_LIKE_CONFIG_JSON` |
| `secretRef` | Reference resolved by the API's configured SecretStore |

Volume sources use `api.runtimeConfig.key`, which defaults to
`flow-like.config.json`. Create and maintain external Secrets/ConfigMaps through
the installation's normal configuration process. Do not put credential-bearing
JSON in values files or duplicate `FLOW_LIKE_CONFIG_*` through `api.env` or
`api.envFrom`. Setup accepts `FLOW_LIKE_CONFIG_SECRET_REF` and stores only the
reference; its target must be available to the API separately.

The API reads the whole document once. After changing an external Secret,
ConfigMap or SecretStore value, restart the API Deployment. Updating the resource
alone does not roll out Pods. Changing the selected reference through Helm does.
Empty source values leave the embedded fallback available; conflicting nonempty
sources stop startup. See the [runtime API contract](/self-hosting/containers/#runtime-api-configuration)
for validation, limits and secret handling.

For an explicit custom fallback build, `build-images.sh` accepts the
repository-relative `FLOW_LIKE_BUILD_CONFIG`. Use only public settings because
the document is embedded in the executable.

The web image reads `web.runtimeConfig.apiUrl`, falling back to `api.publicUrl`,
and optional `redirectUrl`/`logoutUrl` at container startup. Omitted redirect and
logout settings use the browser origin with `/callback` and `/`. Helm changes
roll out the same web image. Supply public HTTP(S) URLs without credentials,
queries or fragments. See [Runtime web configuration](/self-hosting/containers/#runtime-web-configuration)
for the separate static-metadata and third-party OAuth relay limits.

`api.frontendBaseUrl` points hosted Event shortlinks to the same web deployment.
Setup derives it from `PUBLIC_WEB_URL`, defaulting to `http://localhost:3001`.
When API and web share an origin, route `/c`, `/f` and `/u` to web and
`/frontend` to API. See [Hosted frontends](/self-hosting/containers/#host-chat-form-and-page-frontends)
for the static rewrites and existing login callback.

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
runtime hub configuration. See [Realtime signaling](/self-hosting/signaling/)
for authentication, fanout and client/server rollout compatibility.

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
