import { describe, expect, test } from "bun:test";
import {
	MAC_MODEL_HOST_FEATURES,
	MODEL_HOST_FEATURES,
	SAMPLE_DIGESTS,
	gpuBoxModels,
	macMiniModels,
	overviewOf,
} from "../../../../../lib/device-management/model/__fixtures__/sample-models";
import type {
	HostedModel,
	ModelAssetDigest,
} from "../../../../../lib/device-management/models";
import {
	deviceFactsOf,
	downloadNeed,
	engineAvailable,
	hostedTwin,
	inlineInstallFits,
	optionView,
	optionViews,
} from "./install-plan";
import type { ModelCandidate, ModelOption, OptionFile } from "./model-options";

const GIB = 1024 ** 3;
const gpuBox = deviceFactsOf(overviewOf(gpuBoxModels()), MODEL_HOST_FEATURES);

const file = (
	name: string,
	size: number,
	digest: ModelAssetDigest = SAMPLE_DIGESTS.qwen8,
): OptionFile => ({
	file_name: name,
	size,
	digest,
	sources: ["https://cdn.flow-like.com/bits/x"],
	weights: true,
});

function option(overrides: Partial<ModelOption> = {}): ModelOption {
	return {
		id: "q4",
		label: "Q4_K_M",
		engine: "llamacpp",
		files: [file("Qwen3-8B-Q4_K_M.gguf", 5_027_784_064)],
		projectors: [],
		recommended: true,
		facts: { kvBytesPerToken: 147_456, layers: 36, contextLength: 40_960 },
		...overrides,
	};
}

const candidate = (options: ModelOption[]): ModelCandidate => ({
	source: "hub",
	key: "hub:qwen",
	name: "Qwen3-8B",
	kinds: ["chat"],
	options,
});

describe("device facts", () => {
	test("files of models that finished downloading count as present; a downloading model's don't", () => {
		expect(gpuBox.present.has(`blake3:${SAMPLE_DIGESTS.qwen8.hex}`)).toBe(true);
		expect(gpuBox.present.has(`sha256:${SAMPLE_DIGESTS.gemmaModel.hex}`)).toBe(
			false,
		);
		expect(gpuBox.store).toEqual({
			bytes: 5_575_494_884,
			budget: 1_278_000_000_000,
		});
	});

	test("a model that lost a file holds none to share; one whose engine failed still holds its files", () => {
		const failed = (
			model: HostedModel,
			reason: "asset_missing" | "engine_exited",
		): HostedModel => ({
			id: model.id,
			display_name: model.display_name,
			kind: model.kind,
			engine: model.engine,
			assets: model.assets,
			settings: model.settings,
			residency: model.residency,
			revision: model.revision,
			state: "failed",
			reason,
		});
		const sample = gpuBoxModels();
		sample.models = sample.models.map((model) => {
			if (model.id === "qwen3-8b") return failed(model, "asset_missing");
			if (model.id === "nomic-embed-v1.5")
				return failed(model, "engine_exited");
			return model;
		});
		const device = deviceFactsOf(overviewOf(sample), MODEL_HOST_FEATURES);
		expect(device.present.has(`blake3:${SAMPLE_DIGESTS.qwen8.hex}`)).toBe(
			false,
		);
		expect(device.present.has(`sha256:${SAMPLE_DIGESTS.nomicModel.hex}`)).toBe(
			true,
		);
		const choice = {
			candidate: candidate([option()]),
			option: option(),
			kind: "chat" as const,
		};
		expect(downloadNeed(choice, gpuBox)).toEqual({ files: 0, bytes: 0 });
		expect(downloadNeed(choice, device)).toEqual({
			files: 1,
			bytes: 5_027_784_064,
		});
	});

	test("an engine is available when the agent advertises its runtime", () => {
		expect(engineAvailable("llamacpp", MODEL_HOST_FEATURES)).toBe(true);
		expect(engineAvailable("onnx", MODEL_HOST_FEATURES)).toBe(true);
		expect(engineAvailable("mlx", MODEL_HOST_FEATURES)).toBe(false);
		expect(engineAvailable("mlx", MAC_MODEL_HOST_FEATURES)).toBe(true);
		expect(engineAvailable("llamacpp", undefined)).toBe(false);
	});
});

