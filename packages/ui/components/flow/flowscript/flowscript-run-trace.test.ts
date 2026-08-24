import { describe, expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { parseFlowScriptAnchors } from "./flowscript-anchors";
import {
	type FlowScriptRunLike,
	createCoalescedInvoker,
	deriveRunStatsInlays,
	deriveRunTraceLines,
	runStatsKey,
} from "./flowscript-run-trace";

const FIXTURE_DIR = join(import.meta.dir, "../../../../../tests/ast");

function fixture(name: string): string {
	return readFileSync(join(FIXTURE_DIR, name), "utf8");
}

function run(
	boardId: string,
	nodes: string[],
	alreadyExecuted: string[] = [],
): FlowScriptRunLike {
	return {
		boardId,
		nodes: new Set(nodes),
		already_executed: new Set(alreadyExecuted),
	};
}

const LINES = new Map([
	["nodeA", 3],
	["nodeB", 7],
	["nodeC", 12],
	["nodeD", 20],
]);

describe("FlowScript run trace line derivation", () => {
	test("maps executing and finished node ids to sorted line sets for this board only", () => {
		const trace = deriveRunTraceLines({
			boardId: "board1",
			runs: new Map([
				["run1", run("board1", ["nodeC", "nodeA"], ["nodeB"])],
				["other", run("board2", ["nodeD"], ["nodeD"])],
			]),
			firstLineById: LINES,
		});
		expect(trace.executing).toEqual([3, 12]);
		expect(trace.done).toEqual([7]);
		expect(trace.remote).toEqual([]);
		expect(trace.key).toBe("e:3,12|d:7|r:");
	});

	test("a line stays executing over done; ids without an anchor line are dropped", () => {
		const trace = deriveRunTraceLines({
			boardId: "board1",
			runs: new Map([
				["run1", run("board1", ["nodeA", "ghost"], ["nodeA", "phantom"])],
			]),
			firstLineById: LINES,
		});
		expect(trace.executing).toEqual([3]);
		expect(trace.done).toEqual([]);
	});

	test("remote executions use the peer slot, dedupe across peers and never paint over local lines", () => {
		const trace = deriveRunTraceLines({
			boardId: "board1",
			runs: new Map([["run1", run("board1", ["nodeA"])]]),
			remoteExecutions: [
				{ sub: "alice", executingNodes: ["nodeA", "nodeB"] },
				{ sub: "bob", executingNodes: ["nodeB", "nodeC"] },
				{ sub: "stranger", executingNodes: ["nodeD"] },
			],
			firstLineById: LINES,
			slotFor: (sub) => (sub === "alice" ? 2 : sub === "bob" ? 5 : undefined),
		});
		// nodeA is locally executing — Alice's wash is suppressed there.
		expect(trace.remote).toEqual([
			{ line: 7, slot: 2 },
			{ line: 12, slot: 5 },
			{ line: 20, slot: undefined },
		]);
		expect(trace.key).toBe("e:3|d:|r:7@2,12@5,20@n");
	});

	test("identical states produce identical keys (the decoration write is skipped)", () => {
		const input = {
			boardId: "board1",
			runs: new Map([["run1", run("board1", ["nodeB"], ["nodeA"])]]),
			firstLineById: LINES,
		};
		expect(deriveRunTraceLines(input).key).toBe(deriveRunTraceLines(input).key);
	});
});

describe("FlowScript run stats inlays", () => {
	test("anchored statement lines get visit counts, error counts only when present", () => {
		const anchors = parseFlowScriptAnchors(
			[
				"const a = now()   //@n:nodeA0000000000000000001",
				"const v = 1   //@v:variableanchor000000001",
				"const b = a.toSqlTimestamp()   //@n:nodeB0000000000000000001",
				"const c = never()   //@n:nodeC0000000000000000001",
			].join("\n"),
		);
		const inlays = deriveRunStatsInlays(anchors.anchors, {
			nodeA0000000000000000001: { visits: 12, errors: 0 },
			nodeB0000000000000000001: { visits: 3, errors: 2 },
			variableanchor000000001: { visits: 9, errors: 0 },
		});
		expect(inlays).toEqual([
			{ line: 1, text: "  · 12×" },
			{ line: 3, text: "  · 3× ⚠2" },
		]);
		expect(runStatsKey(inlays)).toBe("1:  · 12×|3:  · 3× ⚠2");
	});

	test("zero-visit heat and un-visited anchors produce no inlay", () => {
		const anchors = parseFlowScriptAnchors(
			"const a = now()   //@n:nodeA0000000000000000001",
		);
		expect(
			deriveRunStatsInlays(anchors.anchors, {
				nodeA0000000000000000001: { visits: 0, errors: 0 },
			}),
		).toEqual([]);
		expect(deriveRunStatsInlays(anchors.anchors, {})).toEqual([]);
	});

	test("maps a real rendered board's heatmap onto its anchor lines", () => {
		const text = fixture("ttwctnp08u18sg2z6nmcqqak.anchored.flow");
		const anchors = parseFlowScriptAnchors(text);
		const inlays = deriveRunStatsInlays(anchors.anchors, {
			liqnumu9en44cq30tu9t5kez: { visits: 4, errors: 1 },
		});
		expect(inlays).toEqual([
			{
				line: anchors.firstLineById.get("liqnumu9en44cq30tu9t5kez") ?? -1,
				text: "  · 4× ⚠1",
			},
		]);
	});
});

describe("FlowScript run store subscription coalescing", () => {
	test("a burst of triggers inside one window costs a single invocation", () => {
		const scheduled: (() => void)[] = [];
		let invocations = 0;
		const invoker = createCoalescedInvoker(
			() => {
				invocations += 1;
			},
			100,
			(callback) => {
				scheduled.push(callback);
				return scheduled.length - 1;
			},
			() => {},
		);
		invoker.trigger();
		invoker.trigger();
		invoker.trigger();
		expect(scheduled).toHaveLength(1);
		scheduled[0]();
		expect(invocations).toBe(1);
		// The window has flushed — the next trigger schedules a fresh pass.
		invoker.trigger();
		expect(scheduled).toHaveLength(2);
		scheduled[1]();
		expect(invocations).toBe(2);
	});

	test("dispose cancels the pending pass and blocks later triggers", () => {
		const scheduled: (() => void)[] = [];
		const cancelled: unknown[] = [];
		let invocations = 0;
		const invoker = createCoalescedInvoker(
			() => {
				invocations += 1;
			},
			100,
			(callback) => {
				scheduled.push(callback);
				return scheduled.length - 1;
			},
			(handle) => {
				cancelled.push(handle);
			},
		);
		invoker.trigger();
		invoker.dispose();
		expect(cancelled).toEqual([0]);
		// A stale timer that still fires after dispose must not invoke.
		scheduled[0]();
		invoker.trigger();
		expect(scheduled).toHaveLength(1);
		expect(invocations).toBe(0);
	});
});
