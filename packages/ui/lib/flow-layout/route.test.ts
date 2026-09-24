import { describe, expect, test } from "bun:test";
import { computeFlowLayout, computeFlowLayoutDetailed } from "./index";
import { measureNodeBox, pinOffsetY } from "./measure";
import { type DataRoute, planDataRoutes, sampleDataRoute } from "./route";
import { GraphBuilder, allScenarios } from "./test-fixtures";
import type { AutoLayoutInput, LayoutBox } from "./types";
type Point = {
	x: number;
	y: number;
};
type Positions = Map<string, [number, number]>;
function inside(point: Point, box: LayoutBox): boolean {
	return (
		point.x > box.x + 0.01 &&
		point.x < box.x + box.width - 0.01 &&
		point.y > box.y + 0.01 &&
		point.y < box.y + box.height - 0.01
	);
}
function expectClear(
	input: AutoLayoutInput,
	positions: Positions,
	route: DataRoute,
): void {
	const boxes = input.layerNodes
		.map((node) => {
			const [x, y] = positions.get(node.id) ?? node.coordinates ?? [0, 0];
			const measured = input.nodeSizes?.get(node.id);
			const size = measured
				? { width: measured[0], height: measured[1] }
				: measureNodeBox(node);
			return { x, y, ...size };
		})
		.concat(input.obstacles ?? []);
	for (const [a, b] of sampleDataRoute(input, positions, route)) {
		for (let i = 0; i <= 12; i++) {
			const point = {
				x: a.x + ((b.x - a.x) * i) / 12,
				y: a.y + ((b.y - a.y) * i) / 12,
			};
			expect(boxes.some((box) => inside(point, box))).toBe(false);
		}
	}
	for (const point of route.waypoints) {
		for (const box of boxes) {
			expect(
				point.x < box.x + box.width &&
					point.x + 16 > box.x &&
					point.y < box.y + box.height &&
					point.y + 12 > box.y,
			).toBe(false);
		}
	}
}
function crossingCount(
	a: Array<readonly [Point, Point]>,
	b: Array<readonly [Point, Point]>,
): number {
	const cross = (p: Point, q: Point, r: Point) =>
		(q.x - p.x) * (r.y - p.y) - (q.y - p.y) * (r.x - p.x);
	let count = 0;
	for (const [a1, a2] of a) {
		for (const [b1, b2] of b) {
			if (
				cross(a1, a2, b1) * cross(a1, a2, b2) < 0 &&
				cross(b1, b2, a1) * cross(b1, b2, a2) < 0
			)
				count++;
		}
	}
	return count;
}
function skipEdge(pathType: AutoLayoutInput["edgePathType"] = "default") {
	const graph = new GraphBuilder();
	graph.exec("source", { dataOuts: 1 });
	graph.exec("middle", { dataIns: 2 });
	graph.exec("target", { dataIns: 1 });
	graph.execLink("source", "middle");
	graph.execLink("middle", "target");
	graph.dataLink("source", "target");
	const positions: Positions = new Map([
		["source", [0, 0]],
		["middle", [230, 0]],
		["target", [460, 0]],
	]);
	return { input: graph.build({ edgePathType: pathType }), positions };
}
describe("data route planning", () => {
	test("keeps routes stable when zoom adds floating point noise to measured handles", () => {
		const graph = new GraphBuilder();
		for (let index = 0; index < 7; index++) {
			graph.exec(`step-${index}`, {
				start: index === 0,
				execIn: index !== 0,
				dataIns: 4,
				dataOuts: 4,
			});
			if (index > 0) graph.execLink(`step-${index - 1}`, `step-${index}`);
		}
		graph.dataLink("step-0", "step-5", 0, 2);
		graph.dataLink("step-0", "step-5", 1, 1);
		graph.dataLink("step-0", "step-6", 2, 0);
		graph.dataLink("step-1", "step-6", 0, 2);
		graph.dataLink("step-2", "step-6", 1, 1);
		graph.dataLink("step-1", "step-4", 1, 0);
		graph.dataLink("step-2", "step-5", 0, 0);
		const input = graph.build();
		const offsets = new Map(
			input.layerNodes.flatMap((node) =>
				Object.values(node.pins).map(
					(pin) =>
						[
							pin.id,
							{ x: pin.pin_type === "Input" ? 0 : 150, y: pinOffsetY(pin) },
						] as const,
				),
			),
		);
		input.pinOffsets = offsets;
		for (const edgePathType of [
			"default",
			"straight",
			"step",
			"smoothstep",
		] as const) {
			const expected = computeFlowLayoutDetailed(
				{ ...input, edgePathType },
				"routed",
			);
			for (const noise of [-0.00005, 0.00005]) {
				const noisyInput = {
					...input,
					edgePathType,
					pinOffsets: new Map(
						[...offsets].map(([id, offset]) => [
							id,
							{ x: offset.x + noise, y: offset.y - noise },
						]),
					),
				};
				const actual = computeFlowLayoutDetailed(noisyInput, "routed");
				expect(actual.positions).toEqual(expected.positions);
				expect(actual.routing).toEqual(expected.routing);
				expect(planDataRoutes(noisyInput, expected.positions)).toEqual(
					required(expected.routing).routes,
				);
			}
		}
	});
	test("separates crossing data connections without moving nodes", () => {
		const graph = new GraphBuilder();
		for (const id of ["a", "b", "c", "d"]) graph.pure(id);
		graph.dataLink("a", "b");
		graph.dataLink("c", "d");
		const input = graph.build();
		const positions: Positions = new Map([
			["a", [0, 62]],
			["b", [800, 62]],
			["c", [230, -28]],
			["d", [460, 122]],
		]);
		const routes = planDataRoutes(input, positions);
		const direct = routes.map((route) =>
			sampleDataRoute(input, positions, { ...route, waypoints: [] }),
		);
		const routed = routes.map((route) =>
			sampleDataRoute(input, positions, route),
		);
		expect(crossingCount(direct[0], direct[1])).toBeGreaterThan(0);
		expect(crossingCount(routed[0], routed[1])).toBe(0);
		for (const route of routes) expectClear(input, positions, route);
	});
	for (const pathType of [
		"default",
		"straight",
		"step",
		"smoothstep",
	] as const) {
		test(`routes a screenshot-style skip edge around the intervening node (${pathType})`, () => {
			const { input, positions } = skipEdge(pathType);
			const routes = planDataRoutes(input, positions);
			expect(routes).toHaveLength(1);
			expect(routes[0].waypoints).toHaveLength(2);
			expect(routes[0].unresolved).toBeUndefined();
			expectClear(input, positions, routes[0]);
		});
	}
	test("leaves a clear connection without reroutes", () => {
		const graph = new GraphBuilder();
		graph.pure("a");
		graph.pure("b");
		graph.dataLink("a", "b");
		const routes = planDataRoutes(
			graph.build(),
			new Map([
				["a", [0, 0]],
				["b", [230, 0]],
			]),
		);
		expect(routes).toEqual([
			{
				from: "a",
				to: "b",
				fromPin: "a:out-0",
				toPin: "b:in-0",
				waypoints: [],
			},
		]);
	});
	test("reduces crossings with execution wires even when the direct data path clears nodes", () => {
		const graph = new GraphBuilder();
		graph.pure("a");
		graph.pure("b");
		graph.exec("e");
		graph.exec("f");
		graph.dataLink("a", "b");
		graph.execLink("e", "f");
		const input = graph.build();
		const positions: Positions = new Map([
			["a", [0, 62]],
			["b", [800, 62]],
			["e", [230, -28]],
			["f", [460, 122]],
		]);
		const routes = planDataRoutes(input, positions);
		const execRoute: DataRoute = {
			from: "e",
			to: "f",
			fromPin: "e:exec-out-0",
			toPin: "f:exec-in",
			waypoints: [],
		};
		const execution = sampleDataRoute(input, positions, execRoute);
		const baseline = sampleDataRoute(input, positions, {
			...routes[0],
			waypoints: [],
		});
		expect(crossingCount(baseline, execution)).toBeGreaterThan(0);
		expect(routes[0].waypoints.length).toBeGreaterThan(0);
		expect(
			crossingCount(sampleDataRoute(input, positions, routes[0]), execution),
		).toBe(0);
		expectClear(input, positions, routes[0]);
	});
	test("handles measured node sizes and measured pin centres", () => {
		const { input, positions } = skipEdge();
		input.nodeSizes = new Map([["middle", [190, 160]]]);
		input.pinOffsets = new Map([
			["source:out-0", { x: 150, y: 70 }],
			["target:in-0", { x: 0, y: 90 }],
		]);
		const route = planDataRoutes(input, positions)[0];
		const samples = sampleDataRoute(input, positions, route);
		expect(samples[0][0]).toEqual({ x: 150, y: 70 });
		expect(samples[samples.length - 1][1]).toEqual({ x: 460, y: 90 });
		expect(route.waypoints.length).toBeGreaterThan(0);
		expectClear(input, positions, route);
	});
	test("keeps scoped routes off unselected nodes and does not route boundary-crossing connections", () => {
		const { input, positions } = skipEdge();
		input.only = new Set(["source", "target"]);
		required(
			input.layerNodes.find((node) => node.id === "middle"),
		).coordinates = [230, 0, 0];
		positions.delete("middle");
		const routes = planDataRoutes(input, positions);
		expect(routes).toHaveLength(1);
		expect(routes[0].waypoints.length).toBeGreaterThan(0);
		expectClear(input, positions, routes[0]);
		input.only = new Set(["source"]);
		expect(planDataRoutes(input, positions)).toEqual([]);
	});
	test("respects obstacles outside the logical graph", () => {
		const { input, positions } = skipEdge();
		input.obstacles = [{ x: 205, y: 90, width: 200, height: 200 }];
		const route = planDataRoutes(input, positions)[0];
		expect(route.unresolved).toBeUndefined();
		expect(route.waypoints.length).toBeGreaterThan(0);
		expectClear(input, positions, route);
	});
	test("preserves connections when no bounded route clears the obstacles", () => {
		const { input, positions } = skipEdge();
		input.obstacles = [{ x: 155, y: -5000, width: 65, height: 10000 }];
		const route = planDataRoutes(input, positions)[0];
		expect(route.waypoints).toEqual([]);
		expect(route.unresolved).toBe(true);
	});
	test("preserves manual reroutes and their real pin positions", () => {
		const graph = new GraphBuilder();
		const manual = graph.pure("manual");
		manual.name = "reroute";
		graph.pure("b");
		graph.dataLink("manual", "b");
		const input = graph.build();
		const positions: Positions = new Map([
			["manual", [0, 22]],
			["b", [230, 0]],
		]);
		const route = planDataRoutes(input, positions)[0];
		expect(route.from).toBe("manual");
		expect(route.fromPin).toBe("manual:out-0");
		expect(sampleDataRoute(input, positions, route)[0][0]).toEqual({
			x: 16,
			y: 28,
		});
		expect(route.waypoints).toEqual([]);
	});
	test("fan-out routes are deterministic and dots do not overlap each other", () => {
		const graph = new GraphBuilder();
		graph.pure("source");
		graph.exec("middle", { dataIns: 6 });
		const positions: Positions = new Map([
			["source", [0, 10]],
			["middle", [230, 0]],
		]);
		for (let i = 0; i < 4; i++) {
			graph.pure(`target-${i}`);
			graph.dataLink("source", `target-${i}`);
			positions.set(`target-${i}`, [500, i * 80]);
		}
		const input = graph.build();
		const routes = planDataRoutes(input, positions);
		expect(
			planDataRoutes(
				{ ...input, layerNodes: [...input.layerNodes].reverse() },
				positions,
			),
		).toEqual(routes);
		const dots = routes.flatMap((route) => route.waypoints);
		expect(dots.length).toBeGreaterThan(0);
		for (const route of routes) {
			expect(route.waypoints.length).toBeLessThanOrEqual(4);
			if (route.waypoints.length) expectClear(input, positions, route);
		}
		for (let i = 0; i < dots.length; i++) {
			for (let j = 0; j < i; j++) {
				expect(
					Math.abs(dots[i].x - dots[j].x) < 16 &&
						Math.abs(dots[i].y - dots[j].y) < 12,
				).toBe(false);
			}
		}
	});
	test("backward connections are bounded and any chosen route clears its own endpoint boxes", () => {
		const graph = new GraphBuilder();
		graph.pure("a");
		graph.exec("middle", { dataIns: 4 });
		graph.pure("b");
		graph.dataLink("a", "b");
		const input = graph.build();
		const positions: Positions = new Map([
			["a", [460, 0]],
			["middle", [230, 0]],
			["b", [0, 0]],
		]);
		const route = planDataRoutes(input, positions)[0];
		expect(route.waypoints.length).toBeLessThanOrEqual(4);
		if (route.waypoints.length) expectClear(input, positions, route);
		else expect(route.unresolved).toBe(true);
	});
	for (const pathType of [
		"default",
		"straight",
		"step",
		"smoothstep",
	] as const) {
		test(`dense skip connections clear nodes and generated dots (${pathType})`, () => {
			const graph = new GraphBuilder();
			for (let index = 0; index < 7; index++) {
				graph.exec(`step-${index}`, {
					start: index === 0,
					execIn: index !== 0,
					dataIns: 4,
					dataOuts: 4,
				});
				if (index > 0) graph.execLink(`step-${index - 1}`, `step-${index}`);
			}
			for (const [from, to, output, input] of [
				[0, 5, 0, 2],
				[0, 5, 1, 1],
				[0, 6, 2, 0],
				[1, 6, 0, 2],
				[2, 6, 1, 1],
				[1, 4, 1, 0],
				[2, 5, 0, 0],
			])
				graph.dataLink(`step-${from}`, `step-${to}`, output, input);
			const input = graph.build({ edgePathType: pathType });
			const positions = computeFlowLayout(input);
			const routes = planDataRoutes(input, positions);
			expect(routes).toHaveLength(7);
			expect(
				routes.every(
					(route) => !route.unresolved && route.waypoints.length > 0,
				),
			).toBe(true);
			expect(
				planDataRoutes(
					{ ...input, layerNodes: [...input.layerNodes].reverse() },
					positions,
				),
			).toEqual(routes);
			for (const route of routes) expectClear(input, positions, route);
			const execRoutes: DataRoute[] = Array.from({ length: 6 }, (_, index) => ({
				from: `step-${index}`,
				to: `step-${index + 1}`,
				fromPin: `step-${index}:exec-out-0`,
				toPin: `step-${index + 1}:exec-in`,
				waypoints: [],
			}));
			const dots = routes.flatMap((route) =>
				route.waypoints.map((point) => ({ ...point, width: 16, height: 12 })),
			);
			for (const route of [...routes, ...execRoutes]) {
				for (const [a, b] of sampleDataRoute(input, positions, route)) {
					for (let index = 0; index <= 12; index++) {
						const point = {
							x: a.x + ((b.x - a.x) * index) / 12,
							y: a.y + ((b.y - a.y) * index) / 12,
						};
						expect(dots.some((box) => inside(point, box))).toBe(false);
					}
				}
			}
		});
	}
	test("routes a 300-node board within a bounded planning budget", () => {
		const scenario = required(
			allScenarios().find((item) => item.name.startsWith("large-300")),
		);
		const positions = computeFlowLayout(scenario.input);
		const started = performance.now();
		const routes = planDataRoutes(scenario.input, positions);
		expect(performance.now() - started).toBeLessThan(1000);
		expect(routes.length).toBeGreaterThan(100);
		expect(routes.every((route) => route.waypoints.length <= 4)).toBe(true);
	});
	test("bounds work on 100 obstructed skip edges", () => {
		const graph = new GraphBuilder();
		const positions: Positions = new Map();
		for (let i = 0; i < 100; i++) {
			graph.pure(`a-${i}`);
			graph.exec(`obstacle-${i}`);
			graph.pure(`b-${i}`);
			graph.dataLink(`a-${i}`, `b-${i}`);
			positions.set(`a-${i}`, [0, i * 180]);
			positions.set(`obstacle-${i}`, [230, i * 180]);
			positions.set(`b-${i}`, [460, i * 180]);
		}
		const started = performance.now();
		const routes = planDataRoutes(graph.build(), positions);
		expect(performance.now() - started).toBeLessThan(1500);
		expect(routes.filter((route) => route.waypoints.length > 0)).toHaveLength(
			100,
		);
	});
});

function required<T>(value: T | null | undefined): T {
	if (value === undefined || value === null)
		throw new Error("Missing reroute graph value");
	return value;
}
