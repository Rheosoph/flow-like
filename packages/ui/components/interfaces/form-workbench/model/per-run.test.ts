import { describe, expect, test } from "bun:test";
import {
	type FieldValues,
	type FileSlot,
	type Preset,
	SECRET_MASK,
	type ShortWords,
	type StartMessage,
	type WorkbenchEventInput,
	type WorkbenchField,
} from "../contracts";
import { formatDate } from "./date-text";
import { fieldsFromEvent } from "./fields";
import {
	type OfferRun,
	afterRunRows,
	applyPerRun,
	autoPerRun,
	filesAndDates,
	offerNames,
	pairing,
	startMessage,
} from "./per-run";
import { HIDDEN_VALUE, baselineOf, defaultsOf } from "./values";

const TODAY = "2026-10-05";
const FLOWPATH =
	'{"title":"FlowPath","type":"object","properties":{"path":{"type":"string"},"store_ref":{"type":"string"}},"required":["path","store_ref"]}';
const TERMS =
	'{"type":"object","properties":{"currency":{"type":"string"},"net_days":{"type":"integer"}}}';
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
			default_value: bytes({ currency: "EUR", net_days: 14 }),
		}),
	],
	TODAY,
);
const SMALL = fieldsFromEvent(
	[
		input("order", "Order number", "String"),
		input("quantity", "Quantity", "Integer", { default_value: bytes(1) }),
		input("receipt", "Receipt", "Struct", { schema: FLOWPATH }),
	],
	TODAY,
);
const LARGE = fieldsFromEvent(
	[
		input("portal_url", "Portal URL", "String"),
		input("customer_number", "Customer number", "String"),
		input("date_from", "From", "Date"),
		input("date_to", "To", "Date", { optional: true }),
		input("baseline_export", "Baseline export", "Struct", {
			schema: FLOWPATH,
			optional: true,
		}),
		input("reference_documents", "Reference documents", "Struct", {
			schema: FLOWPATH,
			value_type: "Array",
			optional: true,
		}),
		input("report_title", "Report title", "String"),
	],
	TODAY,
);

const PER_RUN = ["invoice_file", "supporting_documents", "invoice_date"];
const noSecret = () => false;
const words: ShortWords = {
	none: "none",
	empty: "empty",
	on: "On",
	off: "Off",
	files: (count) => `${count} files`,
	entries: (count) => `${count} entries`,
	date: (iso) => formatDate(iso, "en-GB"),
};

function slot(name: string, size = 1000): FileSlot {
	return {
		id: name,
		name,
		size,
		type: null,
		state: "sent",
		progress: null,
		ref: null,
		error: null,
		sentAt: null,
		expiresAt: null,
	};
}

const INVOICES = [18, 19, 20, 21, 22, 23, 24, 25, 26, 27].map((n) =>
	slot(`invoice-RE-2026-09${n}.pdf`),
);
const DATES = ["18", "21", "21", "22", "23", "24", "25", "28", "29", "30"].map(
	(day) => `2026-09-${day}`,
);

describe("per run for one series (flpAutoPerRun)", () => {
	test("Extract invoice at its defaults: Invoice, Supporting documents and Invoice date", () => {
		expect(autoPerRun(MEDIUM, "invoice_file", defaultsOf(MEDIUM))).toEqual(
			PER_RUN,
		);
	});

	test("the 29-field form: Reference documents comes along, From (which holds 1 Sep 2026) does not", () => {
		const values = { ...defaultsOf(LARGE), date_from: "2026-09-01" };
		expect(autoPerRun(LARGE, "baseline_export", values)).toEqual([
			"baseline_export",
			"reference_documents",
		]);
	});

	test("Make files and dates per run: every file field and every required date field", () => {
		expect(filesAndDates(MEDIUM)).toEqual(PER_RUN);
		expect(filesAndDates(LARGE)).toEqual([
			"date_from",
			"baseline_export",
			"reference_documents",
		]);
	});
});

