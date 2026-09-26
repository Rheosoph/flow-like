import { useTranslation } from "@flow-like/locales";
import type { WidgetContract } from "@flow-like/widget-sdk";
import { useMemo } from "react";
import {
	WIDGET_CSP_KEYS,
	type WidgetCsp,
	type WidgetCspKey,
	type WidgetNetwork,
	type WidgetNetworkSource,
	type WidgetSourceLevel,
	type WidgetUrlTemplate,
	maxWidgetSourceLevel,
	policyHosts,
	readWidgetNetwork,
} from "../components/a2ui/micro-widget-policy";
import type { PackageWidgetEntry } from "./schema/wasm";

/**
 * Capability tags come off `PackageSummary.capabilities`, derived server-side
 * from the permissions the package's nodes declare; the manifest only authors
 * resource tiers, the host allowlist and OAuth scopes. `elevated` marks the
 * ones that let a package reach beyond its sandbox — network, OAuth, model
 * budget, user storage — so listing surfaces can keep those visible when they
 * truncate.
 */
export type CapabilitySeverity = "elevated" | "standard";

export interface ResolvedCapability {
	key: string;
	label: string;
	severity: CapabilitySeverity;
}

/** Listing tag for packages with a widget that declares network addresses or network inputs. */
export const WIDGET_NET_CAPABILITY = "widget.net";

const ELEVATED = new Set([
	"net.http",
	"net.ws",
	"net.tcp",
	"net.udp",
	"net.dns",
	WIDGET_NET_CAPABILITY,
	"oauth",
	"models",
	"storage.user",
	"database.read",
	"database.write",
]);

export function capabilitySeverity(key: string): CapabilitySeverity {
	return ELEVATED.has(key) ? "elevated" : "standard";
}

export function usePackageCapabilities(
	keys: readonly string[] | undefined,
): ResolvedCapability[] {
	const { t } = useTranslation("store");

	return useMemo(() => {
		if (!keys?.length) return [];

		const labels: Record<string, string> = {
			"net.http": t("capabilityNetHttp", "Makes HTTP requests"),
			"net.ws": t("capabilityNetWs", "Opens WebSocket connections"),
			"net.tcp": t("capabilityNetTcp", "Opens TCP connections"),
			"net.udp": t("capabilityNetUdp", "Opens UDP connections"),
			"net.dns": t("capabilityNetDns", "Resolves DNS names"),
			[WIDGET_NET_CAPABILITY]: t(
				"capabilityWidgetNet",
				"Has widgets that ask to reach external sites",
			),
			oauth: t("capabilityOauth", "Acts on your behalf via OAuth"),
			models: t("capabilityModels", "Calls language models"),
			"storage.user": t("capabilityStorageUser", "Reads and writes your files"),
			"database.read": t("capabilityDatabaseRead", "Reads connected databases"),
			"database.write": t(
				"capabilityDatabaseWrite",
				"Writes connected databases",
			),
			"storage.node": t("capabilityStorageNode", "Uses node-scoped storage"),
			"storage.uploads": t("capabilityStorageUploads", "Reads uploaded files"),
			"storage.cache": t("capabilityStorageCache", "Uses the cache directory"),
			variables: t("capabilityVariables", "Reads execution variables"),
			cache: t("capabilityCache", "Uses the execution cache"),
			streaming: t("capabilityStreaming", "Streams output"),
			a2ui: t("capabilityA2ui", "Renders interface elements"),
		};

		return keys.map((key) => ({
			key,
			label: labels[key] ?? key,
			severity: capabilitySeverity(key),
		}));
	}, [keys, t]);
}

/** A granted policy or a flattened contract: sources per directive. */
export interface WidgetNetworkDeclaration {
	readonly csp?: WidgetCsp;
}

/** A network input of one purpose: the host asks the viewer about every address it carries. */
export interface WidgetDeclaredInput {
	path: string;
	directives: WidgetCspKey[];
	template?: WidgetUrlTemplate;
}

/** One purpose group of a widget contract, read for display. */
export interface WidgetDeclaredPurpose {
	reason: string;
	csp: WidgetCsp;
	inputs: WidgetDeclaredInput[];
}

export interface WidgetPurposeSource {
	source: string;
	directives: WidgetCspKey[];
	/** The hub's classification; absent when it sent none. */
	class?: WidgetNetworkSource;
}

