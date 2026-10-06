import type { IBitState } from "../../state/backend-state/bit-state";
import type { IDownloadProgress } from "../bit/bit";
import { isTauri } from "../platform";
import type { PreparedModelAsset } from "./artifacts";
import type {
	ModelAssetDescriptor,
	ModelAssetDigest,
	ModelAssetStatus,
} from "./models";
import { desktopModelPush } from "./native-client";
import type { TunnelModelAssetPush, TunnelUploadFile } from "./tunnel-data";

/*
 * "Send from this computer" (plan §3.1): when a device can't fetch a model
 * asset itself, this computer sends the bytes as a `model_asset` data stream:
 * from its own Bit store on desktop, else as a download it streams through
 * (the "tunnelled download"). The device verifies the digest as always, so
 * where the bytes come from never affects trust.
 */

const PUSH_ATTEMPTS = 3;
const SOURCE_ATTEMPTS = 3;
const BIT_STORE_CHUNK = 1024 * 1024;
const MAX_REMEMBERED = 256;

/** Test seam: the pauses before each retry. */
export const modelPushSeams = { retryMs: [1_000, 4_000, 15_000] };

/** One device's push: `pushModelAsset(deviceId, input)` of the live session, bound to the device. */
export interface ModelPush {
	(input: TunnelModelAssetPush): Promise<ModelAssetStatus>;
	/** Why this push can't carry `file`'s bytes; asked before the device's job is handed over. */
	refuses?(file: TunnelUploadFile): string | undefined;
}

/** Reads `length` bytes at `offset` of `<hash>/<fileName>` in this computer's Bit store (desktop). */
export type BitStoreRead = (
	hash: string,
	fileName: string,
	offset: number,
	length: number,
) => Promise<ArrayBuffer>;

/** What this computer knows about an asset it may send. */
export interface PushableAsset {
	descriptor: ModelAssetDescriptor;
	/** The owning Bit's directory in a Bit store: its file is `<bitHash>/<file_name>`. */
	bitHash?: string;
	/** The pinned Bit whose pack holds the asset; desktop downloads it to get the file. */
	pin?: { id: string; hub?: string };
}

export interface PushProgress {
	/** Bytes the device holds. */
	bytes: number;
	total: number;
}

export interface PushFile extends TunnelUploadFile {
	readonly from: "bit_store" | "download";
	/** A Bit store file's place, `<hash>/<fileName>`: the desktop app sends it itself. */
	readonly local?: { hash: string; fileName: string };
	/** Resolves once the bytes can be read; rejects with why not. */
	ready?(): Promise<void>;
	close(): void;
}

export interface PushEnvironment {
	/** Desktop: reads this computer's Bit store. */
	readBitStore?: BitStoreRead;
	/** Desktop: downloads the asset's pinned Bit into this computer's Bit store; `progress` gets the asset's bytes. */
	downloadLocally?(
		asset: PushableAsset,
		progress: (bytes: number) => void,
	): Promise<void>;
	/** Bytes of the asset this computer downloaded into its Bit store so far, before any is sent. */
	onLocalDownload?(bytes: number): void;
	fetcher?: typeof fetch;
	signal?: AbortSignal;
}

const hostOf = (source: string) => {
	try {
		return new URL(source).host;
	} catch {
		return source;
	}
};

const digestKey = (digest: ModelAssetDigest) =>
	`${digest.algorithm}/${digest.hex}`;

function pause(ms: number, signal?: AbortSignal): Promise<void> {
	return new Promise((resolve, reject) => {
		const abort = () => {
			clearTimeout(timer);
			reject(signal?.reason ?? new Error("The model push was cancelled."));
		};
		const timer = setTimeout(() => {
			signal?.removeEventListener("abort", abort);
			resolve();
		}, ms);
		signal?.addEventListener("abort", abort, { once: true });
	});
}

/**
 * One device's push: in the desktop app its native push, else the live
 * session's, undefined while the session manager offers none.
 */
