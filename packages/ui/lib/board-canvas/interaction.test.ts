import { describe, expect, test } from "bun:test";
import type { Connection } from "@xyflow/react";
import {
	BoardHitIndex,
	ConnectionGesture,
	type WorldPin,
	boundsInPolygon,
	connectionFor,
	edgePoints,
	pointInPolygon,
	snapGroupToGrid,
} from "./interaction";
import type { Scene, SceneEndpoint, SceneNode } from "./scene";

const endpoint = (nodeId: string, direction: "in" | "out"): SceneEndpoint => ({
	nodeId,
	pinId: `${nodeId}.${direction}`,
});
const expectedConnection = (source: string, target: string): Connection => ({
	source,
	sourceHandle: `${source}.out`,
	target,
	targetHandle: `${target}.in`,
});
const allow = () => true;

function node(id: string, x: number, y: number): SceneNode {
	return {
		id,
		x,
		y,
		width: 200,
		height: 100,
		label: id,
		kind: "standard",
		selected: false,
		dimmed: false,
		pins: [
			{
				id: `${id}.in`,
				direction: "input",
				x: 0,
				y: 40,
				label: "Value",
				color: [250, 100, 120, 255],
				kind: "data",
			},
			{
				id: `${id}.out`,
				direction: "output",
				x: 200,
				y: 40,
				label: "Result",
				color: [250, 100, 120, 255],
				kind: "data",
			},
		],
	};
}

function scene(): Scene {
	return {
		nodes: [
			node("a", 0, 0),
			node("b", 400, 0),
			node("c", 400, 200),
			node("d", 0, 200),
		],
		edges: [
			{
				id: "a-b",
				source: endpoint("a", "out"),
				target: endpoint("b", "in"),
				kind: "data",
				selected: true,
				color: [250, 100, 120, 255],
			},
		],
	};
}

function pin(index: BoardHitIndex, nodeId: string, direction: "in" | "out") {
	const found = index.pin(endpoint(nodeId, direction));
	if (!found) throw new Error(`Missing fixture pin ${nodeId}.${direction}`);
	return found;
}