describe("the one-time offer (flpOfferFields)", () => {
	const parcels = (replaced: readonly string[]): OfferRun[] =>
		[
			{
				order: "48240-3",
				quantity: "1",
				receipt: slot("receipt-48240-3.jpg", 455_680),
			},
			{
				order: "48231-1",
				quantity: "1",
				receipt: slot("receipt-48231-1.jpg", 389_120),
			},
			{
				order: "48213-7",
				quantity: "2",
				receipt: slot("receipt-48213-7.jpg", 412_518),
			},
		].map((values) => ({ values, replaced }));

	test("three parcels, each order number typed over the last: Order number and Receipt", () => {
		expect(offerNames(SMALL, parcels(["order"]), [], noSecret)).toEqual([
			"order",
			"receipt",
		]);
	});

	test("each order number edited in place: Receipt only", () => {
		expect(offerNames(SMALL, parcels([]), [], noSecret)).toEqual(["receipt"]);
	});

	test("never before three runs, never a per-run or secret field", () => {
		expect(
			offerNames(SMALL, parcels(["order"]).slice(0, 2), [], noSecret),
		).toEqual([]);
		expect(
			offerNames(SMALL, parcels(["order"]), ["receipt"], noSecret),
		).toEqual(["order"]);
		expect(
			offerNames(
				SMALL,
				parcels(["order"]),
				[],
				(field: WorkbenchField) => field.name === "order",
			),
		).toEqual(["receipt"]);
	});

	test("the 29-field form: Customer number typed over gives the offer, a Report title refined in place gives nothing", () => {
		const run = (customer: string, title: string): FieldValues => ({
			...defaultsOf(LARGE),
			portal_url: "https://portal.example/login",
			customer_number: customer,
			date_from: "2026-09-01",
			report_title: title,
		});
		const runs: OfferRun[] = [
			{
				values: run("K-204420", "Supplier portal check, September (3)"),
				replaced: ["customer_number"],
			},
			{
				values: run("K-204419", "Supplier portal check, September (2)"),
				replaced: ["customer_number"],
			},
			{
				values: run("K-204418", "Supplier portal check, September"),
				replaced: ["customer_number"],
			},
		];
		expect(offerNames(LARGE, runs, [], noSecret)).toEqual(["customer_number"]);
	});

	test("text longer than 40 characters is not offered", () => {
		const long = (n: number): OfferRun => ({
			values: { order: `${"x".repeat(40)}${n}`, quantity: "1", receipt: null },
			replaced: ["order"],
		});
		expect(
			offerNames(SMALL, [long(1), long(2), long(3)], [], noSecret),
		).toEqual([]);
	});
});

describe("the Per run popover (flpAfterRunRows)", () => {
	test("the after-run artboard: what each field goes back to", () => {
		const rows = afterRunRows(MEDIUM, PER_RUN, "invoice_file", null, words);
		expect(rows.map((row) => [row.label, row.on, row.back])).toEqual([
			["Invoice", true, { kind: "nextFile" }],
			["Supporting documents", true, { kind: "empty" }],
			["Vendor", false, { kind: "empty" }],
			["Invoice date", true, { kind: "empty" }],
			["Expected total", false, { kind: "value", text: "0" }],
			["Max pages", false, { kind: "value", text: "20" }],
			["Run OCR", false, { kind: "on" }],
			["Cost centers", false, { kind: "value", text: "4400, 4410" }],
			["Payment terms", false, { kind: "objectDefault" }],
		]);
	});

	test("under Alpenfracht AG, Vendor goes back to Alpenfracht AG and Max pages to 60", () => {
		const preset: Preset = {
			id: "p",
			name: "Alpenfracht AG",
			digit: 1,
			sets: { vendor_name: "Alpenfracht AG", max_pages: 60 },
			kinds: {},
			openDefault: false,
			createdAt: 0,
			updatedAt: 0,
			lastUsedAt: null,
		};
		const rows = afterRunRows(MEDIUM, [], null, preset, words);
		expect(rows[2].back).toEqual({ kind: "value", text: "Alpenfracht AG" });
		expect(rows[5].back).toEqual({ kind: "value", text: "60" });
		expect(rows[0].back).toEqual({ kind: "empty" });
	});
});

