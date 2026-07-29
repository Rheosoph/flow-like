---
title: Local Development
description: Build and run the Kubernetes backend locally with k3d.
sidebar:
  order: 40
---

The checked-in k3d helper creates a local cluster and registry, builds the
application images, writes a gitignored Helm values file, and installs the
chart.

## Prerequisites

Install and start:

- Docker
- k3d
- `kubectl`
- Helm 3
- OpenSSL

Allocate enough Docker resources for the application, single-node database,
Redis, and the enabled monitoring stack. Eight GB of memory is a practical
starting point.

## Configure Storage

From `apps/backend/kubernetes`, create the ignored local environment file:

```bash
cp .env.example .env
```

Set `STORAGE_PROVIDER` and its credentials. Azure and GCP are the
straight-through choices for the checked-in API build. The helper also accepts
`aws` and `s3`, but that API target currently omits the `aws` runtime feature;
those selections do not produce a ready API without rebuilding it. R2 is
compiled in, but scoped operations additionally require `R2_API_TOKEN`, which
the generated local values do not pass through.

Also configure the meta, content, and logs bucket or container names. The
setup script validates the selected provider; it does not install a local
object store or create buckets.

Create the ignored `helm/values-secrets.yaml` override so asynchronous
requests use the implemented warm HTTP pool:

```yaml
execution:
  backend: http
  asyncBackend: http
```

The generated `values-local.yaml` inherits the chart's `redis` asynchronous
default, but the checked-in executor pool does not consume that queue. The
setup script automatically includes `values-secrets.yaml` when present.

## Create the Cluster

```bash
cd apps/backend/kubernetes
./scripts/k3d-setup.sh
```

The script:

1. Creates a k3d cluster with one server and two agents.
2. Creates a local registry exposed at `localhost:5111`.
3. Builds and pushes API, web, executor, and migration images.
4. Generates the `BACKEND_*` execution-key values.
5. Writes `helm/values-local.yaml`, which is gitignored.
6. Installs the chart with one internal CockroachDB pod, Redis, the warm
   executor pool, and the monitoring stack.

Kata is disabled locally. With the override above, the warm executor pool
handles both synchronous and asynchronous execution.

## Local Endpoints

| Service | Access |
| --- | --- |
| API | `http://localhost:8080` through the k3d load balancer and Traefik |
| Grafana | `http://localhost:3002` |
| Web | `kubectl port-forward -n flow-like service/flow-like-web 3001:3001` |
| Prometheus | `kubectl port-forward -n flow-like service/flow-like-prometheus 9090:9090` |
| Cockroach SQL | `kubectl port-forward -n flow-like service/flow-like-cockroachdb-public 26257:26257` |

The generated local values set the Grafana login to `admin` / `admin`. Do not
reuse that configuration outside local development.

## Rebuild and Inspect

```bash
# Rebuild all local images and restart Deployments
./scripts/k3d-setup.sh rebuild

# Show cluster, pod, and service status
./scripts/k3d-setup.sh status

# Inspect the effective values
helm get values flow-like -n flow-like

# Follow API or executor output
kubectl logs -f deployment/flow-like-api -n flow-like
kubectl logs -f deployment/flow-like-executor-pool -n flow-like
```

The build script reuses the existing cluster and registry. Run the full setup
again after changing chart resources that a Deployment restart cannot apply.

## Verify Execution Keys

The chart-generated Secret is `flow-like-api-keys` because local values set
`fullnameOverride: flow-like`:

```bash
kubectl get secret flow-like-api-keys -n flow-like
kubectl get deployment flow-like-executor-pool -n flow-like \
  -o jsonpath='{.spec.template.spec.containers[0].env}'
```

Avoid printing the private `BACKEND_KEY`. The executor only reads
`BACKEND_PUB` and `BACKEND_KID`; the API reads all three keys.

## Troubleshooting

```bash
kubectl get pods -n flow-like
kubectl get events -n flow-like --sort-by=.lastTimestamp
helm status flow-like -n flow-like
```

Typical causes:

- `ImagePullBackOff`: verify `curl http://localhost:5111/v2/_catalog`, then run
  the rebuild action.
- Storage startup failure: check the provider variables in
  `apps/backend/kubernetes/.env` and confirm the buckets already exist.
- Pending PVC: inspect the k3d default `StorageClass` and the associated PVC.
- External request failure: inspect the generated NetworkPolicies and add
  required egress ports to local values.
- Migration failure: inspect the Helm release status and recent namespace
  events; successful hook Jobs can be deleted automatically.

## Remove the Cluster

```bash
./scripts/k3d-setup.sh delete
```

This deletes the k3d cluster and its workloads. The local source tree,
`.env`, generated execution-key PEM files, and Docker build cache remain.
