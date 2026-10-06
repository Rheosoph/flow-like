import {
	type DateLocale,
	type DateReading,
	FORM_LIMITS,
	type FieldKey,
	type FormSessionState,
	type ReadDate,
	type WorkbenchField,
} from "../contracts";
import { foldText } from "./secrets";

export interface IsoParts {
	readonly y: number;
	readonly m: number;
	readonly d: number;
}

const DAY_MS = 86_400_000;
const ENGLISH_MONTHS = [
	"january",
	"february",
	"march",
	"april",
	"may",
	"june",
	"july",
	"august",
	"september",
	"october",
	"november",
	"december",
];
const CJK_LETTERS = /[\u{3040}-\u{30ff}\u{3400}-\u{9fff}\u{ac00}-\u{d7af}]/u;
const MONTH_WORD = /^\p{L}{3,}$/u;

function isLeapYear(y: number) {
	return (y % 4 === 0 && y % 100 !== 0) || y % 400 === 0;
}

function daysInMonth(y: number, m: number) {
	if (m === 2) return isLeapYear(y) ? 29 : 28;
	return [4, 6, 9, 11].includes(m) ? 30 : 31;
}

function validParts(y: number, m: number, d: number) {
	if (!Number.isInteger(y) || y < 1 || y > 9999) return false;
	return m >= 1 && m <= 12 && d >= 1 && d <= daysInMonth(y, m);
}

/** `YYYY-MM-DD` of a real calendar day → its parts; null for anything else. */
export const isoParts = (iso: string): IsoParts | null => {
	const match = /^(\d{4})-(\d{2})-(\d{2})$/.exec(iso);
	if (!match) return null;
	const [y, m, d] = [Number(match[1]), Number(match[2]), Number(match[3])];
	return validParts(y, m, d) ? { y, m, d } : null;
};

function isoOf(y: number, m: number, d: number) {
	const year = String(y).padStart(4, "0");
	return `${year}-${String(m).padStart(2, "0")}-${String(d).padStart(2, "0")}`;
}

/** Days since 1970-01-01 (UTC calendar), valid for every four-digit year. */
function dayNumber(y: number, m: number, d: number) {
	const day = new Date(0);
	day.setUTCFullYear(y, m - 1, d);
	return Math.round(day.getTime() / DAY_MS);
}

/** A date some days away from an ISO date; null when `iso` is not a real day. */
export function shiftIso(iso: string, days: number) {
	const parts = isoParts(iso);
	if (!parts) return null;
	const day = new Date(0);
	day.setUTCFullYear(parts.y, parts.m - 1, parts.d + days);
	return isoOf(day.getUTCFullYear(), day.getUTCMonth() + 1, day.getUTCDate());
}

const reading = (y: number, m: number, d: number): DateReading | null =>
	validParts(y, m, d) ? { iso: isoOf(y, m, d), past: 0 } : null;

/** Day and month without a year: the year (the anchor's, one before or one after) that puts the date nearest the anchor. */
function nearest(d: number, m: number, anchor: IsoParts) {
	const at = dayNumber(anchor.y, anchor.m, anchor.d);
	let best: DateReading | null = null;
	let gap = Number.POSITIVE_INFINITY;
	for (const y of [anchor.y - 1, anchor.y, anchor.y + 1]) {
		const candidate = reading(y, m, d);
		const distance = Math.abs(dayNumber(y, m, d) - at);
		if (candidate && distance < gap) {
			gap = distance;
			best = candidate;
		}
	}
	return best;
}

/** A year as typed: four digits as they are; fewer in the century that puts it nearest to today ("85" in 2026 is 1985). */
function yearOf(text: string, todayYear: number) {
	if (text.length === 4) return Number(text);
	const short = Number(text);
	const century = Math.floor(todayYear / 100) * 100;
	const candidates = [
		century - 100 + short,
		century + short,
		century + 100 + short,
	];
	return candidates.reduce((best, year) =>
		Math.abs(year - todayYear) < Math.abs(best - todayYear) ? year : best,
	);
}

