import {
	bitContentDigest,
	bitSources,
	publicSource,
} from "../../../../../lib/bit/bit-sources";
import {
	huggingFacePinnedDownloadUrl,
	parseHuggingFaceModelReference,
} from "../../../../../lib/bit/huggingface-mlx-import";
import type {
	HuggingFaceGgufRepositoryImport,
	HuggingFaceGgufVariant,
	HuggingFaceMlxRepositoryImport,
	HuggingFaceModelAsset,
	HuggingFaceModelImport,
} from "../../../../../lib/bit/huggingface-model-import";
import {
	isLlamaCppLlmModel,
	isMlxLlmModel,
} from "../../../../../lib/bit/local-model-filter";
import { bitModelName } from "../../../../../lib/bit/model-display-name";
import {
	type ModelFacts,
	bitsPerWeight,
} from "../../../../../lib/device-management/model/models/fit";
import {
	MODEL_DISPLAY_NAME_MAX,
	MODEL_MAX_ASSETS,
	type ModelAssetDigest,
	type ModelEngine,
	type ModelKind,
	type ModelPooling,
	type ModelSpec,
} from "../../../../../lib/device-management/models";
import { type IBit, IBitTypes, IPooling } from "../../../../../lib/schema";

/*
 * What the add-model wizard offers (plan §3.7): every installable version of a
 * model from the Flow-Like hub, a Hugging Face repository or the person's own
 * Bits, and the `ModelSpec` an `install` sends for one choice. A file the
 * device fetches carries a digest pinned before the first byte (plan §3.1):
 * the hub's blake3, Hugging Face's Git LFS sha256, or a sha256 this computer
 * computes for a small file.
 */

/** Bytes this computer downloads to fingerprint a file that has no digest (plan §3.1). */
export const SMALL_FILE_MAX = 64 * 1024 * 1024;
/** Per-file limit of the device's model store. */
export const MODEL_FILE_MAX = 64 * 1024 ** 3;

export type ModelSource = "hub" | "huggingface" | "bits";

/** A file the device fetches for a model. */
export interface OptionFile {
	/** Path inside the model directory: the repository path. */
	file_name: string;
	size: number;
	/** Known up front: the hub's blake3 or Hugging Face's Git LFS sha256. */
	digest?: ModelAssetDigest;
	/** Public HTTPS sources in the order the device tries them. */
	sources: string[];
	/** Loaded into memory: weights or a projector, not a tokenizer or config. */
	weights: boolean;
}

export type OptionBlock =
	/** A file over `SMALL_FILE_MAX` without a content digest. */
	| "no_fingerprint"
	| "split_incomplete"
	| "too_many_files"
	| "file_too_large"
	/** A vision model on llama.cpp without a projector to load. */
	| "projector_missing";

/** One installable version: a quantization of a GGUF repository, an MLX bundle, a Bit. */
export interface ModelOption {
	/** Unique within its candidate. */
	id: string;
	/** "Q4_K_M", "MLX 4-bit"; empty when there is one version. */
	label: string;
	engine: ModelEngine;
	/** In load order: split GGUF parts first to last. */
	files: OptionFile[];
	/** Projectors a llama.cpp vision model may load, recommended first. */
	projectors: OptionFile[];
	pooling?: ModelPooling;
	/** The source recommends this version. */
	recommended: boolean;
	blocked?: OptionBlock;
	facts: Omit<ModelFacts, "kind" | "engine" | "weightBytes">;
}

export interface ModelCandidate {
	source: ModelSource;
	/** "hub:<bit id>", "huggingface:<repo>@<revision>", "bits:<bit id>". */
	key: string;
	/** "Qwen3-8B": the options' labels follow it. */
	name: string;
	/** The kinds the person may pick; one when the source states it. */
	kinds: ModelKind[];
	options: ModelOption[];
	/** "apache-2.0"; undefined when the source declares none. */
	license?: string;
}

/** Facts the source states about the repository, shared by its versions. */
export type SourceFacts = ModelOption["facts"] & {
	quantBits?: number;
	/** The Hub files the repository under an embedding task. */
	embeddingTask?: boolean;
};

/** What the fit check reads, without the source's other hints. */
function fitFacts({
	quantBits: _quantBits,
	embeddingTask: _embeddingTask,
	...facts
}: SourceFacts): ModelOption["facts"] {
	return facts;
}

const SHA256 = /^[0-9a-f]{64}$/u;

const sha256 = (hex: string | undefined) =>
	hex && SHA256.test(hex)
		? ({ algorithm: "sha256", hex } satisfies ModelAssetDigest)
		: undefined;

