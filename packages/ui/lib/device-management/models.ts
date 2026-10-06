import { z } from "zod";
import { type AgentRead, agentSupports } from "./agent-reads";
import type { AgentFeature, AgentFeatures } from "./model/types";
import type { ManagementCall } from "./telemetry";
import { rejectionMessage } from "./transport";
import { managementRejection } from "./types";
import { LiveCallError, rejectionCode } from "./workspace/errors";

/*
 * The `models` management command (plan §3.5), mirroring
 * `packages/device-protocol/src/models.rs`. Every answer fits one 16 KiB reply
 * except `stats`, a bulk read: pass a `call` that reads over a data stream.
 * Hosted models, asset statuses and jobs carry their state flat.
 */

export const MODELS_PAGE_MAX = 32;
export const MODEL_MAX_ASSETS = 32;
export const MODEL_ASSET_MAX_SOURCES = 8;
export const MODEL_ASSET_SOURCE_MAX_LEN = 2048;
export const MODEL_DISPLAY_NAME_MAX = 128;
export const MODEL_MIN_CTX_PER_SLOT = 256;
export const MODEL_MAX_CTX_PER_SLOT = 1 << 20;
export const MODEL_MAX_PARALLEL = 64;
export const MODEL_MAX_THREADS = 1024;
export const MODEL_MAX_GPU_LAYERS = 1024;
export const MODEL_MIN_IDLE_UNLOAD_SECONDS = 60;
export const MODEL_MAX_IDLE_UNLOAD_SECONDS = 7 * 86_400;
export const MODEL_DEFAULT_IDLE_UNLOAD_SECONDS = 15 * 60;
export const MODEL_ENSURE_MAX_PINS = 32;
export const MODEL_MAX_PENDING_ASSETS = 32;
export const MODEL_STATS_MAX_POINTS = 2_160;
export const MODEL_STATS_MAX_CONSUMERS = 64;
export const MODEL_MAX_GPUS = 8;
export const MODEL_MAX_CPU_FEATURES = 32;
export const MODEL_MAX_RUNTIMES = 8;
export const MODEL_OVERVIEW_MAX_RECOMMENDATIONS = 3;
export const MODEL_RECOMMENDATION_MAX_PARAMS = 8;

export const MODEL_KINDS = ["chat", "vision", "embedding"] as const;
export const MODEL_ENGINES = ["llamacpp", "mlx", "onnx"] as const;
export const MODEL_RUNTIMES = ["llamacpp", "mlx"] as const;
export const MODEL_BACKENDS = ["cpu", "vulkan", "metal", "cuda"] as const;
export const MODEL_POOLINGS = ["mean", "cls", "last"] as const;
export const KV_CACHE_TYPES = ["f16", "q8_0", "q4_0"] as const;
export const STATS_STEPS = { minute: 60, hour: 3_600 } as const;
export const RECOMMENDATION_CODES = [
	"gpu_unused",
	"partial_offload",
	"requests_queued",
	"kv_pressure",
	"ctx_truncation",
	"memory_pressure",
	"idle_resident",
	"slow_ttft",
	"cpu_threads",
	"disk_low",
	"runtime_outdated",
	"container_gpu_hidden",
] as const;
export const RECOMMENDATION_TIERS = ["now", "soon", "later"] as const;

export type ModelKind = (typeof MODEL_KINDS)[number];
export type ModelEngine = (typeof MODEL_ENGINES)[number];
export type ModelRuntime = (typeof MODEL_RUNTIMES)[number];
export type ModelBackend = (typeof MODEL_BACKENDS)[number];
export type ModelPooling = (typeof MODEL_POOLINGS)[number];
export type StatsStep = keyof typeof STATS_STEPS;

const count = z.number().int().nonnegative().max(Number.MAX_SAFE_INTEGER);
const time = z.number().int().safe();
const u8 = z.number().int().min(0).max(255);
const u16 = z.number().int().min(0).max(65_535);
const u32 = z.number().int().min(0).max(4_294_967_295);
const managementId = z.string().regex(/^[A-Za-z0-9_:.-]{1,128}$/u);
const jobId = z
	.string()
	.regex(/^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/u);
