import { describe, expect, test } from "bun:test";
import type { GeoGeometry, GeoPoint } from "../src";
import type { JsonSchema } from "../src/contract";
import { validateSchema } from "../src/validate";

const point: GeoPoint = { type: "Point", coordinates: [13.4, 52.5] };
const ring = [
	[0, 0],
	[1, 0],
	[0, 1],
	[0, 0],
];
const anyGeometry = { type: "object", "x-flow-like-type": "geometry" };
const pointSchema = { ...anyGeometry, "x-geometry": "Point" };

describe("geometry contract validation", () => {
	test("accepts every geometry kind and constrains subtypes", () => {
		const geometries = [
			point,
			{
				type: "LineString",
				coordinates: [
					[0, 0],
					[1, 1],
				],
			},
			{ type: "Polygon", coordinates: [ring] },
			{ type: "MultiPoint", coordinates: [[0, 0]] },
			{
				type: "MultiLineString",
				coordinates: [
					[
						[0, 0],
						[1, 1],
					],
				],
			},
			{ type: "MultiPolygon", coordinates: [[ring]] },
			{ type: "GeometryCollection", geometries: [point] },
		];
		for (const geometry of geometries) {
			expect(validateSchema(anyGeometry, geometry)).toEqual({
				valid: true,
				errors: [],
			});
			expect(
				validateSchema(
					{ ...anyGeometry, "x-geometry": geometry.type },
					geometry,
				).valid,
			).toBe(true);
			expect(validateSchema(pointSchema, geometry).valid).toBe(
				geometry.type === "Point",
			);
		}
	});

	test("accepts empty multi-geometries and collections", () => {
		for (const type of ["MultiPoint", "MultiLineString", "MultiPolygon"]) {
			expect(validateSchema(anyGeometry, { type, coordinates: [] }).valid).toBe(
				true,
			);
		}
		const collection: GeoGeometry = {
			type: "GeometryCollection",
			geometries: [],
		};
		expect(validateSchema(anyGeometry, collection).valid).toBe(true);
	});

	test.each([
		["longitude above range", { ...point, coordinates: [180.1, 0] }],
		["longitude below range", { ...point, coordinates: [-180.1, 0] }],
		["latitude above range", { ...point, coordinates: [0, 90.1] }],
		["latitude below range", { ...point, coordinates: [0, -90.1] }],
		["non-finite coordinates", { ...point, coordinates: [Number.NaN, 0] }],
		["Z dimension", { ...point, coordinates: [0, 0, 1] }],
		["missing coordinate", { ...point, coordinates: [0] }],
		["string coordinate", { ...point, coordinates: ["0", 0] }],
		["empty Point", { ...point, coordinates: [] }],
		["short LineString", { type: "LineString", coordinates: [[0, 0]] }],
		["empty Polygon", { type: "Polygon", coordinates: [] }],
		[
			"short ring",
			{
				type: "Polygon",
				coordinates: [
					[
						[0, 0],
						[1, 1],
						[0, 0],
					],
				],
			},
		],
		[
			"open ring",
			{
				type: "Polygon",
				coordinates: [
					[
						[0, 0],
						[1, 0],
						[0, 1],
						[1, 1],
					],
				],
			},
		],
		[
			"empty MultiLineString member",
			{ type: "MultiLineString", coordinates: [[]] },
		],
		["empty MultiPolygon member", { type: "MultiPolygon", coordinates: [[]] }],
		["Feature wrapper", { type: "Feature", geometry: point, properties: {} }],
		["alternate CRS", { ...point, crs: null }],
		[
			"coordinates on collection",
			{ type: "GeometryCollection", geometries: [], coordinates: [] },
		],
		["geometries on Point", { ...point, geometries: [] }],
		[
			"collection null member",
			{ type: "GeometryCollection", geometries: [null] },
		],
		["collection missing geometries", { type: "GeometryCollection" }],
	])("rejects %s", (_name, geometry) => {
		expect(validateSchema(anyGeometry, geometry).valid).toBe(false);
	});

	test("accepts boundary coordinates and antimeridian bounding boxes", () => {
		expect(
			validateSchema(pointSchema, {
				...point,
				coordinates: [180, -90],
				bbox: [170, -90, -170, 90],
			}).valid,
		).toBe(true);
		for (const bbox of [
			[0, 0, 0],
			[0, 10, 0, -10],
			[-181, 0, 0, 0],
			[0, 0, 0, 91],
		]) {
			expect(validateSchema(pointSchema, { ...point, bbox }).valid).toBe(false);
		}
	});

	test("preserves foreign members and polygon winding", () => {
		const polygon = {
			type: "Polygon",
			coordinates: [[...ring].reverse()],
			label: "Boundary",
		};
		const before = JSON.stringify(polygon);
		expect(validateSchema(anyGeometry, polygon).valid).toBe(true);
		expect(JSON.stringify(polygon)).toBe(before);
		for (const metadata of [
			undefined,
			Number.POSITIVE_INFINITY,
			1n,
			new Date(),
		]) {
			expect(validateSchema(pointSchema, { ...point, metadata }).valid).toBe(
				false,
			);
		}
	});

	test("validates compact markers and rejects malformed markers", () => {
		expect(
			validateSchema({ $id: "flow:geometry", "x-geometry": "Point" }, point)
				.valid,
		).toBe(true);
		const malformed: JsonSchema[] = [
			{ $id: "flow:geometry" },
			{ $id: "flow:geometry", "x-geometry": "Point", type: "object" },
			{ "x-geometry": "Point" },
			{ ...anyGeometry, "x-geometry": "Any" },
			{ ...anyGeometry, "x-geometry": "Feature" },
			{ ...anyGeometry, "x-geometry": null },
			{ ...anyGeometry, "x-geometry": 1 },
		];
		for (const schema of malformed) {
			expect(validateSchema(schema, point).valid).toBe(false);
		}
	});

	test("also enforces ordinary schema constraints", () => {
		const schema = {
			...pointSchema,
			properties: { label: { type: "string", minLength: 2 } },
			required: ["label"],
		};
		expect(validateSchema(schema, { ...point, label: "Home" }).valid).toBe(
			true,
		);
		expect(validateSchema(schema, { ...point, label: "X" }).valid).toBe(false);
		expect(validateSchema(schema, point).valid).toBe(false);
	});

	test("requires an explicit nullable branch to accept null", () => {
		expect(validateSchema(pointSchema, null).valid).toBe(false);
		const nullable = { anyOf: [pointSchema, { type: "null" }] };
		expect(validateSchema(nullable, null).valid).toBe(true);
		expect(validateSchema(nullable, point).valid).toBe(true);
		expect(validateSchema(nullable, {}).valid).toBe(false);
	});

	test("validates nested arrays and maps and reports the input path", () => {
		const schema = {
			type: "object",
			additionalProperties: { type: "array", items: pointSchema },
		};
		expect(validateSchema(schema, { stops: [point] }).valid).toBe(true);
		const invalid = validateSchema(schema, {
			stops: [{ ...point, coordinates: [0, 100] }],
		});
		expect(invalid.valid).toBe(false);
		expect(invalid.errors[0]).toContain("$.stops[0]");
	});

	test("bounds collection and foreign-member nesting before recursion", () => {
		let collection: GeoGeometry = point;
		for (let i = 0; i < 32; i++) {
			collection = { type: "GeometryCollection", geometries: [collection] };
		}
		expect(validateSchema(anyGeometry, collection).errors[0]).toContain(
			"depth",
		);
		const cyclic = { ...point, metadata: {} as Record<string, unknown> };
		cyclic.metadata.self = cyclic;
		expect(validateSchema(anyGeometry, cyclic).errors[0]).toContain("depth");
	});

	test("bounds total positions across collection members", () => {
		const multiPoint = {
			type: "MultiPoint",
			coordinates: Array.from({ length: 50_001 }, () => [0, 0]),
		};
		const collection = {
			type: "GeometryCollection",
			geometries: [multiPoint, multiPoint],
		};
		expect(validateSchema(anyGeometry, collection).errors[0]).toContain(
			"position limit",
		);
	});

	test("bounds foreign-member byte size", () => {
		expect(
			validateSchema(pointSchema, { ...point, label: "é".repeat(524_288) })
				.errors[0],
		).toContain("byte limit");
	});
});
