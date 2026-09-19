// Producer-side (strict, canonical) mirror of packages/wasm/schema/src/widget.rs
// and of the purpose-group rules in widget_policy.rs;
// @flow-like/widget-sdk ships the consumer-side (loose) mirror. The
// assignability assertion at the bottom keeps the two from drifting.

import type {
	WidgetContract as SdkWidgetContract,
	WidgetCapabilities,
	WidgetCspPurpose,
	WidgetNetworkInput,
	WidgetUrlTemplate,
} from "@flow-like/widget-sdk";
import {
	type WidgetCspReasonRejection,
	cspReasonProblem,
	foldWidgetCspReason,
	normalizeCspReason,
	reasonContainsAddress,
	validateWidgetCspReason,
} from "./csp-reason";
import {
	CSP_DIRECTIVES,
	CSP_NETWORK_INPUT_KEYS,
	CSP_PURPOSE_KEYS,
	CSP_SOURCE_REJECTION_MESSAGES,
	CSP_TEMPLATE_KEYS,
	HOST_RESERVED_INPUT_KEYS,
	MAX_WIDGET_CSP_PURPOSES,
	MAX_WIDGET_CSP_SOURCES,
	MAX_WIDGET_CSP_SOURCE_BYTES,
	MAX_WIDGET_INPUT_PATH_SEGMENTS,
	MAX_WIDGET_NETWORK_INPUTS,
	MAX_WIDGET_TEMPLATE_SUBDOMAINS,
	canonicalCspSources,
	compareCspDirectives,
	compareCspSources,
	cspSourceBytes,
	cspSourceCount,
	flattenCspPurposes,
	isCspDirective,
	isDnsLabel,
	isStrictlyAscending,
	normalizeCspSource,
	parseWidgetInputPath,
	validateCspSource,
} from "./csp-source";
import { validateWildcardBases } from "./psl";

export type JsonValue =
	| string
	| number
	| boolean
	| null
	| JsonValue[]
	| { [key: string]: JsonValue };

export type JsonObject = { [key: string]: JsonValue };

/** Current contract version; contracts declaring `csp` must use it */
export const CONTRACT_VERSION = 2;
/** Version of contracts without `csp`, readable by hosts that predate `csp` */
export const BASE_CONTRACT_VERSION = 1;
export const WIDGET_PROTOCOL = "flw/1";

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
	default?: JsonValue;
	choices?: string[];
	min?: number;
	max?: number;
	schema?: JsonObject;
	optional?: boolean;
}

export interface ContractEvent {
	payloadSchema: JsonObject | null;
	description?: string;
}

export interface ContractQuery {
	argsSchema: JsonObject | null;
	resultSchema: JsonObject | null;
	description?: string;
	mutation?: boolean;
}

export interface WidgetSizing {
	defaultHeight: number;
	resizable: boolean;
	maxHeight?: number;
}

export interface WidgetContract {
	contractVersion: number;
	id: string;
	inputs: Record<string, ContractInput>;
	events: Record<string, ContractEvent>;
	queries: Record<string, ContractQuery>;
	sizing: WidgetSizing;
	capabilities?: WidgetCapabilities;
	/** Purpose groups; present only with `contractVersion` 2 */
	csp?: WidgetCspPurpose[];
}

/** Mirrors `is_valid_package_id` in packages/wasm/schema/src/widget_frame.rs */
export function isValidPackageId(id: string): boolean {
	return id !== "." && id !== ".." && /^[A-Za-z0-9._-]+$/.test(id);
}

export function isValidWidgetId(id: string): boolean {
	return (
		id.length > 0 &&
		!id.startsWith("-") &&
		!id.endsWith("-") &&
		/^[a-z0-9-]+$/.test(id)
	);
}

export function isValidMemberKey(key: string): boolean {
	return /^[a-zA-Z_][a-zA-Z0-9_]*$/.test(key);
}

function defaultMatchesType(value: JsonValue, input: ContractInput): boolean {
	switch (input.type) {
		case "string":
			return typeof value === "string";
		case "number":
			return typeof value === "number";
		case "integer":
			return typeof value === "number" && Number.isInteger(value);
		case "boolean":
			return typeof value === "boolean";
		case "enum":
			return typeof value === "string" && (input.choices ?? []).includes(value);
		case "json":
			return true;
	}
}