const token = z.string().regex(/^[a-z0-9_.]{1,32}$/u);
const label = (max: number) => z.string().min(1).max(max);
const cursor = managementId.nullable();

/*
 * Agents update on their own schedule: a value this client doesn't know is
 * dropped, never fatal, wherever the client only shows it. A list leaves out
 * the entries it can't read; a failure reason reads as "unknown".
 */
function lenientArray<T extends z.ZodTypeAny>(element: T, max: number) {
	return z
		.array(z.unknown())
		.max(max)
		.transform((items) =>
			items.flatMap((item): z.output<T>[] => {
				const parsed = element.safeParse(item);
				return parsed.success ? [parsed.data] : [];
			}),
		);
}

const reasonOf = <const T extends readonly [string, ...string[]]>(known: T) =>
	z.enum([...known, "unknown"]).catch("unknown");

export const modelAssetDigestSchema = z.object({
	algorithm: z.enum(["sha256", "blake3"]),
	hex: z.string().regex(/^[0-9a-f]{64}$/u),
});
export type ModelAssetDigest = z.infer<typeof modelAssetDigestSchema>;

export interface ModelAssetDescriptor {
	digest: ModelAssetDigest;
	size: number;
	/** Path inside the model directory, e.g. `model.gguf`. */
	file_name: string;
	/** Public HTTPS sources in the order the device tries them. */
	sources?: string[];
}

export const modelAssetStateSchema = z.discriminatedUnion("state", [
	z.object({ state: z.literal("queued") }),
	z.object({ state: z.literal("fetching"), source_index: u8, bytes: count }),
	z.object({ state: z.literal("verifying") }),
	z.object({ state: z.literal("present") }),
	z.object({ state: z.literal("awaiting_push"), bytes: count }),
	z.object({
		state: z.literal("failed"),
		reason: reasonOf([
			"egress_blocked",
			"http_status",
			"digest_mismatch",
			"size_mismatch",
			"disk_budget",
			"no_sources",
			"cancelled",
			"io",
		]),
		http_status: u16.optional(),
	}),
]);
export type ModelAssetState = z.infer<typeof modelAssetStateSchema>;

export const modelSettingsSchema = z.object({
	ctx_per_slot: z
		.number()
		.int()
		.min(MODEL_MIN_CTX_PER_SLOT)
		.max(MODEL_MAX_CTX_PER_SLOT)
		.optional(),
	parallel: z.number().int().min(1).max(MODEL_MAX_PARALLEL).optional(),
	kv_cache_type: z.enum(KV_CACHE_TYPES).optional(),
	/** `"auto"` fits layers to free GPU memory; `{count: 0}` keeps the CPU. */
	gpu_layers: z
		.union([
			z.literal("auto"),
			z.object({ count: z.number().int().min(0).max(MODEL_MAX_GPU_LAYERS) }),
		])
		.optional(),
	threads: z.number().int().min(1).max(MODEL_MAX_THREADS).optional(),
	flash_attn: z.boolean().optional(),
});
/** A field left out takes the device's recommended value. */
export type ModelSettings = z.infer<typeof modelSettingsSchema>;

export const residencySchema = z.discriminatedUnion("mode", [
	z.object({ mode: z.literal("always_on") }),
	z.object({
		mode: z.literal("on_demand"),
		idle_unload_after_seconds: z
			.number()
			.int()
			.min(MODEL_MIN_IDLE_UNLOAD_SECONDS)
			.max(MODEL_MAX_IDLE_UNLOAD_SECONDS),
	}),
	z.object({ mode: z.literal("pinned_off") }),
]);
export type Residency = z.infer<typeof residencySchema>;

export interface ModelSpec {
	display_name: string;
	kind: ModelKind;
	engine: ModelEngine;
	/** In load order; split GGUF parts run first to last. */
	assets: ModelAssetDescriptor[];
	/** File name of the asset llama.cpp loads with `--mmproj`; required for llama.cpp vision models. */
	projector?: string;
	/** Embedding models only. */
	pooling?: ModelPooling;
}

