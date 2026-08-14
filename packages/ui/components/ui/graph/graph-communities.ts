/**
 * Louvain modularity communities.
 *
 * The type-and-hub grouping in `graph-clusters` only works on an ontology with a
 * parent-child spine. A peer ontology — orders, items, locations and suppliers
 * all linked across each other — has no spine, so that grouping stands down and
 * leaves a force layout to draw one undifferentiated hairball.
 *
 * Modularity asks a different question: not "what type is this" but "which nodes
 * link to each other far more than chance would explain". That is the grouping
 * every general-purpose graph tool falls back on, and it is the one that survives
 * an ontology whose structure the schema never declared.
 *
 * Deliberately deterministic: nodes are visited in a fixed order and ties break on
 * the lower community index, so the same sample always draws the same picture.
 * Canonical Louvain shuffles, which would repaint the graph on every rebuild.
 */

export interface CommunityEdge {
	source: string;
	target: string;
}

export interface CommunityResult {
	/** Community index per node, numbered by descending size so 0 is the largest. */
	communityByNode: Map<string, number>;
	/** Members per community, in the same order as the indices. */
	members: string[][];
	modularity: number;
}

export interface CommunityOptions {
	/**
	 * Above 1 splits into more, smaller communities; below 1 merges them. The
	 * default resolves a few hundred nodes into groups a stage can seat.
	 */
	resolution?: number;
	maxLevels?: number;
	maxLocalPasses?: number;
}

const DEFAULT_RESOLUTION = 1;
const MAX_LEVELS = 12;
const MAX_LOCAL_PASSES = 32;
/** Modularity has converged well before this; it only stops pathological input. */
const MIN_GAIN = 1e-9;

interface WeightedGraph {
	/** Neighbour index list per node. */
	neighbors: number[][];
	/** Edge weight parallel to `neighbors`. */
	weights: number[][];
	/** Self-loop weight, which aggregation folds a community's inner edges into. */
	selfLoops: Float64Array;
	/** Weighted degree, self-loops counted twice. */
	degrees: Float64Array;
	/** Sum of all edge weights, self-loops counted once. */
	totalWeight: number;
	order: number;
}

function buildWeightedGraph(
	nodeIndex: ReadonlyMap<string, number>,
	edges: readonly CommunityEdge[],
): WeightedGraph {
	const order = nodeIndex.size;
	const adjacency = new Map<number, Map<number, number>>();
	const selfLoops = new Float64Array(order);
	const degrees = new Float64Array(order);
	let totalWeight = 0;

	for (const edge of edges) {
		const source = nodeIndex.get(edge.source);
		const target = nodeIndex.get(edge.target);
		if (source === undefined || target === undefined) continue;

		totalWeight += 1;
		if (source === target) {
			selfLoops[source] += 1;
			degrees[source] += 2;
			continue;
		}

		for (const [from, to] of [
			[source, target],
			[target, source],
		]) {
			const bucket = adjacency.get(from);
			if (bucket) bucket.set(to, (bucket.get(to) ?? 0) + 1);
			else adjacency.set(from, new Map([[to, 1]]));
		}
		degrees[source] += 1;
		degrees[target] += 1;
	}

	const neighbors: number[][] = new Array(order);
	const weights: number[][] = new Array(order);
	for (let node = 0; node < order; node += 1) {
		const bucket = adjacency.get(node);
		if (!bucket) {
			neighbors[node] = [];
			weights[node] = [];
			continue;
		}
		// Sorted, so the gain scan visits candidate communities in a stable order.
		const sorted = [...bucket.entries()].sort((a, b) => a[0] - b[0]);
		neighbors[node] = sorted.map(([id]) => id);
		weights[node] = sorted.map(([, weight]) => weight);
	}

	return { neighbors, weights, selfLoops, degrees, totalWeight, order };
}

/**
 * One Louvain level: moves nodes between communities while modularity improves.
 * Returns the community index per node, or null when nothing moved.
 */
