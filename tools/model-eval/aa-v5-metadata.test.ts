import { describe, expect, test } from "bun:test";
import { refreshBenchmarkDescription } from "./aa-v5-metadata";

const measured = {
	artificial_analysis_intelligence_index_version: "5.0.1",
	artificial_analysis_intelligence_index: 32.12,
	benchmark_status: "measured",
	benchmark_model_name: "GLM-5.3 (Max)",
	benchmark_reasoning_effort: "max",
	benchmark_source_url: "https://artificialanalysis.ai/models/glm-5-3",
};
const estimated = {
	...measured,
	artificial_analysis_intelligence_index: null,
	benchmark_status: "estimated",
	intelligence_estimate: {
		version: "5.0.1",
		method: "paired-version-calibration",
		score: 19.4,
		low: 11,
		high: 28,
		confidence: "low",
		prior: {
			source: "https://huggingface.co/example/model",
			basis:
				"Input is a base-model result used as a proxy for this modified model. The uncensored artifact has no direct evaluation.",
		},
	},
};
const oldEstimate =
	"Ranking estimates: Estimated intelligence 27, judgment range 20 to 34, low confidence, reviewed 2026-10-27. This is a Flow-Like estimate on the catalog's intelligence scale; it is not a measured Artificial Analysis result. The range is not a statistical confidence interval. ";