export function modelPushOf(
	live: object,
	deviceId: string,
): ModelPush | undefined {
	const push = (live as { pushModelAsset?: unknown }).pushModelAsset;
	const viaLive: ModelPush | undefined =
		typeof push === "function"
			? (input) =>
					(
						push as (
							deviceId: string,
							input: TunnelModelAssetPush,
						) => Promise<ModelAssetStatus>
					).call(live, deviceId, input)
			: undefined;
	return isTauri() ? desktopModelPush(deviceId, viaLive) : viaLive;
}

/** The bytes of Bit `hash` that a pack download's progress events report, if they name it. */
function downloadedOf(events: readonly IDownloadProgress[], hash?: string) {
	let bytes: number | undefined;
	for (const event of events)
		if (event.hash === hash) bytes = Math.max(bytes ?? 0, event.downloaded);
	return bytes;
}

/** Desktop: this computer's Bit store, and the Bit download that fills it. */
export function desktopPushEnvironment(
	bits: Pick<IBitState, "getBit" | "getPackFromBit" | "downloadBit">,
): PushEnvironment {
	return {
		async readBitStore(hash, fileName, offset, length) {
			const { invoke } = await import("@tauri-apps/api/core");
			return invoke<ArrayBuffer>("read_bit_store_chunk", {
				hash,
				fileName,
				offset,
				length,
			});
		},
		async downloadLocally(asset, progress) {
			if (!asset.pin) return;
			const bit = await bits.getBit(asset.pin.id, asset.pin.hub);
			await bits.downloadBit(bit, await bits.getPackFromBit(bit), (events) => {
				const bytes = downloadedOf(events, asset.bitHash);
				if (bytes !== undefined) progress(bytes);
			});
		},
	};
}

/** A file in this computer's Bit store, read through the desktop app in chunks of up to 1 MiB. */
export function bitStoreFile(
	read: BitStoreRead,
	hash: string,
	fileName: string,
	size: number,
): PushFile {
	return {
		from: "bit_store",
		local: { hash, fileName },
		size,
		close() {},
		slice: (start, end) => ({
			async arrayBuffer() {
				const bytes = new Uint8Array(end - start);
				for (let offset = start; offset < end; ) {
					const length = Math.min(BIT_STORE_CHUNK, end - offset);
					const chunk = await read(hash, fileName, offset, length);
					if (!(chunk instanceof ArrayBuffer) || chunk.byteLength !== length)
						throw new Error(
							`Reading ${fileName} on this computer returned ${chunk instanceof ArrayBuffer ? chunk.byteLength : 0} of ${length} bytes at offset ${offset}.`,
						);
					bytes.set(new Uint8Array(chunk), offset - start);
					offset += length;
				}
				return bytes.buffer;
			},
		}),
	};
}

/** A source answered with an error status: the next source is tried at once. */
class SourceRefused extends Error {}

/** Every source of a file failed; trying the same file again changes nothing. */
class SourcesExhausted extends Error {}

/**
 * Whether a 206 starts at `offset`: by its Content-Range, or, where CORS hides
 * that header (cdn.flow-like.com exposes none), by a Content-Length of exactly
 * the rest of the file. The device checks the digest of what it gets anyway.
 */
function startsAt(response: Response, offset: number, size: number): boolean {
	const range = response.headers.get("content-range");
	if (range !== null) return range.startsWith(`bytes ${offset}-`);
	const length = response.headers.get("content-length");
	return length !== null && Number(length) === size - offset;
}

/**
 * A model file fetched from its sources in order and read front to back, so
 * the device's window paces the download. A broken connection resumes with a
 * Range request; redirects (Hugging Face's CDN) are followed.
 */