/** Mirrors `WidgetContract::validate` in packages/wasm/schema/src/widget.rs */
export function validateContract(contract: WidgetContract): string[] {
	const errors: string[] = [];

	const hasCsp = contract.csp !== undefined && contract.csp !== null;
	if (contract.contractVersion === CONTRACT_VERSION && !hasCsp) {
		errors.push(
			`Widget '${contract.id}' uses contractVersion ${CONTRACT_VERSION} without csp; contracts without csp must use contractVersion ${BASE_CONTRACT_VERSION}`,
		);
	} else if (contract.contractVersion === BASE_CONTRACT_VERSION && hasCsp) {
		errors.push(
			`Widget '${contract.id}' declares csp and must use contractVersion ${CONTRACT_VERSION}`,
		);
	} else if (
		contract.contractVersion !== CONTRACT_VERSION &&
		contract.contractVersion !== BASE_CONTRACT_VERSION
	) {
		errors.push(
			`Unsupported contractVersion ${contract.contractVersion} for widget '${contract.id}' (supported: ${BASE_CONTRACT_VERSION}, or ${CONTRACT_VERSION} with csp)`,
		);
	}

	if (hasCsp) {
		const shapeErrors = cspShapeErrors(contract.id, contract.csp);
		errors.push(
			...(shapeErrors.length > 0
				? shapeErrors
				: validateCspPurposes(
						contract.id,
						contract.csp as WidgetCspPurpose[],
						contract.inputs ?? {},
					)),
		);
	}

	if (!isValidWidgetId(contract.id)) {
		errors.push(
			`Invalid widget id '${contract.id}': must be non-empty lowercase kebab-case ([a-z0-9-])`,
		);
	}

	for (const [key, input] of Object.entries(contract.inputs ?? {})) {
		if (!isValidMemberKey(key)) {
			errors.push(
				`Invalid input key '${key}' in widget '${contract.id}': must match [a-zA-Z_][a-zA-Z0-9_]*`,
			);
		}
		if (input.type === "enum" && (input.choices?.length ?? 0) === 0) {
			errors.push(
				`Enum input '${key}' in widget '${contract.id}' must declare non-empty choices`,
			);
		}
		if (
			input.min !== undefined &&
			input.max !== undefined &&
			input.min > input.max
		) {
			errors.push(
				`Input '${key}' in widget '${contract.id}' has min ${input.min} > max ${input.max}`,
			);
		}
		if (
			input.default !== undefined &&
			!defaultMatchesType(input.default, input)
		) {
			errors.push(
				`Default value for input '${key}' in widget '${contract.id}' does not match its declared type`,
			);
		}
	}

	for (const key of Object.keys(contract.events ?? {})) {
		if (!isValidMemberKey(key)) {
			errors.push(
				`Invalid event key '${key}' in widget '${contract.id}': must match [a-zA-Z_][a-zA-Z0-9_]*`,
			);
		}
	}

	for (const key of Object.keys(contract.queries ?? {})) {
		if (!isValidMemberKey(key)) {
			errors.push(
				`Invalid query key '${key}' in widget '${contract.id}': must match [a-zA-Z_][a-zA-Z0-9_]*`,
			);
		}
	}

	return errors;
}

function sortedEntries<T>(map: Record<string, T>): [string, T][] {
	return Object.entries(map).sort(([a], [b]) => (a < b ? -1 : a > b ? 1 : 0));
}

/**
 * Checks the hub publish runs on top of `WidgetContract::validate` and that
 * need the public suffix list: wildcard bases (`wildcard-public-suffix`) and
 * reason rule 8 (`reason-contains-address`). The bundler runs them at build
 * and in `validate`.
 */
export function validatePublishedCsp(contract: WidgetContract): string[] {
	if (!Array.isArray(contract.csp)) return [];
	const errors: string[] = [];
	contract.csp.forEach((purpose, index) => {
		if (
			typeof purpose?.reason === "string" &&
			reasonContainsAddress(purpose.reason)
		) {
			errors.push(
				`Widget '${contract.id}': csp purpose ${index}: ${cspReasonProblem("reason-contains-address")}`,
			);
		}
	});
	const declared = flattenCspPurposes(contract.csp);
	const sources = new Set(Object.values(declared).flat());
	for (const error of validateWildcardBases(sources)) {
		errors.push(`Widget '${contract.id}': ${error}`);
	}
	return errors;
}

