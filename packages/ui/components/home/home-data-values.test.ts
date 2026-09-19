import { describe, expect, test } from "bun:test";
import type { ExecuteSqlResult } from "../../state/backend-state/query-state";
import { formatNumber } from "../settings/data-studio/query-workbench/column-types";
import {
	type HomeDataFormatConfig,
	homeDataColumnKind,
	homeDataDateLabel,
	homeDataLabelFormat,
	homeDataTemporalValue,
	homeDataValueLabel,
} from "./home-data-values";

type Result = Pick<ExecuteSqlResult, "columns" | "rows">;

const APP = "app-1";
const SUB = "42c52474-5081-70d7-2b23-4bd8c38d8fb0";
const OTHER_SUB = "9f0c1d2e-3b4a-4c5d-8e6f-7a8b9c0d1e2f";

/** 2026-09-16T00:00:00Z as DataFusion ships it in each unit. */
const SEPT_16 = new Date(Date.UTC(2026, 8, 16));
const SEPT_16_NS = 1_789_516_800_000_000_000;
const SEPT_16_US = 1_789_516_800_000_000;
const SEPT_16_MS = 1_789_516_800_000;
const SEPT_16_DAYS = 20_712;
/** The raw epoch millis the record widgets used to print. */
const STARTED_AT_MS = 1_789_584_351_307;
const LAST_SUCCESS_AT_MS = 1_789_584_356_974;

const THIS_YEAR = new Date().getUTCFullYear();

const result = (
	type_name: string,
	values: unknown[],
	name = "__group",
): Result => ({
	columns: [{ name, type_name, position: 0 }],
	rows: values.map((value) => ({ [name]: value })),
});

const config = (
	overrides: Partial<HomeDataFormatConfig> = {},
): HomeDataFormatConfig => ({
	appId: APP,
	timeBucket: "none",
	visualization: "bar",
	groupBy: "",
	seriesBy: "",
	measures: [],
	...overrides,
});

const shortDay = (date: Date, withYear: boolean) =>
	date.toLocaleDateString(undefined, {
		month: "short",
		day: "numeric",
		...(withYear ? { year: "numeric" as const } : {}),
		timeZone: "UTC",
	});

const dateTime = (date: Date, withYear: boolean) =>
	date.toLocaleString(undefined, {
		month: "short",
		day: "numeric",
		...(withYear ? { year: "numeric" as const } : {}),
		hour: "2-digit",
		minute: "2-digit",
	});

describe("homeDataTemporalValue", () => {
	test("reads each Arrow unit DataFusion ships as the same instant", () => {
		const iso = SEPT_16.toISOString();
		expect(
			homeDataTemporalValue(SEPT_16_NS, "Timestamp(ns)")?.toISOString(),
		).toBe(iso);
		expect(
			homeDataTemporalValue(SEPT_16_US, "Timestamp(µs)")?.toISOString(),
		).toBe(iso);
		expect(
			homeDataTemporalValue(SEPT_16_MS, 'Timestamp(ms, "UTC")')?.toISOString(),
		).toBe(iso);
		expect(homeDataTemporalValue(SEPT_16_DAYS, "Date32")?.toISOString()).toBe(
			iso,
		);
	});

	test("trusts a declared unit over the magnitude", () => {
		expect(homeDataTemporalValue(86_400, "Timestamp(s)")?.toISOString()).toBe(
			"1970-01-02T00:00:00.000Z",
		);
	});

	test("reads numeric strings and bigints in the declared unit", () => {
		const iso = SEPT_16.toISOString();
		expect(
			homeDataTemporalValue(
				"1789516800000000000",
				"Timestamp(ns)",
			)?.toISOString(),
		).toBe(iso);
		expect(
			homeDataTemporalValue(
				1_789_516_800_000_000n,
				"Timestamp(µs)",
			)?.toISOString(),
		).toBe(iso);
	});

	test("believes an undeclared number only once it is large enough to be an epoch", () => {
		expect(homeDataTemporalValue(STARTED_AT_MS, "Int64")?.toISOString()).toBe(
			"2026-09-16T18:45:51.307Z",
		);
		expect(
			homeDataTemporalValue(String(STARTED_AT_MS), "Int64")?.toISOString(),
		).toBe("2026-09-16T18:45:51.307Z");
		for (const value of [0, 42, 12_000, 20_712])
			expect(homeDataTemporalValue(value, "Int64"), String(value)).toBeNull();
	});

	test("parses text and rejects missing values", () => {
		expect(
			homeDataTemporalValue("2026-09-16T00:00:00Z", "Utf8")?.toISOString(),
		).toBe(SEPT_16.toISOString());
		expect(homeDataTemporalValue(null, "Timestamp(ns)")).toBeNull();
		expect(homeDataTemporalValue(undefined)).toBeNull();
	});
});

