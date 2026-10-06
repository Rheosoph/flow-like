import { describe, expect, test } from "bun:test";
import {
	FIELD_KEY_SEPARATOR,
	type FileSlot,
	type InitialSessionState,
	type ReduceSession,
} from "../contracts";
import {
	FIXTURE_HOSTED_HOST,
	type FixtureName,
	fixture,
	fixtureClock,
	fixtureDefaults,
	fixtureForm,
} from "../testing/fixtures";
import { reduceSession } from "./reduce";
import { focusedKey, sessionDriver } from "./reduce-driver";
import { initialSessionState } from "./state";

const _reduce: ReduceSession = reduceSession;
const _initial: InitialSessionState = initialSessionState;
void _reduce;
void _initial;

const NET_DAYS = `payment_terms${FIELD_KEY_SEPARATOR}net_days`;

const from = (name: FixtureName) =>
	sessionDriver(fixture(name), fixtureClock(name));

describe("initialSessionState", () => {
	test("the form at its defaults; memory not loaded; the cap of an unresolved target", () => {
		const form = fixtureForm("medium");
		const state = initialSessionState(form, fixtureClock("idle"));
		expect(state.rail.values).toEqual(fixtureDefaults("medium"));
		expect(state.runs).toEqual([]);
		expect(state.memory.loaded).toBe(false);
		expect(state.memory.prefs.nextRunNumber).toBe(1);
		expect(state.layout).toBeNull();
		expect(state.queue).toEqual({
			target: null,
			tierLimit: null,
			cap: 2,
			hold: null,
			retryAt: null,
		});
		expect(state.view.stageFollowsNewest).toBe(true);
		expect(state.seq).toBe(0);
	});

	test("a hosted link runs in the cloud from the start", () => {
		const form = fixtureForm("medium", FIXTURE_HOSTED_HOST);
		const state = initialSessionState(form, fixtureClock("idle"));
		expect(state.queue.target).toBe("remote");
		expect(state.queue.cap).toBe(2);
	});
});

describe("edits", () => {
	test("an edit clears its own message, not the others", () => {
		const driver = from("invalid");
		driver.command({ type: "setValue", key: "vendor_name", value: "Nordwind" });
		expect(Object.keys(driver.state.rail.problems)).toEqual(["invoice_file"]);
		expect(driver.state.rail.values.vendor_name).toBe("Nordwind");
	});

	test("an edit ends the dock's Undo and its message", () => {
		const driver = from("idle");
		driver.command({ type: "setValue", key: "max_pages", value: "60" });
		driver.command({ type: "resetField", key: "max_pages" });
		expect(driver.state.undo?.kind).toBe("fieldReset");
		driver.command({ type: "setValue", key: "vendor_name", value: "A" });
		expect(driver.state.undo).toBeNull();
		expect(driver.state.view.message).toBeNull();
	});

	test("the first change of a text entry decides how it began", () => {
		const driver = from("idle");
		driver.command({
			type: "setValue",
			key: "vendor_name",
			value: "N",
			how: "replace",
		});
		driver.command({
			type: "setValue",
			key: "vendor_name",
			value: "No",
			how: "inPlace",
		});
		expect(driver.state.rail.entryBegan).toEqual({ vendor_name: "replaced" });
		driver.command({
			type: "setValue",
			key: "max_pages",
			value: "3",
			how: "replace",
		});
		expect(driver.state.rail.entryBegan).toEqual({ vendor_name: "replaced" });
	});
});

