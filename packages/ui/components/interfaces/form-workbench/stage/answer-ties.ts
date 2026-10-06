/*
 * Display ties for the answer (canvas `wbAnswer`): a date, an amount and a percentage stay on one line
 * because their spaces become no-break spaces ("17 Oct 2026", "€ 714.00", "19 %"). Two more keep a line
 * from ending badly. The canvas gets that from `text-wrap: pretty` on flat markup; on the static editor's
 * nested spans Chrome breaks otherwise, so the ties do it: the label of a date or amount that closes the
 * line stays with it ("Due: 17 Oct 2026" is not left as a lone value), and a dot between two items stays
 * with the word before it, so no line starts with one. A line of prose also keeps its last two words
 * together, so a paragraph never ends on one word alone. The renderer gets the tied text; Copy answer keeps
 * the original. Code and table rows are left as written.
 */

const NBSP = "\u{a0}";

/** A day, a capitalised month in full or abbreviated, a year; the day is no list number ("2. October"). */
const DATE = /(^|\D)(\d{1,2}) (\p{Lu}\p{Ll}{2,8}\.?) (\d{4})(?!\d)/gu;
const AMOUNT = /([€$£¥]) (?=\d)/gu;
const SIGN = /(\d) (?=[%€])/gu;
const DATE_VALUE = `\\d{1,2}${NBSP}\\p{Lu}\\p{Ll}{2,8}\\.?${NBSP}\\d{4}`;
const AMOUNT_VALUE = `[€$£¥]${NBSP}\\d[\\d.,]*`;
/** A tied date or amount, then only closing marks (a full stop, a bracket, bold) up to the end of the line. */
const LAST_VALUE = `(?:${DATE_VALUE}[.,]?|${AMOUNT_VALUE})[\\s;:!?)*_]*$`;
const LABEL = new RegExp(`(:(?:\\*\\*|__)?) (?=${LAST_VALUE})`, "gu");
const DOT = /(\S) (?=[·•](?: |$))/gu;

const tie = (text: string) =>
	text
		.replace(DATE, `$1$2${NBSP}$3${NBSP}$4`)
		.replace(AMOUNT, `$1${NBSP}`)
		.replace(SIGN, `$1${NBSP}`)
		.replace(LABEL, `$1${NBSP}`)
		.replace(DOT, `$1${NBSP}`);

const FENCE = /^ {0,3}(`{3,}|~{3,})/;
const CODE_SPAN = /(`+[^`\n]+`+)/;
/** A table row, or a line indented as code. */
const UNTIED_LINE = /^(?:\s*\||\t| {4})/;
/** The last two words of a line when both are plain words, not already tied to a value. */
const LAST_WORDS = /(^| )([^ \u{a0}]+) ([^ \u{a0}]+ *)$/u;
const MIN_WORDS = 3;

/**
 * Ties the last word to the one before it, when the line keeps at least one other place to break and
 * neither word is part of a tied date, amount or label, so no long unbreakable run comes of it.
 */
function tieLastWord(line: string, parts: readonly string[]) {
	const tied = parts.join("");
	if (UNTIED_LINE.test(line) || tied.trim().split(/ +/).length < MIN_WORDS)
		return tied;
	const last = parts.length - 1;
	return parts
		.map((part, index) =>
			index === last ? part.replace(LAST_WORDS, `$1$2${NBSP}$3`) : part,
		)
		.join("");
}

/** Closing a fence needs the same character, at least as many times. */
const closes = (fence: string, line: string) => {
	const found = FENCE.exec(line)?.[1];
	return (
		found !== undefined && found[0] === fence[0] && found.length >= fence.length
	);
};

export function tieAnswer(markdown: string): string {
	let fence: string | null = null;
	return markdown
		.split("\n")
		.map((line) => {
			if (fence !== null) {
				if (closes(fence, line)) fence = null;
				return line;
			}
			const opening = FENCE.exec(line)?.[1];
			if (opening !== undefined) {
				fence = opening;
				return line;
			}
			const parts = line
				.split(CODE_SPAN)
				.map((part, index) => (index % 2 === 0 ? tie(part) : part));
			return tieLastWord(line, parts);
		})
		.join("\n");
}
