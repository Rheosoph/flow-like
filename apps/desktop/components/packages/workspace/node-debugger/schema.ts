import type { IBit } from "@flow-like/flow-like-ui/lib/schema/bit/bit";
import { IBitTypes } from "@flow-like/flow-like-ui/lib/schema/bit/bit";
import type {
	WasmNodeDefinition,
	WasmPinDefinition,
} from "@flow-like/flow-like-ui/lib/schema/developer";

export interface JsonSchema {
	type?: string;
	properties?: Record<string, JsonSchema>;
	required?: string[];
	items?: JsonSchema;
	title?: string;
	description?: string;
	default?: unknown;
	enum?: unknown[];
	format?: string;
	minimum?: number;
	maximum?: number;
	oneOf?: JsonSchema[];
	anyOf?: JsonSchema[];
	allOf?: JsonSchema[];
	$ref?: string;
	definitions?: Record<string, JsonSchema>;
}

export const MODEL_BIT_TYPES = new Set<IBitTypes>([
	IBitTypes.Llm,
	IBitTypes.Vlm,
	IBitTypes.Tts,
	IBitTypes.Stt,
]);

export function parseSchema(raw: string | undefined): JsonSchema | null {
	if (!raw) return null;
	try {
		const parsed = JSON.parse(raw) as JsonSchema;
		if (parsed && typeof parsed === "object") return parsed;
	} catch {
		/* malformed schema */
	}
	return null;
}

export function getBitKey(bit: Pick<IBit, "id" | "hub">): string {
	return bit.hub ? `${bit.hub}:${bit.id}` : bit.id;
}

export function getBitDisplayName(bit: IBit): string {
	return Object.values(bit.meta ?? {})[0]?.name ?? bit.id;
}

export function getBitProviderName(bit: IBit): string | null {
	if (typeof bit.parameters !== "object" || bit.parameters == null) return null;
	const provider = (
		bit.parameters as {
			provider?: { provider_name?: string | null };
		}
	).provider?.provider_name;
	return typeof provider === "string" && provider.length > 0 ? provider : null;
}

export function isSelectedBitValue(
	value: unknown,
): value is Pick<IBit, "id" | "hub"> {
	return (
		typeof value === "object" &&
		value !== null &&
		typeof (value as { id?: unknown }).id === "string"
	);
}

function resolveRef(
	ref: string,
	rootSchema: JsonSchema,
): JsonSchema | undefined {
	const parts = ref.replace(/^#\//, "").split("/");
	let current: unknown = rootSchema;
	for (const part of parts) {
		if (current && typeof current === "object" && part in current) {
			current = (current as Record<string, unknown>)[part];
		} else {
			return undefined;
		}
	}
	return current as JsonSchema | undefined;
}

export function resolveSchema(
	schema: JsonSchema,
	root: JsonSchema,
	seen = new Set<string>(),
): JsonSchema {
	if (schema.$ref) {
		if (seen.has(schema.$ref)) return schema;
		const nextSeen = new Set(seen).add(schema.$ref);
		const resolved = resolveRef(schema.$ref, root);
		if (resolved) return resolveSchema(resolved, root, nextSeen);
	}
	if (schema.allOf && schema.allOf.length > 0) {
		if (schema.allOf.length === 1) {
			return resolveSchema(schema.allOf[0], root, seen);
		}
		let merged: JsonSchema = {};
		for (const sub of schema.allOf) {
			const resolved = resolveSchema(sub, root, seen);
			merged = {
				...merged,
				...resolved,
				...(merged.properties || resolved.properties
					? { properties: { ...merged.properties, ...resolved.properties } }
					: {}),
			};
		}
		return merged;
	}
	if (schema.anyOf?.length === 1) {
		return resolveSchema(schema.anyOf[0], root, seen);
	}
	return schema;
}

export function createDefaultFromSchema(
	schema: JsonSchema,
	root: JsonSchema,
	depth = 0,
): unknown {
	if (depth > 32) return null;
	const resolved = resolveSchema(schema, root);
	if (resolved.default !== undefined) return resolved.default;

	if (resolved.enum && resolved.enum.length > 0) return resolved.enum[0];

	switch (resolved.type) {
		case "string":
			return "";
		case "integer":
		case "number":
			return 0;
		case "boolean":
			return false;
		case "array":
			return [];
		case "object": {
			if (!resolved.properties) return {};
			const obj: Record<string, unknown> = {};
			for (const [key, propSchema] of Object.entries(resolved.properties)) {
				obj[key] = createDefaultFromSchema(propSchema, root, depth + 1);
			}
			return obj;
		}
		default:
			return null;
	}
}

function isBitSchema(schema: JsonSchema | null): boolean {
	if (!schema) return false;

	const resolved = resolveSchema(schema, schema);
	if (resolved.type !== "object") return false;

	const properties = resolved.properties ?? {};
	const typeSchema = properties.type
		? resolveSchema(properties.type, schema)
		: undefined;
	const typeValues = (typeSchema?.enum ?? []).filter(
		(value): value is string => typeof value === "string",
	);

	return (
		"id" in properties &&
		"type" in properties &&
		(typeValues.some((value) => MODEL_BIT_TYPES.has(value as IBitTypes)) ||
			"parameters" in properties ||
			"meta" in properties ||
			"hub" in properties ||
			"hash" in properties ||
			"model_slug" in properties ||
			"model_evaluation" in properties)
	);
}

export function isModelBitPin(pin: WasmPinDefinition): boolean {
	return pin.data_type === "Struct" && isBitSchema(parseSchema(pin.schema));
}

export function getDefaultValue(pin: WasmPinDefinition): unknown {
	if (pin.default_value !== undefined && pin.default_value !== null) {
		return pin.default_value;
	}
	if (pin.value_type === "Array") return [];
	switch (pin.data_type) {
		case "Boolean":
			return false;
		case "Integer":
			return 0;
		case "Float":
			return 0.0;
		case "Struct":
			return {};
		default:
			return "";
	}
}

function initInputDefaults(node: WasmNodeDefinition): Record<string, unknown> {
	const defaults: Record<string, unknown> = {};
	for (const pin of node.pins) {
		if (pin.pin_type === "Input" && pin.data_type !== "Execution") {
			defaults[pin.name] = getDefaultValue(pin);
		}
	}
	return defaults;
}

export function applySpecialInputDefaults(
	node: WasmNodeDefinition,
	values: Record<string, unknown>,
	availableModelBits: IBit[],
): Record<string, unknown> {
	if (availableModelBits.length === 0) return values;

	let nextValues = values;
	for (const pin of node.pins) {
		if (
			pin.pin_type !== "Input" ||
			pin.data_type === "Execution" ||
			!isModelBitPin(pin) ||
			isSelectedBitValue(values[pin.name])
		) {
			continue;
		}

		if (nextValues === values) {
			nextValues = { ...values };
		}
		nextValues[pin.name] = availableModelBits[0];
	}

	return nextValues;
}

export function buildInitialInputValues(
	node: WasmNodeDefinition,
	availableModelBits: IBit[],
): Record<string, unknown> {
	return applySpecialInputDefaults(
		node,
		initInputDefaults(node),
		availableModelBits,
	);
}

export function isDataPin(pin: WasmPinDefinition, type: "Input" | "Output") {
	return pin.pin_type === type && pin.data_type !== "Execution";
}
