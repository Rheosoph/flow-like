import { describe, expect, test } from "bun:test";
import { SECRET_MASK, type WorkbenchField } from "../contracts";
import { fixture } from "../testing/fixtures";
import {
	INPUT_TEXT_MAX,
	longTextOf,
	momentOf,
	secretCheckOf,
	valueTextOf,
} from "./stage-text";
import { shortWords } from "./test-words";

const words = shortWords();
const never = () => false;
const context = { words, isSecret: never };

const fields = fixture("done").form.fields;
const fieldNamed = (name: string) =>
	fields.find((field) => field.name === name) as WorkbenchField;

const doneValues = fixture("done").runs[0]?.copy.values ?? {};

describe("longTextOf", () => {
	test("files read by name, one per line", () => {
		const docs = fieldNamed("supporting_documents");
		expect(longTextOf(docs, doneValues[docs.name], context)).toBe(
			"Lieferschein LS-77120.pdf\nPO-48213.pdf",
		);
		const invoice = fieldNamed("invoice_file");
		expect(longTextOf(invoice, doneValues[invoice.name], context)).toBe(
			"invoice-RE-2026-0917.pdf",
		);
	});

	test("dates in the viewer's reading, switches On and Off, lists joined", () => {
		expect(longTextOf(fieldNamed("invoice_date"), "2026-09-17", context)).toBe(
			"17 Sep 2026",
		);
		expect(longTextOf(fieldNamed("run_ocr"), true, context)).toBe("On");
		expect(longTextOf(fieldNamed("run_ocr"), false, context)).toBe("Off");
		expect(
			longTextOf(fieldNamed("cost_centers"), ["4400", "4410"], context),
		).toBe("4400, 4410");
	});

	test("an object reads its properties with their labels, never broken inside one", () => {
		const terms = fieldNamed("payment_terms");
		const text = longTextOf(terms, doneValues[terms.name], context);
		expect(text.split(" · ").length).toBe(terms.props.length);
		for (const part of text.split(" · ")) expect(part).not.toContain(" ");
		expect(text).toContain("Net days 14");
	});

	test("an object's property kept off this device reads the mask", () => {
		const terms = fieldNamed("payment_terms");
		const [first, second] = terms.props;
		const text = longTextOf(
			terms,
			{ [first.name]: "EUR", [second.name]: { $hidden: true } },
			context,
		);
		const parts = text.split(" · ");
		expect(parts[1]?.replaceAll(" ", " ")).toBe(
			`${second.label} ${SECRET_MASK}`,
		);
	});

	test("empty values read as nothing", () => {
		expect(longTextOf(fieldNamed("vendor_name"), "", context)).toBe("");
		expect(longTextOf(fieldNamed("invoice_date"), "", context)).toBe("");
		expect(longTextOf(fieldNamed("cost_centers"), [], context)).toBe("");
		expect(longTextOf(fieldNamed("invoice_file"), null, context)).toBe("");
	});

	test("a long text is cut", () => {
		const long = "x".repeat(INPUT_TEXT_MAX + 50);
		const text = longTextOf(fieldNamed("vendor_name"), long, context);
		expect(text.length).toBe(INPUT_TEXT_MAX);
		expect(text.endsWith("…")).toBe(true);
	});
});

describe("valueTextOf", () => {
	const vendor = fieldNamed("vendor_name");

	test("a value, an empty value, and a value kept off this device", () => {
		expect(valueTextOf(vendor, "Nordwind", context)).toEqual({
			kind: "value",
			text: "Nordwind",
		});
		expect(valueTextOf(vendor, "", context)).toEqual({ kind: "empty" });
		expect(valueTextOf(vendor, { $hidden: true }, context)).toEqual({
			kind: "hidden",
		});
	});

	test("secrets read the mask, never the value", () => {
		const secret = { words, isSecret: secretCheckOf([]) };
		const key = {
			...vendor,
			name: "api_token",
			label: "API token",
			key: "api_token",
		};
		expect(valueTextOf(key, "hunter2", secret)).toEqual({
			kind: "value",
			text: SECRET_MASK,
		});
		expect(valueTextOf(vendor, "sk-live_abcdefghijklmnop1234", secret)).toEqual(
			{
				kind: "value",
				text: SECRET_MASK,
			},
		);
		expect(valueTextOf(vendor, "Nordwind", secret)).toMatchObject({
			text: "Nordwind",
		});
	});

	test("a field the person told the form not to save is masked too", () => {
		const secret = { words, isSecret: secretCheckOf([vendor.key]) };
		expect(valueTextOf(vendor, "Nordwind", secret)).toEqual({
			kind: "value",
			text: SECRET_MASK,
		});
	});
});

describe("momentOf", () => {
	const formWords = { ...words, today: "Today", yesterday: "Yesterday" };
	const now = new Date(2026, 9, 5, 14, 30).getTime();

	test("today reads as the day word and the time of day", () => {
		const at = new Date(2026, 9, 5, 14, 2).getTime();
		expect(momentOf(at, now, "en-GB", formWords)).toEqual({
			day: "Today",
			time: "14:02",
		});
	});

	test("yesterday and older days keep their day text", () => {
		const yesterday = new Date(2026, 9, 4, 9, 47).getTime();
		expect(momentOf(yesterday, now, "en-GB", formWords)).toEqual({
			day: "Yesterday",
			time: "09:47",
		});
		const older = new Date(2026, 8, 28, 16, 5).getTime();
		const moment = momentOf(older, now, "en-GB", formWords);
		expect(moment.day).toContain("28");
		expect(moment.time).toBe("16:05");
	});
});
