---
title: Helm Chart
description: Current components, values, and Secret contracts for the Flow-Like Helm chart.
sidebar:
  order: 50
---

The chart in `apps/backend/kubernetes/helm/` deploys the application and its
execution infrastructure. Keep non-secret operator settings in reviewed values
files and apply credentials as existing Kubernetes Secrets.

## What the chart deploys

| Component | Default |
| --- | --- |
| API and web | One replica each |
| Rust execution manager | One replica, ten active runs, two additional warm slots |
| Queue bridge | One Redis consumer with concurrency ten |
| Single-use runner and gateway | Created dynamically for each warm slot |
| RustFS | One persistent Pod, initializer and two public data gateway replicas |
| Redis | Authenticated single instance with persistence |
| Internal CockroachDB | One evaluation node |
| Database migration | Release-specific Job |
| Prometheus, Grafana and Tempo | Enabled |
| Compiler, signaling, sink services and public ingress | Optional |

The `executorPool` Deployment is rendered only in `trusted_shared` mode. Its
values do not control isolated capacity. Default values require generated
credentials, public endpoints and real image references before installation.

## Execution values

```yaml
execution:
  isolationMode: per_run
  backend: http
  asyncBackend: redis
  queueName: exec:jobs:v3

executionManager:
  replicaCount: 1
  workerThreads: 2
  maxConcurrentExecutions: 10
  warmPoolSize: 2
  warmPoolCreationConcurrency: 2
  warmPoolMaxAgeSeconds: 600
  startupGraceSeconds: 30
  terminalGraceSeconds: 60
  cleanupTimeoutSeconds: 30

executor:
  timeout: 3600

runtimeClass:
  create: false
  name: runsc
  handler: runsc

networkPolicy:
  enabled: true
```

Install gVisor and Cilium before applying these values. The chart's isolated
policy resources require the Cilium CRD and deny node, Kubernetes API and
metadata egress. It refuses isolated deployment with disabled NetworkPolicy,
missing image digests or incompatible dispatch settings.

Manager replicas each maintain their own additional reserve. Set
`executionManager.sandbox` resource limits and node placement to match the
execution nodes. See [Executor](/self-hosting/kubernetes/executor/) for the
relationship between concurrency, reserve, preparation rate and throughput.

## Existing Secret contracts

All referenced Secrets must exist in the release namespace.

| Helm reference | Required keys |
| --- | --- |
| `jwt.existingSecret` | `BACKEND_KEY`, `BACKEND_PUB`, `BACKEND_KID` |
| `api.existingSecret` | `SINK_TOKEN_ENCRYPTION_KEY`, `SINK_SECRET`, `MAINTENANCE_TOKEN` |
| `execution.existingSecret` | `EXECUTION_MANAGER_TOKEN` |
| `rustfs.existingSecret` | `RUSTFS_ROOT_USER`, `RUSTFS_ROOT_PASSWORD` |
| `storage.s3.existingSecret` | `AWS_ACCESS_KEY_ID`, `AWS_SECRET_ACCESS_KEY`, `STS_ISSUER_ACCESS_KEY`, `STS_ISSUER_SECRET_KEY` |
| `redis.auth.existingSecret` | `REDIS_PASSWORD` and complete URL-encoded `REDIS_URL` |
| `redis.externalExistingSecret` | Complete authenticated `REDIS_URL` |
| `database.external.existingSecret` | `DATABASE_URL` |

Setup generates these contracts together. `BACKEND_KEY` and `BACKEND_PUB`
contain base64-encoded ES256 PEM material; Kubernetes Secret encoding is a
separate layer. Executors receive the public material only.

External Redis requires `redis.enabled=false`. External SQL requires
`database.type=external`; set `database.external.provider` to `postgresql`
or `cockroachdb`. The default bundled database must remain one node.

### Object storage

Bundled storage uses `storage.provider=s3` and `rustfs.enabled=true`.
The public, internal and STS origins, bucket names and session lifetime are values;
the root, API and issuer credentials are separate Secrets. For external storage,
set `rustfs.enabled=false` and supply all endpoints and a provider with the
required session-policy enforcement.

The isolated chart supports this S3 path. The AWS, Azure, GCP and R2 provider
blocks remain available in trusted shared mode. Follow
[Storage](/self-hosting/kubernetes/storage/) for their boundaries and the R2
temporary-token requirement.

### Hosted model providers

The `llm.*.existingSecret` values configure the API's hosted-model proxy.
For example, OpenRouter uses `OPENROUTER_API_KEY` and optionally
`OPENROUTER_ENDPOINT`; OpenAI uses `HOSTED_OPENAI_API_KEY` and optionally
`HOSTED_OPENAI_ENDPOINT`. These installation-wide keys do not enter isolated runners.

## Images and builds

`scripts/build-images.sh` writes values for every application image. In isolated
mode, `executionManager.image.digest` pins the manager and enforcement gateway,
while `executionManager.sandbox.image` is a full
`repository@sha256:...` executor reference.

Rebuild and push the manager and executor together when changing their protocol.
The runner image contains the Rust slot adapter. Apply the generated image values
last so example tags or empty digests cannot override them.

`global.imageRegistry` is a literal repository prefix; include its trailing
slash when setting it manually. Generated full repository references set this
prefix to an empty string. Use `global.imagePullSecrets` for private registries.

## Public ingress

API and web ingress share the chart's `ingress` block:

```yaml
ingress:
  enabled: true
  className: nginx
  hosts:
    - host: api.example.com
      paths:
        - {path: /, pathType: Prefix, service: api}
    - host: app.example.com
      paths:
        - {path: /, pathType: Prefix, service: web}
  tls:
    - secretName: flow-like-tls
      hosts: [api.example.com, app.example.com]
```

Object data ingress is configured separately under `rustfs.gateway.ingress`.
Preserve its signed host and path. Keep internal STS, manager, database, Redis
and metrics endpoints private.

The chart derives nginx streaming timeouts from execution duration. Other ingress
controllers and upstream load balancers need equivalent settings.

## Render, deploy and upgrade

From `apps/backend/kubernetes/`:

```bash
./scripts/deploy.sh \
  -f helm/values-production.yaml \
  -f .generated/values-generated.yaml \
  -f values-operator.yaml \
  -f .generated/values-images.yaml
```

The helper lints and renders the ordered values, checks Cilium and waits for the
release workloads and initialization Jobs. Apply generated Secrets separately
before the first install. It does not create the namespace or rotate credentials.

Keep Redis replay claims, cancellation records and persistent storage during
upgrades. Drain accepted work before changing queue protocols; rebuild both
pinned manager and executor images when their wire protocol changes. Back up and
review SQL schema changes before replacing API images.

```bash
helm status flow-like -n flow-like
helm get values flow-like -n flow-like
helm history flow-like -n flow-like
kubectl get events -n flow-like --sort-by=.lastTimestamp
```

Follow [Installation](/self-hosting/kubernetes/installation/) for the complete
initial setup and [Monitoring](/self-hosting/kubernetes/monitoring/) for scrape
configuration and current monitoring limitations.
