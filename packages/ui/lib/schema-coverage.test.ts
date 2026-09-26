import { describe, expect, test } from "bun:test";
import { schemaCovers } from "./schema-coverage";

const covers = (output: unknown, input: unknown) =>
	schemaCovers(JSON.stringify(output), JSON.stringify(input));

const object = (
	properties: Record<string, unknown>,
	required: string[] = Object.keys(properties),
	extra: Record<string, unknown> = {},
) => ({ type: "object", properties, required, ...extra });

const string = { type: "string" };
const u64 = { type: "integer", format: "uint64", minimum: 0 };

/** Shaped like schemars output for `Customer { id, name, email: Option, address: Address, tags }`. */
const CUSTOMER = {
	$schema: "https://json-schema.org/draft/2020-12/schema",
	title: "Customer",
	...object(
		{
			id: u64,
			name: string,
			email: { type: ["string", "null"] },
			address: { $ref: "#/$defs/Address" },
			tags: { type: "array", items: string },
		},
		["id", "name", "address", "tags"],
	),
	$defs: {
		Address: { ...object({ street: string, city: string }) },
	},
};

const CUSTOMER_NAME = { title: "CustomerName", ...object({ name: string }) };

const CUSTOMER_CITY = {
	title: "CustomerCity",
	...object({ id: u64, address: { $ref: "#/$defs/CityOnly" } }),
	$defs: { CityOnly: object({ city: string }) },
};

const tree = (extra: Record<string, unknown>) => ({
	title: "Tree",
	...object({
		label: string,
		...extra,
		children: { type: "array", items: { $ref: "#" } },
	}),
});

