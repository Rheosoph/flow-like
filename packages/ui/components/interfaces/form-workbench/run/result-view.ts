/*
 * A returned value prepared for reading (SURFACE §5): labelled rows and tables, never a raw JSON
 * dump as the main view. Labels, tones and number formats follow the canvas's FL_RESULT_VIEW.
 * Pure: the caller passes the viewer's locale, its date formatter and translated words; the
 * defaults are English, so the module never needs `t`.
 */
import type {
	ResultGroup,
	ResultModel,
	ResultRow,
	ResultTable,
	ResultTone,
	ResultView,
} from "../contracts";
import { formatTook } from "./format";

export interface ResultWords {
	readonly yes: string;
	readonly no: string;
	/** null, an empty list or an empty object as the whole result. */
	readonly none: string;
	/** "" as the whole result. */
	readonly emptyText: string;
}

export interface ResultFormat {
	/** Intl locale for numbers (`ViewerHabits.locale`). */
	readonly locale?: string;
	/** `YYYY-MM-DD` → "17 Sep 2026" in the viewer's language (`ShortWords.date`). */
	readonly date?: (iso: string) => string;
	readonly words?: Partial<ResultWords>;
}

interface Context {
	readonly locale: string;
	readonly date: (iso: string) => string;
	readonly words: ResultWords;
}

interface Cell {
	readonly text: string;
	readonly mono: boolean;
	readonly tone: ResultTone | null;
}

interface Sections {
	readonly groups: ResultGroup[];
	readonly tables: ResultTable[];
}

type JsonRecord = Record<string, unknown>;

const DEFAULT_WORDS: ResultWords = {
	yes: "Yes",
	no: "No",
	none: "None",
	emptyText: "Empty text",
};
const ROW_EMPTY = "—";
const CELL_EMPTY = "";
const MAX_DEPTH = 6;
/** Longer strings read as a paragraph, not as one value. */
const LONG_TEXT = 160;
const MONTHS = [
	"Jan",
	"Feb",
	"Mar",
	"Apr",
	"May",
	"Jun",
	"Jul",
	"Aug",
	"Sep",
	"Oct",
	"Nov",
	"Dec",
];

const words = (...list: string[]): ReadonlySet<string> => new Set(list);

/** Keys whose value is an identifier: kept as written, in mono, numbers never grouped. */
const ID_WORDS = words(
	"id",
	"ids",
	"uuid",
	"guid",
	"number",
	"no",
	"nr",
	"num",
	"code",
	"ref",
	"reference",
	"sku",
	"iban",
	"bic",
	"hash",
	"key",
	"serial",
	"ean",
	"gtin",
	"isbn",
	"slug",
	"email",
	"url",
	"uri",
	"link",
	"href",
	"path",
	"file",
	"filename",
);
const MONEY_WORDS = words(
	"price",
	"net",
	"gross",
	"vat",
	"tax",
	"total",
	"subtotal",
	"amount",
	"cost",
	"costs",
	"fee",
	"fees",
	"sum",
	"difference",
	"diff",
	"balance",
	"discount",
	"paid",
	"due",
	"charge",
	"charges",
	"payment",
	"revenue",
	"income",
	"expense",
	"expenses",
	"salary",
	"budget",
	"refund",
	"deposit",
	"outstanding",
);
/** A fraction from 0 to 1 shown as a percentage ("vat_rate": 0.19 → "19 %"). */
const RATE_WORDS = words("rate", "ratio", "share");
const PERCENT_WORDS = words("percent", "percentage", "pct");
const MS_WORDS = words("ms", "millis", "milliseconds");
const SECOND_WORDS = words("seconds", "secs", "sec");
/** A non-zero difference asks for a look ("difference": 714 → warning). */
const DELTA_WORDS = words(
	"difference",
	"diff",
	"delta",
	"variance",
	"discrepancy",
	"deviation",
	"shortfall",
);
const STATUS_WORDS = words(
	"status",
	"state",
	"result",
	"outcome",
	"verdict",
	"decision",
	"health",
);
const SEVERITY_WORDS = words(
	"severity",
	"priority",
	"risk",
	"impact",
	"urgency",
);
/** Yes is good, no asks for a look ("verified", "arithmetic_ok"). */
const GOOD_FLAGS = words(
	"ok",
	"valid",
	"verified",
	"passed",
	"approved",
	"matched",
	"match",
	"success",
	"successful",
	"confirmed",
	"complete",
	"completed",
	"accepted",
	"paid",
	"healthy",
	"compliant",
	"balanced",
	"consistent",
	"correct",
);
/** Yes asks for a look, no is unremarkable ("duplicate"). */
const BAD_FLAGS = words(
	"duplicate",
	"duplicated",
	"error",
	"errors",
	"failed",
	"failure",
	"blocked",
	"expired",
	"overdue",
	"fraud",
	"fraudulent",
	"mismatch",
	"mismatched",
	"missing",
	"flagged",
	"suspicious",
	"rejected",
	"invalid",
	"outdated",
	"stale",
	"conflict",
	"conflicts",
);

