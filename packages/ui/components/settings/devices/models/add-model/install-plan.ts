import { agentSupports } from "../../../../../lib/device-management/agent-reads";
import {
	type DiskFit,
	type HostFacts,
	type SettingsAdvice,
	diskFit,
	recommendSettings,
} from "../../../../../lib/device-management/model/models/fit";
import type {
	AgentFeature,
	AgentFeatures,
} from "../../../../../lib/device-management/model/types";
import type {
	HostedModel,
	ModelAssetDigest,
	ModelEngine,
	ModelKind,
	ModelsOverview,
} from "../../../../../lib/device-management/models";
import type { InstallBlock } from "./fit-copy";
import {
	type ModelCandidate,
	type ModelChoice,
	type ModelOption,
	type OptionFile,
	choiceBlock,
	choiceFacts,
	choiceFiles,
} from "./model-options";

/*
 * The add-model wizard against one device (plan §3.6, §3.7): which versions
 * fit and how, what still has to be downloaded, and why a version can't be
 * added there (no runtime for its engine, no room on the model disk).
 */

export interface DeviceModelFacts extends HostFacts {
	features: AgentFeatures | undefined;
	/** Asset digests of the device's models whose files are in its store. */
	present: ReadonlySet<string>;
	store: { bytes: number; budget: number };
	models: readonly HostedModel[];
}

const digestKey = (digest: ModelAssetDigest) =>
	`${digest.algorithm}:${digest.hex}`;

/** Failures of the engine, not the files: the model's files stay in the store. */
const FILES_KEPT: ReadonlySet<string> = new Set([
	"runtime_missing",
	"insufficient_memory",
	"engine_exited",
	"health_timeout",
]);

/** Not while it downloads, after it lost a file (`asset_missing`) or in a state this client doesn't know. */
function holdsFiles(model: HostedModel) {
	if (model.state === "failed") return FILES_KEPT.has(model.reason);
	return (
		model.state === "stopped" ||
		model.state === "loading" ||
		model.state === "loaded" ||
		model.state === "unloading"
	);
}

export function deviceFactsOf(
	overview: ModelsOverview,
	features: AgentFeatures | undefined,
) {
	const present = new Set<string>();
	for (const model of overview.models)
		if (holdsFiles(model))
			for (const digest of model.assets) present.add(digestKey(digest));
	const facts: DeviceModelFacts = {
		system: overview.system,
		runtimes: overview.runtimes,
		features,
		present,
		store: {
			bytes: overview.summary.store_bytes,
			budget: overview.summary.store_budget_bytes,
		},
		models: overview.models,
	};
	return facts;
}

const RUNTIME_FLAG: Record<ModelEngine, AgentFeature> = {
	llamacpp: "model_runtime_llamacpp",
	mlx: "model_runtime_mlx",
	onnx: "model_runtime_onnx",
};

/** The device's agent says it can run models on this engine. */
export const engineAvailable = (
	engine: ModelEngine,
	features: AgentFeatures | undefined,
) => agentSupports(features, RUNTIME_FLAG[engine]);

const missingOn = (device: DeviceModelFacts) => (file: OptionFile) =>
	!file.digest || !device.present.has(digestKey(file.digest));

/** Bytes and files the device still downloads: what no model of it holds yet. */
export function downloadNeed(choice: ModelChoice, device: DeviceModelFacts) {
	const files = choiceFiles(choice).filter(missingOn(device));
	return {
		files: files.length,
		bytes: files.reduce((total, file) => total + file.size, 0),
	};
}

export function installBlock(
	choice: ModelChoice,
	device: DeviceModelFacts,
	disk: DiskFit,
) {
	const block: InstallBlock | undefined =
		choiceBlock(choice) ??
		(engineAvailable(choice.option.engine, device.features)
			? undefined
			: "engine_unavailable") ??
		(disk.fits ? undefined : "disk");
	return block;
}

export interface OptionView {
	option: ModelOption;
	choice: ModelChoice;
	/** Recommended settings and the fit with them. */
	advice: SettingsAdvice;
	download: { files: number; bytes: number };
	disk: DiskFit;
	block?: InstallBlock;
}

export interface ChoiceInput {
	kind: ModelKind;
	/** The projector of a llama.cpp vision model, when the person picked one. */
	projector?: string;
}

/** One version of the candidate on this device. */
export function optionView(
	candidate: ModelCandidate,
	option: ModelOption,
	input: ChoiceInput,
	device: DeviceModelFacts,
) {
	const choice: ModelChoice = { candidate, option, ...input };
	const download = downloadNeed(choice, device);
	const disk = diskFit(
		download.bytes,
		device.store,
		device.system.model_volume.free,
	);
	const block = installBlock(choice, device, disk);
	const view: OptionView = {
		option,
		choice,
		advice: recommendSettings(choiceFacts(choice), device),
		download,
		disk,
		...(block ? { block } : {}),
	};
	return view;
}

export const optionViews = (
	candidate: ModelCandidate,
	input: ChoiceInput,
	device: DeviceModelFacts,
) =>
	candidate.options.map((option) =>
		optionView(candidate, option, input, device),
	);

/** A hosted model with exactly these files, when the device has one already. */
export function hostedTwin(choice: ModelChoice, device: DeviceModelFacts) {
	const files = choiceFiles(choice);
	if (files.some((file) => !file.digest)) return undefined;
	const wanted = new Set(
		files.map((file) => digestKey(file.digest as ModelAssetDigest)),
	);
	return device.models.find(
		(model) =>
			model.assets.length === wanted.size &&
			model.assets.every((digest) => wanted.has(digestKey(digest))),
	);
}
