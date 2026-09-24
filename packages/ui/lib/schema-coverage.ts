type Schema = Record<string, unknown>;

const MAX_DEPTH = 64;
const MAX_CACHED_SCHEMAS = 512;

/** Keywords that never restrict which values a schema admits. */
const ANNOTATIONS = new Set([
	"$schema",
	"$id",
	"$anchor",
	"$comment",
	"$defs",
	"definitions",
	"title",
	"description",
	"default",
	"examples",
	"readOnly",
	"writeOnly",
	"deprecated",
]);

/**
 * Keywords whose meaning depends on the schema around them, so an identical keyword on the output
 * side proves nothing. A schema using one is only covered by an identical schema.
 */
const CONTEXTUAL = [
	"unevaluatedProperties",
	"unevaluatedItems",
	"$dynamicRef",
	"$recursiveRef",
];

const REFERENCES = ["$ref", ...CONTEXTUAL];

/**
 * Keywords compared structurally. Every other assertion the input makes must be repeated verbatim
 * by the output.
 */
const STRUCTURAL = new Set([
	"$ref",
	"allOf",
	"anyOf",
	"oneOf",
	"type",
	"enum",
	"const",
	"properties",
	"required",
	"additionalProperties",
	"items",
	"prefixItems",
	"minItems",
	"maxItems",
	"uniqueItems",
	"minLength",
	"maxLength",
	"minimum",
	"maximum",
	"exclusiveMinimum",
	"exclusiveMaximum",
]);

const INVALID = Symbol("invalid schema");
const UNRESOLVED = Symbol("unresolved reference");
const parsedSchemas = new Map<string, unknown>();

/**
 * Parse schema text once per distinct text: catalog filtering compares one dragged pin with
 * thousands of candidates. `undefined` for text that is not JSON. Callers must not mutate the result.
 */
export function parseSchemaText(schema: string): unknown {
	let parsed = parsedSchemas.get(schema);
	if (parsed === undefined) {
		try {
			parsed = JSON.parse(schema);
		} catch {
			parsed = INVALID;
		}
		if (parsedSchemas.size >= MAX_CACHED_SCHEMAS) parsedSchemas.clear();
		parsedSchemas.set(schema, parsed);
	}
	return parsed === INVALID ? undefined : parsed;
}

/**
 * Mirrors `flow_like::flow::pin::schema_covers`.
 *
 * Whether every value the `output` schema admits is also admitted by `input`, so a pin declaring
 * `output` may feed a pin declaring `input`. The output has to declare every property the input
 * declares, with a schema that is itself covered, and require at least what the input requires.
 * It may declare more, and titles are ignored: a producer struct with extra fields feeds a
 * consumer that reads a subset of them. Anything that cannot be proven covered is refused.
 */
export function schemaCovers(output: string, input: string): boolean {
	if (output === input) return true;
	const parsedOutput = parseSchemaText(output);
	const parsedInput = parseSchemaText(input);
	if (parsedOutput === undefined || parsedInput === undefined) return false;
	const outputRoot = typeUnionsSplit(parsedOutput);
	const inputRoot = typeUnionsSplit(parsedInput);
	return new Coverage(outputRoot, inputRoot).covers(outputRoot, inputRoot, 0);
}

/** Keywords holding one sub-schema that coverage compares structurally. */
const SUBSCHEMA_KEYWORDS = new Set(["additionalProperties", "items"]);
/** Keywords holding a map of sub-schemas that coverage compares structurally. */
const SUBSCHEMA_MAP_KEYWORDS = new Set(["properties", "$defs", "definitions"]);
/** Keywords holding a list of sub-schemas that coverage compares structurally. */
const SUBSCHEMA_LIST_KEYWORDS = new Set(["allOf", "anyOf", "oneOf"]);
const splitSchemas = new WeakMap<object, unknown>();

/** `splitTypeUnions` of a parsed (shared, immutable) schema, computed once per parse. */
function typeUnionsSplit(schema: unknown): unknown {
	if (typeof schema !== "object" || schema === null) return schema;
	let split = splitSchemas.get(schema);
	if (split === undefined) {
		split = splitTypeUnions(schema);
		splitSchemas.set(schema, split);
	}
	return split;
}

/**
 * Mirrors `split_type_unions`: `{type: [A, B], ...rest}` becomes
 * `{anyOf: [{type: A, ...rest}, {type: B, ...rest}]}`, annotations (`$defs` among them) staying
 * outside. A type list is a union, but coverage only splits explicit `anyOf` outputs, so serde's
 * `Option<Vec<T>>` was never covered by the `anyOf` the same field projects to.
 */
