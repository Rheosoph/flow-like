import type { ContractInput, WidgetContract } from "@flow-like/widget-sdk";
import { validateInputValue } from "@flow-like/widget-sdk";

export type WidgetPropDraftValue = string | boolean | undefined;
export type WidgetPropsDraft = Record<string, WidgetPropDraftValue>;

export interface ParsedWidgetPropsDraft {
	props: Record<string, unknown>;
	errors: Record<string, string[]>;
	valid: boolean;
}

function numericFallback(input: ContractInput): number | undefined {
	if (input.type === "integer") {
		const lower =
			input.min === undefined ? Number.NEGATIVE_INFINITY : Math.ceil(input.min);
		const upper =
			input.max === undefined
				? Number.POSITIVE_INFINITY
				: Math.floor(input.max);
		if (lower > upper) return undefined;
		return Math.min(upper, Math.max(lower, 0));
	}

	let value = 0;
	if (input.min !== undefined && value < input.min) value = input.min;
	if (input.max !== undefined && value > input.max) value = input.max;
	return value;
}

/** An editable starter value when a currently omitted field is enabled. */
export function emptyWidgetPropDraft(
	input: ContractInput,
): WidgetPropDraftValue {
	const declaredDefault = serializeDraftValue(input, input.default);
	if (declaredDefault !== undefined) return declaredDefault;

	switch (input.type) {
		case "string":
			return "";
		case "number":
		case "integer": {
			const fallback = numericFallback(input);
			return fallback === undefined ? "" : String(fallback);
		}
		case "boolean":
			return false;
		case "enum":
			return input.choices?.[0] ?? "";
		case "json": {
			const schema = input.schema;
			if (schema) {
				const schemaCandidates = [
					schema.default,
					schema.const,
					Array.isArray(schema.enum) ? schema.enum[0] : undefined,
				];
				for (const candidate of schemaCandidates) {
					if (candidate === undefined) continue;
					const serialized = JSON.stringify(candidate, null, 2);
					if (serialized !== undefined) return serialized;
				}
			}

			const schemaType = schema?.type;
			if (schemaType === "array") return "[]";
			if (schemaType === "object") return "{}";
			if (schemaType === "string") return '""';
			if (schemaType === "number" || schemaType === "integer") return "0";
			if (schemaType === "boolean") return "false";
			return "null";
		}
	}
}

function serializeDraftValue(
	input: ContractInput,
	value: unknown,
): WidgetPropDraftValue {
	if (value === undefined) return undefined;

	switch (input.type) {
		case "boolean":
			return typeof value === "boolean" ? value : undefined;
		case "number":
		case "integer":
			return typeof value === "number" ? String(value) : undefined;
		case "json":
			try {
				return JSON.stringify(value, null, 2);
			} catch {
				return undefined;
			}
		case "string":
		case "enum":
			return typeof value === "string" ? value : undefined;
	}
}

export function createWidgetPropsDraft(
	contract: WidgetContract,
	values: Record<string, unknown> = {},
): WidgetPropsDraft {
	const draft: WidgetPropsDraft = {};

	for (const [key, input] of Object.entries(contract.inputs ?? {})) {
		const value = values[key] !== undefined ? values[key] : input.default;
		draft[key] = serializeDraftValue(input, value);
	}

	return draft;
}

function parseDraftValue(
	input: ContractInput,
	value: WidgetPropDraftValue,
): { value?: unknown; errors: string[] } {
	if (value === undefined) return { value: undefined, errors: [] };

	switch (input.type) {
		case "string":
		case "enum":
			return typeof value === "string"
				? { value, errors: [] }
				: { errors: [`$: expected ${input.type}`] };
		case "boolean":
			return typeof value === "boolean"
				? { value, errors: [] }
				: { errors: ["$: expected boolean"] };
		case "number":
		case "integer": {
			if (typeof value !== "string" || value.trim() === "") {
				return { errors: ["$: value is required"] };
			}
			const parsed = Number(value);
			return Number.isFinite(parsed)
				? { value: parsed, errors: [] }
				: { errors: [`$: expected ${input.type}`] };
		}
		case "json": {
			if (typeof value !== "string" || value.trim() === "") {
				return { errors: ["$: value is required"] };
			}
			try {
				return { value: JSON.parse(value), errors: [] };
			} catch (error) {
				return {
					errors: [
						`Invalid JSON: ${error instanceof Error ? error.message : String(error)}`,
					],
				};
			}
		}
	}
}

export function parseWidgetPropsDraft(
	contract: WidgetContract,
	draft: WidgetPropsDraft,
): ParsedWidgetPropsDraft {
	const props: Record<string, unknown> = {};
	const errors: Record<string, string[]> = {};

	for (const [key, input] of Object.entries(contract.inputs ?? {})) {
		const parsed = parseDraftValue(input, draft[key]);
		if (parsed.errors.length > 0) {
			errors[key] = parsed.errors;
			continue;
		}

		const validation = validateInputValue(input, parsed.value);
		if (!validation.valid) {
			errors[key] = validation.errors;
			continue;
		}

		if (parsed.value !== undefined) props[key] = parsed.value;
	}

	return { props, errors, valid: Object.keys(errors).length === 0 };
}
