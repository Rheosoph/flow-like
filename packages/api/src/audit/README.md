# Cryptographic audit trail

Operators can use the audit trail to identify who requested a change, inspect its
result, and check whether the stored records still match their hashes and server
signatures. Platform administrators read the root chain. App owners can read their
app's branch through `/api/v1/audit/entries?chain_id=<app-id>`.

## Coverage and failure behavior

When `audit.enabled` is true, authenticated POST, PUT, PATCH and DELETE requests
record `api.request.attempt` before dispatch and `api.request.finish` when the
handler produces response headers. Both entries share a request ID. The finish
entry records the HTTP status and the number of failed domain audit writes.
App-scoped routes put these records on the app chain. Other routes use the root
chain. Telemetry ingestion is excluded.

The middleware records the matched route template, method and actor. It does not
read request bodies, query strings, credential headers or concrete paths.
Action-specific hooks add resource IDs and bounded metadata for app, permission,
board, file, widget, database, graph and other mutations. A file access grant
records the authorization to upload or read. Provider access or event logs are
needed to establish what happened after a client received a signed URL or scoped
storage credentials.

An attempt write failure returns HTTP 503 before the handler runs. An outcome or
domain audit write failure after dispatch is traced and adds
`x-flow-like-audit-status: incomplete` to the original response. The response
retains the handler's status because a mutation may already have committed.
Operators should investigate attempts without a finish and finishes with a
nonzero `domain_audit_failures`. Domain changes and their audit entries are not
one atomic transaction. A crash can leave an attempt with an unknown outcome.
Execution state updates have the same boundary. Callback retries can repair a
missing terminal entry, but background crashes have no durable outbox recovery.

With `audit.log_executions` enabled, persisted run transitions record starts,
completion, failure, cancellation and timeout on the app chain. Repeated terminal
callbacks reuse the first record for that run and action. Streaming and background
work record their execution outcomes separately from HTTP response status. The
checked-in platform configuration enables execution logging; the configuration
type's default remains false for installations that have not opted in.

Anonymous requests and authentication failures are outside the mutation middleware's
coverage. Inbound and sink execution paths use explicit lifecycle hooks. This
trail does not capture offline desktop edits or every direct storage operation.

## Integrity and compatibility

New entry hashes start with `v2:`. They use a canonical JSON encoding that preserves
field boundaries and covers the record ID, sequence, timestamp, actor identity and
type, optional IP, action, resource, chain scope, summary, details, previous hash,
previous signature and signing key ID. P-256 ECDSA signs the resulting BLAKE3 hash.
Timestamps are normalized to the database's millisecond precision before hashing.

Writers serialize through a retained `MutationLock` row before reading a chain's
tail. This also coordinates the first append and the root chain, whose nullable
`chainId` cannot provide uniqueness by itself. Transaction retries retain the same
record ID so an acknowledged-late commit does not create a duplicate. Upgrade all
API writers together: older writers do not participate in this coordination and
cannot verify the v2 format. No existing audit entries are rewritten.

`GET /api/v1/audit/verify` checks the root chain. Supply `chain_id` to select a
branch and optional positive, inclusive `from` and `to` sequence bounds. The
verifier reads a database snapshot, checks sequence continuity, resolves branch
anchors from the root chain, reconstructs hashes and verifies signatures. The
`entries_checked` and assurance counters include an immediate predecessor or root
anchor when the selected range depends on it.
The dashboard automatically verifies root chains with at most 1,000 entries;
larger chains show "Not checked" and require an operator to request full
verification in the chain explorer.

`valid` means the requested verification checks succeeded. `fully_authenticated`
additionally requires signed v2 entries with available verification keys. Legacy
entries retain their original hash algorithm; its omitted metadata and ambiguous
field boundaries cannot be repaired retroactively. They are counted as legacy
and do not qualify as fully authenticated. Unsigned entries are counted separately.
An unavailable historical public key causes verification to fail with an
`unverifiable_signatures` count. It does not by itself prove that a record changed.

Verification cannot establish events that were never recorded. Detecting deletion
of a whole chain or its final records requires a previously retained checkpoint
outside this database. Retain signed chain heads independently when that evidence
is required. A valid subrange also does not certify all earlier history.

## Signing keys and IP addresses

`BACKEND_KEY` supplies the base64-encoded P-256 PKCS#8 PEM signing key and
`BACKEND_KID` identifies it. With `audit.require_signing` true, API startup refuses
to proceed without a usable signing key. The checked-in configuration requires it.

Before rotating the signing key, retain its public key. Supply historical keys in
the `AUDIT_VERIFYING_KEYS` secret as a JSON object mapping key IDs to P-256 SPKI
PEM public keys. The value uses JSON newlines inside each PEM string:

```json
{
  "previous-key-id": "-----BEGIN PUBLIC KEY-----\n<base64 public key>\n-----END PUBLIC KEY-----\n"
}
```

All API replicas must have the same signing key and ID, and the retained public
keys needed to verify their history. The registry accepts public keys only and
rejects a historical key that conflicts with the active signing key's ID.

`audit.log_ip` is false by default. When enabled, request audit records and
synchronous domain hooks can include a syntactically valid client IP from the
authentication middleware. Forwarded headers must be controlled by the deployment's
trusted proxy; parsing an address does not establish its provenance. Signed IP
fields are immutable. `ip_retention_days` does not currently erase them, so keep
IP recording disabled if automatic IP expiry is required.

## Regression checks

Unit tests cover framing, metadata tampering, signature failures, missing sequence
boundaries, legacy compatibility and middleware failure behavior. The PostgreSQL
integration tests in `packages/api/tests/audit_integrity.rs` use a disposable
database to check concurrent appends, retries, signed branches and persisted values.
They must not be pointed at a production database.
