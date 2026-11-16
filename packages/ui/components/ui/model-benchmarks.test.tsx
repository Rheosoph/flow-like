import { describe, expect, test } from "bun:test";
import { createI18n } from "@flow-like/locales";
import { renderToStaticMarkup } from "react-dom/server";
import { bitRating } from "../../lib/bit/model-rating";
import { type IBit, IBitTypes } from "../../lib/schema/bit/bit";
import {
	type IModelEvaluation,
	IntelligenceIndexBadge,
	ModelBenchmarks,
	ModelScoreBadge,
} from "./model-benchmarks";
import { ModelRatingBadge, ModelRatingPanel } from "./model-rating";

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

const currentVersion = {
	artificial_analysis_intelligence_index_version: "5.0.1",
	benchmark_status: "measured" as const,
};

const estimate = {
	score: 23,
	low: 18,
	high: 28,
	confidence: "low" as const,
	reasoning: "Estimated from the model card and comparable models.",
	sources: ["https://huggingface.co/example/model"],
	updated_at: "2026-10-27",
};

const currentEstimate = {
	...estimate,
	version: "5.0.1",
	method: "paired-version-calibration",
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

const qualityEstimate = {
	...estimate,
	category: IBitTypes.Tts as const,
	criteria: "Speech naturalness, intelligibility and pronunciation.",
	score: 85,
	low: 75,
	high: 90,
};

describe("model benchmarks", () => {
	test("shows category quality estimates with their scale, evidence and uncertainty", () => {
		const model = evaluation({ quality_estimate: qualityEstimate });
		const markup = renderToStaticMarkup(
			<ModelBenchmarks evaluation={model} type={IBitTypes.Tts} />,
		);
		for (const text of [
			"Estimated quality",
			"85/100",
			"Text to speech",
			"Scores compare models within the same category",
			"Range: 75.0 to 90.0",
			"Low confidence",
			qualityEstimate.criteria,
			qualityEstimate.reasoning,
		]) {
			expect(markup).toContain(text);
		}
		expect(markup).toContain('href="https://huggingface.co/example/model"');
		expect(markup).not.toContain("Artificial Analysis");
		expect(markup).not.toContain("Estimated intelligence");
		const badge = renderToStaticMarkup(
			<ModelScoreBadge evaluation={model} type={IBitTypes.Tts} />,
		);
		expect(badge).toContain("Est. quality 85/100");
		expect(badge).toContain(
			'aria-label="Estimated quality: 85/100 (Text to speech)"',
		);
	});

	test("keeps quality categories and intelligence scores separate", () => {
		const model = evaluation({
			quality_estimate: qualityEstimate,
			...currentVersion,
			artificial_analysis_intelligence_index: 45.1,
		});
		expect(
			renderToStaticMarkup(
				<ModelScoreBadge evaluation={model} type={IBitTypes.Llm} />,
			),
		).toContain("45.1");
		expect(
			renderToStaticMarkup(
				<ModelScoreBadge evaluation={model} type={IBitTypes.Stt} />,
			),
		).toBe("");
		const ttsBadge = renderToStaticMarkup(
			<ModelScoreBadge evaluation={model} type={IBitTypes.Tts} />,
		);
		expect(ttsBadge).toContain("85/100");
		expect(ttsBadge).not.toContain("45.1");
		expect(
			renderToStaticMarkup(
				<ModelBenchmarks
					evaluation={evaluation({ quality_estimate: qualityEstimate })}
					type={IBitTypes.Stt}
				/>,
			),
		).toBe("");
	});

	test("retains raw publisher benchmarks beside category estimates", () => {
		const markup = renderToStaticMarkup(
			<ModelBenchmarks
				evaluation={evaluation({
					quality_estimate: qualityEstimate,
					reported_benchmarks: [
						{
							...reportedBenchmark,
							name: "Word error rate",
							score: 2.4,
							unit: "% WER",
							direction: "lower",
						},
					],
				})}
			/>,
		);
		expect(markup).toContain("Estimated quality");
		expect(markup).toContain("2.4 % WER");
		expect(markup).toContain("Lower is better");
	});

	test("renders current coding, tool-use, and factuality benchmarks", () => {
		const markup = renderToStaticMarkup(
			<ModelBenchmarks
				evaluation={evaluation({
					...currentVersion,
					hle_diamond: 0.21,
					critpt_agentic_hard: 0.41,
					frontiercode_v2_0: 0.31,
					terminalbench_science_v0_1: 0.61,
					gdp_pdf_all_pass: 0.51,
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
			"HLE-Diamond",
			"CritPt v2 (Hard Subset)",
			"FrontierCode-AA 2.0",
			"Terminal-Bench-Science 0.1",
			"GDP.pdf",
			"AutomationBench-AA",
			"AA-Omniscience v1.1 Accuracy",
			"AA-Omniscience v1.1 Non-Hallucination Rate",
		]) {
			expect(markup).toContain(label);
		}
		expect(markup).not.toContain("Terminal-Bench 2.1");
		expect(markup).not.toContain("TAU-Banking");
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
				evaluation={evaluation({
					...currentVersion,
					terminalbench_v4_0: 0,
					scicode: null,
				})}
			/>,
		);
		expect(markup).toContain("Terminal-Bench 4.0");
		expect(markup).toContain("0.0%");
		expect(markup).not.toContain("SciCode");
		expect(
			renderToStaticMarkup(
				<IntelligenceIndexBadge
					evaluation={evaluation({
						...currentVersion,
						artificial_analysis_intelligence_index: 0,
					})}
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

	test("never displays invalid current indices as scores", () => {
		for (const value of [
			undefined,
			null,
			Number.NaN,
			Number.POSITIVE_INFINITY,
			-1,
			101,
		]) {
			const model = evaluation({
				...currentVersion,
				artificial_analysis_intelligence_index: value,
				intelligence_estimate: estimate,
			});
			for (const markup of [
				renderToStaticMarkup(<ModelBenchmarks evaluation={model} />),
				renderToStaticMarkup(<IntelligenceIndexBadge evaluation={model} />),
			]) {
				expect(markup).toContain("Not evaluated in AA v5.0.1");
				expect(markup).not.toContain("Estimated intelligence");
				expect(markup).not.toContain("NaN");
				expect(markup).not.toContain("Infinity");
			}
		}
	});

	test("requires an exact current version and measured status for headline scores", () => {
		for (const version of [undefined, "4.0", "5.0", "5.0.0"]) {
			const model = evaluation({
				...currentVersion,
				artificial_analysis_intelligence_index_version: version,
				artificial_analysis_intelligence_index: 55.1,
				artificial_analysis_coding_index: 78.2,
				intelligence_estimate: estimate,
				terminalbench_v4_0: 0.6,
			});
			for (const markup of [
				renderToStaticMarkup(<ModelBenchmarks evaluation={model} />),
				renderToStaticMarkup(<IntelligenceIndexBadge evaluation={model} />),
			]) {
				expect(markup).toContain("Not evaluated in AA v5.0.1");
				expect(markup).not.toContain("55.1");
				expect(markup).not.toContain("78.2");
				expect(markup).not.toContain("23.0");
				expect(markup).not.toContain("Terminal-Bench");
			}
		}
		for (const status of [undefined, "not_evaluated", "unmatched"] as const) {
			const model = evaluation({
				...currentVersion,
				benchmark_status: status,
				artificial_analysis_intelligence_index: 55.1,
			});
			expect(
				renderToStaticMarkup(<IntelligenceIndexBadge evaluation={model} />),
			).not.toContain("55.1");
		}
	});

	test("shows measured zero and the exact index version without legacy estimates", () => {
		for (const score of [0, 45.1]) {
			const model = evaluation({
				...currentVersion,
				artificial_analysis_intelligence_index: score,
				intelligence_estimate: estimate,
				legacy_evaluations: { artificial_analysis_intelligence_index: 55.1 },
			});
			const markup = renderToStaticMarkup(
				<ModelBenchmarks evaluation={model} />,
			);
			const badge = renderToStaticMarkup(
				<IntelligenceIndexBadge evaluation={model} />,
			);
			for (const output of [markup, badge]) {
				expect(output).toContain(score.toFixed(1));
				expect(output).toContain("AA v5.0.1");
				expect(output).not.toContain("Estimated intelligence");
				expect(output).not.toContain(estimate.reasoning);
				expect(output).not.toContain("55.1");
			}
			expect(badge).toContain(
				`aria-label="Artificial Analysis Intelligence Index v5.0.1: ${score.toFixed(1)}"`,
			);
		}
	});

	test("explains missing and unmatched current results without reusing legacy estimates", () => {
		for (const status of ["not_evaluated", "unmatched"] as const) {
			const model = evaluation({
				...currentVersion,
				benchmark_status: status,
				artificial_analysis_intelligence_index: null,
				intelligence_estimate: estimate,
				legacy_evaluations: { artificial_analysis_intelligence_index: 55.1 },
			});
			const expected =
				status === "unmatched"
					? "No matching AA v5.0.1 model"
					: "Not evaluated in AA v5.0.1";
			for (const markup of [
				renderToStaticMarkup(<ModelBenchmarks evaluation={model} />),
				renderToStaticMarkup(<IntelligenceIndexBadge evaluation={model} />),
			]) {
				expect(markup).toContain(expected);
				expect(markup).not.toContain("23.0");
				expect(markup).not.toContain("55.1");
			}
		}
		expect(
			renderToStaticMarkup(<ModelScoreBadge type={IBitTypes.Llm} />),
		).toContain("Not evaluated in AA v5.0.1");
	});

	test("labels calibrated current estimates with their range and provenance", () => {
		const model = evaluation({
			...currentVersion,
			benchmark_status: "estimated",
			artificial_analysis_intelligence_index: null,
			intelligence_estimate: currentEstimate,
		});
		const markup = renderToStaticMarkup(<ModelBenchmarks evaluation={model} />);
		expect(markup).toContain("Estimated AA v5.0.1 Intelligence Index");
		expect(markup).toContain("23.0");
		expect(markup).toContain("Range: 18.0 to 28.0");
		expect(markup).toContain("Low confidence");
		expect(markup).toContain(estimate.reasoning);
		expect(markup).toContain('href="https://huggingface.co/example/model"');
		expect(markup).not.toContain("Evaluated reasoning effort:");
		expect(
			renderToStaticMarkup(<IntelligenceIndexBadge evaluation={model} />),
		).toContain("Est. AA v5.0.1 23.0");
	});

	test("labels the shared card rating and detail panel with the current version", () => {
		for (const estimated of [false, true]) {
			const model = evaluation({
				...currentVersion,
				benchmark_status: estimated ? "estimated" : "measured",
				artificial_analysis_intelligence_index: estimated ? null : 42.9,
				intelligence_estimate: estimated ? currentEstimate : undefined,
			});
			const rating = bitRating({
				type: IBitTypes.Llm,
				model_evaluation: model,
			} as IBit);
			if (!rating) throw new Error("Expected a current rating");
			const badge = renderToStaticMarkup(
				<ModelRatingBadge rating={rating} type={IBitTypes.Llm} />,
			);
			const panel = renderToStaticMarkup(
				<ModelRatingPanel rating={rating} type={IBitTypes.Llm} />,
			);
			expect(badge).toContain(estimated ? "Est. AA v5.0.1" : "AA v5.0.1");
			expect(panel).toContain(
				estimated
					? "Estimated AA v5.0.1 Intelligence Index"
					: "AA v5.0.1 Intelligence Index",
			);
			expect(panel).toContain("points");
			expect(panel).not.toContain("/ 100");
		}
		expect(
			renderToStaticMarkup(
				<ModelRatingBadge rating={null} type={IBitTypes.Vlm} />,
			),
		).toContain("Not evaluated in AA v5.0.1");
	});

	test("keeps provenance and methodology when the detail panel owns the score", () => {
		const markup = renderToStaticMarkup(
			<ModelBenchmarks
				hideRating
				evaluation={evaluation({
					...currentVersion,
					artificial_analysis_intelligence_index: 42.9,
					benchmark_reasoning_effort: "xhigh",
					benchmark_verified_at: "2026-11-16",
				})}
			/>,
		);
		expect(markup).not.toContain("42.9");
		expect(markup).toContain("xhigh");
		expect(markup).toContain("Source checked 2026-11-16");
		expect(markup).toContain("weighted benchmark tasks");
		expect(
			renderToStaticMarkup(
				<ModelBenchmarks
					hideRating
					evaluation={evaluation({ quality_estimate: qualityEstimate })}
				/>,
			),
		).toBe("");
	});

	test("rejects unversioned, uncalibrated and malformed current estimates", () => {
		for (const invalid of [
			estimate,
			{ ...currentEstimate, version: "5.0" },
			{ ...currentEstimate, method: "catalog-estimate" },
			{ ...currentEstimate, low: 24 },
			{ ...currentEstimate, high: 22 },
			{ ...currentEstimate, score: Number.NaN },
			{ ...currentEstimate, updated_at: "invalid" },
		]) {
			const model = evaluation({
				...currentVersion,
				benchmark_status: "estimated",
				intelligence_estimate: invalid,
			});
			for (const markup of [
				renderToStaticMarkup(<ModelBenchmarks evaluation={model} />),
				renderToStaticMarkup(<IntelligenceIndexBadge evaluation={model} />),
			]) {
				expect(markup).toContain("Not evaluated in AA v5.0.1");
				expect(markup).not.toContain("23.0");
			}
		}
	});

	test("exposes the evaluated effort, source date and methodology", () => {
		const markup = renderToStaticMarkup(
			<ModelBenchmarks
				evaluation={evaluation({
					...currentVersion,
					artificial_analysis_intelligence_index: 42.8914543,
					benchmark_reasoning_effort: "xhigh",
					benchmark_verified_at: "2026-11-16T13:20:00Z",
					benchmark_source_url:
						"https://artificialanalysis.ai/models/qwen4-max",
				})}
			/>,
		);
		expect(markup).toContain("42.9");
		expect(markup).toContain("Evaluated reasoning effort:");
		expect(markup).toContain("xhigh");
		expect(markup).toContain("Source checked 2026-11-16");
		expect(markup).toContain(
			'href="https://artificialanalysis.ai/models/qwen4-max"',
		);
		expect(markup).toContain("weighted benchmark tasks");
		expect(markup).toContain(
			'href="https://artificialanalysis.ai/methodology/intelligence-benchmarking"',
		);
	});

	test("keeps raw Elo and signed Omniscience scores off percentage bars", () => {
		const markup = renderToStaticMarkup(
			<ModelBenchmarks
				evaluation={evaluation({
					...currentVersion,
					aa_briefcase_elo: 1432.7,
					artificial_analysis_omniscience: -12.4,
				})}
			/>,
		);
		expect(markup).toContain("AA-Briefcase v1.1 (Elo)");
		expect(markup).toContain("1432.7");
		expect(markup).toContain("AA-Omniscience v1.1 Index");
		expect(markup).toContain("-12.4");
		expect(markup).not.toContain("%");
		expect(markup).not.toContain("style=");
	});

	test("only renders unique HTTPS evidence links without credentials", () => {
		const model = evaluation({
			...currentVersion,
			benchmark_status: "estimated",
			intelligence_estimate: {
				...currentEstimate,
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
		expect(
			markup.match(/href="https:\/\/huggingface.co\/example\/model"/g),
		).toHaveLength(1);
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

	test("keeps publisher results alongside current intelligence and missing results", () => {
		for (const measured of [false, true]) {
			const markup = renderToStaticMarkup(
				<ModelBenchmarks
					evaluation={evaluation({
						...currentVersion,
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
				expect(markup).toContain("Not evaluated in AA v5.0.1");
				expect(markup).not.toContain("Estimated intelligence");
				expect(markup).not.toContain("Range: 18.0 to 28.0");
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
		expect(markup).toContain("Not evaluated in AA v5.0.1");
		expect(markup).not.toContain("artificialanalysis.ai/models/");
	});
});
