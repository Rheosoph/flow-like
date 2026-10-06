import type { IPinOptions } from "../../lib/schema/flow/pin";
import { IVariableType } from "../../lib/schema/flow/variable";

export interface PinOptionsTarget {
	data_type: IVariableType;
	options?: IPinOptions | null;
	schema?: string | null;
}

export interface PinOptionSections {
	validValues: boolean;
	range: boolean;
	step: boolean;
	schema: boolean;
	valueShape: boolean;
	sensitive: boolean;
}

export interface PinOptionsDraft {
	validValues: string[];
	pendingValue: string;
	min: string;
	max: string;
	step: string;
	schema: string;
	sensitive: boolean;
	enforceSchema: boolean;
	enforceValueShape: boolean;
}

export type PinOptionsIssue =
	| "rangeIncomplete"
	| "rangeInverted"
	| "stepNotPositive"
	| "schemaInvalidJson";

export type SchemaStatus =
	| { kind: "empty" }
	| { kind: "json"; title: string | null }
	| { kind: "identifier" }
	| { kind: "invalid" };

export interface PinOptionsResult {
	options: IPinOptions | null;
	schema?: string | null;
}

const RANGE_TYPES = new Set<IVariableType>([
	IVariableType.Integer,
	IVariableType.Float,
	IVariableType.Byte,
]);

const SCHEMA_TYPES = new Set<IVariableType>([
	IVariableType.Struct,
	IVariableType.Generic,
	IVariableType.Geometry,
]);

export function pinOptionSections(pin: PinOptionsTarget): PinOptionSections {
	const options = pin.options ?? {};
	return {
		validValues:
			pin.data_type === IVariableType.String ||
			(options.valid_values?.length ?? 0) > 0,
		range: RANGE_TYPES.has(pin.data_type) || options.range != null,
		step: pin.data_type === IVariableType.Float || options.step != null,
		schema:
			SCHEMA_TYPES.has(pin.data_type) ||
			Boolean(pin.schema) ||
			Boolean(options.enforce_schema),
		valueShape:
			pin.data_type === IVariableType.Generic ||
			Boolean(options.enforce_generic_value_type),
		sensitive:
			pin.data_type !== IVariableType.Execution || Boolean(options.sensitive),
	};
}

export const hasPinOptionSections = (pin: PinOptionsTarget) =>
	Object.values(pinOptionSections(pin)).some(Boolean);

export function hasActivePinOptions(pin: PinOptionsTarget): boolean {
	const options = pin.options;
	return Boolean(
		pin.schema ||
			options?.sensitive ||
			options?.enforce_schema ||
			options?.enforce_generic_value_type ||
			options?.valid_values?.length ||
			options?.range ||
			options?.step != null,
	);
}

export function splitValueDraft(draft: string): {
	complete: string[];
	rest: string;
} {
	const parts = draft.split(/[,\n\r]/);
	const rest = parts.pop() ?? "";
	return { complete: parts, rest: rest.trimStart() };
}

export function appendValues(
	values: readonly string[],
	additions: readonly string[],
): string[] {
	const next = [...values];
	for (const raw of additions) {
		const value = raw.trim();
		if (value !== "" && !next.includes(value)) next.push(value);
	}
	return next;
}

