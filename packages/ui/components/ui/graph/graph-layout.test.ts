import { describe, expect, test } from "bun:test";
import Graph from "graphology";
import type { GraphCluster } from "./graph-clusters";
import {
	CLUSTER_GAP,
	type ClusterDisc,
	LABEL_EXTENT_NODE_CAP,
	NODE_GAP,
	applyClusterLayout,
	computeLabelExtents,
	computeSeedSpread,
	createDeterministicPosition,
	getLayoutBounds,
	packClusterDiscs,
	packNodesOnGrid,
	partitionByConnectivity,
	placeCircularLayout,
	placeDetachedNodes,
	placeHierarchyLayout,
	placeHubStar,
	placePhyllotaxis,
	placeRadialLayout,
	relaxOverlaps,
} from "./graph-layout";

function buildGraph(
	nodeCount: number,
	position: (index: number) => { x: number; y: number },
	size = 10,
): Graph {
	const graph = new Graph({ multi: true, type: "directed" });
	for (let index = 0; index < nodeCount; index += 1) {
		graph.addNode(`n${index}`, { ...position(index), size });
	}
	return graph;
}

function worstOverlap(graph: Graph, ids: readonly string[]): number {
	let worst = 0;
	for (let i = 0; i < ids.length; i += 1) {
		for (let j = i + 1; j < ids.length; j += 1) {
			const ax = graph.getNodeAttribute(ids[i], "x") as number;
			const ay = graph.getNodeAttribute(ids[i], "y") as number;
			const bx = graph.getNodeAttribute(ids[j], "x") as number;
			const by = graph.getNodeAttribute(ids[j], "y") as number;
			const radii =
				(graph.getNodeAttribute(ids[i], "size") as number) +
				(graph.getNodeAttribute(ids[j], "size") as number);
			const distance = Math.hypot(bx - ax, by - ay);
			worst = Math.max(worst, radii - distance);
		}
	}
	return worst;
}

describe("relaxOverlaps", () => {
	test("separates nodes stacked on the exact same point", () => {
		const graph = buildGraph(40, () => ({ x: 0, y: 0 }));
		const ids = graph.nodes();

		relaxOverlaps(graph, ids, { iterations: 200 });

		expect(worstOverlap(graph, ids)).toBeLessThanOrEqual(0);
	});

	test("clears a crowded disc the simulation would leave overlapping", () => {
		// Mirrors the pre-fix state: every node inside one small gravity well.
		const graph = buildGraph(200, (index) => {
			const angle = (index * 2.399963) % (Math.PI * 2);
			const radius = 50 * Math.sqrt((index % 30) / 30);
			return { x: Math.cos(angle) * radius, y: Math.sin(angle) * radius };
		});
		const ids = graph.nodes();

		relaxOverlaps(graph, ids, { iterations: 200 });

		expect(worstOverlap(graph, ids)).toBeLessThanOrEqual(0);
	});

	test("respects per-node sizes", () => {
		const graph = new Graph();
		graph.addNode("big", { x: 0, y: 0, size: 30 });
		graph.addNode("small", { x: 1, y: 0, size: 5 });

		relaxOverlaps(graph, ["big", "small"], { iterations: 100 });

		const distance = Math.hypot(
			(graph.getNodeAttribute("small", "x") as number) -
				(graph.getNodeAttribute("big", "x") as number),
			(graph.getNodeAttribute("small", "y") as number) -
				(graph.getNodeAttribute("big", "y") as number),
		);
		expect(distance).toBeGreaterThanOrEqual(35 + NODE_GAP - 1e-6);
	});

	test("leaves an already-spaced layout untouched", () => {
		const graph = buildGraph(9, (index) => ({
			x: (index % 3) * 500,
			y: Math.floor(index / 3) * 500,
		}));

		const passes = relaxOverlaps(graph, graph.nodes(), { iterations: 50 });

		expect(passes).toBe(1);
		expect(graph.getNodeAttribute("n4", "x")).toBe(500);
	});

	test("is a no-op below two nodes", () => {
		const graph = buildGraph(1, () => ({ x: 3, y: 4 }));
		expect(relaxOverlaps(graph, graph.nodes())).toBe(0);
		expect(graph.getNodeAttribute("n0", "x")).toBe(3);
	});
});

