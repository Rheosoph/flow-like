/*
 * The fields of a quick action or form as a device reports them (`event_form`,
 * design R2 §1.8, §4.3, §6.5): which control each field gets, the value a
 * control starts with, and the value a run sends for it. A field of a type
 * this client does not know never becomes a text box: it has no control.
 */

/** What a device says about one field; the shape of `EventFormField`. */
export interface EventFormFieldSpec {
	name: string;
	/** The pin's own words. */
	data_type: string;
	value_type: string;
	optional: boolean;
	sensitive: boolean;
	default: unknown;
	options: readonly string[] | null;
}

export const FIELD_DATA_TYPES = [
	"String",
	"Integer",
	"Float",
	"Boolean",
	"Date",
	"Struct",
	"Generic",
	"Geometry",
	"PathBuf",
	"Byte",
] as const;
export type FieldDataType = (typeof FIELD_DATA_TYPES)[number];

export const FIELD_VALUE_TYPES = [
	"Normal",
	"Array",
	"HashSet",
	"HashMap",
] as const;
export type FieldValueType = (typeof FIELD_VALUE_TYPES)[number];

/** The control a field is shown with. */
export type FieldControl =
	| "text"
	| "password"
	| "integer"
	| "number"
	| "switch"
	| "date"
	| "select"
	| "json"
	| "file";

/** How a control's value becomes the value a run sends. */
export type FieldBase =
	| "string"
	| "integer"
	| "number"
	| "boolean"
	| "date"
	| "json"
	| "file";

export interface FieldShape {
	control: FieldControl;
	base: FieldBase;
	dataType: FieldDataType;
	valueType: FieldValueType;
	required: boolean;
}

/** A control's value: text for every control but the switch. */
export type FieldValue = string | boolean;

export type FieldProblem =
	| "required"
	| "integer"
	| "number"
	| "date"
	| "json"
	| "object"
	| "array"
	| "unique"
	| "items"
	| "option"
	| "file"
	| "unknown";

const isOneOf = <T extends string>(
	list: readonly T[],
	value: string,
): value is T => (list as readonly string[]).includes(value);

const BASE: Record<FieldDataType, FieldBase> = {
	String: "string",
	Geometry: "string",
	Integer: "integer",
	Float: "number",
	Boolean: "boolean",
	Date: "date",
	Struct: "json",
	Generic: "json",
	PathBuf: "file",
	Byte: "file",
};

const PLAIN_CONTROL: Record<FieldBase, FieldControl> = {
	string: "text",
	integer: "integer",
	number: "number",
	boolean: "switch",
	date: "date",
	json: "json",
	file: "file",
};

/** Bases whose value is one of a field's options when it has some. */
const CHOOSABLE = new Set<FieldBase>(["string", "integer", "number", "date"]);

function controlOf(
	dataBase: FieldBase,
	valueType: FieldValueType,
	field: EventFormFieldSpec,
): Pick<FieldShape, "control" | "base"> {
	if (dataBase === "file") return { control: "file", base: "file" };
	const base = valueType === "Normal" ? dataBase : "json";
	if (base === "boolean") return { control: "switch", base };
	if (field.options?.length && CHOOSABLE.has(base))
		return { control: "select", base };
	if (field.sensitive) return { control: "password", base };
	return { control: PLAIN_CONTROL[base], base };
}

/** The control of one field; null for a type this client does not know. */
export function fieldShape(field: EventFormFieldSpec): FieldShape | null {
	const { data_type: dataType, value_type: valueType } = field;
	if (!isOneOf(FIELD_DATA_TYPES, dataType)) return null;
	if (!isOneOf(FIELD_VALUE_TYPES, valueType)) return null;
	return {
		...controlOf(BASE[dataType], valueType, field),
		dataType,
		valueType,
		required: !field.optional,
	};
}

export type FormSupport =
	| { ok: true }
	| {
			ok: false;
			/** `file`: a field takes a file · `unknown_field`: a field of a type this client can't show · `truncated`: the device left fields out. */
			reason: "file" | "unknown_field" | "truncated";
			field?: string;
	  };

