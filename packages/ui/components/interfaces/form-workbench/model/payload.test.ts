import { describe, expect, test } from "bun:test";
import {
	type BuildPayload,
	FIELD_KEY_SEPARATOR,
	type FileRef,
	type FileSlot,
	REQUEST_FILES_STORE_REF,
	type WorkbenchEventInput,
	type WorkbenchField,
} from "../contracts";
import { fieldsFromEvent } from "./fields";
import { buildPayload, dateWire, readNumber, sentValue } from "./payload";
import { HIDDEN_VALUE } from "./values";

const _contract: BuildPayload = buildPayload;
void _contract;

const TODAY = "2026-10-05";
const FLOWPATH =
	'{"title":"FlowPath","type":"object","properties":{"path":{"type":"string"},"store_ref":{"type":"string"}},"required":["path","store_ref"]}';
const bytes = (value: unknown) =>
	Array.from(new TextEncoder().encode(JSON.stringify(value)));

function input(
	name: string,
	dataType: string,
	rest: Partial<WorkbenchEventInput> = {},
): WorkbenchEventInput {
	return {
		id: name,
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

const one = (item: WorkbenchEventInput): WorkbenchField =>
	fieldsFromEvent([item], TODAY)[0];
const send = (item: WorkbenchEventInput, value: unknown) =>
	buildPayload([one(item)], { [item.name]: value as never });

function flowRef(path: string): FileRef {
	return {
		kind: "flowpath",
		flowPath: {
			path,
			store_ref: REQUEST_FILES_STORE_REF,
			cache_store_ref: null,
		},
		url: `https://files.example/${path}`,
	};
}

function slot(
	name: string,
	ref: FileRef | null,
	rest: Partial<FileSlot> = {},
): FileSlot {
	return {
		id: `slot-${name}`,
		name,
		size: 1000,
		type: "application/pdf",
		state: "sent",
		progress: null,
		ref,
		error: null,
		sentAt: 1,
		expiresAt: null,
		...rest,
	};
}

describe("files", () => {
	const invoice = input("invoice_file", "Struct", { schema: FLOWPATH });
	const documents = input("supporting_documents", "Struct", {
		schema: FLOWPATH,
		value_type: "Array",
		optional: true,
	});

	test("FlowPath fields retain the upload URL for handover, for one file and a list", () => {
		expect(send(invoice, slot("a.pdf", flowRef("tmp/a.pdf")))).toEqual({
			ok: true,
			payload: {
				invoice_file: {
					path: "tmp/a.pdf",
					store_ref: REQUEST_FILES_STORE_REF,
					cache_store_ref: null,
					url: "https://files.example/tmp/a.pdf",
					name: "a.pdf",
				},
			},
		});
		expect(
			send(documents, [
				slot("a.pdf", flowRef("tmp/a")),
				slot("b.pdf", flowRef("tmp/b")),
			]),
		).toEqual({
			ok: true,
			payload: {
				supporting_documents: [
					{
						path: "tmp/a",
						store_ref: REQUEST_FILES_STORE_REF,
						cache_store_ref: null,
						url: "https://files.example/tmp/a",
						name: "a.pdf",
					},
					{
						path: "tmp/b",
						store_ref: REQUEST_FILES_STORE_REF,
						cache_store_ref: null,
						url: "https://files.example/tmp/b",
						name: "b.pdf",
					},
				],
			},
		});
	});

	test("a locally staged file keeps its FlowPath without download metadata", () => {
		const flowPath = {
			path: "tmp/global/apps/a/events/e/requests/r/photo.png",
			store_ref: REQUEST_FILES_STORE_REF,
			cache_store_ref: null,
		};
		for (const url of [null, ""]) {
			expect(
				send(invoice, slot("photo.png", { kind: "flowpath", flowPath, url })),
			).toEqual({ ok: true, payload: { invoice_file: flowPath } });
		}
	});

	test("a FlowPath field never falls back to a URL", () => {
		expect(
			send(invoice, slot("a.pdf", { kind: "url", url: "https://x/a.pdf" })),
		).toEqual({
			ok: false,
			problems: { invoice_file: { code: "fileFailed", fileName: "a.pdf" } },
		});
	});

	test("a legacy PathBuf/Byte field sends URL strings, as today", () => {
		const receipt = input("receipt", "PathBuf");
		expect(
			send(receipt, slot("r.jpg", { kind: "url", url: "https://x/r.jpg" })),
		).toEqual({
			ok: true,
			payload: { receipt: "https://x/r.jpg" },
		});
		expect(send(receipt, slot("r.jpg", flowRef("tmp/r")))).toEqual({
			ok: true,
			payload: { receipt: "https://files.example/tmp/r" },
		});
		expect(
			send(input("blobs", "Byte", { value_type: "Array" }), [
				slot("a", { kind: "url", url: "data:a" }),
			]),
		).toEqual({
			ok: true,
			payload: { blobs: ["data:a"] },
		});
		expect(send(receipt, slot("r.jpg", { kind: "inline" }))).toEqual({
			ok: false,
			problems: { receipt: { code: "fileSending", fileName: "r.jpg" } },
		});
	});

	test("a slot that is not sent cannot go: sending, failed, a reminder", () => {
		const problemFor = (state: FileSlot["state"]) =>
			send(invoice, slot("a.pdf", null, { state }));
		expect(problemFor("sending")).toEqual({
			ok: false,
			problems: { invoice_file: { code: "fileSending", fileName: "a.pdf" } },
		});
		expect(problemFor("waiting")).toEqual({
			ok: false,
			problems: { invoice_file: { code: "fileSending", fileName: "a.pdf" } },
		});
		expect(problemFor("failed")).toEqual({
			ok: false,
			problems: { invoice_file: { code: "fileFailed", fileName: "a.pdf" } },
		});
		expect(problemFor("reminder")).toEqual({
			ok: false,
			problems: { invoice_file: { code: "pickAgain", fileName: "a.pdf" } },
		});
	});

	test("an empty optional file field is left out; a required one is a problem", () => {
		expect(send(documents, [])).toEqual({ ok: true, payload: {} });
		expect(send(invoice, null)).toEqual({
			ok: false,
			problems: { invoice_file: { code: "required" } },
		});
	});
});

describe("dates", () => {
	test("a scalar date and a list of dates send an instant (question 1)", () => {
		expect(send(input("invoice_date", "Date"), "2026-09-18")).toEqual({
			ok: true,
			payload: { invoice_date: "2026-09-18T00:00:00Z" },
		});
		expect(
			send(input("days", "Date", { value_type: "Array" }), [
				"2026-09-18",
				"2026-09-21",
			]),
		).toEqual({
			ok: true,
			payload: { days: ["2026-09-18T00:00:00Z", "2026-09-21T00:00:00Z"] },
		});
		expect(send(input("invoice_date", "Date"), "2026-02-30")).toEqual({
			ok: false,
			problems: { invoice_date: { code: "date" } },
		});
	});

	test("an object property follows its schema: format date sends the day, date-time an instant", () => {
		const schema = JSON.stringify({
			type: "object",
			properties: {
				due: { type: "string", format: "date" },
				sent: { type: "string", format: "date-time" },
			},
		});
		expect(
			send(input("terms", "Struct", { schema }), {
				due: "2026-09-18",
				sent: "2026-09-19",
			}),
		).toEqual({
			ok: true,
			payload: { terms: { due: "2026-09-18", sent: "2026-09-19T00:00:00Z" } },
		});
		const prop = one(input("terms", "Struct", { schema })).props[0];
		expect(dateWire(prop, "2026-09-18")).toBe("2026-09-18");
		expect(dateWire(one(input("d", "Date")), "2026-09-18T10:00:00+02:00")).toBe(
			"2026-09-18T10:00:00+02:00",
		);
	});

	test("a date choice sends its day as an instant", () => {
		expect(
			send(
				input("day", "Date", { valid_values: ["2026-09-18"] }),
				"2026-09-18",
			),
		).toEqual({
			ok: true,
			payload: { day: "2026-09-18T00:00:00Z" },
		});
	});
});

describe("numbers", () => {
	test("strict parsing; decimals in a whole-number field; the pin's range", () => {
		expect(send(input("max_pages", "Integer"), " 20 ")).toEqual({
			ok: true,
			payload: { max_pages: 20 },
		});
		expect(send(input("max_pages", "Integer"), "4.2")).toEqual({
			ok: false,
			problems: { max_pages: { code: "noDecimals" } },
		});
		expect(send(input("max_pages", "Integer"), "abc")).toEqual({
			ok: false,
			problems: { max_pages: { code: "integer" } },
		});
		expect(send(input("total", "Float"), "11769.10")).toEqual({
			ok: true,
			payload: { total: 11769.1 },
		});
		expect(send(input("total", "Float"), "Infinity")).toEqual({
			ok: false,
			problems: { total: { code: "number" } },
		});
		expect(send(input("score", "Float", { range: [0, 1] }), "1.5")).toEqual({
			ok: false,
			problems: { score: { code: "range", min: 0, max: 1 } },
		});
	});

	test("a lone comma: refused for a viewer with a decimal point, read for one with a decimal comma", () => {
		const total = one(input("total", "Float"));
		expect(readNumber(total, "1,5", ".")).toEqual({ code: "number" });
		expect(readNumber(total, "1,5", ",")).toBe(1.5);
		expect(readNumber(total, "1,5")).toBe(1.5);
		expect(sentValue(total, "1,5", { decimalSign: "." })).toEqual({
			kind: "problems",
			problems: { total: { code: "number" } },
		});
	});

	test("an integer choice sends a number", () => {
		expect(
			send(input("level", "Integer", { valid_values: ["1", "2"] }), "2"),
		).toEqual({ ok: true, payload: { level: 2 } });
		expect(
			send(input("level", "Integer", { valid_values: ["1", "2"] }), "3"),
		).toEqual({
			ok: false,
			problems: { level: { code: "option" } },
		});
	});
});

describe("lists, rows, objects and JSON", () => {
	test("list entries are typed; a set has no duplicates; fixed choices are kept", () => {
		expect(
			send(input("ids", "Integer", { value_type: "Array" }), ["1", "22"]),
		).toEqual({ ok: true, payload: { ids: [1, 22] } });
		expect(
			send(input("ids", "Integer", { value_type: "Array" }), ["1", "x"]),
		).toEqual({ ok: false, problems: { ids: { code: "items" } } });
		expect(
			send(input("tags", "String", { value_type: "HashSet" }), ["a", "a"]),
		).toEqual({ ok: false, problems: { tags: { code: "unique" } } });
		expect(
			send(
				input("sizes", "String", {
					value_type: "Array",
					valid_values: ["S", "M"],
				}),
				["S", "XL"],
			),
		).toEqual({
			ok: false,
			problems: { sizes: { code: "option" } },
		});
		expect(
			send(
				input("tags", "String", { value_type: "Array", optional: true }),
				[],
			),
		).toEqual({ ok: true, payload: {} });
	});

	test("an object sends its properties without empty optional ones; an empty required property is a problem at its key", () => {
		const schema = JSON.stringify({
			type: "object",
			properties: {
				currency: { type: "string", enum: ["EUR", "USD"] },
				net_days: { type: "integer" },
				discount_percent: { type: ["number", "null"] },
				urgent: { type: "boolean" },
			},
			required: ["currency", "net_days"],
		});
		const terms = input("payment_terms", "Struct", { schema, optional: true });
		expect(
			send(terms, {
				currency: "EUR",
				net_days: "14",
				discount_percent: "",
				urgent: false,
			}),
		).toEqual({
			ok: true,
			payload: {
				payment_terms: { currency: "EUR", net_days: 14, urgent: false },
			},
		});
		expect(
			send(terms, { currency: "EUR", net_days: "", discount_percent: "x" }),
		).toEqual({
			ok: false,
			problems: {
				[`payment_terms${FIELD_KEY_SEPARATOR}net_days`]: { code: "required" },
				[`payment_terms${FIELD_KEY_SEPARATOR}discount_percent`]: {
					code: "number",
				},
			},
		});
	});

	const row = (key: string, value: string) => ({ id: key, key, value });

	test("rows of a map: values typed by its data type", () => {
		expect(
			send(input("limits", "Integer", { value_type: "HashMap" }), [
				row("a", "1"),
				row("b", "2"),
			]),
		).toEqual({
			ok: true,
			payload: { limits: { a: 1, b: 2 } },
		});
		expect(
			send(input("limits", "Integer", { value_type: "HashMap" }), [
				row("a", ""),
			]),
		).toEqual({
			ok: false,
			problems: { limits: { code: "items" } },
		});
		expect(
			send(input("flags", "Boolean", { value_type: "HashMap" }), [
				row("a", "True"),
				row("b", "false"),
			]),
		).toEqual({
			ok: true,
			payload: { flags: { a: true, b: false } },
		});
	});

	test("rows of a free object: values read as JSON when they are; keys are unique and named", () => {
		expect(
			send(input("meta", "Struct"), [
				row("customer", '"12345"'),
				row("days", "14"),
				row("name", "Nordwind"),
				row("tags", '["a"]'),
			]),
		).toEqual({
			ok: true,
			payload: {
				meta: { customer: "12345", days: 14, name: "Nordwind", tags: ["a"] },
			},
		});
		expect(
			send(input("meta", "Struct"), [row("a", "1"), row("a ", "2")]),
		).toEqual({ ok: false, problems: { meta: { code: "unique" } } });
		expect(
			send(input("meta", "Struct"), [{ id: "x", key: " ", value: "1" }]),
		).toEqual({ ok: false, problems: { meta: { code: "items" } } });
		expect(send(input("meta", "Struct"), [])).toEqual({
			ok: true,
			payload: { meta: {} },
		});
	});

	test("JSON boxes: a single Generic value falls back to the text; a Geometry must be an object", () => {
		expect(send(input("any", "Generic"), "hello")).toEqual({
			ok: true,
			payload: { any: "hello" },
		});
		expect(send(input("any", "Generic"), '{"a":1}')).toEqual({
			ok: true,
			payload: { any: { a: 1 } },
		});
		expect(send(input("area", "Geometry"), '"POINT(1 2)"')).toEqual({
			ok: false,
			problems: { area: { code: "object" } },
		});
		expect(
			send(input("area", "Geometry"), '{"type":"Point","coordinates":[1,2]}'),
		).toEqual({
			ok: true,
			payload: { area: { type: "Point", coordinates: [1, 2] } },
		});
		expect(
			send(input("flags", "Boolean", { value_type: "Array" }), "[true, 1]"),
		).toEqual({
			ok: false,
			problems: { flags: { code: "items" } },
		});
		expect(
			send(
				input("nested", "Struct", { schema: '{"$ref":"#/$defs/A"}' }),
				"[1]",
			),
		).toEqual({
			ok: false,
			problems: { nested: { code: "object" } },
		});
	});
});

describe("what is always, never or not sent", () => {
	test("switches are always sent: an optional switch with default On set to Off sends false", () => {
		const ocr = input("run_ocr", "Boolean", {
			optional: true,
			default_value: bytes(true),
		});
		expect(send(ocr, false)).toEqual({ ok: true, payload: { run_ocr: false } });
		expect(buildPayload([one(ocr)], {})).toEqual({
			ok: true,
			payload: { run_ocr: true },
		});
	});

	test("an empty optional field is left out, an empty required field is a problem; text is sent as typed", () => {
		expect(send(input("note", "String", { optional: true }), "  ")).toEqual({
			ok: true,
			payload: {},
		});
		expect(send(input("vendor", "String"), "")).toEqual({
			ok: false,
			problems: { vendor: { code: "required" } },
		});
		expect(send(input("vendor", "String"), " Nordwind ")).toEqual({
			ok: true,
			payload: { vendor: " Nordwind " },
		});
	});

	test("a hidden value cannot be sent; an unsupported field blocks only when required", () => {
		expect(send(input("token", "String"), HIDDEN_VALUE)).toEqual({
			ok: false,
			problems: { token: { code: "enterAgain" } },
		});
		expect(send(input("shape", "Polygon"), null)).toEqual({
			ok: false,
			problems: { shape: { code: "unsupported" } },
		});
		expect(send(input("shape", "Polygon", { optional: true }), null)).toEqual({
			ok: true,
			payload: {},
		});
	});

	test("a form's payload by pin name with every problem by key; a form without fields sends {}", () => {
		const fields = fieldsFromEvent(
			[
				input("vendor", "String"),
				input("pages", "Integer", { optional: true, default_value: bytes(20) }),
				input("day", "Date"),
			],
			TODAY,
		);
		expect(
			buildPayload(fields, { vendor: "", pages: "x", day: "2026-09-18" }),
		).toEqual({
			ok: false,
			problems: { vendor: { code: "required" }, pages: { code: "integer" } },
		});
		expect(
			buildPayload(fields, { vendor: "N", pages: "20", day: "2026-09-18" }),
		).toEqual({
			ok: true,
			payload: { vendor: "N", pages: 20, day: "2026-09-18T00:00:00Z" },
		});
		expect(buildPayload([], {})).toEqual({ ok: true, payload: {} });
	});
});
