import type Graph from "graphology";

/** Matches the size a node gets when no style override applies. */
export const DEFAULT_NODE_SIZE = 10;
/** Breathing room, in layout units, kept between two node circles. */
export const NODE_GAP = 8;
/** Share of an overlap resolved per relaxation pass. */
const RELAX_STRENGTH = 0.55;
/** Ceiling on pair tests per pass so degenerate inputs cannot lock the frame. */
const MAX_PAIR_CHECKS = 1_200_000;
/** Target area utilisation when seeding nodes on a disc. */
const SEED_PACKING_FRACTION = 0.55;
const TAU = Math.PI * 2;

export interface LayoutPosition {
	x: number;
	y: number;
}

export interface LayoutBounds {
	minX: number;
	minY: number;
	maxX: number;
	maxY: number;
	width: number;
	height: number;
	centerX: number;
	centerY: number;
}

export interface ConnectivityPartition {
	/** Nodes with at least one edge — the only ones a force layout can place. */
	connected: string[];
	/** Detached nodes, sorted so their arrangement is stable across rebuilds. */
	isolated: string[];
}

export function hashLayoutSeed(seed: string): number {
	let hash = 2166136261;
	for (let index = 0; index < seed.length; index += 1) {
		hash ^= seed.charCodeAt(index);
		hash = Math.imul(hash, 16777619);
	}
	return hash >>> 0;
}

/**
 * Radius of the disc that fits `nodeCount` circles without crowding. Seeding
 * across it means the force layout starts from a spread-out cloud instead of
 * an already-collapsed blob it would have to untangle.
 */
export function computeSeedSpread(
	nodeCount: number,
	nodeSize: number = DEFAULT_NODE_SIZE,
): number {
	const packRadius = nodeSize + NODE_GAP / 2;
	return Math.max(
		60,
		packRadius * Math.sqrt(Math.max(1, nodeCount) / SEED_PACKING_FRACTION),
	);
}

export function createDeterministicPosition(
	seed: string,
	spread: number,
): LayoutPosition {
	const hash = hashLayoutSeed(seed);
	const angle = (((hash >>> 8) % 3600) / 3600) * TAU;
	const radius = spread * Math.sqrt((hash & 0xffff) / 0xffff);
	return { x: Math.cos(angle) * radius, y: Math.sin(angle) * radius };
}

export function createAnchoredPosition(
	anchor: LayoutPosition,
	seed: string,
	spread: number,
): LayoutPosition {
	const hash = hashLayoutSeed(seed);
	const angle = (((hash >>> 8) % 3600) / 3600) * TAU;
	const radius = spread * (0.4 + ((hash & 0xff) / 0xff) * 0.6);
	return {
		x: anchor.x + Math.cos(angle) * radius,
		y: anchor.y + Math.sin(angle) * radius,
	};
}

export function partitionByConnectivity(graph: Graph): ConnectivityPartition {
	const connected: string[] = [];
	const isolated: string[] = [];
	graph.forEachNode((nodeId) => {
		if (graph.degree(nodeId) > 0) connected.push(nodeId);
		else isolated.push(nodeId);
	});
	isolated.sort();
	return { connected, isolated };
}

function readRadius(graph: Graph, nodeId: string): number {
	const size = graph.getNodeAttribute(nodeId, "size");
	return typeof size === "number" && Number.isFinite(size) && size > 0
		? size
		: DEFAULT_NODE_SIZE;
}

function readCoordinate(graph: Graph, nodeId: string, key: "x" | "y"): number {
	const value = graph.getNodeAttribute(nodeId, key);
	return typeof value === "number" && Number.isFinite(value) ? value : 0;
}

function maxRadiusOf(graph: Graph, nodeIds: readonly string[]): number {
	let maxRadius = 0;
	for (const nodeId of nodeIds) {
		maxRadius = Math.max(maxRadius, readRadius(graph, nodeId));
	}
	return maxRadius || DEFAULT_NODE_SIZE;
}

