import { describe, expect, test } from "bun:test";
import {
	canApplyFlowScript,
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
