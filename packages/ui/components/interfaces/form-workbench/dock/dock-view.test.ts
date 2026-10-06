import { describe, expect, test } from "bun:test";
import type {
	FileSlot,
	FormSessionState,
	LeftOutFile,
	RunEntry,
	ShortWords,
} from "../contracts";
import { type FixtureName, fixture } from "../testing/fixtures";
import { PHONE_LAYOUT, layoutFor, withLayout } from "../testing/layouts";
import {
	afterRunOf,
	dockLineOf,
	knownPerRunNames,
	leftOutFieldOf,
	makeOffered,
	matchingRows,
	outputSignOf,
	perRunRowsOf,
	phoneLineOf,
	quickCapOf,
	runLabelKindOf,
	stepKeysOf,
	stopTargetOf,
} from "./dock-view";
import { stepIndex } from "./use-field-stepper";

const WORDS: ShortWords = {
	none: "none",
	empty: "empty",
	on: "On",
	off: "Off",
	files: (count) => `${count} files`,
	entries: (count) => `${count} entries`,
	date: (iso) => iso,
};

const line = (name: FixtureName) => dockLineOf(fixture(name), WORDS);

describe("the status line, highest first", () => {
	test("a first visit counts what is left to fill in", () => {
		expect(line("idle")).toEqual({ kind: "missing", count: 3 });
	});

	test("an invalid press counts the fields that need a look", () => {
		expect(line("invalid")).toEqual({ kind: "problems", count: 2 });
	});

	test("the rail equals the run on the stage: same inputs", () => {
		expect(line("running")).toEqual({ kind: "compared", n: 14, changes: 0 });
	});

	test("a start message wins over the queue line behind it", () => {
		const shown = line("series");
		expect(shown.kind).toBe("message");
	});

	test("the queue line when nothing else speaks: running and queued runs", () => {
		expect(line("queued")).toEqual({ kind: "queue", running: 3, queued: 1 });
	});

	test("an unseen failure behind the overflow", () => {
		const shown = line("series-failed");
		expect(shown.kind).toBe("failure");
		if (shown.kind !== "failure") return;
		expect(shown.runs).toHaveLength(1);
		expect(shown.runs[0].n).toBe(18);
		expect(shown.runs[0].step).toMatchObject({ number: 2, title: "Run OCR" });
	});

	test("the offer is a question until it is answered", () => {
		expect(line("small-offer")).toEqual({
			kind: "question",
			question: { kind: "offer", names: ["order", "receipt"] },
		});
	});

	test("a required file field this page cannot fill is named by its label", () => {
		expect(line("hosted-files")).toEqual({ kind: "blocked", label: "Invoice" });
	});

	test("files of the entry sending come first", () => {
		const shown = line("uploading");
		expect(shown.kind).toBe("sending");
	});

	test("a form without fields never reads a hold", () => {
		const state = fixture("none-done");
		const held: FormSessionState = {
			...state,
			queue: {
				...state.queue,
				hold: { runs: [1, 2], step: { number: 2, title: "Run OCR" } },
			},
		};
		expect(dockLineOf(held, WORDS).kind).not.toBe("hold");
		const withFields = fixture("done");
		const heldWithFields: FormSessionState = {
			...withFields,
			queue: {
				...withFields.queue,
				hold: { runs: [1, 2], step: { number: 2, title: "Run OCR" } },
			},
		};
		expect(dockLineOf(heldWithFields, WORDS).kind).toBe("hold");
	});

	test("an edit since the run on the stage counts; a per-run field is left out of the count", () => {
		const state = fixture("running");
		const edited: FormSessionState = {
			...state,
			rail: {
				...state.rail,
				values: { ...state.rail.values, max_pages: "30" },
			},
		};
		expect(dockLineOf(edited, WORDS)).toEqual({
			kind: "compared",
			n: 14,
			changes: 1,
		});
		const perRun: FormSessionState = {
			...edited,
			memory: {
				...edited.memory,
				prefs: { ...edited.memory.prefs, perRun: ["max_pages"] },
			},
		};
		expect(dockLineOf(perRun, WORDS)).toEqual({
			kind: "compared",
			n: 14,
			changes: 0,
		});
	});
});

