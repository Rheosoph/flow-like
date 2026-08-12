import type Graph from "graphology";
import type { GraphCluster } from "./graph-clusters";

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
/** Widest the detached-node band may get relative to its height. */
const MAX_BAND_ASPECT = 2;
/** Column pitch relative to row pitch, leaving room for node captions. */
const LABEL_PITCH = 2.4;
/** Padding kept between a group's outermost node and the edge of its disc. */
export const CLUSTER_PADDING = 18;
/** Clearance between two group discs, wide enough to read as a boundary. */
export const CLUSTER_GAP = 26;
/** Vogel's angle: successive points never line up into visible spokes. */
export const GOLDEN_ANGLE = 2.39996323;
/** Relaxation passes per group — the seeded placements are already near-final. */
const CLUSTER_RELAX_ITERATIONS = 8;
/** Spiral probes tried before a disc is parked outside everything already placed. */
const MAX_PACK_PROBES = 20000;
/** Probe pitch relative to the disc being placed; finer probing packs tighter. */
const PACK_PROBE_FACTOR = 0.6;
/** Phyllotaxis radius growth relative to node pitch, tuned to leave no overlap. */
const PHYLLOTAXIS_PITCH_FACTOR = 0.62;
/** Groups laid out between frames. */
const CLUSTER_LAYOUT_CHUNK = 16;
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

/**
 * Row-major grid placement. Never overlaps, needs no simulation.
 *
 * Columns are pitched wider than rows because captions are drawn to the right
 * of a node: at circle-tight spacing every label runs over its neighbour, which
 * is unreadable exactly when a grid is used — a set of same-typed nodes whose
 * captions are the only thing telling them apart.
 */
