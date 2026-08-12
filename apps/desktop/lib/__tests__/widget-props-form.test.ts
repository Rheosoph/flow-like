import type { WidgetContract } from "@flow-like/widget-sdk";
import { describe, expect, it } from "vitest";
import {
	createWidgetPropsDraft,
	emptyWidgetPropDraft,
	parseWidgetPropsDraft,
} from "../widget-props-form";

const CONTRACT: WidgetContract = {
	contractVersion: 1,
	id: "sales-chart",
	inputs: {
		title: { type: "string", default: "Revenue" },
		variant: {
			type: "enum",
			choices: ["bar", "line"],
			default: "bar",
		},
		limit: { type: "integer", min: 1, max: 20, default: 5 },
		ratio: { type: "number" },
		visible: { type: "boolean", default: false },
		rows: {
			type: "json",
			default: [],
			schema: {
				type: "array",
				items: {
					type: "object",
					required: ["label", "value"],
					properties: {
						label: { type: "string" },
						value: { type: "number" },
					},
				},
			},
		},
		note: { type: "string", optional: true },
	},
	events: {},
	queries: {},
	sizing: { defaultHeight: 320, resizable: true },
};

describe("widget props form draft", () => {
	it("serializes defaults and leaves missing values unset", () => {
		expect(createWidgetPropsDraft(CONTRACT)).toEqual({
			title: "Revenue",
			variant: "bar",
			limit: "5",
			ratio: undefined,
			visible: false,
			rows: "[]",
			note: undefined,
		});
	});

	it("parses every control back to its declared runtime type", () => {
		const result = parseWidgetPropsDraft(CONTRACT, {
			title: "Q3 revenue",
			variant: "line",
			limit: "12",
			ratio: "1.25",
			visible: true,
			rows: '[{"label":"Jul","value":4200}]',
			note: undefined,
		});

		expect(result).toEqual({
			valid: true,
			errors: {},
			props: {
				title: "Q3 revenue",
				variant: "line",
				limit: 12,
				ratio: 1.25,
				visible: true,
				rows: [{ label: "Jul", value: 4200 }],
			},
		});
	});

	it("reports type, bounds, enum, and JSON syntax errors per field", () => {
		const result = parseWidgetPropsDraft(CONTRACT, {
			...createWidgetPropsDraft(CONTRACT),
			variant: "pie",
			limit: "20.5",
			ratio: "not-a-number",
			rows: "[{",
		});

		expect(result.valid).toBe(false);
		expect(result.errors.variant?.[0]).toContain("not one of");
		expect(result.errors.limit?.[0]).toContain("expected integer");
		expect(result.errors.ratio?.[0]).toContain("expected number");
		expect(result.errors.rows?.[0]).toContain("Invalid JSON");
	});

	it("validates structured values against their JSON Schema", () => {
		const result = parseWidgetPropsDraft(CONTRACT, {
			...createWidgetPropsDraft(CONTRACT),
			rows: '[{"label":"Jul","value":"4200"}]',
		});

		expect(result.valid).toBe(false);
		expect(result.errors.rows).toContain(
			"$[0].value: expected type number, got string",
		);
	});

	it("omits unset optional props but rejects a blank required number", () => {
		const draft = createWidgetPropsDraft(CONTRACT);
		draft.ratio = "";

		const result = parseWidgetPropsDraft(CONTRACT, draft);

		expect(result.valid).toBe(false);
		expect(result.errors.ratio).toEqual(["$: value is required"]);
		expect(result.errors.note).toBeUndefined();
		expect(result.props).not.toHaveProperty("note");
	});

	it("distinguishes an omitted optional value from an included blank draft", () => {
		const contract: WidgetContract = {
			...CONTRACT,
			inputs: {
				threshold: { type: "number", optional: true },
				metadata: { type: "json", optional: true },
			},
		};

		expect(parseWidgetPropsDraft(contract, {})).toEqual({
			valid: true,
			errors: {},
			props: {},
		});
		const included = parseWidgetPropsDraft(contract, {
			threshold: "",
			metadata: "",
		});
		expect(included.valid).toBe(false);
		expect(included.errors).toEqual({
			threshold: ["$: value is required"],
			metadata: ["$: value is required"],
		});
	});

	it("preserves an explicit null JSON value over a declared default", () => {
		const contract: WidgetContract = {
			...CONTRACT,
			inputs: {
				data: { type: "json", default: { fallback: true } },
			},
		};

		expect(createWidgetPropsDraft(contract, { data: null })).toEqual({
			data: "null",
		});
	});

	it("creates sensible values when an optional field is enabled", () => {
		expect(emptyWidgetPropDraft({ type: "boolean" })).toBe(false);
		expect(emptyWidgetPropDraft({ type: "enum", choices: ["a", "b"] })).toBe(
			"a",
		);
		expect(emptyWidgetPropDraft({ type: "integer", min: 2.2, max: 8 })).toBe(
			"3",
		);
		expect(emptyWidgetPropDraft({ type: "integer", min: 2.2, max: 2.8 })).toBe(
			"",
		);
		expect(
			emptyWidgetPropDraft({ type: "string", default: "declared default" }),
		).toBe("declared default");
		expect(
			emptyWidgetPropDraft({
				type: "json",
				schema: { type: "object" },
			}),
		).toBe("{}");
		expect(
			emptyWidgetPropDraft({
				type: "json",
				schema: { const: { mode: "fixed" } },
			}),
		).toBe('{\n  "mode": "fixed"\n}');
	});
});
