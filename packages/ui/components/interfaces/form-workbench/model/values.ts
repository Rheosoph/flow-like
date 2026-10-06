/*
 * Rail values per field kind (PLAN §3.3 step 2): what counts as empty, comparison keys, short text,
 * starting values (defaults under the active preset) and the safe conversions of saved values
 * (spec §5). Pure; the only imports are the contracts and `lib/event-form.ts`.
 */
import { isFieldDate } from "../../../../lib/event-form";
import {
	type CopyValue,
	FIELD_KEY_SEPARATOR,
	type FieldChange,
	type FieldKey,
	type FieldKind,
	type FieldValue,
	type FieldValues,
	type FileSlot,
	type FileSlotState,
	type GroupValue,
	type HiddenValue,
	type PairRow,
	type Preset,
	SECRET_MASK,
	type ShortWords,
	type WorkbenchField,
} from "../contracts";

/** Whether a field, or a value at one of its keys, is kept out of sight ("••••"). The caller folds in noSave and looksSecret. */
export type IsSecret = (field: WorkbenchField, value?: CopyValue) => boolean;

type ValueRecord = Readonly<Record<string, CopyValue>>;

const SLOT_STATES: ReadonlySet<string> = new Set<FileSlotState>([
	"waiting",
	"sending",
	"sent",
	"failed",
	"reminder",
]);
const FILE_KINDS: ReadonlySet<FieldKind> = new Set<FieldKind>([
	"file",
	"files",
]);
const HIDDEN_KEY = "\u0000hidden";
export const HIDDEN_VALUE: HiddenValue = { $hidden: true };

// ─── Shapes ─────────────────────────────────────────────────────────────────

export function isRecord(
	value: unknown,
): value is Readonly<Record<string, unknown>> {
	return typeof value === "object" && value !== null && !Array.isArray(value);
}

/** `{ $hidden: true }`: a value kept out of storage (a run's copy read back, a stored mark). */
export function isHiddenValue(value: unknown): value is HiddenValue {
	return isRecord(value) && value.$hidden === true;
}

export function isFileSlot(value: unknown): value is FileSlot {
	return (
		isRecord(value) &&
		typeof value.id === "string" &&
		typeof value.name === "string" &&
		typeof value.state === "string" &&
		SLOT_STATES.has(value.state)
	);
}

function isPairRow(value: unknown): value is PairRow {
	return (
		isRecord(value) &&
		typeof value.id === "string" &&
		typeof value.key === "string" &&
		typeof value.value === "string"
	);
}

export function isFileField(field: WorkbenchField): boolean {
	return FILE_KINDS.has(field.kind);
}

/** The text of a text-like value; numbers and switches spelled out, anything else empty. */
export function textOf(value: unknown): string {
	if (typeof value === "string") return value;
	if (typeof value === "number" || typeof value === "boolean")
		return String(value);
	return "";
}

const blank = (value: unknown) => textOf(value).trim() === "";
const isString = (value: unknown): value is string => typeof value === "string";

export function listOf(value: unknown): readonly string[] {
	return Array.isArray(value) ? value.filter(isString) : [];
}

/** The file slots a value holds: one for a one-file field, all of them for several. */
export function slotsOf(value: unknown): readonly FileSlot[] {
	if (Array.isArray(value)) return value.filter(isFileSlot);
	return isFileSlot(value) ? [value] : [];
}

export const isReminder = (slot: FileSlot) => slot.state === "reminder";

/** A "Pick again" row (S4): a file of an older run, which is not a value. */
export function hasReminder(value: CopyValue | undefined): boolean {
	return slotsOf(value).some(isReminder);
}

/** A group's properties; anything else reads as no properties. */
export function groupOf(value: unknown): GroupValue {
	return isRecord(value) && !isHiddenValue(value) && !isFileSlot(value)
		? (value as GroupValue)
		: {};
}

const filledRow = (row: PairRow) =>
	row.key.trim() !== "" || row.value.trim() !== "";

