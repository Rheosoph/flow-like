import {
	addNodeCommand,
	connectPinsCommand,
	disconnectPinsCommand,
	moveNodeCommand,
	removeNodeCommand,
} from "../command/generic-command";
import type { IGenericCommand } from "../schema/flow/board/commands/generic-command";
import type { INode, IPin } from "../schema/flow/node";
import type { ILayer } from "../schema/flow/run";
import type { AutoRerouteChain } from "./normalize-reroutes";
import type { DataRoute } from "./route";
interface RerouteCommandInput {
	routes: readonly DataRoute[];
	chains: readonly AutoRerouteChain[];
	reroute: INode;
	currentLayer: string | undefined;
	pinCache: ReadonlyMap<string, [IPin, INode | ILayer, boolean]>;
}
const routePin = (node: INode, name: string) =>
	Object.values(node.pins).find((pin) => pin.name === name);
const key = (route: {
	fromPin: string;
	toPin: string;
}) => `${route.fromPin}->${route.toPin}`;
/** Build one undoable edit, reusing generated nodes whenever the route allows it. */
export function buildAutoRerouteCommands({
	routes,
	chains,
	reroute,
	currentLayer,
	pinCache,
}: RerouteCommandInput): IGenericCommand[] {
	if (!routePin(reroute, "route_in") || !routePin(reroute, "route_out")) {
		throw new Error("The reroute node is missing its routing pins.");
	}
	const commands: IGenericCommand[] = [];
	const oldChains = new Map(chains.map((chain) => [key(chain), chain]));
	const planned = new Map(routes.map((route) => [key(route), route]));
	// Normalized connections without visible handles still need their old topology.
	for (const chain of chains) {
		if (!planned.has(key(chain)))
			planned.set(key(chain), { ...chain, waypoints: [], unresolved: true });
	}
	for (const route of planned.values()) {
		// A failed search keeps any existing reroute chain and its geometry.
		if (route.unresolved) continue;
		const [fromPin, fromNode] = pinCache.get(route.fromPin) ?? [];
		const [toPin, toNode] = pinCache.get(route.toPin) ?? [];
		if (!fromPin || !fromNode || !toPin || !toNode) continue;
		if (fromPin.data_type === "Execution") continue;
		const previous = oldChains.get(key(route))?.nodes ?? [];
		if (!route.waypoints.length && !previous.length) continue;
		const sameTopology = previous.length === route.waypoints.length;
		const next: INode[] = [];
		for (let index = 0; index < route.waypoints.length; index++) {
			const point = route.waypoints[index];
			const coordinates = [point.x, point.y, 0];
			const existing = previous[index];
			if (existing) {
				next.push(existing);
				if (
					existing.coordinates?.[0] !== coordinates[0] ||
					existing.coordinates?.[1] !== coordinates[1]
				) {
					commands.push(
						moveNodeCommand({
							node_id: existing.id,
							from_coordinates: existing.coordinates ?? [0, 0, 0],
							to_coordinates: coordinates,
							current_layer: currentLayer,
						}),
					);
				}
				continue;
			}
			const template = structuredClone(reroute);
			template.auto_reroute = true;
			template.coordinates = coordinates;
			for (const pin of Object.values(template.pins)) {
				pin.data_type = fromPin.data_type;
				pin.value_type = fromPin.value_type;
				pin.schema = fromPin.schema;
				pin.options = fromPin.options
					? structuredClone(fromPin.options)
					: undefined;
				pin.connected_to = [];
				pin.depends_on = [];
				pin.default_value = null;
			}
			const added = addNodeCommand({
				node: template,
				current_layer: currentLayer,
			});
			commands.push(added.command);
			next.push(added.node);
		}
		if (sameTopology) continue;
		const connections = (nodes: INode[]) => {
			const wires: {
				from_node: string;
				from_pin: string;
				to_node: string;
				to_pin: string;
			}[] = [];
			let source = { node: fromNode.id, pin: fromPin.id };
			for (const node of nodes) {
				wires.push({
					from_node: source.node,
					from_pin: source.pin,
					to_node: node.id,
					to_pin: required(routePin(node, "route_in")).id,
				});
				source = {
					node: node.id,
					pin: required(routePin(node, "route_out")).id,
				};
			}
			wires.push({
				from_node: source.node,
				from_pin: source.pin,
				to_node: toNode.id,
				to_pin: toPin.id,
			});
			return wires;
		};
		for (const wire of connections(previous))
			commands.push(disconnectPinsCommand(wire));
		for (const node of previous.slice(next.length)) {
			const disconnected = structuredClone(node);
			for (const pin of Object.values(disconnected.pins)) {
				pin.connected_to = [];
				pin.depends_on = [];
			}
			commands.push(
				removeNodeCommand({ node: disconnected, connected_nodes: [] }),
			);
		}
		for (const wire of connections(next))
			commands.push(connectPinsCommand(wire));
	}
	return commands;
}

function required<T>(value: T | null | undefined): T {
	if (value === undefined || value === null)
		throw new Error("Missing reroute graph value");
	return value;
}
