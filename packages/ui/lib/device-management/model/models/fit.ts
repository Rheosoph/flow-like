import {
	MODEL_DEFAULT_IDLE_UNLOAD_SECONDS,
	MODEL_MAX_THREADS,
	MODEL_MIN_CTX_PER_SLOT,
	type ModelEngine,
	type ModelKind,
	type ModelSettings,
	type Residency,
	type RuntimeInfo,
	type SystemFacts,
} from "../../models";

/*
 * Will a model fit on a device, and with which settings (plan §3.4 engine
 * defaults, §3.6 fit check)? Memory when loaded is weights + context cache
 * (bytes per token × context per slot × slots) + runtime overhead, measured
 * against what is free now: GPU memory, unified memory (Apple silicon) or RAM.
 * Only GPUs the engine's runtime on the device can use count. Speed is a class
 * from the memory bandwidth class of where the weights sit.
 */

const KIB = 1024;
const MIB = 1024 * KIB;
const GIB = 1024 * MIB;

export type KvCacheType = NonNullable<ModelSettings["kv_cache_type"]>;
export type SettingsField = keyof ModelSettings;
export type FitVerdict = "gpu" | "partial" | "cpu" | "too_large";
export type SpeedClass = "fast" | "good" | "slow" | "crawl";
export type MemoryPool = "vram" | "unified" | "ram";

/** Bytes per cached element: f16, and llama.cpp's q8_0/q4_0 blocks of 32 with an f16 scale. */
const KV_ELEMENT_BYTES: Record<KvCacheType, number> = {
	f16: 2,
	q8_0: 34 / 32,
	q4_0: 18 / 32,
};

/**
 * The settings each engine on the device reads; the others don't apply to it.
 * MLX answers one request at a time, so its context per slot is its whole
 * cache; the ONNX worker reads none.
 */
export const ENGINE_FIELDS: Record<ModelEngine, readonly SettingsField[]> = {
	llamacpp: [
		"ctx_per_slot",
		"parallel",
		"kv_cache_type",
		"gpu_layers",
		"threads",
		"flash_attn",
	],
	mlx: ["ctx_per_slot", "kv_cache_type"],
	onnx: [],
};

export interface ModelFacts {
	kind: ModelKind;
	engine: ModelEngine;
	/** Bytes of the files the engine loads: the weights, and the projector of a vision model. Unknown for a hosted model. */
	weightBytes?: number;
	/** Total parameters, when the source states them. */
	params?: number;
	/** Bits per weight of the quantization (Q4_K_M ≈ 4.89). */
	bitsPerWeight?: number;
	/** f16 context-cache bytes per token, from `config.json`. */
	kvBytesPerToken?: number;
	/** Transformer layers, from `config.json`. */
	layers?: number;
	/** Mixture of experts: the share of the weights each token reads. */
	activeShare?: number;
	/** The longest context the model handles. */
	contextLength?: number;
}

/* config.json. */

type Json = Record<string, unknown>;

const isRecord = (value: unknown): value is Json =>
	typeof value === "object" && value !== null && !Array.isArray(value);

const positive = (value: unknown) =>
	typeof value === "number" && Number.isSafeInteger(value) && value > 0
		? value
		: undefined;

function firstPositive(config: Json, keys: readonly string[]) {
	for (const key of keys) {
		const value = positive(config[key]);
		if (value !== undefined) return value;
	}
	return undefined;
}

/** A vision model keeps its language model's shape under `text_config`. */
function textConfig(config: Json) {
	const text = config.text_config;
	return isRecord(text) ? { ...config, ...text } : config;
}

function headDimOf(text: Json, heads: number | undefined) {
	const hidden = firstPositive(text, ["hidden_size", "n_embd"]);
	return (
		firstPositive(text, ["head_dim"]) ??
		(hidden && heads ? Math.floor(hidden / heads) : undefined)
	);
}

