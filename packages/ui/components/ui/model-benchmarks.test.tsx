import { describe, expect, test } from "bun:test";
import { createI18n } from "@flow-like/locales";
import { renderToStaticMarkup } from "react-dom/server";
import {
	type IModelEvaluation,
	IntelligenceIndexBadge,
	ModelBenchmarks,
} from "./model-benchmarks";

createI18n({ language: "en" });

function evaluation(
	evaluations: IModelEvaluation["evaluations"],
): IModelEvaluation {
	return {
		slug: "kimi-k3-1-pro",
		name: "Kimi K3.1 Pro",
		creator_name: "Kimi",
		creator_slug: "kimi",
		evaluations,
	};
}

const estimate = {
	score: 23,
	low: 18,
	high: 28,
	confidence: "low" as const,
	reasoning: "Estimated from the model card and comparable models.",
	sources: ["https://huggingface.co/example/model"],
	updated_at: "2026-10-27",
};

const reportedBenchmark = {
	name: "OmniDocBench v1.6 Overall",
	score: 95.06,
	unit: "score",
	source: "https://huggingface.co/tencent/WeVisDoc-2B",
	scope: "publisher-reported",
	notes: "Original checkpoint results; local quantization was not benchmarked.",
	direction: "higher" as const,
};

describe("model benchmarks", () => {
	test("renders current coding, tool-use, and factuality benchmarks", () => {
		const markup = renderToStaticMarkup(
			<ModelBenchmarks
				evaluation={evaluation({
					terminalbench_v4_0: 0.6717,
					terminalbench_v2_1: 0.764,
					tau_banking: 0.23,
					automationbench_aa: 0.733,
					artificial_analysis_omniscience_accuracy: 0.51,
					artificial_analysis_omniscience_hallucination_rate: 0.439,
				})}
			/>,
		);
		for (const label of [
			"Terminal-Bench 4.0",
			"Terminal-Bench 2.1",
			"TAU-Banking",
			"AutomationBench-AA",
			"AA-Omniscience Accuracy",
			"AA-Omniscience Non-Hallucination Rate",
		]) {
			expect(markup).toContain(label);
		}
		expect(markup).toContain("67.2%");
		expect(markup).toContain("73.3%");
		expect(markup).toContain("56.1%");
		expect(markup).toContain(
			'href="https://artificialanalysis.ai/models/kimi-k3-1-pro"',
		);
	});

	test("shows measured zero results without presenting missing benchmarks", () => {
		const markup = renderToStaticMarkup(
			<ModelBenchmarks
				evaluation={evaluation({ terminalbench_v4_0: 0, scicode: null })}
			/>,
		);
		expect(markup).toContain("Terminal-Bench 4.0");
		expect(markup).toContain("0.0%");
		expect(markup).not.toContain("SciCode");
		expect(
			renderToStaticMarkup(
				<IntelligenceIndexBadge
					evaluation={evaluation({ artificial_analysis_intelligence_index: 0 })}
				/>,
			),
		).toContain("0.0");
	});

	test("attributes direct writing results to their own leaderboard", () => {
		for (const chance of [0, 0.61, 0.85]) {
			const markup = renderToStaticMarkup(
				<ModelBenchmarks
					evaluation={evaluation({
						writing_benchmark_estimated_win_chance: chance,
						writing_benchmark_match_type: "direct",
						writing_benchmark_version: "pairwise",
						writing_benchmark_source_model_name: "Kimi K3.1 Pro (high)",
						writing_benchmark_source_url:
							"https://github.com/lechmazur/writing",
					})}
				/>,
			);
			expect(markup).toContain("Creative writing win chance");
			expect(markup).toContain(`${(chance * 100).toFixed(1)}%`);
			expect(markup).toContain("tested writing leaderboard");
			expect(markup).toContain("Kimi K3.1 Pro (high)");
			expect(markup).toContain('href="https://github.com/lechmazur/writing"');
			expect(markup).not.toContain("artificialanalysis.ai");
		}
	});

	test("never presents inferred or aliased creativity as a measured benchmark", () => {
		for (const matchType of ["alias", "similarity"] as const) {
			expect(
				renderToStaticMarkup(
					<ModelBenchmarks
						evaluation={evaluation({
							writing_benchmark_creativity_estimate: 0.8,
							writing_benchmark_estimated_win_chance: 0.85,
							writing_benchmark_match_type: matchType,
						})}
					/>,
				),
			).toBe("");
		}
	});

	test("ignores unsafe writing source URLs", () => {
		const markup = renderToStaticMarkup(
			<ModelBenchmarks
				evaluation={evaluation({
					writing_benchmark_estimated_win_chance: 0.85,
					writing_benchmark_match_type: "direct",
					writing_benchmark_source_url: "javascript:alert(1)",
				})}
			/>,
		);
		expect(markup).toContain("85.0%");
		expect(markup).not.toContain("javascript:");
		expect(markup).not.toContain("Benchmark sources:");
	});

	test("omits missing and invalid results", () => {
		for (const value of [
			undefined,
			null,
			Number.NaN,
			Number.POSITIVE_INFINITY,
			-1,
		]) {
			expect(
				renderToStaticMarkup(
					<ModelBenchmarks
						evaluation={evaluation({ terminalbench_v4_0: value })}
					/>,
				),
			).toBe("");
			expect(
				renderToStaticMarkup(
					<IntelligenceIndexBadge
						evaluation={evaluation({
							artificial_analysis_intelligence_index: value,
						})}
					/>,
				),
			).toBe("");
		}
	});

	test("labels an intelligence estimate with its evidence and uncertainty", () => {
		const model = evaluation({ intelligence_estimate: estimate });
		const markup = renderToStaticMarkup(<ModelBenchmarks evaluation={model} />);
		expect(markup).toContain("Estimated intelligence");
		expect(markup).toContain("23.0");
		expect(markup).toContain("Range: 18.0 to 28.0");
		expect(markup).toContain("Low confidence");
		expect(markup).toContain(estimate.reasoning);
		expect(markup).toContain("Estimated 2026-10-27");
		expect(markup).toContain('href="https://huggingface.co/example/model"');
		expect(markup).toContain("Estimate sources:");
		expect(markup).not.toContain("Artificial Analysis");
		expect(markup).not.toContain("artificialanalysis.ai");

		const badge = renderToStaticMarkup(
			<IntelligenceIndexBadge evaluation={model} />,
		);
		expect(badge).toContain("Est. 23.0");
		expect(badge).toContain('aria-label="Estimated intelligence: 23.0"');
		expect(badge).not.toContain("Artificial Analysis");
	});

	test("prefers a measured index over estimates, including measured zero", () => {
		for (const score of [0, 45.1]) {
			const model = evaluation({
				artificial_analysis_intelligence_index: score,
				intelligence_estimate: estimate,
			});
			const markup = renderToStaticMarkup(
				<ModelBenchmarks evaluation={model} />,
			);
			const badge = renderToStaticMarkup(
				<IntelligenceIndexBadge evaluation={model} />,
			);
			for (const output of [markup, badge]) {
				expect(output).toContain(score.toFixed(1));
				expect(output).toContain("Artificial Analysis");
				expect(output).not.toContain("Estimated intelligence");
				expect(output).not.toContain(estimate.reasoning);
				expect(output).not.toContain("Est.");
			}
		}
	});

	test("shows valid zero estimates and falls back from invalid measured indices", () => {
		const model = evaluation({
			artificial_analysis_intelligence_index: Number.NaN,
			intelligence_estimate: {
				...estimate,
				score: 0,
				low: 0,
				high: 5,
				confidence: "medium",
			},
		});
		const markup = renderToStaticMarkup(<ModelBenchmarks evaluation={model} />);
		expect(markup).toContain("Range: 0.0 to 5.0");
		expect(markup).toContain("Medium confidence");
		expect(markup).not.toContain("Artificial Analysis");
		expect(
			renderToStaticMarkup(<IntelligenceIndexBadge evaluation={model} />),
		).toContain("Est. 0.0");
	});

	test("ignores malformed estimates and invalid or inverted ranges", () => {
		const invalidEstimates = [
			null,
			"23",
			{},
			{ ...estimate, score: -1 },
			{ ...estimate, score: Number.NaN },
			{ ...estimate, score: Number.POSITIVE_INFINITY },
			{ ...estimate, score: "23" },
			{ ...estimate, low: 24 },
			{ ...estimate, low: -1 },
			{ ...estimate, high: 22 },
			{ ...estimate, high: 101 },
			{ ...estimate, high: Number.POSITIVE_INFINITY },
			{ ...estimate, confidence: "high" },
			{ ...estimate, reasoning: " " },
			{ ...estimate, sources: [null] },
			{ ...estimate, sources: "https://example.com" },
			{ ...estimate, updated_at: "unknown" },
		];
		for (const invalid of invalidEstimates) {
			const model = evaluation({
				intelligence_estimate: invalid as typeof estimate,
			});
			expect(renderToStaticMarkup(<ModelBenchmarks evaluation={model} />)).toBe(
				"",
			);
			expect(
				renderToStaticMarkup(<IntelligenceIndexBadge evaluation={model} />),
			).toBe("");
		}
	});

	test("only renders unique HTTPS evidence links without credentials", () => {
		const model = evaluation({
			intelligence_estimate: {
				...estimate,
				sources: [
					"javascript:alert(1)",
					"data:text/html,unsafe",
					"http://example.com/insecure",
					"//example.com/relative",
					"https://user:password@example.com/private",
					...estimate.sources,
					...estimate.sources,
				],
			},
		});
		const markup = renderToStaticMarkup(<ModelBenchmarks evaluation={model} />);
		expect(markup).toContain(estimate.reasoning);
		expect(markup.match(/href=/g)).toHaveLength(1);
		expect(markup).toContain('href="https://huggingface.co/example/model"');
		expect(markup).not.toContain("javascript:");
		expect(markup).not.toContain("data:text");
		expect(markup).not.toContain("example.com");
	});

	test("shows publisher results with units and provenance without an AA index", () => {
		const markup = renderToStaticMarkup(
			<ModelBenchmarks
				evaluation={evaluation({ reported_benchmarks: [reportedBenchmark] })}
			/>,
		);
		expect(markup).toContain("Publisher benchmarks");
		expect(markup).toContain("OmniDocBench v1.6 Overall");
		expect(markup).toContain("95.06 score");
		expect(markup).toContain("Higher is better");
		expect(markup).toContain("Scope: publisher-reported");
		expect(markup).toContain(reportedBenchmark.notes);
		expect(markup).toContain(`href="${reportedBenchmark.source}"`);
		expect(markup).toContain("Source: huggingface.co");
		expect(markup).not.toContain("Artificial Analysis");
		expect(markup).not.toContain("artificialanalysis.ai");
	});

	test("keeps publisher results alongside estimated and measured intelligence", () => {
		for (const measured of [false, true]) {
			const markup = renderToStaticMarkup(
				<ModelBenchmarks
					evaluation={evaluation({
						intelligence_estimate: estimate,
						artificial_analysis_intelligence_index: measured ? 45.1 : null,
						reported_benchmarks: [reportedBenchmark],
					})}
				/>,
			);
			expect(markup).toContain("Publisher benchmarks");
			expect(markup).toContain("95.06 score");
			if (measured) {
				expect(markup).toContain("45.1");
				expect(markup).toContain("artificialanalysis.ai/models/kimi-k3-1-pro");
				expect(markup).not.toContain("Estimated intelligence");
			} else {
				expect(markup).toContain("Estimated intelligence");
				expect(markup).toContain("Range: 18.0 to 28.0");
				expect(markup).not.toContain("artificialanalysis.ai");
			}
		}
	});

	test("preserves publisher units and lower-is-better scores without percentage bars", () => {
		const benchmarks = [
			{ name: "Perplexity", score: 5.6961, unit: "perplexity" },
			{ name: "Role-Play Bench", score: 1, unit: "rank" },
			{ name: "Rewriting errors", score: 13.8095238, unit: "percent flagged" },
			{ name: "Custom metric", score: 0, unit: "custom units" },
			{ name: "Signed metric", score: -3.5, unit: "points" },
		].map((metric) => ({
			...reportedBenchmark,
			...metric,
			direction: "lower" as const,
		}));
		const markup = renderToStaticMarkup(
			<ModelBenchmarks
				evaluation={evaluation({ reported_benchmarks: benchmarks })}
			/>,
		);
		for (const value of [
			"5.6961 perplexity",
			"1 rank",
			"13.8095 percent flagged",
			"0 custom units",
			"-3.5 points",
		]) {
			expect(markup).toContain(value);
		}
		expect(markup.match(/Lower is better/g)).toHaveLength(5);
		expect(markup).not.toContain("Higher is better");
		expect(markup).not.toContain("style=");
		expect(markup).not.toContain("%");
	});

	test("discards malformed publisher results while retaining valid entries", () => {
		const invalidEntries = [
			null,
			"95",
			[],
			{},
			{ ...reportedBenchmark, score: "95" },
			{ ...reportedBenchmark, score: Number.NaN },
			{ ...reportedBenchmark, score: Number.POSITIVE_INFINITY },
			{ ...reportedBenchmark, name: " " },
			{ ...reportedBenchmark, name: 123 },
			{ ...reportedBenchmark, unit: null },
			{ ...reportedBenchmark, scope: {} },
			{ ...reportedBenchmark, notes: false },
			{ ...reportedBenchmark, direction: "up" },
			{ ...reportedBenchmark, source: 123 },
		];
		for (const entry of invalidEntries) {
			const reported = entry as typeof reportedBenchmark;
			expect(
				renderToStaticMarkup(
					<ModelBenchmarks
						evaluation={evaluation({ reported_benchmarks: [reported] })}
					/>,
				),
			).toBe("");
			const markup = renderToStaticMarkup(
				<ModelBenchmarks
					evaluation={evaluation({
						reported_benchmarks: [reported, reportedBenchmark],
					})}
				/>,
			);
			expect(markup.match(/OmniDocBench v1.6 Overall/g)).toHaveLength(1);
		}
		for (const invalidList of [null, "benchmarks", {}]) {
			expect(
				renderToStaticMarkup(
					<ModelBenchmarks
						evaluation={evaluation({
							reported_benchmarks: invalidList as (typeof reportedBenchmark)[],
						})}
					/>,
				),
			).toBe("");
		}
	});

	test("requires HTTPS publisher sources without embedded credentials", () => {
		for (const source of [
			"javascript:alert(1)",
			"data:text/html,unsafe",
			"http://example.com/insecure",
			"//example.com/relative",
			"https://user:password@example.com/private",
			"https://user@example.com/private",
			"not a URL",
		]) {
			expect(
				renderToStaticMarkup(
					<ModelBenchmarks
						evaluation={evaluation({
							reported_benchmarks: [{ ...reportedBenchmark, source }],
						})}
					/>,
				),
			).toBe("");
		}
	});

	test("does not create AA model links for synthetic estimate slugs", () => {
		const markup = renderToStaticMarkup(
			<ModelBenchmarks
				evaluation={{
					...evaluation({
						intelligence_estimate: estimate,
						reported_benchmarks: [reportedBenchmark],
						gpqa: 0.5,
					}),
					slug: "flow-like-estimate-document-reader",
				}}
			/>,
		);
		expect(markup).toContain("Publisher benchmarks");
		expect(markup).toContain("Estimated intelligence");
		expect(markup).not.toContain("artificialanalysis.ai/models/");
	});
});