describe("partitionByConnectivity", () => {
	test("splits linked nodes from detached ones", () => {
		const graph = new Graph({ multi: true, type: "directed" });
		for (const id of ["c", "a", "b", "d"]) {
			graph.addNode(id, { x: 0, y: 0, size: 10 });
		}
		graph.addEdge("a", "b");

		const partition = partitionByConnectivity(graph);

		expect(partition.connected.sort()).toEqual(["a", "b"]);
		expect(partition.isolated).toEqual(["c", "d"]);
	});
});

describe("packNodesOnGrid", () => {
	test("places every node without overlap", () => {
		const graph = buildGraph(250, () => ({ x: 0, y: 0 }));
		const ids = graph.nodes();

		packNodesOnGrid(graph, ids);

		expect(worstOverlap(graph, ids)).toBeLessThanOrEqual(0);
	});

	test("pitches columns wider than rows so captions have room", () => {
		const graph = buildGraph(100, () => ({ x: 0, y: 0 }));
		const ids = graph.nodes();

		packNodesOnGrid(graph, ids);

		const distinctX = [
			...new Set(ids.map((id) => graph.getNodeAttribute(id, "x") as number)),
		].sort((a, b) => a - b);
		const distinctY = [
			...new Set(ids.map((id) => graph.getNodeAttribute(id, "y") as number)),
		].sort((a, b) => a - b);

		expect(distinctX[1] - distinctX[0]).toBeGreaterThan(
			(distinctY[1] - distinctY[0]) * 2,
		);
	});

	test("stays roughly square despite the wider column pitch", () => {
		const graph = buildGraph(100, () => ({ x: 0, y: 0 }));
		const bounds = packNodesOnGrid(graph, graph.nodes());

		const aspect = (bounds?.width ?? 1) / (bounds?.height ?? 1);
		expect(aspect).toBeGreaterThan(0.6);
		expect(aspect).toBeLessThan(1.7);
	});

	test("centers the grid on the requested point", () => {
		const graph = buildGraph(4, () => ({ x: 0, y: 0 }));
		packNodesOnGrid(graph, graph.nodes(), { centerX: 100, centerY: -50 });

		const bounds = getLayoutBounds(graph, graph.nodes());
		expect(bounds?.centerX).toBeCloseTo(100, 6);
		expect(bounds?.centerY).toBeCloseTo(-50, 6);
	});
});

describe("placeDetachedNodes", () => {
	test("parks detached nodes clear of the connected core", () => {
		const graph = new Graph({ multi: true, type: "directed" });
		graph.addNode("core-a", { x: -100, y: 0, size: 10 });
		graph.addNode("core-b", { x: 100, y: 0, size: 10 });
		graph.addEdge("core-a", "core-b");
		for (let index = 0; index < 30; index += 1) {
			graph.addNode(`free-${index}`, { x: 0, y: 0, size: 10 });
		}

		const partition = partitionByConnectivity(graph);
		const coreBounds = getLayoutBounds(graph, partition.connected);
		placeDetachedNodes(graph, partition.isolated, coreBounds);

		const bandBounds = getLayoutBounds(graph, partition.isolated);
		expect(bandBounds).not.toBeNull();
		expect(coreBounds).not.toBeNull();
		expect(bandBounds?.minX).toBeGreaterThan(coreBounds?.maxX ?? 0);
		expect(worstOverlap(graph, partition.isolated)).toBeLessThanOrEqual(0);
	});

	test("keeps the band from dwarfing a small core", () => {
		const graph = new Graph({ multi: true, type: "directed" });
		graph.addNode("core-a", { x: -20, y: 0, size: 10 });
		graph.addNode("core-b", { x: 20, y: 0, size: 10 });
		graph.addEdge("core-a", "core-b");
		for (let index = 0; index < 4000; index += 1) {
			graph.addNode(`free-${index}`, { x: 0, y: 0, size: 10 });
		}

		const partition = partitionByConnectivity(graph);
		const band = placeDetachedNodes(
			graph,
			partition.isolated,
			getLayoutBounds(graph, partition.connected),
		);

		expect(band).not.toBeNull();
		expect((band?.width ?? 0) / (band?.height ?? 1)).toBeLessThanOrEqual(3);
	});

	test("falls back to a centered grid when nothing is connected", () => {
		const graph = buildGraph(12, () => ({ x: 0, y: 0 }));
		const ids = graph.nodes();

		placeDetachedNodes(graph, ids, null);

		expect(worstOverlap(graph, ids)).toBeLessThanOrEqual(0);
	});
});