describe("Stop", () => {
	const target = (name: FixtureName) => stopTargetOf(fixture(name));

	test("acts on the run on the stage while it is live", () => {
		expect(target("running")?.n).toBe(14);
		expect(target("streaming")?.n).toBe(14);
	});

	test("a queued run on the stage is taken out of the queue by Stop", () => {
		expect(target("queued")?.n).toBe(18);
	});

	test("hidden when the run on the stage has ended or none is on it", () => {
		expect(target("done")).toBeNull();
		expect(target("failed")).toBeNull();
		expect(target("stopped")).toBeNull();
		expect(target("idle")).toBeNull();
		expect(target("uploading")).toBeNull();
	});

	test("a live run that is not on the stage is not stopped by it", () => {
		const state = fixture("series");
		const older = state.runs.find((run) => run.status === "done");
		expect(older).toBeDefined();
		const picked = {
			...state,
			view: { ...state.view, selectedRunId: older?.id ?? null },
		};
		expect(stopTargetOf(picked)).toBeNull();
	});
});

describe("a form without fields at its cap", () => {
	test("three runs going on this device: the Run waits", () => {
		expect(quickCapOf(fixture("none-cap"))).toEqual({ running: 3 });
	});

	test("one run going, or none: no cap", () => {
		expect(quickCapOf(fixture("none"))).toBeNull();
		expect(quickCapOf(fixture("none-done"))).toBeNull();
	});

	test("forms with fields queue instead", () => {
		expect(quickCapOf(fixture("queued"))).toBeNull();
	});

	test("no cap in the cloud for the highest tier", () => {
		const state = fixture("none-cap");
		expect(
			quickCapOf({ ...state, queue: { ...state.queue, cap: -1 } }),
		).toBeNull();
	});

	test("the cloud's cap counts too: two runs on a two-run plan", () => {
		const state = fixture("none-cap");
		const two = {
			...state,
			runs: state.runs.slice(0, 2),
			queue: { ...state.queue, cap: 2 },
		};
		expect(quickCapOf(two)).toEqual({ running: 2 });
	});
});

describe("the Run button's label", () => {
	test("Run until a run has ended, then Run again (strip, phone)", () => {
		expect(runLabelKindOf(fixture("none"), "strip")).toBe("run");
		expect(runLabelKindOf(fixture("none-done"), "strip")).toBe("runAgain");
		expect(runLabelKindOf(fixture("none"), "phone")).toBe("run");
		expect(runLabelKindOf(fixture("none-done"), "phone")).toBe("runAgain");
	});

	test("while the first run still goes the button is just Run", () => {
		expect(runLabelKindOf(fixture("none-cap"), "strip")).toBe("run");
	});

	test("a form with fields keeps Run on the phone, in the rail and in the hero", () => {
		expect(runLabelKindOf(fixture("done"), "phone")).toBe("run");
		expect(runLabelKindOf(fixture("done"), "rail")).toBe("run");
		expect(runLabelKindOf(fixture("none-done"), "hero")).toBe("run");
	});
});

describe("what the phone shows above its bar", () => {
	test("problems, sending and messages; never the counts or Ready", () => {
		const invalid = fixture("invalid");
		expect(phoneLineOf(invalid, line("invalid"))?.kind).toBe("problems");
		expect(phoneLineOf(fixture("idle"), line("idle"))).toBeNull();
		expect(phoneLineOf(fixture("running"), line("running"))).toBeNull();
		expect(
			phoneLineOf(fixture("hosted-files"), line("hosted-files"))?.kind,
		).toBe("blocked");
	});

	test("the refused press of a form at its cap is the sentence", () => {
		const state = fixture("none-cap");
		expect(phoneLineOf(state, dockLineOf(state, WORDS))).toEqual({
			kind: "capped",
			running: 3,
		});
	});

	test("the failure line beats the cap sentence", () => {
		const state = fixture("none-cap");
		const failure = {
			kind: "failure" as const,
			runs: [{ runId: "x", n: 4, step: null }],
		};
		expect(phoneLineOf(state, failure)?.kind).toBe("failure");
	});
});

