// Mirror of the source grammar in packages/wasm/schema/src/widget_policy.rs.
// Both implementations are driven by
// packages/wasm/schema/tests/fixtures/widget_csp.json.

import { domainToASCII } from "node:url";
import type { WidgetCsp } from "@flow-like/widget-sdk";

export const MAX_WIDGET_CSP_SOURCES = 16;

/** Contract keys in the order serde serializes them */
export const CSP_DIRECTIVES = [
	"connectSrc",
	"imgSrc",
	"fontSrc",
	"mediaSrc",
	"styleSrc",
] as const;

export type CspDirective = (typeof CSP_DIRECTIVES)[number];

export const CSP_ALLOWED_SCHEMES: Readonly<
	Record<CspDirective, readonly string[]>
> = {
	connectSrc: ["https", "wss"],
	imgSrc: ["https"],
	fontSrc: ["https"],
	mediaSrc: ["https"],
	styleSrc: ["https"],
};

export type CspSourceRejection =
	| "empty"
	| "non-ascii"
	| "wildcard"
	| "keyword"
	| "forbidden-character"
	| "uppercase"
	| "userinfo"
	| "query-or-fragment"
	| "ip-literal"
	| "missing-scheme"
	| "scheme-not-allowed"
	| "port"
	| "path"
	| "invalid-host"
	| "reserved-name";

export const CSP_SOURCE_REJECTION_MESSAGES: Readonly<
	Record<CspSourceRejection, string>
> = {
	empty: "source is empty",
	"non-ascii": "non-ASCII characters are not allowed (use punycode)",
	wildcard: "wildcards are not allowed",
	keyword: "keywords, nonces and hashes are not allowed",
	"forbidden-character": "contains a forbidden character",
	uppercase: "uppercase characters are not allowed",
	userinfo: "userinfo is not allowed",
	"query-or-fragment": "queries and fragments are not allowed",
	"ip-literal": "IP literals are not allowed",
	"missing-scheme": "must have the form scheme://host",
	"scheme-not-allowed": "scheme is not allowed for this directive",
	port: "ports are not allowed",
	path: "paths are not allowed",
	"invalid-host": "host is not a valid public DNS name",
	"reserved-name": "host uses a reserved or local-only name",
};

const MAX_HOST_LEN = 253;

const RESERVED_NAME_SUFFIXES = [
	"localhost",
	"local",
	"internal",
	"lan",
	"home.arpa",
	"test",
	"example",
	"invalid",
	"onion",
];

const SOURCE_CHARSET = /^[a-z0-9.:/-]+$/;
const DNS_LABEL = /^[a-z0-9](?:[a-z0-9-]{0,61}[a-z0-9])?$/;

export function isCspDirective(key: string): key is CspDirective {
	return (CSP_DIRECTIVES as readonly string[]).includes(key);
}

function hasNonAscii(value: string): boolean {
	for (let index = 0; index < value.length; index++) {
		if (value.charCodeAt(index) > 0x7f) return true;
	}
	return false;
}

/** ASCII whitespace or control characters, `;`, `,` and quotes */
function hasForbiddenDelimiter(value: string): boolean {
	for (let index = 0; index < value.length; index++) {
		const code = value.charCodeAt(index);
		if (code <= 0x20 || code === 0x7f || ";,\"'".includes(value[index] ?? "")) {
			return true;
		}
	}
	return false;
}

/**
 * Checks one declared source against the grammar and returns the first
 * failing check, in the same order as `validate_csp_source` in Rust.
 */
