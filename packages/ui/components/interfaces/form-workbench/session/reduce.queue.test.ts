import { describe, expect, test } from "bun:test";
import {
	FORM_LIMITS,
	type RunOutcome,
	type RunOutput,
	type RunStep,
	type SessionEffect,
} from "../contracts";
import { formatClock, formatTook } from "../run/format";
import { elapsedMs } from "../run/run-view";
import { type FixtureName, fixture, fixtureClock } from "../testing/fixtures";
import { distinctEffects } from "./reduce";
import {
	type SessionDriver,
	outputWith,
	pickedFiles,
	sessionDriver,
} from "./reduce-driver";

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

const DONE: RunOutcome = { kind: "succeeded" };
const FAILED: RunOutcome = {
	kind: "failed",
	failure: "flow",
	message: "Array value is not an array",
	detail: null,
};

const step = (
	number: number,
	title: string,
	state: RunStep["state"],
): RunStep => ({
	id: `step-${number}`,
	number,
	title,
	detail: null,
	message: null,
	state,
});

/** A run that got to step 2, Run OCR, when it ended. */
const atRunOcr: RunOutput = {
	...outputWith(""),
	steps: [step(1, "Read documents", "done"), step(2, "Run OCR", "active")],
};

const run = (driver: SessionDriver, n: number) => {
	const found = driver.state.runs.find((entry) => entry.n === n);
	if (!found) throw new Error(`no run ${n}`);
	return found;
};

/** A new entry of the series: its date, then Run. */
function pressSeries(driver: SessionDriver, day: string) {
	driver.command({ type: "setValue", key: "invoice_date", value: day });
	driver
		.advance(2000)
		.command({ type: "run", leaveAsIs: false, from: "chord" });
	return driver.state.runs[0];
}

describe("the queue (spec M5)", () => {
	test("a run that ends frees its place: the first queued run starts and takes the stage", () => {
		const driver = from("series");
		expect(run(driver, 18).status).toBe("queued");
		driver
			.advance(10_000)
			.settle(run(driver, 15).id, DONE, outputWith("Done."));
		expect(run(driver, 15).status).toBe("done");
		expect(run(driver, 18).status).toBe("starting");
		expect(ofType(driver.last, "dispatchRun")).toEqual([
			{ type: "dispatchRun", runId: run(driver, 18).id },
		]);
		expect(driver.state.view.selectedRunId).toBe(run(driver, 18).id);
		expect(driver.state.rail.comparedRunId).toBe(run(driver, 18).id);
		expect(driver.state.announcement).toMatchObject({ kind: "done", n: 15 });
	});

	test("reading an older run keeps it on the stage when the next run starts", () => {
		const driver = from("series");
		driver.command({
			type: "selectRun",
			runId: run(driver, 16).id,
			how: "tab",
		});
		expect(driver.state.view.stageFollowsNewest).toBe(false);
		driver
			.advance(10_000)
			.settle(run(driver, 15).id, DONE, outputWith("Done."));
		expect(run(driver, 18).status).toBe("starting");
		expect(driver.state.view.selectedRunId).toBe(run(driver, 16).id);
	});

	test("a failure the person did not pick stays unseen until a pick or Show", () => {
		const driver = from("series");
		const failed = run(driver, 17);
		driver.advance(12_000).settle(failed.id, FAILED, atRunOcr);
		const ended = run(driver, 17);
		expect(ended.status).toBe("failed");
		expect(ended.failedAt).toEqual({ number: 2, title: "Run OCR" });
		expect(ended.unseenFailure).toBe(true);
		expect(ended.output?.steps[1].state).toBe("failed");
		expect(driver.state.announcement).toMatchObject({
			kind: "failed",
			n: 17,
			step: { number: 2, title: "Run OCR" },
		});
		driver.command({ type: "selectRun", runId: failed.id, how: "show" });
		expect(run(driver, 17).unseenFailure).toBe(false);
	});

	test("a failure of the run the person is reading counts as seen", () => {
		const driver = from("series");
		driver.command({
			type: "selectRun",
			runId: run(driver, 16).id,
			how: "tab",
		});
		driver.advance(5000).settle(run(driver, 16).id, FAILED, atRunOcr);
		expect(run(driver, 16).unseenFailure).toBe(false);
	});

	test("two failures in a row at the same step hold the queue; new presses still queue; Resume", () => {
		const driver = from("series");
		driver.advance(5000).settle(run(driver, 15).id, FAILED, atRunOcr);
		expect(driver.state.queue.hold).toBeNull();
		expect(run(driver, 18).status).toBe("starting");
		driver.advance(5000).settle(run(driver, 16).id, FAILED, atRunOcr);
		expect(driver.state.queue.hold).toEqual({
			runs: [15, 16],
			step: { number: 2, title: "Run OCR" },
		});
		const pressed = pressSeries(driver, "2026-09-23");
		expect(pressed.status).toBe("queued");
		driver.command({ type: "resumeQueue" });
		expect(driver.state.queue.hold).toBeNull();
		expect(run(driver, pressed.n).status).toBe("starting");
	});

	test("a done run between two failures does not hold the queue", () => {
		const driver = from("series");
		driver.advance(5000).settle(run(driver, 15).id, FAILED, atRunOcr);
		driver.advance(5000).settle(run(driver, 16).id, DONE, outputWith("Done."));
		driver.advance(5000).settle(run(driver, 17).id, FAILED, atRunOcr);
		expect(driver.state.queue.hold).toBeNull();
	});

	test("Clear queue: queued runs do not start, next files go, Undo puts the files back", () => {
		const driver = from("series");
		const nextBefore = driver.state.rail.nextFiles.invoice_file;
		driver.command({ type: "clearQueue" });
		expect(run(driver, 18).status).toBe("notStarted");
		expect(run(driver, 18).outcome).toEqual({
			kind: "notStarted",
			reason: "removedFromQueue",
		});
		expect(driver.state.rail.nextFiles.invoice_file).toEqual([]);
		expect(driver.state.view.message?.message).toEqual({
			kind: "queueCleared",
			runs: 1,
			files: 5,
		});
		expect(driver.state.view.message?.undo).toBe(true);
		driver.command({ type: "undo" });
		expect(driver.state.rail.nextFiles.invoice_file).toEqual(nextBefore);
		expect(run(driver, 18).status).toBe("notStarted");
	});

	test("Clear queue on a field holding only a Pick again reminder ends its series", () => {
		const driver = from("series");
		driver.command({ type: "useInputs", runId: "run-14" });
		expect(driver.state.rail.nextFiles.invoice_file).toHaveLength(6);
		driver.command({ type: "clearQueue" });
		expect(driver.state.rail.nextFiles.invoice_file).toBeUndefined();
		expect(driver.state.view.question).toEqual({
			kind: "seriesEnd",
			names: ["invoice_file", "supporting_documents", "invoice_date"],
		});
	});
});

