import { describe, expect, test } from "bun:test";
import { ResponsiveBar } from "@nivo/bar";
import { isRenderableChartExport } from "./chart-nivo-preview";

describe("Nivo chart module loading", () => {
	test("accepts current forward-ref Nivo component exports", () => {
		expect(typeof ResponsiveBar).toBe("object");
		expect(isRenderableChartExport(ResponsiveBar)).toBe(true);
	});

	test("rejects unrelated module values", () => {
		expect(isRenderableChartExport(null)).toBe(false);
		expect(isRenderableChartExport({ render: () => null })).toBe(false);
		expect(isRenderableChartExport("ResponsiveBar")).toBe(false);
	});
});
