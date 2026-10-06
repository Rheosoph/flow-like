import { beforeAll, describe, expect, test } from "bun:test";
import i18next, { type TFunction } from "i18next";
import {
	formatDate,
	formatReading,
	formatWhen,
	shortWordsOf,
} from "./date-text";

const WORDS = { today: "Today", yesterday: "Yesterday" };
const at = (iso: string) => Date.parse(iso);

describe("dates in the viewer's language", () => {
	test("18 Sep 2026 and its reading", () => {
		expect(formatDate("2026-09-18", "en-GB")).toBe("18 Sep 2026");
		expect(formatReading("2026-09-18", "en-GB")).toBe("Fri 18 Sep 2026");
		expect(formatReading("2026-09-01", "en-GB")).toBe("Tue 1 Sep 2026");
		expect(formatReading("2026-10-02", "en-GB")).toBe("Fri 2 Oct 2026");
	});

	test("other languages keep Intl's order, without a comma after the weekday", () => {
		expect(formatDate("2026-09-18", "en-US")).toBe("Sep 18, 2026");
		expect(formatReading("2026-09-18", "en-US")).toBe("Fri Sep 18, 2026");
		expect(formatReading("2026-09-18", "de-DE")).toBe("Fr. 18. Sept. 2026");
		expect(formatReading("2026-09-18", "ja-JP")).toContain("2026年9月18日");
	});

	test("Sep where newer ICU writes Sept (Chrome and WebKit for en-GB)", () => {
		const original = Intl.DateTimeFormat.prototype.formatToParts;
		Intl.DateTimeFormat.prototype.formatToParts = function (date) {
			return original
				.call(this, date)
				.map((part) =>
					part.type === "month" && part.value === "Sep"
						? { ...part, value: "Sept" }
						: part,
				);
		};
		try {
			expect(formatDate("2026-09-17", "en-GB")).toBe("17 Sep 2026");
			expect(formatReading("2026-09-17", "en-GB")).toBe("Thu 17 Sep 2026");
		} finally {
			Intl.DateTimeFormat.prototype.formatToParts = original;
		}
	});

	test("text that is no day comes back as it is; a bad locale falls back to English", () => {
		expect(formatDate("2026-02-30", "en-GB")).toBe("2026-02-30");
		expect(formatDate("soon", "en-GB")).toBe("soon");
		expect(formatReading("", "en-GB")).toBe("");
		expect(formatDate("2026-09-18", "not a locale!")).toContain("2026");
	});
});

describe("when a run had a value (recent values)", () => {
	const now = at("2026-10-05T16:30:00Z");

	test("Today 14:02, Yesterday, then the day", () => {
		expect(
			formatWhen(at("2026-10-05T14:02:00Z"), now, "en-GB", WORDS, "UTC"),
		).toBe("Today 14:02");
		expect(
			formatWhen(at("2026-10-05T09:47:00Z"), now, "en-GB", WORDS, "UTC"),
		).toBe("Today 09:47");
		expect(
			formatWhen(at("2026-10-05T09:47:00Z"), now, "en-US", WORDS, "UTC"),
		).toMatch(/^Today 9:47\sAM$/u);
		expect(
			formatWhen(at("2026-10-04T23:59:00Z"), now, "en-GB", WORDS, "UTC"),
		).toBe("Yesterday");
		expect(
			formatWhen(at("2026-09-28T09:15:00Z"), now, "en-GB", WORDS, "UTC"),
		).toBe("Mon 28 Sep");
		expect(
			formatWhen(at("2025-09-28T09:15:00Z"), now, "en-GB", WORDS, "UTC"),
		).toBe("Sun 28 Sep 2025");
	});

	test("days follow the viewer's time zone", () => {
		const late = at("2026-10-04T23:30:00Z");
		expect(formatWhen(late, now, "en-GB", WORDS, "UTC")).toBe("Yesterday");
		expect(formatWhen(late, now, "en-GB", WORDS, "Europe/Berlin")).toBe(
			"Today 01:30",
		);
	});
});

describe("the words of short values", () => {
	let t: TFunction<"interfaces">;

	beforeAll(async () => {
		const instance = i18next.createInstance();
		await instance.init({
			lng: "en",
			resources: {},
			interpolation: { escapeValue: false },
		});
		t = instance.getFixedT(
			"en",
			"interfaces",
		) as unknown as TFunction<"interfaces">;
	});

	test("English defaults with plurals, dates through Intl", () => {
		const words = shortWordsOf(t, "en-GB");
		expect([
			words.none,
			words.empty,
			words.on,
			words.off,
			words.today,
			words.yesterday,
		]).toEqual(["none", "empty", "On", "Off", "Today", "Yesterday"]);
		expect([
			words.files(1),
			words.files(2),
			words.entries(1),
			words.entries(3),
		]).toEqual(["1 file", "2 files", "1 entry", "3 entries"]);
		expect(words.date("2026-09-18")).toBe("18 Sep 2026");
	});
});