function distanceFrom(
	graph: Graph,
	nodeId: string,
	originX = 0,
	originY = 0,
): number {
	return Math.hypot(
		(graph.getNodeAttribute(nodeId, "x") as number) - originX,
		(graph.getNodeAttribute(nodeId, "y") as number) - originY,
	);
}

function buildHubGraph(
	hubId: string,
	childCount: number,
	hubSize = 20,
	childSize = 10,
): Graph {
	const graph = new Graph({ multi: true, type: "directed" });
	graph.addNode(hubId, { x: 0, y: 0, size: hubSize, label: hubId });
	for (let index = 0; index < childCount; index += 1) {
		const childId = `${hubId}-c${index}`;
		graph.addNode(childId, { x: 0, y: 0, size: childSize, label: childId });
		graph.addEdge(hubId, childId);
	}
	return graph;
}

function hubCluster(
	hubId: string,
	graph: Graph,
	represented: number,
): GraphCluster {
	const childIds = graph.neighbors(hubId);
	return {
		id: `hub:${hubId}`,
		kind: "hub",
		title: hubId,
		memberIds: [hubId, ...childIds],
		hubId,
		childIds,
		represented,
		exact: true,
	};
}

describe("placeHubStar", () => {
	test("rings every child clear of the hub, one ring pitch apart", () => {
		const graph = buildHubGraph("doc", 12);
		const children = graph.neighbors("doc");

		placeHubStar(graph, "doc", children, { centerX: 40, centerY: -15 });

		expect(graph.getNodeAttribute("doc", "x")).toBe(40);
		expect(graph.getNodeAttribute("doc", "y")).toBe(-15);

		const distances = children.map((id) => distanceFrom(graph, id, 40, -15));
		const rings = [
			...new Set(distances.map((value) => Math.round(value))),
		].sort((a, b) => a - b);

		expect(rings).toHaveLength(2);
		expect(Math.min(...distances)).toBeGreaterThanOrEqual(20 + NODE_GAP + 10);
		expect(rings[1] - rings[0]).toBeCloseTo(2 * 10 + NODE_GAP, 6);
	});

	test("leaves a childless hub at the requested centre", () => {
		const graph = buildHubGraph("doc", 0);

		const bounds = placeHubStar(graph, "doc", [], { centerX: 7, centerY: 3 });

		expect(bounds?.centerX).toBeCloseTo(7, 6);
		expect(bounds?.centerY).toBeCloseTo(3, 6);
	});

	test("is deterministic for the same seed", () => {
		const first = buildHubGraph("doc", 30);
		const second = buildHubGraph("doc", 30);

		placeHubStar(first, "doc", first.neighbors("doc"), { seed: "hub:doc" });
		placeHubStar(second, "doc", second.neighbors("doc"), { seed: "hub:doc" });

		expect(first.nodes().map((id) => distanceFrom(first, id))).toEqual(
			second.nodes().map((id) => distanceFrom(second, id)),
		);
	});
});

