import { describe, expect, test } from "bun:test";

import { importGeoJsonRuntime } from "../hooks/use-frontend-runtime-tool-executor";
import type { IDatabaseSchemaField } from "../state/backend-state/db-state";
import {
	batchGeoJsonRows,
	geoJsonPropertyColumnName,
	isRfc3339DateTime,
	planGeoJsonImport,
} from "./geojson-import";

const OPTIONS = { geometryColumn: "geometry", keyColumn: "feature_id" };

const square = (x: number, y: number, size = 1) => [
	[x, y],
	[x + size, y],
	[x + size, y + size],
	[x, y + size],
	[x, y],
];

const polygonWithHole = {
	type: "Polygon",
	coordinates: [square(13, 52, 1), square(13.25, 52.25, 0.5).reverse()],
};

function feature(
	properties: Record<string, unknown> | null,
	extra: Record<string, unknown> = {},
) {
	return {
		type: "Feature",
		geometry: { type: "Point", coordinates: [13.4, 52.5] },
		properties,
		...extra,
	};
}

function collection(features: unknown[], extra: Record<string, unknown> = {}) {
	return { type: "FeatureCollection", features, ...extra };
}

function columnTypes(plan: ReturnType<typeof planGeoJsonImport>) {
	return Object.fromEntries(
		plan.columns.map((column) => [column.name, column.type]),
	);
}

