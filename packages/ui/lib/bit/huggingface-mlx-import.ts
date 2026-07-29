import { type IBit, IBitTypes, type ILlmParameters } from "../schema";
import {
	inferMlxAssetBitType,
	mlxAssetPathError,
	validateMlxModelAssets,
} from "./mlx-model-pack";

const HUGGING_FACE_ORIGIN = "https://huggingface.co";
const MAX_TREE_PAGES = 32;
const MAX_RUNTIME_FILES = 512;
const MAX_JSON_FILE_SIZE = 8 * 1024 * 1024;
const MAX_CONTEXT_LENGTH = 2_000_000;
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

export type HuggingFaceMlxModelKind = "llm" | "vlm";

export type HuggingFaceMlxFetch = (
	input: string,
	init?: RequestInit,
) => Promise<Response>;

export interface HuggingFaceMlxAsset {
	path: string;
	size: number;
	downloadUrl: string;
	oid?: string;
	lfsOid?: string;
}

export interface HuggingFaceMlxImport {
	repoId: string;
	repositoryUrl: string;
	revision: string;
	kind: HuggingFaceMlxModelKind;
	kindEvidence: string[];
	modelName: string;
	author: string;
	authorUrl: string;
	license: string;
	tags: string[];
	contextLength: number;
	architecture?: string;
	assets: HuggingFaceMlxAsset[];
	ignoredPaths: string[];
	totalSize: number;
	warnings: string[];
}

