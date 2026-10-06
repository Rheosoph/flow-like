/*
 * A run's input as the stage reads it: the long form for the Inputs list ("Lieferschein LS-77120.pdf"
 * and "PO-48213.pdf" on two lines, an object's properties joined by " · ", numbers grouped) and which
 * values stay masked. The change chips and the Compare table use the model's short form (`diffValues`)
 * with the same number reading (`withNumberText`). Pure.
 */
import {
	type CopyValue,
	SECRET_MASK,
	type ShortWords,
	type WorkbenchField,
} from "../contracts";
import { type FormWords, formatWhen } from "../model/date-text";
import { isSecretField, looksSecret } from "../model/secrets";
import {
	type IsSecret,
	filledRows,
	groupOf,
	isHiddenValue,
	listOf,
	slotsOf,
	textOf,
} from "../model/values";

/** A text longer than this is cut in the Inputs list. */
export const INPUT_TEXT_MAX = 600;
const JSON_TEXT_MAX = 200;
const NBSP = " ";

/** `IsSecret` for a form: a flagged or secret-named field, one "Don't save" covers, or a key-like value. */
export const secretCheckOf =
	(noSave: readonly string[]): IsSecret =>
	(field, value) =>
		isSecretField(field, noSave) || looksSecret(textOf(value));

export interface TextContext {
	readonly words: ShortWords;
	readonly isSecret: IsSecret;
	/** A typed number in the viewer's reading (`formatTypedNumber`); as typed without it. */
	readonly number?: (text: string) => string;
}

function clipped(text: string, max: number) {
	return text.length > max ? `${text.slice(0, max - 1).trimEnd()}…` : text;
}

type LongText = (
	field: WorkbenchField,
	value: CopyValue | undefined,
	context: TextContext,
) => string;

const plainText: LongText = (_field, value) =>
	clipped(textOf(value).trim(), INPUT_TEXT_MAX);

const numberText: LongText = (field, value, context) =>
	context.number
		? clipped(context.number(textOf(value)), INPUT_TEXT_MAX)
		: plainText(field, value, context);

/** "Net days 14": the property's label and value, never broken between them; a secret or kept-out property reads "••••". */
function propertyText(
	prop: WorkbenchField,
	value: CopyValue | undefined,
	context: TextContext,
) {
	const shown =
		isHiddenValue(value) || context.isSecret(prop, value)
			? SECRET_MASK
			: longTextOf(prop, value, context);
	return `${prop.label} ${shown || context.words.empty}`.replaceAll(" ", NBSP);
}

const LONG_TEXT: Readonly<Record<WorkbenchField["kind"], LongText>> = {
	text: plainText,
	number: numberText,
	choice: plainText,
	date: (_field, value, { words }) => {
		const iso = textOf(value).trim();
		return iso === "" ? "" : words.date(iso);
	},
	bool: (_field, value, { words }) => (value === true ? words.on : words.off),
	chips: (_field, value) => listOf(value).join(", "),
	file: (_field, value) => slotsOf(value)[0]?.name ?? "",
	files: (_field, value) =>
		slotsOf(value)
			.map((slot) => slot.name)
			.join("\n"),
	group: (field, value, context) =>
		field.props
			.map((prop) => propertyText(prop, groupOf(value)[prop.name], context))
			.join(" · "),
	pairs: (_field, value) =>
		filledRows(value)
			.map((row) => `${row.key}: ${row.value}`)
			.join("\n"),
	json: (_field, value) =>
		clipped(textOf(value).replace(/\s+/g, " ").trim(), JSON_TEXT_MAX),
	unsupported: () => "",
};

/** The long text of a value; "" when it is empty. A hidden value has none (see `valueTextOf`). */
export function longTextOf(
	field: WorkbenchField,
	value: CopyValue | undefined,
	context: TextContext,
) {
	return LONG_TEXT[field.kind](field, value, context);
}

/** One input row of a run's Inputs list: a value, nothing, or a value kept off this device. */
export type ValueText =
	| { readonly kind: "value"; readonly text: string }
	| { readonly kind: "empty" }
	| { readonly kind: "hidden" };

const EMPTY_TEXT: ValueText = { kind: "empty" };

export function valueTextOf(
	field: WorkbenchField,
	value: CopyValue | undefined,
	context: TextContext,
): ValueText {
	if (isHiddenValue(value)) return { kind: "hidden" };
	const text = longTextOf(field, value, context);
	if (text === "") return EMPTY_TEXT;
	const masked = context.isSecret(field, value);
	return { kind: "value", text: masked ? SECRET_MASK : text };
}

/** When a run was pressed, split for the overflow menu: "Today" / "Yesterday" / "Mon 28 Sep" and the time of day. */
export interface RunMoment {
	readonly day: string;
	readonly time: string;
}

export const momentOf = (
	at: number,
	now: number,
	locale: string,
	words: FormWords,
): RunMoment => {
	const when = formatWhen(at, now, locale, words);
	return {
		day: when.startsWith(`${words.today} `) ? words.today : when,
		time: new Intl.DateTimeFormat(locale, { timeStyle: "short" }).format(at),
	};
};
