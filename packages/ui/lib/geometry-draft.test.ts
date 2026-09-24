import { describe, expect, test } from "bun:test";
import { normalizeGeometryValue } from "./geometry";
import {
	FIRST_RING,
	addDraftPosition,
	addDraftRing,
	addDraftShape,
	clampPosition,
	clampRingRef,
	convertGeometryDraft,
	draftPositionCount,
	draftToGeometry,
	emptyGeometryDraft,
	geometryToDraft,
	removeDraftPosition,
	removeDraftRing,
	removeDraftShape,
	updateDraftPosition,
} from "./geometry-draft";

const square: GeoJSON.Position[] = [
	[0, 0],
	[1, 0],
	[1, 1],
	[0, 1],
	[0, 0],
];
const hole: GeoJSON.Position[] = [
	[0.2, 0.2],
	[0.2, 0.4],
	[0.4, 0.4],
	[0.2, 0.2],
];
const geometries: [
	GeoJSON.Point,
	GeoJSON.MultiPoint,
	GeoJSON.LineString,
	GeoJSON.MultiLineString,
	GeoJSON.Polygon,
	GeoJSON.MultiPolygon,
] = [
	{ type: "Point", coordinates: [13.405, 52.52] },
	{
		type: "MultiPoint",
		coordinates: [
			[0, 0],
			[1, 1],
		],
	},
	{
		type: "LineString",
		coordinates: [
			[0, 0],
			[1, 1],
			[2, 0],
		],
	},
	{
		type: "MultiLineString",
		coordinates: [
			[
				[0, 0],
				[1, 1],
			],
			[
				[2, 2],
				[3, 3],
			],
		],
	},
	{ type: "Polygon", coordinates: [square, hole] },
	{
		type: "MultiPolygon",
		coordinates: [[square], [square.map(([x, y]) => [x + 5, y + 5])]],
	},
];

describe("geometry draft roundtrip", () => {
	for (const geometry of geometries) {
		test(`${geometry.type} survives draft and emit`, () => {
			const draft = geometryToDraft(geometry);
			expect(draft).not.toBeNull();
			expect(draftToGeometry(draft!)).toEqual(geometry);
			expect(() => normalizeGeometryValue(draftToGeometry(draft!))).not.toThrow();
		});
	}

	test("polygon rings are stored open and closed again on emit", () => {
		const draft = geometryToDraft({ type: "Polygon", coordinates: [square] })!;
		expect(draft.shapes[0][0]).toHaveLength(4);
		expect(draftToGeometry(draft)).toEqual({
			type: "Polygon",
			coordinates: [square],
		});
	});

	test("GeometryCollection and malformed values are not draftable", () => {
		expect(
			geometryToDraft({ type: "GeometryCollection", geometries: [] }),
		).toBeNull();
		expect(geometryToDraft({ type: "Point", coordinates: [1] })).toBeNull();
		expect(
			geometryToDraft({ type: "LineString", coordinates: [[0, "a"]] }),
		).toBeNull();
		expect(geometryToDraft(null)).toBeNull();
	});

	test("empty drafts emit null and keep one ring to draw into", () => {
		const draft = emptyGeometryDraft("Polygon");
		expect(draft.shapes).toEqual([[[]]]);
		expect(draftToGeometry(draft)).toBeNull();
		expect(draftPositionCount(draft)).toBe(0);
		expect(draftToGeometry(geometryToDraft({ type: "MultiLineString", coordinates: [] })!)).toBeNull();
	});

	test("empty holes and empty parts are dropped on emit", () => {
		let draft = geometryToDraft({ type: "Polygon", coordinates: [square] })!;
		draft = addDraftRing(draft, 0);
		expect(draft.shapes[0]).toHaveLength(2);
		expect(draftToGeometry(draft)).toEqual({
			type: "Polygon",
			coordinates: [square],
		});
		let multi = convertGeometryDraft(draft, "MultiPolygon");
		multi = addDraftShape(multi);
		expect(multi.shapes).toHaveLength(2);
		expect(draftToGeometry(multi)).toEqual({
			type: "MultiPolygon",
			coordinates: [[square]],
		});
	});
});