/** Name/value rows that hold something (an empty row is no entry). */
export function filledRows(value: unknown): readonly PairRow[] {
	return Array.isArray(value) ? value.filter(isPairRow).filter(filledRow) : [];
}

// ─── Keys ───────────────────────────────────────────────────────────────────

/** `name`, or `group␟property` for an object's property (FIELD_KEY_SEPARATOR). */
export function fieldKey(group: string | null, name: string): FieldKey {
	return group === null ? name : `${group}${FIELD_KEY_SEPARATOR}${name}`;
}

/** The field name and, for an object's property, the property name. */
export function splitKey(key: FieldKey): {
	readonly name: string;
	readonly property: string | null;
} {
	const at = key.indexOf(FIELD_KEY_SEPARATOR);
	if (at < 0) return { name: key, property: null };
	return {
		name: key.slice(0, at),
		property: key.slice(at + FIELD_KEY_SEPARATOR.length),
	};
}

/** The value at a FieldKey: a field's value, or one property of an object. */
export function valueAt(
	values: FieldValues,
	key: FieldKey,
): FieldValue | undefined;
export function valueAt(
	values: ValueRecord,
	key: FieldKey,
): CopyValue | undefined;
export function valueAt(
	values: ValueRecord,
	key: FieldKey,
): CopyValue | undefined {
	const { name, property } = splitKey(key);
	const value = values[name];
	return property === null ? value : groupOf(value)[property];
}

/** The values with one key set; an object's property is set inside its group. */
export function withValue(
	values: FieldValues,
	key: FieldKey,
	value: FieldValue,
): FieldValues {
	const { name, property } = splitKey(key);
	if (property === null) return { ...values, [name]: value };
	return { ...values, [name]: { ...groupOf(values[name]), [property]: value } };
}

// ─── Empty values and emptiness ─────────────────────────────────────────────

function propsValue(
	field: WorkbenchField,
	propValue: (prop: WorkbenchField) => FieldValue,
): GroupValue {
	return Object.fromEntries(
		field.props.map((prop) => [prop.name, propValue(prop)]),
	);
}

const EMPTY_VALUE: Readonly<
	Record<FieldKind, (field: WorkbenchField) => FieldValue>
> = {
	text: () => "",
	number: () => "",
	date: () => "",
	choice: () => "",
	json: () => "",
	bool: () => false,
	chips: () => [],
	file: () => null,
	files: () => [],
	group: (field) => propsValue(field, emptyValue),
	pairs: () => [],
	unsupported: () => null,
};

/** What a field holds when nothing is entered. */
export function emptyValue(field: WorkbenchField): FieldValue {
	return EMPTY_VALUE[field.kind](field);
}

const never = () => false;
const noValueSlot = (value: unknown) =>
	!slotsOf(value).some((slot) => !isReminder(slot));

const IS_EMPTY: Readonly<Record<FieldKind, (value: unknown) => boolean>> = {
	text: blank,
	number: blank,
	date: blank,
	choice: blank,
	json: blank,
	bool: never,
	group: never,
	pairs: never,
	chips: (value) => listOf(value).length === 0,
	file: noValueSlot,
	files: noValueSlot,
	unsupported: () => true,
};

/**
 * `flpEmpty`. Switches, objects and name/value rows are never empty. A hidden value and a "Pick
 * again" reminder are no values. Whitespace alone is no text.
 */
export function isEmpty(
	field: WorkbenchField,
	value: CopyValue | undefined,
): boolean {
	return isHiddenValue(value) || IS_EMPTY[field.kind](value);
}

// ─── Comparison keys ────────────────────────────────────────────────────────

/** One comma and no dot is a decimal comma: "11769,10" reads as 11769.10. */
export function decimalText(text: string): string {
	return /^[+-]?\d*,\d+$/.test(text) ? text.replace(",", ".") : text;
}

function numberKey(text: string): string {
	const trimmed = text.trim();
	const number = Number(decimalText(trimmed));
	return trimmed !== "" && Number.isFinite(number) ? String(number) : trimmed;
}

