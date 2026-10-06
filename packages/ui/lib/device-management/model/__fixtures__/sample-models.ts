/*
 * Model hosting samples (plan §3.4–§3.7) on the sample fleet's clock. A
 * `ModelHostSample` is what one device's model host holds; the fake agent
 * answers every `models` request from one, and screens and unit tests use the
 * answers built here (`overviewOf`, `statsOf`) directly.
 *
 *   gpuBoxModels()   Linux x64, RTX 4090: Qwen3-8B on llama.cpp/Vulkan, an ONNX
 *                    embedding model, Gemma 3 downloading with one blocked file
 *   macMiniModels()  Mac mini M4 (unified memory, Metal): MLX chat + GGUF embeddings on the CPU
 *   crowdedMacModels()  the Mac mini plus twelve MLX models: the overview sets `next`
 *   emptyModels()    a CPU-only box that hosts nothing yet
 *   PRE_MODEL_FEATURES  an agent from before model hosting
 */
import {
	type HostedModel,
	MODELS_PAGE_MAX,
	MODEL_OVERVIEW_MAX_RECOMMENDATIONS,
	type ModelJob,
	type ModelStats,
	type ModelsOverview,
	type ModelsSummary,
	type Recommendation,
	type RuntimeInfo,
	STATS_STEPS,
	type StatsStep,
	type SystemFacts,
} from "../../models";
import { AGENT_FEATURES, type AgentFeatures } from "../types";
import { SAMPLE_NOW } from "./sample-fleet";

const GIB = 1024 ** 3;
const HOUR = STATS_STEPS.hour;
const DAY_HOURS = 24;

/** The hour `SAMPLE_NOW` falls in; usage samples end with it. */
export const SAMPLE_MODELS_HOUR = Math.floor(SAMPLE_NOW / HOUR) * HOUR;
/** Mira's whole-device grant in the sample edge policy. */
export const SAMPLE_MODEL_GRANT = "b7a0e44c-72e4-4b87-9b5d-3a98b09cdd6f";

/** Every flag of the sample fleet's agent: no model hosting. */
export const PRE_MODEL_FEATURES = Object.fromEntries(
	AGENT_FEATURES.filter((flag) => !flag.startsWith("model_")).map((flag) => [
		flag,
		1,
	]),
) as AgentFeatures;
export const MODEL_HOST_FEATURES: AgentFeatures = {
	...PRE_MODEL_FEATURES,
	model_store: 1,
	model_host: 1,
	model_runtime_llamacpp: 1,
	model_runtime_onnx: 1,
};
export const MAC_MODEL_HOST_FEATURES: AgentFeatures = {
	...MODEL_HOST_FEATURES,
	model_runtime_mlx: 1,
};

export interface ModelHostSample {
	system: SystemFacts;
	runtime_manifest_url?: string;
	runtimes: RuntimeInfo[];
	/** Any order; answers list them by id. */
	models: HostedModel[];
	jobs: ModelJob[];
	/** Most urgent first. */
	recommendations: Recommendation[];
	/** Hourly usage per model id for the 24 hours ending with `SAMPLE_MODELS_HOUR`. */
	usage: Record<string, ModelStats>;
	store: { bytes: number; budget: number };
}

/** The asset digests the samples name, by file. */
export const SAMPLE_DIGESTS = {
	qwen8: { algorithm: "blake3", hex: "1".repeat(64) },
	nomicModel: { algorithm: "sha256", hex: "2".repeat(64) },
	nomicTokenizer: { algorithm: "sha256", hex: "3".repeat(64) },
	gemmaModel: { algorithm: "sha256", hex: "4".repeat(64) },
	gemmaProjector: { algorithm: "sha256", hex: "5".repeat(64) },
	qwen4: { algorithm: "sha256", hex: "6".repeat(64) },
	bge: { algorithm: "blake3", hex: "7".repeat(64) },
} as const satisfies Record<string, HostedModel["assets"][number]>;

/** Job ids of the GPU box's downloads. */
export const SAMPLE_JOB_IDS = {
	fetching: "3f9a6c1e-2b4d-4c8e-9f1a-7b6c5d4e3f21",
	blocked: "8d2e4f6a-1c3b-4d5e-8f7a-9b0c1d2e3f40",
} as const;

/* Usage. */

