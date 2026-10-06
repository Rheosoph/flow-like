import { describe, expect, test } from "bun:test";
import {
	FIELD_KEY_SEPARATOR,
	type FieldValues,
	type FileSlot,
	type FormModel,
	type FormPrefs,
	type FormSessionState,
	type Preset,
	type RailState,
	type RunEntry,
	type WorkbenchEventInput,
	type WorkbenchField,
} from "../contracts";
import { fieldsFromEvent } from "./fields";
import {
	activePresetOf,
	comparedRunOf,
	fieldMarkers,
	perRunNamesOf,
} from "./markers";
import { defaultsOf } from "./values";

const TODAY = "2026-10-05";
const FLOWPATH =
	'{"title":"FlowPath","type":"object","properties":{"path":{"type":"string"},"store_ref":{"type":"string"}},"required":["path","store_ref"]}';
const TERMS = JSON.stringify({
	type: "object",
	properties: {
		currency: { type: "string", enum: ["EUR", "USD"] },
		net_days: { type: "integer" },
		note: { type: "string" },
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
const field = (name: string) =>
	MEDIUM.find((item) => item.name === name) as WorkbenchField;
const prop = (name: string) =>
	field("payment_terms").props.find(
		(item) => item.name === name,
	) as WorkbenchField;

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

const FILLED: FieldValues = {
	...defaultsOf(MEDIUM),
	invoice_file: slot("invoice-RE-2026-0917.pdf"),
	supporting_documents: [slot("a.pdf"), slot("b.pdf")],
	vendor_name: "Nordwind Logistik GmbH",
	invoice_date: "2026-09-17",
	expected_total: "11769.10",
};

const PREFS: FormPrefs = {
	version: 2,
	perRun: [],
	auto: [],
	introduced: false,
	offerAnswered: false,
	noSave: [],
	forgotten: {},
	nextRunNumber: 15,
	fieldSeenAt: {},
};

function run(id: string, values: FieldValues): RunEntry {
	return {
		id,
		n: 14,
		copy: {
			values,
			presetName: null,
			leftAsIs: false,
			perRun: [],
			replaced: [],
		},
	} as unknown as RunEntry;
}

interface Setup {
	readonly values?: FieldValues;
	readonly perRun?: readonly string[];
	readonly auto?: readonly string[];
	readonly nextFiles?: RailState["nextFiles"];
	readonly presets?: readonly Preset[];
	readonly activePresetId?: string | null;
	readonly comparedRunId?: string | null;
	readonly runs?: readonly RunEntry[];
}

const SETUP: Required<Setup> = {
	values: FILLED,
	perRun: [],
	auto: [],
	nextFiles: {},
	presets: [],
	activePresetId: null,
	comparedRunId: null,
	runs: [],
};

function state(
	setup: Setup = {},
): Pick<FormSessionState, "form" | "rail" | "runs" | "memory"> {
	const { values, nextFiles, activePresetId, comparedRunId, ...rest } = {
		...SETUP,
		...setup,
	};
	return {
		form: { fields: MEDIUM } as unknown as FormModel,
		rail: {
			values,
			nextFiles,
			activePresetId,
			comparedRunId,
		} as unknown as RailState,
		runs: rest.runs,
		memory: {
			loaded: true,
			prefs: { ...PREFS, perRun: rest.perRun, auto: rest.auto },
			presets: rest.presets,
		},
	};
}

describe("the dot and Reset (spec 2.0)", () => {
	test("on Done only Expected total has the dot; typed required fields get Clear, not a dot", () => {
		const done = state();
		expect(fieldMarkers(done, field("expected_total"))).toMatchObject({
			changed: true,
			reset: "reset",
		});
		expect(fieldMarkers(done, field("vendor_name"))).toMatchObject({
			changed: false,
			reset: "clear",
		});
		expect(fieldMarkers(done, field("invoice_date"))).toMatchObject({
			changed: false,
			reset: "clear",
		});
		expect(fieldMarkers(done, field("max_pages"))).toMatchObject({
			changed: false,
			reset: null,
		});
		expect(fieldMarkers(done, field("run_ocr"))).toMatchObject({
			changed: false,
			reset: null,
		});
	});

	test("a per-run field never has the dot but keeps Reset", () => {
		const perRun = state({ perRun: ["expected_total"] });
		expect(fieldMarkers(perRun, field("expected_total"))).toMatchObject({
			changed: false,
			reset: "reset",
			perRun: "perRun",
		});
	});

	test("under a preset its value is the starting value", () => {
		const nordwind: Preset = {
			id: "p",
			name: "Nordwind Logistik GmbH",
			digit: 2,
			sets: { vendor_name: "Nordwind Logistik GmbH" },
			kinds: {},
			openDefault: true,
			createdAt: 0,
			updatedAt: 0,
			lastUsedAt: null,
		};
		const active = state({ presets: [nordwind], activePresetId: "p" });
		expect(activePresetOf(active)).toBe(nordwind);
		expect(fieldMarkers(active, field("vendor_name"))).toMatchObject({
			changed: false,
			reset: null,
		});
		const edited = state({
			presets: [nordwind],
			activePresetId: "p",
			values: { ...FILLED, vendor_name: "Other" },
		});
		expect(fieldMarkers(edited, field("vendor_name"))).toMatchObject({
			changed: true,
			reset: "reset",
		});
	});

	test("an object's properties have markers of their own", () => {
		const key = `payment_terms${FIELD_KEY_SEPARATOR}net_days`;
		const values = {
			...FILLED,
			payment_terms: { currency: "EUR", net_days: "30", note: "hi" },
		};
		const terms = state({ values });
		expect(prop("net_days").key).toBe(key);
		expect(fieldMarkers(terms, prop("net_days"))).toMatchObject({
			changed: true,
			reset: "reset",
			perRun: null,
			optional: false,
		});
		expect(fieldMarkers(terms, prop("note"))).toMatchObject({
			changed: false,
			reset: "clear",
			optional: true,
		});
		expect(fieldMarkers(terms, field("payment_terms"))).toMatchObject({
			changed: true,
			optional: true,
		});
	});
});

describe("Per run, Next file, Optional and the count", () => {
	test("Next file while next files wait, Per run otherwise; the series' auto fields count too", () => {
		const series = state({
			auto: ["invoice_file", "invoice_date"],
			nextFiles: { invoice_file: [slot("next.pdf")] },
		});
		expect(perRunNamesOf(series)).toEqual(["invoice_file", "invoice_date"]);
		expect(fieldMarkers(series, field("invoice_file")).perRun).toBe("nextFile");
		expect(fieldMarkers(series, field("invoice_date")).perRun).toBe("perRun");
		expect(fieldMarkers(series, field("vendor_name")).perRun).toBeNull();
		const last = state({
			auto: ["invoice_file"],
			nextFiles: { invoice_file: [] },
		});
		expect(fieldMarkers(last, field("invoice_file")).perRun).toBe("perRun");
	});

	test("Optional on optional fields and objects, never on switches; the file count", () => {
		const done = state();
		expect(fieldMarkers(done, field("supporting_documents"))).toMatchObject({
			optional: true,
			count: 2,
		});
		expect(fieldMarkers(done, field("invoice_file"))).toMatchObject({
			optional: false,
			count: null,
		});
		expect(fieldMarkers(done, field("run_ocr")).optional).toBe(false);
		expect(fieldMarkers(done, field("expected_total")).optional).toBe(true);
	});
});

describe("the 2 px edge against the compared run", () => {
	test("only with a compared run; per-run fields never", () => {
		const before = { ...FILLED, max_pages: "40", invoice_date: "2026-09-16" };
		const compared = state({
			runs: [run("r14", before)],
			comparedRunId: "r14",
			perRun: ["invoice_date"],
		});
		expect(comparedRunOf(compared)?.id).toBe("r14");
		expect(fieldMarkers(compared, field("max_pages")).differs).toBe(true);
		expect(fieldMarkers(compared, field("invoice_date")).differs).toBe(false);
		expect(fieldMarkers(compared, field("vendor_name")).differs).toBe(false);
		const context = state({ runs: [run("r14", before)] });
		expect(comparedRunOf(context)).toBeNull();
		expect(fieldMarkers(context, field("max_pages")).differs).toBe(false);
		expect(comparedRunOf(state({ comparedRunId: "gone" }))).toBeNull();
	});
});
