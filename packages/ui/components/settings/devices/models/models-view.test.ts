import { describe, expect, test } from "bun:test";
import {
	SAMPLE_JOB_IDS,
	SAMPLE_MODEL_GRANT,
	emptyModels,
	gpuBoxModels,
	macMiniModels,
	overviewOf,
	statsOf,
} from "../../../../lib/device-management/model/__fixtures__/sample-models";
import type {
	ModelJob,
	ModelsOverview,
} from "../../../../lib/device-management/models";
import {
	backendOf,
	consumersOf,
	dayWindow,
	gpuMemoryPercent,
	jobProgress,
	modelsHeadline,
	nowCount,
	orderJobs,
	sameResidency,
	sameSettings,
	usageOf,
} from "./models-view";

const GIB = 1024 ** 3;

function withModels(
	overview: ModelsOverview,
	patch: Partial<ModelsOverview["summary"]>,
): ModelsOverview {
	return { ...overview, summary: { ...overview.summary, ...patch } };
}

describe("headline", () => {
	test("a host without models says so and nothing else", () => {
		const headline = modelsHeadline(overviewOf(emptyModels()));
		expect(headline.lead).toEqual({ kind: "empty" });
		expect(headline.requests24h).toBe(0);
		expect(headline.gpuMemoryPercent).toBeUndefined();
	});

	test("the GPU box serves two of three models, one still downloading", () => {
		const headline = modelsHeadline(overviewOf(gpuBoxModels()));
		expect(headline.lead).toEqual({ kind: "serving", loaded: 2, models: 3 });
		expect(headline.downloading).toBe(1);
		expect(headline.gpuMemoryPercent).toBe(29);
		expect(headline.tokens24h).toBeGreaterThan(1_000_000);
		expect(headline.errors24h).toBe(6);
	});

	test("a failed model wins over serving; none loaded is idle", () => {
		const overview = overviewOf(gpuBoxModels());
		expect(modelsHeadline(withModels(overview, { failed: 1 })).lead).toEqual({
			kind: "failed",
			failed: 1,
		});
		expect(modelsHeadline(withModels(overview, { loaded: 0 })).lead).toEqual({
			kind: "idle",
			models: 3,
		});
	});

	test("GPU memory is the busiest GPU that reports it; unknown memory is no share", () => {
		const system = overviewOf(gpuBoxModels()).system;
		expect(
			gpuMemoryPercent({
				...system,
				gpus: [
					...system.gpus,
					{
						name: "Intel UHD Graphics 770",
						backend: "vulkan",
						memory_total: null,
						memory_free: null,
					},
					{
						name: "Second card",
						backend: "vulkan",
						memory_total: 8 * GIB,
						memory_free: 2 * GIB,
					},
				],
			}),
		).toBe(75);
		expect(gpuMemoryPercent(overviewOf(emptyModels()).system)).toBeUndefined();
	});
});

describe("models", () => {
	test("now-tier recommendations badge the tab", () => {
		expect(nowCount(overviewOf(gpuBoxModels()).recommendations)).toBe(2);
		expect(nowCount(overviewOf(macMiniModels()).recommendations)).toBe(0);
	});

	test("an engine runs on its installed GPU runtime, else on what is installed", () => {
		const gpu = gpuBoxModels().runtimes;
		expect(backendOf("llamacpp", gpu)).toBe("vulkan");
		expect(backendOf("onnx", gpu)).toBe("cpu");
		const mac = macMiniModels().runtimes;
		expect(backendOf("llamacpp", mac)).toBe("cpu");
		expect(backendOf("mlx", mac)).toBe("metal");
		expect(backendOf("llamacpp", emptyModels().runtimes)).toBeUndefined();
	});

	test("the day window covers the current hour and the 23 before it", () => {
		const nowS = 1_790_769_600 + 125;
		const window = dayWindow(nowS);
		expect(window.step).toBe("hour");
		expect((window.to - window.from) / 3600).toBe(24);
		expect(window.from).toBeLessThanOrEqual(nowS);
		expect(window.to).toBeGreaterThan(nowS);
		expect(window.to % 3600).toBe(0);
	});
});

