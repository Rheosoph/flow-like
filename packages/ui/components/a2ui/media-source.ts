import { resolveCraftPreview } from "./display/file-preview-source";
import { inferFileName } from "./media-filename";
import type { BoundValue, SurfaceComponent } from "./types";

export { inferFileName } from "./media-filename";

function literalString(value: string): BoundValue {
	return { literalString: value };
}

function literalBool(value: boolean): BoundValue {
	return { literalBool: value };
}

export type MediaKind =
	| "pdf"
	| "image"
	| "video"
	| "audio"
	| "code"
	| "text"
	| "unknown";

function getExtension(src: string, filename?: string): string {
	return inferFileName(src, filename).split(".").pop()?.toLowerCase() ?? "";
}

export function inferFileType(
	mimeType: string | undefined,
	filename: string | undefined,
	src: string,
): MediaKind {
	const lowerMime = mimeType?.toLowerCase() ?? "";

	if (lowerMime.startsWith("image/")) return "image";
	if (lowerMime.startsWith("video/")) return "video";
	if (lowerMime.startsWith("audio/")) return "audio";
	if (lowerMime === "application/pdf") return "pdf";
	if (lowerMime.startsWith("text/")) return "text";
	if (lowerMime.includes("json") || lowerMime.includes("javascript"))
		return "code";

	const ext = getExtension(src, filename);
	if (
		["png", "jpg", "jpeg", "gif", "bmp", "webp", "svg", "ico", "avif"].includes(
			ext,
		)
	)
		return "image";
	if (["mp4", "mkv", "webm", "ogv", "ogg", "avi", "mov"].includes(ext))
		return "video";
	if (
		[
			"mp3",
			"wav",
			"ogg",
			"oga",
			"opus",
			"flac",
			"aac",
			"m4a",
			"aif",
			"aiff",
		].includes(ext)
	)
		return "audio";
	if (ext === "pdf") return "pdf";
	if (
		["txt", "csv", "md", "mdx", "ini", "conf", "cfg", "log", "env"].includes(
			ext,
		)
	)
		return "text";
	if (
		[
			"json",
			"xml",
			"css",
			"js",
			"jsx",
			"ts",
			"tsx",
			"py",
			"java",
			"c",
			"cpp",
			"h",
			"hpp",
			"cs",
			"go",
			"rb",
			"php",
			"swift",
			"kt",
			"rs",
			"html",
			"yml",
			"yaml",
			"toml",
			"sql",
			"sh",
			"bash",
			"scss",
			"sass",
			"less",
			"vue",
			"svelte",
		].includes(ext)
	)
		return "code";

	return "unknown";
}

export function inferMimeTypeFromSource(
	src: string,
	filename?: string,
): string {
	const ext = getExtension(src, filename);
	const mimeByExtension: Record<string, string> = {
		pdf: "application/pdf",
		png: "image/png",
		jpg: "image/jpeg",
		jpeg: "image/jpeg",
		gif: "image/gif",
		bmp: "image/bmp",
		webp: "image/webp",
		svg: "image/svg+xml",
		ico: "image/x-icon",
		avif: "image/avif",
		mp4: "video/mp4",
		mkv: "video/x-matroska",
		webm: "video/webm",
		ogv: "video/ogg",
		avi: "video/x-msvideo",
		mov: "video/quicktime",
		mp3: "audio/mpeg",
		wav: "audio/wav",
		ogg: "audio/ogg",
		oga: "audio/ogg",
		opus: "audio/opus",
		flac: "audio/flac",
		aac: "audio/aac",
		m4a: "audio/mp4",
		aif: "audio/aiff",
		aiff: "audio/aiff",
		txt: "text/plain",
		csv: "text/csv",
		md: "text/markdown",
		mdx: "text/markdown",
		json: "application/json",
		xml: "application/xml",
		css: "text/css",
		js: "text/javascript",
		jsx: "text/javascript",
		ts: "text/typescript",
		tsx: "text/typescript",
		html: "text/html",
		yml: "application/yaml",
		yaml: "application/yaml",
		toml: "application/toml",
		sql: "application/sql",
	};
	return mimeByExtension[ext] ?? "";
}

export function applyMediaSourceUpdate(
	component: SurfaceComponent,
	updateValue: Record<string, unknown>,
): SurfaceComponent {
	const src = String(updateValue.src ?? updateValue.url ?? "");
	const mimeType =
		typeof updateValue.mimeType === "string" ? updateValue.mimeType : undefined;
	const filename =
		typeof updateValue.filename === "string" ? updateValue.filename : undefined;
	const mediaKind =
		typeof updateValue.mediaKind === "string"
			? updateValue.mediaKind
			: inferFileType(mimeType, filename, src);

	const componentData = component.component as unknown as Record<
		string,
		unknown
	>;
	const componentType = componentData.type as string | undefined;
	const srcValue = literalString(src);
	const filenameValue = filename ? literalString(filename) : undefined;
	const mimeTypeValue = mimeType ? literalString(mimeType) : undefined;
	const fileTypeValue = mediaKind ? literalString(mediaKind) : undefined;

	let nextComponentData: Record<string, unknown>;

	switch (componentType) {
		case "image":
			nextComponentData = {
				...componentData,
				src: srcValue,
				url: src,
				...(filename && componentData.alt === undefined
					? { alt: literalString(filename) }
					: {}),
			};
			break;
		case "avatar":
			nextComponentData = {
				...componentData,
				src: srcValue,
				...(filename && componentData.fallback === undefined
					? { fallback: literalString(filename.slice(0, 2).toUpperCase()) }
					: {}),
			};
			break;
		case "video":
			nextComponentData = {
				...componentData,
				src: srcValue,
				controls: componentData.controls ?? literalBool(true),
			};
			break;
		case "lottie":
		case "iframe":
			nextComponentData = {
				...componentData,
				src: srcValue,
				...(componentType === "iframe" ? { srcdoc: undefined } : {}),
			};
			break;
		case "filePreview":
		case "fileEditor": {
			const viewer = resolveCraftPreview(src, filename, mimeType)?.viewer;
			// Node media kinds are broad hints. Let the registry select Office,
			// spreadsheet, and other non-native viewers from the file metadata.
			const previewType =
				mediaKind === "file" ||
				(viewer && !viewer.nativePreview && viewer.extension !== "svg")
					? literalString("unknown")
					: fileTypeValue;
			nextComponentData = {
				...componentData,
				src: srcValue,
				url: srcValue,
				...(filenameValue ? { filename: filenameValue } : {}),
				...(mimeTypeValue ? { mimeType: mimeTypeValue } : {}),
				...(componentType === "filePreview" && previewType
					? { fileType: previewType }
					: {}),
				...(componentType === "filePreview" &&
				(mediaKind === "video" || mediaKind === "audio")
					? { showControls: componentData.showControls ?? literalBool(true) }
					: {}),
			};
			break;
		}
		default:
			nextComponentData = {
				...componentData,
				src: srcValue,
				url: srcValue,
				...(filenameValue ? { filename: filenameValue } : {}),
				...(mimeTypeValue ? { mimeType: mimeTypeValue } : {}),
				...(fileTypeValue ? { fileType: fileTypeValue } : {}),
			};
			break;
	}

	return {
		...component,
		component: nextComponentData as unknown as SurfaceComponent["component"],
	};
}
