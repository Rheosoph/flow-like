import type { ComponentInfo } from "./build";
import type { LEdge, LGraph } from "./types";

function traversalEdges(edges: LEdge[]): LEdge[] {
	return edges
		.filter((edge) => !edge.reversed)
		.slice()
		.sort((a, b) => {
			if (a.kind !== b.kind) return a.kind === "exec" ? -1 : 1;
			return (
				a.fromPin.index - b.fromPin.index ||
				a.toPin.index - b.toPin.index ||
				a.to.localeCompare(b.to)
			);
		});
}

/**
 * Reading order within a column comes from a pin-ordered DFS, not from a
 * barycenter sweep or the node's previous coordinates.
 *
 * The practical effect: a Branch renders True above False because True is the
 * first exec output pin, and adding a node elsewhere on the board cannot
 * permute an existing fan-out.
 */
export function assignOrder(graph: LGraph, components: ComponentInfo[]): void {
	let sequence = 0;
	const visited = new Set<string>();

	const visit = (startId: string) => {
		if (visited.has(startId)) return;

		const stack: Array<{ id: string; edges: LEdge[]; next: number }> = [];
		visited.add(startId);
		const startNode = graph.nodes.get(startId);
		if (!startNode) return;
		startNode.order = sequence++;
		stack.push({ id: startId, edges: traversalEdges(startNode.out), next: 0 });

		while (stack.length > 0) {
			const frame = stack[stack.length - 1];
			if (frame.next >= frame.edges.length) {
				stack.pop();
				continue;
			}
			const edge = frame.edges[frame.next];
			frame.next += 1;
			if (visited.has(edge.to)) continue;

			const target = graph.nodes.get(edge.to);
			if (!target) continue;
			visited.add(edge.to);
			target.order = sequence++;
			stack.push({ id: edge.to, edges: traversalEdges(target.out), next: 0 });
		}
	};

	for (const component of components) {
		for (const root of component.roots) visit(root);
		for (const id of [...component.nodeIds].sort((a, b) =>
			a.localeCompare(b),
		)) {
			visit(id);
		}
	}
}