/** Layers, f16 context-cache bytes per token, expert share and context limit of a Hugging Face `config.json`. */
export function shapeFromConfig(config: unknown) {
	if (!isRecord(config)) return {};
	const text = textConfig(config);
	const layers = firstPositive(text, ["num_hidden_layers", "n_layer"]);
	const heads = firstPositive(text, ["num_attention_heads", "n_head"]);
	const kvHeads = firstPositive(text, ["num_key_value_heads"]) ?? heads;
	const headDim = headDimOf(text, heads);
	const kvBytesPerToken =
		layers && kvHeads && headDim
			? 2 * layers * kvHeads * headDim * KV_ELEMENT_BYTES.f16
			: undefined;
	const shape: Pick<
		ModelFacts,
		"layers" | "kvBytesPerToken" | "activeShare" | "contextLength"
	> = {
		...(layers ? { layers } : {}),
		...(kvBytesPerToken ? { kvBytesPerToken } : {}),
		...activeShareOf(text),
		...contextOf(text),
	};
	return shape;
}

/** Experts per token over experts, doubled for the attention and shared weights every token reads. */
function activeShareOf(config: Json) {
	const experts = firstPositive(config, [
		"num_local_experts",
		"num_experts",
		"n_routed_experts",
	]);
	const perToken = firstPositive(config, ["num_experts_per_tok"]);
	if (!experts || !perToken || perToken >= experts) return {};
	return { activeShare: Math.min(1, (2 * perToken) / experts) };
}

function contextOf(config: Json) {
	const contextLength = firstPositive(config, [
		"max_position_embeddings",
		"max_seq_len",
		"n_positions",
	]);
	return contextLength ? { contextLength } : {};
}

/* Bits per weight. */

const QUANT_BITS: Record<string, number> = {
	IQ1_S: 1.56,
	IQ1_M: 1.75,
	IQ2_XXS: 2.06,
	IQ2_XS: 2.31,
	IQ2_S: 2.5,
	IQ2_M: 2.7,
	Q2_K: 2.96,
	IQ3_XXS: 3.06,
	IQ3_XS: 3.3,
	IQ3_S: 3.44,
	IQ3_M: 3.66,
	Q3_K_S: 3.5,
	Q3_K_M: 3.91,
	Q3_K_L: 4.27,
	IQ4_XS: 4.25,
	IQ4_NL: 4.5,
	MXFP4: 4.25,
	Q4_0: 4.55,
	Q4_1: 5,
	Q4_K_S: 4.58,
	Q4_K_M: 4.89,
	Q5_0: 5.54,
	Q5_1: 6,
	Q5_K_S: 5.54,
	Q5_K_M: 5.7,
	Q6_K: 6.56,
	Q8_0: 8.5,
	BF16: 16,
	F16: 16,
	F32: 32,
};

/** "Q4_K_M" → 4.89; an unlisted "Q4_K_XL" → about 4.6; "UD_" prefixes ignored. Undefined when unknown. */
export function bitsPerWeight(quantization: string | undefined) {
	if (!quantization) return undefined;
	const quant = quantization
		.toUpperCase()
		.replace(/^UD[-_]/u, "")
		.replace(/_MOE$/u, "");
	return QUANT_BITS[quant] ?? familyBits(quant);
}

/** An unlisted variant of a known family: its integer bits plus the usual scale overhead. */
function familyBits(quant: string) {
	const digits = /^I?Q(\d)/u.exec(quant)?.[1];
	return digits ? Number(digits) + 0.6 : undefined;
}

/* Shape. */

/** Typical shapes of grouped-query models by parameters, for models without a `config.json`. */
const SHAPES: readonly (readonly [
	maxParams: number,
	kv: number,
	layers: number,
])[] = [
	[1.5e9, 32 * KIB, 28],
	[9.5e9, 144 * KIB, 36],
	[16e9, 192 * KIB, 48],
	[36e9, 256 * KIB, 64],
	[80e9, 320 * KIB, 80],
];
const LARGEST_SHAPE = [Number.POSITIVE_INFINITY, 512 * KIB, 96] as const;

const DEFAULT_BITS = 4.89;

/** Stated parameters, else weights × 8 / bits per weight. */
export function paramsOf(facts: ModelFacts) {
	if (facts.params) return facts.params;
	const bits = facts.bitsPerWeight ?? DEFAULT_BITS;
	return ((facts.weightBytes ?? 0) * 8) / bits;
}