function jsonKey(text: string): string {
	try {
		return JSON.stringify(JSON.parse(text));
	} catch {
		return text.trim();
	}
}

const trimmedKey = (_field: WorkbenchField, value: unknown) =>
	textOf(value).trim();
const nameOf = (slot: FileSlot) => slot.name;
const rowKey = (row: PairRow) => [row.key.trim(), row.value];

const KEY_OF: Readonly<
	Record<FieldKind, (field: WorkbenchField, value: unknown) => string>
> = {
	text: trimmedKey,
	date: trimmedKey,
	choice: trimmedKey,
	number: (_field, value) => numberKey(textOf(value)),
	json: (_field, value) => jsonKey(textOf(value)),
	bool: (_field, value) => (value === true ? "1" : "0"),
	chips: (_field, value) => JSON.stringify(listOf(value)),
	file: (_field, value) => slotsOf(value).map(nameOf).join(""),
	files: (_field, value) => JSON.stringify(slotsOf(value).map(nameOf)),
	group: (field, value) =>
		JSON.stringify(
			field.props.map((prop) => valueKey(prop, groupOf(value)[prop.name])),
		),
	pairs: (_field, value) => JSON.stringify(filledRows(value).map(rowKey)),
	unsupported: () => "",
};

/**
 * `flpKey`: equal keys send the same run. Files by name (sizes ignored, a reminder by its name),
 * numbers by value, JSON by content, rows without their ids, a hidden value by itself.
 */
export function valueKey(
	field: WorkbenchField,
	value: CopyValue | undefined,
): string {
	return isHiddenValue(value) ? HIDDEN_KEY : KEY_OF[field.kind](field, value);
}

/** `flpSame`: whether two value sets would send the same run. */
export function sameValues(
	fields: readonly WorkbenchField[],
	a: ValueRecord | null | undefined,
	b: ValueRecord | null | undefined,
): boolean {
	return fields.every(
		(field) =>
			valueKey(field, a?.[field.name]) === valueKey(field, b?.[field.name]),
	);
}

// ─── Short text ─────────────────────────────────────────────────────────────

function plainShort(
	_field: WorkbenchField,
	value: unknown,
	words: ShortWords,
): string {
	return textOf(value).replace(/\s+/g, " ").trim() || words.empty;
}

function countShort(
	count: number,
	many: (count: number) => string,
	none: string,
) {
	return count > 0 ? many(count) : none;
}

const SHORT: Readonly<
	Record<
		FieldKind,
		(field: WorkbenchField, value: unknown, words: ShortWords) => string
	>
> = {
	text: plainShort,
	number: plainShort,
	choice: plainShort,
	json: (_field, value, words) =>
		jsonKey(textOf(value)).replace(/\s+/g, " ") || words.empty,
	date: (_field, value, words) =>
		blank(value) ? words.empty : words.date(textOf(value).trim()),
	bool: (_field, value, words) => (value === true ? words.on : words.off),
	chips: (_field, value, words) => listOf(value).join(", ") || words.none,
	file: (_field, value, words) => slotsOf(value)[0]?.name ?? words.none,
	files: (_field, value, words) =>
		countShort(slotsOf(value).length, words.files, words.none),
	group: (field, value, words) =>
		field.props
			.map(
				(prop) =>
					`${prop.label} ${shortText(prop, groupOf(value)[prop.name], words)}`,
			)
			.join(" · "),
	pairs: (_field, value, words) =>
		countShort(filledRows(value).length, words.entries, words.none),
	unsupported: (_field, _value, words) => words.none,
};

/** `flpShort`: "4400, 4410", "On", "18 Sep 2026", "2 files"; a hidden value is SECRET_MASK. */
export function shortText(
	field: WorkbenchField,
	value: CopyValue | undefined,
	words: ShortWords,
): string {
	return isHiddenValue(value)
		? SECRET_MASK
		: SHORT[field.kind](field, value, words);
}

