import { describe, expect, test } from "bun:test";
import type { ContractInput } from "../src/contract";
import { validateInputValue, validateSchema } from "../src/validate";

describe("validateSchema", () => {
	test("null or undefined schema is always valid", () => {
		expect(validateSchema(null, 42).valid).toBe(true);
		expect(validateSchema(undefined, { any: "thing" }).valid).toBe(true);
	});

	test("empty schema accepts everything", () => {
		expect(validateSchema({}, "x").valid).toBe(true);
		expect(validateSchema({}, null).valid).toBe(true);
	});

	describe("type", () => {
		test("string", () => {
			expect(validateSchema({ type: "string" }, "ok").valid).toBe(true);
			expect(validateSchema({ type: "string" }, 1).valid).toBe(false);
		});

		test("number", () => {
			expect(validateSchema({ type: "number" }, 1.5).valid).toBe(true);
			expect(validateSchema({ type: "number" }, "1.5").valid).toBe(false);
		});

		test("integer", () => {
			expect(validateSchema({ type: "integer" }, 3).valid).toBe(true);
			expect(validateSchema({ type: "integer" }, 3.5).valid).toBe(false);
			expect(validateSchema({ type: "integer" }, "3").valid).toBe(false);
		});

		test("boolean", () => {
			expect(validateSchema({ type: "boolean" }, false).valid).toBe(true);
			expect(validateSchema({ type: "boolean" }, 0).valid).toBe(false);
		});

		test("null", () => {
			expect(validateSchema({ type: "null" }, null).valid).toBe(true);
			expect(validateSchema({ type: "null" }, undefined).valid).toBe(false);
		});

		test("array", () => {
			expect(validateSchema({ type: "array" }, []).valid).toBe(true);
			expect(validateSchema({ type: "array" }, {}).valid).toBe(false);
		});

		test("object", () => {
			expect(validateSchema({ type: "object" }, {}).valid).toBe(true);
			expect(validateSchema({ type: "object" }, []).valid).toBe(false);
			expect(validateSchema({ type: "object" }, null).valid).toBe(false);
		});

		test("array of types", () => {
			const schema = { type: ["string", "null"] };
			expect(validateSchema(schema, "x").valid).toBe(true);
			expect(validateSchema(schema, null).valid).toBe(true);
			expect(validateSchema(schema, 1).valid).toBe(false);
		});

		test("unknown type names pass through", () => {
			expect(validateSchema({ type: "date-time" }, "whatever").valid).toBe(
				true,
			);
		});
	});

	describe("enum and const", () => {
		test("enum with primitives", () => {
			const schema = { enum: ["bar", "line"] };
			expect(validateSchema(schema, "bar").valid).toBe(true);
			expect(validateSchema(schema, "pie").valid).toBe(false);
		});

		test("enum with deep values", () => {
			const schema = { enum: [{ a: [1, 2] }, null] };
			expect(validateSchema(schema, { a: [1, 2] }).valid).toBe(true);
			expect(validateSchema(schema, null).valid).toBe(true);
			expect(validateSchema(schema, { a: [1, 3] }).valid).toBe(false);
		});

		test("const", () => {
			expect(validateSchema({ const: 5 }, 5).valid).toBe(true);
			expect(validateSchema({ const: 5 }, 6).valid).toBe(false);
			expect(validateSchema({ const: { x: 1 } }, { x: 1 }).valid).toBe(true);
		});
	});

	describe("numeric bounds", () => {
		test("minimum / maximum", () => {
			const schema = { type: "number", minimum: 1, maximum: 10 };
			expect(validateSchema(schema, 1).valid).toBe(true);
			expect(validateSchema(schema, 10).valid).toBe(true);
			expect(validateSchema(schema, 0).valid).toBe(false);
			expect(validateSchema(schema, 11).valid).toBe(false);
		});

		test("exclusiveMinimum / exclusiveMaximum", () => {
			const schema = { exclusiveMinimum: 0, exclusiveMaximum: 1 };
			expect(validateSchema(schema, 0.5).valid).toBe(true);
			expect(validateSchema(schema, 0).valid).toBe(false);
			expect(validateSchema(schema, 1).valid).toBe(false);
		});
	});

	describe("string constraints", () => {
		test("minLength / maxLength", () => {
			const schema = { minLength: 2, maxLength: 4 };
			expect(validateSchema(schema, "ab").valid).toBe(true);
			expect(validateSchema(schema, "a").valid).toBe(false);
			expect(validateSchema(schema, "abcde").valid).toBe(false);
		});

		test("pattern", () => {
			const schema = { pattern: "^[a-z]+$" };
			expect(validateSchema(schema, "abc").valid).toBe(true);
			expect(validateSchema(schema, "Abc").valid).toBe(false);
		});

		test("invalid pattern is ignored", () => {
			expect(validateSchema({ pattern: "([" }, "anything").valid).toBe(true);
		});
	});

	describe("arrays", () => {
		test("items schema", () => {
			const schema = { type: "array", items: { type: "number" } };
			expect(validateSchema(schema, [1, 2, 3]).valid).toBe(true);
			expect(validateSchema(schema, [1, "2"]).valid).toBe(false);
		});

		test("tuple items", () => {
			const schema = { items: [{ type: "string" }, { type: "number" }] };
			expect(validateSchema(schema, ["a", 1]).valid).toBe(true);
			expect(validateSchema(schema, [1, "a"]).valid).toBe(false);
		});

		test("minItems / maxItems", () => {
			const schema = { minItems: 1, maxItems: 2 };
			expect(validateSchema(schema, [1]).valid).toBe(true);
			expect(validateSchema(schema, []).valid).toBe(false);
			expect(validateSchema(schema, [1, 2, 3]).valid).toBe(false);
		});
	});

	describe("objects", () => {
		test("properties and required", () => {
			const schema = {
				type: "object",
				properties: { x: { type: "string" }, y: { type: "number" } },
				required: ["x"],
			};
			expect(validateSchema(schema, { x: "a", y: 1 }).valid).toBe(true);
			expect(validateSchema(schema, { y: 1 }).valid).toBe(false);
			expect(validateSchema(schema, { x: 1 }).valid).toBe(false);
			expect(validateSchema(schema, { x: "a" }).valid).toBe(true);
		});

		test("additionalProperties: false", () => {
			const schema = {
				properties: { x: { type: "string" } },
				additionalProperties: false,
			};
			expect(validateSchema(schema, { x: "a" }).valid).toBe(true);
			expect(validateSchema(schema, { x: "a", extra: 1 }).valid).toBe(false);
		});

		test("additionalProperties as schema", () => {
			const schema = {
				properties: { x: { type: "string" } },
				additionalProperties: { type: "number" },
			};
			expect(validateSchema(schema, { x: "a", extra: 1 }).valid).toBe(true);
			expect(validateSchema(schema, { x: "a", extra: "no" }).valid).toBe(false);
		});
	});

	describe("combinators", () => {
		test("anyOf", () => {
			const schema = { anyOf: [{ type: "string" }, { type: "number" }] };
			expect(validateSchema(schema, "a").valid).toBe(true);
			expect(validateSchema(schema, 1).valid).toBe(true);
			expect(validateSchema(schema, true).valid).toBe(false);
		});

		test("oneOf requires exactly one match", () => {
			const schema = {
				oneOf: [{ type: "number" }, { type: "integer" }],
			};
			expect(validateSchema(schema, 1.5).valid).toBe(true);
			expect(validateSchema(schema, 2).valid).toBe(false);
			expect(validateSchema(schema, "x").valid).toBe(false);
		});

		test("allOf", () => {
			const schema = {
				allOf: [{ type: "number" }, { minimum: 5 }],
			};
			expect(validateSchema(schema, 7).valid).toBe(true);
			expect(validateSchema(schema, 3).valid).toBe(false);
		});
	});

	test("$ref is treated as valid (schemas must be pre-inlined)", () => {
		expect(
			validateSchema({ $ref: "#/definitions/thing" }, { any: 1 }).valid,
		).toBe(true);
	});

	test("nested $ref inside properties is treated as valid", () => {
		const schema = {
			type: "object",
			properties: { row: { $ref: "#/definitions/row" } },
		};
		expect(validateSchema(schema, { row: "anything" }).valid).toBe(true);
	});

	test("composite: sales rows schema", () => {
		const schema = {
			type: "array",
			items: {
				type: "object",
				properties: {
					x: { type: "string", minLength: 1 },
					y: { type: "number", minimum: 0 },
				},
				required: ["x", "y"],
				additionalProperties: false,
			},
			maxItems: 3,
		};
		expect(
			validateSchema(schema, [
				{ x: "Q1", y: 10 },
				{ x: "Q2", y: 0 },
			]).valid,
		).toBe(true);
		const bad = validateSchema(schema, [{ x: "", y: -1, z: true }]);
		expect(bad.valid).toBe(false);
		expect(bad.errors.length).toBe(3);
		expect(bad.errors.join("\n")).toContain("$[0]");
	});

	test("composite: discriminated union via oneOf", () => {
		const schema = {
			oneOf: [
				{
					type: "object",
					properties: { kind: { const: "a" }, size: { type: "integer" } },
					required: ["kind", "size"],
				},
				{
					type: "object",
					properties: { kind: { const: "b" }, label: { type: "string" } },
					required: ["kind", "label"],
				},
			],
		};
		expect(validateSchema(schema, { kind: "a", size: 2 }).valid).toBe(true);
		expect(validateSchema(schema, { kind: "b", label: "x" }).valid).toBe(true);
		expect(validateSchema(schema, { kind: "b", size: 2 }).valid).toBe(false);
	});

	test("error paths name the failing location", () => {
		const result = validateSchema(
			{
				type: "object",
				properties: { rows: { type: "array", items: { type: "number" } } },
			},
			{ rows: [1, "two"] },
		);
		expect(result.valid).toBe(false);
		expect(result.errors[0]).toContain("$.rows[1]");
	});
});

