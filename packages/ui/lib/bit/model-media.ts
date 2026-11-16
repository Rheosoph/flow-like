import type { IBit } from "../schema/bit/bit";

export interface PreviewMedia {
	images: string[];
	audio: string[];
	video: string[];
}

const IMAGE = /\.(png|jpe?g|webp|gif|avif|svg)$/i;
const AUDIO = /\.(mp3|wav|ogg|oga|m4a|aac|flac|opus|weba)$/i;
const VIDEO = /\.(mp4|webm|mov|m4v)$/i;

/** The path of an http(s) or same-origin URL, or null for anything a page must not load. */
function mediaPath(value: string): string | null {
	try {
		const url = new URL(value, "https://local.invalid");
		if (!["https:", "http:"].includes(url.protocol)) return null;
		if (url.username || url.password) return null;
		return url.pathname;
	} catch {
		return null;
	}
}

/** Sample outputs a bit lists in `meta.en.preview_media`, sorted by kind from their file names. */
export function bitPreviewMedia(bit: IBit): PreviewMedia {
	const media: PreviewMedia = { images: [], audio: [], video: [] };
	const urls = bit.meta?.en?.preview_media;
	if (!Array.isArray(urls)) return media;
	for (const url of new Set(urls)) {
		if (typeof url !== "string" || !url.trim()) continue;
		const path = mediaPath(url.trim());
		if (!path) continue;
		if (IMAGE.test(path)) media.images.push(url.trim());
		else if (AUDIO.test(path)) media.audio.push(url.trim());
		else if (VIDEO.test(path)) media.video.push(url.trim());
	}
	return media;
}
