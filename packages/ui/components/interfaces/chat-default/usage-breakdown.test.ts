import { describe, expect, test } from "bun:test";
import type { IChatUsageStat } from "../../../lib/llm/usage";
import { formatTokens, usageBreakdown } from "./usage-breakdown";

const usage = (prompt: number, completion: number, extra = {}) => ({
	prompt_tokens: prompt,
	completion_tokens: completion,
	total_tokens: prompt + completion,
	...extra,
});

const stats: IChatUsageStat[] = [
	{
		step_name: "Assistant",
		stats: {
			usage: usage(3_000, 300, { cost: 0.03 }),
			duration_ms: 4_000,
			iterations: 2,
			calls_omitted: 1,
			calls: [
				{
					model: "summary-model",
					purpose: "context_compaction",
					usage: usage(1_000, 100, { cost: 0.01 }),
					duration_ms: 1_500,
				},
				{
					model: "main-model",
					usage: usage(2_000, 200, {
						cost: 0.02,
						prompt_tokens_details: { cached_tokens: 1_500 },
					}),
					duration_ms: 2_500,
				},
			],
		},
	},
	{
		step_name: "Screenshot",
		stats: { usage: usage(400, 40), model: "vision-model", duration_ms: 900 },
	},
];

describe("usageBreakdown", () => {
	const breakdown = usageBreakdown(stats);

	test("totals come from the step usage, call count includes omitted calls", () => {
		expect(breakdown.totalTokens).toBe(3_740);
		expect(breakdown.cost).toBeCloseTo(0.03);
		expect(breakdown.durationMs).toBe(4_900);
		expect(breakdown.callCount).toBe(4);
	});

	test("ranks models by tokens and keeps unreported cost as null", () => {
		expect(breakdown.models.map((row) => row.model)).toEqual([
			"main-model",
			"summary-model",
			"vision-model",
		]);
		expect(breakdown.models[2].cost).toBeNull();
		expect(breakdown.models[0].share).toBeCloseTo(2_200 / 3_740);
	});

	test("a step without calls becomes one call of its reported model", () => {
		expect(breakdown.steps[1].calls).toHaveLength(1);
		expect(breakdown.steps[1].calls[0]).toMatchObject({
			model: "vision-model",
			durationMs: 900,
			cachedTokens: null,
		});
	});

	test("isolates context summary cost and models that left cache reads unreported", () => {
		expect(breakdown.summary).toEqual({ totalTokens: 1_100, cost: 0.01 });
		expect(breakdown.cacheUnreported).toEqual({
			models: ["summary-model", "vision-model"],
			promptTokens: 1_400,
		});
	});
});

describe("formatTokens", () => {
	test("drops trailing zeros and rounds large counts", () => {
		expect(formatTokens(950)).toBe("950");
		expect(formatTokens(9_000)).toBe("9k");
		expect(formatTokens(18_420)).toBe("18.4k");
		expect(formatTokens(184_786)).toBe("185k");
		expect(formatTokens(2_500_000)).toBe("2.5M");
	});
});