describe("schemaCovers", () => {
	test("a producer with more fields covers a consumer reading a subset", () => {
		expect(covers(CUSTOMER, CUSTOMER_NAME)).toBe(true);
		expect(covers(CUSTOMER, CUSTOMER_CITY)).toBe(true);
		expect(covers(tree({ weight: { type: "number" } }), tree({}))).toBe(true);
	});

	test("a consumer asking for more than the producer declares is refused", () => {
		expect(covers(CUSTOMER_NAME, CUSTOMER)).toBe(false);
		expect(covers(tree({}), tree({ weight: { type: "number" } }))).toBe(false);
		expect(
			covers(
				{ type: "object", properties: { sub: string } },
				{ type: "object", properties: { count: { type: "number" } } },
			),
		).toBe(false);
	});

	test("an optional field covers only an optional one", () => {
		const required = object({ email: string });
		const optional = object({ email: { type: ["string", "null"] } }, []);
		expect(covers(required, optional)).toBe(true);
		expect(covers(optional, required)).toBe(false);
	});

	test("field types must be covered too", () => {
		expect(
			covers(
				object({ n: { type: "integer" } }),
				object({ n: { type: "number" } }),
			),
		).toBe(true);
		expect(
			covers(
				object({ n: { type: "number" } }),
				object({ n: { type: "integer" } }),
			),
		).toBe(false);
		expect(
			covers(
				object({ n: { type: "integer", format: "int64" } }),
				object({ n: { type: "integer", format: "int32" } }),
			),
		).toBe(false);
	});

	test("a consumer denying unknown fields is covered only by the same fields", () => {
		const strict = object({ name: string }, ["name"], {
			additionalProperties: false,
		});
		expect(covers(CUSTOMER, strict)).toBe(false);
		expect(covers(strict, CUSTOMER_NAME)).toBe(true);
		expect(covers({ ...strict, title: "Other" }, strict)).toBe(true);
	});

	test("map values are compared covariantly", () => {
		const map = (value: unknown) => ({
			type: "object",
			additionalProperties: value,
		});
		expect(covers(map({ type: "integer" }), map({ type: "number" }))).toBe(
			true,
		);
		expect(covers(map({ type: "number" }), map({ type: "integer" }))).toBe(
			false,
		);
		expect(
			covers(
				{ type: "object", properties: { a: string } },
				map({ type: "integer" }),
			),
		).toBe(false);
	});

	test("enums and unions are covered by their subsets", () => {
		expect(
			covers(
				{ type: "string", enum: ["a"] },
				{ type: "string", enum: ["a", "b"] },
			),
		).toBe(true);
		expect(
			covers(
				{ type: "string", enum: ["a", "c"] },
				{ type: "string", enum: ["a", "b"] },
			),
		).toBe(false);
		expect(covers(string, { type: "string", enum: ["a"] })).toBe(false);
		expect(covers(string, { type: ["string", "null"] })).toBe(true);
		expect(covers({ type: ["string", "null"] }, string)).toBe(false);
		expect(
			covers(
				{ anyOf: [string, { type: "null" }] },
				{ anyOf: [{ type: "null" }, string] },
			),
		).toBe(true);
	});

	test("numeric and length bounds must be at least as tight", () => {
		expect(
			covers({ type: "integer", minimum: 1 }, { type: "integer", minimum: 0 }),
		).toBe(true);
		expect(covers({ type: "integer" }, { type: "integer", minimum: 0 })).toBe(
			false,
		);
		expect(
			covers(
				{ type: "array", items: string, minItems: 2 },
				{ type: "array", items: string, minItems: 1 },
			),
		).toBe(true);
		expect(covers(string, { type: "string", maxLength: 4 })).toBe(false);
	});

	test("identical text behind a ref is not trusted across documents", () => {
		const withDefinition = (definition: unknown) => ({
			type: "object",
			properties: { inner: { not: { $ref: "#/$defs/X" } } },
			$defs: { X: definition },
		});
		expect(
			covers(
				withDefinition({ type: "string" }),
				withDefinition({ type: "integer" }),
			),
		).toBe(false);
	});

	test("unknown input assertions must be repeated by the output", () => {
		expect(
			covers(
				{ type: "string", pattern: "^a" },
				{ type: "string", pattern: "^a" },
			),
		).toBe(true);
		expect(covers(string, { type: "string", pattern: "^a" })).toBe(false);
		expect(covers({ type: "object" }, { not: { type: "object" } })).toBe(false);
	});

	test("property names are never read off the prototype", () => {
		expect(
			covers(
				{ type: "object", properties: {} },
				object({ constructor: string }),
			),
		).toBe(false);
	});

	test("unparseable schemas only cover themselves", () => {
		expect(schemaCovers("not json", "not json")).toBe(true);
		expect(schemaCovers("not json", '{"type":"object"}')).toBe(false);
		expect(schemaCovers('{"type":"object"}', "not json")).toBe(false);
	});

	test("a type list output is covered by the equivalent anyOf", () => {
		const nullableBytes = {
			type: ["array", "null"],
			items: { type: "integer" },
		};
		const projection = {
			anyOf: [{ type: "array", items: { type: "integer" } }, { type: "null" }],
		};
		expect(covers(nullableBytes, projection)).toBe(true);
		expect(
			covers(nullableBytes, { type: "array", items: { type: "integer" } }),
		).toBe(false);
		expect(
			covers(
				{ type: ["string", "null"], enum: ["a", null] },
				{ type: ["string", "null"] },
			),
		).toBe(true);
		const tuple = {
			type: "array",
			prefixItems: [{ type: "string" }, { type: ["integer", "null"] }],
		};
		expect(covers(tuple, { ...tuple, description: "a pair" })).toBe(true);
	});

	test("a bit does not cover a cached embedding model", () => {
		const bitTypes = { type: "string", enum: ["Llm", "Embedding"] };
		expect(
			covers(
				{
					title: "Bit",
					type: "object",
					properties: { id: string, type: { $ref: "#/$defs/BitTypes" } },
					$defs: { BitTypes: bitTypes },
				},
				{
					title: "CachedEmbeddingModel",
					...object({
						cache_key: string,
						model_type: { $ref: "#/$defs/BitTypes" },
					}),
					$defs: { BitTypes: bitTypes },
				},
			),
		).toBe(false);
	});
});
