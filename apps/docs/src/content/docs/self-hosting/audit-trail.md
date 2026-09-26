---
title: Audit trail storage
description: Configure the audit worker, the audit keys and the immutable audit bucket of a self-hosted deployment
---

Audit evidence survives an API compromise only when the API cannot sign history,
change worker-owned database rows, or write the audit bucket. Docker Compose,
Kubernetes, Azure and GCP use a dedicated audit worker for that boundary. The worker
seals pending records, signs epochs, publishes checkpoints, archives and prunes.
What the trail records is described in
[Platform administration](/dev/platform-administration/#audit-trail).

## Who holds what

| Component | Holds | Can |
| --- | --- | --- |
| API | Entry key, public verifying keys, API database login | Insert pending records and read evidence |
| Audit worker | Entry key, signing authority, audit bucket identity, worker database login | Seal, sign, checkpoint, archive and prune |
| Migration job | Schema owner database login | Migrate tables and reconcile restricted roles |
| Independent verifier | Public verifying keys, read-only database login, retained checkpoints | Detect divergence from previously witnessed history |

Keep worker secrets and identities inaccessible to the API. A different container
with the same cloud identity or database owner login does not provide this boundary.
The entry key authenticates submissions from the API; a compromised API can still
invent or omit new events. Previously witnessed history is the protection target.

## Run the audit worker

| Deployment | Where the worker runs |
| --- | --- |
| Docker Compose and Swarm | Separate `audit-worker` service using `flow-like-audit-worker` |
| Kubernetes | Separate Deployment and service account; `audit.worker` controls this Deployment |
| Azure | Scheduled Container Apps Job using `flow-like-azure-audit-worker` |
| GCP | Scheduled Cloud Run Job using `flow-like-gcp-audit-worker` |
| AWS Lambda | Existing `apps/backend/aws/audit-worker` function |
| Local development and AWS ECS | Legacy in-process mode remains available; isolate AWS ECS signing in the Lambda worker and set `AUDIT_WORKER=off` on the API |

The four dedicated deployment targets never start a worker inside their API. Their
API binaries reject mounted audit signing credentials, audit bucket settings, and
`AUDIT_WORKER=on`. API state loads public verifying keys without fetching a private
audit key from the secret store. Removing environment variables must be accompanied
by removing the API identity's access to the underlying secrets, signing key and
storage.

The shared worker binary accepts `--once` for scheduled jobs and otherwise runs once
a minute. It loads only its compiled audit policy and the database connection, without
starting API routes or loading the backend JWT private key. Signing and an audit
bucket are required. A failed signing connection or checkpoint check produces a
failed run. `--health-check` fails after 15 minutes without a successful tick.

The worker coordinates through `AuditWorkerLease`, which the API role cannot access.
Scheduled jobs release the lease after each tick; a stopped continuous worker's
lease expires within five minutes. Disabling the dedicated worker leaves records
pending. It does not transfer the work to the API.

The migration job owns the SQL boundary between the two logins: it grants the
restricted roles and fails when the API login can still change evidence beyond
appending pending `AuditRecord` rows. The worker does not check grants itself, so run
the migration job after every schema or role change.

### Audit policy

The dedicated worker's audit policy is the `audit` section of the
`flow-like.config.json` its image was built with, the same build input as the API
image. The `flow-like-audit-worker` recipe takes `FLOW_LIKE_CONFIG` like the Compose
and Kubernetes API recipes and defaults to the same self-hosting example. The Azure
and GCP worker recipes take the API's `flow_like_config` BuildKit secret and
`FLOW_LIKE_CONFIG_SHA256` build argument. A document without an `audit` section
yields the defaults. `"enabled": false` stops the worker at startup, and signing is
required whatever `require_signing` says.

The worker reads no configuration at runtime. `FLOW_LIKE_CONFIG_JSON`,
`FLOW_LIKE_CONFIG_FILE` or `FLOW_LIKE_CONFIG_SECRET_REF` on the API replace the API's
document only; they do not reach the worker. Set audit policy in the build-time
config and rebuild the worker whenever it changes. The AWS Lambda worker is the
exception: it builds the Lambda API's state and reads the same configuration as the
Lambda API.

### Pause the worker

Pausing is a platform action, not a worker setting. Disable the schedule on AWS
(EventBridge Scheduler) or GCP (Cloud Scheduler), switch the Azure Container Apps Job
to a manual trigger, set `audit.replicaCount: 0` in the Helm chart, or stop the
Compose service. Records stay pending while the worker is paused and are sealed once
it runs again.

### Entry key

Every API and worker needs the same explicit `AUDIT_ENTRY_KEY`. Setup scripts
generate this separately from `BACKEND_KEY`, so the worker never needs the backend
JWT signing key. A worker holding a different entry key quarantines nothing: every
record carries the id of the key that authenticated it, so the worker holds the chain
(`held_chains` in its tick report, log line `audit entry key mismatch: chain <id>
carries kid <x|null>, worker holds [<kids>]`), seals nothing on it and retries on the
next tick. Supply the missing key as `AUDIT_ENTRY_KEY_PREVIOUS` and the chain seals;
no row has to be deleted. The dedicated worker reads only `AUDIT_ENTRY_KEY` and
`AUDIT_ENTRY_KEY_PREVIOUS`, never `BACKEND_KEY` or `BACKEND_KEY_PREVIOUS`, so a key
the API derived from `BACKEND_KEY` reaches it only as the explicit value computed
below. The AWS Lambda worker derives it from `BACKEND_KEY` and `BACKEND_KEY_PREVIOUS`
like the API.

If an existing deployment left `AUDIT_ENTRY_KEY` unset, preserve its derived key
before switching workers. The derivation uses BLAKE3 derive-key with the exact context
`flow-like 2026-09 audit entry key v1` and the UTF-8 bytes of the existing `BACKEND_KEY`
base64 text, after trimming surrounding whitespace. Keep that text encoded; decoding
it first produces a different key. With Python 3 and `b3sum` installed and the existing
`BACKEND_KEY` exported, this prints the compatible entry key as base64 of 32 bytes:

```sh
python3 - <<'PY'
import base64, os, subprocess
key = os.environ["BACKEND_KEY"].strip()
if not key:
    raise SystemExit("BACKEND_KEY is empty")
digest = subprocess.run(
    ["b3sum", "--derive-key", "flow-like 2026-09 audit entry key v1", "--raw"],
    input=key.encode("utf-8"), stdout=subprocess.PIPE, check=True,
).stdout
print(base64.b64encode(digest).decode("ascii"))
PY
```

Store the output as `AUDIT_ENTRY_KEY` on every API and worker before restarting them.
If rotating to a new entry key during the same change, store this derived value as
`AUDIT_ENTRY_KEY_PREVIOUS` on both instead. Keep `BACKEND_KEY` off the worker.

### Rotate the entry key

1. Set the new key as `AUDIT_ENTRY_KEY` and the old one as `AUDIT_ENTRY_KEY_PREVIOUS`
   on every process, worker included, and roll them.
2. Wait until everything written before the switch is sealed and signed:
   `seal_after_seconds` plus `epoch_interval_seconds` plus a few minutes.
3. Remove `AUDIT_ENTRY_KEY_PREVIOUS` and roll again.

Deployments that derive the entry key from `BACKEND_KEY` set `AUDIT_ENTRY_KEY`
explicitly before rotating `BACKEND_KEY`; otherwise the derived key changes with it and
the old one is gone. On AWS, where the API and the Lambda worker both derive it,
setting the old value as `BACKEND_KEY_PREVIOUS` on both keeps it accepted instead.

### Held chains

A seal that fails its hash or its MAC before an epoch signed it is an integrity
incident: the worker logs `unanchored audit seal fails its hash or MAC` at error level,
puts that chain on hold in `AuditHeldChain`, and never signs, archives or prunes any of
its seals. Every other chain carries on, and the dashboard counts held chains. Alert on
that message.

To resolve one, investigate what changed the row. If the cause was a lost entry key,
restore it as `AUDIT_ENTRY_KEY_PREVIOUS` and delete the chain's `AuditHeldChain` row;
the next run signs the chain again. Otherwise keep the rows as evidence: nothing
deletes them.

A chain whose pending records were authenticated with a key the worker does not hold
is held without a row: the worker logs `audit entry key mismatch`, counts the chain in
`held_chains`, seals and quarantines nothing on it, and tries again on the next tick.
Only a record whose key the worker does hold and whose MAC still fails is quarantined.

### Log lines that fail closed

| Line | Level | Effect |
| --- | --- | --- |
| `audit entry key mismatch: chain <id> carries kid <x\|null>, worker holds [<kids>]` | error | That chain is held and `held_chains` counts it. The tick fails only when every due chain was held |

Alert on it. A privilege boundary violation surfaces in the migration job, which
fails, not in the worker.

## Environment

| Variable | Set on | Meaning |
| --- | --- | --- |
| `AUDIT_WORKER` | API | Dedicated targets accept unset or `off`; `on` is rejected. Legacy local/AWS ECS mode still uses this switch |
| `AUDIT_ENTRY_KEY` | API and worker | Base64 of 32 random bytes, the same everywhere. Required explicitly by the dedicated worker |
| `AUDIT_ENTRY_KEY_PREVIOUS` | API and worker | The entry key used before a rotation, still accepted while records and seals written with it are sealed and signed (see [Rotate the entry key](#rotate-the-entry-key)) |
| `AUDIT_SIGNING_KEY` | Worker | Audit key: base64 of a PKCS#8 PEM P-256 private key. Exactly one of this and `AUDIT_KMS_KEY_ID` is set; the worker refuses to start with both |
| `AUDIT_KMS_KEY_ID` | Worker | Audit key in a key service instead: an AWS KMS key id, ARN or alias, a Cloud KMS key version (`projects/.../cryptoKeyVersions/N`), an Azure Key Vault key (`https://<vault>/keys/<name>/<version>`) or a Vault transit key (`transit/audit`) |
| `AUDIT_KMS_PROVIDER` | Worker | `aws`, `gcp`, `azure` or `vault`. Unset: Vault when `AUDIT_VAULT_ADDR` is set, otherwise taken from the shape of `AUDIT_KMS_KEY_ID` |
| `AUDIT_VAULT_ADDR` | Worker | Vault or OpenBao address, for example `https://vault:8200` |
| `AUDIT_VAULT_TOKEN_FILE`, `AUDIT_VAULT_TOKEN` | Worker | Vault token file or token. The file takes precedence and is reread on each request so a Vault Agent can renew it |
| `AUDIT_VAULT_CA_FILE` | Worker | PEM bundle of the CA that signed the Vault certificate, trusted in addition to the public roots. Required for a Vault with a private CA; refused together with an `http://` address |
| `AUDIT_KMS_REGION` | Worker | Region of an AWS KMS key. Unset: the region in the key ARN, else the deployment's default |
| `AUDIT_KMS_AWS_ACCESS_KEY_ID`, `AUDIT_KMS_AWS_SECRET_ACCESS_KEY` | Worker | Credentials for AWS KMS, both or neither. Required with a bundled object store, whose `AWS_ACCESS_KEY_ID` the default chain would otherwise use |
| `AUDIT_KID` | Worker | Key id written into epochs. Default: `audit-es256-` and 16 hex digits of the public key's fingerprint |
| `AUDIT_VERIFYING_KEYS` | API | Public keys of audit keys the process does not hold, including retired ones, as `{"<kid>": "<SPKI PEM>"}` |
| `AUDIT_BUCKET` or `AWS_AUDIT_BUCKET` | Worker | Audit bucket on S3 or an S3-compatible store |
| `AUDIT_BUCKET_ENDPOINT` | Worker | S3 endpoint of the audit bucket. Default: the endpoint of the other buckets |
| `AUDIT_BUCKET_ACCESS_KEY_ID`, `AUDIT_BUCKET_SECRET_ACCESS_KEY` | Worker | Dedicated identity for the audit bucket, both or neither. Without them the shared S3 credentials, or the task, pod or web identity, are used |
| `AUDIT_BUCKET_KMS_KEY_ARN` | Worker | SSE-KMS key sent with every upload |
| `GCP_AUDIT_BUCKET` | Worker | Audit bucket on Google Cloud Storage, with the credentials of the other GCP buckets |
| `AZURE_AUDIT_CONTAINER` | Worker | Audit container in `AZURE_STORAGE_ACCOUNT_NAME`, with the credentials of the other containers |
| `AUDIT_BUCKET_LOCK_MODE`, `AUDIT_BUCKET_RETENTION_YEARS` | Compose object-store bootstrap | New buckets default to `COMPLIANCE` and 4 years. Existing insufficient policies fail validation without being changed |
| `DATABASE_URL` | Each process | Its own restricted login. The worker and API never receive the migration owner login |
| `SINK_TOKEN_ENCRYPTION_KEY` | Worker, when using webhooks | Same encryption secret as the API; without it the worker skips webhook delivery |

The first bucket variable that is set wins, in the order `AUDIT_BUCKET`,
`AWS_AUDIT_BUCKET`, `GCP_AUDIT_BUCKET`, `AZURE_AUDIT_CONTAINER`. S3 uploads carry a
SHA-256 checksum, which Object Lock buckets require.

The worker logs the key id and its PEM public key when it gets its audit key:
`audit signing key loaded` for `AUDIT_SIGNING_KEY` at startup, `audit key-service key
connected` for `AUDIT_KMS_KEY_ID` when the worker starts. Copy that pair into
`AUDIT_VERIFYING_KEYS` of every process without the key, so they can verify epochs,
and hand it to app owners who verify exports offline.

### Keys in a key service

A key service keeps the audit key out of every process. There are three levels to
choose from: `AUDIT_SIGNING_KEY`, a private key in the deployment's own secret, which
is the default and needs nothing else; a self-hosted Vault or OpenBao `transit` mount,
where the key never leaves the mount and signing is free; and a cloud key service,
where the key lives in an HSM someone else operates and every signature is billed.

The build needs the matching feature of `flow-like-api`: `audit-kms-aws`,
`audit-kms-gcp` or `audit-kms-azure`. Vault and OpenBao are available in every build. A
key id for a provider the build lacks stops startup with an error that names the
feature.

| Provider | Key | Worker permissions |
| --- | --- | --- |
| AWS KMS | Asymmetric, key spec `ECC_NIST_P256`, usage `SIGN_VERIFY` | `kms:Sign`, `kms:GetPublicKey` |
| Cloud KMS | Algorithm `EC_SIGN_P256_SHA256` | Sign and view the public key, for example `roles/cloudkms.signerVerifier` on the key |
| Azure Key Vault | EC key on curve P-256 | Key permissions `get` and `sign` |
| Vault or OpenBao | Transit key of type `ecdsa-p256` | Read `<mount>/keys/<name>` and update `<mount>/sign/<name>` |

Every provider signs the 32-byte digest with ECDSA P-256 and SHA-256, the same
signature a local key produces, and the worker checks each signature against the
public key before it stores it. Cloud key versions use distinct identifiers. For
Vault, the worker pins the transit key version when it connects. If `AUDIT_KID` and
`AUDIT_VERIFYING_KEYS` register a public key, it selects that key's retained version;
otherwise it selects the latest version. Rotating the transit key does not change
the version used by a running worker. Keep the pinned version enabled for signing
until the worker is configured to use the replacement key.

#### Vault or OpenBao

A `transit` mount is the key service for deployments without a cloud account, and the
one Docker Compose and Kubernetes are set up for. The key never leaves the mount, and
signing is free of per-request charges.

```bash
bao secrets enable transit                          # or: vault secrets enable transit
bao write -f transit/keys/audit type=ecdsa-p256
```

Give the worker a token whose policy allows exactly the two calls it makes:

```hcl
path "transit/keys/audit"  { capabilities = ["read"] }
path "transit/sign/audit"  { capabilities = ["update"] }
```

Then point the worker at it:

```bash
AUDIT_KMS_KEY_ID=transit/audit        # or just `audit` for the default mount
AUDIT_VAULT_ADDR=https://vault:8200
AUDIT_VAULT_TOKEN_FILE=/vault/secrets/token   # or AUDIT_VAULT_TOKEN
AUDIT_VAULT_CA_FILE=/etc/flow-like/vault-ca.pem   # when Vault uses a private CA
```

`AUDIT_VAULT_TOKEN_FILE` is read again for every request, so a Vault Agent sidecar can
renew the token without restarting the worker. Renewal extends token validity; a
short initial TTL does not bound how long a copied token can be abused. Keep the
agent and its authentication identity with the worker, outside the API trust boundary.

Most self-hosted Vaults present a certificate from a private CA, and only the built-in
public roots are trusted otherwise. `AUDIT_VAULT_CA_FILE`, a PEM bundle of that CA, is
what makes an `https://` address reachable at all; without it the alternative is plain
`http://`, which puts the bearer token on the network in the clear. The worker refuses
a CA bundle together with an `http://` address and warns whenever it reaches a Vault
over plain HTTP. An `http://` address is acceptable only inside a private network you
already trust with that token.

### Signing cost

Each signature is one billed request to the key service. The number of signatures
does not grow with traffic, the number of apps or the number of records:

| Signed | When | At most |
| --- | --- | --- |
| Epoch | Once the oldest unsigned seal is `retention.epoch_interval_seconds` old (default 300), or earlier when 20,000 seals wait | 12 an hour, plus one per 20,000 seals under heavy load |
| Prune watermarks | One signature per prune run covers every chain it prunes; a run signs at most once an hour | 1 an hour |
| Archive manifest | Once per month | 1 a month |
| Checkpoint | Once per UTC day after the first epoch exists | 1 a day |

A worker process also refuses to sign more than 120 times in a rolling hour, which
limits request costs. On the four dedicated targets, API processes verify with
public keys only. Set `AUDIT_KID` and put that key id into
`AUDIT_VERIFYING_KEYS`, and the worker skips reading the public key from cloud key
services at startup as well. Vault still requires a metadata read to identify and
pin the matching version. Failed requests are retried once.

Seals wait for their epoch under a MAC made with the entry key, so a longer
`epoch_interval_seconds` lowers the cost without leaving sealed records unprotected;
it only delays when records are covered by the audit key.

Because that schedule is the only thing that produces a signature, the bill barely
moves with the size of the deployment. Prices checked in September 2026 (ECDSA P-256,
one key, one platform, per month):

| Records a month | Seals a month | Signatures | AWS KMS | Cloud KMS software | Cloud KMS HSM | Key Vault Standard | Key Vault Premium | Vault or OpenBao |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| 1 million | 2,000 | ~9,500 | $1.14 | $0.09 | $2.64 | $0.14 | $5.14 | — |
| 100 million | 200,000 | ~9,500 | $1.14 | $0.09 | $2.64 | $0.14 | $5.14 | — |
| 10 billion | 20 million | ~9,500 | $1.14 | $0.09 | $2.64 | $0.14 | $5.14 | — |
| 100 billion | 200 million | ~10,700 | $1.16 | $0.09 | $2.66 | $0.16 | $5.16 | — |

Each provider charges a monthly key fee plus $0.15 per 10,000 signatures, or $0.03 for
a Cloud KMS software key. The key fee is $1.00 for AWS KMS, $0.06 or $2.50 for a Cloud
KMS software or HSM key, nothing for Key Vault Standard and $5.00 for Key Vault
Premium. AWS KMS keys already live in HSMs, so a dedicated HSM cluster is not needed.
A self-hosted Vault or OpenBao charges nothing per signature; it costs the machine it
runs on.

Volume reaches the bill only above roughly 87 billion records a month, where the
175 million seals no longer fit in the 20,000 seals an epoch covers. Signing each
record instead would cost $1,500 per 100 million records on AWS KMS, more than a
thousand times the whole audit trail, which is why nothing below an epoch is signed.
Even a worker that signs at its 120-an-hour cap all month stays near $2.31.

What grows with usage is storage: records in the database until they are pruned, and
compressed NDJSON in the audit bucket for the retention horizon.

Four settings lower the bill further:

- Raise `retention.epoch_interval_seconds`. At 900 instead of 300 the epochs drop to
  about 2,900 a month ($1.05 with AWS KMS). It delays only when sealed records become
  covered by the audit key; the MAC protects them meanwhile.
- Share one audit signing key across replicas of the dedicated worker. Only the
  lease holder signs; a separate deployment with its own key pays another key fee.
- Set `AUDIT_KID` and register its public key in `AUDIT_VERIFYING_KEYS`, which removes
  the public-key read that AWS KMS bills on every worker start.
- Use a software key where an audit does not require an HSM, or self-host the key
  service with [Vault or OpenBao](#vault-or-openbao).

## Create the audit bucket

Whoever creates the bucket makes two choices the application does not: its
immutability and its storage tier. Heads live under `heads/`, one small object per
day. Archives live under `archive/YYYY/MM/`, one manifest and one or more parts per
month. Small completion receipts live under `receipts/archive/YYYY/MM/`. The first
legacy batch is `legacy/audit-entry.jsonl.zst`; later batches use distinct names in
`legacy/`.

| Provider | Immutability | Long-term tier | Note |
| --- | --- | --- | --- |
| AWS S3 | Object Lock, enabled when the bucket is created | Lifecycle transition of `archive/` to Glacier Deep Archive from day 0 | The lock survives the transition. Restores take hours. The 180-day minimum storage charge does not matter at multi-year retention |
| Azure Blob | Container or version-level immutability policy | Lifecycle management to the Archive tier | Immutability applies in every tier, and rehydration is allowed under a locked policy |
| Google Cloud Storage | Retention policy with Bucket Lock | Bucket default storage class Archive | Reads stay fast in Archive class but cost more |
| RustFS (bundled) | Object Lock, enabled when the bucket is created | One tier | The bootstrap creates the bucket with Object Lock on |

Keep `heads/`, `checkpoints/` and `receipts/` in the standard tier. The worker checks the receipts
before resuming an unfinished archive or signing its manifest, so database changes
cannot authorize pruning records that were never uploaded. A missing receipt stops
that month's archive step. Other worker steps, including pruning, still run.

For an unfinished month created without receipts, stop the worker before recovery.
If every source record, seal and epoch is still present and nothing from that month
was pruned, back up its archive metadata and remove all its `AuditArchive` rows,
including a pending part-zero manifest. Preserve the source rows, watermarks and
bucket objects. Restarting the worker then exports the month again. For completed
archives or months with missing source rows, restore trusted receipts from a bucket
backup or arrange a separately verified migration of the archived objects. There is
no automatic migration for those archives; do not manufacture receipts from database
rows or remove their manifest rows.

Only twelve archive manifests a year, plus their parts and receipts, are written, so per-object and
per-request charges of archive tiers stay negligible. Abort incomplete multipart
uploads after a few days, and expire objects once their lock has passed if nothing
else must keep them.

**Lock length.** The lock runs from the moment an object is written, and each
manifest states `retain_until`: 31 December of `archive_years_after_year_end` years
after the month. A month is written a few days after it closes, so a default retention
of `archive_years_after_year_end` plus one year covers every `retain_until`: four
years (1461 days) with the default of 3. New Compose and Helm configurations use four years. Existing three-year locks
leave January to November archives without a lock for their last one to eleven
months; extending a default policy does not repair old object versions automatically.

**Lock mode.** Governance mode keeps a privileged path to delete an archive when a
data protection authority or a court orders it. Compliance mode prevents deletion of protected versions until the lock expires.
The storage administrator must select a policy that meets the deployment's retention requirements. To keep records of an incident or dispute beyond their retention, set an Object
Lock legal hold on the affected archive objects.

**Credentials.** The worker's identity may write and read objects and list the
bucket, nothing else. On AWS that is `s3:PutObject`, `s3:GetObject` and
`s3:ListBucket`, plus `kms:GenerateDataKey` and `kms:Decrypt` on the key named by
`AUDIT_BUCKET_KMS_KEY_ARN`. Grant no delete permission and no
`s3:BypassGovernanceRetention`. API processes that do not run the worker get no access
to the audit bucket.

### Docker Compose

The object-store bootstrap creates `AUDIT_BUCKET` next to the metadata, content and
log buckets, with Object Lock enabled and the default retention from
`AUDIT_BUCKET_LOCK_MODE` and `AUDIT_BUCKET_RETENTION_YEARS`. It also creates the
identity `AUDIT_BUCKET_ACCESS_KEY_ID` / `AUDIT_BUCKET_SECRET_ACCESS_KEY`, whose policy
allows writing and reading only and denies deletion, retention and legal hold changes
and governance bypass. `scripts/setup-env.py` generates that identity and a dedicated
`AUDIT_SIGNING_KEY`. The worker reaches the object store directly, not through the
bucket-only gateway.

Object Lock can only be enabled when a bucket is created. The bootstrap stops with an
error when `AUDIT_BUCKET` already exists without it; choose a new bucket name. Leave
`AUDIT_BUCKET` empty to keep every record in the database.

To keep the key out of the `.env` instead, set `AUDIT_KMS_KEY_ID` and
`AUDIT_VAULT_ADDR` (with `AUDIT_VAULT_TOKEN` or `AUDIT_VAULT_TOKEN_FILE`) in the
environment of `scripts/setup-env.py`. It then writes those settings instead of
generating an `AUDIT_SIGNING_KEY`, and `scripts/preflight.py` checks that the address
and a token are present before the stack starts.

### Kubernetes

The Helm chart places signing settings in the worker Deployment. It uses separate
secrets for the entry key, bucket credentials, signing credentials and worker database
login; the audit policy is compiled into the worker image (see
[Audit policy](#audit-policy)). The API receives only the entry key, public keys and
its own database login. The object-store initializer receives bucket credentials,
without the signing key or Vault token.

`setup-config.py` generates TLS certificates for bundled CockroachDB and separate
migration, API and worker passwords. Runtime pods receive the CA certificate only;
the database gets its node key and the initializer gets the root client key. Node
and root client certificates expire after one year and need operator renewal.
Existing insecure databases require a planned TLS migration before using this chart.

The chart refuses privileged API scheduling configurations that could create pods
mounting worker secrets. Run such scheduling in a separately constrained namespace
before enabling it alongside isolated audit storage.

Vault Agent injection belongs on the worker pod, using `audit.podAnnotations` and
worker-only mounts. Set `vaultTokenFile` to its token sink, or supply a token through
the worker signing secret. The API has no access to either source.

The chart's network policy lets control-plane pods reach the public internet on port
443 only, so a Vault on another port needs a rule of its own:

```yaml
networkPolicy:
  controlPlaneExtraEgress:
    - to:
        - namespaceSelector:
            matchLabels:
              kubernetes.io/metadata.name: vault
      ports: [{ protocol: TCP, port: 8200 }]
```

The object-store conformance job writes an object under a one-day governance lock and
checks that deleting that object version fails even for the root identity, so a
storage regression cannot silently make the archive writable. The bundled RustFS
image is pinned to 1.0.0-rc.5; releases before 1.0.0-rc.1 did not enforce Object Lock
(CVE-2026-73288).

## Independent checkpoint verification

The worker writes `checkpoints/YYYY/MM/DD.json` once per UTC day. Its signature covers
the epoch, every retained chain tip, the key id and the write timestamp. Before
signing or pruning again, the worker checks its latest checkpoint against the
database. A missing or changed retained tip stops the tick, including after a worker
restart. Signed prune watermarks account for legitimate retention.

Copy checkpoint objects to an independently administered destination. Run the
verifier there with pinned public keys and a read-only database login:

```bash
flow-like-audit-worker --verify-checkpoint /retained/2026-09-20.json
```

Set `AUDIT_VERIFYING_KEYS` and `DATABASE_URL` for that command. It checks the
checkpoint signature, epoch timeline and retained per-chain tips without receiving
an entry key, signing credential or bucket write access. It fails when a checkpoint
is older than 48 hours by default; `AUDIT_CHECKPOINT_MAX_AGE_SECONDS` changes that
limit. Alert on failed checks and missing daily copies. Verification of the complete
record contents still uses the archive/export proofs.

Daily checkpoints leave activity after the last retained checkpoint unwitnessed.
An epoch-only object under `heads/` lacks the per-chain tips required to detect some
terminal-seal deletions. Use the new checkpoint objects for independent retention.
A local object store shares the host administrator's trust boundary; protect against
host compromise with storage and retained copies administered elsewhere.

## Upgrade an existing deployment

1. Stop the old in-process audit workers before starting the new worker. For a rolling
   upgrade, disable their worker first and drain the old API replicas. Do not run old
   and new worker versions together: they use different lease tables.
2. Apply `20260919120001_audit_seals` and `20260920120001_audit_worker_isolation`.
   PostgreSQL migrations are under `prisma/migrations/`; Aurora DSQL has its own
   migration directory. Use the migration owner connection.
3. Provision distinct API and worker database roles. The deployment migration applies
   `apps/backend/shared/audit_database_roles.ts` after creating the schema. Existing
   owner or administrative roles are rejected for either runtime identity. On Aurora
   DSQL, run the migration job once with `DSQL_AUDIT_ROLE_ARN` set to the worker's IAM
   role; see [the DSQL audit worker role](/self-hosting/aws/database/#audit-worker-role).
4. Preserve the entry key during the switch. Give the worker its signing authority and
   audit bucket identity, and build its image from the API's config
   ([Audit policy](#audit-policy)). Give the API the public keys
   in `AUDIT_VERIFYING_KEYS`; remove its access to worker secrets and cloud roles.
5. Check the actual storage retention. Existing inadequate policies stop bootstrap;
   plan their remediation separately. Provision TLS and renew certificates before
   migrating a previously insecure bundled Kubernetes database.
6. Start the worker and confirm a signed epoch and checkpoint appear. Schedule the
   independent checkpoint copy and verifier. Alert on failed jobs, stale checkpoints,
   pending records older than the configured threshold and held chains.

The legacy `AuditEntry` trail is exported as raw batches; its hashes are not
converted to the new format. Keep existing evidence and storage versions throughout
the upgrade.
