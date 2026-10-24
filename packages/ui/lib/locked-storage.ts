import { sha256 } from "@noble/hashes/sha2";
import type {
	ICreateLockedUploadRequest,
	IGetLockedItemRequest,
	IListLockedItemsRequest,
	IListLockedItemsResponse,
	ILockedDownload,
	ILockedItemDetail,
	ILockedItemVersion,
	ILockedStorageState,
	ILockedStorageStatus,
	ILockedUploadOptions,
	ILockedUploadTicket,
} from "../state/backend-state/locked-storage-state";
import { BulkUploadAbortError } from "./bulk-upload";
import { uploadToSignedUrl } from "./signed-upload";

export type LockedStorageAction =
	| "status"
	| "list"
	| "get"
	| "upload"
	| "commit"
	| "abort"
	| "download"
	| "retention"
	| "legal-hold"
	| "auto-delete"
	| "purge";

export interface ILockedStorageRequest {
	readonly method: "GET" | "POST";
	readonly body?: unknown;
	readonly signal?: AbortSignal;
}

/** Sends one call to `apps/{appId}/locked/{action}` and returns the parsed JSON. */
export type LockedStorageTransport = <T>(
	appId: string,
	action: LockedStorageAction,
	request: ILockedStorageRequest,
) => Promise<T>;

export function lockedStoragePath(
	appId: string,
	action: LockedStorageAction,
): string {
	return `apps/${appId}/locked/${action}`;
}

/** WebCrypto needs the whole file in memory; above this the hash streams. */
const SUBTLE_HASH_MAX_BYTES = 64 * 1024 * 1024;
const HASH_CHUNK_BYTES = 4 * 1024 * 1024;

function toHex(bytes: Uint8Array): string {
	return Array.from(bytes, (byte) => byte.toString(16).padStart(2, "0")).join(
		"",
	);
}

function throwIfAborted(signal?: AbortSignal) {
	if (signal?.aborted) throw new BulkUploadAbortError();
}

/** Lowercase hex SHA-256 of the exact bytes, streamed for large files. */
export async function sha256Hex(
	blob: Blob,
	options: {
		signal?: AbortSignal;
		onBytes?: (loaded: number) => void;
	} = {},
): Promise<string> {
	const { signal, onBytes } = options;
	throwIfAborted(signal);
	if (blob.size <= SUBTLE_HASH_MAX_BYTES && globalThis.crypto?.subtle) {
		const digest = await globalThis.crypto.subtle.digest(
			"SHA-256",
			await blob.arrayBuffer(),
		);
		throwIfAborted(signal);
		onBytes?.(blob.size);
		return toHex(new Uint8Array(digest));
	}
	const hash = sha256.create();
	try {
		for (let offset = 0; offset < blob.size; ) {
			throwIfAborted(signal);
			const end = Math.min(blob.size, offset + HASH_CHUNK_BYTES);
			hash.update(new Uint8Array(await blob.slice(offset, end).arrayBuffer()));
			offset = end;
			onBytes?.(offset);
		}
		throwIfAborted(signal);
		return toHex(hash.digest());
	} finally {
		hash.destroy();
	}
}

/**
 * The locked-storage API over any authenticated JSON transport. Hosts differ
 * only in how a request reaches the hub, so web and desktop share the flow.
 */
export class LockedStorageClient implements ILockedStorageState {
	constructor(private readonly transport: LockedStorageTransport) {}

	private post<T>(
		appId: string,
		action: LockedStorageAction,
		body: unknown,
		signal?: AbortSignal,
	): Promise<T> {
		return this.transport<T>(appId, action, { method: "POST", body, signal });
	}

	getLockedStorageStatus(appId: string): Promise<ILockedStorageStatus> {
		return this.transport<ILockedStorageStatus>(appId, "status", {
			method: "GET",
		});
	}

	listLockedItems(
		appId: string,
		request: IListLockedItemsRequest,
	): Promise<IListLockedItemsResponse> {
		return this.post(appId, "list", request);
	}

	getLockedItem(
		appId: string,
		request: IGetLockedItemRequest,
	): Promise<ILockedItemDetail> {
		return this.post(appId, "get", request);
	}

	async uploadLockedFile(
		appId: string,
		file: File,
		options: ILockedUploadOptions = {},
	): Promise<ILockedItemDetail> {
		const { signal, onProgress } = options;
		const total = file.size;
		const hash = await sha256Hex(file, {
			signal,
			onBytes: (loaded) => onProgress?.({ phase: "hashing", loaded, total }),
		});
		const ticket = await this.post<ILockedUploadTicket>(
			appId,
			"upload",
			lockedUploadRequest(file, hash, options),
			signal,
		);
		const target = { item_id: ticket.item.id, version_id: ticket.version.id };
		try {
			await uploadToSignedUrl(ticket.upload.url, file, {
				headers: ticket.upload.headers,
				signal,
				onBytes: (loaded) =>
					onProgress?.({ phase: "uploading", loaded, total }),
			});
		} catch (error) {
			await this.post(appId, "abort", target).catch((abortError) =>
				console.warn(
					`Aborting locked upload ${target.version_id} of ${file.name} failed:`,
					abortError,
				),
			);
			throw error;
		}
		onProgress?.({ phase: "committing", loaded: total, total });
		return this.post<ILockedItemDetail>(appId, "commit", target);
	}

	downloadLockedItem(
		appId: string,
		itemId: string,
		version?: number,
	): Promise<ILockedDownload> {
		return this.post(appId, "download", { item_id: itemId, version });
	}

	setLockedRetention(
		appId: string,
		itemId: string,
		versionId: string,
		retainUntil: string,
	): Promise<ILockedItemVersion> {
		return this.post(appId, "retention", {
			item_id: itemId,
			version_id: versionId,
			retain_until: retainUntil,
		});
	}

	setLockedLegalHold(
		appId: string,
		itemId: string,
		versionId: string,
		enabled: boolean,
	): Promise<ILockedItemVersion> {
		return this.post(appId, "legal-hold", {
			item_id: itemId,
			version_id: versionId,
			enabled,
		});
	}

	setLockedAutoDelete(
		appId: string,
		itemId: string,
		versionId: string,
		enabled: boolean,
	): Promise<ILockedItemVersion> {
		return this.post(appId, "auto-delete", {
			item_id: itemId,
			version_id: versionId,
			enabled,
		});
	}

	purgeLockedItem(
		appId: string,
		itemId: string,
		versionId?: string,
	): Promise<ILockedItemDetail> {
		return this.post(appId, "purge", {
			item_id: itemId,
			version_id: versionId,
		});
	}
}

function lockedUploadRequest(
	file: File,
	hash: string,
	options: ILockedUploadOptions,
): ICreateLockedUploadRequest {
	const request: ICreateLockedUploadRequest = {
		file_name: file.name,
		content_type: file.type || undefined,
		size: file.size,
		sha256: hash,
		retention_days: options.retentionDays,
		delete_after_retention: options.deleteAfterRetention || undefined,
		comment: options.comment?.trim() || undefined,
		metadata: options.metadata,
	};
	if (options.itemId) return { ...request, item_id: options.itemId };
	return {
		...request,
		scope: options.scope,
		folder: options.folder,
		name: options.name,
		if_exists: options.ifExists,
	};
}