function shapeOf(facts: ModelFacts) {
	const params = paramsOf(facts);
	const [, kv, layers] = SHAPES.find(([max]) => params <= max) ?? LARGEST_SHAPE;
	return {
		kvBytesPerToken: facts.kvBytesPerToken ?? kv,
		layers: facts.layers ?? layers,
		/** False when the shape comes from the table, not from `config.json`. */
		measured: facts.kvBytesPerToken !== undefined,
	};
}

/* Memory. */

export interface MemoryNeed {
	weights: number;
	/** Context cache: bytes per token × context per slot × slots. */
	kv: number;
	/** Compute buffers and the runtime itself. */
	overhead: number;
	total: number;
}

const DEFAULT_CTX = 8_192;
const LEAN_CTX = 2_048;

/** Settings the engine falls back to when a field is left out. */
const ENGINE_DEFAULTS = {
	ctx_per_slot: DEFAULT_CTX,
	parallel: 1,
	kv_cache_type: "f16",
} as const;

function overheadOf(engine: ModelEngine, weights: number) {
	return engine === "onnx"
		? 256 * MIB + weights * 0.2
		: 512 * MIB + weights * 0.05;
}

/** Context-cache bytes per token with `settings` (f16 unless they say otherwise). */
export function kvBytesPerToken(facts: ModelFacts, settings: ModelSettings) {
	if (facts.engine === "onnx") return 0;
	const element =
		KV_ELEMENT_BYTES[settings.kv_cache_type ?? ENGINE_DEFAULTS.kv_cache_type];
	return (shapeOf(facts).kvBytesPerToken / KV_ELEMENT_BYTES.f16) * element;
}

/** Requests the engine serves at once: only llama.cpp keeps several slots. */
const slotsOf = (engine: ModelEngine, settings: ModelSettings) =>
	engine === "llamacpp" ? (settings.parallel ?? ENGINE_DEFAULTS.parallel) : 1;

export function memoryNeed(facts: ModelFacts, settings: ModelSettings) {
	const weights = facts.weightBytes ?? 0;
	const ctx = settings.ctx_per_slot ?? ENGINE_DEFAULTS.ctx_per_slot;
	const slots = slotsOf(facts.engine, settings);
	const kv = Math.round(kvBytesPerToken(facts, settings) * ctx * slots);
	const overhead = Math.round(overheadOf(facts.engine, weights));
	const need: MemoryNeed = {
		weights,
		kv,
		overhead,
		total: weights + kv + overhead,
	};
	return need;
}

/* Where it runs. */

/** What the fit check reads of a device: its hardware, and the runtime packs it has installed or may install. */
export interface HostFacts {
	system: SystemFacts;
	runtimes: readonly RuntimeInfo[];
}

type GpuFacts = SystemFacts["gpus"][number];
type Backend = GpuFacts["backend"];
type KnownGpu = GpuFacts & { memory_total: number; memory_free: number };

export interface GpuPool {
	/** "NVIDIA GeForce RTX 4090", or the first of several of the same backend. */
	name: string;
	count: number;
	backend: Backend;
	/** Apple silicon: the GPU shares the processor's memory. */
	unified: boolean;
	/** Unified memory counts only as far as the system has it free too. */
	free: number;
	total: number;
}

/** The agent's `memory_budget` of unified memory: three quarters of RAM. */
const unifiedTotal = (system: SystemFacts) =>
	Math.floor(system.ram.total / 4) * 3;

/**
 * A GPU with the memory it offers: what a runtime read, or on Apple silicon,
 * whose GPU shares RAM, the agent's share of it while no runtime has read it.
 * A discrete GPU no runtime has read has none yet.
 */
function withMemory(gpu: GpuFacts, system: SystemFacts) {
	if (gpu.backend === "cpu") return undefined;
	if (gpu.memory_total !== null && gpu.memory_free !== null) {
		const known: KnownGpu = {
			...gpu,
			memory_total: gpu.memory_total,
			memory_free: gpu.memory_free,
		};
		return known;
	}
	if (gpu.backend !== "metal") return undefined;
	const total = unifiedTotal(system);
	const unified: KnownGpu = {
		...gpu,
		memory_total: total,
		memory_free: Math.min(system.ram.free, total),
	};
	return unified;
}

