import {
	Position,
	getBezierPath,
	getSmoothStepPath,
	getStraightPath,
} from "@xyflow/react";
import { buildLayoutGraph } from "./build";
import { REROUTE_HEIGHT, REROUTE_WIDTH, stableHandleOffset } from "./measure";
import type { AutoLayoutInput, LEdge, LayoutBox } from "./types";

interface Point {
	x: number;
	y: number;
}

export interface DataRoute {
	from: string;
	to: string;
	fromPin: string;
	toPin: string;
	/** Top-left positions of the reroute nodes, in connection order. */
	waypoints: Point[];
	/** No bounded candidate cleared the nodes; retain the original connection. */
	unresolved?: boolean;
}

interface Bounds {
	left: number;
	top: number;
	right: number;
	bottom: number;
}

interface Segment extends Bounds {
	a: Point;
	b: Point;
}

interface Obstacle extends Bounds {
	id: string;
}

interface Wire {
	key: string;
	edge: LEdge;
	source: Point;
	target: Point;
	segments: Segment[];
}

interface WireSegment extends Segment {
	wire: Wire;
}

const CLEARANCE = 8;
const LANE_GAP = 24;
const CELL_SIZE = 128;
const FLATNESS = 0.25;
const MAX_CANDIDATES = 48;

function overlaps(a: Bounds, b: Bounds): boolean {
	return (
		a.left <= b.right &&
		a.right >= b.left &&
		a.top <= b.bottom &&
		a.bottom >= b.top
	);
}

/** Index individual segments so a long wire does not scan the whole board. */
class SpatialIndex<T extends Bounds> {
	private cells = new Map<string, Set<T>>();
	private wide = new Set<T>();

	private keys(box: Bounds): string[] {
		const x0 = Math.floor(box.left / CELL_SIZE);
		const y0 = Math.floor(box.top / CELL_SIZE);
		const x1 = Math.floor(box.right / CELL_SIZE);
		const y1 = Math.floor(box.bottom / CELL_SIZE);
		if ((x1 - x0 + 1) * (y1 - y0 + 1) > 256) return [];
		const keys: string[] = [];
		for (let x = x0; x <= x1; x++) {
			for (let y = y0; y <= y1; y++) keys.push(`${x}:${y}`);
		}
		return keys;
	}

	add(value: T): void {
		const keys = this.keys(value);
		if (keys.length === 0) this.wide.add(value);
		for (const key of keys) {
			const cell = this.cells.get(key) ?? new Set<T>();
			cell.add(value);
			this.cells.set(key, cell);
		}
	}

	remove(value: T): void {
		this.wide.delete(value);
		for (const key of this.keys(value)) this.cells.get(key)?.delete(value);
	}

	query(box: Bounds): Set<T> {
		const result = new Set<T>();
		const keys = this.keys(box);
		const cells = keys.length
			? keys.map((key) => this.cells.get(key))
			: this.cells.values();
		for (const cell of cells) {
			for (const value of cell ?? []) {
				if (overlaps(box, value)) result.add(value);
			}
		}
		for (const value of this.wide) {
			if (overlaps(box, value)) result.add(value);
		}
		return result;
	}
}

function segment(a: Point, b: Point): Segment {
	return {
		a,
		b,
		left: Math.min(a.x, b.x),
		top: Math.min(a.y, b.y),
		right: Math.max(a.x, b.x),
		bottom: Math.max(a.y, b.y),
	};
}

function distance(a: Point, b: Point): number {
	return Math.hypot(a.x - b.x, a.y - b.y);
}

function midpoint(a: Point, b: Point): Point {
	return { x: (a.x + b.x) / 2, y: (a.y + b.y) / 2 };
}

function distanceToSegment(point: Point, a: Point, b: Point): number {
	const dx = b.x - a.x;
	const dy = b.y - a.y;
	const length = dx * dx + dy * dy;
	const t = length
		? Math.max(
				0,
				Math.min(1, ((point.x - a.x) * dx + (point.y - a.y) * dy) / length),
			)
		: 0;
	return Math.hypot(point.x - a.x - t * dx, point.y - a.y - t * dy);
}