/**
 * Rebuild the contract with serde field order, BTreeMap-sorted keys, and the
 * same skip-serializing semantics as the Rust types, so the emitted JSON
 * matches what `serde_json` would produce. Every `csp` purpose is
 * canonicalized in place (group order kept), an empty `csp` is dropped, and
 * `contractVersion` follows from whether any purpose remains.
 */
export function canonicalizeContract(contract: WidgetContract): WidgetContract {
	const purposes = (contract.csp ?? []).map(canonicalizeCspPurpose);
	const csp = purposes.length > 0 ? purposes : undefined;
	const inputs: Record<string, ContractInput> = {};
	for (const [key, input] of sortedEntries(contract.inputs)) {
		inputs[key] = {
			type: input.type,
			...(input.description !== undefined && {
				description: input.description,
			}),
			...(input.default !== undefined && { default: input.default }),
			...(input.choices !== undefined && { choices: input.choices }),
			...(input.min !== undefined && { min: input.min }),
			...(input.max !== undefined && { max: input.max }),
			...(input.schema !== undefined && { schema: input.schema }),
			...(input.optional === true && { optional: true }),
		};
	}

	const events: Record<string, ContractEvent> = {};
	for (const [key, event] of sortedEntries(contract.events)) {
		events[key] = {
			payloadSchema: event.payloadSchema ?? null,
			...(event.description !== undefined && {
				description: event.description,
			}),
		};
	}

	const queries: Record<string, ContractQuery> = {};
	for (const [key, query] of sortedEntries(contract.queries)) {
		queries[key] = {
			argsSchema: query.argsSchema ?? null,
			resultSchema: query.resultSchema ?? null,
			...(query.description !== undefined && {
				description: query.description,
			}),
			...(query.mutation === true && { mutation: true }),
		};
	}

	return {
		contractVersion: csp ? CONTRACT_VERSION : BASE_CONTRACT_VERSION,
		id: contract.id,
		inputs,
		events,
		queries,
		sizing: {
			defaultHeight: contract.sizing.defaultHeight,
			resizable: contract.sizing.resizable,
			...(contract.sizing.maxHeight !== undefined && {
				maxHeight: contract.sizing.maxHeight,
			}),
		},
		...(contract.capabilities && {
			capabilities: Object.fromEntries(
				(["workers", "media", "microphone", "wasm", "downloads"] as const)
					.filter((key) => contract.capabilities?.[key] !== undefined)
					.map((key) => [key, contract.capabilities?.[key]]),
			),
		}),
		...(csp && { csp }),
	};
}

export function contractToJson(contract: WidgetContract): string {
	return JSON.stringify(canonicalizeContract(contract), null, 2);
}

function canonicalTemplate(
	template: WidgetUrlTemplate | undefined,
): WidgetUrlTemplate | undefined {
	if (!template) return undefined;
	const subdomains = canonicalCspSources(template.subdomains ?? []);
	const subdomainsInput = template.subdomainsInput ?? undefined;
	if (subdomains.length === 0 && subdomainsInput === undefined) {
		return undefined;
	}
	return {
		...(subdomains.length > 0 && { subdomains }),
		...(subdomainsInput !== undefined && { subdomainsInput }),
	};
}

function canonicalNetworkInput(input: WidgetNetworkInput): WidgetNetworkInput {
	const template = canonicalTemplate(input.template);
	return {
		path: input.path,
		directives: [...new Set(input.directives)].sort(compareCspDirectives),
		...(template && { template }),
	};
}

/**
 * Canonical form (`WidgetCspPurpose::canonicalize` plus serde field order):
 * source lists sorted and deduplicated, directives in `CSP_DIRECTIVES`
 * order, template subdomains sorted, empty templates removed and inputs
 * sorted by path.
 */
export function canonicalizeCspPurpose(
	purpose: WidgetCspPurpose,
): WidgetCspPurpose {
	const canonical: WidgetCspPurpose = { reason: purpose.reason };
	for (const directive of CSP_DIRECTIVES) {
		const sources = canonicalCspSources(purpose[directive] ?? []);
		if (sources.length > 0) canonical[directive] = sources;
	}
	const inputs = (purpose.inputs ?? [])
		.map(canonicalNetworkInput)
		.sort((a, b) => compareCspSources(a.path, b.path));
	if (inputs.length > 0) canonical.inputs = inputs;
	return canonical;
}

/**
 * Authoring normalization: reasons are NFC with collapsed whitespace, sources
 * are lowercased and punycoded, then every purpose is canonicalized. Group
 * order is kept.
 */
