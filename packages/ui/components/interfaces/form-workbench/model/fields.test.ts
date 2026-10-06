import { describe, expect, test } from "bun:test";
import { EMPTY_STRING_REF } from "../../../../lib/board-refs";
import type { IEvent } from "../../../../lib/schema/flow/event";
import {
	FIELD_KEY_SEPARATOR,
	type FieldKind,
	type FieldsFromEvent,
	type FileMode,
	type HostCapabilities,
	type ViewerHabits,
	type WorkbenchEventInput,
	type WorkbenchField,
} from "../contracts";
import {
	createFormModel,
	fieldKey,
	fieldsFromEvent,
	formContentKey,
	humanizeLabel,
	isFlowPathSchema,
	normalizeRoute,
	pinOptional,
	splitKey,
	targets,
} from "./fields";

const _contract: FieldsFromEvent = fieldsFromEvent;
void _contract;

const TODAY = "2026-10-05";
const FLOWPATH = JSON.stringify({
	$schema: "https://json-schema.org/draft/2020-12/schema",
	title: "FlowPath",
	type: "object",
	properties: {
		path: { type: "string" },
		store_ref: { type: "string" },
		cache_store_ref: { type: ["string", "null"] },
	},
	required: ["path", "store_ref"],
});
const OPEN_OBJECT = '{"type":"object","additionalProperties":true}';
const TERMS = JSON.stringify({
	type: "object",
	properties: {
		currency: { type: "string", enum: ["EUR", "USD", "CHF", "GBP"] },
		net_days: { type: "integer", format: "uint32", minimum: 0, maximum: 365 },
		discount_percent: { type: ["number", "null"], format: "double" },
		discount_days: { type: ["integer", "null"] },
		due: { type: "string", format: "date" },
		sent_at: { type: "string", format: "date-time" },
		urgent: { type: "boolean" },
	},
	required: ["currency", "net_days", "urgent"],
});

const bytes = (value: unknown) =>
	Array.from(new TextEncoder().encode(JSON.stringify(value)));

function input(
	name: string,
	dataType: string,
	rest: Partial<WorkbenchEventInput> = {},
): WorkbenchEventInput {
	return {
		id: `pin-${name}`,
		name,
		friendly_name: "",
		description: "",
		data_type: dataType,
		value_type: "Normal",
		schema: null,
		default_value: null,
		index: 0,
		optional: false,
		...rest,
	};
}

const one = (item: WorkbenchEventInput) => fieldsFromEvent([item], TODAY)[0];
const kindOf = (
	dataType: string,
	valueType: string,
	rest: Partial<WorkbenchEventInput> = {},
) => one(input("x", dataType, { value_type: valueType, ...rest }));

describe("which inputs become fields", () => {
	test("the catalog's payload pin (any case) and Execution pins are dropped; author order is kept", () => {
		const fields = fieldsFromEvent(
			[
				input("Payload", "Struct", { schema: OPEN_OBJECT }),
				input("exec_out", "Execution"),
				input("vendor_name", "String"),
				input("invoice_date", "Date"),
				input("vendor_name", "Integer"),
			],
			TODAY,
		);
		expect(fields.map((field) => [field.name, field.index])).toEqual([
			["vendor_name", 0],
			["invoice_date", 1],
		]);
		expect(fields[0].kind).toBe("text");
	});

	test("a form without inputs has no fields", () => {
		expect(fieldsFromEvent([], TODAY)).toEqual([]);
		expect(fieldsFromEvent([input("payload", "Struct")], TODAY)).toEqual([]);
	});
});

const UNRESOLVED = { schema: "16248035215404677707" };
const REF = { schema: '{"$ref":"#/$defs/A","$defs":{"A":{}}}' };
const NESTED = {
	schema: '{"type":"object","properties":{"a":{"type":"object"}}}',
};
const ANY_OF = {
	schema:
		'{"type":"object","properties":{"a":{"anyOf":[{"type":"string"},{"type":"null"}]}}}',
};

