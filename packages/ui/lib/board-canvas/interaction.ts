import type { Connection } from "@xyflow/react";
import type {
	Scene,
	SceneEdge,
	SceneEndpoint,
	SceneNode,
	ScenePin,
} from "./scene";

export interface Point {
	x: number;
	y: number;
}
export interface Bounds extends Point {
	width: number;
	height: number;
}
export interface HitObstacle extends Bounds {
	id: string;
}
export interface WorldPin extends ScenePin {
	nodeId: string;
}
export type ConnectionValidator = (
	connection: Connection,
	hoveredPin: WorldPin,
) => boolean;

const CELL_SIZE = 256;
const MAX_CELLS = 128;
export const PIN_SNAP_RADIUS = 22;
export const PIN_RELEASE_RADIUS = 30;
export const RECONNECT_PATH_RADIUS = 6;
export const RECONNECT_END_LENGTH = 32;

const endpointKey = (pin: SceneEndpoint) => `${pin.nodeId}\0${pin.pinId}`;
const pinKey = (pin: WorldPin) =>
	endpointKey({ nodeId: pin.nodeId, pinId: pin.id });
const distance = (a: Point, b: Point) => Math.hypot(a.x - b.x, a.y - b.y);
const contains = (rect: Bounds, point: Point) =>
	point.x >= rect.x &&
	point.x <= rect.x + rect.width &&
	point.y >= rect.y &&
	point.y <= rect.y + rect.height;
const intersects = (a: Bounds, b: Bounds) =>
	a.x <= b.x + b.width &&
	a.x + a.width >= b.x &&
	a.y <= b.y + b.height &&
	a.y + a.height >= b.y;

/** Even-odd containment includes the outline, matching a closed freehand selection. */
export function pointInPolygon(
	point: Point,
	polygon: readonly Point[],
): boolean {
	if (polygon.length < 3) return false;
	let inside = false;
	for (let i = 0, j = polygon.length - 1; i < polygon.length; j = i++) {
		const a = polygon[j];
		const b = polygon[i];
		if (segmentDistance(point, a, b) < 1e-7) return true;
		if (
			a.y > point.y !== b.y > point.y &&
			point.x < ((b.x - a.x) * (point.y - a.y)) / (b.y - a.y) + a.x
		)
			inside = !inside;
	}
	return inside;
}

/** Containment rejects concave cutouts even when all four corners are inside. */
export function boundsInPolygon(
	rect: Bounds,
	polygon: readonly Point[],
): boolean {
	const corners = [
		{ x: rect.x, y: rect.y },
		{ x: rect.x + rect.width, y: rect.y },
		{ x: rect.x + rect.width, y: rect.y + rect.height },
		{ x: rect.x, y: rect.y + rect.height },
	];
	if (!corners.every((point) => pointInPolygon(point, polygon))) return false;
	for (let i = 0; i < polygon.length; i++) {
		const a = polygon[i];
		const b = polygon[(i + 1) % polygon.length];
		let start = 0;
		let end = 1;
		for (const axis of ["x", "y"] as const) {
			const delta = b[axis] - a[axis];
			const min = rect[axis] + 1e-7;
			const max = rect[axis] + (axis === "x" ? rect.width : rect.height) - 1e-7;
			if (delta === 0) {
				if (a[axis] < min || a[axis] > max) end = -1;
			} else {
				const t0 = (min - a[axis]) / delta;
				const t1 = (max - a[axis]) / delta;
				start = Math.max(start, Math.min(t0, t1));
				end = Math.min(end, Math.max(t0, t1));
			}
		}
		if (end >= start) return false;
	}
	return true;
}

/** A single grid correction preserves every selected node's relative offset. */
export function snapGroupToGrid<T extends { id: string; position: Point }>(
	nodes: readonly T[],
	grid: readonly [number, number],
	axes: { x: boolean; y: boolean } = { x: true, y: true },
): T[] {
	const anchor = nodes[0]?.position;
	if (!anchor || !grid.every((step) => Number.isFinite(step) && step > 0))
		return [...nodes];
	const dx = axes.x ? Math.round(anchor.x / grid[0]) * grid[0] - anchor.x : 0;
	const dy = axes.y ? Math.round(anchor.y / grid[1]) * grid[1] - anchor.y : 0;
	return nodes.map((node) => ({
		...node,
		position: { x: node.position.x + dx, y: node.position.y + dy },
	}));
}

