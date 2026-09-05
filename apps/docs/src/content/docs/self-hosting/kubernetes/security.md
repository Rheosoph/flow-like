---
title: Security
description: Security boundaries and checked-in Helm chart limitations
sidebar:
  order: 90
---

The default deployment isolates untrusted executions with a single-use gVisor
runner and a separate gateway that enforces egress. The API, manager, queue bridge
and storage issuer remain trusted control-plane components. Protect their
credentials and restrict who can modify the deployment namespace.

## Execution boundary

```yaml
execution:
  isolationMode: per_run
  backend: http
  asyncBackend: redis
networkPolicy:
  enabled: true
runtimeClass:
  create: false
  name: runsc
  handler: runsc
```

Install the gVisor handler on execution nodes before referencing this
RuntimeClass. Setting `runtimeClass.create=true` creates the Kubernetes
reference only.

The manager prepares a clean runner and gateway before tenant input arrives.
The runner executes one signed dispatch and is then destroyed. Its filesystem is
read-only apart from bounded temporary storage, it runs as a non-root user, and
it drops Linux capabilities. The gateway occupies a separate Pod so tenant code
cannot change its assigned policy.

Runner and gateway Pods have no mounted Kubernetes service account token. The
runner receives no API signing key, database URL, Redis password, RustFS root key
or installation-wide model-provider credential. Runtime environment variables
can be visible to workflow code, so keep tenant-visible capabilities scoped to
the assigned execution.

## Network enforcement

The chart requires Cilium for `per_run` mode. Configure policy enforcement,
Kubernetes NetworkPolicy support and `allow-localhost=policy` before deployment.
The deploy helper checks the Cilium CRD, configuration and rollout state.

A runner's policy permits only its own gateway on TCP port 3128. Manager Pods
can reach the runner's assignment endpoint and the gateway's control endpoint.
Additional Cilium deny policies block local/remote nodes, Kubernetes API and
metadata destinations. Explicit host ingress preserves kubelet health probes.

Standard Kubernetes NetworkPolicy allows local-node traffic. The required Cilium
rules close that path. Each warm runner also tests gateway reachability and
prohibited endpoints before accepting input. Qualify these controls on the real
cluster, including node-local services, metadata, other tenants and IPv6 where
used.

The gateway permits the run's authorized callbacks, selected object-store origin
and exact HTTPS integration hosts. For an HTTPS object store, enable
`executionManager.objectStoreTlsGateway` only for an endpoint that blocks
administration, root and STS paths: a TLS tunnel cannot inspect those paths itself.

## Service accounts and namespace ownership

The trusted manager has namespace-scoped permission to manage execution Pods,
NetworkPolicies and cancellation ConfigMaps. Runtime Pods have no such role.
The API has the permissions needed to check release initialization Jobs; it does
not need to supervise tenant sandboxes.

Use a dedicated namespace. Restrict permission to create Pods, change labels,
edit NetworkPolicies or bind service accounts there. A user who can rewrite those
objects can undermine the intended boundary.

Inspect the actual resources:

```bash
kubectl get serviceaccount,role,rolebinding -n flow-like
kubectl get pods -n flow-like --show-labels
kubectl get networkpolicy,ciliumnetworkpolicy -n flow-like
```

## Object-store and service credentials

Setup generates separate RustFS root, API and STS issuer identities. Root
credentials enter only RustFS and its initializer. The API obtains temporary
credentials narrowed with a session policy; the provider must enforce that policy
within the issuer's base permissions. Object prefixes are enforced by storage
authorization, while the gateway restricts reachable origins and API paths.

Keep credentials in existing Secrets and preserve them on upgrade. Review the
[Secret contracts](/self-hosting/kubernetes/helm/#existing-secret-contracts).
API startup rejects missing signing material and mismatched signing keys. Browser
CORS origins must be explicit.

Bundled Redis uses an authenticated account shared by trusted components. Runner
and gateway Pods do not receive it. A dedicated manager Redis ACL needs `PING`,
`SET`, its `exec:claims:v1:<namespace>:<release>:*` key pattern, and `SELECT`
if using a nonzero database. The Rust client uses RESP2 and disables
library-identification commands.

## Cancellation and recovery

The manager records cancellation before searching for assigned Pods, then waits
for confirmed termination. Keep restrictive policies in place if confirmation
fails. A partitioned node may still be running a sandbox after its Pod object is
forcibly removed.

Replay claims and cancellation records survive manager replacement. Preserve
them when restoring Redis or changing deployment versions. A rollback of those
records can permit already executed work to run again. Reconcile ambiguous queue
items and use idempotency for external side effects.

## Supply chain and qualification

The manager and runner images require immutable digests. Review the images and
WASM artifacts used by the installation, protect registry write access and apply
your cluster's image admission policy.

Before exposing tenants, verify cross-tenant storage denial, missing/expired
session tokens, direct node and API access, cancellation, manager replacement,
node loss and queue recovery. The Helm storage test exercises several STS and
gateway denials; it does not establish complete isolation or throughput.

The `trusted_shared` mode exists for explicitly trusted workflows and local
development. It reuses executor processes and does not provide this per-execution
boundary.
