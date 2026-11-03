import { storageDisplayName } from "../../lib/storage-tree";

export interface CraftViewer {
	id: string;
	label: string;
	extension: string;
	outputExtension: string;
	mime: string;
	inputMime: string;
	maxBytes: number;
	nativePreview?: "pdf" | "image" | "audio" | "video";
}

const MIME: Record<string, string> = {
	pdf: "application/pdf",
	docx: "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
	pptx: "application/vnd.openxmlformats-officedocument.presentationml.presentation",
	xlsx: "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
	xlsm: "application/vnd.ms-excel.sheet.macroEnabled.12",
	xltx: "application/vnd.openxmlformats-officedocument.spreadsheetml.template",
	xltm: "application/vnd.ms-excel.template.macroEnabled.12",
	csv: "text/csv",
	tsv: "text/tab-separated-values",
	tab: "text/tab-separated-values",
	svg: "image/svg+xml",
	svgz: "image/svg+xml",
	png: "image/png",
	jpg: "image/jpeg",
	jpeg: "image/jpeg",
	jpe: "image/jpeg",
	jfif: "image/jpeg",
	gif: "image/gif",
	apng: "image/apng",
	webp: "image/webp",
	bmp: "image/bmp",
	tif: "image/tiff",
	tiff: "image/tiff",
	ico: "image/x-icon",
	wav: "audio/wav",
	wave: "audio/wav",
	aif: "audio/aiff",
	aiff: "audio/aiff",
	flac: "audio/flac",
	mp3: "audio/mpeg",
	ogg: "audio/ogg",
	oga: "audio/ogg",
	m4a: "audio/mp4",
	aac: "audio/aac",
	mid: "audio/midi",
	midi: "audio/midi",
	smf: "audio/midi",
	mp4: "video/mp4",
	m4v: "video/mp4",
	mov: "video/quicktime",
	webm: "video/webm",
	mkv: "video/x-matroska",
	avi: "video/x-msvideo",
	step: "model/step",
	stp: "model/step",
	iges: "model/iges",
	igs: "model/iges",
	stl: "model/stl",
	obj: "model/obj",
	"3mf": "model/3mf",
	glb: "model/gltf-binary",
	gltf: "model/gltf+json",
	fcstd: "application/x-extension-fcstd",
	"3dm": "model/vnd.3dm",
	indd: "application/x-indesign",
	indt: "application/x-indesign",
	xcf: "image/x-xcf",
	xpm: "image/x-xpixmap",
	dcm: "application/dicom",
	dicom: "application/dicom",
	dc3: "application/dicom",
	dds: "image/vnd-ms.dds",
	dpx: "image/x-dpx",
	cin: "image/x-cin",
	iff: "image/x-iff",
	lbm: "image/x-iff",
	ilbm: "image/x-iff",
	jpf: "image/jp2",
	jpx: "image/jpx",
	jp2: "image/jp2",
	j2k: "image/jp2",
	j2c: "image/jp2",
	jpc: "image/jp2",
};

