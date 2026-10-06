import { describe, expect, test } from "bun:test";
import type { RuntimeInfo, SystemFacts } from "../../models";
import {
	type ModelHostSample,
	emptyModels,
	gpuBoxModels,
	macMiniModels,
} from "../__fixtures__/sample-models";
import {
	ENGINE_FIELDS,
	type HostFacts,
	type ModelFacts,
	bitsPerWeight,
	diskFit,
	fitModel,
	flashForCache,
	gpuLeftOut,
	gpuPool,
	memoryNeed,
	recommendSettings,
	shapeFromConfig,
	speedClass,
} from "./fit";

const GIB = 1024 ** 3;
const MIB = 1024 ** 2;
const hostOf = ({ system, runtimes }: ModelHostSample): HostFacts => ({
	system,
	runtimes,
});
const gpuBox = hostOf(gpuBoxModels());
const macMini = hostOf(macMiniModels());
const cpuBox = hostOf(emptyModels());

const withGpus = (host: HostFacts, gpus: SystemFacts["gpus"]): HostFacts => ({
	...host,
	system: { ...host.system, gpus },
});

const pack = (
	runtime: RuntimeInfo["runtime"],
	backend: RuntimeInfo["backend"],
	installed: boolean,
): RuntimeInfo => ({ runtime, backend, build: "b10809", installed, size: 1 });

const QWEN3_8B_CONFIG = {
	architectures: ["Qwen3ForCausalLM"],
	hidden_size: 4096,
	num_hidden_layers: 36,
	num_attention_heads: 32,
	num_key_value_heads: 8,
	head_dim: 128,
	max_position_embeddings: 40_960,
};

const qwen8: ModelFacts = {
	kind: "chat",
	engine: "llamacpp",
	weightBytes: 5_027_784_064,
	bitsPerWeight: 4.89,
	...shapeFromConfig(QWEN3_8B_CONFIG),
};

/** Qwen2.5-32B Q4_K_M: 64 layers, 8 KV heads of 128. */
const qwen32: ModelFacts = {
	kind: "chat",
	engine: "llamacpp",
	weightBytes: 19_851_336_256,
	kvBytesPerToken: 2 * 64 * 8 * 128 * 2,
	layers: 64,
	contextLength: 32_768,
};

describe("config.json", () => {
	test("Qwen3-8B: 36 layers × 8 KV heads × 128 → 144 KiB per token at f16, and its context limit", () => {
		expect(shapeFromConfig(QWEN3_8B_CONFIG)).toEqual({
			layers: 36,
			kvBytesPerToken: 147_456,
			contextLength: 40_960,
		});
	});

	test("a vision model's shape comes from its text_config", () => {
		const gemma = shapeFromConfig({
			architectures: ["Gemma3ForConditionalGeneration"],
			text_config: {
				hidden_size: 2560,
				num_hidden_layers: 34,
				num_attention_heads: 8,
				num_key_value_heads: 4,
				head_dim: 256,
			},
			vision_config: { hidden_size: 1152, num_hidden_layers: 27 },
		});
		expect(gemma).toEqual({
			layers: 34,
			kvBytesPerToken: 2 * 34 * 4 * 256 * 2,
		});
	});

	test("without head_dim the head size is hidden size over heads; without KV heads every head caches", () => {
		expect(
			shapeFromConfig({
				hidden_size: 4096,
				num_hidden_layers: 32,
				num_attention_heads: 32,
				num_key_value_heads: 8,
			}).kvBytesPerToken,
		).toBe(131_072);
		expect(
			shapeFromConfig({
				n_embd: 3072,
				n_layer: 32,
				n_head: 32,
			}).kvBytesPerToken,
		).toBe(2 * 32 * 32 * 96 * 2);
	});

	test("a mixture of experts reads a share of its weights per token", () => {
		expect(
			shapeFromConfig({ num_experts: 128, num_experts_per_tok: 8 }),
		).toEqual({ activeShare: 0.125 });
	});

	test("anything but an object, or missing sizes, gives nothing", () => {
		expect(shapeFromConfig("config")).toEqual({});
		expect(shapeFromConfig([QWEN3_8B_CONFIG])).toEqual({});
		expect(shapeFromConfig({ num_hidden_layers: "36" })).toEqual({});
	});
});

