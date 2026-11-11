"use client";

import type { IBackendState } from "../../../state/backend-state";
import type { IAttachment } from "./chat-db";

/** Mirror `PORTABLE_INLINE_IMAGE_BYTES` and `PORTABLE_INLINE_IMAGES_BYTES` in flow-like-types. */
const INLINE_IMAGE_BYTES = 3_750_000;
const INLINE_IMAGES_BYTES = 4_000_000;

/**
 * Whether `file` reaches a model unchanged when sent inline: it is no image, or it fits its share
 * of what one message inlines. A larger image is better sent as a signed link, so a provider that
 * fetches links reads the original rather than a copy shrunk to fit.
 */
export function fitsInlineUnchanged(file: File, files: readonly File[]) {
	if (!file.type.startsWith("image/")) return true;
	const images = files.filter((each) => each.type.startsWith("image/")).length;
	return (
		file.size <=
		Math.min(INLINE_IMAGE_BYTES, INLINE_IMAGES_BYTES / Math.max(images, 1))
	);
}

export async function fileToAttachment(
	files: File[],
	backend: IBackendState,
	offline: boolean | ((file: File) => boolean),
	assertCurrent?: () => void,
): Promise<IAttachment[]> {
	if (!files || files.length === 0) return [];

	const attachments: IAttachment[] = [];

	for (const file of files) {
		assertCurrent?.();
		const url = await backend.helperState.fileToUrl(
			file,
			typeof offline === "function" ? offline(file) : offline,
		);
		assertCurrent?.();
		attachments.push({
			name: file.name,
			type: file.type,
			size: file.size,
			url: url,
		});
	}

	return attachments;
}

export interface ProcessedAttachment {
	url: string;
	name: string;
	/** Decoded, path-stripped name — the only variant safe to render. */
	displayName: string;
	/** Lower-case extension without the dot, empty when the name carries none. */
	ext: string;
	type: "image" | "video" | "audio" | "pdf" | "document" | "website" | "other";
	pageNumber?: number;
	isDataUrl: boolean;
	thumbnailUrl?: string;
	previewText?: string;
	size?: number | null;
	anchor?: string;
	/** The `storage://` url `url` was signed from; the stable identity of the file. */
	sourceUrl?: string;
	/** Set while a stored file has no signed link yet, or when it cannot be signed. */
	access?: "pending" | "unavailable";
}

export const attachmentKey = (
	file: Pick<ProcessedAttachment, "url" | "sourceUrl">,
) => file.sourceUrl ?? file.url;

export function getDisplayFileName(name: string) {
	let decoded = name;
	try {
		decoded = decodeURIComponent(name);
	} catch {
		decoded = name;
	}
	const parts = decoded.split(/[/\\]/);
	return parts[parts.length - 1] || decoded;
}

export function getFileExtension(displayName: string) {
	const dot = displayName.lastIndexOf(".");
	if (dot <= 0 || dot === displayName.length - 1) return "";
	const ext = displayName.slice(dot + 1).toLowerCase();
	return /^[a-z0-9]{1,8}$/.test(ext) ? ext : "";
}

/**
 * Splits a name so the extension can be pinned while the stem ellipsizes —
 * truncating `report-final.xlsx` from the right would drop the one part that
 * says what the file is.
 */
export function splitFileName(displayName: string) {
	const ext = getFileExtension(displayName);
	if (!ext) return { stem: displayName, suffix: "" };
	return {
		stem: displayName.slice(0, displayName.length - ext.length - 1),
		suffix: `.${ext}`,
	};
}

export function getAttachmentHost(url: string) {
	try {
		return new URL(url).hostname.replace(/^www\./, "");
	} catch {
		return "";
	}
}