/** Long edges use a bounded overflow set instead of allocating thousands of grid cells. */
class SpatialIndex<T> {
	private cells = new Map<string, Set<string>>();
	private entries = new Map<
		string,
		{ value: T; bounds: Bounds; cells: string[] }
	>();
	private overflow = new Set<string>();
	private keys(bounds: Bounds): string[] {
		const x0 = Math.floor(bounds.x / CELL_SIZE);
		const x1 = Math.floor((bounds.x + bounds.width) / CELL_SIZE);
		const y0 = Math.floor(bounds.y / CELL_SIZE);
		const y1 = Math.floor((bounds.y + bounds.height) / CELL_SIZE);
		if ((x1 - x0 + 1) * (y1 - y0 + 1) > MAX_CELLS) return [];
		const keys: string[] = [];
		for (let x = x0; x <= x1; x++)
			for (let y = y0; y <= y1; y++) keys.push(`${x}:${y}`);
		return keys;
	}
	set(id: string, value: T, bounds: Bounds) {
		this.remove(id);
		const cells = this.keys(bounds);
		this.entries.set(id, { value, bounds, cells });
		if (!cells.length) this.overflow.add(id);
		for (const cell of cells) {
			let bucket = this.cells.get(cell);
			if (!bucket) {
				bucket = new Set();
				this.cells.set(cell, bucket);
			}
			bucket.add(id);
		}
	}
	remove(id: string) {
		for (const cell of this.entries.get(id)?.cells ?? []) {
			const bucket = this.cells.get(cell);
			bucket?.delete(id);
			if (!bucket?.size) this.cells.delete(cell);
		}
		this.overflow.delete(id);
		this.entries.delete(id);
	}
	query(bounds: Bounds): T[] {
		const keys = this.keys(bounds);
		const ids = keys.length
			? new Set(this.overflow)
			: new Set(this.entries.keys());
		for (const key of keys)
			for (const id of this.cells.get(key) ?? []) ids.add(id);
		const result: T[] = [];
		for (const id of ids) {
			const entry = this.entries.get(id);
			if (entry && intersects(entry.bounds, bounds)) result.push(entry.value);
		}
		return result;
	}
}

export function connectionFor(
	a: WorldPin,
	b: WorldPin,
): Connection | undefined {
	if (a.nodeId === b.nodeId || a.direction === b.direction) return undefined;
	const [source, target] = a.direction === "output" ? [a, b] : [b, a];
	return {
		source: source.nodeId,
		sourceHandle: source.id,
		target: target.nodeId,
		targetHandle: target.id,
	};
}

export function edgePoints(a: Point, d: Point, vertical = false): Point[] {
	const delta = vertical ? d.y - a.y : d.x - a.x;
	const offset = delta >= 0 ? delta * 0.5 : 6.25 * Math.sqrt(-delta);
	const b = {
		x: a.x + (vertical ? 0 : offset),
		y: a.y + (vertical ? offset : 0),
	};
	const c = {
		x: d.x - (vertical ? 0 : offset),
		y: d.y - (vertical ? offset : 0),
	};
	return Array.from({ length: 33 }, (_, i) => {
		const t = i / 32;
		const u = 1 - t;
		return {
			x:
				a.x * u ** 3 + 3 * b.x * u * u * t + 3 * c.x * u * t * t + d.x * t ** 3,
			y:
				a.y * u ** 3 + 3 * b.y * u * u * t + 3 * c.y * u * t * t + d.y * t ** 3,
		};
	});
}

function segmentDistance(point: Point, a: Point, b: Point) {
	const dx = b.x - a.x;
	const dy = b.y - a.y;
	const t = Math.max(
		0,
		Math.min(
			1,
			((point.x - a.x) * dx + (point.y - a.y) * dy) / (dx * dx + dy * dy || 1),
		),
	);
	return distance(point, { x: a.x + t * dx, y: a.y + t * dy });
}

