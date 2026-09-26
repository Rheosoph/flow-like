/**
 * Host-side mirror of `flow_like_wasm_schema::widget_policy`. The backend is
 * authoritative: it validates every source and derives the policy from the
 * installed bundle or the published version row. The host only reads the
 * descriptor it gets back, compares grants against it and renders it.
 */

export const MICRO_WIDGET_CAPABILITIES = [
	"microphone",
	"downloads",
	"media",
	"workers",
	"wasm",
] as const;

export type MicroWidgetCapability = (typeof MICRO_WIDGET_CAPABILITIES)[number];

export const WIDGET_CSP_KEYS = [
	"connectSrc",
	"imgSrc",
	"fontSrc",
	"mediaSrc",
	"styleSrc",
] as const;

export type WidgetCspKey = (typeof WIDGET_CSP_KEYS)[number];

export const WIDGET_CSP_DIRECTIVES: Readonly<Record<WidgetCspKey, string>> = {
	connectSrc: "connect-src",
	imgSrc: "img-src",
	fontSrc: "font-src",
	mediaSrc: "media-src",
	styleSrc: "style-src",
};

export type WidgetCsp = { [K in WidgetCspKey]?: string[] };

/** Serialized like the Rust struct: false capabilities and empty lists are omitted. */
export interface WidgetPolicy {
	workers?: boolean;
	wasm?: boolean;
	media?: boolean;
	microphone?: boolean;
	downloads?: boolean;
	csp?: WidgetCsp;
}

export type WidgetPolicyStatus = "ok" | "invalid";

/** Ordered: each level means more unrelated parties can be on the other end. */
export const WIDGET_SOURCE_LEVELS = [
	"known",
	"external",
	"shared",
	"broad",
] as const;

export type WidgetSourceLevel = (typeof WIDGET_SOURCE_LEVELS)[number];

export const WIDGET_SOURCE_KINDS = [
	"service",
	"exact",
	"subdomains",
	"tenant-host",
	"tenant-subdomains",
	"shared-host",
	"shared-wildcard",
	"platform-host",
	"platform-storage",
] as const;

/** Unknown kinds from a newer backend are kept; copy falls back by level. */
export type WidgetSourceKind =
	| (typeof WIDGET_SOURCE_KINDS)[number]
	| (string & {});

/** Mirrors `widget_sources::WIDGET_SOURCE_ABOUT_KEYS`. */
export const WIDGET_SOURCE_ABOUT_KEYS = [
	"widgetSourceAboutObjectStorage",
	"widgetSourceAboutSharedCdn",
	"widgetSourceAboutAppHosting",
	"widgetSourceAboutCloudPlatform",
	"widgetSourceAboutPackageCdn",
	"widgetSourceAboutLibraryCdn",
	"widgetSourceAboutCodeHosting",
	"widgetSourceAboutUserContent",
	"widgetSourceAboutMapData",
	"widgetSourceAboutMapTiles",
	"widgetSourceAboutSatelliteImagery",
	"widgetSourceAboutWebFonts",
	"widgetSourceAboutRequestCapture",
	"widgetSourceAboutTunnel",
] as const;

export type WidgetSourceAboutKey = (typeof WIDGET_SOURCE_ABOUT_KEYS)[number];

export interface WidgetUrlTemplate {
	subdomains?: string[];
	subdomainsInput?: string;
}

/** A declared input whose values the host may turn into runtime sources. */
export interface WidgetNetworkInputSlot {
	path: string;
	/** Zero-based index of the purpose that declares the input. */
	purpose: number;
	directives: WidgetCspKey[];
	template?: WidgetUrlTemplate;
}

/** This deployment's Flow-Like storage; runtime sources may only reach `origin + pathPrefix + appId + "/"`. */
export interface PlatformStorageScope {
	origin: string;
	pathPrefix: string;
}

export type WidgetRuntimeStatus = "none" | "ok" | "unavailable" | "invalid";

export interface WidgetRuntimeRejection {
	slot: string;
	source: string;
	code: string;
}

export interface WidgetRuntimeDescriptor {
	status: WidgetRuntimeStatus;
	declaredDigest: string;
	/** Present exactly when at least one runtime source was accepted. */
	runtimeDigest?: string;
	rejected: WidgetRuntimeRejection[];
	invalidReason?: string;
}

/** Engine gates are denylists: a missing object means every feature is on. */
export interface WidgetEngineSupport {
	wildcardSources: boolean;
	runtimeSources: boolean;
	localMedia: boolean;
}

export type WidgetSourceOrigin = "declared" | "runtime";

export interface WidgetNetworkSource {
	source: string;
	directives: WidgetCspKey[];
	origin: WidgetSourceOrigin;
	slot?: string;
	kind: WidgetSourceKind;
	level: WidgetSourceLevel;
	host: string;
	emphasis: string;
	providerId?: string;
	provider?: string;
	aboutKey?: WidgetSourceAboutKey;
}

export interface WidgetNetworkPurpose {
	reason: string;
	level: WidgetSourceLevel;
	inputs?: string[];
	sources: WidgetNetworkSource[];
}

