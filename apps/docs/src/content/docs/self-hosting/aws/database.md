---
title: Database (Aurora DSQL)
description: Run the AWS backend on Amazon Aurora DSQL with IAM authentication, and ship schema changes with the DSQL migration pipeline.
sidebar:
  order: 30
---

The AWS backend (`apps/backend/aws`) stores platform data in
[Amazon Aurora DSQL](https://docs.aws.amazon.com/aurora-dsql/latest/userguide/what-is-aurora-dsql.html).
DSQL speaks the PostgreSQL wire protocol but differs from PostgreSQL in ways
that shape the setup: there is no password (every connection uses a short-lived
IAM token), one database (`postgres`), one DDL statement per transaction,
asynchronous index builds, and no enums or array columns. The Prisma schema in
`packages/api/prisma/schema/` stays the schema of record for every target;
DSQL gets its own apply pipeline.

Two binaries talk to the database at runtime, both Lambdas: the API
(`apps/backend/aws/api`) and the file tracker
(`apps/backend/aws/file-tracker`). A third component, the migration job
(`apps/backend/aws/migration`), is a one-off ECS Fargate task that applies the
committed migrations and grants the runtime role.

## Environment contract

The presence of `DSQL_CLUSTER_ENDPOINT` selects DSQL; without it the API and
file tracker keep reading `DATABASE_URL` from SSM under `SECRET_PREFIX` as
before. All three components share the same validation
(`packages/aws-data/src/dsql.rs`, mirrored in `apps/backend/aws/migration/migrate.ts`).

| Variable | API / file tracker | Migration job |
| --- | --- | --- |
| `DSQL_CLUSTER_ENDPOINT` | required, `<id>.dsql.<region>.on.aws` | required |
| `DSQL_REGION` | optional, must match the endpoint | optional, must match the endpoint |
| `DSQL_USER` | database role, default `admin`; production uses `flow_like_api` | always `admin` |
| `DSQL_TOKEN_DURATION_SECS` | token lifetime, default `3600` (1800–604800) | fixed `900`, minted per connection |
| `DSQL_MAX_CONNECTIONS` | pool size, default `4` | n/a |
| `DSQL_RUNTIME_ROLE_ARN` | n/a | optional, the Lambdas' IAM role ARN; unset skips the grant step with a warning |
| `DSQL_RUNTIME_DB_ROLE` | n/a | optional, default `flow_like_api` |
| `DSQL_MIGRATIONS_DIR` | n/a | optional, default `prisma/migrations-dsql` (what the image ships) |
| `DSQL_JOB_WAIT_TIMEOUT_SECS` | n/a | optional, default `7200` (60–86400), budget per wait on `sys.jobs` |

Forbidden alongside a DSQL endpoint, even when empty: `DATABASE_URL`,
`PGPASSWORD`, `PGPASSFILE`, `PGSERVICE`, `PGSERVICEFILE`, `PGHOST`,
`PGHOSTADDR`, `PGPORT`, `PGUSER`, `PGDATABASE`, `PGSSLMODE`, `PGSSLROOTCERT`,
`PGSSLCERT`, `PGSSLKEY`, `PGOPTIONS`. Every process refuses to start when one
is present, so a static credential can never replace the token.

Credentials come from the default AWS chain (the Lambda execution role, the
ECS task role). Tokens are checked only when a connection is opened; the Rust
pool mints one for `DSQL_TOKEN_DURATION_SECS` (one hour by default; the
1800-second floor keeps the half-life rotation ahead of the longest Lambda
invocation), swaps it at half-life, and retires connections after 25 minutes,
well inside DSQL's 60-minute connection limit. TLS is always `verify-full`
against the public CA chain; never put a TLS-terminating proxy or PgBouncer in
front of the cluster.

## IAM policies

Runtime role (API and file tracker Lambdas) - connect as a non-admin role only:

```json
{
  "Version": "2012-10-17",
  "Statement": [
    {
      "Effect": "Allow",
      "Action": "dsql:DbConnect",
      "Resource": "arn:aws:dsql:<region>:<account>:cluster/<cluster-id>"
    }
  ]
}
```

Migration task role - admin access, used only by the migration job:

```json
{
  "Version": "2012-10-17",
  "Statement": [
    {
      "Effect": "Allow",
      "Action": "dsql:DbConnectAdmin",
      "Resource": "arn:aws:dsql:<region>:<account>:cluster/<cluster-id>"
    }
  ]
}
```

Keep `dsql:DbConnectAdmin` off the runtime role. The `admin` database role
owns `public`; the Lambdas need only the grants below.

## One-time grant

The migration job performs this step itself, idempotently, on every run that
has `DSQL_RUNTIME_ROLE_ARN` set (the runtime role; `DSQL_RUNTIME_DB_ROLE` is
the database role). Without it - a development cluster that has no runtime
role yet - the job applies the schema and logs a warning instead. For
reference, or to run it by hand as `admin`:

```sql
CREATE ROLE flow_like_api WITH LOGIN;
AWS IAM GRANT flow_like_api TO 'arn:aws:iam::<account>:role/<runtime-role>';
GRANT USAGE ON SCHEMA public TO flow_like_api;
GRANT SELECT, INSERT, UPDATE, DELETE ON ALL TABLES IN SCHEMA public TO flow_like_api;
GRANT USAGE, SELECT ON ALL SEQUENCES IN SCHEMA public TO flow_like_api;
ALTER DEFAULT PRIVILEGES IN SCHEMA public GRANT SELECT, INSERT, UPDATE, DELETE ON TABLES TO flow_like_api;
ALTER DEFAULT PRIVILEGES IN SCHEMA public GRANT USAGE, SELECT ON SEQUENCES TO flow_like_api;
```

Check the mapping with `SELECT * FROM sys.iam_pg_role_mappings;`. The Lambdas
then run with `DSQL_USER=flow_like_api`.

## Applying migrations

Build the image from the repository root:

```sh
docker build -f apps/backend/aws/migration/Dockerfile -t flow-like-aws-migration .
```

Run it as a one-off ECS Fargate task under the migration task role. The
environment goes in as a container override (the container is called
`migration` in the task definition here):

```sh
aws ecs run-task \
  --cluster <ecs-cluster> \
  --launch-type FARGATE \
  --task-definition flow-like-aws-migration \
  --network-configuration 'awsvpcConfiguration={subnets=[<subnet-id>],securityGroups=[<security-group-id>],assignPublicIp=ENABLED}' \
  --overrides '{"containerOverrides":[{"name":"migration","environment":[{"name":"DSQL_CLUSTER_ENDPOINT","value":"<id>.dsql.<region>.on.aws"},{"name":"DSQL_RUNTIME_ROLE_ARN","value":"arn:aws:iam::<account>:role/<runtime-role>"}]}]}'
```

The same image runs from a workstation whose AWS credentials hold
`dsql:DbConnectAdmin` on the cluster. The migrations are baked into the image
under `prisma/migrations-dsql`, so `DSQL_MIGRATIONS_DIR` stays at its default:

```sh
docker run --rm \
  -e AWS_REGION=<region> \
  -e AWS_ACCESS_KEY_ID -e AWS_SECRET_ACCESS_KEY -e AWS_SESSION_TOKEN \
  -e DSQL_CLUSTER_ENDPOINT=<id>.dsql.<region>.on.aws \
  -e DSQL_RUNTIME_ROLE_ARN=arn:aws:iam::<account>:role/<runtime-role> \
  flow-like-aws-migration
```

Without the image, straight from the checkout, `DSQL_MIGRATIONS_DIR` points
the job at the committed files:

```sh
cd apps/backend/aws/migration && bun install
DSQL_CLUSTER_ENDPOINT=<id>.dsql.<region>.on.aws \
DSQL_RUNTIME_ROLE_ARN=arn:aws:iam::<account>:role/<runtime-role> \
DSQL_MIGRATIONS_DIR=../../../../packages/api/prisma/migrations-dsql \
bun run migrate.ts
```

Leave `DSQL_RUNTIME_ROLE_ARN` out on a development cluster that has no
runtime role yet; the job applies the schema and skips the grant with a
warning.

The job takes a 30-minute lease in `_flow_migration_lock` (a second concurrent
run exits `3`), first waits for any `sys.jobs` entry still running from an
earlier run, then applies every pending `migration.sql` statement by
statement. `CREATE TABLE` and `CREATE INDEX ASYNC` overlap with the index
builds already running; every other statement - the `ADD CONSTRAINT … FOREIGN
KEY`s, the `VALIDATE CONSTRAINT`s - first waits for the jobs the migration
has submitted so far, because a foreign key cannot reference a unique index
that is still building. A statement that hits an OCC conflict is retried, and
an "already exists" on the retry counts as applied. Once every statement is
committed the row gets `applied_steps_count = 1`; `finished_at` is set only
after the migration's async jobs have completed. Before verifying `pg_index`
and `pg_constraint` the job drains `sys.jobs` once more, then grants the
runtime role and finishes with `prisma migrate status` against
`_prisma_migrations`. A run that dies while waiting (SIGKILL, the
`DSQL_JOB_WAIT_TIMEOUT_SECS` budget) leaves a row with `applied_steps_count =
1` and no `finished_at`; the next run waits for the jobs and finishes it. Only
a row with `logs` set - a failed statement or a failed job - needs a human.
The job does not use `prisma migrate deploy`: Prisma sends a migration file as
one batch, which PostgreSQL runs in one implicit transaction and DSQL rejects.
Details and recovery steps are in `apps/backend/aws/migration/README.md`.

Run the job before deploying a Lambda revision that needs the new schema;
sessions opened before a schema change see one `OC001` conflict on their next
statement, which the API's transaction retry absorbs.

## Developer workflow: generating a migration

Every schema change lands in `packages/api/prisma/schema/` first (the
CockroachDB/PostgreSQL targets keep using `prisma db push`). For DSQL, generate
and commit a migration:

```sh
cd packages/api
bun install
bun run db:dsql:diff -- <name>
```

`scripts/dsql-migration.ts`:

1. derives the DSQL mirror with `scripts/make-prisma-mirror.sh --target dsql`,
   which fails on anything DSQL cannot create (enums, scalar lists, GIN
   indexes, native types other than `@db.Date`/`@db.Timestamp`);
2. runs `prisma migrate diff --script` from the base to the mirror;
3. rewrites the SQL with `dsql-lint --fix` (`CREATE INDEX` → `CREATE INDEX
   ASYNC`, foreign keys → `NOT VALID`); unfixable errors abort;
4. appends `ALTER TABLE ASYNC "t" VALIDATE CONSTRAINT "c";` for every
   `NOT VALID` foreign key;
5. writes `prisma/migrations-dsql/<timestamp>_<name>/migration.sql`,
   `migration_lock.toml` and `schema.snapshot.prisma`, and lints the result
   (must be clean).

The diff base is chosen automatically: `--from-empty` for the first migration,
otherwise the snapshot written by the previous run
(`prisma/migrations-dsql/schema.snapshot.prisma`). The snapshot is the
supported base; it is committed with every migration, and the sequence of
committed files is what a cluster receives. Prisma's `--from-migrations` with
a local PostgreSQL shadow database is not an option: the committed files
contain `CREATE INDEX ASYNC` and `ALTER TABLE ASYNC`, which plain PostgreSQL
cannot replay.

`--from-url` (experimental) diffs against the live cluster instead. It is not
a drift check to rely on: DSQL introspection renders every index as `USING
btree_index … INCLUDE (…)`, which Prisma cannot map back to the schema, so the
diff is full of spurious index drops and recreations that have to be pruned by
hand. The URL never goes on the command line (it carries the admin token);
export it as `DSQL_DIFF_URL` or pipe it on stdin, and the script redacts it
from every message:

```sh
TOKEN=$(aws dsql generate-db-connect-admin-auth-token --hostname <endpoint> --region <region> | jq -Rr @uri)
DSQL_DIFF_URL="postgresql://admin:${TOKEN}@<endpoint>:5432/postgres?sslmode=require&sslaccept=strict" \
  bun run db:dsql:diff -- <name> --from-url
```

`dsql-lint` is pinned to one version in one place, `DSQL_LINT_VERSION` in
`scripts/dsql-migration.ts` (currently `0.2.17`): the generator refuses any
other version, and CI (`.github/workflows/clippy.yml`) reads the same constant
to lint every committed `migration.sql` with the matching npm build. Install
it with `cargo install dsql-lint --version 0.2.17`, or set
`DSQL_LINT="npx --yes --package=@aws/dsql-lint@0.2.17 dsql-lint"`.

DSQL cannot change a column's type, add `NOT NULL` to an existing column, add a
`NOT NULL`/`DEFAULT` column inline, or add a primary key later. Plan schema
evolution accordingly: new required columns start nullable, or the table is
recreated.
