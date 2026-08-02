import { describe, expect, test } from "bun:test";
import {
	createContractInputValue,
	updateWidgetContractProps,
} from "./widget-contract-form";

describe("widget contract builder helpers", () => {
	test("creates typed values for newly included inputs", () => {
		expect(createContractInputValue({ type: "boolean" })).toBe(false);
		expect(
			createContractInputValue({ type: "integer", min: 2.2, max: 8 }),
		).toBe(3);
		expect(
			createContractInputValue({ type: "enum", choices: ["", "active"] }),
		).toBe("");
		expect(
			createContractInputValue({
				type: "json",
				schema: {
					type: "array",
					items: { type: "string" },
				},
			}),
		).toEqual([]);
	});

	test("clones declared structured defaults", () => {
		const input = { type: "json" as const, default: [{ label: "Jan" }] };
		const value = createContractInputValue(input) as Array<{
			label: string;
		}>;
		const first = value[0];
		if (!first) throw new Error("Expected a cloned default item");
		first.label = "Feb";
		expect(input.default).toEqual([{ label: "Jan" }]);
	});

	test("merges values and deletes omitted optional keys", () => {
		const current = { title: "Revenue", note: "temporary" };
		expect(updateWidgetContractProps(current, "title", "Forecast")).toEqual({
			title: "Forecast",
			note: "temporary",
		});
		expect(updateWidgetContractProps(current, "note", undefined)).toEqual({
			title: "Revenue",
		});
		expect(current).toEqual({ title: "Revenue", note: "temporary" });
	});
});
