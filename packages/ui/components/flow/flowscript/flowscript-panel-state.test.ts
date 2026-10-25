import { describe, expect, test } from "bun:test";
import {
	canApplyFlowScript,
	flowScriptApplyFailureReport,
	flowScriptApplyReport,
	isRejectedFlowScriptDraft,
	resolveFlowScriptScope,
	resolveJoinableScopeNodeIds,
	sameScopeNodeIds,
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
			{ rejectedDraft: true },
		]) {
			expect(canApplyFlowScript({ ...editableDirtyState, ...override })).toBe(
				false,
			);
		}
	});
});

describe("FlowScript apply report", () => {
	const draft = { text: "edited", baseline: "rendered" };

	test("a fetch failure stays visible without rejecting the draft or disabling retry", () => {
		const report = flowScriptApplyFailureReport(
			new TypeError("Failed to fetch"),
			"Unknown apply error",
		);
		expect(report.failed).toBe(true);
		expect(report.diagnostics).toEqual(["Failed to fetch"]);
		expect(report.rejected).toBeUndefined();
		expect(
			canApplyFlowScript({
				...editableDirtyState,
				rejectedDraft: isRejectedFlowScriptDraft(report, draft),
			}),
		).toBe(true);
	});

	test("failure reports preserve backend messages and handle missing details", () => {
		expect(
			flowScriptApplyFailureReport(
				{ message: "Gateway timeout" },
				"Unknown apply error",
			).diagnostics,
		).toEqual(["Gateway timeout"]);
		expect(
			flowScriptApplyFailureReport(undefined, "Unknown apply error")
				.diagnostics,
		).toEqual(["Unknown apply error"]);
	});

	test("a clean apply reports nothing", () => {
		expect(
			flowScriptApplyReport({ commandCount: 3, diagnostics: [] }, draft),
		).toBeUndefined();
	});

	test("diagnostics with no executed command reject the draft", () => {
		const report = flowScriptApplyReport(
			{ commandCount: 0, diagnostics: ["removing `A` leaves `B` unrun"] },
			draft,
		);
		expect(report?.rejected).toEqual(draft);
		expect(isRejectedFlowScriptDraft(report, draft)).toBe(true);
	});

	test("diagnostics alongside executed commands stay warnings", () => {
		const report = flowScriptApplyReport(
			{ commandCount: 2, diagnostics: ["advisory"] },
			draft,
		);
		expect(report?.diagnostics).toEqual(["advisory"]);
		expect(report?.rejected).toBeUndefined();
		expect(isRejectedFlowScriptDraft(report, draft)).toBe(false);
	});

	test("editing the draft or refreshing the baseline lifts the rejection", () => {
		const report = flowScriptApplyReport(
			{ commandCount: 0, diagnostics: ["blocked"] },
			draft,
		);
		expect(isRejectedFlowScriptDraft(report, { ...draft, text: "fixed" })).toBe(
			false,
		);
		expect(
			isRejectedFlowScriptDraft(report, { ...draft, baseline: "newer" }),
		).toBe(false);
		expect(isRejectedFlowScriptDraft(undefined, draft)).toBe(false);
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

describe("FlowScript file scope", () => {
	const files = {
		hasModules: true,
		backendSupportsFiles: true,
		file: "module-a",
	};

	test("edits the canvas's file on a modular board", () => {
		expect(resolveFlowScriptScope(undefined, true, files)).toEqual({
			kind: "file",
			file: "module-a",
		});
		expect(
			resolveFlowScriptScope(undefined, true, { ...files, file: "main" }),
		).toEqual({ kind: "file", file: "main" });
	});

	test("a board without modules keeps the whole-board render", () => {
		expect(
			resolveFlowScriptScope(undefined, true, { ...files, hasModules: false }),
		).toEqual({ kind: "full" });
	});

	test("degrades to the full render when the backend cannot render files", () => {
		expect(
			resolveFlowScriptScope(undefined, true, {
				...files,
				backendSupportsFiles: false,
			}),
		).toEqual({ kind: "full" });
	});

	test("a selection scope overrides the file while it is active", () => {
		expect(resolveFlowScriptScope(["node-a"], true, files)).toEqual({
			kind: "scoped",
			nodeIds: ["node-a"],
		});
		// Exiting the selection scope drops back to the file the canvas is on.
		expect(resolveFlowScriptScope(undefined, true, files)).toEqual({
			kind: "file",
			file: "module-a",
		});
	});

	test("a selection the backend cannot scope still lands on the file", () => {
		expect(resolveFlowScriptScope(["node-a"], false, files)).toEqual({
			kind: "file",
			file: "module-a",
		});
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

describe("shared scope set equality", () => {
	test("matches the same node set regardless of order and duplicates", () => {
		expect(sameScopeNodeIds(["a", "b"], ["b", "a"])).toBe(true);
		expect(sameScopeNodeIds(["a", "a", "b"], ["b", "a"])).toBe(true);
		expect(sameScopeNodeIds([], [])).toBe(true);
	});

	test("rejects subsets, supersets and disjoint sets", () => {
		expect(sameScopeNodeIds(["a"], ["a", "b"])).toBe(false);
		expect(sameScopeNodeIds(["a", "b"], ["a"])).toBe(false);
		expect(sameScopeNodeIds(["a"], ["c"])).toBe(false);
		expect(sameScopeNodeIds([], ["a"])).toBe(false);
	});
});

describe("joining a peer's shared scope", () => {
	const known = new Set(["node-a", "node-b"]);
	const isKnown = (id: string) => known.has(id);

	test("keeps every id the local board knows", () => {
		expect(resolveJoinableScopeNodeIds(["node-a", "node-b"], isKnown)).toEqual([
			"node-a",
			"node-b",
		]);
	});

	test("drops unknown ids but joins the surviving partial scope", () => {
		expect(
			resolveJoinableScopeNodeIds(["node-a", "node-gone"], isKnown),
		).toEqual(["node-a"]);
	});

	test("an all-unknown scope yields nothing to join", () => {
		expect(resolveJoinableScopeNodeIds(["x", "y"], isKnown)).toEqual([]);
		expect(resolveJoinableScopeNodeIds([], isKnown)).toEqual([]);
	});

	test("dedupes and returns an independent copy of the peer's ids", () => {
		const requested = ["node-a", "node-a", "node-b"];
		const joined = resolveJoinableScopeNodeIds(requested, isKnown);
		expect(joined).toEqual(["node-a", "node-b"]);
		// Mutating the peer's array after joining must not reach the local scope.
		requested.length = 0;
		expect(joined).toEqual(["node-a", "node-b"]);
	});
});
