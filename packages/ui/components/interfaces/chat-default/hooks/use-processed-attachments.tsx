import { useMemo } from "react";
import {
	type ProcessedAttachment,
	getDisplayFileName,
	getFileExtension,
} from "../attachment";
import type { IAttachment } from "../chat-db";

export const useProcessedAttachments = (attachments: IAttachment[]) => {
	return useMemo(() => {
		const processed: ProcessedAttachment[] = [];

		for (const attachment of attachments) {
			const attachmentUrl =
				typeof attachment === "string" ? attachment : attachment.url;
			const attachmentData: Partial<IAttachment> =
				typeof attachment === "object" ? attachment : {};

			let type: ProcessedAttachment["type"] = "other";
			let name = attachmentData.name ?? "";
			let isDataUrl = false;

			if (attachmentUrl.startsWith("data:")) {
				isDataUrl = true;
				const mimeMatch = attachmentUrl.match(/^data:([^;]+)/);
				const mimeType = mimeMatch?.[1] ?? "";

				if (attachmentData.type) {
					if (attachmentData.type.startsWith("image/")) type = "image";
					else if (attachmentData.type.startsWith("video/")) type = "video";
					else if (attachmentData.type.startsWith("audio/")) type = "audio";
					else if (attachmentData.type === "application/pdf") type = "pdf";
					else if (
						attachmentData.type.includes("document") ||
						attachmentData.type.includes("text")
					)
						type = "document";
				} else {
					if (mimeType.startsWith("image/")) type = "image";
					else if (mimeType.startsWith("video/")) type = "video";
					else if (mimeType.startsWith("audio/")) type = "audio";
					else if (mimeType === "application/pdf") type = "pdf";
				}

				if (!name) {
					const extension = mimeType.split("/")[1] || "file";
					name = `${type === "other" ? "Data" : type.charAt(0).toUpperCase() + type.slice(1)}.${extension}`;
				}
			} else {
				try {
					const url = new URL(attachmentUrl);
					const pathname = url.pathname.toLowerCase();

					if (!name) {
						const pathParts = pathname.split("/");
						const filename = pathParts[pathParts.length - 1];
						name = filename || url.hostname;
					}

					const cleanPath = pathname.split("?")[0];

					if (attachmentData.type) {
						if (attachmentData.type.startsWith("image/")) type = "image";
						else if (attachmentData.type.startsWith("video/")) type = "video";
						else if (attachmentData.type.startsWith("audio/")) type = "audio";
						else if (attachmentData.type === "application/pdf") type = "pdf";
						else if (
							attachmentData.type.includes("document") ||
							attachmentData.type.includes("text")
						)
							type = "document";
						else if (attachmentData.type.includes("html")) type = "website";
					} else {
						if (cleanPath.match(/\.(jpg|jpeg|png|gif|webp|svg|bmp|tiff)$/))
							type = "image";
						else if (cleanPath.match(/\.(mp4|webm|mov|avi|mkv|m4v|3gp|ogv)$/))
							type = "video";
						else if (cleanPath.match(/\.(mp3|wav|ogg|m4a|flac|aac|wma)$/))
							type = "audio";
						else if (cleanPath.match(/\.pdf$/)) type = "pdf";
						else if (
							cleanPath.match(/\.(doc|docx|txt|md|rtf|xls|xlsx|ppt|pptx)$/)
						)
							type = "document";
						else if (url.protocol === "http:" || url.protocol === "https:") {
							type = "website";
							if (!name || name === url.hostname) name = url.hostname;
						}
					}
				} catch {
					const lowerUrl = attachmentUrl.toLowerCase();
					if (lowerUrl.match(/\.(jpg|jpeg|png|gif|webp|svg|bmp|tiff)$/)) {
						type = "image";
						name = name || attachmentUrl.split("/").pop() || attachmentUrl;
					} else if (lowerUrl.match(/\.(mp4|webm|mov|avi|mkv|m4v|3gp|ogv)$/)) {
						type = "video";
						name = name || attachmentUrl.split("/").pop() || attachmentUrl;
					} else if (lowerUrl.match(/\.(mp3|wav|ogg|m4a|flac|aac|wma)$/)) {
						type = "audio";
						name = name || attachmentUrl.split("/").pop() || attachmentUrl;
					} else {
						name = name || attachmentUrl.split("/").pop() || attachmentUrl;
					}
				}
			}

			const displayName = getDisplayFileName(name);

			processed.push({
				url: attachmentUrl,
				name,
				displayName,
				ext: getFileExtension(displayName),
				type,
				pageNumber: attachmentData.page,
				isDataUrl,
				thumbnailUrl: attachmentData.thumbnail_url,
				previewText: attachmentData.preview_text,
				size: attachmentData.size,
				anchor: attachmentData.anchor,
			});
		}

		return processed;
	}, [attachments]);
};
