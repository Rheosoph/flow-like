import { describe, expect, test } from "bun:test";
import type { ResultRow, ResultView, ToResultModel } from "../contracts";
import { leadRowOf, resultLabel, toResultModel } from "./result-view";

const _contract: ToResultModel = toResultModel;

/** SP/kit/DATA.js FL_RESULT: what the medium form's run returned. */
const FL_RESULT = {
	invoice_number: "RE-2026-0917",
	status: "needs_review",
	vendor: {
		name: "Nordwind Logistik GmbH",
		vat_id: "DE123456789",
		supplier_id: "S-10482",
		verified: true,
	},
	invoice_date: "2026-09-17",
	due_date: "2026-10-17",
	currency: "EUR",
	totals: {
		net: 10490.0,
		vat_rate: 0.19,
		vat: 1993.1,
		gross: 12483.1,
		expected_gross: 11769.1,
		difference: 714.0,
	},
	line_items: [
		{
			position: 1,
			description: "Pallet transport Hamburg–Munich",
			quantity: 12,
			unit_price: 410.0,
			net: 4920.0,
			cost_center: "4400",
			po_match: true,
		},
		{
			position: 2,
			description: "Cold-chain surcharge",
			quantity: 12,
			unit_price: 85.0,
			net: 1020.0,
			cost_center: "4400",
			po_match: true,
		},
		{
			position: 3,
			description: "Customs handling (flat fee)",
			quantity: 1,
			unit_price: 3950.0,
			net: 3950.0,
			cost_center: null,
			po_match: true,
		},
		{
			position: 4,
			description: "Express delivery",
			quantity: 2,
			unit_price: 300.0,
			net: 600.0,
			cost_center: "4410",
			po_match: false,
		},
	],
	findings: [
		{
			code: "po_mismatch",
			severity: "high",
			line: 4,
			message: "No matching position on PO-48213.",
		},
		{
			code: "terms_mismatch",
			severity: "medium",
			line: null,
			message:
				"Invoice says 30 days net; agreed terms are 14 days with 2 % discount.",
		},
		{
			code: "cost_center_missing",
			severity: "low",
			line: 3,
			message: "No cost center assigned.",
		},
	],
	checks: {
		arithmetic_ok: true,
		duplicate: false,
		pages_read: 14,
		ocr_used: true,
		confidence: 0.94,
	},
	duration_ms: 48211,
};

type CanvasRow = {
	k: string;
	v: string;
	mono?: boolean;
	tone?: ResultRow["tone"];
};

/** SP/kit/DATA.js FL_RESULT_VIEW: the same value as an end user should see it. */
const FL_RESULT_VIEW: {
	groups: { title: string; rows: CanvasRow[] }[];
	tables: { title: string; head: string[]; num: boolean[]; rows: string[][] }[];
} = {
	groups: [
		{
			title: "",
			rows: [
				{ k: "Invoice number", v: "RE-2026-0917", mono: true },
				{ k: "Status", v: "Needs review", tone: "warning" },
				{ k: "Invoice date", v: "17 Sep 2026" },
				{ k: "Due date", v: "17 Oct 2026" },
				{ k: "Currency", v: "EUR" },
			],
		},
		{
			title: "Vendor",
			rows: [
				{ k: "Name", v: "Nordwind Logistik GmbH" },
				{ k: "VAT ID", v: "DE123456789", mono: true },
				{ k: "Supplier ID", v: "S-10482", mono: true },
				{ k: "Verified", v: "Yes", tone: "good" },
			],
		},
		{
			title: "Totals",
			rows: [
				{ k: "Net", v: "10,490.00", mono: true },
				{ k: "VAT rate", v: "19 %", mono: true },
				{ k: "VAT", v: "1,993.10", mono: true },
				{ k: "Gross", v: "12,483.10", mono: true },
				{ k: "Expected gross", v: "11,769.10", mono: true },
				{ k: "Difference", v: "714.00", mono: true, tone: "warning" },
			],
		},
		{
			title: "Checks",
			rows: [
				{ k: "Arithmetic ok", v: "Yes", tone: "good" },
				{ k: "Duplicate", v: "No" },
				{ k: "Pages read", v: "14", mono: true },
				{ k: "OCR used", v: "Yes" },
				{ k: "Confidence", v: "0.94", mono: true },
			],
		},
	],
	tables: [
		{
			title: "Line items",
			head: [
				"Position",
				"Description",
				"Quantity",
				"Unit price",
				"Net",
				"Cost center",
				"PO match",
			],
			num: [true, false, true, true, true, false, false],
			rows: [
				[
					"1",
					"Pallet transport Hamburg–Munich",
					"12",
					"410.00",
					"4,920.00",
					"4400",
					"Yes",
				],
				["2", "Cold-chain surcharge", "12", "85.00", "1,020.00", "4400", "Yes"],
				[
					"3",
					"Customs handling (flat fee)",
					"1",
					"3,950.00",
					"3,950.00",
					"",
					"Yes",
				],
				["4", "Express delivery", "2", "300.00", "600.00", "4410", "No"],
			],
		},
		{
			title: "Findings",
			head: ["Code", "Severity", "Line", "Message"],
			num: [false, false, true, false],
			rows: [
				["po_mismatch", "High", "4", "No matching position on PO-48213."],
				[
					"terms_mismatch",
					"Medium",
					"",
					"Invoice says 30 days net; agreed terms are 14 days with 2 % discount.",
				],
				["cost_center_missing", "Low", "3", "No cost center assigned."],
			],
		},
	],
};