interface UsageProfile {
	/** Requests per hour, oldest first, 24 values. */
	requests: readonly number[];
	promptPerRequest: number;
	completionPerRequest: number;
	/** Decode speed in tokens per second; 0 for embeddings. */
	tokensPerSecond: number;
	/** Requests per hour from which the queue builds up. */
	busyFrom: number;
	ttft: { p50: number; p95: number; busyP95: number };
	/** Shares of the requests, summing to 1. */
	consumers: readonly [ModelStats["consumers"][number]["consumer"], number][];
}

function usageOf(modelId: string, profile: UsageProfile): ModelStats {
	const busy = profile.requests.map((count) => count >= profile.busyFrom);
	const prompt = profile.requests.map(
		(count) => count * profile.promptPerRequest,
	);
	const completion = profile.requests.map(
		(count) => count * profile.completionPerRequest,
	);
	const total = profile.requests.reduce((sum, count) => sum + count, 0);
	const errors = busy.map(Number);
	const errorTotal = errors.reduce((sum, count) => sum + count, 0);
	return {
		model_id: modelId,
		from: SAMPLE_MODELS_HOUR - (DAY_HOURS - 1) * HOUR,
		step: "hour",
		series: {
			requests: [...profile.requests],
			errors,
			prompt_tokens: prompt,
			completion_tokens: completion,
			cached_tokens: prompt.map((tokens) => Math.round(tokens * 0.45)),
			decode_ms: completion.map((tokens) =>
				profile.tokensPerSecond
					? Math.round((tokens * 1000) / profile.tokensPerSecond)
					: 0,
			),
			ttft_p50_ms: profile.requests.map((count) =>
				count ? profile.ttft.p50 : null,
			),
			ttft_p95_ms: profile.requests.map((count, index) =>
				count ? (busy[index] ? profile.ttft.busyP95 : profile.ttft.p95) : null,
			),
			queue_wait_p95_ms: profile.requests.map((count, index) =>
				count ? (busy[index] ? 1_800 : 0) : null,
			),
		},
		consumers: profile.consumers.map(([consumer, share], index) => ({
			consumer,
			requests: Math.round(total * share),
			errors: index === 0 ? errorTotal : 0,
			prompt_tokens: Math.round(
				prompt.reduce((sum, tokens) => sum + tokens, 0) * share,
			),
			completion_tokens: Math.round(
				completion.reduce((sum, tokens) => sum + tokens, 0) * share,
			),
		})),
	};
}

const DAYTIME = [
	14, 9, 6, 4, 3, 5, 11, 26, 47, 70, 84, 91, 88, 93, 96, 90, 81, 69, 54, 44, 37,
	29, 21, 16,
] as const;
const times = (factor: number) =>
	DAYTIME.map((count) => Math.round(count * factor));

/* The GPU box. */

const QWEN8_SETTINGS = {
	ctx_per_slot: 8_192,
	parallel: 4,
	gpu_layers: "auto",
	flash_attn: true,
} as const satisfies HostedModel["settings"];
const ON_DEMAND = {
	mode: "on_demand",
	idle_unload_after_seconds: 900,
} as const;

const pack = (
	runtime: RuntimeInfo["runtime"],
	backend: RuntimeInfo["backend"],
	build: string,
	installed: boolean,
	size: number,
): RuntimeInfo => ({ runtime, backend, build, installed, size });

function gpuBoxSystem(): SystemFacts {
	return {
		cpu: {
			brand: "AMD Ryzen 9 7950X 16-Core Processor",
			arch: "x86_64",
			features: ["avx2", "avx512f", "fma"],
			physical_cores: 16,
		},
		ram: { total: 64 * GIB, free: 38 * GIB },
		gpus: [
			{
				name: "NVIDIA GeForce RTX 4090",
				backend: "vulkan",
				memory_total: 24 * GIB,
				memory_free: 17 * GIB,
			},
		],
		model_volume: { total: 2_000_398_934_016, free: 1_420_000_000_000 },
	};
}

