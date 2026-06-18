// Processing — FlowScript node declarations (generated, do not edit).
// One declare-function per catalog node. Names are camelCase node types.

// === Processing/Privacy ===

/**
 * Configure which PII types to detect. Connect to PII Mask nodes for fine-grained control.
 * @param email (optional) — Detect email addresses (e.g., user@example.com)
 * @param phone (optional) — Detect phone numbers (international formats)
 * @param url (optional) — Detect URLs and web addresses
 * @param ipAddress (optional) — Detect IPv4 and IPv6 addresses
 * @param creditCard (optional) — Detect credit card numbers (13-19 digits)
 * @param iban (optional) — Detect IBAN bank account numbers (international)
 * @param vatEu (optional) — Detect EU VAT numbers
 * @param ssn (optional) — Detect US Social Security Numbers (XXX-XX-XXXX)
 * @param germanTaxId (optional) — Detect German Steuer-ID (11 digits)
 * @param ahvSwiss (optional) — Detect Swiss AHV numbers (756.XXXX.XXXX.XX)
 * @param svnrAustria (optional) — Detect Austrian social insurance numbers
 * @param passport (optional) — Detect passport numbers (various formats)
 * @param driversLicense (optional) — Detect driver's license numbers (basic patterns)
 * @param addressUs (optional) — Detect US street addresses
 * @param addressDe (optional) — Detect German addresses (Straße, Platz, Weg, etc.)
 * @param postcodeUk (optional) — Detect UK postcodes
 * @param postcodeDe (optional) — Detect German postcodes (5 digits)
 * @param date (optional) — Detect date patterns (DD/MM/YYYY, YYYY-MM-DD, etc.)
 * @returns options — PII Detection Options configuration struct
 */
declare function processingPiiDetectionOptions({ email?: bool, phone?: bool, url?: bool, ipAddress?: bool, creditCard?: bool, iban?: bool, vatEu?: bool, ssn?: bool, germanTaxId?: bool, ahvSwiss?: bool, svnrAustria?: bool, passport?: bool, driversLicense?: bool, addressUs?: bool, addressDe?: bool, postcodeUk?: bool, postcodeDe?: bool, date?: bool }): Struct;

/**
 * Masks Personally Identifiable Information using regex patterns. Detects emails, phones, SSNs, credit cards, IBANs, addresses (US/DE/UK), and more. For names or contextual PII, use the AI-based node.
 * @param text (optional) — The text to scan for PII
 * @param options (optional) — Configuration for which PII types to detect. Connect a PII Detection Options node or use defaults (all enabled).
 * @param detectEmail (optional) — Override: Enable/disable email detection
 * @param detectPhone (optional) — Override: Enable/disable phone number detection (international)
 * @param detectCreditCard (optional) — Override: Enable/disable credit card detection
 * @param detectIban (optional) — Override: Enable/disable IBAN detection
 * @param detectAddress (optional) — Override: Enable/disable address detection (US and DE)
 * @param detectSsn (optional) — Override: Enable/disable SSN and tax ID detection
 * @param detectUrl (optional) — Override: Enable/disable URL detection
 * @param detectIp (optional) — Override: Enable/disable IP address detection
 * @param maskChar (optional) — Character used for masking (default: *)
 * @param preserveLength (optional) — If true, mask preserves original length. If false, uses mask text.
 * @param maskText (optional) — Text to use when preserve_length is false (default: [REDACTED])
 * @returns maskedText — Text with PII masked
 * @returns detectionCount — Number of PII instances detected and masked
 * @returns detections — JSON array with detection details (type, position, length)
 * @impure has side effects / drives control flow
 */
declare function processingPiiMaskRegex({ text?: string, options?: Struct, detectEmail?: bool, detectPhone?: bool, detectCreditCard?: bool, detectIban?: bool, detectAddress?: bool, detectSsn?: bool, detectUrl?: bool, detectIp?: bool, maskChar?: string, preserveLength?: bool, maskText?: string }): { maskedText: string, detectionCount: int, detections: Struct };

