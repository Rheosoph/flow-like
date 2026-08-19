/**
 * Bulk temporary uploads (a2ui file inputs, chat attachments).
 *
 * The single-file path - `IHelperState.fileToTemporaryFile` - costs one presign
 * round trip plus one transfer per file and hashes the whole file for its dedupe
 * key. That is fine for a handful of attachments and hopeless for a folder with
 * thousands of files, so this module presigns in batches and hands the transfers
 * to {@link runBulkUpload}, which owns concurrency, retries and progress for
 * every bulk upload in the app.
 */

import type { ITemporaryUploadedFile } from "../state/backend-state";
import {
	type BulkUploadProgressCallback,
	type IBulkUploadTask,
	isAbortError,
	runBulkUpload,
} from "./bulk-upload";
import { uploadToSignedUrl } from "./signed-upload";
import {
	readTemporaryUploadCache,
	temporaryUploadMetadataKey,
	writeTemporaryUploadCache,
} from "./temporary-upload-cache";

/**
 * Files presigned per request. Matches `MAX_BATCH_FILES` in
 * `packages/api/src/routes/tmp.rs`, which rejects anything larger.
 */
export const TEMPORARY_PRESIGN_BATCH_SIZE = 100;

/** One presigned upload slot, as returned by `POST /tmp/batch`. */
export interface ITemporaryPresignedUpload {
	uploadUrl: string;
	downloadUrl: string;
	key?: string;
	contentType?: string;
	flowPath?: ITemporaryUploadedFile["flowPath"];
	uploadExpiresAt?: string;
	downloadExpiresAt?: string;
	sizeLimitBytes?: number;
}

export interface ITemporaryUploadResult {
	file: File;
	uploaded?: ITemporaryUploadedFile;
	error?: string;
}

export interface ITemporaryUploadBatchOptions {
	/** Cache scope, identical to the one the single-file upload path uses. */
	scope: string;
	/** Presigns one batch; results must align with the given files. */
	presign: (
		files: File[],
		signal: AbortSignal,
	) => Promise<ITemporaryPresignedUpload[]>;
	onProgress?: BulkUploadProgressCallback;
	signal?: AbortSignal;
	concurrency?: number;
	batchSize?: number;
}