describe("typed dates (M4)", () => {
	test("leaving the field commits '18' against run 14's 17 Sep: 18 Sep 2026", () => {
		const driver = from("reopen");
		driver.command({ type: "setText", key: "invoice_date", text: "18" });
		driver.command({ type: "blurField", key: "invoice_date" });
		const { rail } = driver.state;
		expect(rail.values.invoice_date).toBe("2026-09-18");
		expect(rail.texts).toEqual({});
		expect(rail.dateAnchors.invoice_date).toBe("2026-09-18");
		expect(rail.problems).toEqual({});
	});

	test("text that cannot be read shows its message and keeps the text", () => {
		const driver = from("reopen");
		driver.command({ type: "setText", key: "invoice_date", text: "31" });
		driver.command({ type: "blurField", key: "invoice_date" });
		expect(driver.state.rail.problems.invoice_date).toEqual({ code: "date" });
		expect(driver.state.rail.texts.invoice_date).toBe("31");
		expect(driver.state.rail.values.invoice_date).toBe("");
		driver.command({ type: "setText", key: "invoice_date", text: "3" });
		expect(driver.state.rail.problems).toEqual({});
	});

	test("a day far back only commits on ↵: '1' after 30 Sep", () => {
		const driver = from("idle");
		driver.command({ type: "setText", key: "invoice_date", text: "30/9" });
		driver.command({ type: "enter", fromKey: "invoice_date" });
		expect(driver.state.rail.values.invoice_date).toBe("2026-09-30");
		const moved = driver.state.view.focus;
		expect(focusedKey(driver.state)).toBe("invoice_file");
		driver.command({ type: "setText", key: "invoice_date", text: "1" });
		driver.command({ type: "enter", fromKey: "invoice_date" });
		expect(driver.state.rail.values.invoice_date).toBe("2026-09-01");
		expect(driver.state.view.focus).toBe(moved);
		driver.command({ type: "enter", fromKey: "invoice_date" });
		expect(focusedKey(driver.state)).toBe("invoice_file");
	});

	test("commitText commits a number's typed text as its value", () => {
		const driver = from("idle");
		driver.command({ type: "setText", key: "max_pages", text: "45" });
		driver.command({ type: "commitText", key: "max_pages" });
		expect(driver.state.rail.values.max_pages).toBe("45");
		expect(driver.state.rail.texts).toEqual({});
	});

	test("validation on blur: an empty required field says so; an edit clears it", () => {
		const driver = from("idle");
		driver.command({ type: "blurField", key: "vendor_name" });
		expect(driver.state.rail.problems.vendor_name).toEqual({
			code: "required",
		});
		expect(driver.state.rail.pressed).toBe(false);
		driver.command({ type: "setValue", key: "vendor_name", value: "A" });
		expect(driver.state.rail.problems).toEqual({});
	});

	test("a blur the form causes itself (moving the cursor on) does not validate; typed text still commits", () => {
		const driver = from("idle");
		driver.command({ type: "setValue", key: "invoice_date", value: "" });
		driver.command({ type: "resetField", key: "vendor_name" });
		expect(focusedKey(driver.state)).toBe("vendor_name");
		driver.command({ type: "blurField", key: "invoice_date" });
		expect(driver.state.rail.problems.invoice_date).toBeUndefined();

		driver.command({ type: "setText", key: "max_pages", text: "45" });
		driver.command({ type: "blurField", key: "max_pages" });
		expect(driver.state.rail.values.max_pages).toBe("45");

		driver.command({
			type: "focusHandled",
			seq: driver.state.view.focus?.seq ?? 0,
		});
		driver.command({ type: "blurField", key: "invoice_date" });
		expect(driver.state.rail.problems.invoice_date).toEqual({
			code: "required",
		});
	});

	test("the field the session sends the cursor to still validates when it is left", () => {
		const driver = from("idle");
		driver.command({ type: "resetField", key: "vendor_name" });
		driver.command({ type: "blurField", key: "vendor_name" });
		expect(driver.state.rail.problems.vendor_name).toEqual({
			code: "required",
		});
	});
});

