import { describe, expect, test } from "bun:test";
import { SAMPLE_NOW } from "../../../../../lib/device-management/model/__fixtures__/sample-fleet";
import {
	SAMPLE_MODELS_HOUR,
	SAMPLE_MODEL_GRANT,
	gpuBoxModels,
	statsOf,
} from "../../../../../lib/device-management/model/__fixtures__/sample-models";
import {
	MODEL_STATS_MAX_POINTS,
	type ModelStatsInput,
} from "../../../../../lib/device-management/models";
import {
	RANGE_SPEC,
	bucketHoursOf,
	bucketed,
	consumerRows,
	rangeWindow,
	reported,
	speedSeries,
	statsView,
} from "./stats-view";

const HOUR = 3_600;
const sum = (values: readonly number[]) =>
	values.reduce((total, value) => total + value, 0);

const answer = (input: ModelStatsInput) =>
	statsOf(gpuBoxModels(), {
		modelId: input.modelId ?? null,
		from: input.from,
		to: input.to,
		step: input.step,
	});

describe("ranges", () => {
	test("each range asks for whole hours up to the end of the current one", () => {
		for (const [range, points] of [
			["day", 24],
			["week", 168],
			["quarter", MODEL_STATS_MAX_POINTS],
		] as const) {
			const window = rangeWindow(range, SAMPLE_NOW);
			expect(window.to).toBe(SAMPLE_MODELS_HOUR + HOUR);
			expect((window.to - window.from) / HOUR).toBe(points);
			expect(window.step).toBe("hour");
			expect(window.modelId).toBeUndefined();
		}
		expect(rangeWindow("day", SAMPLE_NOW, "qwen3-8b").modelId).toBe("qwen3-8b");
	});

	test("the window only moves when the hour changes", () => {
		const a = rangeWindow("week", SAMPLE_MODELS_HOUR);
		const b = rangeWindow("week", SAMPLE_MODELS_HOUR + HOUR - 1);
		const c = rangeWindow("week", SAMPLE_MODELS_HOUR + HOUR);
		expect(b).toEqual(a);
		expect(c.from - a.from).toBe(HOUR);
	});

	test("columns sum one, six or twenty-four hours", () => {
		expect(bucketHoursOf(RANGE_SPEC.day.hours)).toBe(1);
		expect(bucketHoursOf(RANGE_SPEC.week.hours)).toBe(6);
		expect(bucketHoursOf(RANGE_SPEC.quarter.hours)).toBe(24);
		expect(bucketed([1, 2, 3, 4, 5, 6, 7], 3)).toEqual([6, 15, 7]);
	});
});

describe("the statistics view", () => {
	test("a day of Qwen3-8B: totals, hourly columns, timings and the slowest hour", () => {
		const stats = answer(rangeWindow("day", SAMPLE_NOW, "qwen3-8b"));
		const view = statsView(stats);
		expect(view.from).toBe(SAMPLE_MODELS_HOUR - 23 * HOUR);
		expect(view.to).toBe(SAMPLE_MODELS_HOUR + HOUR);
		expect(view.totals).toMatchObject({
			requests: 1_088,
			errors: 6,
			tokensIn: 1_088 * 620,
			tokensOut: 1_088 * 390,
			ttftP95Max: 1_900,
			queueP95Max: 1_800,
		});
		expect(view.totals.tokensPerSecond).toBeCloseTo(86, 0);
		expect(view.columns.step).toBe(HOUR);
		expect(view.columns.answered).toHaveLength(24);
		expect(sum(view.columns.answered) + sum(view.columns.failed)).toBe(1_088);
		expect(view.hourly.ttftP50.every((value) => value === 190)).toBe(true);
		for (const speed of view.hourly.tokensPerSecond)
			expect(speed).toBeCloseTo(86, 0);
	});

	test("a week sums six-hour columns and keeps timings hourly, with gaps where nothing was reported", () => {
		const view = statsView(answer(rangeWindow("week", SAMPLE_NOW)));
		expect(view.hours).toBe(168);
		expect(view.columns.step).toBe(6 * HOUR);
		expect(view.columns.tokensOut).toHaveLength(28);
		expect(view.hourly.ttftP95).toHaveLength(168);
		expect(view.hourly.ttftP95[0]).toBeNull();
		expect(sum(view.columns.answered) + sum(view.columns.failed)).toBe(
			view.totals.requests,
		);
		expect(sum(view.columns.tokensIn)).toBe(view.totals.tokensIn);
	});

	test("ninety days are ninety daily columns", () => {
		const view = statsView(answer(rangeWindow("quarter", SAMPLE_NOW)));
		expect(view.hours).toBe(2_160);
		expect(view.columns.answered).toHaveLength(90);
		expect(view.columns.step).toBe(24 * HOUR);
	});

	test("an embedding model generates nothing: no speed, no speed line", () => {
		const view = statsView(
			answer(rangeWindow("day", SAMPLE_NOW, "nomic-embed-v1.5")),
		);
		expect(view.totals.tokensOut).toBe(0);
		expect(view.totals.tokensPerSecond).toBeUndefined();
		expect(reported(view.hourly.tokensPerSecond)).toBe(false);
		expect(reported(view.hourly.ttftP95)).toBe(true);
	});

	test("speed is generated tokens per second of decode time, nothing for hours without decode time", () => {
		const series = answer(rangeWindow("day", SAMPLE_NOW, "qwen3-8b")).series;
		const speeds = speedSeries({
			...series,
			completion_tokens: [100, 0, 50],
			decode_ms: [1_000, 0, 0],
		});
		expect(speeds).toEqual([100, null, null]);
	});
});

/** Mira holds the sample grant. */
function miraOf(grantId: string) {
	return grantId === SAMPLE_MODEL_GRANT ? "user-mira" : undefined;
}

function nobody() {
	return undefined;
}

describe("callers", () => {
	const consumers = answer(
		rangeWindow("day", SAMPLE_NOW, "qwen3-8b"),
	).consumers;

	test("most tokens first, named for the viewer", () => {
		const rows = consumerRows(consumers, {
			owner: true,
			grantUser: miraOf,
		});
		expect(rows.map((row) => row.consumer.kind)).toEqual([
			"you",
			"service",
			"person",
		]);
		expect(rows[1]?.consumer).toMatchObject({ serviceId: "invoice-extractor" });
		expect(rows[2]?.consumer).toMatchObject({ userId: "user-mira" });
		expect(rows[0]?.errors).toBe(6);
		expect(rows[0]?.tokensIn).toBeGreaterThan(rows[1]?.tokensIn ?? 0);
	});

	test("seen from the grant holder, their own grant is the viewer and the owner is the owner", () => {
		const rows = consumerRows(consumers, {
			owner: false,
			myGrantId: SAMPLE_MODEL_GRANT,
			grantUser: nobody,
		});
		expect(rows.map((row) => row.consumer.kind)).toEqual([
			"owner",
			"service",
			"you",
		]);
	});
});
