import { sha256 } from "@noble/hashes/sha2";
import {
	type HuggingFaceMlxFetch,
	huggingFacePinnedDownloadUrl,
	parseHuggingFaceModelReference,
	validateHuggingFacePinnedGgufDownloadUrl,
} from "../../../../../lib/bit/huggingface-mlx-import";
import { inspectHuggingFaceModelRepository } from "../../../../../lib/bit/huggingface-model-import";
import { shapeFromConfig } from "../../../../../lib/device-management/model/models/fit";
import type { ModelAssetDigest } from "../../../../../lib/device-management/models";
import type { IBit } from "../../../../../lib/schema";
import type { IBitState } from "../../../../../state/backend-state/bit-state";
import {
	type OptionFile,
	type SourceFacts,
	fromHubPack,
	fromHuggingFace,
	fromUserBit,
	huggingFaceOrigin,
	userBitLinks,
} from "./model-options";

/*
 * The add-model wizard's reads outside the device: a Hugging Face repository
 * and the facts the fit check uses (its `config.json`, else its base model's;
 * GGUF parameters), Git LFS digests at a pinned commit, and sha256
 * fingerprints of files without published digests.
 */

export type Fetcher = HuggingFaceMlxFetch;

const HUGGING_FACE = "https://huggingface.co";
const JSON_MAX = 8 * 1024 * 1024;
const TREE_PAGES = 10;

type Json = Record<string, unknown>;

const record = (value: unknown) =>
	typeof value === "object" && value !== null && !Array.isArray(value)
		? (value as Json)
		: undefined;

const positive = (value: unknown) =>
	typeof value === "number" && Number.isSafeInteger(value) && value > 0
		? value
		: undefined;

const encodePath = (path: string) =>
	path.split("/").map(encodeURIComponent).join("/");

async function readJson(fetcher: Fetcher, url: string) {
	const response = await fetcher(url, {
		headers: { Accept: "application/json" },
	});
	if (!response.ok)
		throw new Error(`Reading ${url} failed with HTTP ${response.status}.`);
	const text = await response.text();
	if (text.length > JSON_MAX)
		throw new Error(`${url} is larger than ${JSON_MAX} bytes.`);
	return JSON.parse(text) as unknown;
}

/** Facts are a best effort: whatever can't be read is estimated from the file sizes. */
const quietly = <T>(read: Promise<T>) => read.catch((): undefined => undefined);

/** The first base model a model card names, e.g. a GGUF repository's original. */
function baseModelOf(info: Json | undefined) {
	const card = record(info?.cardData);
	const base = card?.base_model;
	const first = Array.isArray(base) ? base[0] : base;
	if (typeof first !== "string") return undefined;
	try {
		return parseHuggingFaceModelReference(first);
	} catch {
		return undefined;
	}
}

async function configOf(
	repo: string,
	revision: string,
	info: Json | undefined,
	fetcher: Fetcher,
) {
	const own = await quietly(
		readJson(
			fetcher,
			huggingFacePinnedDownloadUrl(repo, revision, "config.json"),
		),
	);
	if (own !== undefined) return own;
	const base = baseModelOf(info);
	return base
		? quietly(
				readJson(
					fetcher,
					`${HUGGING_FACE}/${encodePath(base)}/resolve/main/config.json`,
				),
			)
		: undefined;
}

const EMBEDDING_TASKS = new Set(["feature-extraction", "sentence-similarity"]);

function factsOf(info: Json | undefined, config: unknown) {
	const gguf = record(info?.gguf);
	const params = positive(gguf?.total);
	const context = positive(gguf?.context_length);
	const quantBits = positive(record(record(config)?.quantization)?.bits);
	const facts: SourceFacts = {
		...shapeFromConfig(config),
		...(params ? { params } : {}),
		...(context ? { contextLength: context } : {}),
		...(quantBits ? { quantBits } : {}),
		...(EMBEDDING_TASKS.has(String(info?.pipeline_tag))
			? { embeddingTask: true }
			: {}),
	};
	return facts;
}

