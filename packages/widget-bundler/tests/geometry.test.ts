import { describe, expect, test } from "bun:test";
import { writeFileSync } from "node:fs";
import { join } from "node:path";
import { validateInputValue, validateSchema } from "@flow-like/widget-sdk";
import { extractContract } from "../src/extract";
import { tmpDir } from "./helpers";

function extract(source: string) {
	const path = join(tmpDir("flwb-geometry"), "widget.config.ts");
	writeFileSync(
		path,
		`${source}
export default defineWidget<Inputs, Events, Queries>({ id: "geo-widget", name: "Geometry" });`,
	);
	return extractContract(path).contract;
}

const pointSchema = {
	type: "object",
	"x-flow-like-type": "geometry",
	"x-geometry": "Point",
};

describe("geometry contracts", () => {
	test("derives annotated inputs, containers, events and query schemas", () => {
		const contract = extract(`
/** @geometry Point */
interface Point { type: "Point"; coordinates: [number, number] }
interface RawPoint { type: "Point"; coordinates: [number, number] }
interface Inputs {
	/** Map center
	 * @geometry Point
	 * @default {"type":"Point","coordinates":[13.4,52.5]}
	 */
	center: RawPoint;
	selected?: Point;
	points: Point[];
	/** @uniqueItems true */
	uniquePoints: Point[];
	byId: Record<string, Point>;
	metadata: { geometry: string; label: string };
	/** @default {"geometry":"Point","$ref":"literal"} */
	data: { geometry: string; $ref: string };
}
interface Events { selected: Point }
interface Queries { locate: { args: Point; returns: Point[] } }
`);
		expect(contract.inputs.center).toEqual({
			type: "json",
			description: "Map center",
			default: { type: "Point", coordinates: [13.4, 52.5] },
			schema: {
				...pointSchema,
				description: "Map center",
				default: { type: "Point", coordinates: [13.4, 52.5] },
			},
		});
		expect(contract.inputs.selected).toEqual({
			type: "json",
			schema: pointSchema,
			optional: true,
		});
		expect(contract.inputs.points?.schema).toEqual({
			type: "array",
			items: pointSchema,
		});
		expect(contract.inputs.uniquePoints?.schema).toEqual({
			type: "array",
			items: pointSchema,
			uniqueItems: true,
		});
		expect(contract.inputs.byId?.schema).toEqual({
			type: "object",
			additionalProperties: pointSchema,
		});
		expect(contract.inputs.metadata?.schema?.properties).toEqual({
			geometry: { type: "string" },
			label: { type: "string" },
		});
		expect(contract.inputs.data?.default).toEqual({
			geometry: "Point",
			$ref: "literal",
		});
		expect(contract.events.selected?.payloadSchema).toEqual(pointSchema);
		expect(contract.queries.locate).toEqual({
			argsSchema: pointSchema,
			resultSchema: { type: "array", items: pointSchema },
		});
	}, 30000);

	test("SDK geometry types produce schemas that validate widget updates", () => {
		const sdkPath = join(import.meta.dir, "../../widget-sdk/src/index.ts");
		const contract = extract(`
import type { GeoPoint, GeoLineString, GeoPolygon, GeoMultiPoint, GeoMultiLineString, GeoMultiPolygon, GeoGeometryCollection, GeoGeometry } from ${JSON.stringify(sdkPath)};
interface Inputs {
	point: GeoPoint;
	line: GeoLineString;
	polygon: GeoPolygon;
	multiPoint: GeoMultiPoint;
	multiLine: GeoMultiLineString;
	multiPolygon: GeoMultiPolygon;
	collection: GeoGeometryCollection;
	any: GeoGeometry;
	/** @default [] */
	points: GeoPoint[];
	/** @default {} */
	byId: Record<string, GeoPoint>;
}
interface Events { moved: GeoPoint }
interface Queries { locate: { args: GeoGeometry; returns: GeoPoint } }
`);
		for (const [name, kind] of Object.entries({
			point: "Point",
			line: "LineString",
			polygon: "Polygon",
			multiPoint: "MultiPoint",
			multiLine: "MultiLineString",
			multiPolygon: "MultiPolygon",
			collection: "GeometryCollection",
		})) {
			expect(contract.inputs[name]?.schema).toMatchObject({
				type: "object",
				"x-flow-like-type": "geometry",
				"x-geometry": kind,
			});
		}
		expect(contract.inputs.any?.schema).toMatchObject({
			"x-flow-like-type": "geometry",
		});
		expect(contract.inputs.any?.schema).not.toHaveProperty("x-geometry");
		expect(JSON.stringify(contract)).not.toContain('"$ref"');
		const point = { type: "Point", coordinates: [13.4, 52.5] };
		const invalid = { type: "Point", coordinates: [181, 52.5] };
		const { point: pointInput, points, byId, collection } = contract.inputs;
		if (!pointInput || !points || !byId || !collection)
			throw new Error("Missing geometry inputs");
		expect(validateInputValue(pointInput, point).valid).toBe(true);
		expect(validateInputValue(pointInput, invalid).valid).toBe(false);
		expect(validateInputValue(points, [invalid]).valid).toBe(false);
		expect(validateInputValue(byId, { home: invalid }).valid).toBe(false);
		expect(
			validateInputValue(collection, {
				type: "GeometryCollection",
				geometries: [point, { type: "GeometryCollection", geometries: [] }],
			}).valid,
		).toBe(true);
		expect(
			validateSchema(contract.events.moved?.payloadSchema, invalid).valid,
		).toBe(false);
		expect(
			validateSchema(contract.queries.locate?.resultSchema, point).valid,
		).toBe(true);
	}, 30000);

	test.each(["Feature", "point", "", "42"])(
		"rejects invalid @geometry %s",
		(kind) => {
			expect(() =>
				extract(`
interface Inputs {
	/** @geometry ${kind} */
	location: { type: "Point"; coordinates: number[] };
}
interface Events {}
interface Queries {}
`),
			).toThrow(/Invalid @geometry.*input 'location'/);
		},
		30000,
	);

	test.each([
		["Point", "{ type: 'Point'; coordinates: string }"],
		["Point", "{ type: 'Point'; coordinates: string[] }"],
		[
			"GeometryCollection",
			"{ type: 'GeometryCollection'; geometries: string }",
		],
		["Any", "{ type: 'Polygon'; coordinates: number }"],
	])(
		"rejects incompatible coordinate shapes for %s",
		(kind, type) => {
			expect(() =>
				extract(`
interface Inputs {
	/** @geometry ${kind} */
	location: ${type};
}
interface Events {}
interface Queries {}
`),
			).toThrow(/incompatible coordinates or geometries/);
		},
		30000,
	);

	test.each([
		"string",
		"number",
		"{ type: 'Point'; coordinates: number[] }[]",
		"{ type: 'Point'; coordinates: number[] } | string",
	])(
		"rejects geometry annotations on %s",
		(type) => {
			expect(() =>
				extract(`
interface Inputs {
	/** @geometry Point */
	location: ${type};
}
interface Events {}
interface Queries {}
`),
			).toThrow(/must annotate a geometry object type/);
		},
		30000,
	);

	test.each([
		"Record<string, Point>",
		"{ label: string }",
		"{ type: 'LineString'; coordinates: [number, number][] }",
	])(
		"rejects incompatible annotated type %s",
		(type) => {
			expect(() =>
				extract(`
interface Point { type: "Point"; coordinates: [number, number] }
interface Inputs {
	/** @geometry Point */
	location: ${type};
}
interface Events {}
interface Queries {}
`),
			).toThrow(/must describe a compatible GeoJSON geometry object/);
		},
		30000,
	);

	test.each([
		["Point", '{"type":"Point","coordinates":[181,0]}'],
		["Point", "null"],
		["Point[]", '[{"type":"Point","coordinates":[0,91]}]'],
		["Record<string, Point>", '{"home":{"type":"Point","coordinates":[0,91]}}'],
	])(
		"rejects invalid defaults for %s: %s",
		(type, defaultValue) => {
			expect(() =>
				extract(`
/** @geometry Point */
interface Point { type: "Point"; coordinates: [number, number] }
interface Inputs {
	/** @default ${defaultValue} */
	location: ${type};
}
interface Events {}
interface Queries {}
`),
			).toThrow(/Invalid @default for geometry input 'location'/);
		},
		30000,
	);
});
