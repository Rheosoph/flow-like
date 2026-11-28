import { AUDIO_MODEL_PREVIEWS } from "./catalog-preview-audio";
import type {
	CatalogPreviewAsset,
	CatalogPreviewEntry,
	PreviewKind,
} from "./catalog-preview-types";
import { VISUAL_MODEL_PREVIEWS } from "./catalog-preview-visual";

export const CATALOG_PREVIEWS: CatalogPreviewEntry[] = [
	...VISUAL_MODEL_PREVIEWS,
	...AUDIO_MODEL_PREVIEWS,
];
export interface PreviewBit {
	id: string;
	type: string;
	repository?: string | null;
	parameters?: Record<string, unknown> | null;
	meta: Record<string, Record<string, unknown>>;
}
const EXTENSIONS: Record<PreviewKind, RegExp> = {
	image: /\.(png|jpe?g|webp|gif|avif|svg)$/i,
	audio: /\.(mp3|wav|ogg|oga|m4a|aac|flac|opus|weba)$/i,
	video: /\.(mp4|webm|mov|m4v)$/i,
};
const ALLOWED_TYPES: Record<PreviewKind, string[]> = {
	image: [
		"ImageGeneration",
		"VideoGeneration",
		"Forecasting",
		"ImageEmbedding",
		"ObjectDetection",
		"Stt",
		"SystemOne",
	],
	audio: ["Tts", "Stt"],
	video: ["VideoGeneration", "ImageGeneration"],
};
const SOURCE_SECTION = "Preview samples and sources:";

function httpsUrl(value: string): URL {
	if (typeof value !== "string" || value.trim() !== value || /\s/.test(value))
		throw new Error("Preview URLs must be nonempty URLs without whitespace");
	const url = new URL(value);
	if (url.protocol !== "https:" || url.username || url.password)
		throw new Error(
			"Preview media and sources require HTTPS URLs without credentials",
		);
	if (
		[...url.searchParams.keys()].some((key) =>
			/^(x-amz-|x-goog-|x-oss-|signature$|sig$|expires$|token$|access[_-]?token$|auth(?:orization)?$|api[_-]?key$|key[_-]?pair[_-]?id$|policy$|jwt$)/i.test(
				key,
			),
		)
	)
		throw new Error(
			"Preview URLs must not contain temporary signatures or access tokens",
		);
	return url;
}

function validateAsset(asset: CatalogPreviewAsset): void {
	const url = httpsUrl(asset.url);
	httpsUrl(asset.source_url);
	if (!EXTENSIONS[asset.kind]?.test(url.pathname))
		throw new Error(
			`Preview URL has no supported ${asset.kind} extension: ${asset.url}`,
		);
	if (
		typeof asset.description !== "string" ||
		!asset.description.trim() ||
		/[\r\n]/.test(asset.description)
	)
		throw new Error(
			"Every preview needs a single-line attribution description",
		);
	if (
		asset.scope !== undefined &&
		!["output", "base_model_output", "input_demo"].includes(asset.scope)
	)
		throw new Error("Invalid preview sample scope");
}

export function previewDescription(
	description: unknown,
	media: CatalogPreviewAsset[],
): string {
	if (description != null && typeof description !== "string")
		throw new Error("Invalid current long description");
	const original = typeof description === "string" ? description : "";
	const entries = media.map((asset, index) => {
		validateAsset(asset);
		const scope =
			asset.scope === "base_model_output"
				? " Sample from the underlying model; this catalog variant was not separately sampled."
				: asset.scope === "input_demo"
					? " This is example input used in the publisher's demonstration."
					: "";
		return `${index + 1}. ${asset.description}${scope} Source: ${asset.source_url}`;
	});
	const section = `${SOURCE_SECTION}\n${entries.join("\n")}`;
	// Replace only the numbered source lines, preserving owner copy before and after them.
	const managedSection =
		/(^|\n\n)Preview samples and sources:\n\d+\. [^\r\n]+ Source: https:\/\/\S+(?:\n\d+\. [^\r\n]+ Source: https:\/\/\S+)*/;
	if (managedSection.test(original))
		return original.replace(
			managedSection,
			(_match, prefix: string) => `${prefix}${section}`,
		);
	return [original, section].filter(Boolean).join("\n\n");
}