const isWeights = (path: string) => /\.(gguf|safetensors|onnx)$/iu.test(path);

/* Hugging Face. */

function huggingFaceFile(asset: HuggingFaceModelAsset): OptionFile {
	const digest = sha256(asset.lfsOid);
	return {
		file_name: asset.path,
		size: asset.size,
		...(digest ? { digest } : {}),
		sources: [asset.downloadUrl],
		weights: isWeights(asset.path),
	};
}

function variantOption(
	variant: HuggingFaceGgufVariant,
	imported: HuggingFaceGgufRepositoryImport,
	facts: SourceFacts,
): ModelOption {
	const bits = bitsPerWeight(variant.quantization);
	const projectors = projectorsOf(imported).map(huggingFaceFile);
	return {
		id: variant.id,
		label: variant.label,
		engine: "llamacpp",
		files: variant.files.map(huggingFaceFile),
		projectors,
		recommended: variant.id === imported.recommendedVariantId,
		...(variant.complete ? {} : { blocked: "split_incomplete" as const }),
		facts: { ...fitFacts(facts), ...(bits ? { bitsPerWeight: bits } : {}) },
	};
}

/** The recommended projector first. */
function projectorsOf(imported: HuggingFaceGgufRepositoryImport) {
	const first = imported.recommendedProjectorPath;
	return [...imported.projectors].sort(
		(a, b) => Number(b.path === first) - Number(a.path === first),
	);
}

function ggufKinds(
	imported: HuggingFaceGgufRepositoryImport,
	embeddingTask: boolean,
): ModelKind[] {
	if (imported.kind === "llm")
		return embeddingTask ? ["embedding", "chat"] : ["chat"];
	if (imported.kind === "vlm") return ["vision", "chat"];
	const kinds: ModelKind[] = imported.projectors.length
		? ["chat", "vision"]
		: ["chat"];
	return embeddingTask ? ["embedding", ...kinds] : [...kinds, "embedding"];
}

const withoutGgufSuffix = (name: string) =>
	name.replace(/[-_.]gguf$/iu, "") || name;

function fromGguf(
	imported: HuggingFaceGgufRepositoryImport,
	{ embeddingTask, ...facts }: SourceFacts,
) {
	const options = imported.variants.map((variant) =>
		variantOption(variant, imported, facts),
	);
	return {
		name: withoutGgufSuffix(imported.modelName),
		kinds: ggufKinds(imported, embeddingTask === true),
		options: recommendedFirst(options),
	};
}

const recommendedFirst = (options: ModelOption[]) =>
	[...options].sort((a, b) => Number(b.recommended) - Number(a.recommended));

function mlxLabel(facts: SourceFacts) {
	return facts.quantBits ? `MLX ${facts.quantBits}-bit` : "MLX";
}

function fromMlx(imported: HuggingFaceMlxRepositoryImport, facts: SourceFacts) {
	const bits = facts.quantBits ? facts.quantBits + 0.5 : 16;
	const option: ModelOption = {
		id: "mlx",
		label: mlxLabel(facts),
		engine: "mlx",
		files: imported.assets.map(huggingFaceFile),
		projectors: [],
		recommended: true,
		facts: { ...fitFacts(facts), bitsPerWeight: bits },
	};
	return {
		name: imported.modelName,
		kinds: [imported.kind === "vlm" ? "vision" : "chat"] as ModelKind[],
		options: [option],
	};
}

/** Every version of a Hugging Face repository, recommended first. */
export function fromHuggingFace(
	imported: HuggingFaceModelImport,
	facts: SourceFacts,
): ModelCandidate {
	const parts =
		imported.format === "gguf"
			? fromGguf(imported, facts)
			: fromMlx(imported, facts);
	return {
		source: "huggingface",
		key: `huggingface:${imported.repoId}@${imported.revision}`,
		...parts,
		...licenseOf(imported.license),
	};
}

const licenseOf = (license: string | null | undefined) =>
	license && license !== "unknown" ? { license } : {};

/* Bits: the Flow-Like hub and the person's own. */

/** The engine that hosts a Bit on a device; undefined when no device engine can. */
export function engineOfBit(bit: IBit): ModelEngine | undefined {
	if (isMlxLlmModel(bit))
		return bit.type === IBitTypes.SystemOne ? undefined : "mlx";
	if (!isLlamaCppLlmModel(bit)) return undefined;
	const file = bit.file_name?.toLowerCase() ?? "";
	if (bit.type === IBitTypes.Embedding && file.endsWith(".onnx")) return "onnx";
	return file.endsWith(".gguf") ? "llamacpp" : undefined;
}