describe("homeDataColumnKind", () => {
	test("judges an aliased group under the column it came from", () => {
		const grouped = result("Int64", [STARTED_AT_MS, LAST_SUCCESS_AT_MS]);
		expect(homeDataColumnKind(grouped, "__group", "started_at")).toBe(
			"temporal",
		);
		expect(homeDataColumnKind(grouped, "__group", "count")).toBe("number");
	});

	test("demotes instant-named integers that hold counters or durations", () => {
		expect(
			homeDataColumnKind(result("Int64", [3, 42]), "__group", "started_at"),
		).toBe("number");
		expect(
			homeDataColumnKind(
				result("Int64", [12_000, 250], "response_time"),
				"response_time",
			),
		).toBe("number");
	});

	test("classifies under the result name when no source is given", () => {
		const records = result("Int64", [LAST_SUCCESS_AT_MS], "last_success_at");
		expect(homeDataColumnKind(records, "last_success_at")).toBe("temporal");
		expect(homeDataColumnKind(records, "last_success_at", "")).toBe("temporal");
	});

	test("finds people and files through the source name and values", () => {
		const people = result("Utf8", ["system", SUB], "__series");
		expect(homeDataColumnKind(people, "__series", "created_by")).toBe("user");
		expect(homeDataColumnKind(people, "__series", "status")).toBe("text");

		const files = result("Utf8", [`apps/${APP}/upload/reports/summary.pdf`]);
		expect(homeDataColumnKind(files, "__group", "attachment", APP)).toBe(
			"file",
		);
		expect(homeDataColumnKind(files, "__group", "attachment", "app-2")).toBe(
			"text",
		);
	});

	test("reads a column the result does not have as text", () => {
		expect(homeDataColumnKind(result("Int64", [1]), "missing")).toBe("text");
	});
});

