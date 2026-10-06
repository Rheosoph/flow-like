import { beforeAll, describe, expect, test } from "bun:test";
import i18next from "i18next";
import {
	type DockMessage,
	type DockMessageEntry,
	FORM_LIMITS,
	type FileSlot,
	type FormSessionState,
	type ShortWords,
	type StartMessage,
} from "../contracts";
import { shortWordsOf } from "../model/date-text";
import { fakeActions } from "../testing/fake-actions";
import { fixture } from "../testing/fixtures";
import { type InterfacesT, listJoinerOf } from "./copy";
import type { PhoneLine } from "./dock-view";
import { type LineContext, lineViewOf } from "./line-view";

let t: InterfacesT;

beforeAll(async () => {
	const instance = i18next.createInstance();
	await instance.init({
		lng: "en",
		resources: {},
		interpolation: { escapeValue: false },
	});
	t = instance.getFixedT("en", "interfaces") as unknown as InterfacesT;
});

function build(
	line: PhoneLine,
	state: FormSessionState = fixture("idle"),
	words: ShortWords = shortWordsOf(t, "en-GB"),
) {
	const fake = fakeActions();
	const context: LineContext = {
		t,
		copy: {
			mac: true,
			decimalSign: ".",
			warnBytes: FORM_LIMITS.warnFileBytes,
			list: listJoinerOf("en"),
		},
		words,
		actions: fake.actions,
		state,
	};
	return { view: lineViewOf(line, context), fake };
}

