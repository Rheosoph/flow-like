import { describe, expect, test } from "vitest";
import type {
	FlowIrCommitToken,
	UnifiedCopilotResponse,
} from "../schema/copilot";
import {
	BoardSpecialistRoutingFailures,
	BoardSpecialistScopeError,
	assertBoardSpecialistResponse,
	boardSpecialistRoutingFailure,
	compactCopilotBoardReference,
} from "./board-chat-contract";
import legacyBoardFixture from "./fixtures/copilot-board-reference.json";

function response(
	overrides: Partial<UnifiedCopilotResponse> = {},
): UnifiedCopilotResponse {
	return {
		message: "Board inspected",
		active_scope: "Board",
		commands: [],
		components: [],
		suggestions: [],
		...overrides,
	};
}

describe("Board specialist contract", () => {
	test("matches the legacy Board fixture deserialized by the API", () => {
		expect(compactCopilotBoardReference(legacyBoardFixture.id)).toEqual(
			legacyBoardFixture,
		);
	});

	test("accepts a board answer and rejects page drafts even if labeled Board", () => {
		expect(() => assertBoardSpecialistResponse(response())).not.toThrow();
		for (const draft of [
			response({ active_scope: "Frontend" }),
			response({ active_scope: "Both" }),
			response({ components: [{ id: "page-root" }] as never }),
			response({ root_component_id: "page-root" }),
		]) {
			expect(() => assertBoardSpecialistResponse(draft)).toThrow(
				BoardSpecialistScopeError,
			);
		}
	});

	test.each([
		null,
		"Board response",
		17,
		[],
		{ active_scope: "Board", message: "ok", components: {} },
		{ active_scope: "Board", message: "ok", commands: "apply" },
		{ active_scope: "Board", message: "ok", canvas_settings: {} },
		{ active_scope: "Board", message: 17 },
	])("rejects malformed response %j with a routing diagnostic", (value) => {
		expect(() => assertBoardSpecialistResponse(value)).toThrow(
			BoardSpecialistScopeError,
		);
	});

	test("preserves the created board and blocks further routing attempts in that turn", () => {
		const failure = boardSpecialistRoutingFailure(
			new BoardSpecialistScopeError(
				response({ active_scope: "Frontend", message: "Unused page" }),
			),
			"app-compatibility",
			"board-compatibility",
			true,
		);
		expect(failure).toMatchObject({
			status: "error",
			code: "FLOWPILOT_BOARD_SCOPE_MISMATCH",
			app_id: "app-compatibility",
			board_id: "board-compatibility",
			created_board_id: "board-compatibility",
			applied: false,
			retryable: false,
		});
		expect(failure.message).not.toContain("Unused page");
		const failures = new BoardSpecialistRoutingFailures();
		failures.record("turn-1", failure);
		expect(failures.get("turn-1")).toBe(failure);
		expect(failures.get("turn-2")).toBeUndefined();
	});

	test("keeps an unexpected review token available for cleanup without claiming a board was created", () => {
		const token = {
			board_id: "existing-board",
			draft_id: "draft-1",
			revision: 1,
			base_fingerprint: "base-1",
			claim_id: "claim-1",
		} satisfies FlowIrCommitToken;
		const error = new BoardSpecialistScopeError(
			response({
				active_scope: "Frontend",
				flow_ir_commit: token,
			}),
		);
		expect(error.flowIrCommit).toBe(token);
		expect(
			boardSpecialistRoutingFailure(error, "app-1", "existing-board", false),
		).not.toHaveProperty("created_board_id");
	});

	test("does not pass a malformed review token to cleanup", () => {
		const malformed = response({ flow_ir_commit: "not-a-token" as never });
		expect(() => assertBoardSpecialistResponse(malformed)).toThrow(
			BoardSpecialistScopeError,
		);
		expect(
			new BoardSpecialistScopeError(malformed).flowIrCommit,
		).toBeUndefined();
	});
});