const viewers = new Map<string, CraftViewer>();
function register(
	id: string,
	label: string,
	extensions: string,
	output?: string,
	maxMiB = 50,
	nativePreview?: CraftViewer["nativePreview"],
) {
	for (const extension of extensions.split(" ")) {
		const outputExtension = output ?? extension;
		viewers.set(extension, {
			id,
			label,
			extension,
			outputExtension,
			maxBytes: maxMiB * 1024 * 1024,
			nativePreview,
			mime: MIME[outputExtension] ?? "application/octet-stream",
			inputMime: MIME[extension] ?? "application/octet-stream",
		});
	}
}
register("wordcraft", "Flows Word", "docx");
register("pdfcraft", "Flows PDF", "pdf", undefined, 200, "pdf");
register("deckcraft", "Flows Slides", "pptx");
register("deckcraft", "Flows Slides", "potx ppsx", "pptx");
register("gridcraft", "Flows Sheets", "xlsx xlsm xltx xltm csv tsv tab");
register("gridcraft", "Flows Sheets", "xls xlt xlsb ods", "xlsx");
register(
	"vectorcraft",
	"Flows Vector",
	"svg svgz ai eps emf wmf vectorcraft vctemplate",
);
register(
	"vectorcraft",
	"Flows Vector",
	"drawcraft ait af afdesign afpub jpf jpx jp2 j2k j2c jpc",
	"vectorcraft",
);
register(
	"photocraft",
	"Flows Photo",
	"pcraft ora psd psb tif tiff dib tga icb vda vst ico qoi exr hdr pnm fits fit fts dds dpx cin iff lbm ilbm",
);
register(
	"photocraft",
	"Flows Photo",
	"png jpg jpeg jpe jfif bmp",
	undefined,
	50,
	"image",
);
register("photocraft", "Flows Photo", "psdt", "psd");
register("photocraft", "Flows Photo", "apng gif webp", "pcraft", 50, "image");
register(
	"photocraft",
	"Flows Photo",
	"pbm pgm ppm pam pfm pdn pcx kra dng cr2 cr3 nef nrw arw pef orf rw2 raf afphoto xcf xpm dcm dicom dc3",
	"pcraft",
);
register("soundcraft", "Flows Audio", "wav wave flac", undefined, 100, "audio");
register(
	"soundcraft",
	"Flows Audio",
	"aif aiff mid midi smf scraftz",
	undefined,
	100,
);
register(
	"soundcraft",
	"Flows Audio",
	"mp3 ogg oga m4a aac",
	"wav",
	100,
	"audio",
);
register("soundcraft", "Flows Audio", "bwf aifc caf", "wav", 100);
register("soundcraft", "Flows Audio", "scraft", "scraftz", 100);
register("filmcraft", "Flows Video", "mp4", undefined, 200, "video");
register("filmcraft", "Flows Video", "m4v mov webm", "mp4", 200, "video");
register("filmcraft", "Flows Video", "mkv avi mxf mts m2ts", "mp4", 200);
register("filmcraft", "Flows Video", "fcproj", undefined, 200);
register("designcraft", "Flows Design", "idml designcraft");
register("designcraft", "Flows Design", "indd indt", "designcraft");
register("cadcraft", "Flows CAD", "dxf", undefined, 100);
// A SolidCraft project keeps modeling edits without replacing an imported model.
register(
	"solidcraft",
	"Flows CAD",
	"solidcraft step stp iges igs stl obj 3mf glb gltf fcstd 3dm",
	"solidcraft",
	100,
);

export function craftViewer(filename: string): CraftViewer | undefined {
	return viewers.get(
		storageDisplayName(filename).split(".").at(-1)?.toLowerCase() ?? "",
	);
}

export const CRAFT_EDITOR_IDS = [
	"wordcraft",
	"pdfcraft",
	"deckcraft",
	"gridcraft",
	"vectorcraft",
	"photocraft",
	"soundcraft",
	"filmcraft",
	"designcraft",
	"cadcraft",
	"solidcraft",
] as const;
export type CraftEditorId = (typeof CRAFT_EDITOR_IDS)[number];

const CREATION_FORMATS: Record<CraftEditorId, string> = {
	wordcraft: "docx",
	pdfcraft: "pdf",
	deckcraft: "pptx",
	gridcraft: "xlsx",
	vectorcraft: "vectorcraft",
	photocraft: "pcraft",
	soundcraft: "scraftz",
	filmcraft: "fcproj",
	designcraft: "designcraft",
	cadcraft: "dxf",
	solidcraft: "solidcraft",
};

export function craftCreationFilename(editor: string): string | undefined {
	const extension = CREATION_FORMATS[editor as CraftEditorId];
	return typeof extension === "string" ? `Untitled.${extension}` : undefined;
}

export function craftCreationViewer(editor: string): CraftViewer | undefined {
	const filename = craftCreationFilename(editor);
	return filename ? craftViewer(filename) : undefined;
}

export function craftSaveLabel(viewer: CraftViewer): string {
	return viewer.extension === viewer.outputExtension
		? "Save"
		: `Save as .${viewer.outputExtension}`;
}
