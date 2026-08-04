import type { JsonSchema } from "@flow-like/widget-sdk";

export function isSchemaRecord(
	value: unknown,
): value is Record<string, unknown> {
	return typeof value === "object" && value !== null && !Array.isArray(value);
}

function cloneJsonValue(value: unknown): unknown {
	if (value === undefined) return undefined;
	return JSON.parse(JSON.stringify(value));
}

function rawJsonSchemaType(schema: JsonSchema): string | undefined {
	if (typeof schema.type === "string") return schema.type;
	if (Array.isArray(schema.type)) {
		return schema.type.find(
			(candidate): candidate is string =>
				typeof candidate === "string" && candidate !== "null",
		);
	}
	if (isSchemaRecord(schema.properties)) return "object";
	if (isSchemaRecord(schema.items)) return "array";
	return undefined;
}

function firstSchemaBranch(schema: JsonSchema): JsonSchema | null {
	const variants = Array.isArray(schema.oneOf)
		? schema.oneOf
		: Array.isArray(schema.anyOf)
			? schema.anyOf
			: null;
	if (!variants) return null;

	const nonNull: JsonSchema[] = [];
	for (const variant of variants) {
		if (!isSchemaRecord(variant)) continue;
		if (rawJsonSchemaType(variant) !== "null") nonNull.push(variant);
	}
	return nonNull.length === 1 ? nonNull[0] : null;
}

export function resolveJsonSchema(schema: JsonSchema): JsonSchema {
	const branch = firstSchemaBranch(schema);
	if (!branch) return schema;
	return { ...branch, ...schema, oneOf: undefined, anyOf: undefined };
}

export function jsonSchemaType(schema: JsonSchema): string | undefined {
	return rawJsonSchemaType(resolveJsonSchema(schema));
}

export function homogeneousArrayItemSchema(
	schema: JsonSchema | null | undefined,
): JsonSchema | null {
	if (!schema) return null;
	const resolved = resolveJsonSchema(schema);
	if (jsonSchemaType(resolved) !== "array") return null;
	return isSchemaRecord(resolved.items) ? resolved.items : null;
}

function schemaSeed(schema: JsonSchema): unknown {
	for (const key of ["default", "const"] as const) {
		if (Object.hasOwn(schema, key)) return cloneJsonValue(schema[key]);
	}
	if (Array.isArray(schema.enum) && schema.enum.length > 0) {
		return cloneJsonValue(schema.enum[0]);
	}
	return undefined;
}

function numericSchemaDefault(
	schema: JsonSchema,
	integer: boolean,
): number | undefined {
	const minimum =
		typeof schema.minimum === "number" ? schema.minimum : undefined;
	const maximum =
		typeof schema.maximum === "number" ? schema.maximum : undefined;
	if (integer) {
		const lower =
			minimum === undefined ? Number.NEGATIVE_INFINITY : Math.ceil(minimum);
		const upper =
			maximum === undefined ? Number.POSITIVE_INFINITY : Math.floor(maximum);
		if (lower > upper) return undefined;
		return Math.min(upper, Math.max(lower, 0));
	}
	let value = 0;
	if (minimum !== undefined && value < minimum) value = minimum;
	if (maximum !== undefined && value > maximum) value = maximum;
	return value;
}

export function createJsonSchemaValue(schema: JsonSchema): unknown {
	const resolved = resolveJsonSchema(schema);
	const seeded = schemaSeed(resolved);
	if (seeded !== undefined) return seeded;

	switch (jsonSchemaType(resolved)) {
		case "string":
			return "";
		case "number":
			return numericSchemaDefault(resolved, false);
		case "integer":
			return numericSchemaDefault(resolved, true);
		case "boolean":
			return false;
		case "array":
			return [];
		case "object": {
			const properties = isSchemaRecord(resolved.properties)
				? resolved.properties
				: {};
			const required = new Set(
				Array.isArray(resolved.required)
					? resolved.required.filter(
							(candidate): candidate is string => typeof candidate === "string",
						)
					: [],
			);
			const value: Record<string, unknown> = {};
			for (const [key, property] of Object.entries(properties)) {
				if (!isSchemaRecord(property)) continue;
				const hasSeed =
					Object.hasOwn(property, "default") ||
					Object.hasOwn(property, "const") ||
					(Array.isArray(property.enum) && property.enum.length > 0);
				if (required.has(key) || hasSeed) {
					value[key] = createJsonSchemaValue(property);
				}
			}
			return value;
		}
		case "null":
			return null;
		default:
			return null;
	}
}

export interface ParsedWidgetListDraft {
	items: unknown[] | null;
	error: string | null;
}

export function parseWidgetListDraft(value: unknown): ParsedWidgetListDraft {
	if (value === undefined) return { items: [], error: null };
	if (typeof value !== "string" || value.trim() === "") {
		return { items: null, error: "List value is empty" };
	}
	try {
		const parsed: unknown = JSON.parse(value);
		return Array.isArray(parsed)
			? { items: parsed, error: null }
			: { items: null, error: "Expected a JSON array" };
	} catch (error) {
		return {
			items: null,
			error: `Invalid JSON: ${error instanceof Error ? error.message : String(error)}`,
		};
	}
}

export function serializeWidgetList(items: unknown[]): string {
	return JSON.stringify(items, null, 2);
}

function compactValue(value: unknown): string | null {
	if (typeof value === "string") return value || "Empty string";
	if (typeof value === "number" || typeof value === "boolean") {
		return String(value);
	}
	if (value === null) return "null";
	return null;
}

export interface WidgetListItemSummary {
	title: string;
	detail?: string;
}

export function summarizeWidgetListItem(
	item: unknown,
	index: number,
): WidgetListItemSummary {
	const direct = compactValue(item);
	if (direct !== null) return { title: direct };
	if (Array.isArray(item)) {
		return {
			title: `${item.length} ${item.length === 1 ? "item" : "items"}`,
		};
	}
	if (!isSchemaRecord(item)) return { title: `Item ${index + 1}` };

	const entries = Object.entries(item)
		.map(([key, value]) => [key, compactValue(value)] as const)
		.filter((entry): entry is readonly [string, string] => entry[1] !== null);
	const preferred = ["label", "name", "title", "id"]
		.map((key) => entries.find(([entryKey]) => entryKey === key))
		.find(Boolean);
	const primary = preferred ?? entries[0];
	if (!primary) return { title: `Item ${index + 1}` };

	const detail = entries
		.filter(([key]) => key !== primary[0])
		.slice(0, 3)
		.map(([key, value]) => `${key}: ${value}`)
		.join(" · ");
	return {
		title: primary[1],
		...(detail && { detail }),
	};
}
