import type {
	FitVerdict,
	GpuLeftOut,
	ModelFit,
	SpeedClass,
} from "../../../../../lib/device-management/model/models/fit";
import { bytesText } from "../../observe/observe-data";
import type { DevicesT } from "../../primitives/area-context";
import type { ChipTone } from "../../primitives/tone";
import type { OptionBlock } from "./model-options";

/*
 * Words for the fit check (plan §3.6): where a model runs on the device, how
 * fast, how much memory it takes and why a version can't be added. Literal
 * `devices:`-prefixed keys for the extractor.
 */

type Copy = (t: DevicesT) => string;

export const VERDICT_TONE: Record<FitVerdict, ChipTone> = {
	gpu: "good",
	partial: "warning",
	cpu: "outline",
	too_large: "critical",
};

const VERDICTS: Record<FitVerdict, Copy> = {
	gpu: (t) => t("devices:models.fit.verdict.gpu", "Fits on the GPU"),
	partial: (t) => t("devices:models.fit.verdict.partial", "Partly on the GPU"),
	cpu: (t) => t("devices:models.fit.verdict.cpu", "Processor only"),
	too_large: (t) => t("devices:models.fit.verdict.tooLarge", "Too large"),
};

export const verdictLabel = (t: DevicesT, verdict: FitVerdict) =>
	VERDICTS[verdict](t);

const SPEEDS: Record<SpeedClass, Copy> = {
	fast: (t) => t("devices:models.fit.speed.fast", "Fast"),
	good: (t) => t("devices:models.fit.speed.good", "Good speed"),
	slow: (t) => t("devices:models.fit.speed.slow", "Slow"),
	crawl: (t) => t("devices:models.fit.speed.crawl", "Very slow"),
};

/** "Fast · about 83 tokens/s"; undefined for embeddings and models that don't fit. */
export function speedText(t: DevicesT, fit: ModelFit) {
	if (!fit.speed || fit.tokensPerSecond === undefined) return undefined;
	return t(
		"devices:models.fit.speed.rate",
		"{{speed}} · about {{rate, number}} tokens/s",
		{ speed: SPEEDS[fit.speed](t), rate: Math.round(fit.tokensPerSecond) },
	);
}

const MEMORY: Record<FitVerdict, (t: DevicesT, fit: ModelFit) => string> = {
	gpu: (t, fit) =>
		fit.pool === "unified"
			? t(
					"devices:models.fit.memory.unified",
					"About {{size}} of unified memory · {{free}} free",
					{
						size: bytesText(fit.memory.total),
						free: bytesText(fit.free.gpu),
					},
				)
			: t(
					"devices:models.fit.memory.gpu",
					"About {{size}} of GPU memory · {{free}} free",
					{ size: bytesText(fit.onGpu), free: bytesText(fit.free.gpu) },
				),
	partial: (t, fit) =>
		t(
			"devices:models.fit.memory.partial",
			"About {{gpu}} on the GPU and {{ram}} in RAM",
			{ gpu: bytesText(fit.onGpu), ram: bytesText(fit.inRam) },
		),
	cpu: (t, fit) =>
		t(
			"devices:models.fit.memory.ram",
			"About {{size}} of RAM · {{free}} free",
			{ size: bytesText(fit.memory.total), free: bytesText(fit.free.ram) },
		),
	too_large: (t, fit) =>
		t(
			"devices:models.fit.memory.tooLarge",
			"Needs about {{size}}, more than the device has free",
			{ size: bytesText(fit.memory.total) },
		),
};

/** Memory when loaded, against what is free now. */
export const memoryText = (t: DevicesT, fit: ModelFit) =>
	MEMORY[fit.verdict](t, fit);

/** "Weights 4.7 GiB · context 4.5 GiB · runtime 750 MiB". */
export function memoryParts(t: DevicesT, fit: ModelFit) {
	return t(
		"devices:models.fit.memory.parts",
		"Weights {{weights}} · context {{context}} · runtime {{runtime}}",
		{
			weights: bytesText(fit.memory.weights),
			context: bytesText(fit.memory.kv),
			runtime: bytesText(fit.memory.overhead),
		},
	);
}