describe("planGeoJsonImport", () => {
	test("types heterogeneous properties over every feature", () => {
		const plan = planGeoJsonImport(
			collection([
				feature({
					physicalSiteId: "P1",
					area: 12,
					height: 3.5,
					active: true,
					opened: "2024-01-02T03:04:05Z",
					code: "A",
				}),
				feature({
					physicalSiteId: "P2",
					area: 7,
					height: 4,
					active: false,
					opened: "2024-02-29T23:00:00.125+01:00",
					note: "sparse",
				}),
				feature({ physicalSiteId: 42, area: null, code: 7 }),
			]),
			OPTIONS,
		);

		expect(columnTypes(plan)).toEqual({
			feature_id: "string",
			geometry: "geometry",
			physical_site_id: "string",
			area: "int64",
			height: "float64",
			active: "boolean",
			opened: "timestamp:ms:UTC",
			code: "string",
			note: "string",
		});
		expect(plan.fields).toContainEqual({
			name: "feature_id",
			type: "string",
			nullable: false,
			primary_key: true,
		});
		expect(plan.fields).toContainEqual({
			name: "geometry",
			type: "geometry",
			nullable: true,
		});
		expect(plan.rows[2]).toMatchObject({
			physical_site_id: "42",
			area: null,
			code: "7",
			note: null,
			opened: null,
		});
		expect(plan.rows[1].note).toBe("sparse");
		expect(plan.warnings.join("\n")).toContain(
			"mix types: physical_site_id, code",
		);
	});

	test("maps property names to snake_case columns and keeps the source name", () => {
		const plan = planGeoJsonImport(
			collection([
				feature({
					physicalSiteId: "P1",
					HTTPServerName: "x",
					"addr:city": "Berlin",
					"2020 value": 1,
					Café: "open",
				}),
			]),
			OPTIONS,
		);

		expect(plan.columns.slice(2)).toEqual([
			{
				name: "physical_site_id",
				type: "string",
				source_property: "physicalSiteId",
			},
			{
				name: "http_server_name",
				type: "string",
				source_property: "HTTPServerName",
			},
			{ name: "addr_city", type: "string", source_property: "addr:city" },
			{ name: "_2020_value", type: "int64", source_property: "2020 value" },
			{ name: "cafe", type: "string", source_property: "Café" },
		]);
	});

	test("stores nested objects and arrays as JSON text", () => {
		const plan = planGeoJsonImport(
			collection([
				feature({
					official_location_ids: ["loc-1", "loc-2"],
					osm_tags: { "addr:city": "Berlin", building: "yes" },
				}),
			]),
			OPTIONS,
		);

		expect(columnTypes(plan)).toMatchObject({
			official_location_ids: "string",
			osm_tags: "string",
		});
		expect(plan.rows[0]).toMatchObject({
			official_location_ids: '["loc-1","loc-2"]',
			osm_tags: '{"addr:city":"Berlin","building":"yes"}',
		});
		expect(plan.warnings.join("\n")).toContain(
			"JSON text in: official_location_ids, osm_tags",
		);
	});

	test("stores null for sparse properties named like Object.prototype members", () => {
		const plan = planGeoJsonImport(
			JSON.parse(`{"type":"FeatureCollection","features":[
				{"type":"Feature","geometry":null,"properties":{"constructor":"Acme","toString":"x","valueOf":1,"__proto__":{"a":1}}},
				{"type":"Feature","geometry":null,"properties":{}}
			]}`),
			OPTIONS,
		);

		expect(plan.rows[0]).toMatchObject({
			constructor: "Acme",
			to_string: "x",
			value_of: 1,
			proto: '{"a":1}',
		});
		expect(plan.rows[1]).toMatchObject({
			constructor: null,
			to_string: null,
			value_of: null,
			proto: null,
		});
	});

	test("keys rows by Feature.id, generating and de-duplicating keys without displacing real ids", () => {
		const plan = planGeoJsonImport(
			collection([
				feature({}, { id: "site-1" }),
				feature({}),
				feature({}, { id: "site-1" }),
				feature({}, { id: 7 }),
				feature({}, { id: "feature-2" }),
				feature({}, { id: "" }),
			]),
			OPTIONS,
		);

		expect(plan.rows.map((row) => row.feature_id)).toEqual([
			"site-1",
			"feature-2-2",
			"site-1-2",
			"7",
			"feature-2",
			"feature-6",
		]);
		expect(plan.warnings.join("\n")).toContain("2 of 6 features had no id");
		expect(plan.warnings.join("\n")).toContain("1 duplicate feature id(s)");
	});

	test("passes a polygon with a hole through as two rings", () => {
		const plan = planGeoJsonImport(
			collection([
				feature({ name: "courtyard" }, { geometry: polygonWithHole }),
			]),
			OPTIONS,
		);

		expect(plan.skipped).toEqual([]);
		const geometry = plan.rows[0].geometry as {
			type: string;
			coordinates: number[][][];
		};
		expect(geometry.type).toBe("Polygon");
		expect(geometry.coordinates).toHaveLength(2);
		expect(geometry.coordinates[1]).toHaveLength(5);
	});

	test("drops Z/M ordinates and 3D bboxes with a warning", () => {
		const plan = planGeoJsonImport(
			collection([
				feature(
					{},
					{
						geometry: {
							type: "LineString",
							bbox: [13, 52, 10, 14, 53, 20],
							coordinates: [
								[13, 52, 10],
								[14, 53, 20, 1],
							],
						},
					},
				),
				feature({}),
			]),
			OPTIONS,
		);

		expect(plan.skipped).toEqual([]);
		expect(plan.rows[0].geometry).toEqual({
			type: "LineString",
			coordinates: [
				[13, 52],
				[14, 53],
			],
		});
		expect(plan.warnings[0]).toBe(
			"Dropped Z/M ordinates from 1 feature geometry; coordinates are stored as 2D longitude/latitude.",
		);
	});

	test("rejects a non-WGS84 crs and accepts CRS84 declarations", () => {
		expect(() =>
			planGeoJsonImport(
				collection([feature({})], {
					crs: { type: "name", properties: { name: "EPSG:3857" } },
				}),
				OPTIONS,
			),
		).toThrow("declares crs 'EPSG:3857'");
		expect(() =>
			planGeoJsonImport(
				collection([feature({})], {
					crs: { type: "EPSG", properties: { code: 25832 } },
				}),
				OPTIONS,
			),
		).toThrow("declares crs 'EPSG:25832'");
		expect(
			planGeoJsonImport(
				collection([feature({})], {
					crs: { type: "EPSG", properties: { code: 4326 } },
				}),
				OPTIONS,
			).rows,
		).toHaveLength(1);

		const plan = planGeoJsonImport(
			collection([feature({})], {
				crs: {
					type: "name",
					properties: { name: "urn:ogc:def:crs:OGC:1.3:CRS84" },
				},
			}),
			OPTIONS,
		);
		expect(plan.rows).toHaveLength(1);
	});

	test("suffixes properties that collide with the key, the geometry or each other", () => {
		const plan = planGeoJsonImport(
			collection([
				feature({
					geometry: "raw",
					feature_id: "legacy",
					siteName: "a",
					site_name: "b",
				}),
			]),
			OPTIONS,
		);

		expect(plan.columns.map((column) => column.name)).toEqual([
			"feature_id",
			"geometry",
			"geometry_2",
			"feature_id_2",
			"site_name",
			"site_name_2",
		]);
		expect(plan.rows[0]).toMatchObject({
			geometry_2: "raw",
			feature_id_2: "legacy",
			site_name: "a",
			site_name_2: "b",
		});
	});

	test("refuses key and geometry columns that are not valid or distinct", () => {
		expect(() =>
			planGeoJsonImport(collection([]), { ...OPTIONS, keyColumn: "_rowid" }),
		).toThrow("key_column '_rowid'");
		expect(() =>
			planGeoJsonImport(collection([]), {
				geometryColumn: "shape",
				keyColumn: "Shape",
			}),
		).toThrow("must differ");
	});

	test("skips invalid features with a reason and keeps null geometries", () => {
		const plan = planGeoJsonImport(
			collection([
				feature(
					{ n: 1 },
					{ geometry: { type: "Point", coordinates: [200, 10] } },
				),
				{ type: "Point", coordinates: [1, 2] },
				feature({ n: 3 }, { geometry: null }),
				feature(["not", "an", "object"] as unknown as Record<string, unknown>),
				feature({ n: 5 }, { geometry: { type: "Feature" } }),
			]),
			OPTIONS,
		);

		expect(plan.featureCount).toBe(5);
		expect(plan.rows).toHaveLength(1);
		expect(plan.rows[0]).toMatchObject({ geometry: null, n: 3 });
		expect(plan.rowFeatureIndexes).toEqual([2]);
		expect(plan.skipped.map((skip) => skip.feature_index)).toEqual([
			0, 1, 3, 4,
		]);
		expect(plan.skipped[0].reason).toContain("outside WGS 84");
		expect(plan.skipped[1].reason).toContain("expected a GeoJSON Feature");
		expect(plan.skipped[2].reason).toContain("properties must be an object");
		expect(plan.skipped[3].reason).toContain(
			"geometry must be a GeoJSON geometry",
		);
	});

	test("accepts a bare Feature and an array of Features", () => {
		const single = planGeoJsonImport(
			feature({ a: 1 }, { id: "only" }),
			OPTIONS,
		);
		expect(single.rows).toEqual([
			{
				feature_id: "only",
				geometry: { type: "Point", coordinates: [13.4, 52.5] },
				a: 1,
			},
		]);

		const list = planGeoJsonImport(
			[feature({ a: 1 }), feature({ b: true })],
			OPTIONS,
		);
		expect(list.featureCount).toBe(2);
		expect(columnTypes(list)).toMatchObject({ a: "int64", b: "boolean" });
	});

	test("refuses input that is not a FeatureCollection, Feature or Feature list", () => {
		expect(() =>
			planGeoJsonImport({ type: "Polygon", coordinates: [] }, OPTIONS),
		).toThrow("contains a 'Polygon' object");
	});
});

