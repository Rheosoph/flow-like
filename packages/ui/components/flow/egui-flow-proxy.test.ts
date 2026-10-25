import { describe, expect, test } from "bun:test";
import { type Edge, type Node, Position } from "@xyflow/react";
import { adoptUserNodes, getNodesInside } from "@xyflow/system";
import { DOM_RENDERER_NODE_BUDGET } from "../../lib/board-canvas/dom-visibility";
import type { Scene } from "../../lib/board-canvas/scene";
import {
	createDomFlowGeometryCache,
	createNativeFlowProjectionCache,
	prepareNativeFlowEdges,
	prepareNativeFlowNodes,
	prepareNativeSceneNodes,
} from "./egui-flow-proxy";

const graph: Node = {
	id: "node",
	type: "node",
	position: { x: 20, y: 30 },
	data: { node: { id: "node", pins: {} } },
	width: 310,
	height: 190,
	measured: { width: 310, height: 190 },
	style: { opacity: 0.8 },
};
const scene: Scene = {
	edges: [],
	nodes: [
		{
			id: "node",
			x: 20,
			y: 30,
			width: 150,
			height: 73,
			label: "Node",
			kind: "standard",
			selected: false,
			dimmed: false,
			pins: [
				{
					id: "input",
					direction: "input",
					x: 0,
					y: 28,
					label: "Input",
					kind: "data",
					color: [1, 2, 3, 255],
				},
				{
					id: "ref_out_node",
					direction: "output",
					x: 75,
					y: 73,
					label: "",
					kind: "reference",
					color: [1, 2, 3, 255],
				},
			],
		},
	],
};

