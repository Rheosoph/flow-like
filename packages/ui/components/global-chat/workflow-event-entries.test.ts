import { describe, expect, test } from "bun:test";
import {
	type WorkflowBoardLike,
	collectRunnableWorkflowEventEntries,
	isRunnableWorkflowEventEntry,
	shouldApplyFlowScriptWorkspace,
} from "./workflow-event-entries";

function eventNode(id: string, name: string, connectedTo: string[] = []) {
	return {
		id,
		name,
		friendly_name: `${name} ${id}`,
		pins: {
			[`${id}-exec-out`]: {
				id: `${id}-exec-out`,
				pin_type: "Output",
				data_type: "Execution",
				connected_to: connectedTo,
			},
		},
	};
}

function targetNode(id: string) {
	return {
		id,
		name: "log",
		pins: {
			[`${id}-exec-in`]: {
				id: `${id}-exec-in`,
				pin_type: "Input",
				data_type: "Execution",
				connected_to: [],
			},
		},
	};
}

describe("FlowPilot workflow Event entries", () => {
	test("only queued workspaces can be applied", () => {
		expect(shouldApplyFlowScriptWorkspace("queued")).toBe(true);
		for (const status of [
			undefined,
			"submitted",
			"validation_errors",
			"no_changes",
		]) {
			expect(shouldApplyFlowScriptWorkspace(status)).toBe(false);
		}
	});

	test("returns connected new and reused entries with origin metadata", () => {
		const target = targetNode("target");
		const board: WorkflowBoardLike = {
			nodes: {
				existing: eventNode("existing", "events_simple", ["target-exec-in"]),
				created: eventNode("created", "events_generic", ["target-exec-in"]),
				target,
			},
		};

		const entries = collectRunnableWorkflowEventEntries(
			board,
			"board-1",
			new Set(["existing"]),
			(type) => (type === "events_simple" ? ["cron"] : ["generic_form"]),
		);

		expect(entries).toEqual([
			{
				id: "existing",
				board_id: "board-1",
				name: "events_simple existing",
				node_type: "events_simple",
				supported_event_types: ["cron"],
				created_this_run: false,
			},
			{
				id: "created",
				board_id: "board-1",
				name: "events_generic created",
				node_type: "events_generic",
				supported_event_types: ["generic_form"],
				created_this_run: true,
			},
		]);
	});

	test("excludes empty, stale, data-only, and unsupported entries", () => {
		const dataOnly = eventNode("data", "events_chat");
		dataOnly.pins["data-exec-out"] = {
			id: "data-value",
			pin_type: "Output",
			data_type: "String",
			connected_to: ["target-exec-in"],
		};
		const board: WorkflowBoardLike = {
			nodes: {
				empty: eventNode("empty", "events_simple"),
				stale: eventNode("stale", "events_generic", ["missing-pin"]),
				data: dataOnly,
				unsupported: eventNode("unsupported", "events_mail", [
					"target-exec-in",
				]),
				target: targetNode("target"),
			},
		};

		expect(
			collectRunnableWorkflowEventEntries(
				board,
				"board-1",
				new Set(),
				() => [],
			),
		).toEqual([]);
		expect(isRunnableWorkflowEventEntry(board, "empty")).toBe(false);
		expect(isRunnableWorkflowEventEntry(board, "stale")).toBe(false);
		expect(isRunnableWorkflowEventEntry(board, "target")).toBe(false);
	});

	test("accepts only a connected persisted entry for direct Event registration", () => {
		const board: WorkflowBoardLike = {
			nodes: {
				entry: eventNode("entry", "events_simple", ["target-exec-in"]),
				target: targetNode("target"),
			},
		};

		expect(isRunnableWorkflowEventEntry(board, "entry")).toBe(true);
		expect(isRunnableWorkflowEventEntry(board, "missing")).toBe(false);
	});
});
