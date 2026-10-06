/*
 * Values as this device saves them (spec §5): JSON, files as names and sizes, secrets as
 * `{ $hidden: true }`; and back again, matched by field name with the safe conversions of
 * `fitValue`. Run records, presets and recent values all go through here.
 */
import type {
	CopyValue,
	FieldKey,
	FieldKind,
	FieldValue,
	FileSlot,
	JsonValue,
	StoredFileMark,
	StoredHiddenMark,
	StoredInput,
	WorkbenchField,
} from "../contracts";
import {
	HIDDEN_VALUE,
	emptyValue,
	filledRows,
	fitValue,
	groupOf,
	isHiddenValue,
	isRecord,
	isReminder,
	listOf,
	slotsOf,
	textOf,
} from "./values";

const HIDDEN_MARK: StoredHiddenMark = { $hidden: true };

type Hidden = ReadonlySet<FieldKey>;
type Store = (
	field: WorkbenchField,
	value: CopyValue,
	hidden: Hidden,
) => StoredInput;

function fileMark(slot: FileSlot): StoredFileMark {
	return { $file: { name: slot.name, size: slot.size } };
}

const storedText: Store = (_field, value) => textOf(value);

function storedProp(
	prop: WorkbenchField,
	value: FieldValue | undefined,
	hidden: Hidden,
): JsonValue {
	if (hidden.has(prop.key) || isHiddenValue(value)) return { $hidden: true };
	if (prop.kind === "bool") return value === true;
	return value === undefined || value === null ? null : textOf(value);
}

const STORE: Readonly<Record<FieldKind, Store>> = {
	text: storedText,
	number: storedText,
	date: storedText,
	choice: storedText,
	json: storedText,
	bool: (_field, value) => value === true,
	chips: (_field, value) => [...listOf(value)],
	file: (_field, value) => {
		const slot = slotsOf(value)[0];
		return slot ? fileMark(slot) : null;
	},
	files: (_field, value) => slotsOf(value).map(fileMark),
	group: (field, value, hidden) =>
		Object.fromEntries(
			field.props.map((prop) => [
				prop.name,
				storedProp(prop, groupOf(value)[prop.name], hidden),
			]),
		),
	pairs: (_field, value) =>
		filledRows(value).map((row) => ({ key: row.key, value: row.value })),
	unsupported: () => null,
};

/** One value as saved: `hidden` holds the FieldKeys kept out (secret fields, "Don't save", secret-looking values). */
export function toStoredInput(
	field: WorkbenchField,
	value: CopyValue | undefined,
	hidden: Hidden,
): StoredInput {
	if (hidden.has(field.key) || isHiddenValue(value)) return HIDDEN_MARK;
	if (value === undefined || value === null) return null;
	return STORE[field.kind](field, value, hidden);
}

/** A run's copy (or the rail) as a record saves it, by field name. */
export function toStoredInputs(
	fields: readonly WorkbenchField[],
	values: Readonly<Record<string, CopyValue>>,
	hidden: Hidden,
): Readonly<Record<string, StoredInput>> {
	return Object.fromEntries(
		fields.map((field) => [
			field.name,
			toStoredInput(field, values[field.name], hidden),
		]),
	);
}

export interface StoredReadBack {
	/** By field name, only for saved inputs that still fit; a field the record lacks is absent. */
	readonly values: Readonly<Record<string, CopyValue>>;
	/** Saved inputs that no longer fit this form: their field is gone, or the value no longer fits. */
	readonly misfit: number;
	/** Files that must be picked again ("Pick again" reminders among `values`). */
	readonly pickAgain: number;
	/** FieldKeys kept out of storage: "Enter {label} again". */
	readonly enterAgain: readonly FieldKey[];
}

interface ReadField {
	readonly value: CopyValue;
	readonly pickAgain: number;
	readonly enterAgain: readonly FieldKey[];
}

export interface ReadOptions {
	/**
	 * A run's copy keeps `{ $hidden: true }` on an object's kept-out properties, so it is not repeated
	 * without them (S4); without it they come back empty, as the rail takes them.
	 */
	readonly keepHidden?: boolean;
}

/** The `{ $hidden: true }` mark as an object property's value. */
const HIDDEN_PROP: FieldValue = { $hidden: true };

const hiddenProp = (prop: WorkbenchField, options: ReadOptions): FieldValue =>
	options.keepHidden ? HIDDEN_PROP : emptyValue(prop);

function readGroup(
	field: WorkbenchField,
	stored: Readonly<Record<string, unknown>>,
	options: ReadOptions,
): ReadField {
	const value: Record<string, FieldValue> = {};
	const enterAgain: FieldKey[] = [];
	for (const prop of field.props) {
		const saved = stored[prop.name];
		if (isHiddenValue(saved)) enterAgain.push(prop.key);
		value[prop.name] = isHiddenValue(saved)
			? hiddenProp(prop, options)
			: (fitValue(prop, saved) ?? prop.defaultValue);
	}
	return { value, pickAgain: 0, enterAgain };
}

function readField(
	field: WorkbenchField,
	stored: unknown,
	options: ReadOptions,
): ReadField | undefined {
	if (isHiddenValue(stored))
		return { value: HIDDEN_VALUE, pickAgain: 0, enterAgain: [field.key] };
	if (field.kind === "group" && isRecord(stored))
		return readGroup(field, stored, options);
	const value = fitValue(field, stored);
	if (value === undefined) return undefined;
	return {
		value,
		pickAgain: slotsOf(value).filter(isReminder).length,
		enterAgain: [],
	};
}

/**
 * Saved inputs back as values (a history run's copy, "Use these inputs" from an older run): files
 * come back as "Pick again" reminders, kept-out values as `{ $hidden: true }` (an object's kept-out
 * properties only with `keepHidden`). The caller fills fields the record lacks with their starting
 * values ("New fields start at their default").
 */
export function fromStoredInputs(
	fields: readonly WorkbenchField[],
	inputs: Readonly<Record<string, StoredInput | CopyValue>>,
	options: ReadOptions = {},
): StoredReadBack {
	const values: Record<string, CopyValue> = {};
	const enterAgain: FieldKey[] = [];
	let misfit = 0;
	let pickAgain = 0;
	for (const [name, stored] of Object.entries(inputs)) {
		const field = fields.find((item) => item.name === name);
		const read = field ? readField(field, stored, options) : undefined;
		if (!read) {
			misfit += 1;
			continue;
		}
		values[name] = read.value;
		pickAgain += read.pickAgain;
		enterAgain.push(...read.enterAgain);
	}
	return { values, misfit, pickAgain, enterAgain };
}
