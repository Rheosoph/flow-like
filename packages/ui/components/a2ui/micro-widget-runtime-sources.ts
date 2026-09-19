import {
	type PlatformStorageScope,
	type WidgetInputPathStep,
	type WidgetNetworkInputSlot,
	type WidgetRuntimeSourceRequest,
	type WidgetUrlTemplate,
	parseWidgetInputPath,
	widgetRuntimeRequestKey,
} from "./micro-widget-policy";

/**
 * Host extraction of runtime network sources (§14.4.3). Only origins (or this
 * app's platform-storage path scope) ever leave the host: paths, queries and
 * signatures stay in the props the widget receives. Issues carry codes and
 * counts, never values. Mirrored by
 * `packages/wasm/schema/tests/fixtures/widget_runtime_extraction.json`.
 */

export const RUNTIME_EXTRACTION_LIMITS = {
	maxValueLength: 8192,
	maxNodesPerSlot: 2048,
	maxStringLeavesPerSlot: 256,
	maxSourcesPerSlot: 8,
	maxSourcesTotal: 8,
	maxTemplateHosts: 16,
} as const;

/** Issue slot of the cross-slot overflow. */
export const RUNTIME_CROSS_SLOT_ISSUE_SLOT = "*";

export const RUNTIME_VALUE_ISSUE_CODES = [
	"too-long",
	"not-absolute",
	"scheme-not-allowed",
	"not-a-url",
	"userinfo",
	"port",
	"trailing-dot",
	"ip-literal",
	"template-not-allowed",
	"template-unexpanded",
	"template-too-large",
	"platform-storage",
	"too-many-values",
	"too-many-sources",
] as const;

export type RuntimeValueIssueCode = (typeof RUNTIME_VALUE_ISSUE_CODES)[number];

export interface RuntimeValueIssue {
	slot: string;
	code: RuntimeValueIssueCode;
	count: number;
}

export interface RuntimeSourceExtraction {
	request: WidgetRuntimeSourceRequest[];
	/** Stable identity of `request`. */
	key: string;
	issues: RuntimeValueIssue[];
}