/** Parameters, layers, context-cache size and context limit of a repository at `revision`. */
export async function repositoryFacts(
	repo: string,
	revision: string,
	fetcher: Fetcher = fetch,
) {
	const info = record(
		await quietly(
			readJson(
				fetcher,
				`${HUGGING_FACE}/api/models/${encodePath(repo)}/revision/${encodeURIComponent(revision)}`,
			),
		),
	);
	return factsOf(info, await configOf(repo, revision, info, fetcher));
}

/** A Hugging Face repository: its versions, pinned to its current commit. */
export async function huggingFaceCandidate(
	reference: string,
	fetcher: Fetcher = fetch,
) {
	const imported = await inspectHuggingFaceModelRepository(reference, fetcher);
	const facts = await repositoryFacts(
		imported.repoId,
		imported.revision,
		fetcher,
	);
	return fromHuggingFace(imported, facts);
}

function originFacts(bit: IBit, fetcher: Fetcher) {
	const origin = huggingFaceOrigin(bit);
	return origin
		? repositoryFacts(origin.repo, origin.revision, fetcher)
		: Promise.resolve({});
}

/** A hub model with every file of its pack. */
export async function hubCandidate(
	bit: IBit,
	bits: Pick<IBitState, "getPackFromBit">,
	fetcher: Fetcher = fetch,
) {
	const [pack, facts] = await Promise.all([
		bits.getPackFromBit(bit),
		originFacts(bit, fetcher),
	]);
	return fromHubPack(bit, pack.bits, facts);
}

/** One of the person's Bits, with the Git LFS digests of its pinned files. */
export async function userBitCandidate(bit: IBit, fetcher: Fetcher = fetch) {
	const [digests, facts] = await Promise.all([
		lfsDigests(userBitLinks(bit), fetcher),
		originFacts(bit, fetcher),
	]);
	return fromUserBit(bit, digests, facts);
}

/* Git LFS digests. */

interface PinnedFile {
	link: string;
	repo: string;
	revision: string;
	path: string;
	folder: string;
}

/** A pinned `resolve` link split into repository, commit and path; undefined for any other link. */
function pinnedFile(link: string) {
	try {
		validateHuggingFacePinnedGgufDownloadUrl(link);
	} catch {
		return undefined;
	}
	const segments = new URL(link).pathname
		.split("/")
		.slice(1)
		.map(decodeURIComponent);
	const [owner = "", name = "", , revision = "", ...path] = segments;
	const file: PinnedFile = {
		link,
		repo: `${owner}/${name}`,
		revision,
		path: path.join("/"),
		folder: path.slice(0, -1).join("/"),
	};
	return file;
}

function nextPage(link: string | null) {
	const next = link?.match(/<([^>]+)>\s*;\s*rel="?next"?/iu)?.[1];
	if (!next) return undefined;
	const url = new URL(next, HUGGING_FACE);
	return url.origin === HUGGING_FACE ? url.toString() : undefined;
}

interface TreeEntry {
	path?: unknown;
	lfs?: { oid?: unknown };
}

/** A busy Hugging Face (429, 5xx) or a dropped connection is worth another try. */
function listingFailed(file: PinnedFile, reason: string, transient: boolean) {
	const where = file.folder ? `${file.repo}/${file.folder}` : file.repo;
	return new Error(
		`Reading the Git LFS fingerprints of ${where} at ${file.revision} from Hugging Face failed: ${reason}.${transient ? " Try again in a moment." : ""}`,
	);
}

async function listingPage(file: PinnedFile, url: string, fetcher: Fetcher) {
	try {
		return await fetcher(url, { headers: { Accept: "application/json" } });
	} catch (error) {
		const reason = error instanceof Error ? error.message : String(error);
		throw listingFailed(file, reason, true);
	}
}

/** One folder of a repository at a commit, page by page; undefined when Hugging Face has no such commit or folder. */
async function folderEntries(file: PinnedFile, fetcher: Fetcher) {
	const folder = file.folder ? `/${encodePath(file.folder)}` : "";
	let url: string | undefined =
		`${HUGGING_FACE}/api/models/${encodePath(file.repo)}/tree/${encodeURIComponent(file.revision)}${folder}`;
	const entries: TreeEntry[] = [];
	for (let page = 0; url && page < TREE_PAGES; page++) {
		const response = await listingPage(file, url, fetcher);
		if (response.status === 404) return undefined;
		if (!response.ok)
			throw listingFailed(
				file,
				`HTTP ${response.status}`,
				response.status === 429 || response.status >= 500,
			);
		const body = (await response.json()) as unknown;
		if (Array.isArray(body)) entries.push(...(body as TreeEntry[]));
		url = nextPage(response.headers.get("link"));
	}
	return entries;
}

