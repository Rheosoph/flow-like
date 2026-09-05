# flow-like-compiler

`flow-like-compiler` is the WASM compilation worker. It receives a
`CompilationJob`, downloads the raw `.wasm` from a presigned storage URL,
compiles it to pre-compiled `.cwasm` for each requested target, uploads the
artifacts through presigned upload URLs, and reports the outcome back to the API
with a JWT-signed callback.

The crate is a shared library. The deployment wrappers live in
`apps/backend/` (`docker-compose/compiler`, the AWS and Azure workers). Every
wrapper mounts the same router:

```rust
use flow_like_compiler::{compiler_router, CompilerState};

let state = CompilerState::from_env();
let app = compiler_router(state);
```

- `POST /compile` — accepts a `CompilationJob`, returns a `CompilationResult`
- `GET /health`

## Trust model for storage URLs

The compiler treats every URL in a job as untrusted input, including the signed
storage URLs. It never infers the storage backend from the URL — the job carries
a `CompilationStorageProvider` that records *how the URL was signed*, and the
worker validates the URL against the rules for that provider
(`src/compile.rs`, `validate_storage_url`).

Two rules apply to every job, before the provider-specific ones:

1. The URL must be HTTPS on port 443.
2. The host must be a name, never a literal IP address.

Then, per provider:

| Provider stamp | Accepted without configuration |
| --- | --- |
| `AzureBlob` | `<AZURE_STORAGE_ACCOUNT_NAME>.blob.core.windows.net` only |
| `AwsS3` | Public AWS S3 hosts only — `s3.*` or `*.s3.*` ending in `amazonaws.com` / `amazonaws.com.cn` |
| `GoogleCloudStorage` | `storage.googleapis.com` and `*.storage.googleapis.com` |

`COMPILER_ALLOWED_STORAGE_HOSTS` is the single escape hatch from all of the
above, and it is an *exact origin* allowlist — never a suffix or wildcard match.

## S3-compatible storage: Cloudflare R2, MinIO, Ceph, Wasabi

**This section is required reading for any deployment that is not on public AWS
S3.** Getting it wrong makes every external compilation fail, and the failure
looks like a job validation error rather than a configuration error.

### Why it is needed

Flow-Like configures every S3-compatible endpoint as `FlowLikeStore::AWS`,
because they all speak the S3 API and all sign with SigV4. The API stamps every
`FlowLikeStore::AWS` job as `CompilationStorageProvider::AwsS3`
(`packages/api/src/compilation/dispatch.rs`, `compilation_storage_provider`).
The compiler therefore holds an R2, MinIO, Ceph or Wasabi URL to the public-AWS
host rules above, which those hosts do not satisfy, and rejects the job.

Allowlisting the endpoint origin is what tells the compiler that this specific
non-AWS origin is a legitimate S3 endpoint for this deployment.

You need an entry whenever `AWS_ENDPOINT` is set to anything other than an AWS
S3 endpoint. You do **not** need one for public AWS S3, Azure Blob against the
configured account, or public GCS.

### Accepted entry forms

Comma-separated. Each entry is one of:

| Form | Means | Example |
| --- | --- | --- |
| Bare host | HTTPS on port 443 | `abc123.r2.cloudflarestorage.com` |
| Full origin | Exactly this scheme, host and port | `https://s3.eu-central-1.wasabisys.com`, `http://minio:9000` |

Entries are lowercased. A path, query string, fragment, or user information in
an entry is rejected, and a rejected entry is skipped with a
`tracing::warn!` — the process still starts, so a typo shows up as a failing
compilation, not as a crash. Matching is on scheme, host and port together: a
bare host entry does not authorize the same host on a different port, and an
`https://` entry does not authorize `http://`.

### Cloudflare R2

R2's S3 endpoint is account-scoped. Use the host of the endpoint you already put
in `AWS_ENDPOINT`, without the scheme:

```bash
AWS_ENDPOINT=https://<account-id>.r2.cloudflarestorage.com
AWS_USE_PATH_STYLE=true
COMPILER_ALLOWED_STORAGE_HOSTS=<account-id>.r2.cloudflarestorage.com
```

R2 is HTTPS on 443, so the bare-host form is the right one and nothing about the
transport posture changes. If your bucket lives in a jurisdiction-scoped
endpoint, allowlist that exact host instead — for example
`<account-id>.eu.r2.cloudflarestorage.com`. Keep `AWS_USE_PATH_STYLE=true` so the
bucket stays in the path and the host remains the single account endpoint you
allowlisted; virtual-host-style addressing would put the bucket name into the
host and no longer match.

### Self-hosted MinIO on plaintext

A MinIO reachable inside a Compose or Kubernetes network on plain HTTP needs the
full-origin form, because both the scheme and the non-default port deviate from
the default:

```bash
AWS_ENDPOINT=http://minio:9000
AWS_USE_PATH_STYLE=true
COMPILER_ALLOWED_STORAGE_HOSTS=http://minio:9000
```

If MinIO terminates TLS on 9000, use `https://minio:9000` — still the
full-origin form, because 9000 is not 443. If it is behind a TLS proxy on 443,
the bare host `minio.example.com` is enough.

