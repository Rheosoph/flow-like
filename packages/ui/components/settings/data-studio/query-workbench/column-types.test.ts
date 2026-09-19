import { describe, expect, test } from "bun:test";
import type { QueryColumn } from "../../../../state/backend-state/query-state";
import {
	classifyColumn,
	classifyResultColumn,
	isNumericTypeName,
} from "./column-types";

const SUB = "42c52474-5081-70d7-2b23-4bd8c38d8fb0";

const column = (name: string, type_name = "Utf8"): QueryColumn =>
	({ name, type_name }) as QueryColumn;

describe("classifyColumn", () => {
	test("reads a text column that names a person as a person", () => {
		expect(classifyColumn(column("created_by"))).toBe("user");
		expect(classifyColumn(column("user_sub"))).toBe("user");
		expect(classifyColumn(column("feedback_reporter"))).toBe("user");
	});

	test("leaves the kinds a declared type already settles", () => {
		expect(classifyColumn(column("owner", "Boolean"))).toBe("boolean");
		expect(classifyColumn(column("created_by", "Timestamp"))).toBe("temporal");
		expect(classifyColumn(column("owner_id", "Int64"))).toBe("number");
		expect(classifyColumn(column("owner", "Struct"))).toBe("json");
	});

	test("leaves text that only sounds like a person", () => {
		expect(classifyColumn(column("username"))).toBe("text");
		expect(classifyColumn(column("group_by"))).toBe("text");
		expect(classifyColumn(column("subject"))).toBe("text");
	});
});

describe("classifyResultColumn", () => {
	test("keeps the user kind when a row actually names an account", () => {
		const rows = [{ created_by: "system" }, { created_by: SUB }];
		expect(classifyResultColumn(column("created_by"), rows)).toBe("user");
	});

	test("demotes a person-named column that holds no account at all", () => {
		// The doc fixtures' `owner` column holds team names, not people.
		const rows = [{ owner: "Core Experience" }, { owner: "Trust" }];
		expect(classifyResultColumn(column("owner"), rows)).toBe("text");
	});

	test("trusts the name when there is nothing to sample", () => {
		expect(classifyResultColumn(column("created_by"), [])).toBe("user");
	});

	test("leaves every other kind exactly as classifyColumn had it", () => {
		const rows = [{ score: 1 }];
		expect(classifyResultColumn(column("score", "Float64"), rows)).toBe(
			"number",
		);
		expect(classifyResultColumn(column("username"), rows)).toBe("text");
	});
});

describe("integer instants", () => {
	test("reads durations and times of day as something other than instants", () => {
		expect(classifyColumn(column("elapsed", "Duration(ms)"))).toBe("number");
		expect(classifyColumn(column("opens_at", "Time64(ns)"))).toBe("text");
		expect(classifyColumn(column("opens_at", "Time32(ms)"))).toBe("text");
		expect(
			classifyResultColumn(column("response_time", "Int64"), [
				{ response_time: 12_000 },
				{ response_time: 250 },
			]),
		).toBe("number");
	});

	test("keeps pre-1990 epoch instants temporal", () => {
		expect(
			classifyResultColumn(column("born_at", "Int64"), [
				{ born_at: 489_024_000_000 },
			]),
		).toBe("temporal");
	});

	test("keeps an epoch-millis created_at temporal", () => {
		const rows = [
			{ created_at: null },
			{ created_at: 1_789_584_351_307 },
			{ created_at: 1_789_584_356_974 },
		];
		expect(classifyResultColumn(column("created_at", "Int64"), rows)).toBe(
			"temporal",
		);
	});

	test("demotes a created_at that only holds small counters", () => {
		const rows = [{ created_at: 3 }, { created_at: 42 }];
		expect(classifyResultColumn(column("created_at", "Int64"), rows)).toBe(
			"number",
		);
		expect(
			classifyResultColumn(column("created_at", "Float64"), [
				{ created_at: 0.5 },
			]),
		).toBe("number");
	});

	test("keeps a sparse instant temporal when the sample holds only unset values", () => {
		const unset = [
			...Array.from({ length: 60 }, () => ({ last_success_at: null })),
			...Array.from({ length: 60 }, () => ({ last_success_at: 0 })),
			{ last_success_at: 1_789_584_356_974 },
		];
		expect(
			classifyResultColumn(column("last_success_at", "Int64"), unset),
		).toBe("temporal");
		expect(
			classifyResultColumn(column("deleted_at", "Int64"), [
				{ deleted_at: null },
			]),
		).toBe("temporal");
		expect(
			classifyResultColumn(column("created_at", "Int64"), [
				{ created_at: 0 },
				{ created_at: 7 },
			]),
		).toBe("number");
	});

	test("never demotes a column whose type declares an instant", () => {
		for (const type_name of [
			"Timestamp(µs)",
			'Timestamp(ns, "UTC")',
			"Date32",
		]) {
			expect(
				classifyResultColumn(column("created_at", type_name), [
					{ created_at: 3 },
					{ created_at: 42 },
				]),
				type_name,
			).toBe("temporal");
			expect(
				classifyResultColumn(column("bucket", type_name), [{ bucket: 20_712 }]),
				type_name,
			).toBe("temporal");
		}
	});
});

describe("isNumericTypeName", () => {
	test("accepts integer, float and decimal Arrow types", () => {
		for (const type_name of [
			"Int64",
			"UInt32",
			"Float64",
			"Decimal128(38, 10)",
		])
			expect(isNumericTypeName(type_name), type_name).toBe(true);
	});

	test("rejects instants, durations, intervals and non-numbers", () => {
		for (const type_name of [
			"Timestamp(µs)",
			'Timestamp(ns, "UTC")',
			"Date32",
			"Date64",
			"Duration(ms)",
			"Interval(MonthDayNano)",
			"Boolean",
			"Utf8",
		])
			expect(isNumericTypeName(type_name), type_name).toBe(false);
	});
});

describe("declared geometry columns", () => {
	test("uses extension metadata before physical Arrow type classification", () => {
		for (const type_name of ["Binary", "Struct", "FixedSizeList(Float64)"])
			expect(
				classifyColumn({
					...column("shape", type_name),
					metadata: { "ARROW:extension:name": "geoarrow.wkb" },
				}),
			).toBe("geometry");
	});
	test("never infers geometry from objects or arbitrary binary", () => {
		const rows = [{ shape: { type: "Point", coordinates: [1, 2] } }];
		expect(classifyResultColumn(column("shape", "Struct"), rows)).toBe("json");
		expect(classifyResultColumn(column("shape", "Binary"), rows)).toBe("text");
	});
});
