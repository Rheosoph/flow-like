---
title: Audit trail storage
description: Configure the audit worker, the audit keys and the immutable audit bucket of a self-hosted deployment
---

Besides the database, the audit trail uses an audit worker that seals, signs,
archives and prunes, an audit key that signs, and an audit bucket that keeps the
monthly archives and daily heads. What the trail records, its retention settings and
how verification works are described in
[Platform administration](/dev/platform-administration/#audit-trail).

Both the key and the bucket are optional, and leaving them out never deletes evidence
that was not archived. Without an audit bucket nothing is archived and evidence stays
in the database; activity records are still deleted when their window ends. Without
an audit key records are sealed but not signed, and nothing is pruned.

## Who holds what

| Component | Holds | Can |
| --- | --- | --- |
| API | Entry key | Insert records |
| Audit worker | Entry key, audit key, audit bucket credentials | Seal, sign, write heads and archives, prune |
| Audit bucket | Monthly archives, daily heads, the one-time legacy export | Keep every object immutable for its lock period |

Give the audit key and the bucket credentials only to the worker, and let the bucket
credentials write and read but not delete or bypass the lock. A compromised API can
then neither touch the archives and heads nor re-sign rewritten history.

## Run the audit worker

| Deployment | Where the worker runs |
| --- | --- |
| Docker Compose, Kubernetes, Azure, GCP, AWS ECS API | Inside every API process, once a minute. `AUDIT_WORKER=off` (Helm: `audit.worker: false`) turns it off for a process |
| AWS Lambda API | The `audit-worker` function in `apps/backend/aws/audit-worker`, invoked every minute by EventBridge Scheduler. Its README lists the function, schedule, bucket, key and IAM resources the deployment needs. An ECS API next to it sets `AUDIT_WORKER=off` |

Any number of processes may run the worker: a lease in the database lets one work at
a time and the others skip. To keep the audit key and bucket credentials away from the
processes that serve requests, run one dedicated instance with the worker on and the
audit settings, and set `AUDIT_WORKER=off` without audit key or bucket settings on the
others.

Only one worker works at a time, and it keeps the lease between its runs, so extra
replicas stay idle and never contact the key service. A worker that stops releases its
lease within five minutes, and another takes over.

Every process, worker or not, needs the same entry key. Set the same
`AUDIT_ENTRY_KEY` everywhere, or leave it unset where all processes share
`BACKEND_KEY`, from which it is derived. A worker with a different entry key
quarantines every record the API writes and holds every chain whose seal is still
waiting for its epoch.

### Rotate the entry key

1. Set the new key as `AUDIT_ENTRY_KEY` and the old one as `AUDIT_ENTRY_KEY_PREVIOUS`
   on every process, worker included, and roll them.
2. Wait until everything written before the switch is sealed and signed:
   `seal_after_seconds` plus `epoch_interval_seconds` plus a few minutes.
3. Remove `AUDIT_ENTRY_KEY_PREVIOUS` and roll again.

Deployments that derive the entry key from `BACKEND_KEY` set `AUDIT_ENTRY_KEY`
explicitly before rotating `BACKEND_KEY`; otherwise the derived key changes with it and
the old one is gone.

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

## Environment

| Variable | Set on | Meaning |
| --- | --- | --- |
| `AUDIT_WORKER` | API | `off`, `false` or `0` stops the in-process worker. Default on |
| `AUDIT_ENTRY_KEY` | API and worker | Base64 of 32 random bytes, the same everywhere. Unset: derived from `BACKEND_KEY` |
| `AUDIT_ENTRY_KEY_PREVIOUS` | API and worker | The entry key used before a rotation, still accepted while records and seals written with it are sealed and signed (see [Rotate the entry key](#rotate-the-entry-key)) |
| `AUDIT_SIGNING_KEY` | Worker | Audit key: base64 of a PKCS#8 PEM P-256 private key. Exactly one of this and `AUDIT_KMS_KEY_ID` is set; the worker refuses to start with both |
| `AUDIT_KMS_KEY_ID` | Worker | Audit key in a key service instead: an AWS KMS key id, ARN or alias, a Cloud KMS key version (`projects/.../cryptoKeyVersions/N`) or an Azure Key Vault key (`https://<vault>/keys/<name>/<version>`) |
| `AUDIT_KMS_PROVIDER` | Worker | `aws`, `gcp` or `azure`. Unset: taken from the shape of `AUDIT_KMS_KEY_ID` |
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
| `AUDIT_BUCKET_LOCK_MODE`, `AUDIT_BUCKET_RETENTION_YEARS` | Compose object-store bootstrap | Object Lock mode (`GOVERNANCE` by default, or `COMPLIANCE`) and default retention in years (3 by default) of a new audit bucket |

The first bucket variable that is set wins, in the order `AUDIT_BUCKET`,
`AWS_AUDIT_BUCKET`, `GCP_AUDIT_BUCKET`, `AZURE_AUDIT_CONTAINER`. S3 uploads carry a
SHA-256 checksum, which Object Lock buckets require.

The worker logs the key id and its PEM public key when it gets its audit key:
`audit signing key loaded` for `AUDIT_SIGNING_KEY` at startup, `audit key-service key
connected` for `AUDIT_KMS_KEY_ID` when the worker starts. Copy that pair into
`AUDIT_VERIFYING_KEYS` of every process without the key, so they can verify epochs,
and hand it to app owners who verify exports offline.

### Keys in a key service

A key service keeps the audit key out of every process. The build needs the
matching feature of `flow-like-api`: `audit-kms-aws`, `audit-kms-gcp` or
`audit-kms-azure`. A key id for a provider the build lacks stops startup with an
error that names the feature.

| Provider | Key | Worker permissions |
| --- | --- | --- |
| AWS KMS | Asymmetric, key spec `ECC_NIST_P256`, usage `SIGN_VERIFY` | `kms:Sign`, `kms:GetPublicKey` |
| Cloud KMS | Algorithm `EC_SIGN_P256_SHA256` | Sign and view the public key, for example `roles/cloudkms.signerVerifier` on the key |
| Azure Key Vault | EC key on curve P-256 | Key permissions `get` and `sign` |

Every provider signs the 32-byte digest with ECDSA P-256 and SHA-256, the same
signature a local key produces, and the worker checks each signature against the
public key before it stores it. Asymmetric keys do not rotate in place: a new key is a
new key id.

### Signing cost

Each signature is one billed request to the key service. The number of signatures
does not grow with traffic, the number of apps or the number of records:

| Signed | When | At most |
| --- | --- | --- |
| Epoch | Once the oldest unsigned seal is `retention.epoch_interval_seconds` old (default 300), or earlier when 2,000 seals wait | 12 an hour, plus one per 2,000 seals under heavy load |
| Prune watermarks | One signature per prune run covers every chain it prunes; a run signs at most once an hour | 1 an hour |
| Archive manifest | Once per month | 1 a month |

A worker process also refuses to sign more than 120 times in a rolling hour, so a
fault cannot turn into a bill. API processes never contact the key service: they
verify with public keys only. Set `AUDIT_KID` and put that key id into
`AUDIT_VERIFYING_KEYS`, and the worker skips reading the public key from the service
at startup as well. Failed requests are retried once.

Seals wait for their epoch under a MAC made with the entry key, so a longer
`epoch_interval_seconds` lowers the cost without leaving sealed records unprotected;
it only delays when records are covered by the audit key.

With prices checked in September 2026 (ECDSA P-256), a month of routine signing
(at most about 9,000 signatures) costs about $1.14 with AWS KMS, $0.09 with a Cloud KMS
software key, $2.64 with a Cloud KMS HSM key, $0.14 with Azure Key Vault Standard and
$5.14 with Azure Key Vault Premium. The monthly key fee is most of it; signing costs
$0.15 per 10,000 requests ($0.03 for Cloud KMS software keys). AWS KMS keys already
live in HSMs, so a dedicated HSM cluster is not needed.

## Create the audit bucket

Whoever creates the bucket makes two choices the application does not: its
immutability and its storage tier. Heads live under `heads/`, one small object per
day. Archives live under `archive/YYYY/MM/`, one manifest and one or more parts per
month. The legacy export is `legacy/audit-entry.jsonl.zst`.

| Provider | Immutability | Long-term tier | Note |
| --- | --- | --- | --- |
| AWS S3 | Object Lock, enabled when the bucket is created | Lifecycle transition of `archive/` to Glacier Deep Archive from day 0 | The lock survives the transition. Restores take hours. The 180-day minimum storage charge does not matter at multi-year retention |
| Azure Blob | Container or version-level immutability policy | Lifecycle management to the Archive tier | Immutability applies in every tier, and rehydration is allowed under a locked policy |
| Google Cloud Storage | Retention policy with Bucket Lock | Bucket default storage class Archive | Reads stay fast in Archive class but cost more |
| RustFS (bundled) | Object Lock, enabled when the bucket is created | One tier | The bootstrap creates the bucket with Object Lock on |

Keep `heads/` in the standard tier so heads stay cheap to read. Only twelve archive
objects a year, plus parts for very large months, are written, so per-object and
per-request charges of archive tiers stay negligible. Abort incomplete multipart
uploads after a few days, and expire objects once their lock has passed if nothing
else must keep them.

**Lock length.** The lock runs from the moment an object is written, and each
manifest states `retain_until`: 31 December of `archive_years_after_year_end` years
after the month. A month is written a few days after it closes, so a default retention
of `archive_years_after_year_end` plus one year covers every `retain_until`: four
years (1461 days) with the default of 3. Three years, the Compose and Helm default,
leaves the archives of January to November without a lock for their last one to
eleven months.

**Lock mode.** Governance mode keeps a privileged path to delete an archive when a
data protection authority or a court orders it. Compliance mode makes deletion
impossible until the lock expires; regulated profiles such as SEC Rule 17a-4 require
it. To keep records of an incident or dispute beyond their retention, set an Object
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

### Kubernetes

The Helm chart reads the audit settings from `audit` in `values.yaml`: `worker`,
`bucket`, `lockMode`, `retentionYears`, `endpoint` (external storage only),
`kmsKeyArn`, `kmsKeyId`, `kmsProvider`, `kid`, `verifyingKeys` and `existingSecret`.
The secret holds `AUDIT_BUCKET_ACCESS_KEY_ID`, `AUDIT_BUCKET_SECRET_ACCESS_KEY` and
optionally `AUDIT_SIGNING_KEY`; it is required with bundled RustFS, and
`scripts/setup-config.py` generates it with a dedicated audit key. With bundled RustFS
the object-store init job creates the bucket with Object Lock, the default retention
from `lockMode` and `retentionYears`, and the write-only identity.

The object-store conformance job writes an object under a one-day governance lock and
checks that deleting that object version fails even for the root identity, so a
storage regression cannot silently make the archive writable. The bundled RustFS
image is pinned to 1.0.0-rc.5; releases before 1.0.0-rc.1 did not enforce Object Lock
(CVE-2026-73288).

## Upgrade an existing deployment

1. Apply the `20260919120001_audit_seals` migration before starting the new API:
   `prisma/migrations/` for PostgreSQL, `prisma/migrations-dsql/` for Aurora DSQL. It
   creates the new audit tables and leaves `AuditEntry` in place.
2. Set the same `AUDIT_ENTRY_KEY` on every process, or make sure they share
   `BACKEND_KEY`.
3. Give the worker its audit key and its bucket. Copy the logged public key into
   `AUDIT_VERIFYING_KEYS` of the other processes.
4. Watch **Admin > Logs**: a new epoch appears within `epoch_interval_seconds` plus a
   minute of the first sealed records, the oldest pending record stays younger than
   `seal_after_seconds` plus a minute, and `legacy_entries` falls to zero once the old
   entries are exported to `legacy/audit-entry.jsonl.zst`.

The previous hash chain is not converted. Its rows are exported once, raw, and then
deleted; the export is kept by the bucket like any archive. Without an audit bucket
the old rows stay in the database.
