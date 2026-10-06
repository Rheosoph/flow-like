/*
 * What a run sends for its inputs (PLAN §3.3 step 5). FlowPath fields send FlowPath objects and
 * never a URL; legacy PathBuf/Byte fields send URL strings; scalar dates and lists of dates send
 * `YYYY-MM-DDT00:00:00Z`, an object property follows its schema format; switches are always sent;
 * an empty optional field is left out so the server fills its default. Scalar and JSON parsing is
 * `lib/event-form.ts`'s.
 */
import {
	type EventFormFieldSpec,
	type FieldProblem as EventFormProblem,
	type FieldResult,
	fieldPayload,
	isFieldDate,
} from "../../../../lib/event-form";
import type {
	CopyValue,
	FieldKey,
	FieldKind,
	FieldProblem,
	FieldProblemCode,
	FileRef,
	FileSlot,
	FileSlotState,
	PayloadResult,
	WorkbenchField,
} from "../contracts";
import {
	decimalText,
	filledRows,
	groupOf,
	isEmpty,
	isHiddenValue,
	isRecord,
	listOf,
	readsAsJson,
	slotsOf,
	textOf,
} from "./values";

/** How one field goes into the run: a value, left out, or why it cannot be sent (by FieldKey). */
export type Sent =
	| { readonly kind: "value"; readonly value: unknown }
	| { readonly kind: "omit" }
	| {
			readonly kind: "problems";
			readonly problems: Readonly<Record<FieldKey, FieldProblem>>;
	  };

export interface SendOptions {
	/** The viewer's decimal sign while validating; without it one lone comma reads as a decimal comma. */
	readonly decimalSign?: "." | ",";
}

type Send = (
	field: WorkbenchField,
	value: CopyValue,
	options: SendOptions,
) => Sent;

const OMIT: Sent = { kind: "omit" };
const DAY = /^\d{4}-\d{2}-\d{2}$/;

const sent = (value: unknown): Sent => ({ kind: "value", value });

function problem(field: WorkbenchField, found: FieldProblem): Sent {
	return { kind: "problems", problems: { [field.key]: found } };
}

const code = (name: FieldProblemCode): FieldProblem => ({ code: name });

function missing(field: WorkbenchField): Sent {
	if (!field.required) return OMIT;
	return problem(
		field,
		code(field.kind === "unsupported" ? "unsupported" : "required"),
	);
}

/** The shape `lib/event-form.ts` parses, for this field's own data and value type. */
function formSpec(
	field: WorkbenchField,
	rest: Partial<EventFormFieldSpec> = {},
): EventFormFieldSpec {
	return {
		name: field.name,
		data_type: field.dataType,
		value_type: field.valueType,
		optional: true,
		sensitive: false,
		default: null,
		options: null,
		...rest,
	};
}

const EVENT_FORM_CODES: Readonly<Record<EventFormProblem, FieldProblemCode>> = {
	required: "required",
	integer: "integer",
	number: "number",
	date: "date",
	json: "json",
	object: "object",
	array: "array",
	unique: "unique",
	items: "items",
	option: "option",
	file: "fileFailed",
	unknown: "unsupported",
};

function fromEventForm(field: WorkbenchField, result: FieldResult): Sent {
	if (!result.ok) return problem(field, code(EVENT_FORM_CODES[result.problem]));
	return "value" in result ? sent(result.value) : missing(field);
}

/** A day as a run sends it: an instant for scalar dates and lists, the day for `format: date`. */
export function dateWire(field: WorkbenchField, day: string): string {
	if (!DAY.test(day) || field.dateFormat === "date") return day;
	return `${day}T00:00:00Z`;
}

// ─── Numbers ────────────────────────────────────────────────────────────────

const FRACTION = /^[+-]?\d*\.\d+$/;

function numberInput(text: string, sign: SendOptions["decimalSign"]): string {
	return sign === "." ? text : decimalText(text);
}

function inRange(field: WorkbenchField, number: number): FieldProblem | null {
	const range = field.range;
	if (!range || (number >= range[0] && number <= range[1])) return null;
	return { code: "range", min: range[0], max: range[1] };
}

/** A typed number as the run sends it, or why it cannot be sent (strict parsing, then the pin's range). */
export function readNumber(
	field: WorkbenchField,
	text: string,
	sign?: SendOptions["decimalSign"],
): number | FieldProblem {
	const input = numberInput(text.trim(), sign);
	const dataType = field.integer ? "Integer" : "Float";
	const spec = formSpec(field, { data_type: dataType, value_type: "Normal" });
	const result = fieldPayload(spec, input);
	if (!result.ok || !("value" in result))
		return code(
			field.integer && FRACTION.test(input) ? "noDecimals" : notNumber(field),
		);
	const number = Number(result.value);
	return inRange(field, number) ?? number;
}