export class BoardHitIndex {
	readonly nodes = new Map<string, SceneNode>();
	readonly edges = new Map<string, SceneEdge>();
	private pins = new Map<string, WorldPin>();
	private nodeIndex = new SpatialIndex<SceneNode>();
	private pinIndex = new SpatialIndex<WorldPin>();
	private edgeIndex = new SpatialIndex<{ edge: SceneEdge; points: Point[] }>();
	private incident = new Map<string, Set<string>>();
	private selectedEdges: SceneEdge[] = [];
	private obstacleIndex = new SpatialIndex<HitObstacle>();
	constructor(scene: Scene, obstacles: readonly HitObstacle[] = []) {
		for (const obstacle of obstacles)
			this.obstacleIndex.set(obstacle.id, obstacle, obstacle);
		for (const node of scene.nodes) this.setNode({ ...node });
		for (const edge of scene.edges) {
			this.edges.set(edge.id, edge);
			if (edge.selected) this.selectedEdges.push(edge);
			for (const id of [edge.source.nodeId, edge.target.nodeId]) {
				let incident = this.incident.get(id);
				if (!incident) {
					incident = new Set();
					this.incident.set(id, incident);
				}
				incident.add(edge.id);
			}
			this.setEdge(edge);
		}
	}
	private setNode(node: SceneNode) {
		this.nodes.set(node.id, node);
		this.nodeIndex.set(node.id, node, node);
		for (const local of node.pins) {
			const pin = {
				...local,
				nodeId: node.id,
				x: node.x + local.x,
				y: node.y + local.y,
			};
			this.pins.set(pinKey(pin), pin);
			this.pinIndex.set(pinKey(pin), pin, { ...pin, width: 0, height: 0 });
		}
	}
	private setEdge(edge: SceneEdge) {
		const a = this.pin(edge.source);
		const b = this.pin(edge.target);
		if (!a || !b) {
			this.edgeIndex.remove(edge.id);
			return;
		}
		const points = edgePoints(a, b, edge.kind === "reference");
		const xs = points.map((p) => p.x);
		const ys = points.map((p) => p.y);
		const x = Math.min(...xs);
		const y = Math.min(...ys);
		this.edgeIndex.set(
			edge.id,
			{ edge, points },
			{ x, y, width: Math.max(...xs) - x, height: Math.max(...ys) - y },
		);
	}
	moveNodes(positions: readonly { id: string; x: number; y: number }[]) {
		const affected = new Set<string>();
		for (const position of positions) {
			const node = this.nodes.get(position.id);
			if (!node) continue;
			this.setNode({ ...node, ...position });
			for (const edge of this.incident.get(node.id) ?? []) affected.add(edge);
		}
		for (const id of affected) {
			const edge = this.edges.get(id);
			if (edge) this.setEdge(edge);
		}
	}
	pin(endpoint: SceneEndpoint): WorldPin | undefined {
		return this.pins.get(endpointKey(endpoint));
	}
	nodeAt(point: Point): SceneNode | undefined {
		return this.nodeIndex
			.query({ ...point, width: 0, height: 0 })
			.filter((n) => contains(n, point))
			.at(-1);
	}
	nodesInside(rect: Bounds): SceneNode[] {
		return this.nodeIndex
			.query(rect)
			.filter(
				(n) =>
					contains(rect, n) &&
					contains(rect, { x: n.x + n.width, y: n.y + n.height }),
			);
	}
	blockedAt(point: Point): boolean {
		return (
			!!this.nodeAt(point) ||
			this.obstacleIndex
				.query({ ...point, width: 0, height: 0 })
				.some((obstacle) => contains(obstacle, point))
		);
	}
	closestPin(
		point: Point,
		zoom: number,
		filter: (pin: WorldPin) => boolean = () => true,
		radiusPx = PIN_SNAP_RADIUS,
	): WorldPin | undefined {
		const radius = radiusPx / Math.max(zoom, 0.01);
		let nearest: WorldPin | undefined;
		let best = radius;
		for (const pin of this.pinIndex.query({
			x: point.x - radius,
			y: point.y - radius,
			width: radius * 2,
			height: radius * 2,
		})) {
			const d = distance(point, pin);
			if (d <= best && filter(pin)) {
				nearest = pin;
				best = d;
			}
		}
		return nearest;
	}
	edgeAt(point: Point, zoom: number): SceneEdge | undefined {
		const radius = 7 / Math.max(zoom, 0.01);
		let nearest: SceneEdge | undefined;
		let best = radius;
		for (const { edge, points } of this.edgeIndex.query({
			x: point.x - radius,
			y: point.y - radius,
			width: radius * 2,
			height: radius * 2,
		})) {
			for (let i = 1; i < points.length; i++) {
				const d = segmentDistance(point, points[i - 1], points[i]);
				if (d <= best) {
					nearest = edge;
					best = d;
				}
			}
		}
		return nearest;
	}
	/** Only the short curve corridor attached to an endpoint can start a reconnect. */
	nearEndpoint(
		point: Point,
		zoom: number,
		filter: (pin: WorldPin) => boolean = () => true,
		explicitEdgeId?: string,
		canReconnect: (edge: SceneEdge, end: "source" | "target") => boolean = () =>
			true,
	): { edge: SceneEdge; end: "source" | "target" } | undefined {
		const scale = Math.max(zoom, 0.01);
		const radius = RECONNECT_PATH_RADIUS / scale;
		const length = RECONNECT_END_LENGTH / scale;
		let result: { edge: SceneEdge; end: "source" | "target" } | undefined;
		let best = Number.POSITIVE_INFINITY;
		let bestEndpoint = Number.POSITIVE_INFINITY;
		const consider = (edge: SceneEdge, end: "source" | "target", d: number) => {
			const pin = this.pin(edge[end]);
			if (!pin || !filter(pin) || !canReconnect(edge, end)) return;
			const endpointDistance = distance(point, pin);
			const key = `${edge.id}\0${end}`;
			const previousKey = result ? `${result.edge.id}\0${result.end}` : "";
			if (
				d < best ||
				(d === best &&
					(endpointDistance < bestEndpoint ||
						(endpointDistance === bestEndpoint && key < previousKey)))
			) {
				result = { edge, end };
				best = d;
				bestEndpoint = endpointDistance;
			}
		};
		if (explicitEdgeId !== undefined) {
			// The SVG hit target supplies the actual path for straight and stepped edges.
			const edge = this.edges.get(explicitEdgeId);
			if (edge)
				for (const end of ["source", "target"] as const) {
					const pin = this.pin(edge[end]);
					if (pin && distance(pin, point) <= length) consider(edge, end, 0);
				}
			return result;
		}
		for (const { edge, points } of this.edgeIndex.query({
			x: point.x - radius,
			y: point.y - radius,
			width: radius * 2,
			height: radius * 2,
		})) {
			for (const end of ["source", "target"] as const) {
				const start = end === "source" ? 0 : points.length - 1;
				const step = end === "source" ? 1 : -1;
				if (distance(point, points[start]) > length + radius) continue;
				let remaining = length;
				for (
					let i = start;
					i + step >= 0 && i + step < points.length;
					i += step
				) {
					const a = points[i];
					const next = points[i + step];
					const segmentLength = distance(a, next);
					const fraction = Math.min(1, remaining / (segmentLength || 1));
					const b = {
						x: a.x + (next.x - a.x) * fraction,
						y: a.y + (next.y - a.y) * fraction,
					};
					const d = segmentDistance(point, a, b);
					if (d <= radius) consider(edge, end, d);
					remaining -= segmentLength;
					if (remaining <= 0) break;
				}
			}
		}
		return result;
	}
	selectedEndpoint(
		point: Point,
		zoom: number,
		canReconnect: (edge: SceneEdge, end: "source" | "target") => boolean = () =>
			true,
	): { edge: SceneEdge; end: "source" | "target" } | undefined {
		let result: { edge: SceneEdge; end: "source" | "target" } | undefined;
		let best = 12 / Math.max(zoom, 0.01);
		for (const edge of this.selectedEdges) {
			for (const end of ["source", "target"] as const) {
				const pin = this.pin(edge[end]);
				if (pin && canReconnect(edge, end) && distance(pin, point) <= best) {
					result = { edge, end };
					best = distance(pin, point);
				}
			}
		}
		return result;
	}
}

