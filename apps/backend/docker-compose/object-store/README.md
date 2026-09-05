# Bundled object storage

The bundled service uses RustFS `1.0.0-rc.5`. Bootstrap creates separate private metadata,
content and log buckets, an API IAM user and a distinct STS issuer IAM user. Both users are
restricted to the application buckets and explicitly denied administrative operations. The
API user is also explicitly denied `AssumeRole`. These are regular IAM users because RustFS
service accounts cannot issue STS sessions. Only the initializer and store receive root keys.

Bootstrap uses S3 SigV4 with the pinned release's native `/rustfs/admin/v3` API, following its
[STS compatibility test](https://github.com/rustfs/rustfs/blob/1.0.0-rc.5/crates/e2e_test/src/sts_query_compat_test.rs)
and [IAM handlers](https://github.com/rustfs/rustfs/blob/1.0.0-rc.5/rustfs/src/admin/handlers/policies.rs).

## Endpoints and credentials

Set `S3_STS_PROVIDER=rustfs`, `AWS_USE_PATH_STYLE=true`, `AWS_REGION=us-east-1` and a direct
private `STS_ENDPOINT_URL`. Set distinct `STS_ISSUER_ACCESS_KEY`/`STS_ISSUER_SECRET_KEY`,
`AWS_ACCESS_KEY_ID`/`AWS_SECRET_ACCESS_KEY`, and `RUSTFS_ROOT_USER`/`RUSTFS_ROOT_PASSWORD` pairs.
The initializer requires all three pairs. Secrets support `_FILE`; unreadable or empty files
fail, and only trailing line endings are removed. The API requires exactly one of a value and
its `_FILE` setting. Bootstrap and standalone storage readers give files precedence, but keep
only one configured form when sharing configuration with the API.

`STS_SESSION_TTL_SECONDS` requests 900 to 43,200 seconds. The library default is 3,600;
Compose and Helm request 7,200 to cover hour-long runs and supervisor margins. The API uses the
expiration returned by STS and does not reuse credentials near expiry. RustFS omits AWS KMS
policy clauses and rejects AWS SSE-KMS and S3 Express settings. Static API credentials do not
need a fabricated session token. The AWS provider retains its existing role and KMS behavior.

`S3_PUBLIC_ENDPOINT` is the exact origin in signed URLs and exported runtime credentials.
It must resolve from browsers, API containers, compilers and runtimes. Primary API stores
also use this origin because the same store signs URLs. Resolve the public hostname through
the S3 gateway inside the deployment. Changing a signed URL's host or path breaks its signature.
API LanceDB connections use `S3_INTERNAL_ENDPOINT`. `AWS_ENDPOINT` remains a legacy fallback.
`META_BUCKET_ENDPOINT`, `CONTENT_BUCKET_ENDPOINT` and `LOGS_BUCKET_ENDPOINT` override exported
per-bucket endpoints. Region, path style and explicit HTTP settings travel with shared
credentials. HTTPS certificate verification stays enabled; private certificate authorities
must be installed into each caller's trust store. CA files are not embedded in credentials.

## Initialization and drift

Set three distinct names in `META_BUCKET`, `CONTENT_BUCKET`, and `LOG_BUCKET`. Browser CORS
uses exact origins from comma-separated `S3_CORS_ALLOWED_ORIGINS`; wildcard origins fail.
Existing bucket policies stop initialization for review. Bootstrap installs CORS and lifecycle
rules: incomplete multipart uploads expire after one day and content under `tmp/` after two.
Existing IAM policies must match; existing users must have only their expected policy and no
group membership. Existing user passwords are verified, never silently replaced. Rotating a
configured secret therefore requires an explicit IAM rotation or a new identity.

## Verification

From the repository root, run the offline bootstrap checks:

```sh
python3 -m venv /tmp/flow-like-store-tests
/tmp/flow-like-store-tests/bin/pip install -r apps/backend/docker-compose/object-store/requirements.txt
PYTHONDONTWRITEBYTECODE=1 /tmp/flow-like-store-tests/bin/python -m unittest discover -s apps/backend/docker-compose/object-store -p 'test_*.py'
```

After Compose starts a disposable qualification deployment, run from its directory:

```sh
docker compose run --rm --entrypoint python object-store-init /opt/object-store/conformance.py
```

The optional script creates unique temporary prefixes and cleans up its objects. It checks
allowed access, sibling and confusable prefixes, empty-prefix listing, explicit deny, other
buckets, copy-source authorization, multipart upload/abort, missing or invalid tokens, nested
STS issuance, API-user STS denial, admin denial and public-origin signed GET/PUT. The API also
has a local fake-STS test exercising its locked AWS SDK, issuer signature and returned expiry.

Kubernetes packages the same conformance contract as `helm test <release> -n <namespace> --logs`.
Run it after initialization and before accepting tenants, and repeat when changing the store
version, policies, public endpoint or issuer identity. A passing run must include successful
authorized operations and denied sibling-prefix, copy-source and administration attempts.

The suite does not wait for real expiry or exercise multipart copy, IAM restore, storage
failover or every application path. Qualification also needs an hour-long workflow that can
read/write near its deadline, denied operations after session expiry, and a backup restored
with its IAM metadata. Recheck old credentials after restore and rotation. Sandbox termination
does not immediately revoke credentials already exported to a caller; do not treat it as a
storage revocation mechanism. Record the store digest and probe results with each release.
