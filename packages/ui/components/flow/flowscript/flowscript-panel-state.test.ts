import { describe, expect, test } from "bun:test";
import {
	canApplyFlowScript,
	resolveFlowScriptScope,
	shouldReloadFlowScriptAfterApply,
} from "./flowscript-panel-state";

const editableDirtyState = {
	readOnly: false,
	dirty: true,
	applying: false,
	loading: false,
	boardChangedBehindEdits: false,
};

describe("FlowScript apply state", () => {
	test("allows a current dirty draft to be applied", () => {
		expect(canApplyFlowScript(editableDirtyState)).toBe(true);
	});

	test("blocks a dirty draft after the board changes behind it", () => {
		expect(
			canApplyFlowScript({
				...editableDirtyState,
				boardChangedBehindEdits: true,
			}),
		).toBe(false);
	});

	test("also blocks non-actionable editor states", () => {
		for (const override of [
			{ readOnly: true },
			{ dirty: false },
			{ applying: true },
			{ loading: true },
		]) {
			expect(canApplyFlowScript({ ...editableDirtyState, ...override })).toBe(
				false,
			);
		}
	});
});

describe("FlowScript canonical reload", () => {
	test("reloads after a correction-only stale-anchor repair", () => {
		expect(
			shouldReloadFlowScriptAfterApply({
				commandCount: 0,
				correctionCount: 1,
				diagnosticCount: 0,
			}),
		).toBe(true);
	});

	test("keeps the draft when another diagnostic blocks the corrected plan", () => {
		expect(
			shouldReloadFlowScriptAfterApply({
				commandCount: 0,
				correctionCount: 1,
				diagnosticCount: 1,
			}),
		).toBe(false);
	});

	test("does not reload a true no-op", () => {
		expect(
			shouldReloadFlowScriptAfterApply({
				commandCount: 0,
				correctionCount: 0,
				diagnosticCount: 0,
			}),
		).toBe(false);
	});
});

describe("FlowScript editing scope", () => {
	test("a selection with backend support becomes a scoped mode", () => {
		const mode = resolveFlowScriptScope(["node-a", "node-b"], true);
		expect(mode).toEqual({ kind: "scoped", nodeIds: ["node-a", "node-b"] });
	});

	test("copies the requested ids so later selection changes cannot mutate the scope", () => {
		const requested = ["node-a"];
		const mode = resolveFlowScriptScope(requested, true);
		requested.push("node-b");
		expect(mode).toEqual({ kind: "scoped", nodeIds: ["node-a"] });
	});

	test("degrades to the full render without backend support", () => {
		expect(resolveFlowScriptScope(["node-a"], false)).toEqual({
			kind: "full",
		});
	});

	test("no requested nodes means the full render", () => {
		expect(resolveFlowScriptScope(undefined, true)).toEqual({ kind: "full" });
		expect(resolveFlowScriptScope([], true)).toEqual({ kind: "full" });
	});
});

describe("FlowScript apply state with merge conflicts", () => {
	test("unresolved statement-merge conflicts block apply", () => {
		expect(
			canApplyFlowScript({ ...editableDirtyState, unresolvedConflicts: true }),
		).toBe(false);
		expect(
			canApplyFlowScript({ ...editableDirtyState, unresolvedConflicts: false }),
		).toBe(true);
	});
});