describe("the Output segment's sign", () => {
	test("a spinner while a run goes on", () => {
		expect(outputSignOf(fixture("running"))).toBe("spinner");
		expect(outputSignOf(fixture("uploading"))).toBe("spinner");
	});

	test("a dot for a question or an unseen result while the Inputs pane shows", () => {
		const asking = fixture("asking");
		expect(
			outputSignOf({ ...asking, view: { ...asking.view, pane: "inputs" } }),
		).toBe("dot");
		const done = fixture("done");
		expect(
			outputSignOf({
				...done,
				view: { ...done.view, pane: "inputs", outputUnseen: true },
			}),
		).toBe("dot");
	});

	test("no sign when the Output pane is open or nothing is waiting", () => {
		expect(outputSignOf(fixture("done"))).toBeNull();
		const asking = fixture("asking");
		expect(
			outputSignOf({ ...asking, view: { ...asking.view, pane: "output" } }),
		).toBeNull();
	});
});

describe("the after-run line", () => {
	test("hidden on a first visit: the setting has not been introduced", () => {
		expect(
			afterRunOf(fixture("idle"), fixture("idle").layout ?? PHONE_LAYOUT).show,
		).toBe(false);
	});

	test("shown once next files are picked: 3 inputs are per run, with the keyboard icon", () => {
		const state = fixture("series");
		const view = afterRunOf(state, state.layout ?? PHONE_LAYOUT);
		expect(view.show).toBe(true);
		expect(view.names).toHaveLength(3);
		expect(view.keyboardIcon).toBe(true);
	});

	test("three fields have no keyboard icon", () => {
		const state = fixture("small-reset");
		const view = afterRunOf(state, state.layout ?? PHONE_LAYOUT);
		expect(view.show).toBe(true);
		expect(view.names).toEqual(["order", "receipt"]);
		expect(view.keyboardIcon).toBe(false);
	});

	test("the offer alone does not introduce it; answering does", () => {
		const state = fixture("small-offer");
		expect(afterRunOf(state, state.layout ?? PHONE_LAYOUT).show).toBe(false);
	});

	test("not on the narrow layout, and not without a fine pointer for the icon", () => {
		const state = fixture("series");
		expect(afterRunOf(withLayout(state, PHONE_LAYOUT), PHONE_LAYOUT).show).toBe(
			false,
		);
		const touch = layoutFor(1360, 836, { touch: true });
		const view = afterRunOf(withLayout(state, touch), touch);
		expect(view.show).toBe(true);
		expect(view.keyboardIcon).toBe(false);
	});

	test("settings of fields the form no longer has are ignored", () => {
		const state = fixture("series");
		const stale = {
			...state,
			memory: {
				...state.memory,
				prefs: { ...state.memory.prefs, perRun: ["gone"], auto: [] },
			},
		};
		expect(knownPerRunNames(stale)).toEqual([]);
	});
});

