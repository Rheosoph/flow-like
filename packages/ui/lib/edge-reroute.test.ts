import { describe, expect, test } from "bun:test";
import { buildEdgeRerouteCommands } from "./edge-reroute";
import {
	ICommandType,
	type IGenericCommand,
} from "./schema/flow/board/commands/generic-command";
import type { INode } from "./schema/flow/node";
import type { IPin } from "./schema/flow/pin";
import { IPinType, IValueType, IVariableType } from "./schema/flow/pin";
import type { ILayer } from "./schema/flow/run";

const pin = (
	id: string,
	name: string,
	pin_type: IPinType,
	data_type: IVariableType,
	extra: Partial<IPin> = {},
): IPin => ({
	id,
	name,
	friendly_name: name,
	description: "",
	connected_to: [],
	depends_on: [],
	index: 1,
	pin_type,
	value_type: IValueType.Normal,
	data_type,
	default_value: null,
	...extra,
});

const node = (id: string, name: string, pins: IPin[]): INode => ({
	id,
	name,
	friendly_name: name,
	description: "",
	category: "Test",
	pins: Object.fromEntries(pins.map((p) => [p.id, p])),
});

const rerouteTemplate = () =>
	node("reroute-template", "reroute", [
		pin("tpl_in", "route_in", IPinType.Input, IVariableType.Generic),
		pin("tpl_out", "route_out", IPinType.Output, IVariableType.Generic),
	]);

const wire = (fromPin: IPin, toPin: IPin) => {
	const from = node("from", "producer", [fromPin]);
	const to = node("to", "consumer", [toPin]);
	const pinCache = new Map<string, [IPin, INode | ILayer, boolean]>([
		[fromPin.id, [fromPin, from, true]],
		[toPin.id, [toPin, to, true]],
	]);
	return {
		pinCache,
		edge: { sourceHandle: fromPin.id, targetHandle: toPin.id },
	};
};

const addedNode = (commands: IGenericCommand[] | undefined): INode => {
	const added = commands?.[0]?.node;
	if (!added) throw new Error("expected the batch to start with AddNode");
	return added;
};

describe("buildEdgeRerouteCommands", () => {
	test("splits the wire into add, disconnect and two connects", () => {
		const { pinCache, edge } = wire(
			pin("out", "exec_out", IPinType.Output, IVariableType.Execution),
			pin("in", "exec_in", IPinType.Input, IVariableType.Execution),
		);

		const commands = buildEdgeRerouteCommands({
			reroute: rerouteTemplate(),
			edge,
			position: { x: 100, y: 50 },
			currentLayer: "layer-1",
			pinCache,
		});

		expect(commands?.map((c) => c.command_type)).toEqual([
			ICommandType.AddNode,
			ICommandType.DisconnectPin,
			ICommandType.ConnectPin,
			ICommandType.ConnectPin,
		]);

		const [add, disconnect, connectIn, connectOut] = commands ?? [];
		const reroute = addedNode(commands);
		const routeIn = Object.values(reroute.pins).find(
			(p) => p.name === "route_in",
		);
		const routeOut = Object.values(reroute.pins).find(
			(p) => p.name === "route_out",
		);

		expect(add?.current_layer).toBe("layer-1");
		expect(reroute.coordinates).toEqual([92, 44, 0]);
		expect(disconnect).toMatchObject({
			from_node: "from",
			from_pin: "out",
			to_node: "to",
			to_pin: "in",
		});
		expect(connectIn).toMatchObject({
			from_node: "from",
			from_pin: "out",
			to_node: reroute.id,
			to_pin: routeIn?.id,
		});
		expect(connectOut).toMatchObject({
			from_node: reroute.id,
			from_pin: routeOut?.id,
			to_node: "to",
			to_pin: "in",
		});
	});

	test("types the reroute from the source pin without touching the template", () => {
		const template = rerouteTemplate();
		const { pinCache, edge } = wire(
			pin("out", "items", IPinType.Output, IVariableType.Struct, {
				value_type: IValueType.Array,
				schema: "schema-ref",
			}),
			pin("in", "items", IPinType.Input, IVariableType.Struct, {
				value_type: IValueType.Array,
			}),
		);

		const commands = buildEdgeRerouteCommands({
			reroute: template,
			edge,
			position: { x: 0, y: 0 },
			currentLayer: undefined,
			pinCache,
		});

		const reroute = addedNode(commands);
		for (const p of Object.values(reroute.pins)) {
			expect(p.data_type).toBe(IVariableType.Struct);
			expect(p.value_type).toBe(IValueType.Array);
			expect(p.schema).toBe("schema-ref");
		}
		expect(Object.keys(template.pins)).toEqual(["tpl_in", "tpl_out"]);
		expect(template.pins.tpl_in.data_type).toBe(IVariableType.Generic);
	});

	test("ignores function-reference edges", () => {
		const { pinCache } = wire(
			pin("out", "exec_out", IPinType.Output, IVariableType.Execution),
			pin("in", "exec_in", IPinType.Input, IVariableType.Execution),
		);

		expect(
			buildEdgeRerouteCommands({
				reroute: rerouteTemplate(),
				edge: { sourceHandle: "ref_out_a", targetHandle: "ref_in_b" },
				position: { x: 0, y: 0 },
				currentLayer: undefined,
				pinCache,
			}),
		).toBeUndefined();
	});
});
