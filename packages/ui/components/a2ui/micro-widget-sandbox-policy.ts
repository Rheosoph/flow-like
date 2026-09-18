import {
	type MicroWidgetCapability,
	WIDGET_CSP_DIRECTIVES,
	type WidgetCspKey,
	type WidgetPolicy,
	policyCapabilities,
	policySources,
} from "./micro-widget-policy";

export type MicroWidgetCspKeyword =
	| "'none'"
	| "'unsafe-inline'"
	| "'wasm-unsafe-eval'"
	| "data:"
	| "blob:"
	| "bundle";

/**
 * A keyword, `bundle` for the widget's own packaged files whatever scheme
 * serves them, or an approved network source such as `https://tiles.example.com`.
 */
export type MicroWidgetCspSource = MicroWidgetCspKeyword | (string & {});

export interface MicroWidgetCspDirective {
	directive: string;
	sources: MicroWidgetCspSource[];
	grantedBy?: MicroWidgetCapability;
	/** Set when the row carries approved network sources from this contract key. */
	network?: WidgetCspKey;
}

export interface MicroWidgetSandboxToken {
	token: string;
	grantedBy?: MicroWidgetCapability;
}

export interface MicroWidgetSandboxPolicy {
	sandbox: MicroWidgetSandboxToken[];
	csp: MicroWidgetCspDirective[];
}

export interface MicroWidgetSandboxOptions {
	/** From the descriptor's `engine`; local media is on unless an engine gate turned it off. */
	localMedia?: boolean;
}

/** Local data only: never a network location, never part of a policy or its digest. */
export const MICRO_WIDGET_LOCAL_SOURCES: readonly MicroWidgetCspKeyword[] = [
	"data:",
	"blob:",
];

const CLOSED_DIRECTIVES = [
	"frame-src",
	"child-src",
	"object-src",
	"manifest-src",
	"base-uri",
	"form-action",
] as const;

export function isMicroWidgetNetworkSource(source: string): boolean {
	return source.includes("://");
}

export function isMicroWidgetLocalSource(source: string): boolean {
	return (MICRO_WIDGET_LOCAL_SOURCES as readonly string[]).includes(source);
}

/**
 * The document policy a widget runs under for an approved policy. Mirrors
 * `widget_document_csp` in `packages/wasm/schema/src/widget_frame.rs` row for
 * row; `micro-widget-sandbox-policy.test.ts` pins both to the shared fixture.
 */
export function describeMicroWidgetSandbox(
	policy: WidgetPolicy | null | undefined,
	options: MicroWidgetSandboxOptions = {},
): MicroWidgetSandboxPolicy {
	const capabilities = policyCapabilities(policy);
	const has = (name: MicroWidgetCapability) => capabilities.includes(name);
	const local = [...MICRO_WIDGET_LOCAL_SOURCES];

	const extended = (
		directive: string,
		base: MicroWidgetCspSource[],
		key: WidgetCspKey,
		capability?: MicroWidgetCapability,
	): MicroWidgetCspDirective => {
		const opened = capability !== undefined && has(capability);
		const hosts = policySources(policy, key);
		const sources = [...base, ...(opened ? ["bundle"] : []), ...hosts];
		return {
			directive,
			sources: sources.length > 0 ? sources : ["'none'"],
			...(opened ? { grantedBy: capability } : {}),
			...(hosts.length > 0 ? { network: key } : {}),
		};
	};
	const withBase = (base: MicroWidgetCspSource[], key: WidgetCspKey) =>
		extended(WIDGET_CSP_DIRECTIVES[key], base, key);

	const sandbox: MicroWidgetSandboxToken[] = [{ token: "allow-scripts" }];
	if (has("downloads")) {
		sandbox.push({ token: "allow-downloads", grantedBy: "downloads" });
	}

	const csp: MicroWidgetCspDirective[] = [
		{ directive: "default-src", sources: ["'none'"] },
		{
			directive: "script-src",
			sources: has("wasm")
				? ["'unsafe-inline'", "'wasm-unsafe-eval'", "bundle"]
				: ["'unsafe-inline'", "bundle"],
			...(has("wasm") ? { grantedBy: "wasm" as const } : {}),
		},
		withBase(["'unsafe-inline'", "bundle"], "styleSrc"),
		withBase(["data:", "blob:", "bundle"], "imgSrc"),
		withBase(["data:", "bundle"], "fontSrc"),
		extended("connect-src", local, "connectSrc", "workers"),
		{
			directive: "worker-src",
			...(has("workers")
				? { sources: ["blob:", "bundle"], grantedBy: "workers" as const }
				: { sources: ["'none'"] }),
		},
		extended(
			"media-src",
			options.localMedia === false ? [] : local,
			"mediaSrc",
			"media",
		),
		...CLOSED_DIRECTIVES.map((directive) => ({
			directive,
			sources: ["'none'"] as MicroWidgetCspSource[],
		})),
	];

	return { sandbox, csp };
}

/** Render the described policy the way the backend serializes it for these bundle sources. */
export function serializeMicroWidgetCsp(
	policy: MicroWidgetSandboxPolicy,
	bundleSources: readonly string[],
	includeSandbox = false,
): string {
	const directives = policy.csp.map(({ directive, sources }) => {
		const expanded = sources.flatMap((source) =>
			source === "bundle" ? bundleSources : [source],
		);
		return [directive, ...(expanded.length > 0 ? expanded : ["'none'"])].join(
			" ",
		);
	});
	if (includeSandbox) {
		directives.push(
			["sandbox", ...policy.sandbox.map(({ token }) => token)].join(" "),
		);
	}
	return directives.join("; ");
}
