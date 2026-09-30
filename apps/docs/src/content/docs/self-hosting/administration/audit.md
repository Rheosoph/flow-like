---
title: Audit operations
description: Configure audit coverage and retention, monitor the worker, and verify records
---

## Audit trail

The audit trail records who changed what, and lets anyone holding a copy check later
that nothing was changed, reordered or removed. A request writes each record with a
single insert. Within minutes the audit worker seals it into its chain and commits
the seal to a platform-wide epoch signed with the audit key. Closed months are
archived to an immutable audit bucket, and records leave the database once their
retention window has passed, behind a signed watermark so the chain still verifies.

Platform administrators read every chain under **Admin > Logs**. App owners read,
verify and export their own app's chains; see
[App audit trail and export](/apps/audit-trail/). The bucket, the worker and the
keys are deployment settings; see [Audit trail storage](/self-hosting/audit-trail/).

### Levels

`audit.level` selects how much of the mutation surface is recorded. Each level
includes the ones below it.

| Level | Records |
| --- | --- |
| `minimal` | Identity and access: roles, memberships, invites, personal access tokens, API keys, app connections, sinks, platform administration, app creation, visibility and publication, and deletions of primary resources (apps, boards, events, pages, widgets, templates, routes, tables, graph overlays). |
| `standard` (default) | `minimal` plus content changes: board saves and versions, FlowScript and IR commits, events, canaries, regression suites, pages, widgets, templates, routes, metadata, process notes, table creation and row changes, graph overlay schema changes, upload grants and file deletions. |
| `verbose` | `standard` plus `api.request.attempt` and `api.request.finish` for every mutation, editor commands (`board.commands.*`), graph node and edge writes, file read grants and execution lifecycle records. |

Execution lifecycle records also follow `audit.log_executions`: the switch
enables them at any level, and `verbose` records them without it. The
checked-in configuration for the public instance uses `standard` with
`log_executions` off, so it records access and content changes but not
per-request or per-run records. An installation without `level` in its
configuration runs at `standard`; set `verbose` to keep the previous behavior.

The level applies when a record is written. Records carry no sequence number, so
an action that is not recorded at the configured level leaves no gap. Actions the
level classifier does not know are recorded at `standard`. The classification
lives in `packages/api/src/audit/level.rs` and its test lists every action name.

### Chains and retention classes

Every record belongs to one chain, and every chain to one retention class.

| Chain | Holds | Readable by |
| --- | --- | --- |
| `platform` | Changes outside any app or package, such as personal access tokens and platform administration | Administrators (`Admin`) |
| `<app-id>` | The app's evidence records | The app's Owners and administrators |
| `package:<package-id>` | The package's evidence records | Administrators |
| `<chain>#activity` | Activity records of the same scope, for example `<app-id>#activity` or `platform#activity` | Same as the scope's chain |

| Class | Actions | Retention |
| --- | --- | --- |
| Evidence | Everything recorded at `minimal` and `standard` | Archived to the audit bucket a few days after its month closes, where the bucket's lock keeps it. Leaves the database `evidence_hot_days` after the month closes; without an audit bucket it stays there |
| Activity | Actions recorded only at `verbose`: `api.request.*`, `board.commands.*`, `execution.*`, graph node and edge writes, file read grants | Deleted from the database after `activity_days`, or `high_risk_activity_days` for high-risk AI systems. Never archived |

Activity records sit on their own chains, so each class ages out on its own
schedule without breaking the other. At `standard` with `log_executions` off, the
activity chains stay empty.

### Coverage and failure behavior

At the `verbose` level, authenticated POST, PUT, PATCH and DELETE requests
record `api.request.attempt` before dispatch and `api.request.finish` when the
handler produces response headers. Both records share a request ID. The finish
record holds the HTTP status and the number of failed domain audit writes.
App-scoped routes put these records on the app's activity chain when the caller
holds a role in that app. A caller without one is recorded on `platform#activity`
with the requested app in `details.requested_app_id`, so naming an app in a path
cannot write into its chains. Other routes use `platform#activity`. Telemetry
ingestion is excluded. At lower levels the request context is still established so
domain hooks record the actor IP and a failed domain write still marks the response
as described below.