export type ConnectionDrop =
	| { kind: "connect"; connection: Connection; oldEdgeId?: string }
	| { kind: "disconnect"; edgeId: string }
	| { kind: "palette"; pin: WorldPin }
	| { kind: "cancel" | "noop" };

/** Transient intent only. Confirmed graph state is changed by the host command service. */
export class ConnectionGesture {
	private target?: SceneEndpoint;
	private originalEndpoints?: string;
	constructor(
		readonly anchor: SceneEndpoint,
		readonly reconnect?: { edgeId: string; end: "source" | "target" },
	) {}
	resolve(
		index: BoardHitIndex,
		point: Point,
		zoom: number,
		validate: ConnectionValidator,
	): { anchor: WorldPin; target?: WorldPin; invalid?: WorldPin } | undefined {
		const anchor = index.pin(this.anchor);
		if (!anchor || (this.reconnect && !index.edges.has(this.reconnect.edgeId)))
			return undefined;
		if (this.reconnect) {
			const edge = index.edges.get(this.reconnect.edgeId);
			if (!edge) return undefined;
			const retained =
				edge[this.reconnect.end === "source" ? "target" : "source"];
			if (endpointKey(retained) !== endpointKey(this.anchor)) return undefined;
			const endpoints = JSON.stringify([edge.source, edge.target]);
			if (
				this.originalEndpoints !== undefined &&
				this.originalEndpoints !== endpoints
			)
				return undefined;
			this.originalEndpoints = endpoints;
		}
		const valid = (pin: WorldPin) => {
			const connection = connectionFor(anchor, pin);
			return !!connection && validate(connection, pin);
		};
		const held = this.target && index.pin(this.target);
		const heldDistance = held
			? distance(held, point) * zoom
			: Number.POSITIVE_INFINITY;
		const retainHeld =
			held && heldDistance <= PIN_RELEASE_RADIUS && valid(held);
		const nearest = index.closestPin(point, zoom, (pin) =>
			pin === held ? !!retainHeld : valid(pin),
		);
		let target = nearest;
		if (held && retainHeld) {
			// Keep a small midpoint dead band, but let adjacent compatible pins win.
			// Scale it down when zoom makes their screen-space separation tiny.
			const margin = nearest
				? Math.min(2, distance(held, nearest) * zoom * 0.2)
				: 0;
			if (!nearest || distance(nearest, point) * zoom + margin >= heldDistance)
				target = held;
		}
		this.target = target && { nodeId: target.nodeId, pinId: target.id };
		return {
			anchor,
			target,
			invalid: target
				? undefined
				: index.closestPin(
						point,
						zoom,
						(pin) => pinKey(pin) !== pinKey(anchor),
					),
		};
	}
	drop(
		index: BoardHitIndex,
		point: Point,
		zoom: number,
		validate: ConnectionValidator,
	): ConnectionDrop {
		const resolved = this.resolve(index, point, zoom, validate);
		if (!resolved) return { kind: "cancel" };
		const oldEdge = this.reconnect && index.edges.get(this.reconnect.edgeId);
		if (resolved.target) {
			const connection = connectionFor(resolved.anchor, resolved.target);
			if (!connection) return { kind: "cancel" };
			if (
				oldEdge &&
				oldEdge.source.nodeId === connection.source &&
				oldEdge.source.pinId === connection.sourceHandle &&
				oldEdge.target.nodeId === connection.target &&
				oldEdge.target.pinId === connection.targetHandle
			)
				return { kind: "noop" };
			return { kind: "connect", connection, oldEdgeId: oldEdge?.id };
		}
		if (resolved.invalid) return { kind: "cancel" };
		// A new wire opens the palette anywhere, as on React Flow. A moved wire end only
		// disconnects on open canvas; node bodies, comments and media keep it.
		if (!oldEdge) return { kind: "palette", pin: resolved.anchor };
		return index.blockedAt(point)
			? { kind: "cancel" }
			: { kind: "disconnect", edgeId: oldEdge.id };
	}
}