describe("AA v5 description refresh", () => {
	test("replaces an old headline while retaining Prime and runtime provenance", () => {
		const caveat =
			"This Prime variant uses the underlying model evaluation. Artificial Analysis has not published a separate Prime evaluation; its benchmark pricing and speed refer to the underlying standard model.";
		const description = `A higher-throughput model.\n\nArtificial Analysis reports an Intelligence Index of 44.8 for GLM-5.3 (Max). The score describes that benchmark configuration; workflow reasoning settings can differ. ${caveat}\n\nSource: https://artificialanalysis.ai/models/old-glm\n\nStandard rates are $1.50 per million input tokens. Context: 1,000,000 tokens.`;
		const refreshed = refreshBenchmarkDescription(description, measured);
		expect(refreshed).toContain(
			"Intelligence Index v5.0.1: 32.12 for GLM-5.3 (Max)",
		);
		expect(refreshed).toContain("Evaluated reasoning effort: max");
		expect(refreshed).toContain(caveat);
		expect(refreshed).toContain(
			"Standard rates are $1.50 per million input tokens. Context: 1,000,000 tokens.",
		);
		expect(refreshed).toContain(measured.benchmark_source_url);
		expect(refreshed).not.toContain("44.8");
		expect(refreshed).not.toContain("models/old-glm");
		expect(refreshBenchmarkDescription(refreshed, measured)).toBe(refreshed);
	});

	test("updates estimates without keeping old numerical anchors", () => {
		const description = `${oldEstimate}Estimated on the current catalog intelligence scale, near GLM5.3 (44.8) and KimiK3 (43.6). Tencent reports competitive engineering evaluations and strong GPQA/tool results. These selected tasks do not reproduce the AA suite, so use a broad uncertainty interval.\n\nRanking sources:\n- https://huggingface.co/tencent/Hy4-preview`;
		const refreshed = refreshBenchmarkDescription(description, estimated);
		expect(refreshed).toContain(
			"AA v5.0.1 scale: 19.4, judgment range 11 to 28, low confidence",
		);
		expect(refreshed).toContain("legacy catalog mixed index versions");
		expect(refreshed).toContain("not an exact v4.3 conversion");
		expect(refreshed).toContain("not a measured Artificial Analysis result");
		expect(refreshed).toContain(
			"Tencent reports competitive engineering evaluations",
		);
		expect(refreshed).toContain("https://huggingface.co/tencent/Hy4-preview");
		for (const stale of [
			"Estimated intelligence 27",
			"44.8",
			"43.6",
			"reviewed 2026-10-27",
		])
			expect(refreshed).not.toContain(stale);
		expect(refreshBenchmarkDescription(refreshed, estimated)).toBe(refreshed);
	});

	test("preserves modified-checkpoint and production-alias caveats", () => {
		const modified = `${oldEstimate}A quantized and modified Qwen3.8-27B model for local coding, reasoning and security work. Base-model AA results span roughly 20 to 34 depending on reasoning effort. The 27 estimate allows for quantization, modification and runtime uncertainty; base benchmarks are not copied as measurements of this checkpoint.`;
		const refreshed = refreshBenchmarkDescription(modified, estimated);
		expect(refreshed).toContain("A quantized and modified Qwen3.8-27B model");
		expect(refreshed).toContain(
			"quantization, modification and runtime uncertainty",
		);
		expect(refreshed).toContain(
			"base benchmarks are not copied as measurements of this checkpoint",
		);
		expect(refreshed).not.toContain("20 to 34");
		expect(refreshed).not.toContain("The 27 estimate");
		const alias = refreshBenchmarkDescription(
			`${oldEstimate}Close proxy to AA Qwen3.8-Flash-Next (39.8). Qwen explicitly identifies Flash as the production version based on Flash-Next. Production features and hosted settings can differ.`,
			estimated,
		);
		expect(alias).toContain("Close proxy to AA Qwen3.8-Flash-Next.");
		expect(alias).toContain(
			"Production features and hosted settings can differ.",
		);
		expect(alias).not.toContain("39.8");
	});

	test("retains quantization uncertainty after removing an old numeric uplift", () => {
		const refreshed = refreshBenchmarkDescription(
			`${oldEstimate}Those models provide current index anchors near 18 and 17. A modest uplift gives 21, with uncertainty for self-reported results and local Q4 quantization.`,
			estimated,
		);
		expect(refreshed).toContain(
			"uncertainty for self-reported results and local Q4 quantization",
		);
		expect(refreshed).not.toContain("near 18 and 17");
		expect(refreshed).not.toContain("gives 21");
	});

	test("keeps independent creativity, language and cost evidence", () => {
		const description =
			"Capability ratings: Ratings for coding, creativity, factuality, function calling, multilinguality, openness, reasoning, safety, speed include researched estimates. Local inference has no billed token fee; hardware and electricity are excluded from cost efficiency. The Safety rating follows the catalog's factual-reliability convention. It is not a harmful-content safety assessment. Sources and per-field reasoning are stored with the ratings.\n\nEstimated capability evidence (2026-10-27):\n- coding: 0.95. Old AA-derived score.\n- multilinguality: 0.82. English and Chinese evidence. Sources: https://example.com/languages\n- creativity: 0.41. A writing estimate. Sources: https://example.com/writing";
		const refreshed = refreshBenchmarkDescription(description, measured);
		expect(refreshed).toContain(
			"Ratings for creativity, multilinguality, openness, speed include researched estimates.",
		);
		expect(refreshed).toContain(
			"Local inference has no billed token fee; hardware and electricity are excluded from cost efficiency.",
		);
		expect(refreshed).toContain("multilinguality: 0.82");
		expect(refreshed).toContain("creativity: 0.41");
		expect(refreshed).toContain("does not measure harmful-content safety");
		expect(refreshed).not.toContain("coding: 0.95");
		expect(refreshed).not.toContain("factual-reliability convention");
		expect(refreshBenchmarkDescription(refreshed, measured)).toBe(refreshed);
	});

	test("updates inline GPT-OSS and Terra version statements without dropping API details", () => {
		const oss =
			"Supports a 131,072-token context, structured outputs, streaming and low, medium or high reasoning effort. Text input and output only. Current Artificial Analysis v5.0 intelligence is unavailable; the catalog labels its inferred score as an estimate and preserves the earlier index as a historical benchmark.";
		const terra =
			"Supports structured outputs, streaming and configurable reasoning effort from none through max. OpenRouter exposes the model through its Chat Completions API with tool calling. The intelligence score is Artificial Analysis v5.0 at max reasoning effort. Missing capability measurements are recorded as estimates with sources.";
		for (const text of [oss, terra]) {
			const refreshed = refreshBenchmarkDescription(text, estimated);
			expect(refreshed).toContain("structured outputs, streaming");
			expect(refreshed).toContain("AA v5.0.1 scale");
			expect(refreshed).not.toContain("Artificial Analysis v5.0 ");
			expect(refreshed).not.toContain(
				"Missing capability measurements are recorded as estimates",
			);
			expect(refreshBenchmarkDescription(refreshed, estimated)).toBe(refreshed);
		}
	});

	test("keeps Qwen4's runtime effort and token prices beside its measured effort", () => {
		const description =
			"Artificial Analysis reports an Intelligence Index of 42.89 at xhigh reasoning effort. OpenRouter enables reasoning by default at high effort, so everyday results and latency can differ from the benchmark configuration.\n\nStandard rates are $1.50 per million input tokens and $4.50 per million output tokens.";
		const refreshed = refreshBenchmarkDescription(description, {
			...measured,
			artificial_analysis_intelligence_index: 42.8914543,
			benchmark_reasoning_effort: "xhigh",
		});
		expect(refreshed).toContain("42.89");
		expect(refreshed).toContain("Evaluated reasoning effort: xhigh");
		expect(refreshed).toContain(
			"OpenRouter enables reasoning by default at high effort",
		);
		expect(refreshed).toContain(
			"$1.50 per million input tokens and $4.50 per million output tokens",
		);
	});

	test("adds a current summary to unscored prose and qualifies modified-model equivalence", () => {
		const description =
			"- Identical capability to the standard build, including image understanding.\n- Needs roughly 17 GB of free memory.";
		const refreshed = refreshBenchmarkDescription(description, estimated);
		expect(refreshed).toContain("base model as a proxy");
		expect(refreshed).toContain(
			"modified build has no direct AA v5.0.1 measurement",
		);
		expect(refreshed).toContain("Needs roughly 17 GB of free memory.");
		expect(refreshed).not.toContain("Identical capability");
		expect(refreshBenchmarkDescription(refreshed, estimated)).toBe(refreshed);
	});

	test("retains the original when a replacement score is invalid and accepts measured zero", () => {
		const original =
			"Artificial Analysis reports an Intelligence Index of 44.8 for this model.";
		for (const invalid of [
			{},
			{ ...measured, artificial_analysis_intelligence_index_version: "5.0" },
			{ ...measured, artificial_analysis_intelligence_index: Number.NaN },
			{
				...estimated,
				intelligence_estimate: { ...estimated.intelligence_estimate, low: 25 },
			},
		])
			expect(refreshBenchmarkDescription(original, invalid)).toBe(original);
		expect(
			refreshBenchmarkDescription(original, {
				...measured,
				artificial_analysis_intelligence_index: 0,
			}),
		).toContain("v5.0.1: 0 for");
	});
});
