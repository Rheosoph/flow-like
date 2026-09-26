// Mirror of the source grammar, limits and input path grammar in
// packages/wasm/schema/src/widget_policy.rs. Both implementations are driven
// by packages/wasm/schema/tests/fixtures/widget_csp.json. No local runtime
// imports: tests load this module under Node as well.

import { domainToASCII } from "node:url";
import type {
	WidgetCsp,
	WidgetCspDirective,
	WidgetCspPurpose,
} from "@flow-like/widget-sdk";

export const MAX_WIDGET_CSP_SOURCES = 16;
export const MAX_WIDGET_CSP_SOURCE_BYTES = 1536;
export const MAX_WIDGET_CSP_PURPOSES = 8;
export const MAX_WIDGET_NETWORK_INPUTS = 8;
export const MAX_WIDGET_INPUT_PATH_SEGMENTS = 6;
export const MAX_WIDGET_TEMPLATE_SUBDOMAINS = 16;

/** Widget inputs the host injects itself; network inputs never read them. */
export const HOST_RESERVED_INPUT_KEYS: readonly string[] = [
	"publicMediaGrants",
];

/** Contract keys in the order serde serializes them (`CspDirective::ALL`) */
export const CSP_DIRECTIVES = [
	"connectSrc",
	"imgSrc",
	"fontSrc",
	"mediaSrc",
	"styleSrc",
] as const satisfies readonly WidgetCspDirective[];

export type CspDirective = WidgetCspDirective;

export const CSP_PURPOSE_KEYS = [
	"reason",
	...CSP_DIRECTIVES,
	"inputs",
] as const;

export const CSP_NETWORK_INPUT_KEYS = [
	"path",
	"directives",
	"template",
] as const;
export const CSP_TEMPLATE_KEYS = ["subdomains", "subdomainsInput"] as const;

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
	wildcard: 'wildcards are only allowed as a leading "*." label of the host',
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
	"nip.io",
	"sslip.io",
	"traefik.me",
	"localtest.me",
	"lvh.me",
	"localhost.direct",
];

const SOURCE_CHARSET = /^[a-z0-9.:/*-]+$/;
const WILDCARD_FORM = /^[a-z]+:\/\/\*\.[^*]+$/;
const DNS_LABEL = /^[a-z0-9](?:[a-z0-9-]{0,61}[a-z0-9])?$/;
const MEMBER_KEY = /^[A-Za-z_][A-Za-z0-9_]*/;

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
 * Checks one declared source against the grammar
 * `scheme "://" [ "*." ] host` and returns the first failing check, in the
 * same order as `validate_csp_source` in Rust. Public suffix checks on
 * wildcard bases live in `validateWildcardBases`.
 */
export function validateCspSource(
	directive: CspDirective,
	source: string,
): CspSourceRejection | null {
	if (source.length === 0) return "empty";
	if (hasNonAscii(source)) return "non-ascii";
	if (source.includes("*") && !WILDCARD_FORM.test(source)) return "wildcard";
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
	return rest.startsWith("*.")
		? validateHost(rest.slice(2), MAX_HOST_LEN - 2)
		: validateHost(rest, MAX_HOST_LEN);
}

function validateHost(host: string, maxLen: number): CspSourceRejection | null {
	if (host.length === 0 || host.length > maxLen) return "invalid-host";
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

export function isDnsLabel(label: string): boolean {
	return DNS_LABEL.test(label);
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

export function isStrictlyAscending(
	items: readonly string[],
	compare: (a: string, b: string) => number = compareCspSources,
): boolean {
	return items.every(
		(item, index) => index === 0 || compare(items[index - 1] ?? "", item) < 0,
	);
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
	const rejection = validateHost(ascii, MAX_HOST_LEN);
	return rejection === null || rejection === "reserved-name" ? ascii : null;
}

/**
 * Authoring normalization: lowercases the source and converts an
 * internationalized host (or wildcard base) to punycode. Anything the grammar
 * rejects is left for `validateCspSource` to report.
 */
export function normalizeCspSource(source: string): string {
	const lowered = source.toLowerCase();
	const separator = lowered.indexOf("://");
	if (separator <= 0) return lowered;
	const prefix = lowered.slice(0, separator + 3);
	const rest = lowered.slice(separator + 3);
	const hostEnd = rest.search(/[:/?#]/);
	const authority = hostEnd === -1 ? rest : rest.slice(0, hostEnd);
	const wildcard = authority.startsWith("*.") ? "*." : "";
	const host = authority.slice(wildcard.length);
	if (!hasNonAscii(host)) return lowered;
	const ascii = punycodeHost(host);
	if (ascii === null) return lowered;
	return `${prefix}${wildcard}${ascii}${hostEnd === -1 ? "" : rest.slice(hostEnd)}`;
}

/** Sorted ascending in UTF-8 byte order without duplicates. */
export function canonicalCspSources(sources: readonly string[]): string[] {
	return [...new Set(sources)].sort(compareCspSources);
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

/** Σ(len + 1) over every entry: the bytes the sources add to a header. */
export function cspSourceBytes(csp: WidgetCsp): number {
	return CSP_DIRECTIVES.reduce(
		(bytes, directive) =>
			bytes +
			(csp[directive] ?? []).reduce(
				(sum, source) => sum + ENCODER.encode(source).length + 1,
				0,
			),
		0,
	);
}

/** Per-directive union of every purpose's sources (`flatten_csp_purposes`). */
export function flattenCspPurposes(
	purposes: readonly WidgetCspPurpose[],
): WidgetCsp {
	const csp: WidgetCsp = {};
	for (const directive of CSP_DIRECTIVES) {
		const sources = canonicalCspSources(
			purposes.flatMap((purpose) => purpose[directive] ?? []),
		);
		if (sources.length > 0) csp[directive] = sources;
	}
	return csp;
}

export type WidgetInputPathSegment =
	| { kind: "key"; key: string }
	| { kind: "items" }
	| { kind: "values" };

export interface WidgetInputPath {
	root: string;
	segments: WidgetInputPathSegment[];
}

/** Parses `root *( "." key / "[]" / ".*" )` with `key = [A-Za-z_][A-Za-z0-9_]*`. */
export function parseWidgetInputPath(path: string): WidgetInputPath | null {
	const root = MEMBER_KEY.exec(path)?.[0];
	if (root === undefined) return null;
	const segments: WidgetInputPathSegment[] = [];
	let rest = path.slice(root.length);
	while (rest.length > 0) {
		if (rest.startsWith("[]")) {
			segments.push({ kind: "items" });
			rest = rest.slice(2);
		} else if (rest.startsWith(".*")) {
			segments.push({ kind: "values" });
			rest = rest.slice(2);
		} else {
			const key = rest.startsWith(".")
				? MEMBER_KEY.exec(rest.slice(1))?.[0]
				: undefined;
			if (key === undefined) return null;
			segments.push({ kind: "key", key });
			rest = rest.slice(1 + key.length);
		}
	}
	return { root, segments };
}

export function isValidWidgetInputPath(path: string): boolean {
	const parsed = parseWidgetInputPath(path);
	return (
		parsed !== null && parsed.segments.length < MAX_WIDGET_INPUT_PATH_SEGMENTS
	);
}

export function compareCspDirectives(a: string, b: string): number {
	return (
		CSP_DIRECTIVES.indexOf(a as CspDirective) -
		CSP_DIRECTIVES.indexOf(b as CspDirective)
	);
}