/** data type, value type, input extras → kind, file mode, item kind (data-contract.md "Type matrix", old bugs fixed). */
const MATRIX: [
	string,
	string,
	Partial<WorkbenchEventInput>,
	FieldKind,
	FileMode | null,
	WorkbenchField["itemKind"],
][] = [
	["String", "Normal", {}, "text", null, null],
	["Integer", "Normal", {}, "number", null, null],
	["Float", "Normal", {}, "number", null, null],
	["Boolean", "Normal", {}, "bool", null, null],
	["Date", "Normal", {}, "date", null, null],
	["Generic", "Normal", {}, "json", null, null],
	["Geometry", "Normal", {}, "json", null, null],
	["PathBuf", "Normal", {}, "file", "url", null],
	["Byte", "Normal", {}, "file", "url", null],
	["Struct", "Normal", { schema: FLOWPATH }, "file", "flowpath", null],
	["Struct", "Normal", { schema: TERMS }, "group", null, null],
	["Struct", "Normal", {}, "pairs", null, null],
	["Struct", "Normal", { schema: OPEN_OBJECT }, "pairs", null, null],
	["Struct", "Normal", UNRESOLVED, "pairs", null, null],
	["Struct", "Normal", { schema: "not json" }, "pairs", null, null],
	["Struct", "Normal", REF, "json", null, null],
	["Struct", "Normal", NESTED, "json", null, null],
	["Struct", "Normal", ANY_OF, "json", null, null],
	["String", "Array", {}, "chips", null, "text"],
	["Integer", "Array", {}, "chips", null, "number"],
	["Float", "HashSet", {}, "chips", null, "number"],
	["Date", "HashSet", {}, "chips", null, "date"],
	["Boolean", "Array", {}, "json", null, null],
	["Generic", "Array", {}, "json", null, null],
	["Geometry", "HashSet", {}, "json", null, null],
	["Struct", "Array", {}, "json", null, null],
	["Struct", "Array", { schema: FLOWPATH }, "files", "flowpath", null],
	["Struct", "HashSet", { schema: FLOWPATH }, "files", "flowpath", null],
	["PathBuf", "Array", {}, "files", "url", null],
	["Byte", "HashSet", {}, "files", "url", null],
	["String", "HashMap", {}, "pairs", null, null],
	["Integer", "HashMap", {}, "pairs", null, null],
	["Float", "HashMap", {}, "pairs", null, null],
	["Boolean", "HashMap", {}, "pairs", null, null],
	["Date", "HashMap", {}, "pairs", null, null],
	["Struct", "HashMap", {}, "json", null, null],
	["Struct", "HashMap", { schema: FLOWPATH }, "json", null, null],
	["Generic", "HashMap", {}, "json", null, null],
	["Geometry", "HashMap", {}, "json", null, null],
	["PathBuf", "HashMap", {}, "unsupported", null, null],
	["Byte", "HashMap", {}, "unsupported", null, null],
	["Polygon", "Normal", {}, "unsupported", null, null],
	["String", "Tree", {}, "unsupported", null, null],
];