describe("Stop (spec M5)", () => {
	test("a running run is asked to stop once; it ends as stopped and is announced", () => {
		const driver = from("series");
		const id = run(driver, 17).id;
		driver.command({ type: "stop", runId: id });
		expect(ofType(driver.last, "stopRun")).toEqual([
			{ type: "stopRun", runId: id },
		]);
		expect(run(driver, 17).stopRequested).toBe(true);
		driver.command({ type: "stop", runId: id });
		expect(driver.last).toEqual([]);
		driver.advance(1000).settle(id, { kind: "stopped" });
		expect(run(driver, 17).status).toBe("stopped");
		expect(driver.state.announcement).toMatchObject({ kind: "stopped", n: 17 });
	});

	test("a run whose connection ended before a result is announced", () => {
		const driver = from("series");
		const id = run(driver, 17).id;
		driver.advance(1000).settle(id, { kind: "unknown" });
		expect(run(driver, 17).status).toBe("unknown");
		expect(driver.state.announcement).toMatchObject({
			kind: "unknown",
			n: 17,
			step: null,
		});
	});

	test("a stop is announced at the clock the stage shows; a finished run at the duration it reads", () => {
		const announced = (outcome: RunOutcome) => {
			const driver = from("streaming");
			const id = driver.state.runs[0].id;
			driver.advance(650).settle(id, outcome);
			const ended = driver.state.runs[0];
			const announcement = driver.state.announcement;
			if (announcement?.kind !== ended.status)
				throw new Error(`no ${ended.status} announcement`);
			const ms = elapsedMs(ended, 0);
			return { ms, seconds: announcement.seconds };
		};
		const stopped = announced({ kind: "stopped" });
		expect(stopped.ms % 1000).toBeGreaterThanOrEqual(500);
		expect(formatClock(stopped.seconds * 1000)).toBe(formatClock(stopped.ms));
		const done = announced(DONE);
		expect(formatTook(done.seconds * 1000)).toBe(formatTook(done.ms));
		expect(done.seconds).toBe(stopped.seconds + 1);
	});

	test("a queued run is taken out of the queue: Not started, no stop request", () => {
		const driver = from("series");
		driver.command({ type: "stop", runId: run(driver, 18).id });
		expect(run(driver, 18).status).toBe("notStarted");
		expect(ofType(driver.last, "stopRun")).toEqual([]);
		expect(ofType(driver.last, "persistRun")[0].record.status).toBe(
			"notStarted",
		);
	});

	test("Stop on a sending run whose file the form keeps lets the upload go on", () => {
		const driver = from("small");
		driver.command({ type: "setValue", key: "order", value: "1" });
		driver.command({
			type: "pickFiles",
			name: "receipt",
			files: pickedFiles([{ name: "r.jpg", size: 10 }]),
			mode: "replace",
		});
		driver.command({ type: "run", leaveAsIs: false, from: "button" });
		driver.command({ type: "stop", runId: driver.state.runs[0].id });
		expect(driver.state.runs[0].status).toBe("notStarted");
		expect(ofType(driver.last, "abortUpload")).toEqual([]);
		expect(driver.openUploads.map((effect) => effect.slotId)).toEqual([
			"pick-r.jpg",
		]);
	});

	test("Stop on a sending run whose file only it holds aborts that upload", () => {
		const driver = from("small");
		driver.command({ type: "setPerRun", name: "receipt", on: true });
		driver.command({ type: "setValue", key: "order", value: "1" });
		driver.command({
			type: "pickFiles",
			name: "receipt",
			files: pickedFiles([{ name: "r.jpg", size: 10 }]),
			mode: "replace",
		});
		driver.command({ type: "run", leaveAsIs: false, from: "button" });
		expect(driver.state.rail.values.receipt).toBeNull();
		driver.command({ type: "removeFromQueue", runId: driver.state.runs[0].id });
		expect(ofType(driver.last, "abortUpload")).toEqual([
			{ type: "abortUpload", slotId: "pick-r.jpg" },
		]);
		expect(ofType(driver.last, "releaseFiles")).toEqual([]);
	});

	test("Remove from queue sends the cursor to Run; Stop asks for no focus (the dock and ⌘. keep theirs)", () => {
		const driver = from("queued");
		const queued = driver.state.runs.find((run) => run.status === "queued");
		if (!queued) throw new Error("the queued fixture has no queued run");
		driver.command({ type: "removeFromQueue", runId: queued.id });
		expect(driver.state.runs.find((run) => run.id === queued.id)?.status).toBe(
			"notStarted",
		);
		expect(driver.state.view.focus?.target).toEqual({ kind: "run" });

		const live = from("streaming");
		live.command({ type: "stop", runId: live.state.runs[0].id });
		expect(live.state.runs[0].stopRequested).toBe(true);
		expect(live.state.view.focus).toBeNull();
	});

	test("Remove from queue on a run that no longer waits changes nothing", () => {
		const driver = from("streaming");
		const before = driver.state;
		driver.command({ type: "removeFromQueue", runId: before.runs[0].id });
		expect(driver.state).toBe(before);
	});
});

