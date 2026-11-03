import {
	craftCreationFilename,
	craftCreationViewer,
	craftViewer,
} from "../../storage/craft-viewers";
import { inferFileName } from "../media-filename";

const DOCUMENT_EXTENSIONS: Record<string, string> = {
	"application/vnd.openxmlformats-officedocument.wordprocessingml.document":
		"docx",
	"application/vnd.openxmlformats-officedocument.presentationml.presentation":
		"pptx",
	"application/vnd.openxmlformats-officedocument.presentationml.template":
		"potx",
	"application/vnd.openxmlformats-officedocument.presentationml.slideshow":
		"ppsx",
	"application/vnd.openxmlformats-officedocument.spreadsheetml.sheet": "xlsx",
	"application/vnd.ms-excel.sheet.macroenabled.12": "xlsm",
	"application/vnd.openxmlformats-officedocument.spreadsheetml.template":
		"xltx",
	"application/vnd.ms-excel.template.macroenabled.12": "xltm",
	"application/vnd.ms-excel.sheet.binary.macroenabled.12": "xlsb",
	"application/vnd.ms-excel": "xls",
	"application/vnd.oasis.opendocument.spreadsheet": "ods",
	"application/pdf": "pdf",
	"text/csv": "csv",
	"text/tab-separated-values": "tsv",
	"model/step": "step",
	"application/step": "step",
	"model/iges": "iges",
	"application/iges": "iges",
	"model/stl": "stl",
	"application/sla": "stl",
	"model/obj": "obj",
	"model/3mf": "3mf",
	"model/gltf-binary": "glb",
	"model/gltf+json": "gltf",
	"application/x-extension-fcstd": "fcstd",
	"application/x-freecad": "fcstd",
	"model/vnd.3dm": "3dm",
	"application/x-indesign": "indd",
	"image/x-xcf": "xcf",
	"image/x-xpixmap": "xpm",
	"application/dicom": "dcm",
	"image/vnd-ms.dds": "dds",
	"image/x-dpx": "dpx",
	"image/x-cin": "cin",
	"image/x-iff": "iff",
	"image/jp2": "jp2",
	"image/jpx": "jpx",
};

/** Downloads can have an opaque URL and only a MIME type. */
export function resolveCraftPreview(
	source: string,
	filename?: string,
	mimeType?: string,
) {
	const name = inferFileName(source, filename);
	const viewer = craftViewer(name);
	if (viewer) return { viewer, filename: name };
	// An explicit extension remains authoritative, including unsupported .ppt/.doc.
	if ((filename || !source.startsWith("data:")) && /\.[a-z0-9]+$/i.test(name))
		return undefined;
	const extension =
		DOCUMENT_EXTENSIONS[
			(
				mimeType ??
				(source.startsWith("data:") ? source.slice(5).split(/[;,]/)[0] : "")
			)
				.split(";")[0]
				.trim()
				.toLowerCase()
		];
	if (!extension) return undefined;
	const inferredViewer = craftViewer(`file.${extension}`);
	return inferredViewer
		? {
				viewer: inferredViewer,
				filename: `${filename || (name === "file" || source.startsWith("data:") ? "document" : name)}.${extension}`,
			}
		: undefined;
}

/** Explicit editor selection keeps a source from opening in a different editor. */
export function resolveCraftEditor(
	editor: string,
	source?: string,
	filename?: string,
	mimeType?: string,
) {
	const viewer = craftCreationViewer(editor);
	if (!viewer) return undefined;
	if (!source) {
		const name = filename ? inferFileName("", filename) : undefined;
		return {
			viewer,
			filename: name
				? `${name.replace(/\.[^.]+$/, "")}.${viewer.outputExtension}`
				: (craftCreationFilename(editor) ??
					`Untitled.${viewer.outputExtension}`),
		};
	}
	const detected = resolveCraftPreview(source, filename, mimeType);
	if (detected) return detected.viewer.id === editor ? detected : undefined;
	const name = inferFileName(source, filename);
	if ((filename || !source.startsWith("data:")) && /\.[a-z0-9]+$/i.test(name))
		return undefined;
	return {
		viewer,
		filename: `${name === "file" || source.startsWith("data:") ? "document" : name}.${viewer.extension}`,
	};
}
