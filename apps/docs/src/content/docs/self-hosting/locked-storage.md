---
title: Locked storage
description: Configure write-once, versioned file storage on an Object Lock or immutable bucket for revision-safe records
---

Locked storage keeps files that must not change after they are filed: invoices,
contracts, signed reports. Every version is one immutable object in a dedicated
bucket with Object Lock or immutability enabled, and nothing can delete it before
its retention ends. The feature is off until a locked bucket is configured, and it
is available to online apps only.

The [audit trail](/self-hosting/audit-trail/) also writes to an Object Lock bucket,
with the opposite access model: there a separate worker holds a write-only identity
and the API has no access. Here the API is the only process with credentials for
the bucket and decides what may be written and when.

## How locked storage works

Each app has its own locked files, in one of two scopes: `project` files belong to
the app, `user` files to one member. A file is an item with a name and a virtual
folder; each upload adds a version. A version is stored under a key that is never
reused or overwritten:

```text
apps/{app_id}/project/{item_id}/{version_id}
apps/{app_id}/users/{user_id}/{item_id}/{version_id}
```

The database is the index and records every lock fact of a version: size, content
type, SHA-256 and whether the store verified it, the provider's version id (S3
`VersionId`, GCS generation, Azure version id), lock mode, retain-until date, legal
hold, enforcement, uploader and lock time. Version numbers are assigned at commit,
so abandoned uploads leave no gaps.

Nobody but the API process holds credentials for the bucket. Browsers, desktop
clients and executors receive presigned URLs for one exact object, never a prefix.
An upload URL must be used within an hour; the store checks expiry when the PUT
starts, so a large upload that started in time completes. Download URLs last 10
minutes. Executors call the same actions under
`/api/v1/execution/apps/{app_id}/locked/` with their execution token.

### Upload flow

1. The client requests an upload ticket with the file's size and SHA-256. The API
   checks the payer's storage quota, records a pending version and returns a
   presigned PUT together with the exact headers to send.
2. The client PUTs the bytes with exactly those headers. On S3 the signed
   `x-amz-checksum-sha256` makes the store reject bytes that do not match.
3. The client commits. The API inspects the object's size, checksum and version id,
   applies the lock (object mode) or verifies the bucket rule (bucket mode), assigns
   the version number and charges the bytes. Commit is idempotent.

A version is revision-safe from commit on. R2, GCS and Azure have no native SHA-256
check on upload, so the API streams the object through itself at commit, up to
`LOCKED_BUCKET_VERIFY_MAX_BYTES`. A larger version is stored with
`checksum_verified = false`.

## Enforcement modes

| Mode | What locks a version | Bucket setup | Retention per upload | Extension and legal hold |
| --- | --- | --- | --- | --- |
| `object` (default) | The API sets retention on the object version at commit, after verifying the upload | Object Lock or immutability support, but **no** default retention rule | Chosen at upload within the configured bounds | Retention can be extended; legal hold where the provider supports it |
| `bucket` | A bucket-level default rule or policy locks every object when it is written | Default retention rule or policy covering `apps/` | Ignored; the bucket rule decides | No extension; legal hold where the provider supports it |

Use object mode wherever the provider can lock individual versions. A default rule
on such a bucket would lock never-committed uploads at write, so they could no
longer be aborted; Docker Compose and Kubernetes refuse that combination.
Bucket mode is required for Cloudflare R2, which cannot lock individual objects. In
bucket mode, set `LOCKED_BUCKET_RETENTION_DAYS` to the rule's retention: the API
records it as each version's retain-until date.

Retention runs from the moment the lock is applied: commit in object mode, the write
in bucket mode.

### Lock mode

`LOCKED_BUCKET_LOCK_MODE` selects how strict the lock is:

- `COMPLIANCE` (default): nobody can shorten or remove the lock before it expires,
  including the storage account's root identity. Retention can only be extended.
- `GOVERNANCE`: an identity with a bypass permission, such as
  `s3:BypassGovernanceRetention`, can shorten or remove it. This keeps a privileged
  path for a deletion ordered by a court or a data protection authority. The API
  identity never gets that permission.