export interface WidgetClassifiedPurpose {
	reason: string;
	/** Absent without classification. */
	level?: WidgetSourceLevel;
	sources: WidgetPurposeSource[];
	inputs: WidgetDeclaredInput[];
}

export type WidgetReviewFlag =
	| { kind: "broad"; source: string; provider: string }
	| { kind: "shared"; source: string }
	| { kind: "runtime-connect"; input: string }
	| { kind: "platform-storage"; input: string };

function isRecord(value: unknown): value is Record<string, unknown> {
	return typeof value === "object" && value !== null && !Array.isArray(value);
}

function compareStrings(left: string, right: string): number {
	return left < right ? -1 : left > right ? 1 : 0;
}

function readSourceList(value: unknown): string[] {
	if (!Array.isArray(value)) return [];
	const sources = value.filter(
		(item): item is string => typeof item === "string" && item.length > 0,
	);
	return [...new Set(sources)].sort(compareStrings);
}

function readDirectives(value: unknown): WidgetCspKey[] {
	return Array.isArray(value)
		? WIDGET_CSP_KEYS.filter((key) => value.includes(key))
		: [];
}

function readTemplate(value: unknown): WidgetUrlTemplate | undefined {
	if (!isRecord(value)) return undefined;
	const template: WidgetUrlTemplate = {};
	const subdomains = readSourceList(value.subdomains);
	if (subdomains.length > 0) template.subdomains = subdomains;
	if (
		typeof value.subdomainsInput === "string" &&
		value.subdomainsInput.length > 0
	) {
		template.subdomainsInput = value.subdomainsInput;
	}
	return template.subdomains || template.subdomainsInput ? template : undefined;
}

function readInput(value: unknown): WidgetDeclaredInput | null {
	if (
		!isRecord(value) ||
		typeof value.path !== "string" ||
		value.path.length === 0
	) {
		return null;
	}
	const directives = readDirectives(value.directives);
	if (directives.length === 0) return null;
	const template = readTemplate(value.template);
	return template
		? { path: value.path, directives, template }
		: { path: value.path, directives };
}

function readPurpose(value: unknown): WidgetDeclaredPurpose | null {
	if (!isRecord(value)) return null;
	const csp: WidgetCsp = {};
	for (const key of WIDGET_CSP_KEYS) {
		const sources = readSourceList(value[key]);
		if (sources.length > 0) csp[key] = sources;
	}
	const inputs = Array.isArray(value.inputs)
		? value.inputs
				.map(readInput)
				.filter((input): input is WidgetDeclaredInput => input !== null)
				.sort((left, right) => compareStrings(left.path, right.path))
		: [];
	if (Object.keys(csp).length === 0 && inputs.length === 0) return null;
	return {
		reason: typeof value.reason === "string" ? value.reason : "",
		csp,
		inputs,
	};
}

/**
 * Purpose groups a widget contract declares, in declaration order. Manifest
 * contracts are publisher-written, so malformed entries are skipped; the
 * viewer still approves every address before the widget can reach it.
 */
export function readWidgetPurposes(
	contract: WidgetContract | null | undefined,
): WidgetDeclaredPurpose[] {
	const csp: unknown = contract?.csp;
	if (!Array.isArray(csp)) return [];
	return csp
		.map(readPurpose)
		.filter((purpose): purpose is WidgetDeclaredPurpose => purpose !== null);
}

/** Per-directive union over every purpose, as the enforced policy flattens it. */
export function flattenWidgetPurposes(
	purposes: readonly WidgetDeclaredPurpose[],
): WidgetCsp {
	const csp: WidgetCsp = {};
	for (const key of WIDGET_CSP_KEYS) {
		const sources = readSourceList(
			purposes.flatMap((purpose) => purpose.csp[key] ?? []),
		);
		if (sources.length > 0) csp[key] = sources;
	}
	return csp;
}

/** `scheme://host[/path]` → `host`; a wildcard keeps its `*.`. */
export function widgetSourceHost(source: string): string {
	const separator = source.indexOf("://");
	const rest = separator < 0 ? source : source.slice(separator + 3);
	const slash = rest.indexOf("/");
	return slash < 0 ? rest : rest.slice(0, slash);
}