function splitTypeUnions(schema: unknown): unknown {
	if (!isSchema(schema)) return schema;
	const entries = Object.entries(schema).map(
		([key, value]): [string, unknown] => [key, splitSubschemas(key, value)],
	);
	const kinds = Object.hasOwn(schema, "type") ? schema.type : undefined;
	if (!Array.isArray(kinds) || kinds.length < 2)
		return Object.fromEntries(entries);
	const annotations = entries.filter(([key]) => ANNOTATIONS.has(key));
	const rest = entries.filter(
		([key]) => key !== "type" && !ANNOTATIONS.has(key),
	);
	return Object.fromEntries([
		...annotations,
		[
			"anyOf",
			kinds.map((kind) => Object.fromEntries([...rest, ["type", kind]])),
		],
	]);
}

/**
 * Only structurally compared sub-schemas are rewritten; both roots are, so keywords compared
 * verbatim (`prefixItems`, array-form `items`, `not`, …) still compare like with like.
 */
function splitSubschemas(key: string, value: unknown): unknown {
	if (SUBSCHEMA_KEYWORDS.has(key) && isSchema(value))
		return splitTypeUnions(value);
	if (SUBSCHEMA_MAP_KEYWORDS.has(key) && isSchema(value))
		return Object.fromEntries(
			Object.entries(value).map(([name, member]) => [
				name,
				splitTypeUnions(member),
			]),
		);
	if (SUBSCHEMA_LIST_KEYWORDS.has(key) && Array.isArray(value))
		return value.map(splitTypeUnions);
	return value;
}

class Coverage {
	private readonly assumed = new Map<unknown, Set<unknown>>();

	constructor(
		private readonly outputRoot: unknown,
		private readonly inputRoot: unknown,
	) {}

	covers(outputSchema: unknown, inputSchema: unknown, depth: number): boolean {
		if (typeof inputSchema === "boolean")
			return inputSchema || outputSchema === false;
		if (!isSchema(inputSchema)) return false;
		if (!constrains(inputSchema)) return true;
		if (typeof outputSchema === "boolean") return !outputSchema;
		if (!isSchema(outputSchema)) return false;
		if (depth > MAX_DEPTH) return false;

		// Only a recursive `$ref` can bring a pair back onto the stack; assuming it holds is what
		// lets two recursive types be compared at all.
		let pending = this.assumed.get(outputSchema);
		if (pending?.has(inputSchema)) return true;
		if (!pending) {
			pending = new Set();
			this.assumed.set(outputSchema, pending);
		}
		pending.add(inputSchema);
		const covered = this.coversOutput(outputSchema, inputSchema, depth + 1);
		pending.delete(inputSchema);
		return covered;
	}

	/**
	 * The output admits the intersection of its `$ref`, its `allOf` members and its own keywords,
	 * and the union of its `anyOf`/`oneOf` variants, so proving any one of those covered suffices.
	 */
	private coversOutput(output: Schema, input: Schema, depth: number): boolean {
		const target = reference(this.outputRoot, output);
		if (
			target !== undefined &&
			target !== UNRESOLVED &&
			this.covers(target, input, depth)
		)
			return true;
		if (
			members(output, "allOf").some((member) =>
				this.covers(member, input, depth),
			)
		)
			return true;
		for (const key of ["anyOf", "oneOf"]) {
			const variants = members(output, key);
			if (
				variants.length > 0 &&
				variants.every((variant) => this.covers(variant, input, depth))
			)
				return true;
		}
		return this.coversInput(output, input, depth);
	}

	/**
	 * The input admits only the intersection of its `$ref`, its `allOf` members, one of its
	 * `anyOf`/`oneOf` variants and its own keywords, so each must hold. `oneOf` is read as `anyOf`:
	 * pin schemas describe serde enums, whose variants do not overlap.
	 */
	private coversInput(output: Schema, input: Schema, depth: number): boolean {
		const target = reference(this.inputRoot, input);
		if (target === UNRESOLVED) return false;
		if (target !== undefined && !this.covers(output, target, depth))
			return false;
		if (
			!members(input, "allOf").every((member) =>
				this.covers(output, member, depth),
			)
		)
			return false;
		for (const key of ["anyOf", "oneOf"]) {
			if (
				has(input, key) &&
				!members(input, key).some((variant) =>
					this.covers(output, variant, depth),
				)
			)
				return false;
		}
		return this.coversKeywords(output, input, depth);
	}

