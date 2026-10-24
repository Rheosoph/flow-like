import { describe, expect, test } from "bun:test";
import type { BoardCommand } from "../../../lib/schema/flow/copilot";
import {
	destructiveCommandSummaries,
	groupBoardCommands,
	summarizeApplyImpact,
	summarizeBoardCommands,
} from "./flowscript-apply-preview";

function command(partial: Record<string, unknown>): BoardCommand {
	return partial as unknown as BoardCommand;
}

const PLAN: BoardCommand[] = [
	command({
		command_type: "AddNode",
		node_type: "log_info",
		summary: "Add Log Info",
	}),
	command({ command_type: "AddNode", node_type: "control_branch" }),
	command({ command_type: "AddPlaceholder", name: "todo" }),
	command({
		command_type: "ConnectPins",
		from_node: "a",
		from_pin: "exec_out",
		to_node: "b",
		to_pin: "exec_in",
		summary: "Wire a → b",
	}),
	command({
		command_type: "RemoveNode",
		node_id: "dead1",
		summary: "Remove Old Logger",
	}),
	command({
		command_type: "DeleteVariable",
		variable_id: "var1",
		summary: "Remove variable retries",
	}),
	command({ command_type: "CreateVariable", name: "retries" }),
	command({
		command_type: "UpdateNodePin",
		node_id: "n",
		pin_id: "p",
		value: 1,
	}),
	// A Rust-side variant the hand-mirrored TS union does not know yet.
	command({ command_type: "RemoveLayer", layer_id: "layer1" }),
	// A hypothetical future variant must not break the preview.
	command({ command_type: "SomethingNew", summary: "??" }),
];

describe("FlowScript apply preview summary", () => {
	test("counts commands by category", () => {
		const counts = summarizeBoardCommands(PLAN);
		expect(counts).toEqual({
			addedNodes: 3,
			removedNodes: 1,
			wires: 1,
			removedWires: 0,
			variables: 2,
			layers: 1,
			comments: 0,
			updates: 2,
			total: 10,
		});
	});

	test("an empty plan is all zeros", () => {
		expect(summarizeBoardCommands([]).total).toBe(0);
	});

	test("description-only edits remain visible updates, including clearing", () => {
		const commands: BoardCommand[] = [
			{
				command_type: "SetNodeDescription",
				node_id: "tool",
				description: "Look up an order.",
				summary: "Describe findOrder",
			},
			{
				command_type: "SetNodeDescription",
				node_id: "event",
				description: "",
			},
		];
		expect(summarizeBoardCommands(commands)).toMatchObject({
			updates: 2,
			total: 2,
			removedNodes: 0,
		});
		expect(summarizeApplyImpact(commands)).toEqual({
			deletions: [],
			moves: [],
		});
		expect(groupBoardCommands(commands)).toEqual([
			{
				type: "SetNodeDescription",
				items: [
					{
						key: "Describe findOrder#1",
						label: "Describe findOrder",
						entityId: "tool",
					},
					{ key: "event#1", label: "event", entityId: "event" },
				],
			},
		]);
	});

	test("recognizes both serialized and Rust deletion names", () => {
		const commands = [
			command({ command_type: "DeleteVariable", variable_id: "v1" }),
			command({ command_type: "RemoveVariable", variable_id: "v2" }),
			command({ command_type: "DeleteComment", comment_id: "c1" }),
			command({ command_type: "RemoveComment", comment_id: "c2" }),
		];
		expect(summarizeBoardCommands(commands)).toMatchObject({
			variables: 2,
			comments: 2,
			updates: 0,
		});
		expect(destructiveCommandSummaries(commands)).toEqual([
			{ kind: "variable", label: "v1" },
			{ kind: "variable", label: "v2" },
			{ kind: "comment", label: "c1" },
			{ kind: "comment", label: "c2" },
		]);
	});
});