describe("native canvas connection gestures", () => {
	test("DOM comments and media cancel drops instead of opening the palette or disconnecting", () => {
		const index = new BoardHitIndex(scene(), [
			{ id: "comment", x: 700, y: 100, width: 200, height: 100 },
		]);
		for (const reconnect of [
			undefined,
			{ edgeId: "a-b", end: "target" as const },
		]) {
			expect(
				new ConnectionGesture(endpoint("a", "out"), reconnect).drop(
					index,
					{ x: 750, y: 150 },
					1,
					allow,
				),
			).toEqual({ kind: "cancel" });
		}
		expect(
			new ConnectionGesture(endpoint("a", "out")).drop(
				index,
				{ x: 950, y: 150 },
				1,
				allow,
			).kind,
		).toBe("palette");
	});
	test("input and output drags produce the same normalized connection", () => {
		const graph = scene();
		const before = structuredClone(graph);
		const index = new BoardHitIndex(graph);
		for (const [from, to] of [
			[pin(index, "a", "out"), pin(index, "b", "in")],
			[pin(index, "b", "in"), pin(index, "a", "out")],
		]) {
			const validated: Connection[] = [];
			const gesture = new ConnectionGesture({
				nodeId: from.nodeId,
				pinId: from.id,
			});
			expect(
				gesture.drop(index, to, 1, (connection) => {
					validated.push(connection);
					return true;
				}),
			).toEqual({
				kind: "connect",
				connection: expectedConnection("a", "b"),
				oldEdgeId: undefined,
			});
			expect(validated).toContainEqual(expectedConnection("a", "b"));
		}
		expect(graph).toEqual(before);
	});

	test("reconnecting either endpoint retains the opposite endpoint and old edge identity", () => {
		const index = new BoardHitIndex(scene());
		const target = new ConnectionGesture(endpoint("a", "out"), {
			edgeId: "a-b",
			end: "target",
		});
		expect(target.drop(index, pin(index, "c", "in"), 1, allow)).toEqual({
			kind: "connect",
			connection: expectedConnection("a", "c"),
			oldEdgeId: "a-b",
		});
		const source = new ConnectionGesture(endpoint("b", "in"), {
			edgeId: "a-b",
			end: "source",
		});
		expect(source.drop(index, pin(index, "d", "out"), 1, allow)).toEqual({
			kind: "connect",
			connection: expectedConnection("d", "b"),
			oldEdgeId: "a-b",
		});
	});

	test("panning a fixed endpoint offscreen preserves new and reconnected wires in both directions", () => {
		const graph = scene();
		for (const node of graph.nodes) {
			if (node.id === "c" || node.id === "d") node.x = 4000;
			if (node.id === "d") node.y = 400;
		}
		const index = new BoardHitIndex(graph);
		const cases = [
			{
				anchor: endpoint("a", "out"),
				target: pin(index, "c", "in"),
				expected: expectedConnection("a", "c"),
			},
			{
				anchor: endpoint("c", "in"),
				target: pin(index, "a", "out"),
				expected: expectedConnection("a", "c"),
			},
			{
				anchor: endpoint("a", "out"),
				target: pin(index, "c", "in"),
				expected: expectedConnection("a", "c"),
				reconnect: { edgeId: "a-b", end: "target" as const },
			},
			{
				anchor: endpoint("b", "in"),
				target: pin(index, "d", "out"),
				expected: expectedConnection("d", "b"),
				reconnect: { edgeId: "a-b", end: "source" as const },
			},
		];
		for (const { anchor, target, expected, reconnect } of cases) {
			const fixed = index.pin(anchor);
			if (!fixed) throw new Error("Missing fixture endpoint");
			let viewportX = 400 - fixed.x;
			const exposed = (pin: WorldPin) =>
				pin.x + viewportX >= 0 && pin.x + viewportX <= 800;
			const hidden = new Set<string>();
			let covered = false;
			const validatedPins: string[] = [];
			const validate = (connection: Connection, hovered: WorldPin) => {
				if (!hovered) return false;
				validatedPins.push(hovered.id);
				return (
					[connection.source, connection.target].every(
						(id) => !hidden.has(id),
					) &&
					!covered &&
					exposed(hovered)
				);
			};
			const gesture = new ConnectionGesture(anchor, reconnect);
			expect(exposed(fixed)).toBe(true);
			expect(exposed(target)).toBe(false);
			gesture.resolve(index, fixed, 1, validate);
			viewportX = 400 - target.x;
			expect(exposed(fixed)).toBe(false);
			expect(gesture.drop(index, target, 1, validate)).toEqual({
				kind: "connect",
				connection: expected,
				oldEdgeId: reconnect?.edgeId,
			});
			expect(validatedPins).toContain(target.id);
			expect(validatedPins).not.toContain(fixed.id);
			viewportX = 400 - fixed.x;
			expect(gesture.drop(index, target, 1, validate)).toEqual({
				kind: "cancel",
			});
			viewportX = 400 - target.x;
			covered = true;
			expect(gesture.drop(index, target, 1, validate)).toEqual({
				kind: "cancel",
			});
			covered = false;
			hidden.add(anchor.nodeId);
			expect(gesture.drop(index, target, 1, validate)).toEqual({
				kind: "cancel",
			});
		}
	});

	test("returning either reconnect handle to its existing endpoint is a no-op", () => {
		const index = new BoardHitIndex(scene());
		const target = new ConnectionGesture(endpoint("a", "out"), {
			edgeId: "a-b",
			end: "target",
		});
		expect(target.drop(index, pin(index, "b", "in"), 1, allow)).toEqual({
			kind: "noop",
		});
		const source = new ConnectionGesture(endpoint("b", "in"), {
			edgeId: "a-b",
			end: "source",
		});
		expect(source.drop(index, pin(index, "a", "out"), 1, allow)).toEqual({
			kind: "noop",
		});
	});

	test("host type validation rejects a pin drop without disconnecting the old edge", () => {
		const index = new BoardHitIndex(scene());
		const gesture = new ConnectionGesture(endpoint("a", "out"), {
			edgeId: "a-b",
			end: "target",
		});
		const invalid = pin(index, "c", "in");
		const validate = (connection: Connection) =>
			connection.targetHandle !== invalid.id;
		expect(gesture.resolve(index, invalid, 1, validate)).toMatchObject({
			invalid,
		});
		expect(gesture.drop(index, invalid, 1, validate)).toEqual({
			kind: "cancel",
		});
		expect(index.edges.get("a-b")?.target).toEqual(endpoint("b", "in"));
	});

	test("same-owner and same-direction pins cannot become targets", () => {
		const index = new BoardHitIndex(scene());
		const output = pin(index, "a", "out");
		for (const invalid of [pin(index, "a", "in"), pin(index, "b", "out")]) {
			expect(connectionFor(output, invalid)).toBeUndefined();
			expect(
				new ConnectionGesture(endpoint("a", "out")).drop(
					index,
					invalid,
					1,
					allow,
				),
			).toEqual({ kind: "cancel" });
		}
	});

	test("pin snapping uses screen pixels at low, normal and high zoom", () => {
		const index = new BoardHitIndex(scene());
		const target = pin(index, "b", "in");
		for (const zoom of [0.25, 1, 3]) {
			const nearby = { x: target.x - 20 / zoom, y: target.y };
			const outside = { x: target.x - 23 / zoom, y: target.y };
			expect(
				new ConnectionGesture(endpoint("a", "out")).resolve(
					index,
					nearby,
					zoom,
					allow,
				)?.target?.id,
			).toBe(target.id);
			expect(
				new ConnectionGesture(endpoint("a", "out")).resolve(
					index,
					outside,
					zoom,
					allow,
				)?.target,
			).toBeUndefined();
		}
	});

	test("an isolated snapped target keeps its wider release radius", () => {
		const index = new BoardHitIndex(scene());
		const gesture = new ConnectionGesture(endpoint("a", "out"));
		expect(
			gesture.resolve(index, { x: 400, y: 40 }, 1, allow)?.target?.id,
		).toBe("b.in");
		expect(
			gesture.resolve(index, { x: 373, y: 40 }, 1, allow)?.target?.id,
		).toBe("b.in");
		expect(
			gesture.resolve(index, { x: 369, y: 40 }, 1, allow)?.target,
		).toBeUndefined();
	});

	test("adjacent compatible inputs switch in either direction without leaving the node", () => {
		const graph = scene();
		graph.nodes[1].pins.push({
			...graph.nodes[1].pins[0],
			id: "b.second",
			y: 55,
		});
		const index = new BoardHitIndex(graph);
		for (const zoom of [0.1, 0.25, 1, 3]) {
			const gesture = new ConnectionGesture(endpoint("a", "out"));
			for (const [y, id] of [
				[40, "b.in"],
				[55, "b.second"],
				[40, "b.in"],
				[55, "b.second"],
			] as const)
				expect(
					gesture.resolve(index, { x: 400, y }, zoom, allow)?.target?.id,
				).toBe(id);
			expect(gesture.drop(index, { x: 400, y: 55 }, zoom, allow)).toMatchObject(
				{
					kind: "connect",
					connection: { sourceHandle: "a.out", targetHandle: "b.second" },
				},
			);
		}
	});

	test("near-equal distances keep a stable target while incompatible neighbors never capture", () => {
		const graph = scene();
		graph.nodes[1].pins.push({
			...graph.nodes[1].pins[0],
			id: "b.second",
			y: 55,
		});
		const index = new BoardHitIndex(graph);
		const gesture = new ConnectionGesture(endpoint("a", "out"));
		gesture.resolve(index, { x: 400, y: 40 }, 1, allow);
		expect(
			gesture.resolve(index, { x: 400, y: 48 }, 1, allow)?.target?.id,
		).toBe("b.in");
		expect(
			gesture.resolve(index, { x: 400, y: 51 }, 1, allow)?.target?.id,
		).toBe("b.second");
		expect(
			gesture.resolve(index, { x: 400, y: 47 }, 1, allow)?.target?.id,
		).toBe("b.second");
		const firstOnly = (connection: Connection) =>
			connection.targetHandle === "b.in";
		expect(
			gesture.resolve(index, { x: 400, y: 55 }, 1, firstOnly)?.target?.id,
		).toBe("b.in");
	});

	test("remote removal of the anchor or old edge cancels a gesture", () => {
		const graph = scene();
		const original = new BoardHitIndex(graph);
		const gesture = new ConnectionGesture(endpoint("a", "out"), {
			edgeId: "a-b",
			end: "target",
		});
		gesture.resolve(original, pin(original, "c", "in"), 1, allow);
		for (const changed of [
			{ ...graph, edges: [] },
			{ ...graph, nodes: graph.nodes.filter((n) => n.id !== "a") },
		]) {
			expect(
				gesture.drop(new BoardHitIndex(changed), { x: 400, y: 240 }, 1, allow),
			).toEqual({ kind: "cancel" });
		}
	});

	test("a remote edit of the retained reconnect endpoint cancels stale intent", () => {
		const graph = scene();
		const gesture = new ConnectionGesture(endpoint("a", "out"), {
			edgeId: "a-b",
			end: "target",
		});
		gesture.resolve(new BoardHitIndex(graph), { x: 400, y: 240 }, 1, allow);
		const changed: Scene = {
			...graph,
			edges: [{ ...graph.edges[0], source: endpoint("d", "out") }],
		};
		expect(
			gesture.drop(new BoardHitIndex(changed), { x: 400, y: 240 }, 1, allow),
		).toEqual({ kind: "cancel" });
	});

	test("empty drops request a palette for new wires and disconnection for reconnects", () => {
		const index = new BoardHitIndex(scene());
		const empty = { x: 800, y: 600 };
		expect(
			new ConnectionGesture(endpoint("a", "out")).drop(index, empty, 1, allow),
		).toEqual({ kind: "palette", pin: pin(index, "a", "out") });
		expect(
			new ConnectionGesture(endpoint("a", "out"), {
				edgeId: "a-b",
				end: "target",
			}).drop(index, empty, 1, allow),
		).toEqual({ kind: "disconnect", edgeId: "a-b" });
		expect(
			new ConnectionGesture(endpoint("a", "out"), {
				edgeId: "a-b",
				end: "target",
			}).drop(index, { x: 500, y: 280 }, 1, allow),
		).toEqual({ kind: "cancel" });
	});

	test("a remote edit of the moving endpoint also cancels a previously resolved reconnect", () => {
		const graph = scene();
		const gesture = new ConnectionGesture(endpoint("a", "out"), {
			edgeId: "a-b",
			end: "target",
		});
		gesture.resolve(new BoardHitIndex(graph), { x: 400, y: 240 }, 1, allow);
		const changed: Scene = {
			...graph,
			edges: [{ ...graph.edges[0], target: endpoint("d", "in") }],
		};
		expect(
			gesture.drop(new BoardHitIndex(changed), { x: 800, y: 600 }, 1, allow),
		).toEqual({ kind: "cancel" });
	});
});

