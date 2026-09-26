import { afterEach, beforeEach, describe, expect, spyOn, test } from "bun:test";
import {
	formatRunTimingReport,
	openRunTimingReports,
	recordNativePreamble,
	recordRunStep,
	resetRunTiming,
	startRunTrace,
	timeRunStep,
} from "./run-timing";

const tick = () => new Promise((resolve) => setTimeout(resolve, 2));

describe("run timing", () => {
	let info: ReturnType<typeof spyOn>;

	beforeEach(() => {
		resetRunTiming();
		info = spyOn(console, "info").mockImplementation(() => {});
	});

	afterEach(() => {
		info.mockRestore();
		resetRunTiming();
	});

	test("records nothing while no trace is open", async () => {
		await timeRunStep("before", async () => tick());
		const trace = startRunTrace("load");
		const report = trace.finish();
		expect(report?.steps).toEqual([]);
	});

	test("nests steps that run inside another step", async () => {
		const trace = startRunTrace("load");
		await timeRunStep("outer", async () => {
			await timeRunStep("inner", async () => tick());
		});
		await timeRunStep("after", async () => tick());
		const report = trace.finish();

		expect(report?.steps.map((step) => [step.name, step.depth])).toEqual([
			["outer", 0],
			["inner", 1],
			["after", 0],
		]);
	});

	describe("with a coarse clock", () => {
		let clock = 1000;
		let now: ReturnType<typeof spyOn>;

		beforeEach(() => {
			clock = 1000;
			now = spyOn(performance, "now").mockImplementation(() => clock);
		});

		afterEach(() => {
			now.mockRestore();
		});

		const depths = (trace: ReturnType<typeof startRunTrace>) =>
			trace.finish()?.steps.map((step) => [step.name, step.depth]);

		test("keeps steps that start in the same tick in the order they started", async () => {
			const trace = startRunTrace("load");
			await timeRunStep("get_event", async () => {
				await timeRunStep("get_event.local", async () => {
					clock = 1010;
				});
				clock = 1040;
			});
			await timeRunStep("is_offline", async () => {});
			await timeRunStep("presign", async () => {
				clock = 1090;
			});

			expect(depths(trace)).toEqual([
				["get_event", 0],
				["get_event.local", 1],
				["is_offline", 0],
				["presign", 0],
			]);
		});

		test("does not nest a step that starts where another ends", async () => {
			const trace = startRunTrace("load");
			await timeRunStep("packages", async () => {
				clock = 1300;
			});
			await timeRunStep("rpa_approval", async () => {});
			await timeRunStep("hub_config", async () => {
				await timeRunStep("hub_config.cached", async () => {});
				clock = 1320;
			});

			expect(depths(trace)).toEqual([
				["packages", 0],
				["rpa_approval", 0],
				["hub_config", 0],
				["hub_config.cached", 1],
			]);
		});
	});

	test("propagates a rejection and still records the step", async () => {
		const trace = startRunTrace("load");
		const failure = new Error("boom");
		await expect(
			timeRunStep("failing", async () => {
				throw failure;
			}),
		).rejects.toBe(failure);
		expect(trace.finish()?.steps.map((step) => step.name)).toEqual(["failing"]);
	});

	test("attaches the native breakdown of the marked run", () => {
		const trace = startRunTrace("load");
		recordNativePreamble("run-1", {
			total_ms: 42,
			steps: [
				{ step: "app_load", ms: 2 },
				{ step: "open_board", ms: 40 },
				{ step: 7, ms: "bad" },
			],
		});
		recordNativePreamble("run-2", "not a preamble");
		trace.mark("run_initiated", "run-1");
		const report = trace.finish();

		expect(report?.native).toEqual({
			totalMs: 42,
			steps: [
				{ step: "app_load", ms: 2 },
				{ step: "open_board", ms: 40 },
			],
		});
		expect(report?.marks.map((mark) => mark.name)).toEqual(["run_initiated"]);
	});

	test("keeps the marked run's native breakdown once later runs evict it", () => {
		const trace = startRunTrace("load");
		recordNativePreamble("load-run", {
			total_ms: 7,
			steps: [{ step: "open_board", ms: 7 }],
		});
		trace.mark("run_initiated", "load-run");
		for (let index = 0; index < 40; index++) {
			recordNativePreamble(`interval-${index}`, { total_ms: 1, steps: [] });
		}

		expect(trace.finish()?.native).toEqual({
			totalMs: 7,
			steps: [{ step: "open_board", ms: 7 }],
		});
	});

	test("finishes once and publishes the report", () => {
		const trace = startRunTrace("load");
		recordRunStep("explicit", 0);
		const report = trace.finish();

		expect(trace.finish()).toBeUndefined();
		expect(info).toHaveBeenCalledTimes(1);
		expect(
			(globalThis as { __flowLikeRunTimings?: unknown[] }).__flowLikeRunTimings,
		).toContain(report);
	});

	test("shows the step an unfinished trace is still waiting on", async () => {
		const trace = startRunTrace("load");
		await timeRunStep("done", async () => tick());
		let release: () => void = () => {};
		const hanging = timeRunStep(
			"hanging",
			() =>
				new Promise<void>((resolve) => {
					release = resolve;
				}),
		);

		const [open] = openRunTimingReports();
		expect(open?.open).toBe(true);
		expect(
			open?.steps.map((step) => [step.name, step.pending ?? false]),
		).toEqual([
			["done", false],
			["hanging", true],
		]);
		expect(open && formatRunTimingReport(open)).toContain("hanging (pending)");

		release();
		await hanging;
		trace.finish();
		expect(openRunTimingReports()).toEqual([]);
	});

	test("formats one line per step with the native summary last", () => {
		const text = formatRunTimingReport({
			label: "page onLoad",
			totalMs: 120,
			marks: [{ name: "run_initiated", atMs: 100 }],
			steps: [
				{ name: "get_board", offsetMs: 0, ms: 80, depth: 0 },
				{ name: "get_board.remote", offsetMs: 5, ms: 60, depth: 1 },
			],
			native: { totalMs: 20, steps: [{ step: "open_board", ms: 18 }] },
		});

		expect(text.split("\n")).toEqual([
			"[run-timing] page onLoad: 120 ms (run_initiated +100 ms)",
			"     +0 ms     80 ms  get_board",
			"     +5 ms     60 ms    get_board.remote",
			"native execute_event 20 ms: open_board 18",
		]);
	});
});