/** Display-only classification; never enforcement. */
export interface WidgetNetwork {
	level: WidgetSourceLevel;
	catalogVersion: number;
	pslVersion: string;
	stale: boolean;
	purposes: WidgetNetworkPurpose[];
}

export interface WidgetPolicyDescriptor {
	/** `registry:<hub host>`, `local` (desktop) or `hub` (web). */
	source: string;
	packageId: string;
	packageVersion?: string;
	bundleHash: string;
	widgetId: string;
	preview: boolean;
	status: WidgetPolicyStatus;
	invalidReason?: string;
	/** Effective policy (declared plus accepted runtime), stripped for previews; the baseline when `status` is `invalid`. */
	policy: WidgetPolicy;
	policyDigest: string;
	/** Empty for invalid and preview descriptors and for backends that predate runtime sources. */
	networkInputs: WidgetNetworkInputSlot[];
	platformStorage?: PlatformStorageScope[];
	runtime?: WidgetRuntimeDescriptor;
	engine?: WidgetEngineSupport;
	network?: WidgetNetwork;
}

export interface WidgetRuntimeSourceRequest {
	slot: string;
	sources: string[];
}

/** Desktop addresses the bundle by `bundleHash`, web by `packageVersion`. */
export interface WidgetPolicyRequest {
	packageId: string;
	packageVersion: string;
	bundleHash?: string | null;
	widgetId: string;
	preview: boolean;
	appId?: string | null;
	/** Non-empty only outside previews; web then describes through `POST`. */
	runtimeSources?: WidgetRuntimeSourceRequest[];
}

export interface WidgetGrantRequest extends WidgetPolicyRequest {
	policyDigest: string;
}

export interface WidgetGrantResponse {
	/** Null when the policy is empty: the baseline frame needs no grant. */
	grant: string | null;
	/** Seconds until the grant stops widening the document. */
	expiresIn: number;
	policyDigest: string;
	/** Web only: the URL carrier of accepted runtime sources (`{grant}~{runtime}`). */
	runtime: string | null;
}

export const WIDGET_POLICY_SOURCE_LOCAL = "local";
export const WIDGET_POLICY_SOURCE_HUB = "hub";
export const WIDGET_POLICY_REGISTRY_PREFIX = "registry:";
/** Grants migrated from the capability-only store apply to every registry host. */
export const WIDGET_POLICY_SOURCE_ANY_REGISTRY = "registry:*";

/** Longest base64url (no padding) encoding of the 1024-byte runtime slot JSON. */
export const MAX_WIDGET_RUNTIME_COMPONENT_LENGTH = 1366;

const POLICY_DIGEST = /^sha256:[0-9a-f]{64}$/;
const DESKTOP_GRANT = /^[0-9a-f]{64}$/;
const WEB_GRANT = /^[A-Za-z0-9_-]+\.[A-Za-z0-9_-]+\.[A-Za-z0-9_-]+$/;
const BASE64URL = /^[A-Za-z0-9_-]+$/;
const MAX_WEB_GRANT_LENGTH = 2048;
const POLICY_CHANGED = "policy_changed";
const INPUT_PATH_KEY = /^[A-Za-z_][A-Za-z0-9_]*/;
const MAX_INPUT_PATH_STEPS = 6;
const TEMPLATE_SUBDOMAIN = /^[a-z0-9]([a-z0-9-]{0,61}[a-z0-9])?$/;
const MAX_TEMPLATE_SUBDOMAINS = 16;
const MAX_PROVIDER_LENGTH = 40;
const HTTPS_ORIGIN = /^https:\/\/[a-z0-9.-]+$/;

function isRecord(value: unknown): value is Record<string, unknown> {
	return typeof value === "object" && value !== null && !Array.isArray(value);
}

function hasOwn(value: object, key: string): boolean {
	return Object.prototype.hasOwnProperty.call(value, key);
}

function compareStrings(left: string, right: string): number {
	return left < right ? -1 : left > right ? 1 : 0;
}

export function registryPolicySource(hubHost: string): string {
	return `${WIDGET_POLICY_REGISTRY_PREFIX}${hubHost}`;
}

export function isRegistryPolicySource(source: string): boolean {
	return source.startsWith(WIDGET_POLICY_REGISTRY_PREFIX);
}

export function isDesktopWidgetGrant(value: string): boolean {
	return DESKTOP_GRANT.test(value);
}

export function isWebWidgetGrant(value: string): boolean {
	return value.length <= MAX_WEB_GRANT_LENGTH && WEB_GRANT.test(value);
}

/** Mirrors `widget_frame::is_runtime_component`: base64url without padding, at most 1366 characters. */
export function isWidgetRuntimeComponent(value: string): boolean {
	return (
		value.length > 0 &&
		value.length <= MAX_WIDGET_RUNTIME_COMPONENT_LENGTH &&
		value.length % 4 !== 1 &&
		BASE64URL.test(value)
	);
}

export function isWidgetSourceLevel(
	value: unknown,
): value is WidgetSourceLevel {
	return (
		typeof value === "string" &&
		(WIDGET_SOURCE_LEVELS as readonly string[]).includes(value)
	);
}

export function widgetSourceLevelRank(level: WidgetSourceLevel): number {
	return WIDGET_SOURCE_LEVELS.indexOf(level);
}