const notNumber = (field: WorkbenchField): FieldProblemCode =>
	field.integer ? "integer" : "number";

function numberSent(
	field: WorkbenchField,
	value: CopyValue,
	options: SendOptions,
): Sent {
	const read = readNumber(field, textOf(value), options.decimalSign);
	return typeof read === "number" ? sent(read) : problem(field, read);
}

// ─── Text, dates, choices, JSON ─────────────────────────────────────────────

function dateSent(field: WorkbenchField, value: CopyValue): Sent {
	const day = textOf(value).trim();
	return isFieldDate(day)
		? sent(dateWire(field, day))
		: problem(field, code("date"));
}

function choiceSent(field: WorkbenchField, value: CopyValue): Sent {
	const spec = formSpec(field, {
		value_type: "Normal",
		options: field.options,
	});
	const result = fromEventForm(field, fieldPayload(spec, textOf(value).trim()));
	if (result.kind !== "value" || field.dataType !== "Date") return result;
	return sent(dateWire(field, String(result.value)));
}

function parsedJson(text: string): { readonly value: unknown } | null {
	try {
		return { value: JSON.parse(text) };
	} catch {
		return null;
	}
}

/** A single Geometry is a GeoJSON object; the start node refuses text. */
function geometrySent(field: WorkbenchField, text: string) {
	const parsed = parsedJson(text);
	if (!parsed) return problem(field, code("json"));
	return isRecord(parsed.value)
		? sent(parsed.value)
		: problem(field, code("object"));
}

/** A JSON box. A single Generic value that is not JSON is sent as the text itself. */
function jsonSent(field: WorkbenchField, value: CopyValue) {
	const text = textOf(value);
	if (field.valueType !== "Normal")
		return fromEventForm(field, fieldPayload(formSpec(field), text));
	if (field.dataType === "Generic")
		return sent(parsedJson(text)?.value ?? text);
	if (field.dataType === "Geometry") return geometrySent(field, text);
	return fromEventForm(field, fieldPayload(formSpec(field), text));
}

// ─── Lists ──────────────────────────────────────────────────────────────────

const CHIP_VALUE: Readonly<
	Record<
		"text" | "number" | "date",
		(field: WorkbenchField, item: string) => unknown
	>
> = {
	text: (field, item) =>
		field.options && !field.options.includes(item) ? undefined : item,
	number: numberItem,
	date: dateItem,
};

function chipsSent(field: WorkbenchField, value: CopyValue) {
	const convert = CHIP_VALUE[field.itemKind ?? "text"];
	const items = listOf(value).map((item) => convert(field, item));
	if (items.some((item) => item === undefined))
		return problem(
			field,
			code(field.options && field.itemKind === "text" ? "option" : "items"),
		);
	const unique =
		new Set(items.map((item) => JSON.stringify(item))).size === items.length;
	return field.valueType === "HashSet" && !unique
		? problem(field, code("unique"))
		: sent(items);
}

// ─── Name/value rows ────────────────────────────────────────────────────────

/** A free object's value: text that reads as JSON is that JSON, anything else is text. */
function freeValue(text: string) {
	if (!readsAsJson(text)) return text;
	return parsedJson(text.trim())?.value ?? text;
}

function numberItem(field: WorkbenchField, text: string) {
	const read = readNumber(field, text);
	return typeof read === "number" ? read : undefined;
}

function booleanItem(_field: WorkbenchField, text: string) {
	const word = text.trim().toLowerCase();
	if (word === "true") return true;
	return word === "false" ? false : undefined;
}

function dateItem(field: WorkbenchField, text: string) {
	const day = text.trim();
	return isFieldDate(day) ? dateWire(field, day) : undefined;
}

const PAIR_VALUE: Readonly<
	Record<string, (field: WorkbenchField, text: string) => unknown>
> = {
	Struct: (_field, text) => freeValue(text),
	String: (_field, text) => text,
	Integer: numberItem,
	Float: numberItem,
	Boolean: booleanItem,
	Date: dateItem,
};

function pairsSent(field: WorkbenchField, value: CopyValue) {
	const object: Record<string, unknown> = {};
	const convert = PAIR_VALUE[field.dataType] ?? PAIR_VALUE.String;
	for (const row of filledRows(value)) {
		const key = row.key.trim();
		if (Object.hasOwn(object, key)) return problem(field, code("unique"));
		const item = key === "" ? undefined : convert(field, row.value);
		if (item === undefined) return problem(field, code("items"));
		object[key] = item;
	}
	return sent(object);
}

