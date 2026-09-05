---
title: Storage Providers
description: Configure object storage and scoped runtime credentials for development
sidebar:
  order: 15
---

Flow-Like stores application metadata, content, and execution logs in object
storage. The API also issues credentials scoped to a user, application, or run.
Those are related but separate configuration concerns:

- `STORAGE_PROVIDER` selects the object-store protocol used by the API.
- `RUNTIME_CREDENTIALS_PROVIDER` selects how scoped credentials are created.

If `RUNTIME_CREDENTIALS_PROVIDER` is not present, the API uses
`STORAGE_PROVIDER`. Do not define either variable as an empty string: an explicit
empty value is a configuration error.

For a complete deployment configuration, use the
[Docker Compose storage guide](/self-hosting/docker-compose/storage/) or the
[Kubernetes storage guide](/self-hosting/kubernetes/storage/). This page focuses
on local development and provider-specific tests.

## Supported configurations

| Backing store | `STORAGE_PROVIDER` | Runtime credentials | Notes |
| --- | --- | --- | --- |
| Amazon S3 | `aws` | `aws` | Uses AWS STS and `RUNTIME_ROLE_ARN` for scoped credentials |
| RustFS | `aws` | `aws` | Set `S3_STS_PROVIDER=rustfs`; uses a private STS endpoint and a dedicated issuer IAM user |
| Azure Blob Storage | `azure` | `azure` | Uses time-limited Azure SAS credentials |
| Google Cloud Storage | `gcp` | `gcp` | Uses signed, scoped GCS credentials |
| Cloudflare R2 | `aws` | `r2` | S3-compatible storage plus R2's temporary-credentials API |
| MinIO or generic S3 | `aws` | Provider-dependent | Works as a backing store through a custom S3 endpoint; scoped credentials need a separately supported mechanism |

The API Cargo features are `aws`, `azure`, `gcp`, and `r2`. There is no
provider-specific `minio` feature.

## Common bucket names

Every provider needs content and log storage. Metadata can share the content
bucket when `META_BUCKET` is omitted.
The self-hosted `per_run` deployment requires three distinct buckets; its
bootstrap and gateway reject shared metadata/content/log bucket names.

```dotenv
CONTENT_BUCKET=flow-like-content
META_BUCKET=flow-like-meta
LOG_BUCKET=flow-like-logs
```

Provider-specific names such as `AWS_CONTENT_BUCKET`,
`AZURE_CONTENT_CONTAINER`, and `GCP_CONTENT_BUCKET` override the generic names.

## Amazon S3

```dotenv
STORAGE_PROVIDER=aws
RUNTIME_CREDENTIALS_PROVIDER=aws

AWS_REGION=eu-central-1
AWS_ACCESS_KEY_ID=replace-me
AWS_SECRET_ACCESS_KEY=replace-me
CONTENT_BUCKET=flow-like-content
META_BUCKET=flow-like-meta
LOG_BUCKET=flow-like-logs

RUNTIME_ROLE_ARN=arn:aws:iam::123456789012:role/FlowLikeRuntimeRole
```

The backing store can use static environment credentials, an instance role, or
the normal AWS web-identity credential chain. `RUNTIME_ROLE_ARN` is additionally
required when the API must assume a role to issue short-lived, prefix-scoped
credentials.

For a non-AWS S3 endpoint, add:

```dotenv
AWS_ENDPOINT=http://localhost:9000
AWS_USE_PATH_STYLE=true
```

### Buckets encrypted with a customer-managed KMS key

S3-managed encryption (SSE-S3) and the AWS-managed `aws/s3` key need no
configuration. A **customer-managed** key does: S3 refuses every read without
`kms:Decrypt` on the key and every write without `kms:GenerateDataKey`.

Grant both to the runtime role behind `RUNTIME_ROLE_ARN`, and to the API's own
identity. The API signs presigned URLs and writes dispatch staging payloads
directly. Restrict them there with `kms:ViaService` and the S3 encryption
context; that role policy is the security boundary.

Flow-Like adds the matching KMS statement to every scoped credential it mints,
with no configuration required. This is not optional plumbing: scoped
credentials are STS session policies, which *intersect* with the runtime role
rather than inherit from it, so a session policy naming only `s3:*` actions
strips the role's KMS grant and every request against such a bucket fails,
including requests made through a presigned URL, where it surfaces as an opaque
`AccessDenied` long after the credential was handed out.

