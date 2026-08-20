import type { ITemporaryUploadedFile } from "../state/backend-state";

const DEFAULT_CACHE_TTL_MS = 60 * 60 * 1000;
const EXPIRY_SAFETY_MARGIN_MS = 60 * 1000;

interface CachedTemporaryUpload {
	value: ITemporaryUploadedFile;
	expiresAt: number;
}

const cachedUploads = new Map<string, CachedTemporaryUpload>();
const pendingUploads = new Map<string, Promise<ITemporaryUploadedFile>>();

function bytesToHex(bytes: ArrayBuffer): string {
	return Array.from(new Uint8Array(bytes))
		.map((byte) => byte.toString(16).padStart(2, "0"))
		.join("");
}

async function hashFile(file: File): Promise<string> {
	if (!globalThis.crypto?.subtle) {
		return [
			"metadata",
			file.name,
			file.type,
			file.size,
			file.lastModified,
		].join(":");
	}

	const digest = await globalThis.crypto.subtle.digest(
		"SHA-256",
		await file.arrayBuffer(),
	);
	return bytesToHex(digest);
}

function cacheExpiry(uploaded: ITemporaryUploadedFile): number {
	const parsed = uploaded.downloadExpiresAt
		? Date.parse(uploaded.downloadExpiresAt)
		: Number.NaN;

	if (Number.isFinite(parsed)) {
		return Math.max(0, parsed - EXPIRY_SAFETY_MARGIN_MS);
	}

	return Date.now() + DEFAULT_CACHE_TTL_MS;
}

export async function temporaryUploadCacheKey(
	file: File,
	scope: string,
): Promise<string> {
	const hash = await hashFile(file);
	return [scope, file.name, file.type, file.size, hash].join("|");
}

/**
 * Identity key that never reads the file's bytes. Bulk uploads (folders with
 * thousands of files) use this: hashing every file's content would read the whole
 * selection into memory. The folder-relative path keeps same-named files in
 * different subfolders apart.
 */
export function temporaryUploadMetadataKey(file: File, scope: string): string {
	const path =
		(file as File & { webkitRelativePath?: string }).webkitRelativePath ||
		file.name;
	const hash = ["metadata", path, file.type, file.size, file.lastModified].join(
		":",
	);
	return [scope, file.name, file.type, file.size, hash].join("|");
}

export function readTemporaryUploadCache(
	key: string,
): ITemporaryUploadedFile | undefined {
	const cached = cachedUploads.get(key);
	if (cached && cached.expiresAt > Date.now()) return cached.value;

	cachedUploads.delete(key);
	return undefined;
}

export function writeTemporaryUploadCache(
	key: string,
	uploaded: ITemporaryUploadedFile,
): void {
	cachedUploads.set(key, { value: uploaded, expiresAt: cacheExpiry(uploaded) });
}

export async function getOrUploadTemporaryFile(
	file: File,
	scope: string,
	upload: () => Promise<ITemporaryUploadedFile>,
): Promise<ITemporaryUploadedFile> {
	const key = await temporaryUploadCacheKey(file, scope);
	const cached = readTemporaryUploadCache(key);
	if (cached) return cached;

	const pending = pendingUploads.get(key);
	if (pending) return pending;

	const promise = upload()
		.then((uploaded) => {
			writeTemporaryUploadCache(key, uploaded);
			return uploaded;
		})
		.finally(() => {
			pendingUploads.delete(key);
		});

	pendingUploads.set(key, promise);
	return promise;
}

export function clearTemporaryUploadCache(): void {
	cachedUploads.clear();
	pendingUploads.clear();
}