describe("geometry draft mutations", () => {
	test("point placement replaces instead of appending", () => {
		let draft = emptyGeometryDraft("Point");
		draft = addDraftPosition(draft, FIRST_RING, [1, 2]);
		draft = addDraftPosition(draft, FIRST_RING, [3, 4]);
		expect(draftToGeometry(draft)).toEqual({ type: "Point", coordinates: [3, 4] });
	});

	test("line vertices append, move and remove by index", () => {
		let draft = emptyGeometryDraft("LineString");
		draft = addDraftPosition(draft, FIRST_RING, [0, 0]);
		draft = addDraftPosition(draft, FIRST_RING, [1, 1]);
		draft = addDraftPosition(draft, FIRST_RING, [2, 2]);
		draft = updateDraftPosition(draft, FIRST_RING, 1, [5, 5]);
		draft = removeDraftPosition(draft, FIRST_RING, 0);
		expect(draftToGeometry(draft)).toEqual({
			type: "LineString",
			coordinates: [
				[5, 5],
				[2, 2],
			],
		});
	});

	test("positions are wrapped, clamped and rounded", () => {
		expect(clampPosition([190, 95])).toEqual([-170, 90]);
		expect(clampPosition([-190, -95])).toEqual([170, -90]);
		expect(clampPosition([180, 0])).toEqual([180, 0]);
		expect(clampPosition([1.23456789, 0])).toEqual([1.234568, 0]);
		const draft = addDraftPosition(emptyGeometryDraft("Point"), FIRST_RING, [
			541, 12,
		]);
		expect(draftToGeometry(draft)).toEqual({
			type: "Point",
			coordinates: [-179, 12],
		});
	});

	test("exterior rings and the last part are never removed", () => {
		let draft = geometryToDraft({ type: "Polygon", coordinates: [square, hole] })!;
		expect(removeDraftRing(draft, FIRST_RING)).toBe(draft);
		draft = removeDraftRing(draft, { shape: 0, ring: 1 });
		expect(draft.shapes[0]).toHaveLength(1);
		let multi = geometryToDraft(geometries[3])!;
		multi = removeDraftShape(multi, 0);
		multi = removeDraftShape(multi, 0);
		expect(multi.shapes).toEqual([[[]]]);
		expect(addDraftRing(multi, 0)).toBe(multi);
		expect(addDraftShape(geometryToDraft(geometries[0])!).shapes).toHaveLength(1);
	});

	test("ring references are clamped after removals", () => {
		const draft = geometryToDraft(geometries[3])!;
		expect(clampRingRef({ shape: 5, ring: 9 }, draft)).toEqual({
			shape: 1,
			ring: 0,
		});
		const same = { shape: 1, ring: 0 };
		expect(clampRingRef(same, draft)).toBe(same);
	});
});

describe("geometry draft conversion", () => {
	test("keeps positions when switching between single kinds", () => {
		const line = geometryToDraft(geometries[2])!;
		expect(draftToGeometry(convertGeometryDraft(line, "MultiPoint"))).toEqual({
			type: "MultiPoint",
			coordinates: geometries[2].coordinates,
		});
		expect(draftToGeometry(convertGeometryDraft(line, "Point"))).toEqual({
			type: "Point",
			coordinates: [0, 0],
		});
		expect(draftToGeometry(convertGeometryDraft(line, "Polygon"))).toEqual({
			type: "Polygon",
			coordinates: [[...geometries[2].coordinates, [0, 0]]],
		});
	});

	test("polygon holes survive the MultiPolygon roundtrip", () => {
		const polygon = geometryToDraft(geometries[4])!;
		const multi = convertGeometryDraft(polygon, "MultiPolygon");
		expect(draftToGeometry(multi)).toEqual({
			type: "MultiPolygon",
			coordinates: [[square, hole]],
		});
		expect(draftToGeometry(convertGeometryDraft(multi, "Polygon"))).toEqual(
			geometries[4],
		);
	});

	test("parts become rings and rings become parts", () => {
		const lines = geometryToDraft(geometries[3])!;
		expect(convertGeometryDraft(lines, "Polygon").shapes).toEqual([
			lines.shapes.flat(),
		]);
		expect(convertGeometryDraft(lines, "MultiPolygon").shapes).toEqual(
			lines.shapes,
		);
		expect(convertGeometryDraft(lines, "LineString").shapes[0][0]).toHaveLength(
			4,
		);
		expect(convertGeometryDraft(lines, "MultiLineString")).toBe(lines);
	});

	test("empty drafts convert without gaining positions", () => {
		const converted = convertGeometryDraft(emptyGeometryDraft("Point"), "Polygon");
		expect(converted).toEqual(emptyGeometryDraft("Polygon"));
	});
});
