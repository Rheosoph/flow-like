import { describe, expect, test } from "bun:test";
import {
	detectEpochUnit,
	formatAbsoluteDateTime,
	formatCalendarDate,
	formatRelativeTime,
	fromDateInputValue,
	fromDateTimeInputValue,
	inferTemporalValue,
	looksLikeTemporalName,
	parseTemporalValue,
	toDateInputValue,
	toDateTimeInputValue,
	toEpochNumber,
} from "./date";

const DAY_MS = 86_400_000;

describe("formatRelativeTime", () => {
	test("reads in the coarsest unit that fits", () => {
		const now = Date.now();
		expect(formatRelativeTime(new Date(now - 2 * DAY_MS))).toBe("2 days ago");
		expect(formatRelativeTime(new Date(now - 21 * DAY_MS))).toBe("3 weeks ago");
		expect(formatRelativeTime(new Date(now - 400 * DAY_MS))).toBe("last year");
	});

	test("keeps future instants in the future", () => {
		expect(formatRelativeTime(new Date(Date.now() + 3 * DAY_MS))).toBe(
			"in 3 days",
		);
	});

	test("falls back rather than rendering NaN", () => {
		expect(formatRelativeTime("not a date", "long", "—")).toBe("—");
	});
});

describe("formatAbsoluteDateTime", () => {
	test("returns the fallback for unparseable input", () => {
		expect(formatAbsoluteDateTime(null, "—")).toBe("—");
	});

	test("renders a full date and a time", () => {
		const text = formatAbsoluteDateTime(new Date("2026-08-14T10:30:00Z"));
		expect(text).toContain("2026");
		expect(text.length).toBeGreaterThan(10);
	});
});

describe("parseTemporalValue", () => {
	test("reads a bare number by the epoch unit its magnitude implies", () => {
		expect(parseTemporalValue(20_679)?.toISOString()).toBe(
			"2026-08-14T00:00:00.000Z",
		);
		expect(parseTemporalValue(1_786_665_600)?.toISOString()).toBe(
			"2026-08-14T00:00:00.000Z",
		);
		expect(parseTemporalValue(1_786_665_600_000)?.toISOString()).toBe(
			"2026-08-14T00:00:00.000Z",
		);
		expect(parseTemporalValue(1_786_665_600_000_000)?.toISOString()).toBe(
			"2026-08-14T00:00:00.000Z",
		);
		expect(parseTemporalValue(1_786_665_600_000_000_000)?.toISOString()).toBe(
			"2026-08-14T00:00:00.000Z",
		);
	});

	test("accepts bigint timestamps", () => {
		expect(parseTemporalValue(1_786_665_600_000n)?.toISOString()).toBe(
			"2026-08-14T00:00:00.000Z",
		);
	});

	test("leaves numeric-looking strings to the string parser", () => {
		expect(parseTemporalValue("2026")?.getUTCFullYear()).toBe(2026);
	});

	test("reads naive backend timestamps as UTC", () => {
		expect(parseTemporalValue("2026-08-14 10:30:00")?.toISOString()).toBe(
			"2026-08-14T10:30:00.000Z",
		);
	});

	test("returns null for values that are not instants", () => {
		expect(parseTemporalValue(Number.NaN)).toBeNull();
		expect(parseTemporalValue(true)).toBeNull();
		expect(parseTemporalValue(null)).toBeNull();
	});

	test("obeys a declared unit instead of guessing from the magnitude", () => {
		// A second count small enough that the ladder would read it as days.
		expect(parseTemporalValue(86_400, "second")?.toISOString()).toBe(
			"1970-01-02T00:00:00.000Z",
		);
		expect(parseTemporalValue(86_400)?.toISOString()).toBe(
			"2206-07-23T00:00:00.000Z",
		);
	});
});

