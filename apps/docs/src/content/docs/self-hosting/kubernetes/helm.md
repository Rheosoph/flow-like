---
title: Helm Chart
description: Current components, values, and Secret contracts for the Flow-Like Helm chart.
sidebar:
  order: 50
---

The chart source is `apps/backend/kubernetes/helm/`. Use a version-controlled
non-secret values file for deployment settings and externally managed
Kubernetes Secrets for credentials.

## What the Chart Deploys

| Component | Default | Resource |
| --- | --- | --- |
| Web application | Enabled, 1 replica | Deployment and Service |
| API | Enabled, 1 replica | Deployment and Service |
| Warm executor pool | Enabled, 1 replica | Deployment and Service |
| Per-run execution | Incomplete | API can create a Job; the checked-in executor's job runner is pending |
| Internal CockroachDB | Enabled, single node | StatefulSet, Services, and initialization Job |
| Redis | Enabled, standalone with authentication | Deployment, Service, Secret, and PVC |
| Database migration | Enabled | Helm install/upgrade Job |
| Prometheus, Grafana, and Tempo | Enabled | Monitoring Deployments, Services, and storage |
| WASM compiler | Disabled | Deployment and Service |
| Ingress | Disabled | Ingress |
| Sink services | Disabled | CronJob and optional bot workloads |

API and executor autoscaling are configurable but disabled by default.

The internal CockroachDB workload uses `start-single-node --insecure`. It is an
evaluation and development option, not a production database topology. Select
`database.type: external` for production.

## Required Overrides

The default file is optimized for repository-local k3d development:

- Application images use the local `k3d-flow-like.localhost:5000` registry and
  `pullPolicy: Never`.
- `storage.provider` defaults to `azure`, but credentials are intentionally
  empty.
- Backend execution keys are intentionally empty.

Consequently, the default values alone are not an installable production
configuration. Override the application images, storage provider, storage
credentials, and JWT Secret.

## Core Values

```yaml
api:
  enabled: true
  replicaCount: 1
  autoscaling:
    enabled: false

web:
  enabled: true
  replicaCount: 1
  autoscaling:
    enabled: false

executorPool:
  enabled: true
  replicaCount: 1
  autoscaling:
    enabled: false

execution:
  backend: http
  asyncBackend: http
  executorUrl: ""

database:
  type: internal
  internal:
    replicas: 1
    persistence:
      size: 10Gi

redis:
  enabled: true
  auth:
    enabled: true
  master:
    persistence:
      enabled: true
      size: 8Gi

monitoring:
  enabled: true

ingress:
  enabled: false
```

The values accept `http`, `redis`, and `kubernetes_job`. With only the
checked-in chart workloads, use `http` for both synchronous and asynchronous
lanes:

- the executor pool implements HTTP;
- it does not consume the Redis execution list;
- the one-job executor entrypoint used by `kubernetes_job` is not implemented.

Use `redis` only with a separately deployed compatible queue consumer.
`execution.executorUrl` is automatically derived from the warm executor
Service when left empty.

## Storage Providers

Select exactly one provider and configure its matching block:

| `storage.provider` | Values block | Intended service | Stock API status |
| --- | --- | --- | --- |
| `aws` | `storage.aws` | Native AWS S3, including optional STS role | API must be rebuilt with its `aws` feature |
| `azure` | `storage.azure` | Azure Blob Storage | Included |
| `gcp` | `storage.gcp` | Google Cloud Storage | Included |
| `r2` | `storage.r2` | Cloudflare R2 | Included; generated Secret lacks `R2_API_TOKEN` |
| `s3` | `storage.s3` | Generic S3-compatible endpoint | API must be rebuilt with its `aws` feature; scoped STS compatibility is not guaranteed |

For example, an existing Azure Secret is selected with:

```yaml
storage:
  provider: azure
  azure:
    existingSecret: flow-like-storage
```

The chart also accepts credential values directly and will create a storage
Secret. Keep those values in an uncommitted, access-controlled secrets file;
do not pass long-lived credentials as `--set` arguments.

See [Storage Configuration](/self-hosting/kubernetes/storage/) for the current
provider boundaries and complete examples.

## Existing Secret Contracts

`existingSecret` always names a Secret in the release namespace.

### Backend Execution Keys

```yaml
jwt:
  existingSecret: flow-like-backend-jwt
```

Required keys:

- `BACKEND_KEY`: base64-encoded ES256 private-key PEM
- `BACKEND_PUB`: base64-encoded ES256 public-key PEM
- `BACKEND_KID`: identifier published with the signing key

The API receives the private and public material. Executors and the optional
compiler read the public key and key identifier from the same Secret.

### External Database

```yaml
database:
  type: external
  external:
    existingSecret: flow-like-database
```

Required key: `DATABASE_URL`.

### Redis Authentication

```yaml
redis:
  auth:
    enabled: true
    existingSecret: flow-like-redis
```

Required key: `REDIS_PASSWORD`. The chart uses it for the Redis server and
constructs the API's in-cluster `REDIS_URL`.

### Object Storage

Every externally managed storage Secret should include
`STORAGE_PROVIDER` plus the provider-specific credentials and logical storage
names:

