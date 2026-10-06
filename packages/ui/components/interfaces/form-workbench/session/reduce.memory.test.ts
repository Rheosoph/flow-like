import { describe, expect, test } from "bun:test";
import {
	FIELD_KEY_SEPARATOR,
	type FileSlot,
	type FormModel,
	type FormPrefs,
	type SessionEffect,
	type StoredRunRecord,
} from "../contracts";
import { hashValue } from "../model/secrets";
import { repeatBlocker } from "../run/run-view";
import {
	type FixtureName,
	fixture,
	fixtureClock,
	fixtureForm,
} from "../testing/fixtures";
import { recordOf } from "./history";
import { applyLoadedMemory } from "./reduce";
import { fileNames, sessionDriver } from "./reduce-driver";
import { EMPTY_PREFS, initialSessionState } from "./state";

const from = (name: FixtureName) =>
	sessionDriver(fixture(name), fixtureClock(name));

const ofType = <T extends SessionEffect["type"]>(
	effects: readonly SessionEffect[],
	type: T,
) =>
	effects.filter(
		(effect): effect is Extract<SessionEffect, { type: T }> =>
			effect.type === type,
	);

const form = fixtureForm("medium");
const history = (): readonly StoredRunRecord[] =>
	fixture("reopen").runs.map((run) => recordOf(run, form, "profile:local"));

describe("memoryLoaded", () => {
	test("saved runs after this session's; numbers continue past every run; first prefs saved", () => {
		const clock = fixtureClock("reopen");
		const state = initialSessionState(form, clock);
		const step = applyLoadedMemory(
			state,
			{ prefs: null, presets: [], runs: history() },
			clock,
		);
		expect(step.state.memory.loaded).toBe(true);
		expect(step.state.runs.map((run) => run.n)).toEqual(
			Array.from({ length: 14 }, (_, index) => 14 - index),
		);
		expect(step.state.runs.every((run) => run.origin === "history")).toBe(true);
		expect(step.state.memory.prefs.nextRunNumber).toBe(15);
		const prefs = ofType(step.effects, "persistPrefs")[0]?.prefs;
		expect(prefs?.nextRunNumber).toBe(15);
		expect(Object.keys(prefs?.fieldSeenAt ?? {})).toEqual(
			form.fields.map((field) => field.name),
		);
		expect(step.state.view.selectedRunId).toBeNull();
		expect(step.state.rail.comparedRunId).toBeNull();
	});

	test("a series never survives a reload; the on-open preset applies without a message", () => {
		const clock = fixtureClock("reopen");
		const state = initialSessionState(form, clock);
		const prefs: FormPrefs = {
			...EMPTY_PREFS,
			auto: ["invoice_file"],
			nextRunNumber: 40,
			fieldSeenAt: Object.fromEntries(
				form.fields.map((field) => [field.name, clock.now]),
			),
		};
		const presets = fixture("presets").memory.presets;
		const step = applyLoadedMemory(
			state,
			{ prefs, presets, runs: history() },
			clock,
		);
		expect(step.state.memory.prefs.auto).toEqual([]);
		expect(step.state.memory.prefs.nextRunNumber).toBe(40);
		expect(step.state.rail.activePresetId).toBe("preset-nordwind");
		expect(step.state.rail.values.vendor_name).toBe("Nordwind Logistik GmbH");
		expect(step.state.view.message).toBeNull();
		expect(step.state.undo).toBeNull();
		expect(ofType(step.effects, "persistPrefs")).toHaveLength(1);
	});

	test("runs of this session stay first and keep their entries", () => {
		const driver = from("done");
		const session = driver.state.runs[0];
		driver.input({
			type: "memoryLoaded",
			memory: { prefs: null, presets: [], runs: history() },
		});
		expect(driver.state.runs[0]).toBe(session);
		expect(driver.state.runs.filter((run) => run.n === 14)).toHaveLength(1);
	});
});

