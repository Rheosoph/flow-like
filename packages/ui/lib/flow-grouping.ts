import { measureLayerBox, measureNodeBox } from "./flow-layout/measure";
import type { IBoard, ILayer } from "./schema/flow/board";
import { ILayerType } from "./schema/flow/board";
import type { INode, IPin } from "./schema/flow/node";
import { IVariableType } from "./schema/flow/node";

export interface GroupBoundaryPort {
	id: string;
	direction: "input" | "output";
	label: string;
	dataType: string;
	/** The visible endpoints outside the proposed group. */
	connections: Array<{ nodeId: string; pinId: string }>;
}

export interface GroupSuggestion {
	id: string;
	label: string;
	memberIds: string[];
	/** Existing layer chips count once; reroutes do not add complexity. */
	nodeCount: number;
	internalEdgeCount: number;
	inputCount: number;
	outputCount: number;
	bounds: { x: number; y: number; width: number; height: number };
	/** Mean member position, matching the layer command's position on redo. */
	anchor: { x: number; y: number };
	score: number;
	boundaryPorts: GroupBoundaryPort[];
}

export interface GroupSuggestionInput {
	board: IBoard;
	currentLayer?: string;
	only?: ReadonlySet<string>;
	/** Member lists the user skipped; a candidate that is mostly one of them is not offered. */
	skip?: ReadonlyArray<readonly string[]>;
	/** Flow-space region searched first (the visible viewport), since the budget may not reach the whole board. */
	focus?: GroupSuggestion["bounds"];
	nodeSizes?: ReadonlyMap<string, readonly [number, number]>;
}

interface Vertex {
	id: string;
	label: string;
	reroute: boolean;
	protected: boolean;
	bounds: GroupSuggestion["bounds"];
}

interface Endpoint {
	nodeId: string;
	pin: IPin;
}

interface Edge {
	from: Endpoint;
	to: Endpoint;
}

interface LogicalEdge {
	from: string;
	to: string;
	key: string;
}

interface Graph {
	input: GroupSuggestionInput;
	vertices: Map<string, Vertex>;
	edges: Edge[];
	logicalEdges: LogicalEdge[];
	neighbors: Map<string, Set<string>>;
	logicalNeighbors: Map<string, Set<string>>;
	components: Map<string, Set<string>>;
	cycles: Map<string, Set<string>>;
	fnLinks: Array<{
		from: string | undefined;
		to: string | undefined;
		key: string;
	}>;
	rerouteGroups: Array<{ members: string[]; neighbors: Set<string> }>;
	/** Positions in the lists above, by node, so a candidate only visits its own wiring. */
	edgesByNode: Map<string, number[]>;
	logicalEdgesByNode: Map<string, number[]>;
	fnLinksByNode: Map<string, number[]>;
	rerouteGroupsByNeighbor: Map<string, number[]>;
}

const MAX_GROUP_NODES = 24;
const MAX_CANDIDATES = 1200;
/** Jaccard similarity from which a candidate counts as a variant of a skipped group. */
const SKIPPED_OVERLAP = 0.5;

function scope(value: string | null | undefined): string | undefined {
	return value || undefined;
}

function sorted(values: Iterable<string>): string[] {
	return [...values].sort((a, b) => a.localeCompare(b));
}

/** Identifies a suggestion by who is in it, so it survives rewiring inside the group. */
export function groupMembershipKey(memberIds: Iterable<string>): string {
	return sorted(new Set(memberIds)).join("\0");
}

function indexByNode<T>(
	items: readonly T[],
	nodesOf: (item: T) => Iterable<string | undefined>,
): Map<string, number[]> {
	const index = new Map<string, number[]>();
	items.forEach((item, position) => {
		for (const id of new Set(nodesOf(item))) {
			if (id === undefined) continue;
			const positions = index.get(id);
			if (positions) positions.push(position);
			else index.set(id, [position]);
		}
	});
	return index;
}

function mostlyOverlaps(
	members: ReadonlySet<string>,
	skipped: ReadonlyArray<readonly string[]>,
): boolean {
	return skipped.some((other) => {
		let shared = 0;
		for (const id of other) if (members.has(id)) shared++;
		return (
			shared > 0 &&
			shared / (members.size + other.length - shared) >= SKIPPED_OVERLAP
		);
	});
}