function flattenCubic(
	a: Point,
	b: Point,
	c: Point,
	d: Point,
	points: Point[],
	depth = 0,
): void {
	if (
		depth >= 12 ||
		Math.max(distanceToSegment(b, a, d), distanceToSegment(c, a, d)) <= FLATNESS
	) {
		points.push(d);
		return;
	}
	const ab = midpoint(a, b);
	const bc = midpoint(b, c);
	const cd = midpoint(c, d);
	const abc = midpoint(ab, bc);
	const bcd = midpoint(bc, cd);
	const center = midpoint(abc, bcd);
	flattenCubic(a, ab, abc, center, points, depth + 1);
	flattenCubic(center, bcd, cd, d, points, depth + 1);
}

/** Flatten the same SVG commands emitted by the renderer, including rounded steps. */
function samplePath(path: string): Point[] {
	const tokens = path.match(/[MLCQ]|-?(?:\d*\.)?\d+(?:e[-+]?\d+)?/gi) ?? [];
	const points: Point[] = [];
	let index = 0;
	let current: Point = { x: 0, y: 0 };
	const point = (): Point => ({
		x: Number(tokens[index++]),
		y: Number(tokens[index++]),
	});
	while (index < tokens.length) {
		const command = tokens[index++];
		if (command === "M" || command === "L") {
			current = point();
			points.push(current);
		} else if (command === "C") {
			const first = point();
			const second = point();
			const end = point();
			flattenCubic(current, first, second, end, points);
			current = end;
		} else if (command === "Q") {
			const control = point();
			const end = point();
			flattenCubic(
				current,
				{
					x: current.x + ((control.x - current.x) * 2) / 3,
					y: current.y + ((control.y - current.y) * 2) / 3,
				},
				{
					x: end.x + ((control.x - end.x) * 2) / 3,
					y: end.y + ((control.y - end.y) * 2) / 3,
				},
				end,
				points,
			);
			current = end;
		}
	}
	return points;
}

function sampleLeg(
	source: Point,
	target: Point,
	pathType: AutoLayoutInput["edgePathType"],
): Point[] {
	const args = {
		sourceX: source.x,
		sourceY: source.y,
		targetX: target.x,
		targetY: target.y,
		sourcePosition: Position.Right,
		targetPosition: Position.Left,
	};
	const [path] =
		pathType === "straight"
			? getStraightPath(args)
			: pathType === "step" || pathType === "smoothstep"
				? getSmoothStepPath(args)
				: getBezierPath(args);
	return samplePath(path);
}

function routeSegments(
	wire: Wire,
	waypoints: Point[],
	pathType: AutoLayoutInput["edgePathType"],
): Segment[] {
	const result: Segment[] = [];
	let source = wire.source;
	for (let index = 0; index <= waypoints.length; index++) {
		const dot = waypoints[index];
		const target = dot
			? { x: dot.x, y: dot.y + REROUTE_HEIGHT / 2 }
			: wire.target;
		const points = sampleLeg(source, target, pathType);
		for (let i = 1; i < points.length; i++) {
			if (distance(points[i - 1], points[i]) > 0.001)
				result.push(segment(points[i - 1], points[i]));
		}
		if (dot) source = { x: dot.x + REROUTE_WIDTH, y: target.y };
	}
	return result;
}

function obstacle(id: string, box: LayoutBox, padding = CLEARANCE): Obstacle {
	return {
		id,
		left: box.x - padding,
		top: box.y - padding,
		right: box.x + box.width + padding,
		bottom: box.y + box.height + padding,
	};
}