/** A level from a backend; anything unknown counts as the worst level. */
export function readWidgetSourceLevel(value: unknown): WidgetSourceLevel {
	return isWidgetSourceLevel(value) ? value : "broad";
}

export function maxWidgetSourceLevel(
	levels: Iterable<WidgetSourceLevel>,
): WidgetSourceLevel | null {
	let max: WidgetSourceLevel | null = null;
	for (const level of levels) {
		if (
			max === null ||
			widgetSourceLevelRank(level) > widgetSourceLevelRank(max)
		)
			max = level;
	}
	return max;
}

/** A grant at `granted` covers a request at `requested` unless the request is riskier. */
export function widgetSourceLevelCovers(
	granted: WidgetSourceLevel | null | undefined,
	requested: WidgetSourceLevel,
): boolean {
	return (
		isWidgetSourceLevel(granted) &&
		widgetSourceLevelRank(requested) <= widgetSourceLevelRank(granted)
	);
}

export function isWidgetSourceAboutKey(
	value: unknown,
): value is WidgetSourceAboutKey {
	return (
		typeof value === "string" &&
		(WIDGET_SOURCE_ABOUT_KEYS as readonly string[]).includes(value)
	);
}

function isCspKey(value: unknown): value is WidgetCspKey {
	return (
		typeof value === "string" &&
		(WIDGET_CSP_KEYS as readonly string[]).includes(value)
	);
}

/** Capabilities set to exactly `true` as own keys; anything else requests nothing. */
export function policyCapabilities(
	policy: WidgetPolicy | null | undefined,
): MicroWidgetCapability[] {
	if (!isRecord(policy)) return [];
	return MICRO_WIDGET_CAPABILITIES.filter(
		(name) => hasOwn(policy, name) && policy[name] === true,
	);
}

export function policySources(
	policy: WidgetPolicy | null | undefined,
	key: WidgetCspKey,
): string[] {
	if (!isRecord(policy) || !hasOwn(policy, "csp")) return [];
	const csp: unknown = policy.csp;
	if (!isRecord(csp) || !hasOwn(csp, key)) return [];
	const sources: unknown = csp[key];
	return Array.isArray(sources)
		? sources.filter(
				(source): source is string =>
					typeof source === "string" && source.length > 0,
			)
		: [];
}

/** Every (directive, source) pair in directive order. */
export function policySourceEntries(
	policy: WidgetPolicy | null | undefined,
): [WidgetCspKey, string][] {
	return WIDGET_CSP_KEYS.flatMap((key) =>
		policySources(policy, key).map(
			(source) => [key, source] as [WidgetCspKey, string],
		),
	);
}

function sourceHost(source: string): string {
	const separator = source.indexOf("://");
	const rest = separator < 0 ? source : source.slice(separator + 3);
	const slash = rest.indexOf("/");
	return slash < 0 ? rest : rest.slice(0, slash);
}

/** Distinct hosts across every directive, sorted. `wss://h` and `https://h` count once. */
export function policyHosts(policy: WidgetPolicy | null | undefined): string[] {
	const hosts = new Set<string>();
	for (const [, source] of policySourceEntries(policy)) {
		hosts.add(sourceHost(source));
	}
	return [...hosts].sort();
}

export function policyHostCount(
	policy: WidgetPolicy | null | undefined,
): number {
	return policyHosts(policy).length;
}

export function isEmptyPolicy(
	policy: WidgetPolicy | null | undefined,
): boolean {
	return (
		policyCapabilities(policy).length === 0 &&
		policySourceEntries(policy).length === 0
	);
}

/**
 * A grant covers a request when every requested capability was granted and
 * every requested source was granted for the same directive. Anything new
 * re-prompts; anything removed does not.
 */
export function policyCovers(
	granted: WidgetPolicy | null | undefined,
	requested: WidgetPolicy | null | undefined,
): boolean {
	if (!isRecord(granted)) return false;
	const grantedCapabilities = policyCapabilities(granted);
	if (
		!policyCapabilities(requested).every((name) =>
			grantedCapabilities.includes(name),
		)
	) {
		return false;
	}
	return WIDGET_CSP_KEYS.every((key) => {
		const grantedSources = policySources(granted, key);
		return policySources(requested, key).every((source) =>
			grantedSources.includes(source),
		);
	});
}

/** Canonical copy: true capabilities only, each source list sorted and deduplicated. */
export function normalizeWidgetPolicy(
	policy: WidgetPolicy | null | undefined,
): WidgetPolicy {
	const normalized: WidgetPolicy = {};
	for (const name of policyCapabilities(policy)) normalized[name] = true;
	const csp: WidgetCsp = {};
	for (const key of WIDGET_CSP_KEYS) {
		const sources = [...new Set(policySources(policy, key))].sort(
			compareStrings,
		);
		if (sources.length > 0) csp[key] = sources;
	}
	if (Object.keys(csp).length > 0) normalized.csp = csp;
	return normalized;
}

export function capabilityPolicy(
	capabilities: readonly MicroWidgetCapability[],
): WidgetPolicy {
	const policy: WidgetPolicy = {};
	for (const name of MICRO_WIDGET_CAPABILITIES) {
		if (capabilities.includes(name)) policy[name] = true;
	}
	return policy;
}

