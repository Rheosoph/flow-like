import type { IGenericCommand, INode, IPin } from "@flow-like/flow-like-ui";
import { describe, expect, test } from "vitest";
import {
	findUnresolvedPinReferences,
	repairUnreplayableCommandBatch,
} from "../command-sync";

function pin(id: string, name: string): IPin {
	return {
		id,
		name,
		friendly_name: name,
		description: "",
		connected_to: [],
		depends_on: [],
		data_type: "Generic",
		value_type: "Normal",
		index: 0,
		pin_type: "Input",
	} as unknown as IPin;
}

function node(id: string, pins: IPin[]): INode {
	return {
		id,
		name: "control_call_function",
		friendly_name: "Call setup",
		description: "",
		category: "control",
		pins: Object.fromEntries(pins.map((p) => [p.id, p])),
	} as unknown as INode;
}

/**
 * The exact shape of the batches that wedged three boards: the node is added carrying only its
 * static pin, `on_update` mints the mirrored pins locally, and the trailing ConnectPin references
 * one of those minted ids — which exists on no other machine.
 */
function wedgedBatch(): IGenericCommand[] {
	return [
		{
			command_type: "AddNode",
			node: node("call-node", [pin("static-pin", "function_layer_id")]),
			current_layer: null,
		},
		{
			command_type: "AddNode",
			node: node("source-node", [pin("source-out", "value")]),
			current_layer: null,
		},
		{
			command_type: "ConnectPin",
			from_node: "source-node",
			from_pin: "source-out",
			to_node: "call-node",
			to_pin: "minted-pin",
		},
	] as unknown as IGenericCommand[];
}

const localNodes: Record<string, INode> = {
	"call-node": node("call-node", [
		pin("static-pin", "function_layer_id"),
		pin("minted-pin", "database"),
	]),
};

describe("findUnresolvedPinReferences", () => {
	test("reports a ConnectPin target that no earlier command materialises", () => {
		const { missing, firstUnresolvedIndex } = findUnresolvedPinReferences(
			wedgedBatch(),
		);
		expect(firstUnresolvedIndex).toBe(2);
		expect([...(missing.get("call-node") ?? [])]).toEqual(["minted-pin"]);
	});

	test("counts pins that arrive with a Function layer, not just with nodes", () => {
		const commands = [
			{
				command_type: "UpsertLayer",
				layer: {
					id: "fn-layer",
					pins: { "layer-pin": pin("layer-pin", "exec_in") },
					nodes: {},
				},
			},
			{
				command_type: "AddNode",
				node: node("source-node", [pin("source-out", "value")]),
			},
			{
				command_type: "ConnectPin",
				from_node: "source-node",
				from_pin: "source-out",
				to_node: "fn-layer",
				to_pin: "layer-pin",
			},
		] as unknown as IGenericCommand[];

		expect(findUnresolvedPinReferences(commands).missing.size).toBe(0);
	});
});

describe("repairUnreplayableCommandBatch", () => {
	test("restates the owning node before the first unresolved connection", () => {
		const repaired = repairUnreplayableCommandBatch(wedgedBatch(), localNodes);
		expect(repaired).toBeDefined();
		if (!repaired) return;

		expect(repaired.map((command) => command.command_type)).toEqual([
			"AddNode",
			"AddNode",
			"UpdateNode",
			"ConnectPin",
		]);
		expect(repaired[2].node?.pins["minted-pin"]).toBeDefined();
		expect(findUnresolvedPinReferences(repaired).missing.size).toBe(0);
	});

	test("leaves an already replayable batch untouched", () => {
		const commands = [
			{
				command_type: "AddNode",
				node: node("call-node", [
					pin("static-pin", "function_layer_id"),
					pin("minted-pin", "database"),
				]),
			},
			{
				command_type: "AddNode",
				node: node("source-node", [pin("source-out", "value")]),
			},
			{
				command_type: "ConnectPin",
				from_node: "source-node",
				from_pin: "source-out",
				to_node: "call-node",
				to_pin: "minted-pin",
			},
		] as unknown as IGenericCommand[];

		expect(
			repairUnreplayableCommandBatch(commands, localNodes),
		).toBeUndefined();
	});

	test("refuses a partial repair when the local board no longer has the pin", () => {
		expect(
			repairUnreplayableCommandBatch(wedgedBatch(), {
				"call-node": node("call-node", [
					pin("static-pin", "function_layer_id"),
				]),
			}),
		).toBeUndefined();
		expect(repairUnreplayableCommandBatch(wedgedBatch(), {})).toBeUndefined();
	});
});
