export const CONTRACT_VERSION = 1;

export type JsonSchema = Record<string, unknown>;

export type ContractInputType =
	| "string"
	| "number"
	| "integer"
	| "boolean"
	| "enum"
	| "json";

export interface ContractInput {
	type: ContractInputType;
	description?: string;
	default?: unknown;
	choices?: string[];
	min?: number;
	max?: number;
	schema?: JsonSchema;
	optional?: boolean;
}

export interface ContractEvent {
	payloadSchema?: JsonSchema | null;
	description?: string;
}

export interface ContractQuery {
	argsSchema?: JsonSchema | null;
	resultSchema?: JsonSchema | null;
	description?: string;
	/** Mutations change widget state and require a live acknowledgement. */
	mutation?: boolean;
}

export interface WidgetSizing {
	defaultHeight?: number;
	resizable?: boolean;
	maxHeight?: number;
}

export interface WidgetCapabilities {
	workers?: boolean;
	media?: boolean;
	microphone?: boolean;
	wasm?: boolean;
	downloads?: boolean;
}

/** CSP fetch directives a widget may extend, in canonical order. */
export type WidgetCspDirective =
	| "connectSrc"
	| "imgSrc"
	| "fontSrc"
	| "mediaSrc"
	| "styleSrc";

/**
 * Network sources per CSP fetch directive. Each entry is `scheme://host` or
 * `scheme://*.host` (`https`, plus `wss` for `connectSrc`) without port or
 * path.
 */
export interface WidgetCsp {
	connectSrc?: string[];
	imgSrc?: string[];
	fontSrc?: string[];
	mediaSrc?: string[];
	styleSrc?: string[];
}

/** Explicit expansion of `{s}` placeholders in runtime URL hosts. */
export interface WidgetUrlTemplate {
	/** Lowercase DNS labels substituted for `{s}` */
	subdomains?: string[];
	/** A string or json input whose value lists the labels for `{s}` */
	subdomainsInput?: string;
}

/**
 * A widget input whose string values carry URLs. The host takes only
 * `scheme://host` from each value and asks the viewer to approve it.
 */
export interface WidgetNetworkInput {
	/** `root *( "." key / "[]" / ".*" )`, e.g. `tileUrl` or `layers[].url` */
	path: string;
	directives: WidgetCspDirective[];
	template?: WidgetUrlTemplate;
}

/**
 * One purpose group: a reason the viewer reads next to the sources, plus
 * static sources and/or network inputs.
 */
export interface WidgetCspPurpose extends WidgetCsp {
	reason: string;
	inputs?: WidgetNetworkInput[];
}

export interface WidgetContract {
	contractVersion: number;
	id: string;
	inputs?: Record<string, ContractInput>;
	events?: Record<string, ContractEvent>;
	queries?: Record<string, ContractQuery>;
	sizing?: WidgetSizing;
	capabilities?: WidgetCapabilities;
	/** Present only with `contractVersion` 2 */
	csp?: WidgetCspPurpose[];
}

export function contractDefaults(
	contract: WidgetContract | null | undefined,
): Record<string, unknown> {
	const defaults: Record<string, unknown> = {};
	if (!contract?.inputs) return defaults;
	for (const [key, input] of Object.entries(contract.inputs)) {
		if (input.default !== undefined) defaults[key] = input.default;
	}
	return defaults;
}