describe("the type matrix: value type first, then data type", () => {
	for (const [dataType, valueType, rest, kind, fileMode, itemKind] of MATRIX)
		test(`${dataType} × ${valueType}${rest.schema ? " (with schema)" : ""} → ${kind}`, () => {
			const field = kindOf(dataType, valueType, rest);
			expect([field.kind, field.fileMode, field.itemKind]).toEqual([
				kind,
				fileMode,
				itemKind,
			]);
		});

	test("fixed choices make a choice for String, Integer, Float and Date only", () => {
		const options = { valid_values: ["S", "M", "S", "L"] };
		expect(kindOf("String", "Normal", options)).toMatchObject({
			kind: "choice",
			options: ["S", "M", "L"],
		});
		expect(kindOf("Integer", "Normal", { valid_values: ["1", "2"] }).kind).toBe(
			"choice",
		);
		expect(kindOf("Float", "Normal", { valid_values: ["0.5"] }).kind).toBe(
			"choice",
		);
		expect(
			kindOf("Date", "Normal", { valid_values: ["2026-09-18"] }).kind,
		).toBe("choice");
		expect(kindOf("Boolean", "Normal", { valid_values: ["true"] }).kind).toBe(
			"bool",
		);
		expect(kindOf("String", "Normal", { valid_values: [] }).kind).toBe("text");
		expect(kindOf("String", "Array", options)).toMatchObject({
			kind: "chips",
			options: ["S", "M", "L"],
		});
	});

	test("range and step come from the pin; a one-ended or reversed range is none", () => {
		expect(
			kindOf("Integer", "Normal", { range: [1, 10], step: 2 }),
		).toMatchObject({ range: [1, 10], step: 2, integer: true });
		expect(
			kindOf("Float", "Normal", { range: [10, 1], step: 0 }),
		).toMatchObject({ range: null, step: null, integer: false });
		expect(
			kindOf("Float", "Normal", { range: [1] as unknown as [number, number] })
				.range,
		).toBeNull();
	});

	test("required is !optional, except switches, objects and name/value rows", () => {
		expect(kindOf("String", "Normal").required).toBe(true);
		expect(kindOf("String", "Normal", { optional: true }).required).toBe(false);
		expect(kindOf("Boolean", "Normal").required).toBe(false);
		expect(kindOf("Struct", "Normal", { schema: TERMS }).required).toBe(false);
		expect(kindOf("String", "HashMap").required).toBe(false);
		expect(kindOf("PathBuf", "Normal").required).toBe(true);
	});

	test("short controls: numbers, dates, switches and choices of up to two", () => {
		expect(kindOf("Integer", "Normal").short).toBe(true);
		expect(kindOf("Date", "Normal").short).toBe(true);
		expect(kindOf("Boolean", "Normal").short).toBe(true);
		expect(
			kindOf("String", "Normal", { valid_values: ["Chrome", "Edge"] }).short,
		).toBe(true);
		expect(
			kindOf("String", "Normal", { valid_values: ["EUR", "USD", "CHF"] }).short,
		).toBe(false);
		expect(kindOf("String", "Normal").short).toBe(false);
	});
});

describe("FlowPath fields", () => {
	test("the schema schemars writes, or path and store_ref both required", () => {
		expect(isFlowPathSchema(FLOWPATH)).toBe(true);
		expect(isFlowPathSchema(JSON.parse(FLOWPATH))).toBe(true);
		expect(
			isFlowPathSchema(
				'{"properties":{"path":{},"store_ref":{}},"required":["path","store_ref"]}',
			),
		).toBe(true);
		expect(
			isFlowPathSchema(
				'{"properties":{"path":{},"store_ref":{}},"required":["path"]}',
			),
		).toBe(false);
		expect(isFlowPathSchema(`{"type":"array","items":${FLOWPATH}}`)).toBe(true);
	});

	test("an unresolved ref key, empty or broken text is not FlowPath", () => {
		expect(isFlowPathSchema("16248035215404677707")).toBe(false);
		expect(isFlowPathSchema("")).toBe(false);
		expect(isFlowPathSchema(null)).toBe(false);
		expect(isFlowPathSchema("{oops")).toBe(false);
		expect(isFlowPathSchema(OPEN_OBJECT)).toBe(false);
	});
});

describe("labels and help", () => {
	test("identifiers are humanised, labels are kept", () => {
		expect(humanizeLabel("youtube_url")).toBe("Youtube url");
		expect(humanizeLabel("invoiceDate")).toBe("Invoice date");
		expect(humanizeLabel("vendor")).toBe("Vendor");
		expect(humanizeLabel("API_KEY")).toBe("Api key");
		expect(humanizeLabel("Invoice date")).toBe("Invoice date");
		expect(humanizeLabel("URL")).toBe("URL");
		expect(humanizeLabel("E-mail")).toBe("E-mail");
		expect(humanizeLabel("  Run OCR ")).toBe("Run OCR");
	});

	test("the friendly name, else the name", () => {
		expect(
			one(input("vendor_name", "String", { friendly_name: "Vendor" })).label,
		).toBe("Vendor");
		expect(
			one(input("vendor_name", "String", { friendly_name: "youtube_url" }))
				.label,
		).toBe("Youtube url");
		expect(
			one(input("invoice_date", "Date", { friendly_name: "  " })).label,
		).toBe("Invoice date");
	});

	test("help is the description unless it is empty or an unresolved ref key", () => {
		expect(
			one(
				input("a", "String", { description: " PDF or scan of the invoice. " }),
			).help,
		).toBe("PDF or scan of the invoice.");
		expect(
			one(input("a", "String", { description: EMPTY_STRING_REF })).help,
		).toBeNull();
		expect(
			one(input("a", "String", { description: "8423759023485" })).help,
		).toBeNull();
		expect(one(input("a", "String", { description: "" })).help).toBeNull();
	});
});

