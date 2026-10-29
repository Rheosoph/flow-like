import { describe, expect, test } from "bun:test";
import { type IBoard, type ILayer, ILayerType } from "../schema/flow/board";
import {
	ICommandType,
	type IGenericCommand,
} from "../schema/flow/board/commands/generic-command";
import type { INode } from "../schema/flow/node";
import {
	type IPin,
	IPinType,
	IValueType,
	IVariableType,
} from "../schema/flow/pin";
import { buildDynamicPinRemovalCommands } from "./dynamic-pins";

type Board = Pick<IBoard, "nodes" | "layers">;

function pin(id: string, name: string, index: number, input = false): IPin {
	return {
		id,
		name,
		friendly_name: id,
		description: "",
		index,
		pin_type: input ? IPinType.Input : IPinType.Output,
		data_type: IVariableType.String,
		value_type: IValueType.Normal,
		connected_to: [],
		depends_on: [],
	};
}

function node(id: string, pins: IPin[]): INode {
	return {
		id,
		name: id,
		friendly_name: id,
		category: "test",
		description: "",
		coordinates: [10, 20, 3],
		pins: Object.fromEntries(pins.map((pin) => [pin.id, pin])),
	};
}

function layer(id: string, pins: IPin[]): ILayer {
	return {
		id,
		name: id,
		type: ILayerType.Collapsed,
		comments: {},
		coordinates: [0, 0, 0],
		nodes: {},
		variables: {},
		pins: Object.fromEntries(pins.map((pin) => [pin.id, pin])),
	};
}

function fixture() {
	const source = node("source", [
		pin("in", "input", 8, true),
		pin("first", "branch", 1),
		pin("second", "branch", 2),
		pin("third", "branch", 3),
		pin("tail", "other", 7),
	]);
	const peer = node("peer", [pin("target", "target", 1, true)]);
	return {
		board: { nodes: { source, peer }, layers: {} } as Board,
		source,
	};
}

const plan = (board: Board, source = board.nodes.source, pinId = "third") =>
	buildDynamicPinRemovalCommands({
		node: source,
		board,
		pinId,
		position: { x: 50, y: 80 },
	});

function freeze<T>(value: T): T {
	if (value && typeof value === "object") {
		for (const item of Object.values(value)) freeze(item);
		Object.freeze(value);
	}
	return value;
}

function required<T>(value: T | undefined | null): T {
	if (value == null) throw new Error("Expected a fixture value");
	return value;
}

function endpoint(board: Board, owner?: string, id?: string) {
	return (board.nodes[required(owner)] ?? board.layers[required(owner)]).pins[
		required(id)
	];
}

/** Replay the backend's relevant receipt behavior, including reverse batch order. */
function apply(board: Board, commands: IGenericCommand[]) {
	return commands.map((command) => {
		if (command.command_type === ICommandType.UpdateNode && command.node) {
			const old_node = structuredClone(board.nodes[command.node.id]);
			board.nodes[command.node.id] = structuredClone(command.node);
			return { ...command, old_node };
		}
		const from = endpoint(board, command.from_node, command.from_pin);
		const to = endpoint(board, command.to_node, command.to_pin);
		from.connected_to = from.connected_to.filter((id) => id !== to.id);
		to.depends_on = to.depends_on.filter((id) => id !== from.id);
		return command;
	});
}

function undo(board: Board, receipts: IGenericCommand[]) {
	for (const receipt of receipts.toReversed()) {
		if (receipt.command_type === ICommandType.UpdateNode && receipt.old_node) {
			board.nodes[receipt.old_node.id] = structuredClone(receipt.old_node);
			continue;
		}
		const from = endpoint(board, receipt.from_node, receipt.from_pin);
		const to = endpoint(board, receipt.to_node, receipt.to_pin);
		from.connected_to = [...new Set([...from.connected_to, to.id])].sort();
		to.depends_on = [...new Set([...to.depends_on, from.id])].sort();
	}
}

