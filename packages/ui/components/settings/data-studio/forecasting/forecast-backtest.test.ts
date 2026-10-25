import { describe, expect, it } from "bun:test";
import type { StudioHistoryPoint } from "../../../../state/backend-state/query-state";
import { forecastBacktestRange } from "./forecast-backtest";

const series = (id: string, count: number, start = 0): StudioHistoryPoint[] =>
	Array.from({ length: count }, (_, i) => ({
		series_id: id,
		target: "sales",
		timestamp_ms: start + i * 1000,
		value: i === 3 ? null : i,
	}));

describe("historical forecast cutoff", () => {
	it("maps a step offset to the exact held-out window without mutating history", () => {
		const history = series("a", 10).reverse();
		const before = structuredClone(history);
		expect(forecastBacktestRange(history, "sales", "a", 3, 2)).toMatchObject({
			valid: true,
			available: true,
			maxOffset: 5,
			totalCount: 10,
			contextCount: 5,
			maxHorizon: 8,
			cutoff: 4000,
			start: 5000,
			end: 7000,
			oldest: 1000,
			latest: 6000,
		});
		expect(forecastBacktestRange(history, "sales", "a", 3, 0)).toMatchObject({
			cutoff: 6000,
			start: 7000,
			end: 9000,
		});
		expect(history).toEqual(before);
	});
	it("limits the slider by the shortest series and keeps missing observations on the clock", () => {
		const history = [
			...series("a", 10),
			...series("b", 6, 10000),
			...series("c", 2).map((p) => ({ ...p, target: "power" })),
		];
		expect(forecastBacktestRange(history, "sales", "a", 3, 1)).toMatchObject({
			valid: true,
			seriesCount: 2,
			maxOffset: 1,
			totalCount: 10,
			contextCount: 6,
			maxHorizon: 4,
			cutoff: 5000,
		});
		expect(forecastBacktestRange(history, "sales", "b", 3, 1)).toMatchObject({
			valid: true,
			cutoff: 11000,
			start: 12000,
			end: 14000,
		});
		expect(forecastBacktestRange(history, "sales", "a", 3, 2).valid).toBe(
			false,
		);
	});
	it("rejects cutoffs without a full window and at least two prior steps", () => {
		for (const [horizon, offset] of [
			[0, 0],
			[9, 0],
			[2, -1],
			[2, 1.2],
			[2, Number.POSITIVE_INFINITY],
		])
			expect(
				forecastBacktestRange(series("a", 10), "sales", "a", horizon, offset)
					.valid,
			).toBe(false);
		expect(forecastBacktestRange([], "sales", "a", 2, 0).available).toBe(false);
		expect(
			forecastBacktestRange(series("a", 10), "sales", "missing", 2, 0)
				.available,
		).toBe(false);
	});
});