describe("native node proxies", () => {
	test("supplies exact geometry and handle bounds without hiding or mutating nodes", () => {
		const before = structuredClone(graph);
		const [proxy] = prepareNativeFlowNodes([graph], scene);
		expect(proxy).toMatchObject({
			type: "node",
			width: 150,
			height: 73,
			measured: { width: 150, height: 73 },
		});
		expect(proxy.hidden).not.toBe(true);
		expect(proxy.handles).toEqual([
			{
				id: "input",
				type: "target",
				position: Position.Left,
				x: -6,
				y: 22,
				width: 12,
				height: 12,
			},
			{
				id: "ref_out_node",
				type: "source",
				position: Position.Bottom,
				x: 69,
				y: 67,
				width: 12,
				height: 12,
			},
		]);
		expect(proxy.style).toMatchObject({
			pointerEvents: "all",
			background: "none",
			opacity: 0.8,
		});
		expect(graph).toEqual(before);
	});
	test("DOM promotion retains real editor measurements and z-order without selecting the node", () => {
		const layer = { ...graph, type: "layerNode", zIndex: 19, selected: false };
		const domScene = {
			...scene,
			nodes: scene.nodes.map((node) => ({ ...node, dom: true })),
		};
		const [proxy] = prepareNativeFlowNodes([layer], domScene, layer.id);
		expect(proxy.zIndex).toBe(19);
		expect(proxy.selected).toBe(false);
		expect(proxy.style).toBe(layer.style);
		expect(proxy.data.__flowNativeDom).toBe(true);
		expect(proxy.data.__flowNativeHovered).toBe(true);
		expect(proxy.width).toBe(layer.width);
		expect(proxy.height).toBe(layer.height);
		expect(proxy.measured).toBe(layer.measured);
		expect(proxy.handles).toBe(layer.handles);
		expect(prepareNativeSceneNodes([graph], new Set([graph.id]))[0]).toBe(
			graph,
		);
		for (const zIndex of [1, 19, 25]) {
			const comment = {
				...graph,
				id: `comment-${zIndex}`,
				type: "commentNode",
				zIndex,
			};
			expect(prepareNativeFlowNodes([comment], scene)[0]).toBe(comment);
		}
	});
	test("DOM editors without fixed dimensions keep natural sizing when promoted", () => {
		const original: Node = {
			...graph,
			width: undefined,
			height: undefined,
			measured: undefined,
			handles: undefined,
		};
		const [proxy] = prepareNativeFlowNodes([original], {
			...scene,
			nodes: scene.nodes.map((node) => ({ ...node, dom: true })),
		});
		expect(proxy.width).toBeUndefined();
		expect(proxy.height).toBeUndefined();
		expect(proxy.measured).toBeUndefined();
		expect(proxy.handles).toBeUndefined();
		expect(proxy.data.__flowNativeDom).toBe(true);
	});
	test("custom or unprojected SVG edges retain their original data and styling", () => {
		const edges: Edge[] = [
			{ id: "native", source: "a", target: "b", data: { pathType: "default" } },
			{
				id: "custom",
				source: "a",
				target: "b",
				label: "value",
				reconnectable: "target",
			},
			{ id: "missing", source: "a", target: "b" },
		];
		const shape = {
			source: { nodeId: "a", pinId: "out" },
			target: { nodeId: "b", pinId: "in" },
			kind: "data" as const,
			selected: false,
			color: [1, 2, 3, 255] as const,
		};
		const projected = prepareNativeFlowEdges(edges, {
			nodes: [],
			edges: [
				{ ...shape, id: "native", dom: false },
				{ ...shape, id: "custom", dom: true },
			],
		});
		expect(projected.map((edge) => edge.data?.__flowNativeDom)).toEqual([
			false,
			true,
			true,
		]);
		expect(projected[0].data?.pathType).toBe("default");
		expect(projected[1].label).toBe("value");
		expect(projected.map((edge) => edge.reconnectable)).toEqual([
			false,
			"target",
			undefined,
		]);
		expect(edges[0].data?.__flowNativeDom).toBeUndefined();
	});
	test("selected editors and unsupported DOM nodes retain their styles", () => {
		const selected = {
			...graph,
			selected: true,
			style: { pointerEvents: "auto" as const },
		};
		const comment: Node = {
			id: "comment",
			type: "commentNode",
			position: { x: 0, y: 0 },
			data: {},
		};
		const projected = prepareNativeFlowNodes([selected, comment], scene);
		expect(projected[0].style).toBe(selected.style);
		expect(projected[1]).toBe(comment);
	});
	test("selected editor measurements cannot persist in native layout after deselection", () => {
		const selected = { ...graph, selected: true };
		expect(prepareNativeSceneNodes([selected])[0]).toBe(selected);
		const [deselected] = prepareNativeSceneNodes([
			{ ...selected, selected: false },
		]);
		expect(deselected.measured).toBeUndefined();
		expect(deselected.width).toBeUndefined();
		expect(deselected.height).toBeUndefined();
		expect(deselected.data).toBe(graph.data);
		expect(
			prepareNativeSceneNodes([prepareNativeFlowNodes([graph], scene)[0]])[0]
				.measured,
		).toBeUndefined();
	});
});

function cacheFixture(count = 3) {
	const nodes: Node[] = Array.from({ length: count }, (_, index) => ({
		...graph,
		id: `node-${index}`,
		position: { x: index * 200, y: 30 },
	}));
	const edges: Edge[] = nodes.slice(1).map((node, index) => ({
		id: `edge-${index}`,
		source: nodes[index].id,
		target: node.id,
		reconnectable: "target",
	}));
	const fixtureScene: Scene = {
		nodes: nodes.map((node) => ({
			...scene.nodes[0],
			id: node.id,
			x: node.position.x,
			dom: false,
			pins: scene.nodes[0].pins.map((pin) => ({ ...pin })),
		})),
		edges: edges.map((edge) => ({
			id: edge.id,
			source: { nodeId: edge.source, pinId: "ref_out_node" },
			target: { nodeId: edge.target, pinId: "input" },
			kind: "data",
			selected: false,
			dom: false,
			color: [1, 2, 3, 255],
		})),
	};
	return { nodes, edges, scene: fixtureScene };
}

