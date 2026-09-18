// Mirror of `validate_widget_csp_reason` / `fold_widget_csp_reason` in
// packages/wasm/schema/src/widget_policy.rs and of reason rule 8 in
// packages/wasm/schema/src/widget_sources/text.rs. Driven by the reason
// sections of packages/wasm/schema/tests/fixtures/widget_csp.json and
// widget_source_classification.json.

import { isIcannTld } from "./psl";

export type WidgetCspReasonRejection =
	| "reason-empty"
	| "reason-not-nfc"
	| "reason-forbidden-character"
	| "reason-whitespace"
	| "reason-length"
	| "reason-too-few-letters"
	| "reason-mixed-script"
	| "reason-contains-address"
	| "reason-mentions-product"
	| "reason-claims-assurance"
	| "reason-claims-attribution"
	| "reason-duplicate";

export const CSP_REASON_REJECTION_MESSAGES: Readonly<
	Record<WidgetCspReasonRejection, string>
> = {
	"reason-empty": "reason is empty",
	"reason-not-nfc": "reason must be NFC-normalized",
	"reason-forbidden-character":
		"reason may only contain letters, marks (at most 2 in a row), numbers, spaces and , . : ; ( ) - ' / &",
	"reason-whitespace":
		"reason must not start or end with a space or contain two spaces in a row",
	"reason-length": "reason must be 8 to 120 characters long",
	"reason-too-few-letters": "reason must contain at least 3 letters",
	"reason-mixed-script":
		"reason mixes ASCII letters with letters of another script in one word",
	"reason-contains-address":
		"reason must not contain web addresses, email addresses or domain names",
	"reason-mentions-product": "reason must not mention Flow-Like",
	"reason-claims-assurance":
		"reason must not claim that sources are verified, trusted, approved, official, certified, secure or safe",
	"reason-claims-attribution":
		"reason of a purpose with inputs must not claim who provides the addresses",
	"reason-duplicate": "reasons of two purposes must differ",
};

/** `<message> (<code>)`, the form Rust uses inside contract errors. */
export function cspReasonProblem(rejection: WidgetCspReasonRejection): string {
	return `${CSP_REASON_REJECTION_MESSAGES[rejection]} (${rejection})`;
}

const MIN_CHARS = 8;
const MAX_CHARS = 120;
const MIN_LETTERS = 3;
const MAX_MARK_RUN = 2;
const PUNCTUATION = new Set([",", ".", ":", ";", "(", ")", "-", "'", "/", "&"]);
const PRODUCT = "flowlike";
const ASSURANCE_WORDS = new Set([
	"verified",
	"trusted",
	"approved",
	"official",
	"certified",
	"secure",
	"securely",
	"safe",
	"safely",
]);
const ATTRIBUTION_WORDS = new Set([
	"app",
	"apps",
	"application",
	"admin",
	"admins",
	"administrator",
	"administrators",
	"organization",
	"organisation",
	"company",
	"workspace",
	"project",
	"owner",
]);
const IDEOGRAPHIC_FULL_STOP = "\u3002";

const ALPHABETIC = /^\p{Alphabetic}$/u;
const MARK = /^\p{M}$/u;
const NUMERIC = /^\p{N}$/u;
const LONE_SURROGATE = /\p{Cs}/u;

const isAlphabetic = (c: string) => ALPHABETIC.test(c);
const isMark = (c: string) => MARK.test(c);
const isNumeric = (c: string) => NUMERIC.test(c);
const isAscii = (c: string) => (c.codePointAt(0) ?? 0) < 0x80;
const isAsciiAlphanumeric = (c: string | undefined) =>
	c !== undefined && /^[A-Za-z0-9]$/.test(c);
const isLetterOrMark = (c: string) => isAlphabetic(c) || isMark(c);
const isLatinExtended = (c: string) => {
	const code = c.codePointAt(0) ?? 0;
	return (code >= 0xc0 && code <= 0x24f) || (code >= 0x1e00 && code <= 0x1eff);
};

function isJoinable(c: string | undefined): boolean {
	return c !== undefined && !isAscii(c) && isLetterOrMark(c);
}

function charactersAllowed(chars: readonly string[]): boolean {
	let markRun = 0;
	for (const [position, c] of chars.entries()) {
		if (isMark(c)) {
			markRun += 1;
			if (markRun > MAX_MARK_RUN) return false;
			continue;
		}
		markRun = 0;
		const joiner =
			(c === "\u200c" || c === "\u200d") &&
			position > 0 &&
			isJoinable(chars[position - 1]) &&
			isJoinable(chars[position + 1]);
		if (
			!(
				isAlphabetic(c) ||
				isNumeric(c) ||
				c === " " ||
				PUNCTUATION.has(c) ||
				joiner
			)
		) {
			return false;
		}
	}
	return true;
}