// ─── Starting values ────────────────────────────────────────────────────────

/** Every field's default by name (no preset). */
export function defaultsOf(fields: readonly WorkbenchField[]): FieldValues {
	return Object.fromEntries(
		fields.map((field) => [field.name, field.defaultValue]),
	);
}

/** The value a preset sets for a field when it still fits; files are never set. */
export function presetValue(
	field: WorkbenchField,
	preset: Preset | null,
): FieldValue | undefined {
	if (!preset || isFileField(field) || !Object.hasOwn(preset.sets, field.name))
		return undefined;
	return fitValue(field, preset.sets[field.name]);
}

/** What a field goes back to: the active preset's value where it sets one, else the default. */
export function startingValue(
	field: WorkbenchField,
	preset: Preset | null,
): FieldValue {
	return presetValue(field, preset) ?? field.defaultValue;
}

/** Whether a field has a starting value of its own: a default, or one the active preset sets. */
export function hasStartingValue(
	field: WorkbenchField,
	preset: Preset | null,
): boolean {
	return field.hasDefault || presetValue(field, preset) !== undefined;
}

/** `flpBaseline`: every field's starting value by name. */
export function baselineOf(
	fields: readonly WorkbenchField[],
	preset: Preset | null,
): FieldValues {
	return Object.fromEntries(
		fields.map((field) => [field.name, startingValue(field, preset)]),
	);
}

function isChanged(
	field: WorkbenchField,
	value: FieldValue | undefined,
	preset: Preset | null,
	perRun: readonly string[],
): boolean {
	if (perRun.includes(field.name) || !hasStartingValue(field, preset))
		return false;
	return (
		valueKey(field, value) !== valueKey(field, startingValue(field, preset))
	);
}

/**
 * `flpChanged`: the fields that carry the 6 px dot and count in "Changed": they have a starting
 * value of their own, differ from it and are not per run. A typed required field is no change.
 */
export function changedNames(
	fields: readonly WorkbenchField[],
	values: FieldValues,
	preset: Preset | null,
	perRun: readonly string[],
): readonly string[] {
	return fields
		.filter((field) => isChanged(field, values[field.name], preset, perRun))
		.map((field) => field.name);
}

// ─── Differences ────────────────────────────────────────────────────────────

export interface DiffOptions {
	/** Field names left out (per-run fields in a rail-against-run comparison). */
	readonly skip?: readonly string[];
	readonly words: ShortWords;
	readonly isSecret: IsSecret;
}

function changeOf(
	field: WorkbenchField,
	key: FieldKey,
	from: CopyValue | undefined,
	to: CopyValue | undefined,
	options: DiffOptions,
): FieldChange | null {
	if (valueKey(field, from) === valueKey(field, to)) return null;
	const masked = [from, to].some(
		(value) => isHiddenValue(value) || options.isSecret(field, value),
	);
	const text = (value: CopyValue | undefined) =>
		masked ? SECRET_MASK : shortText(field, value, options.words);
	return { name: key, label: field.label, from: text(from), to: text(to) };
}

function fieldChanges(
	field: WorkbenchField,
	from: CopyValue | undefined,
	to: CopyValue | undefined,
	options: DiffOptions,
): readonly (FieldChange | null)[] {
	if (field.kind !== "group" || isHiddenValue(from) || isHiddenValue(to))
		return [changeOf(field, field.key, from, to, options)];
	return field.props.map((prop) =>
		changeOf(
			prop,
			prop.key,
			groupOf(from)[prop.name],
			groupOf(to)[prop.name],
			options,
		),
	);
}

const isChange = (change: FieldChange | null): change is FieldChange =>
	change !== null;

/**
 * What differs from `a` to `b`, in field order; an object's properties one by one. Secret fields
 * and hidden values read SECRET_MASK on both sides.
 */
export function diffValues(
	fields: readonly WorkbenchField[],
	a: ValueRecord,
	b: ValueRecord,
	options: DiffOptions,
): readonly FieldChange[] {
	const skip = options.skip ?? [];
	return fields
		.filter((field) => !skip.includes(field.name))
		.flatMap((field) =>
			fieldChanges(field, a[field.name], b[field.name], options),
		)
		.filter(isChange);
}

