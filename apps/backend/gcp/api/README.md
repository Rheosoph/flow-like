# Flow-Like GCP API

The GCP image is keyless. It authenticates to Cloud SQL, Cloud Storage,
Firestore, Pub/Sub and Secret Manager with tokens minted by the instance
metadata server for its Cloud Run service account. It does not accept a service
account key, a PostgreSQL password, or `DATABASE_URL`, and startup fails if one
is present rather than ignoring it.

## Secure image build

The API embeds reviewed, non-secret identity and OAuth-provider metadata at
compile time; JWKS are fetched through the bounded runtime cache. Pass the GCP
configuration contents through a BuildKit secret, and pass only its non-secret
SHA-256 digest as a build argument so a metadata change invalidates the cached
compile layer. A local `flow-like.gcp.config.json` must be gitignored and
dockerignored so it stays out of version control and the build context. The
tracked repo-root `flow-like.config.json` is the committed public default that
builds require; it does enter the context, the builder overwrites it with the
secret for the duration of the build `RUN`, and the same `RUN` removes the copy
as a standalone file. The reviewed non-secret contents are intentionally
embedded in `/app/api`, so this input must never contain client secrets:

```sh
CONFIG_PATH=/secure/path/flow-like.gcp.config.json
CONFIG_SHA256="$(openssl dgst -sha256 "$CONFIG_PATH" | awk '{print $NF}')"
docker buildx build \
  --secret id=flow_like_config,src="$CONFIG_PATH" \
  --build-arg FLOW_LIKE_CONFIG_SHA256="$CONFIG_SHA256" \
  -f apps/backend/gcp/api/Dockerfile \
  .
```

The Dockerfile verifies the digest and rejects a config whose provider is not `gcp`. Keep the source
under the protected CI workspace, record its digest in release evidence, and do
not include client secrets in it.