const KIND_OF_TYPE: Partial<Record<IBitTypes, ModelKind>> = {
	[IBitTypes.Llm]: "chat",
	[IBitTypes.SystemOne]: "systemone",
	[IBitTypes.Vlm]: "vision",
	[IBitTypes.Embedding]: "embedding",
};

export const kindOfBit = (bit: IBit) => KIND_OF_TYPE[bit.type];

/** A model Bit a device can host: chat, vision or embedding on one of its engines. */
export const isHostableBit = (bit: IBit) =>
	kindOfBit(bit) !== undefined && engineOfBit(bit) !== undefined;

const POOLING: Partial<Record<string, ModelPooling>> = {
	[IPooling.Cls]: "cls",
	[IPooling.Mean]: "mean",
};

/** The Hugging Face repository and revision a Bit names, when it names one. */
export function huggingFaceOrigin(bit: IBit) {
	const provider = bit.parameters?.provider;
	const repo = typeof provider?.model_id === "string" ? provider.model_id : "";
	const revision =
		typeof provider?.version === "string" ? provider.version : "";
	try {
		if (parseHuggingFaceModelReference(repo) !== repo) return undefined;
	} catch {
		return undefined;
	}
	return /^[0-9a-f]{40,64}$/iu.test(revision) ? { repo, revision } : undefined;
}

function pinnedSource(
	origin: ReturnType<typeof huggingFaceOrigin>,
	path: string,
) {
	if (!origin) return undefined;
	try {
		return huggingFacePinnedDownloadUrl(origin.repo, origin.revision, path);
	} catch {
		return undefined;
	}
}

/** A hub Bit's file: digest and sources as a deployer names them (plan §3.1), so signed links never reach the device. */
function bitFile(bit: IBit, root: IBit): OptionFile {
	const name = bit.file_name ?? "";
	const digest = bitContentDigest(bit, root);
	return {
		file_name: name,
		size: bit.size ?? 0,
		...(digest ? { digest } : {}),
		sources: bitSources(bit, root),
		weights: isWeights(name),
	};
}

const QUANT =
	/(?:^|[-_.])((?:UD[-_])?(?:IQ\d(?:_[A-Z0-9]+)*|Q\d(?:_[A-Z0-9]+)*|BF16|F16|F32|MXFP\d(?:_[A-Z0-9]+)*))(?=$|[-_.])/giu;

/** "Qwen3-8B-Q4_K_M.gguf" → "Q4_K_M". */
export function quantizationOf(fileName: string) {
	const stem = (fileName.split("/").pop() ?? fileName)
		.replace(/-\d{1,6}-of-\d{1,6}\.gguf$/iu, "")
		.replace(/\.gguf$/iu, "");
	const last = [...stem.matchAll(QUANT)].at(-1)?.[1];
	return last?.replace("-", "_").toUpperCase();
}

function bitContext(bit: IBit) {
	const parameters = bit.parameters ?? {};
	const context =
		typeof parameters.context_length === "number"
			? parameters.context_length
			: parameters.input_length;
	return typeof context === "number" && context > 0
		? { contextLength: context }
		: {};
}

function bitPooling(bit: IBit) {
	const pooling = POOLING[String(bit.parameters?.pooling ?? "")];
	return bit.type === IBitTypes.Embedding && pooling ? { pooling } : {};
}

/** Files of a hub pack: the root's file and every dependency's; the projector apart. */
function packFiles(root: IBit, pack: readonly IBit[]) {
	const byId = new Map<string, IBit>([[root.id, root]]);
	for (const bit of pack) byId.set(bit.id, bit);
	const files: OptionFile[] = [];
	const projectors: OptionFile[] = [];
	for (const bit of byId.values()) {
		if (!bit.file_name) continue;
		const target = bit.type === IBitTypes.Projection ? projectors : files;
		target.push(bitFile(bit, root));
	}
	return { files, projectors };
}

function bitOption(
	root: IBit,
	files: Pick<ModelOption, "files" | "projectors">,
	facts: SourceFacts,
) {
	const quant = quantizationOf(root.file_name ?? "");
	const bits = bitsPerWeight(quant);
	const option: ModelOption = {
		id: root.id,
		label: quant ?? "",
		engine: engineOfBit(root) ?? "llamacpp",
		...files,
		...bitPooling(root),
		recommended: true,
		facts: {
			...bitContext(root),
			...fitFacts(facts),
			...(bits ? { bitsPerWeight: bits } : {}),
		},
	};
	return option;
}