function distanceBetween(
	a: GroupSuggestion["bounds"],
	b: GroupSuggestion["bounds"],
): number {
	return Math.hypot(
		Math.max(0, b.x - (a.x + a.width), a.x - (b.x + b.width)),
		Math.max(0, b.y - (a.y + a.height), a.y - (b.y + b.height)),
	);
}

/** Ascending positions, so callers see entries in the list's own order. */
function positionsOf(
	index: Map<string, number[]>,
	ids: Iterable<string>,
): number[] {
	const positions = new Set<number>();
	for (const id of ids) {
		for (const position of index.get(id) ?? []) positions.add(position);
	}
	return [...positions].sort((a, b) => a - b);
}

function addNeighbor(
	map: Map<string, Set<string>>,
	from: string,
	to: string,
): void {
	const neighbors = map.get(from) ?? new Set<string>();
	neighbors.add(to);
	map.set(from, neighbors);
}

function components(
	ids: Iterable<string>,
	neighbors: Map<string, Set<string>>,
): Map<string, Set<string>> {
	const result = new Map<string, Set<string>>();
	for (const id of sorted(ids)) {
		if (result.has(id)) continue;
		const members = new Set<string>([id]);
		const queue = [id];
		for (let index = 0; index < queue.length; index++) {
			for (const next of neighbors.get(queue[index]) ?? []) {
				if (members.has(next)) continue;
				members.add(next);
				queue.push(next);
			}
		}
		for (const member of members) result.set(member, members);
	}
	return result;
}

/** Iterative Kosaraju keeps a large cycle from overflowing the JavaScript stack. */
function stronglyConnected(
	ids: Iterable<string>,
	edges: LogicalEdge[],
): Map<string, Set<string>> {
	const forward = new Map<string, Set<string>>();
	const backward = new Map<string, Set<string>>();
	for (const edge of edges) {
		addNeighbor(forward, edge.from, edge.to);
		addNeighbor(backward, edge.to, edge.from);
	}
	const visited = new Set<string>();
	const order: string[] = [];
	for (const id of sorted(ids)) {
		if (visited.has(id)) continue;
		const stack: Array<{ id: string; expanded: boolean }> = [
			{ id, expanded: false },
		];
		while (stack.length) {
			const frame = stack.pop();
			if (!frame) break;
			if (frame.expanded) {
				order.push(frame.id);
				continue;
			}
			if (visited.has(frame.id)) continue;
			visited.add(frame.id);
			stack.push({ id: frame.id, expanded: true });
			for (const next of sorted(forward.get(frame.id) ?? [])) {
				if (!visited.has(next)) stack.push({ id: next, expanded: false });
			}
		}
	}
	const result = new Map<string, Set<string>>();
	for (const id of order.reverse()) {
		if (result.has(id)) continue;
		const members = new Set<string>([id]);
		const stack = [id];
		while (stack.length) {
			const next = stack.pop();
			if (!next) break;
			for (const previous of backward.get(next) ?? []) {
				if (result.has(previous) || members.has(previous)) continue;
				members.add(previous);
				stack.push(previous);
			}
		}
		for (const member of members) result.set(member, members);
	}
	return result;
}

