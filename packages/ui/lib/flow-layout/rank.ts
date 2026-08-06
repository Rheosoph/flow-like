import type { ComponentInfo } from "./build";
import type { LGraph, LNode, StyleConfig } from "./types";

/**
 * Topological order over non-reversed edges. The graph is a proven DAG by the
 * time this runs, so a plain Kahn sweep is total — there is no fallback branch
 * and no coordinate tiebreak.
 */
export function topologicalOrder(graph: LGraph): string[] {
	const indegree = new Map<string, number>();
	for (const id of graph.order) indegree.set(id, 0);
	for (const edge of graph.edges) {
		if (edge.reversed) continue;
		indegree.set(edge.to, (indegree.get(edge.to) ?? 0) + 1);
	}

	const ready = graph.order.filter((id) => (indegree.get(id) ?? 0) === 0);
	const sorted: string[] = [];

	while (ready.length > 0) {
		const id = ready.shift() as string;
		sorted.push(id);
		const node = graph.nodes.get(id);
		if (!node) continue;
		for (const edge of node.out) {
			if (edge.reversed) continue;
			const next = (indegree.get(edge.to) ?? 0) - 1;
			indegree.set(edge.to, next);
			if (next === 0) ready.push(edge.to);
		}
	}

	// Defensive: a DAG always drains, but never drop a node.
	if (sorted.length < graph.order.length) {
		const placed = new Set(sorted);
		for (const id of graph.order) if (!placed.has(id)) sorted.push(id);
	}
	return sorted;
}

/**
 * Longest-path ranking, then a right-alignment pass that slides pure nodes as
 * far right as their consumers allow.
 *
 * Invariant established here and asserted by the tests: for every non-reversed
 * edge u -> v, `column(u) < column(v)`. A pure node can therefore never render
 * to the right of something it feeds.
 */
export function assignColumns(graph: LGraph, topo: string[]): void {
	for (const id of graph.order) {
		const node = graph.nodes.get(id);
		if (node) node.column = 0;
	}

	for (const id of topo) {
		const node = graph.nodes.get(id);
		if (!node) continue;
		for (const edge of node.out) {
			if (edge.reversed) continue;
			const target = graph.nodes.get(edge.to);
			if (!target) continue;
			target.column = Math.max(target.column, node.column + 1);
		}
	}

	// Right-align pure nodes against their consumers so data wires stay short.
	// Moving right is always safe: the longest-path pass already guarantees
	// every consumer sits at least one column further right.
	for (let index = topo.length - 1; index >= 0; index--) {
		const node = graph.nodes.get(topo[index]);
		if (!node || node.kind !== "pure") continue;

		let limit = Number.POSITIVE_INFINITY;
		for (const edge of node.out) {
			if (edge.reversed) continue;
			const target = graph.nodes.get(edge.to);
			if (!target) continue;
			limit = Math.min(limit, target.column - 1);
		}
		if (Number.isFinite(limit)) node.column = Math.max(node.column, limit);
	}
}

/**
 * The exec node a pure node hangs under. Preference order: the left-most exec
 * consumer (so the value lands next to the first thing that reads it), else the
 * right-most exec producer. Purely topological — no coordinates are consulted.
 */
