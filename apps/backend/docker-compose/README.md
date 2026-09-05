# Docker Compose self-hosting

This deployment bundles PostgreSQL, authenticated Redis, and RustFS object storage. Its default execution mode launches a fresh gVisor sandbox for each run. API, compiler, web, signaling, queue bridges, and execution managers have independent replica and resource settings. Compose remains a single Docker host deployment: extra containers do not provide host failover.

RustFS is pinned to the multi-platform digest for `1.0.0-rc.5`, a release candidate. Qualify the exact images for storage authorization, sandbox isolation, failure recovery and representative load before admitting untrusted tenants. The [self-hosting documentation](../../docs/src/content/docs/self-hosting/docker-compose/overview.md) covers prerequisites, configuration, monitoring and troubleshooting.

## New installation

Use a Linux Docker daemon that supports Engine API 1.47, Docker Compose 2.24.4 or newer, Python 3, OpenSSL, and sufficient memory/disk for the configured limits and Rust builds. Docker introduced API 1.47 in Engine 27.2; preflight checks the daemon's supported range. See the [Docker API version matrix](https://docs.docker.com/reference/api/engine/#api-version-matrix).

The defaults reserve capacity for two APIs, a compiler, PostgreSQL, Redis, RustFS, ten active runs and two additional warm slots. Every slot also needs its gateway. Set run concurrency and memory limits for your host before starting.

Configure gVisor in the execution daemon's `/etc/docker/daemon.json`, preserving any existing daemon settings:

```json
{
  "runtimes": {
    "runsc": {
      "path": "/usr/local/bin/runsc",
      "runtimeArgs": ["--network=none", "--host-uds=open"]
    }
  }
}
```