function gpuBoxHosted(): HostedModel[] {
	return [
		{
			id: "qwen3-8b",
			display_name: "Qwen3-8B Q4_K_M",
			kind: "chat",
			engine: "llamacpp",
			assets: [SAMPLE_DIGESTS.qwen8],
			settings: QWEN8_SETTINGS,
			residency: { mode: "always_on" },
			revision: 3,
			state: "loaded",
			ram_bytes: 966_367_641,
			vram_bytes: 6_871_947_673,
			slots: 4,
			slots_busy: 3,
		},
		{
			id: "nomic-embed-v1.5",
			display_name: "Nomic Embed v1.5",
			kind: "embedding",
			engine: "onnx",
			assets: [SAMPLE_DIGESTS.nomicModel, SAMPLE_DIGESTS.nomicTokenizer],
			settings: {},
			residency: ON_DEMAND,
			revision: 2,
			state: "loaded",
			ram_bytes: 590_558_003,
			vram_bytes: 0,
			slots: 1,
			slots_busy: 0,
		},
		{
			id: "gemma-3-4b",
			display_name: "Gemma 3 4B Q4_K_M",
			kind: "vision",
			engine: "llamacpp",
			assets: [SAMPLE_DIGESTS.gemmaModel, SAMPLE_DIGESTS.gemmaProjector],
			settings: { threads: 8 },
			residency: ON_DEMAND,
			revision: 1,
			state: "acquiring",
		},
	];
}

function gpuBoxJobs(): ModelJob[] {
	return [
		{
			job_id: SAMPLE_JOB_IDS.fetching,
			digest: SAMPLE_DIGESTS.gemmaModel,
			size: 2_489_909_536,
			file_name: "gemma-3-4b-it-Q4_K_M.gguf",
			source_host: "cdn.flow-like.com",
			bytes_per_second: 52_428_800,
			updated_at: SAMPLE_NOW - 2,
			state: "fetching",
			source_index: 0,
			bytes: 1_073_741_824,
		},
		{
			job_id: SAMPLE_JOB_IDS.blocked,
			digest: SAMPLE_DIGESTS.gemmaProjector,
			size: 851_251_104,
			file_name: "mmproj-gemma-3-4b-it-f16.gguf",
			source_host: "huggingface.co",
			updated_at: SAMPLE_NOW - 600,
			state: "failed",
			reason: "egress_blocked",
		},
	];
}

function gpuBoxRecommendations(): Recommendation[] {
	const qwen = { model_id: "qwen3-8b", expected_revision: 3 } as const;
	const alwaysOn = { mode: "always_on" } as const;
	return [
		{
			code: "kv_pressure",
			tier: "now",
			model_id: qwen.model_id,
			params: { kv_percent: 93 },
			fix: {
				kind: "configure",
				...qwen,
				settings: { ...QWEN8_SETTINGS, kv_cache_type: "q8_0" },
				residency: alwaysOn,
			},
		},
		{
			code: "requests_queued",
			tier: "now",
			model_id: qwen.model_id,
			params: { deferred: 3, queue_p95_ms: 1_800 },
			fix: {
				kind: "configure",
				...qwen,
				settings: { ...QWEN8_SETTINGS, parallel: 6 },
				residency: alwaysOn,
			},
		},
		{
			code: "cpu_threads",
			tier: "later",
			model_id: "gemma-3-4b",
			params: { threads: 8, physical_cores: 16 },
			fix: {
				kind: "configure",
				model_id: "gemma-3-4b",
				expected_revision: 1,
				settings: {},
				residency: ON_DEMAND,
			},
		},
	];
}

function gpuBoxUsage(): ModelHostSample["usage"] {
	return {
		"qwen3-8b": usageOf("qwen3-8b", {
			requests: DAYTIME,
			promptPerRequest: 620,
			completionPerRequest: 390,
			tokensPerSecond: 86,
			busyFrom: 84,
			ttft: { p50: 190, p95: 640, busyP95: 1_900 },
			consumers: [
				[{ kind: "owner" }, 0.55],
				[{ kind: "grant", grant_id: SAMPLE_MODEL_GRANT }, 0.15],
				[{ kind: "placement", placement_id: "invoice-extractor" }, 0.3],
			],
		}),
		"nomic-embed-v1.5": usageOf("nomic-embed-v1.5", {
			requests: times(3),
			promptPerRequest: 180,
			completionPerRequest: 0,
			tokensPerSecond: 0,
			busyFrom: Number.POSITIVE_INFINITY,
			ttft: { p50: 18, p95: 45, busyP95: 45 },
			consumers: [
				[{ kind: "placement", placement_id: "invoice-extractor" }, 1],
			],
		}),
	};
}

