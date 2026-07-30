---
title: Scripts
description: Current behavior and limitations of the Kubernetes helper scripts
sidebar:
  order: 80
---

Kubernetes helper scripts live in:

```text
apps/backend/kubernetes/scripts/
```

Some scripts predate the current Helm chart. Use the status table before
running one:

| Script | Current use |
| --- | --- |
| `k3d-setup.sh` | Complete local k3d build and Helm deployment |
| `deploy.sh` | Thin `helm upgrade --install` wrapper |
| `build-images.sh` | Build API and executor images only |
| `migrate-db.sh` | Development schema push from host tooling |
| `setup-config.sh` | Legacy resource generator; names are not wired into the current chart |
| `dev-bootstrap.sh` | Writes local PostgreSQL values, but its suggested Compose next step is unavailable in this directory |
| `dev.sh` | Stale; expects a Compose file that is not present |

## `k3d-setup.sh`

This is the maintained all-in-one local Kubernetes path:

```bash
cd apps/backend/kubernetes

# Create the cluster, build images, and install the chart.
./scripts/k3d-setup.sh

# Rebuild images and restart Deployments.
./scripts/k3d-setup.sh rebuild

# Print cluster and workload status.
./scripts/k3d-setup.sh status

# Delete the k3d cluster.
./scripts/k3d-setup.sh delete
```

It:

- requires Docker, k3d, kubectl, and Helm;
- creates the `flow-like` k3d cluster and a local registry on host port `5111`;
- builds API, executor, migration, and web images;
- pushes those images to the local registry;
- generates a backend JWT keypair;
- generates `helm/values-local.yaml`;
- deploys internal CockroachDB, Redis, the executor pool, web, monitoring, and
  the API through Helm.

The script does **not** deploy MinIO. It loads storage from
`apps/backend/kubernetes/.env` and supports `aws`, `azure`, `gcp`, `r2`, or
generic `s3`. Copy and configure the example first:

```bash
cp apps/backend/kubernetes/.env.example \
  apps/backend/kubernetes/.env
```

The generated `values-local.yaml` contains storage credentials and generated
JWT material. It is gitignored; keep it that way.

See [Local Development](/self-hosting/kubernetes/local-development/) for the
full workflow.

## `deploy.sh`

`deploy.sh` creates the namespace if needed and runs `helm upgrade --install`:

```bash
cd apps/backend/kubernetes
./scripts/deploy.sh
```

Defaults:

```dotenv
NAMESPACE=flow-like
RELEASE=flow-like
VALUES=apps/backend/kubernetes/helm/values.yaml
```

Override them in the environment and pass additional Helm arguments through:

```bash
NAMESPACE=flow-like-staging \
RELEASE=flow-like-staging \
VALUES=/absolute/path/to/values-staging.yaml \
./scripts/deploy.sh --atomic --timeout 10m
```

The default `values.yaml` contains placeholder/empty secrets and is not a
production secret source. Supply a protected values file or existing Secrets.

## `build-images.sh`

This smaller helper builds only:

- `flow-like-k8s-api`;
- `flow-like-k8s-executor`.

```bash
cd apps/backend/kubernetes

REGISTRY=registry.example.com/team \
TAG=2026-07-29 \
PUSH=true \
./scripts/build-images.sh
```

It does not build the web, compiler, or migration images. The k3d script uses a
separate build path and builds web and migration images as well.

## `migrate-db.sh`

Host mode loads `apps/backend/kubernetes/.env`, builds `DATABASE_URL` when
needed, generates the PostgreSQL Prisma mirror when available, and runs:

```text
prisma db push --accept-data-loss
```

Run it only after reviewing the target database:

```bash
cd apps/backend/kubernetes
./scripts/migrate-db.sh
```

:::caution
This is a schema push with `--accept-data-loss`, not a versioned production
migration workflow. Back up important data and inspect the proposed schema
change first.
:::

The script advertises `--docker`, but that branch runs
`docker compose run --rm db-migrate` from
`apps/backend/kubernetes`. No Compose file or `db-migrate` service is checked
in there, so that mode is currently nonfunctional.

The Helm chart has its own migration Job path; prefer that for chart-managed
deployments.

## Legacy scripts

### `setup-config.sh`

This script creates fixed resources such as `flow-like-db`, `flow-like-s3`,
`flow-like-api-config`, and `flow-like-executor-config`. The current Helm
templates do not reference those names, and the script only models its legacy
S3 environment shape.

Use Helm values and the chart's `existingSecret` options instead. Run
`setup-config.sh` only if you also maintain custom manifests that consume its
resources.

### `dev-bootstrap.sh` and `dev.sh`

`dev-bootstrap.sh` writes PostgreSQL variables and a `DATABASE_URL` to
`apps/backend/kubernetes/.env`, then prints `docker compose up -d` as the next
step. `dev.sh` also starts Compose directly.

There is no `apps/backend/kubernetes/docker-compose.yml` in the repository, so
those next steps do not work in the current tree. Use `k3d-setup.sh`, or use the
separate [Docker Compose deployment](/self-hosting/docker-compose/overview/).

## Related

- [Local Development](/self-hosting/kubernetes/local-development/)
- [Helm](/self-hosting/kubernetes/helm/)
- [Configuration](/self-hosting/kubernetes/configuration/)
