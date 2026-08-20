"use client";

import { createContext, useContext } from "react";
import { AIUsageAppContext } from "./ai-usage-context";

/**
 * Where an editable surface stores media dropped, pasted or picked inside it.
 *
 * Uploads land in app storage under `{prefix}/…` and the document stores the durable
 * `storage://{prefix}/…` path, never a signed URL — signed URLs expire, and a document
 * outlives them.
 */
export interface UploadedMedia {
	/** Durable `storage://…` reference written into the document. */
	readonly url: string;
	/** Path inside the app's upload area, without the scheme. */
	readonly path: string;
	readonly name: string;
	readonly size: number;
	readonly type: string;
}

export interface EditorUploadConfig {
	/** App owning the storage bucket. Without it the editor cannot upload. */
	readonly appId?: string;
	/** Folder prefix inside the app's upload area. */
	readonly prefix: string;
	/** `user` writes to the caller's private area instead of the shared app area. */
	readonly scope: "app" | "user";
	/** Notified after a successful upload, so a host surface can raise its own event. */
	readonly onUploaded?: (media: UploadedMedia) => void;
	/** Notified when an upload fails, with the originating file name. */
	readonly onUploadError?: (name: string, message: string) => void;
}

export const DEFAULT_UPLOAD_PREFIX = "editor";

export const EditorUploadContext = createContext<
	EditorUploadConfig | undefined
>(undefined);

/**
 * Resolve the active upload target, inheriting `appId` from the surrounding editor when the
 * surface did not name one explicitly.
 */
export function useEditorUpload(): EditorUploadConfig {
	const config = useContext(EditorUploadContext);
	const inheritedAppId = useContext(AIUsageAppContext);

	return {
		appId: config?.appId ?? inheritedAppId,
		prefix: config?.prefix ?? DEFAULT_UPLOAD_PREFIX,
		scope: config?.scope ?? "app",
		onUploaded: config?.onUploaded,
		onUploadError: config?.onUploadError,
	};
}

export const STORAGE_URL_PREFIX = "storage://";

/** Normalize a prefix to a slash-free-edges storage folder. */
export function normalizeUploadPrefix(prefix: string): string {
	let start = 0;
	let end = prefix.length;
	while (start < end && prefix[start] === "/") start += 1;
	while (end > start && prefix[end - 1] === "/") end -= 1;
	return prefix.slice(start, end);
}

export function isStorageUrl(url: string | undefined): url is string {
	return typeof url === "string" && url.startsWith(STORAGE_URL_PREFIX);
}

export function toStorageUrl(prefix: string, filename: string): string {
	const folder = normalizeUploadPrefix(prefix);
	return folder
		? `${STORAGE_URL_PREFIX}${folder}/${filename}`
		: `${STORAGE_URL_PREFIX}${filename}`;
}

export function storagePathFromUrl(url: string): string {
	return url.slice(STORAGE_URL_PREFIX.length);
}
