---
title: GCP deployment
description: Configure Cloud Run, Cloud SQL IAM authentication, storage isolation, secrets, and schema deployment.
sidebar:
  order: 10
---

The GCP API runs on Cloud Run and authenticates to Cloud SQL, Cloud Storage,
Firestore, Pub/Sub, and Secret Manager through its runtime service account.
Provision those resources, database grants, and secrets before routing traffic
to a revision. Deploy [execution and queue workers](/self-hosting/gcp/workers/)
and [scheduled jobs](/self-hosting/gcp/automation/) with separate identities.

## Build and runtime configuration

Build the shared API image from the repository root without cloud credentials
or installation configuration:

```sh
docker buildx build --platform linux/amd64 \
  --load --tag flow-like-gcp-api:local \
  -f apps/backend/gcp/api/Dockerfile .
```

Supply a complete configuration with `provider: "gcp"`, the installation's
OIDC/OAuth settings, and a `mail.smtp` block. Select exactly one nonempty source:

| Variable | Source |
| --- | --- |
| `FLOW_LIKE_CONFIG_JSON` | Complete JSON, optionally injected from a Cloud Run secret |
| `FLOW_LIKE_CONFIG_FILE` | Readable JSON file mounted read-only |
| `FLOW_LIKE_CONFIG_SECRET_REF` | Secret Manager key or qualified reference such as `secret://gcp-secret-manager/HUB_CONFIG` |

The document replaces the complete public fallback and is read once at
startup. Conflicting or invalid sources stop startup. Grant access to a
referenced configuration secret individually, and restart or deploy a new
revision after changes. OAuth client secrets must be separate secret-store
entries referenced by `client_secret_env`; literal client secrets are rejected.
The public embedded fallback does not configure a GCP installation.