describe("freehand selection containment", () => {
	const square = [
		{ x: 0, y: 0 },
		{ x: 100, y: 0 },
		{ x: 100, y: 100 },
		{ x: 0, y: 100 },
	];
	test("selects full bounds on the outline and rejects partial or degenerate lassos", () => {
		expect(
			boundsInPolygon({ x: 0, y: 0, width: 100, height: 100 }, square),
		).toBe(true);
		expect(
			boundsInPolygon({ x: 90, y: 90, width: 20, height: 20 }, square),
		).toBe(false);
		expect(pointInPolygon({ x: 50, y: 50 }, square.slice(0, 2))).toBe(false);
		expect(
			boundsInPolygon(
				{ x: 10, y: 10, width: 20, height: 20 },
				[...square].reverse(),
			),
		).toBe(true);
	});
	test("rejects a concave cutout crossing a node even when every corner is inside", () => {
		const concave = [
			{ x: 0, y: 0 },
			{ x: 100, y: 0 },
			{ x: 100, y: 100 },
			{ x: 60, y: 100 },
			{ x: 60, y: 30 },
			{ x: 40, y: 30 },
			{ x: 40, y: 100 },
			{ x: 0, y: 100 },
		];
		for (const point of [
			{ x: 10, y: 10 },
			{ x: 90, y: 10 },
			{ x: 90, y: 90 },
			{ x: 10, y: 90 },
		])
			expect(pointInPolygon(point, concave)).toBe(true);
		expect(
			boundsInPolygon({ x: 10, y: 10, width: 80, height: 80 }, concave),
		).toBe(false);
		expect(
			boundsInPolygon({ x: 10, y: 40, width: 20, height: 50 }, concave),
		).toBe(true);
	});
});