export function packNodesOnGrid(
	graph: Graph,
	nodeIds: readonly string[],
	options: GridLayoutOptions = {},
): LayoutBounds | null {
	if (nodeIds.length === 0) return null;

	const gap = options.gap ?? NODE_GAP;
	const rowPitch = 2 * maxRadiusOf(graph, nodeIds) + gap;
	const columnPitch = rowPitch * LABEL_PITCH;
	// Fewer, wider columns keep the grid roughly square despite the pitch.
	const columns = Math.max(
		1,
		options.columns ?? Math.ceil(Math.sqrt(nodeIds.length / LABEL_PITCH)),
	);
	const rows = Math.ceil(nodeIds.length / columns);
	const startX = (options.centerX ?? 0) - ((columns - 1) * columnPitch) / 2;
	const startY = (options.centerY ?? 0) - ((rows - 1) * rowPitch) / 2;

	for (let index = 0; index < nodeIds.length; index += 1) {
		graph.setNodeAttribute(
			nodeIds[index],
			"x",
			startX + (index % columns) * columnPitch,
		);
		graph.setNodeAttribute(
			nodeIds[index],
			"y",
			startY + Math.floor(index / columns) * rowPitch,
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

	const rowPitch = 2 * maxRadiusOf(graph, isolated) + gap;
	const columnPitch = rowPitch * LABEL_PITCH;

	if (!coreBounds) {
		return packNodesOnGrid(graph, isolated, { gap });
	}

	// The band tracks the core's height so the two read as one composition, but
	// never gets so wide that auto-fit shrinks the core to a dot.
	const rows = Math.max(
		Math.ceil(Math.sqrt((isolated.length * LABEL_PITCH) / MAX_BAND_ASPECT)),
		Math.min(isolated.length, Math.round(coreBounds.height / rowPitch) || 1),
	);
	const columns = Math.ceil(isolated.length / rows);
	const bandWidth = (columns - 1) * columnPitch;

	return packNodesOnGrid(graph, isolated, {
		gap,
		columns,
		centerX: coreBounds.maxX + columnPitch + bandWidth / 2,
		centerY: coreBounds.centerY,
	});
}

function captionOf(graph: Graph, nodeId: string): string {
	const caption = graph.getNodeAttribute(nodeId, "label");
	return typeof caption === "string" ? caption : nodeId;
}

function compareIds(a: string, b: string): number {
	return a < b ? -1 : a > b ? 1 : 0;
}

/** (degree desc, caption asc, id asc), so a ring reads top-down whatever the input order. */
function sortByProminence(graph: Graph, nodeIds: readonly string[]): string[] {
	return [...nodeIds].sort(
		(a, b) =>
			graph.degree(b) - graph.degree(a) ||
			captionOf(graph, a).localeCompare(captionOf(graph, b)) ||
			compareIds(a, b),
	);
}

function translateNodes(
	graph: Graph,
	nodeIds: readonly string[],
	deltaX: number,
	deltaY: number,
): void {
	for (const nodeId of nodeIds) {
		graph.setNodeAttribute(
			nodeId,
			"x",
			readCoordinate(graph, nodeId, "x") + deltaX,
		);
		graph.setNodeAttribute(
			nodeId,
			"y",
			readCoordinate(graph, nodeId, "y") + deltaY,
		);
	}
}

export interface HubStarOptions {
	centerX?: number;
	centerY?: number;
	gap?: number;
	/** Rings are phase-shifted per group so adjacent stars never align into false rows. */
	seed?: string;
}

/**
 * Puts a parent at the centre of concentric rings of its children.
 *
 * The parent is the group's heading: it is the one caption worth reading, and a
 * ring makes the relationship legible without drawing a hull around anything.
 */
export function placeHubStar(
	graph: Graph,
	hubId: string,
	childIds: readonly string[],
	options: HubStarOptions = {},
): LayoutBounds | null {
	if (!graph.hasNode(hubId)) return null;

	const centerX = options.centerX ?? 0;
	const centerY = options.centerY ?? 0;
	const gap = options.gap ?? NODE_GAP;
	const seed = options.seed ?? hubId;
	const children = sortByProminence(
		graph,
		childIds.filter((childId) => childId !== hubId && graph.hasNode(childId)),
	);

	graph.setNodeAttribute(hubId, "x", centerX);
	graph.setNodeAttribute(hubId, "y", centerY);
	if (children.length === 0) return getLayoutBounds(graph, [hubId]);

	const childRadius = maxRadiusOf(graph, children);
	const pitch = 2 * childRadius + gap;
	const innerRadius = readRadius(graph, hubId) + gap + childRadius;
	let placed = 0;

	for (let ring = 0; placed < children.length; ring += 1) {
		const radius = innerRadius + ring * pitch;
		const slots = Math.max(1, Math.floor((TAU * radius) / pitch));
		const taken = Math.min(slots, children.length - placed);
		const phase = ((hashLayoutSeed(`${seed}:${ring}`) % 3600) / 3600) * TAU;

		for (let slot = 0; slot < taken; slot += 1) {
			const angle = phase + (slot / taken) * TAU;
			graph.setNodeAttribute(
				children[placed],
				"x",
				centerX + Math.cos(angle) * radius,
			);
			graph.setNodeAttribute(
				children[placed],
				"y",
				centerY + Math.sin(angle) * radius,
			);
			placed += 1;
		}
	}

	return getLayoutBounds(graph, [hubId, ...children]);
}

export interface PhyllotaxisOptions {
	centerX?: number;
	centerY?: number;
	gap?: number;
	pitch?: number;
}

/**
 * Sunflower placement in caller order. Density is uniform and there are no rows,
 * so the caller's ranking survives as plain distance from the centre.
 */
export function placePhyllotaxis(
	graph: Graph,
	memberIds: readonly string[],
	options: PhyllotaxisOptions = {},
): LayoutBounds | null {
	const members = memberIds.filter((nodeId) => graph.hasNode(nodeId));
	if (members.length === 0) return null;

	const centerX = options.centerX ?? 0;
	const centerY = options.centerY ?? 0;
	const gap = options.gap ?? NODE_GAP;
	const pitch =
		options.pitch ??
		(2 * maxRadiusOf(graph, members) + gap) * PHYLLOTAXIS_PITCH_FACTOR;

	for (let index = 0; index < members.length; index += 1) {
		const radius = pitch * Math.sqrt(index + 0.5);
		const angle = index * GOLDEN_ANGLE;
		graph.setNodeAttribute(
			members[index],
			"x",
			centerX + Math.cos(angle) * radius,
		);
		graph.setNodeAttribute(
			members[index],
			"y",
			centerY + Math.sin(angle) * radius,
		);
	}

	return getLayoutBounds(graph, members);
}

export interface ClusterDisc {
	id: string;
	radius: number;
	/** Objects the group stands for — decides which disc holds the centre of the stage. */
	represented: number;
	/** Member count, the tiebreak when two groups stand for the same population. */
	size: number;
}

/**
 * Places group discs largest-first on a golden-angle spiral of candidate centres.
 *
 * Ranking by population rather than by node count is what puts the part of the
 * ontology the reader should look at first in the middle of the stage.
 */
export function packClusterDiscs(
	discs: readonly ClusterDisc[],
	gap: number = CLUSTER_GAP,
): Map<string, LayoutPosition> {
	const ordered = [...discs].sort(
		(a, b) =>
			b.represented - a.represented ||
			b.size - a.size ||
			compareIds(a.id, b.id),
	);

	const placed: { x: number; y: number; radius: number }[] = [];
	const centers = new Map<string, LayoutPosition>();
	let frontier = 0;

	for (const disc of ordered) {
		const step = Math.max(1, (disc.radius + gap) * PACK_PROBE_FACTOR);
		let center: LayoutPosition | null = null;

		for (let probe = 0; probe < MAX_PACK_PROBES && !center; probe += 1) {
			const radius = step * Math.sqrt(probe);
			const angle = probe * GOLDEN_ANGLE;
			const x = Math.cos(angle) * radius;
			const y = Math.sin(angle) * radius;
			let clear = true;
			for (const other of placed) {
				const minDistance = other.radius + disc.radius + gap;
				const deltaX = other.x - x;
				const deltaY = other.y - y;
				if (deltaX * deltaX + deltaY * deltaY < minDistance * minDistance) {
					clear = false;
					break;
				}
			}
			if (clear) center = { x, y };
		}

		// Beyond the probe budget, parking past everything placed always clears.
		const resolved = center ?? { x: frontier + disc.radius + gap, y: 0 };
		placed.push({ ...resolved, radius: disc.radius });
		centers.set(disc.id, resolved);
		frontier = Math.max(
			frontier,
			Math.hypot(resolved.x, resolved.y) + disc.radius,
		);
	}

	return centers;
}

export interface ClusterLayoutOptions {
	gap?: number;
	clusterGap?: number;
	padding?: number;
	relaxIterations?: number;
	onProgress?: (fraction: number) => void;
	/** Hands the frame back so progress paints and the page stays responsive. */
	yieldToFrame?: () => Promise<void>;
	isCancelled?: () => boolean;
}

/** A group may name nodes the canvas dropped, so placement always works off this. */
function presentMembers(graph: Graph, cluster: GraphCluster): string[] {
	return cluster.memberIds.filter((nodeId) => graph.hasNode(nodeId));
}

/**
 * Lays out one group around its own origin and reports the disc it occupies.
 * Members are centred on (0, 0) so packing only has to add a translation.
 */
export function layoutCluster(
	graph: Graph,
	cluster: GraphCluster,
	options: ClusterLayoutOptions = {},
): ClusterDisc | null {
	const memberIds = presentMembers(graph, cluster);
	if (memberIds.length === 0) return null;

	const gap = options.gap ?? NODE_GAP;
	const hubId =
		cluster.hubId && graph.hasNode(cluster.hubId) ? cluster.hubId : null;

	if (hubId) {
		placeHubStar(
			graph,
			hubId,
			memberIds.filter((nodeId) => nodeId !== hubId),
			{ gap, seed: cluster.id },
		);
	} else {
		placePhyllotaxis(graph, memberIds, { gap });
	}

	relaxOverlaps(graph, memberIds, {
		gap,
		iterations: options.relaxIterations ?? CLUSTER_RELAX_ITERATIONS,
	});

	const bounds = getLayoutBounds(graph, memberIds);
	if (!bounds) return null;
	translateNodes(graph, memberIds, -bounds.centerX, -bounds.centerY);

	let radius = 0;
	for (const nodeId of memberIds) {
		const x = readCoordinate(graph, nodeId, "x");
		const y = readCoordinate(graph, nodeId, "y");
		radius = Math.max(radius, Math.hypot(x, y) + readRadius(graph, nodeId));
	}

	return {
		id: cluster.id,
		radius: radius + (options.padding ?? CLUSTER_PADDING),
		represented: cluster.represented,
		size: memberIds.length,
	};
}

/**
 * Lays out every group, then packs the groups themselves.
 *
 * Deliberately runs no global relaxation afterwards: a whole-graph pass would
 * smear exactly the boundaries between groups that carry the meaning here.
 */
export async function applyClusterLayout(
	graph: Graph,
	clusters: readonly GraphCluster[],
	options: ClusterLayoutOptions = {},
): Promise<LayoutBounds | null> {
	const discs: ClusterDisc[] = [];
	const membersByCluster = new Map<string, string[]>();

	for (let index = 0; index < clusters.length; index += 1) {
		const cluster = clusters[index];
		const disc = layoutCluster(graph, cluster, options);
		options.onProgress?.((index + 1) / clusters.length);
		if (disc) {
			discs.push(disc);
			membersByCluster.set(cluster.id, presentMembers(graph, cluster));
		}

		// Every other build phase yields between chunks; without this the progress
		// the caller publishes never paints and a cancel is only seen at the end.
		if ((index + 1) % CLUSTER_LAYOUT_CHUNK === 0) {
			await options.yieldToFrame?.();
			if (options.isCancelled?.()) return null;
		}
	}

	const placed: string[] = [];
	for (const [clusterId, center] of packClusterDiscs(
		discs,
		options.clusterGap ?? CLUSTER_GAP,
	)) {
		const memberIds = membersByCluster.get(clusterId);
		if (!memberIds) continue;
		translateNodes(graph, memberIds, center.x, center.y);
		placed.push(...memberIds);
	}

	return getLayoutBounds(graph, placed);
}
