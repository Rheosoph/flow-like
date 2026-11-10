import type { IStorageState } from "../../state/backend-state/storage-state";

export const STORAGE_ATTACH_MAX_FILES = 10;
export const STORAGE_ATTACH_MAX_BYTES = 35 * 1024 * 1024;

export class StorageAttachError extends Error {}

const MIME_BY_EXTENSION: Readonly<Record<string, string>> = {
	png: "image/png",
	jpg: "image/jpeg",
	jpeg: "image/jpeg",
	gif: "image/gif",
	webp: "image/webp",
	svg: "image/svg+xml",
	pdf: "application/pdf",
	txt: "text/plain",
	md: "text/markdown",
	csv: "text/csv",
	json: "application/json",
	html: "text/html",
	mp3: "audio/mpeg",
	wav: "audio/wav",
	mp4: "video/mp4",
	zip: "application/zip",
	docx: "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
	xlsx: "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
	pptx: "application/vnd.openxmlformats-officedocument.presentationml.presentation",
};

export function mimeTypeForName(name: string, fallback = ""): string {
	const declared = fallback.split(";")[0].trim();
	if (declared && declared !== "application/octet-stream") return declared;
	const extension = name.split(".").pop()?.toLowerCase() ?? "";
	return MIME_BY_EXTENSION[extension] ?? "application/octet-stream";
}

export interface StoragePick {
	readonly path: string;
	readonly name: string;
	readonly size: number;
}

export function checkStoragePicks(
	picks: readonly StoragePick[],
): "tooMany" | "tooLarge" | null {
	if (picks.length > STORAGE_ATTACH_MAX_FILES) return "tooMany";
	const total = picks.reduce((sum, pick) => sum + pick.size, 0);
	return total > STORAGE_ATTACH_MAX_BYTES ? "tooLarge" : null;
}

async function storageBlob(
	storage: IStorageState,
	appId: string,
	path: string,
): Promise<Blob> {
	const local = await storage.readStorageItem?.(appId, path, false);
	if (local) return new Blob([local]);
	const [result] = await storage.downloadStorageItems(appId, [path]);
	if (!result?.url || result.error)
		throw new StorageAttachError(result?.error ?? "download not authorized");
	const response = await fetch(result.url);
	if (!response.ok)
		throw new StorageAttachError(`download failed (${response.status})`);
	return response.blob();
}

export async function fetchStorageFiles(
	storage: IStorageState,
	appId: string,
	picks: readonly StoragePick[],
): Promise<File[]> {
	return Promise.all(
		picks.map(async (pick) => {
			const blob = await storageBlob(storage, appId, pick.path);
			return new File([blob], pick.name, {
				type: mimeTypeForName(pick.name, blob.type),
			});
		}),
	);
}
