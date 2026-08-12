import type { ContractInput } from "@flow-like/widget-sdk";
import { createJsonSchemaValue } from "./widget-schema-form";

function cloneContractValue(value: unknown): unknown {
	if (value === undefined) return undefined;
	return JSON.parse(JSON.stringify(value));
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

/** Creates an editable runtime value when an omitted contract input is enabled. */
export function createContractInputValue(input: ContractInput): unknown {
	if (input.default !== undefined) return cloneContractValue(input.default);

	switch (input.type) {
		case "string":
			return "";
		case "number":
		case "integer":
			return numericFallback(input);
		case "boolean":
			return false;
		case "enum":
			return input.choices?.[0] ?? "";
		case "json":
			return input.schema ? createJsonSchemaValue(input.schema) : null;
	}
}

/** Applies a builder prop value; undefined means remove the key entirely. */
export function updateWidgetContractProps(
	current: Record<string, unknown>,
	key: string,
	value: unknown,
): Record<string, unknown> {
	const next = { ...current };
	if (value === undefined) delete next[key];
	else next[key] = value;
	return next;
}
