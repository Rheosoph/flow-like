import { describe, expect, test } from "bun:test";
import {
	FIELD_KEY_SEPARATOR,
	type FieldValues,
	type FileSlot,
	type FormSessionState,
	type HostCapabilities,
	type WorkbenchEventInput,
	type WorkbenchField,
} from "../contracts";
import { fieldsFromEvent } from "./fields";
import {
	type ProblemContext,
	blockedLabels,
	blockedNames,
	firstProblemKey,
	formPhase,
	isBlocked,
	missingCount,
	problemOf,
	railProblems,
	railSlots,
} from "./validate";
import { defaultsOf } from "./values";

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
			range: [1, 500],
		}),
		input("payment_terms", "Payment terms", "Struct", {
			schema: TERMS,
			optional: true,
			default_value: bytes({ currency: "EUR", net_days: 14 }),
		}),
	],
	TODAY,
);
const field = (name: string) =>
	MEDIUM.find((item) => item.name === name) as WorkbenchField;

const APP: ProblemContext = {
	host: {
		flowPathFiles: true,
		uploads: "temporary",
		inlineFileLimitBytes: null,
	},
	viewer: { decimalSign: "." },
};
const HOSTED: ProblemContext = {
	host: {
		flowPathFiles: false,
		uploads: "inline",
		inlineFileLimitBytes: 1_523_712,
	},
	viewer: { decimalSign: "," },
};

function slot(name: string, state: FileSlot["state"], size = 1000): FileSlot {
	return {
		id: name,
		name,
		size,
		type: null,
		state,
		progress: null,
		ref: null,
		error: null,
		sentAt: null,
		expiresAt: null,
	};
}

describe("one field's problem", () => {
	test("an empty required field; an optional one is fine", () => {
		expect(problemOf(field("vendor_name"), "", undefined, APP)).toEqual({
			code: "required",
		});
		expect(
			problemOf(field("vendor_name"), "Nordwind", undefined, APP),
		).toBeNull();
		expect(problemOf(field("expected_total"), "", undefined, APP)).toBeNull();
	});

	test("a date being typed is the reducer's to judge; a committed date is checked", () => {
		expect(problemOf(field("invoice_date"), "", "31", APP)).toBeNull();
		expect(problemOf(field("invoice_date"), "", " ", APP)).toEqual({
			code: "required",
		});
		expect(
			problemOf(field("invoice_date"), "2026-09-18", undefined, APP),
		).toBeNull();
	});

	test("numbers: the viewer's decimal sign, whole numbers, the pin's range", () => {
		expect(
			problemOf(field("expected_total"), "11769,10", undefined, APP),
		).toEqual({ code: "number" });
		expect(
			problemOf(field("expected_total"), "11769,10", undefined, HOSTED),
		).toBeNull();
		expect(problemOf(field("max_pages"), "3.7", undefined, APP)).toEqual({
			code: "noDecimals",
		});
		expect(problemOf(field("max_pages"), "600", undefined, APP)).toEqual({
			code: "range",
			min: 1,
			max: 500,
		});
	});

	test("files: a failed upload, a reminder, too large for an inline host; sending is fine at a press", () => {
		const invoice = field("invoice_file");
		expect(
			problemOf(invoice, slot("a.pdf", "sending"), undefined, APP),
		).toBeNull();
		expect(problemOf(invoice, slot("a.pdf", "failed"), undefined, APP)).toEqual(
			{ code: "fileFailed", fileName: "a.pdf" },
		);
		expect(
			problemOf(invoice, slot("a.pdf", "reminder"), undefined, APP),
		).toEqual({ code: "pickAgain", fileName: "a.pdf" });
		expect(
			problemOf(
				field("supporting_documents"),
				[slot("b.pdf", "reminder")],
				undefined,
				APP,
			),
		).toEqual({
			code: "pickAgain",
			fileName: "b.pdf",
		});
		const legacy = fieldsFromEvent(
			[input("receipt", "Receipt", "PathBuf")],
			TODAY,
		)[0];
		expect(
			problemOf(legacy, slot("big.jpg", "sent", 2_000_000), undefined, HOSTED),
		).toEqual({
			code: "fileTooLarge",
			fileName: "big.jpg",
			limitBytes: 1_523_712,
		});
		expect(problemOf(invoice, null, undefined, APP)).toEqual({
			code: "required",
		});
	});

	test("a FlowPath field this host cannot fill: a problem only when required", () => {
		expect(problemOf(field("invoice_file"), null, undefined, HOSTED)).toEqual({
			code: "fileNotHere",
		});
		expect(
			problemOf(field("supporting_documents"), [], undefined, HOSTED),
		).toBeNull();
	});

	test("an object is judged property by property", () => {
		expect(problemOf(field("payment_terms"), {}, undefined, APP)).toBeNull();
		expect(
			problemOf(field("payment_terms").props[1], "", undefined, APP),
		).toEqual({ code: "required" });
	});
});

