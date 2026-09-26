import {
	addNodeCommand,
	connectPinsCommand,
	disconnectPinsCommand,
} from "./command/generic-command";
import type { IGenericCommand } from "./schema/flow/board/commands/generic-command";
import type { INode } from "./schema/flow/node";
import type { IPin } from "./schema/flow/pin";
import type { ILayer } from "./schema/flow/run";

/** Half the rendered reroute node, so it lands centered on the click. */
const REROUTE_HALF_SIZE = { x: 8, y: 6 };

interface EdgeHandles {
	sourceHandle?: string | null;
	targetHandle?: string | null;
}

interface EdgeRerouteParams {
	reroute: INode;
	edge: EdgeHandles;
	position: { x: number; y: number };
	currentLayer: string | undefined;
	pinCache: Map<string, [IPin, INode | ILayer, boolean]>;
}

const pinByName = (node: INode, name: string) =>
	Object.values(node.pins).find((pin) => pin.name === name);

/**
 * Splits a wire with a reroute node as one command batch, so a single undo restores the wire.
 *
 * The reroute's pins take the source pin's type up front instead of waiting for its `on_update`:
 * connecting a Generic `route_out` into an execution input would replace every other wire into it.
 *
 * Returns `undefined` for edges that are not pin-to-pin wires (function references).
 */
export function buildEdgeRerouteCommands({
	reroute,
	edge,
	position,
	currentLayer,
	pinCache,
}: EdgeRerouteParams): IGenericCommand[] | undefined {
	const [fromPin, fromNode] = pinCache.get(edge.sourceHandle ?? "") ?? [];
	const [toPin, toNode] = pinCache.get(edge.targetHandle ?? "") ?? [];
	if (!fromPin || !fromNode || !toPin || !toNode) return undefined;

	const template = structuredClone(reroute);
	for (const pin of Object.values(template.pins)) {
		pin.data_type = fromPin.data_type;
		pin.value_type = fromPin.value_type;
		pin.schema = fromPin.schema;
		pin.connected_to = [];
		pin.depends_on = [];
	}

	const { command, node } = addNodeCommand({
		node: {
			...template,
			coordinates: [
				position.x - REROUTE_HALF_SIZE.x,
				position.y - REROUTE_HALF_SIZE.y,
				0,
			],
		},
		current_layer: currentLayer,
	});

	const routeIn = pinByName(node, "route_in");
	const routeOut = pinByName(node, "route_out");
	if (!routeIn || !routeOut) return undefined;

	return [
		command,
		disconnectPinsCommand({
			from_node: fromNode.id,
			from_pin: fromPin.id,
			to_node: toNode.id,
			to_pin: toPin.id,
		}),
		connectPinsCommand({
			from_node: fromNode.id,
			from_pin: fromPin.id,
			to_node: node.id,
			to_pin: routeIn.id,
		}),
		connectPinsCommand({
			from_node: node.id,
			from_pin: routeOut.id,
			to_node: toNode.id,
			to_pin: toPin.id,
		}),
	];
}
