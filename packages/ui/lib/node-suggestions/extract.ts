import {
	type IBoard,
	type ILayer,
	ILayerType,
	type INode,
	type IPin,
	IPinType,
	IVariableType,
} from "../schema/flow/board";
import { contextOf, expandTransition } from "./facts";
import {
	type BoardFacts,
	type EdgeKind,
	type FactOutcome,
	type FactTransition,
	MAX_PRED,
	MAX_UPS,
	type SuggestionContext,
} from "./types";

const REROUTE = "reroute";
const FOLDED_LAYERS = new Set<string>([ILayerType.Collapsed, ILayerType.Macro]);
const EXEC_PIN_PRIORITY = new Map([
	["exec_out", 0],
	["then", 0],
	["exec_success", 1],
]);
const SCHEMA_CACHE_LIMIT = 4096;

type PinOwner =
	| { kind: "node"; node: INode; pin: IPin }
	| { kind: "layer"; layer: ILayer; pin: IPin };

interface GraphEdge {
	src: number;
	srcPin: IPin;
	dst: number;
	dstPin: IPin;
	exec: boolean;
}

interface ExecTarget {
	priority: number;
	y: number;
	x: number;
	id: string;
	node: number;
}

interface NodeContext {
	pred: number[];
	ups: number[];
}

type ContextRecord = Omit<FactTransition, "dst" | "dstPin">;

interface BoardGraph {
	refs: IBoard["refs"] | undefined;
	nodes: INode[];
	indexOf: Map<string, number>;
	edges: GraphEdge[];
	connected: Set<string>;
	order: number[];
	providers: number[][];
	execPred: Int32Array;
	typeOf: Int32Array;
	bagAt: Int32Array;
	types: string[];
	repeated: number[];
	contexts: (NodeContext | undefined)[];
	facts?: BoardFacts;
}

const graphs = new WeakMap<IBoard, BoardGraph>();
const schemaTitles = new Map<string, string>();

export function schemaTitle(
	refs: IBoard["refs"] | null | undefined,
	schema: string | null | undefined,
): string {
	if (!schema) return "";
	const resolved: unknown =
		refs && Object.hasOwn(refs, schema) ? refs[schema] : schema;
	if (typeof resolved !== "string" || resolved === "") return "";
	const cached = schemaTitles.get(resolved);
	if (cached !== undefined) return cached;
	let title = "";
	try {
		const parsed: unknown = JSON.parse(resolved);
		if (
			parsed &&
			typeof parsed === "object" &&
			"title" in parsed &&
			typeof parsed.title === "string"
		) {
			title = parsed.title;
		}
	} catch {
		title = "";
	}
	if (schemaTitles.size >= SCHEMA_CACHE_LIMIT) schemaTitles.clear();
	schemaTitles.set(resolved, title);
	return title;
}

export function extractBoardFacts(board: IBoard): BoardFacts {
	const graph = graphOf(board);
	graph.facts ??= buildFacts(graph);
	return graph.facts;
}

/**
 * Context of a live output pin, built like a training fact except for the bag: every node on the
 * board already exists before the one being suggested, so it spans the whole board.
 */
export function buildSuggestionContext(
	board: IBoard,
	nodeId: string,
	pinId: string,
): SuggestionContext | undefined {
	const graph = graphOf(board);
	const index = graph.indexOf.get(nodeId);
	if (index === undefined) return undefined;
	const pin = pinOf(graph.nodes[index], pinId);
	if (pin?.pin_type !== IPinType.Output) return undefined;
	const record = contextRecord(graph, index, pin);
	const facts = { types: graph.types, transitions: [], outcomes: [] };
	return contextOf(
		expandTransition(facts, {
			...record,
			bag: graph.types.length,
			dst: record.src,
			dstPin: "",
		}),
	);
}

export interface OutputPinTargets {
	node: INode;
	pin: IPin;
	kind: EdgeKind;
	/** Node types the pin's wires end on, reroutes and folded layers followed; empty when unconnected. */
	targets: string[];
}