	private coversKeywords(
		output: Schema,
		input: Schema,
		depth: number,
	): boolean {
		if (CONTEXTUAL.some((key) => has(input, key)))
			return (
				deepEqual(output, input) &&
				!Object.values(input).some(mentionsReference)
			);

		const outputTypes = types(output);
		const inputTypes = types(input);
		if (inputTypes) {
			if (!outputTypes) return false;
			if (
				!outputTypes.every(
					(kind) =>
						inputTypes.includes(kind) ||
						(kind === "integer" && inputTypes.includes("number")),
				)
			)
				return false;
		}

		for (const key of ["enum", "const"]) {
			if (!has(input, key)) continue;
			const value = input[key];
			if (key === "enum" && !Array.isArray(value)) return false;
			const admitted = key === "enum" ? (value as unknown[]) : [value];
			const produced = literals(output);
			if (
				!produced?.every((literal) =>
					admitted.some((candidate) => deepEqual(literal, candidate)),
				)
			)
				return false;
		}

		if (
			couldBe(outputTypes, "object") &&
			!this.coversObject(output, input, depth)
		)
			return false;
		if (
			couldBe(outputTypes, "array") &&
			!this.coversArray(output, input, depth)
		)
			return false;
		if (
			couldBe(outputTypes, "string") &&
			!(
				atLeast(output, input, "minLength") &&
				atMost(output, input, "maxLength")
			)
		)
			return false;
		if (couldBe(outputTypes, "number") && !numericBounds(output, input))
			return false;

		return Object.entries(input).every(
			([key, value]) =>
				ANNOTATIONS.has(key) ||
				STRUCTURAL.has(key) ||
				verbatim(get(output, key), value),
		);
	}

	private coversObject(output: Schema, input: Schema, depth: number): boolean {
		const outputRequired = strings(output, "required");
		if (
			!strings(input, "required").every((name) => outputRequired.includes(name))
		)
			return false;

		const outputProperties = objectAt(output, "properties");
		const inputProperties = objectAt(input, "properties");
		for (const [name, admitted] of Object.entries(inputProperties ?? {})) {
			if (!outputProperties || !has(outputProperties, name)) return false;
			if (!this.covers(outputProperties[name], admitted, depth)) return false;
		}

		const extra = get(input, "additionalProperties");
		if (extra === undefined || extra === true) return true;
		if (has(output, "patternProperties")) return false;
		for (const [name, produced] of Object.entries(outputProperties ?? {})) {
			if (inputProperties && has(inputProperties, name)) continue;
			if (!this.covers(produced, extra, depth)) return false;
		}
		const produced = get(output, "additionalProperties");
		return this.covers(produced === undefined ? true : produced, extra, depth);
	}

	private coversArray(output: Schema, input: Schema, depth: number): boolean {
		if (!verbatim(get(output, "prefixItems"), get(input, "prefixItems")))
			return false;
		const admitted = get(input, "items");
		if (admitted !== undefined) {
			const produced = get(output, "items");
			if (Array.isArray(admitted)) {
				if (!verbatim(produced, admitted)) return false;
			} else if (Array.isArray(produced)) {
				return false;
			} else if (
				!this.covers(produced === undefined ? true : produced, admitted, depth)
			) {
				return false;
			}
		}
		return (
			atLeast(output, input, "minItems") &&
			atMost(output, input, "maxItems") &&
			(get(input, "uniqueItems") !== true ||
				get(output, "uniqueItems") === true)
		);
	}
}

function isSchema(value: unknown): value is Schema {
	return value !== null && typeof value === "object" && !Array.isArray(value);
}

function has(schema: Schema, key: string): boolean {
	return Object.prototype.hasOwnProperty.call(schema, key);
}

function get(schema: Schema, key: string): unknown {
	return has(schema, key) ? schema[key] : undefined;
}

function objectAt(schema: Schema, key: string): Schema | undefined {
	const value = get(schema, key);
	return isSchema(value) ? value : undefined;
}

function constrains(schema: Schema): boolean {
	return Object.keys(schema).some((key) => !ANNOTATIONS.has(key));
}