describe("usage", () => {
	const sample = gpuBoxModels();
	const day = (modelId: string) =>
		statsOf(sample, { modelId, ...dayWindow(1_790_769_600) });

	test("speed is generated tokens over decode time; embeddings have none", () => {
		const chat = usageOf(day("qwen3-8b"));
		expect(chat.requests).toBe(1_088);
		expect(Math.round(chat.tokensPerSecond ?? 0)).toBe(86);
		expect(usageOf(day("nomic-embed-v1.5")).tokensPerSecond).toBeUndefined();
		expect(usageOf(day("gemma-3-4b")).requests).toBe(0);
	});

	test("callers: the viewer, the owner, people with access by name, services; busiest first", () => {
		const consumers = usageOf(day("qwen3-8b")).consumers;
		const grantUser = (grantId: string) =>
			grantId === SAMPLE_MODEL_GRANT ? "usr_mira" : undefined;
		expect(
			consumersOf(consumers, { owner: true, grantUser }).map(
				(consumer) => consumer.kind,
			),
		).toEqual(["you", "service", "person"]);
		const asMira = consumersOf(consumers, {
			owner: false,
			myGrantId: SAMPLE_MODEL_GRANT,
			grantUser,
		});
		expect(asMira.map((consumer) => consumer.kind)).toEqual([
			"owner",
			"service",
			"you",
		]);
		expect(
			consumersOf(consumers, { owner: false, grantUser: () => undefined }).at(
				-1,
			),
		).toEqual({ key: `grant:${SAMPLE_MODEL_GRANT}`, kind: "person" });
	});
});

describe("downloads", () => {
	const jobs = gpuBoxModels().jobs;
	const fetching = jobs.find(
		(job) => job.job_id === SAMPLE_JOB_IDS.fetching,
	) as ModelJob;

	test("a running download has its share, bytes and time left at the current rate", () => {
		const progress = jobProgress(fetching);
		expect(Math.round(progress.percent ?? 0)).toBe(43);
		expect(progress.bytes).toBe(1_073_741_824);
		expect(progress.etaS).toBe(Math.ceil((2_489_909_536 - GIB) / 52_428_800));
	});

	test("a failed or queued download has no progress; finished ones are whole", () => {
		const blocked = jobs.find(
			(job) => job.job_id === SAMPLE_JOB_IDS.blocked,
		) as ModelJob;
		expect(jobProgress(blocked)).toEqual({});
		expect(
			jobProgress({ ...fetching, state: "present" } as ModelJob).percent,
		).toBe(100);
	});

	test("active downloads come first, then failed ones, then finished ones", () => {
		const done = {
			...jobs[1],
			job_id: "11111111-2222-4333-8444-555555555555",
			state: "present",
		} as ModelJob;
		expect(
			orderJobs([done, ...[...jobs].reverse()]).map((job) => job.state),
		).toEqual(["fetching", "failed", "present"]);
	});
});

describe("settings and residency", () => {
	test("settings compare field by field, as the device does: order doesn't matter, a left-out field only equals a left-out one", () => {
		expect(
			sameSettings(
				{ parallel: 4, ctx_per_slot: 8_192, gpu_layers: "auto" },
				{ gpu_layers: "auto", ctx_per_slot: 8_192, parallel: 4 },
			),
		).toBe(true);
		expect(sameSettings({ parallel: 4 }, { parallel: 4, threads: 8 })).toBe(
			false,
		);
		expect(
			sameSettings({ gpu_layers: { count: 8 } }, { gpu_layers: "auto" }),
		).toBe(false);
	});

	test("residencies differ by mode, and on demand also by its idle time", () => {
		const onDemand = (seconds: number) =>
			({ mode: "on_demand", idle_unload_after_seconds: seconds }) as const;
		expect(sameResidency(onDemand(900), onDemand(900))).toBe(true);
		expect(sameResidency(onDemand(900), onDemand(600))).toBe(false);
		expect(sameResidency({ mode: "always_on" }, onDemand(900))).toBe(false);
		expect(sameResidency({ mode: "pinned_off" }, { mode: "pinned_off" })).toBe(
			true,
		);
	});
});
