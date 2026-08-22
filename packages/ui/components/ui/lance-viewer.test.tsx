import { describe, expect, test } from "bun:test";
import type { LanceField } from "./lance-viewer";
import {
	arrowToLanceSchema,
	buildRowIdentityFilter,
	resolveTemporalCell,
	resolveUserCell,
} from "./lance-viewer";

const schema = arrowToLanceSchema({
	fields: [
		{ name: "created_at", data_type: { Timestamp: ["Millisecond", "UTC"] } },
		{ name: "seen_at", data_type: { Timestamp: ["Microsecond", null] } },
		{ name: "day", data_type: "Date32" },
		{ name: "label", data_type: "LargeUtf8" },
		{ name: "score", data_type: "Float64" },
		{ name: "flag", data_type: "Bool" },
		{ name: "note", data_type: "Utf8", nullable: true },
		{ name: "tags", data_type: { LargeList: [{ data_type: "Utf8" }] } },
		{
			name: "vector",
			data_type: { FixedSizeList: [{ data_type: "Float32" }, 3] },
		},
	],
});

describe("arrowToLanceSchema", () => {
	test("maps timestamp and date columns to the date kind with their unit", () => {
		const byName = new Map(schema.fields.map((f) => [f.name, f]));
		expect(byName.get("created_at")).toMatchObject({
			kind: "date",
			temporal: "millisecond",
		});
		expect(byName.get("seen_at")).toMatchObject({
			kind: "date",
			temporal: "microsecond",
		});
		expect(byName.get("day")).toMatchObject({ kind: "date", temporal: "day" });
		expect(byName.get("tags")).toMatchObject({
			kind: "array",
			items: "string",
		});
		expect(byName.get("vector")).toMatchObject({ kind: "vector", dims: 3 });
	});
});

describe("arrow data types the derived serde form spells as objects", () => {
	test("unwraps a list child field, which is not a tuple", () => {
		const [tags] = arrowToLanceSchema({
			fields: [
				{
					name: "tags",
					data_type: { List: { name: "item", data_type: "Utf8" } },
				},
			],
		}).fields;
		expect(tags).toMatchObject({ kind: "array", items: "string" });
	});

	test("reads through a dictionary to the values it encodes", () => {
		const [status] = arrowToLanceSchema({
			fields: [
				{ name: "status", data_type: { Dictionary: ["Int32", "Utf8"] } },
			],
		}).fields;
		expect(status).toMatchObject({ kind: "string" });
	});

	test("keeps counted types numeric rather than unknown", () => {
		const kinds = arrowToLanceSchema({
			fields: [
				{ name: "amount", data_type: { Decimal128: [10, 2] } },
				{ name: "elapsed", data_type: { Duration: "Millisecond" } },
				{ name: "clock", data_type: { Time64: "Microsecond" } },
			],
		}).fields.map((f) => f.kind);
		expect(kinds).toEqual(["number", "number", "number"]);
	});
});

describe("resolveTemporalCell", () => {
	test("declares the unit and storage shape of a timestamp column", () => {
		expect(
			resolveTemporalCell(
				{ name: "created_at", kind: "date", temporal: "microsecond" },
				1_786_869_111_840_123,
			),
		).toEqual({ unit: "microsecond", wire: "number" });
	});

	test("reads an epoch integer whose column was never declared temporal", () => {
		expect(
			resolveTemporalCell(
				{ name: "created_at", kind: "number" },
				1_786_869_111_840,
			),
		).toEqual({ unit: "millisecond", wire: "number" });
		expect(
			resolveTemporalCell(
				{ name: "duration_ms", kind: "number" },
				1_786_869_111_840,
			),
		).toBeNull();
	});

	test("keeps a text column textual", () => {
		expect(
			resolveTemporalCell({ name: "created_at", kind: "string" }, "2026-08-14"),
		).toEqual({ unit: "millisecond", wire: "string" });
		expect(
			resolveTemporalCell({ name: "label", kind: "string" }, "hello"),
		).toBeNull();
	});
});