export function buildContentDisposition(
	filename: string,
	disposition: "inline" | "attachment" = "inline",
): string {
	let fallback = filename
		.normalize("NFKD")
		.replace(/[^\x20-\x7E]+/g, "")
		.replace(/["\\]/g, "_")
		.trim();

	if (!fallback) fallback = "file";

	return `${disposition}; filename="${fallback}"; filename*=UTF-8''${encodeURIComponent(filename)}`;
}

/**
 * Task paths are the orchestrator's identity for a file, so they must be unique:
 * a selection can legitimately contain two files with the same relative path.
 * The server mints the real storage key, this never reaches it.
 */
function taskPath(file: File, index: number): string {
	const relative =
		(file as File & { webkitRelativePath?: string }).webkitRelativePath ||
		file.name;
	return `${index}/${relative}`;
}

function toUploadedFile(
	presigned: ITemporaryPresignedUpload,
): ITemporaryUploadedFile {
	return {
		url: presigned.downloadUrl,
		key: presigned.key,
		contentType: presigned.contentType,
		flowPath: presigned.flowPath,
		uploadExpiresAt: presigned.uploadExpiresAt,
		downloadExpiresAt: presigned.downloadExpiresAt,
		sizeLimitBytes: presigned.sizeLimitBytes,
	};
}

function errorMessage(error: unknown): string {
	return error instanceof Error ? error.message : String(error);
}

interface PreparedBatch {
	results: ITemporaryUploadResult[];
	tasks: IBulkUploadTask[];
	indexByPath: Map<string, number>;
	cacheKeys: string[];
}

/** Splits a selection into "already uploaded" and "still to transfer". */
function prepareBatch(files: File[], scope?: string): PreparedBatch {
	const results: ITemporaryUploadResult[] = files.map((file) => ({ file }));
	const cacheKeys = scope
		? files.map((file) => temporaryUploadMetadataKey(file, scope))
		: [];
	const tasks: IBulkUploadTask[] = [];
	const indexByPath = new Map<string, number>();

	files.forEach((file, index) => {
		const cached = scope
			? readTemporaryUploadCache(cacheKeys[index])
			: undefined;
		if (cached) {
			results[index].uploaded = cached;
			return;
		}

		const path = taskPath(file, index);
		indexByPath.set(path, index);
		tasks.push({ path, file });
	});

	return { results, tasks, indexByPath, cacheKeys };
}

function applyFailures(
	prepared: PreparedBatch,
	failures: readonly { path: string; error: string }[],
): void {
	for (const failure of failures) {
		const index = prepared.indexByPath.get(failure.path);
		if (index !== undefined) prepared.results[index].error = failure.error;
	}
}

/** Marks everything that never made it as failed, keeping partial successes. */
function failRemaining(prepared: PreparedBatch, message: string): void {
	for (const index of prepared.indexByPath.values()) {
		const result = prepared.results[index];
		if (!result.uploaded && !result.error) result.error = message;
	}
}

/**
 * Presigns `files` in batches and transfers them with bounded parallelism.
 * Per-file failures are reported on the result instead of rejecting, so one bad
 * file cannot sink a folder upload; an abort still rejects.
 */
export async function uploadTemporaryFilesInBatches(
	files: File[],
	options: ITemporaryUploadBatchOptions,
): Promise<ITemporaryUploadResult[]> {
	const prepared = prepareBatch(files, options.scope);
	if (prepared.tasks.length === 0) return prepared.results;

	try {
		const outcome = await runBulkUpload<ITemporaryPresignedUpload>(
			prepared.tasks,
			{
				prepare: async (paths, signal) => {
					const batch = paths.map(
						(path) => files[prepared.indexByPath.get(path) as number],
					);
					const presigned = await options.presign(batch, signal);
					const targets = new Map<string, ITemporaryPresignedUpload>();
					paths.forEach((path, position) => {
						const slot = presigned[position];
						if (slot?.uploadUrl) targets.set(path, slot);
					});
					return targets;
				},
				send: async (target, task, onBytes, signal) => {
					await uploadToSignedUrl(target.uploadUrl, task.file, {
						onBytes,
						signal,
						contentType: task.file.type || target.contentType,
						contentDisposition: buildContentDisposition(task.file.name),
					});

					const index = prepared.indexByPath.get(task.path) as number;
					const uploaded = toUploadedFile(target);
					prepared.results[index].uploaded = uploaded;
					prepared.results[index].error = undefined;
					writeTemporaryUploadCache(prepared.cacheKeys[index], uploaded);
				},
			},
			{
				onProgress: options.onProgress,
				signal: options.signal,
				concurrency: options.concurrency,
				batchSize: options.batchSize ?? TEMPORARY_PRESIGN_BATCH_SIZE,
			},
		);

		applyFailures(prepared, outcome.failed);
	} catch (error) {
		if (isAbortError(error)) throw error;
		// A `prepare` failure condemns the run - auth, network, a rejected batch.
		failRemaining(prepared, errorMessage(error));
	}

	return prepared.results;
}

/**
 * Same batching shape for upload paths that have no presign step: desktop
 * offline runs write into the local cache directory instead.
 */
export async function uploadTemporaryFilesLocally(
	files: File[],
	upload: (file: File) => Promise<ITemporaryUploadedFile>,
	options?: {
		onProgress?: BulkUploadProgressCallback;
		signal?: AbortSignal;
		concurrency?: number;
	},
): Promise<ITemporaryUploadResult[]> {
	const prepared = prepareBatch(files);
	if (prepared.tasks.length === 0) return prepared.results;

	try {
		const outcome = await runBulkUpload<File>(
			prepared.tasks,
			{
				prepare: async (paths) =>
					new Map(
						paths.map((path) => [
							path,
							files[prepared.indexByPath.get(path) as number],
						]),
					),
				send: async (file, task, onBytes) => {
					const uploaded = await upload(file);
					const index = prepared.indexByPath.get(task.path) as number;
					prepared.results[index].uploaded = uploaded;
					prepared.results[index].error = undefined;
					onBytes(file.size);
				},
			},
			{
				onProgress: options?.onProgress,
				signal: options?.signal,
				concurrency: options?.concurrency ?? 4,
				refreshTargetOnRetry: false,
			},
		);

		applyFailures(prepared, outcome.failed);
	} catch (error) {
		if (isAbortError(error)) throw error;
		failRemaining(prepared, errorMessage(error));
	}

	return prepared.results;
}