/** The agent's `recommended_backend`: Metal on Apple silicon, Vulkan on an x86_64 Linux host with a GPU, else the processor. */
function recommendedBackend(system: SystemFacts) {
	const gpus = system.gpus.filter((gpu) => gpu.backend !== "cpu");
	if (gpus.some((gpu) => gpu.backend === "metal")) return "metal";
	return system.cpu.arch === "x86_64" && gpus.length > 0 ? "vulkan" : "cpu";
}

/**
 * The backends llama.cpp runs on, like the agent's `Supervisor::pack`: its
 * installed packs, else the pack the first load installs, unless the packs
 * the device lists leave that one out.
 */
function llamaBackends(host: HostFacts) {
	const packs = host.runtimes.filter((pack) => pack.runtime === "llamacpp");
	const installed = packs.filter((pack) => pack.installed);
	if (installed.length > 0)
		return new Set<string>(installed.map((pack) => pack.backend));
	const next = recommendedBackend(host.system);
	const listed =
		packs.length === 0 || packs.some((pack) => pack.backend === next);
	return new Set<string>(listed ? [next] : []);
}

/** GPUs `engine` can run on: MLX on Apple silicon's, llama.cpp on those its runtime supports, ONNX on none. */
function engineGpus(host: HostFacts, engine: ModelEngine) {
	if (engine === "onnx") return [];
	const backends: ReadonlySet<string> =
		engine === "mlx" ? new Set(["metal"]) : llamaBackends(host);
	return host.system.gpus.filter((gpu) => backends.has(gpu.backend));
}

function poolOf(gpus: readonly KnownGpu[], system: SystemFacts) {
	const first = gpus[0] as KnownGpu;
	let free = 0;
	let total = 0;
	for (const gpu of gpus) {
		free += gpu.memory_free;
		total += gpu.memory_total;
	}
	const unified = first.backend === "metal";
	const pool: GpuPool = {
		name: first.name,
		count: gpus.length,
		backend: first.backend,
		unified,
		free: unified ? Math.min(free, system.ram.free) : free,
		total,
	};
	return pool;
}

/** The GPUs `engine` can use of the backend with the most free memory; llama.cpp splits layers across them. */
export function gpuPool(host: HostFacts, engine: ModelEngine) {
	const byBackend = new Map<string, KnownGpu[]>();
	for (const gpu of engineGpus(host, engine)) {
		const known = withMemory(gpu, host.system);
		if (known)
			byBackend.set(known.backend, [
				...(byBackend.get(known.backend) ?? []),
				known,
			]);
	}
	let best: GpuPool | undefined;
	for (const gpus of byBackend.values()) {
		const pool = poolOf(gpus, host.system);
		if (!best || pool.free > best.free) best = pool;
	}
	return best;
}

export type GpuLeftOut =
	/** The llama.cpp runtime on the device has no backend for it. */
	| "no_runtime"
	/** The installed runtime for its backend lists no GPU, e.g. a container that hides it. */
	| "not_seen"
	/** A discrete GPU whose memory no runtime has read yet. */
	| "memory_unknown";

const installedFor = (host: HostFacts, backend: string) =>
	host.runtimes.some(
		(pack) =>
			pack.runtime === "llamacpp" && pack.installed && pack.backend === backend,
	);

function leftOutWhy(
	gpu: GpuFacts,
	host: HostFacts,
	backends: ReadonlySet<string>,
) {
	if (gpu.backend === "cpu") return undefined;
	if (!backends.has(gpu.backend)) return "no_runtime" as GpuLeftOut;
	if (withMemory(gpu, host.system)) return undefined;
	return (
		installedFor(host, gpu.backend) ? "not_seen" : "memory_unknown"
	) as GpuLeftOut;
}

/** A GPU of the device the check leaves out for a llama.cpp model, and why. */
export function gpuLeftOut(host: HostFacts, engine: ModelEngine) {
	if (engine !== "llamacpp") return undefined;
	const backends = llamaBackends(host);
	for (const gpu of host.system.gpus) {
		const why = leftOutWhy(gpu, host, backends);
		if (why) return { gpu: gpu.name, why };
	}
	return undefined;
}

