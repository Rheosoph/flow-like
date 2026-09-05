---
title: Troubleshooting
description: Diagnose configuration, isolated execution, object storage and recovery failures
sidebar:
  order: 27
---

Start with validation, container state and the affected service's recent logs:

```bash
python3 scripts/preflight.py
docker compose ps --all
docker compose logs --tail=100 db-init object-store-init api execution-manager queue-bridge
```

The initializers should exit successfully. Long-running services should become
healthy. Use `preflight.py --config-only` if the Docker daemon is unavailable.
Avoid posting rendered configuration or full workflow payloads in support logs.

## Setup or preflight fails

| Symptom | Check |
| --- | --- |
| Existing `.env` is refused | Setup protects existing secrets. Merge new settings into that file. |
| Image digest is absent or stale | Run `python3 scripts/prepare-images.py` on the target daemon. |
| `runsc` validation fails | Install gVisor and configure both `--network=none` and `--host-uds=open`. |
| Mixed execution profiles | Use `per-run` for `per_run`, or `trusted` for `trusted_shared`. |
| Warm pool is zero | The Rust manager requires at least one unused slot. |
| Database budget is exceeded | Reduce API pools/replicas or increase the reviewed PostgreSQL limit. |
| Stop grace is too short | Cover execution, startup, finalization and cleanup budgets. |
| Private environment permissions fail | Restore mode `0600`; keep the file out of Git. |

Do not bypass a gVisor failure by switching an untrusted installation to shared
workers.

## An initializer blocks startup

For a database failure:

```bash
docker compose logs db-init postgres
```

Confirm connectivity and the schema change being attempted. The initializer
uses a migration advisory lock and rejects destructive changes. Do not remove
these checks to force an upgrade. After correcting the cause, run the reviewed
initializer and rerun the startup helper.

For storage bootstrap:

```bash
docker compose logs object-store-init object-store
```

Confirm the bucket names and generated identities match the intended deployment.
Bootstrap refuses unexpected existing policies. A copied environment file with
new keys does not rotate an existing store's credentials. Preserve the original
configuration or perform an explicit, tested rotation.

The external-datastore overlay removes automatic database initialization.
Apply reviewed schema changes with `docker compose run --rm db-init` before
starting incompatible API images.

## Health and login

```bash
curl --fail http://localhost:8080/health
curl --fail http://localhost:3001/health
docker compose exec execution-manager /app/execution-manager healthcheck
docker compose exec api curl --fail http://execution-gateway:9000/ready
```

Manager and compiler ports are internal. Host port 9000 belongs to the object
data gateway by default.

For login or browser CORS failures, compare the hub's OIDC and signaling settings
with `PUBLIC_API_URL`, `NEXT_PUBLIC_*` and both allowed-origin lists. Rebuild
web/API images when their embedded configuration changes. Add the required
desktop origins explicitly.

## Executions are rejected or stop early

Inspect `executor_ready_sandboxes`, active capacity and preparation errors.
A depleted reserve returns explicit non-admission, allowing queue retries.
Check runner/gateway image availability, host CPU/memory/PIDs, gVisor settings
and the manager's writable state volume before increasing concurrency.

The runner has no container network. A denied HTTPS integration needs an exact
host grant and an HTTP client that honors the supplied proxy. Raw sockets,
arbitrary callback paths and general user-token API access are unavailable.

An insufficient credential-lifetime error means the provider's actual remaining
session cannot cover the configured queue wait, execution and grace periods.
Changing the requested TTL cannot exceed the provider's policy limits. There is
no automatic renewal.

Cleanup failure closes admission. Fix Docker or volume access and restart the
manager with its existing `execution_manager_state` volume so reconciliation
can find abandoned resources. Do not delete ownership/replay records to make
a repeated run appear new.

## Queued jobs do not progress

Check queue bridges, Redis health and manager readiness. The default queue is
`exec:jobs:v3`; producers and consumers must use the same version.

Inspect counts without printing queued credentials or payloads:

```bash
docker compose exec redis sh -c 'REDISCLI_AUTH="$REDIS_RUNTIME_PASSWORD" redis-cli --user runtime LLEN exec:jobs:v3'
docker compose exec redis sh -c 'REDISCLI_AUTH="$REDIS_RUNTIME_PASSWORD" redis-cli --user runtime HLEN exec:jobs:v3:pending'
docker compose exec redis sh -c 'REDISCLI_AUTH="$REDIS_RUNTIME_PASSWORD" redis-cli --user runtime LLEN exec:jobs:v3:dead'
```

Ready jobs older than the configured wait limit and uncertain delivery attempts
are retained for reconciliation. Compare each affected run with application
state and external effects before authorizing another attempt. Do not bulk-move
retained payloads back to the ready list or clear them to bypass queue capacity.

Redis uses `noeviction`; full memory can stop admission and bookkeeping.
Investigate retention and host capacity before changing memory limits.

## Object requests fail

Use [Storage](/self-hosting/docker-compose/storage/) to check that every client,
compiler and run gateway reaches the same signed origin. Signature errors often
follow a changed host, port, path or proxy rewrite. Browser failures can also
come from storage CORS or an unreachable `s3.localhost` name.

For HTTPS storage, verify the public endpoint forwards bucket data and denies
STS/admin routes before enabling its gateway contract flag. Test temporary
prefix-scoped credentials through both the public gateway and private store
with the supplied conformance script.

## Recovery and removal

Back up database state, objects and IAM metadata, Redis, keys/configuration and
the manager's local ownership database. Restore onto a clean host and verify
authorization, representative workflows and any uncertain deliveries before
reopening traffic.

`docker compose down` retains named volumes. `down --volumes` deletes bundled
application, object-store, replay and monitoring state. It is a removal command,
not a recovery procedure.