describe("bits per weight", () => {
	test.each([
		["Q4_K_M", 4.89],
		["q8_0", 8.5],
		["UD-Q4_K_XL", 4.6],
		["MXFP4_MOE", 4.25],
		["IQ3_XXS", 3.06],
		["BF16", 16],
	])("%s → %d", (quant, bits) => {
		expect(bitsPerWeight(quant)).toBe(bits);
	});

	test("unknown quantizations have none", () => {
		expect(bitsPerWeight(undefined)).toBeUndefined();
		expect(bitsPerWeight("model")).toBeUndefined();
	});
});

describe("memory", () => {
	test("weights + context cache (per token × context × slots) + overhead", () => {
		const need = memoryNeed(qwen8, { ctx_per_slot: 8_192, parallel: 4 });
		expect(need.kv).toBe(147_456 * 8_192 * 4);
		expect(need.overhead).toBe(
			Math.round(512 * 1024 ** 2 + 0.05 * 5_027_784_064),
		);
		expect(need.total).toBe(need.weights + need.kv + need.overhead);
	});

	test("an 8-bit cache takes 34/64 of the f16 one; ONNX keeps none", () => {
		const f16 = memoryNeed(qwen8, { ctx_per_slot: 4_096, parallel: 1 }).kv;
		const q8 = memoryNeed(qwen8, {
			ctx_per_slot: 4_096,
			parallel: 1,
			kv_cache_type: "q8_0",
		}).kv;
		expect(q8).toBe(Math.round((f16 * 34) / 64));
		expect(
			memoryNeed(
				{ kind: "embedding", engine: "onnx", weightBytes: 550_000_000 },
				{ parallel: 2 },
			).kv,
		).toBe(0);
	});

	test("without config.json the cache comes from the parameter class", () => {
		const fit = fitModel(
			{
				kind: "chat",
				engine: "llamacpp",
				weightBytes: 5_000_000_000,
				bitsPerWeight: 4.89,
			},
			{ ctx_per_slot: 1_000, parallel: 1 },
			gpuBox,
		);
		expect(fit.memory.kv).toBe(144 * 1024 * 1_000);
		expect(fit.estimatedShape).toBe(true);
		expect(fitModel(qwen8, {}, gpuBox).estimatedShape).toBe(false);
	});
});

