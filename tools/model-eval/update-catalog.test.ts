import { describe, expect, test } from "bun:test";
import {
	type CatalogBitRow,
	catalogReleaseDate,
	findQwenBit,
	planQualityUpdates,
	qwenBenchmarkState,
	savedBitFromEvents,
} from "./update-catalog";

const whisper: CatalogBitRow = {
	id: "whisper",
	type: "STT",
	modelSlug: null,
	repository: "https://huggingface.co/openai/whisper-tiny",
	parameters: { model_type: "WhisperTiny" },
};

describe("catalog update boundaries", () => {
	test("seed reruns preserve independently refreshed benchmark state", () => {
		const evaluations = {
			artificial_analysis_intelligence_index_version: "5.1",
			artificial_analysis_intelligence_index: 50,
		};
		const state = qwenBenchmarkState({
			model_evaluation: { evaluations },
			parameters: {
				model_classification: { coding: 0.8 },
				classification_unavailable_fields: ["safety"],
				benchmark_notes: "Updated benchmark evidence",
			},
		});
		expect(state.evaluations).toEqual(evaluations);
		expect(state.parameters.model_classification).toEqual({ coding: 0.8 });
		expect(state.parameters.benchmark_notes).toBe("Updated benchmark evidence");
	});

	test("new seeds identify the measured benchmark version", () => {
		const state = qwenBenchmarkState();
		expect(
			state.evaluations?.artificial_analysis_intelligence_index_version,
		).toBe("5.0.1");
		expect(state.evaluations?.benchmark_status).toBe("measured");
	});
	test("preserves release dates when replaying public evaluations through admin writes", () => {
		expect(catalogReleaseDate("2026-11-16T00:00:00+00:00")).toBe("2026-11-16");
		expect(catalogReleaseDate("2026-11-16")).toBe("2026-11-16");
		expect(catalogReleaseDate(null)).toBeNull();
		expect(catalogReleaseDate("unknown")).toBeNull();
	});

	test("recognizes successful streamed writes and rejects partial or failed writes", () => {
		const done =
			'event: done\r\ndata: {"id":"saved-bit","model_slug":"qwen4-max"}\r\n\r\n';
		expect(savedBitFromEvents(`event: progress\ndata: {}\n\n${done}`).id).toBe(
			"saved-bit",
		);
		expect(() => savedBitFromEvents("event: progress\ndata: {}\n\n")).toThrow(
			"did not confirm",
		);
		expect(() =>
			savedBitFromEvents(`${done}event: error\ndata: {}\n\n`),
		).toThrow("write failure");
		expect(() => savedBitFromEvents("event: done\ndata: {}\n\n")).toThrow(
			"Invalid saved",
		);
	});

	test("includes public API category spellings in the quality plan", () => {
		const result = planQualityUpdates([{ ...whisper, type: "Stt" }]);
		expect(result.updates).toHaveLength(1);
	});

	test("rates known model identities without touching assets or LLMs", () => {
		const result = planQualityUpdates([
			whisper,
			{ ...whisper, id: "asset", type: "FILE" },
			{ ...whisper, id: "llm", type: "LLM", modelSlug: "measured-llm" },
		]);
		expect(result.updates.map(({ bit }) => bit.id)).toEqual(["whisper"]);
		expect(result.unmatched).toEqual([]);
		expect(whisper.modelSlug).toBeNull();
	});

	test("does not assign a family score to an unknown model", () => {
		const unknown = {
			...whisper,
			repository: "https://huggingface.co/example/new-asr",
			parameters: { model_type: "UnknownAsr" },
		};
		const result = planQualityUpdates([unknown]);
		expect(result.updates).toEqual([]);
		expect(result.unmatched).toEqual([unknown]);
	});

	test("refuses to replace a model's existing evaluation association", () => {
		expect(() =>
			planQualityUpdates([
				{ ...whisper, modelSlug: "measured-transcription-model" },
			]),
		).toThrow("already links");
	});

	test("supports repeated application to the same evaluation", () => {
		const first = planQualityUpdates([whisper]).updates[0];
		const second = planQualityUpdates([
			{ ...whisper, modelSlug: first.rating.slug },
		]);
		expect(second.updates[0].rating.slug).toBe(first.rating.slug);
	});

	test("matches the hosted provider and exact Qwen model id", () => {
		const hosted = {
			...whisper,
			id: "qwen",
			type: "VLM",
			parameters: {
				provider: {
					provider_name: "hosted:openrouter",
					model_id: "qwen/qwen4-max",
				},
			},
		};
		const custom = {
			...hosted,
			id: "custom",
			parameters: {
				provider: { provider_name: "openrouter", model_id: "qwen/qwen4-max" },
			},
		};
		expect(findQwenBit([custom, hosted])).toBe(hosted);
		expect(findQwenBit([custom])).toBeUndefined();
		expect(() => findQwenBit([hosted, { ...hosted, id: "duplicate" }])).toThrow(
			"Multiple hosted",
		);
	});
});
