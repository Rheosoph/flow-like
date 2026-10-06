import { describe, expect, test } from "bun:test";
import type { RunEntry } from "../contracts";
import { fixture } from "../testing/fixtures";
import {
	MAX_TABS,
	MIN_TABS,
	overflowMenuOf,
	rovingTarget,
	showsPin,
	stripTabsOf,
	tabFit,
	tabTextOf,
	ticksClock,
} from "./strip-model";

const base = fixture("done").runs[0] as RunEntry;

function runs(
	count: number,
	patch: (index: number) => Partial<RunEntry> = () => ({}),
) {
	return Array.from({ length: count }, (_, index) => ({
		...base,
		id: `run-${count - index}`,
		n: count - index,
		...patch(index),
	}));
}

describe("tabFit", () => {
	test("a 960 px stage fits six tabs, as the canvas draws it", () => {
		expect(
			tabFit({ stageWidth: 960, withRunButton: false, withFailure: false }),
		).toBe(6);
	});

	test("a hidden unseen failure reserves 180 px instead of 110 px", () => {
		expect(
			tabFit({ stageWidth: 960, withRunButton: false, withFailure: true }),
		).toBe(5);
	});

	test("the zero-field Run button takes room too", () => {
		expect(
			tabFit({ stageWidth: 960, withRunButton: true, withFailure: false }),
		).toBe(5);
	});

	test("never fewer than two, never more than six", () => {
		expect(
			tabFit({ stageWidth: 100, withRunButton: true, withFailure: true }),
		).toBe(MIN_TABS);
		expect(
			tabFit({ stageWidth: 4000, withRunButton: false, withFailure: false }),
		).toBe(MAX_TABS);
	});
});

describe("stripTabsOf", () => {
	const input = (list: readonly RunEntry[], over = {}) => ({
		runs: list,
		selectedId: null,
		pinnedId: null,
		stageWidth: 960,
		withRunButton: false,
		...over,
	});

	test("shows the newest six and puts the rest behind the overflow", () => {
		const list = runs(14);
		const tabs = stripTabsOf(input(list));
		expect(tabs.visible.map((run) => run.n)).toEqual([14, 13, 12, 11, 10, 9]);
		expect(tabs.overflow).toHaveLength(8);
	});

	test("a run that fits shows no overflow", () => {
		const tabs = stripTabsOf(input(runs(4)));
		expect(tabs.visible).toHaveLength(4);
		expect(tabs.overflow).toHaveLength(0);
	});

	test("the selected and the pinned run take the place of the last tabs", () => {
		const list = runs(14);
		const tabs = stripTabsOf(
			input(list, { selectedId: "run-2", pinnedId: "run-1" }),
		);
		const numbers = tabs.visible.map((run) => run.n);
		expect(numbers).toContain(2);
		expect(numbers).toContain(1);
		expect(numbers.slice(0, 2)).toEqual([14, 13]);
		expect(numbers).toEqual([...numbers].sort((a, b) => b - a));
		expect(tabs.visible).toHaveLength(6);
	});

	test("an unseen failure behind the overflow widens the reserve and lists it first", () => {
		const list = runs(14, (index) =>
			index === 7
				? { status: "failed" as const, unseenFailure: true }
				: { status: "done" as const },
		);
		const tabs = stripTabsOf(input(list));
		expect(tabs.visible).toHaveLength(5);
		expect(tabs.hiddenFailures.map((run) => run.n)).toEqual([7]);
		const menu = overflowMenuOf(tabs);
		expect(menu.failures.map((run) => run.n)).toEqual([7]);
		expect(menu.others.map((run) => run.n)).not.toContain(7);
		expect(menu.others.length + menu.failures.length).toBe(
			tabs.overflow.length,
		);
	});

	test("a failure that would sit on the sixth tab goes behind the overflow, marked", () => {
		const list = runs(14, (index) =>
			index === 5
				? { status: "failed" as const, unseenFailure: true }
				: { status: "done" as const },
		);
		const tabs = stripTabsOf(input(list));
		expect(tabs.visible).toHaveLength(5);
		expect(tabs.hiddenFailures.map((run) => run.n)).toEqual([9]);
	});

	test("a failure that is visible costs no reserve", () => {
		const list = runs(8, (index) =>
			index === 0 ? { status: "failed" as const, unseenFailure: true } : {},
		);
		const tabs = stripTabsOf(input(list));
		expect(tabs.visible).toHaveLength(6);
		expect(tabs.hiddenFailures).toHaveLength(0);
	});
});

describe("tabTextOf", () => {
	const at = (patch: Partial<RunEntry>) => ({ ...base, ...patch });
	const start = 1_000_000;

	test("a live run shows its clock", () => {
		const run = at({ status: "running", startedAt: start, endedAt: null });
		expect(tabTextOf(run, start + 31_000)).toEqual({
			kind: "clock",
			ms: 31_000,
		});
		expect(ticksClock(run)).toBe(true);
	});

	test("a run that asks keeps its clock", () => {
		const run = at({ status: "asking", startedAt: start, endedAt: null });
		expect(tabTextOf(run, start + 5_000)).toEqual({ kind: "clock", ms: 5_000 });
	});

	test("a finished run shows how long it took", () => {
		const run = at({
			status: "done",
			startedAt: start,
			endedAt: start + 48_000,
		});
		expect(tabTextOf(run, start + 90_000)).toEqual({
			kind: "took",
			ms: 48_000,
		});
	});

	test("queued, sending, starting and not started runs show their word", () => {
		for (const status of [
			"queued",
			"sending",
			"starting",
			"notStarted",
		] as const) {
			const run = at({ status, startedAt: null, endedAt: null });
			expect(tabTextOf(run, start)).toEqual({ kind: "word" });
			expect(ticksClock(run)).toBe(false);
		}
	});

	test("a run that failed before it started has no duration", () => {
		const run = at({ status: "failed", startedAt: null, endedAt: start });
		expect(tabTextOf(run, start)).toEqual({ kind: "word" });
	});
});

describe("showsPin", () => {
	const ids = { canPin: true, selectedId: "a", pinnedId: "b" };
	const run = (id: string, status: RunEntry["status"] = "done") => ({
		...base,
		id,
		status,
	});

	test("on the selected and the pinned tab only", () => {
		expect(showsPin(run("a"), ids)).toBe(true);
		expect(showsPin(run("b"), ids)).toBe(true);
		expect(showsPin(run("c"), ids)).toBe(false);
	});

	test("never on queued, sending or not-started tabs", () => {
		for (const status of ["queued", "sending", "notStarted"] as const)
			expect(showsPin(run("a", status), ids)).toBe(false);
	});

	test("never without room to compare", () => {
		expect(showsPin(run("a"), { ...ids, canPin: false })).toBe(false);
	});
});

describe("rovingTarget", () => {
	test("arrows move one tab and stop at the ends", () => {
		expect(rovingTarget("ArrowRight", 1, 4)).toBe(2);
		expect(rovingTarget("ArrowLeft", 1, 4)).toBe(0);
		expect(rovingTarget("ArrowLeft", 0, 4)).toBe(0);
		expect(rovingTarget("ArrowRight", 3, 4)).toBe(3);
	});

	test("Home and End jump", () => {
		expect(rovingTarget("Home", 2, 5)).toBe(0);
		expect(rovingTarget("End", 2, 5)).toBe(4);
	});

	test("other keys and an empty strip do nothing", () => {
		expect(rovingTarget("Enter", 0, 3)).toBeNull();
		expect(rovingTarget("ArrowRight", 0, 0)).toBeNull();
	});
});