export function normalizeCspPurposes(
	purposes: readonly WidgetCspPurpose[],
): WidgetCspPurpose[] {
	return purposes.map((purpose) => {
		const normalized: WidgetCspPurpose = {
			...purpose,
			reason: normalizeCspReason(purpose.reason),
		};
		for (const directive of CSP_DIRECTIVES) {
			const sources = purpose[directive];
			if (sources) normalized[directive] = sources.map(normalizeCspSource);
		}
		return canonicalizeCspPurpose(normalized);
	});
}

function isRecord(value: unknown): value is Record<string, unknown> {
	return typeof value === "object" && value !== null && !Array.isArray(value);
}

function isStringArray(value: unknown): value is string[] {
	return (
		Array.isArray(value) && value.every((entry) => typeof entry === "string")
	);
}

function unknownKeys(
	value: Record<string, unknown>,
	allowed: readonly string[],
): string[] {
	return Object.keys(value).filter((key) => !allowed.includes(key));
}

function templateShapeErrors(template: unknown): string[] {
	if (template === undefined || template === null) return [];
	if (!isRecord(template)) return ["template must be an object"];
	const errors = unknownKeys(template, CSP_TEMPLATE_KEYS).map(
		(key) =>
			`template declares unknown key "${key}" (allowed: ${CSP_TEMPLATE_KEYS.join(", ")})`,
	);
	if (
		template.subdomains !== undefined &&
		!isStringArray(template.subdomains)
	) {
		errors.push("template subdomains must be an array of strings");
	}
	const input = template.subdomainsInput;
	if (input !== undefined && input !== null && typeof input !== "string") {
		errors.push("template subdomainsInput must be a string");
	}
	return errors;
}

function networkInputShapeErrors(input: unknown): string[] {
	if (!isRecord(input)) return ["must be an object"];
	const errors = unknownKeys(input, CSP_NETWORK_INPUT_KEYS).map(
		(key) =>
			`declares unknown key "${key}" (allowed: ${CSP_NETWORK_INPUT_KEYS.join(", ")})`,
	);
	if (typeof input.path !== "string") errors.push("path must be a string");
	if (!Array.isArray(input.directives)) {
		errors.push("directives must be an array");
	} else {
		for (const directive of input.directives) {
			if (typeof directive !== "string" || !isCspDirective(directive)) {
				errors.push(
					`declares unknown directive ${JSON.stringify(directive)} (allowed: ${CSP_DIRECTIVES.join(", ")})`,
				);
			}
		}
	}
	errors.push(...templateShapeErrors(input.template));
	return errors;
}

function purposeShapeErrors(purpose: unknown): string[] {
	if (!isRecord(purpose)) return ["must be an object"];
	const errors = unknownKeys(purpose, CSP_PURPOSE_KEYS).map(
		(key) =>
			`declares unknown key "${key}" (allowed: ${CSP_PURPOSE_KEYS.join(", ")})`,
	);
	if (typeof purpose.reason !== "string") {
		errors.push("must declare a string reason");
	}
	for (const directive of CSP_DIRECTIVES) {
		const sources = purpose[directive];
		if (sources !== undefined && !isStringArray(sources)) {
			errors.push(`${directive} must be an array of strings`);
		}
	}
	if (purpose.inputs !== undefined) {
		if (!Array.isArray(purpose.inputs)) {
			errors.push("inputs must be an array");
		} else {
			purpose.inputs.forEach((input, index) => {
				for (const error of networkInputShapeErrors(input)) {
					errors.push(`input ${index}: ${error}`);
				}
			});
		}
	}
	return errors;
}

/**
 * What serde refuses to parse (`deny_unknown_fields`, required fields, the
 * array shape). Semantic rules run only on a well-formed declaration.
 */
export function cspShapeErrors(widgetId: string, csp: unknown): string[] {
	if (!Array.isArray(csp)) {
		return [
			`Widget '${widgetId}': csp must be an array of purpose groups ({ reason, connectSrc?, imgSrc?, fontSrc?, mediaSrc?, styleSrc?, inputs? })`,
		];
	}
	return csp.flatMap((purpose, index) =>
		purposeShapeErrors(purpose).map(
			(error) => `Widget '${widgetId}': csp purpose ${index}: ${error}`,
		),
	);
}

/** The input fields validation reads; `ContractInput` satisfies it. */
export interface CspContractInput {
	readonly type: string;
}