/** Strict parse of a stored or received policy. Unknown fields are rejected, as in Rust. */
export function readWidgetPolicy(value: unknown): WidgetPolicy | null {
	if (!isRecord(value)) return null;
	const policy: WidgetPolicy = {};
	for (const [key, field] of Object.entries(value)) {
		if (key === "csp") {
			const csp = readWidgetCsp(field);
			if (!csp) return null;
			if (Object.keys(csp).length > 0) policy.csp = csp;
			continue;
		}
		if (!(MICRO_WIDGET_CAPABILITIES as readonly string[]).includes(key)) {
			return null;
		}
		if (typeof field !== "boolean") return null;
		if (field) policy[key as MicroWidgetCapability] = true;
	}
	return policy;
}

function readWidgetCsp(value: unknown): WidgetCsp | null {
	if (!isRecord(value)) return null;
	const csp: WidgetCsp = {};
	for (const [key, sources] of Object.entries(value)) {
		if (!isCspKey(key)) return null;
		if (
			!Array.isArray(sources) ||
			!sources.every(
				(source) => typeof source === "string" && source.length > 0,
			)
		) {
			return null;
		}
		if (sources.length > 0) csp[key] = [...sources];
	}
	return csp;
}

export type WidgetInputPathStep =
	| { kind: "key"; key: string }
	| { kind: "items" }
	| { kind: "values" };

/** Mirrors `parse_widget_input_path`: `root *( "." key / "[]" / ".*" )`, fewer than 6 steps. */
export function parseWidgetInputPath(
	path: string,
): { root: string; steps: WidgetInputPathStep[] } | null {
	const root = INPUT_PATH_KEY.exec(path)?.[0];
	if (!root) return null;
	const steps: WidgetInputPathStep[] = [];
	let rest = path.slice(root.length);
	while (rest.length > 0) {
		if (rest.startsWith("[]")) {
			steps.push({ kind: "items" });
			rest = rest.slice(2);
		} else if (rest.startsWith(".*")) {
			steps.push({ kind: "values" });
			rest = rest.slice(2);
		} else {
			if (!rest.startsWith(".")) return null;
			const key = INPUT_PATH_KEY.exec(rest.slice(1))?.[0];
			if (!key) return null;
			steps.push({ kind: "key", key });
			rest = rest.slice(1 + key.length);
		}
	}
	return steps.length < MAX_INPUT_PATH_STEPS ? { root, steps } : null;
}

type Fail = (reason: string) => never;

function onlyKeys(
	value: Record<string, unknown>,
	allowed: readonly string[],
	what: string,
	fail: Fail,
): void {
	const unknown = Object.keys(value).find((key) => !allowed.includes(key));
	if (unknown !== undefined)
		fail(`has an unknown field "${unknown}" in ${what}`);
}

function readStrictStringList(
	value: unknown,
	what: string,
	fail: Fail,
): string[] {
	if (!Array.isArray(value) || !value.every((item) => typeof item === "string"))
		fail(`has an invalid ${what}`);
	return [...(value as string[])];
}

function readTemplate(value: unknown, fail: Fail): WidgetUrlTemplate {
	if (!isRecord(value)) fail("has an invalid network input template");
	const template = value as Record<string, unknown>;
	onlyKeys(
		template,
		["subdomains", "subdomainsInput"],
		"a network input template",
		fail,
	);
	const read: WidgetUrlTemplate = {};
	if (template.subdomains !== undefined) {
		const subdomains = readStrictStringList(
			template.subdomains,
			"template subdomain list",
			fail,
		);
		if (
			subdomains.length > MAX_TEMPLATE_SUBDOMAINS ||
			!subdomains.every((label) => TEMPLATE_SUBDOMAIN.test(label))
		) {
			fail("has invalid template subdomains");
		}
		if (subdomains.length > 0) read.subdomains = subdomains;
	}
	if (template.subdomainsInput !== undefined) {
		if (
			typeof template.subdomainsInput !== "string" ||
			INPUT_PATH_KEY.exec(template.subdomainsInput)?.[0] !==
				template.subdomainsInput
		) {
			fail('has an invalid template "subdomainsInput"');
		}
		read.subdomainsInput = template.subdomainsInput as string;
	}
	return read;
}

function readNetworkInputs(
	value: unknown,
	fail: Fail,
): WidgetNetworkInputSlot[] {
	if (value === undefined || value === null) return [];
	if (!Array.isArray(value)) fail('has an invalid "networkInputs"');
	const paths = new Set<string>();
	return (value as unknown[]).map((entry) => {
		if (!isRecord(entry)) fail('has an invalid entry in "networkInputs"');
		const slot = entry as Record<string, unknown>;
		onlyKeys(
			slot,
			["path", "purpose", "directives", "template"],
			"a network input",
			fail,
		);
		const { path, purpose, directives, template } = slot;
		if (
			typeof path !== "string" ||
			!parseWidgetInputPath(path) ||
			paths.has(path)
		)
			fail(`has an invalid network input path ${JSON.stringify(path)}`);
		paths.add(path as string);
		if (
			typeof purpose !== "number" ||
			!Number.isInteger(purpose) ||
			purpose < 0
		) {
			fail(`has an invalid purpose for network input "${path}"`);
		}
		if (
			!Array.isArray(directives) ||
			directives.length === 0 ||
			!directives.every(isCspKey) ||
			new Set(directives).size !== directives.length
		) {
			fail(`has invalid directives for network input "${path}"`);
		}
		const read: WidgetNetworkInputSlot = {
			path: path as string,
			purpose: purpose as number,
			directives: [...(directives as WidgetCspKey[])],
		};
		if (template !== undefined && template !== null) {
			read.template = readTemplate(template, fail);
		}
		return read;
	});
}