function hitsBox(line: Segment, box: Bounds): boolean {
	if (!overlaps(line, box)) return false;
	let low = 0;
	let high = 1;
	for (const [origin, delta, min, max] of [
		[line.a.x, line.b.x - line.a.x, box.left, box.right],
		[line.a.y, line.b.y - line.a.y, box.top, box.bottom],
	]) {
		if (Math.abs(delta) < 0.0001) {
			if (origin < min || origin > max) return false;
		} else {
			const first = (min - origin) / delta;
			const last = (max - origin) / delta;
			low = Math.max(low, Math.min(first, last));
			high = Math.min(high, Math.max(first, last));
			if (low > high) return false;
		}
	}
	return true;
}

function collisions(
	wire: Wire,
	segments: Segment[],
	boxes: SpatialIndex<Obstacle>,
): number {
	const hit = new Set<string>();
	for (const line of segments) {
		for (const box of boxes.query(line)) {
			// The handle's own padding is open on its outward side.
			if (box.id === wire.edge.from && line.left >= wire.source.x - FLATNESS)
				continue;
			if (box.id === wire.edge.to && line.right <= wire.target.x + FLATNESS)
				continue;
			if (!hit.has(box.id) && hitsBox(line, box)) hit.add(box.id);
		}
	}
	return hit.size;
}

function intersects(a: Segment, b: Segment): boolean {
	if (!overlaps(a, b)) return false;
	const cross = (p: Point, q: Point, r: Point) =>
		(q.x - p.x) * (r.y - p.y) - (q.y - p.y) * (r.x - p.x);
	const a1 = cross(a.a, a.b, b.a);
	const a2 = cross(a.a, a.b, b.b);
	const b1 = cross(b.a, b.b, a.a);
	const b2 = cross(b.a, b.b, a.b);
	if (Math.abs(a1) + Math.abs(a2) + Math.abs(b1) + Math.abs(b2) < 0.001) {
		return (
			Math.max(
				Math.min(a.right, b.right) - Math.max(a.left, b.left),
				Math.min(a.bottom, b.bottom) - Math.max(a.top, b.top),
			) > 12
		);
	}
	return a1 * a2 <= 0 && b1 * b2 <= 0;
}

function crossings(
	wire: Wire,
	segments: Segment[],
	wires: SpatialIndex<WireSegment>,
): number {
	const crossed = new Map<string, Point[]>();
	const overlapping = new Set<string>();
	for (const line of segments) {
		for (const other of wires.query(line)) {
			if (other.wire.key === wire.key) continue;
			// Shared ports are intentional junctions. Crossings elsewhere still count.
			const shared =
				wire.edge.fromPin.id === other.wire.edge.fromPin.id
					? wire.source
					: wire.edge.toPin.id === other.wire.edge.toPin.id
						? wire.target
						: undefined;
			if (shared) {
				if (
					distanceToSegment(shared, line.a, line.b) < 1 &&
					distanceToSegment(shared, other.a, other.b) < 1
				)
					continue;
				if (
					distanceToSegment(line.a, other.a, other.b) < FLATNESS &&
					distanceToSegment(line.b, other.a, other.b) < FLATNESS
				)
					continue;
			}
			if (!intersects(line, other)) continue;
			const dx = line.b.x - line.a.x;
			const dy = line.b.y - line.a.y;
			const otherDx = other.b.x - other.a.x;
			const otherDy = other.b.y - other.a.y;
			const determinant = dx * otherDy - dy * otherDx;
			if (Math.abs(determinant) < 0.000001) {
				overlapping.add(other.wire.key);
				continue;
			}
			const t =
				((other.a.x - line.a.x) * otherDy - (other.a.y - line.a.y) * otherDx) /
				determinant;
			const point = { x: line.a.x + t * dx, y: line.a.y + t * dy };
			const points = crossed.get(other.wire.key) ?? [];
			// Flattened curves can report the same crossing on adjacent segments.
			// Separate intersections of the same pair of wires must still count.
			if (!points.some((existing) => distance(existing, point) < 1)) {
				points.push(point);
				crossed.set(other.wire.key, points);
			}
		}
	}
	return (
		[...crossed.values()].reduce((sum, points) => sum + points.length, 0) +
		overlapping.size
	);
}

