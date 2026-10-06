import { describe, expect, test } from "bun:test";
import type { RunEntry } from "../contracts";
import { linePlaces } from "../model/queue";
import { fixture, fixtureClock } from "../testing/fixtures";
import { barViewOf } from "./bar-model";

function viewOf(
	name: Parameters<typeof fixture>[0],
	pick: (runs: readonly RunEntry[]) => RunEntry,
) {
	const state = fixture(name);
	const run = pick(state.runs);
	return {
		run,
		view: barViewOf(run, {
			now: fixtureClock(name).now,
			places: linePlaces(state.runs),
		}),
	};
}

const newest = (runs: readonly RunEntry[]) => runs[0] as RunEntry;

describe("barViewOf", () => {
	test("running: the status word, the clock and the active step", () => {
		const { view } = viewOf("running", newest);
		expect(view.lead).toEqual({ kind: "status" });
		expect(view.clockMs).toBe(31_000);
		expect(view.detail).toEqual({
			kind: "step",
			step: { number: 4, title: "Match purchase order" },
		});
	});

	test("done: how long it took, no clock, no step", () => {
		const { view } = viewOf("done", newest);
		expect(view.lead).toEqual({ kind: "doneIn", ms: 48_000 });
		expect(view.clockMs).toBeNull();
		expect(view.detail).toBeNull();
	});

	test("failed: the time and the step it failed at", () => {
		const { view } = viewOf("failed", newest);
		expect(view.lead).toEqual({ kind: "failedAfter", ms: 12_000 });
		expect(view.detail).toEqual({
			kind: "step",
			step: { number: 2, title: "Run OCR" },
		});
	});

	test("stopped: the clock it stopped at and the step it was in", () => {
		const { view } = viewOf("stopped", newest);
		expect(view.lead).toEqual({ kind: "stoppedAt", ms: 37_000 });
		expect(view.detail).toEqual({
			kind: "step",
			step: { number: 4, title: "Match purchase order" },
		});
	});

	test("asking: its own word, the clock keeps running", () => {
		const { view } = viewOf("asking", newest);
		expect(view.status).toBe("asking");
		expect(view.clockMs).toBe(39_000);
		expect(view.detail?.kind).toBe("step");
	});

	test("queued: the place in line", () => {
		const { run, view } = viewOf(
			"queued",
			(runs) => runs.find((r) => r.status === "queued") as RunEntry,
		);
		expect(run.status).toBe("queued");
		expect(view.lead).toEqual({ kind: "status" });
		expect(view.detail).toEqual({ kind: "inLine", place: 1 });
		expect(view.clockMs).toBeNull();
	});

	test("a start the API refused waits for a free place", () => {
		const state = fixture("queued");
		const queued = state.runs.find((r) => r.status === "queued") as RunEntry;
		const view = barViewOf(
			{ ...queued, waitingForPlace: true },
			{ now: 0, places: linePlaces(state.runs) },
		);
		expect(view.detail).toEqual({ kind: "waitingForPlace" });
	});

	test("sending: counts the files it waits for", () => {
		const { run, view } = viewOf("uploading", newest);
		expect(run.status).toBe("sending");
		expect(view.detail).toEqual({
			kind: "sendingSoon",
			files: run.pendingSlotIds.length,
		});
	});

	test("not started: the reason", () => {
		const state = fixture("done");
		const run = {
			...newest(state.runs),
			status: "notStarted" as const,
			startedAt: null,
			endedAt: null,
			outcome: { kind: "notStarted" as const, reason: "fileNotSent" as const },
		};
		const view = barViewOf(run, { now: 0, places: {} });
		expect(view.lead).toEqual({ kind: "status" });
		expect(view.detail).toEqual({ kind: "notStarted", reason: "fileNotSent" });
	});

	test("a history run keeps its step in the summary and no clock", () => {
		const state = fixture("done");
		const history = state.runs.find((r) => r.origin === "history") as RunEntry;
		const stopped: RunEntry = {
			...history,
			status: "stopped",
			outcome: { kind: "stopped" },
			summary: {
				...history.summary,
				stepReached: { number: 3, title: "Extract fields" },
			},
		};
		const view = barViewOf(stopped, { now: 0, places: {} });
		expect(view.detail).toEqual({
			kind: "step",
			step: { number: 3, title: "Extract fields" },
		});
		expect(view.clockMs).toBeNull();
	});

	test("a run that failed before it started has only its word", () => {
		const state = fixture("done");
		const run: RunEntry = {
			...newest(state.runs),
			status: "failed",
			startedAt: null,
			endedAt: 5,
			failedAt: null,
			output: null,
		};
		expect(barViewOf(run, { now: 5, places: {} }).lead).toEqual({
			kind: "status",
		});
	});
});
