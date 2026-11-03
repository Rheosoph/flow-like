import { describe, expect, test } from "bun:test";
import { excelPathFixture } from "./excel-path-fixture";
import { computeFlowLayoutDetailed } from "./index";
import { measureNodeBox } from "./measure";
import { type WireRoute, planWireRoutes, sampleWireRoute } from "./route";
import { GraphBuilder } from "./test-fixtures";
import type { AutoLayoutInput } from "./types";

function required<T>(value: T | undefined): T {
	if (value === undefined) throw new Error("Missing route fixture value");
	return value;
}

function expectClear(
	input: AutoLayoutInput,
	positions: Map<string, [number, number]>,
	routes: WireRoute[],
) {
	const boxes = input.layerNodes.map((node) => {
		const [x, y] = required(positions.get(node.id));
		return { x, y, ...measureNodeBox(node) };
	});
	const dots = new Map<string, WireRoute["waypoints"][number]>();
	for (const route of routes) {
		for (const [index, point] of route.waypoints.entries()) {
			const key = point.shared ?? `${route.fromPin}->${route.toPin}:${index}`;
			dots.set(key, point);
		}
	}
	boxes.push(
		...[...dots.values()].map((point) => ({ ...point, width: 16, height: 12 })),
	);
	for (const route of routes) {
		expect(route.unresolved).toBeUndefined();
		for (const [a, b] of sampleWireRoute(input, positions, route)) {
			for (let index = 0; index <= 12; index++) {
				const x = a.x + ((b.x - a.x) * index) / 12;
				const y = a.y + ((b.y - a.y) * index) / 12;
				expect(
					boxes.some(
						(box) =>
							x > box.x + 0.01 &&
							x < box.x + box.width - 0.01 &&
							y > box.y + 0.01 &&
							y < box.y + box.height - 0.01,
					),
				).toBe(false);
			}
		}
	}
}

describe("local fan-out routing", () => {
	for (const edgePathType of [
		"default",
		"straight",
		"step",
		"smoothstep",
	] as const) {
		for (const gap of [80, 120]) {
			test(`keeps nearby consumers direct while routing a blocked skip (${edgePathType}, ${gap}px gap)`, () => {
				const graph = new GraphBuilder();
				for (const id of ["source", "nearby", "distant"])
					graph.exec(id, { dataIns: 4, dataOuts: 4 });
				graph.execLink("source", "nearby");
				graph.execLink("nearby", "distant");
				// Two upward curves from one output share their departure. Flattened
				// curves can appear to cross there and trigger unnecessary reroutes.
				graph.dataLink("source", "nearby", 2, 0);
				graph.dataLink("source", "nearby", 2, 1);
				graph.dataLink("source", "distant", 2, 0);
				const input = graph.build({ edgePathType });
				const positions = new Map<string, [number, number]>([
					["source", [0, 0]],
					["nearby", [150 + gap, 0]],
					["distant", [2 * (150 + gap), 0]],
				]);
				const routes = planWireRoutes(input, positions);
				expect(routes).toHaveLength(5);
				for (const route of routes) {
					expect(route.unresolved).toBeUndefined();
					if (route.from === "source" && route.to === "distant")
						expect(route.waypoints.length).toBeGreaterThan(0);
					else expect(route.waypoints).toEqual([]);
				}
				expectClear(input, positions, routes);
				expect(
					planWireRoutes(
						{ ...input, layerNodes: [...input.layerNodes].reverse() },
						positions,
					),
				).toEqual(routes);
			});
		}
		test(`shares distant Excel consumers while keeping local values direct (${edgePathType})`, () => {
			const input = excelPathFixture(edgePathType);
			const result = computeFlowLayoutDetailed(input, "routed");
			const routes = required(result.routing).routes;
			const values = routes.filter(
				(route) => route.fromPin === "read-number:out-1",
			);
			const folder = required(
				values.find((route) => route.to === "format-folder"),
			);
			const file = required(values.find((route) => route.to === "format-file"));
			expect(folder.waypoints.length).toBeGreaterThan(0);
			expect(folder.waypoints.every((point) => point.shared)).toBe(true);
			expect(file.waypoints.slice(0, folder.waypoints.length)).toEqual(
				folder.waypoints,
			);
			for (const target of ["format-number", "substring-number"])
				expect(
					required(values.find((route) => route.to === target)).waypoints,
				).toEqual([]);
			expectClear(input, result.positions, routes);
			const reversed = computeFlowLayoutDetailed(
				{ ...input, layerNodes: [...input.layerNodes].reverse() },
				"routed",
			);
			expect(reversed.positions).toEqual(result.positions);
			expect(reversed.routing).toEqual(result.routing);
		});
	}
});