/** Apply discovered Hub metadata to a draft virtual MLX root Bit. */
export function applyHuggingFaceMlxImportToBit(
	current: IBit,
	imported: HuggingFaceMlxImport,
): IBit {
	const type = imported.kind === "vlm" ? IBitTypes.Vlm : IBitTypes.Llm;
	const currentParameters = (current.parameters ??
		{}) as Partial<ILlmParameters>;
	const existingTags = current.meta?.en?.tags ?? [];
	const tags = [...new Set([...imported.tags, ...existingTags, "mlx"])];
	const description =
		current.meta?.en?.description?.trim() ||
		`${imported.modelName} — MLX ${imported.kind.toUpperCase()} imported from Hugging Face`;

	return {
		...current,
		type,
		name: imported.modelName,
		repository: imported.repositoryUrl,
		authors: [imported.authorUrl],
		license: imported.license,
		download_link: null,
		file_name: null,
		size: 0,
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
					(imported.kind === "vlm" ? "Vision and chat" : "Chat"),
			},
		},
		parameters: {
			...currentParameters,
			context_length: imported.contextLength,
			model_classification:
				currentParameters.model_classification ?? DEFAULT_MODEL_CLASSIFICATION,
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

/** Turn a discovered immutable Hub manifest into draft dependency Bits. */
export function createHuggingFaceMlxAssetBits(
	imported: HuggingFaceMlxImport,
	createAsset: (fileName?: string) => IBit,
): IBit[] {
	return imported.assets.map((file) => ({
		...createAsset(file.path),
		type: inferMlxAssetBitType(file.path),
		file_name: file.path,
		download_link: file.downloadUrl,
		size: file.size,
	}));
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

function encodeRepoPath(path: string): string {
	return path.split("/").map(encodeURIComponent).join("/");
}

function portablePathKey(path: string): string {
	return path.normalize("NFC").toLowerCase();
}

/**
 * Accept a canonical Hub model URL or `owner/repository`. Subpaths such as
 * `/tree/main` and `/blob/main/config.json` are intentionally reduced to the
 * repository because discovery is pinned to the API-returned commit SHA.
 */
export function parseHuggingFaceModelReference(reference: string): string {
	const input = reference.trim();
	if (!input)
		throw new Error("Enter a Hugging Face model URL or owner/repository");

	let components: string[];
	if (/^https?:\/\//i.test(input)) {
		let url: URL;
		try {
			url = new URL(input);
		} catch {
			throw new Error("The Hugging Face repository URL is invalid");
		}
		if (
			url.protocol !== "https:" ||
			url.hostname.toLowerCase() !== "huggingface.co" ||
			url.username ||
			url.password
		) {
			throw new Error(
				"Only exact https://huggingface.co model URLs are supported",
			);
		}
		components = url.pathname
			.split("/")
			.filter(Boolean)
			.slice(0, 2)
			.map((component) => {
				try {
					return decodeURIComponent(component);
				} catch {
					throw new Error(
						"The Hugging Face repository URL contains invalid escaping",
					);
				}
			});
	} else {
		components = input.replace(/\/+$/, "").split("/");
	}

	if (components.length !== 2) {
		throw new Error(
			"Use a Hugging Face model URL or repository id such as owner/model",
		);
	}

	const [owner, rawRepository] = components;
	const repository = rawRepository.endsWith(".git")
		? rawRepository.slice(0, -4)
		: rawRepository;
	const safeComponent = /^[A-Za-z0-9](?:[A-Za-z0-9._-]{0,94}[A-Za-z0-9])?$/;
	if (!safeComponent.test(owner) || !safeComponent.test(repository)) {
		throw new Error("The Hugging Face owner or repository name is invalid");
	}

	return `${owner}/${repository}`;
}

export function huggingFacePinnedDownloadUrl(
	repoId: string,
	revision: string,
	path: string,
): string {
	if (!/^[a-f0-9]{40,64}$/i.test(revision)) {
		throw new Error("Hugging Face did not return a valid immutable revision");
	}
	const pathError = mlxAssetPathError(path);
	if (pathError)
		throw new Error(`Unsafe Hugging Face path "${path}": ${pathError}`);
	return `${HUGGING_FACE_ORIGIN}/${encodeRepoId(repoId)}/resolve/${encodeURIComponent(revision)}/${encodeRepoPath(path)}?download=true`;
}

/**
 * Validate a credential-free, immutable Hugging Face GGUF download URL.
 *
 * User-owned GGUF bits are direct references rather than CDN mirrors, so only
 * canonical model-file URLs pinned to a full Hub commit SHA are safe to keep.
 */
export function validateHuggingFacePinnedGgufDownloadUrl(
	reference: string,
): void {
	const expected =
		"Use an HTTPS huggingface.co resolve URL pinned to a full commit SHA";
	const input = reference.trim();
	let url: URL;
	try {
		url = new URL(input);
	} catch {
		throw new Error(expected);
	}

	if (
		input !== reference ||
		url.protocol !== "https:" ||
		url.hostname.toLowerCase() !== "huggingface.co" ||
		url.port ||
		url.username ||
		url.password ||
		url.hash
	) {
		throw new Error(expected);
	}

	const query = [...url.searchParams.entries()];
	if (
		query.length > 1 ||
		(query.length === 1 &&
			(query[0]?.[0] !== "download" || query[0]?.[1] !== "true"))
	) {
		throw new Error(
			"Hugging Face download URLs cannot contain credentials or mutable query parameters",
		);
	}

	const encodedSegments = url.pathname.split("/").slice(1);
	if (
		encodedSegments.length < 5 ||
		encodedSegments.some((segment) => !segment) ||
		encodedSegments[2] !== "resolve"
	) {
		throw new Error(expected);
	}

	const segments = encodedSegments.map((segment) => {
		let decoded: string;
		try {
			decoded = decodeURIComponent(segment);
		} catch {
			throw new Error("The Hugging Face download URL has invalid escaping");
		}
		if (
			!decoded ||
			decoded === "." ||
			decoded === ".." ||
			decoded.includes("/") ||
			decoded.includes("\\") ||
			decoded.includes("\0")
		) {
			throw new Error("The Hugging Face download URL has an unsafe file path");
		}
		return decoded;
	});

	const safeRepoComponent = /^[A-Za-z0-9](?:[A-Za-z0-9._-]{0,94}[A-Za-z0-9])?$/;
	if (
		!safeRepoComponent.test(segments[0] ?? "") ||
		!safeRepoComponent.test(segments[1] ?? "") ||
		!/^[a-f0-9]{40,64}$/i.test(segments[3] ?? "")
	) {
		throw new Error(expected);
	}

	const filePath = segments.slice(4).join("/");
	const pathError = mlxAssetPathError(filePath);
	if (pathError) {
		throw new Error(`Unsafe Hugging Face GGUF path: ${pathError}`);
	}
	if (!filePath.toLowerCase().endsWith(".gguf")) {
		throw new Error("The Hugging Face download URL must reference a GGUF file");
	}
}

function runtimeSupportFile(path: string): boolean {
	const lowerPath = path.toLowerCase();
	const baseName = lowerPath.split("/").pop() ?? "";
	return (
		lowerPath.endsWith(".json") ||
		lowerPath.endsWith(".jinja") ||
		lowerPath.endsWith(".tiktoken") ||
		baseName === "tokenizer.model" ||
		baseName === "sentencepiece.bpe.model" ||
		baseName === "spiece.model" ||
		baseName === "merges.txt" ||
		baseName === "vocab.txt"
	);
}

function isSafetensors(path: string): boolean {
	return path.toLowerCase().endsWith(".safetensors");
}

function parseTreeFile(entry: HuggingFaceTreeFile): ParsedTreeFile | undefined {
	if (entry.type !== "file" || typeof entry.path !== "string") return undefined;
	const path = entry.path;
	const pathError = mlxAssetPathError(path);
	if (pathError) {
		throw new Error(
			`Unsafe Hugging Face repository path "${path}": ${pathError}`,
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
		throw new Error(`Hugging Face did not report a valid size for "${path}"`);
	}

	return {
		path,
		size: rawSize,
		oid: stringValue(entry.oid),
		lfsOid: stringValue(lfs?.oid),
	};
}

function apiErrorMessage(status: number, label: string, body: string): string {
	if (status === 401 || status === 403) {
		return `${label} is private, gated, or not accessible. Shared MLX imports currently require a public repository`;
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
		value <= MAX_CONTEXT_LENGTH
		? value
		: undefined;
}

function inferContextLength(
	config: Record<string, unknown> | undefined,
	tokenizerConfig: Record<string, unknown> | undefined,
): number {
	const textConfig = nestedRecord(config, "text_config");
	const candidates = [
		textConfig?.max_position_embeddings,
		textConfig?.model_max_length,
		config?.max_position_embeddings,
		config?.model_max_length,
		config?.max_seq_len,
		config?.seq_length,
		config?.n_positions,
		tokenizerConfig?.model_max_length,
	];
	for (const candidate of candidates) {
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

function inferModelKind(
	info: HuggingFaceModelInfo,
	config: Record<string, unknown> | undefined,
	processor: Record<string, unknown> | undefined,
): { kind: HuggingFaceMlxModelKind; evidence: string[]; warnings: string[] } {
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
	if (containsVisionValue(processor)) {
		evidence.push("processor configuration contains an image/vision processor");
	}

	const kind: HuggingFaceMlxModelKind = evidence.length > 0 ? "vlm" : "llm";
	if (kind === "vlm" && pipelineTag === "text-generation") {
		warnings.push(
			"Hub labels this model as text-generation, but its repository contains vision signals",
		);
	}
	if (!pipelineTag) {
		warnings.push(
			"Hugging Face does not declare a pipeline task for this model",
		);
	}
	if (kind === "llm") evidence.push("no vision metadata was detected");
	return { kind, evidence, warnings };
}

function referencedSafetensorShards(
	index: Record<string, unknown> | undefined,
): Set<string> | undefined {
	if (!index) return undefined;
	const weightMap = index.weight_map;
	if (!isRecord(weightMap)) {
		throw new Error("model.safetensors.index.json is missing its weight_map");
	}
	const shards = new Set<string>();
	for (const rawPath of Object.values(weightMap)) {
		if (typeof rawPath !== "string") {
			throw new Error(
				"model.safetensors.index.json contains an invalid shard path",
			);
		}
		const pathError = mlxAssetPathError(rawPath);
		if (pathError || !isSafetensors(rawPath)) {
			throw new Error(
				`model.safetensors.index.json contains unsafe shard path "${rawPath}"`,
			);
		}
		shards.add(rawPath);
	}
	if (shards.size === 0) {
		throw new Error(
			"model.safetensors.index.json does not reference any shards",
		);
	}
	return shards;
}

function selectRuntimeFiles(
	files: ParsedTreeFile[],
	shards: Set<string> | undefined,
): { selected: ParsedTreeFile[]; ignored: string[] } {
	const available = new Set(files.map((file) => file.path));
	if (shards) {
		for (const shard of shards) {
			if (!available.has(shard)) {
				throw new Error(
					`model.safetensors.index.json references missing shard "${shard}"`,
				);
			}
		}
	}

	const selected: ParsedTreeFile[] = [];
	const ignored: string[] = [];
	for (const file of files) {
		const includeWeight =
			isSafetensors(file.path) && (!shards || shards.has(file.path));
		if (includeWeight || runtimeSupportFile(file.path)) selected.push(file);
		else ignored.push(file.path);
	}
	selected.sort((left, right) =>
		left.path === right.path ? 0 : left.path < right.path ? -1 : 1,
	);
	ignored.sort((left, right) => (left === right ? 0 : left < right ? -1 : 1));
	if (selected.length > MAX_RUNTIME_FILES) {
		throw new Error(
			`MLX repository contains ${selected.length} runtime files; the importer limit is ${MAX_RUNTIME_FILES}`,
		);
	}
	return { selected, ignored };
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

function modelTags(info: HuggingFaceModelInfo): string[] {
	const tags = stringValues(info.tags).filter(
		(tag) =>
			tag.length <= 64 &&
			!tag.toLowerCase().startsWith("region:") &&
			!tag.toLowerCase().startsWith("base_model:"),
	);
	if (!tags.some((tag) => tag.toLowerCase() === "mlx")) tags.unshift("mlx");
	return [...new Set(tags)].slice(0, 64);
}

function modelArchitecture(
	config: Record<string, unknown> | undefined,
): string | undefined {
	const architectures = stringValues(config?.architectures);
	return architectures[0] ?? stringValue(config?.model_type);
}

/**
 * Inspect a public Hugging Face MLX repository and produce the complete,
 * immutable dependency manifest used by the store authoring UI.
 */
export async function inspectHuggingFaceMlxRepository(
	reference: string,
	fetcher: HuggingFaceMlxFetch = fetch,
): Promise<HuggingFaceMlxImport> {
	const requestedRepoId = parseHuggingFaceModelReference(reference);
	const encodedRequestedRepoId = encodeRepoId(requestedRepoId);
	const { value: rawInfo } = await fetchJson(
		fetcher,
		`${HUGGING_FACE_ORIGIN}/api/models/${encodedRequestedRepoId}`,
		"Hugging Face model",
	);
	if (!isRecord(rawInfo)) {
		throw new Error("Hugging Face model API returned an invalid response");
	}
	const info = rawInfo as HuggingFaceModelInfo;
	if (info.private === true || (info.gated !== false && info.gated != null)) {
		throw new Error(
			"Private and gated Hugging Face repositories cannot be copied into the shared store without an explicit redistribution/authentication flow",
		);
	}

	const repoId = stringValue(info.id) ?? requestedRepoId;
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
	if (!markedMlx) {
		throw new Error(
			"This repository is not marked as an MLX model by Hugging Face",
		);
	}

	const files = await listRepositoryTree(fetcher, repoId, revision);
	const filesByPath = new Map(files.map((file) => [file.path, file]));
	const configFile = filesByPath.get("config.json");
	const tokenizerConfigFile = filesByPath.get("tokenizer_config.json");
	const indexFile = filesByPath.get("model.safetensors.index.json");
	const processorFile =
		filesByPath.get("processor_config.json") ??
		filesByPath.get("preprocessor_config.json");
	const [config, tokenizerConfig, index, processor] = await Promise.all([
		fetchSmallRepositoryJson(fetcher, repoId, revision, configFile),
		fetchSmallRepositoryJson(fetcher, repoId, revision, tokenizerConfigFile),
		fetchSmallRepositoryJson(fetcher, repoId, revision, indexFile),
		fetchSmallRepositoryJson(fetcher, repoId, revision, processorFile),
	]);

	const shards = referencedSafetensorShards(index);
	const { selected, ignored } = selectRuntimeFiles(files, shards);
	const inferred = inferModelKind(info, config, processor);
	const assets = selected.map((file) => ({
		path: file.path,
		size: file.size,
		downloadUrl: huggingFacePinnedDownloadUrl(repoId, revision, file.path),
		oid: file.oid,
		lfsOid: file.lfsOid,
	}));
	const manifestErrors = validateMlxModelAssets(
		assets.map((asset) => ({
			file_name: asset.path,
			download_link: asset.downloadUrl,
		})),
		inferred.kind === "vlm",
	);
	if (manifestErrors.length > 0) {
		throw new Error(
			`Repository is not a usable MLX bundle: ${manifestErrors.join(". ")}`,
		);
	}

	const [owner, repositoryName] = repoId.split("/");
	const author = stringValue(info.author) ?? owner;
	const totalSize = assets.reduce((total, asset) => total + asset.size, 0);
	if (!Number.isSafeInteger(totalSize)) {
		throw new Error("The MLX repository is too large to represent safely");
	}
	const license = modelLicense(info);
	if (license === "unknown") {
		inferred.warnings.push(
			"Hugging Face does not declare a model license; verify redistribution rights before upload",
		);
	}
	if (totalSize > 16 * 1024 * 1024 * 1024) {
		inferred.warnings.push(
			"This model exceeds 16 GiB; many iPhone and iPad devices will not have enough unified memory to run it",
		);
	}

	return {
		repoId,
		repositoryUrl: `${HUGGING_FACE_ORIGIN}/${encodeRepoId(repoId)}`,
		revision,
		kind: inferred.kind,
		kindEvidence: inferred.evidence,
		modelName: repositoryName,
		author,
		authorUrl: `${HUGGING_FACE_ORIGIN}/${encodeURIComponent(author)}`,
		license,
		tags: modelTags(info),
		contextLength: inferContextLength(config, tokenizerConfig),
		architecture: modelArchitecture(config),
		assets,
		ignoredPaths: ignored,
		totalSize,
		warnings: inferred.warnings,
	};
}