/** Sent as `{type: "models", request}`. Writes are journaled by the envelope's operation ID. */
export type ModelsRequest =
	| { kind: "overview" }
	| { kind: "models"; after?: string | null; limit?: number }
	| {
			kind: "stats";
			model_id?: string | null;
			from: number;
			to: number;
			step: StatsStep;
	  }
	| { kind: "jobs"; after?: string | null; limit?: number }
	| { kind: "recommendations"; after?: string | null; limit?: number }
	| { kind: "probe" }
	| {
			kind: "install";
			model_id: string;
			model: ModelSpec;
			settings?: ModelSettings;
			residency?: Residency;
	  }
	| {
			kind: "configure";
			model_id: string;
			expected_revision: number;
			settings: ModelSettings;
			residency: Residency;
	  }
	| { kind: "load"; model_id: string }
	| { kind: "unload"; model_id: string }
	| { kind: "remove"; model_id: string; expected_revision: number }
	| {
			kind: "ensure";
			project_id: string;
			pins: { bit_id: string; metadata_sha256: string }[];
	  }
	| { kind: "install_runtime"; runtime: ModelRuntime; backend: ModelBackend }
	| { kind: "remove_runtime"; runtime: ModelRuntime; backend: ModelBackend }
	| { kind: "cancel_job"; job_id: string };

export function modelsCommand(request: ModelsRequest) {
	return { type: "models", request } as const;
}

const HOSTED_MODEL_STATES: ReadonlySet<string> = new Set([
	"acquiring",
	"stopped",
	"loading",
	"loaded",
	"unloading",
	"failed",
]);

const hostedModelStateSchema = z.union([
	z.discriminatedUnion("state", [
		z.object({ state: z.literal("acquiring") }),
		z.object({ state: z.literal("stopped") }),
		z.object({ state: z.literal("loading") }),
		z.object({
			state: z.literal("loaded"),
			ram_bytes: count,
			vram_bytes: count,
			slots: u8,
			slots_busy: u8,
		}),
		z.object({ state: z.literal("unloading") }),
		z.object({
			state: z.literal("failed"),
			reason: reasonOf([
				"asset_missing",
				"runtime_missing",
				"insufficient_memory",
				"engine_exited",
				"health_timeout",
			]),
		}),
	]),
	/** A state a newer agent added: shown as unknown, never fatal. */
	z
		.object({
			state: z.string().refine((state) => !HOSTED_MODEL_STATES.has(state)),
		})
		.transform(() => ({ state: "unknown" as const })),
]);

export const hostedModelSchema = z
	.object({
		id: managementId,
		display_name: label(MODEL_DISPLAY_NAME_MAX),
		kind: z.enum(MODEL_KINDS),
		engine: z.enum(MODEL_ENGINES),
		assets: z.array(modelAssetDigestSchema).min(1).max(MODEL_MAX_ASSETS),
		settings: modelSettingsSchema,
		residency: residencySchema,
		/** `configure` and `remove` send it back as `expected_revision`. */
		revision: count,
	})
	.and(hostedModelStateSchema)
	.refine(
		(model) => model.state !== "loaded" || model.slots_busy <= model.slots,
		"busy slots exceed slots",
	);
export type HostedModel = z.infer<typeof hostedModelSchema>;

export const modelAssetStatusSchema = z
	.object({ digest: modelAssetDigestSchema, job_id: jobId.optional() })
	.and(modelAssetStateSchema);
export type ModelAssetStatus = z.infer<typeof modelAssetStatusSchema>;

/** `install` and `ensure`: `pending` lists at most 32 of the assets not present yet. */
export const modelAssetSummarySchema = z
	.object({
		total: u32,
		present: u32,
		pending: lenientArray(modelAssetStatusSchema, MODEL_MAX_PENDING_ASSETS),
	})
	.refine(
		(summary) =>
			summary.present <= summary.total &&
			summary.pending.length <= summary.total - summary.present &&
			summary.pending.every((status) => status.state !== "present"),
		"asset counts disagree",
	);
export type ModelAssetSummary = z.infer<typeof modelAssetSummarySchema>;