The statement uses `Resource: "*"`, which cannot widen anything the role does
not already allow, and keeps the 2048-character STS policy budget free. Its
actions track what the credential can do with S3: `kms:Decrypt` for read
scopes, plus `kms:GenerateDataKey` for write scopes.

To narrow it further rather than rely on the role policy alone, name the keys:

```dotenv
# One key for every bucket
S3_KMS_KEY_ARN=arn:aws:kms:eu-central-1:123456789012:key/1234abcd-12ab-34cd-56ef-1234567890ab

# Or per bucket, overriding the shared value
META_BUCKET_KMS_KEY_ARN=arn:aws:kms:...
CONTENT_BUCKET_KMS_KEY_ARN=arn:aws:kms:...
LOG_BUCKET_KMS_KEY_ARN=arn:aws:kms:...
```

The session statement then names only the key ARNs for buckets a given scope
can reach and adds its own `kms:ViaService` fence. Use full key
ARNs; a bare key id or an alias cannot be a policy resource. A bucket left
unset contributes no resource, which is correct when it is not on a
customer-managed key and wrong if it is, so prefer `S3_KMS_KEY_ARN` whenever
the buckets share a key.

Setting these variables has a second effect: the configured key is sent as an
explicit SSE-KMS header on every write. That is what a bucket policy denying
writes without `x-amz-server-side-encryption` needs, and it is unnecessary
otherwise, since a bucket's default encryption applies the key on its own. Add
`S3_KMS_BUCKET_KEY=true` to request an S3 Bucket Key and collapse the
per-object KMS calls. S3 Express One Zone buckets take their key from the
bucket and reject these headers, so they are left alone.

## Azure Blob Storage

```dotenv
STORAGE_PROVIDER=azure
RUNTIME_CREDENTIALS_PROVIDER=azure

AZURE_STORAGE_ACCOUNT_NAME=flowlikedev
AZURE_STORAGE_ACCOUNT_KEY=replace-me
AZURE_CONTENT_CONTAINER=flow-like-content
AZURE_META_CONTAINER=flow-like-meta
AZURE_LOG_CONTAINER=flow-like-logs
```

The account key is used to build stores and sign scoped SAS credentials. Keep
the account key on the API; clients and executors should receive only the
scoped credentials generated for their work.

## Google Cloud Storage

```dotenv
STORAGE_PROVIDER=gcp
RUNTIME_CREDENTIALS_PROVIDER=gcp

GCP_PROJECT_ID=my-project
GOOGLE_APPLICATION_CREDENTIALS_JSON={"type":"service_account","project_id":"my-project"}
GCP_CONTENT_BUCKET=flow-like-content
GCP_META_BUCKET=flow-like-meta
GCP_LOG_BUCKET=flow-like-logs
```

`GOOGLE_APPLICATION_CREDENTIALS_JSON` is the service-account JSON itself, not a
path to a key file. The API uses it to create signed, scoped credentials.

## Cloudflare R2

R2 uses the S3 protocol for ordinary object-store access, but its own
temporary-credentials API for scoped runtime access:

```dotenv
STORAGE_PROVIDER=aws
RUNTIME_CREDENTIALS_PROVIDER=r2

AWS_ENDPOINT=https://ACCOUNT_ID.r2.cloudflarestorage.com
AWS_REGION=auto
AWS_USE_PATH_STYLE=true
AWS_ACCESS_KEY_ID=replace-me
AWS_SECRET_ACCESS_KEY=replace-me

R2_ENDPOINT=https://ACCOUNT_ID.r2.cloudflarestorage.com
R2_ACCOUNT_ID=replace-me
R2_ACCESS_KEY_ID=replace-me
R2_SECRET_ACCESS_KEY=replace-me
R2_API_TOKEN=replace-me

CONTENT_BUCKET=flow-like-content
META_BUCKET=flow-like-meta
LOG_BUCKET=flow-like-logs
```

## RustFS

Compose and Kubernetes bundle a digest-pinned RustFS release and initialize
three private buckets, a restricted API storage user and a separate STS issuer.
The runtime-credential implementation uses the `aws` feature with the RustFS
policy dialect:

