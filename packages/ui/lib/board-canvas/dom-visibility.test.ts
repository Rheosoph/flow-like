import { describe, expect, test } from "bun:test";
import type { InternalNode, Node } from "@xyflow/react";
import {
	DOM_RENDERER_MIN_ZOOM,
	DOM_RENDERER_NODE_BUDGET,
	WARM_EDITOR_LIMIT,
	domRendererMinZoom,
	equalVisibleIds,
	visibleDomIds,
	warmEditorIds,
} from "./dom-visibility";

function node(id: string, x: number, y: number, measured = true): InternalNode {
	return {
		id,
		data: {},
		position: { x, y },
		measured: measured ? { width: 180, height: 80 } : {},
		internals: { positionAbsolute: { x, y }, z: 0, userNode: {} as Node },
	};
}
const a = node("a", 100, 100);
const b = node("b", 10000, 100);
const c = node("c", -1000, 100);
const options = {
	nodes: [a, b, c],
	edges: [],
	nodeLookup: new Map([a, b, c].map((node) => [node.id, node])),
	transform: [0, 0, 1] as const,
	width: 1000,
	height: 800,
};
describe("standalone rich-content visibility", () => {
	test("keeps at most256 ordinary editors warm and excludes media", () => {
		const editors = Array.from({ length: WARM_EDITOR_LIMIT }, (_, index) => ({
			...node(String(index), 10000, 0),
			type: index % 2 ? "flowNode" : "node",
		}));
		const media = { ...node("media", 10000, 0), type: "media" };
		const retained = warmEditorIds([...editors, media]);
		expect(retained.size).toBe(WARM_EDITOR_LIMIT);
		expect(retained.has("media")).toBe(false);
		expect(
			visibleDomIds({
				...options,
				nodes: editors,
				nodeLookup: new Map(editors.map((node) => [node.id, node])),
				retained,
			}).nodes,
		).toHaveLength(WARM_EDITOR_LIMIT);
	});
	test("larger editor populations return to viewport culling without stale warm IDs", () => {
		const editors = Array.from(
			{ length: WARM_EDITOR_LIMIT + 1 },
			(_, index) => ({
				...node(String(index), index ? 10000 : 0, 0),
				type: "node",
			}),
		);
		const retained = warmEditorIds(editors);
		expect(retained.size).toBe(0);
		expect(
			visibleDomIds({
				...options,
				nodes: editors,
				nodeLookup: new Map(editors.map((node) => [node.id, node])),
				retained,
			}).nodes,
		).toEqual(["0"]);
		expect(warmEditorIds(editors.slice(1)).size).toBe(WARM_EDITOR_LIMIT);
	});
	test("culls distant rich nodes while retaining crossing SVG edges", () => {
		expect(
			visibleDomIds({
				...options,
				edges: [{ id: "cross", source: "c", target: "b" }],
			}),
		).toEqual({ nodes: ["a"], edges: ["cross"] });
	});
	test("camera changes expose cached geometry without needing mounted wrappers", () => {
		expect(
			visibleDomIds({ ...options, transform: [-10000, 0, 1] }).nodes,
		).toEqual(["b"]);
	});
	test("keeps selected, active editors, and unmeasured nodes available", () => {
		const unknown = node("unknown", 30000, 0, false);
		expect(
			visibleDomIds({
				...options,
				nodes: [a, { ...b, selected: true }, c, unknown],
				nodeLookup: new Map([...options.nodeLookup, [unknown.id, unknown]]),
				retained: new Set(["c"]),
			}).nodes,
		).toEqual(["a", "b", "c", "unknown"]);
	});
	test("unchanged visible IDs reuse the camera subscription snapshot", () => {
		const before = visibleDomIds(options);
		const after = visibleDomIds({ ...options, transform: [10, 10, 1] });
		expect(equalVisibleIds(before, after)).toBe(true);
		expect(
			equalVisibleIds(
				before,
				visibleDomIds({ ...options, transform: [-10000, 0, 1] }),
			),
		).toBe(false);
	});
});

describe("React Flow zoom floor", () => {
	const width = 1600;
	const height = 1000;
	const grid = (count: number, pitch: number, left = 0, top = 0): Node[] => {
		const columns = Math.ceil(Math.sqrt(count));
		return Array.from({ length: count }, (_, index) => ({
			id: `${left}:${index}`,
			data: {},
			position: {
				x: left + (index % columns) * pitch,
				y: top + Math.floor(index / columns) * pitch,
			},
		}));
	};
	/** Windows anchored at a node are exact for grids and close for clusters. */
	const mostInView = (nodes: Node[], zoom: number) =>
		Math.max(
			...nodes.map(
				({ position: anchor }) =>
					nodes.filter(
						({ position }) =>
							position.x >= anchor.x &&
							position.x < anchor.x + width / zoom &&
							position.y >= anchor.y &&
							position.y < anchor.y + height / zoom,
					).length,
			),
		);

	test("boards within the budget keep React Flow's fixed floor", () => {
		const nodes = grid(DOM_RENDERER_NODE_BUDGET, 40);
		expect(domRendererMinZoom(nodes, width, height)).toBe(
			DOM_RENDERER_MIN_ZOOM,
		);
		expect(domRendererMinZoom(grid(5000, 300), 0, height)).toBe(
			DOM_RENDERER_MIN_ZOOM,
		);
	});

	test("a dense board never mounts more than the budget at the floor", () => {
		const nodes = grid(2500, 300);
		expect(mostInView(nodes, DOM_RENDERER_MIN_ZOOM)).toBeGreaterThan(
			DOM_RENDERER_NODE_BUDGET * 4,
		);
		const floor = domRendererMinZoom(nodes, width, height);
		expect(floor).toBeGreaterThan(DOM_RENDERER_MIN_ZOOM);
		expect(floor).toBeLessThan(0.4);
		expect(mostInView(nodes, floor)).toBeLessThanOrEqual(
			DOM_RENDERER_NODE_BUDGET,
		);
	});

	test("a dense cluster sets the floor even when the board is mostly empty", () => {
		const nodes = [...grid(900, 120), ...grid(900, 120, 200_000, 200_000)];
		const floor = domRendererMinZoom(nodes, width, height);
		expect(mostInView(nodes, floor)).toBeLessThanOrEqual(
			DOM_RENDERER_NODE_BUDGET,
		);
		expect(mostInView(nodes, floor * 0.8)).toBeGreaterThan(
			DOM_RENDERER_NODE_BUDGET / 3,
		);
	});

	test("nodes stacked on one spot cap the floor at 1", () => {
		const nodes = Array.from(
			{ length: DOM_RENDERER_NODE_BUDGET + 1 },
			(_, index) => ({ id: `${index}`, data: {}, position: { x: 0, y: 0 } }),
		);
		expect(domRendererMinZoom(nodes, width, height)).toBe(1);
	});
});