/** A hub model: its pack's files with the hub's blake3 digests. */
export function fromHubPack(
	root: IBit,
	pack: readonly IBit[],
	facts: SourceFacts = {},
): ModelCandidate {
	const kind = kindOfBit(root) ?? "chat";
	return {
		source: "hub",
		key: `hub:${root.id}`,
		name: bitModelName(root) ?? root.id,
		kinds: kind === "vision" ? ["vision", "chat"] : [kind],
		options: [bitOption(root, packFiles(root, pack), facts)],
		...licenseOf(root.license),
	};
}

interface UserManifestFile {
	path: string;
	size: number;
	lfs_oid?: string;
}

function manifestFiles(bit: IBit) {
	const manifest = bit.parameters?.huggingface;
	const origin = {
		repo: String(manifest?.repo_id ?? ""),
		revision: String(manifest?.revision ?? "").toLowerCase(),
	};
	const files = Array.isArray(manifest?.files)
		? (manifest.files as UserManifestFile[])
		: [];
	return files.map((file): OptionFile => {
		const digest = sha256(file.lfs_oid);
		const source = publicSource(pinnedSource(origin, file.path));
		return {
			file_name: file.path,
			size: file.size,
			...(digest ? { digest } : {}),
			sources: source ? [source] : [],
			weights: isWeights(file.path),
		};
	});
}

interface Projection {
	download_link?: unknown;
	file_name?: unknown;
	size?: unknown;
}

/** A user GGUF's projector rides in its provider params (`params.projection`). */
function userProjector(bit: IBit, digests: ReadonlyMap<string, string>) {
	const projection = bit.parameters?.provider?.params?.projection as
		| Projection
		| undefined;
	const name = projection?.file_name;
	const link = publicSource(projection?.download_link);
	if (typeof name !== "string" || !link) return [];
	return [userFile(name, link, Number(projection?.size) || 0, digests)];
}

function userFile(
	name: string,
	link: string,
	size: number,
	digests: ReadonlyMap<string, string>,
): OptionFile {
	const digest = sha256(digests.get(link));
	return {
		file_name: name,
		size,
		...(digest ? { digest } : {}),
		sources: [link],
		weights: isWeights(name),
	};
}

/** The pinned Hugging Face file a user GGUF Bit downloads, with its projector. */
function userGgufFiles(bit: IBit, digests: ReadonlyMap<string, string>) {
	const name = bit.file_name ?? "";
	const link = publicSource(bit.download_link);
	return {
		files: link ? [userFile(name, link, bit.size ?? 0, digests)] : [],
		projectors: userProjector(bit, digests),
	};
}

/** The public links of a user GGUF Bit: its weights and projector. */
export function userBitLinks(bit: IBit) {
	const projection = bit.parameters?.provider?.params?.projection as
		| Projection
		| undefined;
	const links: string[] = [];
	for (const link of [bit.download_link, projection?.download_link]) {
		const source = publicSource(link);
		if (source) links.push(source);
	}
	return links;
}

/**
 * One of the person's own Bits. User Bits carry source identities, not
 * content hashes (plan §1): GGUF digests come from `lfsDigests` (Git LFS
 * sha256 by download link), MLX ones from the manifest's `lfs_oid`.
 */
export function fromUserBit(
	bit: IBit,
	lfsDigests: ReadonlyMap<string, string>,
	facts: SourceFacts = {},
): ModelCandidate {
	const engine = engineOfBit(bit);
	const files =
		engine === "mlx"
			? { files: manifestFiles(bit), projectors: [] }
			: userGgufFiles(bit, lfsDigests);
	const kind = kindOfBit(bit) ?? "chat";
	return {
		source: "bits",
		key: `bits:${bit.id}`,
		name: bitModelName(bit) ?? bit.id,
		kinds: kind === "vision" ? ["vision", "chat"] : [kind],
		options: [bitOption(bit, files, facts)],
		...licenseOf(bit.license),
	};
}

/* A choice and its spec. */

export interface ModelChoice {
	candidate: ModelCandidate;
	option: ModelOption;
	kind: ModelKind;
	/** A llama.cpp vision model: the projector's file name. */
	projector?: string;
}

const needsProjector = (choice: ModelChoice) =>
	choice.kind === "vision" && choice.option.engine === "llamacpp";

