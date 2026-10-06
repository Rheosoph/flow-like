import { describe, expect, test } from "bun:test";
import { moveNodeCommand } from "../command/generic-command";
import {
	ICommandType,
	type IGenericCommand,
} from "../schema/flow/board/commands/generic-command";
import type { INode, IPin } from "../schema/flow/node";
import { IPinType, IValueType, IVariableType } from "../schema/flow/node";
import type { ILayer } from "../schema/flow/run";
import { computeFlowLayoutDetailed } from "./index";
import { normalizeAutoReroutes } from "./normalize-reroutes";
import { buildAutoRerouteCommands } from "./reroute-commands";
import { GraphBuilder } from "./test-fixtures";
import type { AutoLayoutInput } from "./types";
function template(): INode {
	return {
		id: "template",
		name: "reroute",
		friendly_name: "Reroute",
		category: "Control",
		description: "",
		pins: Object.fromEntries(
			["route_in", "route_out"].map((name, index) => [
				name,
				{
					id: name,
					name,
					friendly_name: name,
					description: "",
					pin_type: index ? IPinType.Output : IPinType.Input,
					data_type: IVariableType.Generic,
					value_type: IValueType.Normal,
					index: 1,
					connected_to: [],
					depends_on: [],
				},
			]),
		),
	};
}
function cache(input: AutoLayoutInput) {
	const result = new Map<string, [IPin, INode | ILayer, boolean]>();
	for (const node of input.layerNodes) {
		for (const pin of Object.values(node.pins))
			result.set(pin.id, [pin, node, true]);
	}
	return result;
}
function fixture(): AutoLayoutInput {
	const graph = new GraphBuilder();
	graph.exec("event", { start: true, execIn: false, dataOuts: 1 });
	graph.exec("middle", { dataIns: 1, dataOuts: 2 });
	graph.exec("target", { dataIns: 1 });
	graph.execLink("event", "middle");
	graph.execLink("middle", "target");
	graph.dataLink("event", "target");
	graph.dataLink("event", "middle");
	const input: AutoLayoutInput = {
		layerNodes: [...graph.nodes.values()],
		layerEntities: [],
		currentLayer: undefined,
	};
	const pins = cache(input);
	for (const [pin] of pins.values()) {
		for (const target of pin.connected_to)
			pins.get(target)?.[0].depends_on.push(pin.id);
	}
	return input;
}
function executionFixture(): AutoLayoutInput {
	const graph = new GraphBuilder();
	graph.exec("branch", {
		start: true,
		execIn: false,
		execOuts: ["true", "false"],
	});
	graph.exec("middle");
	graph.exec("target", { execOuts: 0 });
	graph.execLink("branch", "middle", "true");
	graph.execLink("branch", "target", "false");
	graph.execLink("middle", "target");
	const input = graph.build();
	const pins = cache(input);
	for (const [pin] of pins.values()) {
		for (const target of pin.connected_to)
			pins.get(target)?.[0].depends_on.push(pin.id);
	}
	return input;
}
// Applies the public command payload to a detached board. Rust batch tests exercise undo.
function apply(
	input: AutoLayoutInput,
	commands: IGenericCommand[],
): AutoLayoutInput {
	const next = structuredClone(input);
	const nodes = new Map(next.layerNodes.map((node) => [node.id, node]));
	for (const command of commands) {
		switch (command.command_type) {
			case ICommandType.MoveNode:
				required(nodes.get(required(command.node_id))).coordinates = [
					...required(command.to_coordinates),
				];
				break;
			case ICommandType.AddNode:
				nodes.set(required(command.node).id, {
					...structuredClone(required(command.node)),
					layer: command.current_layer,
				});
				break;
			case ICommandType.RemoveNode:
				nodes.delete(required(command.node).id);
				break;
			case ICommandType.ConnectPin: {
				const source = required(nodes.get(required(command.from_node))).pins[
					required(command.from_pin)
				];
				const target = required(nodes.get(required(command.to_node))).pins[
					required(command.to_pin)
				];
				if (source.data_type === IVariableType.Execution) {
					source.connected_to = [target.id];
					for (const node of nodes.values()) {
						for (const pin of Object.values(node.pins))
							pin.depends_on = pin.depends_on.filter((id) => id !== source.id);
					}
					target.depends_on.push(source.id);
				} else {
					source.connected_to = [
						...new Set([...source.connected_to, target.id]),
					];
					target.depends_on = [source.id];
				}
				break;
			}
			case ICommandType.DisconnectPin: {
				const source = required(nodes.get(required(command.from_node))).pins[
					required(command.from_pin)
				];
				const target = required(nodes.get(required(command.to_node))).pins[
					required(command.to_pin)
				];
				source.connected_to = source.connected_to.filter(
					(id) => id !== target.id,
				);
				target.depends_on = target.depends_on.filter((id) => id !== source.id);
				break;
			}
			default:
				throw new Error(`Unexpected command ${command.command_type}`);
		}
	}
	next.layerNodes = [...nodes.values()];
	return next;
}
function layoutCommands(input: AutoLayoutInput) {
	const result = computeFlowLayoutDetailed(input, "routed");
	const commands = [...result.positions].map(([id, coordinates]) =>
		moveNodeCommand({
			node_id: id,
			to_coordinates: [...coordinates, 0],
			current_layer: input.currentLayer,
		}),
	);
	commands.push(
		...buildAutoRerouteCommands({
			...required(result.routing),
			reroute: template(),
			currentLayer: input.currentLayer,
			pinCache: cache(input),
		}),
	);
	return { result, commands };
}
const logicalEdges = (input: AutoLayoutInput) =>
	normalizeAutoReroutes(input)
		.input.layerNodes.flatMap((node) =>
			Object.values(node.pins).flatMap((pin) =>
				pin.connected_to.map((id) => `${pin.id}->${id}`),
			),
		)
		.sort();