describe("fit", () => {
	test("the GPU box holds Qwen3-8B with 4 slots of 8K on its RTX 4090, fast", () => {
		const fit = fitModel(
			qwen8,
			{ ctx_per_slot: 8_192, parallel: 4, kv_cache_type: "f16" },
			gpuBox,
		);
		expect(fit).toMatchObject({
			verdict: "gpu",
			pool: "vram",
			gpuShare: 1,
			gpu: { name: "NVIDIA GeForce RTX 4090", count: 1, unified: false },
			free: { gpu: 17 * GIB, ram: 38 * GIB },
			speed: "fast",
		});
		expect(fit.onGpu).toBe(fit.memory.total);
		expect(fit.inRam).toBe(0);
		expect(fit.tokensPerSecond).toBeGreaterThan(30);
	});

	test("a 32B model splits between the GPU and the processor and crawls", () => {
		const fit = fitModel(qwen32, { ctx_per_slot: 8_192, parallel: 4 }, gpuBox);
		expect(fit.verdict).toBe("partial");
		expect(fit.pool).toBe("vram");
		expect(fit.gpuShare).toBeGreaterThan(0.5);
		expect(fit.gpuShare).toBeLessThan(1);
		expect(fit.onGpu).toBeLessThanOrEqual(17 * GIB);
		expect(fit.onGpu + fit.inRam).toBe(fit.memory.total);
		expect(fit.speed).toBe("crawl");
	});

	test("without a GPU it runs on the processor, slowly; too big for RAM is too large", () => {
		const cpu = fitModel(qwen8, { ctx_per_slot: 8_192, parallel: 1 }, cpuBox);
		expect(cpu).toMatchObject({
			verdict: "cpu",
			pool: "ram",
			gpuShare: 0,
			speed: "slow",
		});
		expect(cpu.gpu).toBeUndefined();
		const huge = fitModel(
			{ kind: "chat", engine: "llamacpp", weightBytes: 42_520_000_000 },
			{ ctx_per_slot: 2_048, parallel: 1 },
			cpuBox,
		);
		expect(huge.verdict).toBe("too_large");
		expect(huge.speed).toBeUndefined();
	});

	test("gpu_layers 0 keeps the processor; a pinned count that doesn't fit is too large", () => {
		expect(fitModel(qwen8, { gpu_layers: { count: 0 } }, gpuBox).verdict).toBe(
			"cpu",
		);
		expect(
			fitModel(qwen32, { gpu_layers: { count: 64 }, parallel: 4 }, gpuBox)
				.verdict,
		).toBe("too_large");
		const half = fitModel(
			qwen32,
			{ gpu_layers: { count: 32 }, ctx_per_slot: 4_096, parallel: 1 },
			gpuBox,
		);
		expect(half).toMatchObject({ verdict: "partial", gpuShare: 0.5 });
	});

	test("MLX runs in the Mac's unified memory or not at all", () => {
		const mlx: ModelFacts = {
			kind: "chat",
			engine: "mlx",
			weightBytes: 2_300_000_000,
			...shapeFromConfig(QWEN3_8B_CONFIG),
		};
		const fit = fitModel(mlx, { ctx_per_slot: 8_192, parallel: 2 }, macMini);
		expect(fit).toMatchObject({ verdict: "gpu", pool: "unified" });
		expect(fit.gpu?.unified).toBe(true);
		expect(
			fitModel(mlx, { ctx_per_slot: 8_192, parallel: 2 }, gpuBox).verdict,
		).toBe("too_large");
	});

	test("ONNX embeddings run on the processor and have no speed class", () => {
		const fit = fitModel(
			{ kind: "embedding", engine: "onnx", weightBytes: 550_000_000 },
			{ parallel: 2, threads: 16 },
			gpuBox,
		);
		expect(fit).toMatchObject({ verdict: "cpu", pool: "ram" });
		expect(fit.gpu).toBeUndefined();
		expect(fit.speed).toBeUndefined();
	});

	test("a discrete GPU whose memory no runtime read doesn't count", () => {
		const [card] = gpuBox.system.gpus;
		if (!card) throw new Error("The GPU box sample has no GPU.");
		const probed = {
			...withGpus(gpuBox, [{ ...card, memory_total: null, memory_free: null }]),
			runtimes: [],
		};
		expect(gpuLeftOut(probed, "llamacpp")).toEqual({
			gpu: "NVIDIA GeForce RTX 4090",
			why: "memory_unknown",
		});
		expect(gpuLeftOut(gpuBox, "llamacpp")).toBeUndefined();
		expect(gpuPool(probed, "llamacpp")).toBeUndefined();
		expect(fitModel(qwen8, { parallel: 1 }, probed).verdict).toBe("cpu");
	});

	test("an installed GPU runtime that lists no GPU: the one the OS shows isn't seen", () => {
		const [card] = gpuBox.system.gpus;
		if (!card) throw new Error("The GPU box sample has no GPU.");
		const hidden = withGpus(gpuBox, [
			{ ...card, memory_total: null, memory_free: null },
		]);
		expect(gpuLeftOut(hidden, "llamacpp")).toEqual({
			gpu: "NVIDIA GeForce RTX 4090",
			why: "not_seen",
		});
		expect(fitModel(qwen8, { parallel: 1 }, hidden).verdict).toBe("cpu");
	});

	test("GPUs of one backend pool their memory", () => {
		const card = {
			name: "NVIDIA GeForce RTX 3090",
			backend: "vulkan" as const,
			memory_total: 24 * GIB,
			memory_free: 20 * GIB,
		};
		const twin = withGpus(gpuBox, [card, card]);
		expect(gpuPool(twin, "llamacpp")).toMatchObject({
			count: 2,
			free: 40 * GIB,
			total: 48 * GIB,
		});
		expect(
			fitModel(qwen32, { ctx_per_slot: 8_192, parallel: 4 }, twin).verdict,
		).toBe("gpu");
	});

	test.each([
		[45, "fast"],
		[30, "fast"],
		[20, "good"],
		[5, "slow"],
		[1.2, "crawl"],
	] as const)("%d tokens/s is %s", (rate, speed) => {
		expect(speedClass(rate)).toBe(speed);
	});
});