export function assignOwners(graph: LGraph, topo: string[]): void {
	const consumers = new Map<string, Set<string>>();
	const producers = new Map<string, Set<string>>();
	for (const id of graph.order) {
		consumers.set(id, new Set());
		producers.set(id, new Set());
	}

	for (let index = topo.length - 1; index >= 0; index--) {
		const node = graph.nodes.get(topo[index]);
		if (!node) continue;
		const own = consumers.get(node.id) as Set<string>;
		for (const edge of node.out) {
			if (edge.reversed) continue;
			const target = graph.nodes.get(edge.to);
			if (!target) continue;
			if (target.kind === "exec" || target.kind === "entity") {
				own.add(target.id);
				continue;
			}
			for (const id of consumers.get(target.id) ?? []) own.add(id);
		}
	}

	for (const id of topo) {
		const node = graph.nodes.get(id);
		if (!node) continue;
		const own = producers.get(node.id) as Set<string>;
		for (const edge of node.in) {
			if (edge.reversed) continue;
			const source = graph.nodes.get(edge.from);
			if (!source) continue;
			if (source.kind === "exec" || source.kind === "entity") {
				own.add(source.id);
				continue;
			}
			for (const pid of producers.get(source.id) ?? []) own.add(pid);
		}
	}

	for (const id of graph.order) {
		const node = graph.nodes.get(id);
		if (!node || node.kind !== "pure") continue;

		const consumerIds = [...(consumers.get(id) ?? [])].sort((a, b) =>
			a.localeCompare(b),
		);
		if (consumerIds.length > 0) {
			node.owner = consumerIds.reduce((best, candidate) => {
				const bestNode = graph.nodes.get(best);
				const candidateNode = graph.nodes.get(candidate);
				if (!bestNode) return candidate;
				if (!candidateNode) return best;
				return candidateNode.column < bestNode.column ? candidate : best;
			});
			node.depth = Math.max(
				1,
				(graph.nodes.get(node.owner)?.column ?? node.column) - node.column,
			);
			continue;
		}

		const producerIds = [...(producers.get(id) ?? [])].sort((a, b) =>
			a.localeCompare(b),
		);
		if (producerIds.length > 0) {
			node.owner = producerIds.reduce((best, candidate) => {
				const bestNode = graph.nodes.get(best);
				const candidateNode = graph.nodes.get(candidate);
				if (!bestNode) return candidate;
				if (!candidateNode) return best;
				return candidateNode.column > bestNode.column ? candidate : best;
			});
			node.depth = Math.max(
				1,
				node.column - (graph.nodes.get(node.owner)?.column ?? node.column),
			);
		}
	}
}

/** Shifts each component's columns so the left-most column is 0. */
export function normaliseColumns(
	graph: LGraph,
	components: ComponentInfo[],
): void {
	for (const component of components) {
		let min = Number.POSITIVE_INFINITY;
		for (const id of component.nodeIds) {
			const node = graph.nodes.get(id);
			if (node) min = Math.min(min, node.column);
		}
		if (!Number.isFinite(min) || min === 0) continue;
		for (const id of component.nodeIds) {
			const node = graph.nodes.get(id);
			if (node) node.column -= min;
		}
	}
}

/**
 * Column x positions derived from the widest node in each column, so two
 * columns can never overlap horizontally regardless of node size. This is what
 * reduces overlap resolution to a per-column vertical problem.
 */
export function assignColumnX(
	graph: LGraph,
	component: ComponentInfo,
	cfg: StyleConfig,
): Map<number, number> {
	const widthByColumn = new Map<number, number>();
	for (const id of component.nodeIds) {
		const node = graph.nodes.get(id);
		if (!node) continue;
		widthByColumn.set(
			node.column,
			Math.max(widthByColumn.get(node.column) ?? 0, node.width),
		);
	}

	const columns = [...widthByColumn.keys()].sort((a, b) => a - b);
	const xByColumn = new Map<number, number>();
	let cursor = 0;
	for (const column of columns) {
		xByColumn.set(column, cursor);
		cursor += (widthByColumn.get(column) ?? 0) + cfg.hGap;
	}

	for (const id of component.nodeIds) {
		const node = graph.nodes.get(id);
		if (node) node.x = xByColumn.get(node.column) ?? 0;
	}
	return xByColumn;
}

export function nodesByColumn(
	graph: LGraph,
	component: ComponentInfo,
): Map<number, LNode[]> {
	const grouped = new Map<number, LNode[]>();
	for (const id of [...component.nodeIds].sort((a, b) => a.localeCompare(b))) {
		const node = graph.nodes.get(id);
		if (!node) continue;
		const bucket = grouped.get(node.column) ?? [];
		bucket.push(node);
		grouped.set(node.column, bucket);
	}
	return grouped;
}