function readPlatformStorage(
	value: unknown,
	fail: Fail,
): PlatformStorageScope[] {
	if (!Array.isArray(value)) fail('has an invalid "platformStorage"');
	return (value as unknown[]).map((entry) => {
		if (!isRecord(entry)) fail('has an invalid entry in "platformStorage"');
		const scope = entry as Record<string, unknown>;
		onlyKeys(scope, ["origin", "pathPrefix"], "a platform storage scope", fail);
		const { origin, pathPrefix } = scope;
		if (typeof origin !== "string" || !HTTPS_ORIGIN.test(origin))
			fail(`has an invalid platform storage origin ${JSON.stringify(origin)}`);
		if (
			typeof pathPrefix !== "string" ||
			!pathPrefix.startsWith("/") ||
			!pathPrefix.endsWith("/")
		) {
			fail(
				`has an invalid platform storage path prefix ${JSON.stringify(pathPrefix)}`,
			);
		}
		return { origin: origin as string, pathPrefix: pathPrefix as string };
	});
}

const RUNTIME_STATUSES: readonly WidgetRuntimeStatus[] = [
	"none",
	"ok",
	"unavailable",
	"invalid",
];

function readRuntime(value: unknown, fail: Fail): WidgetRuntimeDescriptor {
	if (!isRecord(value)) fail('has an invalid "runtime"');
	const runtime = value as Record<string, unknown>;
	const { status, declaredDigest, runtimeDigest, rejected, invalidReason } =
		runtime;
	if (!RUNTIME_STATUSES.includes(status as WidgetRuntimeStatus))
		fail('has an invalid "runtime.status"');
	if (typeof declaredDigest !== "string" || !POLICY_DIGEST.test(declaredDigest))
		fail('has an invalid "runtime.declaredDigest"');
	const read: WidgetRuntimeDescriptor = {
		status: status as WidgetRuntimeStatus,
		declaredDigest: declaredDigest as string,
		rejected: [],
	};
	if (runtimeDigest !== undefined && runtimeDigest !== null) {
		if (typeof runtimeDigest !== "string" || !POLICY_DIGEST.test(runtimeDigest))
			fail('has an invalid "runtime.runtimeDigest"');
		if (read.status !== "ok") fail('has a "runtime.runtimeDigest" without ok');
		read.runtimeDigest = runtimeDigest as string;
	}
	if (rejected !== undefined && rejected !== null) {
		if (!Array.isArray(rejected)) fail('has an invalid "runtime.rejected"');
		read.rejected = (rejected as unknown[]).map((entry) => {
			if (!isRecord(entry)) fail('has an invalid "runtime.rejected" entry');
			const rejection = entry as Record<string, unknown>;
			onlyKeys(
				rejection,
				["slot", "source", "code"],
				"a runtime rejection",
				fail,
			);
			if (
				typeof rejection.slot !== "string" ||
				typeof rejection.source !== "string" ||
				typeof rejection.code !== "string"
			) {
				fail('has an invalid "runtime.rejected" entry');
			}
			return {
				slot: rejection.slot as string,
				source: rejection.source as string,
				code: rejection.code as string,
			};
		});
	}
	if (invalidReason !== undefined && invalidReason !== null) {
		if (typeof invalidReason !== "string")
			fail('has an invalid "runtime.invalidReason"');
		read.invalidReason = invalidReason as string;
	}
	return read;
}

function readEngine(value: unknown): WidgetEngineSupport | undefined {
	if (!isRecord(value)) return undefined;
	const { wildcardSources, runtimeSources, localMedia } = value;
	if (
		typeof wildcardSources !== "boolean" ||
		typeof runtimeSources !== "boolean" ||
		typeof localMedia !== "boolean"
	) {
		return undefined;
	}
	return { wildcardSources, runtimeSources, localMedia };
}

function codePointLength(value: string): number {
	return [...value].length;
}