describe("after a run took its copy", () => {
	const baseline = baselineOf(MEDIUM, null);

	test("ten invoices: each run pairs its own file and date; the last one empties the field", () => {
		let values: FieldValues = {
			...baseline,
			invoice_file: INVOICES[0],
			vendor_name: "Nordwind Logistik GmbH",
		};
		let nextFiles: Readonly<Record<string, readonly FileSlot[]>> = {
			invoice_file: INVOICES.slice(1),
		};
		const taken: string[] = [];
		const messages: (StartMessage | null)[] = [];
		DATES.forEach((day, index) => {
			const copy = { ...values, invoice_date: day };
			const pairs = pairing(MEDIUM, copy, PER_RUN, noSecret, words);
			taken.push(pairs.join(", "));
			const applied = applyPerRun(MEDIUM, copy, nextFiles, PER_RUN, baseline);
			messages.push(
				startMessage({
					n: 15 + index,
					state: "started",
					files: 0,
					pairs,
					lastFile: applied.lastFile,
					leftAsIs: false,
				}),
			);
			values = applied.values;
			nextFiles = applied.nextFiles;
			expect(values.invoice_date).toBe("");
			expect(values.vendor_name).toBe("Nordwind Logistik GmbH");
		});
		expect(taken[0]).toBe("invoice-RE-2026-0918.pdf, 18 Sep 2026");
		expect(taken[3]).toBe("invoice-RE-2026-0921.pdf, 22 Sep 2026");
		expect(taken[9]).toBe("invoice-RE-2026-0927.pdf, 30 Sep 2026");
		expect(messages.map((message) => message?.lastFile)).toEqual([
			false,
			false,
			false,
			false,
			false,
			false,
			false,
			false,
			false,
			true,
		]);
		expect(values.invoice_file).toBeNull();
		expect(nextFiles).toEqual({});
	});

	test("the next file moves in and is named; a field that is not per run keeps its value", () => {
		const values = {
			...baseline,
			invoice_file: INVOICES[0],
			max_pages: "40",
			invoice_date: "2026-09-18",
		};
		const applied = applyPerRun(
			MEDIUM,
			values,
			{ invoice_file: INVOICES.slice(1, 3) },
			["invoice_date"],
			baseline,
		);
		expect(applied.values.invoice_file).toBe(INVOICES[1]);
		expect(applied.values.max_pages).toBe("40");
		expect(applied.values.invoice_date).toBe("");
		expect(applied.nextFiles).toEqual({ invoice_file: [INVOICES[2]] });
		expect([applied.nextFile, applied.lastFile]).toEqual([
			"invoice_file",
			false,
		]);
	});

	test("with nothing per run and no next files, nothing changes", () => {
		const values = { ...baseline, vendor_name: "N" };
		expect(applyPerRun(MEDIUM, values, {}, [], baseline)).toEqual({
			values,
			nextFiles: {},
			nextFile: null,
			lastFile: false,
		});
	});
});

describe("the pairing and the start message", () => {
	test("per-run values in field order, empty ones left out, secrets and hidden values ••••, long ones cut", () => {
		const copy = {
			...defaultsOf(MEDIUM),
			invoice_file: INVOICES[0],
			vendor_name: "x".repeat(50),
			invoice_date: "2026-09-18",
		};
		expect(
			pairing(
				MEDIUM,
				copy,
				["invoice_file", "supporting_documents", "vendor_name"],
				noSecret,
				words,
			),
		).toEqual(["invoice-RE-2026-0918.pdf", `${"x".repeat(39)}…`]);
		const secret = (field: WorkbenchField) => field.name === "vendor_name";
		expect(
			pairing(MEDIUM, copy, ["vendor_name", "invoice_date"], secret, words),
		).toEqual([SECRET_MASK, "18 Sep 2026"]);
		expect(
			pairing(
				MEDIUM,
				{ ...copy, invoice_date: HIDDEN_VALUE },
				["invoice_date"],
				noSecret,
				words,
			),
		).toEqual([SECRET_MASK]);
	});

	test("nothing per run and started: no message; queued, sending and left as is always say so", () => {
		const base: StartMessage = {
			n: 4,
			state: "started",
			files: 0,
			pairs: [],
			lastFile: false,
			leftAsIs: false,
		};
		expect(startMessage(base)).toBeNull();
		expect(startMessage({ ...base, state: "queued" })).toEqual({
			...base,
			state: "queued",
		});
		expect(startMessage({ ...base, state: "sending", files: 2 })).toEqual({
			...base,
			state: "sending",
			files: 2,
		});
		expect(startMessage({ ...base, n: 16, leftAsIs: true })).toEqual({
			...base,
			n: 16,
			leftAsIs: true,
		});
		expect(startMessage({ ...base, pairs: ["a.pdf"] })).toEqual({
			...base,
			pairs: ["a.pdf"],
		});
	});
});
