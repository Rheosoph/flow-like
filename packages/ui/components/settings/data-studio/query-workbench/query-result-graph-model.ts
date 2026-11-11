import type {
	GraphOverlay,
	SubgraphEdge,
	SubgraphNode,
	SubgraphResult,
} from "../../../../state/backend-state/graph-state";
import {
	buildOverlayFromSubgraph,
	enrichSubgraphWithStyles,
} from "../../../ui/graph/subgraph-utils";
import { cellToString } from "./column-types";

export const QUERY_GRAPH_MAX_NODES = 10_000;
export const QUERY_GRAPH_MAX_EDGES = 20_000;

export interface QueryGraphEndpoint {
	column: string;
	label: string;
}

export interface QueryGraphLimits {
	nodes: number;
	edges: number;
}

export interface QueryResultGraphModel {
	overlay: GraphOverlay;
	data: SubgraphResult;
}

const DEFAULT_LIMITS: QueryGraphLimits = {
	nodes: QUERY_GRAPH_MAX_NODES,
	edges: QUERY_GRAPH_MAX_EDGES,
};

/**
 * A node takes the source label once it appears as a source anywhere, so its
 * type does not depend on row order.
 */
function touchNode(
	nodes: Map<string, SubgraphNode>,
	id: string,
	value: unknown,
	endpoint: QueryGraphEndpoint,
	isSource: boolean,
): void {
	const existing = nodes.get(id);
	if (!existing) {
		nodes.set(id, {
			id,
			label: endpoint.label,
			caption: id,
			props: { [endpoint.column]: value },
		});
		return;
	}
	existing.props[endpoint.column] = value;
	if (isSource) existing.label = endpoint.label;
}

/**
 * Turns result rows into a graph: equal values in the source and target
 * columns are one node, and every row is an edge carrying the whole row.
 */
export function buildQueryResultGraph(
	rows: readonly Record<string, unknown>[],
	source: QueryGraphEndpoint,
	target: QueryGraphEndpoint,
	limits: QueryGraphLimits = DEFAULT_LIMITS,
): QueryResultGraphModel {
	const nodes = new Map<string, SubgraphNode>();
	const edges: SubgraphEdge[] = [];
	const edgeLabel =
		source.column === target.column
			? source.label
			: `${source.label} → ${target.label}`;
	let truncated = false;

	for (const [index, row] of rows.entries()) {
		const from = row[source.column];
		const to = row[target.column];
		const fromId = cellToString(from);
		const toId = cellToString(to);
		if (!fromId || !toId) continue;

		const newNodes =
			Number(!nodes.has(fromId)) + Number(toId !== fromId && !nodes.has(toId));
		if (edges.length >= limits.edges || nodes.size + newNodes > limits.nodes) {
			truncated = true;
			break;
		}

		touchNode(nodes, fromId, from, source, true);
		touchNode(nodes, toId, to, target, false);
		edges.push({
			id: `row:${index}`,
			source: fromId,
			target: toId,
			label: edgeLabel,
			props: row,
		});
	}

	const nodeList = [...nodes.values()];
	const overlay = buildOverlayFromSubgraph(nodeList, edges, {
		name: edgeLabel,
	});
	return {
		overlay,
		data: enrichSubgraphWithStyles(
			{ nodes: nodeList, edges, truncated },
			overlay,
		),
	};
}