describe("native dynamic pin removal", () => {
	test("removes only an extra family member, preserves current metadata and reindexes each direction", () => {
		const { board, source } = fixture();
		const stale = structuredClone(source);
		source.comment = "A peer's current comment";
		const before = structuredClone(board);
		freeze(board);
		const commands = required(plan(board, stale));
		expect(commands).toHaveLength(1);
		expect(commands[0].node?.comment).toBe(source.comment);
		expect(commands[0].node?.coordinates).toEqual([50, 80, 3]);
		expect(
			Object.values(required(commands[0].node).pins).map((p) => [
				p.id,
				p.index,
			]),
		).toEqual([
			["in", 1],
			["first", 1],
			["second", 2],
			["tail", 3],
		]);
		expect(board).toEqual(before);
	});

	test("disconnects node and canonical layer peers before deletion and restores the whole batch", () => {
		const { board, source } = fixture();
		board.layers.layer = layer("layer", [pin("boundary", "boundary", 1, true)]);
		source.pins.third.connected_to = ["boundary", "target"];
		board.nodes.peer.pins.target.depends_on = ["third"];
		board.layers.layer.pins.boundary.depends_on = ["third"];
		const commands = required(plan(board));
		expect(commands.map((command) => command.command_type)).toEqual([
			ICommandType.DisconnectPin,
			ICommandType.DisconnectPin,
			ICommandType.UpdateNode,
		]);
		expect(commands[0].to_node).toBe("layer");
		const before = structuredClone(board);
		const receipts = apply(board, commands);
		expect(board.nodes.source.pins.third).toBeUndefined();
		expect(board.nodes.peer.pins.target.depends_on).toEqual([]);
		expect(board.layers.layer.pins.boundary.depends_on).toEqual([]);
		undo(board, receipts);
		expect(board).toEqual(before);
	});

	test("input removal uses canonical connectivity even when a layer source is an Input pin", () => {
		const { board, source } = fixture();
		for (const id of ["first", "second", "third"])
			source.pins[id].pin_type = IPinType.Input;
		board.layers.layer = layer("layer", [pin("boundary", "boundary", 1, true)]);
		source.pins.third.depends_on = ["boundary"];
		board.layers.layer.pins.boundary.connected_to = ["third"];
		const commands = required(plan(board));
		expect(commands[0]).toMatchObject({
			command_type: ICommandType.DisconnectPin,
			from_node: "layer",
			from_pin: "boundary",
			to_node: "source",
			to_pin: "third",
		});
	});

	test("same-node and self links stay in the UpdateNode receipt instead of invalid self-connect undo commands", () => {
		const { board, source } = fixture();
		source.pins.third.connected_to = ["in", "third"];
		source.pins.third.depends_on = ["third"];
		source.pins.in.depends_on = ["third"];
		const before = structuredClone(board);
		const commands = required(plan(board));
		expect(commands).toHaveLength(1);
		expect(commands[0].node?.pins.in.depends_on).toEqual([]);
		const receipts = apply(board, commands);
		undo(board, receipts);
		expect(board).toEqual(before);
	});

	test("deduplicates reciprocal wires and removes one-sided peer references", () => {
		const { board, source } = fixture();
		source.pins.third.connected_to = ["missing", "target", "target"];
		board.nodes.peer.pins.target.depends_on = ["third"];
		expect(plan(board)).toHaveLength(2);
		source.pins.third.connected_to = [];
		const commands = required(plan(board));
		expect(commands).toHaveLength(2);
		apply(board, commands);
		expect(board.nodes.peer.pins.target.depends_on).toEqual([]);
	});

	test("rejects missing, changed, protected and minimum-family pins", () => {
		const { board, source } = fixture();
		expect(plan(board, source, "missing")).toBeUndefined();
		expect(plan(board, source, "first")).toBeUndefined();
		expect(plan(board, source, "second")).toBeUndefined();
		const stale = structuredClone(source);
		stale.pins.third.name = "different-family";
		expect(plan(board, stale)).toBeUndefined();
		source.pins = Object.fromEntries(
			Object.entries(source.pins).filter(([id]) => id !== "second"),
		);
		expect(plan(board)).toBeUndefined();
		board.nodes = { peer: board.nodes.peer };
		expect(plan(board, source)).toBeUndefined();
	});
});
