import { beforeAll, describe, expect, test } from "bun:test";
import i18next, { type TFunction } from "i18next";
import type {
	FieldProblem,
	FieldProblemCode,
	WorkbenchField,
} from "../contracts";
import { dateLocaleOf } from "../model/dates";
import { lowerLabel, problemText } from "./problem-text";

let t: TFunction<"interfaces">;

beforeAll(async () => {
	const instance = i18next.createInstance();
	await instance.init({
		lng: "en",
		resources: {},
		interpolation: { escapeValue: false },
	});
	t = instance.getFixedT(
		"en",
		"interfaces",
	) as unknown as TFunction<"interfaces">;
});

const field = (patch: Partial<WorkbenchField> = {}): WorkbenchField => ({
	key: "vendor_name",
	name: "vendor_name",
	label: "Vendor",
	help: null,
	kind: "text",
	dataType: "String",
	valueType: "Normal",
	required: true,
	sensitive: false,
	defaultOmitted: false,
	defaultValue: "",
	hasDefault: false,
	options: null,
	range: null,
	step: null,
	integer: false,
	fileMode: null,
	itemKind: null,
	dateFormat: null,
	props: [],
	short: false,
	index: 0,
	...patch,
});

const viewer = (lang: string, decimalSign: "." | "," = ".") => ({
	dateLocale: dateLocaleOf(lang),
	decimalSign,
});

function say(
	problem: FieldProblem,
	patch: Partial<WorkbenchField> = {},
	lang = "en-GB",
	decimalSign: "." | "," = ".",
) {
	return problemText({
		t,
		field: field(patch),
		viewer: viewer(lang, decimalSign),
		problem,
	});
}

describe("labels in a sentence", () => {
	test("a capitalised word is lowered, acronyms and phrases with capitals stay", () => {
		expect(lowerLabel("Invoice")).toBe("invoice");
		expect(lowerLabel("Customer number")).toBe("customer number");
		expect(lowerLabel("URL")).toBe("URL");
		expect(lowerLabel("API key")).toBe("API key");
		expect(lowerLabel("e-mail")).toBe("e-mail");
	});
});

describe("a missing value says what to do (canvas wording)", () => {
	const cases: [WorkbenchField["kind"], string][] = [
		["text", "Enter the vendor."],
		["number", "Enter the vendor."],
		["json", "Enter the vendor."],
		["file", "Add the vendor file."],
		["files", "Add at least one file."],
		["chips", "Add the vendor."],
		["date", "Choose a date."],
		["choice", "Choose one."],
	];
	test.each(cases)("%s", (kind, text) => {
		expect(say({ code: "required" }, { kind })).toBe(text);
	});

	test("a withheld default is never shown and the sentence says so", () => {
		expect(say({ code: "required" }, { defaultOmitted: true })).toBe(
			"Enter Vendor. The app's own value is not shown here.",
		);
	});
});

describe("a missing file does not say file twice", () => {
	const cases: [string, string][] = [
		["Invoice", "Add the invoice file."],
		["Invoice file", "Add the invoice file."],
		["File", "Add the file."],
		["File to scan", "Add the file to scan."],
		["PDF files", "Add the PDF files."],
		["Profile", "Add the profile file."],
	];
	test.each(cases)("%s", (label, text) => {
		expect(say({ code: "required" }, { kind: "file", label })).toBe(text);
	});
});

describe("numbers", () => {
	test("digits only, whole numbers, ranges in the viewer's decimal sign", () => {
		expect(say({ code: "integer" })).toBe("Enter a whole number, digits only.");
		expect(say({ code: "number" })).toBe("Enter a number, digits only.");
		expect(say({ code: "noDecimals" })).toBe(
			"Enter a whole number without decimals.",
		);
		expect(say({ code: "range", min: 0.5, max: 99 })).toBe(
			"Enter a number from 0.5 to 99.",
		);
		expect(say({ code: "range", min: 0.5, max: 99 }, {}, "de-DE", ",")).toBe(
			"Enter a number from 0,5 to 99.",
		);
	});
});

