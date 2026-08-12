import type * as vscode from "vscode";
import type { FlowInterface, FlowVariable } from "./flowDocument";
import type { NodeSignature, SignatureRegistry } from "./signatures";

/** A resolved FlowScript type. Object shapes carry named members so member
 * access (`base.field`) and completion can walk arbitrarily deep through node
 * return structs and inferred struct-literal shapes. */
export type Shape =
	| {
			readonly kind: "object";
			readonly text: string;
			readonly fields: Map<string, Shape>;
			readonly docs?: Map<string, string>;
			readonly origin?: string;
	  }
	| { readonly kind: "array"; readonly text: string; readonly element?: Shape }
	| { readonly kind: "scalar"; readonly text: string };

export interface Accessor {
	readonly kind: "field" | "index";
	readonly name?: string;
}

/** Build a `Shape` from a TS-flavoured type string (`Struct`, `string[]`, `Map<string, T>`, …). */
export function shapeFromTypeText(
	raw: string,
	interfaces?: ReadonlyMap<string, FlowInterface>,
	seen: Set<string> = new Set(),
): Shape {
	const text = raw.trim();
	const union = splitTopLevel(text, "|");
	if (union.length > 1) {
		return {
			kind: "scalar",
			text: union
				.map((part) => shapeFromTypeText(part, interfaces, seen).text)
				.join(" | "),
		};
	}
	if (text.endsWith("[]")) {
		return {
			kind: "array",
			text,
			element: shapeFromTypeText(text.slice(0, -2), interfaces, seen),
		};
	}
	const map = /^Map\s*<\s*string\s*,\s*(.+)\s*>$/.exec(text);
	if (map) {
		return {
			kind: "object",
			text,
			fields: new Map(),
			origin: `Map<string, ${shapeFromTypeText(map[1], interfaces, seen).text}>`,
		};
	}
	const set = /^Set\s*<\s*(.+)\s*>$/.exec(text);
	if (set) {
		return {
			kind: "array",
			text,
			element: shapeFromTypeText(set[1], interfaces, seen),
		};
	}
	const iface = interfaces?.get(text);
	if (interfaces && iface) {
		return shapeFromInterface(iface, interfaces, seen);
	}
	return { kind: "scalar", text };
}

function shapeFromInterface(
	iface: FlowInterface,
	interfaces: ReadonlyMap<string, FlowInterface>,
	seen: Set<string>,
): Shape {
	if (seen.has(iface.name)) {
		return { kind: "scalar", text: iface.name };
	}
	const next = new Set(seen);
	next.add(iface.name);
	const fields = new Map<string, Shape>();
	for (const field of iface.fields) {
		fields.set(field.name, shapeFromTypeText(field.typeText, interfaces, next));
	}
	return {
		kind: "object",
		text: iface.name,
		fields,
		origin: `interface ${iface.name}`,
	};
}

function splitTopLevel(text: string, separator: string): string[] {
	const parts: string[] = [];
	let depth = 0;
	let inString = false;
	let start = 0;
	for (let i = 0; i < text.length; i++) {
		const ch = text[i];
		if (inString) {
			if (ch === "\\" && i + 1 < text.length) {
				i++;
			} else if (ch === '"') {
				inString = false;
			}
			continue;
		}
		if (ch === '"') {
			inString = true;
		} else if (ch === "<" || ch === "(" || ch === "[") {
			depth++;
		} else if (ch === ">" || ch === ")" || ch === "]") {
			depth--;
		} else if (ch === separator && depth === 0) {
			parts.push(text.slice(start, i).trim());
			start = i + 1;
		}
	}
	parts.push(text.slice(start).trim());
	return parts.filter(Boolean);
}

interface JsonSchema {
	type?: string | string[];
	title?: string;
	properties?: Record<string, JsonSchema>;
	items?: JsonSchema;
	additionalProperties?: JsonSchema | boolean;
	required?: string[];
	description?: string;
	$ref?: string;
	$defs?: Record<string, JsonSchema>;
	anyOf?: JsonSchema[];
	allOf?: JsonSchema[];
	oneOf?: JsonSchema[];
	enum?: unknown[];
	format?: string;
}

/** Parse a JSON Schema string and build a `Shape`, resolving `$ref`/`$defs` and unwrapping
 * nullable `anyOf`/`oneOf` unions. Returns `undefined` when the text is not valid JSON. */
export function shapeFromJsonSchema(raw: string): Shape | undefined {
	let root: JsonSchema;
	try {
		root = JSON.parse(raw) as JsonSchema;
	} catch {
		return undefined;
	}
	const defs = root.$defs ?? {};
	return schemaToShape(root, defs, new Set());
}

/** Resolve a `#/$defs/Name` ref against the root `$defs` table. */
function resolveRef(
	ref: string,
	defs: Record<string, JsonSchema>,
): JsonSchema | undefined {
	const match = /^#\/\$defs\/(.+)$/.exec(ref);
	return match ? defs[match[1]] : undefined;
}