function buildGraph(input: GroupSuggestionInput): Graph {
	const { board, nodeSizes } = input;
	const currentLayer = scope(input.currentLayer);
	const allNodes = new Map<string, INode>();
	const nodeLayers = new Map<string, string | undefined>();
	for (const layer of Object.values(board.layers ?? {})) {
		for (const node of Object.values(layer.nodes ?? {})) {
			allNodes.set(node.id, node);
			nodeLayers.set(node.id, scope(node.layer) ?? layer.id);
		}
	}
	for (const node of Object.values(board.nodes ?? {})) {
		allNodes.set(node.id, node);
		nodeLayers.set(node.id, scope(node.layer));
	}
	const vertices = new Map<string, Vertex>();
	const visibleLayers = new Map<string, ILayer>();
	for (const layer of Object.values(board.layers ?? {})) {
		if (scope(layer.parent_id) !== currentLayer || layer.id === currentLayer)
			continue;
		if (layer.type !== ILayerType.Collapsed && layer.type !== ILayerType.Macro)
			continue;
		visibleLayers.set(layer.id, layer);
		const measured = nodeSizes?.get(layer.id);
		const size = measureLayerBox(layer);
		vertices.set(layer.id, {
			id: layer.id,
			label: layer.name,
			reroute: false,
			protected: false,
			bounds: {
				x: layer.coordinates[0] ?? 0,
				y: layer.coordinates[1] ?? 0,
				width: measured?.[0] ?? size.width,
				height: measured?.[1] ?? size.height,
			},
		});
	}
	for (const node of allNodes.values()) {
		if (nodeLayers.get(node.id) !== currentLayer) continue;
		const measured = nodeSizes?.get(node.id);
		const size = measureNodeBox(node);
		vertices.set(node.id, {
			id: node.id,
			label: node.friendly_name || node.name,
			reroute: node.name === "reroute",
			protected: node.start === true || node.event_callback === true,
			bounds: {
				x: node.coordinates?.[0] ?? 0,
				y: node.coordinates?.[1] ?? 0,
				width: measured?.[0] ?? size.width,
				height: measured?.[1] ?? size.height,
			},
		});
	}
	const visibleAncestor = (layerId: string | undefined): string | undefined => {
		const seen = new Set<string>();
		let id = layerId;
		while (id && !seen.has(id)) {
			if (visibleLayers.has(id)) return id;
			seen.add(id);
			id = scope(board.layers?.[id]?.parent_id);
		}
		return undefined;
	};
	const nodeOwner = (id: string): string | undefined =>
		vertices.has(id)
			? id
			: (visibleAncestor(nodeLayers.get(id)) ??
				visibleAncestor(board.layers?.[id]?.id));
	const pins = new Map<string, Endpoint>();
	for (const node of allNodes.values()) {
		const owner = nodeOwner(node.id) ?? node.id;
		for (const pin of Object.values(node.pins ?? {}))
			pins.set(pin.id, { nodeId: owner, pin });
	}
	for (const layer of Object.values(board.layers ?? {})) {
		const owner = visibleAncestor(layer.id) ?? layer.id;
		for (const pin of Object.values(layer.pins ?? {})) {
			const boundary =
				layer.id === currentLayer
					? `${layer.id}${pin.pin_type === "Input" ? "-input" : "-return"}`
					: owner;
			pins.set(pin.id, { nodeId: boundary, pin });
		}
	}
	const edges: Edge[] = [];
	const seenEdges = new Set<string>();
	for (const from of pins.values()) {
		for (const target of from.pin.connected_to ?? []) {
			const to = pins.get(target);
			if (!to || from.nodeId === to.nodeId) continue;
			const key = `${from.pin.id}->${to.pin.id}`;
			if (seenEdges.has(key)) continue;
			seenEdges.add(key);
			edges.push({ from, to });
		}
	}
	edges.sort(
		(a, b) =>
			a.from.pin.id.localeCompare(b.from.pin.id) ||
			a.to.pin.id.localeCompare(b.to.pin.id),
	);
	const neighbors = new Map<string, Set<string>>();
	const outgoing = new Map<string, Edge[]>();
	for (const edge of edges) {
		if (vertices.has(edge.from.nodeId) && vertices.has(edge.to.nodeId)) {
			addNeighbor(neighbors, edge.from.nodeId, edge.to.nodeId);
			addNeighbor(neighbors, edge.to.nodeId, edge.from.nodeId);
		}
		const next = outgoing.get(edge.from.nodeId) ?? [];
		next.push(edge);
		outgoing.set(edge.from.nodeId, next);
	}
	const logicalEdges: LogicalEdge[] = [];
	const logicalKeys = new Set<string>();
	for (const edge of edges) {
		const from = vertices.get(edge.from.nodeId);
		if (!from || from.reroute) continue;
		const pending = [edge];
		const visited = new Set<string>();
		while (pending.length) {
			const step = pending.pop();
			if (!step) break;
			const to = vertices.get(step.to.nodeId);
			if (!to) continue;
			if (to.reroute) {
				if (visited.has(to.id)) continue;
				visited.add(to.id);
				pending.push(...(outgoing.get(to.id) ?? []));
				continue;
			}
			const key = `${edge.from.pin.id}->${step.to.pin.id}`;
			if (to.id === from.id || logicalKeys.has(key)) continue;
			logicalKeys.add(key);
			logicalEdges.push({ from: from.id, to: to.id, key });
		}
	}
	const logicalNeighbors = new Map<string, Set<string>>();
	for (const edge of logicalEdges) {
		addNeighbor(logicalNeighbors, edge.from, edge.to);
		addNeighbor(logicalNeighbors, edge.to, edge.from);
	}
	const fnLinks: Graph["fnLinks"] = [];
	for (const node of allNodes.values()) {
		for (const target of node.fn_refs?.fn_refs ?? []) {
			fnLinks.push({
				from: nodeOwner(node.id),
				to: nodeOwner(target),
				key: `${node.id}->${target}`,
			});
		}
	}
	fnLinks.sort((a, b) => a.key.localeCompare(b.key));
	const rerouteIds = sorted(
		[...vertices.values()]
			.filter((node) => node.reroute)
			.map((node) => node.id),
	);
	const rerouteNeighbors = new Map<string, Set<string>>();
	for (const id of rerouteIds) {
		for (const next of neighbors.get(id) ?? []) {
			if (vertices.get(next)?.reroute) addNeighbor(rerouteNeighbors, id, next);
		}
	}
	const rerouteGroups: Graph["rerouteGroups"] = [];
	for (const members of new Set(
		components(rerouteIds, rerouteNeighbors).values(),
	)) {
		const external = new Set<string>();
		for (const id of members) {
			for (const next of neighbors.get(id) ?? []) {
				if (!members.has(next)) external.add(next);
			}
		}
		rerouteGroups.push({ members: sorted(members), neighbors: external });
	}
	return {
		input,
		vertices,
		edges,
		logicalEdges,
		edgesByNode: indexByNode(edges, (edge) => [
			edge.from.nodeId,
			edge.to.nodeId,
		]),
		logicalEdgesByNode: indexByNode(logicalEdges, (edge) => [
			edge.from,
			edge.to,
		]),
		fnLinksByNode: indexByNode(fnLinks, (link) => [link.from, link.to]),
		rerouteGroupsByNeighbor: indexByNode(
			rerouteGroups,
			(group) => group.neighbors,
		),
		neighbors,
		logicalNeighbors,
		components: components(vertices.keys(), neighbors),
		cycles: stronglyConnected(
			vertices.keys(),
			edges
				.filter(
					(edge) =>
						vertices.has(edge.from.nodeId) && vertices.has(edge.to.nodeId),
				)
				.map((edge) => ({
					from: edge.from.nodeId,
					to: edge.to.nodeId,
					key: "",
				})),
		),
		fnLinks,
		rerouteGroups,
	};
}