function readNetworkSource(
	value: unknown,
	policy: WidgetPolicy,
): WidgetNetworkSource | null {
	if (!isRecord(value)) return null;
	const { source, directives, origin, slot, kind, level, host, emphasis } =
		value;
	if (
		typeof source !== "string" ||
		typeof kind !== "string" ||
		typeof host !== "string" ||
		host.length === 0 ||
		(origin !== "declared" && origin !== "runtime") ||
		!Array.isArray(directives)
	) {
		return null;
	}
	const listed = WIDGET_CSP_KEYS.filter(
		(key) =>
			directives.includes(key) && policySources(policy, key).includes(source),
	);
	if (listed.length === 0) return null;
	const read: WidgetNetworkSource = {
		source,
		directives: listed,
		origin,
		kind,
		level: readWidgetSourceLevel(level),
		host,
		emphasis:
			typeof emphasis === "string" &&
			emphasis.length > 0 &&
			(host === emphasis || host.endsWith(`.${emphasis}`))
				? emphasis
				: host,
	};
	if (typeof slot === "string") read.slot = slot;
	if (typeof value.providerId === "string") read.providerId = value.providerId;
	if (
		typeof value.provider === "string" &&
		value.provider.length > 0 &&
		codePointLength(value.provider) <= MAX_PROVIDER_LENGTH
	) {
		read.provider = value.provider;
	}
	if (isWidgetSourceAboutKey(value.aboutKey)) read.aboutKey = value.aboutKey;
	return read;
}

/**
 * Lenient parse of the display-only classification (§14.2.6). Anything that
 * would misstate what the policy grants makes the whole object malformed, and
 * the dialog falls back to a single broad card.
 */
export function readWidgetNetwork(
	value: unknown,
	policy: WidgetPolicy,
): WidgetNetwork | undefined {
	if (!isRecord(value) || !Array.isArray(value.purposes)) return undefined;
	const { catalogVersion, pslVersion, stale } = value;
	if (
		typeof catalogVersion !== "number" ||
		typeof pslVersion !== "string" ||
		typeof stale !== "boolean"
	) {
		return undefined;
	}
	const purposes: WidgetNetworkPurpose[] = [];
	for (const entry of value.purposes as unknown[]) {
		if (
			!isRecord(entry) ||
			typeof entry.reason !== "string" ||
			!Array.isArray(entry.sources)
		) {
			return undefined;
		}
		const sources = (entry.sources as unknown[])
			.map((source) => readNetworkSource(source, policy))
			.filter((source): source is WidgetNetworkSource => source !== null);
		const purpose: WidgetNetworkPurpose = {
			reason: entry.reason,
			level:
				maxWidgetSourceLevel([
					readWidgetSourceLevel(entry.level),
					...sources.map((source) => source.level),
				]) ?? "broad",
			sources,
		};
		if (
			Array.isArray(entry.inputs) &&
			entry.inputs.every((input) => typeof input === "string")
		) {
			purpose.inputs = [...(entry.inputs as string[])];
		}
		purposes.push(purpose);
	}
	const classified = new Set(
		purposes.flatMap((purpose) =>
			purpose.sources.flatMap((source) =>
				source.directives.map((key) => `${key} ${source.source}`),
			),
		),
	);
	if (
		policySourceEntries(policy).some(
			([key, source]) => !classified.has(`${key} ${source}`),
		)
	) {
		return undefined;
	}
	return {
		level:
			maxWidgetSourceLevel([
				readWidgetSourceLevel(value.level),
				...purposes.map((purpose) => purpose.level),
			]) ?? "broad",
		catalogVersion,
		pslVersion,
		stale,
		purposes,
	};
}

type DescriptorExpectation = Partial<
	Pick<
		WidgetPolicyDescriptor,
		"packageId" | "packageVersion" | "bundleHash" | "widgetId" | "preview"
	>
>;

function describeSubject(value: Record<string, unknown>): string {
	const packageId = typeof value.packageId === "string" ? value.packageId : "?";
	const widgetId = typeof value.widgetId === "string" ? value.widgetId : "?";
	return `${packageId}/${widgetId}`;
}

/**
 * Validate a describe response and require it to answer for the widget that
 * was asked about. Throws with the offending field so a skewed backend shows
 * up as an error rather than as a silently narrower dialog. `networkInputs`,
 * `platformStorage` and `runtime` shape enforcement requests and are parsed
 * strictly; `engine` and `network` are display facts and parsed leniently.
 */
