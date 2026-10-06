/*
 * Dates and times as the viewer reads them (spec M4, M6): Intl in the viewer's language
 * (`navigator.language`), never the app language. "18 Sep 2026", "Fri 18 Sep 2026",
 * "Today 14:02", "Yesterday", "Mon 28 Sep". `shortWordsOf` is the one place these helpers meet `t`.
 */
import type { TFunction } from "i18next";
import type { ShortWords } from "../contracts";

type InterfacesT = TFunction<"interfaces">;

const ISO_DAY = /^(\d{4})-(\d{2})-(\d{2})$/;
const formats = new Map<string, Intl.DateTimeFormat>();

function formatter(
	locale: string,
	options: Intl.DateTimeFormatOptions,
): Intl.DateTimeFormat {
	const id = `${locale}|${JSON.stringify(options)}`;
	const known = formats.get(id);
	if (known) return known;
	let made: Intl.DateTimeFormat;
	try {
		made = new Intl.DateTimeFormat(locale, options);
	} catch {
		made = new Intl.DateTimeFormat("en", options);
	}
	formats.set(id, made);
	return made;
}

function utcOf(iso: string): number | null {
	const match = ISO_DAY.exec(iso);
	if (!match) return null;
	const [, year, month, day] = match;
	const at = Date.UTC(Number(year), Number(month) - 1, Number(day));
	return new Date(at).getUTCDate() === Number(day) ? at : null;
}

/**
 * Intl's text without the comma some languages put after the weekday ("Fri, 18 Sep" → "Fri 18 Sep"),
 * and "Sep" where newer ICU writes "Sept" (en-GB, en-AU, en-IN …; spec and canvas: "17 Sep 2026").
 */
function readableText(parts: readonly Intl.DateTimeFormatPart[]): string {
	return parts
		.map((part, index) => {
			if (part.type === "month" && part.value === "Sept") return "Sep";
			return part.type === "literal" && parts[index - 1]?.type === "weekday"
				? part.value.replace(/^[,，]\s*/, " ")
				: part.value;
		})
		.join("");
}

const DATE: Intl.DateTimeFormatOptions = {
	day: "numeric",
	month: "short",
	year: "numeric",
	timeZone: "UTC",
};
const READING: Intl.DateTimeFormatOptions = { ...DATE, weekday: "short" };

/** `YYYY-MM-DD` → "18 Sep 2026" in the viewer's language; text that is no day is returned as is. */
export function formatDate(iso: string, locale: string): string {
	const at = utcOf(iso);
	return at === null
		? iso
		: readableText(formatter(locale, DATE).formatToParts(at));
}

/** `YYYY-MM-DD` → "Fri 18 Sep 2026", the "Reads as …" line under a typed date. */
export function formatReading(iso: string, locale: string): string {
	const at = utcOf(iso);
	if (at === null) return iso;
	return readableText(formatter(locale, READING).formatToParts(at));
}

export interface WhenWords {
	readonly today: string;
	readonly yesterday: string;
}

/** Every word the model's text helpers need: short values and the recent list's "Today" / "Yesterday". */
export type FormWords = ShortWords & WhenWords;

function dayParts(at: number, timeZone: string | undefined) {
	const parts = formatter("en-US", {
		year: "numeric",
		month: "numeric",
		day: "numeric",
		timeZone,
	}).formatToParts(at);
	const part = (type: string) =>
		Number(parts.find((item) => item.type === type)?.value);
	return { year: part("year"), month: part("month"), day: part("day") };
}

function dayNumber(at: number, timeZone: string | undefined): number {
	const { year, month, day } = dayParts(at, timeZone);
	return Date.UTC(year, month - 1, day) / 86_400_000;
}

/**
 * When a run had a value (recent values, spec M6): "Today 14:02", "Yesterday", "Mon 28 Sep" (with
 * the year when it is not this year's). `timeZone` defaults to the viewer's.
 */
export function formatWhen(
	at: number,
	now: number,
	locale: string,
	words: WhenWords,
	timeZone?: string,
): string {
	const days = dayNumber(now, timeZone) - dayNumber(at, timeZone);
	if (days === 0) {
		const time = formatter(locale, { timeStyle: "short", timeZone });
		return `${words.today} ${time.format(at)}`;
	}
	if (days === 1) return words.yesterday;
	const sameYear = dayParts(at, timeZone).year === dayParts(now, timeZone).year;
	const options: Intl.DateTimeFormatOptions = {
		weekday: "short",
		day: "numeric",
		month: "short",
		year: sameYear ? undefined : "numeric",
		timeZone,
	};
	return readableText(formatter(locale, options).formatToParts(at));
}

/** The words of short value text (`ShortWords`) and of `formatWhen`, with literal keys (PLAN §9). */
export function shortWordsOf(t: InterfacesT, locale: string): FormWords {
	return {
		none: t("interfaces:workbench.short.none", "none"),
		empty: t("interfaces:workbench.short.empty", "empty"),
		on: t("interfaces:workbench.short.on", "On"),
		off: t("interfaces:workbench.short.off", "Off"),
		files: (count) =>
			t("interfaces:workbench.short.files", "{{count}} files", {
				count,
				defaultValue_one: "{{count}} file",
			}),
		entries: (count) =>
			t("interfaces:workbench.short.entries", "{{count}} entries", {
				count,
				defaultValue_one: "{{count}} entry",
			}),
		date: (iso) => formatDate(iso, locale),
		today: t("interfaces:workbench.short.today", "Today"),
		yesterday: t("interfaces:workbench.short.yesterday", "Yesterday"),
	};
}