describe("buildRowIdentityFilter", () => {
	test("prefers an id-like column and quotes it by type", () => {
		expect(
			buildRowIdentityFilter({ id: "a'b", label: "x" }, schema.fields),
		).toBe("`id` = 'a''b'");
		expect(
			buildRowIdentityFilter({ _rowid: 7, label: "x" }, schema.fields),
		).toBe("`_rowid` = 7");
		expect(
			buildRowIdentityFilter(
				{ id: null, _rowid: 3, label: "x" },
				schema.fields,
			),
		).toBe("`_rowid` = 3");
	});

	test("casts serialized temporal values instead of emitting bare integers", () => {
		const filter = buildRowIdentityFilter(
			{
				created_at: 1786869111840,
				seen_at: 1786869111840123,
				day: 20681,
				label: "it's a",
				score: 1.5,
				flag: true,
				note: null,
				tags: ["x"],
				vector: [1, 2, 3],
			},
			schema.fields,
		);
		expect(filter).toBe(
			[
				"`created_at` = CAST(1786869111840 AS TIMESTAMP(3))",
				"`seen_at` = CAST(1786869111840123 AS TIMESTAMP(6))",
				"`day` = CAST(20681 AS DATE)",
				"`label` = 'it''s a'",
				"`score` = 1.5",
				"`flag` = true",
				"`note` IS NULL",
			].join(" AND "),
		);
	});

	test("skips values it cannot render safely and reports when nothing is left", () => {
		expect(
			buildRowIdentityFilter(
				{ created_at: 2 ** 60, tags: ["x"], vector: [1] },
				schema.fields,
			),
		).toBeNull();
		expect(buildRowIdentityFilter({}, schema.fields)).toBeNull();
	});
});

describe("resolveUserCell", () => {
	const field = (name: string, kind: LanceField["kind"] = "string") =>
		({ name, kind }) as LanceField;
	const SUB = "42c52474-5081-70d7-2b23-4bd8c38d8fb0";

	test("reads a sub out of a column that names a person", () => {
		expect(resolveUserCell(field("sub"), SUB)).toBe(SUB);
		expect(resolveUserCell(field("user_sub"), SUB)).toBe(SUB);
		expect(resolveUserCell(field("feedback_reporter"), SUB)).toBe(SUB);
		expect(resolveUserCell(field("created_by"), SUB)).toBe(SUB);
	});

	test("keeps the local placeholder, which resolves to the signed-in user", () => {
		expect(resolveUserCell(field("user_sub"), "local")).toBe("local");
	});

	test("trims, because a stored id can carry padding", () => {
		expect(resolveUserCell(field("owner_id"), ` ${SUB} `)).toBe(SUB);
	});

	test("leaves a column that names something other than a person", () => {
		expect(resolveUserCell(field("app_id"), SUB)).toBeNull();
		expect(resolveUserCell(field("session_id"), SUB)).toBeNull();
		expect(resolveUserCell(field("username"), SUB)).toBeNull();
	});

	test("leaves a value that is not shaped like an account id", () => {
		expect(resolveUserCell(field("created_by"), "system")).toBeNull();
		expect(resolveUserCell(field("owner"), "microsoft")).toBeNull();
		expect(resolveUserCell(field("assigned_to"), "")).toBeNull();
	});

	test("ignores everything that is not text", () => {
		expect(resolveUserCell(field("user_id", "number"), 42)).toBeNull();
		expect(resolveUserCell(field("owner", "object"), { id: SUB })).toBeNull();
		expect(resolveUserCell(field("user_sub"), null)).toBeNull();
	});

	test("never claims a cell the temporal reader already claims", () => {
		const instant = "2026-08-22T10:30:00Z";
		expect(resolveUserCell(field("created_by"), instant)).toBeNull();
		expect(resolveTemporalCell(field("created_at"), instant)).not.toBeNull();
	});
});