export interface ModelFit {
	verdict: FitVerdict;
	pool: MemoryPool;
	gpu?: GpuPool;
	/** Share of weights and context cache on the GPU: 1 all, 0 none. */
	gpuShare: number;
	memory: MemoryNeed;
	/** Bytes on the GPU and in RAM once loaded. */
	onGpu: number;
	inRam: number;
	/** Free memory the verdict was measured against. */
	free: { gpu: number; ram: number };
	/** The context cache was sized from the model's parameters, not its `config.json`. */
	estimatedShape: boolean;
	tokensPerSecond?: number;
	speed?: SpeedClass;
}

interface Placement {
	verdict: FitVerdict;
	share: number;
}

/** A partial offload below this share runs at processor speed, so it counts as the processor. */
const MIN_PARTIAL_SHARE = 0.1;

const cpuOnly = (memory: MemoryNeed, ram: number): Placement => ({
	verdict: memory.total <= ram ? "cpu" : "too_large",
	share: 0,
});

function sides(memory: MemoryNeed, share: number) {
	const layered = memory.weights + memory.kv;
	return {
		onGpu: memory.overhead + share * layered,
		inRam: (1 - share) * layered,
	};
}

/** llama.cpp's `-ngl auto`: every layer that fits goes to the GPU, the rest stays with the processor. */
function autoSplit(memory: MemoryNeed, gpu: GpuPool, ram: number) {
	if (memory.total <= gpu.free)
		return { verdict: "gpu", share: 1 } satisfies Placement;
	const layered = Math.max(1, memory.weights + memory.kv);
	const share = Math.min(
		1,
		Math.max(0, (gpu.free - memory.overhead) / layered),
	);
	if (share >= MIN_PARTIAL_SHARE && sides(memory, share).inRam <= ram)
		return { verdict: "partial", share } satisfies Placement;
	return cpuOnly(memory, ram);
}

/** `{count: n}` pins n layers on the GPU: it loads that way or not at all. */
function pinnedSplit(
	memory: MemoryNeed,
	gpu: GpuPool,
	ram: number,
	share: number,
) {
	const { onGpu, inRam } = sides(memory, share);
	const verdict: FitVerdict =
		onGpu > gpu.free || inRam > ram
			? "too_large"
			: share >= 1
				? "gpu"
				: "partial";
	return { verdict, share } satisfies Placement;
}

function pinnedShare(settings: ModelSettings, layers: number) {
	const value = settings.gpu_layers;
	if (value === undefined || value === "auto") return undefined;
	return Math.min(1, value.count / Math.max(1, layers));
}

/**
 * Apple silicon: layers on the processor sit in the same memory as those on
 * the GPU, so the model fits whole in the pool or not at all.
 */
function unifiedPlacement(memory: MemoryNeed, gpu: GpuPool, share: number) {
	const placement: Placement =
		memory.total <= gpu.free
			? { verdict: "gpu", share }
			: { verdict: "too_large", share: 0 };
	return placement;
}

/** MLX runs on Apple silicon's GPU or not at all. */
function mlxPlacement(memory: MemoryNeed, gpu: GpuPool | undefined) {
	const placement: Placement = gpu?.unified
		? unifiedPlacement(memory, gpu, 1)
		: { verdict: "too_large", share: 0 };
	return placement;
}

function placementOf(
	facts: ModelFacts,
	memory: MemoryNeed,
	settings: ModelSettings,
	host: HostFacts,
) {
	const gpu = gpuPool(host, facts.engine);
	const ram = host.system.ram.free;
	if (facts.engine === "mlx") return { gpu, ...mlxPlacement(memory, gpu) };
	const pinned = pinnedShare(settings, shapeOf(facts).layers);
	if (!gpu || pinned === 0) return { gpu, ...cpuOnly(memory, ram) };
	if (gpu.unified)
		return { gpu, ...unifiedPlacement(memory, gpu, pinned ?? 1) };
	if (pinned !== undefined)
		return { gpu, ...pinnedSplit(memory, gpu, ram, pinned) };
	return { gpu, ...autoSplit(memory, gpu, ram) };
}

