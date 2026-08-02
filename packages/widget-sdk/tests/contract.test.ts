import { describe, expect, test } from "bun:test";
import {
	CONTRACT_VERSION,
	type WidgetContract,
	contractDefaults,
} from "../src/contract";

const contract: WidgetContract = {
	contractVersion: CONTRACT_VERSION,
	id: "sales-chart",
	inputs: {
		title: { type: "string", default: "Sales", description: "Chart headline" },
		variant: { type: "enum", choices: ["bar", "line"], default: "bar" },
		limit: { type: "number", min: 1, max: 500, default: 50 },
		rows: { type: "json", schema: { type: "array" } },
		note: { type: "string", optional: true },
	},
	events: {
		pointSelected: { payloadSchema: { type: "object" } },
		refreshRequested: { payloadSchema: null },
	},
	queries: {
		getValue: { argsSchema: null, resultSchema: { type: "string" } },
	},
	sizing: { defaultHeight: 320, resizable: true },
};

describe("contractDefaults", () => {
	test("collects every declared default", () => {
		expect(contractDefaults(contract)).toEqual({
			title: "Sales",
			variant: "bar",
			limit: 50,
		});
	});

	test("skips inputs without a default", () => {
		const defaults = contractDefaults(contract);
		expect("rows" in defaults).toBe(false);
		expect("note" in defaults).toBe(false);
	});

	test("keeps falsy defaults", () => {
		const defaults = contractDefaults({
			contractVersion: 1,
			id: "w",
			inputs: {
				flag: { type: "boolean", default: false },
				count: { type: "integer", default: 0 },
				label: { type: "string", default: "" },
				data: { type: "json", default: null },
			},
		});
		expect(defaults).toEqual({ flag: false, count: 0, label: "", data: null });
	});

	test("handles missing contract or inputs", () => {
		expect(contractDefaults(undefined)).toEqual({});
		expect(contractDefaults(null)).toEqual({});
		expect(contractDefaults({ contractVersion: 1, id: "w" })).toEqual({});
	});
});

describe("contract JSON shape", () => {
	test("serializes to the camelCase shape shared with Rust", () => {
		const json = JSON.parse(JSON.stringify(contract));
		expect(json.contractVersion).toBe(1);
		expect(json.inputs.title).toEqual({
			type: "string",
			default: "Sales",
			description: "Chart headline",
		});
		expect(json.inputs.variant.choices).toEqual(["bar", "line"]);
		expect(json.inputs.limit.min).toBe(1);
		expect(json.inputs.limit.max).toBe(500);
		expect(json.events.pointSelected.payloadSchema).toEqual({
			type: "object",
		});
		expect(json.events.refreshRequested.payloadSchema).toBeNull();
		expect(json.queries.getValue.resultSchema).toEqual({ type: "string" });
		expect(json.sizing.defaultHeight).toBe(320);
		expect(json.sizing.resizable).toBe(true);
	});
});