export const modelJobSchema = z
	.object({
		job_id: jobId,
		digest: modelAssetDigestSchema,
		size: count.min(1),
		file_name: label(1024),
		/** Host of the source being fetched, e.g. `cdn.flow-like.com`. */
		source_host: z
			.string()
			.regex(/^[A-Za-z0-9.:[\]-]{1,253}$/u)
			.optional(),
		bytes_per_second: count.optional(),
		/** The asset's sources in the device's order, so this computer can send the file when the device can't fetch it. */
		sources: z
			.array(z.string().min(1).max(MODEL_ASSET_SOURCE_MAX_LEN))
			.max(MODEL_ASSET_MAX_SOURCES)
			.optional()
			.catch(undefined),
		updated_at: time,
	})
	.and(modelAssetStateSchema);
export type ModelJob = z.infer<typeof modelJobSchema>;

export const runtimeInfoSchema = z.object({
	runtime: z.enum(MODEL_RUNTIMES),
	backend: z.enum(MODEL_BACKENDS),
	build: z.string().regex(/^[A-Za-z0-9._-]{1,64}$/u),
	installed: z.boolean(),
	/** Bytes on disk once installed, else the download size. */
	size: count,
});
export type RuntimeInfo = z.infer<typeof runtimeInfoSchema>;

const capacitySchema = z
	.object({ total: count, free: count })
	.refine((capacity) => capacity.free <= capacity.total, "free exceeds total");

export const systemFactsSchema = z.object({
	cpu: z.object({
		brand: label(128),
		arch: token,
		features: z.array(token).max(MODEL_MAX_CPU_FEATURES),
		physical_cores: u16,
	}),
	ram: capacitySchema,
	gpus: lenientArray(
		z.object({
			name: label(128),
			backend: z.enum(MODEL_BACKENDS),
			/** Null while only an OS probe saw the GPU. */
			memory_total: count.nullable(),
			memory_free: count.nullable(),
		}),
		MODEL_MAX_GPUS,
	),
	model_volume: capacitySchema,
});
export type SystemFacts = z.infer<typeof systemFactsSchema>;

/** Sent unchanged as the request of a `models` command; unknown fields stay. */
const recommendationFixSchema = z.discriminatedUnion("kind", [
	z
		.object({
			kind: z.literal("configure"),
			model_id: managementId,
			expected_revision: count,
			settings: modelSettingsSchema,
			residency: residencySchema,
		})
		.passthrough(),
	z.object({ kind: z.literal("unload"), model_id: managementId }).passthrough(),
	z
		.object({
			kind: z.literal("install_runtime"),
			runtime: z.enum(MODEL_RUNTIMES),
			backend: z.enum(MODEL_BACKENDS),
		})
		.passthrough(),
]);

export const recommendationSchema = z.object({
	code: z.enum(RECOMMENDATION_CODES),
	tier: z.enum(RECOMMENDATION_TIERS),
	model_id: managementId.optional(),
	params: z
		.record(token, z.union([count, z.string().max(128)]))
		.refine(
			(params) => Object.keys(params).length <= MODEL_RECOMMENDATION_MAX_PARAMS,
			"too many parameters",
		)
		.optional(),
	/** A fix of a kind this client can't send is left out with its recommendation kept. */
	fix: recommendationFixSchema.optional().catch(undefined),
});
/** The device computes it; the client owns the copy for each `code`. */
export type Recommendation = z.infer<typeof recommendationSchema>;

export const modelsSummarySchema = z.object({
	models: u16,
	loaded: u16,
	failed: u16,
	requests_24h: count,
	tokens_24h: count,
	errors_24h: count,
	store_bytes: count,
	store_budget_bytes: count,
});
export type ModelsSummary = z.infer<typeof modelsSummarySchema>;