function makeWires(
	input: AutoLayoutInput,
	positions: ReadonlyMap<string, [number, number]>,
) {
	const graph = buildLayoutGraph({ ...input, only: undefined });
	const original = new Map<string, number[]>();
	for (const node of input.layerNodes)
		original.set(node.id, node.coordinates ?? [0, 0]);
	for (const node of input.layerEntities)
		original.set(node.id, node.coordinates);
	const boxes: Obstacle[] = [];
	for (const id of graph.order) {
		const node = graph.nodes.get(id);
		const xy = positions.get(id) ?? original.get(id);
		if (!node || !xy) continue;
		node.x = xy[0];
		node.y = xy[1];
		boxes.push(
			obstacle(id, {
				x: node.x,
				y: node.y,
				width: node.width,
				height: node.height,
			}),
		);
	}
	for (const [index, box] of (input.obstacles ?? []).entries())
		boxes.push(obstacle(`obstacle:${index}`, box));
	const wires: Wire[] = [];
	for (const edge of graph.edges) {
		const from = graph.nodes.get(edge.from);
		const to = graph.nodes.get(edge.to);
		if (!from || !to) continue;
		const fromOffset = input.pinOffsets?.get(edge.fromPin.id);
		const toOffset = input.pinOffsets?.get(edge.toPin.id);
		const wire: Wire = {
			key: `${edge.fromPin.id}->${edge.toPin.id}`,
			edge,
			source: {
				x: from.x + stableHandleOffset(fromOffset?.x ?? from.width),
				y: from.y + edge.fromPin.offsetY,
			},
			target: {
				x: to.x + stableHandleOffset(toOffset?.x ?? 0),
				y: to.y + edge.toPin.offsetY,
			},
			segments: [],
		};
		wire.segments = routeSegments(wire, [], input.edgePathType);
		wires.push(wire);
	}
	return { wires, boxes };
}

function candidates(wire: Wire, boxes: Obstacle[]): Point[][] {
	const { source, target } = wire;
	const left = Math.min(source.x, target.x) - 64;
	const right = Math.max(source.x, target.x) + 64;
	const nearby = boxes.filter((box) => box.left <= right && box.right >= left);
	const values = new Set<number>();
	for (const y of [source.y, target.y, (source.y + target.y) / 2]) {
		for (const delta of [-48, -24, 0, 24, 48])
			values.add(Math.round(y + delta));
	}
	for (const box of nearby) {
		values.add(Math.floor(box.top - LANE_GAP));
		values.add(Math.ceil(box.bottom + LANE_GAP));
	}
	const minY =
		Math.min(source.y, target.y, ...nearby.map((box) => box.top)) - LANE_GAP;
	const maxY =
		Math.max(source.y, target.y, ...nearby.map((box) => box.bottom)) + LANE_GAP;
	const cost = (y: number) => Math.abs(source.y - y) + Math.abs(target.y - y);
	const lanes = [...values]
		.sort((a, b) => cost(a) - cost(b) || a - b)
		.slice(0, 14);
	lanes.push(Math.floor(minY), Math.ceil(maxY));
	const result: Point[][] = [];
	const dot = (x: number, y: number): Point => ({
		x: Math.round(x),
		y: Math.round(y - REROUTE_HEIGHT / 2),
	});
	const forward = target.x - source.x;
	for (const y of [...new Set(lanes)]) {
		if (forward >= 48 && forward < 104) {
			result.push([dot((source.x + target.x - REROUTE_WIDTH) / 2, y)]);
		} else if (forward >= 104) {
			for (const inset of [24, 40]) {
				result.push([
					dot(source.x + inset, y),
					dot(target.x - inset - REROUTE_WIDTH, y),
				]);
			}
		} else {
			// Backward links leave both endpoints outward before using an outer lane.
			result.push([
				dot(source.x + 24, source.y),
				dot(source.x + 64, y),
				dot(target.x - 80, y),
				dot(target.x - 40, target.y),
			]);
		}
	}
	if (forward >= 160) {
		for (const y of lanes.slice(0, 12)) {
			result.push([
				dot(source.x + 20, source.y),
				dot(source.x + 60, y),
				dot(target.x - 76, y),
				dot(target.x - 36, target.y),
			]);
		}
	}
	return result.slice(0, MAX_CANDIDATES);
}

