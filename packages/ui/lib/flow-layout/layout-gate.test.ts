import { describe, expect, test } from "bun:test";
import type { INode } from "../schema/flow/node";
import { computeFlowLayoutDetailed } from "./index";
import { measureLayerBox, measureNodeBox } from "./measure";
import { type Scenario, allScenarios } from "./test-fixtures";
import type { AutoLayoutInput, LayoutStyle } from "./types";

const STYLES: LayoutStyle[] = ["compact", "balanced", "expanded"];

interface Rect {
	id: string;
	x: number;
	y: number;
	width: number;
	height: number;
}

function buildRects(
	input: AutoLayoutInput,
	positions: ReadonlyMap<string, [number, number]>,
): Rect[] {
	const byId = new Map(input.layerNodes.map((node) => [node.id, node]));
	const entityIds = new Set(input.layerEntities.map((entity) => entity.id));
	const rects: Rect[] = [];

	for (const [id, position] of positions) {
		const node = byId.get(id);
		const box = entityIds.has(id)
			? measureLayerBox(input.boardLayers?.[id] ?? ({ pins: {} } as never))
			: measureNodeBox(node as INode);
		rects.push({
			id,
			x: position[0],
			y: position[1],
			width: box.width,
			height: box.height,
		});
	}
	return rects;
}

function countOverlaps(rects: Rect[]): Array<[string, string, number]> {
	const overlaps: Array<[string, string, number]> = [];
	for (let i = 0; i < rects.length; i++) {
		for (let j = i + 1; j < rects.length; j++) {
			const a = rects[i];
			const b = rects[j];
			const dx = Math.min(a.x + a.width, b.x + b.width) - Math.max(a.x, b.x);
			const dy = Math.min(a.y + a.height, b.y + b.height) - Math.max(a.y, b.y);
			if (dx > 0 && dy > 0) overlaps.push([a.id, b.id, dx * dy]);
		}
	}
	return overlaps;
}

function reapply(
	input: AutoLayoutInput,
	positions: ReadonlyMap<string, [number, number]>,
): AutoLayoutInput {
	return {
		...input,
		layerNodes: input.layerNodes.map((node) => {
			const next = positions.get(node.id);
			return next ? { ...node, coordinates: [next[0], next[1], 0] } : node;
		}),
		layerEntities: input.layerEntities.map((entity) => {
			const next = positions.get(entity.id);
			return next ? { ...entity, coordinates: [next[0], next[1], 0] } : entity;
		}),
	};
}

function shuffled<T>(values: T[], seed: number): T[] {
	const result = [...values];
	let state = seed;
	const random = () => {
		state = (state * 1664525 + 1013904223) % 4294967296;
		return state / 4294967296;
	};
	for (let i = result.length - 1; i > 0; i--) {
		const j = Math.floor(random() * (i + 1));
		[result[i], result[j]] = [result[j], result[i]];
	}
	return result;
}

function serialise(positions: ReadonlyMap<string, [number, number]>): string {
	return [...positions.entries()]
		.sort((a, b) => a[0].localeCompare(b[0]))
		.map(([id, position]) => `${id}:${position[0]},${position[1]}`)
		.join("|");
}

const scenarios = allScenarios();

