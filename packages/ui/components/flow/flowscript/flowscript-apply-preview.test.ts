import { describe, expect, test } from "bun:test";
import type { BoardCommand } from "../../../lib/schema/flow/copilot";
import {
	destructiveCommandSummaries,
	groupBoardCommands,
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