describe("native projection identity cache", () => {
	test("keeps all 10,000 node records and the arrays stable across equivalent scenes", () => {
		const fixture = cacheFixture(10_000);
		const cache = createNativeFlowProjectionCache();
		const nodes = cache.nodes(fixture.nodes, fixture.scene);
		const edges = cache.edges(fixture.edges, fixture.scene);
		const equivalent = structuredClone(fixture.scene);
		expect(cache.nodes([...fixture.nodes], equivalent)).toBe(nodes);
		expect(cache.edges([...fixture.edges], equivalent)).toBe(edges);
		expect(nodes).toHaveLength(10_000);
		expect(edges).toHaveLength(9_999);
	});

	test("promotion and hover replace only affected nodes without rebuilding edges", () => {
		const fixture = cacheFixture();
		const cache = createNativeFlowProjectionCache();
		const initial = cache.nodes(fixture.nodes, fixture.scene);
		const edges = cache.edges(fixture.edges, fixture.scene);
		const promotedScene = {
			...fixture.scene,
			nodes: fixture.scene.nodes.map((node, index) => ({
				...node,
				dom: index === 1,
			})),
		};
		const promoted = cache.nodes(fixture.nodes, promotedScene, "node-1");
		expect(promoted).not.toBe(initial);
		expect(promoted[0]).toBe(initial[0]);
		expect(promoted[2]).toBe(initial[2]);
		expect(promoted[1].data.__flowNativeDom).toBe(true);
		expect(promoted[1].data.__flowNativeHovered).toBe(true);
		expect(cache.edges(fixture.edges, promotedScene)).toBe(edges);
		const noHover = cache.nodes(fixture.nodes, promotedScene);
		expect(noHover[0]).toBe(promoted[0]);
		expect(noHover[2]).toBe(promoted[2]);
		expect(noHover[1]).not.toBe(promoted[1]);
		expect(noHover[1].data.__flowNativeHovered).toBe(false);
		expect(cache.nodes(fixture.nodes, promotedScene)).toBe(noHover);
	});

	test("native dimensions and handle geometry invalidate only their node", () => {
		const fixture = cacheFixture();
		const cache = createNativeFlowProjectionCache();
		let previous = cache.nodes(fixture.nodes, fixture.scene);
		const changed = structuredClone(fixture.scene);
		const target = changed.nodes[1];
		for (const update of [
			() => {
				target.width += 20;
			},
			() => {
				target.height += 10;
			},
			() => {
				target.pins[0].x += 7;
			},
			() => {
				target.pins[0].y += 5;
			},
			() => {
				target.pins[0].id = "renamed-input";
			},
			() => {
				target.pins[0].direction = "output";
			},
			() => {
				target.pins[0].kind = "reference";
			},
			() => {
				target.pins.reverse();
			},
			() => {
				target.pins.pop();
			},
		]) {
			update();
			const next = cache.nodes(fixture.nodes, changed);
			expect(next[0]).toBe(previous[0]);
			expect(next[2]).toBe(previous[2]);
			expect(next[1]).not.toBe(previous[1]);
			expect(next[1]).toEqual(
				prepareNativeFlowNodes(fixture.nodes, changed)[1],
			);
			previous = next;
		}
		const repaint = structuredClone(changed);
		repaint.nodes[1].label = "A different label";
		repaint.nodes[1].pins[0].color = [50, 60, 70, 255];
		expect(cache.nodes(fixture.nodes, repaint)).toBe(previous);
	});

	test("DOM resize and handles follow the latest source, and demotion clears them", () => {
		const fixture = cacheFixture();
		const cache = createNativeFlowProjectionCache();
		const domScene = {
			...fixture.scene,
			nodes: fixture.scene.nodes.map((node, index) => ({
				...node,
				dom: index === 1,
			})),
		};
		const initial = cache.nodes(fixture.nodes, domScene);
		const resized: Node = {
			...fixture.nodes[1],
			width: undefined,
			height: undefined,
			measured: { width: 490, height: 290 },
			handles: [
				{
					id: "custom-input",
					type: "target",
					position: Position.Top,
					x: 110,
					y: 40,
					width: 14,
					height: 14,
				},
			],
		};
		const inputs = fixture.nodes.with(1, resized);
		const resizedProjection = cache.nodes(inputs, domScene);
		expect(resizedProjection[0]).toBe(initial[0]);
		expect(resizedProjection[2]).toBe(initial[2]);
		expect(resizedProjection[1]).not.toBe(initial[1]);
		expect(resizedProjection[1].width).toBeUndefined();
		expect(resizedProjection[1].measured).toBe(resized.measured);
		expect(resizedProjection[1].handles).toBe(resized.handles);
		const demoted = cache.nodes(inputs, fixture.scene);
		expect(demoted[1].data.__flowNativeDom).toBe(false);
		expect(demoted[1].measured).toEqual({ width: 150, height: 73 });
		expect(demoted[1].handles?.map((handle) => handle.id)).toEqual([
			"input",
			"ref_out_node",
		]);
		expect(resized.measured).toEqual({ width: 490, height: 290 });
	});

	test("source edits are visible even when geometry and membership are unchanged", () => {
		const fixture = cacheFixture();
		const cache = createNativeFlowProjectionCache();
		const initial = cache.nodes(fixture.nodes, fixture.scene);
		const edited = {
			...fixture.nodes[1],
			position: { x: 700, y: 800 },
			selected: true,
			data: { value: "updated" },
			style: { opacity: 0.5 },
		};
		const next = cache.nodes(fixture.nodes.with(1, edited), fixture.scene);
		expect(next[0]).toBe(initial[0]);
		expect(next[2]).toBe(initial[2]);
		expect(next[1]).toMatchObject({
			position: edited.position,
			selected: true,
			data: { value: "updated" },
			style: { opacity: 0.5 },
		});
	});

	test("edge membership and endpoint edits preserve unaffected edge identities", () => {
		const fixture = cacheFixture();
		const cache = createNativeFlowProjectionCache();
		const initial = cache.edges(fixture.edges, fixture.scene);
		const promotedScene = {
			...fixture.scene,
			edges: fixture.scene.edges.map((edge, index) => ({
				...edge,
				dom: index === 0,
			})),
		};
		const promoted = cache.edges(fixture.edges, promotedScene);
		expect(promoted[1]).toBe(initial[1]);
		expect(promoted[0]).not.toBe(initial[0]);
		expect(promoted[0].reconnectable).toBe("target");
		const edited: Edge = {
			...fixture.edges[0],
			target: "node-2",
			targetHandle: "another-input",
			label: "changed",
			reconnectable: "source",
			style: { stroke: "red" },
		};
		const next = cache.edges(fixture.edges.with(0, edited), promotedScene);
		expect(next[1]).toBe(promoted[1]);
		expect(next[0]).toMatchObject({
			target: "node-2",
			targetHandle: "another-input",
			label: "changed",
			reconnectable: "source",
			style: { stroke: "red" },
		});
		expect(cache.edges(fixture.edges.with(0, edited), promotedScene)).toBe(
			next,
		);
	});

	test("reorders, removals, missing geometry, and per-canvas reset retain correct records", () => {
		const fixture = cacheFixture();
		const cache = createNativeFlowProjectionCache();
		const initial = cache.nodes(fixture.nodes, fixture.scene);
		const reversed = cache.nodes([...fixture.nodes].reverse(), fixture.scene);
		expect(reversed).toEqual([...initial].reverse());
		expect(reversed[0]).toBe(initial[2]);
		const removed = cache.nodes(fixture.nodes.slice(1), fixture.scene);
		expect(removed).toHaveLength(2);
		expect(removed[0]).toBe(initial[1]);
		const unsupported: Node = { ...graph, id: "comment", type: "commentNode" };
		const mixed = cache.nodes([unsupported, ...fixture.nodes], fixture.scene);
		expect(mixed[0]).toBe(unsupported);
		const withoutGeometry = cache.nodes(fixture.nodes, {
			nodes: [],
			edges: [],
		});
		expect(withoutGeometry[0]).toBe(fixture.nodes[0]);
		expect(
			createNativeFlowProjectionCache().nodes(fixture.nodes, fixture.scene)[0],
		).not.toBe(initial[0]);
		cache.clear();
		expect(cache.nodes(fixture.nodes, fixture.scene)[0]).not.toBe(initial[0]);
	});
});