describe("placePhyllotaxis", () => {
	test("puts the first member nearest the centre and never moves back inward", () => {
		const graph = buildGraph(120, () => ({ x: 0, y: 0 }));
		const ids = graph.nodes();

		placePhyllotaxis(graph, ids);

		const distances = ids.map((id) => distanceFrom(graph, id));
		expect(distances[0]).toBe(Math.min(...distances));
		for (let index = 1; index < distances.length; index += 1) {
			expect(distances[index]).toBeGreaterThan(distances[index - 1]);
		}
	});

	test("seeds a dense set without overlap", () => {
		const graph = buildGraph(200, () => ({ x: 0, y: 0 }));
		const ids = graph.nodes();

		placePhyllotaxis(graph, ids);

		expect(worstOverlap(graph, ids)).toBeLessThanOrEqual(0);
	});
});

describe("packClusterDiscs", () => {
	const discs: ClusterDisc[] = Array.from({ length: 60 }, (_, index) => ({
		id: `cluster-${index}`,
		radius: 30 + (index % 7) * 25,
		represented: 1000 - index * 7,
		size: 40 - (index % 11),
	}));

	function worstDiscOverlap(centers: Map<string, { x: number; y: number }>) {
		let worst = Number.NEGATIVE_INFINITY;
		for (const a of discs) {
			for (const b of discs) {
				if (a.id >= b.id) continue;
				const first = centers.get(a.id);
				const second = centers.get(b.id);
				if (!first || !second) continue;
				const distance = Math.hypot(second.x - first.x, second.y - first.y);
				worst = Math.max(worst, a.radius + b.radius + CLUSTER_GAP - distance);
			}
		}
		return worst;
	}

	test("places the most-represented group at the centre of the stage", () => {
		const centers = packClusterDiscs(discs);

		expect(centers.get("cluster-0")).toEqual({ x: 0, y: 0 });
	});

	test("clears every other disc by the group gap", () => {
		expect(worstDiscOverlap(packClusterDiscs(discs))).toBeLessThanOrEqual(1e-6);
	});

	test("ignores the order the groups arrive in", () => {
		const reversed = packClusterDiscs([...discs].reverse());

		expect([...reversed]).toEqual([...packClusterDiscs(discs)]);
	});
});

describe("applyClusterLayout", () => {
	test("keeps groups apart and every node readable", () => {
		const graph = new Graph({ multi: true, type: "directed" });
		const clusters: GraphCluster[] = [];
		for (const [hubId, childCount] of [
			["doc-a", 40],
			["doc-b", 18],
			["doc-c", 7],
		] as const) {
			graph.addNode(hubId, { x: 0, y: 0, size: 24, label: hubId });
			const childIds: string[] = [];
			for (let index = 0; index < childCount; index += 1) {
				const childId = `${hubId}-c${index}`;
				graph.addNode(childId, { x: 0, y: 0, size: 8, label: childId });
				graph.addEdge(hubId, childId);
				childIds.push(childId);
			}
			clusters.push({
				id: `hub:${hubId}`,
				kind: "hub",
				title: hubId,
				memberIds: [hubId, ...childIds],
				hubId,
				childIds,
				represented: childCount * 10,
				exact: false,
			});
		}

		const bounds = applyClusterLayout(graph, clusters);

		expect(bounds).not.toBeNull();
		expect(worstOverlap(graph, graph.nodes())).toBeLessThanOrEqual(0);
		// The dominant group holds the middle; the smallest is pushed outward.
		expect(distanceFrom(graph, "doc-a")).toBeLessThan(
			distanceFrom(graph, "doc-c"),
		);
	});

	test("lays out a group with no hub and reports progress once per group", () => {
		const graph = buildGraph(60, () => ({ x: 0, y: 0 }));
		const ids = graph.nodes();
		const clusters: GraphCluster[] = [
			{
				id: "type:Person",
				kind: "type",
				title: "Person",
				memberIds: ids.slice(0, 30),
				represented: 30,
				exact: true,
			},
			{
				id: "type:Team",
				kind: "type",
				title: "Team",
				memberIds: ids.slice(30),
				represented: 30,
				exact: true,
			},
		];
		const progress: number[] = [];

		applyClusterLayout(graph, clusters, {
			onProgress: (fraction) => progress.push(fraction),
		});

		expect(progress).toEqual([0.5, 1]);
		expect(worstOverlap(graph, ids)).toBeLessThanOrEqual(0);
	});

	test("skips groups whose nodes are not in the graph", () => {
		const graph = buildHubGraph("doc", 6);
		const clusters: GraphCluster[] = [
			hubCluster("doc", graph, 6),
			{
				id: "type:Ghost",
				kind: "type",
				title: "Ghost",
				memberIds: ["missing-a", "missing-b"],
				represented: 2,
				exact: true,
			},
		];

		expect(() => applyClusterLayout(graph, clusters)).not.toThrow();
		expect(graph.order).toBe(7);
	});
});

