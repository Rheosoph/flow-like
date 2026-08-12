---
title: Installation
description: Install the Flow-Like backend on Kubernetes with the repository Helm chart.
sidebar:
  order: 12
---

The chart lives at `apps/backend/kubernetes/helm/`. Its checked-in image
defaults target the local k3d registry, so a production install must provide
images that your cluster can pull.

For a local evaluation, use the
[k3d setup](/self-hosting/kubernetes/local-development/) instead.

## Before You Start

You need:

- A Kubernetes cluster, `kubectl`, and Helm 3
- A default `StorageClass`, or explicit storage classes for persistent volumes
- Published API, web, executor, and migration images
- One configured object-storage provider. The checked-in API build supports
  Azure, GCP, and R2 runtime credentials; see the
  [storage status table](/self-hosting/kubernetes/storage/#current-provider-status)
  before choosing AWS or generic S3.
- An ES256 key pair used to sign and verify execution tokens

Create the storage buckets or containers before installing. The Azure example
below uses separate `meta`, `content`, and `logs` containers.

## 1. Create the Namespace

```bash
kubectl create namespace flow-like
```

Reusing an existing namespace is fine.

## 2. Create the Backend-Key Secret

From the repository root, generate a key pair and write only its environment
entries to a private local file:

```bash
umask 077
./tools/gen-execution-keys.sh --export \
  | grep -E '^BACKEND_(KEY|PUB|KID)=' \
  > flow-like-backend-jwt.env
```

Create a Kubernetes Secret without placing its values in the shell command:

```bash
kubectl -n flow-like create secret generic flow-like-backend-jwt \
  --from-env-file=flow-like-backend-jwt.env \
  --dry-run=client -o yaml \
  | kubectl apply -f -
```

The Secret must contain `BACKEND_KEY`, `BACKEND_PUB`, and `BACKEND_KID`.
Protect the private key and delete the temporary environment file securely
after the Secret has been stored.

## 3. Create the Storage Secret

For Azure Blob Storage, create a private
`flow-like-storage.env` file:

```dotenv
STORAGE_PROVIDER=azure
AZURE_STORAGE_ACCOUNT_NAME=replace-me
AZURE_STORAGE_ACCOUNT_KEY=replace-me
AZURE_META_CONTAINER=meta
AZURE_CONTENT_CONTAINER=content
AZURE_LOG_CONTAINER=logs
```

Then load it into the cluster:

```bash
kubectl -n flow-like create secret generic flow-like-storage \
  --from-env-file=flow-like-storage.env \
  --dry-run=client -o yaml \
  | kubectl apply -f -
```

Provider readiness and the other Secret contracts are listed in the
[storage guide](/self-hosting/kubernetes/storage/) and
[Helm chart reference](/self-hosting/kubernetes/helm/#existing-secret-contracts).

## 4. Create a Values File

Save the following as `flow-like-values.yaml`, then replace every image
repository and tag with artifacts available to your cluster:

```yaml
fullnameOverride: flow-like

api:
  image:
    repository: registry.example.com/flow-like/api
    tag: replace-me
    pullPolicy: IfNotPresent

web:
  image:
    repository: registry.example.com/flow-like/web
    tag: replace-me
    pullPolicy: IfNotPresent

executor:
  image:
    repository: registry.example.com/flow-like/executor
    tag: replace-me
    pullPolicy: IfNotPresent

executorPool:
  image:
    repository: registry.example.com/flow-like/executor
    tag: replace-me
    pullPolicy: IfNotPresent

jwt:
  existingSecret: flow-like-backend-jwt

storage:
  provider: azure
  azure:
    existingSecret: flow-like-storage

database:
  type: internal
  migration:
    image:
      repository: registry.example.com/flow-like/migration
      tag: replace-me
      pullPolicy: IfNotPresent
  internal:
    replicas: 1

execution:
  backend: http
  asyncBackend: http

runtimeClass:
  create: false

monitoring:
  enabled: false
```

The internal database is a single-node, insecure CockroachDB intended for
evaluation and development. Use an external database for production.

`runtimeClass.create: false` is appropriate when the cluster does not already
provide the configured Kata handler. The default HTTP execution backend uses
the warm executor pool. The checked-in executor does not yet implement its
one-job entrypoint, so do not select per-run Kubernetes Jobs even if a runtime
class is available.

The chart's default asynchronous backend is `redis`, but the Kubernetes
executor-pool binary does not consume that Redis list. The example therefore
uses `asyncBackend: http`. Keep `redis` only when you deploy a compatible queue
consumer.

## 5. Render and Install

Validate the values locally:

```bash
helm lint apps/backend/kubernetes/helm \
  --values flow-like-values.yaml

helm template flow-like apps/backend/kubernetes/helm \
  --namespace flow-like \
  --values flow-like-values.yaml
```

Install or update the release:

```bash
helm upgrade --install flow-like apps/backend/kubernetes/helm \
  --namespace flow-like \
  --values flow-like-values.yaml
```

## 6. Use an External Database in Production

Create a private `flow-like-database.env` file containing a complete
PostgreSQL or CockroachDB connection URL:

```dotenv
DATABASE_URL=postgresql://flowlike:replace-me@database.example.com:5432/flowlike?sslmode=require
```

Create the Secret:

```bash
kubectl -n flow-like create secret generic flow-like-database \
  --from-env-file=flow-like-database.env \
  --dry-run=client -o yaml \
  | kubectl apply -f -
```

Replace the `database` block in `flow-like-values.yaml`:

```yaml
database:
  type: external
  external:
    existingSecret: flow-like-database
  migration:
    image:
      repository: registry.example.com/flow-like/migration
      tag: replace-me
      pullPolicy: IfNotPresent
```

The migration Job reads the same `DATABASE_URL` as the API and runs during
install and upgrade.

## 7. Verify the Release

```bash
helm status flow-like -n flow-like
kubectl get pods,svc,pvc -n flow-like
kubectl rollout status deployment/flow-like-api -n flow-like
kubectl rollout status deployment/flow-like-executor-pool -n flow-like
```

If Ingress is disabled, open the web service locally:

```bash
kubectl port-forward -n flow-like service/flow-like-web 3001:3001
```

Then visit `http://localhost:3001`.

## Common Failures

- `ImagePullBackOff`: replace the local k3d image defaults and configure
  `global.imagePullSecrets` when the registry is private.
- Missing backend-key errors: check that `jwt.existingSecret` names a Secret
  with all three `BACKEND_*` keys.
- Storage startup errors: verify the selected provider, credential keys, and
  pre-created bucket or container names.
- Pending PVCs: set the relevant `storageClass` values or configure a default
  cluster `StorageClass`.
- RuntimeClass errors: disable RuntimeClass creation for the warm-pool setup.
  Creating the object does not install its runtime handler, and the checked-in
  isolated Job runner is not operational.
