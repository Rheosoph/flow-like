import { describe, expect, test } from "bun:test";
import type { FormSessionState, WorkbenchField } from "../contracts";
import { completion, recallFor } from "../model/recent";
import { fixture, fixtureClock, fixtureForm } from "../testing/fixtures";
import { DESKTOP_LAYOUT } from "../testing/layouts";
import { recordOf } from "./history";
import {
	type SessionDriver,
	fileNames,
	focusedKey,
	pickedFiles,
	sessionDriver,
} from "./reduce-driver";
import { initialSessionState } from "./state";

/** FLP_INVOICES: ten Nordwind invoices, each with its own date. */
const INVOICES = [
	["0918", 1198080, "2026-09-18"],
	["0919", 1361920, "2026-09-21"],
	["0920", 1003520, "2026-09-21"],
	["0921", 1247232, "2026-09-22"],
	["0922", 1144832, "2026-09-23"],
	["0923", 4089446, "2026-09-24"],
	["0924", 1310720, "2026-09-25"],
	["0925", 1093632, "2026-09-28"],
	["0926", 1222656, "2026-09-29"],
	["0927", 1175552, "2026-09-30"],
].map(([number, size, date]) => ({
	name: `invoice-RE-2026-${number}.pdf`,
	size: size as number,
	date: date as string,
}));

/** The folder the dialog opens on also holds 0917, which run 14 already sent. */
const FOLDER = [
	{ name: "invoice-RE-2026-0917.pdf", size: 1284096 },
	...INVOICES,
];

const field = (state: FormSessionState, name: string) =>
	state.form.fields.find((item) => item.name === name) as WorkbenchField;

/** "Extract invoice" opened again: runs 1–14 on this device, none of this session (spec §6). */
function reopened(): SessionDriver {
	const form = fixtureForm("medium");
	const clock = fixtureClock("reopen");
	const runs = fixture("reopen").runs.map((run) =>
		recordOf(run, form, "profile:local"),
	);
	const driver = sessionDriver(initialSessionState(form, clock), clock);
	driver.command({ type: "setLayout", layout: DESKTOP_LAYOUT });
	driver.input({ type: "targetResolved", target: "local", tierLimit: null });
	driver.input({
		type: "memoryLoaded",
		memory: { prefs: null, presets: [], runs },
	});
	return driver;
}

/** Steps 1–6 of the walk: the pick, the vendor by its suggestion; returns the keys pressed. */
function startSeries(driver: SessionDriver) {
	let keys = 2;
	driver.command({
		type: "pickFiles",
		name: "invoice_file",
		files: pickedFiles(FOLDER),
		mode: "replace",
	});
	driver.sendUploads();
	keys += 1;
	driver.command({ type: "enter", fromKey: "invoice_file" });
	keys += 2;
	driver.command({
		type: "setValue",
		key: "vendor_name",
		value: "N",
		how: "replace",
	});
	const vendor = field(driver.state, "vendor_name");
	const recall = recallFor(driver.state, vendor, true);
	const suggestion = recall
		? completion(recall.source, vendor, "N", recall.forgotten)
		: null;
	expect(suggestion).toBe("Nordwind Logistik GmbH");
	driver.command({
		type: "setValue",
		key: "vendor_name",
		value: suggestion ?? "",
		how: "inPlace",
	});
	keys += 1;
	driver.command({ type: "enter", fromKey: "vendor_name" });
	expect(focusedKey(driver.state)).toBe("invoice_date");
	return keys;
}

/** Two digits of the day and ↵: three keys per invoice. */
function typeDay(driver: SessionDriver, iso: string) {
	const day = String(Number(iso.slice(8)));
	for (let length = 1; length <= day.length; length += 1)
		driver.command({
			type: "setText",
			key: "invoice_date",
			text: day.slice(0, length),
		});
	driver.command({ type: "enter", fromKey: "invoice_date" });
	return day.length + 1;
}