describe("homeDataLabelFormat and homeDataValueLabel", () => {
	const nanosGroup = result("Timestamp(ns)", [SEPT_16_NS]);
	const shows2026 = THIS_YEAR !== 2026;

	test("labels a nanosecond DATE_TRUNC group at its bucket's precision", () => {
		const label = (timeBucket: HomeDataFormatConfig["timeBucket"]) => {
			const format = homeDataLabelFormat(
				nanosGroup,
				"__group",
				config({ timeBucket, visualization: "area", groupBy: "started_at" }),
			);
			expect(format.kind).toBe("temporal");
			expect(format.sourceName).toBe("started_at");
			expect(format.bucket).toBe(timeBucket);
			expect(format.dateOnly).toBe(true);
			return homeDataValueLabel(SEPT_16_NS, format);
		};

		expect(label("day")).toBe(shortDay(SEPT_16, shows2026));
		expect(label("week")).toBe(shortDay(SEPT_16, shows2026));
		expect(label("month")).toBe(
			SEPT_16.toLocaleDateString(undefined, {
				month: "short",
				year: "numeric",
				timeZone: "UTC",
			}),
		);
		expect(label("quarter")).toBe("Q3 2026");
		expect(label("year")).toBe(
			SEPT_16.toLocaleDateString(undefined, {
				year: "numeric",
				timeZone: "UTC",
			}),
		);
		expect(label("day")).not.toContain("1789516800");
	});

	test("labels microsecond, millisecond and Date32 groups the same way", () => {
		const expected = shortDay(SEPT_16, shows2026);
		for (const [typeName, value] of [
			["Timestamp(µs)", SEPT_16_US],
			['Timestamp(ms, "UTC")', SEPT_16_MS],
			["Date32", SEPT_16_DAYS],
		] as const) {
			const format = homeDataLabelFormat(
				result(typeName, [value]),
				"__group",
				config({
					timeBucket: "day",
					visualization: "line",
					groupBy: "first_seen_at",
				}),
			);
			expect(homeDataValueLabel(value, format), typeName).toBe(expected);
		}
	});

	test("reads an undeclared epoch-millis record column as a date and time", () => {
		const format = homeDataLabelFormat(
			result("Int64", [STARTED_AT_MS], "started_at"),
			"started_at",
			config({ visualization: "record" }),
		);
		expect(format.kind).toBe("temporal");
		expect(format.bucket).toBe("none");
		expect(format.dateOnly).toBe(false);
		expect(homeDataValueLabel(STARTED_AT_MS, format)).toBe(
			dateTime(new Date(STARTED_AT_MS), shows2026),
		);
	});

	test("reads a min or max measure in its field's kind, every other aggregate as a number", () => {
		const measures = [
			{ aggregation: "max", field: "started_at", label: "" },
			{ aggregation: "count", field: "", label: "" },
			{ aggregation: "min", field: "owner_sub", label: "" },
		] as HomeDataFormatConfig["measures"];
		const aggregated: Result = {
			columns: [
				{ name: "__measure_0", type_name: "Int64", position: 0 },
				{ name: "__measure_1", type_name: "Int64", position: 1 },
				{ name: "__measure_2", type_name: "Utf8", position: 2 },
			],
			rows: [{ __measure_0: STARTED_AT_MS, __measure_1: 12, __measure_2: SUB }],
		};
		const formatOf = (name: string, visualization = "stat") =>
			homeDataLabelFormat(
				aggregated,
				name,
				config({
					measures,
					visualization: visualization as HomeDataFormatConfig["visualization"],
				}),
			);
		expect(formatOf("__measure_0").kind).toBe("temporal");
		expect(formatOf("__measure_0").sourceName).toBe("started_at");
		expect(formatOf("__measure_1").kind).toBe("number");
		expect(formatOf("__measure_1").measure).toBe(true);
		expect(formatOf("__measure_2").kind).toBe("user");
		expect(formatOf("__measure_0", "boxplot").kind).toBe("number");
		expect(
			homeDataLabelFormat(
				result("Float64", [0.4], "__share"),
				"__share",
				config({ measures }),
			).kind,
		).toBe("number");
	});

	test("shows the year only when the dates leave the current year", () => {
		const day = config({ timeBucket: "day", groupBy: "created_at" });
		const yearsOf = (values: number[]) =>
			homeDataLabelFormat(
				result('Timestamp(ms, "UTC")', values),
				"__group",
				day,
			).showYear;

		expect(
			yearsOf([Date.UTC(THIS_YEAR, 0, 1), Date.UTC(THIS_YEAR, 0, 2)]),
		).toBe(false);
		expect(
			yearsOf([Date.UTC(THIS_YEAR - 1, 11, 31), Date.UTC(THIS_YEAR, 0, 1)]),
		).toBe(true);
		expect(yearsOf([Date.UTC(2020, 5, 1)])).toBe(THIS_YEAR !== 2020);

		const spanning = homeDataLabelFormat(
			result('Timestamp(ms, "UTC")', [
				Date.UTC(THIS_YEAR - 1, 11, 31),
				Date.UTC(THIS_YEAR, 0, 1),
			]),
			"__group",
			day,
		);
		const newYearsEve = new Date(Date.UTC(THIS_YEAR - 1, 11, 31));
		expect(homeDataValueLabel(newYearsEve.getTime(), spanning)).toBe(
			shortDay(newYearsEve, true),
		);
	});

	test("drops the time when every instant is a whole UTC day", () => {
		const midnights = [Date.UTC(THIS_YEAR, 0, 1), Date.UTC(THIS_YEAR, 0, 2)];
		const allMidnight = homeDataLabelFormat(
			result(
				"Timestamp(µs)",
				midnights.map((millis) => millis * 1000),
				"last_updated",
			),
			"last_updated",
			config({ visualization: "table" }),
		);
		expect(allMidnight.dateOnly).toBe(true);
		expect(homeDataValueLabel(midnights[1] * 1000, allMidnight)).toBe(
			shortDay(new Date(midnights[1]), false),
		);

		const withTime = Date.UTC(THIS_YEAR, 0, 2, 18, 45);
		const mixed = homeDataLabelFormat(
			result(
				"Timestamp(µs)",
				[midnights[0] * 1000, withTime * 1000],
				"last_updated",
			),
			"last_updated",
			config({ visualization: "table" }),
		);
		expect(mixed.dateOnly).toBe(false);
		expect(homeDataValueLabel(withTime * 1000, mixed)).toBe(
			dateTime(new Date(withTime), false),
		);
	});

	test("treats a Date32 column as whole days even without a bucket", () => {
		const format = homeDataLabelFormat(
			result("Date32", [SEPT_16_DAYS], "signup_date"),
			"signup_date",
			config({ visualization: "table" }),
		);
		expect(format.kind).toBe("temporal");
		expect(format.dateOnly).toBe(true);
	});

	test("resolves account ids to display names and falls back to the id", () => {
		const userNames = new Map([[SUB, "Ada Lovelace"]]);
		const format = homeDataLabelFormat(
			result("Utf8", [SUB, OTHER_SUB, "system"], "__series"),
			"__series",
			config({ timeBucket: "month", seriesBy: "created_by" }),
			userNames,
		);
		expect(format.kind).toBe("user");
		expect(format.bucket).toBe("none");
		expect(homeDataValueLabel(SUB, format)).toBe("Ada Lovelace");
		expect(homeDataValueLabel(` ${SUB} `, format)).toBe("Ada Lovelace");
		expect(homeDataValueLabel(OTHER_SUB, format)).toBe(OTHER_SUB);
		expect(homeDataValueLabel("system", format)).toBe("system");

		const unresolved = homeDataLabelFormat(
			result("Utf8", [SUB], "owner_id"),
			"owner_id",
			config({ visualization: "table" }),
		);
		expect(unresolved.kind).toBe("user");
		expect(homeDataValueLabel(SUB, unresolved)).toBe(SUB);
	});

	test("labels a stored upload by its file name", () => {
		const path = `apps/${APP}/upload/reports/q3%20summary.pdf`;
		const format = homeDataLabelFormat(
			result("Utf8", [path]),
			"__group",
			config({ groupBy: "attachment" }),
		);
		expect(format.kind).toBe("file");
		expect(homeDataValueLabel(path, format)).toBe("q3 summary.pdf");
		expect(homeDataValueLabel("notes without a file", format)).toBe(
			"notes without a file",
		);
	});

	test("carries the column's geometry metadata", () => {
		const metadata = { "ARROW:extension:name": "geoarrow.wkb" };
		const format = homeDataLabelFormat(
			{
				columns: [
					{ name: "shape", type_name: "Binary", position: 0, metadata },
				],
				rows: [],
			},
			"shape",
			config(),
		);
		expect(format.kind).toBe("geometry");
		expect(format.metadata).toEqual(metadata);
	});

	test("labels missing values the same for every kind", () => {
		const temporal = homeDataLabelFormat(
			nanosGroup,
			"__group",
			config({ timeBucket: "day", groupBy: "started_at" }),
		);
		const number = homeDataLabelFormat(
			result("Int64", [1], "value"),
			"value",
			config(),
		);
		for (const format of [temporal, number])
			for (const value of [null, undefined])
				expect(homeDataValueLabel(value, format)).toBe("No value");
	});

	test("reads histogram bins as numbers even with a stale time bucket", () => {
		const format = homeDataLabelFormat(
			result("Float64", [0, 25, 1250.5]),
			"__group",
			config({
				visualization: "histogram",
				timeBucket: "month",
				groupBy: "amount",
			}),
		);
		expect(format.kind).toBe("number");
		expect(format.bucket).toBe("none");
		expect(format.dateOnly).toBe(false);
		expect(homeDataValueLabel(1250.5, format)).toBe(formatNumber(1250.5));
	});

	test("keeps whole numbers as written, since they are years and ids as often as amounts", () => {
		const format = homeDataLabelFormat(
			result("Int64", [2024, 2025], "__group"),
			"__group",
			config({ groupBy: "year" }),
		);
		expect(format.kind).toBe("number");
		expect(format.measure).toBe(false);
		expect(homeDataValueLabel(2024, format)).toBe("2024");
		expect(homeDataValueLabel(12.5, format)).toBe(formatNumber(12.5));
	});

	test("never reads a time of day as an instant", () => {
		const format = homeDataLabelFormat(
			result("Time64(ns)", [36_000_000_000_000], "opens_at"),
			"opens_at",
			config(),
		);
		expect(format.kind).toBe("text");
		expect(homeDataValueLabel(36_000_000_000_000, format)).toBe(
			"36000000000000",
		);
	});
});

