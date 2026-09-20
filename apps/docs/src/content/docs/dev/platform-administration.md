---
title: Platform administration
description: Operate the audit trail and manage starter profiles and published home layouts
---

Platform administrators manage shared profile defaults and use audit records
to investigate changes. These features require an online backend, including
when their administration screens are opened in the desktop app.

## Audit trail

Operators can use the audit trail to identify who requested a change, inspect its
result, and check whether the stored records still match their hashes and server
signatures. Platform administrators read the root chain. App owners can read their
app's branch through `/api/v1/audit/entries?chain_id=<app-id>`.

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

The level applies when an entry is written. An action that is not recorded at
the configured level leaves no gap in the chain: sequences stay contiguous and
verification is unaffected. Actions the level classifier does not know are
recorded at `standard`. The classification lives in
`packages/api/src/audit/level.rs` and its test lists every action name.

### Coverage and failure behavior

At the `verbose` level, authenticated POST, PUT, PATCH and DELETE requests
record `api.request.attempt` before dispatch and `api.request.finish` when the
handler produces response headers. Both entries share a request ID. The finish
entry records the HTTP status and the number of failed domain audit writes.
App-scoped routes put these records on the app chain when the caller holds a role
in that app. A caller without one is recorded on the root chain with the requested
app in `details.requested_app_id`, so naming an app in a path cannot write into or
create its chain. Other routes use the root chain. Telemetry ingestion is excluded. At lower levels the request context is
still established so domain hooks record the actor IP and a failed domain
write still marks the response as described below.

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
`verbose` its finish entry carries `domain_audit_outcome: "unknown"`.
Operators should investigate attempts without a finish and finishes with a
nonzero `domain_audit_failures`. Domain changes and their audit entries are not
one atomic transaction. A crash can leave an attempt with an unknown outcome.
Execution state updates have the same boundary. Callback retries can repair a
missing terminal entry, but background crashes have no durable outbox recovery.

With `audit.log_executions` enabled or at the `verbose` level, persisted run
transitions record starts, completion, failure, cancellation, timeout and
admission rejections on the app chain. Repeated terminal callbacks reuse the
first record for that run and action. Streaming and background work record
their execution outcomes separately from HTTP response status. Run details
remain available in the execution run index regardless of this setting.

Anonymous requests and authentication failures are outside the mutation middleware's
coverage. Inbound and sink execution paths use explicit lifecycle hooks. This
trail does not capture offline desktop edits or every direct storage operation.

### Integrity and compatibility

New entry hashes start with `v2:`. They use a canonical JSON encoding that preserves
field boundaries and covers the record ID, sequence, timestamp, actor identity and
type, optional IP, action, resource, chain scope, summary, details, previous hash,
previous signature and signing key ID. P-256 ECDSA signs the resulting BLAKE3 hash.
Timestamps are normalized to the database's millisecond precision before hashing.

Entry timestamps are taken under the chain lock and are never earlier than the
entry they link to, so sequence order and time order agree within a chain and with
a branch's root anchor. U+0000 in any stored string is replaced with U+FFFD before
hashing, because PostgreSQL rejects it and the entry for an already committed
mutation must not fail on hostile text.

Writers serialize through a retained `MutationLock` row before reading a chain's
tail. An append waits at most three seconds for that row on PostgreSQL and
CockroachDB, retries lost commit races for up to ten seconds, and gives up after
fifteen. An exhausted budget is logged as `audit append exhausted its retry budget`. This also coordinates the first append and the root chain, whose nullable
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
Verification streams the chain in batches of 1,000 inside that snapshot, so its
memory use does not grow with the chain. The result also reports `empty` when the
chain or range holds no entries, and `anchor_sequence`, the root sequence a branch
is anchored to. `GET /api/v1/audit/entries` pages with `before_sequence`; `offset`
is capped at 10,000.

The dashboard automatically verifies root chains with at most 1,000 entries;
larger chains and all branch chains show "Signed, not checked" or "Unsigned, not
checked" until an operator requests verification in the chain explorer.

`valid` means the requested verification checks succeeded. `fully_authenticated`
additionally requires signed v2 entries with available verification keys. Legacy
entries retain their original hash algorithm; its omitted metadata and ambiguous
field boundaries cannot be repaired retroactively. They are counted as legacy
and do not qualify as fully authenticated. Unsigned entries are counted separately.
An unsigned entry that follows a signed entry or a signed anchor is reported as
broken at its sequence: entry hashes need no key, so stripping signatures is what
a rewrite without the signing key looks like. A chain that starts unsigned and
later becomes signed stays valid. An unavailable historical public key causes verification to fail with an
`unverifiable_signatures` count. It does not by itself prove that a record changed.

