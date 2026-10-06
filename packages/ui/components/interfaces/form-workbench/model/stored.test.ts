import { describe, expect, test } from "bun:test";
import {
	FIELD_KEY_SEPARATOR,
	type FieldValues,
	type FileSlot,
	type WorkbenchEventInput,
} from "../contracts";
import { fieldsFromEvent } from "./fields";
import { fromStoredInputs, toStoredInput, toStoredInputs } from "./stored";
import { HIDDEN_VALUE, defaultsOf, sameValues } from "./values";

const TODAY = "2026-10-05";
const FLOWPATH =
	'{"title":"FlowPath","type":"object","properties":{"path":{"type":"string"},"store_ref":{"type":"string"}},"required":["path","store_ref"]}';
const TERMS = JSON.stringify({
	type: "object",
	properties: {
		currency: { type: "string", enum: ["EUR", "USD"] },
		net_days: { type: "integer" },
		pin: { type: "string" },
	},
	required: ["currency", "net_days"],
});
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

const FIELDS = fieldsFromEvent(
	[
		input("invoice_file", "Struct", { schema: FLOWPATH }),
		input("supporting_documents", "Struct", {
			schema: FLOWPATH,
			value_type: "Array",
			optional: true,
		}),
		input("vendor_name", "String"),
		input("api_key", "String", { optional: true }),
		input("invoice_date", "Date"),
		input("expected_total", "Float", {
			optional: true,
			default_value: bytes(0),
		}),
		input("run_ocr", "Boolean", { optional: true, default_value: bytes(true) }),
		input("cost_centers", "String", { value_type: "Array", optional: true }),
		input("payment_terms", "Struct", { schema: TERMS, optional: true }),
		input("labels", "String", { value_type: "HashMap", optional: true }),
		input("extra", "Generic", { optional: true }),
	],
	TODAY,
);

function slot(name: string, size: number | null): FileSlot {
	return {
		id: `s-${name}`,
		name,
		size,
		type: null,
		state: "sent",
		progress: null,
		ref: null,
		error: null,
		sentAt: 1,
		expiresAt: null,
	};
}

const COPY: FieldValues = {
	...defaultsOf(FIELDS),
	invoice_file: slot("invoice-RE-2026-0902.pdf", 1_198_080),
	supporting_documents: [
		slot("Lieferschein LS-77120.pdf", 318_464),
		slot("PO-48213.pdf", null),
	],
	vendor_name: "Nordwind Logistik GmbH",
	api_key: "sk-live-1234",
	invoice_date: "2026-09-17",
	expected_total: "6188.00",
	run_ocr: false,
	cost_centers: ["4400", "4410"],
	payment_terms: { currency: "EUR", net_days: "14", pin: "1234" },
	labels: [
		{ id: "a", key: "team", value: "AP" },
		{ id: "b", key: "", value: "" },
	],
	extra: '{ "a": 1 }',
};
const HIDDEN = new Set(["api_key", `payment_terms${FIELD_KEY_SEPARATOR}pin`]);

describe("what a run record saves", () => {
	const stored = toStoredInputs(FIELDS, COPY, HIDDEN);

	test("files as names and sizes, secrets as { $hidden: true }, numbers as typed, rows without ids", () => {
		expect(stored).toEqual({
			invoice_file: {
				$file: { name: "invoice-RE-2026-0902.pdf", size: 1_198_080 },
			},
			supporting_documents: [
				{ $file: { name: "Lieferschein LS-77120.pdf", size: 318_464 } },
				{ $file: { name: "PO-48213.pdf", size: null } },
			],
			vendor_name: "Nordwind Logistik GmbH",
			api_key: { $hidden: true },
			invoice_date: "2026-09-17",
			expected_total: "6188.00",
			run_ocr: false,
			cost_centers: ["4400", "4410"],
			payment_terms: {
				currency: "EUR",
				net_days: "14",
				pin: { $hidden: true },
			},
			labels: [{ key: "team", value: "AP" }],
			extra: '{ "a": 1 }',
		});
		expect(JSON.parse(JSON.stringify(stored))).toEqual(stored);
	});

	test("a hidden value stays hidden; an empty file field is null", () => {
		expect(toStoredInput(FIELDS[2], HIDDEN_VALUE, new Set())).toEqual({
			$hidden: true,
		});
		expect(toStoredInput(FIELDS[0], null, new Set())).toBeNull();
		expect(toStoredInput(FIELDS[0], undefined, new Set())).toBeNull();
	});
});

describe("saved inputs back as values", () => {
	const stored = toStoredInputs(FIELDS, COPY, HIDDEN);

	test("files come back as Pick again reminders, kept-out values as hidden; the rest as they were", () => {
		const back = fromStoredInputs(FIELDS, stored);
		expect([back.misfit, back.pickAgain, back.enterAgain]).toEqual([
			0,
			3,
			["api_key", `payment_terms${FIELD_KEY_SEPARATOR}pin`],
		]);
		expect(back.values.invoice_file).toMatchObject({
			name: "invoice-RE-2026-0902.pdf",
			state: "reminder",
			ref: null,
		});
		expect(back.values.api_key).toEqual(HIDDEN_VALUE);
		expect(back.values.payment_terms).toEqual({
			currency: "EUR",
			net_days: "14",
			pin: "",
		});
		expect(back.values.labels).toEqual([
			{ id: "saved-0", key: "team", value: "AP" },
		]);
		const visible = FIELDS.filter(
			(field) => !["api_key", "payment_terms"].includes(field.name),
		);
		expect(sameValues(visible, back.values, COPY)).toBe(true);
	});

	test("a run's copy keeps an object's kept-out property hidden; read again, the rail gets it empty", () => {
		const copy = fromStoredInputs(FIELDS, stored, { keepHidden: true });
		expect(copy.values.payment_terms).toEqual({
			currency: "EUR",
			net_days: "14",
			pin: { $hidden: true },
		});
		expect(copy.enterAgain).toEqual([
			"api_key",
			`payment_terms${FIELD_KEY_SEPARATOR}pin`,
		]);
		const rail = fromStoredInputs(FIELDS, copy.values);
		expect(rail.values.payment_terms).toEqual({
			currency: "EUR",
			net_days: "14",
			pin: "",
		});
		expect(rail.enterAgain).toEqual(copy.enterAgain);
		expect(
			toStoredInputs(FIELDS, copy.values, new Set()).payment_terms,
		).toEqual(stored.payment_terms);
	});

	test("an input whose field is gone, or whose value no longer fits, is a misfit; new fields are absent", () => {
		const back = fromStoredInputs(FIELDS, {
			vendor_name: "Nordwind",
			old_field: "x",
			run_ocr: "yes",
			expected_total: 12.5,
			invoice_date: { $hidden: true },
		});
		expect(back.misfit).toBe(2);
		expect(back.values).toEqual({
			vendor_name: "Nordwind",
			expected_total: "12.5",
			invoice_date: HIDDEN_VALUE,
		});
		expect(back.enterAgain).toEqual(["invoice_date"]);
		expect(back.pickAgain).toBe(0);
	});
});