// ─── Fitting saved values (spec §5) ─────────────────────────────────────────

type Fit = (field: WorkbenchField, stored: unknown) => FieldValue | undefined;

const NUMBER_TEXT = /^[+-]?(\d+([.,]\d+)?|[.,]\d+)([eE][+-]?\d+)?$/;
const WHOLE_TEXT = /^([+-]?\d+)(?:[.,]0+)?$/;

/** A one-item list stands for its item ("one-item list ↔ text"). */
function soleItem(stored: unknown) {
	return Array.isArray(stored) && stored.length === 1 ? stored[0] : stored;
}

function scalarText(item: unknown) {
	if (typeof item === "string") return item.trim();
	if (typeof item === "number" && Number.isFinite(item)) return String(item);
	return undefined;
}

function numberText(field: WorkbenchField, text: string) {
	if (text === "") return "";
	if (!NUMBER_TEXT.test(text)) return undefined;
	return field.integer ? WHOLE_TEXT.exec(text)?.[1] : text;
}

function dayText(text: string) {
	if (text === "") return "";
	return isFieldDate(text) ? text.slice(0, 10) : undefined;
}

function fitText(_field: WorkbenchField, stored: unknown) {
	const item = soleItem(stored);
	if (typeof item === "string") return item;
	return typeof item === "boolean" ? String(item) : scalarText(item);
}

function fitNumber(field: WorkbenchField, stored: unknown) {
	const text = scalarText(soleItem(stored));
	return text === undefined ? undefined : numberText(field, text);
}

function fitDate(_field: WorkbenchField, stored: unknown) {
	const item = soleItem(stored);
	return typeof item === "string" ? dayText(item.trim()) : undefined;
}

function sameNumber(a: string, b: string) {
	const x = Number(a);
	return Number.isFinite(x) && x === Number(b);
}

function fitChoice(field: WorkbenchField, stored: unknown) {
	const text = scalarText(soleItem(stored));
	if (text === undefined || text === "") return text;
	const options = field.options ?? [];
	const exact = options.find((option) => option === text);
	if (exact !== undefined || field.dataType === "String") return exact;
	return options.find((option) => sameNumber(option, text));
}

const CHIP_ITEM: Readonly<
	Record<
		"text" | "number" | "date",
		(field: WorkbenchField, text: string) => string | undefined
	>
> = {
	text: (_field, text) => text,
	number: numberText,
	date: (_field, text) => dayText(text),
};

function chipItem(field: WorkbenchField, item: unknown) {
	const text = scalarText(item);
	if (text === undefined || text === "") return undefined;
	return CHIP_ITEM[field.itemKind ?? "text"](field, text);
}

function fitChips(field: WorkbenchField, stored: unknown) {
	if (typeof stored === "string" && stored.trim() === "") return [];
	const list = Array.isArray(stored) ? stored : [stored];
	const items = list.map((item) => chipItem(field, item));
	return items.every(isString) ? items : undefined;
}

interface StoredFile {
	readonly $file: { readonly name: string; readonly size: number | null };
}

function isFileMark(value: unknown): value is StoredFile {
	return (
		isRecord(value) &&
		isRecord(value.$file) &&
		typeof value.$file.name === "string"
	);
}

/** A "Pick again" slot for a file an older run had (S4); it has no upload and no File. */
export function reminderSlot(
	id: string,
	name: string,
	size: number | null,
): FileSlot {
	return {
		id,
		name,
		size,
		type: null,
		state: "reminder",
		progress: null,
		ref: null,
		error: null,
		sentAt: null,
		expiresAt: null,
	};
}

function slotFrom(
	field: WorkbenchField,
	item: unknown,
	index: number,
): FileSlot | undefined {
	if (isFileSlot(item)) return item;
	if (!isFileMark(item)) return undefined;
	const { name, size } = item.$file;
	return reminderSlot(
		`reminder:${field.key}:${index}:${name}`,
		name,
		size ?? null,
	);
}