function monthFold(text: string) {
	return foldText(text)
		.replace(/[.\s]+$/g, "")
		.trim();
}

function unique(words: readonly string[]) {
	return words.filter((word, index) => words.indexOf(word) === index);
}

function safeLocale(lang: string) {
	try {
		return new Intl.DateTimeFormat(lang).resolvedOptions().locale;
	} catch {
		return undefined;
	}
}

function monthInDate(
	locale: string | undefined,
	style: "long" | "short",
	when: number,
) {
	return new Intl.DateTimeFormat(locale, {
		day: "numeric",
		month: style,
		timeZone: "UTC",
	})
		.formatToParts(when)
		.find((part) => part.type === "month")?.value;
}

function monthNames(locale: string | undefined, index: number) {
	const when = Date.UTC(2026, index, 15, 12);
	const names = [ENGLISH_MONTHS[index]];
	for (const style of ["long", "short"] as const) {
		const alone = new Intl.DateTimeFormat(locale, {
			month: style,
			timeZone: "UTC",
		});
		names.push(alone.format(when));
		const inDate = monthInDate(locale, style, when);
		if (inDate) names.push(inDate);
	}
	return unique(names.map(monthFold)).filter((word) => MONTH_WORD.test(word));
}

function orderOf(locale: string | undefined) {
	const parts = new Intl.DateTimeFormat(locale, {
		year: "numeric",
		month: "numeric",
		day: "numeric",
		timeZone: "UTC",
	}).formatToParts(Date.UTC(2026, 8, 18, 12));
	const first = parts.find(
		(part) =>
			part.type === "day" || part.type === "month" || part.type === "year",
	)?.type;
	const literal = parts.find((part) => part.type === "literal");
	const order: DateLocale["order"] =
		first === "year" ? "ymd" : first === "month" ? "mdy" : "dmy";
	return { order, sep: literal?.value.trim().charAt(0) || "/" };
}

const localeCache = new Map<string, DateLocale>();

/**
 * The viewer's date habits from Intl (spec M4, `flpDateLocale`): the order of day, month and year, the
 * separator of numeric dates, month names in the viewer's language and English, and the words for today,
 * yesterday and tomorrow. Pass `navigator.language`, never the app language. An unknown tag reads as the
 * runtime's default locale.
 */
export const dateLocaleOf = (lang: string): DateLocale => {
	const cached = localeCache.get(lang);
	if (cached) return cached;
	const locale = safeLocale(lang);
	const relative = new Intl.RelativeTimeFormat(locale, { numeric: "auto" });
	const word = (days: number, english: string) =>
		unique([foldText(relative.format(days, "day")), english]);
	const result: DateLocale = {
		...orderOf(locale),
		months: ENGLISH_MONTHS.map((_, index) => monthNames(locale, index)),
		words: {
			today: word(0, "today"),
			yesterday: word(-1, "yesterday"),
			tomorrow: word(1, "tomorrow"),
		},
	};
	localeCache.set(lang, result);
	return result;
};

/** Month number from a typed month word ("sep", "September", "sept."), or 0 when none or several months fit. */
function monthOf(word: string, locale: DateLocale) {
	const folded = monthFold(word);
	if (folded.length < 3) return 0;
	const fits = (name: string) =>
		name.startsWith(folded) || folded.startsWith(name);
	const hits = locale.months
		.map((names, index) => (names.some(fits) ? index + 1 : 0))
		.filter((month) => month > 0);
	return hits.length === 1 ? hits[0] : 0;
}

interface ReadContext {
	readonly today: IsoParts;
	readonly todayIso: string;
	readonly anchor: IsoParts;
	readonly locale: DateLocale;
}

/** Day and month in the viewer's order (month first for mdy and ymd). */
function dayMonth(a: string, b: string, context: ReadContext) {
	return context.locale.order === "dmy"
		? nearest(Number(a), Number(b), context.anchor)
		: nearest(Number(b), Number(a), context.anchor);
}

