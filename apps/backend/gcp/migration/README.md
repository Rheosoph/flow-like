# Flow-Like GCP schema migration job

A Cloud Run job that applies the Prisma schema to Cloud SQL for PostgreSQL as
the migration service account. It is run by hand from the runbook
(`gcloud run jobs execute <prefix>-migration --wait`) after every schema change
and never on a schedule. Like every GCP image it is keyless: the token that
authenticates the database session is minted by the instance metadata server
for the job's service account, and startup fails if a password, a connection
string, a key file or a proxy variable is present rather than ignoring it.

What the job does, in order:

1. Validates the environment (below) and refuses anything on the forbidden list.
2. Asks Application Default Credentials — on Cloud Run, the metadata server —
   for an access token scoped to `https://www.googleapis.com/auth/sqlservice.login`,
   the narrowest scope Cloud SQL accepts for IAM database login.
3. Composes `postgresql://<user>:<token>@<host>:5432/<db>?...` in memory and
   passes it as `DATABASE_URL` to exactly two child processes in turn:
   `bun run prisma/pre-push.ts` (`packages/api/prisma/pre-push.ts`: guarded,
   idempotent column type changes `db push` emits without the `USING` clause
   they need on an existing database) and then
   `prisma db push --schema=prisma/schema`. The URL is never logged and never
   written to disk. The runner connects with node-postgres, which gets the
   same TLS posture as Prisma in its own spelling (`uselibpqcompat=true`,
   `sslmode=verify-full&sslrootcert=<pinned CA>` or a bare `sslmode=require`).
4. Exits with the runner's exit code when it fails (Prisma is then not
   started), otherwise with Prisma's.

`--accept-data-loss` is deliberately absent. When the diff would drop a table
or column that still holds data, Prisma applies nothing (the additive parts
included), prints the drops it refused, and exits 1. Read the job log, decide,
and if the drop is intended perform it by hand from the management-subnet host
as the migration identity per `docs/gcp/deployment-runbook.md` — or ship an
additive schema first. Never bake the flag into this image: the job runs as the
schema-owning role.

## Environment contract

Terraform renders the same `database_env` for this job as for the API and the
queue workers, plus the migration identity's own user. The validation here is
the TypeScript twin of `flow_like_gcp_data::postgres`, so both accept and reject
the same values.

```
GCP_POSTGRES_AUTH_MODE=iam                       # exactly "iam"
GCP_POSTGRES_HOST=<PSA private IPv4 | *.sql.goog | *.cloudsql.goog>
GCP_POSTGRES_DATABASE=flowlike                   # <= 63 bytes, no control chars
GCP_POSTGRES_USER=<sa local part>@<project>.iam  # .gserviceaccount.com stripped
GCP_POSTGRES_SERVER_CA=<instance server CA, PEM> # real or "\n"-escaped newlines
```

Optional and ignored: `GCP_PROJECT_ID`, `GCP_REGION` (present in the job env
for parity with the other workloads; nothing here reads them). Port is fixed at
5432.

Forbidden — presence, even empty, is fatal:

```
DATABASE_URL, GCP_POSTGRES_PASSWORD, GCP_POSTGRES_CONNECTION_STRING,
POSTGRES_PASSWORD, PGPASSWORD, PGPASSFILE, PGSERVICE, PGSERVICEFILE, PGHOST,
PGHOSTADDR, PGPORT, PGUSER, PGDATABASE, PGSSLMODE, PGSSLROOTCERT, PGSSLCERT,
PGSSLKEY, PGOPTIONS, PGAPPNAME, INSTANCE_CONNECTION_NAME,
CLOUD_SQL_CONNECTION_NAME, CLOUD_SQL_PROXY_PATH, CSQL_PROXY_*,
CLOUDSDK_API_ENDPOINT_OVERRIDES_SQLADMIN

GOOGLE_APPLICATION_CREDENTIALS, GOOGLE_APPLICATION_CREDENTIALS_JSON,
GOOGLE_CREDENTIALS, GOOGLE_OAUTH_ACCESS_TOKEN, CLOUDSDK_AUTH_ACCESS_TOKEN,
GCE_METADATA_HOST, GCE_METADATA_IP, GCE_METADATA_ROOT,
METADATA_SERVER_DETECTION, HTTP_PROXY, HTTPS_PROXY, ALL_PROXY (and lowercase)
```

The first block is everything libpq would read from the environment plus every
way to reintroduce a static password or the Cloud SQL Auth Proxy; the second is
everything that would redirect, replace or intercept the metadata credential.
Same lists, same reasoning as the API image.

## Transport

Cloud SQL refuses cleartext (`ssl_mode = ENCRYPTED_ONLY`) and the job always
sends `sslmode=require`, so the session is TLS in every case. Whether the
*server certificate* is verified depends on the host shape, because of a
Prisma limitation worth stating plainly:

- Prisma's schema engine understands `sslmode=disable|prefer|require`,
  `sslcert=<CA path>` and `sslaccept=strict|accept_invalid_certs`. libpq's
  `verify-ca`, `verify-full` and `sslrootcert` are **silently ignored** — an
  unknown `sslmode` falls back to `prefer` — so they are never used here; a
  URL that carried them would look verified and not be.
- `sslaccept=strict` verifies the chain **and** the hostname together; there is
  no chain-only mode. Cloud SQL's per-instance CA issues a serving certificate
  whose identity is the instance, not its private address, so against an IP the
  strict check can never pass.

Hence: with `GCP_POSTGRES_HOST` = private IP (what Terraform passes today) the
job sends `sslmode=require&sslaccept=accept_invalid_certs`, logs a warning that
the certificate is not verified, and relies on the VPC-only path, the
login-scoped token and its one-hour lifetime. With `GCP_POSTGRES_HOST` = the
instance DNS name (shared-CA server mode puts it in the SAN) the job writes
`GCP_POSTGRES_SERVER_CA` to a `0600` file in a `0700` temp directory, sends
`sslmode=require&sslaccept=strict&sslcert=<that file>`, and removes the file
on exit. `GCP_POSTGRES_SERVER_CA` is validated in both cases so the environment
is already right on the day the host becomes a name.

Accepted residual: the Prisma CLI passes the datasource URL to its schema
engine as a command-line argument, so the token is visible in that child's
`/proc/<pid>/cmdline` for the duration of the push. Only this job's own
processes share the container, the token carries the SQL login scope alone, and
it expires within the hour.

## Image

`Dockerfile` (build context: `flow-like`) installs `prisma` and
`google-auth-library` from the lockfile, asserts the installed Prisma satisfies
the range `packages/api/package.json` pins, copies `packages/api/prisma/schema`
and `packages/api/prisma/{pre-push.ts,pre-push/}`, rewrites the datasource
provider from `cockroachdb` to `postgresql` with the same
`packages/api/scripts/make-postgres-prisma-mirror.sh` (a shim over
`make-prisma-mirror.sh --target postgresql`) every other Postgres installation
uses, validates the result, and proves the native schema engine loads. Runtime is `oven/bun:1.3.8-debian` as uid 10001 with `libssl3` (the
engine links OpenSSL 3), `CHECKPOINT_DISABLE=1` (no update-check call-outs from
a job with no internet route) and `STOPSIGNAL SIGTERM`, which the entrypoint
forwards to the running push.

## Local checks

```sh
bun install
bun test tests/          # env validation, URL composition, TLS posture
bun run typecheck
```