describe("the rows of the Per run panel", () => {
	test("spec §8.7: Invoice goes to the next file, dates and documents back to empty", () => {
		const state = fixture("after-run");
		const names = knownPerRunNames(state);
		const rows = perRunRowsOf(state, names, WORDS);
		const by = (name: string) => rows.find((row) => row.name === name);
		expect(by("invoice_file")).toMatchObject({
			on: true,
			back: { kind: "nextFile" },
		});
		expect(by("supporting_documents")).toMatchObject({
			on: true,
			back: { kind: "empty" },
		});
		expect(by("invoice_date")).toMatchObject({
			on: true,
			back: { kind: "empty" },
		});
		expect(by("vendor_name")).toMatchObject({
			on: false,
			back: { kind: "empty" },
		});
		expect(by("run_ocr")).toMatchObject({ on: false, back: { kind: "on" } });
		expect(by("max_pages")).toMatchObject({
			on: false,
			back: { kind: "value", text: "20" },
		});
		expect(rows).toHaveLength(state.form.fields.length);
	});

	test("matching by label ignores case and accents", () => {
		const state = fixture("after-run");
		const rows = perRunRowsOf(state, [], WORDS);
		expect(matchingRows(rows, "  DATE ").map((row) => row.label)).toEqual([
			"Invoice date",
		]);
		expect(matchingRows(rows, "")).toHaveLength(rows.length);
		expect(matchingRows(rows, "zzz")).toHaveLength(0);
	});

	test("Make files and dates per run only while nothing is per run", () => {
		const state = fixture("idle");
		expect(makeOffered(state, [])).toBe(true);
		expect(makeOffered(state, ["invoice_file"])).toBe(false);
		expect(makeOffered(fixture("none"), [])).toBe(false);
	});
});

describe("the fields the arrows visit", () => {
	test("after an invalid press: the fields with a message, in form order", () => {
		const state = fixture("invalid");
		expect(stepKeysOf(state, line("invalid"))).toEqual([
			"invoice_file",
			"vendor_name",
		]);
	});

	test("before it: the required fields still empty", () => {
		const state = fixture("idle");
		expect(stepKeysOf(state, line("idle"))).toEqual([
			"invoice_file",
			"vendor_name",
			"invoice_date",
		]);
	});

	test("a field this page cannot fill is no stop", () => {
		const state = fixture("hosted-files");
		const keys = stepKeysOf(state, { kind: "missing", count: 2 });
		expect(keys).not.toContain("invoice_file");
		expect(keys).toContain("vendor_name");
	});

	test("lines without arrows visit nothing", () => {
		expect(stepKeysOf(fixture("running"), line("running"))).toEqual([]);
	});
});

describe("where left-out files wait", () => {
	const slot = (id: string): FileSlot => ({
		id,
		name: `${id}.pdf`,
		size: 1,
		type: "application/pdf",
		state: "sent",
		progress: null,
		ref: null,
		error: null,
		sentAt: null,
		expiresAt: null,
	});
	const left = (id: string): LeftOutFile => ({ slot: slot(id), n: 14 });

	test("the field that holds the files the note is about", () => {
		const state = fixture("next-files");
		const held = {
			...state,
			rail: {
				...state.rail,
				leftOut: { invoice_file: [left("a")], other: [left("b")] },
			},
		};
		expect(leftOutFieldOf(held, [left("a")])).toBe("invoice_file");
		expect(leftOutFieldOf(held, [left("b")])).toBe("other");
		expect(leftOutFieldOf(held, [left("zzz")])).toBeNull();
	});

	test("nothing waits: no field", () => {
		const run: RunEntry | undefined = fixture("series").runs[0];
		expect(run).toBeDefined();
		expect(leftOutFieldOf(fixture("series"), [left("a")])).toBeNull();
	});
});

describe("stepping with the arrows", () => {
	test("the first ▼ lands on the first field and the first ▲ on the last", () => {
		expect(stepIndex(3, null, 1)).toBe(0);
		expect(stepIndex(3, null, -1)).toBe(2);
	});

	test("from a field: the next one, the one before, wrapping both ways", () => {
		expect(stepIndex(3, 0, 1)).toBe(1);
		expect(stepIndex(3, 2, 1)).toBe(0);
		expect(stepIndex(3, 0, -1)).toBe(2);
		expect(stepIndex(3, 1, -1)).toBe(0);
		expect(stepIndex(1, 0, 1)).toBe(0);
		expect(stepIndex(1, 0, -1)).toBe(0);
	});
});