/** Three numbers: a four-digit year first in any order, else the viewer's order. */
function fullDate(a: string, b: string, c: string, context: ReadContext) {
	const year = context.today.y;
	if (a.length === 4 || context.locale.order === "ymd")
		return reading(yearOf(a, year), Number(b), Number(c));
	return context.locale.order === "mdy"
		? reading(yearOf(c, year), Number(a), Number(b))
		: reading(yearOf(c, year), Number(b), Number(a));
}

/** A day alone takes the anchor's month and year; far before the anchor it carries a warning (`past`). */
function dayAlone(day: string, context: ReadContext) {
	const { anchor } = context;
	const result = reading(anchor.y, anchor.m, Number(day));
	if (!result) return null;
	const back =
		dayNumber(anchor.y, anchor.m, anchor.d) -
		dayNumber(anchor.y, anchor.m, Number(day));
	return back > FORM_LIMITS.pastDaysWarn ? { ...result, past: back } : result;
}

function namedMonth(
	day: string,
	word: string,
	year: string | undefined,
	context: ReadContext,
) {
	const month = monthOf(word, context.locale);
	if (!month) return null;
	return year
		? reading(yearOf(year, context.today.y), month, Number(day))
		: nearest(Number(day), month, context.anchor);
}

function threeNumbers(match: RegExpExecArray, context: ReadContext) {
	const [, a, b, c] = match;
	const yearInMiddle = a.length === 3 || c.length === 3;
	if (yearInMiddle || (a.length === 4 && c.length === 4)) return null;
	return fullDate(a, b, c, context);
}

function eightDigits(match: RegExpExecArray, context: ReadContext) {
	const digits = match[1];
	return context.locale.order === "ymd"
		? fullDate(digits.slice(0, 4), digits.slice(4, 6), digits.slice(6), context)
		: fullDate(
				digits.slice(0, 2),
				digits.slice(2, 4),
				digits.slice(4),
				context,
			);
}

interface DateShape {
	readonly pattern: RegExp;
	readonly read: (
		match: RegExpExecArray,
		context: ReadContext,
	) => DateReading | null;
}

/** Typed shapes, first match wins (`flpReadDate`). */
const DATE_SHAPES: readonly DateShape[] = [
	{
		pattern: /^(\d{4})-(\d{1,2})-(\d{1,2})$/,
		read: (m) => reading(Number(m[1]), Number(m[2]), Number(m[3])),
	},
	{ pattern: /^(\d{1,2})\.?$/, read: (m, c) => dayAlone(m[1], c) },
	{
		pattern: /^(\d{1,2})[./\- ](\d{1,2})\.?$/,
		read: (m, c) => dayMonth(m[1], m[2], c),
	},
	{
		pattern: /^(\d{1,4})[./\- ](\d{1,2})[./\- ](\d{1,4})\.?$/,
		read: threeNumbers,
	},
	{ pattern: /^(\d{2})(\d{2})$/, read: (m, c) => dayMonth(m[1], m[2], c) },
	{
		pattern: /^(\d{2})(\d{2})(\d{2})$/,
		read: (m, c) => fullDate(m[1], m[2], m[3], c),
	},
	{ pattern: /^(\d{8})$/, read: eightDigits },
	{
		pattern: /^(\d{1,2})\.? (\p{L}{3,})\.?(?:,? (\d{2}|\d{4}))?$/u,
		read: (m, c) => namedMonth(m[1], m[2], m[3], c),
	},
	{
		pattern: /^(\p{L}{3,})\.? (\d{1,2})(?:,? (\d{2}|\d{4}))?$/u,
		read: (m, c) => namedMonth(m[2], m[1], m[3], c),
	},
];