describe("seed positions", () => {
	test("spread grows with the node count", () => {
		expect(computeSeedSpread(1000)).toBeGreaterThan(computeSeedSpread(100));
	});

	test("seeding a large set keeps the average spacing usable", () => {
		const spread = computeSeedSpread(200);
		const graph = buildGraph(200, () => ({ x: 0, y: 0 }));
		for (const nodeId of graph.nodes()) {
			const position = createDeterministicPosition(nodeId, spread);
			graph.setNodeAttribute(nodeId, "x", position.x);
			graph.setNodeAttribute(nodeId, "y", position.y);
		}

		const bounds = getLayoutBounds(graph, graph.nodes());
		expect(bounds?.width ?? 0).toBeGreaterThan(spread);
	});

	test("is deterministic for the same id", () => {
		expect(createDeterministicPosition("Person:42", 400)).toEqual(
			createDeterministicPosition("Person:42", 400),
		);
	});
});

describe("relaxOverlaps label extents", () => {
	test("clears the horizontal strip a caption occupies", () => {
		const graph = new Graph();
		graph.addNode("left", { x: 0, y: 0, size: 10, label: "A long caption" });
		graph.addNode("right", { x: 30, y: 2, size: 10 });

		const extents = new Map([["left", 80]]);
		relaxOverlaps(graph, ["left", "right"], {
			iterations: 200,
			labelExtents: extents,
		});

		const leftX = graph.getNodeAttribute("left", "x") as number;
		const rightX = graph.getNodeAttribute("right", "x") as number;
		// radius + extent + radius + gap = 10 + 80 + 10 + 8, minus convergence slack.
		expect(rightX - leftX).toBeGreaterThan(90);
	});

	test("leaves vertically-separated nodes alone", () => {
		const graph = new Graph();
		graph.addNode("left", { x: 0, y: 0, size: 10, label: "A long caption" });
		graph.addNode("below", { x: 30, y: 60, size: 10 });

		relaxOverlaps(graph, ["left", "below"], {
			iterations: 50,
			labelExtents: new Map([["left", 80]]),
		});

		expect(graph.getNodeAttribute("below", "x")).toBe(30);
		expect(graph.getNodeAttribute("below", "y")).toBe(60);
	});
});

describe("relaxOverlaps pinned nodes", () => {
	test("a pinned node holds its position while the other yields", () => {
		const graph = new Graph();
		graph.addNode("pinnedNode", { x: 0, y: 0, size: 10, pinned: true });
		graph.addNode("free", { x: 4, y: 0, size: 10 });

		relaxOverlaps(graph, ["pinnedNode", "free"], { iterations: 200 });

		expect(graph.getNodeAttribute("pinnedNode", "x")).toBe(0);
		expect(graph.getNodeAttribute("pinnedNode", "y")).toBe(0);
		const freeX = graph.getNodeAttribute("free", "x") as number;
		expect(freeX).toBeGreaterThan(20);
	});

	test("two pinned nodes stay exactly where they were put", () => {
		const graph = new Graph();
		graph.addNode("a", { x: 0, y: 0, size: 10, pinned: true });
		graph.addNode("b", { x: 2, y: 0, size: 10, pinned: true });

		relaxOverlaps(graph, ["a", "b"], { iterations: 50 });

		expect(graph.getNodeAttribute("b", "x")).toBe(2);
	});
});