export function gpuBoxModels(): ModelHostSample {
	return {
		system: gpuBoxSystem(),
		runtimes: [
			pack("llamacpp", "vulkan", "b10809", true, 87_031_808),
			pack("llamacpp", "cpu", "b10809", false, 16_734_586),
		],
		models: gpuBoxHosted(),
		jobs: gpuBoxJobs(),
		recommendations: gpuBoxRecommendations(),
		usage: gpuBoxUsage(),
		store: { bytes: 5_575_494_884, budget: 1_278_000_000_000 },
	};
}

/* The Mac mini. */

function macMiniSystem(): SystemFacts {
	return {
		cpu: {
			brand: "Apple M4",
			arch: "aarch64",
			features: ["neon", "dotprod", "i8mm"],
			physical_cores: 10,
		},
		ram: { total: 16 * GIB, free: 5_905_580_032 },
		gpus: [
			{
				name: "Apple M4",
				backend: "metal",
				memory_total: 11_453_251_584,
				memory_free: 7_516_192_768,
			},
		],
		model_volume: { total: 494_384_795_648, free: 181_000_000_000 },
	};
}

function macMiniHosted(): HostedModel[] {
	return [
		{
			id: "qwen3-4b-mlx",
			display_name: "Qwen3-4B MLX 4-bit",
			kind: "chat",
			engine: "mlx",
			assets: [SAMPLE_DIGESTS.qwen4],
			settings: { ctx_per_slot: 8_192 },
			residency: { mode: "always_on" },
			revision: 1,
			state: "loaded",
			ram_bytes: 2_791_728_742,
			vram_bytes: 0,
			slots: 1,
			slots_busy: 1,
		},
		{
			id: "bge-small-en-v1.5",
			display_name: "BGE Small EN v1.5 Q8_0",
			kind: "embedding",
			engine: "llamacpp",
			assets: [SAMPLE_DIGESTS.bge],
			settings: {},
			residency: ON_DEMAND,
			revision: 1,
			state: "loaded",
			ram_bytes: 150_994_944,
			vram_bytes: 0,
			slots: 2,
			slots_busy: 0,
		},
	];
}

/** `gpu_unused` is device-wide: the agent names the GPU, never a model. */
function macMiniRecommendations(): Recommendation[] {
	return [
		{
			code: "gpu_unused",
			tier: "soon",
			params: { gpu: "Apple M4" },
			fix: { kind: "install_runtime", runtime: "llamacpp", backend: "metal" },
		},
		{
			code: "ctx_truncation",
			tier: "later",
			model_id: "qwen3-4b-mlx",
			params: {
				percent: 12,
				ctx: 8_192,
				suggested_ctx: 16_384,
				extra_bytes: GIB,
			},
			fix: {
				kind: "configure",
				model_id: "qwen3-4b-mlx",
				expected_revision: 1,
				settings: { ctx_per_slot: 16_384 },
				residency: { mode: "always_on" },
			},
		},
	];
}

function macMiniUsage(): ModelHostSample["usage"] {
	return {
		"qwen3-4b-mlx": usageOf("qwen3-4b-mlx", {
			requests: times(0.2),
			promptPerRequest: 540,
			completionPerRequest: 310,
			tokensPerSecond: 38,
			busyFrom: Number.POSITIVE_INFINITY,
			ttft: { p50: 260, p95: 720, busyP95: 720 },
			consumers: [[{ kind: "owner" }, 1]],
		}),
		"bge-small-en-v1.5": usageOf("bge-small-en-v1.5", {
			requests: times(0.8),
			promptPerRequest: 120,
			completionPerRequest: 0,
			tokensPerSecond: 0,
			busyFrom: Number.POSITIVE_INFINITY,
			ttft: { p50: 30, p95: 70, busyP95: 70 },
			consumers: [[{ kind: "placement", placement_id: "field-notes" }, 1]],
		}),
	};
}

export function macMiniModels(): ModelHostSample {
	return {
		system: macMiniSystem(),
		runtimes: [
			pack("mlx", "metal", "0.29.1", true, 154_140_672),
			pack("llamacpp", "cpu", "b10809", true, 31_457_280),
			pack("llamacpp", "metal", "b10809", false, 11_123_196),
		],
		models: macMiniHosted(),
		jobs: [],
		recommendations: macMiniRecommendations(),
		usage: macMiniUsage(),
		store: { bytes: 2_463_000_000, budget: 162_900_000_000 },
	};
}