/** Every output pin of the board's non-reroute nodes in `outcomes` order, with the node types it feeds. */
export function outputPinTargets(board: IBoard): OutputPinTargets[] {
	const graph = graphOf(board);
	const targets = new Map<string, string[]>();
	for (const edge of graph.edges) {
		const list = targets.get(edge.srcPin.id);
		const type = graph.nodes[edge.dst].name;
		if (list) list.push(type);
		else targets.set(edge.srcPin.id, [type]);
	}
	const pins: OutputPinTargets[] = [];
	for (const node of graph.order) {
		const owner = graph.nodes[node];
		for (const pin of Object.values(owner.pins ?? {})) {
			if (pin.pin_type !== IPinType.Output) continue;
			pins.push({
				node: owner,
				pin,
				kind: kindOf(pin),
				targets: targets.get(pin.id) ?? [],
			});
		}
	}
	return pins;
}

/** Unconnected output pins of a node, the most likely anchor first. */
export function anchorPins(
	board: IBoard,
	nodeId: string,
): { pinId: string; kind: EdgeKind }[] {
	const node = findNode(board, nodeId);
	if (!node || node.name === REROUTE) return [];
	return Object.values(node.pins ?? {})
		.filter(
			(pin) =>
				pin.pin_type === IPinType.Output &&
				(pin.connected_to?.length ?? 0) === 0,
		)
		.sort((a, b) => anchorRank(a) - anchorRank(b) || a.index - b.index)
		.map((pin) => ({ pinId: pin.id, kind: kindOf(pin) }));
}

function kindOf(pin: IPin): EdgeKind {
	return pin.data_type === IVariableType.Execution ? "exec" : "data";
}

function anchorRank(pin: IPin): number {
	if (pin.data_type !== IVariableType.Execution) return 4;
	const priority = EXEC_PIN_PRIORITY.get(pin.name);
	if (priority !== undefined) return priority;
	return pin.name.includes("error") ? 3 : 2;
}

function findNode(board: IBoard, nodeId: string): INode | undefined {
	const node = board.nodes?.[nodeId];
	if (node) return node;
	for (const layer of Object.values(board.layers ?? {})) {
		const nested = layer?.nodes?.[nodeId];
		if (nested) return nested;
	}
	return undefined;
}

function pinOf(node: INode, pinId: string): IPin | undefined {
	const pins = node.pins ?? {};
	return pins[pinId] ?? Object.values(pins).find((pin) => pin.id === pinId);
}

function graphOf(board: IBoard): BoardGraph {
	let graph = graphs.get(board);
	if (!graph) {
		graph = analyze(board);
		graphs.set(board, graph);
	}
	return graph;
}

function collectNodes(board: IBoard): Map<string, INode> {
	const all = new Map<string, INode>();
	for (const node of Object.values(board.nodes ?? {})) {
		if (node?.id) all.set(node.id, node);
	}
	for (const layer of Object.values(board.layers ?? {})) {
		for (const node of Object.values(layer?.nodes ?? {})) {
			if (node?.id && !all.has(node.id)) all.set(node.id, node);
		}
	}
	return all;
}

function collectOwners(
	board: IBoard,
	all: Map<string, INode>,
): Map<string, PinOwner> {
	const owners = new Map<string, PinOwner>();
	for (const node of all.values()) {
		for (const pin of Object.values(node.pins ?? {})) {
			if (pin?.id) owners.set(pin.id, { kind: "node", node, pin });
		}
	}
	for (const layer of Object.values(board.layers ?? {})) {
		if (!layer) continue;
		for (const pin of Object.values(layer.pins ?? {})) {
			if (pin?.id) owners.set(pin.id, { kind: "layer", layer, pin });
		}
	}
	return owners;
}

/** Follows reroutes and Collapsed/Macro boundary pins; Function/Module boundaries end the wire. */
function resolveTargets(
	owners: Map<string, PinOwner>,
	start: readonly string[],
): { node: INode; pin: IPin }[] {
	const found: { node: INode; pin: IPin }[] = [];
	const seen = new Set<string>();
	const stack = [...start];
	while (stack.length > 0) {
		const pinId = stack.pop() as string;
		if (seen.has(pinId)) continue;
		seen.add(pinId);
		const owner = owners.get(pinId);
		if (!owner) continue;
		if (owner.kind === "node") {
			if (owner.node.name !== REROUTE) {
				found.push({ node: owner.node, pin: owner.pin });
				continue;
			}
			for (const pin of Object.values(owner.node.pins ?? {})) {
				if (pin.pin_type === IPinType.Output) {
					stack.push(...(pin.connected_to ?? []));
				}
			}
			continue;
		}
		if (FOLDED_LAYERS.has(owner.layer.type)) {
			stack.push(...(owner.pin.connected_to ?? []));
		}
	}
	return found;
}