function fitSlots(field: WorkbenchField, stored: unknown) {
	const list = Array.isArray(stored) ? stored : [stored];
	const slots = list.map((item, index) => slotFrom(field, item, index));
	return slots.every(isFileSlot) ? slots : undefined;
}

function fitFile(field: WorkbenchField, stored: unknown) {
	const slots = fitSlots(field, stored);
	if (slots === undefined || slots.length > 1) return undefined;
	return slots[0] ?? null;
}

function fitGroup(field: WorkbenchField, stored: unknown) {
	if (!isRecord(stored) || isFileSlot(stored) || isFileMark(stored))
		return undefined;
	return propsValue(
		field,
		(prop) => fitValue(prop, stored[prop.name]) ?? prop.defaultValue,
	);
}

const JSON_NUMBER = /^-?(0|[1-9]\d*)(\.\d+)?([eE][+-]?\d+)?$/;

/** Whether text in a free object's value would be read as JSON (a number, true, false, null, an object, a list, a quoted string). */
export function readsAsJson(text: string): boolean {
	const trimmed = text.trim();
	return (
		JSON_NUMBER.test(trimmed) ||
		["true", "false", "null"].includes(trimmed) ||
		/^[[{"]/.test(trimmed)
	);
}

/**
 * A map or free object's value as the person reads and types it. In a free object (a Struct
 * without a usable schema) text that would read as JSON is quoted, so it is sent back as text.
 */
export function pairTextOf(field: WorkbenchField, value: unknown): string {
	if (typeof value === "string")
		return field.dataType === "Struct" && readsAsJson(value)
			? JSON.stringify(value)
			: value;
	if (value === undefined) return "";
	return typeof value === "object" ? JSON.stringify(value) : String(value);
}

function rowFrom(item: unknown, index: number): PairRow | undefined {
	if (!isRecord(item) || typeof item.key !== "string") return undefined;
	if (typeof item.value !== "string") return undefined;
	const id = typeof item.id === "string" ? item.id : `saved-${index}`;
	return { id, key: item.key, value: item.value };
}

function fitPairs(
	field: WorkbenchField,
	stored: unknown,
): readonly PairRow[] | undefined {
	if (Array.isArray(stored)) {
		const rows = stored.map(rowFrom);
		return rows.every(isPairRow) ? rows : undefined;
	}
	if (!isRecord(stored) || isFileMark(stored)) return undefined;
	return Object.entries(stored).map(([key, value], index) => ({
		id: `saved-${index}`,
		key,
		value: pairTextOf(field, value),
	}));
}

function fitJson(_field: WorkbenchField, stored: unknown): string {
	return typeof stored === "string" ? stored : JSON.stringify(stored, null, 2);
}

const FIT: Readonly<Record<FieldKind, Fit>> = {
	text: fitText,
	number: fitNumber,
	date: fitDate,
	choice: fitChoice,
	json: fitJson,
	bool: (_field, stored) => (typeof stored === "boolean" ? stored : undefined),
	chips: fitChips,
	file: fitFile,
	files: fitSlots,
	group: fitGroup,
	pairs: fitPairs,
	unsupported: () => undefined,
};

/**
 * A saved value (preset, run record, rail value of an older form) as this field's value, or
 * undefined when it no longer fits. Safe conversions (spec §5): whole number ↔ decimal without a
 * fraction, one-item list ↔ text, text that reads as a number → number, an instant → its day.
 * A value outside the field's fixed choices does not fit. `null` fits as the empty value; files
 * saved as names come back as "Pick again" reminders.
 */
export function fitValue(
	field: WorkbenchField,
	stored: unknown,
): FieldValue | undefined {
	if (stored === undefined || isHiddenValue(stored)) return undefined;
	if (stored === null) return emptyValue(field);
	return FIT[field.kind](field, stored);
}
