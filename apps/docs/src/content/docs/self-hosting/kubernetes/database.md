---
title: Database
description: Choose and migrate the SQL database used by the Kubernetes backend.
sidebar:
  order: 30
---

The Flow-Like API stores platform data in a PostgreSQL-compatible relational
database. The Prisma schema is under `packages/api/prisma/schema/`.

## Internal Database

`database.type: internal` deploys one CockroachDB pod:

```yaml
database:
  type: internal
  internal:
    replicas: 1
    persistence:
      storageClass: ""
      size: 10Gi
```

This workload runs CockroachDB with `start-single-node --insecure`. It is a
convenient evaluation and development database, not a highly available or
TLS-hardened production topology.

Do not increase `database.internal.replicas`. Multiple independent
`start-single-node` pods do not form a CockroachDB cluster.

## External Database

Use an externally operated PostgreSQL or CockroachDB service for production:

```yaml
database:
  type: external
  external:
    existingSecret: flow-like-database
```

The named Secret must contain one key:

```dotenv
DATABASE_URL=postgresql://flowlike:replace-me@database.example.com:5432/flowlike?sslmode=require
```

The URL is consumed by both the API and the migration Job. Keep it in an
externally managed Kubernetes Secret; avoid `database.external.connectionString`
in shared or committed values files.

See [Installation](/self-hosting/kubernetes/installation/#6-use-an-external-database-in-production)
for a command that creates the Secret without putting the URL in shell
history.

## Schema Application

The chart enables `database.migration` by default:

```yaml
database:
  migration:
    enabled: true
    image:
      repository: registry.example.com/flow-like/migration
      tag: replace-me
      pullPolicy: IfNotPresent
```

The Job waits on `DATABASE_URL`, then runs Prisma schema push during install
and upgrade. For the internal database it is a post-install hook; for an
external database it is a pre-install hook.

The current migration command uses `prisma db push --accept-data-loss`.
Review that behavior against your backup, change-management, and production
migration policy. To manage schema changes separately, set
`database.migration.enabled: false` and run an approved migration process
before rolling out the API.

The repository helper can run the same schema push from a trusted workstation.
Put `DATABASE_URL` in the ignored `apps/backend/kubernetes/.env` file first:

```bash
cd apps/backend/kubernetes
./scripts/migrate-db.sh
```

It also accepts `--docker` when the local Docker Compose migration service is
configured.

## Verification

Check the API's Secret reference and database readiness without printing the
credential:

```bash
kubectl get secret flow-like-database -n flow-like
kubectl logs -n flow-like -l app.kubernetes.io/component=db-migration
kubectl rollout status deployment/flow-like-api -n flow-like
```

Successful Helm hook Jobs may already have been deleted according to the
chart's hook policy. Use `helm status` and namespace events when no migration
pod remains.
