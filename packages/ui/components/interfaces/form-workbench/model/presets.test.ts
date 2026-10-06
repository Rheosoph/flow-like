import { describe, expect, test } from "bun:test";
import type {
	FieldValues,
	FileSlot,
	Preset,
	ShortWords,
	WorkbenchEventInput,
	WorkbenchField,
} from "../contracts";
import { fieldsFromEvent } from "./fields";
import {
	nextDigit,
	presetChanges,
	presetEdits,
	presetMisfit,
	presetName,
	presetSummaryParts,
	presetTicks,
	toPresetSets,
} from "./presets";
import { defaultsOf } from "./values";

const TODAY = "2026-10-05";
const FLOWPATH =
	'{"title":"FlowPath","type":"object","properties":{"path":{"type":"string"},"store_ref":{"type":"string"}},"required":["path","store_ref"]}';
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
		input("access_token", "Access token", "String", { optional: true }),
	],
	TODAY,
);
const LARGE = fieldsFromEvent(
	[
		input("portal_url", "Portal URL", "String"),
		input("customer_number", "Customer number", "String"),
		input("report_title", "Report title", "String"),
	],
	TODAY,
);

function preset(
	id: string,
	name: string,
	digit: number,
	sets: Preset["sets"],
): Preset {
	return {
		id,
		name,
		digit,
		sets,
		kinds: {},
		openDefault: false,
		createdAt: 0,
		updatedAt: 0,
		lastUsedAt: null,
	};
}

/** FLP_PRESETS.medium. */
const ALPENFRACHT = preset("p-alpenfracht", "Alpenfracht AG", 1, {
	vendor_name: "Alpenfracht AG",
	max_pages: 60,
});
const NORDWIND = preset("p-nordwind", "Nordwind Logistik GmbH", 2, {
	vendor_name: "Nordwind Logistik GmbH",
});

const IDLE = defaultsOf(MEDIUM);
const noSecret = () => false;
const isSecret = (field: WorkbenchField) => field.name === "access_token";
const words: ShortWords = {
	none: "none",
	empty: "empty",
	on: "On",
	off: "Off",
	files: (count) => `${count} files`,
	entries: (count) => `${count} entries`,
	date: (iso) => iso,
};

function slot(name: string): FileSlot {
	return {
		id: name,
		name,
		size: 1,
		type: null,
		state: "sent",
		progress: null,
		ref: null,
		error: null,
		sentAt: null,
		expiresAt: null,
	};
}

describe("applying a preset (flpPresetChanges)", () => {
	test("fields still at their starting value move; ⌘P then 1 on the defaults gives Alpenfracht AG and 60", () => {
		expect(presetChanges(MEDIUM, IDLE, null, ALPENFRACHT)).toEqual([
			{ name: "vendor_name", value: "Alpenfracht AG" },
			{ name: "max_pages", value: "60" },
		]);
	});

	test("an edited field keeps its value unless the new preset sets it; files never change", () => {
		const edited: FieldValues = {
			...IDLE,
			max_pages: "40",
			invoice_file: slot("a.pdf"),
			expected_total: "12",
		};
		expect(presetChanges(MEDIUM, edited, null, NORDWIND)).toEqual([
			{ name: "vendor_name", value: "Nordwind Logistik GmbH" },
		]);
		expect(presetChanges(MEDIUM, edited, null, ALPENFRACHT)).toEqual([
			{ name: "vendor_name", value: "Alpenfracht AG" },
			{ name: "max_pages", value: "60" },
		]);
	});

	test("switching presets: what the old one set goes back to the default where the new one sets nothing", () => {
		const underAlpenfracht = {
			...IDLE,
			vendor_name: "Alpenfracht AG",
			max_pages: "60",
		};
		expect(
			presetChanges(MEDIUM, underAlpenfracht, ALPENFRACHT, NORDWIND),
		).toEqual([
			{ name: "vendor_name", value: "Nordwind Logistik GmbH" },
			{ name: "max_pages", value: "20" },
		]);
		expect(presetChanges(MEDIUM, underAlpenfracht, ALPENFRACHT, null)).toEqual([
			{ name: "vendor_name", value: "" },
			{ name: "max_pages", value: "20" },
		]);
	});
});

