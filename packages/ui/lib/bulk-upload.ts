/**
 * Bulk upload orchestrator.
 *
 * Uploading a folder means thousands of independent transfers, and the naive
 * shape — resolve every destination in one request, then `await` each transfer
 * in a loop — fails three separate ways at that scale: the resolve request
 * exceeds the API body limit, the server caps the batch and silently returns
 * fewer targets than were asked for, and the serial transfer loop takes hours.
 *
 * This module owns the parts that are identical for every backend: chunking the
 * destination resolution, keeping a bounded number of transfers in flight,
 * retrying the failures that are worth retrying, reporting byte-weighted
 * progress, and collecting per-file errors instead of losing the whole run to
 * the first one. Backends supply only the two pieces that actually differ —
 * how a destination is resolved (`prepare`) and how bytes get there (`send`).
 *
 * Concurrency defaults to {@link DEFAULT_UPLOAD_CONCURRENCY}. Browsers cap
 * simultaneous connections to one origin at six for HTTP/1.1, and the S3 REST
 * API is HTTP/1.1 only, so a much larger number does not buy more throughput —
 * it just queues in the network stack where this module can no longer see it,
 * which makes progress and cancellation worse rather than better.
 */

/** Transfers in flight at once. */
export const DEFAULT_UPLOAD_CONCURRENCY = 8;
/**
 * Destinations resolved per `prepare` call. Matches `MAX_PREFIXES` in
 * `packages/api/src/routes/app/data/upload_files.rs`, which rejects anything
 * larger.
 */
export const DEFAULT_UPLOAD_BATCH_SIZE = 100;
/** Attempts per file, first try included. */
export const DEFAULT_UPLOAD_MAX_ATTEMPTS = 4;

const RETRY_BASE_DELAY_MS = 250;
const RETRY_MAX_DELAY_MS = 15_000;
const PROGRESS_THROTTLE_MS = 100;
/** Samples kept for the throughput estimate, ~3s of history at the throttle. */
const RATE_WINDOW_SAMPLES = 30;

export interface IBulkUploadTask {
	/** Destination path relative to the storage root, e.g. `photos/2024/a.jpg`. */
	readonly path: string;
	readonly file: File;
}

export interface IBulkUploadProgress {
	readonly phase: "preparing" | "uploading" | "done";
	readonly totalFiles: number;
	readonly completedFiles: number;
	readonly failedFiles: number;
	readonly totalBytes: number;
	readonly uploadedBytes: number;
	/** Byte-weighted, 0–100. */
	readonly percent: number;
	readonly bytesPerSecond: number;
	readonly etaSeconds?: number;
	readonly currentFile: string;
}

/**
 * The leading `percent` argument keeps every existing `(progress: number)`
 * callback working untouched; `detail` is additive.
 */
export type BulkUploadProgressCallback = (
	percent: number,
	detail?: IBulkUploadProgress,
) => void;

export interface IBulkUploadFailure {
	readonly path: string;
	readonly error: string;
	readonly attempts: number;
}

export interface IBulkUploadResult {
	readonly uploaded: number;
	readonly failed: readonly IBulkUploadFailure[];
	readonly totalBytes: number;
	readonly uploadedBytes: number;
	readonly durationMs: number;
	readonly cancelled: boolean;
}

export interface IBulkUploadHandlers<TTarget> {
	/**
	 * Resolve upload destinations for up to `batchSize` paths.
	 *
	 * A path missing from the returned map is recorded as a failure for that
	 * file alone — a backend that can only satisfy part of a batch must not
	 * take down the rest of the run. Throwing, by contrast, fails the whole
	 * batch, which is the right response to an auth or network error.
	 */
	prepare(
		paths: readonly string[],
		signal: AbortSignal,
	): Promise<ReadonlyMap<string, TTarget>>;

