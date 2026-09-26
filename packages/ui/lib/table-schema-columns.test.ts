import { describe, expect, test } from "bun:test";

import {
	assertInsertKeysAreColumns,
	databaseQueryPage,
} from "../hooks/use-frontend-runtime-tool-executor";
import {
	assertRowKeysAreColumns,
	summarizeTableColumns,
} from "./table-schema-columns";

const field = (
	name: string,
	dataType: unknown,
	metadata: Record<string, string> = {},
	nullable = true,
) => ({
	name,
	data_type: dataType,
	nullable,
	dict_id: 0,
	dict_is_ordered: false,
	metadata,
});

describe("summarizeTableColumns", () => {
	test("maps serde-serialized Arrow fields to create_table types", () => {
		const schema = {
			fields: [
				field(
					"feature_id",
					"Utf8",
					{ "lance-schema:unenforced-primary-key:position": "1" },
					false,
				),
				field("geometry", "Binary", {
					"ARROW:extension:name": "geoarrow.wkb",
					"ARROW:extension:metadata": '{"crs":"EPSG:4326"}',
				}),
				field("title", "LargeUtf8"),
				field("active", "Boolean"),
				field("area", "Int64"),
				field("ratio", "Float32"),
				field("count", "UInt16"),
				field("raw", "Binary"),
				field("day", "Date32"),
				field("opened", { Timestamp: ["Millisecond", "UTC"] }),
				field("seen", { Timestamp: ["Microsecond", null] }),
				field("embedding", {
					FixedSizeList: [field("item", "Float32", {}, false), 384],
				}),
				field("tags", { List: field("item", "Utf8") }),
				field("meta", {
					Struct: [
						field("a", "Int32"),
						field("b", { List: field("item", "Utf8") }),
					],
				}),
				field("legacy_key", "Int64", {
					"lance-schema:unenforced-primary-key": "true",
				}),
			],
			metadata: {},
		};

		expect(summarizeTableColumns(schema)).toEqual([
			{
				name: "feature_id",
				type: "string",
				nullable: false,
				primary_key: true,
			},
			{ name: "geometry", type: "geometry", nullable: true },
			{ name: "title", type: "string", nullable: true },
			{ name: "active", type: "boolean", nullable: true },
			{ name: "area", type: "int64", nullable: true },
			{ name: "ratio", type: "float32", nullable: true },
			{ name: "count", type: "uint16", nullable: true },
			{ name: "raw", type: "binary", nullable: true },
			{ name: "day", type: "date32", nullable: true },
			{ name: "opened", type: "timestamp:ms:UTC", nullable: true },
			{ name: "seen", type: "Timestamp(Microsecond, None)", nullable: true },
			{ name: "embedding", type: "vector", vector_size: 384, nullable: true },
			{ name: "tags", type: "List(Utf8)", nullable: true },
			{ name: "meta", type: "Struct(a: Int32, b: List(Utf8))", nullable: true },
			{ name: "legacy_key", type: "int64", nullable: true, primary_key: true },
		]);
	});

	test("answers no columns for anything that is not a schema", () => {
		expect(summarizeTableColumns(null)).toEqual([]);
		expect(summarizeTableColumns({ fields: "nope" })).toEqual([]);
	});
});

describe("assertRowKeysAreColumns", () => {
	test("accepts rows whose keys are all columns", () => {
		expect(() =>
			assertRowKeysAreColumns("sites", [{ a: 1 }, { b: 2 }, "x"], ["a", "b"]),
		).not.toThrow();
	});

	test("names every unknown key and the table's columns", () => {
		expect(() =>
			assertRowKeysAreColumns(
				"sites",
				[{ a: 1, physicalSiteId: "P1" }, { osm_tags: {} }],
				["a", "physical_site_id"],
			),
		).toThrow(
			"Table 'sites' has no column 'physicalSiteId', 'osm_tags'; its columns are a, physical_site_id. Add them with add_column or rename the keys.",
		);
	});
});

describe("database_tool executor hardening", () => {
	const insertTarget = {
		appId: "app-1",
		tableName: "sites",
		userScoped: false,
		rows: [{ name: "a", physicalSiteId: "P1" }],
	};

	test("rejects insert keys an existing table lacks", async () => {
		const dbState = {
			getSchema: async () => ({ fields: [field("name", "Utf8")] }),
		};
		await expect(
			assertInsertKeysAreColumns(dbState, insertTarget),
		).rejects.toThrow("Table 'sites' has no column 'physicalSiteId'");
	});

	test("skips the check when the table does not exist yet", async () => {
		const dbState = {
			getSchema: async () => {
				throw new Error("Table 'sites' was not found");
			},
		};
		await expect(
			assertInsertKeysAreColumns(dbState, insertTarget),
		).resolves.toBeUndefined();
	});

	test("pages SQL results client-side and reports the total", () => {
		const rows = Array.from({ length: 7 }, (_, index) => ({ index }));
		expect(databaseQueryPage(rows, { sql: "SELECT * FROM t" }, 2, 3)).toEqual({
			row_count: 3,
			total_rows: 7,
			truncated: true,
			rows: [{ index: 2 }, { index: 3 }, { index: 4 }],
		});
		expect(
			databaseQueryPage(rows, { sql: "SELECT * FROM t" }, 5, 3),
		).toMatchObject({ row_count: 2, total_rows: 7, truncated: false });
		expect(databaseQueryPage(rows, { filter: "index > 1" }, 2, 3)).toEqual({
			row_count: 7,
			rows,
		});
	});
});
