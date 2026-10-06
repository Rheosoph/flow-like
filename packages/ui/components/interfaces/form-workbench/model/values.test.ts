import { describe, expect, test } from "bun:test";
import {
	FIELD_KEY_SEPARATOR,
	type FieldValues,
	type FileSlot,
	type Preset,
	SECRET_MASK,
	type ShortWords,
	type WorkbenchEventInput,
	type WorkbenchField,
} from "../contracts";
import { formatDate } from "./date-text";
import { fieldsFromEvent } from "./fields";
import {
	HIDDEN_VALUE,
	baselineOf,
	changedNames,
	decimalText,
	defaultsOf,
	diffValues,
	emptyValue,
	fitValue,
	hasReminder,
	isEmpty,
	reminderSlot,
	sameValues,
	shortText,
	startingValue,
	valueAt,
	valueKey,
	withValue,
} from "./values";

const TODAY = "2026-10-05";
const FLOWPATH =
	'{"title":"FlowPath","type":"object","properties":{"path":{"type":"string"},"store_ref":{"type":"string"}},"required":["path","store_ref"]}';
const TERMS = JSON.stringify({
	type: "object",
	properties: {
		currency: { type: "string", enum: ["EUR", "USD", "CHF", "GBP"] },
		net_days: { type: "integer" },
		discount_percent: { type: ["number", "null"] },
		discount_days: { type: ["integer", "null"] },
	},
	required: ["currency", "net_days"],
});
const bytes = (value: unknown) =>
	Array.from(new TextEncoder().encode(JSON.stringify(value)));