describe("the benchmark (spec §6): ten invoices", () => {
	test("the pick: 0918 in the field, nine next files, 0917 left out, a series", () => {
		const driver = reopened();
		driver.command({
			type: "pickFiles",
			name: "invoice_file",
			files: pickedFiles(FOLDER),
			mode: "replace",
		});
		const { rail, memory, view } = driver.state;
		expect(fileNames(rail.values.invoice_file)).toBe(
			"invoice-RE-2026-0918.pdf",
		);
		expect(rail.nextFiles.invoice_file.map((slot) => slot.name)).toEqual(
			INVOICES.slice(1).map((invoice) => invoice.name),
		);
		expect(rail.leftOut.invoice_file).toHaveLength(1);
		expect(rail.leftOut.invoice_file[0].slot.name).toBe(
			"invoice-RE-2026-0917.pdf",
		);
		expect(rail.leftOut.invoice_file[0].n).toBe(14);
		expect(memory.prefs.auto).toEqual([
			"invoice_file",
			"supporting_documents",
			"invoice_date",
		]);
		expect(view.list).toEqual({
			kind: "nextFiles",
			key: "invoice_file",
			active: -1,
		});
		expect(focusedKey(driver.state)).toBe("invoice_file");
		const uploads = driver.last.filter((effect) => effect.type === "upload");
		expect(
			uploads.map((effect) => effect.type === "upload" && effect.slotId),
		).toEqual([
			"pick-invoice-RE-2026-0918.pdf",
			"pick-invoice-RE-2026-0919.pdf",
		]);
	});

	test("36 keys and 1 pick; every run pairs its own file and date", () => {
		const driver = reopened();
		let keys = startSeries(driver);
		const picks = 1;
		for (const [index, invoice] of INVOICES.entries()) {
			keys += typeDay(driver, invoice.date);
			driver.advance(10_000).sendUploads();
			const run = driver.state.runs[0];
			expect(run.n).toBe(15 + index);
			expect(fileNames(run.copy.values.invoice_file)).toBe(invoice.name);
			expect(run.copy.values.invoice_date).toBe(invoice.date);
			expect(run.copy.values.vendor_name).toBe("Nordwind Logistik GmbH");
			const last = index === INVOICES.length - 1;
			expect(focusedKey(driver.state)).toBe(
				last ? "invoice_file" : "invoice_date",
			);
		}
		expect(keys).toBe(36);
		expect(picks).toBe(1);
	});

	test("run 15's start message names its pairing; the rail holds the next invoice", () => {
		const driver = reopened();
		startSeries(driver);
		typeDay(driver, INVOICES[0].date);
		const { rail, view } = driver.state;
		expect(view.message?.message).toEqual({
			kind: "start",
			start: {
				n: 15,
				runId: driver.state.runs[0].id,
				state: "started",
				files: 0,
				pairs: ["invoice-RE-2026-0918.pdf", "18 Sep 2026"],
				lastFile: false,
				leftAsIs: false,
			},
		});
		expect(fileNames(rail.values.invoice_file)).toBe(
			"invoice-RE-2026-0919.pdf",
		);
		expect(rail.nextFiles.invoice_file).toHaveLength(8);
		expect(rail.values.invoice_date).toBe("");
		expect(view.list).toBeNull();
		expect(driver.state.runs[0].status).toBe("starting");
		expect(view.selectedRunId).toBe(driver.state.runs[0].id);
	});

	test("three run at once on this device; run 18 queues and the stage stays on run 17", () => {
		const driver = reopened();
		startSeries(driver);
		for (const invoice of INVOICES.slice(0, 4)) {
			typeDay(driver, invoice.date);
			driver.advance(10_000).sendUploads();
		}
		const byN = (n: number) => driver.state.runs.find((run) => run.n === n);
		expect([15, 16, 17].map((n) => byN(n)?.status)).toEqual([
			"starting",
			"starting",
			"starting",
		]);
		expect(byN(18)?.status).toBe("queued");
		expect(driver.state.view.selectedRunId).toBe(byN(17)?.id ?? "");
		expect(driver.state.view.message?.message).toEqual({
			kind: "start",
			start: {
				n: 18,
				runId: byN(18)?.id,
				state: "queued",
				files: 0,
				pairs: ["invoice-RE-2026-0921.pdf", "22 Sep 2026"],
				lastFile: false,
				leftAsIs: false,
			},
		});
	});

	test("the tenth run takes the last file; then the series-end question", () => {
		const driver = reopened();
		startSeries(driver);
		for (const invoice of INVOICES) {
			typeDay(driver, invoice.date);
			driver.advance(10_000).sendUploads();
		}
		const { rail, view } = driver.state;
		const start = view.message?.message;
		expect(start?.kind === "start" && start.start.lastFile).toBe(true);
		expect(start?.kind === "start" && start.start.n).toBe(24);
		expect(rail.values.invoice_file).toBeNull();
		expect(rail.nextFiles.invoice_file).toBeUndefined();
		expect(view.question).toEqual({
			kind: "seriesEnd",
			names: ["invoice_file", "supporting_documents", "invoice_date"],
		});
	});
});