	/**
	 * Transfer one file. Report cumulative bytes sent for this file through
	 * `onBytes`; the orchestrator handles the aggregate.
	 */
	send(
		target: TTarget,
		task: IBulkUploadTask,
		onBytes: (loaded: number) => void,
		signal: AbortSignal,
	): Promise<void>;

	/** Defaults to {@link isRetryableUploadError}. */
	isRetryable?(error: unknown): boolean;
}

export interface IBulkUploadOptions {
	concurrency?: number;
	batchSize?: number;
	maxAttempts?: number;
	signal?: AbortSignal;
	onProgress?: BulkUploadProgressCallback;
	/**
	 * Re-run `prepare` for a single file before retrying it. Defaults to true:
	 * the usual reason a transfer fails late in a long run is that its signed
	 * destination expired, and reusing the dead target just burns the retry.
	 */
	refreshTargetOnRetry?: boolean;
}

/** Carries the HTTP status so retry classification can see it. */
export class BulkUploadHttpError extends Error {
	readonly status: number;

	constructor(status: number, message: string) {
		super(message);
		this.name = "BulkUploadHttpError";
		this.status = status;
	}
}

/** Raised when a transfer stops because the caller aborted. */
export class BulkUploadAbortError extends Error {
	constructor(message = "Upload cancelled") {
		super(message);
		this.name = "BulkUploadAbortError";
	}
}

export function isAbortError(error: unknown): boolean {
	if (error instanceof BulkUploadAbortError) return true;
	return error instanceof DOMException && error.name === "AbortError";
}

/**
 * Retry transport faults and the server's own "try again" signals, never a
 * refusal. A 403 is included because an expired signature presents as one, and
 * `refreshTargetOnRetry` mints a fresh destination before the next attempt; a
 * genuine permission failure simply exhausts its attempts.
 */
export function isRetryableUploadError(error: unknown): boolean {
	if (isAbortError(error)) return false;
	if (error instanceof BulkUploadHttpError) {
		const { status } = error;
		if (status === 0) return true;
		if (status === 403 || status === 408 || status === 429) return true;
		return status >= 500;
	}
	return true;
}

/** Joins a prefix and a file's path, preserving folder structure. */
export function buildUploadPath(prefix: string, file: File): string {
	const relative =
		file.webkitRelativePath && file.webkitRelativePath !== ""
			? file.webkitRelativePath
			: file.name;
	const trimmed = prefix.replace(/^\/+|\/+$/g, "");
	return trimmed ? `${trimmed}/${relative}` : relative;
}

export function toUploadTasks(
	prefix: string,
	files: readonly File[],
): IBulkUploadTask[] {
	return files.map((file) => ({ path: buildUploadPath(prefix, file), file }));
}

function clamp(value: number, min: number, max: number): number {
	return Math.min(max, Math.max(min, Math.round(value)));
}

function delay(ms: number, signal?: AbortSignal): Promise<void> {
	return new Promise((resolve, reject) => {
		if (signal?.aborted) {
			reject(new BulkUploadAbortError());
			return;
		}
		const timer = setTimeout(() => {
			signal?.removeEventListener("abort", onAbort);
			resolve();
		}, ms);
		const onAbort = () => {
			clearTimeout(timer);
			reject(new BulkUploadAbortError());
		};
		signal?.addEventListener("abort", onAbort, { once: true });
	});
}

/** Exponential backoff with full jitter, so retries do not resynchronise. */
function backoffDelay(attempt: number): number {
	const ceiling = Math.min(
		RETRY_MAX_DELAY_MS,
		RETRY_BASE_DELAY_MS * 2 ** (attempt - 1),
	);
	return Math.random() * ceiling;
}

function errorMessage(error: unknown): string {
	if (error instanceof Error) return error.message;
	return String(error);
}

interface RateSample {
	readonly at: number;
	readonly bytes: number;
}

/**
 * Upload `tasks`, resolving destinations in batches and keeping `concurrency`
 * transfers in flight.
 *
 * Never rejects for a per-file failure — inspect `failed` on the result. It
 * rejects only when `prepare` throws (auth, network, a rejected batch), since
 * that condemns every remaining file.
 */