/** `undefined` without a `$ref`, `UNRESOLVED` for one that does not point into the document. */
function reference(root: unknown, schema: Schema): unknown {
	if (!has(schema, "$ref")) return undefined;
	const target = schema.$ref;
	if (typeof target !== "string" || !target.startsWith("#")) return UNRESOLVED;
	const pointer = target.slice(1);
	if (pointer === "") return root;
	if (!pointer.startsWith("/")) return UNRESOLVED;
	let current = root;
	for (const token of pointer.slice(1).split("/")) {
		const key = token.replace(/~1/g, "/").replace(/~0/g, "~");
		if (Array.isArray(current) && /^(0|[1-9]\d*)$/.test(key)) {
			current = current[Number(key)];
		} else if (isSchema(current) && has(current, key)) {
			current = current[key];
		} else {
			return UNRESOLVED;
		}
		if (current === undefined) return UNRESOLVED;
	}
	return current;
}

function members(schema: Schema, key: string): unknown[] {
	const value = get(schema, key);
	return Array.isArray(value) ? value : [];
}

function types(schema: Schema): string[] | undefined {
	const kind = get(schema, "type");
	if (kind === undefined) return undefined;
	if (typeof kind === "string") return [kind];
	if (Array.isArray(kind))
		return kind.filter((item): item is string => typeof item === "string");
	return [];
}

function couldBe(kinds: string[] | undefined, kind: string): boolean {
	return (
		!kinds ||
		kinds.includes(kind) ||
		(kind === "number" && kinds.includes("integer"))
	);
}

function literals(schema: Schema): unknown[] | undefined {
	if (has(schema, "const")) return [schema.const];
	const values = get(schema, "enum");
	return Array.isArray(values) ? values : undefined;
}

function strings(schema: Schema, key: string): string[] {
	return members(schema, key).filter(
		(value): value is string => typeof value === "string",
	);
}

function atLeast(output: Schema, input: Schema, key: string): boolean {
	const limit = get(input, key);
	if (limit === undefined) return true;
	const declared = get(output, key);
	const produced = declared === undefined ? 0 : declared;
	return (
		typeof limit === "number" &&
		typeof produced === "number" &&
		produced >= limit
	);
}

function atMost(output: Schema, input: Schema, key: string): boolean {
	const limit = get(input, key);
	if (limit === undefined) return true;
	const produced = get(output, key);
	return (
		typeof limit === "number" &&
		typeof produced === "number" &&
		produced <= limit
	);
}

function numericBounds(output: Schema, input: Schema): boolean {
	const number = (key: string) => {
		const value = get(output, key);
		return typeof value === "number" ? value : undefined;
	};
	const floor = number("minimum");
	const strictFloor = number("exclusiveMinimum");
	const ceiling = number("maximum");
	const strictCeiling = number("exclusiveMaximum");
	const bounded = (key: string, holds: (limit: number) => boolean) => {
		const limit = get(input, key);
		return limit === undefined || (typeof limit === "number" && holds(limit));
	};
	return (
		bounded(
			"minimum",
			(limit) =>
				(floor !== undefined && floor >= limit) ||
				(strictFloor !== undefined && strictFloor >= limit),
		) &&
		bounded(
			"exclusiveMinimum",
			(limit) =>
				(strictFloor !== undefined && strictFloor >= limit) ||
				(floor !== undefined && floor > limit),
		) &&
		bounded(
			"maximum",
			(limit) =>
				(ceiling !== undefined && ceiling <= limit) ||
				(strictCeiling !== undefined && strictCeiling <= limit),
		) &&
		bounded(
			"exclusiveMaximum",
			(limit) =>
				(strictCeiling !== undefined && strictCeiling <= limit) ||
				(ceiling !== undefined && ceiling < limit),
		)
	);
}

/**
 * Identical keyword values, provided neither side leans on a `$ref`: the two schemas resolve
 * references against different documents, so identical text can name different shapes.
 */
function verbatim(output: unknown, input: unknown): boolean {
	return deepEqual(output, input) && !mentionsReference(input);
}

function mentionsReference(value: unknown): boolean {
	if (Array.isArray(value)) return value.some(mentionsReference);
	if (!isSchema(value)) return false;
	return (
		REFERENCES.some((key) => has(value, key)) ||
		Object.values(value).some(mentionsReference)
	);
}

function deepEqual(left: unknown, right: unknown): boolean {
	if (left === right) return true;
	if (Array.isArray(left))
		return (
			Array.isArray(right) &&
			left.length === right.length &&
			left.every((item, index) => deepEqual(item, right[index]))
		);
	if (!isSchema(left) || !isSchema(right)) return false;
	const keys = Object.keys(left);
	return (
		keys.length === Object.keys(right).length &&
		keys.every((key) => has(right, key) && deepEqual(left[key], right[key]))
	);
}