/** Today, yesterday or tomorrow in the viewer's language or English; three letters are enough, one CJK letter too. */
function relativeDay(raw: string, context: ReadContext) {
	if (raw.length < 3 && !CJK_LETTERS.test(raw)) return null;
	const { words } = context.locale;
	const offsets: readonly [readonly string[], number][] = [
		[words.today, 0],
		[words.yesterday, -1],
		[words.tomorrow, 1],
	];
	const hit = offsets.find(([list]) =>
		list.some((word) => word.startsWith(raw)),
	);
	const iso = hit ? shiftIso(context.todayIso, hit[1]) : null;
	return iso ? { iso, past: 0 } : null;
}

/** ja, ko and zh units ("9月18日", "2026년 9월 18일") become dashes. */
function withoutUnits(raw: string) {
	return raw
		.replace(/[年년]\s*/g, "-")
		.replace(/[月월]\s*/g, "-")
		.replace(/[日일]/g, "")
		.replace(/[-\s]+$/, "")
		.trim();
}

function readShapes(typed: string, context: ReadContext) {
	for (const shape of DATE_SHAPES) {
		const match = shape.pattern.exec(typed);
		if (match) return shape.read(match, context);
	}
	return null;
}

/**
 * Reads a typed date in the viewer's order (spec M4, `flpReadDate`). `anchorIso`: this session's last date in
 * the field, else the newest run's value for it, else today (`dateAnchorOf`). `{ iso: "", past: 0 }` for empty
 * text; null when the text cannot be read.
 */
export const readDate: ReadDate = (text, anchorIso, todayIso, locale) => {
	const raw = foldText(text.trim()).replace(/\s+/g, " ");
	if (!raw) return { iso: "", past: 0 };
	const today = isoParts(todayIso);
	if (!today) return null;
	const anchor = (anchorIso ? isoParts(anchorIso) : null) ?? today;
	const context: ReadContext = { today, todayIso, anchor, locale };
	return relativeDay(raw, context) ?? readShapes(withoutUnits(raw), context);
};

/** Typed-date examples in the viewer's order for 21 September 2026: `{ short, full }` ("21/9", "21/9/2026"). */
export function dateExample(locale: DateLocale) {
	const sep = locale.sep;
	if (locale.order === "mdy")
		return { short: `9${sep}21`, full: `9${sep}21${sep}2026` };
	if (locale.order === "ymd")
		return { short: `9${sep}21`, full: `2026${sep}9${sep}21` };
	return { short: `21${sep}9`, full: `21${sep}9${sep}2026` };
}

interface FieldPath {
	readonly name: string;
	readonly property: string | null;
}

function pathOf(fields: readonly WorkbenchField[], key: FieldKey) {
	for (const field of fields) {
		if (field.key === key) return { name: field.name, property: null };
		const prop = field.props.find((candidate) => candidate.key === key);
		if (prop) return { name: field.name, property: prop.name };
	}
	return null;
}

function valueAt(values: Readonly<Record<string, unknown>>, path: FieldPath) {
	const value = values[path.name];
	if (path.property === null) return value;
	if (typeof value !== "object" || value === null || Array.isArray(value))
		return undefined;
	return (value as Readonly<Record<string, unknown>>)[path.property];
}

const isIsoText = (item: unknown): item is string =>
	typeof item === "string" && isoParts(item) !== null;

/** A stored or copied date value: the date itself, or the last date of a list of dates. */
function lastIso(value: unknown) {
	const list = Array.isArray(value) ? [...value].reverse() : [value];
	return list.find(isIsoText) ?? null;
}

/**
 * The anchor of a typed date (spec M4): this session's last date in the field (`rail.dateAnchors`), else the
 * newest run's value for it on this device, else `today`. The reducer's commit and the field's preview both use it.
 */
export function dateAnchorOf(
	state: FormSessionState,
	key: FieldKey,
	today: string,
) {
	const typed = state.rail.dateAnchors[key];
	if (typed && isoParts(typed)) return typed;
	const path = pathOf(state.form.fields, key);
	if (!path) return today;
	for (const run of state.runs) {
		const iso = lastIso(valueAt(run.copy.values, path));
		if (iso) return iso;
	}
	return today;
}