function moveNodesLocally(
	graph: WeightedGraph,
	resolution: number,
	maxPasses: number,
): Int32Array | null {
	// Self-loops are deliberately absent: they contribute the same amount to a
	// node's gain in every candidate community, so they cannot change the choice.
	const { order, neighbors, weights, degrees, totalWeight } = graph;
	if (totalWeight === 0) return null;

	const twoM = 2 * totalWeight;
	const community = new Int32Array(order);
	const communityTotalDegree = new Float64Array(order);
	for (let node = 0; node < order; node += 1) {
		community[node] = node;
		communityTotalDegree[node] = degrees[node];
	}

	// Reused across nodes; only the entries touched this iteration are read back.
	const weightToCommunity = new Float64Array(order);
	const touched: number[] = [];
	let moved = false;

	for (let pass = 0; pass < maxPasses; pass += 1) {
		let movesThisPass = 0;

		for (let node = 0; node < order; node += 1) {
			const own = community[node];
			const nodeDegree = degrees[node];

			for (const index of touched) weightToCommunity[index] = 0;
			touched.length = 0;

			const nodeNeighbors = neighbors[node];
			const nodeWeights = weights[node];
			for (let i = 0; i < nodeNeighbors.length; i += 1) {
				const target = community[nodeNeighbors[i]];
				if (weightToCommunity[target] === 0) touched.push(target);
				weightToCommunity[target] += nodeWeights[i];
			}

			// Take the node out before scoring, so staying put is judged on the same
			// terms as moving and a node is never compared against its own presence.
			communityTotalDegree[own] -= nodeDegree;
			const ownGain =
				(weightToCommunity[own] ?? 0) -
				(resolution * communityTotalDegree[own] * nodeDegree) / twoM;

			let bestCommunity = own;
			let bestGain = ownGain;
			for (const candidate of touched) {
				if (candidate === own) continue;
				const gain =
					weightToCommunity[candidate] -
					(resolution * communityTotalDegree[candidate] * nodeDegree) / twoM;
				// `>` plus the ascending scan order makes the lowest index win a tie.
				if (gain > bestGain + MIN_GAIN) {
					bestGain = gain;
					bestCommunity = candidate;
				}
			}

			communityTotalDegree[bestCommunity] += nodeDegree;
			community[node] = bestCommunity;
			if (bestCommunity !== own) {
				movesThisPass += 1;
				moved = true;
			}
		}

		if (movesThisPass === 0) break;
	}

	return moved ? community : null;
}

/** Renumbers sparse community ids to a dense 0..n-1 range, order preserved. */
function densify(community: Int32Array): { dense: Int32Array; count: number } {
	const mapping = new Map<number, number>();
	const dense = new Int32Array(community.length);
	for (let node = 0; node < community.length; node += 1) {
		const id = community[node];
		let mapped = mapping.get(id);
		if (mapped === undefined) {
			mapped = mapping.size;
			mapping.set(id, mapped);
		}
		dense[node] = mapped;
	}
	return { dense, count: mapping.size };
}

/** Folds each community into a single node, its inner edges becoming a self-loop. */
function aggregate(graph: WeightedGraph, community: Int32Array, count: number) {
	const adjacency = new Map<number, Map<number, number>>();
	const selfLoops = new Float64Array(count);
	const degrees = new Float64Array(count);
	let totalWeight = 0;

	for (let node = 0; node < graph.order; node += 1) {
		const from = community[node];
		selfLoops[from] += graph.selfLoops[node];
		totalWeight += graph.selfLoops[node];
		degrees[from] += 2 * graph.selfLoops[node];

		const nodeNeighbors = graph.neighbors[node];
		const nodeWeights = graph.weights[node];
		for (let i = 0; i < nodeNeighbors.length; i += 1) {
			const to = community[nodeNeighbors[i]];
			const weight = nodeWeights[i];
			degrees[from] += weight;
			if (from === to) {
				// Each inner edge is seen from both ends, so a half each side.
				selfLoops[from] += weight / 2;
				totalWeight += weight / 2;
				continue;
			}
			if (from < to) totalWeight += weight;
			const bucket = adjacency.get(from);
			if (bucket) bucket.set(to, (bucket.get(to) ?? 0) + weight);
			else adjacency.set(from, new Map([[to, weight]]));
		}
	}

	const neighbors: number[][] = new Array(count);
	const weights: number[][] = new Array(count);
	for (let node = 0; node < count; node += 1) {
		const bucket = adjacency.get(node);
		const sorted = bucket
			? [...bucket.entries()].sort((a, b) => a[0] - b[0])
			: [];
		neighbors[node] = sorted.map(([id]) => id);
		weights[node] = sorted.map(([, weight]) => weight);
	}

	return {
		neighbors,
		weights,
		selfLoops,
		degrees,
		totalWeight,
		order: count,
	} satisfies WeightedGraph;
}