Install `runsc` following the [gVisor Docker guide](https://gvisor.dev/docs/user_guide/quick_start/docker/) and restart the daemon after merging the settings. The manager verifies both runtime arguments. The sandbox receives only its own Unix proxy socket; it receives no Docker socket or host directory.

From this directory:

```bash
python3 scripts/setup-env.py
# Review .env and the public URLs in flow-like.config.example.json.
python3 scripts/prepare-images.py
python3 scripts/preflight.py
python3 scripts/up.py --build
```

`setup-env.py` creates `.env` with mode `0600`. It generates independent database, Redis, root storage, API storage, issuer, service-auth, and ES256 signing credentials. It refuses to overwrite an existing file and never prints credentials. `prepare-images.py` builds the runtime and execution-manager images and pins their immutable local image IDs in `.env`. Re-run it when upgrading either image. Other images build during `up.py --build`.

Initial startup creates private metadata, content, and log buckets and separate restricted API and STS issuer identities. It also applies the schema under a PostgreSQL advisory lock before starting the API. Bootstrap failures prevent the API from starting. The migration image contains pinned dependencies and selected schema/migration files; it does not install packages at startup or accept destructive schema changes automatically.

Default published listeners bind to loopback:

| Address | Service |
| --- | --- |
| `http://localhost:3001` | Web application through the edge proxy |
| `http://localhost:8080` | API through the edge proxy |
| `ws://localhost:4444` | Authenticated signaling through the edge proxy |
| `http://s3.localhost:9000` | S3 data gateway |

PostgreSQL, Redis, compiler, runtime/manager, RustFS administration, and metrics have no published ports. Use an operator-managed TLS reverse proxy for public hosting. Explicitly configure its trusted addresses in `proxy/nginx.conf` before accepting forwarded client headers. The supplied proxy overwrites client-provided forwarding headers. Access logs omit queries and headers; S3 access logging and nginx request-context error logs are disabled because they can contain presigned credentials. Use API metrics and sanitized status logs to investigate failures.

Set `PUBLIC_API_URL`, the `NEXT_PUBLIC_*` URLs, `CORS_ALLOWED_ORIGINS`, and `REALTIME_ALLOWED_ORIGINS` to the exact browser origins. Rebuild the web image after changing its public URLs. Update the signaling URL in the hub configuration. Add desktop origins to both `CORS_ALLOWED_ORIGINS` and `REALTIME_ALLOWED_ORIGINS` when needed: `tauri://localhost,http://tauri.localhost,https://tauri.localhost`.

## Storage endpoints and credentials

The gateway/web images use digest-pinned Nginx `1.30.4`; the bundled store uses digest-pinned RustFS `1.0.0-rc.5`. Review and advance these pins through the same qualification when applying security updates.

`S3_PUBLIC_ENDPOINT` is the exact origin used for signed object requests by the API, compiler, desktop/browser clients, and disposable run gateways. All must reach that origin. Never change a signed URL's host, port, or path. For the local example, the `s3.localhost` alias resolves to the S3 gateway inside Compose and loopback on the operator's computer. If your client resolver does not support `.localhost`, add an explicit hosts entry.

For a public TLS endpoint such as `https://storage.example.com`, route that hostname through your TLS proxy to the S3 gateway and set `S3_GATEWAY_ALIAS=object-gateway`, so Compose does not override the public hostname with a plaintext container listener. Keep the full public origin in `COMPILER_ALLOWED_STORAGE_HOSTS`. For per-run execution with HTTPS, also set `EXECUTION_OBJECT_STORE_TLS_GATEWAY=true` only after verifying that the TLS origin forwards bucket data and rejects STS and administration. The sandbox proxy cannot inspect paths inside a TLS CONNECT tunnel, so raw storage/admin endpoints do not satisfy this contract. `S3_INTERNAL_ENDPOINT` serves direct internal LanceDB access; `STS_ENDPOINT_URL` serves private issuance. The API's general object-store client uses the public origin so presigned URLs remain reachable.

Root RustFS keys reach only RustFS and bootstrap. The API uses a normal IAM user for storage and a separate normal IAM user for STS issuance. Sandboxes receive scoped temporary credentials in their authenticated run payload. The S3 gateway only forwards configured bucket paths; it rejects root, STS, and administration requests. RustFS still enforces bucket and prefix policies behind the gateway. Session expiry comes from STS. Killing a sandbox does not instantly revoke exported credentials.

Run the storage contract checks against disposable test prefixes before admitting tenants:

```bash
docker compose build object-store-init
docker compose up -d object-store object-gateway
docker compose run --rm object-store-init
docker compose run --rm --entrypoint python object-store-init /opt/object-store/conformance.py
```

See the [object-store verification contract](object-store/README.md#verification) for the probes and the remaining expiry, restore and failover checks required when qualifying a store/version.

## Execution and capacity

`EXECUTION_ISOLATION_MODE=per_run` with `COMPOSE_PROFILES=per-run` is the default. Interactive requests use the execution manager over HTTP; background requests enter the authenticated Redis queue. Queue bridges consume jobs and ask the manager to launch disposable runs. Queue bridges execute no workflow code in this mode.

Each run has a read-only root, bounded CPU/memory/PIDs/tmpfs, no container network, and a private Unix-socket HTTP proxy. The proxy permits only the run's callback routes, configured S3 buckets, and explicit HTTPS integration hosts in `EXECUTION_ALLOWED_HTTPS_HOSTS`. Raw TCP/UDP integrations and broad API/PAT calls are unsupported in this profile. Configure supported HTTP clients to honor the supplied proxy. An unavailable sandbox or denied integration fails the run; it does not fall back to a shared worker.

The trusted execution manager mounts the Docker socket and can create/remove containers. Treat it as host administration code, restrict access to the host, and place execution on dedicated hosts when scaling beyond Compose. Keep the manager token outside workflows. The per-run gateway receives no host Docker socket or storage root credentials.

| Setting | Capacity controlled |
| --- | --- |
| `API_REPLICAS`, `API_CPUS`, `API_MEMORY` | HTTP API capacity |
| `EXECUTION_MANAGER_REPLICAS`, `MAX_CONCURRENT_EXECUTIONS` | Manager count and admitted runs per manager |
| `EXECUTION_MANAGER_WORKER_THREADS` | Async worker threads per manager, default 2; separate from active execution capacity |
| `SANDBOX_WARM_POOL_SIZE`, `SANDBOX_CREATE_CONCURRENCY` | Pristine ready reserve per manager and parallel replenishment |
| `SANDBOX_IDLE_TIMEOUT_SECONDS`, `SANDBOX_STARTUP_TIMEOUT_SECONDS` | Ready inventory lifetime and preparation/assignment budget |
| `QUEUE_BRIDGE_REPLICAS`, `QUEUE_WORKER_CONCURRENCY` | Background dispatch capacity |
| `SANDBOX_CPUS`, `SANDBOX_MEMORY_MB`, `SANDBOX_PIDS`, `SANDBOX_TMP_MB` | Per-run limits |
| `COMPILER_REPLICAS`, `COMPILER_CPUS`, `COMPILER_MEMORY` | Compiler process capacity |
| `COMPILER_MAX_CONCURRENT_JOBS`, `COMPILER_MAX_PARALLEL_TARGETS` | Simultaneous HTTP compilation jobs and targets per job |
| `WEB_REPLICAS`, `SIGNALING_REPLICAS` | Static web and signaling capacity |
| `DATABASE_POOL_MAX_CONNECTIONS`, `POSTGRES_MAX_CONNECTIONS` | Connection budget |
| `EXECUTION_TIMEOUT_SECONDS` | Execution duration, with separate startup, terminal and cleanup grace |
| `EXECUTION_TERMINAL_GRACE_SECONDS`, `EXECUTION_CLEANUP_TIMEOUT_SECONDS` | Terminal acknowledgement and cleanup budgets |
| `EXECUTION_STOP_GRACE_PERIOD` | Stop grace for draining managers, queue bridges and trusted runtimes; default 65 minutes. Preflight rejects a value shorter than the configured execution and supervisor budgets. |
| `EXECUTION_QUEUE_MAX_DEPTH` | Bound for accepted ready, pending, and quarantined queue entries |
| `EXECUTION_QUEUE_MAX_WAIT_SECONDS` | Maximum ready queue age before retention for reconciliation |

The API, compiler, and execution-manager proxies refresh Docker DNS every ten seconds. Streaming responses disable buffering; execution POSTs are not retried after ambiguous acceptance. Increase service counts in `.env`, run preflight, and rerun `up.py`. Budget one additional API replica during rollout plus administrative connections. Measure CPU-heavy, I/O-heavy, model-heavy, and long-running workloads before attaching throughput expectations to counts.

The Rust manager prewarms single-use runner and gateway containers. Static runtime/catalog/key initialization happens before assignment; tenant code and credentials enter only after a slot is reserved. Used sandboxes are destroyed and replenished in the background. Budget the ready reserve in addition to active executions. Managers on one daemon share the local `execution_manager_state` volume for assignment and cancellation; preserve it across restarts and keep it off network filesystems. The Rust manager requires `SANDBOX_WARM_POOL_SIZE` of at least 1. `EXECUTION_MANAGER_WORKER_THREADS` defaults to 2 and controls async workers independently of execution capacity.

Credential checkout checks actual remaining provider lifetime against queue wait, execution and all grace periods, including cache hits. RustFS sessions and channel grants default to two hours. Automatic renewal is not implemented; insufficient grants fail before dispatch. Warm artifact loading and measured end-to-end millisecond starts remain unqualified. The [scaling guide](../../docs/src/content/docs/self-hosting/docker-compose/scaling.md) explains how to size active and warm capacity and measure replacement rate.

Queue consumers reuse connections and wake on notifications. Explicit non-admission safely requeues work with backoff and its original publication time. Uncertain or expired attempts remain in the reconciliation queue because external effects may already have occurred or grants may have expired. Reconcile them before retrying. The default queue is `exec:jobs:v3`. Drain or reconcile old queues and stop old managers before upgrading; never mix v2 and v3 producers/consumers. Redis uses AOF with one-second fsync and `noeviction`. Host/storage failure can still lose recent writes, and queued payloads contain sensitive run material. Back up and restrict Redis accordingly.

For a separate installation containing only trusted internal workflows, generate with `python3 scripts/setup-env.py --mode trusted`. This selects `EXECUTION_ISOLATION_MODE=trusted_shared`, `COMPOSE_PROFILES=trusted`, and the shared runtime proxy. `RUNTIME_REPLICAS` controls those workers. Preflight rejects mixed trusted/per-run profiles. Shared mode is unsuitable for hostile tenants.

## Existing installations and external services

Do not replace an existing `.env` with the new template or point existing apps at empty bundled buckets. Back up the database, current storage, Redis, hub config, and signing keys first. Merge new settings into your protected configuration. Keep existing bucket names and credentials until a separately verified migration completes. The initializer refuses unexpected existing policies rather than silently expanding them.

To retain external S3/STS, set:

```dotenv
OBJECT_STORE_MODE=external
COMPOSE_FILE=docker-compose.yml:docker-compose.external-store.yml
S3_STS_PROVIDER=aws
S3_INTERNAL_ENDPOINT=https://your-storage-origin
S3_PUBLIC_ENDPOINT=https://your-storage-origin
STS_ENDPOINT_URL=https://your-sts-origin
COMPILER_ALLOWED_STORAGE_HOSTS=https://your-storage-origin
```

Set the appropriate API and issuer credentials and `RUNTIME_ROLE_ARN` for AWS role-based issuance. Use `S3_STS_PROVIDER=rustfs` for a separately hosted qualified RustFS deployment. The external-store overlay removes the bundled store, gateway, and initializer from the active graph. It performs no bucket migration or initialization. Recheck your provider's prefix-scoped issuance, policy size, path-style, and TLS requirements.

For external PostgreSQL/Redis, add `docker-compose.external-datastores.yml` to `COMPOSE_FILE`, set `DATASTORE_MODE=external`, and provide `DATABASE_URL`, `REDIS_URL`, `RUNTIME_REDIS_URL`, `SIGNALING_REDIS_URL`, `SINK_REDIS_URL`, and `METRICS_REDIS_URL`. Percent-encode special characters in URL credentials. Apply reviewed schema updates explicitly with `docker compose run --rm db-init`; the external overlay removes automatic database initialization. Validate schema compatibility before rolling API instances. The current schema updater uses guarded Prisma `db push`; review every schema change and test restore because versioned PostgreSQL release migrations and an automatic schema rollback are not implemented.

For the Rust supervisor upgrade, pause new dispatch and drain managers and queue bridges before replacing images. Rebuild and pin both runner and manager/gateway images with `prepare-images.py`. Preserve the SQLite state volume, retained Redis deliveries and signing keys. The Rust port keeps the ownership record formats and dispatch protocol; clearing records can allow replay. Resume dispatch after preflight, readiness and storage checks pass. See the [execution manager upgrade contract](../execution-manager/README.md#build-verify-and-upgrade).

The legacy `docker-stack.yml` supports trusted workflows with external storage only. Swarm lacks Compose's migration-completion gate, so it now starts APIs at zero replicas unless `SWARM_API_REPLICAS_AFTER_MIGRATION` is explicitly set after successful schema migration. It is not a per-run isolation deployment or a host-failover solution for local volumes. Prefer the Compose path while Swarm-specific storage placement and upgrade checks remain unqualified.

## Monitoring and recovery

Add `monitoring` to `COMPOSE_PROFILES`, then rerun `up.py`. Grafana binds to `127.0.0.1:3002` and uses a generated password. Prometheus, Tempo, and exporters stay private. Set `OTEL_EXPORTER_OTLP_ENDPOINT=http://tempo:4317` to enable tracing; it is empty by default. Prometheus discovers individual API/compiler/execution replicas through DNS. Configure an Alertmanager receiver before expecting alert delivery; the supplied configuration has none.

Back up PostgreSQL, object data and RustFS IAM metadata, deployment configuration/signing keys, and any Redis state your recovery policy requires. Store encrypted copies away from this Docker host. A named volume is not a backup. Restore to a clean host and measure data loss/recovery time before setting an availability promise. Never use `docker compose down --volumes` as an upgrade command.

Local checks that do not start services:

```bash
python3 -m unittest discover -s scripts -p 'test_*.py' -v
python3 scripts/preflight.py --config-only
```

These render real Compose graphs, verify secret/network boundaries, exercise external overlays, and reject incompatible execution settings. For actual Redis Lua and ACL checks without Docker, set `REDIS_SERVER_BIN` to a Redis 7 executable and run `python3 tests/test_redis_acl.py -v`; it creates and destroys its own loopback server and test data. The optional `go -C tests/build-context test ./...` check uses Docker's own ignore-pattern matcher to verify that required workspace sources remain in the build context while synthetic deployment-secret paths are excluded.

These checks do not prove runtime isolation or storage authorization. Container builds, the live RustFS suite, hostile-run probes, failure recovery, and restore tests must run on the target Linux host.
