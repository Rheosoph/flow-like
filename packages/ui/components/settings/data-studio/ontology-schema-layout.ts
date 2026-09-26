export interface SchemaLayoutNode {
	id: string;
	width: number;
	height: number;
}

export interface SchemaLayoutEdge {
	/** Keys `lanes` in the result; defaults to the edge's index. */
	id?: string;
	source: string;
	target: string;
}

export interface SchemaLayoutOptions {
	columnGap?: number;
	rowGap?: number;
	sectionGap?: number;
	laneHeight?: number;
}

export interface SchemaPoint {
	x: number;
	y: number;
}

export interface SchemaLayout {
	/** Top-left corner of every object. */
	positions: Map<string, SchemaPoint>;
	/** For links that skip columns: the centre of the gap reserved for them. */
	lanes: Map<string, SchemaPoint>;
}

const ORDER_SWEEPS = 4;

/**
 * Layered left-to-right layout for small schema graphs. Linked objects are
 * ranked by longest path over the graph with cycles broken in input order,
 * ordered within a column by barycenter sweeps, and centred per column. A link
 * that skips columns reserves a lane in each one it crosses, so it has a gap
 * to run through instead of passing under a card. Objects without links go
 * into a grid below. Output depends only on input order, never on existing
 * coordinates.
 */
export function layoutSchema(
	nodes: readonly SchemaLayoutNode[],
	edges: readonly SchemaLayoutEdge[],
	options: SchemaLayoutOptions = {},
): SchemaLayout {
	const columnGap = options.columnGap ?? 120;
	const rowGap = options.rowGap ?? 40;
	const sectionGap = options.sectionGap ?? 72;
	const laneHeight = options.laneHeight ?? 24;

	const indexOf = new Map(nodes.map((node, index) => [node.id, index]));
	const links = edges
		.map((edge, index) => ({ ...edge, id: edge.id ?? String(index) }))
		.filter(
			(edge) =>
				edge.source !== edge.target &&
				indexOf.has(edge.source) &&
				indexOf.has(edge.target),
		);
	const linked = new Set(links.flatMap((edge) => [edge.source, edge.target]));
	const connected = nodes.filter((node) => linked.has(node.id));
	const isolated = nodes.filter((node) => !linked.has(node.id));

	const positions = new Map<string, SchemaPoint>();
	const lanes = new Map<string, SchemaPoint>();
	let top = 0;

	if (connected.length > 0) {
		const rank = rankNodes(connected, links);
		const { items, segments, laneOf } = reserveLanes(
			connected,
			links,
			rank,
			laneHeight,
		);
		const layers = orderLayers(items, rank, segments);
		const byId = new Map(items.map((item) => [item.id, item]));
		const columnHeights = layers.map((layer) =>
			layer.reduce(
				(sum, id, index) =>
					sum + (byId.get(id)?.height ?? 0) + (index > 0 ? rowGap : 0),
				0,
			),
		);
		const blockHeight = Math.max(...columnHeights);
		const placed = new Map<string, SchemaPoint>();
		let x = 0;
		for (const [layerIndex, layer] of layers.entries()) {
			const columnWidth = Math.max(
				...layer.map((id) => byId.get(id)?.width ?? 0),
			);
			let y = (blockHeight - columnHeights[layerIndex]) / 2;
			for (const id of layer) {
				const item = byId.get(id);
				if (!item) continue;
				placed.set(id, {
					x: Math.round(item.lane ? x + columnWidth / 2 : x),
					y: Math.round(item.lane ? y + item.height / 2 : y),
				});
				y += item.height + rowGap;
			}
			x += columnWidth + columnGap;
		}
		for (const node of connected) {
			const point = placed.get(node.id);
			if (point) positions.set(node.id, point);
		}
		for (const [edgeId, laneId] of laneOf) {
			const point = placed.get(laneId);
			if (point) lanes.set(edgeId, point);
		}
		top = blockHeight + sectionGap;
	}

	if (isolated.length > 0) {
		const columns = Math.max(1, Math.ceil(Math.sqrt(isolated.length * 1.6)));
		const cellWidth =
			Math.max(...isolated.map((node) => node.width)) + columnGap;
		for (let start = 0; start < isolated.length; start += columns) {
			const row = isolated.slice(start, start + columns);
			for (const [column, node] of row.entries()) {
				positions.set(node.id, {
					x: Math.round(column * cellWidth),
					y: Math.round(top),
				});
			}
			top += Math.max(...row.map((node) => node.height)) + rowGap;
		}
	}

	return { positions, lanes };
}

interface LayoutItem extends SchemaLayoutNode {
	lane?: boolean;
}

/**
 * Splits every link that spans more than one column into a chain through one
 * lane placeholder per skipped column. Placeholders are added to `rank` and
 * then ordered and stacked like objects; the middle one is the link's waypoint.
 */
function reserveLanes(
	nodes: readonly SchemaLayoutNode[],
	links: readonly Required<SchemaLayoutEdge>[],
	rank: Map<string, number>,
	laneHeight: number,
): {
	items: LayoutItem[];
	segments: SchemaLayoutEdge[];
	laneOf: Map<string, string>;
} {
	const items: LayoutItem[] = [...nodes];
	const segments: SchemaLayoutEdge[] = [];
	const laneOf = new Map<string, string>();
	for (const link of links) {
		const from = rank.get(link.source) ?? 0;
		const to = rank.get(link.target) ?? 0;
		const step = to > from ? 1 : -1;
		if (Math.abs(to - from) < 2) {
			segments.push(link);
			continue;
		}
		const chain: string[] = [];
		for (let layer = from + step; layer !== to; layer += step) {
			const id = `\u0000lane:${link.id}:${layer}`;
			rank.set(id, layer);
			items.push({ id, width: 0, height: laneHeight, lane: true });
			chain.push(id);
		}
		laneOf.set(link.id, chain[Math.floor((chain.length - 1) / 2)]);
		const path = [link.source, ...chain, link.target];
		for (let index = 1; index < path.length; index += 1) {
			segments.push({ source: path[index - 1], target: path[index] });
		}
	}
	return { items, segments, laneOf };
}