const takesFile = (field: EventFormFieldSpec) =>
	fieldShape(field)?.control === "file";
const unknownType = (field: EventFormFieldSpec) => fieldShape(field) === null;
const unsupported = (
	reason: "file" | "unknown_field" | "truncated",
	field?: EventFormFieldSpec,
): FormSupport => ({
	ok: false,
	reason,
	...(field ? { field: field.name } : {}),
});

/** Whether a form can be sent from here, or the first reason it can't. */
export function formSupport(form: {
	fields: readonly EventFormFieldSpec[];
	fields_truncated: boolean;
	file_fields: number;
}) {
	const file = form.fields.find(takesFile);
	if (form.file_fields > 0 || file) return unsupported("file", file);
	const unknown = form.fields.find(unknownType);
	if (unknown) return unsupported("unknown_field", unknown);
	return form.fields_truncated
		? unsupported("truncated")
		: ({ ok: true } as FormSupport);
}

function textOf(value: unknown) {
	if (typeof value === "string") return value;
	if (typeof value === "number" || typeof value === "boolean")
		return String(value);
	return JSON.stringify(value);
}

const DAY_PREFIX = /^\d{4}-\d{2}-\d{2}/u;

function seedText(shape: FieldShape, value: unknown) {
	if (value === null || value === undefined) return "";
	if (shape.base === "json") return JSON.stringify(value, null, 2);
	const text = textOf(value);
	return shape.control === "date" ? (DAY_PREFIX.exec(text)?.[0] ?? text) : text;
}

/** The value a control starts with: the field's default, or empty. A sensitive field always starts empty. */
export function seedValue(field: EventFormFieldSpec): FieldValue {
	const shape = fieldShape(field);
	if (!shape) return "";
	const value = field.sensitive ? null : field.default;
	return shape.control === "switch" ? value === true : seedText(shape, value);
}

const seedEntry = (field: EventFormFieldSpec) =>
	[field.name, seedValue(field)] as const;

export function seedValues(
	fields: readonly EventFormFieldSpec[],
): Record<string, FieldValue> {
	return Object.fromEntries(fields.map(seedEntry));
}

type Converted =
	| { ok: true; value: unknown }
	| { ok: true; empty: true }
	| { ok: false; problem: FieldProblem };

const accepted = (value: unknown): Converted => ({ ok: true, value });
const refused = (problem: FieldProblem): Converted => ({ ok: false, problem });
const EMPTY: Converted = { ok: true, empty: true };

const INTEGER = /^[+-]?\d+$/u;
const DECIMAL = /^[+-]?(\d+\.?\d*|\.\d+)([eE][+-]?\d+)?$/u;
const DAY = /^\d{4}-\d{2}-\d{2}$/u;
const RFC3339 =
	/^\d{4}-\d{2}-\d{2}[Tt ]([01]\d|2[0-3]):[0-5]\d:([0-5]\d|60)(\.\d+)?([Zz]|[+-]([01]\d|2[0-3]):[0-5]\d)$/u;

function calendarDay(text: string) {
	const [year, month, day] = text.slice(0, 10).split("-").map(Number);
	if (!year || !month || !day) return false;
	const date = new Date(Date.UTC(year, month - 1, day));
	return (
		date.getUTCFullYear() === year &&
		date.getUTCMonth() === month - 1 &&
		date.getUTCDate() === day
	);
}

/** `YYYY-MM-DD` of a real day, or an RFC 3339 date and time: what a device accepts. */
export function isFieldDate(text: string) {
	return (DAY.test(text) || RFC3339.test(text)) && calendarDay(text);
}

const SCALAR: Partial<Record<FieldBase, (text: string) => Converted>> = {
	integer: (text) =>
		INTEGER.test(text) && Number.isSafeInteger(Number(text))
			? accepted(Number(text))
			: refused("integer"),
	number: (text) =>
		DECIMAL.test(text) && Number.isFinite(Number(text))
			? accepted(Number(text))
			: refused("number"),
	date: (text) => (isFieldDate(text) ? accepted(text) : refused("date")),
};

