const PLAIN_NUMBER = /^-?\d+(\.\d+)?$/;
const CURRENCY_FIRST = /^(?:€|\$|£|CHF|EUR|USD|GBP)/i;
const CURRENCY_LAST = /(?:€|\$|£|CHF|EUR|USD|GBP)$/i;

type Range = readonly [number, number];
type DecimalSign = "." | ",";

function decimalsOf(text: string) {
	return (text.replace(",", ".").split(".")[1] ?? "").length;
}

function numberOrNull(text: string) {
	const trimmed = text.trim().replace(",", ".");
	return PLAIN_NUMBER.test(trimmed) ? Number(trimmed) : null;
}

function clamp(value: number, range: Range) {
	return Math.min(range[1], Math.max(range[0], value));
}

/** Empty text starts from the default, else from 0 (inside the range). */
function startOf(fallback: number | null, range: Range | null) {
	if (fallback !== null) return fallback;
	return range ? clamp(0, range) : 0;
}

/** Within the pin's range; without one, a value that starts at 0 or more with a default of 0 or more stops at 0. */
function bounded(
	next: number,
	base: number,
	fallback: number | null,
	range: Range | null,
) {
	if (range) return clamp(next, range);
	const neverNegative = base >= 0 && (fallback === null || fallback >= 0);
	return neverNegative && next < 0 ? 0 : next;
}

/**
 * ↑ / ↓ on a number field (spec S3, `flpNudge`): ± one step (1 unless the pin has one), ± ten steps with
 * Shift (`big`). Empty text starts from the default; the result clamps to the pin's range, or without one stops
 * at 0 when the value and the default are 0 or more. Typed decimals are kept ("11769.10" ↑ → "11770.10").
 * Text that is not a plain number comes back unchanged.
 */
export function nudge(
	text: string,
	dir: 1 | -1,
	big: boolean,
	integer: boolean,
	defaultText: string,
	range: Range | null,
	step: number | null,
) {
	const typed = text.trim();
	const fallback = numberOrNull(defaultText);
	const base = typed === "" ? startOf(fallback, range) : numberOrNull(typed);
	if (base === null) return text;
	const unit = step !== null && step > 0 ? step : 1;
	const decimals = Math.max(decimalsOf(typed), decimalsOf(String(unit)));
	const moved = Number((base + dir * (big ? 10 : 1) * unit).toFixed(decimals));
	const next = bounded(moved, base, fallback, range);
	if (integer) return String(Math.round(next));
	return decimals ? next.toFixed(decimals) : String(next);
}

function countOf(text: string, sign: string) {
	return text.split(sign).length - 1;
}

/** Group signs every three digits after a first group of one to three digits. */
function groupsFit(text: string, sign: string) {
	const groups = text.split(sign);
	return (
		/^[1-9]\d{0,2}$/.test(groups[0]) &&
		groups.slice(1).every((group) => /^\d{3}$/.test(group))
	);
}

interface AmountParts {
	readonly whole: string;
	readonly fraction: string;
}

const grouped = (text: string, sign: string): AmountParts | null =>
	groupsFit(text, sign)
		? { whole: text.split(sign).join(""), fraction: "" }
		: null;

/** Both signs: the last one is the decimal sign. */
const bothSigns = (text: string): AmountParts | null => {
	const decimal = text.lastIndexOf(".") > text.lastIndexOf(",") ? "." : ",";
	const group = decimal === "." ? "," : ".";
	const parts = text.split(decimal);
	if (parts.length !== 2 || !groupsFit(parts[0], group)) return null;
	return { whole: parts[0].split(group).join(""), fraction: parts[1] };
};

/** With exactly three digits after it: a decimal after a 0, a group sign in a whole-number field, else the viewer's sign decides. */
function readsAsDecimal(
	before: string,
	after: string,
	sign: string,
	integer: boolean,
	decimalSign: DecimalSign,
) {
	if (after.length !== 3 || before === "0" || before === "") return true;
	return !integer && sign === decimalSign;
}

/** One sign once: a decimal unless exactly three digits follow (`readsAsDecimal`). */
const oneSign = (
	text: string,
	integer: boolean,
	decimalSign: DecimalSign,
): AmountParts | null => {
	const sign = text.includes(".") ? "." : ",";
	const [before, after] = text.split(sign);
	if (!before && !after) return null;
	return readsAsDecimal(before, after, sign, integer, decimalSign)
		? { whole: before || "0", fraction: after }
		: grouped(text, sign);
};

const splitAmount = (
	text: string,
	integer: boolean,
	decimalSign: DecimalSign,
): AmountParts | null => {
	const dots = countOf(text, ".");
	const commas = countOf(text, ",");
	if (dots > 0 && commas > 0) return bothSigns(text);
	if (dots + commas > 1) return grouped(text, dots > 0 ? "." : ",");
	if (dots + commas === 1) return oneSign(text, integer, decimalSign);
	return { whole: text, fraction: "" };
};

/** Without currency marks, spaces (also no-break and thin ones), ’ and '. */
function bareAmount(text: string) {
	return text
		.replace(/\s/g, "")
		.replace(/[’\x27]/g, "")
		.replace(CURRENCY_FIRST, "")
		.replace(CURRENCY_LAST, "");
}

function plainParts(parts: AmountParts, integer: boolean) {
	if (!/^\d+$/.test(parts.whole)) return false;
	if (!parts.fraction) return true;
	return !integer && /^\d+$/.test(parts.fraction);
}

/**
 * A pasted amount as a plain number (spec S3, `flpCleanAmount`): currency marks, spaces, ’ and ' go; both
 * signs → the last is the decimal sign; one sign several times → group signs; one sign once → `oneSign`.
 * null when it is not a number, still ambiguous, or has decimals in a whole-number field: the text is then
 * pasted as it is and the field's message asks.
 */
export function cleanAmount(
	text: string,
	integer: boolean,
	decimalSign: DecimalSign,
) {
	const bare = bareAmount(text);
	if (!/^-?[\d.,]+$/.test(bare)) return null;
	const sign = bare.startsWith("-") ? "-" : "";
	const parts = splitAmount(bare.slice(sign.length), integer, decimalSign);
	if (!parts || !plainParts(parts, integer)) return null;
	const whole = parts.whole.replace(/^0+(?=\d)/, "");
	return parts.fraction
		? `${sign}${whole}.${parts.fraction}`
		: `${sign}${whole}`;
}

/** The viewer's decimal sign from `Intl.NumberFormat(locale).formatToParts(1.1)`; "." when unknown. */
export const decimalSignOf = (locale: string): DecimalSign => {
	try {
		const part = new Intl.NumberFormat(locale)
			.formatToParts(1.1)
			.find((candidate) => candidate.type === "decimal");
		return part?.value === "," ? "," : ".";
	} catch {
		return ".";
	}
};