Verification cannot establish events that were never recorded. Detecting deletion
of a whole chain or its final records requires a previously retained checkpoint
outside this database. `GET /api/v1/audit/head` returns a chain's newest sequence,
hash, signature and key id. Store it outside the platform and pass it back as
`expected_head_sequence` and `expected_head_hash` to `/audit/verify`: a missing or
different entry at that sequence marks the chain broken there. A valid subrange also does not certify all earlier history.

### Signing keys and IP addresses

`AUDIT_SIGNING_KEY` supplies a base64-encoded P-256 PKCS#8 PEM key used only for
audit entries and `AUDIT_KID` identifies it; without `AUDIT_KID` the id is derived
from the key's fingerprint. Prefer it: every component that issues backend tokens
holds `BACKEND_KEY`, and whoever holds the audit key can re-sign rewritten history.
Without `AUDIT_SIGNING_KEY` the trail falls back to `BACKEND_KEY` and `BACKEND_KID`.
Switching to the dedicated key is a rotation: retain the old public key as
described below. With `audit.require_signing` true, API startup refuses
to proceed without a usable signing key. The checked-in configuration requires it.

A rotated key needs a new key id. At startup the API verifies the newest root
entry signed under the current id with the configured key; a mismatch means the
key changed under the same id, or a replica holds a different key. With
`audit.require_signing` the API refuses to start, otherwise it logs
`AUDIT SIGNING KEY MISMATCH`.

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
authentication middleware. Set `audit.trusted_proxy_hops` to the number of reverse
proxies under the deployment's control that append to `X-Forwarded-For`; the
recorded address is then taken that many entries from the right, which a client
cannot forge, and `X-Real-Ip` is ignored. Unset, the leftmost entry is recorded,
which the client chooses. Signed IP
fields are immutable. `ip_retention_days` does not currently erase them, so keep
IP recording disabled if automatic IP expiry is required.

### Audit regression checks

Unit tests cover framing, metadata tampering, signature failures, missing sequence
boundaries, legacy compatibility and middleware failure behavior. The PostgreSQL
integration tests in `packages/api/tests/audit_integrity.rs` use a disposable
database to check concurrent appends, retries, signed branches and persisted values.
They must not be pointed at a production database.

## Starter profiles

Administrators can curate the profiles offered to new users at `/admin/profiles` in the web and desktop apps. The old `/admin/user` and `/admin/user/edit` routes open the same manager.

The manager supports searching, sorting, creating, editing, duplicating, and deleting templates. The editor has a live profile preview and sections for the name, description, icon, cover, tags, interests, bits, apps, hubs, and flow connection style. Included apps retain their favorite and pin settings. Editing a template does not rewrite existing user profiles. Unsaved drafts survive navigation within the running app, scoped to the backend, account, and template. Saving or explicitly discarding clears the draft; reloading the app clears this memory cache.

Each saved template links to `/admin/home?default=<template-id>`. Its home follows the main default until an administrator publishes a template default. Duplicating a template copies its presentation and starting configuration, while its new home initially follows the main default. Personal layouts, shortcuts, and private bits are not copied. A published template home must be reset to follow the main default before deleting its template.

Both apps use the shared editor. Desktop supplies a native HTTP upload function; the browser uses Fetch. Image uploads accept PNG, JPEG, and WebP up to 10 MiB and 4096 pixels per edge. Images are converted to WebP when the browser can encode it, with a PNG fallback for other engines, keeping their proportions, with a longest edge of 512 pixels for icons or 1600 pixels for covers. Administrators can also supply an HTTP(S) image URL. Failed uploads keep the previous image.

`WriteProfile` controls template edits and media uploads. `ReadProfile` allows browsing, and `WriteLandingPage` separately controls default homes. These administration operations require a reachable backend and its permissions, including in the desktop app. Desktop users can still use their local profiles without signing in.

Image upload URLs accept `?format=webp`, `png`, or `jpeg` and use the corresponding extension; requests without a format keep the existing WebP behavior. Template writes use the requested ID and return the saved profile. Validation bounds names to 120 characters, descriptions to 10,000 characters, bits and apps to 500 entries each, and tags, interests, and additional hubs to 50 entries each. Stored apps, settings, and theme values survive template round trips. Apply the home-layout migration below before using published defaults.