function rankNodes(
	nodes: readonly SchemaLayoutNode[],
	links: readonly SchemaLayoutEdge[],
): Map<string, number> {
	const order = new Map(nodes.map((node, index) => [node.id, index]));
	const outgoing = new Map<string, string[]>(
		nodes.map((node) => [node.id, []]),
	);
	for (const link of links) outgoing.get(link.source)?.push(link.target);
	for (const targets of outgoing.values()) {
		targets.sort((a, b) => (order.get(a) ?? 0) - (order.get(b) ?? 0));
	}

	// Depth-first in input order; an edge into a node still on the stack closes
	// a cycle and is left out of ranking so every pass below runs on a DAG.
	const state = new Map<string, "active" | "done">();
	const forward: SchemaLayoutEdge[] = [];
	for (const root of nodes) {
		if (state.has(root.id)) continue;
		const stack: { id: string; next: number }[] = [{ id: root.id, next: 0 }];
		state.set(root.id, "active");
		while (stack.length > 0) {
			const frame = stack[stack.length - 1];
			const targets = outgoing.get(frame.id) ?? [];
			if (frame.next >= targets.length) {
				state.set(frame.id, "done");
				stack.pop();
				continue;
			}
			const target = targets[frame.next];
			frame.next += 1;
			const targetState = state.get(target);
			if (targetState === "active") continue;
			forward.push({ source: frame.id, target });
			if (!targetState) {
				state.set(target, "active");
				stack.push({ id: target, next: 0 });
			}
		}
	}

	const incoming = new Map<string, number>(nodes.map((node) => [node.id, 0]));
	const successors = new Map<string, string[]>(
		nodes.map((node) => [node.id, []]),
	);
	for (const link of forward) {
		successors.get(link.source)?.push(link.target);
		incoming.set(link.target, (incoming.get(link.target) ?? 0) + 1);
	}

	const rank = new Map<string, number>(nodes.map((node) => [node.id, 0]));
	const ready = nodes
		.filter((node) => (incoming.get(node.id) ?? 0) === 0)
		.map((node) => node.id);
	while (ready.length > 0) {
		const id = ready.shift() as string;
		for (const target of successors.get(id) ?? []) {
			rank.set(
				target,
				Math.max(rank.get(target) ?? 0, (rank.get(id) ?? 0) + 1),
			);
			const remaining = (incoming.get(target) ?? 0) - 1;
			incoming.set(target, remaining);
			if (remaining === 0) ready.push(target);
		}
	}

	// Pull pure sources right so they sit next to the nodes they point at
	// rather than all stacking up in the first column.
	for (const node of [...nodes].reverse()) {
		const targets = successors.get(node.id) ?? [];
		if (targets.length === 0) continue;
		const hasPredecessor = forward.some((link) => link.target === node.id);
		if (hasPredecessor) continue;
		const nearest = Math.min(...targets.map((target) => rank.get(target) ?? 0));
		rank.set(node.id, Math.max(rank.get(node.id) ?? 0, nearest - 1));
	}

	return rank;
}

function orderLayers(
	nodes: readonly SchemaLayoutNode[],
	rank: Map<string, number>,
	links: readonly SchemaLayoutEdge[],
): string[][] {
	const depth = Math.max(...nodes.map((node) => rank.get(node.id) ?? 0)) + 1;
	const layers: string[][] = Array.from({ length: depth }, () => []);
	for (const node of nodes) layers[rank.get(node.id) ?? 0].push(node.id);

	const neighbours = new Map<string, string[]>(
		nodes.map((node) => [node.id, []]),
	);
	for (const link of links) {
		neighbours.get(link.source)?.push(link.target);
		neighbours.get(link.target)?.push(link.source);
	}

	const slot = new Map<string, number>();
	const refreshSlots = () => {
		for (const layer of layers) {
			for (const [index, id] of layer.entries()) {
				slot.set(id, (index + 0.5) / layer.length);
			}
		}
	};
	refreshSlots();

	const sortLayer = (layer: string[], towards: (other: number) => boolean) => {
		const current = new Map(layer.map((id, index) => [id, index]));
		const weight = new Map(
			layer.map((id) => {
				const anchors = (neighbours.get(id) ?? []).filter((other) =>
					towards(rank.get(other) ?? 0),
				);
				if (anchors.length === 0) return [id, slot.get(id) ?? 0];
				const sum = anchors.reduce(
					(total, other) => total + (slot.get(other) ?? 0),
					0,
				);
				return [id, sum / anchors.length];
			}),
		);
		layer.sort(
			(a, b) =>
				(weight.get(a) ?? 0) - (weight.get(b) ?? 0) ||
				(current.get(a) ?? 0) - (current.get(b) ?? 0),
		);
	};

	for (let sweep = 0; sweep < ORDER_SWEEPS; sweep += 1) {
		for (let layerRank = 1; layerRank < depth; layerRank += 1) {
			sortLayer(layers[layerRank], (other) => other < layerRank);
			refreshSlots();
		}
		for (let layerRank = depth - 2; layerRank >= 0; layerRank -= 1) {
			sortLayer(layers[layerRank], (other) => other > layerRank);
			refreshSlots();
		}
	}

	return layers.filter((layer) => layer.length > 0);
}