const canvasRow = (row: CanvasRow): ResultRow => ({
	label: row.k,
	text: row.v,
	mono: row.mono ?? false,
	tone: row.tone ?? null,
});

function structured(view: ResultView) {
	if (view.kind !== "structured")
		throw new Error(`expected a structured view, got ${view.kind}`);
	return view;
}

describe("toResultModel of FL_RESULT", () => {
	const view = structured(_contract(FL_RESULT).view);

	test("groups: the canvas labels, texts, mono and tones, in key order", () => {
		expect(view.groups.map((group) => group.title)).toEqual(
			FL_RESULT_VIEW.groups.map((group) => group.title),
		);
		const [top, ...rest] = view.groups;
		const [canvasTop, ...canvasRest] = FL_RESULT_VIEW.groups;
		expect(top?.rows.slice(0, 5)).toEqual(canvasTop?.rows.map(canvasRow));
		expect(rest.map((group) => group.rows)).toEqual(
			canvasRest.map((group) => group.rows.map(canvasRow)),
		);
	});

	test("the flow's own duration is kept as a readable row, never dropped", () => {
		expect(view.groups[0]?.rows[5]).toEqual({
			label: "Duration",
			text: "48 s",
			mono: true,
			tone: null,
		});
		expect(view.groups[0]?.rows).toHaveLength(6);
	});

	test("tables: the canvas heads, numeric columns and cells", () => {
		expect(
			view.tables.map((table) => ({
				title: table.title,
				head: table.head,
				num: table.numeric,
				rows: table.rows,
			})),
		).toEqual(FL_RESULT_VIEW.tables);
	});

	test("raw JSON for the toggle, JSON for Copy result", () => {
		const model = toResultModel(FL_RESULT);
		expect(JSON.parse(model.raw)).toEqual(FL_RESULT);
		expect(model.raw).toContain('\n  "invoice_number"');
		expect(model.copyText).toBe(model.raw);
	});
});

describe("toResultModel of FL_RESULT_SMALL", () => {
	test("three labelled rows", () => {
		const view = structured(
			toResultModel({
				return_number: "RET-20931",
				status: "accepted",
				label: "sent by e-mail",
			}).view,
		);
		expect(view.tables).toEqual([]);
		expect(view.groups).toEqual([
			{
				title: "",
				rows: [
					{ label: "Return number", text: "RET-20931", mono: true, tone: null },
					{ label: "Status", text: "Accepted", mono: false, tone: "good" },
					{ label: "Label", text: "Sent by e-mail", mono: false, tone: null },
				],
			},
		]);
	});
});

describe("toResultModel of scalars", () => {
	test("false, 0, empty text and null are results with their own words", () => {
		expect(toResultModel(false).view).toEqual({
			kind: "value",
			text: "No",
			mono: false,
			tone: null,
		});
		expect(toResultModel(true).view).toEqual({
			kind: "value",
			text: "Yes",
			mono: false,
			tone: null,
		});
		expect(toResultModel(0).view).toEqual({
			kind: "value",
			text: "0",
			mono: true,
			tone: null,
		});
		expect(toResultModel("").view).toEqual({
			kind: "value",
			text: "Empty text",
			mono: false,
			tone: null,
		});
		expect(toResultModel(null).view).toEqual({
			kind: "value",
			text: "None",
			mono: false,
			tone: null,
		});
		expect(toResultModel(undefined).raw).toBe("null");
		expect(toResultModel(false).raw).toBe("false");
		expect(toResultModel(0).copyText).toBe("0");
	});

	test("translated words come from the caller", () => {
		const words = {
			yes: "Ja",
			no: "Nein",
			none: "Nichts",
			emptyText: "Leerer Text",
		};
		expect(toResultModel(false, { words }).view).toMatchObject({
			text: "Nein",
		});
		expect(toResultModel(null, { words }).view).toMatchObject({
			text: "Nichts",
		});
		expect(toResultModel("", { words }).view).toMatchObject({
			text: "Leerer Text",
		});
	});

	test("numbers, short strings, ids and dates", () => {
		expect(toResultModel(12_483.1).view).toMatchObject({
			text: "12,483.1",
			mono: true,
		});
		expect(toResultModel(2026).view).toMatchObject({ text: "2026" });
		expect(toResultModel("RET-20931").view).toMatchObject({
			text: "RET-20931",
			mono: true,
		});
		expect(toResultModel("done").view).toMatchObject({
			text: "Done",
			mono: false,
		});
		expect(toResultModel("2026-09-17").view).toMatchObject({
			text: "17 Sep 2026",
		});
		expect(toResultModel("2026-02-30").view).toMatchObject({
			text: "2026-02-30",
		});
		expect(toResultModel("done").copyText).toBe("done");
		expect(toResultModel("done").raw).toBe('"done"');
	});
});