The configuration must also carry a `mail.smtp` block — see
[SMTP relay](#smtp-relay). The tracked public default has none, and its absence
is not detected at startup.

## Environment contract

### Required

```text
GCP_PROJECT_ID=<app project id>
GCP_CONTENT_BUCKET=<content bucket>
GCP_META_BUCKET=<meta bucket>          # must NOT equal the content bucket
GCP_CDN_BUCKET=<cdn bucket>
GCP_LOG_BUCKET=<logs bucket>
SECRET_PREFIX=/flow-like/gcp-<environment>
MAIL_PROVIDER=smtp
CORS_ALLOWED_ORIGINS=https://<frontend domain>

GCP_POSTGRES_AUTH_MODE=iam
GCP_POSTGRES_HOST=<PSA private IP or *.cloudsql.goog name>
GCP_POSTGRES_DATABASE=flow_like
GCP_POSTGRES_USER=<service account local part>@<project>.iam
GCP_POSTGRES_SERVER_CA=<instance server CA, PEM>
```

Meta and content must be **separate buckets**.
`GcpRuntimeCredentials::scoped_credentials` narrows a downscoped token by object
prefix *within* a bucket, and both live under the same `apps/<app_id>/` prefix
space. Sharing one bucket therefore collapses the `ReadAppContent` boundary into
`ReadApp` and hands a content-only credential the app's `.board` and `.event`
definitions. Startup rejects the collapsed configuration.

### Pinned

`STORAGE_PROVIDER`, `RUNTIME_CREDENTIALS_PROVIDER`, `META_BUCKET_PROVIDER`,
`CONTENT_BUCKET_PROVIDER` and `LOGS_BUCKET_PROVIDER` must be `gcp` (`gcs` and
`google` are accepted aliases). The image compiles the AWS and Azure feature
sets out entirely, so `default_provider_name()` already resolves to `gcp`; a
value naming another cloud means the workload was handed the wrong environment
and is rejected at startup rather than at the first scoped-credential request.

### Alias agreement

`BucketConfig::from_env` in `packages/api` prefers the `GCP_`-prefixed name for
the meta, content and logs buckets but prefers the generic `CDN_BUCKET_NAME`
over `GCP_CDN_BUCKET`. When both spellings are set they must name the same
bucket: a disagreement makes this process build its CDN store from one bucket
while the rest of the API signs URLs against another, which does not error — it
just 404s. Startup enforces the agreement for all four pairs
(`META_BUCKET`, `CONTENT_BUCKET`, `LOG_BUCKET`, `CDN_BUCKET_NAME`).

### Optional

```text
PORT=8080                                   # Cloud Run injects this
SENTRY_ENDPOINT=<dsn>                       # omit to disable Sentry entirely
OTEL_EXPORTER_OTLP_ENDPOINT=http://<collector>:4317
GCP_REQUIRE_OTEL=true                       # makes a missing endpoint fatal
RUST_LOG=warn
```

### Forbidden

Startup fails when any of these is present, **even with an empty value**:

```text
GOOGLE_APPLICATION_CREDENTIALS      GOOGLE_APPLICATION_CREDENTIALS_JSON
GOOGLE_CREDENTIALS                  GOOGLE_OAUTH_ACCESS_TOKEN
CLOUDSDK_AUTH_ACCESS_TOKEN          GCE_METADATA_HOST
GCE_METADATA_IP                     GCE_METADATA_ROOT
METADATA_SERVER_DETECTION           HTTP_PROXY / HTTPS_PROXY / ALL_PROXY (+ lowercase)

GOOGLE_SERVICE_ACCOUNT              GOOGLE_SERVICE_ACCOUNT_PATH
GOOGLE_SERVICE_ACCOUNT_KEY          SERVICE_ACCOUNT
GOOGLE_SKIP_SIGNATURE               GOOGLE_ALLOW_HTTP
GOOGLE_ALLOW_INVALID_CERTIFICATES   GOOGLE_PROXY_URL
GOOGLE_PROXY_CA_CERTIFICATE         GOOGLE_PROXY_EXCLUDES

STORAGE_EMULATOR_HOST               PUBSUB_EMULATOR_HOST
FIRESTORE_EMULATOR_HOST             DATASTORE_EMULATOR_HOST
SECRET_MANAGER_EMULATOR_HOST

CLOUDSDK_API_ENDPOINT_OVERRIDES_*   GOOGLE_*_CUSTOM_ENDPOINT

DATABASE_URL, PG*, POSTGRES_PASSWORD, GCP_POSTGRES_PASSWORD,
INSTANCE_CONNECTION_NAME, CSQL_PROXY_*   (rejected by flow-like-gcp-data)
```

The reasoning matches Azure's IMDS reasoning. `metadata.google.internal` is a
plaintext, unauthenticated endpoint that hands out the workload's identity to
anything that can reach it; every variable above either redirects that endpoint,
replaces its answer, or puts a third party on the path to it. Presence is fatal
rather than ignored because a variable that reached the deployment is evidence
that something intended it to take effect — and because whether an empty value
parses into something harmless is a property of third-party parsers this image
does not control.

The Cloud SQL Auth Proxy is forbidden for a second reason: it terminates TLS on
localhost, which would turn the `verify-full` connection into a check against a
certificate the proxy chose.

## IAM roles for the API service account

Grant these to the Cloud Run runtime service account (never the default compute
service account, which carries project Editor):

| Role | Scope | Why |
|---|---|---|
| `roles/cloudsql.client` | Cloud SQL instance | `cloudsql.instances.connect`; required even under IAM database authentication |
| `roles/cloudsql.instanceUser` | Cloud SQL instance | `cloudsql.instances.login` — the IAM database login itself |
| `roles/secretmanager.secretAccessor` | **each secret individually** | see the note below |
| `roles/storage.objectUser` | meta bucket | read, write and delete app metadata |
| `roles/storage.objectUser` | content bucket | read, write and delete app content |
| `roles/storage.objectUser` | CDN bucket | the CDN store this crate builds |
| `roles/storage.objectCreator` | logs bucket | append-only; the API never reads or deletes log objects |
| `roles/iam.serviceAccountTokenCreator` | **on itself** | `iam.serviceAccounts.signBlob`, which `InstanceSigningCredentialProvider` needs to mint V4 signed URLs — the image holds no key to sign with |
| `roles/datastore.user` | project (Firestore Native database) | cache and execution-state documents |
| `roles/pubsub.publisher` | execution and compilation topics only | async dispatch; publisher on the topic, never project-wide |
| `roles/run.invoker` | executor Cloud Run service only | synchronous dispatch; the executor's IAM front door admits this identity and nobody else |
| `roles/logging.logWriter` | project | Cloud Run needs it to ship container stdout/stderr to Cloud Logging |

`roles/secretmanager.secretAccessor` must be bound **per secret**, not on the
project. `GcpSecretManagerProvider` always appends the *unprefixed* secret name
as a fallback candidate after the `SECRET_PREFIX`-qualified one, so a
project-wide grant would let one environment's API resolve another environment's
secret through that fallback. Per-secret bindings are what make the prefix an
isolation boundary instead of a naming convention.

`roles/run.invoker` pairs with `EXECUTOR_AUTH=gcp_id_token` in the API
environment. The executor service admits only this service account as an
invoker, so `dispatch_http` / `dispatch_http_sse` mint a Google ID token from
the metadata server — audience `EXECUTOR_URL`, reduced to its bare origin when
the configured value carries a path, because Cloud Run validates `aud` against
the service URL — and attach it as `Authorization: Bearer`. The backend JWT
inside the payload stays the application-layer credential; the ID token
answers Cloud Run's IAM layer, and ingress restriction is a third, independent
control rather than a substitute for either. With `EXECUTOR_AUTH` set, a
dispatch that cannot reach the metadata server fails with an error naming the
missing token instead of traveling on to an anonymous 403.

Deliberately **not** granted:

- Cloud Scheduler roles. Under the deployment's scheduler decision, Cloud
  Scheduler drives a Cloud Run **job** that pokes this API; the API never
  creates or mutates scheduler jobs itself.
- Any role on the Terraform state project. The API only touches the app project.

## Cloud SQL IAM database authentication

`google_sql_user` with `type = CLOUD_IAM_SERVICE_ACCOUNT` creates a login, not a
set of privileges. Connect to `flow_like` as the migration identity and grant
runtime DML rights only. Replace the quoted role names with the exact Terraform
names, and note the truncation rule: Cloud SQL names the IAM database user after
the service-account email with `.gserviceaccount.com` removed, and PostgreSQL
truncates identifiers at 63 bytes:

```sql
grant connect on database flow_like to "<sa-local-part>@<project>.iam";
grant usage on schema public to "<sa-local-part>@<project>.iam";
grant select, insert, update, delete on all tables in schema public
  to "<sa-local-part>@<project>.iam";
grant usage, select, update on all sequences in schema public
  to "<sa-local-part>@<project>.iam";

alter default privileges for role "<migration-sa-local-part>@<project>.iam"
  in schema public
  grant select, insert, update, delete on tables
  to "<sa-local-part>@<project>.iam";
alter default privileges for role "<migration-sa-local-part>@<project>.iam"
  in schema public
  grant usage, select, update on sequences
  to "<sa-local-part>@<project>.iam";
```

Do not grant the API identity DDL, role administration, or database-owner
rights. Run schema migrations as the separate migration identity.

The process requests a token for
`https://www.googleapis.com/auth/sqlservice.login` — the narrowest scope Cloud
SQL accepts — places it only in the SQLx connection options, and enforces TLS
`verify-full` against the pinned instance CA. Cloud SQL signs its serving
certificate with a per-instance CA that is in no public trust store, so
`GCP_POSTGRES_SERVER_CA` is what makes `verify-full` mean anything here; without
it the only options are a weaker SSL mode or the proxy, and both are rejected.

SQLx 0.8 has no async password callback for new pooled connections, so the API
closes readiness five to eight minutes before the token expires and terminates
after a drain window. Per-process jitter staggers instances that received tokens
together. Run **at least two instances** so a drain never empties the service,
and keep the revision restartable so each replacement starts on a fresh token.

## SMTP relay

GCP has no first-party transactional email service, so the deployment brings its
own relay:

```text
MAIL_PROVIDER=smtp
```

`create_mail_client` treats `MAIL_PROVIDER` as a hard-error selector, and
startup rejects any value other than `smtp` on this image. Two things it cannot
check:

1. `SmtpMailClient::new` reads the relay host, port, username and password from
   **environment variables whose names come from the compiled-in
   `mail.smtp` block**. Cloud Run must mount those four names from Secret
   Manager versions. The password therefore lives in the process environment,
   unlike every other secret this API uses — keep the relay credential scoped to
   sending and rotate it on its own schedule.
2. The tracked public `flow-like.config.json` sets `mail.provider = "ses"` and
   carries **no** `smtp` block. With that config the process starts cleanly and
   then fails with `SMTP settings required for SMTP provider` at the first
   outbound email. The BuildKit-secret configuration must supply the block.

## Health and metrics contract

- `/health/live` checks only the process.
- `/health/ready` and `/health/startup` require an accepting token lifecycle and
  a successful Cloud SQL ping.
- `/health` is a compatibility alias for readiness, used by the Cloud Run
  startup/liveness probes and the load balancer's backend health check.
- `/metrics` renders the Prometheus registry.

Cloud Run publishes exactly one container port, so unlike the Azure image there
is no second `METRICS_PORT` listener — `/metrics` is mounted on the serving
router. Keep it out of the load balancer's URL map: Cloud Run ingress restricted
to the load balancer is what keeps the endpoint off the public internet.

Two independent triggers close readiness: the database token entering its safety
window, and `SIGTERM`. They drain differently on purpose. The token path waits
out `READINESS_DRAIN_SECONDS` so the health check observes the closed endpoint
before the listener disappears; the `SIGTERM` path does not, because Cloud Run
sends `SIGTERM` roughly ten seconds before `SIGKILL` and has already removed the
instance from rotation, so waiting would guarantee the hard kill lands mid
request.

## Deploy ordering

Every item below must hold **before** the first revision receives traffic;
`validate_security_prerequisites` fails closed and Cloud Run will otherwise
restart-loop the revision:

1. The six required secrets exist with a version under `SECRET_PREFIX`, and the
   runtime service account holds `roles/secretmanager.secretAccessor` on each:
   `BACKEND_KEY` (≥64 bytes), `BACKEND_PUB` (≥64), `BACKEND_KID` (≥8),
   `SINK_SECRET` (≥32), `SINK_TOKEN_ENCRYPTION_KEY` (≥32), `MAINTENANCE_TOKEN`
   (≥32). The secret store is constructed with `allow_env_override(false)`, so
   an environment variable of the same name will **not** satisfy the check.
2. The Cloud SQL IAM user exists **and** the in-database grants above have been
   applied by the migration identity. Cloud SQL creating the login is not
   enough.
3. `GCP_POSTGRES_SERVER_CA` carries the current instance CA. Rotating the
   instance CA requires redeploying the revision — the PEM is read once at
   startup.
4. The meta and content buckets are distinct, and the four `GCP_*`/generic
   bucket aliases agree.
5. The Firestore database and the Pub/Sub topics exist. The API publishes on
   dispatch; a missing topic surfaces as a failed user action, not a failed
   startup.
6. The image was built from a `flow-like.config.json` whose provider is `gcp`
   and which contains a `mail.smtp` block.

## GCP references

- [Cloud Run container runtime contract](https://cloud.google.com/run/docs/container-contract)
- [Cloud Run service identity](https://cloud.google.com/run/docs/securing/service-identity)
- [Cloud SQL IAM database authentication](https://cloud.google.com/sql/docs/postgres/iam-logins)
- [Cloud SQL server CA and TLS verification](https://cloud.google.com/sql/docs/postgres/configure-ssl-instance)
- [Secret Manager access control](https://cloud.google.com/secret-manager/docs/access-control)
- [Credential Access Boundaries (downscoped tokens)](https://cloud.google.com/iam/docs/downscoping-short-lived-credentials)