export function validateCspSource(
	directive: CspDirective,
	source: string,
): CspSourceRejection | null {
	if (source.length === 0) return "empty";
	if (hasNonAscii(source)) return "non-ascii";
	if (source.includes("*")) return "wildcard";
	if (source.startsWith("'")) return "keyword";
	if (hasForbiddenDelimiter(source)) return "forbidden-character";
	if (/[A-Z]/.test(source)) return "uppercase";
	if (source.includes("@")) return "userinfo";
	if (/[?#]/.test(source)) return "query-or-fragment";
	if (/[[\]]/.test(source)) return "ip-literal";
	if (!SOURCE_CHARSET.test(source)) return "forbidden-character";
	const separator = source.indexOf("://");
	if (separator <= 0) return "missing-scheme";
	if (!CSP_ALLOWED_SCHEMES[directive].includes(source.slice(0, separator))) {
		return "scheme-not-allowed";
	}
	const rest = source.slice(separator + 3);
	const hostEnd = rest.search(/[:/]/);
	if (hostEnd !== -1) return rest[hostEnd] === ":" ? "port" : "path";
	return validateHost(rest);
}

function validateHost(host: string): CspSourceRejection | null {
	if (host.length === 0 || host.length > MAX_HOST_LEN) return "invalid-host";
	const labels = host.split(".");
	if (labels.some((label) => label.length === 0)) return "invalid-host";
	const tld = labels[labels.length - 1] ?? "";
	if (/^[0-9]+$/.test(tld)) return "ip-literal";
	if (!labels.every((label) => DNS_LABEL.test(label))) return "invalid-host";
	if (RESERVED_NAME_SUFFIXES.some((suffix) => hostMatches(host, suffix))) {
		return "reserved-name";
	}
	if (labels.length < 2 || !isTld(tld)) return "invalid-host";
	return null;
}

function isTld(label: string): boolean {
	return label.startsWith("xn--")
		? /^[a-z0-9-]{1,59}$/.test(label.slice(4))
		: /^[a-z]{2,63}$/.test(label);
}

function hostMatches(host: string, reserved: string): boolean {
	return (
		host === reserved ||
		(host.endsWith(reserved) && host[host.length - reserved.length - 1] === ".")
	);
}

const ENCODER = new TextEncoder();

/** UTF-8 byte order, which is how Rust orders `String`s. */
export function compareCspSources(a: string, b: string): number {
	if (a === b) return 0;
	const left = ENCODER.encode(a);
	const right = ENCODER.encode(b);
	const length = Math.min(left.length, right.length);
	for (let index = 0; index < length; index++) {
		const difference = (left[index] ?? 0) - (right[index] ?? 0);
		if (difference !== 0) return difference;
	}
	return left.length - right.length;
}

/** An ASCII character a DNS host cannot contain; non-ASCII is left to IDNA */
const FORBIDDEN_HOST_ASCII = /[^a-z0-9.-￿-]/;

/**
 * `domainToASCII` runs the runtime's own URL host parser: Node strips tabs,
 * splits on backslashes, percent-decodes and refuses mapped hosts that end in
 * a number or contain forbidden code points, Bun does none of that. Only a
 * host without such ASCII whose result is a well-formed DNS name is taken,
 * so both runtimes agree.
 */
function punycodeHost(host: string): string | null {
	if (FORBIDDEN_HOST_ASCII.test(host)) return null;
	const ascii = domainToASCII(host);
	const rejection = validateHost(ascii);
	return rejection === null || rejection === "reserved-name" ? ascii : null;
}

/**
 * Authoring normalization: lowercases the source and converts an
 * internationalized host to punycode. Anything the grammar rejects is left
 * for `validateCspSource` to report.
 */
export function normalizeCspSource(source: string): string {
	const lowered = source.toLowerCase();
	const separator = lowered.indexOf("://");
	if (separator <= 0) return lowered;
	const prefix = lowered.slice(0, separator + 3);
	const rest = lowered.slice(separator + 3);
	const hostEnd = rest.search(/[:/?#]/);
	const host = hostEnd === -1 ? rest : rest.slice(0, hostEnd);
	if (!hasNonAscii(host)) return lowered;
	const ascii = punycodeHost(host);
	if (ascii === null) return lowered;
	return `${prefix}${ascii}${hostEnd === -1 ? "" : rest.slice(hostEnd)}`;
}

/** Sorted ascending in UTF-8 byte order without duplicates. */
export function canonicalCspSources(sources: readonly string[]): string[] {
	return [...new Set(sources)].sort(compareCspSources);
}

/**
 * Normalizes every source, then sorts and deduplicates each directive and
 * drops empty directives. Returns `{}` when nothing remains.
 */
export function normalizeCspDeclaration(csp: WidgetCsp): WidgetCsp {
	const normalized: WidgetCsp = {};
	for (const directive of CSP_DIRECTIVES) {
		const sources = canonicalCspSources(
			(csp[directive] ?? []).map(normalizeCspSource),
		);
		if (sources.length > 0) normalized[directive] = sources;
	}
	return normalized;
}

export function isCspEmpty(csp: WidgetCsp): boolean {
	return CSP_DIRECTIVES.every(
		(directive) => (csp[directive]?.length ?? 0) === 0,
	);
}

export function cspSourceCount(csp: WidgetCsp): number {
	return CSP_DIRECTIVES.reduce(
		(count, directive) => count + (csp[directive]?.length ?? 0),
		0,
	);
}

/**
 * Mirrors `WidgetCsp::validate`: shape (what serde would refuse to parse),
 * grammar, canonical order and the source cap. Accepts untyped JSON.
 */
export function validateCspDeclaration(csp: unknown): string[] {
	if (typeof csp !== "object" || csp === null || Array.isArray(csp)) {
		return ["csp must be an object"];
	}
	const declaration = csp as Record<string, unknown>;
	const errors: string[] = [];
	for (const key of Object.keys(declaration)) {
		if (!isCspDirective(key)) {
			errors.push(
				`csp declares unknown directive "${key}" (allowed: ${CSP_DIRECTIVES.join(", ")})`,
			);
		}
	}
	let count = 0;
	for (const directive of CSP_DIRECTIVES) {
		const value = declaration[directive];
		if (value === undefined) continue;
		if (
			!Array.isArray(value) ||
			!value.every((source) => typeof source === "string")
		) {
			errors.push(`csp ${directive} must be an array of strings`);
			continue;
		}
		const sources = value as string[];
		for (const source of sources) {
			const rejection = validateCspSource(directive, source);
			if (rejection !== null) {
				errors.push(
					`Invalid csp source "${source}" in ${directive}: ${CSP_SOURCE_REJECTION_MESSAGES[rejection]}`,
				);
			}
		}
		if (
			sources.some(
				(source, index) =>
					index > 0 && compareCspSources(sources[index - 1] ?? "", source) >= 0,
			)
		) {
			errors.push(
				`csp ${directive} must be sorted ascending without duplicates`,
			);
		}
		count += sources.length;
	}
	if (count > MAX_WIDGET_CSP_SOURCES) {
		errors.push(
			`csp declares ${count} sources; at most ${MAX_WIDGET_CSP_SOURCES} are allowed`,
		);
	}
	return errors;
}