function coordinate(node: INode, axis: number): number {
	const value = node.coordinates?.[axis];
	return typeof value === "number" && Number.isFinite(value) ? value : 0;
}

function analyze(board: IBoard): BoardGraph {
	const all = collectNodes(board);
	const owners = collectOwners(board, all);
	const nodes: INode[] = [];
	const indexOf = new Map<string, number>();
	for (const node of all.values()) {
		if (node.name === REROUTE) continue;
		indexOf.set(node.id, nodes.length);
		nodes.push(node);
	}

	const edges: GraphEdge[] = [];
	const connected = new Set<string>();
	for (let src = 0; src < nodes.length; src++) {
		for (const srcPin of Object.values(nodes[src].pins ?? {})) {
			if (srcPin.pin_type !== IPinType.Output) continue;
			if (!srcPin.connected_to?.length) continue;
			const exec = srcPin.data_type === IVariableType.Execution;
			for (const target of resolveTargets(owners, srcPin.connected_to)) {
				const dst = indexOf.get(target.node.id);
				if (dst === undefined) continue;
				edges.push({ src, srcPin, dst, dstPin: target.pin, exec });
				connected.add(srcPin.id);
			}
		}
	}

	const size = nodes.length;
	const xs = new Float64Array(size);
	const ys = new Float64Array(size);
	for (let index = 0; index < size; index++) {
		xs[index] = coordinate(nodes[index], 0);
		ys[index] = coordinate(nodes[index], 1);
	}
	const byXY = (a: number, b: number) => xs[a] - xs[b] || ys[a] - ys[b];

	const providers: number[][] = Array.from({ length: size }, () => []);
	const execIn: number[][] = Array.from({ length: size }, () => []);
	const execOut: ExecTarget[][] = Array.from({ length: size }, () => []);
	for (const edge of edges) {
		if (!edge.exec) {
			providers[edge.dst].push(edge.src);
			continue;
		}
		execIn[edge.dst].push(edge.src);
		execOut[edge.src].push({
			priority: EXEC_PIN_PRIORITY.get(edge.srcPin.name) ?? 2,
			y: ys[edge.dst],
			x: xs[edge.dst],
			id: nodes[edge.dst].id,
			node: edge.dst,
		});
	}

	const order = constructionOrder(nodes, providers, execIn, execOut, byXY);
	const pos = new Int32Array(size);
	order.forEach((node, position) => {
		pos[node] = position;
	});

	const execPred = new Int32Array(size).fill(-1);
	for (let dst = 0; dst < size; dst++) {
		for (const src of execIn[dst]) {
			if (execPred[dst] < 0 || pos[src] < pos[execPred[dst]]) {
				execPred[dst] = src;
			}
		}
	}

	const typeIndex = new Map<string, number>();
	const types: string[] = [];
	const instances: number[] = [];
	const typeOf = new Int32Array(size);
	const bagAt = new Int32Array(size);
	for (const node of order) {
		const name = nodes[node].name;
		let type = typeIndex.get(name);
		if (type === undefined) {
			type = types.length;
			typeIndex.set(name, type);
			types.push(name);
			instances.push(0);
		}
		instances[type]++;
		typeOf[node] = type;
		bagAt[node] = types.length;
	}
	const repeated: number[] = [];
	instances.forEach((count, type) => {
		if (count > 1) repeated.push(type);
	});

	return {
		refs: board.refs,
		nodes,
		indexOf,
		edges,
		connected,
		order,
		providers,
		execPred,
		typeOf,
		bagAt,
		types,
		repeated,
		contexts: new Array(size),
	};
}

/**
 * Approximate order in which the user placed the nodes: exec chains from every start, each node's
 * data providers right before it, then whatever is left by position.
 */