describe("automatic reroute graph edits", () => {
	test("inserts, reuses, and removes execution chains while preserving other incoming wires", () => {
		const input = executionFixture();
		const route = {
			from: "branch",
			to: "target",
			fromPin: "branch:false",
			toPin: "target:exec-in",
			waypoints: [
				{ x: 250, y: 120 },
				{ x: 550, y: 120 },
			],
		};
		const commands = buildAutoRerouteCommands({
			routes: [route],
			chains: [],
			reroute: template(),
			currentLayer: undefined,
			pinCache: cache(input),
		});
		const routed = apply(input, commands);
		const dots = routed.layerNodes.filter((node) => node.auto_reroute);
		expect(dots).toHaveLength(2);
		for (const dot of dots) {
			for (const pin of Object.values(dot.pins)) {
				expect(pin.data_type).toBe(IVariableType.Execution);
				expect(pin.default_value).toBeNull();
			}
		}
		expect(cache(routed).get("target:exec-in")?.[0].depends_on).toContain(
			"middle:exec-out-0",
		);
		expect(logicalEdges(routed)).toEqual(logicalEdges(input));
		const normalized = normalizeAutoReroutes(routed);
		expect(normalized.chains).toHaveLength(1);
		expect(normalized.input.layerNodes).toHaveLength(input.layerNodes.length);
		for (const current of [routed, JSON.parse(JSON.stringify(routed))]) {
			expect(
				buildAutoRerouteCommands({
					routes: [route],
					chains: normalizeAutoReroutes(current).chains,
					reroute: template(),
					currentLayer: undefined,
					pinCache: cache(current),
				}),
			).toEqual([]);
		}
		const removed = buildAutoRerouteCommands({
			routes: [{ ...route, waypoints: [] }],
			chains: normalized.chains,
			reroute: template(),
			currentLayer: undefined,
			pinCache: cache(routed),
		});
		const direct = apply(routed, removed);
		expect(direct.layerNodes.some((node) => node.auto_reroute)).toBe(false);
		expect(logicalEdges(direct)).toEqual(logicalEdges(input));
		expect(cache(direct).get("target:exec-in")?.[0].depends_on.sort()).toEqual([
			"branch:false",
			"middle:exec-out-0",
		]);
	});
	test("keeps generated execution reroutes that merge incoming wires", () => {
		const input = executionFixture();
		const commands = buildAutoRerouteCommands({
			routes: [
				{
					from: "branch",
					to: "target",
					fromPin: "branch:false",
					toPin: "target:exec-in",
					waypoints: [{ x: 250, y: 120 }],
				},
			],
			chains: [],
			reroute: template(),
			currentLayer: undefined,
			pinCache: cache(input),
		});
		const routed = apply(input, commands);
		const pins = cache(routed);
		const dot = required(routed.layerNodes.find((node) => node.auto_reroute));
		const routeIn = required(
			Object.values(dot.pins).find((pin) => pin.name === "route_in"),
		);
		required(pins.get("middle:exec-out-0"))[0].connected_to = [routeIn.id];
		required(pins.get("target:exec-in"))[0].depends_on = required(
			pins.get("target:exec-in"),
		)[0].depends_on.filter((id) => id !== "middle:exec-out-0");
		routeIn.depends_on.push("middle:exec-out-0");
		expect(normalizeAutoReroutes(routed).chains).toEqual([]);
		expect(normalizeAutoReroutes(routed).input).toBe(routed);
	});
	test("keeps chains between the two boundaries of the same open layer", () => {
		const input = fixture();
		const routed = apply(input, layoutCommands(input).commands);
		routed.currentLayer = "fn";
		required(routed.layerNodes.find((node) => node.id === "event")).id =
			"fn-input";
		required(routed.layerNodes.find((node) => node.id === "target")).id =
			"fn-return";
		const normalized = normalizeAutoReroutes(routed);
		expect(normalized.chains).toEqual([]);
		expect(normalized.input).toBe(routed);
		const next = layoutCommands(routed);
		expect(
			next.commands.every(
				(command) => command.command_type === ICommandType.MoveNode,
			),
		).toBe(true);
	});
	test.each([
		["data", fixture],
		["execution", executionFixture],
	] as const)(
		"routes %s skips, preserves connections, and reuses identical nodes on repeat",
		(_, makeFixture) => {
			const input = makeFixture();
			const original = structuredClone(input);
			const first = layoutCommands(input);
			const routed = apply(input, first.commands);
			const dots = routed.layerNodes.filter((node) => node.auto_reroute);
			expect(dots.length).toBeGreaterThanOrEqual(2);
			expect(logicalEdges(routed)).toEqual(logicalEdges(input));
			expect(input).toEqual(original);
			const second = layoutCommands(routed);
			expect(
				second.commands.some(
					(command) => command.command_type === ICommandType.AddNode,
				),
			).toBe(false);
			expect(
				second.commands.some(
					(command) => command.command_type === ICommandType.RemoveNode,
				),
			).toBe(false);
			expect(apply(routed, second.commands)).toEqual(routed);
			const restored = JSON.parse(JSON.stringify(routed)) as AutoLayoutInput;
			expect(apply(restored, layoutCommands(restored).commands)).toEqual(
				restored,
			);
		},
	);
	test("preserves an existing chain when no route could be resolved", () => {
		const input = fixture();
		const routed = apply(input, layoutCommands(input).commands);
		const normalized = normalizeAutoReroutes(routed);
		expect(normalized.chains.length).toBeGreaterThan(0);
		const commands = buildAutoRerouteCommands({
			routes: normalized.chains.map((chain) => ({
				...chain,
				waypoints: [],
				unresolved: true,
			})),
			chains: normalized.chains,
			reroute: template(),
			currentLayer: undefined,
			pinCache: cache(routed),
		});
		expect(commands).toEqual([]);
		expect(apply(routed, commands)).toEqual(routed);
	});
	test("preserves normalized chains omitted from the routing plan", () => {
		const input = fixture();
		const routed = apply(input, layoutCommands(input).commands);
		const normalized = normalizeAutoReroutes(routed);
		expect(normalized.chains.length).toBeGreaterThan(0);
		const commands = buildAutoRerouteCommands({
			routes: [],
			chains: normalized.chains,
			reroute: template(),
			currentLayer: undefined,
			pinCache: cache(routed),
		});
		expect(commands).toEqual([]);
		expect(apply(routed, commands)).toEqual(routed);
	});
	test("shrinks a generated chain without dropping sibling connections", () => {
		const input = fixture();
		const routed = apply(input, layoutCommands(input).commands);
		const normalized = normalizeAutoReroutes(routed);
		const commands = buildAutoRerouteCommands({
			routes: normalized.chains.map((chain) => ({ ...chain, waypoints: [] })),
			chains: normalized.chains,
			reroute: template(),
			currentLayer: undefined,
			pinCache: cache(routed),
		});
		const direct = apply(routed, commands);
		expect(direct.layerNodes.some((node) => node.auto_reroute)).toBe(false);
		expect(logicalEdges(direct)).toEqual(logicalEdges(input));
		const firstRemove = commands.findIndex(
			(command) => command.command_type === ICommandType.RemoveNode,
		);
		expect(
			commands
				.slice(0, firstRemove)
				.every(
					(command) => command.command_type === ICommandType.DisconnectPin,
				),
		).toBe(true);
	});
	test("copies data types and schema onto every generated pin", () => {
		const input = fixture();
		const source = input.layerNodes[0].pins["event:out-0"];
		source.data_type = IVariableType.Struct;
		source.value_type = IValueType.Array;
		source.schema = "message-history";
		const routed = apply(input, layoutCommands(input).commands);
		for (const node of routed.layerNodes.filter((node) => node.auto_reroute)) {
			for (const pin of Object.values(node.pins)) {
				expect(pin.data_type).toBe(IVariableType.Struct);
				expect(pin.value_type).toBe(IValueType.Array);
				expect(pin.schema).toBe("message-history");
			}
		}
	});
	test("preserves manual reroutes and branched generated reroutes", () => {
		const input = fixture();
		const routed = apply(input, layoutCommands(input).commands);
		const dot = required(routed.layerNodes.find((node) => node.auto_reroute));
		dot.auto_reroute = false;
		expect(
			normalizeAutoReroutes(routed).input.layerNodes.some(
				(node) => node.id === dot.id,
			),
		).toBe(true);
		dot.auto_reroute = true;
		const output = required(
			Object.values(dot.pins).find((pin) => pin.pin_type === "Output"),
		);
		output.connected_to.push("external-input");
		expect(
			normalizeAutoReroutes(routed).input.layerNodes.some(
				(node) => node.id === dot.id,
			),
		).toBe(true);
	});
	test("includes generated chains between selected endpoints but leaves boundary chains intact", () => {
		const input = fixture();
		const routed = apply(input, layoutCommands(input).commands);
		const dot = required(routed.layerNodes.find((node) => node.auto_reroute));
		const selected = {
			...routed,
			only: new Set(["event", "target"]),
			obstacles: [
				{
					id: dot.id,
					x: required(dot.coordinates)[0],
					y: required(dot.coordinates)[1],
					width: 16,
					height: 12,
				},
			],
		};
		const normalized = normalizeAutoReroutes(selected);
		expect(normalized.chains.length).toBeGreaterThan(0);
		expect(normalized.input.obstacles).toEqual([]);
		const boundary = normalizeAutoReroutes({
			...routed,
			only: new Set(["event", "middle"]),
		});
		expect(boundary.chains).toEqual([]);
		expect(boundary.input.layerNodes.length).toBe(routed.layerNodes.length);
	});
	test("uses real pin owners when displayed endpoints are layer boundaries", () => {
		const input = fixture();
		const pinCache = cache(input);
		const [source] = required(pinCache.get("event:out-0"));
		pinCache.set(source.id, [source, { id: "real-layer" } as ILayer, true]);
		const commands = buildAutoRerouteCommands({
			routes: [
				{
					from: "real-layer-input",
					to: "target",
					fromPin: source.id,
					toPin: "target:in-0",
					waypoints: [{ x: 250, y: 120 }],
				},
			],
			chains: [],
			reroute: template(),
			currentLayer: "real-layer",
			pinCache,
		});
		expect(
			commands.find(
				(command) => command.command_type === ICommandType.DisconnectPin,
			)?.from_node,
		).toBe("real-layer");
		expect(
			commands.find((command) => command.command_type === ICommandType.AddNode)
				?.current_layer,
		).toBe("real-layer");
	});
	test("compact and expanded remain movement-only", () => {
		for (const style of ["compact", "expanded"] as const)
			expect(
				computeFlowLayoutDetailed(fixture(), style).routing,
			).toBeUndefined();
	});
});

function required<T>(value: T | null | undefined): T {
	if (value === undefined || value === null)
		throw new Error("Missing reroute graph value");
	return value;
}