export async function runBulkUpload<TTarget>(
	tasks: readonly IBulkUploadTask[],
	handlers: IBulkUploadHandlers<TTarget>,
	options: IBulkUploadOptions = {},
): Promise<IBulkUploadResult> {
	const startedAt = Date.now();
	const totalFiles = tasks.length;
	const totalBytes = tasks.reduce((sum, task) => sum + task.file.size, 0);

	if (totalFiles === 0) {
		options.onProgress?.(100, {
			phase: "done",
			totalFiles: 0,
			completedFiles: 0,
			failedFiles: 0,
			totalBytes: 0,
			uploadedBytes: 0,
			percent: 100,
			bytesPerSecond: 0,
			currentFile: "",
		});
		return {
			uploaded: 0,
			failed: [],
			totalBytes: 0,
			uploadedBytes: 0,
			durationMs: 0,
			cancelled: false,
		};
	}

	const concurrency = clamp(
		options.concurrency ?? DEFAULT_UPLOAD_CONCURRENCY,
		1,
		32,
	);
	const batchSize = clamp(
		options.batchSize ?? DEFAULT_UPLOAD_BATCH_SIZE,
		1,
		1000,
	);
	const maxAttempts = clamp(
		options.maxAttempts ?? DEFAULT_UPLOAD_MAX_ATTEMPTS,
		1,
		10,
	);
	const refreshTargetOnRetry = options.refreshTargetOnRetry !== false;
	const isRetryable = handlers.isRetryable ?? isRetryableUploadError;
	const externalSignal = options.signal;

	const controller = new AbortController();
	const signal = controller.signal;
	const abortFromCaller = () => controller.abort();
	if (externalSignal) {
		if (externalSignal.aborted) controller.abort();
		else externalSignal.addEventListener("abort", abortFromCaller);
	}

	let settledBytes = 0;
	let completedFiles = 0;
	const failed: IBulkUploadFailure[] = [];
	const inFlightBytes = new Map<string, number>();
	let currentFile = tasks[0]?.file.name ?? "";
	let phase: IBulkUploadProgress["phase"] = "preparing";

	let lastEmitAt = 0;
	const rateSamples: RateSample[] = [];

	const emitProgress = (force = false) => {
		if (!options.onProgress) return;
		const now = Date.now();
		if (!force && now - lastEmitAt < PROGRESS_THROTTLE_MS) return;
		lastEmitAt = now;

		let uploadedBytes = settledBytes;
		for (const bytes of inFlightBytes.values()) uploadedBytes += bytes;
		uploadedBytes = Math.min(uploadedBytes, totalBytes);

		rateSamples.push({ at: now, bytes: uploadedBytes });
		if (rateSamples.length > RATE_WINDOW_SAMPLES) rateSamples.shift();

		let bytesPerSecond = 0;
		const oldest = rateSamples[0];
		if (oldest && now > oldest.at) {
			bytesPerSecond =
				((uploadedBytes - oldest.bytes) * 1000) / (now - oldest.at);
			if (!Number.isFinite(bytesPerSecond) || bytesPerSecond < 0) {
				bytesPerSecond = 0;
			}
		}

		// Zero-byte files are real and a folder of them must still reach 100%,
		// so fall back to file count when there are no bytes to weigh.
		const percent =
			totalBytes > 0
				? (uploadedBytes / totalBytes) * 100
				: (completedFiles / totalFiles) * 100;
		const remainingBytes = Math.max(0, totalBytes - uploadedBytes);
		const etaSeconds =
			bytesPerSecond > 0 ? remainingBytes / bytesPerSecond : undefined;

		// A reporting callback is the caller's UI, not part of the transfer.
		// Letting it throw here would surface as a failed upload.
		try {
			options.onProgress(Math.min(100, Math.max(0, percent)), {
				phase,
				totalFiles,
				completedFiles,
				failedFiles: failed.length,
				totalBytes,
				uploadedBytes,
				percent: Math.min(100, Math.max(0, percent)),
				bytesPerSecond,
				etaSeconds,
				currentFile,
			});
		} catch (error) {
			console.warn("Bulk upload progress callback threw", error);
		}
	};

	interface ReadyTask {
		readonly task: IBulkUploadTask;
		readonly target: TTarget;
	}

	const batches: IBulkUploadTask[][] = [];
	for (let index = 0; index < tasks.length; index += batchSize) {
		batches.push(tasks.slice(index, index + batchSize));
	}

	const queue: ReadyTask[] = [];
	let nextBatch = 0;
	let preparing: Promise<void> | null = null;
	let prepareFailure: unknown = null;

	/**
	 * Resolve the next batch once the queue can no longer keep every worker
	 * busy. Without this the pool drains to empty at each batch boundary and
	 * the whole run stalls for a round trip — 130 times over, for a folder that
	 * needs 130 batches.
	 */
	const PREFETCH_LOW_WATER = concurrency;

	const prepareNextBatch = async (): Promise<void> => {
		const batch = batches[nextBatch];
		nextBatch += 1;

		const targets = await handlers.prepare(
			batch.map((task) => task.path),
			signal,
		);

		for (const task of batch) {
			const target = targets.get(task.path);
			if (target === undefined) {
				// A destination the backend declined to resolve. Recorded here
				// rather than thrown so the remaining files still upload.
				failed.push({
					path: task.path,
					error: "No upload destination returned for this file",
					attempts: 0,
				});
				settledBytes += task.file.size;
				completedFiles += 1;
				continue;
			}
			queue.push({ task, target });
		}
		phase = "uploading";
		emitProgress();
	};

	/** Starts a batch resolve if one is warranted, else returns the in-flight one. */
	const startPrepare = (): Promise<void> | null => {
		if (preparing) return preparing;
		if (nextBatch >= batches.length || signal.aborted) return null;
		// Failures are captured rather than propagated so the only rejection a
		// worker has to handle is an abort.
		preparing = prepareNextBatch()
			.catch((error) => {
				if (prepareFailure === null) prepareFailure = error;
				controller.abort();
			})
			.finally(() => {
				preparing = null;
			});
		return preparing;
	};

	/** Non-blocking: tops the queue up while workers keep uploading. */
	const maybePrefetch = (): void => {
		if (queue.length <= PREFETCH_LOW_WATER) void startPrepare();
	};

	/**
	 * Blocks until the queue has work or the run is genuinely finished.
	 *
	 * The loop deliberately does NOT test `nextBatch < batches.length`.
	 * `prepareNextBatch` claims its index synchronously, so once the *last*
	 * batch has been prefetched that test reads false while the resolve is
	 * still on the wire — every worker would then retire, the run would return
	 * successfully, and the final batch would land in a queue nobody is
	 * consuming. `startPrepare` returns the in-flight promise before it looks
	 * at `nextBatch`, so awaiting whatever it hands back covers that window and
	 * still yields null (ending the loop) once nothing is pending.
	 */
	const ensureQueued = async (): Promise<void> => {
		while (queue.length === 0 && !signal.aborted) {
			const pending = startPrepare();
			if (!pending) return;
			await pending;
		}
	};

	const sendWithRetry = async (ready: ReadyTask): Promise<void> => {
		const { task } = ready;
		let target = ready.target;
		let lastError: unknown;

		for (let attempt = 1; attempt <= maxAttempts; attempt += 1) {
			if (signal.aborted) throw new BulkUploadAbortError();
			inFlightBytes.set(task.path, 0);
			currentFile = task.file.name;

			// Only the transfer belongs in the `try`. Book-keeping the success
			// inside it would let a throw from the progress callback be read as
			// a transfer failure, re-sending a file that already landed and
			// counting it twice.
			let sendError: unknown;
			let sendFailed = false;
			try {
				await handlers.send(
					target,
					task,
					(loaded) => {
						inFlightBytes.set(task.path, Math.min(loaded, task.file.size));
						emitProgress();
					},
					signal,
				);
			} catch (error) {
				sendFailed = true;
				sendError = error;
			}

			if (!sendFailed) {
				inFlightBytes.delete(task.path);
				settledBytes += task.file.size;
				completedFiles += 1;
				emitProgress();
				return;
			}

			inFlightBytes.delete(task.path);
			lastError = sendError;
			if (isAbortError(sendError) || signal.aborted) {
				throw new BulkUploadAbortError();
			}
			if (attempt >= maxAttempts || !isRetryable(sendError)) break;

			await delay(backoffDelay(attempt), signal);

			if (refreshTargetOnRetry) {
				try {
					const refreshed = await handlers.prepare([task.path], signal);
					const next = refreshed.get(task.path);
					if (next !== undefined) target = next;
				} catch (refreshError) {
					// Keep the stale target and let the attempt decide; a
					// resolve outage is reported through the transfer error.
					if (isAbortError(refreshError) || signal.aborted) {
						throw new BulkUploadAbortError();
					}
				}
			}
		}

		failed.push({
			path: task.path,
			error: errorMessage(lastError),
			attempts: maxAttempts,
		});
		settledBytes += task.file.size;
		completedFiles += 1;
		emitProgress();
	};

	let cancelled = false;

	const worker = async (): Promise<void> => {
		for (;;) {
			if (signal.aborted) return;
			if (queue.length === 0) {
				await ensureQueued();
				if (queue.length === 0) return;
			}
			const ready = queue.shift();
			if (!ready) return;
			maybePrefetch();
			try {
				await sendWithRetry(ready);
			} catch (error) {
				if (isAbortError(error)) {
					cancelled = true;
					return;
				}
				throw error;
			}
		}
	};

	try {
		await Promise.all(
			Array.from({ length: Math.min(concurrency, totalFiles) }, () => worker()),
		);
	} finally {
		externalSignal?.removeEventListener("abort", abortFromCaller);
	}

	if (prepareFailure !== null && !isAbortError(prepareFailure)) {
		throw prepareFailure;
	}
	if (signal.aborted) cancelled = true;

	phase = "done";
	emitProgress(true);

	let uploadedBytes = settledBytes;
	for (const bytes of inFlightBytes.values()) uploadedBytes += bytes;

	return {
		// `completedFiles` counts every file that reached a verdict, so this
		// excludes anything a cancellation left unattempted — subtracting from
		// `totalFiles` would have reported those as uploaded.
		uploaded: completedFiles - failed.length,
		// A copy: the caller inspects this synchronously, and sharing the live
		// array would let any straggler append to a result already acted on.
		failed: [...failed],
		totalBytes,
		uploadedBytes: Math.min(uploadedBytes, totalBytes),
		durationMs: Date.now() - startedAt,
		cancelled,
	};
}

/** Raised by {@link summarizeBulkUpload} when a run had per-file failures. */
export class BulkUploadPartialFailureError extends Error {
	readonly result: IBulkUploadResult;

	constructor(message: string, result: IBulkUploadResult) {
		super(message);
		this.name = "BulkUploadPartialFailureError";
		this.result = result;
	}
}

/**
 * Turn a result into the throw-or-return contract the existing
 * `uploadStorageItems` callers expect, without hiding a partial failure behind
 * a success.
 */
export function assertBulkUploadSucceeded(result: IBulkUploadResult): void {
	if (result.failed.length === 0) return;
	const [first] = result.failed;
	const extra = result.failed.length - 1;
	const detail =
		extra > 0
			? `${first.path}: ${first.error} (and ${extra} more)`
			: `${first.path}: ${first.error}`;
	throw new BulkUploadPartialFailureError(
		`${result.failed.length} of ${result.failed.length + result.uploaded} files failed to upload — ${detail}`,
		result,
	);
}
