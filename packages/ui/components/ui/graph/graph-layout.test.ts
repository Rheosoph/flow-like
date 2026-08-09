import { describe, expect, test } from "bun:test";
import Graph from "graphology";
import {
	NODE_GAP,
	computeSeedSpread,
	createDeterministicPosition,
	getLayoutBounds,
	packNodesOnGrid,
	partitionByConnectivity,
	placeDetachedNodes,
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