GCS and Azure call these locked and unlocked retention. Azure has no separate
bypass permission, so use `COMPLIANCE` there; see
[Azure Blob Storage](#azure-blob-storage). Cloudflare R2 has no equivalent of either
mode; see [Cloudflare R2](#cloudflare-r2).

:::caution
A `COMPLIANCE` lock cannot be undone, and the storage stays billed until it ends.
Test the setup with a short retention in a disposable bucket before filing real
documents.
:::

## Environment

These variables are read by the API process only. Executors, the compiler and
workers never receive them. An unset or empty bucket disables the feature; the
status endpoint then reports `enabled: false` and the other actions return 503.

Instead of variables, the settings can live in one `LOCKED_STORAGE` entry of the
API's secret store: a JSON object keyed by the variable names below, for example
`{"LOCKED_BUCKET": "flow-like-locked", "LOCKED_BUCKET_RETENTION_DAYS": "2920"}`.
Set variables override its values. The AWS Lambda deployment uses it, because Lambda
caps all environment variables at 4 KB.

| Variable | Default | Meaning |
| --- | --- | --- |
| `LOCKED_BUCKET`, `AWS_LOCKED_BUCKET`, `GCP_LOCKED_BUCKET`, `AZURE_LOCKED_CONTAINER` | unset | Bucket or container name |
| `LOCKED_BUCKET_PROVIDER` | From the variable that named the bucket; for `LOCKED_BUCKET`, `STORAGE_PROVIDER`, else `s3` | `s3` (also `aws`, `minio`, `rustfs`), `r2`, `gcs` (also `gcp`) or `azure` |
| `LOCKED_BUCKET_ENFORCEMENT` | `object`; `bucket` for `r2` | `object` or `bucket`; `object` with `r2` stops startup |
| `LOCKED_BUCKET_LOCK_MODE` | `COMPLIANCE` | `COMPLIANCE` or `GOVERNANCE` |
| `LOCKED_BUCKET_RETENTION_DAYS` | `3650` | Retention of a new version when the upload names none. In bucket mode, the bucket rule's retention |
| `LOCKED_BUCKET_MIN_RETENTION_DAYS` | `1`; equal to the default in bucket mode | Shortest retention an upload may choose. Shorter requests are raised to it |
| `LOCKED_BUCKET_MAX_RETENTION_DAYS` | `36500` | Longest retention an upload may choose. Longer requests are rejected |
| `LOCKED_BUCKET_MAX_FILE_BYTES` | `5368709120` (5 GiB) | Size limit of one version |
| `LOCKED_BUCKET_VERIFY_MAX_BYTES` | `536870912` (512 MiB) | Largest version the API hashes itself at commit on R2, GCS and Azure |
| `LOCKED_BUCKET_ENDPOINT` | `S3_PUBLIC_ENDPOINT`, then `S3_INTERNAL_ENDPOINT`, then `AWS_ENDPOINT` | S3-compatible endpoint used for signing and for the API's own calls |
| `LOCKED_BUCKET_REGION` | `AWS_REGION`, else `us-east-1` | Region used for signing |
| `LOCKED_BUCKET_ACCESS_KEY_ID`, `LOCKED_BUCKET_SECRET_ACCESS_KEY` | AWS credential chain, `AWS_ACCESS_KEY_ID` | Dedicated identity for the locked bucket. `_FILE` variants are supported |
| `LOCKED_BUCKET_PATH_STYLE` | `AWS_USE_PATH_STYLE` | Path-style addressing |
| `LOCKED_BUCKET_KMS_KEY_ARN` | unset | SSE-KMS key signed into every PUT (AWS S3) |
| `AZURE_LOCKED_STORAGE_ACCOUNT_NAME` | `AZURE_STORAGE_ACCOUNT_NAME` | Separate storage account without hierarchical namespace (see [Azure Blob Storage](#azure-blob-storage)) |
| `AZURE_LOCKED_STORAGE_ACCOUNT_KEY` | `AZURE_STORAGE_ACCOUNT_KEY` when the account is shared | Account key for signing URLs on the dedicated account; leave unset to sign with the managed identity. Retention and legal holds always use the managed identity |

The first bucket variable that is set wins, in the order `AWS_LOCKED_BUCKET`,
`GCP_LOCKED_BUCKET`, `AZURE_LOCKED_CONTAINER`, `LOCKED_BUCKET`. The retention
settings must satisfy 1 ≤ minimum ≤ default ≤ maximum ≤ 36,500 days; otherwise, and
for an unknown provider, enforcement or lock mode, the API refuses to start with an
error that names the variable.

Presigned URLs are bound to the signing endpoint. Browsers, desktop clients and
execution gateways must reach `LOCKED_BUCKET_ENDPOINT` under exactly that host,
port and path, as described in
[Keep the signing origin reachable](/self-hosting/docker-compose/storage/#keep-the-signing-origin-reachable).

## Docker Compose

Set the bucket name in `.env`; an empty value keeps the feature disabled:

```dotenv
LOCKED_BUCKET=flow-like-locked
LOCKED_BUCKET_ENFORCEMENT=object
LOCKED_BUCKET_LOCK_MODE=COMPLIANCE
LOCKED_BUCKET_RETENTION_DAYS=3650
LOCKED_BUCKET_MIN_RETENTION_DAYS=1
LOCKED_BUCKET_MAX_FILE_BYTES=5368709120
```

Then run `python3 scripts/preflight.py` and `python3 scripts/up.py` as usual.
Preflight checks that the name is a lowercase DNS name without periods, which the
object gateway and the execution manager match on, and that it differs from
`META_BUCKET`, `CONTENT_BUCKET`, `LOG_BUCKET` and `AUDIT_BUCKET`.

Compose passes exactly these six variables. Others, such as
`LOCKED_BUCKET_ENDPOINT` or a dedicated identity for an external store, need a
reviewed override of the `api` service.

With bundled RustFS, `object-store-init` sets the bucket up before the API starts:

- It creates the bucket with Object Lock enabled. With enforcement `object` it adds
  no default rule; with `bucket` it adds a default rule from
  `LOCKED_BUCKET_LOCK_MODE` (default `COMPLIANCE`) and `LOCKED_BUCKET_RETENTION_DAYS`
  (default 3650).
- It refuses an existing bucket without Object Lock. Choose a new name or recreate
  the bucket with Object Lock enabled.
- It applies CORS for the signed upload headers, with the browser origins of the
  other buckets.
- On every run it sets two lifecycle rules: abort incomplete multipart uploads
  after one day, and expire noncurrent versions one day after they became
  noncurrent, together with expired delete markers. See
  [Versioning and cost](#versioning-and-cost).
- It attaches the dedicated RustFS policy `flow-like-locked-v1` to the API user
  only. The STS issuer and the executor sessions it issues get no access to the
  bucket.

The bootstrap never changes the lock rule of an existing bucket, so switching
enforcement on an existing bucket is not automatic. The bootstrap validates the
bucket against the configured mode and stops for review when they disagree: a
default rule in object mode, or a missing or weaker rule in bucket mode.

Disabling the feature again is not silent. Once `flow-like-locked-v1` is attached,
unsetting `LOCKED_BUCKET` makes `object-store-init` stop with
`IAM user policy or group drift (flow-like-locked-v1)` until a storage
administrator detaches that policy from the API user. The bucket and its retained
objects stay.

The object gateway forwards the locked bucket only when `LOCKED_BUCKET` is set, and
the execution manager adds it to each run's egress allowlist. Executors still reach
it only through presigned URLs: the per-run gateway forwards object requests only
and always denies the S3 `?object-lock`, `?retention` and `?legal-hold`
subresources, so only the API can change a lock.

With `OBJECT_STORE_MODE=external`, nothing creates the bucket. Set it up as
described under [Create the bucket](#create-the-bucket).

## Kubernetes

The Helm chart takes the same settings from one block:

```yaml
lockedStorage:
  bucket: ""                 # empty disables locked storage
  enforcement: object        # or bucket
  lockMode: COMPLIANCE       # or GOVERNANCE
  retentionDays: 3650
  minRetentionDays: 1
  maxFileBytes: 5368709120
```

The values are rendered into the API, the RustFS bootstrap job, the object gateway
and the execution manager only when `bucket` is set. Chart validation rejects a
name that is not a lowercase DNS name without periods, a name shared with the
metadata, content, log or audit bucket, and retention values out of range.

With bundled RustFS, the bootstrap job creates the bucket and policy exactly as in
Docker Compose, with CORS from `api.corsAllowedOrigins`. With `rustfs.enabled=false`,
create the bucket yourself and grant the API's storage identity the permissions
listed for [AWS S3](#aws-s3). The block carries no separate credentials.

## Create the bucket

Use a bucket that holds nothing else. Every provider needs:

- Object Lock or immutability support enabled, in the mode chosen above.
- No anonymous access, TLS-only access and no bucket policy that grants other
  principals access.
- CORS for the web and desktop origins that also reach the content bucket, methods
  `GET`, `PUT` and `HEAD`, and the headers the API signs into uploads.
- An API identity that can write, read, lock and delete single object versions, and
  no permission to bypass or reconfigure the lock.

The `flow-like-deployment` repository has a Terraform module for AWS, GCP and Azure.

### AWS S3

With the Terraform in the deployment repository, each AWS root (`deployments/aws/dev`
and the SaaS roots) creates the bucket from `locked-storage.tf`, controlled by
`locked_storage_enabled`, which is on by default. The root attaches the bucket's policy
to the API role only and writes the settings to the API's `LOCKED_STORAGE` SSM
parameter, so the Lambda environment gains no variables. It is a separate bucket,
and the content bucket's versioning is not touched. The locked bucket is versioned only because Object
Lock requires it. Each version has its own key, and a lifecycle rule expires
noncurrent versions after a day. Those can only be duplicate uploads of a key, and
retained versions stay locked regardless.

To create the bucket by hand instead, enable Object Lock when you create it; it also
enables versioning. AWS can
enable Object Lock on an existing versioned bucket, but that does not protect
objects already stored, so start with a new bucket:

```bash
aws s3api create-bucket --bucket flow-like-locked --region eu-central-1 \
  --create-bucket-configuration LocationConstraint=eu-central-1 \
  --object-lock-enabled-for-bucket
aws s3api put-public-access-block --bucket flow-like-locked \
  --public-access-block-configuration \
  BlockPublicAcls=true,IgnorePublicAcls=true,BlockPublicPolicy=true,RestrictPublicBuckets=true
aws s3api put-bucket-cors --bucket flow-like-locked --cors-configuration file://cors.json
```

For object mode, add no default retention. For bucket mode, add the rule:

```bash
aws s3api put-object-lock-configuration --bucket flow-like-locked \
  --object-lock-configuration \
  '{"ObjectLockEnabled":"Enabled","Rule":{"DefaultRetention":{"Mode":"COMPLIANCE","Days":3650}}}'
```

Deny requests without TLS with a bucket policy condition on `aws:SecureTransport`
set to `false`. `cors.json` lists the headers the API may sign into a PUT:

```json
{
  "CORSRules": [
    {
      "AllowedOrigins": ["https://app.example.com"],
      "AllowedMethods": ["GET", "PUT", "HEAD"],
      "AllowedHeaders": [
        "content-type",
        "x-amz-checksum-sha256",
        "x-amz-sdk-checksum-algorithm",
        "x-amz-object-lock-mode",
        "x-amz-object-lock-retain-until-date",
        "x-amz-server-side-encryption",
        "x-amz-server-side-encryption-aws-kms-key-id"
      ],
      "ExposeHeaders": ["ETag", "x-amz-version-id"],
      "MaxAgeSeconds": 3600
    }
  ]
}
```

The API identity needs object access on `bucket/*`, list and Object Lock
configuration read access on the bucket and, for SSE-KMS, the key named by
`LOCKED_BUCKET_KMS_KEY_ARN`. `s3:GetObject` also covers `HeadObject`:

```json
{
  "Version": "2012-10-17",
  "Statement": [
    {
      "Sid": "LockedObjects",
      "Effect": "Allow",
      "Action": [
        "s3:PutObject",
        "s3:GetObject",
        "s3:GetObjectVersion",
        "s3:PutObjectRetention",
        "s3:GetObjectRetention",
        "s3:PutObjectLegalHold",
        "s3:GetObjectLegalHold",
        "s3:DeleteObject",
        "s3:DeleteObjectVersion"
      ],
      "Resource": "arn:aws:s3:::flow-like-locked/*"
    },
    {
      "Sid": "LockedBucketConfiguration",
      "Effect": "Allow",
      "Action": ["s3:ListBucket", "s3:GetBucketObjectLockConfiguration"],
      "Resource": "arn:aws:s3:::flow-like-locked"
    },
    {
      "Sid": "LockedBucketKey",
      "Effect": "Allow",
      "Action": ["kms:GenerateDataKey", "kms:Decrypt"],
      "Resource": "arn:aws:kms:eu-central-1:123456789012:key/1234abcd-12ab-34cd-56ef-1234567890ab"
    },
    {
      "Sid": "NeverBypassTheLock",
      "Effect": "Deny",
      "Action": ["s3:BypassGovernanceRetention", "s3:PutBucketObjectLockConfiguration"],
      "Resource": ["arn:aws:s3:::flow-like-locked", "arn:aws:s3:::flow-like-locked/*"]
    }
  ]
}
```

The delete permissions are for never-committed uploads and for purges after
retention. Object Lock refuses deleting a retained version whatever the policy says.
Never grant `s3:BypassGovernanceRetention`, which would let the API remove
`GOVERNANCE` locks, or `s3:PutBucketObjectLockConfiguration`, which would let it
change the bucket's rule. The explicit deny keeps a broader policy attached
elsewhere from granting them.

`s3:ListBucket` makes `HeadObject` on a key that was never uploaded return 404
instead of 403, so commit and the abandoned-upload sweep can tell a missing upload
from a permission error. The `flow-like-deployment` repository provides the module
`modules/aws/locked-bucket`. Its output `api_policy_arn` is the reference policy to
attach to the API role.

Do not add the locked bucket to the content bucket's S3 notifications for the file
tracker. The API accounts locked bytes itself at commit and purge; the tracker
would count them twice.

### MinIO, RustFS and Ceph

Create the bucket with Object Lock, for example
`mc mb --with-lock local/flow-like-locked` on MinIO or an S3 `CreateBucket` with
`ObjectLockEnabledForBucket` on RustFS and Ceph
(`aws s3api create-bucket --object-lock-enabled-for-bucket --endpoint-url ...`). The
bundled Compose and Helm bootstrap does this for RustFS. For bucket mode, add a
default rule, for example
`mc retention set --default COMPLIANCE 3650d local/flow-like-locked`.

Grant the API user a policy with the same actions as on AWS S3, without the KMS
statement; RustFS rejects AWS SSE-KMS. Qualify that the exact store version
enforces the lock: write a test object under a one-day `GOVERNANCE` lock and check
that deleting that version fails.

### Cloudflare R2

R2 supports bucket mode only. Set `LOCKED_BUCKET_PROVIDER=r2`; enforcement then
defaults to `bucket`. Add a bucket lock rule on the prefix `apps/` with the
retention age, in the dashboard, through the API, or with Wrangler, for example:

```bash
wrangler r2 bucket lock add flow-like-locked --name locked --prefix apps/ --retention-days 3650
```

R2 has no per-object retention and no legal hold, so retention extension and legal
hold are unavailable. It also has no compliance mode: an account member allowed to
edit the bucket's settings can remove the rule. Restrict those account roles.

Configure an equivalent CORS policy in the dashboard or with
`wrangler r2 bucket cors set`. Create an R2 API token with **Object Read & Write**
scoped to the locked bucket and use its S3 key pair as
`LOCKED_BUCKET_ACCESS_KEY_ID` and `LOCKED_BUCKET_SECRET_ACCESS_KEY`, with
`LOCKED_BUCKET_ENDPOINT=https://<account_id>.r2.cloudflarestorage.com` and
`LOCKED_BUCKET_REGION=auto`. There is no
Terraform module; the deployment repository does not use the Cloudflare provider.

### Google Cloud Storage

For object mode, create the bucket with Object Retention Lock. The API sets the
retention on each object at commit, `Locked` for `COMPLIANCE` and `Unlocked` for
`GOVERNANCE`:

```bash
gcloud storage buckets create gs://flow-like-locked --location=europe-west3 \
  --enable-per-object-retention \
  --uniform-bucket-level-access \
  --public-access-prevention
```

Object Retention Lock cannot be disabled once enabled and places a lien on project
deletion, so choose the mode before creating the bucket. For bucket mode, use a
Bucket Lock retention policy instead, and lock it only after testing; locking is
irreversible, and the Terraform module requires it with `COMPLIANCE`:

```bash
gcloud storage buckets update gs://flow-like-locked --retention-period=3650d
gcloud storage buckets update gs://flow-like-locked --lock-retention-period
```

Keep Object Versioning off in both modes: a retention policy cannot coexist with
it, and with versioning on a retained live object can still be made noncurrent. The
API identifies versions by their generation. Turn soft delete off, for example with
`gcloud storage buckets update gs://flow-like-locked --clear-soft-delete`; otherwise
purged objects stay billed for the soft delete window after the API has credited
them back. Set CORS with methods `GET`, `PUT` and `HEAD` and the headers listed in
the upload ticket's `headers`.

The API service account needs a custom role on the bucket:

```bash
gcloud iam roles create flowLikeLockedStorage --project=PROJECT_ID \
  --permissions=storage.objects.create,storage.objects.get,storage.objects.update,storage.objects.delete,storage.objects.setRetention,storage.buckets.get
gcloud storage buckets add-iam-policy-binding gs://flow-like-locked \
  --member=serviceAccount:api@PROJECT_ID.iam.gserviceaccount.com \
  --role=projects/PROJECT_ID/roles/flowLikeLockedStorage
```

Never grant `storage.objects.overrideUnlockedRetention`, which shortens unlocked
retention, or `storage.buckets.update`, which changes the bucket's policy. That
rules out `roles/storage.objectAdmin`, while `roles/storage.objectUser` lacks
`setRetention`. Without a service-account key file, the API signs URLs as itself
through `iam.serviceAccounts.signBlob`, which needs
`roles/iam.serviceAccountTokenCreator` on its own service account. The
`flow-like-deployment` module is `modules/gcp/locked-bucket`.

The `deployments/gcp/dev` root creates the bucket when `locked_storage_enabled = true`,
the default. It is named `<project>-<environment>-locked` unless
`locked_storage_bucket_name` overrides it, uses the data CMEK key and admits the
same single frontend origin as the content bucket. The custom role is bound to the
API service account alone once `deploy_workloads` is on, like every other
data-plane grant there. Only the API service receives `GCP_LOCKED_BUCKET`,
`LOCKED_BUCKET_PROVIDER`, `LOCKED_BUCKET_ENFORCEMENT`, `LOCKED_BUCKET_LOCK_MODE` and
`LOCKED_BUCKET_RETENTION_DAYS`; executors and workers get none of them.
`locked_storage_enforcement`, `locked_storage_lock_mode` and
`locked_storage_retention_days` pass through to the module, and bucket mode with
`COMPLIANCE` locks the retention policy irreversibly. The bucket has
`prevent_destroy`, so switching the variable off later fails the plan; remove
`module.locked_storage` from the state instead, which keeps the bucket and its
records.

### Azure Blob Storage

Use a separate StorageV2 account without hierarchical namespace, with blob
versioning on, and name it in `AZURE_LOCKED_STORAGE_ACCOUNT_NAME`. Version-level
immutability needs blob versioning, which Data Lake Gen2 (HNS) accounts lack.
Require HTTPS and TLS 1.2, disable anonymous blob access, and keep blob soft delete
off so a purge frees the bytes the API credits back. Browsers, desktop clients and
runs use the signed URLs against its blob endpoint, so give the account the same
reachability as the data account: public network access where the data account has
it, or a private endpoint where clients reach the data account privately.

Enable version-level immutability support on the container
(`immutableStorageWithVersioning`). It cannot be removed, the container can then
only be deleted when empty, and it rules out point-in-time restore, last-access
tracking, SFTP and NFS on the account.

- Object mode: no default policy. The API sets a blob immutability policy on each
  version at commit, `Locked` for `COMPLIANCE` and `Unlocked` for `GOVERNANCE`.
- Bucket mode: a time-based retention policy on the container, which every new
  version inherits as its default. Lock it after testing; with `COMPLIANCE` the
  Terraform module requires the lock. A locked policy can only be extended.

Use `COMPLIANCE` on Azure: there is no separate bypass permission, so the API
identity itself could shorten or delete an unlocked `GOVERNANCE` version policy.

The API's managed identity needs a custom role on the container and the built-in
**Storage Blob Delegator** role on the account, because user delegation keys for
the SAS URLs are evaluated only at account scope. Set and Delete Blob Immutability
Policy need the `runAsSuperUser` data action, which Storage Blob Data Contributor
lacks; Set Blob Legal Hold needs `containers/write`:

```json
{
  "Name": "Flow-Like locked storage API",
  "Actions": [
    "Microsoft.Storage/storageAccounts/blobServices/containers/read",
    "Microsoft.Storage/storageAccounts/blobServices/containers/write",
    "Microsoft.Storage/storageAccounts/blobServices/containers/immutabilityPolicies/read"
  ],
  "DataActions": [
    "Microsoft.Storage/storageAccounts/blobServices/containers/blobs/read",
    "Microsoft.Storage/storageAccounts/blobServices/containers/blobs/write",
    "Microsoft.Storage/storageAccounts/blobServices/containers/blobs/delete",
    "Microsoft.Storage/storageAccounts/blobServices/containers/blobs/deleteBlobVersion/action",
    "Microsoft.Storage/storageAccounts/blobServices/containers/blobs/immutableStorage/runAsSuperUser/action"
  ],
  "AssignableScopes": ["/subscriptions/<sub>/resourceGroups/<rg>"]
}
```

Storage Blob Data Owner also works but is broader than needed: its `containers/*`
would let the API delete or rewrite an unlocked container policy and set or clear
container legal holds.

Presigned PUTs send `x-ms-blob-type` and `content-type`, so where browsers reach
the account directly its Blob service CORS rule allows those headers, methods
`GET`, `PUT` and `HEAD`, and exposes `ETag` and `x-ms-version-id`. The
`flow-like-deployment` module `modules/azure/locked-storage` sets all of this up.

The `deployments/azure/dev` root creates the account when
`locked_storage_enabled = true`, the default, named
`<compact prefix><unique_suffix>lock` unless `locked_storage_account_name`
overrides it. It mirrors the data account: public network access off, one blob
private endpoint in the shared private-endpoint subnet registered in the
`privatelink.blob` zone, no CORS rule, and the data account's redundancy. Runs and
the API reach it inside the VNet; browser and desktop access needs the same API or
edge proxy or private connectivity as the data account. The custom role and
Storage Blob Delegator are assigned to the API managed identity alone once
`deploy_workloads` is on. Only the API app receives `AZURE_LOCKED_CONTAINER`,
`AZURE_LOCKED_STORAGE_ACCOUNT_NAME`, `LOCKED_BUCKET_PROVIDER`,
`LOCKED_BUCKET_ENFORCEMENT`, `LOCKED_BUCKET_LOCK_MODE` and
`LOCKED_BUCKET_RETENTION_DAYS`, next to the `AZURE_CLIENT_ID` it already has. As on
GCP, switching the variable off later fails the plan because of
`prevent_destroy`; remove `module.locked_storage` from the state instead.

## Freeing storage

Locked bytes count against the storage quota of whoever pays for the app, like any
other upload. When the quota is full, the upload ticket is refused with the same
capacity error as ordinary file uploads, before any bytes are written.

Nothing can be deleted while it is retained. After a version's retention has ended,
and while it has no legal hold, owners and admins purge it:

- in the app's file browser,
- with the node **Delete Expired Locked File**, or
- with the API action `purge`, which purges every expired version of an item when no
  version is named.

Purging deletes the object version, keeps its database record as `purged`
evidence with who purged it, when, and the SHA-256, and credits the bytes back. A
purge of an item is refused with 409, naming the earliest unlock date, while any of
its versions is still retained.

Without a purge, a version stays stored and keeps counting against the quota after
its retention ended. To have it deleted automatically instead, pick **Delete
automatically when retention ends** at upload (`delete_after_retention` in the API,
**Delete After Retention** on the upload nodes), or switch it on or off later in the
version's details, with the node **Set Locked File Auto Delete** or the API action
`auto-delete`. Either way the retention itself stays as
it is. Only those who may purge the file can choose it: the owner for `user` files, an
admin for `project` files. Switching it off also cancels a deletion that is due. The maintenance
sweep then purges the version once its retention has ended and it has no legal hold,
the same way a manual purge does: the `purged` record names `system:retention` as the
purger, the audit trail records `locked_file.version.purge` with a system actor, and
the bytes are credited back. A legal hold postpones the deletion until it is lifted,
and extending the retention moves it.

There is no early deletion and no bypass. To free space sooner, choose a shorter
retention at upload, within `LOCKED_BUCKET_MIN_RETENTION_DAYS` and
`LOCKED_BUCKET_MAX_RETENTION_DAYS`. In bucket mode that choice is ignored and the
bucket rule decides.

## Deletion and cleanup

Locked records outlive the app. The database rows have no foreign keys and are kept
when an app or the uploading user account is deleted, and the objects stay at least
until their retention ends. The API never deletes a retained object.

An upload counts as abandoned 24 hours after its signed URL expired. The grace covers a
PUT that started in time but is still streaming a large file. The sweep runs in the
scheduled `cache_cleanup` maintenance job and, on long-running API servers, also hourly
in process. Each pass settles up to 200 uploads. Never-committed uploads are cleaned up
differently per mode:

- Object mode: the object is not locked yet, so `abort` and the maintenance sweep
  delete it.
- Bucket mode: the bucket rule locked the object when it was written. The sweep
  therefore commits an expired pending upload whose object exists, because the bytes
  are stored and must be charged.

The same job and hourly loop purge the versions uploaded with automatic deletion
whose retention has ended, up to 1,000 per pass (see
[Freeing storage](#freeing-storage)). A version the store still refuses to delete is
logged and retried on the next pass.

Abandoned uploads are left to the sweep. Do not add an age-based rule that deletes
or expires current objects: it would also delete committed versions as soon as
their retention ends, without a `purged` record and without crediting the bytes,
and on a versioned S3 bucket an expiration rule only adds delete markers. The only
lifecycle rules the setup uses are the ones in
[Versioning and cost](#versioning-and-cost), which touch noncurrent versions alone.

### Versioning and cost

Locked storage turns on versioning only on its own dedicated bucket or account and
never changes the versioning of the content, metadata or log buckets. S3 Object Lock
and Azure version-level immutability require it; the GCS bucket runs without it. It costs
almost nothing here because every version has its own object key, written once,
and every delete names the exact version (`versionId` on S3, `versionid` on Azure)
rather than leaving the bytes behind as a billed noncurrent version.

Two things can still create one: the client repeating the PUT of a pending upload,
or a plain delete. One lifecycle rule reclaims those duplicates. Lifecycle never
removes a version while it is retained or on legal hold, and committed versions are
current versions, so it shortens no retention:

| Store | Rule |
| --- | --- |
| RustFS (bundled Compose and Helm) | `NoncurrentVersionExpiration` after 1 day, plus expired delete markers |
| AWS S3 (`modules/aws/locked-bucket`) | The same one-day noncurrent-version rule |
| Azure (`modules/azure/locked-storage`) | Delete previous versions more than one day after they were written; blob soft delete stays off, because soft-deleted versions are billed |
| GCS (`modules/gcp/locked-bucket`) | None needed: no versioning, soft delete 0 |

## Revisionssicherheit and GoBD

:::note
This section is guidance, not legal advice. Agree retention periods and the overall
procedure with your tax advisor or auditor.
:::

German law sets these retention periods (§147 AO, §257 HGB):

| Records | Period |
| --- | --- |
| Books, records, inventories, annual financial statements, management reports, opening balance sheets | 10 years |
| Accounting vouchers (*Buchungsbelege*) | 8 years since 2025 (BEG IV), previously 10 |
| Received and sent business letters (*Handels- und Geschäftsbriefe*), other tax-relevant documents | 6 years |

The default of 3650 days corresponds to ten years, but counted from commit. The
statutory periods start at the end of the calendar year in which the document was
created, received or sent, and they do not end while the documents still matter for
a tax assessment that is open. A document committed in January therefore needs
almost eleven years; choose generous values, for example 4,020 days for ten-year
records, and set a legal hold during an open tax audit.

On every provider except R2, `COMPLIANCE` prevents anyone, including the storage
root, from shortening the lock. Revision-safe storage covers immutability only. GoBD
also requires a procedure documentation (*Verfahrensdokumentation*), an internal
control system and data access for the tax audit (§147 Abs. 6 AO).

## Check the setup

With `LOCKED_BUCKET` set, the storage conformance job (`object-store/conformance.py`
in Compose, `helm test` in Kubernetes; see
[Verify prefix authorization](/self-hosting/docker-compose/storage/#verify-prefix-authorization))
also probes the locked bucket: the issuer and runs are denied, the API can neither
delete nor bypass a retained probe version, and a presigned GET works through the
gateway. The probe uses a one-day `GOVERNANCE` lock under `conformance/`, which the
root identity removes afterwards; otherwise it stays until that lock ends.

`GET /api/v1/apps/{app_id}/locked/status` reports the configuration the API loaded,
to any app member: `enabled`, `provider`, `enforcement`, `retention_mode`, the
default, minimum and maximum retention days, `max_file_bytes`,
`supports_legal_hold` and `supports_retention_extension`.

| Action | Who may call it |
| --- | --- |
| `list`, `get`, `download` | Members with file read access |
| `upload` | Members with file write access; with `delete_after_retention`, only those who may `purge` the file |
| `commit`, `abort` | Members with file write access who uploaded the version, or admins |
| `retention` (extend only, object mode) and `legal-hold` | Admins |
| `purge` and `auto-delete` | Members with file write access: the owner for `user` files, an admin for `project` files |

`user` files require a human caller: a signed-in user, a personal access token or
an execution started by a user. API keys and app connections receive 403.

A commit that returns 412 found the object missing or its size or checksum
different from the ticket. Check that nothing between the client and the store
rewrites the request, and that the client sends the ticket's headers unchanged to
the signed origin.