| Provider | Secret keys |
| --- | --- |
| AWS | `AWS_REGION`, `META_BUCKET`, `CONTENT_BUCKET`, `LOG_BUCKET`, `RUNTIME_ROLE_ARN`; either static `AWS_ACCESS_KEY_ID` and `AWS_SECRET_ACCESS_KEY` or workload identity, plus optional `AWS_ENDPOINT` and `AWS_USE_PATH_STYLE` |
| Azure | `AZURE_STORAGE_ACCOUNT_NAME`, `AZURE_STORAGE_ACCOUNT_KEY`, `AZURE_META_CONTAINER`, `AZURE_CONTENT_CONTAINER`, `AZURE_LOG_CONTAINER`, `META_BUCKET`, `CONTENT_BUCKET` |
| GCP | `GCP_PROJECT_ID`, `GOOGLE_APPLICATION_CREDENTIALS_JSON`, `GCP_META_BUCKET`, `GCP_CONTENT_BUCKET`, `GCP_LOG_BUCKET`, `META_BUCKET`, `CONTENT_BUCKET`, `LOG_BUCKET` |
| R2 | `STORAGE_PROVIDER=aws`, `RUNTIME_CREDENTIALS_PROVIDER=r2`, `R2_ACCOUNT_ID`, `R2_API_TOKEN`, `R2_ACCESS_KEY_ID`, `R2_SECRET_ACCESS_KEY`, the matching `AWS_ENDPOINT`, `AWS_REGION=auto`, `AWS_ACCESS_KEY_ID`, `AWS_SECRET_ACCESS_KEY`, `AWS_USE_PATH_STYLE`, and `META_BUCKET`, `CONTENT_BUCKET`, `LOG_BUCKET` |
| Generic S3 | `S3_ENDPOINT`, `S3_REGION`, `S3_ACCESS_KEY_ID`, `S3_SECRET_ACCESS_KEY`, `S3_USE_PATH_STYLE`, matching `AWS_ENDPOINT`, `AWS_REGION`, `AWS_ACCESS_KEY_ID`, `AWS_SECRET_ACCESS_KEY`, `AWS_USE_PATH_STYLE`, and `META_BUCKET`, `CONTENT_BUCKET`, `LOG_BUCKET` |

The chart-generated R2 and generic S3 Secrets add the AWS aliases
automatically. The R2 template does **not** add `R2_API_TOKEN`, so use an
externally managed Secret when scoped credentials are required. Explicit empty
environment values are not treated as absent; for AWS workload identity, omit
the static credential keys rather than storing empty strings.

### LLM Providers

`llm.openrouter.existingSecret` expects `OPENROUTER_API_KEY` and optionally
`OPENROUTER_ENDPOINT`. `llm.openai.existingSecret` expects `OPENAI_API_KEY` and
optionally `OPENAI_ENDPOINT`.

## Image Configuration

`global.imageRegistry` is a literal prefix. Include the trailing slash when
using it:

```yaml
global:
  imageRegistry: registry.example.com/flow-like/
  imagePullSecrets:
    - name: registry-credentials

api:
  image:
    repository: api
    tag: replace-me
    pullPolicy: IfNotPresent

web:
  image:
    repository: web
    tag: replace-me
    pullPolicy: IfNotPresent

executor:
  image:
    repository: executor
    tag: replace-me
    pullPolicy: IfNotPresent

executorPool:
  image:
    repository: executor
    tag: replace-me
    pullPolicy: IfNotPresent

database:
  migration:
    image:
      repository: migration
      tag: replace-me
      pullPolicy: IfNotPresent
```

The executor image is configured twice: `executor.image` is placed into
per-execution Job specs, while `executorPool.image` is used for reusable
workers. The former does not make Job execution functional because the
checked-in image exits in job-once mode.

## Runtime Class

```yaml
runtimeClass:
  create: false
  name: kata
  handler: kata
```

Setting `create: true` creates a Kubernetes `RuntimeClass` object; it does not
install Kata Containers on any node. Verify the handler at the cluster level
before referencing it from workloads. The warm executor pool does not set
`runtimeClassName`, and the checked-in `kubernetes_job` runner is incomplete.

## Ingress

Each path can target the API or web Service:

```yaml
ingress:
  enabled: true
  className: nginx
  hosts:
    - host: api.example.com
      paths:
        - path: /
          pathType: Prefix
          service: api
    - host: app.example.com
      paths:
        - path: /
          pathType: Prefix
          service: web
  tls:
    - secretName: flow-like-tls
      hosts:
        - api.example.com
        - app.example.com
```

Create the TLS Secret separately or through the certificate controller used by
your cluster.

## Render, Install, and Upgrade

Render first so image names, Secret references, and optional resources can be
reviewed:

```bash
helm lint apps/backend/kubernetes/helm \
  --values flow-like-values.yaml

helm template flow-like apps/backend/kubernetes/helm \
  --namespace flow-like \
  --values flow-like-values.yaml
```

Install or reconcile the release:

```bash
helm upgrade --install flow-like apps/backend/kubernetes/helm \
  --namespace flow-like \
  --create-namespace \
  --values flow-like-values.yaml
```

If the values reference pre-created Secrets, create the namespace and Secrets
before running Helm instead of relying on `--create-namespace`.

Inspect the effective values and rendered resources during troubleshooting:

```bash
helm get values flow-like -n flow-like
helm get manifest flow-like -n flow-like
kubectl get events -n flow-like --sort-by=.lastTimestamp
```
