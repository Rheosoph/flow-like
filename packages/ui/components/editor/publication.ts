import type { Value } from "platejs";
import {
	parsePlateDocument,
	replacePlateDocumentChildren,
} from "../../lib/plate-document";
import { signedUrlExpiry } from "../../lib/stable-asset-url";
import type { IStorageState } from "../../state/backend-state/storage-state";
import { resolveEditorAssetUrl } from "./hooks/use-editor-asset-url";
import { isStorageUrl } from "./upload-context";

export interface PublicationConfig {
	appId?: string;
	scope?: "app" | "user";
	maxAssetBytes?: number;
	maxTotalBytes?: number;
	/** The host application can copy the object to its CDN and return its permanent HTTPS address. */
	resolveAsset?: (reference: string) => Promise<string>;
}

type StorageReader = Pick<
	IStorageState,
	"downloadStorageItems" | "downloadStorageItemsUser"
>;
const MEDIA_TYPES = new Set(["img", "video", "audio", "file", "media_embed"]);
const LINK_PROTOCOLS = new Set(["http:", "https:", "mailto:", "tel:"]);

export function isPublicationUrl(url: string, type: string): boolean {
	if (url.startsWith("#") || (url.startsWith("/") && !url.startsWith("//")))
		return type === "a";
	try {
		const parsed = new URL(url);
		if (parsed.protocol === "data:") {
			const mime = /^data:([^;,]+);base64,/i.exec(url)?.[1]?.toLowerCase();
			if (type === "img")
				return !!mime && /^image\/(png|jpeg|gif|webp|avif)$/.test(mime);
			if (type === "audio")
				return !!mime && /^audio\/(mpeg|mp4|ogg|wav|webm)$/.test(mime);
			if (type === "video")
				return !!mime && /^video\/(mp4|webm|ogg)$/.test(mime);
			return (
				type === "file" &&
				!!mime &&
				/^(application\/(pdf|octet-stream)|text\/plain)$/.test(mime)
			);
		}
		return type === "a"
			? LINK_PROTOCOLS.has(parsed.protocol)
			: ["http:", "https:"].includes(parsed.protocol);
	} catch {
		return false;
	}
}

async function blobDataUrl(blob: Blob): Promise<string> {
	const bytes = new Uint8Array(await blob.arrayBuffer());
	let binary = "";
	for (let offset = 0; offset < bytes.length; offset += 8192)
		binary += String.fromCharCode(...bytes.subarray(offset, offset + 8192));
	return `data:${blob.type || "application/octet-stream"};base64,${btoa(binary)}`;
}

/** Resolve every private reference before generating a standalone document export. */
export async function preparePublicationValue(
	value: Value,
	config: PublicationConfig,
	storage: StorageReader,
): Promise<Value> {
	const maxAssetBytes = config.maxAssetBytes ?? 20 * 1024 * 1024;
	const maxTotalBytes = config.maxTotalBytes ?? 80 * 1024 * 1024;
	const assets = new Map<string, Promise<string>>();
	const existingData = new Set<string>();
	let total = 0;
	const resolve = (url: string) => {
		let pending = assets.get(url);
		if (!pending) {
			pending = (async () => {
				if (config.resolveAsset) {
					const resolved = await config.resolveAsset(url);
					if (!/^https:\/\//i.test(resolved))
						throw new Error(
							"The media resolver must return a permanent HTTPS media URL.",
						);
					return resolved;
				}
				const resolved = await resolveEditorAssetUrl(
					url,
					storage,
					config.appId,
					config.scope,
				);
				const response = await fetch(resolved);
				if (!response.ok)
					throw new Error(`Media download failed (${response.status}).`);
				const announced = Number(response.headers.get("content-length"));
				if (announced > maxAssetBytes)
					throw new Error("Media exceeds the export asset size limit.");
				const reader = response.body?.getReader();
				if (!reader) throw new Error("Media download returned no data.");
				const chunks: Uint8Array[] = [];
				let size = 0;
				try {
					for (;;) {
						const { done, value: chunk } = await reader.read();
						if (done) break;
						size += chunk.length;
						total += chunk.length;
						if (size > maxAssetBytes || total > maxTotalBytes)
							throw new Error(
								"Media exceeds the export size limit. Use a permanent media URL for large files.",
							);
						chunks.push(chunk);
					}
				} catch (error) {
					await reader.cancel();
					throw error;
				}
				return blobDataUrl(
					new Blob(chunks as BlobPart[], {
						type:
							response.headers.get("content-type")?.split(";")[0] ||
							"application/octet-stream",
					}),
				);
			})();
			assets.set(url, pending);
		}
		return pending;
	};
	const visit = async (
		node: Record<string, unknown>,
	): Promise<Record<string, unknown>> => {
		const result = { ...node };
		const type = typeof node.type === "string" ? node.type : "";
		if (typeof node.url === "string") {
			if (/^data:/i.test(node.url) && !existingData.has(node.url)) {
				const encoded = node.url.slice(node.url.indexOf(",") + 1);
				const size =
					Math.floor((encoded.length * 3) / 4) -
					(encoded.endsWith("==") ? 2 : encoded.endsWith("=") ? 1 : 0);
				if (size > maxAssetBytes || total + size > maxTotalBytes)
					throw new Error(
						"Media exceeds the export size limit. Use a permanent media URL for large files.",
					);
				total += size;
				existingData.add(node.url);
			}
			const url =
				MEDIA_TYPES.has(type) &&
				(isStorageUrl(node.url) || signedUrlExpiry(node.url) !== undefined)
					? await resolve(node.url)
					: node.url;
			if (!isPublicationUrl(url, type))
				throw new Error(`This ${type || "content"} URL cannot be exported.`);
			result.url = url;
		}
		if (Array.isArray(node.children))
			result.children = await Promise.all(node.children.map(visit));
		return result;
	};
	return (await Promise.all(value.map((node) => visit(node)))) as Value;
}

export async function preparePublicationContent(
	content: string,
	config: PublicationConfig,
	storage: StorageReader,
): Promise<string> {
	const document = parsePlateDocument(content);
	if (!document)
		throw new Error("The content must be a valid rich text document.");
	const children = await preparePublicationValue(
		document.children,
		config,
		storage,
	);
	return replacePlateDocumentChildren(content, children);
}
