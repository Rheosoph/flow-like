import { describe, expect, test } from "bun:test";
import type { RunEntry, RunStatus, StepRef } from "../contracts";
import {
	type HoldRun,
	type QueueRun,
	capOf,
	holdOf,
	linePlaces,
	startableRunIds,
	unseenFailures,
} from "./queue";

const TIERS = { FREE: 2, PREMIUM: 5, PRO: 20, MAX: 50, ENTERPRISE: -1 };
const OCR: StepRef = { number: 2, title: "Run OCR" };

function queued(n: number, status: RunStatus, extra: Partial<QueueRun> = {}) {
	const run: QueueRun = {
		id: `run-${n}`,
		n,
		status,
		createdAt: n * 1000,
		waitingForPlace: false,
		...extra,
	};
	return run;
}

function ended(
	n: number,
	status: RunStatus,
	failedAt: StepRef | null = null,
	extra: Partial<HoldRun> = {},
) {
	const run: HoldRun = {
		n,
		status,
		failedAt,
		endedAt: n * 1000,
		origin: "session",
		...extra,
	};
	return run;
}

describe("capOf", () => {
	test("runs in the cloud follow the tier: FREE 2, PRO 20, ENTERPRISE no cap", () => {
		expect(capOf("remote", TIERS.FREE)).toBe(2);
		expect(capOf("remote", TIERS.PREMIUM)).toBe(5);
		expect(capOf("remote", TIERS.PRO)).toBe(20);
		expect(capOf("remote", TIERS.MAX)).toBe(50);
		expect(capOf("remote", TIERS.ENTERPRISE)).toBe(-1);
	});

	test("an unknown tier allows 2 in the cloud; an unresolved target counts as the cloud", () => {
		expect(capOf("remote", null)).toBe(2);
		expect(capOf(null, null)).toBe(2);
		expect(capOf(null, TIERS.PRO)).toBe(20);
		expect(capOf("remote", Number.NaN)).toBe(2);
		expect(capOf("remote", 2.7)).toBe(2);
	});

	test("runs on this device go 3 at a time whatever the tier", () => {
		expect(capOf("local", null)).toBe(3);
		expect(capOf("local", TIERS.ENTERPRISE)).toBe(3);
		expect(capOf("local", TIERS.FREE)).toBe(3);
	});
});

describe("startableRunIds", () => {
	test("cap 2 with one run going starts one queued run (spec §9 item 35)", () => {
		const runs = [
			queued(15, "running"),
			queued(16, "queued"),
			queued(17, "queued"),
		];
		expect(startableRunIds(runs, 2, false)).toEqual(["run-16"]);
	});

	test("a held queue starts nothing", () => {
		const runs = [queued(1, "queued"), queued(2, "queued")];
		expect(startableRunIds(runs, 3, true)).toEqual([]);
	});

	test("no cap starts every queued run", () => {
		const runs = [
			queued(1, "running"),
			queued(2, "queued"),
			queued(3, "queued"),
		];
		expect(startableRunIds(runs, -1, false)).toEqual(["run-2", "run-3"]);
	});

	test("sending runs hold no place and are not in line", () => {
		const runs = [
			queued(1, "running"),
			queued(2, "sending"),
			queued(3, "queued"),
		];
		expect(startableRunIds(runs, 2, false)).toEqual(["run-3"]);
	});

	test("starting, asking, running and streaming runs hold a place", () => {
		const busy = [
			queued(1, "starting"),
			queued(2, "asking"),
			queued(3, "running"),
			queued(4, "streaming"),
			queued(5, "queued"),
		];
		expect(startableRunIds(busy, 4, false)).toEqual([]);
		expect(startableRunIds(busy, 5, false)).toEqual(["run-5"]);
	});

	test("a run asking a question never holds the queue", () => {
		const runs = [
			queued(1, "asking"),
			queued(2, "queued"),
			queued(3, "queued"),
		];
		expect(startableRunIds(runs, 3, false)).toEqual(["run-2", "run-3"]);
	});

	test("ended runs hold no place", () => {
		const runs = [
			queued(1, "done"),
			queued(2, "failed"),
			queued(3, "stopped"),
			queued(4, "notStarted"),
			queued(5, "queued"),
		];
		expect(startableRunIds(runs, 1, false)).toEqual(["run-5"]);
	});

	test("first in, first out by press time, whatever order the runs come in", () => {
		const runs = [
			queued(9, "queued"),
			queued(7, "queued"),
			queued(8, "queued"),
		];
		expect(startableRunIds(runs, 2, false)).toEqual(["run-7", "run-8"]);
	});

	test("a run refused for want of a place goes back to the front", () => {
		const runs = [
			queued(1, "queued"),
			queued(2, "queued", { waitingForPlace: true }),
		];
		expect(startableRunIds(runs, 1, false)).toEqual(["run-2"]);
	});

	test("a full cap starts nothing", () => {
		const runs = [
			queued(1, "running"),
			queued(2, "running"),
			queued(3, "queued"),
		];
		expect(startableRunIds(runs, 2, false)).toEqual([]);
	});
});

