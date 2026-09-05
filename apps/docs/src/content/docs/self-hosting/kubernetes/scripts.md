---
title: Scripts
description: Current behavior and limitations of the Kubernetes helper scripts
sidebar:
  order: 80
---

Run Kubernetes helpers from `apps/backend/kubernetes/`. Configuration generation,
image publication and deployment are separate steps so their results can be
reviewed before cluster changes.

| Script | Current behavior |
| --- | --- |
| `setup-config.sh` / `setup-config.py` | Generate private Secrets and matching values locally |
| `build-images.sh` | Build the application images and record image values; optionally push |
| `deploy.sh` | Lint, render, check Cilium, deploy and wait for workloads and Jobs |
| `check-cilium.py` | Read the installed Cilium configuration and rollout status |
| `k3d-setup.sh` | Explicit trusted-mode local setup, rebuild, status or deletion |
| `dev-bootstrap.sh` | Forward to configuration generation |
| `dev.sh` | Forward to the k3d workflow |
| `migrate-db.sh` | Legacy development schema-push helper; prefer the Helm migration Job |

## setup-config.sh

```bash
export PUBLIC_API_URL=https://api.example.com
export PUBLIC_WEB_URL=https://app.example.com
export S3_PUBLIC_ENDPOINT=https://s3.example.com
./scripts/setup-config.sh
```

The helper reads exported environment values, generates missing credentials and
writes `.generated/secrets.yaml` and `.generated/values-generated.yaml` with
mode `0600`. It makes no cluster changes and refuses to overwrite either file.

Options are `--namespace`, `--release` and `--output-dir`. Existing generated
files are the upgrade input; generating a new directory is an explicit credential
rotation workflow, not an automatic replacement.

Export `DATABASE_URL` for external SQL and `REDIS_URL` for external Redis.
Bundled RustFS is the default. External storage requires
`RUSTFS_ENABLED=false` and its endpoint and credential variables. See
[Storage](/self-hosting/kubernetes/storage/#external-s3-compatible-storage).

## build-images.sh

```bash
REGISTRY=registry.example.com/team TAG=release-2026-09 PUSH=true \
  ./scripts/build-images.sh
```

The script builds API, executor, execution-manager, runtime queue bridge, compiler,
signaling, migration, object-store-init and web images. It writes
`.generated/values-images.yaml`; `IMAGE_VALUES_FILE` changes the output path.

Set `COMPONENTS="api executor execution-manager"` for a partial build. Existing
image entries are retained. The manager and executor must be pushed to produce
the immutable digests required by isolated execution. Rebuild them together when
changing the assignment protocol.

`FLOW_LIKE_CONFIG` selects the repository-relative public hub/OIDC JSON embedded
in the API. `PUBLIC_API_URL` and `PUBLIC_WEB_URL` supply the web image's public
URLs. Generated secret files are excluded from root build contexts.

## deploy.sh

The default inputs are the generated values and image files:

```bash
./scripts/deploy.sh -f values-operator.yaml -f .generated/values-images.yaml
```

Apply the namespace and Secrets first. The helper checks the exact rendered
configuration and the target cluster's Cilium prerequisites before running
`helm upgrade --install --wait --wait-for-jobs`.

| Environment variable | Default or purpose |
| --- | --- |
| `K8S_NAMESPACE` | `flow-like` |
| `RELEASE` | `flow-like` |
| `VALUES` | `.generated/values-generated.yaml` |
| `IMAGE_VALUES_FILE` | `.generated/values-images.yaml` |
| `HELM_TIMEOUT` | `20m` |
| `KUBECONFIG` | Cluster and identity shared by Helm and prerequisite checks |

Additional values files are passed in order. Put generated image values last
when earlier examples contain image placeholders. Per-command namespace and
cluster overrides are rejected so preflight and Helm use the same target.

## Local development helpers

```bash
export K3D_EXECUTION_MODE=trusted_shared
export S3_PUBLIC_ENDPOINT=https://s3.dev.example.com
./scripts/dev-bootstrap.sh
./scripts/dev.sh setup
./scripts/dev.sh status
./scripts/dev.sh rebuild
```

Setup and rebuild require the explicit trusted-mode selection. Status is
read-only. The helper imports built images into k3d and deploys the chart; it does
not install gVisor. The object endpoint must be reachable from both the browser
and the cluster.

`./scripts/dev.sh delete` deletes the local cluster and its workloads.
See [Local Development](/self-hosting/kubernetes/local-development/) for access,
configuration and persistence boundaries.

## Database schema helper

Chart deployments should use the release migration Job described in
[Database](/self-hosting/kubernetes/database/#schema-application).

The older `migrate-db.sh` sources the backend `.env` file and runs Prisma schema
push with `--accept-data-loss`. Its `--docker` branch expects a Compose service
that is absent from this Kubernetes directory. It is not the installation path
for the Helm chart; use it only when maintaining a separate, reviewed development
database workflow.