function constructionOrder(
	nodes: INode[],
	providers: number[][],
	execIn: number[][],
	execOut: ExecTarget[][],
	byXY: (a: number, b: number) => number,
): number[] {
	const size = nodes.length;
	const visited = new Uint8Array(size);
	const order: number[] = [];
	const providersOf = (node: number) =>
		[...new Set(providers[node])].sort(byXY);
	const successorsOf = (node: number) =>
		execOut[node]
			.sort(
				(a, b) =>
					a.priority - b.priority ||
					a.y - b.y ||
					a.x - b.x ||
					(a.id < b.id ? -1 : a.id > b.id ? 1 : 0),
			)
			.map((target) => target.node);

	const visit = (root: number) => {
		if (visited[root]) return;
		visited[root] = 1;
		const stack = [
			{ node: root, children: providersOf(root), next: 0, placed: false },
		];
		while (stack.length > 0) {
			const frame = stack[stack.length - 1];
			if (frame.next < frame.children.length) {
				const child = frame.children[frame.next++];
				if (!visited[child]) {
					visited[child] = 1;
					stack.push({
						node: child,
						children: providersOf(child),
						next: 0,
						placed: false,
					});
				}
				continue;
			}
			if (!frame.placed) {
				order.push(frame.node);
				frame.placed = true;
				frame.children = successorsOf(frame.node);
				frame.next = 0;
				continue;
			}
			stack.pop();
		}
	};

	const all = Array.from({ length: size }, (_, index) => index);
	const starts = all.filter(
		(node) =>
			nodes[node].start ||
			nodes[node].event_callback ||
			(execIn[node].length === 0 && execOut[node].length > 0),
	);
	for (const node of starts.sort(byXY)) visit(node);
	for (const node of all.sort(byXY)) visit(node);
	return order;
}

function nodeContext(graph: BoardGraph, node: number): NodeContext {
	const cached = graph.contexts[node];
	if (cached) return cached;
	const pred: number[] = [];
	const chain = [node];
	let current = node;
	while (pred.length < MAX_PRED) {
		const previous = graph.execPred[current];
		if (previous < 0 || chain.includes(previous)) break;
		chain.push(previous);
		pred.push(graph.typeOf[previous]);
		current = previous;
	}
	const { types } = graph;
	const ups = [
		...new Set(graph.providers[node].map((src) => graph.typeOf[src])),
	]
		.sort((a, b) => (types[a] < types[b] ? -1 : types[a] > types[b] ? 1 : 0))
		.slice(0, MAX_UPS);
	const context = { pred, ups };
	graph.contexts[node] = context;
	return context;
}

function contextRecord(
	graph: BoardGraph,
	node: number,
	pin: IPin,
): ContextRecord {
	const { pred, ups } = nodeContext(graph, node);
	return {
		kind: kindOf(pin),
		src: graph.typeOf[node],
		srcPin: pin.name,
		dataType: pin.data_type,
		valueType: pin.value_type,
		schema: schemaTitle(graph.refs, pin.schema),
		pred,
		ups,
		bag: graph.bagAt[node],
	};
}

function buildFacts(graph: BoardGraph): BoardFacts {
	const transitions: FactTransition[] = [];
	const seen = new Set<string>();
	for (const edge of graph.edges) {
		const key = `${edge.src}\u0000${edge.srcPin.name}\u0000${edge.dst}`;
		if (seen.has(key)) continue;
		seen.add(key);
		transitions.push({
			...contextRecord(graph, edge.src, edge.srcPin),
			dst: graph.typeOf[edge.dst],
			dstPin: edge.dstPin.name,
		});
	}
	const outcomes: FactOutcome[] = [];
	for (const node of graph.order) {
		for (const pin of Object.values(graph.nodes[node].pins ?? {})) {
			if (pin.pin_type !== IPinType.Output) continue;
			outcomes.push({
				kind: kindOf(pin),
				src: graph.typeOf[node],
				srcPin: pin.name,
				connected: graph.connected.has(pin.id),
			});
		}
	}
	return {
		types: graph.types,
		transitions,
		outcomes,
		repeated: graph.repeated,
	};
}