describe("linePlaces", () => {
	test("queued runs are numbered in the order they will start", () => {
		const runs = [
			queued(23, "queued"),
			queued(22, "queued"),
			queued(21, "running"),
		];
		expect(linePlaces(runs)).toEqual({ "run-22": 1, "run-23": 2 });
	});

	test("sending runs have no place; a refused run is first in line", () => {
		const runs = [
			queued(4, "queued"),
			queued(5, "sending"),
			queued(6, "queued", { waitingForPlace: true }),
		];
		expect(linePlaces(runs)).toEqual({ "run-6": 1, "run-4": 2 });
	});

	test("nothing queued gives no places", () => {
		expect(linePlaces([queued(1, "running")])).toEqual({});
	});
});

describe("holdOf", () => {
	test("runs 18 and 19 failing at step 2 hold the queue", () => {
		const runs = [
			ended(17, "done"),
			ended(18, "failed", OCR),
			ended(19, "failed", OCR),
		];
		expect(holdOf(runs)).toEqual({ runs: [18, 19], step: OCR });
	});

	test("a done run between two failures holds nothing", () => {
		const runs = [
			ended(18, "failed", OCR),
			ended(19, "done"),
			ended(20, "failed", OCR),
		];
		expect(holdOf(runs)).toBeNull();
	});

	test("a stopped run between two failures holds nothing", () => {
		const runs = [
			ended(18, "failed", OCR),
			ended(19, "stopped"),
			ended(20, "failed", OCR),
		];
		expect(holdOf(runs)).toBeNull();
	});

	test("a run taken out of the queue does not break the series", () => {
		const runs = [
			ended(18, "failed", OCR),
			ended(19, "notStarted"),
			ended(20, "failed", OCR),
		];
		expect(holdOf(runs)).toEqual({ runs: [18, 20], step: OCR });
	});

	test("failures at different steps, or without a step, hold nothing", () => {
		const other: StepRef = { number: 3, title: "Extract fields" };
		const renamed: StepRef = { number: 2, title: "Read text" };
		expect(
			holdOf([ended(1, "failed", OCR), ended(2, "failed", other)]),
		).toBeNull();
		expect(
			holdOf([ended(1, "failed", OCR), ended(2, "failed", renamed)]),
		).toBeNull();
		expect(holdOf([ended(1, "failed"), ended(2, "failed")])).toBeNull();
	});

	test("one failure only shows; the hold needs two", () => {
		expect(holdOf([ended(18, "failed", OCR)])).toBeNull();
		expect(holdOf([])).toBeNull();
	});

	test("runs are read in the order they ended, and only this session's", () => {
		const unsorted = [
			ended(19, "failed", OCR, { endedAt: 3000 }),
			ended(17, "done", null, { endedAt: 1000 }),
			ended(18, "failed", OCR, { endedAt: 2000 }),
		];
		expect(holdOf(unsorted)).toEqual({ runs: [18, 19], step: OCR });
		const history = [
			ended(12, "failed", OCR, { origin: "history" }),
			ended(13, "failed", OCR),
		];
		expect(holdOf(history)).toBeNull();
	});

	test("live runs are not part of the series", () => {
		const runs = [
			ended(18, "failed", OCR),
			ended(19, "running", null, { endedAt: null }),
			ended(20, "failed", OCR),
		];
		expect(holdOf(runs)).toEqual({ runs: [18, 20], step: OCR });
	});
});

describe("unseenFailures", () => {
	test("failed runs not picked yet, in the order given", () => {
		const base = { failedAt: null, unseenFailure: false };
		const runs: Pick<
			RunEntry,
			"id" | "n" | "status" | "unseenFailure" | "failedAt"
		>[] = [
			{ ...base, id: "run-21", n: 21, status: "running" },
			{ ...base, id: "run-20", n: 20, status: "failed", unseenFailure: true },
			{
				...base,
				id: "run-18",
				n: 18,
				status: "failed",
				unseenFailure: true,
				failedAt: OCR,
			},
			{ ...base, id: "run-17", n: 17, status: "failed" },
		];
		expect(unseenFailures(runs)).toEqual([
			{ runId: "run-20", n: 20, step: null },
			{ runId: "run-18", n: 18, step: OCR },
		]);
	});
});
