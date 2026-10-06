/*
 * The form's fields from the event's inputs (PLAN §3.3 step 1, spec F): the kind by value type
 * first, FlowPath detection, keys, labels, help, options and seeding; and the form model.
 */
import { EMPTY_STRING_REF } from "../../../../lib/board-refs";
import {
	FIELD_DATA_TYPES,
	FIELD_VALUE_TYPES,
} from "../../../../lib/event-form";
import { defaultValueFromType } from "../../../../lib/flow-defaults";
import type { IEvent } from "../../../../lib/schema/flow/event";
import type { IEventPayload } from "../../../../lib/schema/flow/event-payload";
import type {
	IValueType,
	IVariableType,
} from "../../../../lib/schema/flow/variable";
import { parseUint8ArrayToJson } from "../../../../lib/uint8";
import {
	type FieldKey,
	type FieldKind,
	type FieldValue,
	type FileMode,
	type FormModel,
	type HostCapabilities,
	PAYLOAD_PIN_NAME,
	type ViewerHabits,
	type WorkbenchEventInput,
	type WorkbenchField,
} from "../contracts";
import {
	emptyValue,
	fieldKey,
	filledRows,
	fitValue,
	isEmpty,
	isFileField,
	isRecord,
} from "./values";

export { fieldKey, splitKey } from "./values";

/** A WorkbenchField that also carries the pin's own optional flag ("Optional" on objects and maps). */
type FormField = WorkbenchField & { readonly optional: boolean };

type ItemKind = WorkbenchField["itemKind"];

interface Shape {
	readonly kind: FieldKind;
	readonly fileMode: FileMode | null;
	readonly itemKind: ItemKind;
	readonly options: readonly string[] | null;
	readonly props: readonly WorkbenchField[];
}

const PLAIN: Omit<Shape, "kind"> = {
	fileMode: null,
	itemKind: null,
	options: null,
	props: [],
};
const plain = (kind: FieldKind): Shape => ({ ...PLAIN, kind });

const DATA_TYPES: ReadonlySet<string> = new Set(FIELD_DATA_TYPES);
const VALUE_TYPES: ReadonlySet<string> = new Set(FIELD_VALUE_TYPES);
const CHOOSABLE: ReadonlySet<string> = new Set([
	"String",
	"Integer",
	"Float",
	"Date",
]);
const MAP_SCALARS: ReadonlySet<string> = new Set([
	"String",
	"Integer",
	"Float",
	"Boolean",
	"Date",
]);
const CHIP_ITEM: Readonly<Record<string, ItemKind>> = {
	String: "text",
	Integer: "number",
	Float: "number",
	Date: "date",
};
const NORMAL_KIND: Readonly<Record<string, FieldKind>> = {
	String: "text",
	Integer: "number",
	Float: "number",
	Boolean: "bool",
	Date: "date",
	Generic: "json",
	Geometry: "json",
};
/** Schema keywords that make an object's shape more than flat properties. */
const COMBINATORS = ["$ref", "anyOf", "oneOf", "allOf", "not", "if"];

// ─── Labels and help ────────────────────────────────────────────────────────

const IDENTIFIER = /^[A-Za-z0-9]+(?:_+[A-Za-z0-9]+)*$/;

function isIdentifier(text: string): boolean {
	if (!IDENTIFIER.test(text)) return false;
	return text.includes("_") || /[a-z][A-Z]/.test(text) || /^[a-z]/.test(text);
}

/**
 * A label for an identifier such as `youtube_url` or `invoiceDate` ("Youtube url", "Invoice
 * date"). Text that already reads as a label ("Invoice date", "URL", "E-mail") is kept.
 */
export function humanizeLabel(text: string): string {
	const trimmed = text.trim();
	if (!isIdentifier(trimmed)) return trimmed;
	const sentence = trimmed
		.replace(/([a-z0-9])([A-Z])/g, "$1 $2")
		.replace(/([A-Z]+)([A-Z][a-z])/g, "$1 $2")
		.split(/[_\s]+/)
		.filter(Boolean)
		.join(" ")
		.toLowerCase();
	return sentence.charAt(0).toUpperCase() + sentence.slice(1);
}

function labelOf(input: WorkbenchEventInput): string {
	return humanizeLabel(input.friendly_name?.trim() || input.name);
}