The endpoint must be addressed by name. The IP-address rejection happens before
the allowlist is consulted and cannot be waived by it, so
`AWS_ENDPOINT=http://127.0.0.1:9000` fails every job with
`signed storage URL must not target an IP address` no matter what is
allowlisted. Use the Compose service name, the Kubernetes Service name, or a
hosts entry.

### Security note: allowlisting an `http` origin is global

The compiler's storage HTTP client is built once, and it sets
`https_only(!config.allows_plaintext_storage())` (`src/compile.rs`,
`storage_client`). A single `http://` entry anywhere in
`COMPILER_ALLOWED_STORAGE_HOSTS` therefore turns the client's HTTPS-only
enforcement **off for the whole compiler process**, not just for that origin.

That is not as wide open as it sounds, and it is worth knowing exactly how wide
it is:

- Every URL still passes `validate_storage_url` first. A cleartext URL is
  accepted only if its scheme, host and port match an allowlisted origin
  exactly, so an arbitrary attacker-supplied `http://` URL is still rejected.
- Redirects are disabled on the storage client
  (`redirect::Policy::none()`), so an allowlisted origin cannot bounce the
  worker to a different host.
- What is genuinely lost is the transport-layer backstop. With `https_only`
  active, reqwest refuses a cleartext request even if validation were ever
  bypassed. With one `http` entry present, that second line of defence is gone
  process-wide.

Practical guidance: use `http` only for an endpoint on a trusted, non-routable
network (a Compose service name, a cluster-internal Service). Anything crossing
a network you do not control should terminate TLS and be allowlisted as
`https://…`, which keeps `https_only` on. Note also that WASM payloads and
compiled artifacts move over this client — over `http` they are readable and
modifiable by anything on the path.

### What the failure looks like

With the allowlist unset or wrong, the job is rejected at validation and the
error names both the origin and the fix. The origin is reconstructed from the
signed URL's scheme, host and port and **never includes the signature**.

An R2 or other HTTPS S3-compatible endpoint fails the provider check:

```text
signed S3 URL origin https://abc123.r2.cloudflarestorage.com:443 is not an
approved endpoint; add https://abc123.r2.cloudflarestorage.com:443 to
COMPILER_ALLOWED_STORAGE_HOSTS to use an S3-compatible endpoint such as
Cloudflare R2 or MinIO
```

A plaintext or non-443 endpoint fails earlier, on the transport check, before
the provider is even considered:

```text
signed storage URL origin http://minio:9000 must be HTTPS on port 443; add
http://minio:9000 to COMPILER_ALLOWED_STORAGE_HOSTS to allow this endpoint
```

Both messages print the origin in the exact form you can paste into
`COMPILER_ALLOWED_STORAGE_HOSTS`. The `:443` suffix in the first message is
equivalent to the bare-host form; either is accepted.

A GCS-compatible private endpoint produces the same shape of message against the
`GoogleCloudStorage` branch.

## Configuration

Compilation settings are read by `CompilerConfig::from_env()`. The HTTP service also reads `COMPILER_MAX_CONCURRENT_JOBS` when it creates its admission semaphore. Explicit concurrency limits must be positive integers; invalid values fail startup. Out-of-range storage limits fall back to their defaults.

| Variable | Default | Notes |
| --- | --- | --- |
| `COMPILER_ALLOWED_STORAGE_HOSTS` | empty | Exact storage origin allowlist. See above |
| `COMPILER_TIMEOUT_SECS` | `600` | End-to-end compilation timeout |
| `COMPILER_STORAGE_TIMEOUT_SECS` | `120` | Per signed storage request. Clamped to 5–300 |
| `COMPILER_CALLBACK_TIMEOUT_MS` | `10000` | Terminal callback timeout |
| `COMPILER_CALLBACK_RETRIES` | `3` | Terminal callback retries |
| `COMPILER_MAX_PARALLEL_TARGETS` | all cores | Concurrent target compilations per job; must be positive |
| `COMPILER_MAX_CONCURRENT_JOBS` | `2` | HTTP jobs per compiler instance; excess requests return 429, and queue consumers retain their own admission limits |
| `COMPILER_MAX_WASM_BYTES` | 256 MiB | Maximum raw `.wasm` download. Clamped to 1 KiB–512 MiB |
| `COMPILER_MAX_ARTIFACT_BYTES` | 512 MiB | Maximum single artifact upload. Clamped to 1 KiB–1 GiB |
| `AZURE_STORAGE_ACCOUNT_NAME` | unset | Required for `AzureBlob` jobs; pins the accepted blob host |
| `AZURE_CONTENT_CONTAINER` | unset | Required for `AzureBlob` jobs; pins the accepted container |
| `AZURE_META_CONTAINER` | unset | Required for `AzureBlob` jobs; pins the accepted container |
| `BACKEND_PUB` | unset | Base64 backend public key for job JWT verification |
| `API_BASE_URL` | unset | Used to fetch the backend public key when `BACKEND_PUB` is not set |
| `BACKEND_KID` | unset | When set, the job JWT's key id must match |

## Development

```bash
cargo check -p flow-like-compiler
cargo test -p flow-like-compiler
```
