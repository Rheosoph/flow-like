---
title: kubectl Basics
description: Essential commands for operating Flow-Like on Kubernetes.
sidebar:
  order: 15
---

This guide assumes the Helm release and namespace are both named `flow-like`. Replace either name if your installation uses different values; chart resource names are derived from the Helm release.

## Confirm the cluster and namespace

```bash
kubectl config current-context
kubectl config get-contexts
kubectl cluster-info
kubectl get namespace flow-like
```

Set the namespace on the current context if you do not want to repeat `-n flow-like`:

```bash
kubectl config set-context --current --namespace=flow-like
```

This changes your local kubeconfig context, not the cluster resources.

## Inspect the installation

```bash
kubectl get deployment,statefulset,job,cronjob,pod,service,pvc,hpa -n flow-like
kubectl get events -n flow-like --sort-by=.lastTimestamp
```

With default chart values, expect:

| Resource | Default form |
|---|---|
| API | Deployment and ClusterIP Service |
| Web app | Deployment and ClusterIP Service |
| Execution manager | Rust Deployment and private ClusterIP Service |
| Queue bridge | Trusted background dispatch Deployment |
| Execution slots | Dynamic single-use runner and gateway Pod pairs |
| RustFS | StatefulSet, initializer Job and bucket-only gateway |
| Redis | Single-replica Deployment, Service named `flow-like-redis-master`, and optional PVC |
| Internal CockroachDB | Single-replica StatefulSet plus headless and public Services |
| Database migration | Job |
| Compiler | Disabled unless configured |
| HPAs | Absent unless autoscaling is enabled |

Use labels when Pod names include generated hashes:

```bash
kubectl get pods -n flow-like \
  -l app.kubernetes.io/component=api \
  -o wide
```

For details and recent events:

```bash
kubectl describe deployment flow-like-api -n flow-like
kubectl describe pod <pod-name> -n flow-like
```

`kubectl top pods -n flow-like` requires the cluster metrics API, commonly provided by Metrics Server.

## Port-forward the API

```bash
kubectl port-forward service/flow-like-api 8083:8080 -n flow-like
```

In another terminal:

```bash
curl -fsS http://localhost:8083/health/ready
curl -fsS http://localhost:8083/api/v1/health
```

The command parts are:

| Part | Meaning |
|---|---|
| `service/flow-like-api` | Kubernetes resource receiving the forwarded connection |
| first `8083` | Local port |
| second `8080` | Service port inside the cluster |
| `-n flow-like` | Namespace containing the Service |

The port forward lasts only while the command is running.

### Optional operator-only services

Forward the internal CockroachDB Admin UI to localhost:

```bash
kubectl port-forward service/flow-like-cockroachdb-public 8084:8080 -n flow-like
```

Open `http://localhost:8084`. Keep database and metrics interfaces bound to localhost; do not publish them through an unauthenticated Ingress.

## Read logs

```bash
# Recent API logs
kubectl logs deployment/flow-like-api -n flow-like --tail=100

# Follow API logs
kubectl logs deployment/flow-like-api -n flow-like --follow

# Logs from a specific Pod and container
kubectl logs <pod-name> -c <container-name> -n flow-like

# Previous container instance after a restart
kubectl logs <pod-name> -c <container-name> -n flow-like --previous
```

List container names before selecting one:

```bash
kubectl get pod <pod-name> -n flow-like \
  -o jsonpath='{.spec.containers[*].name}'
```

## Restart or watch a rollout

```bash
kubectl rollout restart deployment/flow-like-api -n flow-like
kubectl rollout status deployment/flow-like-api -n flow-like
kubectl rollout history deployment/flow-like-api -n flow-like
```

A rollout restart changes live cluster state and briefly replaces Pods. Check readiness and logs after it completes.

## Diagnose an unhealthy Pod

```bash
kubectl get pods -n flow-like
kubectl describe pod <pod-name> -n flow-like
kubectl logs <pod-name> -n flow-like --tail=200
kubectl logs <pod-name> -n flow-like --previous --tail=200
```