const DETAILS: Record<FitVerdict, (t: DevicesT, fit: ModelFit) => string> = {
	gpu: (t, fit) =>
		fit.gpuShare < 1
			? t(
					"devices:models.fit.detail.gpuPinned",
					"{{percent, number}} % of its layers run on {{gpu}}, as set; the rest run on the processor, which is much slower.",
					{
						percent: Math.round(fit.gpuShare * 100),
						gpu: fit.gpu?.name ?? "",
					},
				)
			: t("devices:models.fit.detail.gpu", "Every layer fits on {{gpu}}.", {
					gpu: fit.gpu?.name ?? "",
				}),
	partial: (t, fit) =>
		t(
			"devices:models.fit.detail.partial",
			"{{percent, number}} % of it fits on {{gpu}}; the rest runs on the processor, which is much slower.",
			{ percent: Math.round(fit.gpuShare * 100), gpu: fit.gpu?.name ?? "" },
		),
	cpu: (t, fit) =>
		fit.gpu
			? t(
					"devices:models.fit.detail.cpuNoRoom",
					"{{gpu}} has no room for it, so it runs on the processor.",
					{ gpu: fit.gpu.name },
				)
			: t(
					"devices:models.fit.detail.cpu",
					"It runs on the processor: this device has no GPU it can use.",
				),
	too_large: (t) =>
		t(
			"devices:models.fit.detail.tooLarge",
			"It doesn't fit in the memory free on this device, even with the leanest settings.",
		),
};

/** Where it runs, in one sentence. */
export const fitDetail = (t: DevicesT, fit: ModelFit) =>
	DETAILS[fit.verdict](t, fit);

export const estimatedNote = (t: DevicesT) =>
	t(
		"devices:models.fit.estimated",
		"The context size is estimated from the model's size: its config.json couldn't be read.",
	);

const LEFT_OUT: Record<GpuLeftOut, (t: DevicesT, gpu: string) => string> = {
	memory_unknown: (t) =>
		t(
			"devices:models.fit.gpuUnknown",
			"The GPU's memory is read once a runtime for it is installed. Until then the check counts the processor only.",
		),
	no_runtime: (t, gpu) =>
		t(
			"devices:models.fit.gpuNoRuntime",
			"llama.cpp on this device has no runtime for {{gpu}}, so its models run on the processor.",
			{ gpu },
		),
	not_seen: (t, gpu) =>
		t(
			"devices:models.fit.gpuNotSeen",
			"The llama.cpp runtime on this device doesn't see {{gpu}}, so its models run on the processor.",
			{ gpu },
		),
};

/** Why the check leaves a GPU of the device out. */
export const gpuLeftOutNote = (
	t: DevicesT,
	left: { gpu: string; why: GpuLeftOut },
) => LEFT_OUT[left.why](t, left.gpu);

export type InstallBlock =
	| OptionBlock
	/** The agent advertises no runtime flag for the engine. */
	| "engine_unavailable"
	| "disk";

export interface BlockContext {
	device: string;
	engine: string;
	needed: number;
	available: number;
}

const BLOCKS: Record<InstallBlock, (t: DevicesT, c: BlockContext) => string> = {
	no_fingerprint: (t) =>
		t(
			"devices:models.fit.block.noFingerprint",
			"A file has neither a fingerprint nor a download source to verify. Choose a source that provides one.",
		),
	split_incomplete: (t) =>
		t(
			"devices:models.fit.block.splitIncomplete",
			"Parts of it are missing from the repository.",
		),
	too_many_files: (t) =>
		t(
			"devices:models.fit.block.tooManyFiles",
			"It has more files than a device takes for one model (32).",
		),
	invalid_file_size: (t) =>
		t(
			"devices:models.fit.block.invalidFileSize",
			"One of its files has an invalid size.",
		),
	projector_missing: (t) =>
		t(
			"devices:models.fit.block.projectorMissing",
			"A vision model needs a projector file, and this source has none. Add it as a chat model instead.",
		),
	engine_unavailable: (t, c) =>
		t(
			"devices:models.fit.block.engineUnavailable",
			"{{device}} can't run {{engine}} models.",
			{ device: c.device, engine: c.engine },
		),
	disk: (t, c) =>
		t(
			"devices:models.fit.block.disk",
			"It needs {{needed}} on the model disk, which has {{available}} left.",
			{ needed: bytesText(c.needed), available: bytesText(c.available) },
		),
};

/** Why a version can't be added. */
export const blockText = (
	t: DevicesT,
	block: InstallBlock,
	context: BlockContext,
) => BLOCKS[block](t, context);