describe("GeoJSON import helpers", () => {
	test("geoJsonPropertyColumnName never yields an invalid column name", () => {
		for (const property of ["", "::", "ÄÖÜ", "9", "a".repeat(300)]) {
			const name = geoJsonPropertyColumnName(property);
			expect(name).toMatch(/^[A-Za-z_][A-Za-z0-9_]*$/);
			expect(name.length).toBeLessThanOrEqual(128);
		}
	});

	test("isRfc3339DateTime requires a valid calendar instant with an offset", () => {
		expect(isRfc3339DateTime("2024-02-29T12:00:00Z")).toBe(true);
		expect(isRfc3339DateTime("2024-01-01T00:00:00.5-05:30")).toBe(true);
		expect(isRfc3339DateTime("2023-02-29T12:00:00Z")).toBe(false);
		expect(isRfc3339DateTime("2024-01-01T00:00:00")).toBe(false);
		expect(isRfc3339DateTime("2024-01-01 00:00:00Z")).toBe(false);
		expect(isRfc3339DateTime("2024-01-01")).toBe(false);
	});

	test("batchGeoJsonRows bounds batches by row count and JSON bytes", () => {
		const rows = Array.from({ length: 600 }, (_, index) => ({ index }));
		expect(batchGeoJsonRows(rows).map((batch) => batch.rows.length)).toEqual([
			250, 250, 100,
		]);
		expect(batchGeoJsonRows(rows).map((batch) => batch.start)).toEqual([
			0, 250, 500,
		]);

		const large = Array.from({ length: 5 }, () => ({
			text: "x".repeat(400_000),
		}));
		const batches = batchGeoJsonRows(large);
		expect(batches.map((batch) => batch.rows.length)).toEqual([2, 2, 1]);
		for (const batch of batches) {
			expect(JSON.stringify(batch.rows).length).toBeLessThanOrEqual(1_000_000);
		}

		const oversized = [{ text: "x".repeat(1_200_000) }, { text: "y" }];
		expect(
			batchGeoJsonRows(oversized).map((batch) => batch.rows.length),
		).toEqual([1, 1]);
	});
});