describe("homeDataDateLabel", () => {
	test("puts quarter boundaries in the right quarter", () => {
		const quarter = (date: Date) =>
			homeDataDateLabel(date, {
				bucket: "quarter",
				showYear: false,
				dateOnly: true,
			});
		expect(quarter(new Date(Date.UTC(2026, 0, 1)))).toBe("Q1 2026");
		expect(quarter(new Date(Date.UTC(2026, 2, 31, 23, 59)))).toBe("Q1 2026");
		expect(quarter(new Date(Date.UTC(2026, 3, 1)))).toBe("Q2 2026");
		expect(quarter(new Date(Date.UTC(2026, 11, 31)))).toBe("Q4 2026");
	});

	test("reads bucket starts in UTC so the calendar day never shifts", () => {
		const midnight = new Date(Date.UTC(2026, 0, 1));
		expect(
			homeDataDateLabel(midnight, {
				bucket: "month",
				showYear: false,
				dateOnly: true,
			}),
		).toBe(
			midnight.toLocaleDateString(undefined, {
				month: "short",
				year: "numeric",
				timeZone: "UTC",
			}),
		);
		expect(
			homeDataDateLabel(midnight, {
				bucket: "day",
				showYear: true,
				dateOnly: true,
			}),
		).toBe(shortDay(midnight, true));
	});
});
