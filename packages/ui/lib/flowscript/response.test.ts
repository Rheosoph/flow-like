import { describe, expect, test } from "bun:test";
import { applyFlowScriptFromResponse } from "./response";

describe("FlowScript apply responses", () => {
	test("rejects an empty or garbled apply body instead of reading undefined", () => {
		for (const response of [
			undefined,
			"",
			'{"commands":[',
			{ diagnostics: [] },
		]) {
			expect(() => applyFlowScriptFromResponse(response, "board-1")).toThrow(
				"The hub returned no FlowScript apply result for board board-1",
			);
		}
	});

	test("keeps a blocked deletion plan and defaults missing board commands", () => {
		const blocked = {
			commands: [],
			board_commands: [{ RemoveNode: { node_id: "n1" } }],
			diagnostics: ["Apply blocked: deletes 1 node"],
		};
		expect(applyFlowScriptFromResponse(blocked, "board-1")).toEqual(blocked);
		expect(
			applyFlowScriptFromResponse({ commands: [], diagnostics: [] }, "board-1")
				.board_commands,
		).toEqual([]);
	});
});