describe("a start refused for want of a place (spec M5)", () => {
	function cloud() {
		const driver = from("small");
		driver.input({ type: "targetResolved", target: "remote", tierLimit: 2 });
		const press = (order: string) => {
			driver.command({ type: "setValue", key: "order", value: order });
			driver.command({
				type: "pickFiles",
				name: "receipt",
				files: pickedFiles([{ name: `${order}.jpg`, size: 10 }]),
				mode: "replace",
			});
			driver.sendUploads();
			driver
				.advance(1000)
				.command({ type: "run", leaveAsIs: false, from: "button" });
			return driver.state.runs[0];
		};
		return { driver, press };
	}

	test("goes back to the front of the queue and is tried again after 15 s", () => {
		const { driver, press } = cloud();
		const first = press("1");
		const second = press("2");
		expect(driver.state.queue.cap).toBe(2);
		driver.input({
			type: "runSettled",
			runId: first.id,
			outcome: { kind: "noPlace" },
		});
		const refused = driver.state.runs.find((entry) => entry.id === first.id);
		expect(refused?.status).toBe("queued");
		expect(refused?.waitingForPlace).toBe(true);
		expect(driver.state.queue.retryAt).toBe(driver.now + FORM_LIMITS.retryMs);
		expect(ofType(driver.last, "scheduleRetry")).toEqual([
			{ type: "scheduleRetry", afterMs: FORM_LIMITS.retryMs },
		]);
		const third = press("3");
		expect(third.status).toBe("queued");
		driver.advance(5000).input({ type: "retryDue" });
		expect(
			driver.state.runs.find((entry) => entry.id === first.id)?.status,
		).toBe("queued");
		driver.advance(FORM_LIMITS.retryMs).input({ type: "retryDue" });
		const retried = driver.state.runs.find((entry) => entry.id === first.id);
		expect(retried?.status).toBe("starting");
		expect(retried?.streamId).toBe(`${first.id}#2`);
		expect(
			driver.state.runs.find((entry) => entry.id === third.id)?.status,
		).toBe("queued");
		expect(second.status).toBe("starting");
	});

	test("is tried again as soon as a run of this window ends", () => {
		const { driver, press } = cloud();
		const first = press("1");
		const second = press("2");
		driver.input({
			type: "runSettled",
			runId: first.id,
			outcome: { kind: "noPlace" },
		});
		driver.advance(3000).settle(second.id, DONE, outputWith("ok"));
		expect(driver.state.queue.retryAt).toBeNull();
		expect(
			driver.state.runs.find((entry) => entry.id === first.id)?.status,
		).toBe("starting");
	});
});