describe("seeding: today's behaviour, files never, secrets never", () => {
	test("the stored default is decoded into the control's value", () => {
		expect(
			one(input("v", "String", { default_value: bytes("Nordwind") })),
		).toMatchObject({
			defaultValue: "Nordwind",
			hasDefault: true,
		});
		expect(
			one(input("v", "Integer", { default_value: bytes(20) })).defaultValue,
		).toBe("20");
		expect(
			one(input("v", "Float", { default_value: bytes(2.5) })).defaultValue,
		).toBe("2.5");
		expect(
			one(input("v", "Boolean", { default_value: bytes(true) })).defaultValue,
		).toBe(true);
		expect(
			one(input("v", "Date", { default_value: bytes("2026-10-03T23:30:00Z") }))
				.defaultValue,
		).toBe("2026-10-03");
		expect(
			one(
				input("v", "String", {
					value_type: "Array",
					default_value: bytes(["4400", "4410"]),
				}),
			).defaultValue,
		).toEqual(["4400", "4410"]);
		expect(
			one(input("v", "Generic", { default_value: bytes({ a: 1 }) }))
				.defaultValue,
		).toBe('{\n  "a": 1\n}');
		expect(
			one(input("v", "String", { default_value: bytes("") })),
		).toMatchObject({
			defaultValue: "",
			hasDefault: true,
		});
	});

	test("an optional field without a stored default gets the type default: today, 0, off", () => {
		const optional = { optional: true };
		expect(one(input("v", "Date", optional))).toMatchObject({
			defaultValue: TODAY,
			hasDefault: true,
		});
		expect(one(input("v", "Integer", optional))).toMatchObject({
			defaultValue: "0",
			hasDefault: true,
		});
		expect(one(input("v", "Float", optional))).toMatchObject({
			defaultValue: "0",
			hasDefault: true,
		});
		expect(one(input("v", "Boolean", optional))).toMatchObject({
			defaultValue: false,
			hasDefault: true,
		});
		expect(one(input("v", "String", optional))).toMatchObject({
			defaultValue: "",
			hasDefault: false,
		});
		expect(
			one(input("v", "String", { optional: true, value_type: "Array" })),
		).toMatchObject({
			defaultValue: [],
			hasDefault: false,
		});
		expect(
			one(input("v", "String", { optional: true, value_type: "HashMap" })),
		).toMatchObject({
			defaultValue: [],
			hasDefault: false,
		});
	});

	test("a required field without a default starts empty", () => {
		expect(one(input("v", "Date"))).toMatchObject({
			defaultValue: "",
			hasDefault: false,
		});
		expect(one(input("v", "Integer"))).toMatchObject({
			defaultValue: "",
			hasDefault: false,
		});
		expect(one(input("v", "Boolean"))).toMatchObject({
			defaultValue: false,
			hasDefault: true,
		});
	});

	test("files never get a default", () => {
		expect(
			one(
				input("v", "PathBuf", {
					default_value: bytes("c:/x.pdf"),
					optional: true,
				}),
			),
		).toMatchObject({
			defaultValue: null,
			hasDefault: false,
		});
		expect(
			one(
				input("v", "Struct", {
					schema: FLOWPATH,
					value_type: "Array",
					optional: true,
				}),
			).defaultValue,
		).toEqual([]);
	});

	test("a sensitive pin's default never reaches the control, whatever arrived", () => {
		const sensitive = one(
			input("v", "String", {
				sensitive: true,
				default_value: bytes("hunter2"),
			}),
		);
		expect(sensitive).toMatchObject({
			defaultValue: "",
			hasDefault: false,
			sensitive: true,
		});
		const withheld = one(
			input("v", "String", { default_omitted: true, optional: true }),
		);
		expect(withheld).toMatchObject({
			defaultValue: "",
			hasDefault: false,
			defaultOmitted: true,
		});
	});

	test("a default outside the fixed choices is not used", () => {
		expect(
			one(
				input("v", "String", {
					valid_values: ["Chrome", "Edge"],
					default_value: bytes("Firefox"),
				}),
			).defaultValue,
		).toBe("");
		expect(
			one(
				input("v", "Integer", {
					valid_values: ["1", "2"],
					default_value: bytes(2),
				}),
			).defaultValue,
		).toBe("2");
		expect(
			one(input("v", "Integer", { valid_values: ["1", "2"], optional: true }))
				.defaultValue,
		).toBe("");
	});

	test("a free object's default becomes rows; text that reads as JSON is quoted so it stays text", () => {
		const rows = one(
			input("v", "Struct", {
				default_value: bytes({ customer: "12345", days: 14, ok: true }),
			}),
		);
		expect(rows.defaultValue).toEqual([
			{ id: "saved-0", key: "customer", value: '"12345"' },
			{ id: "saved-1", key: "days", value: "14" },
			{ id: "saved-2", key: "ok", value: "true" },
		]);
		const map = one(
			input("v", "String", {
				value_type: "HashMap",
				default_value: bytes({ customer: "12345" }),
			}),
		);
		expect(map.defaultValue).toEqual([
			{ id: "saved-0", key: "customer", value: "12345" },
		]);
	});
});