```dotenv
STORAGE_PROVIDER=aws
RUNTIME_CREDENTIALS_PROVIDER=aws
S3_STS_PROVIDER=rustfs
AWS_REGION=us-east-1
AWS_USE_PATH_STYLE=true
STS_SESSION_TTL_SECONDS=7200
```

Configure `S3_PUBLIC_ENDPOINT` as the exact origin used in signed object
requests. It must resolve from clients, API, compiler and execution gateways.
Use `S3_INTERNAL_ENDPOINT` for internal LanceDB access and a private
`STS_ENDPOINT_URL` for issuance. The public bucket gateway blocks STS and
administration. Changing a signed URL's host or path invalidates its signature.

Supply separate `AWS_ACCESS_KEY_ID`/`AWS_SECRET_ACCESS_KEY` and
`STS_ISSUER_ACCESS_KEY`/`STS_ISSUER_SECRET_KEY` pairs through protected
configuration. Keep root credentials on the store and initializer. RustFS
issuance uses regular IAM users; it does not require `RUNTIME_ROLE_ARN` or AWS
KMS settings. The RustFS dialect omits AWS KMS session-policy clauses and rejects
AWS SSE-KMS configuration.

The API uses the expiry returned by STS and checks remaining lifetime before
dispatch, including cached credentials. An hour-long run needs additional
lifetime for queue wait and supervisor allowances; the deployment requests
two-hour sessions. Automatic in-run renewal is not implemented.

Use the [Compose storage guide](/self-hosting/docker-compose/storage/) or
[Kubernetes storage guide](/self-hosting/kubernetes/storage/) for generated
secrets, endpoint routing and conformance commands. Test both permitted access
and denied sibling-prefix, copy-source and administration operations against the
exact store version before relying on its session policy.

## MinIO and other S3-compatible stores

MinIO can be used as the backing object store:

```dotenv
STORAGE_PROVIDER=aws
AWS_ENDPOINT=http://localhost:9000
AWS_REGION=us-east-1
AWS_USE_PATH_STYLE=true
AWS_ACCESS_KEY_ID=restricted-development-user
AWS_SECRET_ACCESS_KEY=replace-me
CONTENT_BUCKET=flow-like-content
META_BUCKET=flow-like-meta
LOG_BUCKET=flow-like-logs
```

This configuration covers object access only. Do not assume that an
S3-compatible server implements the AWS STS behavior used by Flow-Like's `aws`
runtime-credential provider. For any deployment that sends scoped credentials
to clients or remote executors, verify the provider-specific credential path
end to end.

## Tests

Serialization, policy-shape, and other non-networked credential tests run
without cloud resources:

```bash
cargo test -p flow-like --lib credentials
cargo test -p flow-like-api --features full --lib credentials
```

Provider integration tests are ignored by default because they use real
credentials and storage:

```bash
# Run every ignored credential test with all providers compiled.
cargo test -p flow-like-api --features full --lib credentials -- --ignored

# Compile and run one provider's ignored tests.
cargo test -p flow-like-api --features aws --lib credentials -- --ignored
cargo test -p flow-like-api --features azure --lib credentials -- --ignored
cargo test -p flow-like-api --features gcp --lib credentials -- --ignored
```

R2 currently has non-networked credential tests but no ignored live-provider
suite in `packages/api/src/credentials/r2_credentials.rs`.

:::caution
Integration tests can write objects and may incur cloud charges. Use dedicated
development buckets or containers and least-privilege credentials.
:::

## Troubleshooting

- **Unknown runtime credentials provider**: ensure
  `RUNTIME_CREDENTIALS_PROVIDER` is absent or one of the compiled providers; do
  not leave it explicitly empty.
- **S3 signature mismatch**: verify `AWS_REGION`, endpoint scheme, and
  `AWS_USE_PATH_STYLE`.
- **Missing logs**: configure `LOG_BUCKET` or the provider-specific log
  bucket/container variable.
- **Backing-store access works but scoped execution fails**: check the runtime
  credential provider separately. Successful S3 reads do not prove that STS,
  R2 temporary credentials, Azure SAS, or GCP signing is configured.