const looksLikeJson = (text: string) => /^[[{]/.test(text);

function parseJson(text: string): { value: unknown } | null {
	try {
		return { value: JSON.parse(text) };
	} catch {
		return null;
	}
}

function schemaTitle(value: unknown) {
	const title =
		value && typeof value === "object"
			? (value as { title?: unknown }).title
			: null;
	return typeof title === "string" ? title : null;
}

export function schemaStatus(text: string): SchemaStatus {
	const trimmed = text.trim();
	if (trimmed === "") return { kind: "empty" };
	if (!looksLikeJson(trimmed)) return { kind: "identifier" };
	const parsed = parseJson(trimmed);
	return parsed
		? { kind: "json", title: schemaTitle(parsed.value) }
		: { kind: "invalid" };
}

function normalizeSchema(text: string) {
	const trimmed = text.trim();
	if (trimmed === "") return null;
	const parsed = looksLikeJson(trimmed) ? parseJson(trimmed) : null;
	return parsed ? JSON.stringify(parsed.value) : trimmed;
}

export function formatSchema(text: string) {
	const trimmed = text.trim();
	const parsed = looksLikeJson(trimmed) ? parseJson(trimmed) : null;
	return parsed ? JSON.stringify(parsed.value, null, 2) : text;
}

export function initialSchemaText(
	pin: PinOptionsTarget,
	refs?: Record<string, string>,
): string {
	const stored = pin.schema ?? "";
	if (pin.data_type === IVariableType.Geometry) return stored;
	return formatSchema(refs?.[stored] ?? stored);
}

const numberDraft = (value?: number | null) =>
	typeof value === "number" && Number.isFinite(value) ? String(value) : "";

export function draftFromPin(
	pin: PinOptionsTarget,
	refs?: Record<string, string>,
): PinOptionsDraft {
	const options = pin.options ?? {};
	return {
		validValues: [...(options.valid_values ?? [])],
		pendingValue: "",
		min: numberDraft(options.range?.[0]),
		max: numberDraft(options.range?.[1]),
		step: numberDraft(options.step),
		schema: initialSchemaText(pin, refs),
		sensitive: Boolean(options.sensitive),
		enforceSchema: Boolean(options.enforce_schema),
		enforceValueShape: Boolean(options.enforce_generic_value_type),
	};
}

const parseNumber = (text: string): number | null => {
	const value = text.trim() === "" ? Number.NaN : Number(text);
	return Number.isFinite(value) ? value : null;
};

export function draftRange(
	draft: Pick<PinOptionsDraft, "min" | "max">,
): [number, number] | null | PinOptionsIssue {
	const min = parseNumber(draft.min);
	const max = parseNumber(draft.max);
	if (min === null && max === null) return null;
	if (min === null || max === null) return "rangeIncomplete";
	if (min > max) return "rangeInverted";
	return [min, max];
}

const stepIssue = (text: string): PinOptionsIssue | null => {
	const step = parseNumber(text);
	return step !== null && step <= 0 ? "stepNotPositive" : null;
};

const schemaIssue = (text: string): PinOptionsIssue | null =>
	schemaStatus(text).kind === "invalid" ? "schemaInvalidJson" : null;

export function validatePinOptionsDraft(
	draft: PinOptionsDraft,
	sections: PinOptionSections,
): PinOptionsIssue[] {
	const range = sections.range ? draftRange(draft) : null;
	const issues = [
		typeof range === "string" ? range : null,
		sections.step ? stepIssue(draft.step) : null,
		sections.schema ? schemaIssue(draft.schema) : null,
	];
	return issues.filter((issue): issue is PinOptionsIssue => issue !== null);
}

function assign<K extends keyof IPinOptions>(
	options: IPinOptions,
	key: K,
	value: IPinOptions[K],
) {
	if (value == null && options[key] == null) return;
	options[key] = value;
}

function assignFlag(
	options: IPinOptions,
	key: "sensitive" | "enforce_schema" | "enforce_generic_value_type",
	checked: boolean,
) {
	if (checked !== Boolean(options[key])) options[key] = checked;
}

export function buildPinOptionsResult(
	pin: PinOptionsTarget,
	draft: PinOptionsDraft,
	refs?: Record<string, string>,
): PinOptionsResult {
	const sections = pinOptionSections(pin);
	const options: IPinOptions = { ...(pin.options ?? {}) };

	if (sections.validValues) {
		const values = appendValues(draft.validValues, [draft.pendingValue]);
		assign(options, "valid_values", values.length > 0 ? values : null);
	}
	if (sections.range) {
		const range = draftRange(draft);
		if (typeof range !== "string") assign(options, "range", range);
	}
	if (sections.step) {
		assign(options, "step", parseNumber(draft.step));
	}
	if (sections.sensitive) assignFlag(options, "sensitive", draft.sensitive);
	if (sections.schema)
		assignFlag(options, "enforce_schema", draft.enforceSchema);
	if (sections.valueShape)
		assignFlag(options, "enforce_generic_value_type", draft.enforceValueShape);

	const result: PinOptionsResult = {
		options:
			pin.options == null && Object.keys(options).length === 0 ? null : options,
	};
	if (sections.schema) {
		const next = normalizeSchema(draft.schema);
		if (next !== normalizeSchema(initialSchemaText(pin, refs)))
			result.schema = next;
	}
	return result;
}