The middleware records the matched route template, method and actor. It does not
read request bodies, query strings, credential headers or concrete paths.
Action-specific hooks add resource IDs and bounded metadata for app, permission,
board, file, widget, database, graph and other mutations. A file access grant
records the authorization to upload or read. Provider access or event logs are
needed to establish what happened after a client received a signed URL or scoped
storage credentials.

At `verbose`, an attempt write failure returns HTTP 503 before the handler runs.
An outcome or domain audit write failure after dispatch is traced and adds
`x-flow-like-audit-status: incomplete` to the original response. The response
retains the handler's status because a mutation may already have committed.
A request deadline (HTTP 504) drops the handler while a domain audit write may
still be waiting, so a timed-out mutation is always marked `incomplete`, and at
`verbose` its finish record carries `domain_audit_outcome: "unknown"`.
Operators should investigate attempts without a finish and finishes with a
nonzero `domain_audit_failures`. Domain changes and their audit records are not
one atomic transaction. A crash can leave an attempt with an unknown outcome.
Execution state updates have the same boundary. Callback retries can repair a
missing terminal record, but background crashes have no durable outbox recovery.

With `audit.log_executions` enabled or at the `verbose` level, persisted run
transitions record starts, completion, failure, cancellation, timeout and
admission rejections on the app's activity chain. A terminal record has an id
derived from its chain, action and run, so repeated callbacks write it once.
Streaming and background work record their execution outcomes separately from
HTTP response status. Run details remain available in the execution run index
regardless of this setting.

Records hold no free text. The UI renders each record as a sentence from its
action, resource and `details`, which carry ids, counts and short codes only and
are capped at 1 KB; larger details are replaced by `{"omitted_bytes": <size>}`.

Anonymous requests and authentication failures are outside the mutation middleware's
coverage. Inbound and sink execution paths use explicit lifecycle hooks. This
trail does not capture offline desktop edits or every direct storage operation.

### How a record becomes tamper-evident

1. **Write.** The API hashes the record and inserts it with a MAC keyed with the
   entry key. It is one insert with no lock, no read and no sequence number, so any
   number of users can write to the same chain at once. The record is listed
   immediately with status `pending`.
2. **Seal.** The audit worker seals a chain once `seal_after_records` records are
   pending or the oldest has waited `seal_after_seconds`. It recomputes each
   record's hash, checks the MAC and writes a seal: the Merkle root over the record
   hashes in `(timestamp, id)` order, linked to the chain's previous seal by that
   seal's hash. The record's MAC is cleared because the seal covers the record, and
   the seal carries its own MAC with the entry key until an epoch signs it.
3. **Epoch.** Once the oldest unsigned seal has waited `epoch_interval_seconds`, or
   20,000 seals wait, the worker writes one epoch: a Merkle root over all waiting
   seals, linked to the previous epoch and signed with the audit key. It first
   re-hashes every seal and checks its MAC; a seal that fails is never signed, is
   logged at error level, and holds back the rest of its chain. Each seal stores its
   inclusion proof, so one chain verifies without reading any other chain.
4. **Daily head.** Once a UTC day has an epoch, the newest epoch is written to
   `heads/YYYY/MM/DD.json` in the audit bucket.
5. **Archive.** `archive_grace_days` after a month closes, once none of its records
   is pending and all of its seals are anchored, the worker re-verifies the month's
   evidence and writes it to the audit bucket as compressed NDJSON parts with a
   signed manifest. A failed check aborts the month; no partial archive is written.
6. **Prune.** Archived evidence leaves the database once its month is older than
   `evidence_hot_days`, activity once it is older than its window; only signed seals
   are ever pruned. Before deleting a chain's seals, the worker writes a watermark
   with the hash of the last seal it deletes. One signature per prune run covers the
   watermarks of every chain it prunes, and a run signs at most once an hour. The
   chain's next seal links to that hash and verification starts there.

   Nothing is pruned on the strength of a database column. Before its hash is signed
   into a watermark, each seal is re-hashed and checked against its chain's class and
   its epoch's signature and inclusion proof, and only the signed fields then decide
   whether it is past its window. Which months count as archived comes from the
   manifests stored with each archive row, accepted only when their signature verifies
   and their epochs link into one unbroken timeline. So rewriting a row, forging an
   archive record or back-dating a seal cannot make the worker sign away evidence; it
   only stops that chain from being pruned, which is logged.