function topologyId(parts: string[]): string {
	const value = JSON.stringify(parts);
	let a = 2166136261;
	let b = 2246822519;
	for (let index = 0; index < value.length; index++) {
		a = Math.imul(a ^ value.charCodeAt(index), 16777619);
		b = Math.imul(b ^ value.charCodeAt(index), 3266489917);
	}
	return `group-${(a >>> 0).toString(16).padStart(8, "0")}${(b >>> 0).toString(16).padStart(8, "0")}`;
}

function evaluate(
	graph: Graph,
	ids: Iterable<string>,
): GroupSuggestion | undefined {
	const memberIds = sorted(new Set(ids));
	const members = new Set(memberIds);
	const substantive = memberIds.filter(
		(id) => !graph.vertices.get(id)?.reroute,
	);
	if (substantive.length < 2) return undefined;
	for (const id of memberIds) {
		const vertex = graph.vertices.get(id);
		if (
			!vertex ||
			vertex.protected ||
			(graph.input.only && !graph.input.only.has(id))
		)
			return undefined;
		for (const sibling of graph.cycles.get(id) ?? []) {
			if (!members.has(sibling)) return undefined;
		}
	}
	const fnLinks = positionsOf(graph.fnLinksByNode, memberIds).map(
		(position) => graph.fnLinks[position],
	);
	for (const link of fnLinks) {
		if (
			(link.from !== undefined && members.has(link.from)) !==
			(link.to !== undefined && members.has(link.to))
		)
			return undefined;
	}
	const reached = new Set<string>([memberIds[0]]);
	const pending = [memberIds[0]];
	for (let index = 0; index < pending.length; index++) {
		for (const next of graph.neighbors.get(pending[index]) ?? []) {
			if (members.has(next) && !reached.has(next)) {
				reached.add(next);
				pending.push(next);
			}
		}
	}
	if (reached.size !== members.size) return undefined;
	const ports = new Map<string, GroupBoundaryPort>();
	const topology = [
		`layer:${scope(graph.input.currentLayer) ?? ""}`,
		...memberIds,
	];
	for (const position of positionsOf(graph.edgesByNode, memberIds)) {
		const edge = graph.edges[position];
		const fromInside = members.has(edge.from.nodeId);
		const toInside = members.has(edge.to.nodeId);
		topology.push(
			`${edge.from.pin.id}:${edge.from.pin.data_type}->${edge.to.pin.id}:${edge.to.pin.data_type}`,
		);
		if (fromInside === toInside) continue;
		const direction = fromInside ? "output" : "input";
		// Match BridgeLayersCleanup: data enters by producer; exec enters by consumer.
		const keyPin =
			fromInside || edge.from.pin.data_type !== IVariableType.Execution
				? edge.from.pin
				: edge.to.pin;
		const key = `${direction}:${keyPin.id}`;
		const port = ports.get(key) ?? {
			id: key,
			direction,
			label: keyPin.friendly_name || keyPin.name,
			dataType: keyPin.data_type,
			connections: [],
		};
		const outside = fromInside ? edge.to : edge.from;
		if (
			!port.connections.some(
				(endpoint) =>
					endpoint.nodeId === outside.nodeId &&
					endpoint.pinId === outside.pin.id,
			)
		) {
			port.connections.push({ nodeId: outside.nodeId, pinId: outside.pin.id });
		}
		ports.set(key, port);
	}
	for (const link of fnLinks) topology.push(`fn:${link.key}`);
	const boundaryPorts = [...ports.values()].sort((a, b) =>
		a.id.localeCompare(b.id),
	);
	for (const port of boundaryPorts)
		port.connections.sort(
			(a, b) =>
				a.nodeId.localeCompare(b.nodeId) || a.pinId.localeCompare(b.pinId),
		);
	const internalEdges = positionsOf(graph.logicalEdgesByNode, memberIds)
		.map((position) => graph.logicalEdges[position])
		.filter((edge) => members.has(edge.from) && members.has(edge.to));
	const boundaryCount = boundaryPorts.length;
	const topologyScore =
		internalEdges.length +
		substantive.length -
		1 -
		boundaryCount * 1.5 +
		internalEdges.length / (boundaryCount + 1);
	let left = Number.POSITIVE_INFINITY;
	let top = Number.POSITIVE_INFINITY;
	let right = Number.NEGATIVE_INFINITY;
	let bottom = Number.NEGATIVE_INFINITY;
	let sumX = 0;
	let sumY = 0;
	for (const id of memberIds) {
		const bounds = graph.vertices.get(id)?.bounds;
		if (!bounds) continue;
		left = Math.min(left, bounds.x);
		top = Math.min(top, bounds.y);
		right = Math.max(right, bounds.x + bounds.width);
		bottom = Math.max(bottom, bounds.y + bounds.height);
		sumX += bounds.x;
		sumY += bounds.y;
	}
	const widths = substantive
		.map((id) => graph.vertices.get(id)?.bounds.width ?? 150)
		.sort((a, b) => a - b);
	const typicalWidth = Math.max(16, widths[Math.floor(widths.length / 2)]);
	let occupiedArea = 0;
	for (const id of substantive) {
		const box = graph.vertices.get(id)?.bounds;
		if (box) occupiedArea += box.width * box.height;
	}
	let longestGap = 0;
	for (const edge of internalEdges) {
		const from = graph.vertices.get(edge.from)?.bounds;
		const to = graph.vertices.get(edge.to)?.bounds;
		if (!from || !to) continue;
		longestGap = Math.max(
			longestGap,
			Math.hypot(
				Math.max(0, from.x - to.x - to.width, to.x - from.x - from.width),
				Math.max(0, from.y - to.y - to.height, to.y - from.y - from.height),
			),
		);
	}
	// Ordinary column gaps barely affect ranking; a long link between distant clusters does.
	const emptyAreaPenalty =
		Math.log2(
			Math.max(
				1,
				((right - left) * (bottom - top)) / Math.max(1, occupiedArea),
			),
		) * 0.5;
	const longLinkPenalty =
		Math.log2(1 + Math.max(0, longestGap - typicalWidth * 3) / typicalWidth) *
		3;
	const score = topologyScore - emptyAreaPenalty - longLinkPenalty;
	const representative = substantive.sort((a, b) => {
		const degree = (id: string) =>
			internalEdges.filter((edge) => edge.from === id || edge.to === id).length;
		return degree(b) - degree(a) || a.localeCompare(b);
	})[0];
	return {
		id: topologyId(topology),
		label: `${graph.vertices.get(representative)?.label || "Connected nodes"} group`,
		memberIds,
		nodeCount: substantive.length,
		internalEdgeCount: internalEdges.length,
		inputCount: boundaryPorts.filter((port) => port.direction === "input")
			.length,
		outputCount: boundaryPorts.filter((port) => port.direction === "output")
			.length,
		bounds: { x: left, y: top, width: right - left, height: bottom - top },
		anchor: { x: sumX / memberIds.length, y: sumY / memberIds.length },
		score,
		boundaryPorts,
	};
}