export function getLayoutBounds(
	graph: Graph,
	nodeIds: readonly string[],
): LayoutBounds | null {
	if (nodeIds.length === 0) return null;

	let minX = Number.POSITIVE_INFINITY;
	let minY = Number.POSITIVE_INFINITY;
	let maxX = Number.NEGATIVE_INFINITY;
	let maxY = Number.NEGATIVE_INFINITY;

	for (const nodeId of nodeIds) {
		if (!graph.hasNode(nodeId)) continue;
		const radius = readRadius(graph, nodeId);
		const x = readCoordinate(graph, nodeId, "x");
		const y = readCoordinate(graph, nodeId, "y");
		minX = Math.min(minX, x - radius);
		minY = Math.min(minY, y - radius);
		maxX = Math.max(maxX, x + radius);
		maxY = Math.max(maxY, y + radius);
	}

	if (!Number.isFinite(minX)) return null;

	return {
		minX,
		minY,
		maxX,
		maxY,
		width: maxX - minX,
		height: maxY - minY,
		centerX: (minX + maxX) / 2,
		centerY: (minY + maxY) / 2,
	};
}

/** Cheap spatial-hash key. Collisions only cost extra distance tests. */
function cellKey(cellX: number, cellY: number): number {
	return (Math.imul(cellX, 73856093) ^ Math.imul(cellY, 19349663)) | 0;
}

export function defaultRelaxIterations(nodeCount: number): number {
	if (nodeCount >= 10000) return 12;
	if (nodeCount >= 2000) return 24;
	return 60;
}

export interface RelaxOverlapsOptions {
	iterations?: number;
	gap?: number;
	maxPairChecks?: number;
}

/**
 * Pushes overlapping nodes apart until every pair clears `radius + radius + gap`.
 *
 * Force layouts only approach that separation asymptotically, so a graph always
 * ships with some overlap when the iteration budget runs out. This pass closes
 * the gap deterministically in bounded time, which is what makes node labels and
 * icons readable regardless of how long the simulation was allowed to run.
 */
export function relaxOverlaps(
	graph: Graph,
	nodeIds: readonly string[],
	options: RelaxOverlapsOptions = {},
): number {
	const count = nodeIds.length;
	if (count < 2) return 0;

	const gap = options.gap ?? NODE_GAP;
	const iterations = options.iterations ?? defaultRelaxIterations(count);
	const pairBudget = options.maxPairChecks ?? MAX_PAIR_CHECKS;

	const xs = new Float64Array(count);
	const ys = new Float64Array(count);
	const radii = new Float64Array(count);
	let maxRadius = 0;

	for (let index = 0; index < count; index += 1) {
		const nodeId = nodeIds[index];
		xs[index] = readCoordinate(graph, nodeId, "x");
		ys[index] = readCoordinate(graph, nodeId, "y");
		radii[index] = readRadius(graph, nodeId);
		maxRadius = Math.max(maxRadius, radii[index]);
	}

	const cellSize = 2 * maxRadius + gap;
	const buckets = new Map<number, number[]>();
	let performed = 0;

	for (let pass = 0; pass < iterations; pass += 1) {
		buckets.clear();
		for (let index = 0; index < count; index += 1) {
			const key = cellKey(
				Math.floor(xs[index] / cellSize),
				Math.floor(ys[index] / cellSize),
			);
			const bucket = buckets.get(key);
			if (bucket) bucket.push(index);
			else buckets.set(key, [index]);
		}

		let checks = 0;
		let resolved = 0;

		for (let i = 0; i < count && checks < pairBudget; i += 1) {
			const cellX = Math.floor(xs[i] / cellSize);
			const cellY = Math.floor(ys[i] / cellSize);

			for (let offsetX = -1; offsetX <= 1; offsetX += 1) {
				for (let offsetY = -1; offsetY <= 1; offsetY += 1) {
					const bucket = buckets.get(cellKey(cellX + offsetX, cellY + offsetY));
					if (!bucket) continue;

					for (const j of bucket) {
						if (j <= i) continue;
						checks += 1;

						const deltaX = xs[j] - xs[i];
						const deltaY = ys[j] - ys[i];
						const minDistance = radii[i] + radii[j] + gap;
						const squared = deltaX * deltaX + deltaY * deltaY;
						if (squared >= minDistance * minDistance) continue;

						const distance = Math.sqrt(squared);
						let normalX: number;
						let normalY: number;
						if (distance < 1e-6) {
							const angle = ((hashLayoutSeed(`${i}:${j}`) % 3600) / 3600) * TAU;
							normalX = Math.cos(angle);
							normalY = Math.sin(angle);
						} else {
							normalX = deltaX / distance;
							normalY = deltaY / distance;
						}

						const shift = (minDistance - distance) * RELAX_STRENGTH * 0.5;
						xs[i] -= normalX * shift;
						ys[i] -= normalY * shift;
						xs[j] += normalX * shift;
						ys[j] += normalY * shift;
						resolved += 1;
					}
				}
			}
		}

		performed += 1;
		if (resolved === 0) break;
	}

	for (let index = 0; index < count; index += 1) {
		graph.setNodeAttribute(nodeIds[index], "x", xs[index]);
		graph.setNodeAttribute(nodeIds[index], "y", ys[index]);
	}

	return performed;
}

