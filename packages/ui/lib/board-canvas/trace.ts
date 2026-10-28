import type { Scene, SceneEdge, SceneEndpoint, SceneNode } from "./scene";

/** The wires a pin or a clicked wire reaches, followed through reroutes to the pins at their far ends. */
export interface WireTrace {
	/** The traced pin; none when the trace follows a clicked wire. */
	origin?: SceneEndpoint;
	edges: string[];
	reroutes: string[];
	ends: SceneEndpoint[];
	/** The clicked wire, drawn strongest. */
	segment?: string;
}

export interface BoardTrace {
	hover?: WireTrace;
	/** The clicked pin or wire. The canvas draws it strongest and previews its off-screen ends. */
	focus?: WireTrace;
	/** Host chrome over the canvas, `[x, y, width, height]` in canvas pixels, that previews avoid. */
	avoid?: [number, number, number, number][];
}

export const NO_TRACE: BoardTrace = {};

/** What a trace starts from: a pin, or a wire and the path it lies on. */
export type TraceAnchor = { pin: SceneEndpoint } | { edgeId: string };

export const sameAnchor = (a?: TraceAnchor, b?: TraceAnchor) =>
	a && b && "pin" in a && "pin" in b
		? sameEndpoint(a.pin, b.pin)
		: a && b && "edgeId" in a && "edgeId" in b
			? a.edgeId === b.edgeId
			: a === b;

/** How a wire is drawn under a trace; `dim` while another path is focused. */
export type WireEmphasis = "hover" | "focus" | "selected" | "dim";

const endpointKey = (endpoint: SceneEndpoint) =>
	`${endpoint.nodeId}\0${endpoint.pinId}`;

export const sameEndpoint = (a?: SceneEndpoint, b?: SceneEndpoint) =>
	a?.nodeId === b?.nodeId && a?.pinId === b?.pinId;

/** Wires by pin, built once per scene. */
export class WireIndex {
	private readonly nodes: Map<string, SceneNode>;
	private readonly edges: Map<string, SceneEdge>;
	private readonly wires = new Map<string, SceneEdge[]>();
	constructor(scene: Scene) {
		this.nodes = new Map(scene.nodes.map((node) => [node.id, node]));
		this.edges = new Map(scene.edges.map((edge) => [edge.id, edge]));
		for (const edge of scene.edges)
			for (const end of [edge.source, edge.target]) {
				const key = endpointKey(end);
				const wires = this.wires.get(key);
				if (wires) wires.push(edge);
				else this.wires.set(key, [edge]);
			}
	}

	trace(anchor: TraceAnchor): WireTrace | undefined {
		return "pin" in anchor
			? this.fromPin(anchor.pin)
			: this.path(anchor.edgeId);
	}

	/**
	 * Follows a pin's wires through reroutes, in both directions when the pin is a reroute's.
	 * A reroute with nothing beyond it ends the path. Undefined when the pin has no wires.
	 */
	fromPin(origin: SceneEndpoint): WireTrace | undefined {
		const starts = this.starts(origin);
		if (!starts) return undefined;
		const walk = new TraceWalk(starts);
		if (starts[0] !== origin) walk.reroutes.add(origin.nodeId);
		this.follow(walk);
		return walk.result({ origin });
	}

	/**
	 * The path a wire lies on: it and, through the reroutes at either end, the wires up to the
	 * pins it connects. Other wires leaving those pins are not on it.
	 */
	path(edgeId: string): WireTrace | undefined {
		const edge = this.edges.get(edgeId);
		if (!edge) return undefined;
		const walk = new TraceWalk([]);
		walk.cross(edge, edge.target, () => this.beyond(edge.target));
		walk.reach(edge.source, () => this.beyond(edge.source));
		this.follow(walk);
		return walk.result({ segment: edgeId });
	}

	private follow(walk: TraceWalk) {
		for (let from = walk.next(); from; from = walk.next()) {
			const fromKey = endpointKey(from);
			for (const edge of this.wires.get(fromKey) ?? []) {
				const far =
					endpointKey(edge.source) === fromKey ? edge.target : edge.source;
				walk.cross(edge, far, () => this.beyond(far));
			}
		}
	}

	/** A reroute's pins all lead somewhere, so a trace from one starts at each. */
	private starts(origin: SceneEndpoint): SceneEndpoint[] | undefined {
		const node = this.nodes.get(origin.nodeId);
		if (!node?.pins.some((pin) => pin.id === origin.pinId)) return undefined;
		if (node.kind !== "reroute") return [origin];
		return node.pins.map((pin) => ({ nodeId: node.id, pinId: pin.id }));
	}

	/** The wired pins a path continues from when it arrives at a reroute; undefined elsewhere. */
	private beyond(arrival: SceneEndpoint): SceneEndpoint[] | undefined {
		const reroute = this.nodes.get(arrival.nodeId);
		if (reroute?.kind !== "reroute") return undefined;
		const direction = reroute.pins.find(
			(pin) => pin.id === arrival.pinId,
		)?.direction;
		return reroute.pins
			.filter((pin) => pin.direction !== direction)
			.map((pin) => ({ nodeId: reroute.id, pinId: pin.id }))
			.filter((endpoint) => this.wires.has(endpointKey(endpoint)));
	}
}

/** One trace's walk: each wire crossed once, each pin queued once. */
class TraceWalk {
	readonly reroutes = new Set<string>();
	private readonly edges = new Set<string>();
	private readonly ends = new Map<string, SceneEndpoint>();
	private readonly seen = new Set<string>();
	private readonly queue: SceneEndpoint[] = [];
	constructor(starts: readonly SceneEndpoint[]) {
		for (const start of starts) this.visit(start);
	}

	next() {
		return this.queue.pop();
	}

	/** `onward` lists where a reroute at `far` continues; undefined when `far` is no reroute. */
	cross(
		edge: SceneEdge,
		far: SceneEndpoint,
		onward: () => SceneEndpoint[] | undefined,
	) {
		if (this.edges.has(edge.id)) return;
		this.edges.add(edge.id);
		this.reach(far, onward);
	}

	reach(far: SceneEndpoint, onward: () => SceneEndpoint[] | undefined) {
		const beyond = onward();
		if (beyond) this.reroutes.add(far.nodeId);
		if (beyond?.length) for (const endpoint of beyond) this.visit(endpoint);
		else this.ends.set(endpointKey(far), far);
	}

	result(start: Pick<WireTrace, "origin" | "segment">): WireTrace | undefined {
		if (!this.edges.size) return undefined;
		return {
			...start,
			edges: [...this.edges],
			reroutes: [...this.reroutes],
			ends: [...this.ends.values()],
		};
	}

	private visit(endpoint: SceneEndpoint) {
		const key = endpointKey(endpoint);
		if (this.seen.has(key)) return;
		this.seen.add(key);
		this.queue.push(endpoint);
	}
}

const edgeSets = new WeakMap<WireTrace, ReadonlySet<string>>();
function tracedEdges(trace: WireTrace): ReadonlySet<string> {
	let edges = edgeSets.get(trace);
	if (!edges) {
		edges = new Set(trace.edges);
		edgeSets.set(trace, edges);
	}
	return edges;
}

export function wireEmphasis(
	trace: BoardTrace,
	edgeId: string,
): WireEmphasis | undefined {
	if (trace.focus?.segment === edgeId) return "selected";
	if (trace.focus && tracedEdges(trace.focus).has(edgeId)) return "focus";
	if (trace.hover && tracedEdges(trace.hover).has(edgeId)) return "hover";
	return trace.focus ? "dim" : undefined;
}
