import { describe, expect, test } from "bun:test";
import { getI18n } from "@flow-like/locales";
import {
	emptyModels,
	gpuBoxModels,
	macMiniModels,
	overviewOf,
} from "../../../../lib/device-management/model/__fixtures__/sample-models";
import {
	RECOMMENDATION_CODES,
	type Recommendation,
} from "../../../../lib/device-management/models";
import type { DevicesT } from "../primitives/area-context";
import { trayTitle } from "./action-copy";
import { headlineCopy, jobFailureText, residencyLabel } from "./models-copy";
import { modelsHeadline } from "./models-view";
import { recommendationCopy, recommendationId } from "./recommendation-copy";

const t = getI18n().getFixedT("en", "devices") as DevicesT;
const DEVICE = "gpu-box";
const names = new Map([["qwen3-8b", "Qwen3-8B Q4_K_M"]]);
const context = {
	device: DEVICE,
	modelName: (id: string) => names.get(id),
};

/** Wire values and parameter keys that must never reach a sentence (R3). */
const WIRE =
	/kv_percent|queue_p95|physical_cores|suggested_ctx|extra_bytes|free_bytes|idle_hours|ttft_p95|memory_percent|\{\{|undefined|NaN|_/;
/** Model and quantization names are the user's own text and may hold underscores. */
const NAMES =
	/Qwen3-8B Q4_K_M|Gemma 3 4B Q4_K_M|Q3_K_M|BGE Small EN v1\.5 Q8_0|Qwen3-4B MLX 4-bit/g;
const leaks = (sentence: string) => WIRE.test(sentence.replace(NAMES, "NAME"));

const FULL: Record<Recommendation["code"], Recommendation["params"]> = {
	gpu_unused: { gpu: "NVIDIA GeForce RTX 4090" },
	partial_offload: { quant: "Q3_K_M", size_bytes: 4_020_000_000 },
	requests_queued: { deferred: 3, queue_p95_ms: 1_800 },
	kv_pressure: { kv_percent: 93 },
	ctx_truncation: {
		percent: 12,
		ctx: 8_192,
		suggested_ctx: 16_384,
		extra_bytes: 1_073_741_824,
	},
	memory_pressure: { memory_percent: 94 },
	idle_resident: { idle_hours: 26 },
	slow_ttft: { ttft_p95_ms: 2_400 },
	cpu_threads: { threads: 8, physical_cores: 16 },
	disk_low: { free_bytes: 12_000_000_000 },
	runtime_outdated: { latest: "b10900" },
	container_gpu_hidden: {},
};

const recommendation = (
	code: Recommendation["code"],
	params?: Recommendation["params"],
): Recommendation => ({
	code,
	tier: "now",
	model_id: "qwen3-8b",
	...(params ? { params } : {}),
});

describe("recommendation copy", () => {
	test("every code has a sentence with its parameters filled in", () => {
		for (const code of RECOMMENDATION_CODES) {
			const copy = recommendationCopy(
				t,
				recommendation(code, FULL[code]),
				context,
			);
			expect([code, copy.sentence.length > 20]).toEqual([code, true]);
			expect([code, leaks(copy.sentence)]).toEqual([code, false]);
		}
	});

	test("every code still reads when the device sends no parameters", () => {
		for (const code of RECOMMENDATION_CODES) {
			const copy = recommendationCopy(t, recommendation(code), context);
			expect([code, leaks(copy.sentence)]).toEqual([code, false]);
		}
	});

	test("parameters are formatted: bytes, durations, counts", () => {
		const sentence = (code: Recommendation["code"]) =>
			recommendationCopy(t, recommendation(code, FULL[code]), context).sentence;
		expect(sentence("kv_pressure")).toContain("Qwen3-8B Q4_K_M is 93 % full");
		expect(sentence("requests_queued")).toContain("3 are queued");
		expect(sentence("requests_queued")).toContain("1.8 s");
		expect(sentence("ctx_truncation")).toContain("16,384 tokens");
		expect(sentence("ctx_truncation")).toContain("1.0 GiB more memory");
		expect(sentence("cpu_threads")).toContain("8 threads on 16 physical cores");
		expect(sentence("disk_low")).toContain("11.2 GiB left");
		expect(sentence("slow_ttft")).toContain("2.4 s");
		expect(
			recommendationCopy(
				t,
				recommendation("requests_queued", { deferred: 1, queue_p95_ms: 640 }),
				context,
			).sentence,
		).toContain("1 is queued and the slowest wait 640 ms");
	});

	test("the model is named by its display name, else its id; a device-wide one names no model; the device is mono", () => {
		const unknown = recommendationCopy(
			t,
			{ ...recommendation("idle_resident"), model_id: "llama-3" },
			context,
		);
		expect(unknown.sentence).toContain(
			"Nothing uses llama-3, but it stays loaded",
		);
		const device = recommendationCopy(
			t,
			{ code: "gpu_unused", tier: "soon" },
			context,
		);
		expect(device.sentence).toBe(
			"The GPU isn't used: models run on the CPU. A GPU runtime runs them faster.",
		);
		expect(device.names).toEqual([DEVICE]);
		expect(
			recommendationCopy(
				t,
				{ code: "kv_pressure", tier: "now", params: { kv_percent: 91 } },
				context,
			).sentence,
		).toContain("The context cache of a model is 91 % full");
		expect(recommendationId(recommendation("kv_pressure"))).toBe(
			"kv_pressure:qwen3-8b",
		);
	});

	test("the samples' recommendations all read", () => {
		for (const sample of [gpuBoxModels(), macMiniModels()]) {
			const overview = overviewOf(sample);
			const modelName = (id: string) =>
				overview.models.find((model) => model.id === id)?.display_name;
			for (const item of overview.recommendations)
				expect(
					leaks(
						recommendationCopy(t, item, { device: DEVICE, modelName }).sentence,
					),
				).toBe(false);
		}
	});
});

describe("headline copy", () => {
	const copy = (overview: ReturnType<typeof overviewOf>) =>
		headlineCopy(t, modelsHeadline(overview), {
			device: DEVICE,
			locale: "en",
		});

	test("no models: what to do next", () => {
		expect(copy(overviewOf(emptyModels()))).toEqual({
			lead: "gpu-box hosts no models yet.",
			rest: "Add one to serve it to your apps and to the people you share the device with.",
		});
	});

	test("serving: the day's traffic, errors, GPU memory and downloads", () => {
		const gpu = copy(overviewOf(gpuBoxModels()));
		expect(gpu.lead).toBe("Serving 2 of 3 models.");
		expect(gpu.rest).toBe(
			"1.7M tokens and 4.4K requests in the last 24 hours · 6 errors · GPU memory 29 % used · 1 model downloading",
		);
		expect(copy(overviewOf(macMiniModels())).lead).toBe("Serving 2 models.");
	});
});

describe("labels", () => {
	test("residency, failures and tray titles are sentences, never wire values", () => {
		expect(
			residencyLabel(t, { mode: "on_demand", idle_unload_after_seconds: 900 }),
		).toBe("Loads on demand, unloads after 15 min idle");
		const blocked = gpuBoxModels().jobs[1];
		if (blocked?.state !== "failed") throw new Error("expected a failed job");
		expect(jobFailureText(t, blocked)).toContain("couldn't reach any download");
		expect(trayTitle(t, "install_runtime")).toBe("Install model runtime");
	});
});