/** Each source of a purpose once, with every directive it is declared under. */
function widgetPurposeSources(
	purpose: WidgetDeclaredPurpose,
): Omit<WidgetPurposeSource, "class">[] {
	const directives = new Map<string, WidgetCspKey[]>();
	for (const key of WIDGET_CSP_KEYS) {
		for (const source of purpose.csp[key] ?? []) {
			directives.set(source, [...(directives.get(source) ?? []), key]);
		}
	}
	return [...directives.entries()]
		.sort(([left], [right]) => compareStrings(left, right))
		.map(([source, keys]) => ({ source, directives: keys }));
}

/**
 * Joins declared purposes with the hub's classification. A purpose takes the
 * riskiest level of its sources; one with inputs is at least `external`, as
 * the hub computes it.
 */
export function classifyWidgetPurposes(
	purposes: readonly WidgetDeclaredPurpose[],
	network: WidgetNetwork | undefined,
): WidgetClassifiedPurpose[] {
	const classes = new Map<string, WidgetNetworkSource>();
	for (const purpose of network?.purposes ?? []) {
		for (const source of purpose.sources) {
			if (source.origin === "declared") classes.set(source.source, source);
		}
	}
	return purposes.map((purpose, index) => {
		const sources = widgetPurposeSources(purpose).map(
			(entry): WidgetPurposeSource => {
				const found = classes.get(entry.source);
				return found ? { ...entry, class: found } : entry;
			},
		);
		const classified: WidgetClassifiedPurpose = {
			reason: purpose.reason,
			sources,
			inputs: purpose.inputs,
		};
		if (!network) return classified;
		const aligned = network.purposes[index];
		const levels: WidgetSourceLevel[] = sources.map(
			(entry) => entry.class?.level ?? "broad",
		);
		if (aligned?.reason === purpose.reason) levels.push(aligned.level);
		if (purpose.inputs.length > 0) levels.push("external");
		classified.level = maxWidgetSourceLevel(levels) ?? "known";
		return classified;
	});
}

export interface PackageWidgetNetworkView {
	purposes: WidgetClassifiedPurpose[];
	/** The hub's display-only classification; absent when it sent none or a malformed one. */
	network: WidgetNetwork | undefined;
	/** Distinct declared hosts; a wildcard counts once. */
	hosts: string[];
	hasInputs: boolean;
}

/** What a package widget declares, joined with the hub's classification when it is well-formed. */
export function describePackageWidgetNetwork(
	widget: Pick<PackageWidgetEntry, "contract" | "network">,
): PackageWidgetNetworkView {
	const declared = readWidgetPurposes(widget.contract);
	const csp = flattenWidgetPurposes(declared);
	const network = readWidgetNetwork(widget.network, { csp });
	return {
		purposes: classifyWidgetPurposes(declared, network),
		network,
		hosts: widgetNetworkHosts({ csp }),
		hasInputs: declared.some((purpose) => purpose.inputs.length > 0),
	};
}

/** Reviewer checks, derived only from the hub's classification and the declared inputs. */
/**
 * Any runtime input can carry a URL into this app's Flow-Like storage, which the
 * host accepts as a path-scoped source, so every input slot is flagged for it.
 */
export function widgetReviewFlags(
	purposes: readonly WidgetClassifiedPurpose[],
): WidgetReviewFlag[] {
	const sources = purposes.flatMap((purpose) => purpose.sources);
	const flags: WidgetReviewFlag[] = [];
	const flagged = new Set<string>();
	for (const level of ["broad", "shared"] as const) {
		for (const { source, class: found } of sources) {
			if (found?.level !== level || flagged.has(source)) continue;
			flagged.add(source);
			flags.push(
				level === "broad"
					? {
							kind: "broad",
							source,
							provider: found.provider ?? found.emphasis,
						}
					: { kind: "shared", source },
			);
		}
	}
	const inputs = purposes.flatMap((purpose) => purpose.inputs);
	for (const input of inputs) {
		if (input.directives.includes("connectSrc")) {
			flags.push({ kind: "runtime-connect", input: input.path });
		}
	}
	for (const path of [...new Set(inputs.map((input) => input.path))].sort(
		compareStrings,
	)) {
		flags.push({ kind: "platform-storage", input: path });
	}
	return flags;
}

/** Distinct hosts across every directive; `wss://h` and `https://h` count once, a wildcard counts once. */
export function widgetNetworkHosts(
	declaration: WidgetNetworkDeclaration | null | undefined,
): string[] {
	return policyHosts(declaration);
}
