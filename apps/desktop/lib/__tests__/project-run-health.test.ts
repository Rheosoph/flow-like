import type { ILogMetadata } from "@flow-like/flow-like-ui";
import {
	summarize,
	toRuns,
} from "@flow-like/flow-like-ui/components/settings/dashboard/use-project-runs";
import { describe, expect, it } from "vitest";

const MICROS_PER_MS = 1_000;
const HOUR_MS = 60 * 60 * 1000;

function meta(overrides: Partial<ILogMetadata> = {}): ILogMetadata {
	const startMs = Date.now() - HOUR_MS;
	return {
		app_id: "app",
		board_id: "board-a",
		event_id: "event-a",
		node_id: "node-a",
		run_id: `run-${Math.random()}`,
		log_level: 1,
		version: "1",
		payload: [],
		start: startMs * MICROS_PER_MS,
		end: (startMs + 2_000) * MICROS_PER_MS,
		...overrides,
	};
}

const names = new Map([
	["board-a", "Intake router"],
	["board-b", "Term extraction"],
]);

describe("toRuns", () => {
	it("converts microsecond timestamps to millisecond starts and microsecond durations", () => {
		const startMs = Date.now() - HOUR_MS;
		const [run] = toRuns(
			[
				meta({
					start: startMs * MICROS_PER_MS,
					end: (startMs + 1_500) * MICROS_PER_MS,
				}),
			],
			names,
		);

		expect(run.startedAt).toBe(startMs);
		expect(run.durationMicros).toBe(1_500_000);
	});

	it("treats Error and Fatal levels as failures, Warn as a warning", () => {
		const runs = toRuns(
			[
				meta({ log_level: 1 }),
				meta({ log_level: 2 }),
				meta({ log_level: 3 }),
				meta({ log_level: 4 }),
			],
			names,
		);

		expect(runs.filter((run) => run.failed)).toHaveLength(2);
		expect(runs.filter((run) => run.warned)).toHaveLength(1);
	});

	it("names runs from their board and falls back for deleted boards", () => {
		const runs = toRuns(
			[meta({ board_id: "board-b" }), meta({ board_id: "gone" })],
			names,
		);

		expect(runs.map((run) => run.boardName)).toContain("Term extraction");
		expect(runs.map((run) => run.boardName)).toContain("Deleted flow");
	});

	it("sorts newest first", () => {
		const now = Date.now();
		const runs = toRuns(
			[
				meta({ start: (now - 3 * HOUR_MS) * MICROS_PER_MS, run_id: "old" }),
				meta({ start: (now - 1 * HOUR_MS) * MICROS_PER_MS, run_id: "new" }),
			],
			names,
		);

		expect(runs[0].runId).toBe("new");
	});
});

describe("summarize", () => {
	it("reports null rather than a fake 100% when nothing ran", () => {
		const summary = summarize([]);

		expect(summary.windowRuns).toBe(0);
		expect(summary.successRate).toBeNull();
		expect(summary.p95Micros).toBeNull();
		expect(summary.lastRunAt).toBeNull();
		expect(summary.hasEverRun).toBe(false);
		expect(summary.hasEverSucceeded).toBe(false);
	});

	it("computes the success rate over the 24h window only", () => {
		const now = Date.now();
		const summary = summarize(
			toRuns(
				[
					meta({ start: (now - HOUR_MS) * MICROS_PER_MS, log_level: 1 }),
					meta({ start: (now - 2 * HOUR_MS) * MICROS_PER_MS, log_level: 1 }),
					meta({ start: (now - 3 * HOUR_MS) * MICROS_PER_MS, log_level: 3 }),
					// Outside the window — must not move the rate.
					meta({ start: (now - 30 * HOUR_MS) * MICROS_PER_MS, log_level: 3 }),
				],
				names,
			),
		);

		expect(summary.windowRuns).toBe(3);
		expect(summary.windowFailed).toBe(1);
		expect(summary.successRate).toBeCloseTo((2 / 3) * 100, 5);
	});

	it("still reports history that predates the window", () => {
		const now = Date.now();
		const summary = summarize(
			toRuns(
				[meta({ start: (now - 40 * HOUR_MS) * MICROS_PER_MS, log_level: 1 })],
				names,
			),
		);

		expect(summary.windowRuns).toBe(0);
		expect(summary.successRate).toBeNull();
		expect(summary.hasEverRun).toBe(true);
		expect(summary.hasEverSucceeded).toBe(true);
	});

	it("groups failures per board and per surface", () => {
		const now = Date.now();
		const summary = summarize(
			toRuns(
				[
					meta({
						start: (now - HOUR_MS) * MICROS_PER_MS,
						board_id: "board-b",
						event_id: "event-b",
						log_level: 3,
					}),
					meta({
						start: (now - HOUR_MS) * MICROS_PER_MS,
						board_id: "board-b",
						event_id: "event-b",
						log_level: 1,
					}),
					meta({
						start: (now - HOUR_MS) * MICROS_PER_MS,
						board_id: "board-a",
						event_id: "event-a",
						log_level: 1,
					}),
				],
				names,
			),
		);

		expect(summary.byBoard.get("board-b")).toMatchObject({
			total: 2,
			failed: 1,
		});
		expect(summary.byBoard.get("board-a")).toMatchObject({
			total: 1,
			failed: 0,
		});
		expect(summary.byEvent.get("event-b")).toMatchObject({
			total: 2,
			failed: 1,
		});
	});

	it("takes p95 from the slow end of the distribution", () => {
		const now = Date.now();
		const runs = Array.from({ length: 20 }, (_, index) => {
			const startMs = now - HOUR_MS;
			return meta({
				run_id: `run-${index}`,
				start: startMs * MICROS_PER_MS,
				end: (startMs + (index + 1) * 100) * MICROS_PER_MS,
			});
		});

		// Durations are 100ms…2000ms, so p95 is the 19th value: 1900ms.
		expect(summarize(toRuns(runs, names)).p95Micros).toBe(1_900_000);
	});

	it("buckets the window into twelve slots with the newest last", () => {
		const now = Date.now();
		const summary = summarize(
			toRuns([meta({ start: (now - HOUR_MS) * MICROS_PER_MS })], names),
		);

		expect(summary.trend).toHaveLength(12);
		expect(summary.trend.reduce((sum, value) => sum + value, 0)).toBe(1);
		expect(summary.trend[11]).toBe(1);
	});
});
