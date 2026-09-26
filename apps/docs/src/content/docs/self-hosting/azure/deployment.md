---
title: Azure deployment
description: Configure the Azure API, PostgreSQL identities, schema job, email, and telemetry collector.
sidebar:
  order: 10
---

The Azure backend runs in Container Apps and uses user-assigned managed
identities for PostgreSQL and Azure services. Prepare the database roles,
runtime configuration, and secret access before directing traffic to a revision.
Use separate identities for the API, migrations, and each
[worker workload](/self-hosting/azure/workers/).

## Build and configure the API

Build from the repository root. The shared image requires no installation
configuration or cloud credential at build time:

```sh
docker buildx build \
  --platform linux/amd64 \
  --load --tag flow-like-azure-api:local \
  -f apps/backend/azure/api/Dockerfile .
```

Supply a complete installation configuration with `provider: "azure"`, your
Entra/OAuth settings, and the organization's feature and tier configuration.
Select exactly one nonempty runtime source:

| Variable | Source |
| --- | --- |
| `FLOW_LIKE_CONFIG_JSON` | Complete JSON, optionally injected from a Container Apps secret |
| `FLOW_LIKE_CONFIG_FILE` | Readable JSON file mounted read-only |
| `FLOW_LIKE_CONFIG_SECRET_REF` | Key Vault key or qualified reference such as `secret://azure-key-vault/HUB-CONFIG` |

The selected document replaces the embedded public fallback and is read once
at startup. Invalid or conflicting sources stop startup; a change requires a
restart or new revision. The managed identity must already be able to read a
referenced secret. OAuth client secrets belong in separate secret-store
entries referenced by `client_secret_env`; literal client secrets are rejected.
The public fallback does not configure an Azure installation.