export function planPreviewUpdates(
	bits: PreviewBit[],
	entries: CatalogPreviewEntry[] = CATALOG_PREVIEWS,
) {
	const byId = new Map(bits.map((bit) => [bit.id, bit]));
	if (byId.size !== bits.length)
		throw new Error("Duplicate Bit IDs in preview inventory");
	const seen = new Set<string>();
	return entries.flatMap((entry) => {
		if (
			!entry.bit_ids.length ||
			!entry.media.length ||
			(!entry.model_id && !entry.repository)
		)
			throw new Error(
				"Each preview entry requires Bit IDs, exact model identity and media",
			);
		for (const asset of entry.media) validateAsset(asset);
		return entry.bit_ids.map((id) => {
			if (seen.has(id))
				throw new Error(`Multiple preview entries target ${id}`);
			seen.add(id);
			const bit = byId.get(id);
			if (!bit) throw new Error(`Preview Bit ${id} no longer exists`);
			const provider = bit.parameters?.provider as
				| Record<string, unknown>
				| undefined;
			if (
				(entry.model_id && provider?.model_id !== entry.model_id) ||
				(entry.repository && bit.repository !== entry.repository)
			)
				throw new Error(`Preview model identity changed for ${id}`);
			if (
				entry.media.some(
					(asset) => !ALLOWED_TYPES[asset.kind].includes(bit.type),
				)
			)
				throw new Error(`Preview media kind does not match ${bit.type}`);
			const meta = bit.meta.en;
			if (
				!meta ||
				typeof meta.name !== "string" ||
				typeof meta.description !== "string"
			)
				throw new Error(`Missing English model metadata for ${id}`);
			const existing = meta.preview_media ?? [];
			if (
				!Array.isArray(existing) ||
				existing.some((url) => typeof url !== "string")
			)
				throw new Error(`Invalid current preview media for ${id}`);
			const urls = [
				...new Set([
					...(existing as string[]),
					...entry.media.map((asset) => asset.url),
				]),
			];
			const longDescription = previewDescription(
				meta.long_description,
				entry.media,
			);
			return {
				bit,
				entry,
				meta: {
					...meta,
					preview_media: urls,
					long_description: longDescription,
				},
				changed:
					JSON.stringify(existing) !== JSON.stringify(urls) ||
					meta.long_description !== longDescription,
			};
		});
	});
}

/** `<svg` after whitespace, an optional XML declaration and comments; a comment ends at its first `-->`. */
const startsWithSvgElement = (text: string): boolean => {
	let rest = text.trimStart();
	if (rest.slice(0, 5).toLowerCase() === "<?xml") {
		const end = rest.indexOf(">");
		if (end < 0 || rest[end - 1] !== "?") return false;
		rest = rest.slice(end + 1).trimStart();
	}
	while (rest.startsWith("<!--")) {
		const end = rest.indexOf("-->", 4);
		if (end < 0) return false;
		rest = rest.slice(end + 3).trimStart();
	}
	return /^<svg[\s>]/i.test(rest);
};

function hasMediaSignature(bytes: Uint8Array, kind: PreviewKind): boolean {
	const ascii = new TextDecoder().decode(bytes.subarray(0, 256));
	const tag = (offset: number) =>
		String.fromCharCode(...bytes.subarray(offset, offset + 4));
	if (kind === "image")
		return (
			[0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a].every(
				(byte, i) => bytes[i] === byte,
			) ||
			(bytes[0] === 0xff && bytes[1] === 0xd8 && bytes[2] === 0xff) ||
			/^GIF8[79]a/.test(ascii) ||
			(tag(0) === "RIFF" && tag(8) === "WEBP") ||
			(tag(4) === "ftyp" && /avif|avis/.test(ascii)) ||
			startsWithSvgElement(ascii)
		);
	if (kind === "video")
		return (
			tag(4) === "ftyp" ||
			(bytes[0] === 0x1a &&
				bytes[1] === 0x45 &&
				bytes[2] === 0xdf &&
				bytes[3] === 0xa3)
		);
	return (
		/^ID3|^OggS|^fLaC/.test(ascii) ||
		(["RIFF", "RF64"].includes(tag(0)) && tag(8) === "WAVE") ||
		(bytes[0] === 0xff && (bytes[1] & 0xe0) === 0xe0) ||
		tag(4) === "ftyp" ||
		(bytes[0] === 0x1a &&
			bytes[1] === 0x45 &&
			bytes[2] === 0xdf &&
			bytes[3] === 0xa3)
	);
}

/** Check actual bytes so an HTTP-200 error page cannot become a broken preview. */
export async function verifyPreviewAsset(
	asset: CatalogPreviewAsset,
	fetcher: typeof fetch = fetch,
) {
	validateAsset(asset);
	const response = await fetcher(asset.url, {
		headers: {
			range: "bytes=0-1023",
			"user-agent": "Mozilla/5.0",
			"accept-encoding": "identity",
		},
		signal: AbortSignal.timeout(30_000),
	});
	if (!response.ok || !response.body) {
		await response.body?.cancel();
		throw new Error(`Preview returned HTTP ${response.status}: ${asset.url}`);
	}
	if (response.url) {
		const redirected = new URL(response.url);
		if (
			redirected.protocol !== "https:" ||
			redirected.username ||
			redirected.password
		) {
			await response.body.cancel();
			throw new Error("Preview redirected outside credential-free HTTPS");
		}
	}
	const reader = response.body.getReader();
	let bytes = new Uint8Array();
	try {
		while (bytes.length < 256) {
			const { done, value } = await reader.read();
			if (done) break;
			const needed = value.subarray(0, 256 - bytes.length);
			const joined = new Uint8Array(bytes.length + needed.length);
			joined.set(bytes);
			joined.set(needed, bytes.length);
			bytes = joined;
		}
	} finally {
		await reader.cancel();
	}
	if (
		/\b(?:text\/html|application\/xhtml\+xml)\b/i.test(
			response.headers.get("content-type") ?? "",
		) ||
		!hasMediaSignature(bytes, asset.kind)
	)
		throw new Error(
			`Preview response is not valid ${asset.kind} media: ${asset.url}`,
		);
	return {
		url: asset.url,
		kind: asset.kind,
		content_type: response.headers.get("content-type"),
		verified_at: new Date().toISOString(),
	};
}
