/*
 * Durations, sizes and typed numbers as the workbench prints them (mono, tabular). Pure; the canvas's
 * flTook / flClock / flSize with hours and gigabytes added.
 */
import type { FieldChange, WorkbenchField } from "../contracts";

const SIZE_UNITS = ["KB", "MB", "GB", "TB"] as const;

function wholeSeconds(ms: number, round: (value: number) => number) {
	return Number.isFinite(ms) ? Math.max(0, round(ms / 1000)) : 0;
}

function twoDigits(value: number) {
	return String(value).padStart(2, "0");
}

/** "48 s", "1 min 12 s", "2 h 5 min": how long a run took (rounded seconds). */
export function formatTook(ms: number) {
	const total = wholeSeconds(ms, Math.round);
	if (total < 60) return `${total} s`;
	if (total < 3600) {
		const seconds = total % 60;
		return `${Math.floor(total / 60)} min${seconds ? ` ${seconds} s` : ""}`;
	}
	const minutes = Math.floor((total % 3600) / 60);
	return `${Math.floor(total / 3600)} h${minutes ? ` ${minutes} min` : ""}`;
}

/** "0:37", "2:11", "1:02:05": the running clock (whole seconds, never rounded up). */
export function formatClock(ms: number) {
	const total = wholeSeconds(ms, Math.floor);
	const minutes = Math.floor(total / 60);
	const seconds = twoDigits(total % 60);
	if (minutes < 60) return `${minutes}:${seconds}`;
	return `${Math.floor(minutes / 60)}:${twoDigits(minutes % 60)}:${seconds}`;
}

/**
 * "1.2 MB", "6.3 KB", "512 bytes" (1 KB = 1024 bytes, one decimal). An unknown size (null,
 * 0, negative, not finite) is "", so callers leave the size out instead of printing "0 bytes".
 */
export function formatBytes(
	bytes: number | null | undefined,
	decimalSign: "." | "," = ".",
) {
	if (typeof bytes !== "number" || !Number.isFinite(bytes) || bytes <= 0)
		return "";
	if (bytes < 1024)
		return bytes === 1 ? "1 byte" : `${Math.round(bytes)} bytes`;
	let value = bytes / 1024;
	let unit = 0;
	while (Math.round(value * 10) / 10 >= 1024 && unit < SIZE_UNITS.length - 1) {
		value /= 1024;
		unit += 1;
	}
	const text = value.toFixed(1);
	return `${decimalSign === "," ? text.replace(".", ",") : text} ${SIZE_UNITS[unit]}`;
}

/** A plain typed number: sign, up to 15 digits, one decimal mark. Longer ones would lose digits. */
const TYPED_NUMBER = /^([+-]?)(\d{1,15})(?:([.,])(\d{1,15}))?$/;
const MAX_DIGITS = 15;
const GROUPED_FROM = 10_000;

const numberFormats = new Map<string, Intl.NumberFormat>();

function numberFormat(locale: string, decimals: number, grouped: boolean) {
	const key = `${locale}\u0001${decimals}\u0001${grouped}`;
	const known = numberFormats.get(key);
	if (known) return known;
	const options: Intl.NumberFormatOptions = {
		minimumFractionDigits: decimals,
		maximumFractionDigits: decimals,
		...(grouped ? {} : { useGrouping: false }),
	};
	let format: Intl.NumberFormat;
	try {
		format = new Intl.NumberFormat(locale, options);
	} catch {
		format = new Intl.NumberFormat("en-GB", options);
	}
	numberFormats.set(key, format);
	return format;
}

/**
 * A typed number as the stage, the change chips, Compare and the Runs list print it: the viewer's signs,
 * the typed decimals, grouped when it has decimals or five digits ("6,188.00", "11,769.10", "20",
 * "1920"). Anything else stays as typed: not a plain number, a decimal comma the viewer does not use,
 * a leading zero ("007"), or more than 15 digits.
 */
export function formatTypedNumber(
	text: string,
	locale: string,
	decimalSign: "." | ",",
): string {
	const typed = text.trim();
	const match = TYPED_NUMBER.exec(typed);
	if (!match) return typed;
	const [, sign, whole, mark, fraction = ""] = match;
	const foreignComma = mark === "," && decimalSign !== ",";
	const leadingZero = whole.length > 1 && whole.startsWith("0");
	if (
		foreignComma ||
		leadingZero ||
		whole.length + fraction.length > MAX_DIGITS
	)
		return typed;
	const value = Number(`${sign}${whole}.${fraction || "0"}`);
	const grouped = fraction !== "" || Math.abs(value) >= GROUPED_FROM;
	return numberFormat(locale, fraction.length, grouped).format(value);
}

/** The FieldKeys of number fields and number properties. */
function numberKeysOf(fields: readonly WorkbenchField[]) {
	const keys = new Set<string>();
	for (const field of fields) {
		if (field.kind === "number") keys.add(field.key);
		for (const prop of field.props)
			if (prop.kind === "number") keys.add(prop.key);
	}
	return keys;
}

/** A change list with its number inputs in the viewer's reading; the mask and the empty word stay. */
export function withNumberText<T extends FieldChange>(
	fields: readonly WorkbenchField[],
	changes: readonly T[],
	number: (text: string) => string,
): readonly T[] {
	const keys = numberKeysOf(fields);
	if (keys.size === 0) return changes;
	return changes.map((change) =>
		keys.has(change.name)
			? { ...change, from: number(change.from), to: number(change.to) }
			: change,
	);
}