/** The Mac mini with twelve more MLX models of twelve files each: more than one overview carries. */
export function crowdedMacModels(): ModelHostSample {
	const sample = macMiniModels();
	for (let index = 0; index < 12; index++)
		sample.models.push({
			id: `mlx-model-${String(index + 1).padStart(2, "0")}`,
			display_name: `MLX Model ${index + 1}`,
			kind: "chat",
			engine: "mlx",
			assets: Array.from({ length: 12 }, (_, file) => ({
				algorithm: "sha256",
				hex: (index * 12 + file).toString(16).padStart(64, "a"),
			})),
			settings: {},
			residency: ON_DEMAND,
			revision: 1,
			state: "stopped",
		});
	return sample;
}

/* Nothing hosted yet. */

export function emptyModels(): ModelHostSample {
	return {
		system: {
			cpu: {
				brand: "Intel(R) Core(TM) i5-1235U",
				arch: "x86_64",
				features: ["avx2", "fma"],
				physical_cores: 10,
			},
			ram: { total: 16 * GIB, free: 11 * GIB },
			gpus: [],
			model_volume: { total: 256_060_514_304, free: 190_000_000_000 },
		},
		runtimes: [pack("llamacpp", "cpu", "b10809", false, 16_734_586)],
		models: [],
		jobs: [],
		recommendations: [],
		usage: {},
		store: { bytes: 0, budget: 171_000_000_000 },
	};
}

/* Answers. */

const sum = (values: readonly number[]) =>
	values.reduce((total, value) => total + value, 0);

const byId = (a: HostedModel, b: HostedModel) => (a.id < b.id ? -1 : 1);

const TIER_ORDER: Record<Recommendation["tier"], number> = {
	now: 0,
	soon: 1,
	later: 2,
};

/** Most urgent first; the device keeps its own order within a tier. */
export function rankRecommendations(
	recommendations: readonly Recommendation[],
): Recommendation[] {
	return recommendations
		.map((recommendation, index) => ({ recommendation, index }))
		.sort(
			(a, b) =>
				TIER_ORDER[a.recommendation.tier] - TIER_ORDER[b.recommendation.tier] ||
				a.index - b.index,
		)
		.map(({ recommendation }) => recommendation);
}

export function summaryOf(sample: ModelHostSample): ModelsSummary {
	const usage = Object.values(sample.usage);
	const column = (pick: (stats: ModelStats) => readonly number[]) =>
		sum(usage.map((stats) => sum(pick(stats))));
	return {
		models: sample.models.length,
		loaded: sample.models.filter((model) => model.state === "loaded").length,
		failed: sample.models.filter((model) => model.state === "failed").length,
		requests_24h: column((stats) => stats.series.requests),
		tokens_24h:
			column((stats) => stats.series.prompt_tokens) +
			column((stats) => stats.series.completion_tokens),
		errors_24h: column((stats) => stats.series.errors),
		store_bytes: sample.store.bytes,
		store_budget_bytes: sample.store.budget,
	};
}

/** One encrypted reply, less what its envelope needs (the agent's `fit_models`). */
const OVERVIEW_BYTES = 16 * 1024 - 512;
const jsonBytes = (value: unknown) =>
	new TextEncoder().encode(JSON.stringify(value)).length;

/**
 * The `overview` answer of a host holding `sample`, read at `observedAt`: like
 * the agent, the first models by id that fit one reply next to the rest, and
 * `next` naming the last one when more follow.
 */
export function overviewOf(
	sample: ModelHostSample,
	observedAt = SAMPLE_NOW,
): ModelsOverview {
	const models = [...sample.models].sort(byId);
	const overview: ModelsOverview = {
		observed_at: observedAt,
		...(sample.runtime_manifest_url
			? { runtime_manifest_url: sample.runtime_manifest_url }
			: {}),
		system: sample.system,
		runtimes: sample.runtimes,
		summary: summaryOf(sample),
		recommendations: rankRecommendations(sample.recommendations).slice(
			0,
			MODEL_OVERVIEW_MAX_RECOMMENDATIONS,
		),
		models: [],
		next: null,
	};
	let budget = OVERVIEW_BYTES - jsonBytes(overview);
	for (const model of models.slice(0, MODELS_PAGE_MAX)) {
		const bytes = jsonBytes(model) + 1;
		if (bytes > budget) break;
		budget -= bytes;
		overview.models.push(model);
	}
	if (overview.models.length < models.length)
		overview.next = overview.models.at(-1)?.id ?? null;
	return overview;
}