function toneTable(
	entries: readonly (readonly [ResultTone, readonly string[]])[],
): ReadonlyMap<string, ResultTone> {
	return new Map(
		entries.flatMap(([tone, list]) =>
			list.map((word) => [word, tone] as const),
		),
	);
}

const STATUS_TONES = toneTable([
	[
		"good",
		[
			"ok",
			"okay",
			"success",
			"succeeded",
			"successful",
			"done",
			"complete",
			"completed",
			"accepted",
			"approved",
			"valid",
			"passed",
			"pass",
			"verified",
			"matched",
			"paid",
			"healthy",
			"resolved",
			"confirmed",
			"delivered",
			"sent",
		],
	],
	[
		"warning",
		[
			"needs_review",
			"review",
			"in_review",
			"pending_review",
			"pending",
			"partial",
			"partially_matched",
			"warning",
			"warn",
			"mismatch",
			"on_hold",
			"hold",
			"waiting",
			"manual",
			"manual_review",
			"incomplete",
			"unverified",
			"expiring",
			"overdue",
			"late",
			"draft",
			"flagged",
			"needs_attention",
			"attention",
			"retry",
			"retrying",
			"degraded",
			"skipped",
		],
	],
	[
		"critical",
		[
			"failed",
			"failure",
			"fail",
			"error",
			"errored",
			"rejected",
			"invalid",
			"denied",
			"declined",
			"cancelled",
			"canceled",
			"blocked",
			"expired",
			"unpaid",
			"missing",
			"fraud",
			"fraudulent",
			"critical",
			"down",
			"offline",
			"aborted",
			"timeout",
			"timed_out",
		],
	],
	[
		"info",
		[
			"processing",
			"running",
			"queued",
			"in_progress",
			"started",
			"new",
			"submitted",
			"scheduled",
			"received",
			"created",
			"open",
			"opened",
		],
	],
]);

const SEVERITY_TONES = toneTable([
	[
		"critical",
		[
			"critical",
			"blocker",
			"fatal",
			"urgent",
			"highest",
			"very_high",
			"severe",
			"sev_1",
			"sev1",
			"p0",
			"p1",
		],
	],
	[
		"warning",
		["high", "medium", "moderate", "major", "elevated", "sev_2", "sev2", "p2"],
	],
	[
		"info",
		["low", "minor", "info", "trivial", "lowest", "sev_3", "sev3", "p3"],
	],
]);

/** Shown in upper case wherever they appear in a key ("vat_id" → "VAT ID"). */
const LABEL_WORDS: ReadonlyMap<string, string> = new Map([
	["id", "ID"],
	["ids", "IDs"],
	["vat", "VAT"],
	["po", "PO"],
	["ocr", "OCR"],
	["url", "URL"],
	["urls", "URLs"],
	["uri", "URI"],
	["api", "API"],
	["pdf", "PDF"],
	["csv", "CSV"],
	["json", "JSON"],
	["xml", "XML"],
	["html", "HTML"],
	["http", "HTTP"],
	["https", "HTTPS"],
	["sku", "SKU"],
	["iban", "IBAN"],
	["bic", "BIC"],
	["ip", "IP"],
	["sql", "SQL"],
	["utc", "UTC"],
	["iso", "ISO"],
	["eu", "EU"],
	["ai", "AI"],
	["llm", "LLM"],
	["kpi", "KPI"],
	["crm", "CRM"],
	["erp", "ERP"],
	["ean", "EAN"],
	["gtin", "GTIN"],
	["isbn", "ISBN"],
	["uuid", "UUID"],
	["guid", "GUID"],
	["faq", "FAQ"],
	["sla", "SLA"],
	["sms", "SMS"],
	["vin", "VIN"],
]);