describe("toResultModel of text", () => {
	test("markdown reads as markdown", () => {
		const answer = "## Invoice RE-2026-0917 extracted\n\n**Vendor:** Nordwind";
		const model = toResultModel(answer);
		expect(model.view).toEqual({ kind: "text", text: answer, markdown: true });
		expect(model.copyText).toBe(answer);
	});

	test("long or multi-line plain text reads as a paragraph", () => {
		const long =
			"The invoice was checked against the purchase order and every line matched. ".repeat(
				3,
			);
		expect(toResultModel(long).view).toEqual({
			kind: "text",
			text: long,
			markdown: false,
		});
		expect(toResultModel("first line\nsecond line").view).toMatchObject({
			kind: "text",
			markdown: false,
		});
	});
});

describe("toResultModel of lists and nesting", () => {
	test("a list of records is one table; gaps stay empty, numbers stay numeric", () => {
		const view = structured(
			toResultModel([
				{ sku: "A-1", qty: 2 },
				{ sku: "B-2", qty: null, note: "back order" },
			]).view,
		);
		expect(view.groups).toEqual([]);
		expect(view.tables).toEqual([
			{
				title: "",
				head: ["SKU", "Qty", "Note"],
				numeric: [false, true, false],
				rows: [
					["A-1", "2", ""],
					["B-2", "", "Back order"],
				],
			},
		]);
	});

	test("any other list reads as numbered rows", () => {
		const view = structured(toResultModel(["first.pdf", 2, null]).view);
		expect(view.groups[0]?.rows).toEqual([
			{ label: "1", text: "first.pdf", mono: false, tone: null },
			{ label: "2", text: "2", mono: true, tone: null },
			{ label: "3", text: "—", mono: false, tone: null },
		]);
		expect(toResultModel([]).view).toMatchObject({
			kind: "value",
			text: "None",
		});
	});

	test("nested objects become titled groups and tables, lists of values one row", () => {
		const view = structured(
			toResultModel({
				order: {
					id: 4471,
					address: { city: "München", zip: "80331" },
					cost_centers: ["4400", "4410"],
					lines: [{ sku: "A-1" }],
					tags: [],
					meta: {},
				},
			}).view,
		);
		expect(view.groups).toEqual([
			{
				title: "Order",
				rows: [
					{ label: "ID", text: "4471", mono: true, tone: null },
					{ label: "Cost centers", text: "4400, 4410", mono: true, tone: null },
					{ label: "Tags", text: "—", mono: false, tone: null },
					{ label: "Meta", text: "—", mono: false, tone: null },
				],
			},
			{
				title: "Order · Address",
				rows: [
					{ label: "City", text: "München", mono: false, tone: null },
					{ label: "Zip", text: "80331", mono: true, tone: null },
				],
			},
		]);
		expect(view.tables).toEqual([
			{
				title: "Order · Lines",
				head: ["SKU"],
				numeric: [false],
				rows: [["A-1"]],
			},
		]);
		expect(toResultModel({}).view).toMatchObject({
			kind: "value",
			text: "None",
		});
	});

	test("mixed lists and deep nesting fall back to compact JSON in mono", () => {
		const deep = { a: { b: { c: { d: { e: { f: { g: 1 } } } } } } };
		const view = structured(toResultModel({ mixed: [1, { x: 1 }], deep }).view);
		expect(view.groups[0]?.rows[0]).toEqual({
			label: "Mixed",
			text: '[1,{"x":1}]',
			mono: true,
			tone: null,
		});
		const deepest = view.groups.at(-1);
		expect(deepest?.title).toBe("Deep · A · B · C · D · E");
		expect(deepest?.rows[0]).toEqual({
			label: "F",
			text: '{"g":1}',
			mono: true,
			tone: null,
		});
	});
});