export class DownloadedFile implements PushFile {
	readonly from = "download";
	private reader?: ReadableStreamDefaultReader<Uint8Array>;
	/** File offset of the next byte the open response delivers. */
	private position = 0;
	private pending?: Uint8Array;
	private source = 0;
	private readonly failures: string[];
	/** Set once a source answered: `ready` asks no more. */
	private answered = false;
	/** Set once every source failed: later reads fail with it at once. */
	private exhausted?: Error;
	/** `failures`: what failed before these sources, e.g. the download into this computer's Bit store. */
	constructor(
		readonly size: number,
		private readonly sources: readonly string[],
		private readonly fileName: string,
		private readonly options: Pick<PushEnvironment, "fetcher" | "signal"> = {},
		failures: readonly string[] = [],
	) {
		this.failures = [...failures];
	}
	slice(start: number, end: number) {
		return { arrayBuffer: () => this.read(start, end) };
	}
	/** Asks the sources in order for the first byte, so nothing is handed to this computer that no source delivers. */
	async ready(): Promise<void> {
		let attempts = 0;
		while (!this.answered) {
			if (this.exhausted) throw this.exhausted;
			this.options.signal?.throwIfAborted();
			try {
				await this.firstByte();
			} catch (error) {
				if (this.options.signal?.aborted) throw error;
				attempts = await this.recover(error, attempts + 1);
			}
		}
	}
	close(): void {
		const reader = this.reader;
		this.reader = undefined;
		this.pending = undefined;
		void reader?.cancel().catch(() => {});
	}
	private async read(start: number, end: number): Promise<ArrayBuffer> {
		if (this.exhausted) throw this.exhausted;
		const bytes = new Uint8Array(end - start);
		let filled = 0;
		let attempts = 0;
		while (filled < bytes.length) {
			this.options.signal?.throwIfAborted();
			const target = start + filled;
			try {
				if (!this.reader || this.position > target) await this.open(target);
				const take = this.copy(await this.next(), bytes, filled, target);
				filled += take;
				if (take) attempts = 0;
			} catch (error) {
				this.close();
				if (this.options.signal?.aborted) throw error;
				attempts = await this.recover(error, attempts + 1);
			}
		}
		return bytes.buffer;
	}
	/** Copies what of `chunk` lies at `target` on into `bytes`; skipped bytes are dropped, the rest kept. */
	private copy(
		chunk: Uint8Array,
		bytes: Uint8Array,
		filled: number,
		target: number,
	) {
		const skip = Math.min(chunk.length, Math.max(0, target - this.position));
		const take = Math.min(chunk.length - skip, bytes.length - filled);
		bytes.set(chunk.subarray(skip, skip + take), filled);
		this.position += skip + take;
		if (skip + take < chunk.length) this.pending = chunk.subarray(skip + take);
		return take;
	}
	/** Retries a broken connection a few times, then moves on to the next source. */
	private async recover(error: unknown, attempts: number): Promise<number> {
		const source = this.sources[this.source] ?? "";
		if (!(error instanceof SourceRefused) && attempts < SOURCE_ATTEMPTS) {
			await pause(
				modelPushSeams.retryMs[attempts - 1] ?? 0,
				this.options.signal,
			);
			return attempts;
		}
		this.failures.push(
			`${hostOf(source)}: ${error instanceof Error ? error.message : String(error)}`,
		);
		this.source++;
		if (this.source < this.sources.length) return 0;
		this.exhausted = new SourcesExhausted(
			`None of the sources of ${this.fileName} delivered it (${this.failures.join("; ")}).`,
		);
		throw this.exhausted;
	}
	private request(range: string | undefined): Promise<Response> {
		const source = this.sources[this.source];
		if (!source)
			throw new Error(
				`${this.fileName} has no source left to download it from.`,
			);
		return (this.options.fetcher ?? fetch)(source, {
			headers: range ? { Range: range } : {},
			credentials: "omit",
			redirect: "follow",
			referrerPolicy: "no-referrer",
			cache: "no-store",
			...(this.options.signal ? { signal: this.options.signal } : {}),
		});
	}
	private async firstByte(): Promise<void> {
		const response = await this.request("bytes=0-0");
		void response.body?.cancel().catch(() => {});
		if (!response.ok) throw new SourceRefused(`HTTP ${response.status}`);
		this.answered = true;
	}
	/** A new response from `offset` on; what the previous one still held is dropped with it. */
	private async open(offset: number): Promise<void> {
		this.close();
		const response = await this.request(
			offset > 0 ? `bytes=${offset}-` : undefined,
		);
		const ranged = response.status === 206;
		const refusal = !response.ok
			? `HTTP ${response.status}`
			: ranged && !startsAt(response, offset, this.size)
				? `it answered another range than byte ${offset}`
				: undefined;
		if (refusal || !response.body) {
			void response.body?.cancel().catch(() => {});
			throw new SourceRefused(refusal ?? `HTTP ${response.status}`);
		}
		this.reader = response.body.getReader();
		this.position = ranged ? offset : 0;
		this.answered = true;
	}
	private async next(): Promise<Uint8Array> {
		const pending = this.pending;
		if (pending?.length) {
			this.pending = undefined;
			return pending;
		}
		const result = await this.reader?.read();
		if (!result || result.done)
			throw new Error(
				`the download ended at byte ${this.position} of ${this.size}`,
			);
		return result.value;
	}
}