function isReservedInputKey(key: string): boolean {
	return HOST_RESERVED_INPUT_KEYS.includes(key);
}

function templateProblems(
	template: WidgetUrlTemplate,
	inputs: Readonly<Record<string, CspContractInput>>,
): string[] {
	const problems: string[] = [];
	const subdomains = template.subdomains ?? [];
	const name = template.subdomainsInput ?? undefined;
	if (subdomains.length === 0 && name === undefined) {
		problems.push(
			"template declares no subdomains or subdomainsInput; omit it",
		);
	}
	if (subdomains.length > MAX_WIDGET_TEMPLATE_SUBDOMAINS) {
		problems.push(
			`template declares ${subdomains.length} subdomains; at most ${MAX_WIDGET_TEMPLATE_SUBDOMAINS} are allowed`,
		);
	}
	for (const subdomain of subdomains) {
		if (!isDnsLabel(subdomain)) {
			problems.push(
				`template subdomain "${subdomain}" is not a lowercase DNS label`,
			);
		}
	}
	if (!isStrictlyAscending(subdomains)) {
		problems.push(
			"template subdomains must be sorted ascending without duplicates",
		);
	}
	if (name !== undefined) {
		if (isReservedInputKey(name)) {
			problems.push(`subdomainsInput "${name}" is reserved by the host`);
		} else {
			const type = Object.hasOwn(inputs, name) ? inputs[name]?.type : undefined;
			if (type === undefined) {
				problems.push(`subdomainsInput "${name}" is not a contract input`);
			} else if (type !== "string" && type !== "json") {
				problems.push(
					`subdomainsInput "${name}" must name a string or json input`,
				);
			}
		}
	}
	return problems;
}

function networkInputProblems(
	input: WidgetNetworkInput,
	inputs: Readonly<Record<string, CspContractInput>>,
): string[] {
	const problems: string[] = [];
	const parsed = parseWidgetInputPath(input.path);
	if (parsed === null) {
		problems.push(
			'path must match root *( "." key / "[]" / ".*" ) with keys [A-Za-z_][A-Za-z0-9_]*',
		);
	} else {
		const { root, segments } = parsed;
		const count = segments.length + 1;
		if (count > MAX_WIDGET_INPUT_PATH_SEGMENTS) {
			problems.push(
				`path has ${count} segments; at most ${MAX_WIDGET_INPUT_PATH_SEGMENTS} are allowed`,
			);
		}
		if (isReservedInputKey(root)) {
			problems.push(`root "${root}" is reserved by the host`);
		} else {
			const type = Object.hasOwn(inputs, root) ? inputs[root]?.type : undefined;
			if (type === undefined) {
				problems.push(`root "${root}" is not a contract input`);
			} else if (type === "string" && segments.length > 0) {
				problems.push(
					`root "${root}" is a string input and must be the whole path`,
				);
			} else if (type !== "string" && type !== "json") {
				problems.push(
					`root "${root}" must be a string or json input; declare static sources for fixed choices`,
				);
			}
		}
	}
	if (input.directives.length === 0) problems.push("declares no directives");
	if (!isStrictlyAscending(input.directives, compareCspDirectives)) {
		problems.push(
			"directives must follow connectSrc, imgSrc, fontSrc, mediaSrc, styleSrc order without duplicates",
		);
	}
	if (input.template) {
		problems.push(...templateProblems(input.template, inputs));
	}
	return problems;
}

function purposeReasonRejection(
	purpose: WidgetCspPurpose,
): WidgetCspReasonRejection | null {
	return validateWidgetCspReason(
		purpose.reason,
		(purpose.inputs?.length ?? 0) > 0,
	);
}

function purposeProblems(
	purpose: WidgetCspPurpose,
	inputs: Readonly<Record<string, CspContractInput>>,
	reasonRejection: WidgetCspReasonRejection | null,
): string[] {
	const problems: string[] = [];
	const networkInputs = purpose.inputs ?? [];
	if (reasonRejection !== null) {
		problems.push(cspReasonProblem(reasonRejection));
	}
	if (cspSourceCount(purpose) === 0 && networkInputs.length === 0) {
		problems.push("declares no sources and no inputs");
	}
	for (const directive of CSP_DIRECTIVES) {
		const sources = purpose[directive] ?? [];
		for (const source of sources) {
			const sourceRejection = validateCspSource(directive, source);
			if (sourceRejection !== null) {
				problems.push(
					`Invalid csp source "${source}" in ${directive}: ${CSP_SOURCE_REJECTION_MESSAGES[sourceRejection]}`,
				);
			}
		}
		if (!isStrictlyAscending(sources)) {
			problems.push(`${directive} must be sorted ascending without duplicates`);
		}
	}
	if (!isStrictlyAscending(networkInputs.map((input) => input.path))) {
		problems.push("inputs must be sorted by path without duplicates");
	}
	for (const input of networkInputs) {
		for (const problem of networkInputProblems(input, inputs)) {
			problems.push(`input "${input.path}": ${problem}`);
		}
	}
	return problems;
}