describe("objects with a flat schema", () => {
	const terms = one(
		input("payment_terms", "Struct", {
			schema: TERMS,
			optional: true,
			default_value: bytes({
				currency: "EUR",
				net_days: 14,
				discount_percent: 2,
			}),
		}),
	);

	test("properties keep schema order with their own keys, kinds and required flags", () => {
		expect(
			terms.props.map((prop) => [
				prop.key,
				prop.name,
				prop.kind,
				prop.dataType,
				prop.required,
				prop.dateFormat,
			]),
		).toEqual([
			[
				`payment_terms${FIELD_KEY_SEPARATOR}currency`,
				"currency",
				"choice",
				"String",
				true,
				null,
			],
			[
				`payment_terms${FIELD_KEY_SEPARATOR}net_days`,
				"net_days",
				"number",
				"Integer",
				true,
				null,
			],
			[
				`payment_terms${FIELD_KEY_SEPARATOR}discount_percent`,
				"discount_percent",
				"number",
				"Float",
				false,
				null,
			],
			[
				`payment_terms${FIELD_KEY_SEPARATOR}discount_days`,
				"discount_days",
				"number",
				"Integer",
				false,
				null,
			],
			[
				`payment_terms${FIELD_KEY_SEPARATOR}due`,
				"due",
				"date",
				"Date",
				false,
				"date",
			],
			[
				`payment_terms${FIELD_KEY_SEPARATOR}sent_at`,
				"sent_at",
				"date",
				"Date",
				false,
				"dateTime",
			],
			[
				`payment_terms${FIELD_KEY_SEPARATOR}urgent`,
				"urgent",
				"bool",
				"Boolean",
				false,
				null,
			],
		]);
		expect(terms.props[0].options).toEqual(["EUR", "USD", "CHF", "GBP"]);
		expect(terms.props[1].range).toEqual([0, 365]);
		expect(terms.props.map((prop) => prop.label).slice(0, 2)).toEqual([
			"Currency",
			"Net days",
		]);
	});

	test("the stored default fills the properties it has; the object as a whole has a default", () => {
		expect(terms.defaultValue).toEqual({
			currency: "EUR",
			net_days: "14",
			discount_percent: "2",
			discount_days: "",
			due: "",
			sent_at: "",
			urgent: false,
		});
		expect(terms.hasDefault).toBe(true);
		expect(terms.props.map((prop) => prop.hasDefault)).toEqual([
			true,
			true,
			true,
			false,
			false,
			false,
			true,
		]);
	});

	test('the pin\'s own optional flag is kept for "Optional"', () => {
		expect(pinOptional(terms)).toBe(true);
		expect(pinOptional(one(input("t", "Struct", { schema: TERMS })))).toBe(
			false,
		);
		expect(
			pinOptional({
				...terms,
				optional: undefined,
			} as unknown as WorkbenchField),
		).toBe(true);
	});
});

