import type { FlowPath } from "./host";

export interface StorageErrorData {
	code: string;
	message: string;
	retryable: boolean;
}
export class StorageError extends Error implements StorageErrorData {
	constructor(
		public readonly code: string,
		message: string,
		public readonly retryable = false,
	) {
		super(message);
		this.name = "StorageError";
	}
}

// FlowPath's extension index permits only strings; metadata adds numeric fields.
export interface FileStat {
	path: string;
	store_ref: string;
	cache_store_ref?: string | null;
	size: number;
	e_tag?: string | null;
	version?: string | null;
	last_modified: string;
}
export interface WriteOptions {
	if_match?: string;
	version?: string;
	create_only?: boolean;
}
export interface ListRequest {
	path: FlowPath;
	limit?: number;
	cursor?: string;
	exclude_directories?: string[];
	recursive?: boolean;
}
export interface ListPage {
	entries: FileStat[];
	cursor: string | null;
}

/** Optional host extension. Failures throw StorageError with the host's code and retryable flag. */
export interface StorageOps {
	stat(path: FlowPath): FileStat;
	readRange(
		path: FlowPath,
		offset: bigint | number,
		length: bigint | number,
		ifMatch?: string,
	): Uint8Array;
	listPage(request: ListRequest): ListPage;
	closeList(cursor: string): void;
	write(path: FlowPath, data: Uint8Array, options?: WriteOptions): FileStat;
	writeStart(path: FlowPath, totalSize?: bigint | number): string;
	writeChunk(id: string, data: Uint8Array): void;
	writeFinish(id: string): FileStat;
	writeAbort(id: string): void;
	delete(path: FlowPath): void;
	move(source: FlowPath, destination: FlowPath, overwrite?: boolean): void;
	copy(source: FlowPath, destination: FlowPath, overwrite?: boolean): void;
	fileUrl(path: FlowPath): string | null;
}

/** WIT bindings for flow-like:node/storage-ops@0.1.0, with result errors thrown by the binding. */
export interface StorageOpsImports {
	statFile(path: string): string;
	readFileRange(
		path: string,
		offset: bigint,
		length: bigint,
		ifMatch?: string,
	): Uint8Array;
	listFilesPage(request: string): string;
	closeList(cursor: string): void;
	writeFile(path: string, data: Uint8Array, options: string): string;
	writeFileStart(path: string, totalSize?: bigint): string;
	writeFileChunk(id: string, data: Uint8Array): void;
	writeFileFinish(id: string): string;
	writeFileAbort(id: string): void;
	deleteFile(path: string): void;
	moveFile(source: string, destination: string, overwrite: boolean): void;
	copyFile(source: string, destination: string, overwrite: boolean): void;
	fileUrl(path: string): string | undefined;
}

function invoke<T>(call: () => T): T {
	try {
		return call();
	} catch (error) {
		if (error instanceof StorageError) throw error;
		const value =
			error && typeof error === "object" && "payload" in error
				? error.payload
				: error;
		if (
			value &&
			typeof value === "object" &&
			"code" in value &&
			"message" in value &&
			"retryable" in value &&
			typeof value.code === "string" &&
			typeof value.message === "string" &&
			typeof value.retryable === "boolean"
		) {
			throw new StorageError(value.code, value.message, value.retryable);
		}
		throw error;
	}
}
export function storageU64(value: bigint | number): bigint {
	if (
		typeof value === "number" &&
		(!Number.isSafeInteger(value) || value < 0)
	) {
		throw new StorageError(
			"invalid_argument",
			"Use a nonnegative safe integer or bigint",
		);
	}
	const integer = BigInt(value);
	if (integer < 0n || integer > 0xffffffffffffffffn)
		throw new StorageError("invalid_argument", "Value must fit in u64");
	return integer;
}
function decode<T>(value: string): T {
	try {
		return JSON.parse(value) as T;
	} catch {
		throw new StorageError(
			"invalid_response",
			"Host returned invalid storage JSON",
		);
	}
}

export function createStorageOpsBridge(raw: StorageOpsImports): StorageOps {
	return {
		stat: (path) =>
			invoke(() => decode<FileStat>(raw.statFile(JSON.stringify(path)))),
		readRange: (path, offset, length, ifMatch) =>
			invoke(() =>
				raw.readFileRange(
					JSON.stringify(path),
					storageU64(offset),
					storageU64(length),
					ifMatch,
				),
			),
		listPage: (request) =>
			invoke(() =>
				decode<ListPage>(raw.listFilesPage(JSON.stringify(request))),
			),
		closeList: (cursor) => invoke(() => raw.closeList(cursor)),
		write: (path, data, options = {}) =>
			invoke(() =>
				decode<FileStat>(
					raw.writeFile(JSON.stringify(path), data, JSON.stringify(options)),
				),
			),
		writeStart: (path, totalSize) =>
			invoke(() =>
				raw.writeFileStart(
					JSON.stringify(path),
					totalSize === undefined ? undefined : storageU64(totalSize),
				),
			),
		writeChunk: (id, data) => invoke(() => raw.writeFileChunk(id, data)),
		writeFinish: (id) =>
			invoke(() => decode<FileStat>(raw.writeFileFinish(id))),
		writeAbort: (id) => invoke(() => raw.writeFileAbort(id)),
		delete: (path) => invoke(() => raw.deleteFile(JSON.stringify(path))),
		move: (source, destination, overwrite = false) =>
			invoke(() =>
				raw.moveFile(
					JSON.stringify(source),
					JSON.stringify(destination),
					overwrite,
				),
			),
		copy: (source, destination, overwrite = false) =>
			invoke(() =>
				raw.copyFile(
					JSON.stringify(source),
					JSON.stringify(destination),
					overwrite,
				),
			),
		fileUrl: (path) => invoke(() => raw.fileUrl(JSON.stringify(path)) ?? null),
	};
}

/** Stream an iterable without buffering it in the SDK. Always finish or abort within one invocation. */
export function writeStorageChunks(
	storage: StorageOps,
	path: FlowPath,
	chunks: Iterable<Uint8Array>,
	totalSize?: bigint | number,
): FileStat {
	const id = storage.writeStart(path, totalSize);
	try {
		for (const chunk of chunks) storage.writeChunk(id, chunk);
		return storage.writeFinish(id);
	} catch (error) {
		try {
			storage.writeAbort(id);
		} catch {
			/* Preserve the original write failure. */
		}
		throw error;
	}
}