function purposeSources(purpose: WidgetCspPurpose): string[] {
	return canonicalCspSources(
		CSP_DIRECTIVES.flatMap((directive) => purpose[directive] ?? []),
	);
}

/**
 * Mirrors `validate_csp_purposes`: every PSL-free rule of §14.2.3 and reason
 * rules 1–7 and 9–12, with the same messages and order. Purpose indexes are
 * zero-based. Expects a declaration that passed {@link cspShapeErrors}.
 */
export function validateCspPurposes(
	widgetId: string,
	purposes: readonly WidgetCspPurpose[],
	inputs: Readonly<Record<string, CspContractInput>>,
): string[] {
	if (purposes.length === 0) {
		return [
			`Widget '${widgetId}' declares an empty csp; omit csp when it declares no purposes`,
		];
	}
	const errors: string[] = [];
	if (purposes.length > MAX_WIDGET_CSP_PURPOSES) {
		errors.push(
			`Widget '${widgetId}': csp declares ${purposes.length} purposes; at most ${MAX_WIDGET_CSP_PURPOSES} are allowed`,
		);
	}
	const sourceOwners = new Map<string, number>();
	const inputOwners = new Map<string, number>();
	const reasonOwners = new Map<string, number>();
	purposes.forEach((purpose, index) => {
		const reasonRejection = purposeReasonRejection(purpose);
		for (const problem of purposeProblems(purpose, inputs, reasonRejection)) {
			errors.push(`Widget '${widgetId}': csp purpose ${index}: ${problem}`);
		}
		for (const source of purposeSources(purpose)) {
			const first = sourceOwners.get(source);
			if (first === undefined) sourceOwners.set(source, index);
			else
				errors.push(
					`Widget '${widgetId}': csp source "${source}" is declared in purposes ${first} and ${index}`,
				);
		}
		const paths = canonicalCspSources(
			(purpose.inputs ?? []).map((input) => input.path),
		);
		for (const path of paths) {
			const first = inputOwners.get(path);
			if (first === undefined) inputOwners.set(path, index);
			else
				errors.push(
					`Widget '${widgetId}': csp input "${path}" is declared in purposes ${first} and ${index}`,
				);
		}
		if (reasonRejection === null) {
			const folded = foldWidgetCspReason(purpose.reason);
			const first = reasonOwners.get(folded);
			if (first === undefined) reasonOwners.set(folded, index);
			else
				errors.push(
					`Widget '${widgetId}': csp purposes ${first} and ${index}: ${cspReasonProblem("reason-duplicate")}`,
				);
		}
	});
	const inputCount = purposes.reduce(
		(count, purpose) => count + (purpose.inputs?.length ?? 0),
		0,
	);
	if (inputCount > MAX_WIDGET_NETWORK_INPUTS) {
		errors.push(
			`Widget '${widgetId}': csp declares ${inputCount} network inputs; at most ${MAX_WIDGET_NETWORK_INPUTS} are allowed`,
		);
	}
	const declared = flattenCspPurposes(purposes);
	const count = cspSourceCount(declared);
	if (count > MAX_WIDGET_CSP_SOURCES) {
		errors.push(
			`Widget '${widgetId}': csp declares ${count} sources; at most ${MAX_WIDGET_CSP_SOURCES} are allowed`,
		);
	}
	const bytes = cspSourceBytes(declared);
	if (bytes > MAX_WIDGET_CSP_SOURCE_BYTES) {
		errors.push(
			`Widget '${widgetId}': csp sources take ${bytes} bytes; at most ${MAX_WIDGET_CSP_SOURCE_BYTES} are allowed`,
		);
	}
	return errors;
}

type AssertAssignable<_T extends U, U> = never;
export type ContractShapeMatchesSdk = AssertAssignable<
	WidgetContract,
	SdkWidgetContract
>;
