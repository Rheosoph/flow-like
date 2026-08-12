"use client";

import type { IBackendState } from "../../../state/backend-state";
import type { IAttachment } from "./chat-db";

export async function fileToAttachment(
	files: File[],
	backend: IBackendState,
	offline: boolean,
): Promise<IAttachment[]> {
	if (!files || files.length === 0) return [];

	const attachments: IAttachment[] = [];

	for (const file of files) {
		const url = await backend.helperState.fileToUrl(file, offline);
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
}

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
