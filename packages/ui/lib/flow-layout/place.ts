import type { ComponentInfo } from "./build";
import { nodesByColumn } from "./rank";
import type { LGraph, LNode, StyleConfig } from "./types";

function primaryIncomingY(graph: LGraph, node: LNode): number | null {
	const candidates = node.in
		.filter((edge) => !edge.reversed)
		.sort((a, b) => {
			if (a.kind !== b.kind) return a.kind === "exec" ? -1 : 1;
			return (
				a.toPin.index - b.toPin.index ||
				a.fromPin.index - b.fromPin.index ||
				a.from.localeCompare(b.from)
			);
		});

	for (const edge of candidates) {
		const source = graph.nodes.get(edge.from);
		if (!source || !source.placed) continue;
		// Straighten at the pin, not at the node's top edge or centre: a chain of
		// nodes with different pin counts still comes out as one flat wire.
		return source.y + edge.fromPin.offsetY - edge.toPin.offsetY;
	}
	return null;
}

/**
 * Vertical placement in three passes:
 *   1. the exec spine, straightened at the pins and packed per column
 *   2. pure clusters, banded below the exec node that consumes them
 *   3. one monotone downward sweep per column
 *
 * Because column x is derived from the widest node in each column, two nodes in
 * different columns can never overlap horizontally. Overlap resolution is
 * therefore exactly a per-column vertical problem, and pass 3 solves it
 * completely in a single top-to-bottom sweep.
 */
export function placeComponent(
	graph: LGraph,
	component: ComponentInfo,
	cfg: StyleConfig,
): void {
	const byColumn = nodesByColumn(graph, component);
	const columns = [...byColumn.keys()].sort((a, b) => a - b);

	for (const id of component.nodeIds) {
		const node = graph.nodes.get(id);
		if (node) {
			node.placed = false;
			node.y = 0;
		}
	}

	// Pass 1 — the exec spine, left to right. Reroutes are excluded: they are
	// 16x12 dots that exist to sit ON a wire, so paying a full vGap for them
	// would drag them off the line they route.
	const spineBottom = new Map<number, number>();
	for (const column of columns) {
		const spine = (byColumn.get(column) ?? [])
			.filter((node) => node.kind !== "pure" && node.kind !== "reroute")
			.sort((a, b) => a.order - b.order || a.id.localeCompare(b.id));

		let cursor = Number.NEGATIVE_INFINITY;
		for (const node of spine) {
			const desired = primaryIncomingY(graph, node) ?? 0;
			node.y = Number.isFinite(cursor)
				? Math.max(desired, cursor + cfg.vGap)
				: desired;
			node.placed = true;
			cursor = node.y + node.height;
		}
		if (Number.isFinite(cursor)) spineBottom.set(column, cursor);
	}

	// Pass 2 — pure clusters hang in a band below their owner. Bands are
	// allocated contiguously per column in owner order, so two owners' clusters
	// can never interleave, and every band starts below that column's spine.
	for (const column of columns) {
		const pures = (byColumn.get(column) ?? []).filter(
			(node) => node.kind === "pure",
		);
		if (pures.length === 0) continue;

		const byOwner = new Map<string, LNode[]>();
		for (const node of pures) {
			const key = node.owner ?? "";
			const members = byOwner.get(key) ?? [];
			members.push(node);
			byOwner.set(key, members);
		}

		const ownerKeys = [...byOwner.keys()].sort((a, b) => {
			const ownerA = graph.nodes.get(a);
			const ownerB = graph.nodes.get(b);
			return (ownerA?.y ?? 0) - (ownerB?.y ?? 0) || a.localeCompare(b);
		});

		let cursor = spineBottom.get(column) ?? Number.NEGATIVE_INFINITY;
		for (const key of ownerKeys) {
			const owner = graph.nodes.get(key);
			const bandTop = owner
				? owner.y + owner.height + cfg.pureVGap
				: (primaryIncomingY(graph, byOwner.get(key)?.[0] as LNode) ?? 0);

			let y = Number.isFinite(cursor)
				? Math.max(bandTop, cursor + cfg.vGap)
				: bandTop;

			const members = (byOwner.get(key) ?? []).sort((a, b) => {
				const [orderA, pinA] = consumerSortKey(graph, a);
				const [orderB, pinB] = consumerSortKey(graph, b);
				return (
					b.depth - a.depth ||
					orderA - orderB ||
					pinA - pinB ||
					a.id.localeCompare(b.id)
				);
			});

			for (const node of members) {
				node.y = y;
				node.placed = true;
				y += node.height + cfg.vGap;
			}
			cursor = y - cfg.vGap;
		}
	}

	// Reroutes ride their wire. Parking them in the gutter between columns keeps
	// them off the column's occupancy problem entirely, so they never displace a
	// real node and never get displaced by one.
	placeReroutes(graph, byColumn, columns, cfg);

	// Pass 3 — monotone downward sweep over the real nodes. Never moves anything
	// up, so it cannot undo the straightening above a collision and it converges
	// in a single pass.
	for (const column of columns) {
		const members = (byColumn.get(column) ?? [])
			.filter((node) => node.kind !== "reroute" || !node.parked)
			.sort(
				(a, b) => a.y - b.y || a.order - b.order || a.id.localeCompare(b.id),
			);

		let bottom = Number.NEGATIVE_INFINITY;
		for (const node of members) {
			if (Number.isFinite(bottom)) {
				node.y = Math.max(node.y, bottom + cfg.vGap);
			}
			bottom = node.y + node.height;
		}
	}
}

