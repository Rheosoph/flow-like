---
title: Configuration
description: How Helm values become runtime configuration for the Kubernetes backend.
sidebar:
  order: 20
---

The Helm chart is the source of truth for cluster configuration:

- Defaults: `apps/backend/kubernetes/helm/values.yaml`
- Templates: `apps/backend/kubernetes/helm/templates/`
- Local k3d input: `apps/backend/kubernetes/.env`
- Generated local overrides: `apps/backend/kubernetes/helm/values-local.yaml`

The older `scripts/setup-config.sh` creates standalone resources with legacy
names and is not part of the current Helm release wiring. Use Helm values and
the documented `existingSecret` contracts for cluster installs.

## Required Configuration

A non-local install must configure:

1. Pullable API, web, executor, and migration images.
2. `jwt.backendKey` / `backendPub`, or `jwt.existingSecret`.
3. One `storage.provider` block and its credentials or `existingSecret`.
4. `database.type` and, for external mode, a `DATABASE_URL`.

The chart manages `REDIS_URL` when its Redis workload is enabled.

## Database Environment

The API and migration Job require:

- `DATABASE_URL`

Internal mode creates a chart-managed Secret. External mode reads it from
`database.external.existingSecret` or, less safely, from
`database.external.connectionString`.

## Storage Environment

The chart supports five user-facing provider values:

| Helm provider | Runtime selection | Main variables | Stock API status |
| --- | --- | --- | --- |
| `aws` | AWS | `AWS_REGION`, optional static keys, `RUNTIME_ROLE_ARN` | Requires rebuilding the API with `aws` |
| `azure` | Azure | `AZURE_STORAGE_ACCOUNT_NAME`, `AZURE_STORAGE_ACCOUNT_KEY`, container names | Included |
| `gcp` | GCP | `GCP_PROJECT_ID`, `GOOGLE_APPLICATION_CREDENTIALS_JSON`, bucket names | Included |
| `r2` | AWS-compatible storage plus R2 runtime credentials | `R2_*` credentials, `R2_API_TOKEN`, and matching `AWS_*` aliases | Included; use an existing Secret because the generated one omits the token |
| `s3` | Generic S3 plus AWS runtime credentials | `S3_*` settings and matching `AWS_*` aliases | Requires rebuilding with `aws`; provider-side STS compatibility must be validated |

Logical storage names are `META_BUCKET`, `CONTENT_BUCKET`, and `LOG_BUCKET`.
Provider-specific names are also emitted where the runtime needs them.

When the chart creates the storage Secret, it writes the required aliases
automatically, except for the R2 API token. For an externally managed Secret,
follow the exact key table in
the [Helm reference](/self-hosting/kubernetes/helm/#object-storage).

## Execution Environment

The API template derives these variables from values:

| Environment variable | Helm value |
| --- | --- |
| `EXECUTION_BACKEND` | `execution.backend` |
| `ASYNC_EXECUTION_BACKEND` | `execution.asyncBackend` |
| `EXECUTOR_URL` | `execution.executorUrl`, or the warm-pool Service |
| `K8S_EXECUTOR_IMAGE` | `global.imageRegistry` + `executor.image` |
| `K8S_EXECUTOR_RUNTIME_CLASS` | `executor.runtimeClass` / `runtimeClass.name` |
| `JOB_TIMEOUT_SECONDS` | `executor.timeout` |
| `JOB_MAX_RETRIES` | `executor.maxRetries` |
| `KUBERNETES_NAMESPACE` | Pod namespace |

The values accept `http`, `redis`, and `kubernetes_job`, but the checked-in
Kubernetes executor pool only implements HTTP. It does not consume the Redis
execution list, and its one-job entrypoint is not implemented. Use `http` for
both lanes unless you deploy another compatible consumer.

The Job dispatcher also reads `K8S_NAMESPACE`, `K8S_RUNTIME_CLASS`,
`K8S_JOB_TIMEOUT`, and `K8S_JOB_MAX_RETRIES`, while the current API template
emits the differently named variables shown above. Those Job-specific values
are therefore not wired to the dispatcher. This is another reason not to
operate `kubernetes_job` from the current chart without code/chart changes.

Execution-token variables come from the Secret selected by
`jwt.existingSecret`: the API uses `BACKEND_KEY`, `BACKEND_PUB`, and
`BACKEND_KID`; executors and the optional compiler use the public key and key
identifier.

## Additional API Variables

Add non-secret variables with `api.env`:

```yaml
api:
  env:
    - name: RUST_LOG
      value: flow_like=debug,tower_http=info
```

Import another ConfigMap or Secret with `api.envFrom`:

```yaml
api:
  envFrom:
    - configMapRef:
        name: flow-like-extra-config
    - secretRef:
        name: flow-like-extra-secret
```

Do not duplicate chart-owned names such as `DATABASE_URL`, `REDIS_URL`, or
`BACKEND_KEY` unless you intentionally understand Kubernetes environment
precedence.

## Redis

With chart-managed authentication, the chart generates a password and an
in-cluster `REDIS_URL`. To supply a stable password:

```yaml
redis:
  auth:
    enabled: true
    existingSecret: flow-like-redis
```

The Secret must contain `REDIS_PASSWORD`; the API template constructs the URL
without copying the password into rendered Helm output.

## LLM Providers

The warm executor pool can read:

- `OPENROUTER_API_KEY` and optional `OPENROUTER_ENDPOINT`
- `OPENAI_API_KEY` and optional `OPENAI_ENDPOINT`

Select their Secrets with `llm.openrouter.existingSecret` and
`llm.openai.existingSecret`.

## Inspect the Result

Render changes before applying them:

```bash
helm lint apps/backend/kubernetes/helm \
  --values flow-like-values.yaml

helm template flow-like apps/backend/kubernetes/helm \
  --namespace flow-like \
  --values flow-like-values.yaml
```

After installation:

```bash
helm get values flow-like -n flow-like
helm get manifest flow-like -n flow-like
kubectl describe deployment flow-like-api -n flow-like
```

Rendered manifests contain Secret names and references. They should not contain
long-lived credential values when all sensitive configuration uses
`existingSecret`.
