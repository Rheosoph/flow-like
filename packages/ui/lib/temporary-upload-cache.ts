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

export async function getOrUploadTemporaryFile(
	file: File,
	scope: string,
	upload: () => Promise<ITemporaryUploadedFile>,
): Promise<ITemporaryUploadedFile> {
	const key = await temporaryUploadCacheKey(file, scope);
	const cached = cachedUploads.get(key);
	if (cached && cached.expiresAt > Date.now()) {
		return cached.value;
	}

	cachedUploads.delete(key);

	const pending = pendingUploads.get(key);
	if (pending) return pending;

	const promise = upload()
		.then((uploaded) => {
			cachedUploads.set(key, {
				value: uploaded,
				expiresAt: cacheExpiry(uploaded),
			});
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