See [runtime API configuration](/self-hosting/containers/#runtime-api-configuration)
for source handling and optional custom compiled defaults. A default compiled
through a BuildKit secret is still embedded in the binary and must contain no
credentials.

## Deploy the audit worker separately

The API records pending audit events. A separate Cloud Run Job seals and signs them,
writes daily signed checkpoints and archives, and prunes archived database rows. The API image
does not run that job. Deploy it before directing traffic to the new API version.

Use `apps/backend/gcp/audit-worker/deploy.py` from the repository root. It prints
the proposed commands unless `--apply` is supplied. It creates separate worker and
scheduler service accounts, a bucket with locked retention, and a minute-by-minute
schedule. The worker has object creation/read access and permission to sign with
one Cloud KMS key. The scheduler can only invoke the job.

Prepare these resources first:

- A versioned Cloud KMS `EC_SIGN_P256_SHA256` signing key. Keep key administration
  outside the API identity. Enable Cloud Run, Cloud Scheduler, Secret Manager,
  Cloud KMS, and Policy Troubleshooter APIs in the relevant projects.
- A Cloud SQL PostgreSQL instance with a private endpoint,
  `cloudsql.iam_authentication=on`, and TLS mode `ENCRYPTED_ONLY`. Register the
  worker service account as a `CLOUD_IAM_SERVICE_ACCOUNT` database user and apply
  the [database grants below](#database-bootstrap-and-token-rotation). The default
  worker login is `flow-like-audit-worker@<project>.iam`; `--name` changes its
  service account and login together. It must differ from the API login. The helper
  verifies the instance settings, private endpoint, and existing IAM user mapping,
  then grants `roles/cloudsql.instanceUser` in the database project. This project
  grant covers every Cloud SQL instance there; database user mappings and SQL
  grants must keep the worker limited to its audit database.
- Secret Manager secrets containing the instance server CA PEM, the shared base64
  `AUDIT_ENTRY_KEY`, the shared `SINK_TOKEN_ENCRYPTION_KEY`, and the worker's audit
  configuration JSON. A minimal configuration is
  `{"audit":{"enabled":true,"require_signing":true}}`. Supply the exact entry and
  sink secret IDs that the API already resolves through its Secret Manager project
  and `SECRET_PREFIX`. The helper grants both identities access to these same
  secrets; it does not change the API's secret lookup settings.
- A private network/subnet that reaches PostgreSQL and an immutable digest of
  `ghcr.io/rheosoph/flow-like-gcp-audit-worker`. Mirror the image to an Artifact
  Registry repository accessible to Cloud Run before deployment and set
  `AUDIT_WORKER_IMAGE` to its complete `@sha256:` reference. Build custom images
  with `docker build -f apps/backend/gcp/audit-worker/Dockerfile .` from the
  `flow-like` repository root. This recipe includes the IAM launcher used by the
  deployment helper and uses the repository's `.dockerignore`.

```sh
python3 apps/backend/gcp/audit-worker/deploy.py \
  --project audit-project --region europe-west1 \
  --bucket organization-audit-evidence \
  --api-service-account api@audit-project.iam.gserviceaccount.com \
  --image "$AUDIT_WORKER_IMAGE" \
  --key-version projects/audit-project/locations/europe-west1/keyRings/audit/cryptoKeys/timeline/cryptoKeyVersions/1 \
  --database-instance audit-postgres --database-host 10.2.3.4 \
  --database-name flow_like --database-ca-secret cloud-sql-server-ca \
  --entry-key-secret "$AUDIT_ENTRY_KEY_SECRET" \
  --encryption-secret "$SINK_TOKEN_ENCRYPTION_KEY_SECRET" \
  --config-secret audit-config \
  --network audit-network --subnet audit-database
```

Set `AUDIT_ENTRY_KEY_SECRET` and `SINK_TOKEN_ENCRYPTION_KEY_SECRET` to those existing
secret IDs before running the example. The CA secret is required and becomes
`GCP_POSTGRES_SERVER_CA`. At each `--once` invocation, the launcher checks that
`GCP_POSTGRES_USER` matches the attached service account and requests a metadata
token with the `sqlservice.login` scope. It builds the child's temporary
`DATABASE_URL`, verifies the server CA for private IPs or the CA and hostname for
DNS endpoints, and removes the temporary CA file on exit. It stops the child
before the token expires. Static database passwords and credential overrides are
rejected; the old `--database-secret` argument is no longer supported.

The private-IP example requires the per-instance `GOOGLE_MANAGED_INTERNAL_CA` CA
mode. For `GOOGLE_MANAGED_CAS_CA` or `CUSTOMER_MANAGED_CAS_CA`, use the instance
certificate DNS name so the launcher applies `verify-full`; verifying a shared
CA alone does not establish which instance answered. The helper reads
`settings.ipConfiguration.serverCaMode` and accepts an omitted mode as the API's
legacy per-instance default. For DNS connections it checks the `dnsNames` entry
with `dnsScope=INSTANCE` and `connectionType=PRIVATE_SERVICES_ACCESS`, normalizing
a trailing dot before passing the hostname to the launcher. Create its private
DNS record in the worker's VPC. This helper supports private services access;
Private Service Connect requires separate endpoint setup. See Google's
[server identity verification guide](https://docs.cloud.google.com/sql/docs/postgres/configure-ssl-instance#server-identity-verification)
and [Cloud SQL instance API](https://docs.cloud.google.com/sql/docs/postgres/admin-api/rest/v1/instances).

To avoid fetching the KMS public key at every startup, also supply
`--audit-kid <current-key-id> --verifying-keys-secret <public-keys-secret-id>`.
The secret contains the shared `AUDIT_VERIFYING_KEYS` JSON map of key IDs to public
PEM keys, including the selected KMS version. Both arguments are required together;
retain older verifying keys while their evidence must remain verifiable. During
entry-key rotation, use `--previous-entry-key-secret` for the API's same
`AUDIT_ENTRY_KEY_PREVIOUS` secret and coordinate API revision replacement with the
worker's latest secret versions.

Review the output, then repeat with `--apply`. That operation locks retention;
the bucket's retention period cannot subsequently be shortened. The default of
1461 days covers the default archive horizon. Increase `--retention-days` when
raising `archive_years_after_year_end`. Existing buckets must already have at
least the requested retention, uniform bucket-level access, and public access
prevention. The script checks the effective lock before deploying the job.

The script requires Policy Troubleshooter to establish that the API cannot sign,
access the audit bucket or worker configuration secret, impersonate the
worker, or change its job/IAM policies. Inherited access or an inconclusive check
stops deployment. The deployment identity needs visibility into ancestor policies;
remove broad API grants rather than bypassing the checks. The entry and sink
encryption secrets are required and shared with the API. The API receives no
worker database token, signing grant, or audit bucket access.
The script also rejects inherited worker permissions to delete evidence, change
retention, administer signing keys/secrets, or modify its own job.

Set `AUDIT_WORKER=off` on the API and remove its `AUDIT_KMS_*`, audit bucket, and
signing-key settings. API startup rejects worker settings. Give it public
`AUDIT_VERIFYING_KEYS` for verification. After applying the deployment, verify a
signed checkpoint from `checkpoints/YYYY/MM/DD.json` and copy checkpoints to
storage controlled independently of this project. Use the worker binary's
`--verify-checkpoint FILE` mode with public verifying keys and a database account
that can only read evidence. Alert on failed jobs and missing or stale checkpoints.

## Required API environment

```text
GCP_PROJECT_ID=<app-project-id>
GCP_CONTENT_BUCKET=<content-bucket>
GCP_META_BUCKET=<metadata-bucket>
GCP_CDN_BUCKET=<cdn-bucket>
GCP_LOG_BUCKET=<logs-bucket>
SECRET_PREFIX=/flow-like/gcp-<environment>
MAIL_PROVIDER=smtp
CORS_ALLOWED_ORIGINS=https://<frontend-domain>

GCP_POSTGRES_AUTH_MODE=iam
GCP_POSTGRES_HOST=<private-IP-or-cloudsql-hostname>
GCP_POSTGRES_DATABASE=flow_like
GCP_POSTGRES_USER=<service-account-local-part>@<project>.iam
GCP_POSTGRES_SERVER_CA=<instance-server-CA-PEM>
```

Set `GCP_POSTGRES_HOST` to an RFC1918 private IPv4 address or the instance's
actual `*.cloudsql.goog` hostname, without a scheme, port, or path. The
database client fixes the port at 5432. The database name and IAM user must be
at most 63 UTF-8 bytes; the user omits `.gserviceaccount.com` from the service
account email. The CA setting accepts PEM with real or escaped newlines and is
read at startup. Rotate the revision when the instance CA changes.

Metadata and content must use separate buckets. Both use the `apps/<app_id>/`
prefix space; a downscoped content token could otherwise also read board and
event definitions. Startup rejects a shared bucket. If generic aliases are
also set, each pair must agree:

| GCP setting | Generic alias |
| --- | --- |
| `GCP_META_BUCKET` | `META_BUCKET` |
| `GCP_CONTENT_BUCKET` | `CONTENT_BUCKET` |
| `GCP_LOG_BUCKET` | `LOG_BUCKET` |
| `GCP_CDN_BUCKET` | `CDN_BUCKET_NAME` |

`STORAGE_PROVIDER`, `RUNTIME_CREDENTIALS_PROVIDER`, `META_BUCKET_PROVIDER`,
`CONTENT_BUCKET_PROVIDER`, and `LOGS_BUCKET_PROVIDER` must select `gcp` when
set; `gcs` and `google` are accepted aliases. Other cloud feature sets are
compiled out of this image.

Cloud Run supplies `PORT` (default `8080` in the image). Optional telemetry
settings are `OTEL_EXPORTER_OTLP_ENDPOINT` for OTLP/gRPC and
`GCP_REQUIRE_OTEL=true` to make a missing endpoint fatal. Use `RUST_LOG` for
log filtering.

## Identity and access

Use a dedicated API service account. Scope its permissions to the application
resources:

| Role | Scope and use |
| --- | --- |
| `roles/cloudsql.client` | Cloud SQL connect permission |
| `roles/cloudsql.instanceUser` | Cloud SQL IAM database login |
| `roles/secretmanager.secretAccessor` | Each allowed secret individually |
| `roles/storage.objectUser` | Each metadata, content, and CDN bucket |
| `roles/storage.objectCreator` | Logs bucket for append-only writes |
| `roles/iam.serviceAccountTokenCreator` | On the API account itself for `signBlob`, used to mint V4 signed URLs |
| `roles/datastore.user` | Firestore database/project for cache and execution state |
| `roles/pubsub.publisher` | Execution and compilation topics |
| `roles/run.invoker` | HTTP executor service |
| `roles/logging.logWriter` | Application project for container logs |

Apply Cloud SQL roles through IAM bindings covering the target instance. IAM
database authentication also requires an in-database login and grants; roles
alone do not grant table access. The API needs no Cloud Scheduler permissions
or access to a separate Terraform state project.

Secret access must be per secret. The provider tries a prefix-qualified name
and then an unprefixed fallback, so project-wide access can cross environment
boundaries even when `SECRET_PREFIX` differs. Before first startup, provision
versions of all required secrets under that prefix:

| Secret | Minimum bytes |
| --- | --- |
| `BACKEND_KEY` | `64` |
| `BACKEND_PUB` | `64` |
| `BACKEND_KID` | `8` |
| `SINK_SECRET` | `32` |
| `SINK_TOKEN_ENCRYPTION_KEY` | `32` |
| `MAINTENANCE_TOKEN` | `32` |

The API disables environment overrides for its secret store. Setting an
environment variable with one of those names does not satisfy the startup
check. Grant separate access to configuration and OAuth secrets as needed.

For synchronous execution, pair the executor invoker grant with
`EXECUTOR_AUTH=gcp_id_token`, `EXECUTION_BACKEND=http`, and `EXECUTOR_URL` on
the API. Dispatch obtains a Google ID token whose audience is the executor
URL's origin. The execution JWT inside the request remains the application's
run credential. A metadata token failure fails dispatch before an anonymous
request can reach the executor.

### Rejected credential sources

The API fails startup when static credentials, endpoint overrides, or proxy
settings are present, including empty values:

```text
GOOGLE_APPLICATION_CREDENTIALS, GOOGLE_APPLICATION_CREDENTIALS_JSON,
GOOGLE_CREDENTIALS, GOOGLE_OAUTH_ACCESS_TOKEN, CLOUDSDK_AUTH_ACCESS_TOKEN,
GCE_METADATA_HOST, GCE_METADATA_IP, GCE_METADATA_ROOT, METADATA_SERVER_DETECTION,
HTTP_PROXY, HTTPS_PROXY, ALL_PROXY (and lowercase forms),
GOOGLE_SERVICE_ACCOUNT, GOOGLE_SERVICE_ACCOUNT_PATH,
GOOGLE_SERVICE_ACCOUNT_KEY, SERVICE_ACCOUNT,
GOOGLE_SKIP_SIGNATURE, GOOGLE_ALLOW_HTTP, GOOGLE_ALLOW_INVALID_CERTIFICATES,
GOOGLE_PROXY_URL, GOOGLE_PROXY_CA_CERTIFICATE, GOOGLE_PROXY_EXCLUDES,
STORAGE_EMULATOR_HOST, PUBSUB_EMULATOR_HOST, FIRESTORE_EMULATOR_HOST,
DATASTORE_EMULATOR_HOST, SECRET_MANAGER_EMULATOR_HOST,
CLOUDSDK_API_ENDPOINT_OVERRIDES_*, GOOGLE_*_CUSTOM_ENDPOINT
```

The database client additionally rejects `DATABASE_URL`, database passwords,
the libpq connection settings (`PGPASSWORD`, `PGPASSFILE`, `PGSERVICE`,
`PGSERVICEFILE`, `PGHOST`, `PGHOSTADDR`, `PGPORT`, `PGUSER`, `PGDATABASE`,
`PGSSLMODE`, `PGSSLROOTCERT`, `PGSSLCERT`, `PGSSLKEY`, `PGOPTIONS`, `PGAPPNAME`),
`INSTANCE_CONNECTION_NAME`, `CLOUD_SQL_CONNECTION_NAME`, `CLOUD_SQL_PROXY_PATH`,
and supported `CSQL_PROXY_*` selectors. These controls keep identity acquisition
on the instance metadata path and database connections on the configured direct
TLS path. The exact lists are in the
[API guard](https://github.com/Rheosoph/flow-like/blob/main/apps/backend/gcp/api/src/config.rs)
and [PostgreSQL client](https://github.com/Rheosoph/flow-like/blob/main/packages/gcp-data/src/postgres.rs).

## Database bootstrap and token rotation

Create Cloud SQL IAM service-account users for the API and migration job. As
the schema-owning migration identity, connect to `flow_like` and grant runtime
DML rights. Replace the quoted roles with the actual database users:

```sql
grant connect on database flow_like to "<api-account>@<project>.iam";
grant usage on schema public to "<api-account>@<project>.iam";
grant select, insert, update, delete on all tables in schema public
  to "<api-account>@<project>.iam";
grant usage, select, update on all sequences in schema public
  to "<api-account>@<project>.iam";

alter default privileges for role "<migration-account>@<project>.iam"
  in schema public grant select, insert, update, delete on tables
  to "<api-account>@<project>.iam";
alter default privileges for role "<migration-account>@<project>.iam"
  in schema public grant usage, select, update on sequences
  to "<api-account>@<project>.iam";
```

Run existing-table grants after the initial schema push. Default privileges
cover future objects created by the migration role. Keep DDL, role
administration, and database ownership off the API identity. The file-tracking
worker needs its own IAM user with update rights on `App` and `User`.

Those broad runtime grants need a final audit-specific restriction after every
schema migration. Register the separate worker service account as a
`CLOUD_IAM_SERVICE_ACCOUNT` Cloud SQL user. Its PostgreSQL username is the service
account email without `.gserviceaccount.com`. Do not create a built-in password
user: Cloud SQL's default built-in user creation grants `cloudsqlsuperuser`, which
is unsuitable for this worker. See [Cloud SQL's system roles](https://docs.cloud.google.com/sql/docs/postgres/users).
With the schema owner's `DATABASE_URL`
available to Bun, run:

```sh
AUDIT_DB_GRANTS_ONLY=true \
API_DATABASE_ROLE='<api-account>@<project>.iam' \
AUDIT_DATABASE_ROLE='flow-like-audit-worker@<project>.iam' \
bun apps/backend/shared/audit_database_roles.ts
```

This mode preserves the IAM login and does not set its password. It restricts API
audit access to reading evidence and inserting pending records; the worker owns
sealing, signing state, archives, and its lease. Run it before starting either
workload, with the `pg` dependency available to Bun. Worker and API roles must not
inherit an application owner or other role that restores those privileges.
Grant-only mode preserves the non-group `cloudsqliamserviceaccount` and
`cloudsqliamuser` login markers and checks them for inherited audit write or DDL
rights before committing the grants.

The API and file tracker request a token scoped to
`https://www.googleapis.com/auth/sqlservice.login` and use SQL connection
options with TLS `verify-full` against the supplied instance CA. Their pools
cannot replace the token for new connections. The API closes readiness five
to eight minutes before expiry, waits for readiness propagation, drains, and
exits. Use at least two instances when availability must survive token rotation
and keep revisions restartable. Per-process jitter staggers the exits.

## Schema deployment and recovery

Build the migration image from the repository root and deploy it as a manually
triggered Cloud Run Job under the separate migration account:

```sh
docker build -f apps/backend/gcp/migration/Dockerfile -t flow-like-gcp-migration .
gcloud run jobs execute <migration-job> --region <region> --wait
```

Configure the same `GCP_POSTGRES_*` settings as the API, with the migration
account's database user. The migration runner additionally accepts an instance
DNS name ending in `.sql.goog`; the Rust runtime client currently accepts
`.cloudsql.goog` or a private IPv4 address. `GCP_PROJECT_ID` and `GCP_REGION`
are optional and unused by this runner.

The image builds and validates a PostgreSQL mirror of the tracked CockroachDB
Prisma schema. The runner obtains a SQL-login token from metadata, runs the
guarded pre-push column conversions, and then runs `prisma db push`. Each child
receives its connection URL through `DATABASE_URL`. The runner does not log
or write that URL to disk, and forwards SIGTERM to the active child.

The migration runner has a transport boundary that differs from the runtime
client. With a private-IP host it uses TLS without server certificate
verification and logs a warning: Prisma cannot verify the CA without also
matching the hostname. With an instance DNS name present in the server
certificate, it enables `sslaccept=strict`, writes the supplied CA to a
restricted temporary file, and removes the file at exit. The pre-push client
uses equivalent node-postgres TLS settings. Use a certificate-matching instance
DNS name when migration server verification is required. The CA is validated
even for private-IP connections.

The job omits `--accept-data-loss`. If Prisma refuses a destructive diff,
inspect the warnings and current schema, including any pre-push conversions
already applied. Perform an intended destructive step separately from a
management host as the migration identity with a short-lived token, then rerun
the job, or deploy an additive schema first. The job returns the failing
pre-push exit code or, after that succeeds, Prisma's exit code. Inspect job logs
before directing an API revision that needs the schema to this database.

Prisma passes the datasource URL to its schema-engine child as an argument,
so its SQL-login token is visible to processes sharing the container during
the push. Run only the migration processes in that container. This is a
current limitation of the
[migration entrypoint](https://github.com/Rheosoph/flow-like/blob/main/apps/backend/gcp/migration/migrate.ts).
For runner checks from `apps/backend/gcp/migration`, use `bun install`,
`bun test tests/`, and `bun run typecheck`.

## SMTP and readiness checks

The GCP image requires `MAIL_PROVIDER=smtp`. In the complete runtime
configuration, `mail.smtp` names the environment variables containing the relay
host, port, username, and password. Supply those values through Secret Manager
references. Scope the relay credential to sending and rotate it independently.
The public fallback has no SMTP block: mail initialization can log
`SMTP settings required for SMTP provider` and continue without a mail client.
Verify an outbound email before routing traffic.

`/health/live` checks the process. `/health/ready`, `/health/startup`, and the
`/health` compatibility alias require an accepting token lifecycle and a
successful Cloud SQL ping. During token rotation, readiness closes before the
listener drains. SIGTERM closes readiness and drains immediately so Cloud Run's
termination grace can be used for active requests.

Cloud Run publishes one container port. `/metrics` is on the serving router;
exclude it from the public load balancer URL map and restrict service ingress
to the load balancer. Before a first revision receives traffic, verify secret
versions/access, database grants, the current CA, distinct buckets with agreeing
aliases, the Firestore database, Pub/Sub topics, and the complete SMTP-enabled
runtime configuration. Missing topics can fail dispatch even when startup
checks pass.
