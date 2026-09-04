# Azure migration job

This image applies `packages/api/prisma/schema` to the Azure Database for
PostgreSQL Flexible Server database with `prisma db push`, authenticated as the
`${name_prefix}-migration-identity` user-assigned managed identity. It is a
manually triggered Container Apps job: one execution, one push, exit code =
Prisma's. There is no password on this cloud; `migrate.ts` mints an Entra token
for `https://ossrdbms-aad.database.windows.net/.default` and uses it as the
PostgreSQL password of connection URLs that exist only in the environment of
the two child processes it spawns in turn: `packages/api/prisma/pre-push.ts`
(column type changes `db push` emits without the `USING` clause they need on an
existing database; idempotent) and Prisma.

`--accept-data-loss` is never passed. When the diff would destroy or narrow
anything, Prisma prints the exact warnings, refuses (the job has no TTY) and
exits 1. The operator reviews the change and, if it is intended, applies the
destructive step by hand from the management host with a short-lived
migration-identity token, then re-runs the job. See the Dockerfile header and
`docs/azure/deployment-runbook.md`.

Required environment variables (same contract and validation as
`packages/azure-data/src/postgres.rs`, which the Azure API and queue workers use):

- `AZURE_POSTGRES_AUTH_MODE`: must be exactly `managed_identity`
- `AZURE_POSTGRES_HOST`: the Flexible Server FQDN (`*.postgres.database.azure.com`)
- `AZURE_POSTGRES_DATABASE`: the database name
- `AZURE_POSTGRES_USER`: the identity's name (`${name_prefix}-migration-identity`),
  bound to its object ID with `pgaadauth_create_principal_with_oid`
- `AZURE_CLIENT_ID`: the migration identity's client ID (UUID)
- `IDENTITY_ENDPOINT`, `IDENTITY_HEADER`: injected by Container Apps

Forbidden (the job refuses to start when any of them is present, even empty):
`DATABASE_URL`, `PGPASSWORD`, `POSTGRES_PASSWORD`, `AZURE_POSTGRES_PASSWORD`,
`AZURE_POSTGRES_CONNECTION_STRING`, `AZURE_POSTGRESQL_CONNECTIONSTRING`, every
other libpq `PG*` source (`PGHOST`, `PGPORT`, `PGUSER`, `PGDATABASE`,
`PGSSLMODE`, `PGSSLROOTCERT`, `PGSSLCERT`, `PGSSLKEY`, `PGPASSFILE`,
`PGSERVICE`, `PGSERVICEFILE`, `PGHOSTADDR`, `PGOPTIONS`, `PGAPPNAME`),
alternate identity endpoints (`MSI_ENDPOINT`, `MSI_SECRET`, `IMDS_ENDPOINT`,
`IDENTITY_SERVER_THUMBPRINT`, `AZURE_AUTHORITY_HOST`) and proxies
(`HTTP_PROXY`, `HTTPS_PROXY`, `ALL_PROXY` and lowercase forms).

Behavior:

- Exit `2` when the environment is refused, `1` when the token cannot be
  acquired or a child is terminated by a signal, otherwise the pre-push
  runner's exit code when it fails (Prisma is then not started) or Prisma's.
- The connection URL uses `sslmode=require&sslaccept=strict`, which is how
  Prisma's engine spells verify-full (it does not implement libpq's `verify-*`
  modes and would silently skip certificate checks on them); the chain is
  verified against the image's CA store and the hostname against
  `AZURE_POSTGRES_HOST`. `application_name=flow-like-azure-migration` for
  `pg_stat_activity`. The pre-push runner connects with node-postgres, which
  gets the same posture in its own spelling
  (`uselibpqcompat=true&sslmode=verify-full`).
- The tracked schema declares `provider = "cockroachdb"`; the image rewrites the
  datasource provider to `postgresql` at build time (the same edit
  `packages/api/scripts/make-postgres-prisma-mirror.sh` makes for every plain-
  PostgreSQL target) and runs `prisma validate` on the result, so the image
  fails to build rather than the job failing to push.
- SIGTERM is forwarded to Prisma so a cancelled or timed-out execution does not
  leave it mid-statement.

Build from the `flow-like` root; the Dockerfile copies
`apps/backend/azure/migration/{package.json,bun.lock,prisma.config.ts,migrate.ts}`,
`packages/api/prisma/schema` and `packages/api/prisma/{pre-push.ts,pre-push/}`:

```sh
docker build -f apps/backend/azure/migration/Dockerfile -t flow-like-azure-migration .
```

Local checks: `bun install`, `bun test`, `bunx tsc --noEmit`.