describe("dates follow the viewer's order and separator (spec M4)", () => {
	test("day first, month first, year first", () => {
		expect(say({ code: "date" }, {}, "en-GB")).toBe(
			"Enter a date such as 21/9 or 21/9/2026.",
		);
		expect(say({ code: "date" }, {}, "de-DE")).toBe(
			"Enter a date such as 21.9 or 21.9.2026.",
		);
		expect(say({ code: "date" }, {}, "en-US")).toBe(
			"Enter a date such as 9/21 or 9/21/2026.",
		);
		expect(say({ code: "date" }, {}, "ja-JP")).toBe(
			"Enter a date such as 9/21 or 2026/9/21.",
		);
	});
});

describe("lists, objects and JSON", () => {
	test("each kind names what is wrong", () => {
		expect(say({ code: "json" })).toBe("Enter valid JSON.");
		expect(say({ code: "object" })).toBe("Enter a JSON object.");
		expect(say({ code: "array" })).toBe("Enter a JSON list.");
		expect(say({ code: "option" })).toBe("Choose one of the options.");
		expect(say({ code: "unique" }, { kind: "chips" })).toBe(
			"Each entry can appear once.",
		);
		expect(say({ code: "unique" }, { kind: "pairs" })).toBe(
			"Each name can appear once.",
		);
		expect(say({ code: "items" }, { kind: "chips" })).toBe(
			"One of the entries is not valid.",
		);
		expect(say({ code: "items" }, { kind: "pairs" })).toBe(
			"Every row needs a name and a value that fits.",
		);
	});
});

describe("files (spec F, M3, S4)", () => {
	test("sending, not sent, too large, not here, enter again, unsupported", () => {
		const name = "invoice-RE-2026-0919.pdf";
		expect(say({ code: "fileSending", fileName: name })).toBe(
			`${name} is still sending.`,
		);
		expect(say({ code: "fileFailed", fileName: name })).toBe(
			`${name} was not sent. Try again or remove it.`,
		);
		expect(
			say({
				code: "fileTooLarge",
				fileName: name,
				limitBytes: 3.5 * 1024 * 1024,
			}),
		).toBe(`${name} is larger than 3.5 MB, the most this page can send.`);
		expect(say({ code: "fileNotHere" }, { label: "Invoice" })).toBe(
			"Invoice can't be sent from this page.",
		);
		expect(say({ code: "enterAgain" })).toBe("Enter Vendor again.");
		expect(say({ code: "unsupported" })).toBe(
			"Vendor can't be filled in on this page.",
		);
	});

	test("a reminder: pick it again, or remove it when optional", () => {
		const problem: FieldProblem = {
			code: "pickAgain",
			fileName: "invoice-RE-2026-0902.pdf",
		};
		expect(say(problem, { required: true })).toBe(
			"Pick invoice-RE-2026-0902.pdf again.",
		);
		expect(say(problem, { required: false })).toBe(
			"Pick invoice-RE-2026-0902.pdf again, or remove it.",
		);
	});
});

describe("every problem code has a sentence", () => {
	const codes: FieldProblemCode[] = [
		"required",
		"integer",
		"number",
		"noDecimals",
		"range",
		"date",
		"json",
		"object",
		"array",
		"unique",
		"items",
		"option",
		"fileSending",
		"fileFailed",
		"fileTooLarge",
		"fileNotHere",
		"pickAgain",
		"enterAgain",
		"unsupported",
	];
	test.each(codes)("%s", (code) => {
		const text = say({
			code,
			fileName: "a.pdf",
			min: 1,
			max: 2,
			limitBytes: 1024,
		});
		expect(text.length).toBeGreaterThan(5);
		expect(text).not.toContain("{{");
		expect(text.startsWith("workbench")).toBe(false);
	});
});