function isObject(value: unknown): value is Record<string, unknown> {
	return typeof value === "object" && value !== null && !Array.isArray(value);
}

/** What a device checks for each member of an array, set or map of these types. */
const MEMBER_CHECK: Partial<Record<FieldDataType, (item: unknown) => boolean>> =
	{
		String: (item) => typeof item === "string",
		Geometry: (item) => typeof item === "string" || isObject(item),
		Integer: (item) => Number.isSafeInteger(item),
		Float: (item) => typeof item === "number" && Number.isFinite(item),
		Boolean: (item) => typeof item === "boolean",
		Date: (item) => typeof item === "string" && isFieldDate(item),
		Struct: isObject,
	};

const jsonText = (item: unknown) => JSON.stringify(item);

function members(shape: FieldShape, value: unknown) {
	if (shape.valueType === "HashMap")
		return isObject(value) ? Object.values(value) : refused("object");
	if (!Array.isArray(value)) return refused("array");
	const unique = new Set(value.map(jsonText)).size === value.length;
	return shape.valueType === "HashSet" && !unique ? refused("unique") : value;
}

function parsed(text: string) {
	try {
		return accepted(JSON.parse(text));
	} catch {
		return refused("json");
	}
}

function collection(shape: FieldShape, value: unknown) {
	const items = members(shape, value);
	if (!Array.isArray(items)) return items;
	const check = MEMBER_CHECK[shape.dataType];
	return !check || items.every(check) ? accepted(value) : refused("items");
}

function jsonValue(shape: FieldShape, text: string) {
	const json = parsed(text);
	if (!json.ok || !("value" in json)) return json;
	if (shape.valueType !== "Normal") return collection(shape, json.value);
	return shape.dataType === "Struct" && !isObject(json.value)
		? refused("object")
		: json;
}

function typed(shape: FieldShape, field: EventFormFieldSpec, text: string) {
	if (text === "" || (shape.base !== "string" && !text.trim())) return EMPTY;
	if (shape.control === "select" && !field.options?.includes(text))
		return refused("option");
	return SCALAR[shape.base]?.(text.trim()) ?? accepted(text);
}

function converted(
	shape: FieldShape,
	field: EventFormFieldSpec,
	value: FieldValue,
) {
	if (shape.base === "file") return refused("file");
	if (shape.base === "boolean") return accepted(value === true);
	const text = typeof value === "string" ? value : "";
	if (shape.base !== "json") return typed(shape, field, text);
	return text.trim() ? jsonValue(shape, text) : EMPTY;
}

export type FieldResult =
	| { ok: true; value: unknown }
	/** Left out of the run: an optional field without a value takes the flow's default. */
	| { ok: true; omitted: true }
	| { ok: false; problem: FieldProblem };

/** The value a run sends for one field, or why it can't be sent. */
export function fieldPayload(
	field: EventFormFieldSpec,
	value: FieldValue | undefined,
): FieldResult {
	const shape = fieldShape(field);
	if (!shape) return { ok: false, problem: "unknown" };
	const result = converted(shape, field, value ?? seedValue(field));
	if (!result.ok) return result;
	if ("empty" in result)
		return shape.required
			? { ok: false, problem: "required" }
			: { ok: true, omitted: true };
	return result;
}

export type FormPayload =
	| { ok: true; payload: Record<string, unknown> }
	| { ok: false; problems: Record<string, FieldProblem> };

/** The run's input: field name → value, without the optional fields left empty. */
export function formPayload(
	fields: readonly EventFormFieldSpec[],
	values: Readonly<Record<string, FieldValue>>,
): FormPayload {
	const payload: Record<string, unknown> = {};
	const problems: Record<string, FieldProblem> = {};
	for (const field of fields) {
		const result = fieldPayload(field, values[field.name]);
		if (!result.ok) problems[field.name] = result.problem;
		else if ("value" in result) payload[field.name] = result.value;
	}
	return Object.keys(problems).length
		? { ok: false, problems }
		: { ok: true, payload };
}