describe("recommended settings", () => {
	test("on the GPU box: the §3.4 defaults with every reason", () => {
		const advice = recommendSettings(qwen8, gpuBox);
		expect(advice.settings).toEqual({
			ctx_per_slot: 8_192,
			parallel: 4,
			kv_cache_type: "f16",
			gpu_layers: "auto",
			threads: 16,
			flash_attn: true,
		});
		expect(advice.reasons).toEqual({
			ctx_per_slot: "standard",
			parallel: "gpu_slots",
			kv_cache_type: "standard",
			gpu_layers: "all_layers",
			threads: "cores",
			flash_attn: "flash",
		});
		expect(advice.residency).toEqual({
			mode: "on_demand",
			idle_unload_after_seconds: 900,
		});
		expect(advice.fit?.verdict).toBe("gpu");
	});

	test("on the Mac mini an MLX model gets the standard context and a full-precision cache", () => {
		const advice = recommendSettings(
			{
				kind: "chat",
				engine: "mlx",
				weightBytes: 2_300_000_000,
				...shapeFromConfig(QWEN3_8B_CONFIG),
			},
			macMini,
		);
		expect(advice.settings).toEqual({
			ctx_per_slot: 8_192,
			kv_cache_type: "f16",
		});
		expect(advice.reasons).toEqual({
			ctx_per_slot: "standard",
			kv_cache_type: "standard",
		});
		expect(advice.fit?.verdict).toBe("gpu");
	});

	test("split across GPU and processor: one slot with an 8-bit cache, the standard context", () => {
		const advice = recommendSettings(qwen32, gpuBox);
		expect(advice.settings).toMatchObject({
			ctx_per_slot: 8_192,
			parallel: 1,
			kv_cache_type: "q8_0",
		});
		expect(advice.reasons).toMatchObject({
			parallel: "memory",
			kv_cache_type: "memory",
			gpu_layers: "some_layers",
		});
		expect(advice.fit?.verdict).toBe("partial");
	});

	test("no GPU: one slot for chat, two for embeddings, and the model's own context limit", () => {
		const chat = recommendSettings({ ...qwen8, contextLength: 4_096 }, cpuBox);
		expect(chat.settings).toMatchObject({ ctx_per_slot: 4_096, parallel: 1 });
		expect(chat.reasons).toMatchObject({
			ctx_per_slot: "model_limit",
			parallel: "cpu_slots",
			gpu_layers: "no_gpu",
		});
		const embedding = recommendSettings(
			{
				kind: "embedding",
				engine: "onnx",
				weightBytes: 550_000_000,
				contextLength: 512,
			},
			gpuBox,
		);
		expect(embedding.settings).toEqual({});
		expect(embedding.reasons).toEqual({});
		expect(embedding.fit?.verdict).toBe("cpu");
	});

	test("too large anywhere: the leanest settings show the smallest need", () => {
		const advice = recommendSettings(
			{
				kind: "chat",
				engine: "llamacpp",
				weightBytes: 42_520_000_000,
				contextLength: 131_072,
			},
			cpuBox,
		);
		expect(advice.settings).toMatchObject({
			ctx_per_slot: 2_048,
			parallel: 1,
			kv_cache_type: "q8_0",
		});
		expect(advice.fit?.verdict).toBe("too_large");
	});

	test("a hosted model's size is unknown: standard values, no fit", () => {
		const advice = recommendSettings(
			{ kind: "chat", engine: "llamacpp" },
			gpuBox,
		);
		expect(advice.fit).toBeUndefined();
		expect(advice.settings).toMatchObject({ ctx_per_slot: 8_192, parallel: 4 });
		expect(advice.reasons.gpu_layers).toBe("some_layers");
		expect(
			recommendSettings({ kind: "chat", engine: "llamacpp" }, cpuBox).reasons
				.gpu_layers,
		).toBe("no_gpu");
	});

	test("every recommended value passes the protocol's bounds", () => {
		for (const host of [gpuBox, macMini, cpuBox])
			for (const facts of [qwen8, qwen32]) {
				const { settings } = recommendSettings(facts, host);
				expect(Number.isInteger(settings.ctx_per_slot)).toBe(true);
				expect(settings.ctx_per_slot).toBeGreaterThanOrEqual(256);
				expect(settings.parallel).toBeGreaterThanOrEqual(1);
				expect(settings.threads).toBeGreaterThanOrEqual(1);
			}
	});
});

