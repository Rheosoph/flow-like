import { describe, expect, test } from "bun:test";
import { FORM_LIMITS, type Preset } from "../contracts";
import { fixture } from "../testing/fixtures";
import {
	blockOf,
	clashOf,
	draftOf,
	initialTicks,
	isTickable,
	saveRowsOf,
	saveSourceOf,
	saveStartOf,
} from "./preset-save-model";
import { secretCheck } from "./rail-model";

const preset = (patch: Partial<Preset>): Preset => ({
	id: "p",
	name: "Preset",
	digit: 1,
	sets: {},
	kinds: {},
	openDefault: false,
	createdAt: 0,
	updatedAt: 0,
	lastUsedAt: null,
	...patch,
});

describe("what the dialog opens with (spec §8: preset-save)", () => {
	const state = fixture("preset-save");
	const start = saveStartOf(state, "save", null);

	test("Vendor is the one ticked input; the name is its value", () => {
		expect(start.ticks).toEqual(["vendor_name"]);
		expect(start.proposal).toEqual({
			kind: "value",
			text: "Nordwind Logistik GmbH",
		});
	});

	test("the first preset applies when the form opens", () => {
		expect(start.openDefault).toBe(true);
		expect(start.updating).toBeNull();
	});

	test("nine inputs: Vendor and the Invoice file differ, seven sit at their defaults", () => {
		const rows = saveRowsOf(
			state.form.fields,
			start.source,
			secretCheck(state.memory.prefs.noSave),
		);
		expect(rows).toHaveLength(9);
		const main = rows
			.filter((row) => !row.atDefault)
			.map((row) => row.field.name);
		expect(main).toEqual(["invoice_file", "vendor_name"]);
		expect(rows.filter((row) => row.atDefault)).toHaveLength(7);
	});

	test("a file is never tickable and says so; per-run inputs are flagged", () => {
		const rows = saveRowsOf(state.form.fields, start.source, secretCheck([]));
		const invoice = rows.find((row) => row.field.name === "invoice_file");
		expect(invoice?.kind).toBe("files");
		expect(invoice ? isTickable(invoice) : true).toBe(false);
		expect(invoice?.perRun).toBe(true);
		const vendor = rows.find((row) => row.field.name === "vendor_name");
		expect(vendor ? isTickable(vendor) : false).toBe(true);
		expect(vendor?.perRun).toBe(false);
	});

	test("a secret field is listed, disabled", () => {
		const rows = saveRowsOf(
			state.form.fields,
			start.source,
			secretCheck(["vendor_name"]),
		);
		const vendor = rows.find((row) => row.field.name === "vendor_name");
		expect(vendor?.kind).toBe("secret");
		expect(vendor ? isTickable(vendor) : true).toBe(false);
		expect(
			initialTicks(
				state.form.fields,
				start.source,
				secretCheck(["vendor_name"]),
			),
		).toEqual([]);
	});
});

describe("update mode and presets that exist", () => {
	const state = fixture("presets");

	test("the dialog opens on the active preset's own name and settings", () => {
		const start = saveStartOf(state, "update", null);
		expect(start.updating?.name).toBe("Nordwind Logistik GmbH");
		expect(start.proposal).toEqual({
			kind: "value",
			text: "Nordwind Logistik GmbH",
		});
		expect(start.openDefault).toBe(true);
		expect(start.ticks).toContain("vendor_name");
	});

	test("a new preset is not on open while another one is", () => {
		const start = saveStartOf(state, "save", null);
		expect(start.updating).toBeNull();
		expect(start.openDefault).toBe(false);
	});

	test("with nothing worth naming the name is numbered", () => {
		const idle = fixture("idle");
		const start = saveStartOf(idle, "save", null);
		expect(start.proposal).toEqual({ kind: "numbered", n: 1 });
		expect(start.ticks).toEqual([]);
	});
});

describe("from a run's menu", () => {
	test("the run's copy is the source, not the rail", () => {
		const state = fixture("done");
		const run = state.runs.find((item) => item.id === "run-9");
		expect(run).toBeDefined();
		const source = saveSourceOf(state, "run-9");
		expect(source.values).toBe(run?.copy.values ?? {});
		expect(source.perRun).toBe(run?.copy.perRun ?? []);
		expect(saveSourceOf(state, "gone").values).toBe(state.rail.values);
	});
});

describe("names", () => {
	const presets = [
		preset({ id: "a", name: "Alpenfracht AG" }),
		preset({ id: "b", name: "Café Müller" }),
	];

	test("a clash ignores case and accents and the preset being updated", () => {
		expect(clashOf(presets, "alpenfracht ag", null)?.id).toBe("a");
		expect(clashOf(presets, "CAFE muller", null)?.id).toBe("b");
		expect(clashOf(presets, "Alpenfracht AG", "a")).toBeNull();
		expect(clashOf(presets, "  ", null)).toBeNull();
		expect(clashOf(presets, "Other", null)).toBeNull();
	});
});

describe("when the primary cannot save", () => {
	const ok = { name: "Nordwind", ticked: 1, presets: 2, replaceId: null };

	test("an empty name, nothing ticked, a form with all its presets", () => {
		expect(blockOf(ok)).toBeNull();
		expect(blockOf({ ...ok, name: "   " })).toBe("name");
		expect(blockOf({ ...ok, ticked: 0 })).toBe("nothing");
		expect(blockOf({ ...ok, presets: FORM_LIMITS.presetsPerForm })).toBe(
			"limit",
		);
	});

	test("replacing a preset is fine at the limit", () => {
		expect(
			blockOf({ ...ok, presets: FORM_LIMITS.presetsPerForm, replaceId: "a" }),
		).toBeNull();
	});

	test("the name is judged before the ticks", () => {
		expect(blockOf({ ...ok, name: "", ticked: 0 })).toBe("name");
	});
});

describe("the draft", () => {
	test("ticked names in form order, trimmed name, ids as given", () => {
		const state = fixture("preset-save");
		const start = saveStartOf(state, "save", null);
		const rows = saveRowsOf(state.form.fields, start.source, secretCheck([]));
		const draft = draftOf({
			name: "  Nordwind  ",
			openDefault: true,
			rows,
			ticked: new Set(["vendor_name", "max_pages"]),
			fromRunId: null,
			replaceId: "x",
		});
		expect(draft).toEqual({
			name: "Nordwind",
			openDefault: true,
			ticked: ["vendor_name", "max_pages"],
			fromRunId: null,
			replaceId: "x",
		});
	});
});
