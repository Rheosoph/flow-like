import type { Edge, InternalNode, Node } from "@xyflow/react";

export const WARM_EDITOR_LIMIT = 256;

/** Small editor populations stay mounted so camera changes do not rebuild them. */
export function warmEditorIds(nodes: readonly Node[]): ReadonlySet<string> {
	const ordinary = nodes.filter(
		(node) => node.type === "node" || node.type === "flowNode",
	);
	return new Set(
		ordinary.length <= WARM_EDITOR_LIMIT ? ordinary.map((node) => node.id) : [],
	);
}

export interface DomVisibilityOptions {
	nodes: readonly Node[];
	edges: readonly Edge[];
	nodeLookup: ReadonlyMap<string, InternalNode>;
	transform: readonly [number, number, number];
	width: number;
	height: number;
	retained?: ReadonlySet<string>;
}

/** Keep a screen-space margin so rich editors mount before entering the camera. */
export function visibleDomIds({
	nodes,
	edges,
	nodeLookup,
	transform: [x, y, zoom],
	width,
	height,
	retained,
}: DomVisibilityOptions): { nodes: string[]; edges: string[] } {
	if (width <= 0 || height <= 0 || zoom <= 0)
		return {
			nodes: nodes.map((node) => node.id),
			edges: edges.map((edge) => edge.id),
		};
	const margin = 200;
	const view = {
		left: (-x - margin) / zoom,
		top: (-y - margin) / zoom,
		right: (width - x + margin) / zoom,
		bottom: (height - y + margin) / zoom,
	};
	const bounds = (id: string) => {
		const node = nodeLookup.get(id);
		const width = node?.measured.width ?? node?.width;
		const height = node?.measured.height ?? node?.height;
		if (!node || width === undefined || height === undefined) return;
		return {
			left: node.internals.positionAbsolute.x,
			top: node.internals.positionAbsolute.y,
			right: node.internals.positionAbsolute.x + width,
			bottom: node.internals.positionAbsolute.y + height,
		};
	};
	const intersects = (rect: typeof view) =>
		rect.left <= view.right &&
		rect.right >= view.left &&
		rect.top <= view.bottom &&
		rect.bottom >= view.top;
	return {
		nodes: nodes
			.filter((node) => {
				if (
					node.selected ||
					node.dragging ||
					node.data.__flowNativeHovered ||
					retained?.has(node.id)
				)
					return true;
				const rect = bounds(node.id);
				return !rect || intersects(rect);
			})
			.map((node) => node.id),
		edges: edges
			.filter((edge) => {
				if (edge.selected || retained?.has(edge.id)) return true;
				const source = bounds(edge.source);
				const target = bounds(edge.target);
				if (!source || !target) return true;
				// Custom paths can bend beyond either endpoint. A generous world-space
				// margin retains those curves while culling distant SVG-only regions.
				const padding = Math.max(
					300,
					Math.abs(source.left - target.left) / 2,
					Math.abs(source.top - target.top) / 2,
				);
				return intersects({
					left: Math.min(source.left, target.left) - padding,
					top: Math.min(source.top, target.top) - padding,
					right: Math.max(source.right, target.right) + padding,
					bottom: Math.max(source.bottom, target.bottom) + padding,
				});
			})
			.map((edge) => edge.id),
	};
}

/** React Flow's floor from before the native canvas; it never zooms out further. */
export const DOM_RENDERER_MIN_ZOOM = 0.1;
/**
 * Rich editors React Flow may mount at once. Each costs about half a megabyte of
 * renderer memory; 2,000 of them get the tab OOM-killed on a small machine.
 */
export const DOM_RENDERER_NODE_BUDGET = 400;

// Cell (x, y) is keyed (x + offset) · stride + y + offset, so x and y shift by arithmetic.
const CELL_OFFSET = 2 ** 20;
const CELL_STRIDE = 2 ** 21;

function addTo(sums: Map<number, number>, key: number, count: number) {
	const sum = (sums.get(key) ?? 0) + count;
	sums.set(key, sum);
	return sum;
}

function cellCounts(
	nodes: readonly Node[],
	cellWidth: number,
	cellHeight: number,
) {
	const cells = new Map<number, number>();
	for (const { position } of nodes)
		addTo(
			cells,
			(Math.floor(position.x / cellWidth) + CELL_OFFSET) * CELL_STRIDE +
				Math.floor(position.y / cellHeight) +
				CELL_OFFSET,
			1,
		);
	return cells;
}

function anyBlockExceeds(cells: Map<number, number>, budget: number) {
	const rows = new Map<number, number>();
	for (const [key, count] of cells)
		for (let dx = 0; dx < 3; dx++) addTo(rows, key - dx * CELL_STRIDE, count);
	const blocks = new Map<number, number>();
	for (const [key, count] of rows)
		for (let dy = 0; dy < 3; dy++)
			if (addTo(blocks, key - dy, count) > budget) return true;
	return false;
}

/**
 * The lowest zoom at which no screen-sized window can hold more than `budget` nodes.
 * Any such window lies inside a 3×3 block of half-screen cells, so block sums bound
 * it from above (by at most 2.25× on evenly spread boards).
 */
export function domRendererMinZoom(
	nodes: readonly Node[],
	width: number,
	height: number,
	budget = DOM_RENDERER_NODE_BUDGET,
) {
	if (nodes.length <= budget || width <= 0 || height <= 0)
		return DOM_RENDERER_MIN_ZOOM;
	const crowded = (zoom: number) =>
		anyBlockExceeds(
			cellCounts(nodes, width / zoom / 2, height / zoom / 2),
			budget,
		);
	let low = DOM_RENDERER_MIN_ZOOM;
	let high = 1;
	if (!crowded(low)) return low;
	if (crowded(high)) return high;
	for (let step = 0; step < 8; step++) {
		const middle = (low + high) / 2;
		if (crowded(middle)) low = middle;
		else high = middle;
	}
	return high;
}

export function equalVisibleIds(
	a: ReturnType<typeof visibleDomIds>,
	b: ReturnType<typeof visibleDomIds>,
) {
	return (
		a.nodes.length === b.nodes.length &&
		a.edges.length === b.edges.length &&
		a.nodes.every((id, index) => id === b.nodes[index]) &&
		a.edges.every((id, index) => id === b.edges[index])
	);
}