function poolName(share: number, gpu: GpuPool | undefined) {
	const pool: MemoryPool =
		share === 0 || !gpu ? "ram" : gpu.unified ? "unified" : "vram";
	return pool;
}

/** How the model sits on the device with `settings`, and how fast it answers. */
export function fitModel(
	facts: ModelFacts,
	settings: ModelSettings,
	host: HostFacts,
) {
	const memory = memoryNeed(facts, settings);
	const { gpu, verdict, share } = placementOf(facts, memory, settings, host);
	const onGpu = share > 0 ? sides(memory, share).onGpu : 0;
	const fit: ModelFit = {
		verdict,
		pool: poolName(share, gpu),
		...(gpu ? { gpu } : {}),
		gpuShare: share,
		memory,
		onGpu: Math.round(onGpu),
		inRam: Math.round(memory.total - onGpu),
		free: { gpu: gpu?.free ?? 0, ram: host.system.ram.free },
		estimatedShape: !shapeOf(facts).measured,
		...speedOf(facts, { share, gpu, verdict }, host.system),
	};
	return fit;
}

/* Speed. */

/** Share of the peak bandwidth decoding reaches. */
const DECODE_EFFICIENCY = 0.6;

const APPLE_TIERS: readonly (readonly [RegExp, number])[] = [
	[/ultra/iu, 800e9],
	[/max/iu, 400e9],
	[/pro/iu, 200e9],
];

function appleBandwidth(system: SystemFacts) {
	const tier = APPLE_TIERS.find(([pattern]) => pattern.test(system.cpu.brand));
	return tier?.[1] ?? 100e9;
}

/** Bytes per second the processor reads weights at: Apple silicon, a desktop or laptop, or a small ARM board. */
function cpuBandwidth(system: SystemFacts) {
	if (/^apple/iu.test(system.cpu.brand)) return appleBandwidth(system);
	const board = system.cpu.arch === "aarch64" && system.cpu.physical_cores <= 8;
	return board ? 15e9 : 50e9;
}

/** A discrete GPU's class by its memory: bigger cards have wider buses. */
function gpuBandwidth(gpu: GpuPool, system: SystemFacts) {
	if (gpu.unified) return appleBandwidth(system);
	const each = gpu.total / gpu.count;
	if (each >= 20 * GIB) return 700e9;
	if (each >= 10 * GIB) return 400e9;
	return each >= 6 * GIB ? 250e9 : 120e9;
}

export function speedClass(tokensPerSecond: number) {
	if (tokensPerSecond >= 30) return "fast" as SpeedClass;
	if (tokensPerSecond >= 12) return "good" as SpeedClass;
	return (tokensPerSecond >= 4 ? "slow" : "crawl") as SpeedClass;
}

interface Where {
	share: number;
	gpu: GpuPool | undefined;
	verdict: FitVerdict;
}

/** Decoding reads the active weights once per token; chat and vision only. */
function speedOf(facts: ModelFacts, where: Where, system: SystemFacts) {
	const read = (facts.weightBytes ?? 0) * (facts.activeShare ?? 1);
	if (
		facts.kind === "embedding" ||
		facts.kind === "systemone" ||
		where.verdict === "too_large" ||
		read <= 0
	)
		return {};
	const cpuSeconds = read / (cpuBandwidth(system) * DECODE_EFFICIENCY);
	const gpuSeconds = where.gpu
		? read / (gpuBandwidth(where.gpu, system) * DECODE_EFFICIENCY)
		: cpuSeconds;
	const seconds = where.share * gpuSeconds + (1 - where.share) * cpuSeconds;
	const tokensPerSecond = Math.round(10 / seconds) / 10;
	return { tokensPerSecond, speed: speedClass(tokensPerSecond) };
}

/* Recommended settings. */

export type AdviceReason =
	/** The model handles no more than this. */
	| "model_limit"
	/** The engine's best-practice default (§3.4). */
	| "standard"
	/** Lowered so the model fits where it runs best. */
	| "memory"
	| "gpu_slots"
	| "cpu_slots"
	| "all_layers"
	| "some_layers"
	| "no_gpu"
	| "cores"
	| "flash"
	| "idle_unload";

