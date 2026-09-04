# AWS migration job (Aurora DSQL)

This image applies the committed DSQL migrations in
`packages/api/prisma/migrations-dsql` to an Aurora DSQL cluster as the `admin`
role. It is a one-off ECS Fargate task (not a Lambda: the initial migration is
736 statements plus a wait for 489 asynchronous index/validation jobs). There is
no password on this cloud; `migrate.ts` mints an IAM token with
`@aws-sdk/dsql-signer` (`getDbConnectAdminAuthToken`) from the default
credential chain and uses it as the PostgreSQL password of connections that
exist only in this process.

## Why not `prisma migrate deploy`

DSQL allows one DDL statement per transaction. The schema engine shipped with
Prisma 7.3.0 (prisma-engines `9d6ad21c`) sends a whole `migration.sql` as one
simple-query batch, which PostgreSQL wraps in an implicit transaction
(prisma/prisma#22922); newer engines split the batch with a PostgreSQL parser
that does not know `CREATE INDEX ASYNC` and fall back to the whole batch. So the
job applies each statement itself and keeps Prisma's bookkeeping:

1. Refuse the environment unless it matches the contract below.
2. Connect with verify-full TLS, take a 30-minute lease row in
   `_flow_migration_lock` (created if missing; DSQL's commit-time OCC lets
   exactly one racing run win). Exit `3` if another run holds it.
3. Create `_prisma_migrations` (Prisma's exact DDL) if missing, load applied
   rows, refuse if a row has `logs` and no `finished_at` (a failed migration)
   or a checksum differs from the file, and compute the pending list in
   directory order - the same diagnosis `migrate deploy` performs. A row with
   `applied_steps_count = 1`, no `logs` and no `finished_at` is a migration
   whose statements are all committed but whose async jobs were never
   confirmed (the run was killed mid-wait); it is finished in step 6, not
   re-applied.
4. Wait for any `sys.jobs` entry still `submitted`/`processing` from an
   earlier run (one cheap query when there is none).
5. Per pending migration: insert the started row (uuid v4 id, sha256 checksum)
   and run every statement as its own autocommit statement, retrying SQLSTATE
   `40001` / `OC000` / `OC001` and reconnecting before DSQL's 60-minute
   connection limit. A retry that hits `42P07`/`42710`/`42701` ("already
   exists") on a `CREATE …` / `ALTER TABLE … ADD …` counts as applied with a
   warning - DSQL can commit a DDL and still report OC001 for it - and for
   `CREATE INDEX ASYNC` the job id is recovered from `sys.jobs` by
   `object_name`. The `job_id` every `CREATE INDEX ASYNC` and `ALTER TABLE
   ASYNC … VALIDATE CONSTRAINT` returns is collected; before any statement
   other than `CREATE TABLE` / `CREATE INDEX ASYNC` (the first `ADD CONSTRAINT
   … FOREIGN KEY`, every `VALIDATE CONSTRAINT`) the collected jobs are waited
   for first, because a foreign key cannot reference a unique index that is
   still building. After the last statement the row gets `applied_steps_count
   = 1`; the remaining jobs are waited for (`CALL sys.wait_for_job`, falling
   back to `SELECT sys.wait_for_job`, then to polling `sys.jobs`), and only
   then `finished_at` is set. On a failed statement or a `failed` job the
   error goes into `logs`, `finished_at` stays NULL and the job exits `1`;
   earlier statements stay committed because DSQL DDL is not transactional.
   Every wait is bounded by `DSQL_JOB_WAIT_TIMEOUT_SECS` and honours SIGTERM
   (the blocked `wait_for_job` is cut by closing the connection); a wait cut
   short leaves the row resumable, listing the pending job ids in the log.
6. Drain `sys.jobs` once more, assert `pg_index` has no invalid index and
   `pg_constraint` has no `NOT VALID` foreign key, then set `finished_at` on
   the rows from step 3 that were only awaiting their jobs.
7. Grant the runtime role idempotently when `DSQL_RUNTIME_ROLE_ARN` is set:
   `CREATE ROLE <role> WITH LOGIN` if missing, `AWS IAM GRANT <role> TO
   '<DSQL_RUNTIME_ROLE_ARN>'` if not yet in `sys.iam_pg_role_mappings`, then
   `GRANT USAGE ON SCHEMA public`, `GRANT SELECT, INSERT, UPDATE, DELETE ON ALL
   TABLES`, `GRANT USAGE, SELECT ON ALL SEQUENCES` and the matching `ALTER
   DEFAULT PRIVILEGES`. Without the ARN the step is skipped with a warning.
8. Run `PRISMA_SCHEMA_DISABLE_ADVISORY_LOCK=1 prisma migrate status --config
   prisma.dsql.config.ts` (DATABASE_URL only in that child's environment) and
   fail unless it reports the history as up to date.
9. Release the lease.

## Environment

Required (same contract as `packages/aws-data/src/dsql.rs`):

- `DSQL_CLUSTER_ENDPOINT`: the bare cluster endpoint `<id>.dsql.<region>.on.aws`

Optional:

- `DSQL_RUNTIME_ROLE_ARN`: IAM role ARN of the api/file-tracker Lambdas; it is
  bound to the database role with `AWS IAM GRANT`. Unset on a development
  cluster without a runtime role: the grant step is skipped with a warning
  (validated as an ARN whenever set)
- `DSQL_REGION`: must equal the region in the endpoint (derived otherwise)
- `DSQL_RUNTIME_DB_ROLE`: database role to create and grant (default `flow_like_api`)
- `DSQL_MIGRATIONS_DIR`: migrations directory, relative to the job (default
  `prisma/migrations-dsql`, which the image copies from
  `packages/api/prisma/migrations-dsql`)
- `DSQL_SCHEMA_DIR`: schema directory for the final `prisma migrate status`
  check (default `prisma/schema`, the postgresql mirror the image builds). Set
  it together with `DSQL_MIGRATIONS_DIR` to run the job from a checkout.
- `DSQL_JOB_WAIT_TIMEOUT_SECS`: budget for one wait on async jobs - per
  migration and per cluster-wide drain - as an integer between 60 and 86400
  (default `7200`). The jobs keep running past it; the next run resumes.
- AWS credentials/region via the default chain (task role on ECS)

Forbidden (the job refuses to start when any is present, even empty):
`DATABASE_URL`, `PGPASSWORD`, `PGPASSFILE`, `PGSERVICE`, `PGSERVICEFILE`,
`PGHOST`, `PGHOSTADDR`, `PGPORT`, `PGUSER`, `PGDATABASE`, `PGSSLMODE`,
`PGSSLROOTCERT`, `PGSSLCERT`, `PGSSLKEY`, `PGOPTIONS`.

IAM for the task role: `dsql:DbConnectAdmin` on the cluster ARN. The runtime
role needs only `dsql:DbConnect`.

Exit codes: `2` environment refused, `3` lease held by another run, `1` any
failure (token, statement, async job, invalid index, grant, status check,
wait budget exhausted), `0` success or already up to date.

## Recovery

A row with `applied_steps_count = 1`, no `logs` and no `finished_at` needs no
action: the run ended while waiting for async jobs and the next run finishes
it once `sys.jobs` is drained and the catalog checks pass.

A failed statement or a failed job leaves a `_prisma_migrations` row with
`logs` set and no `finished_at`, and the job refuses to run again until it is
resolved. Read `logs`, repair the schema by hand (for a failed unique index:
drop it, remove the duplicates, recreate it), then either `prisma migrate
resolve --rolled-back <name>` (the job re-applies the whole file - statements
already applied must be removed or made idempotent first) or `--applied` (if
you finished it by hand). Both need `PRISMA_SCHEMA_DISABLE_ADVISORY_LOCK=1` and
a `DATABASE_URL` composed like the job does (`sslmode=require&sslaccept=strict`,
admin token as password).

## Build and local checks

Build from the `flow-like` root; the Dockerfile copies this directory,
`packages/api/prisma/schema`, `packages/api/scripts/make-prisma-mirror.sh` and
`packages/api/prisma/migrations-dsql`:

```sh
docker build -f apps/backend/aws/migration/Dockerfile -t flow-like-aws-migration .
```

Local: `bun install`, `bun test`, `bunx tsc --noEmit`, `bunx biome check .`.
To run against a cluster from a workstation with admin credentials (drop
`DSQL_RUNTIME_ROLE_ARN` on a cluster that has no runtime role yet):

```sh
DSQL_CLUSTER_ENDPOINT=<id>.dsql.<region>.on.aws \
DSQL_RUNTIME_ROLE_ARN=arn:aws:iam::<account>:role/<runtime-role> \
DSQL_MIGRATIONS_DIR=../../../../packages/api/prisma/migrations-dsql \
bun run migrate.ts
```

New migrations are generated in `packages/api` with `bun run db:dsql:diff -- <name>`
(see `packages/api/scripts/dsql-migration.ts`).