function input(
	name: string,
	label: string,
	dataType: string,
	rest: Partial<WorkbenchEventInput> = {},
): WorkbenchEventInput {
	return {
		id: name,
		name,
		friendly_name: label,
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

/** Extract invoice (FLP_FORMS.medium): FlowPath files, defaults as DATA.js. */
const MEDIUM = fieldsFromEvent(
	[
		input("invoice_file", "Invoice", "Struct", { schema: FLOWPATH }),
		input("supporting_documents", "Supporting documents", "Struct", {
			schema: FLOWPATH,
			value_type: "Array",
			optional: true,
		}),
		input("vendor_name", "Vendor", "String"),
		input("invoice_date", "Invoice date", "Date"),
		input("expected_total", "Expected total", "Float", {
			optional: true,
			default_value: bytes(0),
		}),
		input("max_pages", "Max pages", "Integer", {
			optional: true,
			default_value: bytes(20),
		}),
		input("run_ocr", "Run OCR", "Boolean", {
			optional: true,
			default_value: bytes(true),
		}),
		input("cost_centers", "Cost centers", "String", {
			value_type: "Array",
			optional: true,
			default_value: bytes(["4400", "4410"]),
		}),
		input("payment_terms", "Payment terms", "Struct", {
			schema: TERMS,
			optional: true,
			default_value: bytes({
				currency: "EUR",
				net_days: 14,
				discount_percent: 2,
				discount_days: 14,
			}),
		}),
	],
	TODAY,
);
const field = (name: string) =>
	MEDIUM.find((item) => item.name === name) as WorkbenchField;

const words: ShortWords = {
	none: "none",
	empty: "empty",
	on: "On",
	off: "Off",
	files: (count) => (count === 1 ? "1 file" : `${count} files`),
	entries: (count) => (count === 1 ? "1 entry" : `${count} entries`),
	date: (iso) => formatDate(iso, "en-GB"),
};

function slot(name: string, rest: Partial<FileSlot> = {}): FileSlot {
	return {
		id: `slot-${name}`,
		name,
		size: 1_198_080,
		type: "application/pdf",
		state: "sent",
		progress: null,
		ref: null,
		error: null,
		sentAt: null,
		expiresAt: null,
		...rest,
	};
}

/** FL_FILLED.medium as rail values (numbers as typed text). */
const FILLED: FieldValues = {
	...defaultsOf(MEDIUM),
	invoice_file: slot("invoice-RE-2026-0917.pdf"),
	supporting_documents: [
		slot("Lieferschein LS-77120.pdf"),
		slot("PO-48213.pdf"),
	],
	vendor_name: "Nordwind Logistik GmbH",
	invoice_date: "2026-09-17",
	expected_total: "11769.10",
};

const ALPENFRACHT: Preset = {
	id: "p-alpenfracht",
	name: "Alpenfracht AG",
	digit: 1,
	sets: { vendor_name: "Alpenfracht AG", max_pages: 60 },
	kinds: { vendor_name: "text", max_pages: "number" },
	openDefault: false,
	createdAt: 0,
	updatedAt: 0,
	lastUsedAt: null,
};

describe("empty values", () => {
	test("each kind has its empty value", () => {
		expect(MEDIUM.map((item) => emptyValue(item))).toEqual([
			null,
			[],
			"",
			"",
			"",
			"",
			false,
			[],
			{ currency: "", net_days: "", discount_percent: "", discount_days: "" },
		]);
	});

	test("flpEmpty: switches and objects never; whitespace, a hidden value and a reminder are no values", () => {
		expect(isEmpty(field("vendor_name"), "  ")).toBe(true);
		expect(isEmpty(field("vendor_name"), " a ")).toBe(false);
		expect(isEmpty(field("vendor_name"), HIDDEN_VALUE)).toBe(true);
		expect(isEmpty(field("run_ocr"), false)).toBe(false);
		expect(isEmpty(field("payment_terms"), {})).toBe(false);
		expect(
			isEmpty(field("invoice_file"), slot("a.pdf", { state: "reminder" })),
		).toBe(true);
		expect(
			isEmpty(field("invoice_file"), slot("a.pdf", { state: "sending" })),
		).toBe(false);
		expect(
			isEmpty(field("supporting_documents"), [
				slot("a.pdf", { state: "reminder" }),
			]),
		).toBe(true);
		expect(isEmpty(field("cost_centers"), [])).toBe(true);
		expect(hasReminder([slot("a.pdf", { state: "reminder" })])).toBe(true);
		expect(hasReminder(slot("a.pdf"))).toBe(false);
	});
});

describe("comparison keys (flpKey)", () => {
	test("numbers by value, files by name, JSON by content", () => {
		const total = field("expected_total");
		expect(valueKey(total, "11769.10")).toBe(valueKey(total, "11769.1"));
		expect(valueKey(total, "11769,10")).toBe(valueKey(total, "11769.1"));
		expect(valueKey(total, "abc")).toBe("abc");
		const file = field("invoice_file");
		expect(valueKey(file, slot("a.pdf", { size: 1 }))).toBe(
			valueKey(file, slot("a.pdf", { size: 2, id: "other" })),
		);
		expect(valueKey(file, slot("a.pdf", { state: "reminder" }))).toBe(
			valueKey(file, slot("a.pdf")),
		);
		const json = fieldsFromEvent([input("g", "G", "Generic")], TODAY)[0];
		expect(valueKey(json, '{ "a": 1 }')).toBe(valueKey(json, '{"a":1}'));
	});

	test("rows without their ids; a hidden value only equals a hidden value", () => {
		const pairs = fieldsFromEvent(
			[input("p", "P", "String", { value_type: "HashMap" })],
			TODAY,
		)[0];
		expect(
			valueKey(pairs, [
				{ id: "a", key: "x", value: "1" },
				{ id: "b", key: "", value: "" },
			]),
		).toBe(valueKey(pairs, [{ id: "z", key: "x ", value: "1" }]));
		expect(valueKey(field("vendor_name"), HIDDEN_VALUE)).not.toBe(
			valueKey(field("vendor_name"), ""),
		);
	});

	test("flpSame: two value sets that would send the same run", () => {
		expect(
			sameValues(MEDIUM, FILLED, { ...FILLED, expected_total: "11769.1" }),
		).toBe(true);
		expect(sameValues(MEDIUM, FILLED, { ...FILLED, max_pages: "21" })).toBe(
			false,
		);
		expect(sameValues(MEDIUM, FILLED, null)).toBe(false);
	});
});

describe("short text (flpShort)", () => {
	test("the spec's words for each kind", () => {
		expect(shortText(field("cost_centers"), ["4400", "4410"], words)).toBe(
			"4400, 4410",
		);
		expect(shortText(field("run_ocr"), true, words)).toBe("On");
		expect(shortText(field("invoice_date"), "2026-09-18", words)).toBe(
			"18 Sep 2026",
		);
		expect(shortText(field("invoice_date"), "", words)).toBe("empty");
		expect(
			shortText(
				field("supporting_documents"),
				FILLED.supporting_documents,
				words,
			),
		).toBe("2 files");
		expect(shortText(field("supporting_documents"), [], words)).toBe("none");
		expect(shortText(field("invoice_file"), null, words)).toBe("none");
		expect(shortText(field("invoice_file"), FILLED.invoice_file, words)).toBe(
			"invoice-RE-2026-0917.pdf",
		);
		expect(shortText(field("vendor_name"), HIDDEN_VALUE, words)).toBe(
			SECRET_MASK,
		);
		expect(
			shortText(
				field("payment_terms"),
				field("payment_terms").defaultValue,
				words,
			),
		).toBe(
			"Currency EUR · Net days 14 · Discount percent 2 · Discount days 14",
		);
	});
});

describe("values at keys", () => {
	const key = `payment_terms${FIELD_KEY_SEPARATOR}net_days`;

	test("an object's property is read and set inside its group", () => {
		expect(valueAt(FILLED, key)).toBe("14");
		const next = withValue(FILLED, key, "30");
		expect(valueAt(next, key)).toBe("30");
		expect(valueAt(next, `payment_terms${FIELD_KEY_SEPARATOR}currency`)).toBe(
			"EUR",
		);
		expect(valueAt(FILLED, key)).toBe("14");
		expect(withValue(FILLED, "vendor_name", "Alpenfracht AG").vendor_name).toBe(
			"Alpenfracht AG",
		);
		expect(valueAt({}, key)).toBeUndefined();
	});
});

describe("starting values and the dot (flpBaseline, flpChanged)", () => {
	test("on Done only Expected total carries the dot", () => {
		expect(changedNames(MEDIUM, FILLED, null, [])).toEqual(["expected_total"]);
	});

	test("a per-run field never does; a field without a default never does", () => {
		expect(changedNames(MEDIUM, FILLED, null, ["expected_total"])).toEqual([]);
		expect(
			changedNames(MEDIUM, { ...FILLED, vendor_name: "Other" }, null, []),
		).toEqual(["expected_total"]);
	});

	test("the active preset's value is the starting value", () => {
		const base = baselineOf(MEDIUM, ALPENFRACHT);
		expect([base.vendor_name, base.max_pages, base.run_ocr]).toEqual([
			"Alpenfracht AG",
			"60",
			true,
		]);
		expect(startingValue(field("max_pages"), null)).toBe("20");
		const values = {
			...FILLED,
			expected_total: "0",
			vendor_name: "Alpenfracht AG",
			max_pages: "60",
		};
		expect(changedNames(MEDIUM, values, ALPENFRACHT, [])).toEqual([]);
		expect(
			changedNames(
				MEDIUM,
				{ ...values, vendor_name: "Nordwind" },
				ALPENFRACHT,
				[],
			),
		).toEqual(["vendor_name"]);
	});
});

describe("differences (change chips, Compare, the edited chip)", () => {
	const isSecret = (item: WorkbenchField) => item.name === "vendor_name";

	test("field by field in form order; an object's properties one by one", () => {
		const before = { ...FILLED, max_pages: "40" };
		const after = withValue(
			{ ...FILLED, invoice_file: slot("invoice-RE-2026-0918.pdf") },
			`payment_terms${FIELD_KEY_SEPARATOR}net_days`,
			"30",
		);
		expect(
			diffValues(MEDIUM, before, after, { words, isSecret: () => false }),
		).toEqual([
			{
				name: "invoice_file",
				label: "Invoice",
				from: "invoice-RE-2026-0917.pdf",
				to: "invoice-RE-2026-0918.pdf",
			},
			{ name: "max_pages", label: "Max pages", from: "40", to: "20" },
			{
				name: `payment_terms${FIELD_KEY_SEPARATOR}net_days`,
				label: "Net days",
				from: "14",
				to: "30",
			},
		]);
	});

	test("skipped (per-run) fields are left out; secrets and hidden values read ••••", () => {
		const after = {
			...FILLED,
			vendor_name: "Alpenfracht AG",
			invoice_date: "2026-09-18",
		};
		expect(
			diffValues(MEDIUM, FILLED, after, {
				words,
				isSecret,
				skip: ["invoice_date"],
			}),
		).toEqual([
			{
				name: "vendor_name",
				label: "Vendor",
				from: SECRET_MASK,
				to: SECRET_MASK,
			},
		]);
		const hidden = { ...FILLED, invoice_date: HIDDEN_VALUE };
		expect(
			diffValues(MEDIUM, hidden, after, {
				words,
				isSecret: () => false,
				skip: ["vendor_name"],
			}),
		).toEqual([
			{
				name: "invoice_date",
				label: "Invoice date",
				from: SECRET_MASK,
				to: SECRET_MASK,
			},
		]);
	});
});

describe("saved values that still fit (spec §5)", () => {
	test("whole number ↔ decimal without a fraction; text that reads as a number", () => {
		expect(fitValue(field("max_pages"), 20)).toBe("20");
		expect(fitValue(field("max_pages"), "20.0")).toBe("20");
		expect(fitValue(field("max_pages"), 20.5)).toBeUndefined();
		expect(fitValue(field("expected_total"), "42")).toBe("42");
		expect(fitValue(field("expected_total"), "11769.10")).toBe("11769.10");
		expect(fitValue(field("expected_total"), "abc")).toBeUndefined();
		expect(fitValue(field("expected_total"), true)).toBeUndefined();
	});

	test("one-item list ↔ text", () => {
		expect(fitValue(field("vendor_name"), ["Nordwind"])).toBe("Nordwind");
		expect(fitValue(field("vendor_name"), ["a", "b"])).toBeUndefined();
		expect(fitValue(field("cost_centers"), "4400")).toEqual(["4400"]);
		expect(fitValue(field("cost_centers"), [4400, "4410"])).toEqual([
			"4400",
			"4410",
		]);
		expect(fitValue(field("cost_centers"), "")).toEqual([]);
	});

	test("choices, dates, switches", () => {
		const currency = field("payment_terms").props[0];
		expect(fitValue(currency, "EUR")).toBe("EUR");
		expect(fitValue(currency, "JPY")).toBeUndefined();
		expect(fitValue(field("invoice_date"), "2026-09-17")).toBe("2026-09-17");
		expect(fitValue(field("invoice_date"), "2026-09-17T08:00:00Z")).toBe(
			"2026-09-17",
		);
		expect(fitValue(field("invoice_date"), "17.09.2026")).toBeUndefined();
		expect(fitValue(field("run_ocr"), false)).toBe(false);
		expect(fitValue(field("run_ocr"), "true")).toBeUndefined();
		const level = fieldsFromEvent(
			[input("level", "Level", "Integer", { valid_values: ["1", "2"] })],
			TODAY,
		)[0];
		expect(fitValue(level, 2)).toBe("2");
		expect(fitValue(level, "2.0")).toBe("2");
	});

	test("files saved as names come back as Pick again reminders; slots stay slots", () => {
		const reminder = fitValue(field("invoice_file"), {
			$file: { name: "invoice-RE-2026-0902.pdf", size: 1 },
		});
		expect(reminder).toMatchObject({
			name: "invoice-RE-2026-0902.pdf",
			size: 1,
			state: "reminder",
			ref: null,
		});
		expect(fitValue(field("invoice_file"), FILLED.invoice_file)).toBe(
			FILLED.invoice_file,
		);
		const many = fitValue(field("supporting_documents"), [
			{ $file: { name: "a.pdf", size: null } },
			{ $file: { name: "b.pdf", size: 2 } },
		]);
		expect((many as FileSlot[]).map((item) => [item.name, item.state])).toEqual(
			[
				["a.pdf", "reminder"],
				["b.pdf", "reminder"],
			],
		);
		expect(
			fitValue(field("invoice_file"), [
				{ $file: { name: "a.pdf", size: 1 } },
				{ $file: { name: "b.pdf", size: 1 } },
			]),
		).toBeUndefined();
		expect(reminderSlot("r", "a.pdf", null).state).toBe("reminder");
	});

	test("objects keep what fits and take their defaults for the rest; rows from objects", () => {
		expect(
			fitValue(field("payment_terms"), {
				currency: "USD",
				net_days: 30.5,
				extra: 1,
			}),
		).toEqual({
			currency: "USD",
			net_days: "14",
			discount_percent: "2",
			discount_days: "14",
		});
		expect(fitValue(field("payment_terms"), "EUR")).toBeUndefined();
		const pairs = fieldsFromEvent([input("p", "P", "Struct")], TODAY)[0];
		expect(fitValue(pairs, [{ key: "a", value: "1" }])).toEqual([
			{ id: "saved-0", key: "a", value: "1" },
		]);
		expect(fitValue(pairs, { a: "true", b: 2 })).toEqual([
			{ id: "saved-0", key: "a", value: '"true"' },
			{ id: "saved-1", key: "b", value: "2" },
		]);
	});

	test("null is the empty value; a hidden or missing value does not fit", () => {
		expect(fitValue(field("vendor_name"), null)).toBe("");
		expect(fitValue(field("invoice_file"), null)).toBeNull();
		expect(fitValue(field("vendor_name"), HIDDEN_VALUE)).toBeUndefined();
		expect(fitValue(field("vendor_name"), undefined)).toBeUndefined();
	});

	test("a lone comma is a decimal comma", () => {
		expect(decimalText("11769,10")).toBe("11769.10");
		expect(decimalText("1.234,5")).toBe("1.234,5");
		expect(decimalText("12")).toBe("12");
	});
});