describe("recent values and secrets (spec M6)", () => {
	test("Delete on a recent value saves its hash; the runs keep it", () => {
		const driver = from("recent");
		driver.command({
			type: "forgetRecent",
			name: "vendor_name",
			value: "Alpenfracht AG",
		});
		expect(driver.state.memory.prefs.forgotten).toEqual({
			vendor_name: [hashValue("Alpenfracht AG")],
		});
		expect(ofType(driver.last, "persistPrefs")).toHaveLength(1);
		driver.command({
			type: "forgetRecent",
			name: "vendor_name",
			value: "alpenfracht ag",
		});
		expect(driver.last).toEqual([]);
	});

	test("Don't save {label}: saved runs lose the value, this session's runs keep it", () => {
		const driver = from("done");
		driver.command({ type: "dontSave", name: "vendor_name" });
		expect(driver.state.memory.prefs.noSave).toEqual(["vendor_name"]);
		expect(driver.last).toContainEqual({
			type: "hideField",
			name: "vendor_name",
		});
		const [session, ...saved] = driver.state.runs;
		expect(session.copy.values.vendor_name).toBe("Nordwind Logistik GmbH");
		for (const run of saved)
			expect(run.copy.values.vendor_name).toEqual({ $hidden: true });
		expect(driver.state.view.message?.message).toEqual({
			kind: "noSave",
			label: "Vendor",
		});
	});

	test("Don't save on an object property marks it in saved runs, as a reload reads them back", () => {
		const driver = from("done");
		driver.command({
			type: "dontSave",
			name: `payment_terms${FIELD_KEY_SEPARATOR}net_days`,
		});
		const [session, ...saved] = driver.state.runs;
		expect(session.copy.values.payment_terms).toMatchObject({ net_days: "14" });
		for (const run of saved) {
			expect(run.copy.values.payment_terms).toEqual({
				currency: "EUR",
				net_days: { $hidden: true },
				discount_percent: "2",
				discount_days: "14",
			});
			const withoutFiles = {
				...run,
				copy: {
					...run.copy,
					values: { payment_terms: run.copy.values.payment_terms },
				},
			};
			expect(repeatBlocker(withoutFiles, driver.now)).toBe("hidden");
		}
		driver.command({ type: "useInputs", runId: "run-9" });
		expect(driver.state.view.message?.message).toMatchObject({
			kind: "inputsIn",
			enterAgain: ["Net days"],
		});
		expect(driver.state.rail.values.payment_terms).toMatchObject({
			net_days: "",
		});
	});
});

describe("Use these inputs (spec S4)", () => {
	test("an older run's files come back as Pick again; Undo puts the rail back", () => {
		const driver = from("done");
		const before = driver.state.rail.values;
		driver.command({ type: "selectRun", runId: "run-9", how: "list" });
		driver.command({ type: "useInputs", runId: "run-9" });
		const { rail, view } = driver.state;
		expect((rail.values.invoice_file as FileSlot).state).toBe("reminder");
		expect(fileNames(rail.values.invoice_file)).toBe(
			"invoice-RE-2026-0902.pdf",
		);
		expect(rail.values.max_pages).toBe("40");
		expect(rail.values.run_ocr).toBe(false);
		expect(rail.tab).toBe("inputs");
		expect(rail.comparedRunId).toBe("run-9");
		expect(view.message?.message).toEqual({
			kind: "inputsIn",
			n: 9,
			pickAgain: 3,
			enterAgain: [],
			misfit: 0,
		});
		expect(view.focus?.target).toEqual({ kind: "run" });
		driver.command({ type: "run", leaveAsIs: false, from: "button" });
		expect(driver.state.rail.problems.invoice_file).toEqual({
			code: "pickAgain",
			fileName: "invoice-RE-2026-0902.pdf",
		});
		driver.command({ type: "undo" });
		expect(driver.state.undo).toBeNull();
		expect(driver.state.rail.values).toBe(before);
	});

	test("Undo right after restores the rail", () => {
		const driver = from("done");
		const before = driver.state.rail.values;
		driver.command({ type: "useInputs", runId: "run-9" });
		driver.command({ type: "undo" });
		expect(driver.state.rail.values).toBe(before);
	});

	test("Undo of a file removal puts the cursor on the field whose file came back", () => {
		const driver = from("done");
		const file = driver.state.rail.values.invoice_file as FileSlot;
		driver.command({
			type: "removeFile",
			name: "invoice_file",
			slotId: file.id,
		});
		driver.command({
			type: "focusHandled",
			seq: driver.state.view.focus?.seq ?? 0,
		});
		expect(driver.state.view.focus).toBeNull();
		driver.command({ type: "undo" });
		expect(fileNames(driver.state.rail.values.invoice_file)).toBe(file.name);
		expect(driver.state.view.focus?.target).toEqual({
			kind: "field",
			key: "invoice_file",
			select: false,
			scrollOnly: false,
		});
	});

	test("Undo of other changes leaves the cursor where it is", () => {
		const driver = from("done");
		driver.command({ type: "useInputs", runId: "run-9" });
		driver.command({
			type: "focusHandled",
			seq: driver.state.view.focus?.seq ?? 0,
		});
		driver.command({ type: "undo" });
		expect(driver.state.view.focus).toBeNull();
	});

	test("a value kept out of storage must be entered again", () => {
		const driver = from("done");
		driver.command({ type: "dontSave", name: "vendor_name" });
		driver.command({ type: "useInputs", runId: "run-9" });
		expect(driver.state.rail.values.vendor_name).toBe("");
		const message = driver.state.view.message?.message;
		expect(message?.kind === "inputsIn" && message.enterAgain).toEqual([
			"Vendor",
		]);
	});

	test("during a series the current file goes back to the front of the next files", () => {
		const driver = from("series");
		const current = driver.state.rail.values.invoice_file as FileSlot;
		driver.command({ type: "useInputs", runId: "run-16" });
		expect(fileNames(driver.state.rail.values.invoice_file)).toBe(
			"invoice-RE-2026-0919.pdf",
		);
		expect(driver.state.rail.nextFiles.invoice_file[0].id).toBe(current.id);
		expect(driver.state.rail.values.invoice_date).toBe("2026-09-21");
	});
});

