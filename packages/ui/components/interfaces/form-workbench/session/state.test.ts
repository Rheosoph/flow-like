import { describe, expect, test } from "bun:test";
import type { FormSessionState, RunEntry } from "../contracts";
import { fixture } from "../testing/fixtures";
import { outputWith } from "./reduce-driver";
import { newestEnteredRun, sameButStreamedOutput } from "./state";

const streamed = (state: FormSessionState, update: Partial<RunEntry>) => ({
	...state,
	runs: state.runs.map((run, index) =>
		index === 0 ? { ...run, ...update } : run,
	),
});

describe("sameButStreamedOutput", () => {
	const running = fixture("running");

	test("a chunk of a streamed answer changes nothing the rail or the dock show", () => {
		const next = streamed(running, {
			output: outputWith("Vendor: Nordwind", 7),
			summary: { ...running.runs[0].summary, firstLine: "Vendor: Nordwind" },
		});
		expect(next).not.toBe(running);
		expect(sameButStreamedOutput(running, next)).toBe(true);
		expect(sameButStreamedOutput(running, { ...running })).toBe(true);
	});

	test("a status, a run more or less, or any other part of the state counts", () => {
		expect(
			sameButStreamedOutput(running, streamed(running, { status: "asking" })),
		).toBe(false);
		expect(
			sameButStreamedOutput(running, {
				...running,
				runs: running.runs.slice(1),
			}),
		).toBe(false);
		expect(
			sameButStreamedOutput(running, {
				...running,
				view: { ...running.view },
			}),
		).toBe(false);
		expect(
			sameButStreamedOutput(running, {
				...running,
				rail: { ...running.rail },
			}),
		).toBe(false);
	});
});

describe("newestEnteredRun", () => {
	test("skips runs taken out before they began, keeps waiting ones", () => {
		const [newest, ...rest] = fixture("running").runs.filter(
			(run) => run.origin === "session",
		);
		const gone: RunEntry = { ...newest, id: "gone", status: "notStarted" };
		const waiting: RunEntry = { ...newest, id: "waiting", status: "queued" };
		expect(newestEnteredRun({ runs: [gone, newest, ...rest] })?.id).toBe(
			newest.id,
		);
		expect(newestEnteredRun({ runs: [waiting, gone, newest] })?.id).toBe(
			"waiting",
		);
		expect(newestEnteredRun({ runs: [gone] })).toBeNull();
	});
});