async function hasLocalCopy(
	read: BitStoreRead,
	hash: string,
	descriptor: ModelAssetDescriptor,
): Promise<boolean> {
	try {
		const last = await read(hash, descriptor.file_name, descriptor.size - 1, 1);
		return last instanceof ArrayBuffer && last.byteLength === 1;
	} catch {
		return false;
	}
}

/** `work`, or the signal's reason once it aborts; `work` itself runs on. */
function untilAborted<T>(work: Promise<T>, signal?: AbortSignal): Promise<T> {
	if (!signal) return work;
	void work.catch(() => {});
	return new Promise<T>((resolve, reject) => {
		const abort = () =>
			reject(signal.reason ?? new Error("The model push was cancelled."));
		if (signal.aborted) return abort();
		signal.addEventListener("abort", abort, { once: true });
		work
			.then(resolve, reject)
			.finally(() => signal.removeEventListener("abort", abort));
	});
}

/**
 * Downloads the asset's pinned Bit into this computer's Bit store. A cancelled
 * push stops waiting for it (the download goes on in the desktop app).
 * Answers why the download failed, or nothing.
 */
async function downloadHere(
	asset: PushableAsset,
	environment: PushEnvironment,
): Promise<string | undefined> {
	const { downloadLocally, onLocalDownload, signal } = environment;
	if (!asset.pin || !downloadLocally) return undefined;
	signal?.throwIfAborted();
	const report = (bytes: number) => {
		if (!signal?.aborted)
			onLocalDownload?.(Math.min(bytes, asset.descriptor.size));
	};
	try {
		await untilAborted(downloadLocally(asset, report), signal);
		return undefined;
	} catch (error) {
		if (signal?.aborted) throw signal.reason ?? error;
		return error instanceof Error ? error.message : String(error);
	}
}

/**
 * Where this computer gets the asset's bytes: its own Bit store on desktop,
 * downloading the model there first when it is missing; otherwise a download
 * from the asset's public sources streamed straight through.
 */
export async function pushFile(
	asset: PushableAsset,
	environment: PushEnvironment,
): Promise<PushFile> {
	const { descriptor, bitHash } = asset;
	const read = environment.readBitStore;
	let failed: string | undefined;
	if (read && bitHash) {
		if (!(await hasLocalCopy(read, bitHash, descriptor)))
			failed = await downloadHere(asset, environment);
		if (await hasLocalCopy(read, bitHash, descriptor))
			return bitStoreFile(read, bitHash, descriptor.file_name, descriptor.size);
	}
	const local = failed ? [`the download to this computer: ${failed}`] : [];
	if (descriptor.sources?.length)
		return new DownloadedFile(
			descriptor.size,
			descriptor.sources,
			descriptor.file_name,
			environment,
			local,
		);
	throw new Error(
		failed
			? `Downloading ${descriptor.file_name} to this computer failed (${failed}), and no other source of it is known.`
			: `This computer has no copy of ${descriptor.file_name} and knows no source to download it from.`,
	);
}

export interface PushInput {
	jobId: string;
	file: TunnelUploadFile & Pick<PushFile, "ready">;
	push: ModelPush;
	signal?: AbortSignal;
	onProgress?(progress: PushProgress): void;
}