export interface SettingsAdvice {
	/** The recommended value of every field the engine reads. */
	settings: ModelSettings;
	reasons: Partial<Record<SettingsField, AdviceReason>>;
	residency: Residency;
	residencyReason: AdviceReason;
	/** The fit with these settings; absent while the model's size is unknown. */
	fit?: ModelFit;
}

const RANK: Record<FitVerdict, number> = {
	gpu: 0,
	partial: 1,
	cpu: 2,
	too_large: 3,
};

interface Rung {
	ctx: number;
	slots: number;
	kv: KvCacheType;
}

function narrower(rungs: Rung[], change: Partial<Rung>) {
	const last = rungs.at(-1) as Rung;
	const next = { ...last, ...change };
	if (next.slots < last.slots || next.ctx < last.ctx) rungs.push(next);
}

/** From the standard settings down to the leanest: a cheaper cache, fewer slots, then shorter contexts. */
function ladder(top: Rung) {
	const rungs: Rung[] = [top, { ...top, kv: "q8_0" }];
	narrower(rungs, { slots: Math.ceil(top.slots / 2) });
	narrower(rungs, { slots: 1 });
	narrower(rungs, {
		ctx: Math.max(MODEL_MIN_CTX_PER_SLOT, Math.floor(top.ctx / 2)),
	});
	narrower(rungs, { ctx: Math.min(top.ctx, LEAN_CTX) });
	return rungs;
}

/** An 8-bit or 4-bit context cache: llama.cpp can't start it without flash attention. */
export const cacheNeedsFlash = (settings: ModelSettings) =>
	settings.kv_cache_type !== undefined && settings.kv_cache_type !== "f16";

/** `settings` with flash attention on where llama.cpp's quantized cache needs it. */
export function flashForCache(engine: ModelEngine, settings: ModelSettings) {
	const off =
		engine === "llamacpp" &&
		cacheNeedsFlash(settings) &&
		settings.flash_attn === false;
	return off ? { ...settings, flash_attn: true } : settings;
}

/** Only the fields `engine` reads. */
export function engineSettings(engine: ModelEngine, settings: ModelSettings) {
	const picked: ModelSettings = {};
	const target = picked as Record<string, unknown>;
	const source = settings as Record<string, unknown>;
	for (const field of ENGINE_FIELDS[engine])
		if (source[field] !== undefined) target[field] = source[field];
	return picked;
}

const rungSettings = (engine: ModelEngine, rung: Rung) =>
	engineSettings(engine, {
		ctx_per_slot: rung.ctx,
		parallel: rung.slots,
		kv_cache_type: rung.kv,
		gpu_layers: "auto",
	});

function standardCtx(facts: ModelFacts) {
	const limit = facts.contextLength;
	const ctx = Math.max(
		MODEL_MIN_CTX_PER_SLOT,
		limit ? Math.min(limit, DEFAULT_CTX) : DEFAULT_CTX,
	);
	const reason: AdviceReason =
		limit && limit < DEFAULT_CTX ? "model_limit" : "standard";
	return { ctx, reason };
}

/**
 * §3.4: four slots on a GPU; on the processor one request at a time decodes
 * fastest. MLX and ONNX answer one request at a time.
 */
function standardSlots(facts: ModelFacts, onGpu: boolean) {
	if (facts.engine !== "llamacpp")
		return { slots: 1, reason: "standard" as AdviceReason };
	if (onGpu) return { slots: 4, reason: "gpu_slots" as AdviceReason };
	const slots = facts.kind === "embedding" ? 2 : 1;
	return { slots, reason: "cpu_slots" as AdviceReason };
}

/**
 * The richest rung that reaches the best placement any rung reaches. Split
 * across GPU and processor, one slot with an 8-bit cache keeps the most layers
 * on the GPU; when nothing fits, the leanest rung shows the smallest need.
 */
