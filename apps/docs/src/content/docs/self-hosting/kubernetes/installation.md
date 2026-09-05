---
title: Installation
description: Install the Flow-Like backend on Kubernetes with the repository Helm chart.
sidebar:
  order: 12
---

Install the chart with a private RustFS store and single-use execution sandboxes.
Complete [Prerequisites](/self-hosting/kubernetes/prerequisites/) first, especially
gVisor, Cilium and persistent storage. Commands below assume the release and
namespace are both named `flow-like`.

For trusted local evaluation, use
[Local Development](/self-hosting/kubernetes/local-development/) instead.

## Configure public endpoints and identity

From the repository root:

```bash
cd apps/backend/kubernetes
export PUBLIC_API_URL=https://api.flow-like.example.com
export PUBLIC_WEB_URL=https://app.flow-like.example.com
export S3_PUBLIC_ENDPOINT=https://s3.flow-like.example.com

cp flow-like.config.example.json ../../../flow-like.kubernetes.config.json
export FLOW_LIKE_CONFIG=flow-like.kubernetes.config.json
```

Edit that JSON file for your OIDC issuer, client, JWKS URL, hub domain, web origin
and signaling URL. `FLOW_LIKE_CONFIG` is relative to the repository root and is
embedded in the API binary. Rebuild the API when those settings change.

The object-store endpoint must serve the configured buckets and be reachable from
both browsers and Pods. Configure its ingress and DNS before running workflows.

## Generate private configuration

For production, provide `DATABASE_URL` through your shell's secret-management
workflow before setup. The URL selects an external PostgreSQL database by default;
set `DATABASE_PROVIDER=cockroachdb` for an external CockroachDB service. Without
it, setup retains the internal evaluation database.

```bash
./scripts/setup-config.sh
```

Setup writes:

| File | Contents |
| --- | --- |
| `.generated/secrets.yaml` | ES256 keypair, API secrets, manager token, Redis credentials and separate RustFS root/API/STS identities |
| `.generated/values-generated.yaml` | Matching Secret references and deployment settings |

Both files are created with mode `0600`. Setup does not change the cluster and
refuses to overwrite existing files. Preserve them for upgrades and back them up
with the data services. Use `--namespace`, `--release` and `--output-dir` when
maintaining multiple installations.

## Build and publish the images

```bash
REGISTRY=registry.example.com/flow-like TAG=release-2026-09 PUSH=true \
  ./scripts/build-images.sh
```

The script builds the API, executor, Rust manager and gateway, queue bridge,
compiler, signaling, migration, RustFS initializer and web images. It writes
`.generated/values-images.yaml` with image references. Isolated execution
requires the pushed manager and executor digests.

To rebuild selected components, set `COMPONENTS`, for example
`COMPONENTS="api execution-manager"`. Partial builds preserve other entries in
the image values file. Generated Secrets are excluded from Docker build contexts.

## Review the deployment values

Start from `helm/values-production.yaml` and create an operator override file
such as `values-operator.yaml`. Replace example domains, registry values and
node placement. Configure:

- An external database and its connection budget.
- API and web ingress hosts, ingress class and TLS Secrets.
- The separate `rustfs.gateway.ingress` host and TLS configuration.
- Execution node selectors, manager replica count, concurrency and warm reserve.
- Persistent volume sizes and any external-service network rules.

For an HTTPS object gateway in isolated mode,
`executionManager.objectStoreTlsGateway=true` is required. Set it only when the
endpoint exposes bucket data and blocks root, STS and administration routes.
Ingress must preserve the signed Host header and path.

## Apply Secrets and deploy

```bash
kubectl config current-context
kubectl create namespace flow-like --dry-run=client -o yaml | kubectl apply -f -
kubectl apply -f .generated/secrets.yaml

./scripts/deploy.sh \
  -f helm/values-production.yaml \
  -f .generated/values-generated.yaml \
  -f values-operator.yaml \
  -f .generated/values-images.yaml
```

Helm applies later values last. Keep the generated image file last so its actual
digests replace production examples. The deploy script lints and renders before
checking Cilium and updating the release. It waits for workloads and initialization
Jobs; `HELM_TIMEOUT` defaults to `20m`.

The namespace must already exist. The script does not apply or rotate Secrets.
Choose the cluster and identity through `KUBECONFIG` and its selected context;
use `K8S_NAMESPACE` and `RELEASE` for deployment names.

## Verify the installation

```bash
helm status flow-like -n flow-like
kubectl get pods,jobs,svc,pvc -n flow-like
kubectl rollout status deployment/flow-like-api -n flow-like
kubectl rollout status deployment/flow-like-execution-manager -n flow-like
kubectl logs deployment/flow-like-queue-bridge -n flow-like --tail=100
kubectl port-forward service/flow-like-api 8083:8080 -n flow-like
```

In another terminal:

```bash
curl -fsS http://localhost:8083/health/ready
```

API readiness checks the database, required schema and execution-state store.
Also inspect the manager's warm-slot metrics: a reachable API does not mean clean
execution capacity is available.

Run `helm test flow-like -n flow-like --logs` against a disposable or backed-up
store to exercise scoped STS and object-gateway behavior. The test creates and
removes unique temporary object prefixes. Qualify network isolation, cancellation,
node loss and representative load on the actual cluster before serving tenants.

## Upgrade and recovery

Reuse existing Secrets. Before replacing an older queue protocol, stop new
dispatch and drain or reconcile all accepted jobs. Switch the API and consumers
together to `exec:jobs:v3`. Preserve Redis replay claims and cancellation records;
restoring an older snapshot can allow already executed work to be admitted again.

Allow active runs to drain when replacing managers. Helm's derived termination
grace includes the workflow and supervisor budgets. Review database schema changes
and back up persistent services before upgrade; a Helm rollback does not undo
database or object-store changes.

## Common failures

| Symptom | First check |
| --- | --- |
| Missing digest or rejected values | Pushed image values are present and applied last |
| Cilium preflight failure | Policy configuration, RuntimeClass and Cilium rollout |
| Warm slots remain unavailable | Execution node capacity, Pod events, gateway reachability and denied-endpoint probes |
| API waits in an init container | Release migration or RustFS initialization Job |
| Object download or signature failure | Browser-and-Pod DNS, exact public S3 origin, Host preservation and TLS |
| Pending PVC | StorageClass, access mode, capacity and volume events |
| Cancellation cannot confirm termination | Node or API connectivity; retain the restrictive policy and reconcile the run |