export const modelsOverviewSchema = z.object({
	observed_at: time,
	system: systemFactsSchema,
	runtimes: lenientArray(runtimeInfoSchema, MODEL_MAX_RUNTIMES),
	summary: modelsSummarySchema,
	/** The most urgent; `readModelRecommendations` lists all. */
	recommendations: lenientArray(
		recommendationSchema,
		MODEL_OVERVIEW_MAX_RECOMMENDATIONS,
	),
	/** The first models by id; `readHostedModels` continues after `next`. */
	models: z.array(hostedModelSchema).max(MODELS_PAGE_MAX),
	next: cursor,
});
export type ModelsOverview = z.infer<typeof modelsOverviewSchema>;

export const modelInstalledSchema = z.object({
	model: hostedModelSchema,
	assets: modelAssetSummarySchema,
});
export type ModelInstalled = z.infer<typeof modelInstalledSchema>;

export const runtimeInstalledSchema = z.object({
	runtime: runtimeInfoSchema,
	asset: modelAssetStatusSchema,
});
export type RuntimeInstalled = z.infer<typeof runtimeInstalledSchema>;

export const modelRemovedSchema = z.object({ model_id: managementId });

const consumerSchema = z.discriminatedUnion("kind", [
	z.object({ kind: z.literal("owner") }),
	z.object({ kind: z.literal("grant"), grant_id: managementId }),
	z.object({ kind: z.literal("placement"), placement_id: managementId }),
]);

function statsSchema(points: number) {
	const column = <T extends z.ZodTypeAny>(value: T) =>
		z.array(value).length(points);
	return z.object({
		model_id: managementId.nullable(),
		from: time,
		step: z.enum(["minute", "hour"]),
		series: z.object({
			requests: column(count),
			errors: column(count),
			prompt_tokens: column(count),
			completion_tokens: column(count),
			cached_tokens: column(count),
			/** Tokens per second is `completion_tokens * 1000 / decode_ms`. */
			decode_ms: column(count),
			ttft_p50_ms: column(u32.nullable()),
			ttft_p95_ms: column(u32.nullable()),
			queue_wait_p95_ms: column(u32.nullable()),
		}),
		consumers: lenientArray(
			z.object({
				consumer: consumerSchema,
				requests: count,
				errors: count,
				prompt_tokens: count,
				completion_tokens: count,
			}),
			MODEL_STATS_MAX_CONSUMERS,
		),
	});
}
export type ModelStats = z.infer<ReturnType<typeof statsSchema>>;

function firstIssue(error: unknown): string {
	if (error instanceof z.ZodError) {
		const issue = error.issues[0];
		return issue
			? `${issue.path.join(".") || "result"}: ${issue.message}`
			: "invalid";
	}
	return error instanceof Error ? error.message : String(error);
}

function pageLimit(value: number | undefined, fallback: number, what: string) {
	const limit = value ?? fallback;
	if (!Number.isInteger(limit) || limit < 1 || limit > MODELS_PAGE_MAX)
		throw new RangeError(
			`${what} limit must be an integer from 1 to ${MODELS_PAGE_MAX}, got ${limit}.`,
		);
	return limit;
}

/** `agentRead` for the `models` command: answers `unsupported` without the flag or on refusal. */
async function modelsRead<T>(
	call: ManagementCall,
	features: AgentFeatures | undefined,
	feature: AgentFeature,
	what: string,
	request: ModelsRequest,
	parse: (result: Record<string, unknown>) => T,
): Promise<AgentRead<T>> {
	if (!agentSupports(features, feature))
		return { kind: "unsupported", feature };
	const response = await call(modelsCommand(request));
	if (response.state === "rejected") {
		const rejection = managementRejection(response);
		if (rejection?.code === "unsupported")
			return { kind: "unsupported", feature };
		throw new LiveCallError(
			rejectionCode(rejection),
			rejectionMessage(response) ?? `The device refused to return the ${what}.`,
			rejection
				? { step: "reading_services", code: "rejected", rejection }
				: undefined,
		);
	}
	if (response.state !== "completed")
		throw new Error(
			`The device answered the ${what} request with state "${response.state}" instead of a result. Reconnect and retry.`,
		);
	try {
		return { kind: "ok", data: parse(response.result) };
	} catch (error) {
		throw new Error(
			`The device returned an invalid ${what} (${firstIssue(error)}).`,
		);
	}
}