describe("edits, misfits and the menu row", () => {
	test("how many inputs the preset sets now differ from it", () => {
		expect(
			presetEdits(
				MEDIUM,
				{ ...IDLE, vendor_name: "Nordwind Logistik GmbH" },
				NORDWIND,
			),
		).toBe(0);
		expect(
			presetEdits(MEDIUM, { ...IDLE, vendor_name: "Nordwind" }, NORDWIND),
		).toBe(1);
		expect(presetEdits(MEDIUM, IDLE, null)).toBe(0);
	});

	test("a saved input whose field is gone or no longer fits", () => {
		expect(presetMisfit(MEDIUM, ALPENFRACHT)).toBe(0);
		expect(
			presetMisfit(
				MEDIUM,
				preset("p", "Old", 3, { gone: "x", max_pages: 60.5, vendor_name: "V" }),
			),
		).toBe(2);
	});

	test("the second line: up to three parts in field order, then + n more", () => {
		expect(presetSummaryParts(ALPENFRACHT, MEDIUM, words)).toEqual({
			parts: [
				{ label: "Vendor", text: "Alpenfracht AG" },
				{ label: "Max pages", text: "60" },
			],
			more: 0,
		});
		const big = preset("p", "Big", 3, {
			vendor_name: "V",
			expected_total: 1,
			max_pages: 2,
			run_ocr: false,
			cost_centers: ["1"],
		});
		expect(presetSummaryParts(big, MEDIUM, words)).toEqual({
			parts: [
				{ label: "Vendor", text: "V" },
				{ label: "Expected total", text: "1" },
				{ label: "Max pages", text: "2" },
			],
			more: 2,
		});
	});
});

describe("digits (flpNextDigit)", () => {
	test("the lowest free digit 1–9, kept until deleted; a tenth preset has none", () => {
		expect(nextDigit([])).toBe(1);
		expect(nextDigit([ALPENFRACHT, NORDWIND])).toBe(3);
		expect(nextDigit([NORDWIND])).toBe(1);
		expect(
			nextDigit([1, 2, 3, 4, 5, 6, 7, 8, 9].map((digit) => ({ digit }))),
		).toBe(0);
	});
});

describe("the save dialog", () => {
	test("the proposed name: the first ticked short text that is no web address (flpPresetName)", () => {
		const series = { ...IDLE, vendor_name: "Nordwind Logistik GmbH" };
		expect(presetName(MEDIUM, series, ["vendor_name"], [], noSecret)).toEqual({
			kind: "value",
			text: "Nordwind Logistik GmbH",
		});
		const large = {
			portal_url: "https://portal.nordwind-logistik.example/login",
			customer_number: "K-204418",
			report_title: "Supplier portal check, September",
		};
		expect(
			presetName(LARGE, large, ["portal_url", "customer_number"], [], noSecret),
		).toEqual({ kind: "value", text: "K-204418" });
		expect(
			presetName(
				LARGE,
				{ ...large, portal_url: "www.example.com" },
				["portal_url"],
				[NORDWIND],
				noSecret,
			),
		).toEqual({
			kind: "numbered",
			n: 2,
		});
		expect(
			presetName(
				MEDIUM,
				{ ...IDLE, access_token: "abc" },
				["access_token"],
				[],
				isSecret,
			),
		).toEqual({ kind: "numbered", n: 1 });
	});

	test("first ticks: not files, not per run, not secret; set by the active preset or off its default (flpPresetTicks)", () => {
		const series = {
			...IDLE,
			invoice_file: slot("invoice-RE-2026-0922.pdf"),
			vendor_name: "Nordwind Logistik GmbH",
			access_token: "x",
		};
		expect(
			presetTicks(
				MEDIUM,
				series,
				["invoice_file", "supporting_documents", "invoice_date"],
				null,
				isSecret,
			),
		).toEqual(["vendor_name"]);
		expect(presetTicks(MEDIUM, IDLE, [], ALPENFRACHT, noSecret)).toEqual([
			"vendor_name",
			"max_pages",
		]);
		expect(
			presetTicks(
				MEDIUM,
				{ ...IDLE, invoice_date: "2026-09-18" },
				["invoice_date"],
				null,
				noSecret,
			),
		).toEqual([]);
	});

	test("what a preset saves: the ticked values, never files or secrets, with their kinds", () => {
		const values = {
			...IDLE,
			vendor_name: "Nordwind Logistik GmbH",
			max_pages: "60",
			invoice_file: slot("a.pdf"),
			access_token: "x",
		};
		expect(
			toPresetSets(
				MEDIUM,
				values,
				["vendor_name", "max_pages", "invoice_file", "access_token"],
				isSecret,
			),
		).toEqual({
			sets: { vendor_name: "Nordwind Logistik GmbH", max_pages: "60" },
			kinds: { vendor_name: "text", max_pages: "number" },
		});
	});
});
