---
title: Database
description: Choose and migrate the SQL database used by the Kubernetes backend.
sidebar:
  order: 30
---

The API stores platform state in PostgreSQL or CockroachDB. Choose the database
topology independently of API and execution replica counts.

## Internal database

`database.type=internal` deploys one CockroachDB Pod:

```yaml
database:
  type: internal
  internal:
    replicas: 1
    persistence:
      storageClass: ""
      size: 10Gi
```

It runs `start-single-node --insecure` for evaluation and development.
Increasing replicas does not form a CockroachDB cluster; the chart requires one
internal node. Use an externally operated service for production.

## External database

```yaml
database:
  type: external
  external:
    provider: postgresql
    existingSecret: flow-like-database
  pool:
    maxConnections: 10
    minConnections: 1
```

The Secret contains a complete `DATABASE_URL` for both API and migration Job.
Set `provider=cockroachdb` only for a CockroachDB target. Supply the URL through
the [setup workflow](/self-hosting/kubernetes/installation/#generate-private-configuration)
or your existing Secret manager; keep it out of ordinary Helm values.

Use the database operator's required TLS settings. Private endpoints may need
`networkPolicy.controlPlaneExtraEgress`.

## Connection budget

`database.pool.maxConnections` and `minConnections` apply per API process,
with defaults of ten and one. Multiply the maximum by the greatest API replica
count, include rollout surge and other database clients, and leave capacity for
administration and migrations.

A minimum of zero lets idle API pools release all connections. The maximum must
be positive and at least the minimum. More API replicas can exhaust the database
even when individual Pods are lightly loaded.

## Schema application

`database.migration.enabled=true` creates a release-specific migration Job.
It waits for the database, selects the PostgreSQL or CockroachDB schema, runs
pre-push SQL and applies the Prisma schema. API init containers wait for the
release's Job to finish before starting.

The Job is an ordinary release resource, not a pre-install or post-install hook.
Its completed result remains available for replacement API Pods until the next
upgrade.

The migration command currently uses `prisma db push --accept-data-loss`.
Review schema changes and back up important data before upgrading. If an external
migration process owns schema changes, set `database.migration.enabled=false`
and apply the required schema before rolling out the API.

The older workstation `scripts/migrate-db.sh` has a separate development
configuration path. Its `--docker` mode expects a Compose service absent from
this directory. Prefer the chart Job for Helm installations.

## Verify and recover

```bash
kubectl get jobs -n flow-like -l app.kubernetes.io/component=db-migration
kubectl logs -n flow-like -l app.kubernetes.io/component=db-migration
kubectl rollout status deployment/flow-like-api -n flow-like
kubectl port-forward service/flow-like-api 8083:8080 -n flow-like
```

`GET /health/ready` checks required database tables and columns plus the
execution-state store within two seconds. A failed migration or unavailable
dependency keeps the API out of Service endpoints.

A Helm rollback does not roll back SQL schema or restore data. Restore database,
object storage, service credentials and execution records consistently, then
reconcile accepted runs before resuming dispatch.