/**
 * Where a push goes on: where the device's copy ends. A copy that holds every
 * byte was never verified (its push stopped right before), and the device
 * verifies once a chunk reaches the end, so the last byte goes again; the
 * device takes it as a retry of a byte it has.
 */
function resumeAt(held: number, total: number): number {
	return held === total && total > 0 ? total - 1 : held;
}

/**
 * One stream: the device opens its push session and says where its copy
 * ends, then gets the rest. Opening takes the device's job over, so this
 * computer first makes sure it can deliver the bytes at all.
 */
async function pushOnce(input: PushInput): Promise<ModelAssetStatus> {
	const { jobId, file, push, signal } = input;
	const total = file.size;
	const refused = push.refuses?.(file);
	if (refused) throw new Error(refused);
	await file.ready?.();
	const opened = await push({
		jobId,
		offset: 0,
		...(signal ? { signal } : {}),
	});
	if (opened.state === "present") return opened;
	if (opened.state !== "awaiting_push")
		throw new Error(
			`The device did not open a push for job ${jobId}; its download is ${opened.state}.`,
		);
	input.onProgress?.({ bytes: opened.bytes, total });
	return push({
		jobId,
		offset: resumeAt(opened.bytes, total),
		file,
		...(signal ? { signal } : {}),
		onProgress: (bytes) => input.onProgress?.({ bytes, total }),
	});
}

/** Sends a model asset the device could not fetch; an interrupted push resumes where the device's copy ends. */
export async function pushModelAsset(
	input: PushInput,
): Promise<ModelAssetStatus> {
	let failure: unknown;
	for (let attempt = 0; attempt < PUSH_ATTEMPTS; attempt++) {
		input.signal?.throwIfAborted();
		try {
			const status = await pushOnce(input);
			if (status.state === "present") return status;
			failure = new Error(
				`The device holds ${status.state === "awaiting_push" ? status.bytes : 0} of ${input.file.size} bytes of job ${input.jobId} after the push.`,
			);
		} catch (error) {
			if (input.signal?.aborted || error instanceof SourcesExhausted)
				throw error;
			failure = error;
		}
		if (attempt + 1 < PUSH_ATTEMPTS)
			await pause(modelPushSeams.retryMs[attempt] ?? 0, input.signal);
	}
	throw failure;
}

const remembered = new Map<string, PushableAsset>();

/** Keeps what this window prepared for devices, so a later "Send from this computer" knows each asset's sources. */
export function rememberPushableAssets(
	assets: readonly PreparedModelAsset[],
): void {
	for (const asset of assets) {
		const key = digestKey(asset.descriptor.digest);
		remembered.delete(key);
		remembered.set(key, {
			descriptor: asset.descriptor,
			bitHash: asset.bitHash,
			pin: { id: asset.pin, ...(asset.pinHub ? { hub: asset.pinHub } : {}) },
		});
	}
	for (const key of [...remembered.keys()].slice(0, -MAX_REMEMBERED))
		remembered.delete(key);
}

const isHttps = (source: string) => {
	try {
		return new URL(source).protocol === "https:";
	} catch {
		return false;
	}
};

/**
 * What this computer knows about the asset of a device's download: what this
 * window prepared, else the sources the device itself tries and a hub file's
 * place in a Bit store, which hub Bits name by the blake3 of their bytes.
 */
export function pushableAsset(job: {
	digest: ModelAssetDigest;
	size: number;
	file_name: string;
	sources?: readonly string[];
}): PushableAsset {
	const sources = job.sources?.filter(isHttps) ?? [];
	const known = remembered.get(digestKey(job.digest));
	if (known)
		return known.descriptor.sources?.length || !sources.length
			? known
			: { ...known, descriptor: { ...known.descriptor, sources } };
	return {
		descriptor: {
			digest: job.digest,
			size: job.size,
			file_name: job.file_name,
			...(sources.length ? { sources } : {}),
		},
		...(job.digest.algorithm === "blake3" ? { bitHash: job.digest.hex } : {}),
	};
}