describe("versions on a device", () => {
	test("inline install counts every descriptor, placeholder fingerprint and UTF-8 byte before downloads", () => {
		const selected = option({
			files: [{ ...file("config.json", 900), digest: undefined }],
		});
		const input = {
			choice: {
				candidate: candidate([selected]),
				option: selected,
				kind: "chat" as const,
			},
			modelId: "my-model",
			settings: {},
			residency: { mode: "always_on" as const },
		};
		expect(inlineInstallFits(input, "device")).toBe(true);
		const many = option({
			files: Array.from({ length: 80 }, (_, index) =>
				file(`part-${index}.safetensors`, GIB),
			),
		});
		expect(
			inlineInstallFits(
				{ ...input, choice: { ...input.choice, option: many } },
				"device",
			),
		).toBe(false);
		const unicode = option({
			files: [
				{
					...file("weights.gguf", GIB),
					sources: [`https://cdn.example.com/${"界".repeat(6_000)}`],
				},
			],
		});
		expect(
			inlineInstallFits(
				{ ...input, choice: { ...input.choice, option: unicode } },
				"device",
			),
		).toBe(false);
	});

	test("a digest projection proves shared files but cannot prove an exact hosted twin", () => {
		const selected = option();
		const choice = {
			candidate: candidate([selected]),
			option: selected,
			kind: "chat" as const,
		};
		const sample = gpuBoxModels();
		const model = sample.models.find((item) => item.id === "qwen3-8b");
		if (!model) throw new Error("missing sample model");
		model.asset_count = 256;
		const device = deviceFactsOf(overviewOf(sample), MODEL_HOST_FEATURES);
		expect(hostedTwin(choice, device)).toBeUndefined();
		expect(downloadNeed(choice, device)).toEqual({ files: 0, bytes: 0 });
		const unseen = file("another-part.gguf", GIB, {
			algorithm: "sha256",
			hex: "f".repeat(64),
		});
		expect(
			downloadNeed({ ...choice, option: option({ files: [unseen] }) }, device),
		).toEqual({ files: 1, bytes: GIB });
	});

	test("a file the device holds isn't downloaded again; one without a digest is", () => {
		const choice = {
			candidate: candidate([option()]),
			option: option({
				files: [
					file("Qwen3-8B-Q4_K_M.gguf", 5_027_784_064),
					{ ...file("config.json", 900), digest: undefined },
				],
			}),
			kind: "chat" as const,
		};
		expect(downloadNeed(choice, gpuBox)).toEqual({ files: 1, bytes: 900 });
		expect(hostedTwin(choice, gpuBox)).toBeUndefined();
		expect(hostedTwin({ ...choice, option: option() }, gpuBox)?.id).toBe(
			"qwen3-8b",
		);
	});

	test("each version with its recommended settings, its fit and its download", () => {
		const fresh = file("Qwen3-8B-Q8_0.gguf", 8_709_519_168, {
			algorithm: "sha256",
			hex: "8".repeat(64),
		});
		const [q8] = optionViews(
			candidate([option({ id: "q8", label: "Q8_0", files: [fresh] })]),
			{ kind: "chat" },
			gpuBox,
		);
		expect(q8?.advice.fit?.verdict).toBe("gpu");
		expect(q8?.advice.settings).toMatchObject({
			parallel: 4,
			ctx_per_slot: 8_192,
		});
		expect(q8?.download).toEqual({ files: 1, bytes: 8_709_519_168 });
		expect(q8?.disk.fits).toBe(true);
		expect(q8?.block).toBeUndefined();
	});

	test("MLX on a device without the MLX runtime, and a model the disk can't take, are blocked", () => {
		const mlx = option({ id: "mlx", engine: "mlx", label: "MLX 4-bit" });
		const fresh = { algorithm: "sha256", hex: "9".repeat(64) } as const;
		expect(
			optionView(candidate([mlx]), mlx, { kind: "chat" }, gpuBox).block,
		).toBe("engine_unavailable");
		const mac = deviceFactsOf(
			overviewOf(macMiniModels()),
			MAC_MODEL_HOST_FEATURES,
		);
		const big = option({ files: [file("big.gguf", 200 * GIB, fresh)] });
		const tight = {
			...mac,
			system: {
				...mac.system,
				model_volume: { total: 500 * GIB, free: 50 * GIB },
			},
		};
		const view = optionView(candidate([big]), big, { kind: "chat" }, tight);
		expect(view.block).toBe("disk");
		const fits = option({ files: [file("mid.gguf", 60 * GIB, fresh)] });
		expect(
			optionView(candidate([fits]), fits, { kind: "chat" }, tight).block,
		).toBe("disk");
	});
});