describe("FlowScript apply impact", () => {
	test("separates deleted items from items moved to another layer or the root", () => {
		const impact = summarizeApplyImpact([
			command({
				command_type: "RemoveNode",
				node_id: "old-node",
				summary: "Remove Old Logger",
			}),
			command({
				command_type: "MoveToLayer",
				ids: ["moved-node", "moved-comment"],
				target_layer: "module-1",
				summary: "Move items into reporting",
			}),
			command({
				command_type: "MoveToLayer",
				ids: ["moved-layer"],
				target_layer: null,
			}),
		]);
		expect(impact.deletions).toEqual([
			{
				key: "node:old-node",
				kind: "node",
				id: "old-node",
				label: "Remove Old Logger",
			},
		]);
		expect(impact.moves).toEqual([
			{
				key: "move:moved-node",
				id: "moved-node",
				label: "Move items into reporting",
				targetLayer: "module-1",
			},
			{
				key: "move:moved-comment",
				id: "moved-comment",
				label: "Move items into reporting",
				targetLayer: "module-1",
			},
			{
				key: "move:moved-layer",
				id: "moved-layer",
				label: "moved-layer",
				targetLayer: null,
			},
		]);
	});

	test("a deletion-only plan includes all entity kinds and deduplicates aliases", () => {
		const impact = summarizeApplyImpact([
			command({ command_type: "RemoveNode", node_id: "node-1" }),
			command({ command_type: "DeleteVariable", variable_id: "variable-1" }),
			command({ command_type: "RemoveVariable", variable_id: "variable-1" }),
			command({ command_type: "RemoveLayer", layer_id: "layer-1" }),
			command({ command_type: "DeleteComment", comment_id: "comment-1" }),
			command({ command_type: "RemoveComment", comment_id: "comment-1" }),
		]);
		expect(impact.deletions.map(({ kind, id }) => ({ kind, id }))).toEqual([
			{ kind: "node", id: "node-1" },
			{ kind: "variable", id: "variable-1" },
			{ kind: "layer", id: "layer-1" },
			{ kind: "comment", id: "comment-1" },
		]);
		expect(impact.moves).toEqual([]);
	});

	test("a move-only plan counts each item once at its final destination", () => {
		const impact = summarizeApplyImpact([
			command({
				command_type: "MoveToLayer",
				ids: ["node-1", "node-1", "node-2"],
				target_layer: "module-1",
			}),
			command({
				command_type: "MoveToLayer",
				ids: ["node-1"],
				summary: "Move Logger to board root",
			}),
		]);
		expect(impact.deletions).toEqual([]);
		expect(impact.moves).toEqual([
			{
				key: "move:node-1",
				id: "node-1",
				label: "Move Logger to board root",
				targetLayer: undefined,
			},
			{
				key: "move:node-2",
				id: "node-2",
				label: "node-2",
				targetLayer: "module-1",
			},
		]);
	});

	test("items scheduled for deletion are excluded from moves in any command order", () => {
		const impact = summarizeApplyImpact([
			command({ command_type: "RemoveLayer", layer_id: "layer-1" }),
			command({
				command_type: "MoveToLayer",
				ids: ["node-1", "layer-1", "kept-node"],
			}),
			command({ command_type: "RemoveNode", node_id: "node-1" }),
		]);
		expect(impact.deletions).toHaveLength(2);
		expect(impact.moves.map(({ id }) => id)).toEqual(["kept-node"]);
	});

	test("same-named items retain distinct IDs and stable keys", () => {
		const commands = [
			command({
				command_type: "RemoveNode",
				node_id: "node-1",
				summary: "Remove Logger",
			}),
			command({
				command_type: "RemoveNode",
				node_id: "node-2",
				summary: "Remove Logger",
			}),
			command({
				command_type: "MoveToLayer",
				ids: ["node-3", "node-4"],
				summary: "Move Logger",
			}),
		];
		const impact = summarizeApplyImpact(commands);
		expect(impact.deletions.map(({ id }) => id)).toEqual(["node-1", "node-2"]);
		expect(impact.moves.map(({ id }) => id)).toEqual(["node-3", "node-4"]);
		expect(new Set(impact.deletions.map(({ key }) => key)).size).toBe(2);
		expect(new Set(impact.moves.map(({ key }) => key)).size).toBe(2);
		expect(summarizeApplyImpact(commands)).toEqual(impact);
	});

	test("retains complete deletion and move lists for a large board", () => {
		const deletedIds = Array.from({ length: 20 }, (_, index) => `old-${index}`);
		const movedIds = Array.from(
			{ length: 1658 },
			(_, index) => `kept-${index}`,
		);
		const impact = summarizeApplyImpact([
			...deletedIds.map((id) =>
				command({ command_type: "RemoveNode", node_id: id }),
			),
			command({ command_type: "MoveToLayer", ids: movedIds }),
		]);
		expect(impact.deletions.map(({ id }) => id)).toEqual(deletedIds);
		expect(impact.moves.map(({ id }) => id)).toEqual(movedIds);
		expect(impact.deletions.length + impact.moves.length).toBe(1678);
	});

	test("additions and canvas position changes have no deletion or relocation impact", () => {
		expect(
			summarizeApplyImpact([
				command({ command_type: "AddNode", node_type: "log_info" }),
				command({ command_type: "CreateLayer", name: "Group" }),
				command({ command_type: "ConnectPins" }),
				command({ command_type: "MoveNode", node_id: "node-1" }),
				command({ command_type: "MoveToLayer", ids: [] }),
			]),
		).toEqual({ deletions: [], moves: [] });
	});
});