### Profile editor checks

Run the focused frontend checks with `bun test packages/ui/components/profile-templates`. The responsive browser fixture and verification script live in `tests/home-browser`.

The browser check uses the real list and editor with a local fixture backend. It verifies saved fields, uploads, search, failure recovery, draft restoration, duplication, deletion, and layouts at 390, 768, and 1480 pixels. Run it with `node tests/home-browser/profile-verify.mjs` while the fixture server is running. PNG encoding fallback is simulated in Chrome; this check does not upload to configured storage or run inside the native WebKit view.

## Deploy editable home layouts

Apply the home layout schema before starting an API build that reads the new profile fields. The migration adds nullable `homeLayout` and `homeDefaultId` columns to `Profile`, plus a `HomeDefault` table for published layouts. Existing profile layouts remain unset and follow the published default.

The Aurora DSQL deployment migration job applies `migrations-dsql/20260905103000_profile_home_layouts/migration.sql` through its migration ledger.

For an existing PostgreSQL installation that has not applied the schema change, run this command from `packages/api` with `DATABASE_URL` pointing to that installation:

```sh
psql "$DATABASE_URL" -v ON_ERROR_STOP=1 -f prisma/migrations/20260905103000_profile_home_layouts/migration.sql
```

Apply the migration once. Fresh PostgreSQL installations can use the existing `db:push` workflow, which reads the updated Prisma schema. If an existing installation uses `db:push` instead of the SQL migration, run the migration's final `UPDATE` separately to associate existing profiles with matching template IDs.

The backfill associates a profile with a template only when their IDs match. Profiles created by copying another profile may have a different ID and no recoverable template reference. New template installations carry an explicit reference, and new copies retain it.

After deployment, an administrator with `WriteLandingPage` or `Admin` permission can publish the main default and optional template defaults. Publishing checks the revision loaded when editing began. A conflicting publication returns HTTP 409 so the editor can retain the draft and ask the administrator to reload.

Resetting a user's home clears only `homeLayout`; `homeDefaultId` remains intact.
See [the home layout guide](/start/home/) for the default selection order and
user controls. Removing a template default restores the main default; removing
the main default restores the bundled layout.

Desktop custom layouts are saved locally and synchronized with the profile. Older clients that omit the home fields in partial API writes leave them unchanged. Default configuration contains widget settings, while each viewer loads app data with their own access permissions.

## Payments rollout and recovery

Payments are disabled by default. The implementation supports app purchases through
Stripe destination charges and attended flow payments through direct charges for
independent sellers. Admin-owned apps use platform charges for both products. Both
use hosted Checkout. Cards include eligible Apple Pay and Google Pay presentation;
Link is also allowed by default. Additional wallets belong in the relevant
`payments.marketplace_payment_methods` or `payments.node_payment_methods` list after
account eligibility and refund behavior have been checked. PayPal is accepted only
in the marketplace configuration. The node configuration excludes delayed payment
methods such as SEPA Direct Debit.

Apps owned by a user with the global Flow-Like `Admin` permission collect both
marketplace purchases and Request Payment node charges directly in the platform's
Stripe account. The entire payment stays with Flow-Like, subject to Stripe fees,
tax, refunds and disputes. An app-level administrator role does not select this
route. Global administrators cannot start or resume connected-account onboarding
or reconnect a personal payment account. Their payment screen identifies
Flow-Like as the recipient and shows the platform balance only while they retain
the global permission.

Each payment keeps the recipient selected when it was created. Changing the
owner's global admin permission cancels pending payments and requires a fresh
checkout on the new route. Captured payments, receipts, refunds and disputes keep
their original Stripe account. Historical connected accounts remain available to
the recovery worker; changing a role does not delete or move their funds.

Apply `20260920120000_payments_foundations` followed by
`20260920130000_platform_owned_payments` before deploying this API. The second
migration permits a missing connected-account ID for platform-owned payments and
preserves the recipient of every existing payment. Both PostgreSQL and DSQL
migration trees retain financial rows when their user, app or package is deleted.
Existing platform purchases keep their original financial identity. New marketplace checkout refuses an overlapping
legacy checkout until that session has been resolved.

Before cutover, run `bun packages/api/scripts/payments-inventory.ts` with a
read-only `DATABASE_URL`. Save its JSON output outside the source tree for review.
It reports priced listings, ambiguous owners, duplicate provider references,
paid purchases without access and totals by currency. Review those records before
repairing historical grants. The inventory does not move legacy revenue to sellers.