const ALLOWED_SCHEMES = new Set(["https", "wss"]);
const SCHEME = /^([A-Za-z][A-Za-z0-9+.-]*):/;
const ASCII_WHITESPACE_EDGES = /^[\t\n\f\r ]+|[\t\n\f\r ]+$/g;
const AUTHORITY_END = /[/\\?#]/;
const IPV4 = /^\d+\.\d+\.\d+\.\d+$/;
const DNS_LABEL = /^[a-z0-9]([a-z0-9-]{0,61}[a-z0-9])?$/;
const SWITCH_VALUE = /^[A-Za-z0-9]([A-Za-z0-9-]{0,61}[A-Za-z0-9])?$/;
const APP_ID = /^[A-Za-z0-9_-]{1,64}$/;
const RANGE = /^([a-z])-([a-z])$|^([0-9])-([0-9])$/;

function compareStrings(left: string, right: string): number {
	return left < right ? -1 : left > right ? 1 : 0;
}

function isPlainObject(value: unknown): value is Record<string, unknown> {
	if (typeof value !== "object" || value === null || Array.isArray(value))
		return false;
	const prototype = Object.getPrototypeOf(value);
	return prototype === Object.prototype || prototype === null;
}

function hasOwn(value: object, key: string): boolean {
	return Object.prototype.hasOwnProperty.call(value, key);
}

/** String leaves at the end of the path, or null when the slot is over budget. */
function walkSlot(
	root: string,
	steps: readonly WidgetInputPathStep[],
	props: Readonly<Record<string, unknown>>,
): string[] | null {
	if (!hasOwn(props, root)) return [];
	const values: string[] = [];
	let nodes = 0;
	const visit = (value: unknown, index: number): boolean => {
		nodes++;
		if (nodes > RUNTIME_EXTRACTION_LIMITS.maxNodesPerSlot) return false;
		if (index === steps.length) {
			if (typeof value !== "string") return true;
			values.push(value);
			return values.length <= RUNTIME_EXTRACTION_LIMITS.maxStringLeavesPerSlot;
		}
		const step = steps[index];
		if (step.kind === "key") {
			return isPlainObject(value) && hasOwn(value, step.key)
				? visit(value[step.key], index + 1)
				: true;
		}
		if (step.kind === "items") {
			if (!Array.isArray(value)) return true;
			for (let item = 0; item < value.length; item++) {
				if (!visit(value[item], index + 1)) return false;
			}
			return true;
		}
		if (!isPlainObject(value)) return true;
		for (const key of Object.keys(value)) {
			if (!visit(value[key], index + 1)) return false;
		}
		return true;
	};
	return visit(props[root], 0) ? values : null;
}

type UrlCheck = { candidate: string } | { issue: RuntimeValueIssueCode };

interface ExtractionContext {
	platformStorage: readonly PlatformStorageScope[];
	appId: string | null;
	props: Readonly<Record<string, unknown>>;
}

function checkUrl(text: string, context: ExtractionContext): UrlCheck {
	let url: URL;
	try {
		url = new URL(text);
	} catch {
		return { issue: "not-a-url" };
	}
	if (url.username || url.password) return { issue: "userinfo" };
	if (url.port !== "") return { issue: "port" };
	const host = url.hostname;
	if (host.endsWith(".")) return { issue: "trailing-dot" };
	if (host.startsWith("[") || IPV4.test(host)) return { issue: "ip-literal" };
	if (!ALLOWED_SCHEMES.has(url.protocol.slice(0, -1))) {
		return { issue: "scheme-not-allowed" };
	}
	const origin = `${url.protocol}//${host}`;
	const scope = context.platformStorage.find(
		(entry) => entry.origin === origin,
	);
	if (!scope) return { candidate: origin };
	const { appId } = context;
	if (appId === null || !APP_ID.test(appId))
		return { issue: "platform-storage" };
	const prefix = `${scope.pathPrefix}${appId}/`;
	return url.pathname.startsWith(prefix)
		? { candidate: `${origin}${prefix}` }
		: { issue: "platform-storage" };
}

function templateSubdomains(
	template: WidgetUrlTemplate,
	props: Readonly<Record<string, unknown>>,
): string[] | null {
	if (template.subdomains && template.subdomains.length > 0) {
		return template.subdomains;
	}
	const input = template.subdomainsInput;
	if (!input || !hasOwn(props, input)) return null;
	const value = props[input];
	const labels =
		typeof value === "string"
			? [...value]
			: Array.isArray(value) && value.every((item) => typeof item === "string")
				? (value as string[])
				: null;
	if (!labels || labels.length === 0) return null;
	return labels.every((label) => DNS_LABEL.test(label)) ? labels : null;
}

function expandPlaceholder(
	body: string,
	template: WidgetUrlTemplate,
	props: Readonly<Record<string, unknown>>,
): string[] | null {
	const range = RANGE.exec(body);
	if (range) {
		const from = (range[1] ?? range[3]).charCodeAt(0);
		const to = (range[2] ?? range[4]).charCodeAt(0);
		if (from > to) return null;
		return Array.from({ length: to - from + 1 }, (_, offset) =>
			String.fromCharCode(from + offset),
		);
	}
	if (body.startsWith("switch:")) {
		const values = body.slice("switch:".length).split(",");
		return values.every((value) => SWITCH_VALUE.test(value)) ? values : null;
	}
	if (body === "s") return templateSubdomains(template, props);
	return null;
}

type Piece = string | string[];

/** Literal text and placeholder alternatives, or null when a placeholder cannot be expanded. */
function parseAuthorityTemplate(
	authority: string,
	template: WidgetUrlTemplate,
	props: Readonly<Record<string, unknown>>,
): Piece[] | null {
	const pieces: Piece[] = [];
	let rest = authority;
	while (rest.length > 0) {
		const open = rest.indexOf("{");
		const stray = rest.indexOf("}");
		if (open < 0) {
			if (stray >= 0) return null;
			pieces.push(rest);
			break;
		}
		if (stray >= 0 && stray < open) return null;
		const close = rest.indexOf("}", open);
		if (close < 0) return null;
		const body = rest.slice(open + 1, close);
		if (body.includes("{")) return null;
		const values = expandPlaceholder(body, template, props);
		if (!values) return null;
		if (open > 0) pieces.push(rest.slice(0, open));
		pieces.push(values);
		rest = rest.slice(close + 1);
	}
	return pieces;
}

function expandAuthorities(pieces: readonly Piece[]): string[] {
	let authorities = [""];
	for (const piece of pieces) {
		const options = typeof piece === "string" ? [piece] : piece;
		authorities = authorities.flatMap((prefix) =>
			options.map((option) => `${prefix}${option}`),
		);
	}
	return authorities;
}

interface ValueResult {
	candidates: string[];
	issues: Set<RuntimeValueIssueCode>;
}

function extractValue(
	raw: string,
	slot: WidgetNetworkInputSlot,
	context: ExtractionContext,
): ValueResult {
	const result: ValueResult = { candidates: [], issues: new Set() };
	const fail = (code: RuntimeValueIssueCode) => {
		result.issues.add(code);
		return result;
	};
	if (raw.length > RUNTIME_EXTRACTION_LIMITS.maxValueLength) {
		return fail("too-long");
	}
	const value = raw.replace(ASCII_WHITESPACE_EDGES, "");
	const lower = value.slice(0, 5).toLowerCase();
	if (lower === "data:" || lower === "blob:") return result;
	if (/^[/\\]{2}/.test(value)) return fail("not-absolute");
	const scheme = SCHEME.exec(value)?.[1];
	if (scheme === undefined) return result;
	if (!ALLOWED_SCHEMES.has(scheme.toLowerCase())) {
		return fail("scheme-not-allowed");
	}
	const afterScheme = value.slice(scheme.length + 1);
	if (!afterScheme.startsWith("//")) return fail("not-a-url");
	const hierarchy = afterScheme.slice(2);
	const end = hierarchy.search(AUTHORITY_END);
	const authority = end < 0 ? hierarchy : hierarchy.slice(0, end);
	const tail = end < 0 ? "" : hierarchy.slice(end);

	const record = (check: UrlCheck) => {
		if ("candidate" in check) result.candidates.push(check.candidate);
		else result.issues.add(check.issue);
	};
	if (!authority.includes("{")) {
		record(checkUrl(value, context));
		return result;
	}
	if (!slot.template) return fail("template-not-allowed");
	const pieces = parseAuthorityTemplate(
		authority,
		slot.template,
		context.props,
	);
	if (!pieces) return fail("template-unexpanded");
	const hosts = pieces.reduce(
		(count, piece) => count * (typeof piece === "string" ? 1 : piece.length),
		1,
	);
	if (hosts > RUNTIME_EXTRACTION_LIMITS.maxTemplateHosts) {
		return fail("template-too-large");
	}
	for (const expanded of expandAuthorities(pieces)) {
		record(checkUrl(`${scheme}://${expanded}${tail}`, context));
	}
	return result;
}

interface SlotResult {
	slot: string;
	candidates: Set<string>;
	issues: Map<RuntimeValueIssueCode, number>;
}

function extractSlot(
	slot: WidgetNetworkInputSlot,
	context: ExtractionContext,
	excluded: ReadonlySet<string>,
): SlotResult {
	const result: SlotResult = {
		slot: slot.path,
		candidates: new Set(),
		issues: new Map(),
	};
	const path = parseWidgetInputPath(slot.path);
	if (!path) return result;
	const values = walkSlot(path.root, path.steps, context.props);
	if (values === null) {
		result.issues.set("too-many-values", 1);
		return result;
	}
	for (const value of values) {
		const { candidates, issues } = extractValue(value, slot, context);
		for (const code of issues) {
			result.issues.set(code, (result.issues.get(code) ?? 0) + 1);
		}
		for (const candidate of candidates) {
			if (!excluded.has(candidate)) result.candidates.add(candidate);
		}
	}
	if (result.candidates.size > RUNTIME_EXTRACTION_LIMITS.maxSourcesPerSlot) {
		result.candidates.clear();
		result.issues.set("too-many-sources", 1);
	}
	return result;
}

/**
 * Runtime source request for the declared slots of a descriptor
 * (`descriptor.networkInputs`, never the page contract), read from the props
 * the widget receives (`publicWidgetProps`). `excluded` holds session blocks
 * and this mount's server rejections.
 */
export function extractRuntimeSources(
	slots: readonly WidgetNetworkInputSlot[],
	platformStorage: readonly PlatformStorageScope[],
	appId: string | null,
	props: Readonly<Record<string, unknown>>,
	excluded: ReadonlySet<string>,
): RuntimeSourceExtraction {
	const context: ExtractionContext = { platformStorage, appId, props };
	const results = slots.map((slot) => extractSlot(slot, context, excluded));
	const issues: RuntimeValueIssue[] = results.flatMap(({ slot, issues }) =>
		[...issues].map(([code, count]) => ({ slot, code, count })),
	);
	let request: WidgetRuntimeSourceRequest[] = results
		.filter(({ candidates }) => candidates.size > 0)
		.map(({ slot, candidates }) => ({
			slot,
			sources: [...candidates].sort(compareStrings),
		}))
		.sort((left, right) => compareStrings(left.slot, right.slot));
	const distinct = new Set(request.flatMap(({ sources }) => sources));
	if (distinct.size > RUNTIME_EXTRACTION_LIMITS.maxSourcesTotal) {
		request = [];
		issues.push({
			slot: RUNTIME_CROSS_SLOT_ISSUE_SLOT,
			code: "too-many-sources",
			count: 1,
		});
	}
	issues.sort(
		(left, right) =>
			compareStrings(left.slot, right.slot) ||
			compareStrings(left.code, right.code),
	);
	return {
		request,
		key: widgetRuntimeRequestKey(request),
		issues,
	};
}
