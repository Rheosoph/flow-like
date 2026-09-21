# Audit worker Lambda

Runs one pass of the audit worker per invocation: seal pending records, sign an epoch,
write the daily head, archive closed months, prune past the database window, expire
personal values, export the legacy trail once and deliver customer webhooks. It returns
the `TickReport` as JSON. Design: `todo/874-audit-retention.md`.

It builds the same `State` as the Lambda API (`../shared/api_bootstrap.rs`), so it needs
the API's environment, and it is the only component holding the audit key and write access
to the audit bucket. The Terraform lives in the deployment repository.

## Resources

| Resource | Settings |
| --- | --- |
| Function | Image from `Dockerfile` (arm64), 1024 MB, timeout 900 s, reserved concurrency 1. Same VPC, database and secret access as the API Lambda |
| Schedule | EventBridge Scheduler, `rate(1 minute)`, target this function, input `{}`, retries 0 (the next tick retries). Overlapping runs are harmless: the worker lease stays with the container that keeps ticking and any other reports `skipped` |
| Audit bucket | S3 with Object Lock enabled at creation and a default retention in governance mode (compliance for regulated profiles). The lock starts when an object is written, so 1461 days covers every manifest's `retain_until` (end of the third calendar year after the month); 3 years leaves January to November archives unlocked for their last months. Block Public Access on, TLS-only bucket policy. Lifecycle: `archive/` to Glacier Deep Archive from day 0, `heads/` stays in Standard, abort incomplete multipart uploads after 7 days, expire objects once their lock has passed |
| Audit key | KMS asymmetric key, key spec `ECC_NIST_P256`, key usage `SIGN_VERIFY`. Asymmetric keys do not rotate; a new key gets a new key id |
| Worker role | Audit bucket: `s3:PutObject`, `s3:GetObject`, `s3:ListBucket` only (`s3:AbortMultipartUpload` optional; the lifecycle rule cleans up otherwise). Audit key: `kms:Sign`, `kms:GetPublicKey`. With `AUDIT_BUCKET_KMS_KEY_ARN` also `kms:GenerateDataKey` and `kms:Decrypt` on that key. No delete, no `s3:BypassGovernanceRetention` |
| API roles | No access to the audit bucket and no `kms:Sign` |

## Environment

Everything the API Lambda reads (`SECRET_PREFIX`, database or `DSQL_*`, `CDN_BUCKET_NAME`,
runtime configuration), plus the following plain environment variables of the function:

| Variable | Meaning |
| --- | --- |
| `AUDIT_BUCKET` | Audit bucket name. Unset: nothing is archived and no evidence is pruned |
| `AUDIT_BUCKET_KMS_KEY_ARN` | Optional SSE-KMS key sent with every upload |
| `AUDIT_KMS_KEY_ID` | Key id, key ARN or alias of the audit key |
| `AUDIT_KMS_PROVIDER` | Optional; `aws` is already the default for a key id, ARN or alias |
| `AUDIT_KMS_REGION` | Optional. Default: the region of a key ARN, else the function's region |

The function role supplies the bucket and key credentials, so `AUDIT_BUCKET_ACCESS_KEY_ID`,
`AUDIT_BUCKET_SECRET_ACCESS_KEY`, `AUDIT_KMS_AWS_ACCESS_KEY_ID` and
`AUDIT_KMS_AWS_SECRET_ACCESS_KEY` stay unset, and so does `AUDIT_SIGNING_KEY`: the worker
refuses to start when it is set together with `AUDIT_KMS_KEY_ID`.

These are read through the secret store like the API's other secrets: SecureString
parameters named `<SECRET_PREFIX>/<NAME>` in Parameter Store, the same parameters the API
Lambda reads. A plain environment variable of that name is not used while `SECRET_PREFIX`
is set.

| Name | Meaning |
| --- | --- |
| `AUDIT_KID` | Optional key id recorded in epochs; defaults to the public key fingerprint. When `AUDIT_VERIFYING_KEYS` already holds this id, the worker never calls `kms:GetPublicKey` |
| `AUDIT_ENTRY_KEY` | Must equal the API's value (or both derive it from the same `BACKEND_KEY`), otherwise pending records are quarantined |
| `AUDIT_ENTRY_KEY_PREVIOUS` | Only while rotating the entry key: the previous key, still accepted for records and seals written before the switch |
| `AUDIT_VERIFYING_KEYS` | Public keys of audit keys, as `{"<kid>": "<PEM>"}` |

The container that first wins the worker lease connects the audit key and logs
`audit key-service key connected` with the key id and its PEM public key; containers that
never get the lease never call KMS. Add that pair to `AUDIT_VERIFYING_KEYS` so every API
process can verify epochs without the key. An ECS API next to this function sets
`AUDIT_WORKER=off`. Entry key rotation and held chains are described in the self-hosting
documentation (`apps/docs/src/content/docs/self-hosting/audit-trail.md`).