export interface GridLayoutOptions {
	gap?: number;
	columns?: number;
	centerX?: number;
	centerY?: number;
}

/** Row-major grid placement. Never overlaps, needs no simulation. */
export function packNodesOnGrid(
	graph: Graph,
	nodeIds: readonly string[],
	options: GridLayoutOptions = {},
): LayoutBounds | null {
	if (nodeIds.length === 0) return null;

	const gap = options.gap ?? NODE_GAP;
	const spacing = 2 * maxRadiusOf(graph, nodeIds) + gap;
	const columns = Math.max(
		1,
		options.columns ?? Math.ceil(Math.sqrt(nodeIds.length)),
	);
	const rows = Math.ceil(nodeIds.length / columns);
	const startX = (options.centerX ?? 0) - ((columns - 1) * spacing) / 2;
	const startY = (options.centerY ?? 0) - ((rows - 1) * spacing) / 2;

	for (let index = 0; index < nodeIds.length; index += 1) {
		graph.setNodeAttribute(
			nodeIds[index],
			"x",
			startX + (index % columns) * spacing,
		);
		graph.setNodeAttribute(
			nodeIds[index],
			"y",
			startY + Math.floor(index / columns) * spacing,
		);
	}

	return getLayoutBounds(graph, nodeIds);
}

/**
 * Parks detached nodes in a band beside the connected core. A force layout has
 * no information to place them with, so left in the simulation they collapse
 * into the gravity well and bury the part of the graph that has structure.
 */
export function placeDetachedNodes(
	graph: Graph,
	isolated: readonly string[],
	coreBounds: LayoutBounds | null,
	gap: number = NODE_GAP,
): LayoutBounds | null {
	if (isolated.length === 0) return null;

	const spacing = 2 * maxRadiusOf(graph, isolated) + gap;

	if (!coreBounds) {
		return packNodesOnGrid(graph, isolated, { gap });
	}

	const rows = Math.max(
		1,
		Math.min(isolated.length, Math.round(coreBounds.height / spacing) || 1),
	);
	const columns = Math.ceil(isolated.length / rows);
	const bandWidth = (columns - 1) * spacing;

	return packNodesOnGrid(graph, isolated, {
		gap,
		columns,
		centerX: coreBounds.maxX + spacing * 2 + bandWidth / 2,
		centerY: coreBounds.centerY,
	});
}