describe("the rail's problems", () => {
	const idle: FieldValues = defaultsOf(MEDIUM);

	test("at the defaults: Invoice, Vendor and Invoice date need a look, in form order", () => {
		const problems = railProblems(MEDIUM, { values: idle, texts: {} }, APP);
		expect(problems).toEqual({
			invoice_file: { code: "required" },
			vendor_name: { code: "required" },
			invoice_date: { code: "required" },
		});
		expect(firstProblemKey(MEDIUM, problems)).toBe("invoice_file");
		expect(firstProblemKey(MEDIUM, {})).toBeNull();
	});

	test("FL_INVALID: the date filled, the invoice and the vendor missing", () => {
		const values = { ...idle, invoice_date: "2026-09-17" };
		expect(
			Object.keys(railProblems(MEDIUM, { values, texts: {} }, APP)),
		).toEqual(["invoice_file", "vendor_name"]);
	});

	test("an object's property problem is keyed by the property", () => {
		const values = {
			...idle,
			invoice_file: slot("a.pdf", "sent"),
			vendor_name: "N",
			invoice_date: "2026-09-17",
			payment_terms: { currency: "EUR", net_days: "" },
		};
		const problems = railProblems(MEDIUM, { values, texts: {} }, APP);
		expect(problems).toEqual({
			[`payment_terms${FIELD_KEY_SEPARATOR}net_days`]: { code: "required" },
		});
		expect(firstProblemKey(MEDIUM, problems)).toBe(
			`payment_terms${FIELD_KEY_SEPARATOR}net_days`,
		);
	});

	test("{n} fields to fill in: required stops still empty, blocked fields skipped", () => {
		expect(missingCount(MEDIUM, idle, [])).toBe(3);
		expect(missingCount(MEDIUM, idle, blockedNames(MEDIUM, HOSTED.host))).toBe(
			2,
		);
		expect(missingCount(MEDIUM, { ...idle, vendor_name: "N" }, [])).toBe(2);
		expect(
			missingCount(
				MEDIUM,
				{ ...idle, invoice_file: slot("a.pdf", "reminder") },
				[],
			),
		).toBe(3);
	});
});

describe("blocked fields (flpBlocked)", () => {
	test("FlowPath fields where the host can make no FlowPath; legacy fields are never blocked", () => {
		expect(blockedNames(MEDIUM, HOSTED.host)).toEqual([
			"invoice_file",
			"supporting_documents",
		]);
		expect(blockedNames(MEDIUM, APP.host)).toEqual([]);
		expect(blockedLabels(MEDIUM, HOSTED.host)).toEqual(["Invoice"]);
		const legacy = fieldsFromEvent(
			[input("receipt", "Receipt", "PathBuf")],
			TODAY,
		)[0];
		expect(isBlocked(legacy, HOSTED.host)).toBe(false);
	});
});

describe("the form's phase", () => {
	const state = (values: FieldValues, pressed: boolean, problems: number) =>
		({
			form: { fields: MEDIUM, host: {} as HostCapabilities },
			rail: {
				values,
				pressed,
				problems: problems ? { vendor_name: { code: "required" } } : {},
			},
		}) as unknown as Pick<FormSessionState, "form" | "rail">;
	const idle = defaultsOf(MEDIUM);

	test("files of the current entry sending first, then problems after a press, else idle", () => {
		expect(
			formPhase(
				state({ ...idle, invoice_file: slot("a.pdf", "sending") }, true, 1),
			),
		).toBe("uploading");
		expect(
			formPhase(
				state(
					{ ...idle, supporting_documents: [slot("a.pdf", "waiting")] },
					false,
					0,
				),
			),
		).toBe("uploading");
		expect(formPhase(state(idle, true, 1))).toBe("invalid");
		expect(formPhase(state(idle, false, 1))).toBe("idle");
		expect(
			formPhase(
				state({ ...idle, invoice_file: slot("a.pdf", "sent") }, true, 0),
			),
		).toBe("idle");
		expect(
			railSlots(MEDIUM, {
				...idle,
				invoice_file: slot("a.pdf", "sent"),
				supporting_documents: [slot("b", "sent")],
			}).map((item) => item.name),
		).toEqual(["a.pdf", "b"]);
	});
});