describe("what the runtime reports", () => {
	test("accepted: running with its clock; answer text: streaming; a pending question: asking", () => {
		const driver = from("small");
		driver.command({ type: "setValue", key: "order", value: "1" });
		driver.command({
			type: "pickFiles",
			name: "receipt",
			files: pickedFiles([{ name: "r.jpg", size: 10 }]),
			mode: "replace",
		});
		driver
			.sendUploads()
			.command({ type: "run", leaveAsIs: false, from: "button" });
		const id = driver.state.runs[0].id;
		driver.advance(500).acceptRuns();
		expect(driver.state.runs[0].status).toBe("running");
		expect(driver.state.runs[0].startedAt).toBe(driver.now);
		expect(driver.state.runs[0].backendRunId).toBe(`backend-${id}`);
		driver.input({
			type: "runOutput",
			runId: id,
			output: outputWith("## Return opened"),
		});
		expect(driver.state.runs[0].status).toBe("streaming");
		expect(driver.state.runs[0].summary.firstLine).toBe("Return opened");
		const question: RunOutput = {
			...outputWith("## Return opened"),
			interactions: [
				{
					id: "q1",
					name: "Which parcel?",
					description: "",
					interaction_type: { type: "single_choice", options: [] },
					status: "pending",
					ttl_seconds: 900,
					expires_at: 0,
				},
			],
		} as RunOutput;
		driver.input({ type: "runOutput", runId: id, output: question });
		expect(driver.state.runs[0].status).toBe("asking");
		driver.command({
			type: "respondInteraction",
			runId: id,
			interactionId: "q1",
			value: "a",
		});
		expect(driver.last).toEqual([
			{
				type: "respondInteraction",
				runId: id,
				interactionId: "q1",
				value: "a",
			},
		]);
	});

	test("the tier decides the cloud cap; a device runs 3 at a time", () => {
		const driver = from("small");
		driver.input({ type: "targetResolved", target: "remote", tierLimit: -1 });
		expect(driver.state.queue.cap).toBe(-1);
		driver.input({ type: "targetResolved", target: "remote", tierLimit: 20 });
		expect(driver.state.queue.cap).toBe(20);
		driver.input({ type: "targetResolved", target: "local", tierLimit: 20 });
		expect(driver.state.queue.cap).toBe(3);
	});

	test("no message asks for the same effect twice", () => {
		const driver = from("series");
		driver.advance(5000).settle(run(driver, 15).id, FAILED, atRunOcr);
		pressSeries(driver, "2026-09-23");
		driver.command({ type: "clearQueue" });
		driver.command({ type: "undo" });
		driver.input({ type: "detached" });
		for (const effects of driver.steps)
			expect(distinctEffects(effects)).toHaveLength(effects.length);
	});
});