describe("formats per key", () => {
	const row = (key: string, value: unknown, locale?: string) =>
		structured(toResultModel({ [key]: value }, { locale }).view).groups[0]
			?.rows[0];

	test("money, rates, percentages and durations", () => {
		expect(row("amount_due", 1200)).toMatchObject({ text: "1,200.00" });
		expect(row("discount_rate", 0.025)).toMatchObject({ text: "2.5 %" });
		expect(row("exchange_rate", 1.08)).toMatchObject({ text: "1.08" });
		expect(row("completion_percent", 87.5)).toMatchObject({ text: "87.5 %" });
		expect(row("timeout_seconds", 90)).toMatchObject({
			label: "Timeout",
			text: "1 min 30 s",
		});
		expect(row("count", 1_250_000)).toMatchObject({ text: "1,250,000" });
		expect(row("customer_id", 1_250_000)).toMatchObject({
			text: "1250000",
			mono: true,
		});
	});

	test("the viewer's locale and date formatter", () => {
		expect(row("net", 10490, "de-DE")).toMatchObject({ text: "10.490,00" });
		const view = structured(
			toResultModel(
				{ due_date: "2026-10-17" },
				{ date: (iso) => `date:${iso}` },
			).view,
		);
		expect(view.groups[0]?.rows[0]?.text).toBe("date:2026-10-17");
	});

	test("an instant reads in the viewer's own time", () => {
		const at = new Date("2026-09-17T14:02:00Z");
		const pad = (value: number) => String(value).padStart(2, "0");
		const local = `${at.getFullYear()}-${pad(at.getMonth() + 1)}-${pad(at.getDate())}`;
		const view = structured(
			toResultModel(
				{ created_at: "2026-09-17T14:02:00Z" },
				{ date: (iso) => iso },
			).view,
		);
		expect(view.groups[0]?.rows[0]?.text).toBe(
			`${local} ${pad(at.getHours())}:${pad(at.getMinutes())}`,
		);
	});

	test("tones of status, severity, checks and differences", () => {
		expect(row("status", "Failed")).toMatchObject({
			text: "Failed",
			tone: "critical",
		});
		expect(row("state", "in progress")).toMatchObject({
			text: "In progress",
			tone: "info",
		});
		expect(row("severity", "SEV-1")).toMatchObject({
			text: "SEV-1",
			tone: "critical",
			mono: true,
		});
		expect(row("verified", false)).toMatchObject({
			text: "No",
			tone: "warning",
		});
		expect(row("is_duplicate", true)).toMatchObject({
			text: "Yes",
			tone: "warning",
		});
		expect(row("difference", 0)).toMatchObject({ text: "0.00", tone: null });
		expect(row("label", "needs_review")).toMatchObject({
			text: "Needs review",
			tone: null,
		});
	});

	test("identifiers, links and deliberate casing stay as written", () => {
		expect(row("code", "po_mismatch")).toMatchObject({
			text: "po_mismatch",
			mono: true,
		});
		expect(row("email", "max@example.com")).toMatchObject({
			text: "max@example.com",
			mono: true,
		});
		expect(row("website", "https://example.com/a")).toMatchObject({
			text: "https://example.com/a",
		});
		expect(row("name", "iPhone")).toMatchObject({
			text: "iPhone",
			mono: false,
		});
	});
});

describe("resultLabel", () => {
	test("keys read as labels", () => {
		expect(resultLabel("invoice_number")).toBe("Invoice number");
		expect(resultLabel("vat_id")).toBe("VAT ID");
		expect(resultLabel("invoiceNumber")).toBe("Invoice number");
		expect(resultLabel("HTTPStatus")).toBe("HTTP status");
		expect(resultLabel("po-match")).toBe("PO match");
		expect(resultLabel("duration_ms")).toBe("Duration");
		expect(resultLabel("ms")).toBe("Ms");
		expect(resultLabel("Invoice Number")).toBe("Invoice Number");
		expect(resultLabel("größe_kg")).toBe("Größe kg");
	});
});

describe("leadRowOf", () => {
	test("the first text or number at the top level", () => {
		expect(leadRowOf({ ok: true, return_number: "RET-20931" })).toEqual({
			label: "Return number",
			text: "RET-20931",
			mono: true,
			tone: null,
		});
		expect(leadRowOf({ nested: { a: 1 }, total: 12 })).toMatchObject({
			label: "Total",
			text: "12.00",
		});
		expect(leadRowOf({ ok: true })).toBeNull();
		expect(leadRowOf("text")).toBeNull();
	});
});
