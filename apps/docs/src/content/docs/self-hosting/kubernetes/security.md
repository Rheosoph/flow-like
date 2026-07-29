---
title: Security
description: Security boundaries and checked-in Helm chart limitations
sidebar:
  order: 90
---

The Helm chart supplies useful security building blocks, but it is not a
complete hardened deployment. Review the rendered resources against your
cluster, identity provider, CNI, admission policy, and threat model.

## Execution isolation

The default chart uses a warm executor pool:

```yaml
execution:
  backend: http
executorPool:
  enabled: true
```

Pool pods handle multiple executions over their lifetime. WASM nodes still run
inside Wasmtime, but the container, process environment, service account, and
pod network are shared between runs. Validate request cleanup, board caching,
credential scoping, and concurrency for your deployment.

### Kubernetes Job status

The API can create one Job per dispatch, but the checked-in executor's one-job
entrypoint is not implemented and exits with status `1`. Do not use
`kubernetes_job` as a security isolation control until a compatible runner is
implemented and tested.

### RuntimeClass and Kata

The chart can create a `RuntimeClass` object:

```yaml
runtimeClass:
  create: true
  name: kata
  handler: kata
```

This only registers a reference to a runtime handler. It does not install Kata
Containers or configure the nodes. The handler must already exist and work on
eligible nodes.

The warm executor-pool Deployment does not set `runtimeClassName`. The API's
isolated Job dispatcher can set one, but that execution mode is not currently
operational with the checked-in executor. Creating the RuntimeClass therefore
does not add a VM boundary to the default executor pool.

## Credentials and secrets

The API loads database, storage, backend signing, and optional provider
credentials. Execution requests carry scoped storage credentials and an
executor JWT to the pool.

Prefer the chart's existing-secret options:

```yaml
jwt:
  existingSecret: flow-like-jwt

storage:
  provider: azure
  azure:
    existingSecret: flow-like-storage

database:
  type: external
  external:
    existingSecret: flow-like-database
```

Check the required keys in the corresponding chart template before creating a
Secret. Keep secret values out of ordinary Helm values, rendered-manifest logs,
and source control.

After rebuilding the Kubernetes API with its currently omitted AWS feature,
AWS workload identity can use an annotated ServiceAccount and a storage Secret
that contains provider/bucket configuration without static access keys. Verify
the AWS credential chain and STS `RUNTIME_ROLE_ARN` path end to end. GKE and
AKS identity integrations likewise require provider- and cluster-specific
configuration; the chart does not enable them automatically.

The executor-pool template can inject OpenRouter and OpenAI keys into the
process environment. Component Model WASM runtimes currently inherit the
executor environment for language-runtime compatibility. Treat executor
environment variables as visible to untrusted component code and keep them
minimal. Prefer request-scoped host services where possible.

## Service accounts and RBAC

The chart creates one ServiceAccount and binds a namespaced Role that can
create, delete, get, list, watch, update, and patch Jobs, plus read pods and pod
logs. Both the API and executor pool use that ServiceAccount.

That is broader than the warm pool needs. For a hardened deployment:

- give the API a Job-management ServiceAccount only if Kubernetes Job dispatch
  is enabled;
- give the executor pool a separate ServiceAccount without Job mutation rights;
- remove unused verbs;
- keep all permissions namespaced;
- audit which workloads can read Secrets.

The current chart does not expose separate ServiceAccount values for those
workloads, so least-privilege separation requires chart customization.

## NetworkPolicy

When `networkPolicy.enabled=true`, the chart renders API and executor policies.
Read their selectors and rules literally:

- API ingress allows port `8080` from any namespace.
- API egress allows selected cluster ports to any namespace and HTTPS to
  `0.0.0.0/0`.
- The executor policy selects `app: flow-like-executor`.
- The warm executor pool instead has
  `app.kubernetes.io/component: executor-pool`.
- The isolated Job builder sets `app: flow-like-executor` on the Job object,
  not on the pod template.

The executor policy therefore does not currently select the default pool and
should not be assumed to select generated Job pods. Its empty ingress and
`allowedEgress` rules provide no protection to pods it does not match.

Check actual coverage:

```bash
kubectl get pods --namespace flow-like --show-labels
kubectl get networkpolicy --namespace flow-like --output yaml
kubectl describe networkpolicy --namespace flow-like
```

Your CNI must enforce NetworkPolicy. When egress restriction matters, add a
policy whose selector matches the real executor-pool labels and test DNS,
database, API callback, object storage, OAuth, model-provider, compiler, and
Tempo traffic explicitly.

## Pod security

`executor.securityContext` and `podSecurityPolicy.enabled` appear in
`values.yaml`, but the current executor-pool template does not consume the
former and the chart does not render a PodSecurityPolicy from the latter.
PodSecurityPolicy is also removed from modern Kubernetes.

Enforce pod hardening through the actual workload specs and cluster admission:

- run as a non-root UID;
- disallow privilege escalation;
- drop Linux capabilities;
- use a read-only root filesystem where compatible;
- set seccomp to `RuntimeDefault`;
- restrict host namespaces, host paths, and privileged containers;
- apply Pod Security Admission or an equivalent policy engine.

Render the chart and verify the resulting `securityContext`; do not infer it
from unused values.

## Supply chain

- Pin images by immutable digest for production.
- Scan API, executor, web, compiler, database-migration, and monitoring images.
- Sign images and verify signatures through admission policy.
- Protect the registry and restrict mutable tags.
- Review WASM package publisher, binary hash, permissions, and version before
  installation.
- Maintain an inventory of chart, application, and WASM dependencies.

## Validation checklist

Render the exact production values:

```bash
helm template flow-like apps/backend/kubernetes/helm \
  --namespace flow-like \
  --values values-production.yaml > /tmp/flow-like-rendered.yaml
```

Then verify:

- no plaintext production secrets are embedded;
- API and executor use the intended ServiceAccounts;
- network-policy selectors match real pod labels;
- public Ingress exposes only intended Services and ports;
- every RuntimeClass exists on eligible nodes;
- security contexts are present in rendered pods;
- image references are immutable;
- `EXECUTION_BACKEND` is not `kubernetes_job` with the checked-in executor.

## Related

- [Executor](/self-hosting/kubernetes/executor/)
- [Execution Backends](/self-hosting/execution-backends/)
- [Storage](/self-hosting/kubernetes/storage/)
- [WASM Sandboxing](/dev/wasm-nodes/sandboxing/)