/** Qwen3-4B MLX 4-bit: needs 3.84 GiB with an f16 cache of 8,192 tokens, 3.31 GiB with an 8-bit one. */
const qwen4Mlx: ModelFacts = {
	kind: "chat",
	engine: "mlx",
	weightBytes: 2_262_000_000,
	bitsPerWeight: 4.5,
	...shapeFromConfig({ ...QWEN3_8B_CONFIG, hidden_size: 2560 }),
};

/** A 16 GiB Mac whose Metal GPU only an OS probe saw. */
const bareMac = (free: number): HostFacts => ({
	system: {
		...macMini.system,
		ram: { total: 16 * GIB, free },
		gpus: [
			{
				name: "Apple M4",
				backend: "metal",
				memory_total: null,
				memory_free: null,
			},
		],
	},
	runtimes: [],
});

describe("Apple silicon", () => {
	test("without a llama.cpp pack the GPU gets three quarters of RAM, so MLX fits with its standard settings", () => {
		const mac = bareMac(10 * GIB);
		expect(gpuPool(mac, "mlx")).toMatchObject({
			unified: true,
			total: 12 * GIB,
			free: 10 * GIB,
		});
		const advice = recommendSettings(qwen4Mlx, mac);
		expect(advice.fit?.verdict).toBe("gpu");
		expect(advice.settings).toEqual({
			ctx_per_slot: 8_192,
			kv_cache_type: "f16",
		});
		expect(gpuLeftOut(mac, "llamacpp")).toBeUndefined();
		expect(
			fitModel(qwen8, { ctx_per_slot: 8_192, parallel: 1 }, mac),
		).toMatchObject({ verdict: "gpu", pool: "unified" });
	});

	test("unified memory is one pool: what the system has free bounds it, and nothing splits", () => {
		const tight: HostFacts = {
			system: {
				...macMini.system,
				ram: { total: 16 * GIB, free: 3 * GIB },
				gpus: [
					{
						name: "Apple M4",
						backend: "metal",
						memory_total: 12_124 * MIB,
						memory_free: 12_123 * MIB,
					},
				],
			},
			runtimes: [pack("llamacpp", "metal", true), pack("mlx", "metal", true)],
		};
		const mlx = fitModel(qwen4Mlx, { ctx_per_slot: 8_192 }, tight);
		expect(mlx.memory.total).toBeGreaterThan(3.8 * GIB);
		expect(mlx).toMatchObject({ verdict: "too_large", free: { gpu: 3 * GIB } });
		const large = fitModel(
			{ ...qwen8, weightBytes: 12_500_000_000 },
			{ ctx_per_slot: 8_192, parallel: 1 },
			tight,
		);
		expect(large.memory.total).toBeGreaterThan(12 * GIB);
		expect(large.verdict).toBe("too_large");
		const small = fitModel(
			{ ...qwen8, weightBytes: 1_000_000_000 },
			{ ctx_per_slot: 4_096, parallel: 1 },
			tight,
		);
		expect(small).toMatchObject({ verdict: "gpu", pool: "unified", inRam: 0 });
	});
});