The [Entra External ID fragment](https://github.com/Rheosoph/flow-like/blob/main/apps/backend/azure/api/entra-external-id.fragment.example.json)
shows Azure-specific settings. Replace its example tenant, application, and
URLs, check discovery, issuer, audience, and JWKS, then merge it into the complete
configuration. The fragment alone fails schema validation. For shared source
handling and optional custom compiled defaults, see
[runtime API configuration](/self-hosting/containers/#runtime-api-configuration).
A compiled default remains recoverable from the binary even when supplied as
a BuildKit secret; it must contain no credentials.

## Deploy the audit worker separately

The API records pending audit events. A separate scheduled Container Apps Job
seals and signs them, writes daily signed checkpoints and archives, and prunes archived database
rows. The API image does not run that job. Deploy it before directing traffic to
the new API version.

`apps/backend/azure/audit-worker/deploy.py` prints its proposed operations by
default. With `--apply`, it provisions a separate user-assigned identity, an audit
storage account/container, locked retention, narrowly scoped custom roles, and a
job that runs once a minute. The job exposes no HTTP listener. Its identity can
read/write audit blobs and read/sign with one Key Vault key; it cannot delete
blobs, change retention, or administer keys.

Build the Azure worker image from the repository root:

```sh
docker build --platform linux/amd64 \
  -f apps/backend/azure/audit-worker/Dockerfile \
  -t flow-like-azure-audit-worker .
```

The worker compiles the `audit` section of the same document as the Azure API
image: the tracked public default, or the API build's `flow_like_config` BuildKit
secret with its `FLOW_LIKE_CONFIG_SHA256` build argument. It reads no
configuration at runtime, so the API's `FLOW_LIKE_CONFIG_*` sources do not reach
it. Set audit policy in that build input; see
[Audit policy](/self-hosting/audit-trail/#audit-policy).

Publish it to your registry and select the resulting digest. This image adds an
Entra PostgreSQL launcher to the shared audit binary. It obtains a short-lived
token from the Container Apps identity endpoint, places it in the child process's
connection URL, and runs `--once`. Tokens stay out of files, logs and process
arguments. The launcher stops the process before token expiry, with a maximum
runtime of 55 minutes. Each scheduled execution obtains a token again.

Prepare a versioned P-256 Key Vault key, a Container Apps environment with private
database connectivity, and versioned Key Vault secrets for the worker database
URL, shared base64 entry key and shared export-token encryption key. Both vaults
must use Azure RBAC. Keep the signing key and worker database secret outside the
API's access.

Create the worker's user-assigned identity using the resource group and `--name`
you will pass to the planner. The default name is `flow-like-audit-worker`.
Bind its principal UUID to a separate PostgreSQL Entra role and apply the
[database grants below](#bootstrap-database-roles) before starting the job. The
planner reuses that identity. The database secret must contain a password-free
URL whose username matches `--database-user`, with certificate verification:

```text
postgresql://<audit worker database role>@<server>.postgres.database.azure.com:5432/<database>?sslmode=verify-full
```

Export the selected signing key version's public key as SPKI PEM and prepare a
public JSON file mapping its key ID to the PEM. Keep historical verifier keys in
the map when rotating. The planner passes this map as `AUDIT_VERIFYING_KEYS` and
its current ID as `AUDIT_KID`, avoiding a Key Vault public-key read at every job
startup. During apply, it reads the selected key once and checks its curve,
signing capability, enabled state, and public key before locking storage.

Set the variables in this example to those resource IDs, versioned secret/key
URIs, public JSON file, database role, and image digest:

```sh
python3 apps/backend/azure/audit-worker/deploy.py \
  --subscription "$AZURE_SUBSCRIPTION_ID" --resource-group audit --location westeurope \
  --environment-id "$AZURE_ENVIRONMENT_ID" \
  --api-identity-id "$API_IDENTITY_ID" \
  --storage-account "$AUDIT_STORAGE_ACCOUNT" --image "$AUDIT_WORKER_IMAGE" \
  --key-vault-id "$AUDIT_KEY_VAULT_ID" --key-id "$AUDIT_KEY_VERSION_URI" \
  --kid "$AUDIT_KID" --verifying-keys-file "$AUDIT_VERIFYING_KEYS_FILE" \
  --secrets-vault-id "$AUDIT_SECRETS_VAULT_ID" \
  --database-user "$AUDIT_DATABASE_USER" \
  --database-secret-uri "$AUDIT_DATABASE_SECRET_URI" \
  --entry-key-secret-uri "$AUDIT_ENTRY_KEY_SECRET_URI" \
  --encryption-secret-uri "$SINK_TOKEN_ENCRYPTION_KEY_SECRET_URI"
```

For a private Azure Container Registry, add
`--registry-server <name>.azurecr.io` and grant the worker identity pull access
to the existing registry beforehand. The planner configures identity-based
pulls but does not create a registry or grant registry permissions. Without
this option, the image must be anonymously pullable.

Review the output and repeat with `--apply`. This locks the retention policy,
which cannot subsequently be shortened. The default of 1461 days covers the
default archive horizon; raise `--retention-days` when increasing
`archive_years_after_year_end`. Existing storage must already meet that duration.
The script verifies the lock and rejects protected append writes, shared account
keys, and public blob access before deploying the job. Its schedule retries on
the next minute rather than adding platform retries to a failed execution.

The deployment identity must be able to read the signing key's public part and
enumerate the API identity's security groups and inherited RBAC assignments,
including management-group policies. The script rejects direct, inherited, or
conditional API roles that can use audit storage, sign, read the worker
database secret, or change worker resources and permissions.
Remove broad API grants when this check fails. Key Vault role propagation can
take time; rerun the deployment after a newly assigned role becomes effective.
The worker's inherited roles are checked too: evidence deletion, retention
changes, key/secret administration, and modifying its own job are rejected.

Configure the API's Key Vault secret store to resolve the same `AUDIT_ENTRY_KEY`
and existing `SINK_TOKEN_ENCRYPTION_KEY`, plus the same public `AUDIT_KID` and
`AUDIT_VERIFYING_KEYS`. The Azure API disables process-environment overrides for
these secrets. The two shared secret values must match byte for byte. The
planner does not read their contents; the launcher checks the database URL when
the job starts. Secret references are version-pinned, so rotation requires a
coordinated API reload and worker redeployment. For entry-key rotation, pass
`--previous-entry-key-secret-uri` for the previous key and publish that same
previous value as the API's `AUDIT_ENTRY_KEY_PREVIOUS` while old records drain.

Set `AUDIT_WORKER=off` on the API and remove signing-key, audit bucket, and
`AUDIT_KMS_*` settings. API startup rejects worker settings. Verify a signed
checkpoint from `checkpoints/YYYY/MM/DD.json` after deployment and copy
checkpoints to an independently controlled destination. The shared worker
binary's `--verify-checkpoint FILE` mode uses public verifying keys and a
database account that can only read evidence; invoke that binary directly for
verification because the Azure launcher accepts only `--once`. Alert on failed
jobs and missing or stale checkpoints. Lifecycle tiering and Monitor alert
resources remain separate deployment configuration.

## PostgreSQL identity and lifecycle

Configure the API with:

```text
AZURE_CLIENT_ID=<API managed identity client UUID>
AZURE_POSTGRES_AUTH_MODE=managed_identity
AZURE_POSTGRES_HOST=<server>.postgres.database.azure.com
AZURE_POSTGRES_DATABASE=flow_like
AZURE_POSTGRES_USER=<API managed identity name>
```

The database user is the identity's display/resource name, such as
`flowlike-dev-api-identity`. It is distinct from both the client UUID and the
principal/object UUID. Container Apps supplies `IDENTITY_ENDPOINT` and
`IDENTITY_HEADER`; the endpoint must be HTTP on loopback. The process rejects
alternate identity endpoints, authority overrides, and proxy variables to keep
that header on the local identity path.

The API obtains an Entra token for
`https://ossrdbms-aad.database.windows.net/.default` and uses it in SQL connection
options with TLS `verify-full`. The pool cannot replace the token for newly
opened connections, so readiness closes five to eight minutes before expiry
and the process exits after draining. Keep the app restartable and use at least
two replicas when availability must survive token rotation. Per-process jitter
staggers rotation across replicas.

### Bootstrap database roles

Azure RBAC grants do not create PostgreSQL roles. As the configured Flexible
Server Entra administrator, connect to `postgres` and create the API principal
by object ID. This avoids ambiguous display names:

```sql
select * from pg_catalog.pgaadauth_create_principal_with_oid(
  '<API managed identity name>',
  '<API managed identity principal UUID>',
  'service',
  false,
  false
);
```

Create the separate migration principal the same way, using its own name and
principal UUID, and grant it the schema ownership/DDL rights required to apply
the schema. Then connect to `flow_like` and grant runtime DML rights to the API.
Replace both quoted role names below with the deployment's actual names:

```sql
grant connect on database flow_like to "<API managed identity name>";
grant usage on schema public to "<API managed identity name>";
grant select, insert, update, delete on all tables in schema public
  to "<API managed identity name>";
grant usage, select, update on all sequences in schema public
  to "<API managed identity name>";

alter default privileges for role "<migration managed identity name>"
  in schema public grant select, insert, update, delete on tables
  to "<API managed identity name>";
alter default privileges for role "<migration managed identity name>"
  in schema public grant usage, select, update on sequences
  to "<API managed identity name>";
```

Run the existing-table grants after initial schema creation. Default privileges
cover future objects created by the named migration identity. Keep DDL, role
administration, and database ownership off the API identity.

Restrict the audit tables after every schema migration. Create the audit worker's
Entra principal with `pgaadauth_create_principal_with_oid`, using its own managed
identity name and principal UUID as for the API. Its password-free URL belongs
only in the worker's Key Vault secret. With the schema owner's short-lived
`DATABASE_URL` available to Bun, run:

```sh
AUDIT_DB_GRANTS_ONLY=true \
API_DATABASE_ROLE='<API managed identity name>' \
AUDIT_DATABASE_ROLE='<audit worker managed identity name>' \
bun apps/backend/shared/audit_database_roles.ts
```

This mode preserves both managed-identity logins and does not set their passwords. It
restricts API audit access to reading evidence and inserting pending records;
the worker owns sealing, signing state, archives, and its lease. Run it before
starting either workload, with Bun's `pg` dependency available. Neither role may
inherit a schema owner or another role that restores the revoked privileges.

### Rejected database configuration

The API's database client and migration job reject the following settings when
present, including empty values:

```text
DATABASE_URL, PGPASSWORD, POSTGRES_PASSWORD, AZURE_POSTGRES_PASSWORD,
AZURE_POSTGRES_CONNECTION_STRING, AZURE_POSTGRESQL_CONNECTIONSTRING,
PGHOST, PGHOSTADDR, PGPORT, PGUSER, PGDATABASE, PGSSLMODE, PGSSLROOTCERT,
PGSSLCERT, PGSSLKEY, PGPASSFILE, PGSERVICE, PGSERVICEFILE, PGOPTIONS, PGAPPNAME,
MSI_ENDPOINT, MSI_SECRET, IMDS_ENDPOINT, IDENTITY_SERVER_THUMBPRINT,
AZURE_AUTHORITY_HOST, HTTP_PROXY, HTTPS_PROXY, ALL_PROXY
```

Lowercase proxy names are also rejected. Unset an unwanted setting rather than
assigning an empty value. The API additionally rejects storage keys, SAS tokens,
client secrets, ACS connection strings, emulator switches, signature-skip flags,
and custom storage endpoints. The exact names are maintained in the
[API configuration guard](https://github.com/Rheosoph/flow-like/blob/main/apps/backend/azure/api/src/config.rs)
and [PostgreSQL client](https://github.com/Rheosoph/flow-like/blob/main/packages/azure-data/src/postgres.rs).

## Apply schema changes

Build the migration image from the repository root:

```sh
docker build -f apps/backend/azure/migration/Dockerfile -t flow-like-azure-migration .
```

Run it as a manually triggered Container Apps Job with the same PostgreSQL
settings as the API, replacing `AZURE_CLIENT_ID` and `AZURE_POSTGRES_USER` with
the migration identity. After the role bootstrap, run this job before an API
revision that needs the changed schema:

```sh
az containerapp job start --name <migration-job> --resource-group <resource-group>
```

The image builds and validates a PostgreSQL mirror of the tracked CockroachDB
Prisma schema. At runtime it acquires an Entra login token, runs the guarded
pre-push column conversions, and then runs `prisma db push`. Connection URLs
exist in the child processes' environment. Prisma receives
`sslmode=require&sslaccept=strict`; the pre-push node-postgres client receives
`uselibpqcompat=true&sslmode=verify-full`. Both verify the server certificate.

The job omits `--accept-data-loss`. If Prisma refuses a destructive change,
inspect its warnings and the current schema, including any conversions already
applied by the pre-push step. Apply an intended destructive step separately as
the migration identity with a short-lived token, then rerun the job; an additive
schema change can also avoid the destructive step.

Exit `2` indicates rejected configuration, and exit `1` can indicate token
failure or a signaled child. Otherwise the job returns the failing pre-push
exit code, or Prisma's exit code after pre-push succeeds. Cancellation forwards
SIGTERM to the child. The
[migration entrypoint](https://github.com/Rheosoph/flow-like/blob/main/apps/backend/azure/migration/migrate.ts)
defines this sequence. For local runner checks, use `bun install`, `bun test`,
and `bunx tsc --noEmit` from `apps/backend/azure/migration`.

## Email through Azure Communication Services

Set the following on the API and grant its identity the scoped communications
sender role:

```text
MAIL_PROVIDER=azure_communication_services
ACS_EMAIL_ENDPOINT=https://<resource>.communication.azure.com
ACS_EMAIL_SENDER=DoNotReply@<azure-managed-domain>.azurecomm.net
```

Use the actual verified sender exposed by your communications resource. The
endpoint and sender are identifiers. Authentication uses the API's managed
identity; access keys, connection strings, and `AZURE_CLIENT_SECRET` are
rejected. The sender role needs
`Microsoft.Communication/CommunicationServices/Read` and
`Microsoft.Communication/CommunicationServices/Write`; keep local authentication
disabled on the ACS resource. The client disables engagement tracking and waits
for the send operation to succeed before reporting success.

## Health and telemetry

`/health/live` checks the API process. `/health/ready` and `/health/startup`
require an accepting token lifecycle and a successful PostgreSQL ping.
`/health` aliases readiness for deployments using the compatibility probe.

The [native OTLP collector](https://github.com/Rheosoph/flow-like/blob/main/apps/backend/azure/otel-collector/collector.yaml)
receives OTLP/gRPC on port 4317 and exports logs, traces, and metrics to Azure
Monitor. Set `AZURE_CLIENT_ID`, `DEPLOYMENT_ENVIRONMENT`,
`AZURE_MONITOR_TRACES_ENDPOINT`, `AZURE_MONITOR_METRICS_ENDPOINT`, and
`AZURE_MONITOR_LOGS_ENDPOINT` on the collector. Configure the endpoints for the
deployment's private Data Collection Endpoint and grant the collector identity
`Monitoring Metrics Publisher` scoped to its Data Collection Rule.

Expose 4317 only as internal TCP ingress. Port 13133 is the health extension
for Container Apps probes and needs no ingress. Collector logs go to stdout;
retain them in the environment's Log Analytics workspace to diagnose exporter
and identity failures. The queue is memory-only, so a process loss can discard
queued telemetry. Verify ingestion, retries, alerts, and replica failover before
depending on this pipeline for operations.

Build the collector from the repository root so the image includes its license:

```sh
docker buildx build --platform linux/amd64 \
  --tag "$ACR_LOGIN_SERVER/flowlike/otel-collector:$VERSION" \
  --file apps/backend/azure/otel-collector/Dockerfile --push .
```

Promote a scanned and signed immutable registry digest. When updating the
collector base, update its pinned digest and version comment together.
