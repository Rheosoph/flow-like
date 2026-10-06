import { describe, expect, test } from "bun:test";
import { FORM_LIMITS, type RunEntry } from "../contracts";
import { fixture, fixtureClock } from "../testing/fixtures";
import { hasContent, notesOf, queuedWhyOf } from "./notes-model";

const first = (name: Parameters<typeof fixture>[0]) => {
	const state = fixture(name);
	return { state, run: state.runs[0] as RunEntry, now: fixtureClock(name).now };
};

describe("notesOf", () => {
	test("failed: the failure with the step and the run id", () => {
		const { state, run, now } = first("failed");
		const notes = notesOf(run, state, now);
		expect(notes).toHaveLength(1);
		expect(notes[0]).toMatchObject({
			kind: "failure",
			failure: "flow",
			step: { number: 2, title: "Run OCR" },
			runId: run.backendRunId,
			nothingSaved: (run.output?.attachments.length ?? 0) === 0,
		});
	});

	test("failed: no files were saved only while no file came back", () => {
		const { state, run, now } = first("failed");
		const output = run.output;
		if (!output) throw new Error("the failed fixture has no output");
		const without = { ...run, output: { ...output, attachments: [] } };
		expect(notesOf(without, state, now)[0]).toMatchObject({
			nothingSaved: true,
		});
		const file = { name: "report.pdf", url: "https://x/report.pdf" };
		const withFile = {
			...run,
			output: { ...output, attachments: [file] },
		} as unknown as RunEntry;
		expect(notesOf(withFile, state, now)[0]).toMatchObject({
			nothingSaved: false,
		});
		const saved: RunEntry = {
			...run,
			origin: "history",
			output: null,
			summary: { ...run.summary, fileCount: 2 },
		};
		expect(notesOf(saved, state, now)[0]).toMatchObject({
			nothingSaved: false,
		});
	});

	test("stopped: names the step and says what was kept", () => {
		const { state, run, now } = first("stopped");
		expect(notesOf(run, state, now)).toEqual([
			{
				kind: "stopped",
				step: 4,
				hasFields: true,
				kept: false,
				detached: false,
			},
		]);
	});

	test("a hosted page only stopped listening", () => {
		const { state, run, now } = first("stopped");
		const hosted = {
			...state,
			form: {
				...state.form,
				host: { ...state.form.host, stop: "detach" as const },
			},
		};
		expect(notesOf(run, hosted, now)[0]).toMatchObject({ detached: true });
	});

	test("running with steps has no note; without anything it is pending, then slow", () => {
		const running = first("running");
		expect(notesOf(running.run, running.state, running.now)).toEqual([]);
		const bare: RunEntry = {
			...running.run,
			output: running.run.output && { ...running.run.output, steps: [] },
		};
		expect(notesOf(bare, running.state, running.now)).toEqual([
			{ kind: "pending", slow: true },
		]);
		const early = (bare.startedAt ?? 0) + FORM_LIMITS.slowRunNoteMs - 1;
		expect(notesOf(bare, running.state, early)).toEqual([
			{ kind: "pending", slow: false },
		]);
	});

	test("an asking run has no pending note (its question is its own card)", () => {
		const { state, run, now } = first("asking");
		expect(notesOf(run, state, now)).toEqual([]);
	});

	test("done with output: no note", () => {
		const { state, run, now } = first("done");
		expect(notesOf(run, state, now)).toEqual([]);
	});

	test("empty: this run returned nothing", () => {
		const { state, run, now } = first("none-done");
		expect(run.status).toBe("empty");
		expect(notesOf(run, state, now)).toEqual([{ kind: "nothing" }]);
	});

	test("queued: the cap of this device", () => {
		const { state, now } = first("queued");
		const queued = state.runs.find((r) => r.status === "queued") as RunEntry;
		expect(notesOf(queued, state, now)).toEqual([
			{ kind: "queued", why: "device", cap: 3 },
		]);
	});

	test("an older run's receipt: its answer was not kept", () => {
		const { state, now } = first("done");
		const history = state.runs.find(
			(r) =>
				r.origin === "history" && r.status === "done" && r.summary.hasAnswer,
		) as RunEntry;
		const notes = notesOf(history, state, now);
		expect(notes).toEqual([
			{ kind: "receipt", firstLine: history.summary.firstLine },
		]);
	});

	test("an unknown result: a saved run the page closed on, or this session's run whose stream ended", () => {
		const { state, run, now } = first("done");
		const unknown: RunEntry = {
			...run,
			origin: "history",
			status: "unknown",
			outcome: { kind: "unknown" },
			output: null,
		};
		expect(notesOf(unknown, state, now)).toEqual([
			{ kind: "unknown", lost: false },
		]);
		expect(notesOf({ ...unknown, origin: "session" }, state, now)).toEqual([
			{ kind: "unknown", lost: true },
		]);
	});
});

describe("queuedWhyOf", () => {
	const queue = fixture("queued").queue;

	test("the API's refusal comes first, then the hold, then the cap", () => {
		expect(queuedWhyOf({ waitingForPlace: true }, queue)).toBe("noPlace");
		const hold = {
			...queue,
			hold: { runs: [18, 19], step: { number: 2, title: "Run OCR" } },
		};
		expect(queuedWhyOf({ waitingForPlace: false }, hold)).toBe("hold");
		expect(queuedWhyOf({ waitingForPlace: false }, queue)).toBe("device");
		expect(
			queuedWhyOf(
				{ waitingForPlace: false },
				{ ...queue, target: "remote", cap: 2 },
			),
		).toBe("plan");
		expect(queuedWhyOf({ waitingForPlace: false }, { ...queue, cap: -1 })).toBe(
			"soon",
		);
	});
});

describe("hasContent", () => {
	test("steps, an answer, files or a result count", () => {
		const { run } = first("done");
		expect(hasContent(run)).toBe(true);
		expect(hasContent({ ...run, output: null })).toBe(false);
	});
});