describe("epoch round trips", () => {
	test("writes back the unit it read", () => {
		for (const unit of [
			"day",
			"second",
			"millisecond",
			"microsecond",
		] as const) {
			const date = parseTemporalValue(
				toEpochNumber(new Date("2026-08-14T00:00:00.000Z"), unit),
				unit,
			);
			expect(date?.toISOString()).toBe("2026-08-14T00:00:00.000Z");
		}
	});

	test("names the unit a bare number counts in", () => {
		expect(detectEpochUnit(20_679)).toBe("day");
		expect(detectEpochUnit(1_786_665_600)).toBe("second");
		expect(detectEpochUnit(1_786_665_600_000)).toBe("millisecond");
		expect(detectEpochUnit(1_786_665_600_000_000)).toBe("microsecond");
		expect(detectEpochUnit(1_786_665_600_000_000_000)).toBe("nanosecond");
	});
});

describe("date input values", () => {
	test("round trips wall-clock time through the viewer's zone", () => {
		const date = new Date(2026, 7, 14, 10, 30, 15);
		expect(toDateTimeInputValue(date)).toBe("2026-08-14T10:30:15");
		expect(toDateTimeInputValue(date, "minute")).toBe("2026-08-14T10:30");
		expect(fromDateTimeInputValue("2026-08-14T10:30:15")?.getTime()).toBe(
			date.getTime(),
		);
	});

	test("keeps day-precision values on UTC midnight", () => {
		const day = new Date(Date.UTC(2026, 7, 14));
		expect(toDateInputValue(day)).toBe("2026-08-14");
		expect(fromDateInputValue("2026-08-14")?.toISOString()).toBe(
			"2026-08-14T00:00:00.000Z",
		);
		expect(toEpochNumber(fromDateInputValue("2026-08-14") as Date, "day")).toBe(
			20_679,
		);
	});

	test("rejects input the pickers cannot produce", () => {
		expect(fromDateTimeInputValue("")).toBeNull();
		expect(fromDateTimeInputValue("14.08.2026")).toBeNull();
		expect(fromDateInputValue("not a date")).toBeNull();
	});
});

describe("formatCalendarDate", () => {
	test("reads a day count in the zone it was written in", () => {
		expect(formatCalendarDate(new Date("2026-08-14T00:00:00.000Z"))).toBe(
			formatCalendarDate(new Date(20_679 * 86_400_000)),
		);
		expect(formatCalendarDate("not a date", "medium", "—")).toBe("—");
	});
});

describe("looksLikeTemporalName", () => {
	test("accepts names whose first or last word promises an instant", () => {
		for (const name of [
			"created_at",
			"updated_at",
			"createdAt",
			"deleted_on",
			"event_time",
			"order_date",
			"timestamp",
			"dateOfBirth",
			"time_received",
			"last_updated",
			"valid_until",
		]) {
			expect(looksLikeTemporalName(name)).toBe(true);
		}
	});

	test("rejects measurements that merely contain a temporal substring", () => {
		for (const name of [
			"total_amount",
			"duration",
			"rating",
			"estimate",
			"quantity_on_hand",
			"latitude",
			"update_count",
			"minimum_stock",
		]) {
			expect(looksLikeTemporalName(name)).toBe(false);
		}
	});
});

describe("inferTemporalValue", () => {
	test("reads an epoch integer when the column name promises one", () => {
		expect(
			inferTemporalValue("created_at", 1_786_353_300_000)?.getFullYear(),
		).toBe(2026);
		expect(
			inferTemporalValue("updatedAt", 1_786_532_400_000)?.getFullYear(),
		).toBe(2026);
	});

	test("leaves numbers alone when the name says nothing", () => {
		expect(inferTemporalValue("total_amount", 1_786_353_300_000)).toBeNull();
		expect(inferTemporalValue("subtotal_amount", 12_450)).toBeNull();
	});

	test("refuses a promising name when the number is not a plausible date", () => {
		expect(inferTemporalValue("created_at", 3)).toBeNull();
		expect(inferTemporalValue("updated_at", -8_000_000_000_000)).toBeNull();
	});

	test("still reads ISO text from a column with any name", () => {
		expect(
			inferTemporalValue("notes", "2026-08-14T10:30:00Z")?.getUTCFullYear(),
		).toBe(2026);
	});

	test("treats a quoted integer as the integer case", () => {
		expect(
			inferTemporalValue("created_at", "1786353300000")?.getFullYear(),
		).toBe(2026);
		expect(inferTemporalValue("subtotal_amount", "12450")).toBeNull();
	});
});