/** Reason rule 3: the character allowlist (TS also rejects lone surrogates). */
export function reasonCharactersAllowed(reason: string): boolean {
	return !LONE_SURROGATE.test(reason) && charactersAllowed(Array.from(reason));
}

function splitRuns(
	chars: readonly string[],
	keep: (c: string) => boolean,
): string[][] {
	const runs: string[][] = [[]];
	for (const c of chars) {
		if (keep(c)) runs[runs.length - 1]?.push(c);
		else runs.push([]);
	}
	return runs;
}

/** Reason rule 7: a letter run mixing ASCII and non-Latin letters. */
export function reasonHasMixedScript(reason: string): boolean {
	return splitRuns(Array.from(reason), isLetterOrMark).some(
		(run) =>
			run.some((c) => isAscii(c) && isAlphabetic(c)) &&
			run.some((c) => isAlphabetic(c) && !isAscii(c) && !isLatinExtended(c)),
	);
}

function replaceIdeographicStops(chars: readonly string[]): string[] {
	return chars.map((c, position) =>
		c === IDEOGRAPHIC_FULL_STOP &&
		isAsciiAlphanumeric(chars[position - 1]) &&
		isAsciiAlphanumeric(chars[position + 1])
			? "."
			: c,
	);
}

/**
 * `f` of spec §14.2.4 as `fold_widget_csp_reason`: NFKC, U+3002 between ASCII
 * alphanumerics read as `.`, then lowercase.
 */
export function foldWidgetCspReason(reason: string): string {
	return replaceIdeographicStops(Array.from(reason.normalize("NFKC")))
		.join("")
		.toLowerCase();
}

function reasonWords(folded: string): string[] {
	return splitRuns(
		Array.from(folded),
		(c) => isAlphabetic(c) || isNumeric(c) || isMark(c),
	)
		.filter((run) => run.length > 0)
		.map((run) => run.join(""));
}

/**
 * Reason rules 1–7 and 9–11 of §14.2.4, in order; the first failure is the
 * code. Rule 8 ({@link reasonContainsAddress}) and rule 12 (duplicates across
 * purposes) run separately.
 */
export function validateWidgetCspReason(
	reason: string,
	hasInputs: boolean,
): WidgetCspReasonRejection | null {
	if (reason.length === 0) return "reason-empty";
	if (reason.normalize("NFC") !== reason) return "reason-not-nfc";
	if (!reasonCharactersAllowed(reason)) return "reason-forbidden-character";
	if (reason.startsWith(" ") || reason.endsWith(" ") || reason.includes("  ")) {
		return "reason-whitespace";
	}
	const chars = Array.from(reason);
	if (chars.length < MIN_CHARS || chars.length > MAX_CHARS) {
		return "reason-length";
	}
	if (chars.filter(isAlphabetic).length < MIN_LETTERS) {
		return "reason-too-few-letters";
	}
	if (reasonHasMixedScript(reason)) return "reason-mixed-script";
	const folded = foldWidgetCspReason(reason);
	const lettersAndNumbers = Array.from(folded)
		.filter((c) => isAlphabetic(c) || isNumeric(c))
		.join("");
	if (lettersAndNumbers.includes(PRODUCT)) return "reason-mentions-product";
	const words = reasonWords(folded);
	if (words.some((word) => ASSURANCE_WORDS.has(word))) {
		return "reason-claims-assurance";
	}
	if (hasInputs && words.some((word) => ATTRIBUTION_WORDS.has(word))) {
		return "reason-claims-attribution";
	}
	return null;
}

/**
 * Reason rule 8 `reason-contains-address` as `reason_contains_address`: a
 * URL scheme, `@`, `www.`, or a dotted token whose last label has at least two
 * characters and is non-ASCII or an ICANN TLD. Publish and bundler only.
 */
export function reasonContainsAddress(reason: string): boolean {
	const folded = replaceIdeographicStops(
		Array.from(reason.normalize("NFKC").toLowerCase()),
	).join("");
	if (
		folded.includes("://") ||
		folded.includes("@") ||
		folded.includes("www.")
	) {
		return true;
	}
	return splitRuns(
		Array.from(folded),
		(c) => isLetterOrMark(c) || isNumeric(c) || c === "-" || c === ".",
	).some((run) => {
		const labels = run
			.join("")
			.replace(/^\.+|\.+$/g, "")
			.split(".");
		const last = labels[labels.length - 1] ?? "";
		return (
			labels.length >= 2 &&
			Array.from(last).length >= 2 &&
			(!Array.from(last).every(isAscii) || isIcannTld(last))
		);
	});
}

/** Normalization applied to authored reasons: NFC, collapsed whitespace, trimmed. */
export function normalizeCspReason(reason: string): string {
	return reason.normalize("NFC").replace(/\s+/g, " ").trim();
}
