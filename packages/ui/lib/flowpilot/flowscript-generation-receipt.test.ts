import { afterEach, describe, expect, test } from "vitest";

import type { BoardEditJobPhase } from "../schema/copilot";
import {
	boardEditJobAppliedCommandCount,
	clearFlowScriptGenerationRuns,
	createFlowScriptGenerationTrace,
	extractFlowScriptCompilerReceipt,
	flowScriptGenerationRunsForConversation,
	isSuccessfulFlowScriptCheckReceipt,
	isSuccessfulFlowScriptCommitReceipt,
	updateFlowScriptGenerationRunReceipt,
} from "./flowscript-generation-receipt";

const source = `function answer() { logInfo({ message: "ok" }) }
eventsSimple run() { answer() }`;

function toolEnd(toolName: string, result: unknown) {
	return {
		tool_name: toolName,
		status: "success",
		result,
	};
}

afterEach(() => clearFlowScriptGenerationRuns());

describe("FlowScript generation compiler receipts", () => {
	test("pairs a tagged authored workspace with the full check_flowscript envelope", () => {
		const payload = {
			status: "valid",
			message: "exact batch retained",
			draft_id: "draft-1",
			revision: 3,
			base_fingerprint: "base-1",
			diagnostics: [],
			review_notes: [{ code: "REVIEW_ONLY" }],
			corrections: ["alias:a->b"],
			derived_command_count: 7,
			queued_count: 0,
			future_compiler_field: { exact: true },
		};
		const result = `<flowscript_workspace>${JSON.stringify({ source, status: "valid" })}</flowscript_workspace>\n<flowscript_draft_result>${JSON.stringify(payload)}</flowscript_draft_result>`;
		const extracted = extractFlowScriptCompilerReceipt(
			toolEnd("check_flowscript", result),
			undefined,
			42,
		);

		expect(extracted.candidates).toEqual([{ source, status: "valid" }]);
		expect(extracted.receipt).toMatchObject({
			toolName: "check_flowscript",
			status: "valid",
			draftId: "draft-1",
			revision: 3,
			baseFingerprint: "base-1",
			source,
			derivedCommandCount: 7,
			queuedCount: 0,
			success: true,
			capturedAtMs: 42,
		});
		expect(extracted.receipt?.payload.future_compiler_field).toEqual({
			exact: true,
		});
		expect(extracted.receipt?.reviewNotes).toEqual([{ code: "REVIEW_ONLY" }]);
		const receipt = extracted.receipt;
		if (!receipt) throw new Error("expected compiler receipt");
		expect(isSuccessfulFlowScriptCheckReceipt(receipt)).toBe(true);
	});

	test("does not call a validation-error check receipt successful", () => {
		const extracted = extractFlowScriptCompilerReceipt(
			toolEnd("provider.check_flowscript", {
				status: "validation_errors",
				draft_id: "draft-2",
				revision: 1,
				source,
				diagnostics: [{ code: "FS_PARSE_ERROR" }],
			}),
		);

		expect(extracted.receipt?.success).toBe(false);
		expect(extracted.receipt?.diagnostics).toEqual([
			{ code: "FS_PARSE_ERROR" },
		]);
	});

	test("captures direct SDK commit receipts and publishes a board-scoped run", () => {
		const trace = createFlowScriptGenerationTrace({
			conversationId: "conversation-1",
			requestId: "request-1:agent",
			parentRequestId: "request-1",
			appId: "app-1",
			boardId: "board-1",
			provider: "codex",
			modelId: "codex:gpt-5.6-terra",
			reasoningEffort: "high",
			startedAtMs: 10,
		});
		trace.recordCandidate(
			{ source, status: "submitted" },
			{ capturedAtMs: 20 },
		);
		trace.recordToolEnd(
			toolEnd("check_flowscript", {
				status: "valid",
				draft_id: "draft-1",
				revision: 2,
				source,
				diagnostics: [],
				derived_command_count: 4,
			}),
			30,
		);
		trace.recordToolEnd(
			toolEnd("commit_flowscript", {
				status: "queued",
				draft_id: "draft-1",
				revision: 2,
				source,
				diagnostics: [],
				derived_command_count: 4,
				queued_count: 4,
			}),
			40,
		);
		const pending = trace.finish({
			outcome: "awaiting_approval",
			finalWorkspaceStatus: "queued",
			appliedCommands: 0,
			persistedReadbackVerified: false,
			endedAtMs: 50,
		});
		expect(pending?.outcome).toBe("awaiting_approval");
		expect(pending?.appliedCommands).toBe(0);
		const run = updateFlowScriptGenerationRunReceipt(
			{
				appId: "app-1",
				boardId: "board-1",
				parentRequestId: "request-1",
			},
			{
				outcome: "ok",
				appliedCommands: 4,
				persistedReadbackVerified: true,
				endedAtMs: 60,
			},
		);

		expect(run?.candidates).toHaveLength(3);
		expect(run?.compilerReceipts).toHaveLength(2);
		expect(
			run?.compilerReceipts.some((receipt) =>
				isSuccessfulFlowScriptCheckReceipt(receipt),
			),
		).toBe(true);
		expect(
			run?.compilerReceipts.some((receipt) =>
				isSuccessfulFlowScriptCommitReceipt(receipt),
			),
		).toBe(true);
		expect(run?.appliedCommands).toBe(4);
		expect(flowScriptGenerationRunsForConversation("conversation-1")).toEqual([
			run,
		]);
	});

	test("counts commands only after the native board job has applied", () => {
		const review = {
			commandCount: 4,
			commandCounts: { AddNode: 4 },
			commandSummaries: ["Add nodes"],
			replacementMode: false,
			destructiveEffects: [],
		};
		const commandPayload = {
			status: "error" as const,
			message: "Apply did not complete.",
			commands: [{ type: "AddNode" } as never],
			board_commands: [],
			diagnostics: ["Apply did not complete."],
		};
		const phases: BoardEditJobPhase[] = [
			"preparing",
			"awaiting_approval",
			"applying",
			"denied",
			"stale",
			"failed",
			"cancelled",
		];

		for (const phase of phases) {
			expect(
				boardEditJobAppliedCommandCount({
					phase,
					review,
					result: commandPayload,
				}),
			).toBe(0);
		}
		for (const phase of [
			"applied_pending_delivery",
			"applied",
		] satisfies BoardEditJobPhase[]) {
			expect(boardEditJobAppliedCommandCount({ phase, review })).toBe(4);
		}
	});
});