describe("engines", () => {
	test("MLX reads its context and cache type and answers one request at a time; ONNX reads nothing", () => {
		expect(ENGINE_FIELDS.mlx).toEqual(["ctx_per_slot", "kv_cache_type"]);
		expect(ENGINE_FIELDS.onnx).toEqual([]);
		expect(memoryNeed(qwen4Mlx, { ctx_per_slot: 8_192, parallel: 4 }).kv).toBe(
			memoryNeed(qwen4Mlx, { ctx_per_slot: 8_192 }).kv,
		);
		expect(memoryNeed(qwen4Mlx, { ctx_per_slot: 8_192 }).kv).toBe(
			147_456 * 8_192,
		);
	});

	test("MLX under memory pressure takes an 8-bit cache before a shorter context", () => {
		const advice = recommendSettings(qwen4Mlx, bareMac(3.5 * GIB));
		expect(advice.settings).toEqual({
			ctx_per_slot: 8_192,
			kv_cache_type: "q8_0",
		});
		expect(advice.reasons).toEqual({
			ctx_per_slot: "standard",
			kv_cache_type: "memory",
		});
		expect(advice.fit?.verdict).toBe("gpu");
	});

	test("a quantized llama.cpp cache turns flash attention on", () => {
		expect(
			flashForCache("llamacpp", { kv_cache_type: "q8_0", flash_attn: false }),
		).toEqual({ kv_cache_type: "q8_0", flash_attn: true });
		expect(
			flashForCache("llamacpp", { kv_cache_type: "f16", flash_attn: false }),
		).toEqual({ kv_cache_type: "f16", flash_attn: false });
		expect(flashForCache("llamacpp", { flash_attn: false })).toEqual({
			flash_attn: false,
		});
		expect(
			flashForCache("mlx", { kv_cache_type: "q4_0", flash_attn: false }),
		).toEqual({ kv_cache_type: "q4_0", flash_attn: false });
	});
});

describe("runtimes", () => {
	const card = gpuBox.system.gpus[0];

	test("an installed processor pack runs llama.cpp on the processor, whatever GPU the device has", () => {
		const cpuPack: HostFacts = {
			...gpuBox,
			runtimes: [
				pack("llamacpp", "cpu", true),
				pack("llamacpp", "vulkan", false),
			],
		};
		expect(fitModel(qwen8, { parallel: 1 }, cpuPack).verdict).toBe("cpu");
		expect(gpuLeftOut(cpuPack, "llamacpp")).toEqual({
			gpu: card?.name,
			why: "no_runtime",
		});
		expect(recommendSettings(qwen8, cpuPack).settings).toMatchObject({
			parallel: 1,
		});
		expect(gpuLeftOut(macMini, "llamacpp")).toEqual({
			gpu: "Apple M4",
			why: "no_runtime",
		});
		expect(gpuLeftOut(macMini, "mlx")).toBeUndefined();
	});

	test("an arm64 Linux host has no GPU pack, so its NVIDIA GPU doesn't count", () => {
		const arm: HostFacts = {
			system: {
				...gpuBox.system,
				cpu: { ...gpuBox.system.cpu, arch: "aarch64" },
			},
			runtimes: [pack("llamacpp", "cpu", false)],
		};
		expect(fitModel(qwen8, { parallel: 1 }, arm).verdict).toBe("cpu");
		expect(gpuLeftOut(arm, "llamacpp")?.why).toBe("no_runtime");
		expect(
			fitModel(qwen8, { parallel: 1 }, { ...arm, runtimes: [] }).verdict,
		).toBe("cpu");
	});

	test("before any pack is known the first load installs Vulkan on an x86_64 GPU host; packs without it keep the processor", () => {
		expect(
			fitModel(qwen8, { parallel: 1 }, { ...gpuBox, runtimes: [] }).verdict,
		).toBe("gpu");
		const noVulkan = { ...gpuBox, runtimes: [pack("llamacpp", "cpu", false)] };
		expect(fitModel(qwen8, { parallel: 1 }, noVulkan).verdict).toBe("cpu");
	});
});

describe("disk", () => {
	test("the budget left, at most what the volume has free", () => {
		expect(
			diskFit(5 * GIB, { bytes: 10 * GIB, budget: 100 * GIB }, 50 * GIB),
		).toEqual({
			needed: 5 * GIB,
			available: 50 * GIB,
			fits: true,
		});
		expect(
			diskFit(5 * GIB, { bytes: 98 * GIB, budget: 100 * GIB }, 50 * GIB),
		).toEqual({
			needed: 5 * GIB,
			available: 2 * GIB,
			fits: false,
		});
		expect(diskFit(1, { bytes: 120, budget: 100 }, 50).available).toBe(0);
	});
});
