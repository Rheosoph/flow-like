import { describe, expect, it } from "vitest";
import { resolveCraftEditor, resolveCraftPreview } from "./file-preview-source";

const PPTX_MIME =
	"application/vnd.openxmlformats-officedocument.presentationml.presentation";
const DOCX_MIME =
	"application/vnd.openxmlformats-officedocument.wordprocessingml.document";

describe("explicit editor source selection", () => {
	it("uses the native project format for a new document", () => {
		expect(
			resolveCraftEditor("solidcraft", undefined, "New part.step"),
		).toMatchObject({
			filename: "New part.solidcraft",
			viewer: { id: "solidcraft", extension: "solidcraft" },
		});
	});
	it("uses the selected editor for an opaque initial file", () => {
		expect(
			resolveCraftEditor("wordcraft", "https://files.example/download"),
		).toMatchObject({ filename: "download.docx", viewer: { id: "wordcraft" } });
	});
	it("rejects unsupported editor IDs and mismatched initial files", () => {
		expect(resolveCraftEditor("unknown")).toBeUndefined();
		expect(resolveCraftEditor("solidcraft", "/letter.docx")).toBeUndefined();
		expect(resolveCraftEditor("solidcraft", "/model.dwg")).toBeUndefined();
	});
});

describe("Craft preview source detection", () => {
	it("uses an explicit presentation filename for an opaque signed URL", () => {
		expect(
			resolveCraftPreview(
				"https://files.example/object?signature=abc",
				"Quarterly.PPTX",
			),
		).toMatchObject({
			filename: "Quarterly.PPTX",
			viewer: { id: "deckcraft", extension: "pptx", outputExtension: "pptx" },
		});
	});

	it.each([
		"https://files.example/Quarterly%20review.pptx?signature=abc#preview",
		"https://files.example/object?filename=Quarterly%20review.pptx&signature=abc",
		"storage://files/Quarterly%20review.pptx",
	])("detects the presentation name in %s", (source) => {
		expect(resolveCraftPreview(source)).toMatchObject({
			filename: "Quarterly review.pptx",
			viewer: { id: "deckcraft", extension: "pptx" },
		});
	});

	it("detects an extensionless document from a parameterized MIME type", () => {
		expect(
			resolveCraftPreview(
				"https://files.example/object",
				undefined,
				` ${PPTX_MIME.toUpperCase()}; charset=binary `,
			),
		).toMatchObject({
			filename: "object.pptx",
			viewer: { id: "deckcraft", extension: "pptx" },
		});
	});

	it("uses the MIME type inside a data URL", () => {
		expect(
			resolveCraftPreview(`data:${DOCX_MIME};base64,UEsDBA==`),
		).toMatchObject({
			filename: "document.docx",
			viewer: { id: "wordcraft", extension: "docx" },
		});
	});

	it.each([
		["legacy.ppt", PPTX_MIME],
		["legacy.doc", DOCX_MIME],
		["drawing.dwg", "model/step"],
	])(
		"does not reinterpret unsupported %s from its MIME type",
		(filename, mime) => {
			expect(
				resolveCraftPreview(
					`https://files.example/${filename}`,
					undefined,
					mime,
				),
			).toBeUndefined();
			expect(
				resolveCraftPreview("https://files.example/object", filename, mime),
			).toBeUndefined();
			expect(
				resolveCraftPreview(`data:${mime};base64,UEsDBA==`, filename),
			).toBeUndefined();
		},
	);

	it.each([
		["template.potx", "deckcraft", "pptx"],
		["slideshow.ppsx", "deckcraft", "pptx"],
		["table.csv", "gridcraft", "csv"],
		["table.tsv", "gridcraft", "tsv"],
		["table.xls", "gridcraft", "xlsx"],
		["table.xlsb", "gridcraft", "xlsx"],
		["table.ods", "gridcraft", "xlsx"],
		["part.step", "solidcraft", "solidcraft"],
		["part.FCStd", "solidcraft", "solidcraft"],
		["part.3dm", "solidcraft", "solidcraft"],
		["layout.indd", "designcraft", "designcraft"],
		["layout.indt", "designcraft", "designcraft"],
		["image.xcf", "photocraft", "pcraft"],
		["image.xpm", "photocraft", "pcraft"],
		["scan.dicom", "photocraft", "pcraft"],
		["texture.dds", "photocraft", "dds"],
		["part.solidcraft", "solidcraft", "solidcraft"],
	])(
		"selects the editor and save format for %s",
		(filename, id, outputExtension) => {
			expect(resolveCraftPreview(`/files/${filename}`)).toMatchObject({
				filename,
				viewer: { id, outputExtension },
			});
		},
	);

	it.each([
		["text/csv; charset=utf-8", "csv"],
		["text/tab-separated-values", "tsv"],
		["application/vnd.ms-excel.sheet.macroEnabled.12", "xlsm"],
	])("detects spreadsheet MIME type %s", (mime, extension) => {
		expect(
			resolveCraftPreview("https://files.example/object", undefined, mime),
		).toMatchObject({
			filename: `object.${extension}`,
			viewer: { id: "gridcraft", extension },
		});
	});

	it.each([
		["model/step; charset=binary", "step"],
		["application/step", "step"],
		["model/iges", "iges"],
		["application/iges", "iges"],
		["model/stl", "stl"],
		["application/sla", "stl"],
		["model/obj", "obj"],
		["model/3mf", "3mf"],
		["model/gltf-binary", "glb"],
		["model/gltf+json", "gltf"],
	])("detects CAD MIME type %s", (mime, extension) => {
		expect(
			resolveCraftPreview("https://files.example/object", undefined, mime),
		).toMatchObject({
			filename: `object.${extension}`,
			viewer: { id: "solidcraft", extension, outputExtension: "solidcraft" },
		});
	});

	it("keeps recognized filenames authoritative over a conflicting MIME type", () => {
		expect(
			resolveCraftPreview("/files/table.xlsx", undefined, PPTX_MIME),
		).toMatchObject({
			filename: "table.xlsx",
			viewer: { id: "gridcraft" },
		});
	});

	it("leaves unsupported files to the existing preview fallback", () => {
		expect(resolveCraftPreview("/files/archive.zip")).toBeUndefined();
		expect(
			resolveCraftPreview(
				"/files/object",
				undefined,
				"application/octet-stream",
			),
		).toBeUndefined();
		expect(
			resolveCraftPreview("/files/object", undefined, "application/msword"),
		).toBeUndefined();
	});
});

// Opaque download URLs must route the same way as named files.
it.each([
	["application/x-extension-fcstd", "fcstd", "solidcraft", "solidcraft"],
	["model/vnd.3dm", "3dm", "solidcraft", "solidcraft"],
	["application/x-indesign", "indd", "designcraft", "designcraft"],
	["application/dicom", "dcm", "photocraft", "pcraft"],
	["image/x-xcf", "xcf", "photocraft", "pcraft"],
	["image/vnd-ms.dds", "dds", "photocraft", "dds"],
	["image/jp2", "jp2", "vectorcraft", "vectorcraft"],
])(
	"routes %s for both previews and explicit editors",
	(mimeType, extension, id, outputExtension) => {
		const expected = { viewer: { id, extension, outputExtension } };
		expect(
			resolveCraftPreview(
				"https://files.example/download",
				undefined,
				mimeType,
			),
		).toMatchObject(expected);
		expect(
			resolveCraftEditor(
				id,
				"https://files.example/download",
				undefined,
				mimeType,
			),
		).toMatchObject(expected);
	},
);
