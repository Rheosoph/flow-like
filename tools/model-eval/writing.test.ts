import { describe, expect, test } from "bun:test";
import type { AAModel } from "./types";
import {
	WRITING_BENCHMARK_READMES,
	matchCreativityBenchmark,
	parseWritingBenchmarkTable,
	parseWritingBenchmarks,
	resolveCreativityBenchmarks,
} from "./writing";

const source = WRITING_BENCHMARK_READMES[0];
const leaderboard = `### Leaderboard

| Rank | Model | Comparison score | Estimated win chance | 95% bootstrap interval |
| ---: | --- | ---: | ---: | --- |
| 8 | **Kimi K3.1 Pro (high)** | 2.723 | 85% | 2.634 to 2.799 |
| 20 | Gemini 4 Argon (high) | 0.719 | 61% | 0.603 to 0.825 |
| 60 | Test Low† | -5.131 | 0% | -5.209 to -5.044 |

### Coverage Note

| 1 | Not a result | 10 | 100% | 10 to 10 |
`;

function model(slug: string, name: string): AAModel {
	return {
		slug,
		name,
		model_creator: { id: "kimi", slug: "kimi", name: "Kimi" },
		evaluations: {
			artificial_analysis_intelligence_index: 55,
			gpqa: 0.9,
		},
		median_output_tokens_per_second: 50,
	} as AAModel;
}

describe("writing benchmark formats", () => {
	test("keeps pairwise comparison scores separate from win probabilities and legacy means", () => {
		const rows = parseWritingBenchmarkTable(
			leaderboard,
			source.heading,
			source.version,
			source.url,
		);
		expect(rows).toHaveLength(3);
		expect(rows[0]).toEqual({
			modelName: "Kimi K3.1 Pro (high)",
			comparisonScore: 2.723,
			estimatedWinChance: 0.85,
			meanScore: null,
			normalizedScore: 0.85,
			version: "pairwise",
			sourceUrl: source.url,
		});
		expect(rows[2]).toMatchObject({
			modelName: "Test Low",
			comparisonScore: -5.131,
			estimatedWinChance: 0,
			normalizedScore: 0,
		});
	});

	test("rejects invalid probabilities and missing comparison scores", () => {
		const malformed = `### Leaderboard
| Rank | Model | Comparison score | Estimated win chance |
| --- | --- | --- | --- |
| 1 | Missing | | 50% |
| 2 | Out of range | 1 | 101% |
| 3 | Missing unit | 1 | 0.8 |
| 4 | Invalid score | unknown | 50% |
`;
		expect(
			parseWritingBenchmarkTable(
				malformed,
				source.heading,
				source.version,
				source.url,
			),
		).toEqual([]);
	});

	test("retains archived mean scores without interpreting them as comparisons", () => {
		const archived = `### Overall LLM Means
| Rank | Model | Mean |
| --- | --- | --- |
| 1 | Archived model | 7.5 |
`;
		expect(
			parseWritingBenchmarkTable(
				archived,
				"### Overall LLM Means",
				"v2",
				"archive",
			),
		).toEqual([
			{
				modelName: "Archived model",
				meanScore: 7.5,
				comparisonScore: null,
				estimatedWinChance: null,
				normalizedScore: 0.75,
				version: "v2",
				sourceUrl: "archive",
			},
		]);
	});
});

describe("writing benchmark attribution", () => {
	test("does not assign a preview benchmark to a later stable release", () => {
		const preview = leaderboard.replace(
			"Kimi K3.1 Pro (high)",
			"DeepSeek V4 Pro Preview",
		);
		expect(
			matchCreativityBenchmark(
				model("deepseek-v4-pro", "DeepSeek V4 Pro 0813 (Max)"),
				parseWritingBenchmarks([preview]),
			),
		).toBeNull();
	});

	test("matches Kimi and Gemini while retaining the measured high configuration", () => {
		const benchmarks = parseWritingBenchmarks([leaderboard]);
		const kimi = model("kimi-k3-1-pro", "Kimi K3.1 Pro");
		const gemini = model("gemini-4-argon", "Gemini 4 Argon (High)");
		const resolved = resolveCreativityBenchmarks([kimi, gemini], benchmarks);
		expect(resolved.get(kimi.slug)).toMatchObject({
			matchType: "direct",
			sourceModelName: "Kimi K3.1 Pro (high)",
			sourceModelSlug: kimi.slug,
			creativityEstimate: null,
			estimatedWinChance: 0.85,
		});
		expect(resolved.get(gemini.slug)).toMatchObject({
			matchType: "direct",
			sourceModelName: "Gemini 4 Argon (high)",
			estimatedWinChance: 0.61,
		});
		expect(
			matchCreativityBenchmark(
				model("gemini-4-argon-low", "Gemini 4 Argon (Low)"),
				benchmarks,
			),
		).toBeNull();
	});

	test("labels similarity estimates and never reports them as measured benchmark values", () => {
		const donor = model("kimi-k3-1-pro", "Kimi K3.1 Pro");
		const target = model("unmeasured-model", "Unmeasured model");
		const resolved = resolveCreativityBenchmarks(
			[donor, target],
			parseWritingBenchmarks([leaderboard]),
		).get(target.slug);
		expect(resolved).toMatchObject({
			matchType: "similarity",
			sourceModelName: "Kimi K3.1 Pro (high)",
			sourceModelSlug: donor.slug,
			meanScore: null,
			comparisonScore: null,
			estimatedWinChance: null,
			similarityTier: "same_creator",
			sourceUrl: source.url,
		});
		expect(resolved?.creativityEstimate).toBeCloseTo(0.833, 3);
		expect(resolved?.normalizedScore).toBe(
			resolved?.creativityEstimate ?? undefined,
		);
	});
});