export function parseWidgetPolicyDescriptor(
	value: unknown,
	expected: DescriptorExpectation = {},
): WidgetPolicyDescriptor {
	if (!isRecord(value)) {
		throw new Error("Widget policy response is not an object");
	}
	const subject = describeSubject(value);
	const fail: Fail = (reason: string): never => {
		throw new Error(`Widget policy response for ${subject} ${reason}`);
	};
	const string = (key: string): string => {
		const field = value[key];
		if (typeof field !== "string" || field.length === 0) {
			fail(`has no valid "${key}"`);
		}
		return field as string;
	};
	const optionalString = (key: string): string | undefined => {
		if (!hasOwn(value, key) || value[key] === undefined || value[key] === null)
			return undefined;
		return string(key);
	};

	const status = value.status;
	if (status !== "ok" && status !== "invalid") fail('has no valid "status"');
	if (typeof value.preview !== "boolean") fail('has no valid "preview"');
	const policy = readWidgetPolicy(value.policy);
	if (!policy) fail('has an unsupported "policy"');
	const policyDigest = string("policyDigest");
	if (!POLICY_DIGEST.test(policyDigest)) fail('has no valid "policyDigest"');

	const descriptor: WidgetPolicyDescriptor = {
		source: string("source"),
		packageId: string("packageId"),
		bundleHash: string("bundleHash"),
		widgetId: string("widgetId"),
		preview: value.preview as boolean,
		status: status as WidgetPolicyStatus,
		policy: policy as WidgetPolicy,
		policyDigest,
		networkInputs: [],
	};
	const packageVersion = optionalString("packageVersion");
	if (packageVersion !== undefined) descriptor.packageVersion = packageVersion;
	const invalidReason = optionalString("invalidReason");
	if (invalidReason !== undefined) descriptor.invalidReason = invalidReason;

	const networkInputs = readNetworkInputs(value.networkInputs, fail);
	const platformStorage =
		value.platformStorage === undefined || value.platformStorage === null
			? undefined
			: readPlatformStorage(value.platformStorage, fail);
	const runtime =
		value.runtime === undefined || value.runtime === null
			? undefined
			: readRuntime(value.runtime, fail);
	if (descriptor.status === "ok" && !descriptor.preview) {
		descriptor.networkInputs = networkInputs;
		if (platformStorage) descriptor.platformStorage = platformStorage;
		if (runtime) descriptor.runtime = runtime;
		const engine = readEngine(value.engine);
		if (engine) descriptor.engine = engine;
		const network = readWidgetNetwork(value.network, descriptor.policy);
		if (network) descriptor.network = network;
	}

	for (const key of Object.keys(expected) as (keyof DescriptorExpectation)[]) {
		const want = expected[key];
		if (want !== undefined && descriptor[key] !== want) {
			fail(
				`answers for ${key} ${JSON.stringify(descriptor[key])} instead of ${JSON.stringify(want)}`,
			);
		}
	}
	return descriptor;
}

export function parseWidgetGrantResponse(
	value: unknown,
	isGrant: (grant: string) => boolean,
): WidgetGrantResponse {
	if (!isRecord(value)) {
		throw new Error("Widget grant response is not an object");
	}
	const { grant, expiresIn, policyDigest, runtime } = value;
	if (grant !== null && (typeof grant !== "string" || !isGrant(grant))) {
		throw new Error("Widget grant response carries a malformed grant");
	}
	if (
		typeof expiresIn !== "number" ||
		!Number.isFinite(expiresIn) ||
		expiresIn < 0
	) {
		throw new Error(
			`Widget grant response has an invalid expiresIn: ${JSON.stringify(expiresIn)}`,
		);
	}
	if (typeof policyDigest !== "string" || !POLICY_DIGEST.test(policyDigest)) {
		throw new Error("Widget grant response has no valid policyDigest");
	}
	if (runtime !== undefined && runtime !== null) {
		if (typeof runtime !== "string" || !isWidgetRuntimeComponent(runtime)) {
			throw new Error(
				"Widget grant response carries a malformed runtime component",
			);
		}
		if (grant === null) {
			throw new Error(
				"Widget grant response carries a runtime component without a grant",
			);
		}
	}
	return {
		grant,
		expiresIn,
		policyDigest,
		runtime: typeof runtime === "string" ? runtime : null,
	};
}

/** Distinct sources of a runtime request, sorted. */
export function widgetRuntimeRequestSources(
	request: readonly WidgetRuntimeSourceRequest[],
): string[] {
	return [...new Set(request.flatMap((entry) => entry.sources))].sort(
		compareStrings,
	);
}

/** Canonical request: slots sorted, each with sorted distinct sources, empty slots dropped. */
export function canonicalWidgetRuntimeRequest(
	request: readonly WidgetRuntimeSourceRequest[],
): WidgetRuntimeSourceRequest[] {
	const slots = new Map<string, Set<string>>();
	for (const { slot, sources } of request) {
		const set = slots.get(slot) ?? new Set<string>();
		for (const source of sources) set.add(source);
		slots.set(slot, set);
	}
	return [...slots.entries()]
		.filter(([, sources]) => sources.size > 0)
		.sort(([left], [right]) => compareStrings(left, right))
		.map(([slot, sources]) => ({
			slot,
			sources: [...sources].sort(compareStrings),
		}));
}

export function widgetRuntimeRequestKey(
	request: readonly WidgetRuntimeSourceRequest[],
): string {
	return JSON.stringify(
		canonicalWidgetRuntimeRequest(request).map(({ slot, sources }) => [
			slot,
			sources,
		]),
	);
}

/** One approved (or requested) runtime source for one directive. */
export interface WidgetRuntimeSourceEntry {
	directive: WidgetCspKey;
	source: string;
	level: WidgetSourceLevel;
	slot: string | null;
}

export interface WidgetDeclaredPart {
	policy: WidgetPolicy;
	/** Exactly the sources of `policy.csp`. */
	levels: Record<string, WidgetSourceLevel>;
}

export interface WidgetDescriptorSplit {
	declared: WidgetDeclaredPart;
	runtime: WidgetRuntimeSourceEntry[];
}

/**
 * Splits a descriptor into the part the publisher declared and the sources
 * that arrived at runtime (§14.4.8). The runtime part comes from `network`
 * sources with `origin: "runtime"`; without `network` it is the effective
 * policy minus `declaredPolicy` (the declared-only descriptor's policy), and
 * every level falls back to `broad`.
 */