const ISO_DATE = /^(\d{4})-(\d{2})-(\d{2})$/;
const ISO_DATE_TIME =
	/^\d{4}-\d{2}-\d{2}[T ]\d{2}:\d{2}(?::\d{2}(?:\.\d+)?)?(?:Z|[+-]\d{2}:?\d{2})?$/i;
/** No spaces, at least one digit: "RE-2026-0917", "DE123456789", "4400". */
const ID_LIKE = /^(?=\S*\d)[\p{L}\p{N}][\p{L}\p{N}._\-\/#:]{2,}$/u;
/** Lower-case words and identifiers a flow returns as a value: "needs_review", "sent by e-mail". */
const ENUM_LIKE = /^\p{Ll}[\p{Ll}\p{N}_\-' ]*$/u;
const MARKDOWN =
	/^\s{0,3}(?:#{1,6}\s|[-*+]\s|\d+[.)]\s|>\s|```)|\*\*\S|__\S|\[[^\]\n]+\]\([^)\s]+\)|`[^`\n]+`|^\|.+\|\s*$/m;

const MONEY: Intl.NumberFormatOptions = {
	minimumFractionDigits: 2,
	maximumFractionDigits: 2,
};
const PERCENT: Intl.NumberFormatOptions = {
	maximumFractionDigits: 2,
	useGrouping: false,
};
/** Other numbers: grouped from five digits on, so years and short codes stay "2026", "4400". */
const PLAIN: Intl.NumberFormatOptions = {
	maximumFractionDigits: 6,
	useGrouping: false,
};
const PLAIN_GROUPED: Intl.NumberFormatOptions = { maximumFractionDigits: 6 };

const formatters = new WeakMap<
	Intl.NumberFormatOptions,
	Map<string, Intl.NumberFormat>
>();

function formatterFor(locale: string, options: Intl.NumberFormatOptions) {
	try {
		return new Intl.NumberFormat(locale, options);
	} catch {
		return new Intl.NumberFormat("en-GB", options);
	}
}

function formatNumber(
	locale: string,
	options: Intl.NumberFormatOptions,
	value: number,
) {
	let byLocale = formatters.get(options);
	if (!byLocale) {
		byLocale = new Map();
		formatters.set(options, byLocale);
	}
	let formatter = byLocale.get(locale);
	if (!formatter) {
		formatter = formatterFor(locale, options);
		byLocale.set(locale, formatter);
	}
	return formatter.format(value);
}

const isRecord = (value: unknown): value is JsonRecord =>
	typeof value === "object" && value !== null && !Array.isArray(value);

const isRecordList = (value: unknown): value is JsonRecord[] =>
	Array.isArray(value) && value.length > 0 && value.every(isRecord);

function isEmptyContainer(value: unknown) {
	return Array.isArray(value)
		? value.length === 0
		: isRecord(value) && Object.keys(value).length === 0;
}

function capitalize(text: string) {
	return text.charAt(0).toUpperCase() + text.slice(1);
}

function twoDigits(value: number) {
	return String(value).padStart(2, "0");
}

/** Words of a key in any script: "vat_id" → [vat, id], "invoiceNumber" → [invoice, number]. */
function tokensOf(key: string) {
	return key
		.replace(/([\p{Ll}\p{N}])(\p{Lu})/gu, "$1 $2")
		.replace(/(\p{Lu}+)(\p{Lu}\p{Ll})/gu, "$1 $2")
		.toLowerCase()
		.split(/[^\p{L}\p{N}]+/u)
		.filter(Boolean);
}

function lastTokenOf(key: string) {
	return tokensOf(key).at(-1) ?? "";
}

function isDurationUnit(token: string | undefined) {
	return (
		token !== undefined && (MS_WORDS.has(token) || SECOND_WORDS.has(token))
	);
}

/** "invoice_number" → "Invoice number", "vat_id" → "VAT ID", "duration_ms" → "Duration"; a key with spaces is a label already. */
export function resultLabel(key: string) {
	const trimmed = key.trim();
	if (/\s/.test(trimmed)) return trimmed;
	const tokens = tokensOf(trimmed);
	const shown =
		tokens.length > 1 && isDurationUnit(tokens.at(-1))
			? tokens.slice(0, -1)
			: tokens;
	if (shown.length === 0) return trimmed;
	return shown
		.map(
			(token, index) =>
				LABEL_WORDS.get(token) ?? (index === 0 ? capitalize(token) : token),
		)
		.join(" ");
}

/** "2026-09-17" → "17 Sep 2026" (English months: the default when the caller passes no formatter). */
function englishDate(iso: string) {
	const [year, month, day] = iso.split("-").map(Number);
	return `${day} ${MONTHS[(month ?? 1) - 1]} ${year}`;
}

function isRealDate(iso: string) {
	const match = ISO_DATE.exec(iso);
	if (!match) return false;
	const date = new Date(`${iso}T00:00:00Z`);
	return (
		!Number.isNaN(date.getTime()) &&
		date.getUTCMonth() + 1 === Number(match[2]) &&
		date.getUTCDate() === Number(match[3])
	);
}

/** An instant in the viewer's own time: "17 Sep 2026 14:02". */
function dateTimeText(text: string, ctx: Context) {
	const date = new Date(text);
	if (Number.isNaN(date.getTime())) return null;
	const day = `${date.getFullYear()}-${twoDigits(date.getMonth() + 1)}-${twoDigits(date.getDate())}`;
	return `${ctx.date(day)} ${twoDigits(date.getHours())}:${twoDigits(date.getMinutes())}`;
}

function dateText(text: string, ctx: Context) {
	if (isRealDate(text)) return ctx.date(text);
	return ISO_DATE_TIME.test(text) ? dateTimeText(text, ctx) : null;
}

type NumberStyle = (value: number, locale: string) => string | null;

/** By the key's last word, first match: identifiers, durations, rates, percentages, money. */
const NUMBER_STYLES: readonly (readonly [ReadonlySet<string>, NumberStyle])[] =
	[
		[ID_WORDS, (value) => String(value)],
		[MS_WORDS, (value) => formatTook(value)],
		[SECOND_WORDS, (value) => formatTook(value * 1000)],
		[
			RATE_WORDS,
			(value, locale) =>
				value >= 0 && value <= 1
					? `${formatNumber(locale, PERCENT, value * 100)} %`
					: null,
		],
		[
			PERCENT_WORDS,
			(value, locale) => `${formatNumber(locale, PERCENT, value)} %`,
		],
		[MONEY_WORDS, (value, locale) => formatNumber(locale, MONEY, value)],
	];

function numberText(last: string, value: number, locale: string) {
	if (!Number.isFinite(value)) return String(value);
	for (const [keys, style] of NUMBER_STYLES) {
		const text = keys.has(last) ? style(value, locale) : null;
		if (text !== null) return text;
	}
	return formatNumber(
		locale,
		Math.abs(value) >= 10_000 ? PLAIN_GROUPED : PLAIN,
		value,
	);
}

const booleanTone = (last: string, value: boolean): ResultTone | null => {
	if (GOOD_FLAGS.has(last)) return value ? "good" : "warning";
	return BAD_FLAGS.has(last) && value ? "warning" : null;
};

function stringTone(last: string, text: string) {
	const table = STATUS_WORDS.has(last)
		? STATUS_TONES
		: SEVERITY_WORDS.has(last)
			? SEVERITY_TONES
			: null;
	return table?.get(text.toLowerCase().replace(/[\s-]+/g, "_")) ?? null;
}

/** "needs_review" → "Needs review", "sent by e-mail" → "Sent by e-mail"; anything with deliberate casing stays. */
function humanizeValue(text: string) {
	if (!ENUM_LIKE.test(text)) return text;
	return capitalize(text.includes(" ") ? text : text.replace(/_/g, " "));
}

const plainCell = (text: string, tone: ResultTone | null = null): Cell => ({
	text,
	mono: false,
	tone,
});

const stringCell = (last: string, value: string, ctx: Context): Cell => {
	const text = value.trim();
	const date = dateText(text, ctx);
	if (date) return plainCell(date);
	const tone = stringTone(last, text);
	if (ID_WORDS.has(last) || ID_LIKE.test(text))
		return { text, mono: true, tone };
	return plainCell(humanizeValue(text), tone);
};

const numberCell = (last: string, value: number, ctx: Context): Cell => ({
	text: numberText(last, value, ctx.locale),
	mono: true,
	tone: DELTA_WORDS.has(last) && value !== 0 ? "warning" : null,
});

function booleanCell(last: string, value: boolean, ctx: Context) {
	return plainCell(
		value ? ctx.words.yes : ctx.words.no,
		booleanTone(last, value),
	);
}

function compactJson(value: unknown) {
	try {
		return JSON.stringify(value) ?? String(value);
	} catch {
		return String(value);
	}
}

function isScalar(value: unknown) {
	return value === null || typeof value !== "object";
}

/** null, undefined, whitespace, [] and {}: nothing to show. */
function isBlank(value: unknown) {
	if (value === null || value === undefined) return true;
	if (typeof value === "string") return value.trim() === "";
	return isEmptyContainer(value);
}

const listCell = (
	last: string,
	list: readonly unknown[],
	ctx: Context,
	empty: string,
): Cell => {
	if (!list.every(isScalar))
		return { text: compactJson(list), mono: true, tone: null };
	const cells = list.map((item) => cellOf(last, item, ctx, empty));
	return {
		text: cells.map((cell) => cell.text).join(", "),
		mono: cells.every((cell) => cell.mono),
		tone: null,
	};
};

const cellOf = (
	last: string,
	value: unknown,
	ctx: Context,
	empty: string,
): Cell => {
	if (isBlank(value)) return plainCell(empty);
	if (typeof value === "boolean") return booleanCell(last, value, ctx);
	if (typeof value === "number") return numberCell(last, value, ctx);
	if (typeof value === "string") return stringCell(last, value, ctx);
	if (Array.isArray(value)) return listCell(last, value, ctx, empty);
	return { text: compactJson(value), mono: true, tone: null };
};

const rowOf = (key: string, value: unknown, ctx: Context): ResultRow => ({
	label: resultLabel(key),
	...cellOf(lastTokenOf(key), value, ctx, ROW_EMPTY),
});

function titleOf(path: readonly string[]) {
	return path.map(resultLabel).join(" · ");
}

function columnKeys(records: readonly JsonRecord[]) {
	const keys = new Set<string>();
	for (const record of records)
		for (const key of Object.keys(record)) keys.add(key);
	return Array.from(keys);
}

/** Every value present is a number (a column of numbers with gaps stays numeric). */
function isNumericColumn(records: readonly JsonRecord[], key: string) {
	let seen = false;
	for (const record of records) {
		const value = record[key];
		if (value === null || value === undefined) continue;
		if (typeof value !== "number") return false;
		seen = true;
	}
	return seen;
}

const tableOf = (
	title: string,
	records: readonly JsonRecord[],
	ctx: Context,
): ResultTable => {
	const keys = columnKeys(records);
	const lasts = keys.map(lastTokenOf);
	return {
		title,
		head: keys.map(resultLabel),
		numeric: keys.map((key) => isNumericColumn(records, key)),
		rows: records.map((record) =>
			keys.map(
				(key, column) =>
					cellOf(lasts[column] ?? "", record[key], ctx, CELL_EMPTY).text,
			),
		),
	};
};

const isNested = (
	value: unknown,
	depth: number,
): value is JsonRecord | JsonRecord[] =>
	depth < MAX_DEPTH &&
	(isRecordList(value) || (isRecord(value) && Object.keys(value).length > 0));

/** Scalars of an object form one group; nested objects follow as groups, lists of objects as tables. */
function walkObject(
	path: readonly string[],
	object: JsonRecord,
	ctx: Context,
	into: Sections,
) {
	const rows: ResultRow[] = [];
	const nested: [string, JsonRecord | JsonRecord[]][] = [];
	for (const [key, value] of Object.entries(object)) {
		if (isNested(value, path.length)) nested.push([key, value]);
		else rows.push(rowOf(key, value, ctx));
	}
	if (rows.length > 0) into.groups.push({ title: titleOf(path), rows });
	for (const [key, value] of nested) {
		const childPath = [...path, key];
		if (Array.isArray(value))
			into.tables.push(tableOf(titleOf(childPath), value, ctx));
		else walkObject(childPath, value, ctx, into);
	}
}

const valueView = (text: string, mono: boolean): ResultView => ({
	kind: "value",
	text,
	mono,
	tone: null,
});

const stringView = (value: string, ctx: Context): ResultView => {
	if (!value.trim()) return valueView(ctx.words.emptyText, false);
	const markdown = MARKDOWN.test(value);
	if (markdown || value.includes("\n") || value.length > LONG_TEXT)
		return { kind: "text", text: value, markdown };
	const cell = stringCell("", value, ctx);
	return valueView(cell.text, cell.mono);
};

const objectView = (value: JsonRecord, ctx: Context): ResultView => {
	const sections: Sections = { groups: [], tables: [] };
	walkObject([], value, ctx, sections);
	if (sections.groups.length === 0 && sections.tables.length === 0)
		return valueView(ctx.words.none, false);
	return { kind: "structured", ...sections };
};

/** A list of records is one table; any other list reads as numbered rows. */
const listView = (value: readonly unknown[], ctx: Context): ResultView => {
	if (value.length === 0) return valueView(ctx.words.none, false);
	if (value.every(isRecord))
		return {
			kind: "structured",
			groups: [],
			tables: [tableOf("", value, ctx)],
		};
	const rows = value.map((item, index) => ({
		label: String(index + 1),
		...cellOf("", item, ctx, ROW_EMPTY),
	}));
	return { kind: "structured", groups: [{ title: "", rows }], tables: [] };
};

const viewOf = (value: unknown, ctx: Context): ResultView => {
	if (typeof value === "string") return stringView(value, ctx);
	if (Array.isArray(value)) return listView(value, ctx);
	if (isRecord(value)) return objectView(value, ctx);
	if (value === null || value === undefined)
		return valueView(ctx.words.none, false);
	const cell = cellOf("", value, ctx, ctx.words.none);
	return valueView(cell.text, cell.mono);
};

function rawOf(value: unknown) {
	if (value === undefined) return "null";
	try {
		return JSON.stringify(value, null, 2) ?? String(value);
	} catch {
		return String(value);
	}
}

function contextOf(format: ResultFormat) {
	return {
		locale: format.locale ?? "en-GB",
		date: format.date ?? englishDate,
		words: { ...DEFAULT_WORDS, ...format.words },
	};
}

/**
 * Any JSON a run returned (`{ value: false }` included) as a view, the raw JSON for the toggle,
 * and what "Copy result" copies (a string as it is, anything else as JSON). `format` is optional,
 * so the function is a `ToResultModel`.
 */
export const toResultModel = (
	value: unknown,
	format: ResultFormat = {},
): ResultModel => {
	const raw = rawOf(value);
	return {
		view: viewOf(value, contextOf(format)),
		raw,
		copyText: typeof value === "string" ? value : raw,
	};
};

/** The first text or number at the top level of a result object, as the Runs list's first line uses it. */
export function leadRowOf(value: unknown, format: ResultFormat = {}) {
	if (!isRecord(value)) return null;
	const ctx = contextOf(format);
	for (const [key, item] of Object.entries(value)) {
		if (typeof item === "number" || (typeof item === "string" && item.trim()))
			return rowOf(key, item, ctx);
	}
	return null;
}
