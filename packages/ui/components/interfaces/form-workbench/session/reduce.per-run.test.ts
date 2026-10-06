import { describe, expect, test } from "bun:test";
import type { FileSlot, SessionEffect } from "../contracts";
import { type FixtureName, fixture, fixtureClock } from "../testing/fixtures";
import {
	type SessionDriver,
	focusedKey,
	pickedFiles,
	sessionDriver,
} from "./reduce-driver";

const from = (name: FixtureName) =>
	sessionDriver(fixture(name), fixtureClock(name));

const savedPrefs = (effects: readonly SessionEffect[]) =>
	effects.flatMap((effect) =>
		effect.type === "persistPrefs" ? [effect.prefs] : [],
	);

const PARCELS = [
	{ order: "48213-7", receipt: "receipt-48213-7.jpg" },
	{ order: "48231-1", receipt: "receipt-48231-1.jpg" },
	{ order: "48240-3", receipt: "receipt-48240-3.jpg" },
];

/** One parcel: the order number typed over (or edited in place), its receipt picked, Run. */
function parcel(
	driver: SessionDriver,
	index: number,
	how: "replace" | "inPlace" = "replace",
) {
	const { order, receipt } = PARCELS[index];
	driver.command({ type: "setValue", key: "order", value: order, how });
	driver.command({
		type: "pickFiles",
		name: "receipt",
		files: pickedFiles([{ name: receipt, size: 400_000 }], `p${index}`),
		mode: "replace",
	});
	driver.sendUploads();
	driver
		.advance(30_000)
		.command({ type: "run", leaveAsIs: false, from: "enter" });
	return driver;
}

describe("the setting (spec M1)", () => {
	test("a change applies at once, is saved and introduces the after-run line", () => {
		const driver = from("small");
		driver.command({ type: "setPerRun", name: "order", on: true });
		expect(driver.state.memory.prefs.perRun).toEqual(["order"]);
		expect(driver.state.memory.prefs.introduced).toBe(true);
		expect(savedPrefs(driver.last)).toHaveLength(1);
		expect(driver.state.rail.values.order).toBe("");
		driver.command({ type: "setPerRun", name: "order", on: false });
		expect(driver.state.memory.prefs.perRun).toEqual([]);
		expect(driver.state.memory.prefs.introduced).toBe(true);
	});

	test("a field changed in the popover leaves the series: it is the person's own setting", () => {
		const driver = from("series");
		driver.command({ type: "setPerRun", name: "invoice_date", on: false });
		expect(driver.state.memory.prefs.auto).toEqual([
			"invoice_file",
			"supporting_documents",
		]);
		driver.command({ type: "setPerRun", name: "invoice_file", on: true });
		expect(driver.state.memory.prefs.auto).toEqual(["supporting_documents"]);
		expect(driver.state.memory.prefs.perRun).toEqual(["invoice_file"]);
	});

	test("Uncheck all; Make files and dates per run", () => {
		const driver = from("series");
		driver.command({ type: "uncheckAllPerRun" });
		expect(driver.state.memory.prefs.perRun).toEqual([]);
		expect(driver.state.memory.prefs.auto).toEqual([]);
		driver.command({ type: "perRunFilesAndDates" });
		expect(driver.state.memory.prefs.perRun).toEqual([
			"invoice_file",
			"supporting_documents",
			"invoice_date",
		]);
	});

	test("closing the popover returns the cursor to the marker's field, else to Change", () => {
		const driver = from("after-run");
		driver.command({ type: "closeOverlay" });
		expect(driver.state.view.overlay).toBeNull();
		expect(driver.state.view.focus?.target).toEqual({ kind: "change" });
		driver.command({
			type: "openOverlay",
			overlay: { id: "afterRun", focusName: "payment_terms" },
		});
		driver.command({ type: "closeOverlay" });
		expect(focusedKey(driver.state)).toBe(
			driver.state.form.fields[8].props[0].key,
		);
	});
});

