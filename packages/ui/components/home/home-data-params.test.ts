import { describe, expect, test } from "bun:test";
import type { SqlParameterInfo } from "../../state/backend-state/query-state";
import {
	applyParamOverrides,
	describeBinding,
	groupParamEntries,
	inferParamControl,
	interactiveParamNames,
	optionRange,
	optionValues,
	parseNumberInput,
	toDateInput,
	toDateTimeInput,
	withParamControl,
} from "./home-data-params";
import { normalizeHomeDataConfig } from "./home-data-query";

const param = (
	name: string,
	data_type: string | null,
	bindings: SqlParameterInfo["bindings"] = [],
): SqlParameterInfo => ({ name, data_type, bindings });
const on = (
	operator: SqlParameterInfo["bindings"][number]["operator"],
	column = "created",
) => ({ table: "crm.orders", column, operator });

describe("inferParamControl", () => {
	test("maps Arrow types to controls", () => {
		expect(inferParamControl(param("a", "Boolean"))).toBe("toggle");
		expect(inferParamControl(param("a", "Date32"))).toBe("date");
		expect(inferParamControl(param("a", "Date64"))).toBe("date");
		expect(inferParamControl(param("a", "Timestamp(µs)"))).toBe("datetime");
		expect(inferParamControl(param("a", "Int64"))).toBe("number");
		expect(inferParamControl(param("a", "UInt8"))).toBe("number");
		expect(inferParamControl(param("a", "Float64"))).toBe("number");
		expect(inferParamControl(param("a", "Decimal128(10, 2)"))).toBe("number");
		expect(inferParamControl(param("a", "List(Utf8)"))).toBe("multi-select");
	});
	test("uses the binding for strings", () => {
		expect(inferParamControl(param("a", "Utf8", [on("in", "region")]))).toBe(
			"multi-select",
		);
		expect(inferParamControl(param("a", "Utf8", [on("eq", "region")]))).toBe(
			"select",
		);
		expect(
			inferParamControl(param("a", "Utf8", [on("neq", "region")]), {
				truncated: true,
			}),
		).toBe("combobox");
		expect(inferParamControl(param("a", "Utf8", [on("ilike", "name")]))).toBe(
			"text",
		);
		expect(inferParamControl(param("a", null))).toBe("text");
	});
});

describe("groupParamEntries", () => {
	test("pairs gte/lte parameters on one column", () => {
		const entries = groupParamEntries([
			param("from", "Date32", [on("gte")]),
			param("region", "Utf8", [on("eq", "region")]),
			param("to", "Date32", [on("lte")]),
		]);
		expect(entries).toHaveLength(2);
		expect(entries[0]).toMatchObject({ type: "range", kind: "date" });
		expect(entries[1]).toMatchObject({ type: "single" });
	});
	test("pairs gt/lt numbers and leaves different columns alone", () => {
		expect(
			groupParamEntries([
				param("lo", "Int64", [on("gt", "qty")]),
				param("hi", "Int64", [on("lt", "qty")]),
			])[0],
		).toMatchObject({ type: "range", kind: "number" });
		expect(
			groupParamEntries([
				param("lo", "Int64", [on("gte", "qty")]),
				param("hi", "Int64", [on("lte", "price")]),
			]),
		).toHaveLength(2);
	});
});

describe("helpers", () => {
	test("describes bindings and parses inputs", () => {
		expect(describeBinding(param("a", "Utf8", [on("eq", "region")]))).toBe(
			"crm.orders.region",
		);
		expect(describeBinding(param("a", "Utf8"))).toBeNull();
		expect(parseNumberInput("")).toBeUndefined();
		expect(parseNumberInput("4.5")).toBe(4.5);
		expect(parseNumberInput("x")).toBeUndefined();
		expect(toDateInput("2026-01-05T10:00:00Z")).toBe("2026-01-05");
		expect(toDateInput(5)).toBe("");
		expect(toDateTimeInput("2026-01-05T10:00:00Z")).toBe("2026-01-05T10:00");
	});
	test("reads option shapes", () => {
		expect(optionValues({ values: ["a"], truncated: true })).toEqual({
			values: ["a"],
			truncated: true,
		});
		expect(optionValues({ min: 1, max: 2 }).values).toEqual([]);
		expect(optionRange({ min: 1, max: 2 })).toEqual({ min: 1, max: 2 });
		expect(optionRange(null)).toBeNull();
	});
	test("lists interactive parameters of SQL widgets only", () => {
		const config = normalizeHomeDataConfig({
			sourceKind: "sql",
			paramControls: { a: { interactive: true }, b: { label: "B" } },
		});
		expect(interactiveParamNames(config)).toEqual(["a"]);
		expect(interactiveParamNames({ ...config, sourceKind: "table" })).toEqual(
			[],
		);
	});
});

describe("applyParamOverrides", () => {
	const config = normalizeHomeDataConfig({
		sourceKind: "sql",
		queryParams: { region: "EU", year: 2025 },
		paramControls: { region: { interactive: true } },
	});
	test("merges only interactive parameters over the defaults", () => {
		const merged = applyParamOverrides(config, { region: "US", year: 1999 });
		expect(merged.queryParams).toEqual({ region: "US", year: 2025 });
		expect(config.queryParams.region).toBe("EU");
	});
	test("returns the same config without usable overrides", () => {
		expect(applyParamOverrides(config, undefined)).toBe(config);
		expect(applyParamOverrides(config, { year: 1 })).toBe(config);
		expect(applyParamOverrides(config, { region: undefined })).toBe(config);
	});
});

describe("withParamControl", () => {
	test("merges changes and drops empty entries", () => {
		const one = withParamControl({}, "a", { interactive: true, label: "A" });
		expect(one).toEqual({ a: { interactive: true, label: "A" } });
		const two = withParamControl(one, "a", { interactive: false, label: "" });
		expect(two).toEqual({});
		expect(
			withParamControl({ b: { control: "date" } }, "a", { interactive: true }),
		).toEqual({ b: { control: "date" }, a: { interactive: true } });
	});
});
