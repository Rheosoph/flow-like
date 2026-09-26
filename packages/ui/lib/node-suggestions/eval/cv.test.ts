import { describe, expect, test } from "bun:test";
import {
	type GateItem,
	addRank,
	crossFoldThreshold,
	emptyTally,
	foldOf,
	gatePoint,
	rankMetrics,
	rankOf,
	thresholdForPrecision,
} from "./cv";

const item = (confidence: number, correct: boolean): GateItem => ({
	confidence,
	correct,
});

describe("foldOf", () => {
	test("matches int(md5(app)) % folds of the Python reference", () => {
		expect(foldOf("a", 5)).toBe(2);
		expect(foldOf("flow-like", 5)).toBe(1);
		expect(foldOf("app-1", 5)).toBe(2);
		expect(foldOf("zz", 5)).toBe(0);
		expect(foldOf("zz", 3)).toBe(2);
	});
});

describe("ranking metrics", () => {
	test("rankOf is 1-based and 0 for a miss", () => {
		expect(rankOf(["a", "b", "c"], "a")).toBe(1);
		expect(rankOf(["a", "b", "c"], "c")).toBe(3);
		expect(rankOf(["a", "b", "c"], "x")).toBe(0);
	});

	test("hit@k and MRR@10 over mixed ranks", () => {
		const tally = emptyTally();
		for (const rank of [1, 2, 4, 11, 0]) addRank(tally, rank);
		const metrics = rankMetrics(tally);
		expect(metrics.n).toBe(5);
		expect(metrics.hit1).toBeCloseTo(1 / 5);
		expect(metrics.hit3).toBeCloseTo(2 / 5);
		expect(metrics.hit5).toBeCloseTo(3 / 5);
		expect(metrics.mrr10).toBeCloseTo((1 + 1 / 2 + 1 / 4) / 5);
	});

	test("an empty tally reports zeros", () => {
		expect(rankMetrics(emptyTally())).toEqual({
			n: 0,
			hit1: 0,
			hit3: 0,
			hit5: 0,
			mrr10: 0,
		});
	});
});

describe("gate", () => {
	const items = [
		item(0.9, true),
		item(0.8, true),
		item(0.6, false),
		item(0.6, true),
		item(0.3, false),
		item(0, false),
	];

	test("gatePoint shows ghosts whose confidence reaches the threshold", () => {
		expect(gatePoint(items, 0.6)).toEqual({
			threshold: 0.6,
			coverage: 4 / 6,
			precision: 3 / 4,
			shown: 4,
		});
		expect(gatePoint(items, 0.95)).toEqual({
			threshold: 0.95,
			coverage: 0,
			precision: 0,
			shown: 0,
		});
	});

	test("thresholdForPrecision takes the largest coverage and cuts ties together", () => {
		const at75 = thresholdForPrecision(items, 0.75);
		expect(at75?.threshold).toBe(0.6);
		expect(at75?.shown).toBe(4);
		expect(at75?.coverage).toBeCloseTo(4 / 6);
		// 3 of 5 ≥ 0.6, but the pin without a ghost never counts.
		expect(thresholdForPrecision(items, 0.6)?.threshold).toBe(0.3);
		expect(thresholdForPrecision([item(0.5, false)], 0.5)).toBeUndefined();
	});

	test("crossFoldThreshold applies each fold the threshold of the others", () => {
		const byFold = [
			[item(0.9, true), item(0.4, false)],
			[item(0.8, true), item(0.5, true), item(0.2, false)],
		];
		const gate = crossFoldThreshold(byFold, 1);
		// Fold 0 gets 0.5 from fold 1 (shows 0.9), fold 1 gets 0.9 from fold 0 (shows nothing).
		expect(gate?.thresholds).toEqual([0.5, 0.9]);
		expect(gate?.coverage).toBeCloseTo(1 / 5);
		expect(gate?.precision).toBe(1);
		expect(gate?.precisionRange).toEqual([1, 1]);
	});
});
