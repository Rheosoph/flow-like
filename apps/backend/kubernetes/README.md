# Flow-Like Kubernetes backend

The Helm chart deploys the API, a private RustFS object store, authenticated Redis,
and a Rust execution manager that prepares single-use gVisor Pods before dispatch.
This implementation still needs live cluster isolation and load qualification.
There is no measured few-millisecond start guarantee.
The [self-hosting documentation](../../docs/src/content/docs/self-hosting/kubernetes/overview.md)
covers installation, Helm values, security and day-to-day operation.

## Prepare an installation

Use a Linux cluster with Cilium, persistent-volume storage,
and a working `runsc` RuntimeClass on execution nodes. A RuntimeClass object names
an installed handler; creating the object does not install gVisor. Configure
Cilium with policy enforcement enabled and `allow-localhost=policy`, then finish
its DaemonSet rollout. Isolated mode installs mandatory Cilium deny policies for
runner access to local/remote nodes, Kubernetes API and metadata endpoints.
Standard Kubernetes NetworkPolicy alone permits local-node traffic. Explicit
host ingress rules preserve kubelet probes. The deploy script verifies the
installed Cilium CRD, configuration and DaemonSet state; every prepared slot also
checks that prohibited connections fail before admission. Review
[Cilium deny precedence](https://docs.cilium.io/en/stable/security/policy/deny/)
and [host entities](https://docs.cilium.io/en/stable/security/policy/layer3/).
The manager
needs namespace-scoped permission to create Pods, NetworkPolicies and cancellation
markers. Put this installation in a dedicated namespace and restrict who can edit
its Pod labels or NetworkPolicies.

Install Docker with BuildKit, Helm 3, kubectl, Python 3 and OpenSSL on the build
machine. Configure a container registry the cluster can pull from. Set the actual
browser origins before generating configuration:

```bash
cd apps/backend/kubernetes
export PUBLIC_API_URL=https://api.flow-like.example.com
export PUBLIC_WEB_URL=https://app.flow-like.example.com
export S3_PUBLIC_ENDPOINT=https://s3.flow-like.example.com
cp flow-like.config.example.json ../../../flow-like.kubernetes.config.json
# Edit that file: OIDC issuer/client/JWKS, domain, web origin and signaling URL.
export FLOW_LIKE_CONFIG=flow-like.kubernetes.config.json
# For production, export DATABASE_URL for the external database as well.
./scripts/setup-config.sh
REGISTRY=registry.example.com/flow-like TAG=release-2026-09 PUSH=true ./scripts/build-images.sh
```

Setup writes `.generated/secrets.yaml` with mode 0600 and matching non-secret Helm
values. It generates a matching ES256 keypair, distinct RustFS root/API/STS
identities, Redis credentials and API secrets. It refuses to overwrite existing
files, so an upgrade cannot accidentally replace credentials. Back up the secrets
with the database and storage. Values and credentials from environment variables
are read as data; the script does not source shell files.

`FLOW_LIKE_CONFIG` is relative to the repository root and is embedded into the API
binary at build time. Use it for public hub and OIDC settings; keep credentials in
Secrets. Changing Helm runtime values does not change the embedded hub. Rebuild
the API after changing that file. The default is the Kubernetes self-hosting
example, so a build does not inherit the repository's hosted-service configuration.
Generated secret files are excluded from every root Docker build context.

The build script builds the API, executor, manager, queue bridge, compiler,
signaling, migration, RustFS initializer and web images. It writes
`.generated/values-images.yaml`; pushed manager and executor digests are required
by isolated execution. Building selected components is available through
`COMPONENTS="api execution-manager"`. Partial builds merge their image entries into
the existing generated image values file.

Review [values-production.yaml](helm/values-production.yaml) for ingress, external
database, worker placement and replica settings. Copy the required settings into
your generated values or a separate operator override. Configure TLS certificates
for the API, web and object-store domains, and make the public S3 origin resolvable
and reachable from both browsers and Pods. Requests are signed for that exact
origin, so an ingress must preserve the Host header and path.

```bash
kubectl create namespace flow-like --dry-run=client -o yaml | kubectl apply -f -
kubectl apply -f .generated/secrets.yaml
./scripts/deploy.sh -f helm/values-production.yaml -f .generated/values-generated.yaml -f .generated/values-images.yaml
```

Replace the production example's registry, endpoint and worker-placement settings
before deploying. The example intentionally leaves image digests empty. The last
image values file supplies real digests. The deploy script lints and renders before
changing the release, then waits for workloads and initialization Jobs. It does
not apply or rotate Secrets. Its default timeout is 20 minutes; `HELM_TIMEOUT`
controls the limit. Set `KUBECONFIG` and its selected context before deploying;
Helm and the Cilium prerequisite checks use that same target. Set the namespace
through `K8S_NAMESPACE`. Per-command cluster/identity overrides are rejected.

## Execution capacity and isolation

The API sends interactive work to the manager. Background work enters the retained
Redis queue and a separate queue bridge forwards it to the same manager. Each
prepared slot contains a gVisor runner Pod and a trusted gateway Pod. A policy
allows the runner to reach only its own gateway. The gateway permits the signed
run's callbacks, authorized object store and explicitly allowed HTTPS hosts.
Workflow Pods receive no Kubernetes token, Redis password, storage root key or API
signing key. The environment is discarded after one execution.

| Value | Effect |
| --- | --- |
| `executionManager.replicaCount` | Independent manager partitions and warm reserves |
| `executionManager.workerThreads` | Async worker threads per manager, default 2; independent of active execution capacity |
| `executionManager.maxConcurrentExecutions` | Per-manager active execution limit, bounded by available CPU and memory |
| `executionManager.warmPoolSize` | Additional clean slots to prepare before dispatch |
| `executionManager.queueBridge.replicaCount` | Number of Redis dispatch consumers |
| `executionManager.queueBridge.concurrency` | Jobs each consumer can hold while waiting or executing |
| `executionManager.sandbox.nodeSelector` | Execution node placement |
| `executor.timeout` | Workflow execution budget, default 3,600 seconds |
| `execution.queueMaxWaitSeconds` | Maximum queued wait before retained quarantine, default 300 seconds |

See the [execution manager protocol](execution-manager/README.md) for the per-slot
lifecycle, cancellation and admission details. A consumed slot is replenished
asynchronously. Increasing replicas on fully loaded nodes does not add CPU or
memory. Reserve capacity on independently resourced
execution nodes and benchmark refill rate as well as queue-to-first-node p95/p99.
A warm process removes process startup from admission; signed artifact preparation,
credential issuance and workflow initialization can still add latency. Immediate
requests are refused when no clean slot is ready.

Hour-long runs use a 7,200-second RustFS session request, a 300-second queue age
budget and a 120-second credential margin, plus startup, terminal and cleanup
allowances. The API checks the provider's actual remaining credential lifetime;
the default Kubernetes settings require 4,140 seconds at checkout. External AWS
roles must permit the requested session length. A one-hour role-chained session
cannot support these hour-long runs with the required margin. Accepted queue
delivery is retained until trusted terminal confirmation; uncertain delivery is quarantined instead of replayed.
The v3 queue uses persistent connections and notifications rather than a fixed
250 ms polling delay. Drain or reconcile every v2 job before switching all APIs
and consumers together to `exec:jobs:v3`.

For explicitly trusted local workflows, `execution.isolationMode=trusted_shared`
and `execution.asyncBackend=http` enable the existing executor pool. That mode
shares a process between executions
and does not meet the multi-tenant per-execution requirement.
`K3D_EXECUTION_MODE=trusted_shared ./scripts/k3d-setup.sh` uses that explicit local mode and still requires a browser-and-Pod reachable `S3_PUBLIC_ENDPOINT`.
The chart refuses isolated mode without NetworkPolicy or with the legacy
Kubernetes Job backend. A cluster without the Cilium CRD cannot install the mandatory isolated policy resources.

## Local development entry points

`scripts/dev-bootstrap.sh` generates the same private Secrets and values as
`setup-config.sh`; it does not rewrite `.env` or set database passwords.
`scripts/dev.sh` forwards `setup`, `rebuild`, `status` and `delete` to the k3d
workflow. Setup and rebuild require an explicit trusted-mode selection. Status
remains read-only and does not require that selection.

```bash
export K3D_EXECUTION_MODE=trusted_shared
export S3_PUBLIC_ENDPOINT=https://s3.dev.example.com
./scripts/dev-bootstrap.sh
./scripts/dev.sh setup
./scripts/dev.sh status
```

Use an S3 origin reachable by both the browser and cluster, and complete the public
hub/OIDC configuration before building. Existing generated files are preserved;
subsequent setup uses them. For per-execution tenant isolation, use the Linux
runsc/Cilium installation path above. The production example disables the trusted
executor pool; isolated capacity is configured under `executionManager`.

## Object storage and external services

Bundled RustFS is pinned to `1.0.0-rc.5` by digest. It is a release candidate and
this chart deploys one persistent storage Pod. Multiple API or gateway replicas do
not make the data store highly available. Use a qualified external distributed
store when storage host failover is required.

The initialization Job creates separate private metadata, content and log buckets,
restricted application and STS issuer users, CORS and temporary-content cleanup.
It checks existing IAM policy/user drift instead of silently replacing access
rules. API Pods wait for the release's initializer to finish. Root credentials
are mounted only into RustFS and its initializer. The public object gateway
serves configured bucket paths and blocks STS, root and administration routes.
Prefix authorization is enforced by the temporary STS credentials.

| Secret reference | Required keys |
| --- | --- |
| `jwt.existingSecret` | `BACKEND_KEY`, `BACKEND_PUB`, `BACKEND_KID` |
| `api.existingSecret` | `SINK_TOKEN_ENCRYPTION_KEY`, `SINK_SECRET`, `MAINTENANCE_TOKEN` |
| `execution.existingSecret` | `EXECUTION_MANAGER_TOKEN` |
| `rustfs.existingSecret` | `RUSTFS_ROOT_USER`, `RUSTFS_ROOT_PASSWORD` |
| `storage.s3.existingSecret` | `AWS_ACCESS_KEY_ID`, `AWS_SECRET_ACCESS_KEY`, `STS_ISSUER_ACCESS_KEY`, `STS_ISSUER_SECRET_KEY` |
| `redis.auth.existingSecret` | `REDIS_PASSWORD`, complete URL-encoded `REDIS_URL` |
| `redis.externalExistingSecret` | Complete authenticated `REDIS_URL`, usually `rediss://` |
| `database.external.existingSecret` | `DATABASE_URL` |

To use external S3-compatible storage, set `rustfs.enabled=false`, keep
`storage.provider=s3`, and supply `storage.s3.publicEndpoint`, `internalEndpoint`,
`stsEndpoint` and the credential Secret. `runtimeCredentialsProvider` selects the
STS policy dialect (`rustfs` or `aws`); the actual credential implementation is
AWS-compatible. The provider must enforce a session policy narrower than the
issuer's base policy. See the [storage verification contract](../docker-compose/object-store/README.md#verification)
for allowed and denied operations to test against the exact store version.

The isolated gateway currently supports the S3 configuration. Other chart storage
providers remain available in `trusted_shared` mode.

For an HTTPS object origin in isolated mode, enable
`executionManager.objectStoreTlsGateway` only when the endpoint exposes a
bucket-only data API. A TLS tunnel cannot inspect administration paths. Add narrow
`networkPolicy.executionGatewayExtraEgress` rules if that endpoint resolves to
private addresses. External databases, Redis and custom DNS may need
`networkPolicy.controlPlaneExtraEgress` or an adjusted DNS namespace selector.
NetworkPolicy IP matching before or after Service translation depends on the CNI;
verify these paths in the target cluster.

The trusted execution manager also uses Redis for atomic replay claims. Redis
credentials stay in control-plane Pods and never enter runner or gateway Pods.
Bundled Redis currently shares one broadly authorized account between those
trusted components; per-component Redis ACLs remain follow-up work.

Bundled Redis uses authentication, AOF persistence, `noeviction`, resource limits,
readiness probes and a Recreate rollout to avoid two writers on its PVC. It remains
a single instance with an approximately one-second AOF persistence window.
Set `redis.enabled=false` and `redis.externalExistingSecret` for an external
service. Use URL-encoded credentials; the chart never interpolates a raw password
into a URL. A `rediss://` endpoint uses the client's normal CA verification. Private
CAs need the application's supported trust-store configuration.

## Execution image upgrades

Before replacing execution images, pause new dispatch and drain managers and queue
bridges. Build and pin the manager/gateway image together with the executor image
that contains `/app/execution-slot`. Preserve Redis replay claims, cancellation
markers and signing keys; reconcile uncertain work before resuming. Do not clear
claim state or switch queue versions during active delivery. The
[native supervisor guide](../execution-manager/README.md#build-verify-and-upgrade)
describes the compatible wire and ownership formats.

## Database migrations

Each Helm release creates a database migration Job. API pods wait for that
Job to complete before starting, and `/health/ready` checks the database and
required application tables with a two-second timeout. A failed migration
keeps the new API pods out of service. Helm retains the completed Job until
the next upgrade so replacement pods can perform the same check.

External databases use PostgreSQL by default. Set
`database.external.provider=cockroachdb` only for a CockroachDB target. Both
`database.external.connectionString` and a pre-existing database Secret work
on first installation.

Set `database.pool.maxConnections` and `database.pool.minConnections` to fit
the database connection budget. Their defaults are 10 and 1 per API process.
Multiply the maximum by the largest replica count, include rollout surge and
worker connections, and leave room for database administration. The same
settings are available as `DATABASE_POOL_MAX_CONNECTIONS` and
`DATABASE_POOL_MIN_CONNECTIONS` in other PostgreSQL deployments, including
Azure and GCP. A minimum of zero permits an idle pool to release every
connection; the maximum must be positive and at least the minimum.

## Other services and operating checks

Set compiler concurrency with `compiler.maxConcurrentJobs` and
`compiler.maxParallelTargets`. Storage allowlists retain the complete scheme,
host and port, including HTTP gateways. Compiler and API HPAs own replica counts
when enabled. For signaling replicas greater than one, set
`signaling.fanoutMode=redis` and include the exact browser origin in
`signaling.allowedOrigins`. Configure the hub's signaling URL to this deployment's
WSS endpoint so its JWTs use the same public key.

The static web image uses the complete Bun workspace and a read-only runtime with
private temporary storage. Nginx avoids logging signed query strings. Ingress
streaming timeouts default to 3,660 seconds for nginx controllers; configure
corresponding settings for another controller. Also review controller error logs,
load-balancer timeouts and rollout behavior for long connections.

```bash
kubectl get pods,jobs -n flow-like
kubectl logs deployment/flow-like-execution-manager -n flow-like
kubectl logs deployment/flow-like-queue-bridge -n flow-like
kubectl port-forward -n flow-like svc/flow-like-api 8080:8080
```

The chart tests run without a cluster:

```bash
python3 -m venv /tmp/flow-like-chart-tests
/tmp/flow-like-chart-tests/bin/pip install PyYAML==6.0.2
/tmp/flow-like-chart-tests/bin/python -m unittest discover -s scripts/tests -v
```

Run `helm test flow-like -n flow-like --logs` against a disposable or backed-up
store to check STS sibling-prefix, copy-source, missing-token, admin and presigned
URL behavior. The test uses only the API and issuer identities and removes its
unique temporary object prefixes. It does not test expiry, storage failover or
application throughput. Before exposing tenants, also verify cancellation, network-policy enforcement,
node loss, queue retention and representative load on the real cluster. Use an
hour-long fixture to verify storage access and terminal callbacks near expiry.
Warm slots and Helm rendering tests cannot establish a latency SLA.