export function splitWidgetPolicyDescriptor(
	descriptor: Pick<WidgetPolicyDescriptor, "policy" | "network">,
	declaredPolicy?: WidgetPolicy | null,
): WidgetDescriptorSplit {
	const effective = normalizeWidgetPolicy(descriptor.policy);
	const levels = new Map<string, WidgetSourceLevel>();
	const runtime: WidgetRuntimeSourceEntry[] = [];
	const network = descriptor.network;
	if (network) {
		for (const purpose of network.purposes) {
			for (const source of purpose.sources) {
				const previous = levels.get(source.source);
				levels.set(
					source.source,
					previous &&
						widgetSourceLevelRank(previous) >
							widgetSourceLevelRank(source.level)
						? previous
						: source.level,
				);
				if (source.origin !== "runtime") continue;
				for (const directive of source.directives) {
					runtime.push({
						directive,
						source: source.source,
						level: source.level,
						slot: source.slot ?? null,
					});
				}
			}
		}
	} else if (declaredPolicy) {
		for (const [directive, source] of policySourceEntries(effective)) {
			if (!policySources(declaredPolicy, directive).includes(source)) {
				runtime.push({ directive, source, level: "broad", slot: null });
			}
		}
	}
	const runtimePairs = new Set(
		runtime.map((entry) => `${entry.directive} ${entry.source}`),
	);
	const declared: WidgetPolicy = {};
	for (const name of policyCapabilities(effective)) declared[name] = true;
	const csp: WidgetCsp = {};
	const declaredLevels: Record<string, WidgetSourceLevel> = {};
	for (const [directive, source] of policySourceEntries(effective)) {
		if (runtimePairs.has(`${directive} ${source}`)) continue;
		csp[directive] = [...(csp[directive] ?? []), source];
		declaredLevels[source] = levels.get(source) ?? "broad";
	}
	if (Object.keys(csp).length > 0) declared.csp = csp;
	runtime.sort(
		(left, right) =>
			compareStrings(left.source, right.source) ||
			WIDGET_CSP_KEYS.indexOf(left.directive) -
				WIDGET_CSP_KEYS.indexOf(right.directive),
	);
	return { declared: { policy: declared, levels: declaredLevels }, runtime };
}

/** The backend derived a different policy than the one the user approved. */
export class WidgetPolicyChangedError extends Error {
	readonly code = POLICY_CHANGED;

	constructor(
		message = "The widget's permissions changed since they were approved",
	) {
		super(message);
		this.name = "WidgetPolicyChangedError";
	}
}

function startsWithCode(text: unknown, code: string): boolean {
	return typeof text === "string" && text.trimStart().startsWith(code);
}

function errorText(error: Record<string, unknown>): unknown {
	return typeof error.error === "string" ? error.error : error.message;
}

/**
 * Mint rejected a stale digest: desktop rejects with an error string starting
 * with `policy_changed` (bare or as `{ error }`), web answers HTTP 409.
 */
export function isPolicyChangedError(error: unknown): boolean {
	if (!isRecord(error)) return startsWithCode(error, POLICY_CHANGED);
	if (error.code === POLICY_CHANGED || error.status === 409) return true;
	return startsWithCode(errorText(error), POLICY_CHANGED);
}

/** Web answers 503 when it has no signing key; the host then mounts the baseline frame. */
export function isWidgetGrantUnavailableError(error: unknown): boolean {
	return isRecord(error) && error.status === 503;
}

export type WidgetRuntimeSourcesErrorCode =
	| "INVALID_RUNTIME_SOURCES"
	| "RUNTIME_SOURCES_IN_PREVIEW";

const RUNTIME_ERROR_PREFIXES: Readonly<
	Record<WidgetRuntimeSourcesErrorCode, string>
> = {
	INVALID_RUNTIME_SOURCES: "invalid_runtime_sources",
	RUNTIME_SOURCES_IN_PREVIEW: "runtime_sources_in_preview",
};

/** The backend refused the shape of a runtime request: a host bug, never a user decision. */
export class WidgetRuntimeSourcesError extends Error {
	constructor(
		readonly code: WidgetRuntimeSourcesErrorCode,
		message: string,
	) {
		super(message);
		this.name = "WidgetRuntimeSourcesError";
	}
}

/**
 * Desktop rejects with `invalid_runtime_sources: …` or
 * `runtime_sources_in_preview: …` (bare or as `{ error }`); web answers 400
 * with the upper-case code.
 */
export function widgetRuntimeSourcesErrorCode(
	error: unknown,
): WidgetRuntimeSourcesErrorCode | null {
	for (const code of Object.keys(
		RUNTIME_ERROR_PREFIXES,
	) as WidgetRuntimeSourcesErrorCode[]) {
		const prefix = RUNTIME_ERROR_PREFIXES[code];
		if (!isRecord(error)) {
			if (startsWithCode(error, prefix)) return code;
			continue;
		}
		if (error.code === code || startsWithCode(errorText(error), prefix))
			return code;
	}
	return null;
}

/** An API that predates runtime sources answers the describe `POST` with 404 or 405. */
export function isWidgetRuntimeDescribeUnsupportedError(
	error: unknown,
): boolean {
	return isRecord(error) && (error.status === 404 || error.status === 405);
}