// ─── Objects ────────────────────────────────────────────────────────────────

function groupSent(field: WorkbenchField, value: CopyValue): Sent {
	const object: Record<string, unknown> = {};
	const problems: Record<FieldKey, FieldProblem> = {};
	const group = groupOf(value);
	for (const prop of field.props) {
		const result = sentValue(prop, group[prop.name]);
		if (result.kind === "value") object[prop.name] = result.value;
		if (result.kind === "problems") Object.assign(problems, result.problems);
	}
	return Object.keys(problems).length > 0
		? { kind: "problems", problems }
		: sent(object);
}

// ─── Files ──────────────────────────────────────────────────────────────────

const SLOT_PROBLEM: Readonly<Partial<Record<FileSlotState, FieldProblemCode>>> =
	{
		reminder: "pickAgain",
		failed: "fileFailed",
		waiting: "fileSending",
		sending: "fileSending",
	};

function flowPathOf(ref: FileRef | null) {
	if (ref?.kind !== "flowpath") return null;
	const { path, store_ref, cache_store_ref } = ref.flowPath;
	return { path, store_ref, cache_store_ref: cache_store_ref ?? null };
}

function urlOf(ref: FileRef | null) {
	if (ref?.kind === "url") return ref.url;
	return ref?.kind === "flowpath" ? ref.url : null;
}

type Wire =
	| { readonly ok: true; readonly value: unknown }
	| { readonly ok: false; readonly problem: FieldProblem };

/** One slot as the run sends it. A FlowPath field never falls back to a URL. */
const slotWire = (field: WorkbenchField, slot: FileSlot): Wire => {
	const blocked = SLOT_PROBLEM[slot.state];
	if (blocked)
		return { ok: false, problem: { code: blocked, fileName: slot.name } };
	const value =
		field.fileMode === "flowpath" ? flowPathOf(slot.ref) : urlOf(slot.ref);
	if (value !== null) return { ok: true, value };
	const notYet = slot.ref?.kind === "inline" ? "fileSending" : "fileFailed";
	return { ok: false, problem: { code: notYet, fileName: slot.name } };
};

function fileSent(field: WorkbenchField, value: CopyValue) {
	const slots = slotsOf(value);
	if (slots.length === 0) return missing(field);
	const wires = slots.map((slot) => slotWire(field, slot));
	const bad = wires.find((wire) => !wire.ok);
	if (bad && !bad.ok) return problem(field, bad.problem);
	const values = wires.map((wire) => (wire.ok ? wire.value : null));
	return sent(field.kind === "file" ? values[0] : values);
}

// ─── Every kind ─────────────────────────────────────────────────────────────

const whenFilled =
	(convert: Send): Send =>
	(field, value, options) =>
		isEmpty(field, value) ? missing(field) : convert(field, value, options);

const SEND: Readonly<Record<FieldKind, Send>> = {
	text: whenFilled((_field, value) => sent(textOf(value))),
	number: whenFilled(numberSent),
	date: whenFilled(dateSent),
	choice: whenFilled(choiceSent),
	json: whenFilled(jsonSent),
	chips: whenFilled(chipsSent),
	unsupported: whenFilled(missing),
	bool: (_field, value) => sent(value === true),
	file: fileSent,
	files: fileSent,
	group: groupSent,
	pairs: pairsSent,
};

/**
 * One field as the run sends it. A hidden value cannot be sent ("Enter {label} again"); a value
 * that is not there at all is the field's default, as `fieldPayload` seeds it.
 */
export function sentValue(
	field: WorkbenchField,
	value: CopyValue | undefined,
	options: SendOptions = {},
): Sent {
	if (isHiddenValue(value)) return problem(field, code("enterAgain"));
	return SEND[field.kind](
		field,
		value === undefined ? field.defaultValue : value,
		options,
	);
}

/** `BuildPayload` (PLAN §3.3 step 5): the run's input by pin name, or every problem by FieldKey. */
export function buildPayload(
	fields: readonly WorkbenchField[],
	values: Readonly<Record<string, CopyValue>>,
): PayloadResult {
	const payload: Record<string, unknown> = {};
	const problems: Record<FieldKey, FieldProblem> = {};
	for (const field of fields) {
		const result = sentValue(field, values[field.name]);
		if (result.kind === "value") payload[field.name] = result.value;
		if (result.kind === "problems") Object.assign(problems, result.problems);
	}
	return Object.keys(problems).length > 0
		? { ok: false, problems }
		: { ok: true, payload };
}