describe("FlowScript destructive command summaries", () => {
	test("collects exactly the delete-family commands with their labels", () => {
		const destructive = destructiveCommandSummaries(PLAN);
		expect(destructive).toEqual([
			{ kind: "node", label: "Remove Old Logger" },
			{ kind: "variable", label: "Remove variable retries" },
			{ kind: "layer", label: "layer1" },
		]);
	});

	test("falls back to ids when the compiler attached no summary", () => {
		const destructive = destructiveCommandSummaries([
			command({ command_type: "RemoveNode", node_id: "nodeid123" }),
			command({ command_type: "DeleteComment", comment_id: "commentid1" }),
		]);
		expect(destructive.map((entry) => entry.label)).toEqual([
			"nodeid123",
			"commentid1",
		]);
	});

	test("a purely additive plan has no destructive entries", () => {
		expect(
			destructiveCommandSummaries([
				command({ command_type: "AddNode", node_type: "log_info" }),
				command({ command_type: "ConnectPins" }),
			]),
		).toEqual([]);
	});
});

describe("FlowScript command grouping", () => {
	test("groups by command type in first-seen order with summary labels", () => {
		const groups = groupBoardCommands(PLAN);
		expect(groups.map((group) => group.type)).toEqual([
			"AddNode",
			"AddPlaceholder",
			"ConnectPins",
			"RemoveNode",
			"DeleteVariable",
			"CreateVariable",
			"UpdateNodePin",
			"RemoveLayer",
			"SomethingNew",
		]);
		const addNodes = groups[0];
		expect(addNodes.items.map((item) => item.label)).toEqual([
			"Add Log Info",
			"control_branch",
		]);
		expect(groups[1].items.map((item) => item.label)).toEqual(["todo"]);
	});

	test("duplicate labels within a group get distinct keys", () => {
		const groups = groupBoardCommands([
			command({ command_type: "AddNode", summary: "Add Log Info" }),
			command({ command_type: "AddNode", summary: "Add Log Info" }),
		]);
		const keys = groups[0].items.map((item) => item.key);
		expect(new Set(keys).size).toBe(2);
	});
});

describe("remote-touched preview marking", () => {
	test("items expose the entity id their command targets", () => {
		const groups = groupBoardCommands([
			command({ command_type: "UpdateNode", node_id: "node1", summary: "n" }),
			command({
				command_type: "DeleteVariable",
				variable_id: "var1",
				summary: "v",
			}),
			command({ command_type: "RemoveLayer", layer_id: "layer1" }),
			command({ command_type: "AddPlaceholder", name: "no entity" }),
		]);
		const items = groups.flatMap((group) => group.items);
		expect(items.map((item) => item.entityId)).toEqual([
			"node1",
			"var1",
			"layer1",
			undefined,
		]);
	});
});