describe("group grid snapping", () => {
	test("preserves offsets and caller data, including off-grid group members", () => {
		const nodes = [
			{ id: "a", position: { x: 13, y: -27 } },
			{ id: "b", position: { x: 37.5, y: 8.25 } },
		];
		const before = structuredClone(nodes);
		expect(snapGroupToGrid(nodes, [10, 20])).toEqual([
			{ id: "a", position: { x: 10, y: -20 } },
			{ id: "b", position: { x: 34.5, y: 15.25 } },
		]);
		expect(nodes).toEqual(before);
	});
	test("keeps the locked axis and ignores invalid grid spacing", () => {
		const nodes = [{ id: "a", position: { x: 13, y: 17 } }];
		expect(
			snapGroupToGrid(nodes, [10, 10], { x: true, y: false })[0].position,
		).toEqual({ x: 10, y: 17 });
		for (const grid of [
			[0, 10],
			[-10, 10],
			[Number.NaN, 10],
		] as const)
			expect(snapGroupToGrid(nodes, grid)).toEqual(nodes);
	});
});

describe("native canvas hit index", () => {
	test("moving nodes updates pins, selected endpoints and incident edge hits", () => {
		const graph = scene();
		const before = structuredClone(graph);
		const index = new BoardHitIndex(graph);
		expect(index.nodeAt({ x: 410, y: 10 })?.id).toBe("b");
		expect(index.edgeAt({ x: 300, y: 40 }, 1)?.id).toBe("a-b");
		index.moveNodes([{ id: "b", x: 900, y: 500 }]);
		expect(index.nodeAt({ x: 410, y: 10 })).toBeUndefined();
		expect(index.nodeAt({ x: 910, y: 510 })?.id).toBe("b");
		expect(index.closestPin({ x: 400, y: 40 }, 1)).toBeUndefined();
		expect(pin(index, "b", "in")).toMatchObject({ x: 900, y: 540 });
		expect(index.edgeAt({ x: 300, y: 40 }, 1)).toBeUndefined();
		const curve = edgePoints(pin(index, "a", "out"), pin(index, "b", "in"));
		expect(index.edgeAt(curve[16], 1)?.id).toBe("a-b");
		expect(index.selectedEndpoint({ x: 900, y: 540 }, 1)).toMatchObject({
			edge: { id: "a-b" },
			end: "target",
		});
		expect(index.selectedEndpoint({ x: 400, y: 40 }, 1)).toBeUndefined();
		expect(graph).toEqual(before);
	});

	test("edge and selected-endpoint hit widths stay constant in screen pixels", () => {
		const index = new BoardHitIndex(scene());
		for (const zoom of [0.25, 1, 3]) {
			expect(index.edgeAt({ x: 300, y: 40 + 6 / zoom }, zoom)?.id).toBe("a-b");
			expect(index.edgeAt({ x: 300, y: 40 + 8 / zoom }, zoom)).toBeUndefined();
			expect(
				index.selectedEndpoint({ x: 400, y: 40 + 11 / zoom }, zoom)?.end,
			).toBe("target");
			expect(
				index.selectedEndpoint({ x: 400, y: 40 + 13 / zoom }, zoom),
			).toBeUndefined();
		}
	});

	test("very long edges remain hittable without an unbounded cell expansion", () => {
		const graph = scene();
		graph.nodes[1].x = 1_000_000;
		graph.nodes[1].y = 1_000_000;
		const index = new BoardHitIndex(graph);
		const curve = edgePoints(pin(index, "a", "out"), pin(index, "b", "in"));
		expect(index.edgeAt(curve[16], 1)?.id).toBe("a-b");
	});

	test("function references follow vertical handle tangents for edge hit testing", () => {
		const graph = scene();
		graph.nodes[0].pins.push({
			...graph.nodes[0].pins[1],
			id: "ref_out_a",
			x: 100,
			y: 100,
			kind: "reference",
		});
		graph.nodes[1].x = 300;
		graph.nodes[1].y = 400;
		graph.nodes[1].pins.push({
			...graph.nodes[1].pins[0],
			id: "ref_in_b",
			x: 100,
			y: 0,
			kind: "reference",
		});
		graph.edges = [
			{
				...graph.edges[0],
				kind: "reference",
				source: { nodeId: "a", pinId: "ref_out_a" },
				target: { nodeId: "b", pinId: "ref_in_b" },
			},
		];
		const index = new BoardHitIndex(graph);
		expect(index.edgeAt({ x: 146.875, y: 189.0625 }, 1)?.id).toBe("a-b");
		expect(index.edgeAt({ x: 189.0625, y: 146.875 }, 1)).toBeUndefined();
		expect(index.selectedEndpoint({ x: 400, y: 400 }, 1)?.end).toBe("target");
	});
});
