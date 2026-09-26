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
| `resolve-images.py` | Pin the published images of one tag to digests through the registry API |
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

`--image-pull-secret NAME` (repeatable) writes `global.imagePullSecrets` for
forks and private mirrors and prints the matching
`kubectl create secret docker-registry` command. Setup stores only the name;
create the Secret yourself with a `read:packages` token.

Export `DATABASE_URL` for external SQL and `REDIS_URL` for external Redis.
Bundled RustFS is the default. External storage requires
`RUSTFS_ENABLED=false` and its endpoint and credential variables. See
[Storage](/self-hosting/kubernetes/storage/#external-s3-compatible-storage).

Supply `FLOW_LIKE_CONFIG_FILE` for the host-side Hub JSON or
`FLOW_LIKE_CONFIG_JSON` for raw JSON. Setup keeps that document in a separate
generated Secret, outside values files. `FLOW_LIKE_CONFIG_SECRET_REF` stores a
reference instead; make its value available to the API's SecretStore separately.
See [runtime configuration](/self-hosting/kubernetes/configuration/#runtime-api-and-web-configuration)
for compatibility inputs and restart requirements.

## resolve-images.py

```bash
./scripts/resolve-images.py --tag 1.2.3
./scripts/resolve-images.py --tag dev --arch arm64
GHCR_TOKEN=... ./scripts/resolve-images.py --registry ghcr.io/my-fork --pull-secret ghcr-pull
```

The resolver needs only Python; it never pulls images. For each first-party
repository it requests an anonymous pull token from the registry and reads the
`Docker-Content-Digest` of `<repository>:<tag>`, accepting OCI and Docker
indexes and single manifests. It writes `.generated/values-images.yaml` (or
`--output`) with `repository`, `tag`, `digest` and `pullPolicy: IfNotPresent`
for every image, `executionManager.image.digest`, the full
`executionManager.sandbox.image` executor reference and the sink-trigger image.
Existing unrelated entries in the output file are preserved.

| Option | Purpose |
| --- | --- |
| `--tag` | `dev` (default), `main`, `alpha`, `beta`, `<version>`, `<version>-beta` or an immutable `sha-...-run-...` tag |
| `--registry` | `ghcr.io/rheosoph` (default) or a mirror holding the same repository names |
| `--pull-secret NAME` | Adds the Secret to `global.imagePullSecrets`; repeatable |
| `--arch amd64` or `--arch arm64` | Adds `kubernetes.io/arch` to every workload node selector |

Private packages and forks require `GHCR_TOKEN` (and optionally `GHCR_USER`)
in the environment; the script accepts no token argument and prints none. The
credentials are only sent to a token endpoint on the `--registry` host itself,
so a mirror advertising a foreign authentication realm is queried anonymously.
An HTTP 401, 403 or 404 names the repository and tag that failed.

## build-images.sh

```bash
REGISTRY=registry.example.com/team TAG=release-2026-09 PUSH=true \
  ./scripts/build-images.sh
```

The script builds API, executor, execution-manager, sink-trigger, migration and
web images as `flow-like-kubernetes-<component>`, and the runtime queue bridge,
compiler, signaling and object-store-init images as
`flow-like-docker-compose-<component>`, matching the published repository names.
It writes `.generated/values-images.yaml`; `IMAGE_VALUES_FILE` changes the
output path. With `PUSH=true` every component records its pushed digest.

Set `COMPONENTS="api executor execution-manager"` for a partial build. Existing
image entries are retained. The manager and executor must be pushed to produce
the immutable digests required by isolated execution. Rebuild them together when
changing the assignment protocol. Local builds are single-architecture.

`FLOW_LIKE_BUILD_CONFIG` optionally selects a repository-relative public JSON
document to embed in the API and the audit worker. For the API it is only a
fallback; the worker compiles its `audit` section as its
[audit policy](/self-hosting/audit-trail/#audit-policy) and reads no runtime
config, so set audit policy there. Runtime API settings belong in the
setup-generated Secret; public web URLs belong in Helm runtime configuration.
Generated secret files are excluded from root build contexts.

## deploy.sh

The default inputs are the generated values and image files:

```bash
./scripts/deploy.sh -f values-operator.yaml
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

Additional values files are passed in order after the generated values file.
The image values file is appended after all arguments, so resolved or pushed
digests override image placeholders in operator files. Per-command namespace
and cluster overrides are rejected so preflight and Helm use the same target.

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
read-only. The helper builds the images locally, imports the tag-referenced
entries into k3d and deploys the chart; it does not install gVisor. The object
endpoint must be reachable from both the browser and the cluster.

`./scripts/dev.sh delete` deletes the local cluster and its workloads.
See [Local Development](/self-hosting/kubernetes/local-development/) for access,
configuration and persistence boundaries.

## Validate chart changes locally

Run the chart and helper tests from `apps/backend/kubernetes/` without a cluster:

```sh
python3 -m venv /tmp/flow-like-chart-tests
/tmp/flow-like-chart-tests/bin/pip install PyYAML==6.0.2
/tmp/flow-like-chart-tests/bin/python -m unittest discover -s scripts/tests -v
```

The chart rendering tests need `helm` on `PATH` and PyYAML; without PyYAML
they are skipped rather than failing, so the helper tests still run on a stock
`python3`. These tests validate rendered configuration and helper behavior. Use the live
storage test, isolation probes and recovery procedure in
[Installation](/self-hosting/kubernetes/installation/#verify-the-installation)
for deployment qualification.

## Database schema helper

Chart deployments should use the release migration Job described in
[Database](/self-hosting/kubernetes/database/#schema-application).

The older `migrate-db.sh` sources the backend `.env` file and runs Prisma schema
push with `--accept-data-loss`. Its `--docker` branch expects a Compose service
that is absent from this Kubernetes directory. It is not the installation path
for the Helm chart; use it only when maintaining a separate, reviewed development
database workflow.