describe("the offer (spec M1)", () => {
	test("the third parcel brings 'Make Order number and Receipt per run?'", () => {
		const driver = from("small");
		parcel(parcel(driver, 0), 1);
		expect(driver.state.view.question).toBeNull();
		parcel(driver, 2);
		expect(driver.state.view.question).toEqual({
			kind: "offer",
			names: ["order", "receipt"],
		});
		expect(driver.state.memory.prefs.introduced).toBe(true);
	});

	test("Yes: per run now, both reset, the cursor in Order number", () => {
		const driver = parcel(parcel(parcel(from("small"), 0), 1), 2);
		driver.command({ type: "answerQuestion", yes: true });
		const { memory, rail, view } = driver.state;
		expect(memory.prefs.perRun).toEqual(["order", "receipt"]);
		expect(memory.prefs.offerAnswered).toBe(true);
		expect(rail.values.order).toBe("");
		expect(rail.values.receipt).toBeNull();
		expect(rail.values.quantity).toBe("1");
		expect(view.question).toBeNull();
		expect(view.message?.message).toEqual({
			kind: "perRunNow",
			labels: ["Order number", "Receipt"],
		});
		expect(focusedKey(driver.state)).toBe("order");
		expect(driver.state.runs[0].copy.values.order).toBe("48240-3");
	});

	test("No is final: the after-run line exists, no offer again", () => {
		const driver = parcel(parcel(parcel(from("small"), 0), 1), 2);
		driver.command({ type: "answerQuestion", yes: false });
		expect(driver.state.memory.prefs.offerAnswered).toBe(true);
		expect(driver.state.memory.prefs.introduced).toBe(true);
		expect(driver.state.memory.prefs.perRun).toEqual([]);
		driver.command({
			type: "setValue",
			key: "order",
			value: "1",
			how: "replace",
		});
		driver.command({
			type: "pickFiles",
			name: "receipt",
			files: pickedFiles([{ name: "r4.jpg", size: 1 }]),
			mode: "replace",
		});
		driver.sendUploads().advance(30_000);
		driver.command({ type: "run", leaveAsIs: false, from: "button" });
		expect(driver.state.view.question).toBeNull();
	});

	test("order numbers edited in place: the offer names the receipt only", () => {
		const driver = from("small");
		for (const index of [0, 1, 2]) parcel(driver, index, "inPlace");
		expect(driver.state.view.question).toEqual({
			kind: "offer",
			names: ["receipt"],
		});
	});

	test("no offer while something is per run", () => {
		const driver = from("small");
		driver.command({ type: "setPerRun", name: "quantity", on: true });
		parcel(parcel(parcel(driver, 0), 1), 2);
		expect(driver.state.view.question).toBeNull();
	});
});

describe("a series (spec M1, M3)", () => {
	test("its end asks once; Yes keeps its fields per run", () => {
		const driver = from("series");
		const current = driver.state.rail.values.invoice_file as FileSlot;
		driver.command({ type: "clearNextFiles", name: "invoice_file" });
		expect(driver.state.view.question).toBeNull();
		expect(driver.state.rail.nextFiles.invoice_file).toEqual([]);
		driver.command({
			type: "removeFile",
			name: "invoice_file",
			slotId: current.id,
		});
		expect(driver.state.view.question).toEqual({
			kind: "seriesEnd",
			names: ["invoice_file", "supporting_documents", "invoice_date"],
		});
		driver.command({ type: "answerQuestion", yes: true });
		expect(driver.state.memory.prefs.perRun).toEqual([
			"invoice_file",
			"supporting_documents",
			"invoice_date",
		]);
		expect(driver.state.memory.prefs.auto).toEqual([]);
	});

	test("No turns the series' fields off", () => {
		const driver = from("series");
		const current = driver.state.rail.values.invoice_file as FileSlot;
		driver.command({ type: "clearNextFiles", name: "invoice_file" });
		driver.command({
			type: "removeFile",
			name: "invoice_file",
			slotId: current.id,
		});
		driver.command({ type: "answerQuestion", yes: false });
		expect(driver.state.memory.prefs.perRun).toEqual([]);
		expect(driver.state.memory.prefs.auto).toEqual([]);
	});

	test("a multi-file pick while something is per run makes only that field per run", () => {
		const driver = from("small");
		driver.command({ type: "setPerRun", name: "order", on: true });
		driver.command({
			type: "pickFiles",
			name: "receipt",
			files: pickedFiles([
				{ name: "a.jpg", size: 1 },
				{ name: "b.jpg", size: 2 },
			]),
			mode: "replace",
		});
		expect(driver.state.memory.prefs.auto).toEqual(["receipt"]);
		expect(driver.state.memory.prefs.perRun).toEqual(["order"]);
	});
});
