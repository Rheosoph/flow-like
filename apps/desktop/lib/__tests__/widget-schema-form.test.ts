import type { JsonSchema } from "@flow-like/widget-sdk";
import { describe, expect, it } from "vitest";
import {
	createJsonSchemaValue,
	homogeneousArrayItemSchema,
	jsonSchemaType,
	parseWidgetListDraft,
	serializeWidgetList,
	summarizeWidgetListItem,
} from "../widget-schema-form";

const SALES_ROW_SCHEMA: JsonSchema = {
	type: "object",
	properties: {
		label: { type: "string" },
		value: { type: "number", minimum: 10 },
		category: { type: "string" },
		note: { type: "string" },
	},
	required: ["label", "value", "category"],
};

describe("widget schema form helpers", () => {
	it("creates an editable item from required object properties", () => {
		expect(createJsonSchemaValue(SALES_ROW_SCHEMA)).toEqual({
			label: "",
			value: 10,
			category: "",
		});
	});

	it("recognizes homogeneous lists but leaves tuples to raw JSON", () => {
		expect(
			homogeneousArrayItemSchema({
				type: "array",
				items: SALES_ROW_SCHEMA,
			}),
		).toEqual(SALES_ROW_SCHEMA);
		expect(
			homogeneousArrayItemSchema({
				type: "array",
				items: [{ type: "string" }, { type: "number" }],
			}),
		).toBeNull();
	});

	it("honors schema defaults, constants, enums, and nullable unions", () => {
		expect(createJsonSchemaValue({ type: "string", default: "Hardware" })).toBe(
			"Hardware",
		);
		expect(createJsonSchemaValue({ const: 42 })).toBe(42);
		expect(createJsonSchemaValue({ enum: ["bar", "line"] })).toBe("bar");
		expect(
			jsonSchemaType({ anyOf: [{ type: "null" }, { type: "string" }] }),
		).toBe("string");
		expect(
			jsonSchemaType({ anyOf: [{ type: "string" }, { type: "number" }] }),
		).toBeUndefined();
	});

	it("parses and serializes array drafts", () => {
		const parsed = parseWidgetListDraft('[{"label":"Jul","value":4200}]');
		expect(parsed).toEqual({
			items: [{ label: "Jul", value: 4200 }],
			error: null,
		});
		expect(serializeWidgetList(parsed.items ?? [])).toBe(
			'[\n  {\n    "label": "Jul",\n    "value": 4200\n  }\n]',
		);
	});

	it("reports invalid and non-array drafts without throwing", () => {
		expect(parseWidgetListDraft("{").error).toContain("Invalid JSON");
		expect(parseWidgetListDraft("{}")).toEqual({
			items: null,
			error: "Expected a JSON array",
		});
	});

	it("builds compact summaries for object and primitive items", () => {
		expect(
			summarizeWidgetListItem(
				{ label: "Jul", value: 4200, category: "Hardware" },
				0,
			),
		).toEqual({
			title: "Jul",
			detail: "value: 4200 · category: Hardware",
		});
		expect(summarizeWidgetListItem("Software", 1)).toEqual({
			title: "Software",
		});
	});
});