/**
 * The vertical order of a node's pure inputs should follow the order of the
 * pins they feed, not the alphabetical order of their ids — otherwise random
 * cuid2 ids braid the data wires.
 */
function consumerSortKey(graph: LGraph, node: LNode): [number, number] {
	let bestOrder = Number.POSITIVE_INFINITY;
	let bestPin = Number.POSITIVE_INFINITY;
	for (const edge of node.out) {
		if (edge.reversed) continue;
		const target = graph.nodes.get(edge.to);
		if (!target) continue;
		if (
			target.order < bestOrder ||
			(target.order === bestOrder && edge.toPin.index < bestPin)
		) {
			bestOrder = target.order;
			bestPin = edge.toPin.index;
		}
	}
	return [bestOrder, bestPin];
}

function placeReroutes(
	graph: LGraph,
	byColumn: Map<number, LNode[]>,
	columns: number[],
	cfg: StyleConfig,
): void {
	for (const column of columns) {
		const members = byColumn.get(column) ?? [];
		const reroutes = members.filter((node) => node.kind === "reroute");
		if (reroutes.length === 0) continue;

		const others = members.filter((node) => node.kind !== "reroute");
		for (const node of reroutes) {
			node.y = primaryIncomingY(graph, node) ?? node.y;
			node.placed = true;
		}

		// A column of nothing but reroutes already has the canvas to itself.
		if (others.length === 0) continue;

		const columnWidth = Math.max(...others.map((node) => node.width));
		for (const node of reroutes) {
			node.parked = true;
			node.x = node.x + columnWidth + (cfg.hGap - node.width) / 2;
		}

		// Two wires can route through the same gutter at the same height.
		const ordered = reroutes
			.slice()
			.sort((a, b) => a.y - b.y || a.id.localeCompare(b.id));
		let bottom = Number.NEGATIVE_INFINITY;
		for (const node of ordered) {
			if (Number.isFinite(bottom)) {
				node.y = Math.max(node.y, bottom + cfg.pureVGap);
			}
			bottom = node.y + node.height;
		}
	}
}

export interface Bounds {
	minX: number;
	minY: number;
	maxX: number;
	maxY: number;
}

export function componentBounds(
	graph: LGraph,
	nodeIds: readonly string[],
): Bounds {
	let minX = Number.POSITIVE_INFINITY;
	let minY = Number.POSITIVE_INFINITY;
	let maxX = Number.NEGATIVE_INFINITY;
	let maxY = Number.NEGATIVE_INFINITY;

	for (const id of nodeIds) {
		const node = graph.nodes.get(id);
		if (!node) continue;
		minX = Math.min(minX, node.x);
		minY = Math.min(minY, node.y);
		maxX = Math.max(maxX, node.x + node.width);
		maxY = Math.max(maxY, node.y + node.height);
	}

	if (!Number.isFinite(minX)) return { minX: 0, minY: 0, maxX: 0, maxY: 0 };
	return { minX, minY, maxX, maxY };
}

export function translateNodes(
	graph: LGraph,
	nodeIds: readonly string[],
	dx: number,
	dy: number,
): void {
	if (dx === 0 && dy === 0) return;
	for (const id of nodeIds) {
		const node = graph.nodes.get(id);
		if (!node) continue;
		node.x += dx;
		node.y += dy;
	}
}
