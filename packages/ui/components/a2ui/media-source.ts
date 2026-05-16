import type { BoundValue, SurfaceComponent } from "./types";

function literalString(value: string): BoundValue {
	return { literalString: value };
}

function literalBool(value: boolean): BoundValue {
	return { literalBool: value };
}

function inferFileType(
	mimeType: string | undefined,
	filename: string | undefined,
	src: string,
): string {
	const lowerMime = mimeType?.toLowerCase() ?? "";
	const name = (filename || src.split("?")[0].split("/").pop() || "").toLowerCase();

	if (lowerMime.startsWith("image/")) return "image";
	if (lowerMime.startsWith("video/")) return "video";
	if (lowerMime.startsWith("audio/")) return "audio";
	if (lowerMime === "application/pdf") return "pdf";
	if (lowerMime.startsWith("text/")) return "text";
	if (lowerMime.includes("json") || lowerMime.includes("javascript"))
		return "code";

	if (/\.(png|jpg|jpeg|gif|bmp|webp|svg)$/i.test(name)) return "image";
	if (/\.(mp4|mkv|webm|ogg|avi|mov)$/i.test(name)) return "video";
	if (/\.(mp3|wav|ogg|flac|aac)$/i.test(name)) return "audio";
	if (/\.pdf$/i.test(name)) return "pdf";
	if (/\.(txt|csv|md|mdx|log)$/i.test(name)) return "text";
	if (/\.(json|xml|css|js|jsx|ts|tsx|py|rs|html|yml|yaml|toml)$/i.test(name))
		return "code";

	return "unknown";
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

	const componentData = component.component as unknown as Record<string, unknown>;
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
			nextComponentData = {
				...componentData,
				src: srcValue,
				url: srcValue,
				...(filenameValue ? { filename: filenameValue } : {}),
				...(mimeTypeValue ? { mimeType: mimeTypeValue } : {}),
				...(fileTypeValue ? { fileType: fileTypeValue } : {}),
				...(mediaKind === "video" || mediaKind === "audio"
					? { showControls: componentData.showControls ?? literalBool(true) }
					: {}),
			};
			break;
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