/** Revalidate exact memberships and topology without applying suggestion thresholds. */
export function evaluateGroupCandidates(
	input: GroupSuggestionInput,
	memberLists: ReadonlyArray<Iterable<string>>,
): Array<GroupSuggestion | undefined> {
	if (memberLists.length === 0) return [];
	const graph = buildGraph(input);
	return memberLists.map((memberIds) => evaluate(graph, memberIds));
}

export function evaluateGroupCandidate(
	input: GroupSuggestionInput,
	memberIds: Iterable<string>,
): GroupSuggestion | undefined {
	return evaluateGroupCandidates(input, [memberIds])[0];
}

function closeCandidate(
	graph: Graph,
	seed: Iterable<string>,
): Set<string> | undefined {
	const members = new Set(seed);
	let changed = true;
	while (changed) {
		changed = false;
		const add = (id: string | undefined): boolean => {
			if (
				!id ||
				!graph.vertices.has(id) ||
				graph.vertices.get(id)?.protected ||
				(graph.input.only && !graph.input.only.has(id))
			)
				return false;
			if (!members.has(id)) {
				members.add(id);
				changed = true;
			}
			return true;
		};
		for (const id of members) {
			if (!add(id)) return undefined;
			for (const sibling of graph.cycles.get(id) ?? []) {
				if (!add(sibling)) return undefined;
			}
		}
		for (const position of positionsOf(graph.fnLinksByNode, [...members])) {
			const link = graph.fnLinks[position];
			if (link.from && members.has(link.from) && !add(link.to))
				return undefined;
			if (link.to && members.has(link.to) && !add(link.from)) return undefined;
		}
		for (const position of positionsOf(graph.rerouteGroupsByNeighbor, [
			...members,
		])) {
			const group = graph.rerouteGroups[position];
			if (
				group.neighbors.size < 2 ||
				![...group.neighbors].every((id) => members.has(id))
			)
				continue;
			for (const id of group.members) {
				if (!add(id)) return undefined;
			}
		}
		if (
			[...members].filter((id) => !graph.vertices.get(id)?.reroute).length >
			MAX_GROUP_NODES
		)
			return undefined;
	}
	return members;
}

