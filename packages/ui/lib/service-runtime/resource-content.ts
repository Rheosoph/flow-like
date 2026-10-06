import createDOMPurify from "dompurify";

const passiveTypes = new Set([
	"image/png",
	"image/jpeg",
	"image/gif",
	"image/webp",
	"image/avif",
	"image/bmp",
	"image/x-icon",
	"image/vnd.microsoft.icon",
	"audio/mpeg",
	"audio/mp4",
	"audio/ogg",
	"audio/wav",
	"audio/x-wav",
	"audio/webm",
	"audio/flac",
	"audio/aac",
	"audio/opus",
	"video/mp4",
	"video/webm",
	"video/ogg",
	"video/quicktime",
	"text/plain",
	"text/csv",
	"text/markdown",
	"text/vtt",
	"application/json",
	"application/pdf",
	"model/gltf-binary",
	"model/gltf+json",
	"model/vnd.usdz+zip",
	"font/woff",
	"font/woff2",
]);
const svgTags = new Set([
	"#text",
	"svg",
	"g",
	"defs",
	"title",
	"desc",
	"path",
	"rect",
	"circle",
	"ellipse",
	"line",
	"polyline",
	"polygon",
	"text",
	"tspan",
	"lineargradient",
	"radialgradient",
	"stop",
	"clippath",
	"mask",
	"pattern",
]);
const svgAttributes = new Set([
	"xmlns",
	"viewbox",
	"preserveaspectratio",
	"id",
	"x",
	"y",
	"x1",
	"y1",
	"x2",
	"y2",
	"cx",
	"cy",
	"r",
	"rx",
	"ry",
	"fx",
	"fy",
	"width",
	"height",
	"d",
	"points",
	"transform",
	"fill",
	"fill-rule",
	"fill-opacity",
	"stroke",
	"stroke-width",
	"stroke-linecap",
	"stroke-linejoin",
	"stroke-miterlimit",
	"stroke-dasharray",
	"stroke-dashoffset",
	"stroke-opacity",
	"opacity",
	"clip-path",
	"clip-rule",
	"mask",
	"maskunits",
	"maskcontentunits",
	"gradientunits",
	"gradienttransform",
	"spreadmethod",
	"offset",
	"stop-color",
	"stop-opacity",
	"patternunits",
	"patterncontentunits",
	"patterntransform",
	"font-family",
	"font-size",
	"font-weight",
	"font-style",
	"text-anchor",
	"dominant-baseline",
	"dx",
	"dy",
	"rotate",
	"textlength",
	"lengthadjust",
]);

/** A blob loses the device's CSP, so active documents cannot retain an executable MIME type. */
export async function runtimeAssetBlob(
	parts: BlobPart[],
	contentType: string | null,
): Promise<Blob> {
	const mime = contentType?.split(";", 1)[0].trim().toLowerCase() ?? "";
	if (mime === "image/svg+xml") {
		if (typeof window === "undefined")
			return new Blob(parts, { type: "application/octet-stream" });
		const purifier = createDOMPurify(window);
		if (!purifier.isSupported)
			return new Blob(parts, { type: "application/octet-stream" });
		purifier.addHook("uponSanitizeElement", (_node, data) => {
			// DOMPurify keeps a temporary body around the SVG until serialization.
			if (data.tagName !== "body" && !svgTags.has(data.tagName.toLowerCase()))
				data.allowedTags[data.tagName] = false;
		});
		purifier.addHook("uponSanitizeAttribute", (_node, data) => {
			const name = data.attrName.toLowerCase();
			const value = data.attrValue.trim();
			if (
				!svgAttributes.has(name) ||
				/[\\<>]/.test(value) ||
				value.includes("/*") ||
				(name === "xmlns" && value !== "http://www.w3.org/2000/svg") ||
				(/url\s*\(/i.test(value) &&
					!/^url\(#[A-Za-z_][A-Za-z0-9_.-]*\)$/.test(value))
			)
				data.keepAttr = false;
		});
		const source = await new Blob(parts).text();
		const sanitized = purifier.sanitize(source, {
			USE_PROFILES: { svg: true },
			ALLOW_DATA_ATTR: false,
			ALLOW_ARIA_ATTR: false,
		});
		return new Blob([sanitized], { type: "image/svg+xml" });
	}
	return new Blob(parts, {
		type: passiveTypes.has(mime) ? mime : "application/octet-stream",
	});
}
