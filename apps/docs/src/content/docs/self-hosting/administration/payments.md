---
title: Payments rollout and recovery
description: Configure payment rollout, servicing, and recovery for a hub
---

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

### Schema and cutover

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

### Tax configuration and boundaries

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

### Creation and servicing settings

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
| `legal_texts` and the terms version fields | Select the exact localized owner, seller and buyer terms by kind, version, locale, website URL and BLAKE3 content hash. Draft terms are blocked in live mode. |
| `legacy_checkout_until` | Stop new legacy checkout at an epoch-millisecond deadline while continuing historical settlement. Enabling the marketplace also stops legacy creation. |

### Terms and live activation

The official agreements live in `apps/website/src/content/legal/payments/`.
The website publishes each agreement at
`https://flow-like.com/legal/payments/{kind}/{version}/{locale}/`, where `kind`
is `owner-terms`, `seller-terms` or `purchase-terms`. Append `text.txt` to retrieve
the canonical UTF-8 text. The backend bundles the same source content and verifies
each configured `hash` against its exact bytes. It serves the selected agreement
through `/payments/terms`, so onboarding and checkout do not fetch the website.

Each `legal_texts` entry retains `kind`, `version` and `locale`, with `url` and
`hash` replacing the inline `text`. The hash is the lowercase hexadecimal BLAKE3
digest of the text, including whitespace. Add each new version file to the backend's
bundled legal texts in `packages/core/runtime/src/hub/payments.rs` and retain old
content at its original URL. Deploy the website content before activating
references in the backend configuration. The public hub response
excludes these entries; clients load terms only when needed.

Self-hosted operators can still supply inline `text` for their own agreements,
with an optional HTTPS `url` and matching `hash`. A URL alone does not make the
backend download arbitrary terms: references must resolve to bundled content,
or the entry must supply inline text. Missing content or a mismatched hash blocks
use of that agreement.

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

### Webhooks and credentials

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

### Recovery worker

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

## Related

- [Platform administration](/dev/platform-administration/)