const slotOf = (id: string): FileSlot => ({
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

const entry = (message: DockMessage, undo = false): DockMessageEntry => ({
	seq: 1,
	message,
	undo,
	expiresAt: null,
});

describe("sending, problems and the counts", () => {
	test("sending: a spinner in the info tone", () => {
		const { view } = build({ kind: "sending", current: 2, total: 3 });
		expect(view).toMatchObject({
			text: "Sending 2 of 3 files",
			tone: "info",
			icon: "spinner",
			strong: true,
			actions: [],
			steps: null,
		});
		expect(build({ kind: "sending", current: 1, total: 1 }).view.text).toBe(
			"Sending 1 file",
		);
	});

	test("fields that need a look: critical, with the arrows' names", () => {
		const { view } = build({ kind: "problems", count: 2 });
		expect(view).toMatchObject({
			text: "2 fields need a look",
			tone: "critical",
			icon: "alert",
			steps: {
				prev: "Previous field that needs a look",
				next: "Next field that needs a look",
			},
		});
		expect(build({ kind: "problems", count: 1 }).view.text).toBe(
			"1 field needs a look",
		);
		expect(build({ kind: "problems", count: 2 }).view.actions).toEqual([]);
	});

	test("fields that need a look while the queue is on hold keep Resume queue reachable", () => {
		const invalid = fixture("invalid");
		const held: FormSessionState = {
			...invalid,
			queue: {
				...invalid.queue,
				hold: { runs: [18, 19], step: { number: 2, title: "Run OCR" } },
			},
		};
		const { view, fake } = build({ kind: "problems", count: 2 }, held);
		expect(view.text).toBe("2 fields need a look");
		expect(view.steps).not.toBeNull();
		expect(view.actions.map((action) => action.label)).toEqual([
			"Resume queue",
		]);
		view.actions[0].run();
		expect(fake.calls.map((call) => call.name)).toEqual(["resumeQueue"]);
	});

	test("fields to fill in: muted, with their own arrows", () => {
		const { view } = build({ kind: "missing", count: 3 });
		expect(view).toMatchObject({
			text: "3 fields to fill in",
			tone: "muted",
			strong: false,
			steps: {
				prev: "Previous field to fill in",
				next: "Next field to fill in",
			},
		});
		expect(build({ kind: "missing", count: 1 }).view.text).toBe(
			"1 field to fill in",
		);
	});

	test("compared: the 2 px bar and the count, or the plain sentence", () => {
		expect(build({ kind: "compared", n: 17, changes: 1 }).view).toMatchObject({
			text: "1 change since run 17",
			icon: "bar",
			tone: "ink",
		});
		expect(build({ kind: "compared", n: 17, changes: 2 }).view.text).toBe(
			"2 changes since run 17",
		);
		expect(build({ kind: "compared", n: 14, changes: 0 }).view).toMatchObject({
			text: "Same inputs as run 14",
			icon: "none",
			tone: "muted",
			strong: false,
		});
	});

	test("ready and a required file field this page cannot fill", () => {
		expect(build({ kind: "ready" }).view.text).toBe("Ready");
		expect(build({ kind: "blocked", label: "Invoice" }).view).toMatchObject({
			text: "Invoice can't be sent from this page.",
			tone: "critical",
			icon: "alert",
		});
	});

	test("a form without fields at its cap: the sentence", () => {
		expect(build({ kind: "capped", running: 3 }).view).toMatchObject({
			text: "3 runs are going. You can run again when one ends.",
			actions: [],
		});
	});
});

describe("queue, hold and failures", () => {
	test("the queue line: running and queued, with Clear queue", () => {
		const { view, fake } = build({ kind: "queue", running: 3, queued: 1 });
		expect(view).toMatchObject({
			text: "3 running · 1 queued",
			tone: "info",
			icon: "spinner",
		});
		expect(view.actions.map((action) => action.label)).toEqual(["Clear queue"]);
		view.actions[0].run();
		expect(fake.calls.map((call) => call.name)).toEqual(["clearQueue"]);
	});

	test("running alone has nothing to clear", () => {
		const { view } = build({ kind: "queue", running: 2, queued: 0 });
		expect(view.text).toBe("2 running");
		expect(view.actions).toEqual([]);
	});

	test("the hold: Resume and Clear queue", () => {
		const { view, fake } = build({
			kind: "hold",
			hold: { runs: [18, 19], step: { number: 2, title: "Run OCR" } },
		});
		expect(view.text).toBe(
			"Runs 18 and 19 failed at step 2: Run OCR. The queue is on hold.",
		);
		expect(view.tone).toBe("critical");
		expect(view.actions.map((action) => action.label)).toEqual([
			"Resume",
			"Clear queue",
		]);
		for (const action of view.actions) action.run();
		expect(fake.calls.map((call) => call.name)).toEqual([
			"resumeQueue",
			"clearQueue",
		]);
	});

	test("one failure: its step, and Show (named by the run) puts it on the stage", () => {
		const { view, fake } = build({
			kind: "failure",
			runs: [{ runId: "run-18", n: 18, step: { number: 2, title: "Run OCR" } }],
		});
		expect(view.text).toBe("Run 18 failed at step 2: Run OCR.");
		expect(view.actions[0]).toMatchObject({
			label: "Show",
			ariaLabel: "Show run 18",
		});
		view.actions[0].run();
		expect(fake.argsOf("selectRun")).toEqual([["run-18", "show"]]);
	});

	test("several failures: the count, Show goes to the newest", () => {
		const { view, fake } = build({
			kind: "failure",
			runs: [
				{ runId: "run-19", n: 19, step: null },
				{ runId: "run-18", n: 18, step: null },
			],
		});
		expect(view.text).toBe("2 runs failed.");
		expect(view.actions[0].ariaLabel).toBe("Show run 19");
		view.actions[0].run();
		expect(fake.argsOf("selectRun")).toEqual([["run-19", "show"]]);
	});
});

describe("questions", () => {
	test("the offer names the labels of its fields; Yes and No answer", () => {
		const { view, fake } = build(
			{
				kind: "question",
				question: { kind: "offer", names: ["invoice_file", "invoice_date"] },
			},
			fixture("idle"),
		);
		expect(view.text).toBe("Make Invoice and Invoice date per run?");
		expect(view.tone).toBe("ink");
		expect(view.actions.map((action) => action.label)).toEqual(["Yes", "No"]);
		view.actions[0].run();
		view.actions[1].run();
		expect(fake.argsOf("answerQuestion")).toEqual([[true], [false]]);
	});

	test("the series-end question", () => {
		const { view } = build(
			{
				kind: "question",
				question: {
					kind: "seriesEnd",
					names: ["invoice_file", "supporting_documents", "invoice_date"],
				},
			},
			fixture("idle"),
		);
		expect(view.text).toBe(
			"Should Invoice, Supporting documents and Invoice date stay per run?",
		);
	});
});

describe("messages and their buttons", () => {
	const message = (kind: DockMessage, undo: boolean) =>
		build({ kind: "message", entry: entry(kind, undo) });

	test("a start message has no button", () => {
		const { view } = message(
			{
				kind: "start",
				start: {
					n: 15,
					state: "started",
					files: 0,
					pairs: ["a.pdf"],
					lastFile: false,
					leftAsIs: false,
				},
			},
			false,
		);
		expect(view).toMatchObject({
			text: "Run 15 started: a.pdf.",
			tone: "ink",
			strong: true,
			actions: [],
		});
	});

	test("a start message names its run's per-run values in the viewer's words", () => {
		const idle = fixture("idle");
		const [template] = fixture("running").runs;
		const run = {
			...template,
			id: "run-15-a",
			n: 15,
			copy: {
				...template.copy,
				values: {
					...template.copy.values,
					supporting_documents: [slotOf("a"), slotOf("b")],
					run_ocr: true,
				},
				perRun: ["run_ocr", "supporting_documents"],
			},
		};
		const state: FormSessionState = { ...idle, runs: [run, ...idle.runs] };
		const start: StartMessage = {
			n: 15,
			runId: run.id,
			state: "started",
			files: 0,
			pairs: ["2 files", "On"],
			lastFile: false,
			leftAsIs: false,
		};
		const words: ShortWords = {
			...shortWordsOf(t, "de-DE"),
			on: "Ein",
			files: (count) => `${count} Dateien`,
		};
		const line: PhoneLine = {
			kind: "message",
			entry: entry({ kind: "start", start }),
		};
		expect(build(line, state, words).view.text).toBe(
			"Run 15 started: 2 Dateien, Ein.",
		);
		const gone: FormSessionState = { ...state, runs: idle.runs };
		expect(build(line, gone, words).view.text).toBe(
			"Run 15 started: 2 files, On.",
		);
	});

	test("a failed answer reads as a failure, with nothing to press", () => {
		const { view } = message(
			{ kind: "answerFailed", n: 15, runId: "run-15" },
			false,
		);
		expect(view).toMatchObject({
			text: "Your answer to run 15 could not be sent. Try again.",
			tone: "critical",
			icon: "alert",
			strong: true,
			actions: [],
		});
	});

	test("a message with Undo offers it", () => {
		const { view, fake } = message(
			{ kind: "fieldReset", label: "Max pages" },
			true,
		);
		expect(view.text).toBe("Max pages reset.");
		expect(view.actions.map((action) => action.label)).toEqual(["Undo"]);
		view.actions[0].run();
		expect(fake.calls.map((call) => call.name)).toEqual(["undo"]);
	});

	test("left-out files: Add it (one) or Add them (several) puts them back in their field", () => {
		const state = fixture("next-files");
		const files = [
			{ slot: slotOf("a"), n: 14 },
			{ slot: slotOf("b"), n: 14 },
		];
		const held: FormSessionState = {
			...state,
			rail: { ...state.rail, leftOut: { invoice_file: files } },
		};
		const one = build(
			{
				kind: "message",
				entry: entry({ kind: "leftOut", files: files.slice(0, 1) }),
			},
			held,
		);
		expect(one.view.actions.map((action) => action.label)).toEqual(["Add it"]);
		one.view.actions[0].run();
		expect(one.fake.argsOf("addLeftOut")).toEqual([["invoice_file"]]);
		const many = build(
			{ kind: "message", entry: entry({ kind: "leftOut", files }) },
			held,
		);
		expect(many.view.actions.map((action) => action.label)).toEqual([
			"Add them",
		]);
	});

	test("left-out files that are not held any more: the note stays, the button goes", () => {
		const state = fixture("next-files");
		const { view } = build(
			{
				kind: "message",
				entry: entry({
					kind: "leftOut",
					files: [{ slot: slotOf("gone"), n: 3 }],
				}),
			},
			state,
		);
		expect(view.text).toBe(
			"gone.pdf was already sent in run 3 and was left out.",
		);
		expect(view.actions).toEqual([]);
	});
});