describe("↵ moves on (M2)", () => {
	test("to the next empty required field, its value selected", () => {
		const driver = from("idle");
		driver.command({ type: "setValue", key: "vendor_name", value: "Nordwind" });
		driver.command({ type: "enter", fromKey: "vendor_name" });
		expect(driver.state.view.focus?.target).toEqual({
			kind: "field",
			key: "invoice_date",
			select: true,
			scrollOnly: false,
		});
		expect(driver.state.rail.problems).toEqual({});
	});

	test("an empty required field shows its message and stays", () => {
		const driver = from("idle");
		driver.command({ type: "enter", fromKey: "invoice_date" });
		expect(driver.state.rail.problems.invoice_date).toEqual({
			code: "required",
		});
		expect(driver.state.view.focus).toBeNull();
		expect(driver.state.runs).toEqual([]);
	});

	test("a file field this host cannot fill is no stop; Run explains it under the field", () => {
		const driver = from("hosted-files");
		driver.command({ type: "setValue", key: "vendor_name", value: "Nordwind" });
		driver.command({ type: "enter", fromKey: "vendor_name" });
		expect(focusedKey(driver.state)).toBe("invoice_date");
		driver.command({
			type: "setValue",
			key: "invoice_date",
			value: "2026-09-18",
		});
		driver.command({ type: "enter", fromKey: "invoice_date" });
		expect(driver.state.runs).toEqual([]);
		expect(driver.state.rail.problems).toEqual({
			invoice_file: { code: "fileNotHere" },
		});
		expect(driver.state.rail.pressed).toBe(true);
	});

	test("with nothing missing it runs, unless the inputs equal the newest run of this session", () => {
		const driver = from("done");
		driver.command({ type: "enter", fromKey: "vendor_name" });
		expect(driver.state.runs[0].n).toBe(14);
		expect(driver.state.view.message?.message).toEqual({
			kind: "sameInputs",
			n: 14,
		});
		driver.command({ type: "run", leaveAsIs: false, from: "chord" });
		expect(driver.state.runs[0].n).toBe(15);
		driver.advance(1000).command({ type: "enter", fromKey: "vendor_name" });
		expect(driver.state.view.message?.message).toEqual({
			kind: "sameInputs",
			n: 15,
		});
		driver.command({ type: "setValue", key: "max_pages", value: "21" });
		driver.command({ type: "enter", fromKey: "max_pages" });
		expect(driver.state.runs[0].n).toBe(16);
		expect(driver.state.runs[0].copy.values.max_pages).toBe("21");
	});

	test("↵ on a value that cannot be read stays with the message", () => {
		const driver = from("done");
		driver.command({ type: "setValue", key: "max_pages", value: "twelve" });
		driver.command({ type: "enter", fromKey: "max_pages" });
		expect(driver.state.rail.problems.max_pages?.code).toBe("integer");
		expect(driver.state.runs[0].n).toBe(14);
	});
});

describe("Reset", () => {
	test("⇧⌘⌫: '{label} reset.' with Undo, focus on the field; Undo puts the value back", () => {
		const driver = from("idle");
		driver.command({ type: "setValue", key: "max_pages", value: "60" });
		driver.command({ type: "resetField", key: "max_pages" });
		expect(driver.state.rail.values.max_pages).toBe("20");
		expect(driver.state.view.message?.message).toEqual({
			kind: "fieldReset",
			label: "Max pages",
		});
		expect(driver.state.view.message?.undo).toBe(true);
		expect(focusedKey(driver.state)).toBe("max_pages");
		driver.command({ type: "undo" });
		expect(driver.state.rail.values.max_pages).toBe("60");
		expect(driver.state.undo).toBeNull();
		expect(driver.state.view.message).toBeNull();
	});

	test("an object's property goes back to its own starting value", () => {
		const driver = from("idle");
		driver.command({ type: "setValue", key: NET_DAYS, value: "30" });
		driver.command({ type: "resetField", key: NET_DAYS });
		expect(driver.state.rail.values.payment_terms).toEqual({
			currency: "EUR",
			net_days: "14",
			discount_percent: "2",
			discount_days: "14",
		});
	});

	test("Reset on a file field of a series takes the next file", () => {
		const driver = from("series");
		const current = driver.state.rail.values.invoice_file as FileSlot;
		driver.command({ type: "resetField", key: "invoice_file" });
		expect((driver.state.rail.values.invoice_file as FileSlot).name).toBe(
			"invoice-RE-2026-0923.pdf",
		);
		expect(driver.state.view.message?.message).toEqual({
			kind: "fileRemoved",
			name: current.name,
		});
	});

	test("Reset to defaults leaves a running series' file alone and focuses the first field", () => {
		const driver = from("series");
		const before = driver.state.rail;
		driver.command({ type: "resetAll" });
		const { rail, view } = driver.state;
		expect(rail.values.vendor_name).toBe("");
		expect(rail.values.invoice_file).toBe(before.values.invoice_file);
		expect(rail.nextFiles).toBe(before.nextFiles);
		expect(view.message?.message).toEqual({
			kind: "resetTo",
			presetName: null,
		});
		expect(driver.state.undo?.kind).toBe("resetAll");
		expect(focusedKey(driver.state)).toBe("invoice_file");
		driver.command({ type: "undo" });
		expect(driver.state.rail.values.vendor_name).toBe("Nordwind Logistik GmbH");
	});

	test("the filter and the rail tab", () => {
		const driver = from("large");
		driver.command({ type: "setFilter", filter: { query: "port" } });
		driver.command({ type: "setFilter", filter: { chip: "required" } });
		expect(driver.state.rail.filter).toEqual({
			query: "port",
			chip: "required",
		});
		driver.command({ type: "setRailTab", tab: "runs" });
		expect(driver.state.rail.tab).toBe("runs");
	});
});