describe("importGeoJsonRuntime", () => {
	const fileName = "sites.geojson";
	const attachment = {
		name: fileName,
		type: "application/geo+json",
		size: 1_024,
		url: "https://files.example/tmp/abc123.geojson?sig=1",
	};
	const geojson = collection([
		feature({ physicalSiteId: "P1" }, { id: "a", geometry: polygonWithHole }),
		feature({ physicalSiteId: "P2" }),
		feature(
			{ physicalSiteId: "P3" },
			{ geometry: { type: "Point", coordinates: [500, 0] } },
		),
	]);
	const fetchText = (text: string) => async () => new Response(text);

	function database(options: { schema?: unknown; failInsertAt?: number } = {}) {
		const created: { tableName: string; fields: IDatabaseSchemaField[] }[] = [];
		const inserted: unknown[][] = [];
		return {
			created,
			inserted,
			dbState: {
				getSchema: async () => {
					if (options.schema === undefined) throw new Error("Table not found");
					return options.schema;
				},
				createTable: async (
					_appId: string,
					tableName: string,
					fields: IDatabaseSchemaField[],
				) => {
					created.push({ tableName, fields });
					return { table_name: tableName, created: true, if_not_exists: true };
				},
				addItems: async (
					_appId: string,
					_tableName: string,
					rows: unknown[],
				) => {
					if (inserted.length === options.failInsertAt)
						throw new Error("Utf8Builder does not support serialize_seq");
					inserted.push(rows);
				},
			},
		};
	}

	const options = {
		appId: "app-geo",
		tableName: "Site Polygons",
		fileName: "SITES.geojson",
		geometryColumn: "geometry",
		keyColumn: "feature_id",
		userScoped: false,
		attachments: [attachment],
	};

	test("creates the table from the plan and inserts one row per valid feature", async () => {
		const db = database();
		const result = await importGeoJsonRuntime(
			db.dbState,
			options,
			fetchText(JSON.stringify(geojson)),
		);

		expect(db.created[0].tableName).toBe("site_polygons");
		expect(db.created[0].fields.map((field) => field.name)).toEqual([
			"feature_id",
			"geometry",
			"physical_site_id",
		]);
		expect(db.inserted.flat()).toHaveLength(2);
		expect(result).toMatchObject({
			status: "ok",
			table_name: "site_polygons",
			requested_table_name: "Site Polygons",
			created: true,
			file_name: fileName,
			features: 3,
			rows_inserted: 2,
			key_column: "feature_id",
			geometry_column: "geometry",
			skipped: [{ feature_index: 2 }],
		});
	});

	test("reports a partial import naming the first feature of the failed batch", async () => {
		const db = database({ failInsertAt: 0 });
		const result = await importGeoJsonRuntime(
			db.dbState,
			options,
			fetchText(JSON.stringify(geojson)),
		);

		expect(result).toMatchObject({
			status: "partial",
			rows_inserted: 0,
			failed_feature_index: 0,
			error: "Utf8Builder does not support serialize_seq",
		});
	});

	test("checks an existing table for the planned columns and a geometry column", async () => {
		const field = (name: string, dataType: unknown, metadata = {}) => ({
			name,
			data_type: dataType,
			nullable: true,
			metadata,
		});
		const missing = database({
			schema: {
				fields: [field("feature_id", "Utf8"), field("geometry", "Binary")],
			},
		});
		await expect(
			importGeoJsonRuntime(
				missing.dbState,
				options,
				fetchText(JSON.stringify(geojson)),
			),
		).rejects.toThrow("physical_site_id (string)");

		const notGeometry = database({
			schema: {
				fields: [
					field("feature_id", "Utf8"),
					field("geometry", "Binary"),
					field("physical_site_id", "Utf8"),
				],
			},
		});
		await expect(
			importGeoJsonRuntime(
				notGeometry.dbState,
				options,
				fetchText(JSON.stringify(geojson)),
			),
		).rejects.toThrow("is binary, not a geometry column");

		const ready = database({
			schema: {
				fields: [
					field("feature_id", "Utf8"),
					field("geometry", "Binary", {
						"ARROW:extension:name": "geoarrow.wkb",
					}),
					field("physical_site_id", "Utf8"),
				],
			},
		});
		const result = await importGeoJsonRuntime(
			ready.dbState,
			options,
			fetchText(JSON.stringify(geojson)),
		);
		expect(ready.created).toEqual([]);
		expect(result).toMatchObject({
			status: "ok",
			created: false,
			rows_inserted: 2,
		});
	});

	test("reads only files forwarded to the run", async () => {
		const db = database();
		await expect(
			importGeoJsonRuntime(
				db.dbState,
				{ ...options, attachments: [] },
				fetchText("{}"),
			),
		).rejects.toThrow("data_studio_agent forward_files");
		await expect(
			importGeoJsonRuntime(
				db.dbState,
				{ ...options, fileName: "other.geojson" },
				fetchText("{}"),
			),
		).rejects.toThrow("forwarded files: sites.geojson");
	});

	test("explains unreadable, invalid and oversized files", async () => {
		const db = database();
		await expect(
			importGeoJsonRuntime(db.dbState, options, fetchText("{not json")),
		).rejects.toThrow("could not parse 'sites.geojson' as JSON");
		await expect(
			importGeoJsonRuntime(
				db.dbState,
				options,
				async () =>
					new Response("gone", { status: 404, statusText: "Not Found" }),
			),
		).rejects.toThrow("HTTP 404 Not Found");
		await expect(
			importGeoJsonRuntime(
				db.dbState,
				{
					...options,
					attachments: [{ ...attachment, size: 60 * 1024 * 1024 }],
				},
				fetchText("{}"),
			),
		).rejects.toThrow("above the 50 MB import limit");
		await expect(
			importGeoJsonRuntime(
				db.dbState,
				options,
				fetchText(JSON.stringify(collection([]))),
			),
		).rejects.toThrow("no importable feature");
	});
});