function isHotspot(graph: Graph, suggestion: GroupSuggestion): boolean {
	const { nodeCount, internalEdgeCount, inputCount, outputCount, memberIds } =
		suggestion;
	if (nodeCount < 3 || internalEdgeCount < nodeCount - 1) return false;
	const members = new Set(memberIds);
	const component = graph.components.get(memberIds[0]);
	if (
		component &&
		component.size <= members.size &&
		[...component].every((id) => members.has(id))
	)
		return false;
	const boundary = inputCount + outputCount;
	if (
		!inputCount ||
		!outputCount ||
		boundary > Math.min(6, Math.max(2, Math.floor(internalEdgeCount / 2) + 1))
	)
		return false;
	const branches = memberIds.some(
		(id) =>
			[...(graph.logicalNeighbors.get(id) ?? [])].filter((next) =>
				members.has(next),
			).length >= 3,
	);
	if (!branches && internalEdgeCount < nodeCount) return false;
	return suggestion.score > 2;
}

/** Find a few connected regions whose internal wiring is substantially denser than their boundary. */
export function findGroupSuggestions(
	input: GroupSuggestionInput,
): GroupSuggestion[] {
	const graph = buildGraph(input);
	const eligible = sorted(
		[...graph.vertices.values()]
			.filter(
				(node) =>
					!node.protected &&
					!node.reroute &&
					(!input.only || input.only.has(node.id)),
			)
			.map((node) => node.id),
	);
	const allowed = new Set(eligible);
	const candidates = new Map<string, GroupSuggestion>();
	const seen = new Set<string>();
	let attempted = 0;
	const consider = (members: Set<string> | undefined) => {
		if (!members || attempted >= MAX_CANDIDATES) return;
		const key = groupMembershipKey(members);
		if (seen.has(key)) return;
		seen.add(key);
		if (input.skip && mostlyOverlaps(members, input.skip)) return;
		attempted++;
		const candidate = evaluate(graph, members);
		if (candidate && isHotspot(graph, candidate))
			candidates.set(candidate.id, candidate);
	};
	const focus = input.focus;
	const seeds = eligible
		.map((id) => {
			const bounds = graph.vertices.get(id)?.bounds;
			return {
				id,
				distance: focus && bounds ? distanceBetween(bounds, focus) : 0,
				degree: graph.logicalNeighbors.get(id)?.size ?? 0,
			};
		})
		.sort(
			(a, b) =>
				a.distance - b.distance ||
				b.degree - a.degree ||
				a.id.localeCompare(b.id),
		)
		.map(({ id }) => id);
	const grown = new Set<string>();
	const nearby = new Set<string>();
	const grow = (seed: string) => {
		grown.add(seed);
		const neighbors = [...(graph.logicalNeighbors.get(seed) ?? [])].filter(
			(id) => allowed.has(id),
		);
		for (const id of neighbors) nearby.add(id);
		consider(closeCandidate(graph, [seed, ...neighbors]));
		let members = closeCandidate(graph, [seed]);
		for (
			let step = 0;
			members && step < MAX_GROUP_NODES && attempted < MAX_CANDIDATES;
			step++
		) {
			consider(members);
			const fringe = new Set<string>();
			for (const id of members) {
				for (const next of graph.logicalNeighbors.get(id) ?? []) {
					if (allowed.has(next) && !members.has(next)) fringe.add(next);
				}
			}
			const active = members;
			const affinity = (id: string) => {
				const neighbors = graph.logicalNeighbors.get(id) ?? new Set<string>();
				const inside = [...neighbors].filter((next) => active.has(next)).length;
				return inside * 3 - (neighbors.size - inside);
			};
			const next = sorted(fringe).sort(
				(a, b) => affinity(b) - affinity(a) || a.localeCompare(b),
			)[0];
			if (!next) break;
			members = closeCandidate(graph, [...members, next]);
		}
	};
	// A seed beside one already grown mostly rediscovers the same regions, so it
	// waits until every other part of the board has had a turn at the budget.
	for (const seed of seeds) {
		if (attempted >= MAX_CANDIDATES) break;
		if (!nearby.has(seed)) grow(seed);
	}
	for (const seed of seeds) {
		if (attempted >= MAX_CANDIDATES) break;
		if (!grown.has(seed)) grow(seed);
	}
	const onScreen = (candidate: GroupSuggestion) =>
		focus && distanceBetween(candidate.bounds, focus) === 0 ? 1 : 0;
	const ranked = [...candidates.values()].sort(
		(a, b) =>
			onScreen(b) - onScreen(a) ||
			b.score - a.score ||
			a.memberIds.length - b.memberIds.length ||
			a.id.localeCompare(b.id),
	);
	const chosen: GroupSuggestion[] = [];
	const used = new Set<string>();
	for (const candidate of ranked) {
		if (candidate.memberIds.some((id) => used.has(id))) continue;
		if (
			chosen.some(
				({ bounds }) =>
					candidate.bounds.x < bounds.x + bounds.width + 16 &&
					candidate.bounds.x + candidate.bounds.width + 16 > bounds.x &&
					candidate.bounds.y < bounds.y + bounds.height + 16 &&
					candidate.bounds.y + candidate.bounds.height + 16 > bounds.y,
			)
		)
			continue;
		chosen.push(candidate);
		for (const id of candidate.memberIds) used.add(id);
		if (chosen.length === 3) break;
	}
	return chosen;
}
