import type * as vscode from "vscode";
import {
	type FlowDocumentModel,
	type FlowInterface,
	type FlowVariable,
	type UseDeclaration,
	calleeOf,
	expandUsePath,
} from "./flowDocument";
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
export function shapeFromSignature(
	sig: NodeSignature,
	registry: SignatureRegistry,
): Shape {
	if (sig.returns.length === 0) {
		return { kind: "scalar", text: "void" };
	}
	const outputSchemas = registry.schemasFor(sig)?.outputs;
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

/** Method class of a resolved shape (`string`, `int`, `array`, a struct title, …). */
export function shapeClass(shape: Shape | undefined): string | undefined {
	if (!shape) {
		return undefined;
	}
	if (shape.kind === "array") {
		return shape.text.startsWith("Set<") ? "set" : "array";
	}
	if (shape.kind === "object") {
		if (shape.origin?.startsWith("Map<")) {
			return "map";
		}
		return shape.text && shape.text !== "Struct" && /^[A-Z][\w$]*$/.test(shape.text)
			? shape.text
			: "struct";
	}
	switch (shape.text) {
		case "string":
			return "string";
		case "int":
			return "int";
		case "float":
		case "number":
			return "float";
		case "bool":
			return "bool";
		case "Date":
			return "datetime";
		case "bytes":
		case "Byte":
			return "bytes";
		case "Path":
		case "PathBuf":
			return "path";
		default:
			return undefined;
	}
}

/** Context needed to resolve expressions: the document model plus its `use` scope. */
export interface ResolveContext {
	readonly registry: SignatureRegistry;
	readonly variables: readonly FlowVariable[];
	readonly interfaces?: ReadonlyMap<string, FlowInterface>;
	readonly uses: readonly UseDeclaration[];
	readonly functionReceivers?: ReadonlyMap<string, string | undefined>;
}

export function contextOf(
	registry: SignatureRegistry,
	model: FlowDocumentModel,
): ResolveContext {
	return {
		registry,
		variables: model.variables,
		interfaces: model.interfaces,
		uses: model.uses,
		functionReceivers: model.functionReceivers,
	};
}

/** Resolve a call spelled `ns::member` (after `use` expansion), `member` (flat or opened) or as a method. */
export function resolveCallee(
	callee: string,
	ctx: ResolveContext,
): NodeSignature | undefined {
	const normalized = callee.replace(/\s*::\s*/g, "::");
	if (normalized.includes("::")) {
		const segments = normalized.split("::");
		const path = expandUsePath(segments.slice(0, -1), ctx.uses);
		return ctx.registry.member(path, segments[segments.length - 1]);
	}
	return ctx.registry.get(normalized) ?? openedMembers(normalized, ctx)[0];
}

/** Nodes a bare name refers to through `use ns::*` / `use ns::{ name }`. */
export function openedMembers(name: string, ctx: ResolveContext): NodeSignature[] {
	const out: NodeSignature[] = [];
	for (const use of ctx.uses) {
		if (use.kind !== "glob" && use.kind !== "members") {
			continue;
		}
		if (use.kind === "members" && !use.members.includes(name)) {
			continue;
		}
		const sig = ctx.registry.member(expandUsePath(use.path, ctx.uses), name);
		if (sig && !out.includes(sig)) {
			out.push(sig);
		}
	}
	return out;
}

/** Namespace keys opened by any `use` line (method-dispatch tie-breaker). */
function openedNamespaces(ctx: ResolveContext): Set<string> {
	const keys = new Set<string>();
	for (const use of ctx.uses) {
		if (use.kind !== "invalid") {
			keys.add(expandUsePath(use.path, ctx.uses).join("::"));
		}
	}
	return keys;
}

/** Candidate nodes for `receiver.member(...)`, narrowed by the receiver class and `use` lines. */
export function resolveMethod(
	receiverShape: Shape | undefined,
	member: string,
	ctx: ResolveContext,
): { readonly candidates: NodeSignature[]; readonly cls?: string } {
	const cls = shapeClass(receiverShape);
	let candidates = ctx.registry.methodCandidates(cls, member);
	if (candidates.length > 1) {
		const opened = openedNamespaces(ctx);
		const preferred = candidates.filter((sig) =>
			sig.namespace ? opened.has(sig.namespace.join("::")) : false,
		);
		if (preferred.length > 0) {
			candidates = preferred;
		}
	}
	return { candidates, cls };
}

/**
 * Resolve the shape of an expression: a literal, a variable (declared before `position`), a
 * flat/qualified call, a method call on a resolved receiver, or a member/index chain over any
 * of those (`http::fetch({ url }).response.body`, `s.trim()`, `items[0].name`).
 */
export function expressionShape(
	expr: string,
	ctx: ResolveContext,
	position?: vscode.Position,
	depth = 0,
): Shape | undefined {
	const e = expr.trim();
	if (!e || depth > 8) {
		return undefined;
	}
	if (e[0] === '"' || e[0] === "'" || e[0] === "`") {
		return { kind: "scalar", text: "string" };
	}
	if (/^-?\d/.test(e)) {
		return { kind: "scalar", text: /^-?\d+(\.\d+)?([eE][+-]?\d+)?$/.test(e) && /[.eE]/.test(e) ? "float" : "int" };
	}
	if (e === "true" || e === "false" || e[0] === "!") {
		return { kind: "scalar", text: "bool" };
	}
	if (e[0] === "[") {
		return { kind: "array", text: "Struct[]" };
	}
	if (e[0] === "{") {
		const value = parseLiteral(e);
		return value === undefined ? { kind: "object", text: "Struct", fields: new Map() } : shapeFromValue(value);
	}
	if (e[0] === "(") {
		const close = matchingBracket(e, 0);
		if (close < 0) {
			return undefined;
		}
		return walkPostfix(expressionShape(e.slice(1, close), ctx, position, depth + 1), e.slice(close + 1), ctx);
	}
	const callee = calleeOf(e);
	if (callee.initCall) {
		const sig = resolveCallee(callee.initCall, ctx);
		const open = e.indexOf("(");
		const close = matchingBracket(e, open);
		if (!sig || close < 0) {
			return undefined;
		}
		return walkPostfix(shapeFromSignature(sig, ctx.registry), e.slice(close + 1), ctx);
	}
	const head = /^([A-Za-z_$][\w$]*)/.exec(e);
	if (!head) {
		return undefined;
	}
	const variable = position
		? findVariable(ctx.variables, head[1], position)
		: ctx.variables.find((v) => v.name === head[1]);
	if (!variable) {
		return undefined;
	}
	return walkPostfix(variableShape(variable, ctx.registry, ctx.interfaces, ctx, position, depth + 1), e.slice(head[1].length), ctx);
}

/** Apply `.member`, `.method(...)` and `[index]` steps to a base shape. */
function walkPostfix(
	base: Shape | undefined,
	rest: string,
	ctx: ResolveContext,
): Shape | undefined {
	let current = base;
	let text = rest.trimStart();
	while (text.length > 0 && current) {
		const member = /^\.\s*([A-Za-z_$][\w$]*)/.exec(text);
		if (member) {
			text = text.slice(member[0].length).trimStart();
			if (text.startsWith("(")) {
				const close = matchingBracket(text, 0);
				if (close < 0) {
					return undefined;
				}
				const { candidates } = resolveMethod(current, member[1], ctx);
				current = candidates.length === 1 ? shapeFromSignature(candidates[0], ctx.registry) : undefined;
				text = text.slice(close + 1).trimStart();
			} else if (member[1] === "length" && current.kind === "array") {
				current = { kind: "scalar", text: "int" };
			} else {
				current = walkAccessors(current, [{ kind: "field", name: member[1] }]);
			}
			continue;
		}
		if (text.startsWith("[")) {
			const close = matchingBracket(text, 0);
			if (close < 0) {
				return undefined;
			}
			current = walkAccessors(current, [{ kind: "index" }]);
			text = text.slice(close + 1).trimStart();
			continue;
		}
		return undefined;
	}
	return current;
}

function matchingBracket(text: string, open: number): number {
	let depth = 0;
	for (let i = open; i < text.length; i++) {
		const c = text[i];
		if (c === "(" || c === "[" || c === "{") {
			depth++;
		} else if (c === ")" || c === "]" || c === "}") {
			depth--;
			if (depth === 0) {
				return i;
			}
		}
	}
	return -1;
}

/** Resolve the declared/inferred `Shape` of a variable. */
export function variableShape(
	variable: FlowVariable,
	registry: SignatureRegistry,
	interfaces?: ReadonlyMap<string, FlowInterface>,
	ctx?: ResolveContext,
	position?: vscode.Position,
	depth = 0,
): Shape | undefined {
	const resolveCtx: ResolveContext = ctx ?? {
		registry,
		variables: [],
		interfaces,
		uses: [],
	};
	const selectField = (shape: Shape | undefined): Shape | undefined =>
		variable.initField && shape
			? walkAccessors(shape, [{ kind: "field", name: variable.initField }])
			: shape;
	if (variable.initCall) {
		const sig = resolveCallee(variable.initCall, resolveCtx);
		if (sig) {
			const shape = shapeFromSignature(sig, registry);
			if (variable.iterates && !sig.returns.some((r) => r.name === "value")) {
				return walkAccessors(shape, [{ kind: "index" }]);
			}
			return selectField(shape);
		}
	}
	if (variable.initMethod && depth <= 8) {
		const receiver = expressionShape(
			variable.initMethod.receiverText,
			resolveCtx,
			position ?? variable.range.start,
			depth + 1,
		);
		const { candidates } = resolveMethod(receiver, variable.initMethod.member, resolveCtx);
		if (candidates.length === 1) {
			return selectField(shapeFromSignature(candidates[0], registry));
		}
	}
	if (variable.iterates && depth <= 8) {
		const iterated = expressionShape(
			variable.iterates,
			resolveCtx,
			position ?? variable.range.start,
			depth + 1,
		);
		if (iterated) {
			return walkAccessors(iterated, [{ kind: "index" }]);
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
	if (variable.typeText) {
		return shapeFromTypeText(variable.typeText, interfaces);
	}
	if (variable.initLiteral) {
		const value = parseLiteral(variable.initLiteral);
		if (value !== undefined) {
			const shape = shapeFromValue(value);
			return variable.typeText && shape.kind === "object"
				? { ...shape, text: variable.typeText }
				: shape;
		}
		if (depth <= 8) {
			// Not JSON: a single-quoted / template string, or an expression over other bindings.
			return expressionShape(
				variable.initLiteral,
				resolveCtx,
				position ?? variable.range.start,
				depth + 1,
			);
		}
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