describe("DOM renderer geometry seeds", () => {
	const pin = (id: string, pinType: "Input" | "Output") => ({
		id,
		name: id,
		friendly_name: id,
		pin_type: pinType,
		data_type: "Execution",
		index: 1,
		connected_to: [],
		depends_on: [],
	});
	const flowNode = (index: number): Node => ({
		id: `n${index}`,
		type: "node",
		position: { x: (index % 30) * 300, y: Math.floor(index / 30) * 200 },
		data: {
			node: {
				id: `n${index}`,
				name: "log",
				friendly_name: "Log",
				pins: {
					[`in${index}`]: pin(`in${index}`, "Input"),
					[`out${index}`]: pin(`out${index}`, "Output"),
				},
			},
		},
	});
	const board = (count: number) =>
		Array.from({ length: count }, (_, index) => flowNode(index));
	const view = { x: 0, y: 0, width: 1200, height: 600 };

	test("small boards keep React Flow's own measurement", () => {
		const nodes = board(DOM_RENDERER_NODE_BUDGET);
		expect(createDomFlowGeometryCache()(nodes)).toBe(nodes);
	});

	test("React Flow mounts only nodes in view once never-measured nodes are seeded", () => {
		const nodes = board(900);
		const raw = new Map();
		expect(adoptUserNodes(nodes, raw, new Map())).toBe(false);
		// Without bounds React Flow force-renders every node, wherever it is.
		expect(getNodesInside(raw, view, [0, 0, 1], true)).toHaveLength(900);
		const seeded = createDomFlowGeometryCache()(nodes);
		const lookup = new Map();
		// All nodes count as initialized, so a queued fitView still runs.
		expect(adoptUserNodes(seeded, lookup, new Map())).toBe(true);
		expect(getNodesInside(lookup, view, [0, 0, 1], true)).toHaveLength(12);
		expect(
			seeded[0].handles?.map((handle) => [handle.id, handle.type]),
		).toEqual([
			["in0", "target"],
			["out0", "source"],
		]);
	});

	test("a node's first DOM measurement replaces its estimate for good", () => {
		const nodes = board(900);
		const geometry = createDomFlowGeometryCache();
		const first = geometry(nodes);
		expect(geometry(nodes)[5]).toBe(first[5]);
		expect(Number.isInteger(first[5].measured?.width)).toBe(false);
		const measured = nodes.map((node, index) =>
			index === 5 ? { ...node, measured: { width: 150, height: 61 } } : node,
		);
		const next = geometry(measured);
		expect(next[5]).toBe(measured[5]);
		expect(next[5].handles).toBeUndefined();
		expect(next[6]).toBe(first[6]);
	});

	test("fixed-size comments are seeded without handles and hidden nodes are left alone", () => {
		const comment: Node = {
			id: "comment",
			type: "commentNode",
			position: { x: 0, y: 0 },
			width: 200,
			height: 80,
			data: {},
		};
		const hidden: Node = { ...flowNode(899), id: "hidden", hidden: true };
		const seeded = createDomFlowGeometryCache()([
			...board(DOM_RENDERER_NODE_BUDGET),
			comment,
			hidden,
		]);
		expect(seeded.at(-2)).toMatchObject({
			measured: { width: 200.5, height: 80.5 },
			handles: [],
		});
		expect(seeded.at(-1)).toBe(hidden);
	});
});