/** Samples each rendered segment separately; reroute node interiors are omitted. */
export function sampleDataRoute(
	input: AutoLayoutInput,
	positions: ReadonlyMap<string, [number, number]>,
	route: DataRoute,
): Array<readonly [Point, Point]> {
	const wire = makeWires(input, positions).wires.find(
		(candidate) => candidate.key === `${route.fromPin}->${route.toPin}`,
	);
	return wire
		? routeSegments(wire, route.waypoints, input.edgePathType).map(
				({ a, b }) => [a, b] as const,
			)
		: [];
}

function routeInOrder(
	input: AutoLayoutInput,
	positions: ReadonlyMap<string, [number, number]>,
	order: "longest" | "shortest" | "reverse",
) {
	const { wires, boxes } = makeWires(input, positions);
	const boxIndex = new SpatialIndex<Obstacle>();
	for (const box of boxes) boxIndex.add(box);
	const wireIndex = new SpatialIndex<WireSegment>();
	const indexed = new Map<string, WireSegment[]>();
	const addWire = (wire: Wire) => {
		const segments = wire.segments.map((line) => ({ ...line, wire }));
		indexed.set(wire.key, segments);
		for (const line of segments) wireIndex.add(line);
	};
	for (const wire of wires) addWire(wire);
	const retainedReroutes = new Set(
		input.layerNodes.filter((node) => node.auto_reroute).map((node) => node.id),
	);
	const eligible = wires.filter(
		({ edge }) =>
			edge.kind === "data" &&
			positions.has(edge.from) &&
			positions.has(edge.to) &&
			!retainedReroutes.has(edge.from) &&
			!retainedReroutes.has(edge.to) &&
			(!input.only || (input.only.has(edge.from) && input.only.has(edge.to))),
	);
	const initialCollisions = new Map(
		eligible.map((wire) => [
			wire.key,
			collisions(wire, wire.segments, boxIndex),
		]),
	);
	eligible.sort(
		(a, b) =>
			(initialCollisions.get(b.key) ?? 0) -
				(initialCollisions.get(a.key) ?? 0) ||
			(order === "shortest" ? -1 : 1) *
				(Math.abs(b.target.x - b.source.x) -
					Math.abs(a.target.x - a.source.x)) ||
			a.key.localeCompare(b.key),
	);
	if (order === "reverse") eligible.reverse();
	const results = new Map<string, DataRoute>();
	const routeBoxes = new Map<string, Obstacle[]>();
	// Revisit early choices after all wires have actual routes.
	for (let pass = 0; pass < 3; pass++) {
		let changed = false;
		for (const wire of eligible) {
			const oldBoxes = routeBoxes.get(wire.key) ?? [];
			for (const box of oldBoxes) {
				boxIndex.remove(box);
				boxes.splice(boxes.indexOf(box), 1);
			}
			const originalHits = collisions(wire, wire.segments, boxIndex);
			const originalCrossings = crossings(wire, wire.segments, wireIndex);
			const route: DataRoute = results.get(wire.key) ?? {
				from: wire.edge.from,
				to: wire.edge.to,
				fromPin: wire.edge.fromPin.id,
				toPin: wire.edge.toPin.id,
				waypoints: [],
			};
			results.set(wire.key, route);
			const restoreBoxes = () => {
				for (const box of oldBoxes) {
					boxes.push(box);
					boxIndex.add(box);
				}
			};
			if (originalHits === 0 && originalCrossings === 0) {
				restoreBoxes();
				continue;
			}
			let bestCost = originalHits
				? Number.POSITIVE_INFINITY
				: originalCrossings * 2000 +
					route.waypoints.length * 80 +
					wire.segments.reduce(
						(sum, line) => sum + distance(line.a, line.b),
						0,
					);
			let bestSegments: Segment[] | undefined;
			for (const points of candidates(wire, boxes)) {
				const dots = points.map((point, index) =>
					obstacle(
						`candidate:${index}`,
						{ ...point, width: REROUTE_WIDTH, height: REROUTE_HEIGHT },
						0,
					),
				);
				if (
					dots.some(
						(dot, index) =>
							boxIndex.query(dot).size > 0 ||
							dots.slice(0, index).some((other) => overlaps(dot, other)),
					)
				)
					continue;
				if (
					dots.some((dot) =>
						[...wireIndex.query(dot)].some(
							(line) => line.wire.key !== wire.key && hitsBox(line, dot),
						),
					)
				)
					continue;
				const segments = routeSegments(wire, points, input.edgePathType);
				if (collisions(wire, segments, boxIndex) > 0) continue;
				if (
					dots.some((dot) =>
						segments.some(
							(line) =>
								line.left < dot.right - FLATNESS &&
								line.right > dot.left + FLATNESS &&
								hitsBox(line, {
									...dot,
									left: dot.left + FLATNESS,
									right: dot.right - FLATNESS,
								}),
						),
					)
				)
					continue;
				const crossed = crossings(wire, segments, wireIndex);
				if (!originalHits && crossed >= originalCrossings) continue;
				const cost =
					crossed * 2000 +
					points.length * 80 +
					segments.reduce((sum, line) => sum + distance(line.a, line.b), 0);
				if (cost >= bestCost) continue;
				bestCost = cost;
				route.waypoints = points;
				bestSegments = segments;
			}
			if (!bestSegments) {
				if (originalHits) route.unresolved = true;
				restoreBoxes();
				continue;
			}
			route.unresolved = undefined;
			changed = true;
			for (const line of indexed.get(wire.key) ?? []) wireIndex.remove(line);
			wire.segments = bestSegments;
			addWire(wire);
			const newBoxes: Obstacle[] = [];
			for (const [index, point] of route.waypoints.entries()) {
				const box = obstacle(`${wire.key}:${index}`, {
					...point,
					width: REROUTE_WIDTH,
					height: REROUTE_HEIGHT,
				});
				boxes.push(box);
				boxIndex.add(box);
				newBoxes.push(box);
			}
			routeBoxes.set(wire.key, newBoxes);
		}
		if (!changed) break;
	}
	let hits = 0;
	let crossingCount = 0;
	let length = 0;
	for (const wire of eligible) {
		const dots = routeBoxes.get(wire.key) ?? [];
		for (const dot of dots) boxIndex.remove(dot);
		hits += collisions(wire, wire.segments, boxIndex);
		for (const dot of dots) boxIndex.add(dot);
		crossingCount += crossings(wire, wire.segments, wireIndex);
		length += wire.segments.reduce(
			(sum, line) => sum + distance(line.a, line.b),
			0,
		);
	}
	const routes = [...results.values()].sort(
		(a, b) =>
			a.fromPin.localeCompare(b.fromPin) || a.toPin.localeCompare(b.toPin),
	);
	const dots = routes.reduce((sum, route) => sum + route.waypoints.length, 0);
	return {
		routes,
		crossingCount,
		score: hits * 1_000_000_000 + crossingCount * 2000 + dots * 80 + length,
	};
}

/** Keeps node placement fixed and searches bounded, deterministic wire corridors. */
export function planDataRoutes(
	input: AutoLayoutInput,
	positions: ReadonlyMap<string, [number, number]>,
): DataRoute[] {
	let best = routeInOrder(input, positions, "longest");
	// Early lane choices constrain later wires. Try other orders on tangled boards.
	if (best.crossingCount > 0 && best.routes.length > 1) {
		for (const order of ["shortest", "reverse"] as const) {
			const candidate = routeInOrder(input, positions, order);
			if (candidate.score < best.score) best = candidate;
		}
	}
	return best.routes;
}
