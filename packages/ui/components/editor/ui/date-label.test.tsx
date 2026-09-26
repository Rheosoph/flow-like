import { afterAll, beforeAll, describe, expect, test } from "bun:test";
import { createSlateEditor } from "platejs";
import { PlateStatic } from "platejs/static";
import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { BaseDateKit } from "../plugins/date-base-kit";
import {
	getDateElementDate,
	getDateElementDay,
	getDateElementLabel,
} from "./date-label";

const LONG_DATE: Intl.DateTimeFormatOptions = {
	day: "numeric",
	month: "long",
	year: "numeric",
};

/** The label Plate 49's date components computed, kept to pin parity for old documents. */
function plate49Label(value: string, now: Date) {
	const today = new Date(now);
	const elementDate = new Date(value);
	if (elementDate.toDateString() === today.toDateString()) return "Today";
	if (
		new Date(today.setDate(today.getDate() - 1)).toDateString() ===
		elementDate.toDateString()
	)
		return "Yesterday";
	if (
		new Date(today.setDate(today.getDate() + 2)).toDateString() ===
		elementDate.toDateString()
	)
		return "Tomorrow";
	return elementDate.toLocaleDateString(undefined, LONG_DATE);
}

const localDay = (date: Date) =>
	[
		date.getFullYear(),
		String(date.getMonth() + 1).padStart(2, "0"),
		String(date.getDate()).padStart(2, "0"),
	].join("-");

function renderStatic(date: { date?: string; rawDate?: string }) {
	const editor = createSlateEditor({
		plugins: BaseDateKit,
		nodeId: false,
		value: [
			{
				type: "p",
				children: [
					{ text: "" },
					{ type: "date", ...date, children: [{ text: "" }] },
					{ text: "" },
				],
			},
		],
	});
	const markup = renderToStaticMarkup(createElement(PlateStatic, { editor }));
	const chip = markup.match(
		/<span class="w-fit rounded-sm bg-muted px-1 text-muted-foreground">(.*?)<\/span><span/,
	);
	return chip?.[1].replace(/<\/?span>/g, "");
}

const previousTimeZone = process.env.TZ;
afterAll(() => {
	process.env.TZ = previousTimeZone;
});

for (const timeZone of ["UTC", "America/Los_Angeles", "Asia/Tokyo"]) {
	describe(`date nodes in ${timeZone}`, () => {
		beforeAll(() => {
			process.env.TZ = timeZone;
		});

		const january15 = () =>
			new Date(2024, 0, 15).toLocaleDateString(undefined, LONG_DATE);

		test("legacy toDateString values read as their calendar day, like Plate 49", () => {
			const now = new Date(2024, 5, 1, 12);
			const element = { date: "Mon Jan 15 2024" };

			expect(getDateElementDay(element)).toBe("2024-01-15");
			expect(getDateElementLabel(element, now)).toBe(january15());
			expect(getDateElementLabel(element, now)).toBe(
				plate49Label(element.date, now),
			);
		});

		test("canonical YYYY-MM-DD values never shift a day", () => {
			const now = new Date(2024, 5, 1, 12);
			const element = { date: "2024-01-15" };

			expect(getDateElementDay(element)).toBe("2024-01-15");
			expect(getDateElementLabel(element, now)).toBe(january15());
			expect(getDateElementDate(element)?.getDate()).toBe(15);
			if (timeZone === "America/Los_Angeles")
				expect(plate49Label(element.date, now)).toBe(
					new Date(2024, 0, 14).toLocaleDateString(undefined, LONG_DATE),
				);
		});

		test("rawDate text that Date can parse displays as a date", () => {
			const now = new Date(2024, 5, 1, 12);

			expect(getDateElementLabel({ rawDate: "January 15, 2024" }, now)).toBe(
				january15(),
			);
			expect(getDateElementLabel({ rawDate: "2024/01/15" }, now)).toBe(
				january15(),
			);
		});

		test("other strings Date parses keep Plate 49's local reading", () => {
			const now = new Date(2024, 5, 1, 12);
			const value = "2024-01-15T18:30:00.000Z";

			expect(getDateElementDay({ date: value })).toBe(
				localDay(new Date(value)),
			);
			expect(getDateElementLabel({ date: value }, now)).toBe(
				plate49Label(value, now),
			);
		});

		test("unreadable values show as stored instead of 'Invalid Date'", () => {
			expect(getDateElementDay({ rawDate: "next Tuesday" })).toBeUndefined();
			expect(getDateElementLabel({ rawDate: "next Tuesday" })).toBe(
				"next Tuesday",
			);
			expect(getDateElementLabel({ date: "garbage" })).toBe("garbage");
			expect(getDateElementLabel({})).toBeUndefined();
			expect(getDateElementLabel({ date: "" })).toBeUndefined();
		});

		test("relative labels follow the local calendar day", () => {
			const lateEvening = new Date(2024, 0, 15, 23, 30);

			expect(getDateElementLabel({ date: "2024-01-15" }, lateEvening)).toBe(
				"Today",
			);
			expect(
				getDateElementLabel({ date: "Sun Jan 14 2024" }, lateEvening),
			).toBe("Yesterday");
			expect(getDateElementLabel({ date: "2024-01-16" }, lateEvening)).toBe(
				"Tomorrow",
			);
			expect(
				getDateElementLabel({ date: "Sun Jan 14 2024" }, lateEvening),
			).toBe(plate49Label("Sun Jan 14 2024", lateEvening));
		});

		test("the static chip renders every stored shape", () => {
			expect(renderStatic({ date: "Mon Jan 15 2024" })).toBe(january15());
			expect(renderStatic({ date: "2024-01-15" })).toBe(january15());
			expect(renderStatic({ rawDate: "next Tuesday" })).toBe("next Tuesday");
			expect(renderStatic({})).toBe("Pick a date");
		});
	});
}