describe("computeLabelExtents", () => {
	test("estimates widths only for captioned nodes under the cap", () => {
		const graph = new Graph();
		graph.addNode("titled", { x: 0, y: 0, size: 10, label: "Feedback item" });
		graph.addNode("bare", { x: 0, y: 0, size: 10, label: "" });

		const extents = computeLabelExtents(graph, ["titled", "bare"]);
		expect(extents?.has("titled")).toBeTrue();
		expect(extents?.has("bare")).toBeFalsy();
		expect(extents?.get("titled") ?? 0).toBeGreaterThan(20);
	});

	test("returns nothing above the node cap", () => {
		const graph = buildGraph(LABEL_EXTENT_NODE_CAP + 1, () => ({ x: 0, y: 0 }));
		for (const nodeId of graph.nodes()) {
			graph.setNodeAttribute(nodeId, "label", "caption");
		}
		expect(computeLabelExtents(graph, graph.nodes())).toBeUndefined();
	});
});

describe("deterministic layouts", () => {
	function buildChain(length: number): Graph {
		const graph = new Graph({ multi: true, type: "directed" });
		for (let index = 0; index < length; index += 1) {
			graph.addNode(`n${index}`, { x: 0, y: 0, size: 10 });
		}
		for (let index = 0; index < length - 1; index += 1) {
			graph.addEdge(`n${index}`, `n${index + 1}`);
		}
		return graph;
	}

	test("circular places every node on one non-degenerate ring", () => {
		const graph = buildChain(12);
		const bounds = placeCircularLayout(graph, graph.nodes());
		expect(bounds).not.toBeNull();

		const radii = graph
			.nodes()
			.map((nodeId) =>
				Math.hypot(
					graph.getNodeAttribute(nodeId, "x") as number,
					graph.getNodeAttribute(nodeId, "y") as number,
				),
			);
		const min = Math.min(...radii);
		const max = Math.max(...radii);
		expect(min).toBeGreaterThan(0);
		expect(max - min).toBeLessThan(1);
	});

	test("radial rings grow with hop distance from the centre", () => {
		const graph = buildChain(6);
		placeRadialLayout(graph, graph.nodes(), { centerId: "n0" });

		const radiusOf = (nodeId: string) =>
			Math.hypot(
				graph.getNodeAttribute(nodeId, "x") as number,
				graph.getNodeAttribute(nodeId, "y") as number,
			);
		expect(radiusOf("n0")).toBe(0);
		expect(radiusOf("n1")).toBeGreaterThan(0);
		expect(radiusOf("n2")).toBeGreaterThan(radiusOf("n1"));
		expect(radiusOf("n3")).toBeGreaterThan(radiusOf("n2"));
	});

	test("hierarchy layers follow edge direction left to right", () => {
		const graph = new Graph({ multi: true, type: "directed" });
		graph.addNode("root", { x: 0, y: 0, size: 10 });
		graph.addNode("childA", { x: 0, y: 0, size: 10 });
		graph.addNode("childB", { x: 0, y: 0, size: 10 });
		graph.addNode("grandchild", { x: 0, y: 0, size: 10 });
		graph.addEdge("root", "childA");
		graph.addEdge("root", "childB");
		graph.addEdge("childA", "grandchild");

		placeHierarchyLayout(graph, graph.nodes());

		const xOf = (nodeId: string) =>
			graph.getNodeAttribute(nodeId, "x") as number;
		expect(xOf("root")).toBeLessThan(xOf("childA"));
		expect(xOf("childA")).toBe(xOf("childB"));
		expect(xOf("childA")).toBeLessThan(xOf("grandchild"));
	});
});