function computeModularity(
	graph: WeightedGraph,
	community: Int32Array,
	resolution: number,
): number {
	if (graph.totalWeight === 0) return 0;
	const twoM = 2 * graph.totalWeight;
	const inner = new Map<number, number>();
	const total = new Map<number, number>();

	for (let node = 0; node < graph.order; node += 1) {
		const own = community[node];
		total.set(own, (total.get(own) ?? 0) + graph.degrees[node]);
		inner.set(own, (inner.get(own) ?? 0) + graph.selfLoops[node]);
		const nodeNeighbors = graph.neighbors[node];
		const nodeWeights = graph.weights[node];
		for (let i = 0; i < nodeNeighbors.length; i += 1) {
			if (community[nodeNeighbors[i]] !== own) continue;
			inner.set(own, (inner.get(own) ?? 0) + nodeWeights[i] / 2);
		}
	}

	let modularity = 0;
	for (const [id, innerWeight] of inner) {
		const totalDegree = total.get(id) ?? 0;
		modularity +=
			innerWeight / graph.totalWeight -
			resolution * (totalDegree / twoM) * (totalDegree / twoM);
	}
	return modularity;
}

/**
 * Partitions nodes into communities by modularity.
 *
 * Nodes with no edges each land in their own community; the caller decides what
 * to do with them, because a layout and a legend want opposite things there.
 */
export function detectCommunities(
	nodes: readonly string[],
	edges: readonly CommunityEdge[],
	options: CommunityOptions = {},
): CommunityResult {
	const resolution = options.resolution ?? DEFAULT_RESOLUTION;
	const nodeIndex = new Map(nodes.map((id, index) => [id, index]));
	const base = buildWeightedGraph(nodeIndex, edges);

	// Community of each ORIGINAL node, rewritten at every level.
	let assignment = new Int32Array(base.order);
	for (let node = 0; node < base.order; node += 1) assignment[node] = node;

	let level = base;
	const maxLevels = options.maxLevels ?? MAX_LEVELS;
	for (let pass = 0; pass < maxLevels; pass += 1) {
		const moved = moveNodesLocally(
			level,
			resolution,
			options.maxLocalPasses ?? MAX_LOCAL_PASSES,
		);
		if (!moved) break;

		const { dense, count } = densify(moved);
		const next = new Int32Array(base.order);
		for (let node = 0; node < base.order; node += 1) {
			next[node] = dense[assignment[node]];
		}
		assignment = next;

		if (count === level.order) break;
		level = aggregate(level, dense, count);
	}

	const modularity = computeModularity(base, assignment, resolution);

	// Renumbered largest-first so the caller can rank without a second sort, and
	// so a stage that seats only the first N seats the ones that matter.
	const grouped = new Map<number, string[]>();
	for (let node = 0; node < base.order; node += 1) {
		const own = assignment[node];
		const bucket = grouped.get(own);
		if (bucket) bucket.push(nodes[node]);
		else grouped.set(own, [nodes[node]]);
	}

	const members = [...grouped.values()].sort(
		(a, b) => b.length - a.length || a[0].localeCompare(b[0]),
	);
	const communityByNode = new Map<string, number>();
	members.forEach((bucket, index) => {
		for (const id of bucket) communityByNode.set(id, index);
	});

	return { communityByNode, members, modularity };
}
