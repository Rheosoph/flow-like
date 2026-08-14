Thursday, 14:00. The go-live review. Priya sits across the table with the security checklist; the platform lead has the cluster dashboard open; you have the artifacts below. Six calls to make — everything you need is in the five lessons behind you.

## Artifact A — `staging-01`, the Compose box

The API container restart-loops. Its startup log ends with an error about an unsupported runtime provider. The relevant `.env` lines:

```dotenv
STORAGE_PROVIDER=aws
RUNTIME_CREDENTIALS_PROVIDER=
CDN_BUCKET_NAME=
```

## Artifact B — production values excerpt

`flow-like-values.yaml` for the cluster release, as currently committed:

```yaml
api:
  image:
    repository: k3d-flow-like.localhost:5000/api
    tag: dev
    pullPolicy: Never
database:
  type: internal
execution:
  backend: http
  asyncBackend: redis
runtimeClass:
  create: false
monitoring:
  enabled: true
```

## Artifact C — a teammate's proposal

> "Before go-live, switch `execution.backend` to `kubernetes_job` so every run gets its own pod. That gives us per-tenant isolation for free."

## Artifact D — a bug report from the pilot team

> "I configured `CRM_API_TOKEN` as a **Secret** runtime variable on my machine. Local runs work. Every remote run on the new cluster fails to authenticate against the CRM."

## Artifact E — the desktop rollout script

The IT team's laptop provisioning script contains this line — the templating step that should fill in the URL silently produced nothing:

```bash
export FLOW_LIKE_API_URL=""
```

## Artifact F — Priya's checklist

1. Show me the signing-key rotation plan.
2. Show me where last night's backups live.
3. Show me who gets paged when an alert fires.

Take them one at a time. The room is listening.
