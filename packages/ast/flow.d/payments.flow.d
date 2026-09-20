// Payments — FlowScript node declarations (generated, do not edit).
// One `function` per catalog node, grouped by FlowScript namespace. Call a node as
// `ns::alias({ pin: value })`, or write `use ns::*` once at the top of a .flow file and
// call `alias({ pin: value })`. A `this: T` parameter marks the receiver pin: such a node
// is also a method on that value (`x.alias(...)`, remaining inputs positional or named).
// JSDoc tags carry the node type (`@node`), the receiver pin (`@receiver`) and the legacy
// camelCase spelling (`@alias`), which is still accepted.

declare namespace payments {
    // === Payments ===

    /**
     * Ask the signed-in user to pay the app owner and wait for the verified result
     * @node request_payment @alias requestPayment
     * @param currency (optional) — Supported three-letter currency
     * @param productName (optional) — Plain-text product name
     * @param description (optional) — Plain-text description
     * @param productTaxCode (optional) — Required Stripe tax code for the product or service, selected from Stripe's tax code list
     * @param shippingCountries (optional) — For shipped goods, comma-separated delivery country codes such as DE,FR. Leave empty when no delivery address is needed.
     * @param reference (optional) — Optional application reference
     * @param simulation (optional) — Local Board Test only: paid, canceled, expired or failed. Empty requests a real payment.
     * @param amountMinor (optional) — Total including applicable tax in integer minor units, for example 119 for EUR 1.19
     * @param ttlSeconds (optional) — Requested timeout in seconds, limited by the remaining run quota
     * @returns paymentId — Server payment request identifier, or a sim_ identifier in Board Test
     * @returns reason — Machine-readable outcome reason
     * @impure has side effects / drives control flow
     */
    function request({ currency?: string, productName?: string, description?: string, productTaxCode?: string, shippingCountries?: string, reference?: string, simulation?: string, amountMinor?: int, ttlSeconds?: int }): { paymentId: string, reason: string };
}