/**
 * The Git LFS sha256 of each pinned Hugging Face link, by link; links without
 * one, or at a commit Hugging Face doesn't have, are left out. Any other
 * failed listing fails the lookup, so a busy Hugging Face never reads as a
 * file without a fingerprint.
 */
export async function lfsDigests(
	links: readonly string[],
	fetcher: Fetcher = fetch,
) {
	const digests = new Map<string, string>();
	for (const file of links.map(pinnedFile)) {
		if (!file) continue;
		const entries = (await folderEntries(file, fetcher)) ?? [];
		const entry = entries.find((candidate) => candidate.path === file.path);
		const oid = entry?.lfs?.oid;
		if (typeof oid === "string" && /^[0-9a-f]{64}$/u.test(oid))
			digests.set(file.link, oid);
	}
	return digests;
}

/* Fingerprints. */

const hex = (bytes: Uint8Array) =>
	[...bytes].map((byte) => byte.toString(16).padStart(2, "0")).join("");

const hostOf = (source: string) => {
	try {
		return new URL(source).host;
	} catch {
		return source;
	}
};

async function digestFrom(
	file: OptionFile,
	source: string,
	fetcher: Fetcher,
	signal: AbortSignal | undefined,
) {
	const response = await fetcher(source, signal ? { signal } : undefined);
	if (!response.ok)
		throw new Error(
			`Downloading ${file.file_name} from ${hostOf(source)} to fingerprint it failed with HTTP ${response.status}.`,
		);
	if (!response.body)
		throw new Error("The model download has no response body.");
	const reader = response.body.getReader();
	const hash = sha256.create();
	let size = 0;
	try {
		while (true) {
			signal?.throwIfAborted();
			const next = await reader.read();
			if (next.done) break;
			size += next.value.byteLength;
			if (!Number.isSafeInteger(size) || size > file.size) break;
			hash.update(next.value);
		}
		if (size !== file.size)
			throw new Error(
				`${file.file_name} from ${hostOf(source)} has ${size} bytes; ${file.size} were expected.`,
			);
		signal?.throwIfAborted();
		return {
			algorithm: "sha256",
			hex: hex(hash.digest()),
		} satisfies ModelAssetDigest;
	} finally {
		hash.destroy();
		await reader.cancel().catch(() => {});
		reader.releaseLock();
	}
}

/** The sha256 from the first source that answers with the expected bytes; `signal` stops the download. */
export async function fingerprint(
	file: OptionFile,
	fetcher: Fetcher = fetch,
	signal?: AbortSignal,
) {
	if (!Number.isSafeInteger(file.size) || file.size <= 0)
		throw new Error(`${file.file_name} has an invalid size.`);
	let failure: unknown = new Error(
		`${file.file_name} has no source to fingerprint it from.`,
	);
	for (const source of file.sources) {
		signal?.throwIfAborted();
		try {
			return await digestFrom(file, source, fetcher, signal);
		} catch (error) {
			signal?.throwIfAborted();
			failure = error;
		}
	}
	throw failure;
}

export interface FingerprintOptions {
	/** Counts the finished files. */
	onFile?: (done: number) => void;
	/** Stops the downloads; the digests are then never returned. */
	signal?: AbortSignal;
}

/** Fingerprints every file in turn. */
export async function fingerprintAll(
	files: readonly OptionFile[],
	fetcher: Fetcher = fetch,
	{ onFile, signal }: FingerprintOptions = {},
) {
	const digests = new Map<string, ModelAssetDigest>();
	for (const file of files) {
		digests.set(file.file_name, await fingerprint(file, fetcher, signal));
		signal?.throwIfAborted();
		onFile?.(digests.size);
	}
	return digests;
}