The selected marketplace model makes Flow-Like the merchant of record. Configure
`marketplace_tax_mode` as `platform_supplier`, with Stripe Tax registration and the
appropriate `product_tax_code`. Platform-owned purchases retain their tax amount
on the platform; independent-seller purchases recover that amount from the
destination transfer. Checkout calculates tax within the displayed price, collects
the billing address and creates an invoice issued by Flow-Like. Refund recovery
links each successful refund to a credit note and uses Stripe's tax amounts,
including its rounding for partial refunds. A missing or incomplete tax
calculation holds delivery for reconciliation rather than treating the tax as zero.

For flow payments, `node_tax_mode=seller_supplier` applies to independent owners.
Admin-owned requests record `platform_supplier` because Flow-Like receives the
payment. Each Request Payment node must supply the Stripe `productTaxCode` for
the actual product or service it sells. The node calculates tax within its exact
requested total and creates an invoice in the receiving account. Refund credit
notes use that same account, including after ownership or permission changes.
The marketplace tax code is not a default for arbitrary node products. For
physical goods, set `shippingCountries` to collect a delivery address in the
countries the merchant serves; digital products do not require that field.

Before opening a taxed checkout, the API reads Stripe Tax settings and checks for
an active registration in the liable account and the matching test or live mode.
It also retrieves the selected tax code from Stripe. These checks catch missing
setup, but an active registration in one jurisdiction does not establish coverage
in every market. The merchant must determine where registration is required,
register there and add those registrations to Stripe Tax. Stripe only collects
tax where an active registration applies. A successful zero-tax calculation can
therefore mean that no registration applies, as well as a legitimate exemption.
See [Stripe Tax setup](https://docs.stripe.com/tax/checkout/page) and
[reporting and filing](https://docs.stripe.com/tax/reports).

Stripe's EU **Small Seller** option applies home-country VAT to qualifying
cross-border consumer sales. It is separate from a VAT exemption. The relevant
cross-border sales must total no more than EUR 10,000 excluding VAT in both the
current and preceding calendar year, across the business's sales channels. Other
conditions include establishment in only one EU member state and no election to
apply customer-country taxation. Confirm eligibility with the merchant's tax
adviser and monitor the combined threshold. This consumer-sales rule does not
determine B2B reverse-charge treatment. See the
[EU threshold rules](https://vat-one-stop-shop.ec.europa.eu/one-stop-shop_en) and
[Stripe's small-seller configuration](https://docs.stripe.com/tax/supported-countries/european-union#small-sellers).

Checkout does not currently collect business tax IDs. Automatic B2B VAT
reverse-charge handling is not available through this flow. Stripe's Checkout
tax-ID field verifies format during checkout and can apply reverse charge before
asynchronous validity checks complete; enabling it requires a process for invalid
IDs and corrected invoices. See
[Stripe's tax-ID validation](https://docs.stripe.com/tax/checkout/tax-ids#validation).

Stripe Tax also limits wallet presentation. Google Pay requires a collected
shipping address or an existing customer's saved shipping address. Billing-only
digital checkout can therefore omit Google Pay even when cards and wallets are
enabled. Do not collect a fictitious shipping address to display a wallet. Apple
Pay, Link and other eligible methods remain subject to Stripe account, device and
country support.

The `payments` object in the hub configuration separates creation and servicing:

| Setting | Purpose |
| --- | --- |
| `onboarding_enabled` | Allow account owners to start Stripe onboarding. |
| `marketplace_enabled` | Allow new app marketplace orders. |
| `node_payments_enabled` | Allow attended remote runs to request payments. |
| `servicing_enabled` | Process new payment webhooks, refunds and reconciliation. Leave this enabled when disabling creation. |
| `livemode`, `platform_account_id` | Bind operations and webhook receipts to the expected Stripe environment and platform. |
| `seller_allowlist` | Optional restriction to specific Flow-Like user IDs. Omit it or leave it empty for open enrollment. Authentication, ownership, terms, Stripe readiness and seller blocks still apply. |
| `countries`, `currencies` | Restrict seller countries and payment currencies to configured markets. The initial currency is EUR. |
| `node_tax_mode`, `marketplace_tax_mode`, `live_approved` | Record the approved seller, tax and commercial configuration. There is no live tax default. |
| `frontend_url` | Build payment and onboarding returns on a configured HTTPS origin. Test mode also permits localhost HTTP. |
| `legal_texts` and the terms version fields | Supply the exact localized owner, seller and buyer terms accepted by users. Draft terms are blocked in live mode. |
| `legacy_checkout_until` | Stop new legacy checkout at an epoch-millisecond deadline while continuing historical settlement. Enabling the marketplace also stops legacy creation. |

Before live activation, exercise the exact account controller and charge model in
Stripe test mode, verify the approved terms and tax handling, and confirm payment
recovery is running. The supplied legal texts remain drafts and `live_approved`
remains false until their commercial details and legal review are complete.
Test taxable and zero-tax buyers, incomplete locations, invoice creation, full and
partial refunds, and account scope for both independent and admin-owned apps.
Onboarding changes require a recent OIDC `auth_time`; a token
without that verified claim cannot perform those changes. Owners explicitly enable
flow payments and set limits for each app. An ownership transfer resets those
opt-ins. Native iOS and Android distributions do not expose payment creation.

Keep Stripe keys in the server secret store. Configure these signing secrets for
separate webhook endpoints using API version `2026-08-26.dahlia` for the new endpoints:

| Secret | Endpoint under `/api/v1` |
| --- | --- |
| `STRIPE_WEBHOOK_SECRET` | `/webhook/stripe`, for existing billing and legacy purchases. |
| `STRIPE_CONNECT_WEBHOOK_SECRET` | `/webhook/stripe/connect`, for connected-account events and direct payments. |
| `STRIPE_MARKETPLACE_WEBHOOK_SECRET` | `/webhook/stripe/marketplace`, for platform marketplace, admin-owned flow payments and application-fee events. |

The API also reads `/v1/tax/settings`, `/v1/tax/registrations` and
`/v1/tax_codes/{id}` before opening a taxed checkout. A restricted Stripe key must
permit those reads in the platform account and the connected accounts it serves.
Invoice and credit-note reconciliation needs invoice reads, credit-note listing
and previews, and credit-note creation, in addition to the existing Checkout,
Connect and refund permissions. This integration never creates tax registrations;
recording a registration in Stripe must follow the merchant's actual registration.

The new signing secrets accept a matching `_PREVIOUS` secret during rotation.
Ingress verifies the signature before persisting an event. Unsupported events are
ignored, and wrong-mode, wrong-scope or wrong-version events are quarantined.
Subscribe to Checkout completion, asynchronous success/failure and expiration;
PaymentIntent success/failure; charge updates/refunds/disputes; refund lifecycle;
transfer lifecycle; and application-fee events. The Connect endpoint also receives
account updates and authorization removal. Returning from Stripe never grants
access or establishes account readiness.

Local, Compose, Kubernetes, AWS ECS, Azure and GCP API processes start a payment
worker when payments servicing or legacy premium billing is enabled. Stateless
installations must schedule `POST /api/v1/maintenance/run` with
`{"job":"payments"}` and the dedicated `MAINTENANCE_TOKEN` bearer credential, at
least once per minute. The AWS, Azure and GCP maintenance runners accept the
`payments` job. Their cloud schedules are provisioned outside this repository;
verify that the schedule exists before enabling creation. Daily maintenance alone
is insufficient for interactive payment expiry.

Administrators inspect `/api/v1/admin/payments/queue?kind=operations`, `events` or
`effects` for pending and suspended work. A provider timeout can have an unknown
outcome. Recovery repeats only the original persisted command within its safe
idempotency window; ambiguous provider failures and older operations require
review. Never replace their idempotency key to force another charge. Suspended
outbox effects can be resumed through the scoped admin endpoint after the cause is
resolved; this does not reset the underlying Stripe operation.

Route the error log `Payment recovery requires attention` with target `payments`
to the payment support owner before enabling checkout. The worker checks once per
minute for ambiguous operations, quarantined or suspended work, queue items older
than five minutes and orphan captures still awaiting refunds. Logs contain the
queue, count and oldest age. Use the admin queue to investigate individual items.
Webhook storage keeps object references and recovery hints; canonical retrieval
supplies financial state without retaining customer or bank details from events.

A local disconnect stops new payments and queues cancellation while retaining
historical servicing. Removal of Stripe authorization can block direct refunds.
Platform-scoped marketplace refunds continue independently. Purchase history,
source-specific access grants, refund reservations and ledger entries remain
available for reconciliation. Account balance views include other activity on the
seller's Stripe account; they are not a Flow-Like earnings total.