How many requests the audit key costs in a key service, why that number does not grow
with traffic, and how a self-hosted Vault or OpenBao replaces a cloud key service are
described in [Signing cost](/self-hosting/audit-trail/#signing-cost).

A pending record whose MAC fails is never sealed. The worker marks it `invalid`
(quarantined) and logs its id. Seals, epochs and records are hashed with BLAKE3 over
canonical JSON with a separate domain string for each kind; the exact encoding is
documented in [Verify offline](/apps/audit-trail/#verify-offline). Timestamps are
stored and hashed at millisecond precision. U+0000 in any stored string is replaced
with U+FFFD before hashing, because PostgreSQL rejects it and the record of an
already committed mutation must not fail on hostile text.

Three things verification of the database cannot show: a pending record deleted
before it was sealed (a window of at most `seal_after_seconds` plus one worker run),
the removal of the newest seals and epochs of the whole timeline, which only a head
retained outside the platform reveals (see [Heads](#heads)), and events that were
never recorded.

### The audit worker

Once a minute the worker seals, writes an epoch, writes the daily head, archives,
exports the legacy trail, prunes, expires personal values and delivers app
webhooks. A lease in the database lets one worker run at a time, so extra replicas
and overlapping schedules report `skipped` and do nothing. Each step does a bounded
amount of work per run and can be interrupted at any point; the next run completes
it. A failed step is logged as `audit worker step failed` with its name and retried
in the next run, while the other steps continue.

Requests never wait for the worker. When it stops, records accumulate as pending
and requests are unaffected. The worker logs `audit records pending longer than
the alert threshold` at error level with target `audit` when the oldest pending
record is older than `pending_alert_seconds`. Alert on that message.

| Situation | Effect |
| --- | --- |
| No audit key | Records are still sealed and linked, but no epoch, watermark or manifest signature is written. Seals stay unanchored, nothing is pruned, and verification reports `unanchored_seals` |
| No audit bucket | No heads and no archives. Evidence stays in the database; activity is still deleted after its window |
| Worker and API use different entry keys | The worker cannot check the MACs and quarantines every pending record |
| A seal fails its hash or MAC before its epoch | Logged as `unanchored audit seal fails its hash or MAC` at error level. The seal and the rest of its chain are never signed, archived or pruned and stay in the database as evidence; verification reports the seal. Other chains continue |

Where the worker runs and which credentials it holds is described in
[Audit trail storage](/self-hosting/audit-trail/#run-the-audit-worker).

### Retention settings

Retention lives under `audit.retention` in the platform configuration. Every field
is optional; the example shows the defaults.

```json
"audit": {
  "enabled": true,
  "level": "standard",
  "log_ip": false,
  "require_signing": true,
  "log_executions": false,
  "retention": {
    "evidence_hot_days": 396,
    "archive_years_after_year_end": 3,
    "activity_days": 90,
    "high_risk_activity_days": 183,
    "ip_days": 7,
    "details_days": null,
    "seal_after_records": 500,
    "seal_after_seconds": 300,
    "max_records_per_seal": 1000,
    "epoch_interval_seconds": 300,
    "pending_alert_seconds": 900,
    "archive_grace_days": 3
  }
}
```

| Setting | Default | Meaning |
| --- | --- | --- |
| `evidence_hot_days` | 396 | Days an archived month's evidence stays in the database after the month closes. Older months are read from the archive, which takes hours to restore from deep archive tiers. Evidence is never deleted before its month is archived |
| `archive_years_after_year_end` | 3 | Written into each manifest as `retain_until`: 31 December of that many years after the month. The bucket's lock enforces the period; the application never deletes archives |
| `activity_days` | 90 | Days activity records stay in the database |
| `high_risk_activity_days` | 183 | Minimum days for the activity chains of high-risk AI systems |
| `ip_days` | 7 | Days a recorded client IP stays on its record |
| `details_days` | unset | Days `details` stay on their record. Unset keeps them as long as the record, including in the archive |
| `seal_after_records` | 500 | Seal a chain once this many records are pending |
| `seal_after_seconds` | 300 | Seal a chain once its oldest pending record is this old |
| `max_records_per_seal` | 1000 | Largest seal the worker writes, at most 2,000 |
| `epoch_interval_seconds` | 300 | How long a seal may wait for its signed epoch; earlier when 20,000 seals wait. Each epoch is one key-service request, so this sets the signing cost: at most 12 routine signatures an hour |
| `pending_alert_seconds` | 900 | Age of the oldest pending record that triggers the alert log |
| `archive_grace_days` | 3 | Days after a month closes before it is archived |

The defaults keep 13 months of evidence in the database, which covers a 12-month
SOC 2 observation window and the PCI DSS rule that the latest 12 months are kept
with 3 immediately available. The archive horizon matches the three-year German
limitation period. Longer duties, such as six years under HIPAA or SEC Rule 17a-4,
need a longer `archive_years_after_year_end` and bucket lock, or can be met by each
app owner through [audit export](/apps/audit-trail/#export-the-trail). Deployments
under the BSI minimum standard for logging typically set `evidence_hot_days`,
`activity_days`, `ip_days` and `details_days` to 90.

### High-risk AI systems

Apps whose newest AI Act assessment (the highest version) has the risk category
`HIGH` keep their activity chain for at least `high_risk_activity_days`: six months
by default, the minimum of EU AI Act art. 19 and 26(6). The worker reads the
assessments whenever it prunes, so a new assessment version applies from the next
run. When an app is downgraded, its activity records older than `activity_days`
are deleted from then on.

Execution records are activity records. The window keeps what is recorded but does
not start recording: enable `audit.log_executions`, or run at `verbose`, so runs of
high-risk apps are recorded at all.

### Personal data

`audit.log_ip` is false by default. When enabled, request audit records and
synchronous domain hooks can include a syntactically valid client IP from the
authentication middleware. Set `audit.trusted_proxy_hops` to the number of reverse
proxies under the deployment's control that append to `X-Forwarded-For`; the
recorded address is then taken that many entries from the right, which a client
cannot forge, and `X-Real-Ip` is ignored. Unset, the leftmost entry is recorded,
which the client chooses.

A record's hash covers a salted commitment to its IP and to its `details`, not the
values themselves. The worker removes an IP after `ip_days` and details after
`details_days`, and leaves the commitments. Verification then counts the value
under `redacted_values` and the record still verifies; the UI marks it as
redacted. Archives never contain raw IPs, and contain raw details only while
`details_days` is unset. The actor id stays: it is the accountability record, and
a pseudonymous id that no longer resolves once the account is deleted. Document the
legal basis (GDPR art. 32 and art. 17(3)(e)) and the periods in the record of
processing activities.

Setting `details_days` also expires details in existing database records. The worker
repairs older seals' expiry flags in batches of at most 1,000 per tick, so a large
backlog takes several ticks to clear. Details already written to immutable archives
remain there until those objects expire.

### Keys

| Key | Variable | Held by | Used for |
| --- | --- | --- | --- |
| Entry key | `AUDIT_ENTRY_KEY`, base64 of 32 random bytes | Every API process and the audit worker | MAC of pending records, checked by the worker before sealing |
| Audit key | `AUDIT_SIGNING_KEY` (base64 of a PKCS#8 PEM P-256 private key) or `AUDIT_KMS_KEY_ID` | Only the processes that run the audit worker | Signs epochs, watermarks and archive manifests |
| Verifying keys | `AUDIT_VERIFYING_KEYS` | Every process that verifies | Public keys of audit keys the process does not hold |

Without `AUDIT_ENTRY_KEY` the entry key is derived from `BACKEND_KEY`, so every
process that shares `BACKEND_KEY` agrees on it. Without either, each process uses a
random key and only a worker in the same process can seal its records; records of
every other process are quarantined. With `audit.require_signing` the API refuses to
start in that case. Changing the entry key quarantines the records still pending at
that moment, so let the worker seal them first.

`AUDIT_KID` names the audit key. By default the id is `audit-es256-` followed by 16
hex digits of the public key's fingerprint, so a new key never reuses an id. A
process holding the key logs `audit signing key loaded` at startup with the key id
and the PEM public key. Add that pair to `AUDIT_VERIFYING_KEYS` of every process that
verifies without the key, and give it to anyone who verifies exports or archives
offline. The value is a JSON object mapping key ids to P-256 SPKI PEM public keys,
with JSON newlines inside each PEM string:

```json
{
  "audit-es256-0123456789abcdef": "-----BEGIN PUBLIC KEY-----\n<base64 public key>\n-----END PUBLIC KEY-----\n"
}
```

A rotated key needs a new id. Keep the old public key in `AUDIT_VERIFYING_KEYS` while
anything signed with it remains: epochs until they are pruned, archives until their
`retain_until`. A key id never maps to two different keys; a conflicting entry stops
startup. An epoch whose key id has no registered public key is counted in
`unverifiable_epochs` and makes the report invalid without proving tampering.

### Verification

`GET /api/v1/audit/verify?chain_id=<chain>` checks one chain from its watermark:
seal sequence and links, seal hashes and classes, each record's commitments and
hash against its seal's Merkle root, record counts and time ranges, epoch inclusion
proofs and epoch signatures, the MAC of every seal no epoch covers yet, and the MAC
of the newest 1,000 pending records. It
reads one batch of seals at a time in short, independent queries, so it never holds
a long snapshot. Each process remembers the last fully anchored seal it verified
per chain. Later checks recheck that seal and its records before reading newer seals;
`full=true`, open to everyone who may read the chain, rechecks from the watermark.
`GET /api/v1/audit/verify/epochs` checks continuity, hashes and signatures of the
epoch timeline and is limited to administrators. It also rechecks the last verified
epoch before reading newer epochs unless `full=true`. A failed check invalidates
cached progress, so later incremental checks reread retained history. Previously
verified boundaries remain in memory to detect a deleted tail on repeated checks.

| Field | Meaning |
| --- | --- |
| `valid` | Every check passed, every epoch signature verified and no pending record failed its MAC |
| `first_broken_seal`, `problem` | Sequence of the first seal that failed and what failed. Verification stops there |
| `seals_checked`, `records_checked` | What this run checked |
| `redacted_values` | Expired IPs and details whose commitments remain. Not tampering |
| `unanchored_seals` | Seals no epoch covers yet, protected by their MAC and normally signed within `epoch_interval_seconds` plus a minute. Stays above zero without an audit key |
| `unverifiable_epochs` | Epochs signed with a key id that has no registered public key |
| `pending_records` | Records waiting for a seal |
| `pending_invalid` | Pending records whose MAC fails, plus quarantined records |
| `pruned_before_seq` | Seals up to this sequence were archived or expired and pruned; the signed watermark verified |
| `latest_seal_seq`, `latest_epoch_seq` | The chain's newest seal and the newest epoch covering it |
| `checked_from_seq` | First sequence this run checked. Higher than the watermark means earlier seals were verified by an earlier run in the same server process; `full=true` starts again at the watermark |
| `held` | A seal of this chain failed its hash or MAC before it was signed. The worker signs, archives and prunes nothing of this chain until an operator resolves it |
| `empty` | No seal, pending record or watermark. A deleted chain looks the same unless the process remembers a verified boundary or the caller checks a retained head; also check `valid` |

`problem` names the failure, for example `seal 7 is missing`, `seal does not link to
its predecessor`, `seal holds 12 records but 11 remain`, `record <id> IP does not
match its commitment`, `records do not match the seal's root`, `seal is not part of
its epoch` or `epoch 42 signature is invalid`. `valid: false` without a
`first_broken_seal` points at pending records, quarantined records or a missing
public key.

A quarantined record was changed after it was written, inserted without the entry
key, or written by a process with a different entry key. It keeps status `invalid`,
is never sealed, and keeps its chain invalid through `pending_invalid`. Find its id
in the worker's error log and compare the entry keys of the API processes first.
Quarantined records are archived with their month and pruned with it.

### Heads

A database that lost its newest seals and epochs still verifies. To detect that,
keep a head outside the platform and compare it later:

- `GET /api/v1/audit/head?chain_id=<chain>` returns the chain's newest anchored
  seal and the signed epoch that anchors it. `seal.epoch_proof` proves the seal is
  part of the epoch.
- `POST /api/v1/audit/head/check` with the retained epoch's `seq` and hex `hash`
  answers `matches`, `differs` (the timeline was truncated or rewritten after the
  head was retained) or `archived` (the epoch was pruned; compare it with the
  monthly archive). Any authenticated user may check, because epoch hashes reveal
  nothing about content.
- The worker writes the newest epoch every day to `heads/YYYY/MM/DD.json` in the
  audit bucket, which the API cannot write and whose lock prevents removal.
  Deployments that need an independent time proof can timestamp these files with an
  RFC 3161 time-stamping authority.

### Records and the dashboard

`GET /api/v1/audit/records?chain_id=<chain>` lists records newest first, including
pending ones. It filters by `action` (a trailing `*` matches a prefix), `actor_id`,
`resource_type` and `resource_id`, pages with the returned `next` cursor
(`before_ms`, `before_id`) and returns at most 500 records per page, 50 by default.
Each record has a `status` of `pending`, `sealed` or `invalid`, and `ip_redacted`
and `details_redacted` flags for values that expired. A record whose seal is not in
its chain reads as `invalid`, never as sealed. The filters are not indexed, so a
filtered page looks at 30 days of the chain at a time: a page may come back empty with
a `next` cursor, which continues further back.

`GET /api/v1/admin/logs/chain-status` feeds the dashboard: the key id this process
signs with, the registered verifying key ids, the epoch report and the time of the
latest epoch, pending records and the age of the oldest, quarantined records,
unanchored seals, record and seal totals (estimates where exact counts would scan),
the latest archive, the legacy entries left to export, the platform chain's report,
the most recently sealed chains, and the `pending_alert_seconds` and
`epoch_interval_seconds` thresholds. It holds counts, hashes and ids only and needs
the ReadLogs permission. Unanchored seals older than `epoch_interval_seconds` plus a
few minutes, or an oldest pending record older than `pending_alert_seconds`, mean the
worker is not running or cannot sign.

### Legacy entries

Entries of the previous hash chain (`AuditEntry`) are not converted. With an audit
bucket, the worker exports up to 5,000 rows per run, raw, with one JSON object per
row. The first batch is `legacy/audit-entry.jsonl.zst`; later batches have distinct
names under `legacy/`. Each batch is recorded under archive period `legacy` before
the worker deletes only the ids included in that upload. Entries written by old API
replicas during a rolling upgrade join a later batch. A crash before deletion can
produce duplicate exports, which preserve the original entry ids. `legacy_entries`
in the chain status counts what is left. The export is a raw copy kept for the
retention period; the current verifier does not check the old hashes. Without an
audit bucket the rows stay in the database.

### Audit regression checks

Unit tests in `packages/api/src/audit` cover hashing and framing, commitments,
Merkle proofs, signatures, the line format and every worker step.
`packages/api/tests/audit_integrity.rs` runs the worker against a real PostgreSQL
database: concurrent writers on one chain, sealing and epochs, tampering with sealed
records, seal order and epoch signatures, quarantine of edited pending records,
expiry reported as redaction, the high-risk activity window, archiving a closed
month and pruning it behind a watermark, export pages verified offline, and the
legacy export.

```sh
AUDIT_TEST_DATABASE_URL=postgresql://postgres:postgres@localhost:5432/audit_test \
  cargo test -p flow-like-api --test audit_integrity -- --ignored
```

The tests create the audit tables in an empty database and refuse one that already
has them, so use a new database for every run. Never point them at a production
database.

## Related

- [Platform administration](/dev/platform-administration/)