function projectorOf(choice: ModelChoice) {
	if (
		!needsProjector(choice) &&
		!(choice.kind === "systemone" && choice.option.engine === "llamacpp")
	)
		return undefined;
	const wanted = choice.projector ?? choice.option.projectors[0]?.file_name;
	return choice.option.projectors.find((file) => file.file_name === wanted);
}

/** The files the device fetches for a choice, in load order. */
export function choiceFiles(choice: ModelChoice) {
	const projector = projectorOf(choice);
	return projector ? [...choice.option.files, projector] : choice.option.files;
}

const sumOf = (files: readonly OptionFile[]) =>
	files.reduce((total, file) => total + file.size, 0);

/** Every byte the device downloads for a choice. */
export const choiceBytes = (choice: ModelChoice) => sumOf(choiceFiles(choice));

/** What the fit check needs: what loads into memory, with what the source states. */
export function choiceFacts(choice: ModelChoice) {
	const loaded = choiceFiles(choice).filter((file) => file.weights);
	const facts: ModelFacts = {
		...choice.option.facts,
		kind: choice.kind,
		engine: choice.option.engine,
		weightBytes: sumOf(loaded),
	};
	return facts;
}

const unfingerprinted = (file: OptionFile) =>
	!file.digest && file.size > SMALL_FILE_MAX;

/** Why a choice can't be installed; undefined when it can. */
export function choiceBlock(choice: ModelChoice) {
	if (choice.option.blocked) return choice.option.blocked;
	if (needsProjector(choice) && !projectorOf(choice))
		return "projector_missing" as OptionBlock;
	const files = choiceFiles(choice);
	if (files.length > MODEL_MAX_ASSETS) return "too_many_files" as OptionBlock;
	if (files.some((file) => file.size > MODEL_FILE_MAX))
		return "file_too_large" as OptionBlock;
	return files.some(unfingerprinted)
		? ("no_fingerprint" as OptionBlock)
		: undefined;
}

/** Files whose digest this computer computes before the install. */
export const filesToFingerprint = (choice: ModelChoice) =>
	choiceFiles(choice).filter((file) => !file.digest);

/** UTF-8 bytes, cut at a character so the name stays within `max` bytes. */
function clip(text: string, max: number) {
	const encoder = new TextEncoder();
	let out = "";
	for (const char of text) {
		if (encoder.encode(out + char).length > max) break;
		out += char;
	}
	return out;
}

/** A name the device takes: no control characters, no outer whitespace, at most `MODEL_DISPLAY_NAME_MAX` bytes. */
const deviceName = (text: string) =>
	clip(text.replace(/\p{Cc}+/gu, " ").trim(), MODEL_DISPLAY_NAME_MAX).trimEnd();

/** "Qwen3-8B Q4_K_M"; the candidate's key when the source names nothing. */
export function displayNameOf(choice: ModelChoice) {
	const { name, key } = choice.candidate;
	const label = choice.option.label;
	const full = label && !name.includes(label) ? `${name} ${label}` : name;
	return deviceName(full) || deviceName(key);
}

/** A management id from the name, unused on the device: "qwen3-8b-q4_k_m", then "-2", "-3". */
export function modelIdFor(name: string, taken: ReadonlySet<string>) {
	const base =
		name
			.toLowerCase()
			.replace(/[^a-z0-9_.:-]+/gu, "-")
			.replace(/^-+|-+$/gu, "")
			.slice(0, 120) || "model";
	let id = base;
	for (let next = 2; taken.has(id); next++) id = `${base}-${next}`;
	return id;
}

/**
 * The `ModelSpec` an install sends. `digests` holds what this computer
 * computed, by file name; a file without any digest is an error.
 */
export function modelSpecOf(
	choice: ModelChoice,
	digests: ReadonlyMap<string, ModelAssetDigest> = new Map(),
): ModelSpec {
	const assets = choiceFiles(choice).map((file) => {
		const digest = file.digest ?? digests.get(file.file_name);
		if (!digest)
			throw new Error(
				`Building the model spec of ${displayNameOf(choice)} failed: ${file.file_name} has no fingerprint.`,
			);
		return {
			digest,
			size: file.size,
			file_name: file.file_name,
			...(file.sources.length ? { sources: file.sources } : {}),
		};
	});
	const projector = projectorOf(choice);
	return {
		display_name: displayNameOf(choice),
		kind: choice.kind,
		engine: choice.option.engine,
		assets,
		...(projector ? { projector: projector.file_name } : {}),
		...(choice.kind === "embedding" && choice.option.pooling
			? { pooling: choice.option.pooling }
			: {}),
	};
}
