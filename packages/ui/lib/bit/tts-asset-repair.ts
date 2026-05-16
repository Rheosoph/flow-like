import { createId } from "@paralleldrive/cuid2";
import {
	type IBit,
	IBitTypes,
	type ITtsAssetRef,
	type ITtsModelParameters,
	ITtsModelType,
} from "../schema";

const HF_BASE = "https://huggingface.co";
const QWEN_0_5B_TOKENIZER_REPO = "Qwen/Qwen2.5-0.5B";
const QWEN_1_5B_TOKENIZER_REPO = "Qwen/Qwen2.5-1.5B";
const QWEN3_TTS_TOKENIZER_REPO = "Qwen/Qwen3-TTS-Tokenizer-12Hz";

export type TtsAssetRepairPlan = {
	asset: ITtsAssetRef;
	assetId: string;
	fileName: string;
	repository: string;
	shouldRepair: boolean;
	sourcePath: string;
	sourceRepo: string;
	sourceUrl: string;
};

export function bitDependencyRef(bit: IBit): string {
	return bit.hub ? `${bit.hub}:${bit.id}` : bit.id;
}

export function createTtsRepairMarkerBit(parentBit: IBit): IBit {
	const markerId = createId();

	return {
		...parentBit,
		dependencies: [],
		dependency_tree_hash: "",
		download_link: null,
		file_name: `tts-repair-marker-${parentBit.id}.json`,
		hash: "",
		id: markerId,
		meta: {},
		model_evaluation: null,
		model_slug: null,
		parameters: {
			parent_bit: bitDependencyRef(parentBit),
			repair_marker: true,
			repaired_at: new Date().toISOString(),
		},
		size: 0,
		type: IBitTypes.File,
	};
}

export function createTtsRepairReplacementBit(
	parentBit: IBit,
	assets: ITtsAssetRef[],
	dependencies: string[],
	markerBit: IBit,
): IBit {
	return {
		...parentBit,
		dependencies: Array.from(
			new Set([...dependencies, bitDependencyRef(markerBit)]),
		),
		dependency_tree_hash: "",
		download_link: null,
		hash: "",
		id: createId(),
		parameters: {
			...(parentBit.parameters ?? {}),
			assets,
		},
	};
}

export function getTtsAssetRefs(bit: IBit): ITtsAssetRef[] {
	if (bit.type !== IBitTypes.Tts) return [];
	const parameters = bit.parameters as Partial<ITtsModelParameters> | undefined;
	return Array.isArray(parameters?.assets) ? parameters.assets : [];
}

export function localTtsAssetId(
	reference: string,
	localHub?: null | string,
): null | string {
	const [hub, id] = reference.includes(":")
		? reference.split(":", 2)
		: [localHub ?? "", reference];

	if (!id) return null;
	if (hub && localHub && hub !== localHub) return null;
	return id;
}

export function getTtsAssetRepairPlan(
	parentBit: IBit,
	asset: ITtsAssetRef,
	assetBit: IBit,
	force = false,
): TtsAssetRepairPlan | null {
	const assetId = localTtsAssetId(asset.bit, parentBit.hub);
	if (!assetId) return null;

	const source = getTtsAssetSource(parentBit, asset, assetBit);
	if (!source) return null;

	const shouldRepair =
		force ||
		isBrokenTtsAssetLink(assetBit.download_link) ||
		(asset.required && !assetBit.download_link);

	return {
		asset,
		assetId,
		fileName: source.sourcePath.split("/").pop() ?? assetBit.file_name ?? "",
		repository: `${HF_BASE}/${source.sourceRepo}`,
		shouldRepair,
		sourcePath: source.sourcePath,
		sourceRepo: source.sourceRepo,
		sourceUrl: hfTtsAssetUrl(source.sourceRepo, source.sourcePath),
	};
}

export function isBrokenTtsAssetLink(downloadLink?: null | string): boolean {
	if (!downloadLink) return false;
	return /\/bits\/W(?:\/|%2f)/i.test(downloadLink);
}

export function hfTtsAssetUrl(repo: string, path: string): string {
	const normalizedPath = path
		.split("/")
		.map((part) => encodeURIComponent(part))
		.join("/");
	const mode = path.endsWith(".json") ? "raw" : "resolve";
	return `${HF_BASE}/${repo}/${mode}/main/${normalizedPath}`;
}

function getTtsAssetSource(
	parentBit: IBit,
	asset: ITtsAssetRef,
	assetBit: IBit,
): null | { sourcePath: string; sourceRepo: string } {
	const parameters = parentBit.parameters as
		| Partial<ITtsModelParameters>
		| undefined;
	const modelType = parameters?.model_type;
	const parentRepo =
		normalizeHfRepo(parameters?.provider?.model_id) ??
		normalizeHfRepo(parentBit.repository);

	if (modelType === ITtsModelType.Qwen3Tts) {
		if (asset.relative_path === "tokenizer.json") {
			return {
				sourcePath: "tokenizer.json",
				sourceRepo: QWEN_0_5B_TOKENIZER_REPO,
			};
		}
		if (asset.relative_path.startsWith("speech_tokenizer/")) {
			return {
				sourcePath: asset.relative_path.slice("speech_tokenizer/".length),
				sourceRepo: QWEN3_TTS_TOKENIZER_REPO,
			};
		}
	}

	if (
		modelType === ITtsModelType.VibeVoice &&
		asset.relative_path === "tokenizer.json"
	) {
		return {
			sourcePath: "tokenizer.json",
			sourceRepo: QWEN_1_5B_TOKENIZER_REPO,
		};
	}

	if (
		modelType === ITtsModelType.VibeVoiceRealtime &&
		asset.relative_path === "tokenizer.json"
	) {
		return {
			sourcePath: "tokenizer.json",
			sourceRepo: QWEN_0_5B_TOKENIZER_REPO,
		};
	}

	const assetRepo = normalizeHfRepo(assetBit.repository);
	const sourceRepo = assetRepo ?? parentRepo;
	if (!sourceRepo) return null;

	return {
		sourcePath:
			sourceRepo === parentRepo
				? asset.relative_path
				: assetBit.file_name || asset.relative_path.split("/").pop() || "",
		sourceRepo,
	};
}

function normalizeHfRepo(value?: null | string): null | string {
	const trimmed = trimTrailingSlashes(value?.trim());
	if (!trimmed) return null;

	if (trimmed.startsWith(`${HF_BASE}/`)) {
		return trimmed.slice(HF_BASE.length + 1);
	}

	if (!trimmed.includes("://") && trimmed.split("/").length >= 2) {
		return trimmed;
	}

	return null;
}

function trimTrailingSlashes(value?: string): string {
	if (!value) return "";

	let end = value.length;
	while (end > 0 && value.charCodeAt(end - 1) === 47) {
		end--;
	}

	return end === value.length ? value : value.slice(0, end);
}
