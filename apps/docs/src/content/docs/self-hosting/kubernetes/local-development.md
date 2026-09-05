---
title: Local Development
description: Build and run the Kubernetes backend locally with k3d.
sidebar:
  order: 40
---

The k3d helper provides a local Kubernetes workflow for explicitly trusted
executions. It uses the reusable executor process and does not install gVisor or
the Cilium isolation boundary. Use the Linux cluster
[installation path](/self-hosting/kubernetes/installation/) for untrusted tenants.

## Prepare the local environment

Install Docker, k3d, kubectl, Helm 3, Python 3 and OpenSSL. Allocate resources for
the API, web, internal database, Redis, RustFS and any enabled monitoring services.
The helper uses one k3d server and two agents.

From `apps/backend/kubernetes/`:

```bash
export K3D_EXECUTION_MODE=trusted_shared
export PUBLIC_API_URL=http://localhost:8080
export PUBLIC_WEB_URL=http://localhost:3001
export S3_PUBLIC_ENDPOINT=https://s3.dev.example.com

cp flow-like.config.example.json ../../../flow-like.kubernetes.config.json
export FLOW_LIKE_CONFIG=flow-like.kubernetes.config.json
```

Edit the JSON for the development OIDC provider and public hub settings. The API
embeds it at build time.

Replace the S3 example with an origin reachable from both the browser and the
cluster. Configure that origin to reach the bucket-only object gateway with
matching TLS and Host handling. The helper requires this value; a local-only port
or cluster-only DNS name does not meet both clients' needs.

## Generate and install

```bash
./scripts/dev-bootstrap.sh
./scripts/dev.sh setup
```

The bootstrap command writes private Secrets and matching values under
`.generated/`; it does not modify the cluster. Setup:

1. Creates the k3d cluster when absent and waits for its nodes.
2. Builds the application images and imports them directly into k3d.
3. Reuses the generated configuration, creating it only when absent.
4. Applies its namespace and Secrets.
5. Deploys the chart with `trusted_shared`, HTTP asynchronous dispatch and
   Traefik ingress.

Bundled RustFS is included and its initialization Job creates private buckets.
The helper no longer relies on an external object store, a local image registry
or an implicit `.env` file.

Existing generated files are preserved. Change non-secret settings in the values
file and rebuild images when public hub or web build settings change.

## Access and inspect

The helper maps host port 8080 to the k3d ingress. Public hosts and paths depend on
the generated and operator ingress values. Port forwarding provides explicit
operator access:

| Service | Command |
| --- | --- |
| API | `kubectl port-forward -n flow-like svc/flow-like-api 8083:8080` |
| Web | `kubectl port-forward -n flow-like svc/flow-like-web 3001:3001` |
| Grafana | `kubectl port-forward -n flow-like svc/flow-like-grafana 3000:80` |
| Prometheus | `kubectl port-forward -n flow-like svc/flow-like-prometheus 9090:9090` |

Use the configured Grafana credential workflow; the helper does not set a shared
`admin/admin` password.

```bash
./scripts/dev.sh status
kubectl logs deployment/flow-like-api -n flow-like --tail=100
kubectl logs deployment/flow-like-executor-pool -n flow-like --tail=100
kubectl get events -n flow-like --sort-by=.lastTimestamp
```

## Rebuild and troubleshoot

```bash
./scripts/dev.sh rebuild
helm get values flow-like -n flow-like
kubectl get pods,jobs,pvc -n flow-like
```

Rebuild imports new images and reapplies the chart. Common failures are an
unreachable public object origin, failed migration or bucket initialization,
insufficient Docker resources, and a PVC that cannot bind. Inspect the corresponding
Job and Pod events before changing credentials.

For direct helper access, `k3d-setup.sh` accepts the same
`setup`, `rebuild`, `status` and `delete` actions.
`K3D_CLUSTER_NAME` and `K8S_NAMESPACE` select the local names.

## Remove the cluster

```bash
./scripts/dev.sh delete
```

This deletes the k3d cluster and its workloads. Keep backups of any local data
you need before deletion. Generated configuration and Docker build cache remain
in the workspace and host; protect the generated Secrets if reusing them.