describe("removing a run, leaving, form changes", () => {
	test("Remove from this device: an ended run goes; a live one stays", () => {
		const driver = from("done");
		driver.command({ type: "pinRun", runId: "run-13" });
		driver.command({ type: "removeRun", runId: "run-13" });
		expect(driver.state.runs.some((run) => run.id === "run-13")).toBe(false);
		expect(driver.state.view.pinnedRunId).toBeNull();
		expect(driver.last).toEqual([{ type: "deleteRun", id: "run-13" }]);
		const running = from("running");
		running.command({ type: "removeRun", runId: "run-14" });
		expect(running.state.runs[0].id).toBe("run-14");
	});

	test("leaving: queued runs do not start, uploads stop, Files go, the series ends", () => {
		const driver = from("series");
		driver.input({ type: "detached" });
		const queued = driver.state.runs.find((run) => run.n === 18);
		expect(queued?.status).toBe("notStarted");
		expect(queued?.outcome).toEqual({
			kind: "notStarted",
			reason: "formClosed",
		});
		expect(driver.state.rail.nextFiles).toEqual({});
		expect(driver.state.memory.prefs.auto).toEqual([]);
		const released = ofType(driver.last, "releaseFiles")[0]?.slotIds ?? [];
		expect(released).toContain("slot-invoice-0923");
		expect(ofType(driver.last, "persistRun")[0]?.record.status).toBe(
			"notStarted",
		);
		expect(ofType(driver.last, "persistPrefs")).toHaveLength(1);
		expect(driver.state.runs.find((run) => run.n === 17)?.status).toBe(
			"running",
		);
	});

	test("a changed form rebases the values by name and keeps the runs", () => {
		const driver = from("done");
		const old = driver.state.form;
		const fields = old.fields
			.filter((field) => field.name !== "max_pages")
			.map((field) =>
				field.name === "expected_total"
					? { ...field, label: "Expected" }
					: field,
			);
		const changed: FormModel = { ...old, fields, contentKey: "changed" };
		driver.input({ type: "formChanged", form: changed });
		expect(driver.state.form).toBe(changed);
		expect("max_pages" in driver.state.rail.values).toBe(false);
		expect(driver.state.rail.values.expected_total).toBe("11769.10");
		expect(driver.state.rail.values.vendor_name).toBe("Nordwind Logistik GmbH");
		expect(driver.state.runs).toHaveLength(14);
	});
});