describe("keys and cursor stops", () => {
	test("an object's property key joins both names and splits back", () => {
		const key = fieldKey("payment_terms", "currency");
		expect(key).toBe(`payment_terms${FIELD_KEY_SEPARATOR}currency`);
		expect(splitKey(key)).toEqual({
			name: "payment_terms",
			property: "currency",
		});
		expect(splitKey("vendor_name")).toEqual({
			name: "vendor_name",
			property: null,
		});
		expect(fieldKey(null, "vendor_name")).toBe("vendor_name");
	});

	test("targets list fields and object properties in order; skipped fields are no stop", () => {
		const fields = fieldsFromEvent(
			[
				input("invoice_file", "Struct", { schema: FLOWPATH }),
				input("vendor_name", "String"),
				input("terms", "Struct", {
					schema:
						'{"type":"object","properties":{"a":{"type":"string"},"b":{"type":"integer"}}}',
				}),
			],
			TODAY,
		);
		expect(targets(fields).map((target) => [target.key, target.group])).toEqual(
			[
				["invoice_file", null],
				["vendor_name", null],
				[fieldKey("terms", "a"), "terms"],
				[fieldKey("terms", "b"), "terms"],
			],
		);
		expect(
			targets(fields, ["invoice_file"]).map((target) => target.key)[0],
		).toBe("vendor_name");
	});
});

describe("the form model", () => {
	const host = { kind: "app" } as HostCapabilities;
	const viewer = { mac: true, locale: "en-GB" } as ViewerHabits;
	const event = {
		id: "ev-1",
		node_id: "node-1",
		name: "Extract invoice",
		description: " Upload an invoice. ",
		route: "invoices",
		inputs: [input("vendor_name", "String", { friendly_name: "Vendor" })],
	} as unknown as IEvent;

	test("routes are normalised and deduplicated; the submit label and app id are kept", () => {
		const model = createFormModel(
			event,
			{
				navigate_to_routes: ["chat", "/chat", " /review ", ""],
				submit_label: " Extract ",
			},
			host,
			viewer,
			TODAY,
			"app-1",
		);
		expect(model).toMatchObject({
			appId: "app-1",
			eventId: "ev-1",
			nodeId: "node-1",
			name: "Extract invoice",
			description: "Upload an invoice.",
			routes: ["/chat", "/review", "/"],
			eventRoute: "/invoices",
			submitLabel: "Extract",
			host,
			viewer,
		});
		expect(model.fields.map((field) => field.name)).toEqual(["vendor_name"]);
	});

	test("no config: no routes, no submit label", () => {
		const model = createFormModel(
			{ ...event, route: null } as IEvent,
			undefined,
			host,
			viewer,
			TODAY,
			"app-1",
		);
		expect([model.routes, model.submitLabel, model.eventRoute]).toEqual([
			[],
			null,
			null,
		]);
		expect(
			createFormModel(event, { submit_label: "  " }, host, viewer, TODAY, "a")
				.submitLabel,
		).toBeNull();
	});

	test("the content key changes only with what the form shows", () => {
		const a = createFormModel(event, undefined, host, viewer, TODAY, "app-1");
		const b = createFormModel(event, undefined, host, viewer, TODAY, "app-2");
		expect(a.contentKey).toBe(b.contentKey);
		const renamed = {
			...event,
			inputs: [input("vendor_name", "String", { friendly_name: "Supplier" })],
		} as unknown as IEvent;
		expect(
			createFormModel(renamed, undefined, host, viewer, TODAY, "app-1")
				.contentKey,
		).not.toBe(a.contentKey);
		expect(
			formContentKey({
				name: "a",
				description: "",
				fields: [],
				routes: [],
				eventRoute: null,
				submitLabel: "Go",
			}),
		).not.toBe(
			formContentKey({
				name: "a",
				description: "",
				fields: [],
				routes: [],
				eventRoute: null,
				submitLabel: null,
			}),
		);
	});

	test("routes", () => {
		expect(normalizeRoute(" chat ")).toBe("/chat");
		expect(normalizeRoute("")).toBe("/");
		expect(normalizeRoute("/review")).toBe("/review");
	});
});
