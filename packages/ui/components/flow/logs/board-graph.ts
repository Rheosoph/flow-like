import {
	type IBoard,
	type ILayer,
	ILayerType,
} from "../../../lib/schema/flow/board";
import type { INode } from "../../../lib/schema/flow/node";
import {
	type IPin,
	IPinType,
	IVariableType,
} from "../../../lib/schema/flow/pin";

type IPinOwner =
	| { kind: "node"; id: string }
	| { kind: "layer"; layer: ILayer };

export interface IBoardGraph {
	nodes: Record<string, INode>;
	owners: Map<string, IPinOwner>;
	/** Input pin id → the pin ids that feed it, from both edge directions. */
	sources: Map<string, string[]>;
}

function link(sources: Map<string, string[]>, target: string, source: string) {
	const list = sources.get(target);
	if (!list) sources.set(target, [source]);
	else if (!list.includes(source)) list.push(source);
}

function collectPins(
	pins: Record<string, IPin> | undefined,
	owner: IPinOwner,
	owners: Map<string, IPinOwner>,
	sources: Map<string, string[]>,
) {
	for (const pin of Object.values(pins ?? {})) {
		if (!pin?.id) continue;
		owners.set(pin.id, owner);
		for (const target of pin.connected_to ?? []) link(sources, target, pin.id);
		for (const source of pin.depends_on ?? []) link(sources, pin.id, source);
	}
}

export function buildBoardGraph(board: IBoard | undefined): IBoardGraph {
	const owners = new Map<string, IPinOwner>();
	const sources = new Map<string, string[]>();
	const nodes = board?.nodes ?? {};
	for (const node of Object.values(nodes)) {
		if (node?.id)
			collectPins(node.pins, { kind: "node", id: node.id }, owners, sources);
	}
	for (const layer of Object.values(board?.layers ?? {})) {
		if (layer?.id)
			collectPins(layer.pins, { kind: "layer", layer }, owners, sources);
	}
	return { nodes, owners, sources };
}

function isExec(pin: IPin): boolean {
	return pin.data_type === IVariableType.Execution;
}

function inputPins(node: INode | undefined, exec: boolean): IPin[] {
	return Object.values(node?.pins ?? {})
		.filter((pin) => pin.pin_type === IPinType.Input && isExec(pin) === exec)
		.sort((a, b) => a.index - b.index);
}

/** Nodes feeding one pin, looking through collapsed-layer relay pins. */
function feedingNodes(graph: IBoardGraph, pinId: string): string[] {
	const out: string[] = [];
	const seen = new Set<string>([pinId]);
	const stack = [...(graph.sources.get(pinId) ?? [])];
	while (stack.length > 0) {
		const source = stack.pop() as string;
		if (seen.has(source)) continue;
		seen.add(source);
		const owner = graph.owners.get(source);
		if (!owner) continue;
		if (owner.kind === "node") {
			if (!out.includes(owner.id)) out.push(owner.id);
			continue;
		}
		if (owner.layer.type === ILayerType.Function) continue;
		stack.push(...(graph.sources.get(source) ?? []));
	}
	return out;
}

/**
 * What led to a node: everything its data inputs came from, transitively,
 * plus the nodes that trigger it directly. The execution chain is not walked
 * further back, or every node would be upstream of the last one.
 */
export function upstreamNodes(graph: IBoardGraph, nodeId: string): string[] {
	const out: string[] = [];
	const add = (id: string) => {
		if (id !== nodeId && !out.includes(id)) out.push(id);
	};
	for (const pin of inputPins(graph.nodes[nodeId], true)) {
		for (const id of feedingNodes(graph, pin.id)) add(id);
	}
	const seen = new Set<string>([nodeId]);
	const stack = [nodeId];
	while (stack.length > 0) {
		const current = stack.pop() as string;
		for (const pin of inputPins(graph.nodes[current], false)) {
			for (const id of feedingNodes(graph, pin.id)) {
				add(id);
				if (!seen.has(id)) {
					seen.add(id);
					stack.push(id);
				}
			}
		}
	}
	return out;
}

export interface IMissedNode {
	nodeId: string;
	/** The node and pin that waited for its value. */
	feeds: { nodeId: string; pin: string };
}

/**
 * Data sources of a node that never executed in the run. The walk passes
 * through sources that did run and stops at the first that did not, so the
 * result names the nearest cause rather than its whole ancestry.
 */
export function missedUpstream(
	graph: IBoardGraph,
	nodeId: string,
	visited: ReadonlySet<string>,
): IMissedNode[] {
	const out: IMissedNode[] = [];
	const seen = new Set<string>([nodeId]);
	const queue = [nodeId];
	while (queue.length > 0) {
		const current = queue.shift() as string;
		for (const pin of inputPins(graph.nodes[current], false)) {
			for (const id of feedingNodes(graph, pin.id)) {
				if (seen.has(id)) continue;
				seen.add(id);
				if (visited.has(id)) queue.push(id);
				else {
					out.push({
						nodeId: id,
						feeds: { nodeId: current, pin: pin.friendly_name || pin.name },
					});
				}
			}
		}
	}
	return out;
}

export function nodeLabel(node: INode | undefined): string | undefined {
	if (!node) return undefined;
	return node.friendly_name?.trim() || node.name;
}

/** Board name, then each enclosing layer from the outside in. */
export function layerPath(board: IBoard | undefined, nodeId: string): string[] {
	if (!board) return [];
	const path: string[] = [];
	const seen = new Set<string>();
	let layerId = board.nodes?.[nodeId]?.layer ?? undefined;
	while (layerId && !seen.has(layerId)) {
		seen.add(layerId);
		const layer = board.layers?.[layerId];
		if (!layer) break;
		path.unshift(layer.name);
		layerId = layer.parent_id ?? undefined;
	}
	if (board.name) path.unshift(board.name);
	return path;
}
