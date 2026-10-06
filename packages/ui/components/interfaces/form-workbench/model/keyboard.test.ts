import { describe, expect, test } from "bun:test";
import {
	type EnterMayRun,
	type EnterTarget,
	FIELD_KEY_SEPARATOR,
	type FieldValues,
	type FileSlot,
	type FocusAfterRun,
	type WorkbenchEventInput,
} from "../contracts";
import { fieldsFromEvent } from "./fields";
import { enterHint, enterMayRun, enterTarget, focusAfterRun } from "./keyboard";
import { defaultsOf } from "./values";

const _enterTarget: EnterTarget = enterTarget;
const _enterMayRun: EnterMayRun = enterMayRun;
const _focusAfterRun: FocusAfterRun = focusAfterRun;
void [_enterTarget, _enterMayRun, _focusAfterRun];

const TODAY = "2026-10-05";
const FLOWPATH =
	'{"title":"FlowPath","type":"object","properties":{"path":{"type":"string"},"store_ref":{"type":"string"}},"required":["path","store_ref"]}';
const TERMS = JSON.stringify({
	type: "object",
	properties: {
		currency: { type: "string", enum: ["EUR", "USD"] },
		net_days: { type: "integer" },
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

const MEDIUM = fieldsFromEvent(
	[
		input("invoice_file", "Struct", { schema: FLOWPATH }),
		input("supporting_documents", "Struct", {
			schema: FLOWPATH,
			value_type: "Array",
			optional: true,
		}),
		input("vendor_name", "String"),
		input("invoice_date", "Date"),
		input("expected_total", "Float", {
			optional: true,
			default_value: bytes(0),
		}),
		input("max_pages", "Integer", { optional: true, default_value: bytes(20) }),
		input("run_ocr", "Boolean", { optional: true, default_value: bytes(true) }),
		input("payment_terms", "Struct", {
			schema: TERMS,
			optional: true,
			default_value: bytes({ currency: "EUR", net_days: 14 }),
		}),
	],
	TODAY,
);
const IDLE = defaultsOf(MEDIUM);
const PER_RUN = ["invoice_file", "supporting_documents", "invoice_date"];

function slot(name: string, state: FileSlot["state"] = "sent"): FileSlot {
	return {
		id: name,
		name,
		size: 1,
		type: null,
		state,
		progress: null,
		ref: null,
		error: null,
		sentAt: null,
		expiresAt: null,
	};
}

describe("↵ moves on (flpEnterTarget)", () => {
	test("the benchmark walk: Invoice → Vendor → Invoice date → run", () => {
		const picked: FieldValues = {
			...IDLE,
			invoice_file: slot("invoice-RE-2026-0918.pdf"),
		};
		expect(enterTarget(MEDIUM, picked, "invoice_file", [])).toBe("vendor_name");
		const vendor = { ...picked, vendor_name: "Nordwind Logistik GmbH" };
		expect(enterTarget(MEDIUM, vendor, "vendor_name", [])).toBe("invoice_date");
		expect(
			enterTarget(
				MEDIUM,
				{ ...vendor, invoice_date: "2026-09-18" },
				"invoice_date",
				[],
			),
		).toBeNull();
	});

	test("it wraps to the top and never stops on the field it starts from", () => {
		expect(
			enterTarget(
				MEDIUM,
				{ ...IDLE, vendor_name: "N", invoice_date: "2026-09-18" },
				"max_pages",
				[],
			),
		).toBe("invoice_file");
		expect(
			enterTarget(
				MEDIUM,
				{ ...IDLE, invoice_file: slot("a"), vendor_name: "N" },
				"invoice_date",
				[],
			),
		).toBeNull();
		expect(enterTarget(MEDIUM, IDLE, "nowhere", [])).toBe("invoice_file");
	});

	test("a blocked file field is no stop; a Pick again reminder is one", () => {
		expect(
			enterTarget(MEDIUM, IDLE, "vendor_name", [
				"invoice_file",
				"supporting_documents",
			]),
		).toBe("invoice_date");
		const reminders = {
			...IDLE,
			vendor_name: "N",
			invoice_date: "2026-09-18",
			supporting_documents: [slot("PO-48213.pdf", "reminder")],
			invoice_file: slot("a"),
		};
		expect(enterTarget(MEDIUM, reminders, "vendor_name", [])).toBe(
			"supporting_documents",
		);
	});

	test("an object's required properties are stops of their own", () => {
		const terms = {
			...IDLE,
			invoice_file: slot("a"),
			vendor_name: "N",
			invoice_date: "2026-09-18",
			payment_terms: { currency: "", net_days: "14" },
		};
		expect(enterTarget(MEDIUM, terms, "invoice_date", [])).toBe(
			`payment_terms${FIELD_KEY_SEPARATOR}currency`,
		);
	});

	test("unsupported fields are never a stop", () => {
		const fields = fieldsFromEvent(
			[input("a", "String"), input("shape", "Polygon"), input("b", "String")],
			TODAY,
		);
		expect(enterTarget(fields, { a: "x", shape: null, b: "" }, "a", [])).toBe(
			"b",
		);
		expect(
			enterTarget(fields, { a: "x", shape: null, b: "y" }, "a", []),
		).toBeNull();
	});

	test("enterkeyhint: next while ↵ moves on, else go", () => {
		expect(enterHint(MEDIUM, IDLE, "invoice_file", [])).toBe("next");
		expect(
			enterHint(
				MEDIUM,
				{
					...IDLE,
					invoice_file: slot("a"),
					vendor_name: "N",
					invoice_date: "2026-09-18",
				},
				"vendor_name",
				[],
			),
		).toBe("go");
	});
});

describe("↵ never repeats the newest run (flpEnterMayRun)", () => {
	const filled = {
		...IDLE,
		invoice_file: slot("a.pdf"),
		vendor_name: "N",
		invoice_date: "2026-09-18",
	};

	test("the same inputs as the newest run of this session start nothing", () => {
		expect(enterMayRun(MEDIUM, filled, filled)).toBe(false);
		expect(enterMayRun(MEDIUM, filled, { ...filled, max_pages: "20.0" })).toBe(
			false,
		);
		expect(enterMayRun(MEDIUM, { ...filled, max_pages: "21" }, filled)).toBe(
			true,
		);
		expect(enterMayRun(MEDIUM, filled, null)).toBe(true);
	});
});

describe("the cursor after a run (flpFocusAfterRun)", () => {
	test("in a series: the date after a run, the invoice's drop row after the last file", () => {
		const next = {
			...IDLE,
			invoice_file: slot("invoice-RE-2026-0919.pdf"),
			vendor_name: "N",
			invoice_date: "",
		};
		expect(focusAfterRun(MEDIUM, next, PER_RUN, [])).toBe("invoice_date");
		const last = {
			...IDLE,
			invoice_file: null,
			vendor_name: "N",
			invoice_date: "",
		};
		expect(focusAfterRun(MEDIUM, last, PER_RUN, [])).toBe("invoice_file");
	});

	test("nothing per run: the cursor stays", () => {
		expect(focusAfterRun(MEDIUM, IDLE, [], [])).toBeNull();
	});

	test("no per-run field is missing: the first empty required stop, else the first per-run field", () => {
		const vendorEmpty = {
			...IDLE,
			invoice_file: slot("a"),
			vendor_name: "",
			invoice_date: "2026-09-18",
		};
		expect(focusAfterRun(MEDIUM, vendorEmpty, PER_RUN, [])).toBe("vendor_name");
		const full = { ...vendorEmpty, vendor_name: "N" };
		expect(focusAfterRun(MEDIUM, full, ["max_pages", "invoice_date"], [])).toBe(
			"invoice_date",
		);
		expect(focusAfterRun(MEDIUM, full, ["payment_terms"], [])).toBe(
			`payment_terms${FIELD_KEY_SEPARATOR}currency`,
		);
	});

	test("blocked fields are skipped", () => {
		expect(
			focusAfterRun(MEDIUM, IDLE, PER_RUN, [
				"invoice_file",
				"supporting_documents",
			]),
		).toBe("invoice_date");
	});
});