| Status | First checks |
|---|---|
| `Pending` | Scheduling events, resource requests, PVC binding, node selectors |
| `ImagePullBackOff` | Image name/tag, pull policy, registry credentials |
| `CrashLoopBackOff` | Current and previous logs, environment references, probes |
| `Running` but not ready | Readiness probe, dependencies, Service endpoints |

Check which Pods back the API Service:

```bash
kubectl get endpointslice -n flow-like \
  -l kubernetes.io/service-name=flow-like-api
```

## Inspect execution capacity

```bash
kubectl get pods -n flow-like -l app.kubernetes.io/component=execution-sandbox -o wide
kubectl get pods -n flow-like -l app.kubernetes.io/component=execution-egress -o wide
kubectl logs deployment/flow-like-execution-manager -n flow-like --tail=100
kubectl logs deployment/flow-like-queue-bridge -n flow-like --tail=100
kubectl port-forward service/flow-like-execution-manager 9000:9000 -n flow-like
```

Check `/ready` for supervisor health and `/metrics` for available warm slots.
The default manager can be reachable while its clean reserve is empty. Dynamic
runner Pods use gVisor; the reusable executor-pool Deployment appears only in
`trusted_shared` mode.

An arbitrary debug Pod does not acquire API or gateway access by sharing the
namespace. Inspect the real caller's labels and NetworkPolicies when testing
Pod-to-Service connectivity. Do not remove restrictive runner policies to diagnose
an execution that has not yet terminated.

## Inspect configuration safely

List names and metadata:

```bash
kubectl get configmap,secret -n flow-like
kubectl describe secret flow-like-storage -n flow-like
```

`kubectl describe secret` shows key names and sizes without printing secret values. Avoid `-o yaml`, JSONPath decoding, shell tracing, or screenshots when handling production Secrets.

Check which environment sources the API Pod references without resolving their values:

```bash
kubectl get deployment flow-like-api -n flow-like \
  -o jsonpath='{.spec.template.spec.containers[0].envFrom[*].secretRef.name}'
```

## Scale the API

When `api.autoscaling.enabled=false`:

```bash
kubectl scale deployment/flow-like-api --replicas=3 -n flow-like
kubectl rollout status deployment/flow-like-api -n flow-like
```

Manual scale changes can be overwritten by a later Helm upgrade. Record the intended count in `api.replicaCount`.

When autoscaling is enabled:

```bash
kubectl get hpa flow-like-api -n flow-like
kubectl describe hpa flow-like-api -n flow-like
```

Let the HPA own the replica count and adjust the chart's autoscaling values instead of repeatedly using `kubectl scale`.

## Helm operations

Inspect the installed release:

```bash
helm status flow-like -n flow-like
helm get values flow-like -n flow-like
helm history flow-like -n flow-like
```

Apply updates through the checked-in helper, using the same ordered values files
as installation:

```bash
cd apps/backend/kubernetes
./scripts/deploy.sh -f values-operator.yaml -f .generated/values-images.yaml
```

The helper checks rendered values and Cilium prerequisites before updating the
release. Reuse existing Secrets and preserve Redis replay claims. Drain or
reconcile accepted jobs before queue protocol changes, and allow active managers
to finish their shutdown period. Rebuild and push pinned manager and executor
images together when their protocol changes.

A Helm rollback changes Kubernetes resources; it does not restore SQL schema,
object data or lost Redis claims. Review compatibility and retained execution
state before returning to an earlier revision.

## Quick reference

| Task | Command |
|---|---|
| List workloads | `kubectl get deploy,sts,job,cronjob,pod -n flow-like` |
| Recent events | `kubectl get events -n flow-like --sort-by=.lastTimestamp` |
| API logs | `kubectl logs deploy/flow-like-api -n flow-like --tail=100` |
| API port forward | `kubectl port-forward svc/flow-like-api 8083:8080 -n flow-like` |
| API rollout | `kubectl rollout status deploy/flow-like-api -n flow-like` |
| Describe a Pod | `kubectl describe pod <pod-name> -n flow-like` |
| List Service backends | `kubectl get endpointslice -n flow-like` |

## Related

- [API Reference](/self-hosting/kubernetes/api-reference/)
- [Configuration](/self-hosting/kubernetes/configuration/)
- [Local Development](/self-hosting/kubernetes/local-development/)