function bestRung(facts: ModelFacts, host: HostFacts, rungs: readonly Rung[]) {
	const fits = rungs.map((rung) =>
		fitModel(facts, rungSettings(facts.engine, rung), host),
	);
	const best = Math.min(...fits.map((fit) => RANK[fit.verdict]));
	const indices: number[] = [];
	for (const [index, fit] of fits.entries())
		if (RANK[fit.verdict] === best) indices.push(index);
	const index = pickIndex(best, indices, rungs);
	return { rung: rungs[index] as Rung, fit: fits[index] as ModelFit };
}

function pickIndex(
	best: number,
	indices: readonly number[],
	rungs: readonly Rung[],
) {
	if (best === RANK.too_large) return rungs.length - 1;
	if (best !== RANK.partial) return indices[0] as number;
	const single = indices.find((index) => rungs[index]?.slots === 1);
	return single ?? (indices.at(-1) as number);
}

function chosen(facts: ModelFacts, host: HostFacts) {
	const ctx = standardCtx(facts);
	const lean: Rung = {
		ctx: Math.min(ctx.ctx, LEAN_CTX),
		slots: 1,
		kv: "q8_0",
	};
	const leanFit = fitModel(facts, rungSettings(facts.engine, lean), host);
	const slots = standardSlots(facts, leanFit.gpuShare > 0);
	const top: Rung = { ctx: ctx.ctx, slots: slots.slots, kv: "f16" };
	if (facts.weightBytes === undefined)
		return { top, ctx, slots, rung: top, fit: undefined };
	return { top, ctx, slots, ...bestRung(facts, host, ladder(top)) };
}

const LAYER_REASON: Record<FitVerdict, AdviceReason> = {
	gpu: "all_layers",
	partial: "some_layers",
	cpu: "no_gpu",
	too_large: "no_gpu",
};

const lowered = (
	value: unknown,
	standard: unknown,
	reason: AdviceReason,
): AdviceReason => (value === standard ? reason : "memory");

function gpuReason(
	host: HostFacts,
	engine: ModelEngine,
	fit: ModelFit | undefined,
) {
	if (fit) return LAYER_REASON[fit.verdict];
	return (gpuPool(host, engine) ? "some_layers" : "no_gpu") as AdviceReason;
}

function reasonsFor(engine: ModelEngine, all: SettingsAdvice["reasons"]) {
	const reasons: SettingsAdvice["reasons"] = {};
	for (const field of ENGINE_FIELDS[engine]) reasons[field] = all[field];
	return reasons;
}

/** The settings §3.4 recommends for this model on this device, each with why. */
export function recommendSettings(facts: ModelFacts, host: HostFacts) {
	const { top, rung, fit, ctx, slots } = chosen(facts, host);
	const settings = engineSettings(facts.engine, {
		...rungSettings(facts.engine, rung),
		threads: Math.min(
			MODEL_MAX_THREADS,
			Math.max(1, host.system.cpu.physical_cores),
		),
		flash_attn: true,
	});
	const advice: SettingsAdvice = {
		settings,
		reasons: reasonsFor(facts.engine, {
			ctx_per_slot: lowered(rung.ctx, top.ctx, ctx.reason),
			parallel: lowered(rung.slots, top.slots, slots.reason),
			kv_cache_type: lowered(rung.kv, top.kv, "standard"),
			gpu_layers: gpuReason(host, facts.engine, fit),
			threads: "cores",
			flash_attn: "flash",
		}),
		residency: {
			mode: "on_demand",
			idle_unload_after_seconds: MODEL_DEFAULT_IDLE_UNLOAD_SECONDS,
		},
		residencyReason: "idle_unload",
		...(fit ? { fit: fitModel(facts, settings, host) } : {}),
	};
	return advice;
}

/* Disk. */

export interface DiskFit {
	/** Bytes still to download: files the device doesn't hold yet. */
	needed: number;
	/** What the model disk may still take: its budget left, at most what the volume has free. */
	available: number;
	fits: boolean;
}

export function diskFit(
	needed: number,
	store: { bytes: number; budget: number },
	volumeFree: number,
) {
	const available = Math.max(
		0,
		Math.min(store.budget - store.bytes, volumeFree),
	);
	const disk: DiskFit = { needed, available, fits: needed <= available };
	return disk;
}