/** Collapse a nullable union (`anyOf`/`oneOf` of `[T, null]`) to its non-null branch. */
function unwrapUnion(branches: JsonSchema[]): JsonSchema | undefined {
	return branches.find((b) => b.type !== "null");
}

function schemaToShape(
	schema: JsonSchema,
	defs: Record<string, JsonSchema>,
	seen: Set<string>,
): Shape {
	if (schema.$ref) {
		if (seen.has(schema.$ref)) {
			const name = schema.$ref.split("/").pop() ?? "Struct";
			return { kind: "scalar", text: name };
		}
		const target = resolveRef(schema.$ref, defs);
		if (target) {
			const next = new Set(seen);
			next.add(schema.$ref);
			const shape = schemaToShape(target, defs, next);
			const name = schema.$ref.split("/").pop();
			return name && shape.kind === "object" ? { ...shape, text: name } : shape;
		}
		return { kind: "scalar", text: schema.$ref.split("/").pop() ?? "Struct" };
	}

	const union = schema.anyOf ?? schema.oneOf;
	if (union) {
		const branch = unwrapUnion(union);
		return branch
			? schemaToShape(branch, defs, seen)
			: { kind: "scalar", text: "Struct" };
	}

	const type = Array.isArray(schema.type)
		? schema.type.find((t) => t !== "null")
		: schema.type;

	if (type === "array") {
		const element = schema.items
			? schemaToShape(schema.items, defs, seen)
			: undefined;
		return {
			kind: "array",
			text: `${element?.text ?? "Struct"}[]`,
			element,
		};
	}

	if (type === "object" || schema.properties) {
		const fields = new Map<string, Shape>();
		const docs = new Map<string, string>();
		for (const [key, prop] of Object.entries(schema.properties ?? {})) {
			fields.set(key, schemaToShape(prop, defs, seen));
			if (prop.description) {
				docs.set(key, prop.description);
			}
		}
		return {
			kind: "object",
			text: schema.title ?? "Struct",
			fields,
			docs: docs.size > 0 ? docs : undefined,
		};
	}

	if (schema.enum && schema.enum.length > 0) {
		return {
			kind: "scalar",
			text: schema.enum.map((v) => JSON.stringify(v)).join(" | "),
		};
	}

	return { kind: "scalar", text: scalarTypeText(type, schema.format) };
}

function scalarTypeText(type: string | undefined, format?: string): string {
	switch (type) {
		case "string":
			return "string";
		case "boolean":
			return "bool";
		case "integer":
			return "int";
		case "number":
			return format === "float" ? "float" : "number";
		case "null":
			return "null";
		default:
			return "Struct";
	}
}

/** Object shape for a node's outputs (`{ session: DfSession }`). */
function shapeFromSignature(
	sig: NodeSignature,
	registry: SignatureRegistry,
): Shape {
	if (sig.returns.length === 0) {
		return { kind: "scalar", text: "void" };
	}
	const outputSchemas = registry.schemasFor(sig.name)?.outputs;
	const deepShape = (name: string | undefined, fallback: string): Shape => {
		const schema = name ? outputSchemas?.[name] : undefined;
		const resolved = schema ? shapeFromJsonSchema(schema) : undefined;
		return resolved ?? shapeFromTypeText(fallback);
	};
	if (sig.returns.length === 1 && !sig.returns[0].name) {
		// A single unnamed return still carries a schema when there is exactly one output pin.
		const only = outputSchemas ? Object.values(outputSchemas) : [];
		const resolved =
			only.length === 1 ? shapeFromJsonSchema(only[0]) : undefined;
		return resolved ?? shapeFromTypeText(sig.returns[0].type);
	}
	const fields = new Map<string, Shape>();
	const docs = new Map<string, string>();
	let i = 0;
	for (const r of sig.returns) {
		const name = r.name ?? `value${i++}`;
		fields.set(name, deepShape(r.name, r.type));
		if (r.doc) {
			docs.set(name, r.doc);
		}
	}
	return {
		kind: "object",
		text: returnObjectText(sig),
		fields,
		docs,
		origin: sig.name,
	};
}

export function returnObjectText(sig: NodeSignature): string {
	if (sig.returns.length === 0) {
		return "void";
	}
	if (sig.returns.length === 1 && !sig.returns[0].name) {
		return sig.returns[0].type;
	}
	return `{ ${sig.returns
		.map((r) => `${r.name ?? "value"}: ${r.type}`)
		.join(", ")} }`;
}