export function readModelsOverview(
	call: ManagementCall,
	features: AgentFeatures | undefined,
): Promise<AgentRead<ModelsOverview>> {
	return modelsRead(
		call,
		features,
		"model_host",
		"model overview",
		{ kind: "overview" },
		(result) => modelsOverviewSchema.parse(result),
	);
}

export function readHostedModels(
	call: ManagementCall,
	features: AgentFeatures | undefined,
	input: { after?: string; limit?: number } = {},
): Promise<AgentRead<{ models: HostedModel[]; next: string | null }>> {
	const limit = pageLimit(input.limit, 8, "Hosted model page");
	return modelsRead(
		call,
		features,
		"model_host",
		"hosted model list",
		{ kind: "models", after: input.after ?? null, limit },
		(result) =>
			z
				.object({
					models: z.array(hostedModelSchema).max(limit),
					next: cursor,
				})
				.parse(result),
	);
}

/** Acquisition jobs exist from `model_store` on, before an agent hosts models. */
export function readModelJobs(
	call: ManagementCall,
	features: AgentFeatures | undefined,
	input: { after?: string; limit?: number } = {},
): Promise<AgentRead<{ jobs: ModelJob[]; next: string | null }>> {
	const limit = pageLimit(input.limit, 16, "Model job page");
	return modelsRead(
		call,
		features,
		"model_store",
		"model download list",
		{ kind: "jobs", after: input.after ?? null, limit },
		(result) =>
			z
				.object({ jobs: lenientArray(modelJobSchema, limit), next: cursor })
				.parse(result),
	);
}

export function readModelRecommendations(
	call: ManagementCall,
	features: AgentFeatures | undefined,
	input: { after?: string; limit?: number } = {},
): Promise<
	AgentRead<{ recommendations: Recommendation[]; next: string | null }>
> {
	const limit = pageLimit(input.limit, 8, "Recommendation page");
	return modelsRead(
		call,
		features,
		"model_host",
		"model recommendations",
		{ kind: "recommendations", after: input.after ?? null, limit },
		(result) =>
			z
				.object({
					recommendations: lenientArray(recommendationSchema, limit),
					next: cursor,
				})
				.parse(result),
	);
}

/** Runs the hardware probe again. */
export function probeModelSystem(
	call: ManagementCall,
	features: AgentFeatures | undefined,
): Promise<AgentRead<SystemFacts>> {
	return modelsRead(
		call,
		features,
		"model_host",
		"hardware facts",
		{ kind: "probe" },
		(result) => systemFactsSchema.parse(result),
	);
}

export interface ModelStatsInput {
	/** All models when absent. */
	modelId?: string;
	/** Unix seconds, multiples of the step; `to` is exclusive. */
	from: number;
	to: number;
	step: StatsStep;
}

/** A bulk read: `call` must read over a tunnel data stream (up to 1 MiB). */
export function readModelStats(
	call: ManagementCall,
	features: AgentFeatures | undefined,
	input: ModelStatsInput,
): Promise<AgentRead<ModelStats>> {
	const step = STATS_STEPS[input.step];
	const points = (input.to - input.from) / step;
	if (
		!Number.isSafeInteger(input.from) ||
		!Number.isSafeInteger(input.to) ||
		input.from <= 0 ||
		input.from % step !== 0 ||
		input.to % step !== 0 ||
		points < 1 ||
		points > MODEL_STATS_MAX_POINTS
	)
		throw new RangeError(
			`Model stats need 1 to ${MODEL_STATS_MAX_POINTS} whole ${input.step}s between two multiples of ${step} s, got ${input.from}..${input.to}.`,
		);
	const modelId = input.modelId ?? null;
	return modelsRead(
		call,
		features,
		"model_host",
		`usage statistics of ${modelId ?? "all models"}`,
		{
			kind: "stats",
			model_id: modelId,
			from: input.from,
			to: input.to,
			step: input.step,
		},
		(result) =>
			statsSchema(points)
				.refine(
					(stats) =>
						stats.from === input.from &&
						stats.step === input.step &&
						stats.model_id === modelId,
					"range differs from the request",
				)
				.parse(result),
	);
}
