import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";
import {
	CRAFT_EDITOR_IDS,
	craftCreationFilename,
	craftCreationViewer,
	craftSaveLabel,
	craftViewer,
} from "./craft-viewers";
import { createStorageCraftFile } from "./storage-craft";

describe("storage editor routing and save targets", () => {
	it.each([
		["report.PDF", "pdfcraft", "pdf"],
		["slides.pptx", "deckcraft", "pptx"],
		["slides.potx", "deckcraft", "pptx"],
		["table.xls", "gridcraft", "xlsx"],
		["table.ods", "gridcraft", "xlsx"],
		["table.xlsm", "gridcraft", "xlsm"],
		["shape.svg", "vectorcraft", "svg"],
		["scan.jp2", "vectorcraft", "vectorcraft"],
		["scan.j2k", "vectorcraft", "vectorcraft"],
		["drawing.dxf", "cadcraft", "dxf"],
		["part.solidcraft", "solidcraft", "solidcraft"],
		["part.STEP", "solidcraft", "solidcraft"],
		["part.stp", "solidcraft", "solidcraft"],
		["part.iges", "solidcraft", "solidcraft"],
		["part.igs", "solidcraft", "solidcraft"],
		["part.stl", "solidcraft", "solidcraft"],
		["part.obj", "solidcraft", "solidcraft"],
		["part.3mf", "solidcraft", "solidcraft"],
		["part.glb", "solidcraft", "solidcraft"],
		["part.gltf", "solidcraft", "solidcraft"],
		["part.FCStd", "solidcraft", "solidcraft"],
		["part.3dm", "solidcraft", "solidcraft"],
		["layout.indd", "designcraft", "designcraft"],
		["layout.indt", "designcraft", "designcraft"],
		["layers.xcf", "photocraft", "pcraft"],
		["scan.DCM", "photocraft", "pcraft"],
		["texture.dds", "photocraft", "dds"],
		["frame.dpx", "photocraft", "dpx"],
		["frame.cin", "photocraft", "cin"],
		["image.iff", "photocraft", "iff"],
		["sound.mp3", "soundcraft", "wav"],
		["sound.flac", "soundcraft", "flac"],
		["movie.mov", "filmcraft", "mp4"],
		["project.fcproj", "filmcraft", "fcproj"],
		["photo.png", "photocraft", "png"],
		["animation.gif", "photocraft", "pcraft"],
	])("routes %s to %s and exports %s", (name, id, format) => {
		expect(craftViewer(name)).toMatchObject({ id, outputExtension: format });
	});
	it.each([
		"legacy.ppt",
		"drawing.dwg",
		"slides.odp",
		"old.doc",
		"file.aep",
		"source.ts",
		"config.json",
		"notes.txt",
		"photo.heic",
	])("keeps unsupported or ambiguous %s out of the editor", (name) =>
		expect(craftViewer(name)).toBeUndefined(),
	);
	it("preserves storage address escaping when saving the original", () => {
		const location = "folder/a%2Fb%2520c.xlsx";
		const file = createStorageCraftFile(
			new File(["bytes"], "forged.xlsx"),
			location,
			requireViewer(location),
			"id",
		);
		expect(file.name).toBe("a%2Fb%2520c.xlsx");
		expect(craftSaveLabel(requireViewer(location))).toBe("Save");
	});
	it("creates a distinct valid filename for formats imported through conversion", () => {
		const location = "folder/a%2Fb.xls";
		const file = createStorageCraftFile(
			new File(["bytes"], "forged.exe"),
			location,
			requireViewer(location),
			"unique",
		);
		expect(file.name).toBe("a%2Fb-converted-unique.xlsx");
		expect(craftSaveLabel(requireViewer(location))).toBe("Save as .xlsx");
	});
	it("saves imported CAD as a separate project while preserving native project names", () => {
		const imported = "parts/bracket%2Fleft.step";
		const converted = createStorageCraftFile(
			new File(["project"], "forged.step"),
			imported,
			requireViewer(imported),
			"unique",
		);
		expect(converted.name).toBe("bracket%2Fleft-converted-unique.solidcraft");
		expect(craftSaveLabel(requireViewer(imported))).toBe("Save as .solidcraft");
		const native = `parts/${converted.name}`;
		const saved = createStorageCraftFile(
			new File(["updated project"], "forged.step"),
			native,
			requireViewer(native),
			"another-id",
		);
		expect(saved.name).toBe(converted.name);
		expect(craftSaveLabel(requireViewer(native))).toBe("Save");
	});
});

describe("blank editor native formats", () => {
	it.each([
		["wordcraft", "docx"],
		["pdfcraft", "pdf"],
		["deckcraft", "pptx"],
		["gridcraft", "xlsx"],
		["vectorcraft", "vectorcraft"],
		["photocraft", "pcraft"],
		["soundcraft", "scraftz"],
		["filmcraft", "fcproj"],
		["designcraft", "designcraft"],
		["cadcraft", "dxf"],
		["solidcraft", "solidcraft"],
	])("creates %s in its editable .%s format", (editor, extension) => {
		expect(CRAFT_EDITOR_IDS).toContain(editor);
		expect(craftCreationFilename(editor)).toBe(`Untitled.${extension}`);
		expect(craftCreationViewer(editor)).toMatchObject({
			id: editor,
			extension,
			outputExtension: extension,
		});
	});
	it.each(["unknown", "toString", "__proto__"])(
		"does not create a document for unknown editor %s",
		(editor) => {
			expect(craftCreationFilename(editor)).toBeUndefined();
			expect(craftCreationViewer(editor)).toBeUndefined();
		},
	);
});

function requireViewer(name: string) {
	const viewer = craftViewer(name);
	if (!viewer) throw new Error(`Missing viewer for ${name}`);
	return viewer;
}

it("keeps runtime limits and advertised imports aligned with storage routing", () => {
	for (const filename of [
		"file.pdf",
		"file.pptx",
		"file.xlsx",
		"file.svg",
		"file.jp2",
		"file.jpx",
		"file.jpf",
		"file.j2k",
		"file.j2c",
		"file.jpc",
		"file.png",
		"file.wav",
		"file.mp4",
		"file.fcproj",
		"file.dxf",
		"file.solidcraft",
		"file.step",
		"file.stp",
		"file.iges",
		"file.igs",
		"file.stl",
		"file.obj",
		"file.3mf",
		"file.glb",
		"file.gltf",
		"file.fcstd",
		"file.3dm",
		"file.indd",
		"file.indt",
		"file.xcf",
		"file.xpm",
		"file.dcm",
		"file.dicom",
		"file.dc3",
		"file.dds",
		"file.dpx",
		"file.cin",
		"file.iff",
		"file.lbm",
		"file.ilbm",
		"file.idml",
	]) {
		const viewer = requireViewer(filename);
		const pin = JSON.parse(
			readFileSync(
				resolve(
					import.meta.dirname,
					"../../../craft-viewers",
					viewer.id,
					"upstream.json",
				),
				"utf8",
			),
		);
		expect(pin.extensions).toContain(viewer.extension);
		expect(pin.maxBytes).toBe(viewer.maxBytes);
		expect(pin.label).toBe(viewer.label);
	}
});