/** Infer a `Shape` from a parsed JSON-ish literal value. */
function shapeFromValue(value: unknown): Shape {
	if (Array.isArray(value)) {
		const element = value.length > 0 ? shapeFromValue(value[0]) : undefined;
		return {
			kind: "array",
			text: `${element?.text ?? "Struct"}[]`,
			element,
		};
	}
	if (value !== null && typeof value === "object") {
		const fields = new Map<string, Shape>();
		for (const [key, val] of Object.entries(value)) {
			fields.set(key, shapeFromValue(val));
		}
		return { kind: "object", text: "Struct", fields };
	}
	switch (typeof value) {
		case "string":
			return { kind: "scalar", text: "string" };
		case "boolean":
			return { kind: "scalar", text: "bool" };
		case "number":
			return {
				kind: "scalar",
				text: Number.isInteger(value) ? "int" : "float",
			};
		default:
			return { kind: "scalar", text: "null" };
	}
}

/** Parse a struct/array literal tolerantly (strict JSON, else with bare keys quoted). */
function parseLiteral(text: string): unknown {
	const trimmed = text.trim();
	const candidates = [
		trimmed,
		trimmed.replace(/([{,]\s*)([A-Za-z_$][\w$]*)\s*:/g, '$1"$2":'),
	];
	for (const candidate of candidates) {
		try {
			return JSON.parse(candidate);
		} catch {
			// Try the next candidate.
		}
	}
	return undefined;
}

/** Resolve the declared/inferred `Shape` of a variable. */
export function variableShape(
	variable: FlowVariable,
	registry: SignatureRegistry,
	interfaces?: ReadonlyMap<string, FlowInterface>,
): Shape | undefined {
	if (variable.initCall) {
		const sig = registry.get(variable.initCall);
		if (sig) {
			return shapeFromSignature(sig, registry);
		}
	}
	// A `@schema(…)` decorator is the authoritative type for struct variables: it carries the
	// full field set (and descriptions) so member access can walk arbitrarily deep.
	if (variable.schemaText) {
		const shape = shapeFromJsonSchema(variable.schemaText);
		if (shape) {
			if (variable.typeText && shape.kind === "object") {
				return { ...shape, text: variable.typeText };
			}
			return shape;
		}
	}
	if (variable.typeText) {
		const annotated = shapeFromTypeText(variable.typeText, interfaces);
		if (
			annotated.kind === "object" ||
			(annotated.kind === "array" && annotated.element?.kind === "object")
		) {
			return annotated;
		}
	}
	if (variable.initLiteral) {
		const value = parseLiteral(variable.initLiteral);
		if (value !== undefined) {
			const shape = shapeFromValue(value);
			// Prefer an explicit annotation for the top-level display text.
			if (variable.typeText && shape.kind === "object") {
				return { ...shape, text: variable.typeText };
			}
			return shape;
		}
	}
	if (variable.typeText) {
		return shapeFromTypeText(variable.typeText, interfaces);
	}
	return undefined;
}

/** Walk a chain of accessors from a base shape, returning the shape at each step. */
export function walkAccessors(
	base: Shape,
	accessors: readonly Accessor[],
): Shape | undefined {
	let current: Shape | undefined = base;
	for (const acc of accessors) {
		if (!current) {
			return undefined;
		}
		if (acc.kind === "index") {
			current = current.kind === "array" ? current.element : undefined;
		} else {
			current =
				current.kind === "object"
					? current.fields.get(acc.name ?? "")
					: undefined;
		}
	}
	return current;
}

const TRAILING_CHAIN_RE =
	/([A-Za-z_$][\w$]*(?:\s*\.\s*[A-Za-z_$][\w$]*|\s*\[[^\]]*\])*)$/;

/** Parse the access chain ending at the given line text (up to a column). */
export function parseChain(
	textBeforeCursor: string,
): { base: string; accessors: Accessor[] } | undefined {
	const match = TRAILING_CHAIN_RE.exec(textBeforeCursor);
	if (!match) {
		return undefined;
	}
	let rest = match[1];
	const baseMatch = /^([A-Za-z_$][\w$]*)/.exec(rest);
	if (!baseMatch) {
		return undefined;
	}
	const base = baseMatch[1];
	rest = rest.slice(baseMatch[0].length);
	const accessors: Accessor[] = [];
	while (rest.length > 0) {
		const field = /^\s*\.\s*([A-Za-z_$][\w$]*)/.exec(rest);
		if (field) {
			accessors.push({ kind: "field", name: field[1] });
			rest = rest.slice(field[0].length);
			continue;
		}
		const index = /^\s*\[[^\]]*\]/.exec(rest);
		if (index) {
			accessors.push({ kind: "index" });
			rest = rest.slice(index[0].length);
			continue;
		}
		break;
	}
	return { base, accessors };
}

/** Find the variable named `name` declared closest before `position`. */
export function findVariable(
	variables: readonly FlowVariable[],
	name: string,
	position: vscode.Position,
): FlowVariable | undefined {
	let best: FlowVariable | undefined;
	for (const v of variables) {
		if (v.name !== name) {
			continue;
		}
		if (v.range.start.isBeforeOrEqual(position)) {
			best = v;
		} else if (!best) {
			best = v;
		}
	}
	return best;
}