describe("validateInputValue", () => {
	const input = (partial: Partial<ContractInput>): ContractInput => ({
		type: "string",
		...partial,
	});

	test("string", () => {
		expect(validateInputValue(input({ type: "string" }), "x").valid).toBe(true);
		expect(validateInputValue(input({ type: "string" }), 1).valid).toBe(false);
	});

	test("number with min/max", () => {
		const numeric = input({ type: "number", min: 1, max: 500 });
		expect(validateInputValue(numeric, 50).valid).toBe(true);
		expect(validateInputValue(numeric, 0).valid).toBe(false);
		expect(validateInputValue(numeric, 501).valid).toBe(false);
		expect(validateInputValue(numeric, "50").valid).toBe(false);
	});

	test("integer rejects fractions", () => {
		expect(validateInputValue(input({ type: "integer" }), 2).valid).toBe(true);
		expect(validateInputValue(input({ type: "integer" }), 2.5).valid).toBe(
			false,
		);
	});

	test("boolean", () => {
		expect(validateInputValue(input({ type: "boolean" }), true).valid).toBe(
			true,
		);
		expect(validateInputValue(input({ type: "boolean" }), "true").valid).toBe(
			false,
		);
	});

	test("enum must be one of choices", () => {
		const variant = input({ type: "enum", choices: ["bar", "line"] });
		expect(validateInputValue(variant, "bar").valid).toBe(true);
		expect(validateInputValue(variant, "pie").valid).toBe(false);
		expect(validateInputValue(variant, 1).valid).toBe(false);
	});

	test("json validates against the attached schema", () => {
		const rows = input({
			type: "json",
			schema: { type: "array", items: { type: "object" } },
		});
		expect(validateInputValue(rows, [{}]).valid).toBe(true);
		expect(validateInputValue(rows, [1]).valid).toBe(false);
	});

	test("json without schema accepts anything", () => {
		expect(validateInputValue(input({ type: "json" }), "free").valid).toBe(
			true,
		);
	});

	test("undefined is only valid for optional inputs", () => {
		expect(
			validateInputValue(input({ type: "string", optional: true }), undefined)
				.valid,
		).toBe(true);
		expect(validateInputValue(input({ type: "string" }), undefined).valid).toBe(
			false,
		);
	});
});
