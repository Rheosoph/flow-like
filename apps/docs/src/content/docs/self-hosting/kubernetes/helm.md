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
| Audit worker | Separate Deployment, database role and service account |
| Rust execution manager | One replica, ten active runs, two additional warm slots |
| Queue bridge | One Redis consumer with concurrency ten |
| Single-use runner and gateway | Created dynamically for each warm slot |
| RustFS | One persistent Pod, initializer and two public data gateway replicas |
| Redis | Authenticated single instance with persistence |
| Internal CockroachDB | One evaluation node with TLS and password authentication |
| Database migration | Release-specific Job |
| Prometheus, Grafana and Tempo | Enabled |
| Compiler, signaling, sink services and public ingress | Optional |

The `executorPool` Deployment is rendered only in `trusted_shared` mode. Its
values do not control isolated capacity. Default values require generated
credentials and public endpoints before installation; images default to the
published `dev` tag and isolated mode additionally requires digests.

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
| `database.apiExistingSecret` | API `DATABASE_URL` for bundled SQL |
| `database.migration.existingSecret` | Schema-owner `DATABASE_URL` |
| `audit.database.existingSecret` | Worker `DATABASE_URL` |
| `audit.entrySecret` | `AUDIT_ENTRY_KEY`, optionally `AUDIT_ENTRY_KEY_PREVIOUS` |
| `audit.existingSecret` | `AUDIT_SIGNING_KEY`, or the chosen key service's credentials |
| `audit.bucketSecret` | `AUDIT_BUCKET_ACCESS_KEY_ID`, `AUDIT_BUCKET_SECRET_ACCESS_KEY` |

Setup generates these contracts together. `BACKEND_KEY` and `BACKEND_PUB`
contain base64-encoded ES256 PEM material; Kubernetes Secret encoding is a
separate layer. Executors receive the public material only.

External Redis requires `redis.enabled=false`. External SQL requires
`database.type=external`; set `database.external.provider` to `postgresql`
or `cockroachdb`. The default bundled database must remain one node.

### Audit worker and SQL isolation

The API ingests audit records and verifies public signatures. The separate worker
seals records, signs checkpoints and writes archives. Its signing key or transit
token, bucket credentials and SQL login never enter the API Pod. The object-store
initializer receives only the bucket credentials. New bundled audit buckets use
Object Lock in `COMPLIANCE` mode with four years of retention.

Setup generates a shared entry key and separate API, worker and migration SQL
credentials. The migration Job applies the schema, then grants the API read access
to audit evidence and permission to insert new records. Only the worker can change
existing evidence. For external SQL, provide `DATABASE_URL` for the schema owner,
`API_DATABASE_URL` and `AUDIT_DATABASE_URL` with distinct users and passwords, all
using `sslmode=verify-full`. A private CA can be supplied through `DATABASE_CA_FILE`;
include `sslrootcert=/etc/database/ca.crt` in those URLs.

Bundled CockroachDB requires the generated TLS Secrets. The node key is mounted
only by the database, and the root client key only by its initialization Job. API
and worker Pods receive the public CA certificate. Node and root client
certificates expire after one year. Retain the generated `database-ca-admin`
Secret, which contains the CA signing key and is never mounted by a workload,
for operator-led certificate renewal.

An existing installation using `--insecure` needs a planned change to TLS before
this chart can start its database. Preserve the database volume and existing
credentials, provision the separate TLS and runtime-role Secrets, and review the
rendered migration and database workloads before upgrading. Removing TLS settings
does not restore insecure mode.

The worker's audit policy is the `audit` section of the `flow-like.config.json`
compiled into its image, from the same `FLOW_LIKE_CONFIG` build input as the API
image (`FLOW_LIKE_BUILD_CONFIG` in `scripts/build-images.sh`). It needs no config
Secret or mount. `api.runtimeConfig` overrides the API's configuration only and
never reaches the worker, so a changed audit policy needs a rebuilt worker image.
The worker waits for the migration Job of its revision before it starts; that Job
owns and verifies the SQL boundary between the API and worker logins. To pause
the worker, set `audit.replicaCount=0`: nothing is sealed or archived while it is
scaled down, and grants and credentials are untouched.

`audit.worker=false` stops the dedicated worker; it does not start one inside the
API. The chart rejects `sinkServices.enabled` while the worker is enabled because
the API's CronJob creation permission could otherwise mount the worker's Secrets.
Isolate that scheduler before combining these features. An external key service
on a private address or nonstandard port needs a matching `audit.extraEgress`
rule. See [Audit trail](/self-hosting/audit-trail/) for checkpoint retention and
independent verification.

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

## Images

Every first-party image is a map with `repository`, `tag`, `digest` and
`pullPolicy`:

| Value | Published repository |
| --- | --- |
| `api.image` | `ghcr.io/rheosoph/flow-like-kubernetes-api` |
| `audit.image` | `ghcr.io/rheosoph/flow-like-audit-worker` |
| `web.image` | `ghcr.io/rheosoph/flow-like-kubernetes-web` |
| `executor.image`, `executorPool.image` | `ghcr.io/rheosoph/flow-like-kubernetes-executor` |
| `executionManager.image` | `ghcr.io/rheosoph/flow-like-kubernetes-execution-manager` |
| `database.migration.image` | `ghcr.io/rheosoph/flow-like-kubernetes-migration` |
| `sinkServices.image` | `ghcr.io/rheosoph/flow-like-kubernetes-sink-trigger` |
| `executionManager.queueBridge.image` | `ghcr.io/rheosoph/flow-like-docker-compose-runtime` |
| `compiler.image` | `ghcr.io/rheosoph/flow-like-docker-compose-compiler` |
| `signaling.image` | `ghcr.io/rheosoph/flow-like-docker-compose-signaling` |
| `rustfs.bootstrap.image` | `ghcr.io/rheosoph/flow-like-docker-compose-object-store-init` |

Defaults use the mutable `dev` tag with `pullPolicy: Always`. A non-empty
`digest` must be `sha256:` followed by 64 hex characters; the chart then renders
`repository@digest` and ignores the tag. `scripts/resolve-images.py` fills the
digests for one published tag and `scripts/build-images.sh` records them after a
push; both write `.generated/values-images.yaml`, which `deploy.sh` applies
after every operator file.

In isolated mode, `executionManager.image.digest` pins the manager and
enforcement gateway, while `executionManager.sandbox.image` is a full
`repository@sha256:...` executor reference. Rebuild and push the manager and
executor together when changing their protocol. The runner image contains the
Rust slot adapter.

`global.imageRegistry` is a literal repository prefix; include its trailing
slash when setting it manually. Generated files set this prefix to an empty
string because their repositories are complete. `values-production.yaml` shows
the mirror pattern: prefix `registry.example.com/`, the published repository
names and filled digests. Use `global.imagePullSecrets` for forks, private
mirrors and the private Cloud packages; the API forwards the same names to the
execution Jobs it creates (`K8S_IMAGE_PULL_SECRETS`) and to sink CronJobs
(`SINK_IMAGE_PULL_SECRETS`), and the manager forwards them to sandbox Pods.
Those Jobs and CronJobs are created in the release namespace (`K8S_NAMESPACE`
is always set from the Pod's namespace), where the chart's RBAC and pull
Secrets live.

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
  -f values-operator.yaml
```

The helper starts with `.generated/values-generated.yaml`, appends
`.generated/values-images.yaml` after the arguments, lints and renders the
ordered values, checks Cilium and waits for the release workloads and
initialization Jobs. Apply generated Secrets separately before the first
install. It does not create the namespace or rotate credentials.

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