describe("layout gate", () => {
	for (const style of STYLES) {
		for (const scenario of scenarios) {
			describe(`${scenario.name} / ${style}`, () => {
				const { input } = scenario;
				const expectedCount =
					input.layerNodes.length + input.layerEntities.length;
				const result = computeFlowLayoutDetailed(input, style);

				test("positions every node exactly once", () => {
					expect(result.diagnostics.unplaced).toEqual([]);
					expect(result.positions.size).toBe(expectedCount);
				});

				test("produces no overlapping nodes", () => {
					const overlaps = countOverlaps(buildRects(input, result.positions));
					expect(overlaps.slice(0, 5)).toEqual([]);
				});

				test("keeps every non-reversed edge pointing right", () => {
					const reversed = new Set(
						result.reversedEdges.map((edge) => `${edge.from}->${edge.to}`),
					);
					const columns = result.diagnostics.columns;
					const violations: string[] = [];

					for (const node of input.layerNodes) {
						const ownerColumn = columns.get(node.id);
						if (ownerColumn === undefined) continue;
						for (const pin of Object.values(node.pins)) {
							if (pin.pin_type !== "Output") continue;
							for (const targetPinId of pin.connected_to) {
								const target = input.layerNodes.find((candidate) =>
									Object.hasOwn(candidate.pins, targetPinId),
								);
								if (!target || target.id === node.id) continue;
								if (reversed.has(`${node.id}->${target.id}`)) continue;
								const targetColumn = columns.get(target.id);
								if (targetColumn === undefined) continue;
								if (targetColumn <= ownerColumn) {
									violations.push(`${node.id}->${target.id}`);
								}
							}
						}
					}
					expect(violations.slice(0, 5)).toEqual([]);
				});

				test("is a fixed point", () => {
					const second = computeFlowLayoutDetailed(
						reapply(input, result.positions),
						style,
					);
					expect(serialise(second.positions)).toBe(serialise(result.positions));
				});

				test("does not depend on input array order", () => {
					for (const seed of [1, 7, 99]) {
						const shuffledInput: AutoLayoutInput = {
							...input,
							layerNodes: shuffled(input.layerNodes, seed),
							layerEntities: shuffled(input.layerEntities, seed),
						};
						const other = computeFlowLayoutDetailed(shuffledInput, style);
						expect(serialise(other.positions)).toBe(
							serialise(result.positions),
						);
					}
				});
			});
		}
	}

	test("terminates quickly even on a 300 node board", () => {
		const large = scenarios.find((scenario) =>
			scenario.name.startsWith("large-"),
		) as Scenario;
		const started = performance.now();
		computeFlowLayoutDetailed(large.input, "compact");
		expect(performance.now() - started).toBeLessThan(250);
	});

	test("terminates on a pure data cycle", () => {
		const cycle = scenarios.find(
			(scenario) => scenario.name === "pure-data-cycle",
		) as Scenario;
		const started = performance.now();
		const result = computeFlowLayoutDetailed(cycle.input, "compact");
		expect(performance.now() - started).toBeLessThan(50);
		expect(result.positions.size).toBe(cycle.input.layerNodes.length);
	});
});

// The optional inputs are exactly where a non-rigid anchor or a fractional size
// can reintroduce drift, so they get their own fixed-point gate.
describe("layout gate — optional inputs", () => {
	const scoped = scenarios.filter((scenario) =>
		[
			"branch-diamond",
			"deep-pure-tree",
			"five-event-groups",
			"dense-data-mesh",
			"converging-events",
		].includes(scenario.name),
	);

	for (const style of STYLES) {
		for (const scenario of scoped) {
			const ids = scenario.input.layerNodes.map((node) => node.id);
			const only = new Set(
				ids.slice(0, Math.max(2, Math.ceil(ids.length / 2))),
			);

			test(`${scenario.name} / ${style} — selection-scoped layout is a fixed point`, () => {
				let input: AutoLayoutInput = { ...scenario.input, only };
				const first = computeFlowLayoutDetailed(input, style);
				expect(first.positions.size).toBe(only.size);

				for (let run = 0; run < 3; run++) {
					input = { ...reapply(input, first.positions), only };
					const next = computeFlowLayoutDetailed(input, style);
					expect(serialise(next.positions)).toBe(serialise(first.positions));
				}
			});

			test(`${scenario.name} / ${style} — fractional measured sizes stay a fixed point`, () => {
				const nodeSizes = new Map<string, readonly [number, number]>(
					ids.map((id, index) => [
						id,
						[150.5 + (index % 3) * 11.25, 58.33 + (index % 5) * 7.4] as const,
					]),
				);
				let input: AutoLayoutInput = { ...scenario.input, nodeSizes };
				const first = computeFlowLayoutDetailed(input, style);

				for (let run = 0; run < 3; run++) {
					input = { ...reapply(input, first.positions), nodeSizes };
					const next = computeFlowLayoutDetailed(input, style);
					expect(serialise(next.positions)).toBe(serialise(first.positions));
				}
			});

			test(`${scenario.name} / ${style} — comments follow their nodes and stay stable`, () => {
				const anchorId = ids[0];
				const anchor = scenario.input.layerNodes.find(
					(node) => node.id === anchorId,
				);
				const [ax, ay] = anchor?.coordinates ?? [0, 0];
				const comments = [
					{ id: "note", x: ax - 40, y: ay - 40, width: 600, height: 400 },
				];

				let input: AutoLayoutInput = { ...scenario.input, comments };
				const first = computeFlowLayoutDetailed(input, style);

				for (let run = 0; run < 3; run++) {
					input = { ...reapply(input, first.positions), comments };
					const next = computeFlowLayoutDetailed(input, style);
					expect(serialise(next.positions)).toBe(serialise(first.positions));
				}
			});
		}
	}
});