/** A description, unless it is empty or still an unresolved all-digit ref key. */
function helpOf(description: unknown): string | null {
	const text = typeof description === "string" ? description.trim() : "";
	if (text === EMPTY_STRING_REF || /^\d*$/.test(text)) return null;
	return text;
}

// ─── Schemas ────────────────────────────────────────────────────────────────

function parsedSchema(
	schema: unknown,
): Readonly<Record<string, unknown>> | null {
	if (isRecord(schema)) return schema;
	if (typeof schema !== "string" || !/^\s*\{/.test(schema)) return null;
	try {
		const value: unknown = JSON.parse(schema);
		return isRecord(value) ? value : null;
	} catch {
		return null;
	}
}

function hasFlowPathProperties(schema: Readonly<Record<string, unknown>>) {
	const properties = isRecord(schema.properties) ? schema.properties : {};
	const required = Array.isArray(schema.required) ? schema.required : [];
	return ["path", "store_ref"].every(
		(name) => Object.hasOwn(properties, name) && required.includes(name),
	);
}

/**
 * `flpIsFlowPathSchema`: a resolved schema (JSON text or object) titled "FlowPath", or with
 * properties `path` and `store_ref`, both required. An unresolved all-digit ref key is not.
 */
export function isFlowPathSchema(schema: unknown): boolean {
	const parsed = parsedSchema(schema);
	if (!parsed) return false;
	if (parsed.title === "FlowPath") return true;
	if (parsed.type === "array" && isRecord(parsed.items))
		return isFlowPathSchema(parsed.items);
	return hasFlowPathProperties(parsed);
}

function objectTyped(schema: Readonly<Record<string, unknown>>): boolean {
	const type = schema.type;
	if (type === undefined || type === "object") return true;
	return Array.isArray(type) && type.includes("object");
}

const PROP_TYPES: Readonly<
	Record<string, { readonly kind: FieldKind; readonly dataType: string }>
> = {
	string: { kind: "text", dataType: "String" },
	integer: { kind: "number", dataType: "Integer" },
	number: { kind: "number", dataType: "Float" },
	boolean: { kind: "bool", dataType: "Boolean" },
};
const DATE_FORMATS: Readonly<Record<string, "date" | "dateTime">> = {
	date: "date",
	"date-time": "dateTime",
};

/** The one scalar type of a property (`"string"`, `["integer", "null"]`; an untyped enum is text), or null. */
function scalarType(schema: Readonly<Record<string, unknown>>) {
	if (schema.type === undefined)
		return schema.enum === undefined ? null : "string";
	const types = (
		Array.isArray(schema.type) ? schema.type : [schema.type]
	).filter((item) => item !== "null");
	const only = types.length === 1 ? types[0] : null;
	return typeof only === "string" && Object.hasOwn(PROP_TYPES, only)
		? only
		: null;
}

function enumOptions(values: unknown) {
	if (!Array.isArray(values) || values.length === 0) return null;
	const scalar = (value: unknown) =>
		typeof value === "string" || typeof value === "number";
	return values.every(scalar) ? values.map(String) : null;
}

const finiteNumber = (value: unknown): value is number =>
	typeof value === "number" && Number.isFinite(value);

function numberPair(min: unknown, max: unknown) {
	return finiteNumber(min) && finiteNumber(max) && min <= max
		? ([min, max] as const)
		: null;
}

function positive(value: unknown) {
	return finiteNumber(value) && value > 0 ? value : null;
}

function isShort(kind: FieldKind, options: readonly string[] | null) {
	if (kind === "choice") return (options?.length ?? 0) <= 2;
	return kind === "number" || kind === "date" || kind === "bool";
}

const NEVER_REQUIRED: ReadonlySet<FieldKind> = new Set<FieldKind>([
	"bool",
	"group",
	"pairs",
]);

function requiredOf(kind: FieldKind, optional: boolean) {
	return !optional && !NEVER_REQUIRED.has(kind);
}

interface PropKind {
	readonly kind: FieldKind;
	readonly dataType: string;
	readonly options: readonly string[] | null;
	readonly dateFormat: "date" | "dateTime" | null;
}

function propKind(schema: Readonly<Record<string, unknown>>): PropKind | null {
	const type = scalarType(schema);
	if (type === null) return null;
	const base = PROP_TYPES[type];
	const options = enumOptions(schema.enum);
	if (schema.enum !== undefined && !options) return null;
	if (options && base.kind !== "bool")
		return {
			kind: "choice",
			dataType: base.dataType,
			options,
			dateFormat: null,
		};
	const format = typeof schema.format === "string" ? schema.format : "";
	const dateFormat = type === "string" ? (DATE_FORMATS[format] ?? null) : null;
	if (dateFormat)
		return { kind: "date", dataType: "Date", options: null, dateFormat };
	return {
		kind: base.kind,
		dataType: base.dataType,
		options: null,
		dateFormat: null,
	};
}

function propField(
	group: string,
	name: string,
	schema: unknown,
	index: number,
	required: readonly unknown[],
): WorkbenchField | null {
	if (!isRecord(schema) || COMBINATORS.some((word) => word in schema))
		return null;
	const kind = propKind(schema);
	if (!kind) return null;
	const draft: FormField = {
		key: fieldKey(group, name),
		name,
		label:
			typeof schema.title === "string" ? schema.title : humanizeLabel(name),
		help: helpOf(schema.description),
		kind: kind.kind,
		dataType: kind.dataType,
		valueType: "Normal",
		required: requiredOf(kind.kind, !required.includes(name)),
		optional: !required.includes(name),
		sensitive: false,
		defaultOmitted: false,
		defaultValue: null,
		hasDefault: false,
		options: kind.options,
		range: numberPair(schema.minimum, schema.maximum),
		step: positive(schema.multipleOf),
		integer: kind.dataType === "Integer",
		fileMode: null,
		itemKind: null,
		dateFormat: kind.dateFormat,
		props: [],
		short: isShort(kind.kind, kind.options),
		index,
	};
	return withDefault(draft, schema.default, false);
}

/** A Struct's shape: flat properties (a group), no usable schema (pairs), or nested (JSON). */
function objectShape(name: string, schema: unknown): Shape {
	const parsed = parsedSchema(schema);
	if (!parsed) return plain("pairs");
	if (COMBINATORS.some((word) => word in parsed) || !objectTyped(parsed))
		return plain("json");
	const properties = isRecord(parsed.properties)
		? Object.entries(parsed.properties)
		: [];
	if (properties.length === 0) return plain("pairs");
	const required = Array.isArray(parsed.required) ? parsed.required : [];
	const props = properties.map(([prop, propSchema], index) =>
		propField(name, prop, propSchema, index, required),
	);
	return props.every((prop): prop is WorkbenchField => prop !== null)
		? { ...PLAIN, kind: "group", props }
		: plain("json");
}

// ─── Kinds ──────────────────────────────────────────────────────────────────

function optionsOf(input: WorkbenchEventInput): readonly string[] | null {
	const values = (input.valid_values ?? []).filter(
		(value): value is string => typeof value === "string",
	);
	return values.length > 0 ? [...new Set(values)] : null;
}

function fileModeOf(input: WorkbenchEventInput): FileMode | null {
	if (input.data_type === "PathBuf" || input.data_type === "Byte") return "url";
	return input.data_type === "Struct" && isFlowPathSchema(input.schema)
		? "flowpath"
		: null;
}

/** A map of PathBuf/Byte has no file control and no value a person could type: unsupported. */
function fileShape(mode: FileMode, valueType: string): Shape {
	if (valueType === "HashMap")
		return plain(mode === "url" ? "unsupported" : "json");
	return {
		...PLAIN,
		kind: valueType === "Normal" ? "file" : "files",
		fileMode: mode,
	};
}

function containerShape(input: WorkbenchEventInput): Shape {
	if (input.value_type === "HashMap")
		return plain(MAP_SCALARS.has(input.data_type) ? "pairs" : "json");
	const itemKind = CHIP_ITEM[input.data_type] ?? null;
	if (!itemKind) return plain("json");
	return { ...PLAIN, kind: "chips", itemKind, options: optionsOf(input) };
}

function normalShape(input: WorkbenchEventInput): Shape {
	if (input.data_type === "Struct")
		return objectShape(input.name, input.schema);
	const options = optionsOf(input);
	if (options && CHOOSABLE.has(input.data_type))
		return { ...PLAIN, kind: "choice", options };
	return plain(NORMAL_KIND[input.data_type] ?? "unsupported");
}

/** Value type first (PLAN §3.3): files, lists, maps, then single values by data type. */
function shapeOf(input: WorkbenchEventInput): Shape {
	if (!DATA_TYPES.has(input.data_type) || !VALUE_TYPES.has(input.value_type))
		return plain("unsupported");
	const fileMode = fileModeOf(input);
	if (fileMode) return fileShape(fileMode, input.value_type);
	if (input.value_type === "Normal") return normalShape(input);
	return containerShape(input);
}

function rangeOf(range: unknown): readonly [number, number] | null {
	return Array.isArray(range) && range.length === 2
		? numberPair(range[0], range[1])
		: null;
}

// ─── Seeding ────────────────────────────────────────────────────────────────

/** The stored default as JSON; a missing, empty or `null` default is none. */
function storedDefault(bytes: readonly number[] | null | undefined): unknown {
	if (!bytes || bytes.length === 0) return undefined;
	const value: unknown = parseUint8ArrayToJson([...bytes]);
	return value === null ? undefined : value;
}

/** Today's seeding of optional fields without a stored default: the type default, a date is today. */
function typeDefault(input: WorkbenchEventInput, today: string): unknown {
	if (input.value_type === "Normal" && input.data_type === "Date") return today;
	return defaultValueFromType(
		input.value_type as IValueType,
		input.data_type as IVariableType,
	);
}

function seededValue(field: WorkbenchField, json: unknown): FieldValue {
	if (json === undefined || json === null) return emptyValue(field);
	if (field.kind === "json") return JSON.stringify(json, null, 2);
	return fitValue(field, json) ?? emptyValue(field);
}

/** A seeded type default that shows something: a switch always, rows when there are some. */
function meaningful(field: WorkbenchField, value: FieldValue): boolean {
	if (field.kind === "bool") return true;
	if (field.kind === "pairs") return filledRows(value).length > 0;
	return !isEmpty(field, value);
}

function withDefault<T extends WorkbenchField>(
	field: T,
	json: unknown,
	fromStore: boolean,
): T {
	if (json === undefined)
		return {
			...field,
			defaultValue: emptyValue(field),
			hasDefault: field.kind === "bool",
		};
	return {
		...field,
		defaultValue: seededValue(field, json),
		hasDefault: fromStore || json !== null,
	};
}

function seededGroup<T extends WorkbenchField>(field: T, stored: unknown): T {
	const object = isRecord(stored) ? stored : {};
	const props = field.props.map((prop) =>
		object[prop.name] === undefined
			? prop
			: withDefault(prop, object[prop.name], true),
	);
	return {
		...field,
		props,
		defaultValue: Object.fromEntries(
			props.map((prop) => [prop.name, prop.defaultValue]),
		),
		hasDefault: stored !== undefined || props.some((prop) => prop.hasDefault),
	};
}

const withheld = (field: WorkbenchField) =>
	field.sensitive || field.defaultOmitted || isFileField(field);

function cleared<T extends WorkbenchField>(field: T): T {
	const props = field.props.map((prop) => withDefault(prop, undefined, false));
	return {
		...field,
		props,
		defaultValue: emptyValue(field),
		hasDefault: false,
	};
}

/**
 * The starting value (must-keep #6): the stored default; else, for an optional field, the type
 * default (a date is today, a number 0, a switch off); files never; a sensitive or withheld
 * default is empty whatever arrived (spec M6).
 */
function seeded<T extends WorkbenchField>(
	field: T,
	input: WorkbenchEventInput,
	today: string,
): T {
	if (withheld(field)) return cleared(field);
	const stored = storedDefault(input.default_value);
	if (field.kind === "group") return seededGroup(field, stored);
	if (stored !== undefined) return withDefault(field, stored, true);
	if (input.optional !== true) return withDefault(field, undefined, false);
	const value = seededValue(field, typeDefault(input, today));
	return {
		...field,
		defaultValue: value,
		hasDefault: meaningful(field, value),
	};
}

function draftOf(input: WorkbenchEventInput, index: number): FormField {
	const shape = shapeOf(input);
	const optional = input.optional === true;
	return {
		key: input.name,
		name: input.name,
		label: labelOf(input),
		help: helpOf(input.description),
		kind: shape.kind,
		dataType: input.data_type,
		valueType: input.value_type,
		required: requiredOf(shape.kind, optional),
		optional,
		sensitive: input.sensitive === true,
		defaultOmitted: input.default_omitted === true,
		defaultValue: null,
		hasDefault: false,
		options: shape.options,
		range: rangeOf(input.range),
		step: positive(input.step),
		integer: input.data_type === "Integer",
		fileMode: shape.fileMode,
		itemKind: shape.itemKind,
		dateFormat: null,
		props: shape.props,
		short: isShort(shape.kind, shape.options),
		index,
	};
}

const isFormInput = (input: WorkbenchEventInput) =>
	input.name.toLowerCase() !== PAYLOAD_PIN_NAME &&
	input.data_type !== "Execution";

function localDay(at: Date): string {
	const pad = (value: number) => String(value).padStart(2, "0");
	return `${at.getFullYear()}-${pad(at.getMonth() + 1)}-${pad(at.getDate())}`;
}

/**
 * `FieldsFromEvent`: the visible fields in author order, without the catalog's `payload` pin and
 * Execution pins; a repeated name keeps its first input. `today` is the local day that seeds an
 * optional date without a stored default.
 */
export function fieldsFromEvent(
	inputs: readonly WorkbenchEventInput[],
	today: string = localDay(new Date()),
): readonly WorkbenchField[] {
	const fields: WorkbenchField[] = [];
	const seen = new Set<string>();
	for (const input of inputs) {
		if (!isFormInput(input) || seen.has(input.name)) continue;
		seen.add(input.name);
		fields.push(seeded(draftOf(input, fields.length), input, today));
	}
	return fields;
}

/** Whether the pin itself is optional: "Optional" shows on objects and maps too (never on switches). */
export function pinOptional(field: WorkbenchField): boolean {
	return "optional" in field && typeof field.optional === "boolean"
		? field.optional
		: !field.required;
}

// ─── Cursor stops ───────────────────────────────────────────────────────────

export interface FieldTarget {
	readonly key: FieldKey;
	readonly field: WorkbenchField;
	/** The object field's name for a property, else null. */
	readonly group: string | null;
}

/**
 * `flpTargets`: every place the cursor can stop, in form order: fields, and an object's
 * properties one by one. `skip` names fields that are no stop (blocked file fields).
 */
export function targets(
	fields: readonly WorkbenchField[],
	skip: readonly string[] = [],
): readonly FieldTarget[] {
	return fields
		.filter((field) => !skip.includes(field.name))
		.flatMap((field): readonly FieldTarget[] =>
			field.kind === "group"
				? field.props.map((prop) => ({
						key: prop.key,
						field: prop,
						group: field.name,
					}))
				: [{ key: field.key, field, group: null }],
		);
}

// ─── The form model ─────────────────────────────────────────────────────────

/** A route as the app stores it: trimmed, with a leading slash, "/" for none. */
export function normalizeRoute(route: string): string {
	const trimmed = route.trim();
	if (!trimmed) return "/";
	return trimmed.startsWith("/") ? trimmed : `/${trimmed}`;
}

function routesOf(raw: unknown): readonly string[] {
	if (!Array.isArray(raw)) return [];
	return [...new Set(raw.map((route) => normalizeRoute(String(route))))];
}

function labelText(raw: unknown): string | null {
	return typeof raw === "string" && raw.trim() !== "" ? raw.trim() : null;
}

export interface FormContentParts {
	readonly name: string;
	readonly description: string;
	readonly fields: readonly WorkbenchField[];
	readonly routes: readonly string[];
	readonly eventRoute: string | null;
	readonly submitLabel: string | null;
}

/** Change detection only (`FormModel.contentKey`): fields, routes, labels and texts; never stored. */
export function formContentKey(parts: FormContentParts): string {
	return JSON.stringify([
		parts.name,
		parts.description,
		parts.routes,
		parts.eventRoute,
		parts.submitLabel,
		parts.fields,
	]);
}

/** The form model of one event. `appId` comes from the host: `IEvent` carries none. */
export function createFormModel(
	event: IEvent,
	config: Partial<IEventPayload> | undefined,
	host: HostCapabilities,
	viewer: ViewerHabits,
	today: string,
	appId: string,
): FormModel {
	const fields = fieldsFromEvent(event.inputs ?? [], today);
	const parts: FormContentParts = {
		name: event.name ?? "",
		description: event.description?.trim() ?? "",
		fields,
		routes: routesOf(config?.navigate_to_routes),
		eventRoute: event.route ? normalizeRoute(event.route) : null,
		submitLabel: labelText(config?.submit_label),
	};
	return {
		appId,
		eventId: event.id,
		nodeId: event.node_id,
		name: parts.name,
		description: parts.description,
		fields,
		contentKey: formContentKey(parts),
		submitLabel: parts.submitLabel,
		routes: parts.routes,
		eventRoute: parts.eventRoute,
		host,
		viewer,
	};
}
