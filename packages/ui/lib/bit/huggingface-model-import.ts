import { type IBit, IBitTypes, type ILlmParameters } from "../schema";
import {
	type HuggingFaceMlxAsset,
	type HuggingFaceMlxFetch,
	type HuggingFaceMlxImport,
	huggingFacePinnedDownloadUrl,
	inspectHuggingFaceMlxRepository,
	parseHuggingFaceModelReference,
} from "./huggingface-mlx-import";
export { validateHuggingFacePinnedGgufDownloadUrl } from "./huggingface-mlx-import";
import { mlxAssetPathError } from "./mlx-model-pack";

const HUGGING_FACE_ORIGIN = "https://huggingface.co";
const MAX_TREE_PAGES = 32;
const MAX_JSON_FILE_SIZE = 8 * 1024 * 1024;
const DEFAULT_MODEL_CLASSIFICATION = {
	cost: 0.3,
	creativity: 0.3,
	factuality: 0.3,
	function_calling: 0.3,
	multilinguality: 0.3,
	openness: 0.3,
	reasoning: 0.3,
	coding: 0.3,
	safety: 0.3,
	speed: 0.3,
};

export type HuggingFaceModelKind = "llm" | "vlm" | "unknown";

export interface HuggingFaceModelReference {
	repoId: string;
	requestedPath?: string;
}

export interface HuggingFaceRepositoryAccess {
	private: boolean;
	gated: false | string;
}

export interface HuggingFaceModelAsset {
	path: string;
	size: number;
	downloadUrl: string;
	oid?: string;
	lfsOid?: string;
}

interface HuggingFaceModelMetadata {
	repoId: string;
	repositoryUrl: string;
	revision: string;
	modelName: string;
	author: string;
	authorUrl: string;
	license: string;
	tags: string[];
	contextLength: number;
	architecture?: string;
}

interface HuggingFaceImportCommon extends HuggingFaceModelMetadata {
	requestedPath?: string;
	kind: HuggingFaceModelKind;
	kindEvidence: string[];
	access: HuggingFaceRepositoryAccess;
	warnings: string[];
}

export interface HuggingFaceMlxRepositoryImport
	extends Omit<HuggingFaceMlxImport, "kind">,
		HuggingFaceImportCommon {
	format: "mlx";
	kind: "llm" | "vlm";
	assets: HuggingFaceMlxAsset[];
}

export interface HuggingFaceGgufVariant {
	/** Stable repository-relative variant key. */
	id: string;
	label: string;
	quantization?: string;
	files: HuggingFaceModelAsset[];
	totalSize: number;
	split: boolean;
	complete: boolean;
	requested: boolean;
}

export interface HuggingFaceGgufRepositoryImport
	extends HuggingFaceImportCommon {
	format: "gguf";
	variants: HuggingFaceGgufVariant[];
	recommendedVariantId?: string;
	projectors: HuggingFaceModelAsset[];
	recommendedProjectorPath?: string;
	ignoredPaths: string[];
}

export type HuggingFaceModelImport =
	| HuggingFaceMlxRepositoryImport
	| HuggingFaceGgufRepositoryImport;

export interface HuggingFaceGgufSelectionOptions {
	variantId?: string;
	projectorPath?: string;
	kind?: "llm" | "vlm";
}

export interface HuggingFaceGgufSelection {
	kind: "llm" | "vlm";
	variant: HuggingFaceGgufVariant;
	projector?: HuggingFaceModelAsset;
}

export interface HuggingFaceGgufAdminDraft {
	root: IBit;
	projection?: IBit;
	selection: HuggingFaceGgufSelection;
}

export interface HuggingFaceUserMlxManifestFile {
	path: string;
	size: number;
	role?: string;
	oid?: string;
	lfs_oid?: string;
}

export interface HuggingFaceUserMlxManifest {
	schema: 1;
	repo_id: string;
	revision: string;
	format: "mlx";
	files: HuggingFaceUserMlxManifestFile[];
}

interface HuggingFaceModelInfo {
	id?: unknown;
	author?: unknown;
	sha?: unknown;
	private?: unknown;
	gated?: unknown;
	pipeline_tag?: unknown;
	library_name?: unknown;
	tags?: unknown;
	cardData?: unknown;
}