export interface StatsRequest {
	modelId: string | null;
	from: number;
	to: number;
	step: StatsStep;
}

type Series = ModelStats["series"];
const COUNTS = [
	"requests",
	"errors",
	"prompt_tokens",
	"completion_tokens",
	"cached_tokens",
	"decode_ms",
] as const satisfies readonly (keyof Series)[];
const TIMINGS = [
	"ttft_p50_ms",
	"ttft_p95_ms",
	"queue_wait_p95_ms",
] as const satisfies readonly (keyof Series)[];

function emptySeries(): Series {
	return {
		requests: [],
		errors: [],
		prompt_tokens: [],
		completion_tokens: [],
		cached_tokens: [],
		decode_ms: [],
		ttft_p50_ms: [],
		ttft_p95_ms: [],
		queue_wait_p95_ms: [],
	};
}

/** One requested bucket from the hourly samples: a minute gets a sixtieth of its hour. */
function bucket(samples: readonly ModelStats[], at: number, step: StatsStep) {
	const share = step === "minute" ? HOUR / STATS_STEPS.minute : 1;
	const counts = Object.fromEntries(
		COUNTS.map((column) => [
			column,
			sum(
				samples.map((stats) => {
					const index = Math.floor((at - stats.from) / HOUR);
					return Math.floor((stats.series[column][index] ?? 0) / share);
				}),
			),
		]),
	) as Record<(typeof COUNTS)[number], number>;
	const timings = Object.fromEntries(
		TIMINGS.map((column) => {
			const values = samples.flatMap((stats) => {
				const value =
					stats.series[column][Math.floor((at - stats.from) / HOUR)];
				return typeof value === "number" ? [value] : [];
			});
			return [column, values.length ? Math.max(...values) : null];
		}),
	) as Record<(typeof TIMINGS)[number], number | null>;
	return { ...counts, ...timings };
}

function consumersOf(
	samples: readonly ModelStats[],
	share: number,
): ModelStats["consumers"] {
	const merged = new Map<string, ModelStats["consumers"][number]>();
	for (const row of samples.flatMap((stats) => stats.consumers)) {
		const key = JSON.stringify(row.consumer);
		const known = merged.get(key);
		merged.set(key, {
			consumer: row.consumer,
			requests: (known?.requests ?? 0) + row.requests,
			errors: (known?.errors ?? 0) + row.errors,
			prompt_tokens: (known?.prompt_tokens ?? 0) + row.prompt_tokens,
			completion_tokens:
				(known?.completion_tokens ?? 0) + row.completion_tokens,
		});
	}
	return [...merged.values()]
		.map((row) => ({
			...row,
			requests: Math.round(row.requests * share),
			errors: Math.round(row.errors * share),
			prompt_tokens: Math.round(row.prompt_tokens * share),
			completion_tokens: Math.round(row.completion_tokens * share),
		}))
		.filter((row) => row.requests > 0);
}

/** The `stats` answer for `request`: zeros and no timings outside the sampled day. */
export function statsOf(
	sample: ModelHostSample,
	request: StatsRequest,
): ModelStats {
	const samples = request.modelId
		? [sample.usage[request.modelId]].filter(
				(stats): stats is ModelStats => stats !== undefined,
			)
		: Object.values(sample.usage);
	const width = STATS_STEPS[request.step];
	const series = emptySeries();
	for (let at = request.from; at < request.to; at += width) {
		const values = bucket(samples, at, request.step);
		for (const column of COUNTS) series[column].push(values[column]);
		for (const column of TIMINGS) series[column].push(values[column]);
	}
	const sampled = sum(samples.map((stats) => sum(stats.series.requests)));
	return {
		model_id: request.modelId,
		from: request.from,
		step: request.step,
		series,
		consumers: consumersOf(
			samples,
			sampled ? sum(series.requests) / sampled : 0,
		),
	};
}