interface HuggingFaceTreeFile {
	type?: unknown;
	path?: unknown;
	size?: unknown;
	oid?: unknown;
	lfs?: unknown;
}

interface ParsedTreeFile {
	path: string;
	size: number;
	oid?: string;
	lfsOid?: string;
}

function isRecord(value: unknown): value is Record<string, unknown> {
	return typeof value === "object" && value !== null && !Array.isArray(value);
}

function stringValue(value: unknown): string | undefined {
	return typeof value === "string" && value.trim() ? value.trim() : undefined;
}

function stringValues(value: unknown): string[] {
	return Array.isArray(value)
		? value
				.filter((entry): entry is string => typeof entry === "string")
				.map((entry) => entry.trim())
				.filter(Boolean)
		: [];
}

function encodeRepoId(repoId: string): string {
	return repoId.split("/").map(encodeURIComponent).join("/");
}

function portablePathKey(path: string): string {
	return path.normalize("NFC").toLowerCase();
}

/**
 * Parse a repository reference while retaining an explicitly selected Hub
 * `blob`/`resolve` file. Discovery still pins the repository's current commit.
 */
export function parseHuggingFaceModelReferenceWithPath(
	reference: string,
): HuggingFaceModelReference {
	const repoId = parseHuggingFaceModelReference(reference);
	const input = reference.trim();
	if (!/^https?:\/\//i.test(input)) return { repoId };

	const url = new URL(input);
	const components = url.pathname.split("/").filter(Boolean);
	if (components.length < 5) return { repoId };
	const route = components[2]?.toLowerCase();
	if (route !== "blob" && route !== "resolve") return { repoId };

	const rawPath = components.slice(4);
	if (rawPath.length === 0) return { repoId };
	let requestedPath: string;
	try {
		requestedPath = rawPath.map(decodeURIComponent).join("/");
	} catch {
		throw new Error(
			"The Hugging Face repository URL contains invalid file escaping",
		);
	}
	const pathError = mlxAssetPathError(requestedPath);
	if (pathError) {
		throw new Error(
			`Unsafe Hugging Face path "${requestedPath}": ${pathError}`,
		);
	}
	return { repoId, requestedPath };
}

function apiErrorMessage(status: number, label: string, body: string): string {
	if (status === 401 || status === 403) {
		return `${label} is private, gated, or not accessible`;
	}
	if (status === 404) return `${label} was not found`;
	if (status === 429) {
		return "Hugging Face rate-limited the repository inspection. Wait briefly and try again";
	}
	const detail = body.trim().replace(/\s+/g, " ").slice(0, 240);
	return detail
		? `${label} request failed (${status}): ${detail}`
		: `${label} request failed (${status})`;
}

async function fetchJson(
	fetcher: HuggingFaceMlxFetch,
	url: string,
	label: string,
): Promise<{ value: unknown; response: Response }> {
	const response = await fetcher(url, {
		headers: { Accept: "application/json" },
	});
	if (!response.ok) {
		const body = await response.text().catch(() => "");
		throw new Error(apiErrorMessage(response.status, label, body));
	}
	let value: unknown;
	try {
		value = await response.json();
	} catch {
		throw new Error(`${label} returned invalid JSON`);
	}
	return { value, response };
}

function parseTreeFile(entry: HuggingFaceTreeFile): ParsedTreeFile | undefined {
	if (entry.type !== "file" || typeof entry.path !== "string") return undefined;
	const pathError = mlxAssetPathError(entry.path);
	if (pathError) {
		throw new Error(
			`Unsafe Hugging Face repository path "${entry.path}": ${pathError}`,
		);
	}
	const lfs = isRecord(entry.lfs) ? entry.lfs : undefined;
	const rawSize =
		typeof entry.size === "number"
			? entry.size
			: typeof lfs?.size === "number"
				? lfs.size
				: undefined;
	if (rawSize === undefined || !Number.isSafeInteger(rawSize) || rawSize < 0) {
		throw new Error(
			`Hugging Face did not report a valid size for "${entry.path}"`,
		);
	}
	return {
		path: entry.path,
		size: rawSize,
		oid: stringValue(entry.oid),
		lfsOid: stringValue(lfs?.oid),
	};
}

function nextTreePage(
	response: Response,
	repoId: string,
	revision: string,
): string | undefined {
	const link = response.headers.get("link");
	if (!link) return undefined;
	for (const section of link.split(",")) {
		const match = section.match(/<([^>]+)>\s*;\s*rel="?next"?/i);
		if (!match) continue;
		const next = new URL(match[1], HUGGING_FACE_ORIGIN);
		const treePath = `/api/models/${encodeRepoId(repoId)}/tree/${encodeURIComponent(revision)}`;
		if (next.origin !== HUGGING_FACE_ORIGIN || next.pathname !== treePath) {
			throw new Error("Hugging Face returned an unsafe pagination URL");
		}
		return next.toString();
	}
	return undefined;
}

async function listRepositoryTree(
	fetcher: HuggingFaceMlxFetch,
	repoId: string,
	revision: string,
): Promise<ParsedTreeFile[]> {
	let nextUrl: string | undefined =
		`${HUGGING_FACE_ORIGIN}/api/models/${encodeRepoId(repoId)}/tree/${encodeURIComponent(revision)}?recursive=true&expand=false`;
	const files: ParsedTreeFile[] = [];
	const targets = new Set<string>();

	for (let page = 0; nextUrl && page < MAX_TREE_PAGES; page += 1) {
		const { value, response } = await fetchJson(
			fetcher,
			nextUrl,
			"Hugging Face repository tree",
		);
		if (!Array.isArray(value)) {
			throw new Error(
				"Hugging Face repository tree returned an invalid response",
			);
		}
		for (const rawEntry of value) {
			if (!isRecord(rawEntry)) continue;
			const file = parseTreeFile(rawEntry);
			if (!file) continue;
			const key = portablePathKey(file.path);
			if (targets.has(key)) {
				throw new Error(
					`Hugging Face repository contains colliding path "${file.path}"`,
				);
			}
			targets.add(key);
			files.push(file);
		}
		nextUrl = nextTreePage(response, repoId, revision);
	}
	if (nextUrl) {
		throw new Error(
			`Hugging Face repository tree exceeded ${MAX_TREE_PAGES} API pages`,
		);
	}
	return files;
}

async function fetchSmallRepositoryJson(
	fetcher: HuggingFaceMlxFetch,
	repoId: string,
	revision: string,
	file: ParsedTreeFile | undefined,
): Promise<Record<string, unknown> | undefined> {
	if (!file) return undefined;
	if (file.size > MAX_JSON_FILE_SIZE) {
		throw new Error(
			`${file.path} is unexpectedly large (${file.size} bytes); refusing to inspect it`,
		);
	}
	const { value } = await fetchJson(
		fetcher,
		huggingFacePinnedDownloadUrl(repoId, revision, file.path),
		file.path,
	);
	if (!isRecord(value))
		throw new Error(`${file.path} must contain a JSON object`);
	return value;
}

function nestedRecord(
	record: Record<string, unknown> | undefined,
	key: string,
): Record<string, unknown> | undefined {
	const value = record?.[key];
	return isRecord(value) ? value : undefined;
}

function validContextLength(value: unknown): number | undefined {
	return typeof value === "number" &&
		Number.isSafeInteger(value) &&
		value > 0 &&
		value <= 2_000_000
		? value
		: undefined;
}

function inferContextLength(
	config: Record<string, unknown> | undefined,
	tokenizerConfig: Record<string, unknown> | undefined,
): number {
	const textConfig = nestedRecord(config, "text_config");
	for (const candidate of [
		textConfig?.max_position_embeddings,
		textConfig?.model_max_length,
		config?.max_position_embeddings,
		config?.model_max_length,
		config?.max_seq_len,
		config?.seq_length,
		config?.n_positions,
		tokenizerConfig?.model_max_length,
	]) {
		const contextLength = validContextLength(candidate);
		if (contextLength) return contextLength;
	}
	return 2048;
}

function containsVisionValue(value: unknown, depth = 0): boolean {
	if (depth > 4) return false;
	if (typeof value === "string") {
		return /(vision|image[_-]?processor|vision2seq|llava|idefics|paligemma|pixtral|internvl|qwen.*vl)/i.test(
			value,
		);
	}
	if (Array.isArray(value)) {
		return value.some((entry) => containsVisionValue(entry, depth + 1));
	}
	if (!isRecord(value)) return false;
	return Object.entries(value).some(
		([key, entry]) =>
			/(vision_config|visual|image_token|video_token|image_processor)/i.test(
				key,
			) || containsVisionValue(entry, depth + 1),
	);
}

function inferGgufKind(
	info: HuggingFaceModelInfo,
	config: Record<string, unknown> | undefined,
	processor: Record<string, unknown> | undefined,
	hasProjector: boolean,
): {
	kind: HuggingFaceModelKind;
	evidence: string[];
	warnings: string[];
} {
	const pipelineTag = stringValue(info.pipeline_tag)?.toLowerCase();
	const evidence: string[] = [];
	const warnings: string[] = [];
	const visionPipelines = new Set([
		"image-text-to-text",
		"image-to-text",
		"visual-question-answering",
		"video-text-to-text",
	]);
	if (pipelineTag && visionPipelines.has(pipelineTag)) {
		evidence.push(`Hub task: ${pipelineTag}`);
	}
	if (containsVisionValue(config))
		evidence.push("config.json contains vision fields");
	if (containsVisionValue(processor))
		evidence.push("processor configuration contains an image/vision processor");
	if (hasProjector)
		evidence.push("repository contains a GGUF vision projector");
	if (evidence.length > 0) return { kind: "vlm", evidence, warnings };

	const architectures = stringValues(config?.architectures);
	if (
		pipelineTag === "text-generation" ||
		pipelineTag === "text2text-generation" ||
		architectures.some((architecture) =>
			/(causallm|conditionalgeneration|language.?model)/i.test(architecture),
		)
	) {
		evidence.push(
			pipelineTag
				? `Hub task: ${pipelineTag}`
				: `config architecture: ${architectures[0]}`,
		);
		return { kind: "llm", evidence, warnings };
	}
	warnings.push(
		"Could not determine whether this GGUF repository is an LLM or VLM; choose the model kind before importing",
	);
	return {
		kind: "unknown",
		evidence: ["no modality metadata was detected"],
		warnings,
	};
}

function modelLicense(info: HuggingFaceModelInfo): string {
	const cardData = isRecord(info.cardData) ? info.cardData : undefined;
	const fromCard = stringValue(cardData?.license);
	if (fromCard) return fromCard;
	const tag = stringValues(info.tags).find((entry) =>
		entry.toLowerCase().startsWith("license:"),
	);
	return tag?.slice("license:".length) || "unknown";
}

function modelTags(
	info: HuggingFaceModelInfo,
	format: "mlx" | "gguf",
): string[] {
	const tags = stringValues(info.tags).filter(
		(tag) =>
			tag.length <= 64 &&
			!tag.toLowerCase().startsWith("region:") &&
			!tag.toLowerCase().startsWith("base_model:"),
	);
	if (!tags.some((tag) => tag.toLowerCase() === format)) tags.unshift(format);
	return [...new Set(tags)].slice(0, 64);
}

function modelArchitecture(
	config: Record<string, unknown> | undefined,
): string | undefined {
	return (
		stringValues(config?.architectures)[0] ?? stringValue(config?.model_type)
	);
}

function normalizeAccess(
	info: HuggingFaceModelInfo,
): HuggingFaceRepositoryAccess {
	const gated =
		info.gated === false || info.gated == null
			? false
			: typeof info.gated === "string"
				? info.gated
				: "gated";
	return { private: info.private === true, gated };
}

function toAsset(
	repoId: string,
	revision: string,
	file: ParsedTreeFile,
): HuggingFaceModelAsset {
	return {
		path: file.path,
		size: file.size,
		downloadUrl: huggingFacePinnedDownloadUrl(repoId, revision, file.path),
		oid: file.oid,
		lfsOid: file.lfsOid,
	};
}

function isGguf(path: string): boolean {
	return path.toLowerCase().endsWith(".gguf");
}

function isProjector(path: string): boolean {
	const baseName = path.split("/").pop()?.toLowerCase() ?? "";
	return (
		baseName.endsWith(".gguf") &&
		/(?:^|[-_.])(mmproj|projector)(?:[-_.]|$)/i.test(baseName)
	);
}

function splitGgufPath(
	path: string,
): { base: string; index: number; count: number } | undefined {
	const match = path.match(/^(.*)-(\d{1,6})-of-(\d{1,6})\.gguf$/i);
	if (!match) return undefined;
	const index = Number.parseInt(match[2], 10);
	const count = Number.parseInt(match[3], 10);
	if (
		!Number.isSafeInteger(index) ||
		!Number.isSafeInteger(count) ||
		index < 1 ||
		count < 1
	) {
		return undefined;
	}
	return { base: match[1], index, count };
}

function quantizationFromPath(path: string): string | undefined {
	const baseName = path.split("/").pop() ?? path;
	const stem = baseName
		.replace(/-\d{1,6}-of-\d{1,6}\.gguf$/i, "")
		.replace(/\.gguf$/i, "");
	const matches = [
		...stem.matchAll(
			/(?:^|[-_.])((?:UD[-_])?(?:IQ\d(?:_[A-Z0-9]+)*|Q\d(?:_[A-Z0-9]+)*|BF16|F16|F32|MXFP\d(?:_[A-Z0-9]+)*))(?=$|[-_.])/gi,
		),
	];
	return matches.at(-1)?.[1]?.replace("-", "_").toUpperCase();
}

function variantLabel(id: string, quantization: string | undefined): string {
	return (
		quantization ??
		id
			.split("/")
			.pop()
			?.replace(/\.gguf$/i, "") ??
		id
	);
}

function buildGgufVariants(
	files: ParsedTreeFile[],
	repoId: string,
	revision: string,
	requestedPath: string | undefined,
): HuggingFaceGgufVariant[] {
	interface VariantAccumulator {
		id: string;
		files: Array<{ file: ParsedTreeFile; index?: number; count?: number }>;
		split: boolean;
		expectedCount?: number;
	}
	const groups = new Map<string, VariantAccumulator>();
	for (const file of files) {
		if (!isGguf(file.path) || isProjector(file.path)) continue;
		const split = splitGgufPath(file.path);
		const id = split ? `${split.base}.gguf` : file.path;
		const key = portablePathKey(id);
		const group = groups.get(key) ?? {
			id,
			files: [],
			split: !!split,
			expectedCount: split?.count,
		};
		if (group.split !== !!split) {
			throw new Error(`GGUF variant path "${id}" is ambiguous`);
		}
		if (
			split &&
			group.expectedCount !== undefined &&
			group.expectedCount !== split.count
		) {
			group.expectedCount = -1;
		}
		group.files.push({
			file,
			index: split?.index,
			count: split?.count,
		});
		groups.set(key, group);
	}

	return [...groups.values()]
		.map((group): HuggingFaceGgufVariant => {
			group.files.sort(
				(left, right) =>
					(left.index ?? 0) - (right.index ?? 0) ||
					left.file.path.localeCompare(right.file.path),
			);
			const indices = new Set(group.files.map((entry) => entry.index));
			const complete =
				!group.split ||
				(group.expectedCount !== undefined &&
					group.expectedCount > 0 &&
					group.files.length === group.expectedCount &&
					Array.from(
						{ length: group.expectedCount },
						(_, index) => index + 1,
					).every((index) => indices.has(index)));
			const assets = group.files.map(({ file }) =>
				toAsset(repoId, revision, file),
			);
			const quantization = quantizationFromPath(group.id);
			return {
				id: group.id,
				label: variantLabel(group.id, quantization),
				quantization,
				files: assets,
				totalSize: assets.reduce((total, asset) => total + asset.size, 0),
				split: group.split,
				complete,
				requested: group.files.some(({ file }) => file.path === requestedPath),
			};
		})
		.sort((left, right) => left.id.localeCompare(right.id));
}

function recommendedVariant(
	variants: HuggingFaceGgufVariant[],
): HuggingFaceGgufVariant | undefined {
	const supported = variants.filter(
		(variant) => variant.complete && !variant.split,
	);
	const requested = supported.find((variant) => variant.requested);
	if (requested) return requested;
	const rank = (variant: HuggingFaceGgufVariant): number => {
		switch (variant.quantization) {
			case "Q4_K_M":
				return 0;
			case "Q5_K_M":
				return 1;
			case "Q4_K_S":
				return 2;
			case "Q8_0":
				return 3;
			default:
				return 10;
		}
	};
	return supported.toSorted(
		(left, right) =>
			rank(left) - rank(right) ||
			left.totalSize - right.totalSize ||
			left.id.localeCompare(right.id),
	)[0];
}

function recommendedProjector(
	projectors: HuggingFaceModelAsset[],
	requestedPath: string | undefined,
): HuggingFaceModelAsset | undefined {
	const requested = projectors.find(
		(projector) => projector.path === requestedPath,
	);
	if (requested) return requested;
	const rank = (asset: HuggingFaceModelAsset): number => {
		const quantization = quantizationFromPath(asset.path);
		if (quantization === "F16") return 0;
		if (quantization === "BF16") return 1;
		return 2;
	};
	return projectors.toSorted(
		(left, right) =>
			rank(left) - rank(right) ||
			left.size - right.size ||
			left.path.localeCompare(right.path),
	)[0];
}

/** Inspect a Hub repository without deciding whether its files are referenced or mirrored. */
export async function inspectHuggingFaceModelRepository(
	reference: string,
	fetcher: HuggingFaceMlxFetch = fetch,
): Promise<HuggingFaceModelImport> {
	const parsedReference = parseHuggingFaceModelReferenceWithPath(reference);
	const { value: rawInfo } = await fetchJson(
		fetcher,
		`${HUGGING_FACE_ORIGIN}/api/models/${encodeRepoId(parsedReference.repoId)}`,
		"Hugging Face model",
	);
	if (!isRecord(rawInfo)) {
		throw new Error("Hugging Face model API returned an invalid response");
	}
	const info = rawInfo as HuggingFaceModelInfo;
	const access = normalizeAccess(info);
	if (access.private || access.gated !== false) {
		throw new Error(
			"Private and gated Hugging Face repositories are not supported without an explicit authentication flow",
		);
	}
	const repoId = stringValue(info.id) ?? parsedReference.repoId;
	if (parseHuggingFaceModelReference(repoId) !== repoId) {
		throw new Error("Hugging Face returned an invalid canonical repository id");
	}
	const revision = stringValue(info.sha);
	if (!revision || !/^[a-f0-9]{40,64}$/i.test(revision)) {
		throw new Error("Hugging Face did not return an immutable commit SHA");
	}
	const tags = stringValues(info.tags);
	const libraryName = stringValue(info.library_name)?.toLowerCase();
	const markedMlx =
		libraryName === "mlx" || tags.some((tag) => tag.toLowerCase() === "mlx");

	if (
		markedMlx &&
		!parsedReference.requestedPath?.toLowerCase().endsWith(".gguf")
	) {
		const imported = await inspectHuggingFaceMlxRepository(
			parsedReference.repoId,
			fetcher,
		);
		return {
			...imported,
			format: "mlx",
			requestedPath: parsedReference.requestedPath,
			access,
		};
	}

	const files = await listRepositoryTree(fetcher, repoId, revision);
	if (
		parsedReference.requestedPath &&
		!files.some((file) => file.path === parsedReference.requestedPath)
	) {
		throw new Error(
			`Requested Hugging Face file "${parsedReference.requestedPath}" was not found in the pinned repository snapshot`,
		);
	}
	const ggufFiles = files.filter((file) => isGguf(file.path));
	if (ggufFiles.length === 0) {
		throw new Error(
			"This repository does not contain a supported MLX or GGUF model",
		);
	}
	const byPath = new Map(files.map((file) => [file.path, file]));
	const configFile = byPath.get("config.json");
	const tokenizerConfigFile = byPath.get("tokenizer_config.json");
	const processorFile =
		byPath.get("processor_config.json") ??
		byPath.get("preprocessor_config.json");
	const [config, tokenizerConfig, processor] = await Promise.all([
		fetchSmallRepositoryJson(fetcher, repoId, revision, configFile),
		fetchSmallRepositoryJson(fetcher, repoId, revision, tokenizerConfigFile),
		fetchSmallRepositoryJson(fetcher, repoId, revision, processorFile),
	]);
	const projectors = ggufFiles
		.filter((file) => isProjector(file.path))
		.map((file) => toAsset(repoId, revision, file))
		.sort((left, right) => left.path.localeCompare(right.path));
	const variants = buildGgufVariants(
		ggufFiles,
		repoId,
		revision,
		parsedReference.requestedPath,
	);
	if (variants.length === 0) {
		throw new Error(
			"This repository contains GGUF projector files but no GGUF model variants",
		);
	}
	const recommended = recommendedVariant(variants);
	const projector = recommendedProjector(
		projectors,
		parsedReference.requestedPath,
	);
	const inferred = inferGgufKind(
		info,
		config,
		processor,
		projectors.length > 0,
	);
	const [owner, repositoryName] = repoId.split("/");
	const author = stringValue(info.author) ?? owner;
	const license = modelLicense(info);
	if (license === "unknown") {
		inferred.warnings.push(
			"Hugging Face does not declare a model license; verify usage and redistribution rights",
		);
	}
	const selectedPaths = new Set(ggufFiles.map((file) => file.path));
	return {
		format: "gguf",
		repoId,
		repositoryUrl: `${HUGGING_FACE_ORIGIN}/${encodeRepoId(repoId)}`,
		revision,
		requestedPath: parsedReference.requestedPath,
		kind: inferred.kind,
		kindEvidence: inferred.evidence,
		modelName: repositoryName,
		author,
		authorUrl: `${HUGGING_FACE_ORIGIN}/${encodeURIComponent(author)}`,
		license,
		tags: modelTags(info, "gguf"),
		contextLength: inferContextLength(config, tokenizerConfig),
		architecture: modelArchitecture(config),
		access,
		warnings: inferred.warnings,
		variants,
		recommendedVariantId: recommended?.id,
		projectors,
		recommendedProjectorPath: projector?.path,
		ignoredPaths: files
			.filter((file) => !selectedPaths.has(file.path))
			.map((file) => file.path)
			.sort((left, right) => left.localeCompare(right)),
	};
}

/** Resolve and validate the exact GGUF artifacts selected for an import. */
export function resolveHuggingFaceGgufSelection(
	imported: HuggingFaceGgufRepositoryImport,
	options: HuggingFaceGgufSelectionOptions = {},
): HuggingFaceGgufSelection {
	const variantId = options.variantId ?? imported.recommendedVariantId;
	if (!variantId) {
		throw new Error(
			"This GGUF repository has no directly supported single-file variant",
		);
	}
	const variant = imported.variants.find(
		(candidate) => candidate.id === variantId,
	);
	if (!variant) throw new Error(`Unknown GGUF variant "${variantId}"`);
	if (!variant.complete) {
		throw new Error(
			`GGUF variant "${variant.label}" has an incomplete split file set`,
		);
	}
	if (variant.split) {
		throw new Error(
			`GGUF variant "${variant.label}" is split across ${variant.files.length} files; split GGUF imports are not supported yet`,
		);
	}

	const kind = options.kind ?? imported.kind;
	if (kind === "unknown") {
		throw new Error(
			"Choose whether this GGUF repository is an LLM or VLM before importing",
		);
	}
	const requestedProjectorPath =
		options.projectorPath ?? imported.recommendedProjectorPath;
	const projector = requestedProjectorPath
		? imported.projectors.find(
				(candidate) => candidate.path === requestedProjectorPath,
			)
		: undefined;
	if (requestedProjectorPath && !projector) {
		throw new Error(`Unknown GGUF projector "${requestedProjectorPath}"`);
	}
	if (kind === "vlm" && !projector) {
		throw new Error(
			"GGUF VLM imports require a selected mmproj/projector GGUF file",
		);
	}
	return {
		kind,
		variant,
		projector: kind === "vlm" ? projector : undefined,
	};
}

function applyCommonModelMetadata(
	current: IBit,
	imported: HuggingFaceModelMetadata,
	kind: "llm" | "vlm",
	format: "mlx" | "gguf",
): IBit {
	const currentParameters = (current.parameters ??
		{}) as Partial<ILlmParameters>;
	const existingTags = current.meta?.en?.tags ?? [];
	const tags = [...new Set([...imported.tags, ...existingTags, format])];
	const description =
		current.meta?.en?.description?.trim() ||
		`${imported.modelName} — ${format.toUpperCase()} ${kind.toUpperCase()} imported from Hugging Face`;
	return {
		...current,
		type: kind === "vlm" ? IBitTypes.Vlm : IBitTypes.Llm,
		name: imported.modelName,
		repository: imported.repositoryUrl,
		authors: [imported.authorUrl],
		license: imported.license,
		meta: {
			...current.meta,
			en: {
				...current.meta?.en,
				name: imported.modelName,
				description,
				tags,
				website: current.meta?.en?.website?.trim() || imported.repositoryUrl,
				docs_url: current.meta?.en?.docs_url?.trim() || imported.repositoryUrl,
				use_case:
					current.meta?.en?.use_case?.trim() ||
					(kind === "vlm" ? "Vision and chat" : "Chat"),
			},
		},
		parameters: {
			...currentParameters,
			context_length: imported.contextLength,
			model_classification:
				currentParameters.model_classification ?? DEFAULT_MODEL_CLASSIFICATION,
		},
	};
}

/**
 * Build the existing admin GGUF shape: a concrete model root and, for VLMs,
 * a separate Projection Bit. The admin endpoint subsequently mirrors each URL.
 */
export function createHuggingFaceGgufAdminDraft(
	current: IBit,
	imported: HuggingFaceGgufRepositoryImport,
	createProjection: () => IBit,
	options: HuggingFaceGgufSelectionOptions = {},
): HuggingFaceGgufAdminDraft {
	const selection = resolveHuggingFaceGgufSelection(imported, options);
	const modelAsset = selection.variant.files[0];
	const common = applyCommonModelMetadata(
		current,
		imported,
		selection.kind,
		"gguf",
	);
	const currentParameters = common.parameters as ILlmParameters;
	const root: IBit = {
		...common,
		download_link: modelAsset.downloadUrl,
		file_name: modelAsset.path,
		size: modelAsset.size,
		parameters: {
			...currentParameters,
			provider: {
				...currentParameters.provider,
				provider_name: "Local",
				model_id: imported.repoId,
				version: imported.revision,
				params: currentParameters.provider?.params ?? {},
			},
		},
	};
	const projection = selection.projector
		? {
				...createProjection(),
				type: IBitTypes.Projection,
				download_link: selection.projector.downloadUrl,
				file_name: selection.projector.path,
				size: selection.projector.size,
				repository: imported.repositoryUrl,
				authors: [imported.authorUrl],
				license: imported.license,
			}
		: undefined;
	return { root, projection, selection };
}

/** Create the versioned, self-contained source manifest stored by one user Bit. */
export function createHuggingFaceUserMlxManifest(
	imported: HuggingFaceMlxRepositoryImport | HuggingFaceMlxImport,
): HuggingFaceUserMlxManifest {
	return {
		schema: 1,
		repo_id: imported.repoId,
		revision: imported.revision,
		format: "mlx",
		files: imported.assets.map((asset) => ({
			path: asset.path,
			size: asset.size,
			...(asset.oid ? { oid: asset.oid } : {}),
			...(asset.lfsOid ? { lfs_oid: asset.lfsOid } : {}),
		})),
	};
}

/**
 * Build one user-owned virtual MLX Bit. Its Hub files remain immutable direct
 * references and are materialized as ephemeral dependency Bits by the core.
 */
export function applyHuggingFaceMlxImportToUserBit(
	current: IBit,
	imported: HuggingFaceMlxRepositoryImport | HuggingFaceMlxImport,
): IBit {
	const kind = imported.kind;
	const common = applyCommonModelMetadata(current, imported, kind, "mlx");
	const currentParameters = common.parameters as ILlmParameters;
	return {
		...common,
		download_link: null,
		file_name: null,
		size: 0,
		dependencies: [],
		parameters: {
			...currentParameters,
			huggingface: createHuggingFaceUserMlxManifest(imported),
			provider: {
				...currentParameters.provider,
				provider_name: "MLX",
				model_id: imported.repoId,
				version: imported.revision,
				params: currentParameters.provider?.params ?? {},
			},
		},
	};
}
