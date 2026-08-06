import type { ILayer } from "../schema/flow/board";
import type { INode, IPin } from "../schema/flow/node";
import { IPinType } from "../schema/flow/node";
import {
	isExecPin,
	isInputPin,
	isOutputPin,
	isPlausibleSize,
	isRerouteNode,
	measureLayerBox,
	measureNodeBox,
	pinOffsetY,
	visiblePinsOf,
} from "./measure";
import type {
	AutoLayoutInput,
	LEdge,
	LGraph,
	LNode,
	LNodeKind,
	PinRef,
} from "./types";

function makePinRef(pin: IPin, isReroute: boolean): PinRef {
	return {
		id: pin.id,
		index: pin.index,
		offsetY: pinOffsetY(pin, isReroute),
		isExec: isExecPin(pin),
	};
}

function buildPinOwnerMap(
	nodes: INode[],
	entities: readonly { id: string }[],
	boardLayers?: Record<string, ILayer>,
): Map<string, string> {
	const pinOwner = new Map<string, string>();
	for (const node of nodes) {
		for (const pin of Object.values(node.pins ?? {})) {
			pinOwner.set(pin.id, node.id);
		}
	}
	if (!boardLayers) return pinOwner;

	for (const entity of entities) {
		const layer = boardLayers[entity.id];
		if (!layer) continue;
		for (const pin of Object.values(layer.pins ?? {})) {
			pinOwner.set(pin.id, entity.id);
		}
	}
	return pinOwner;
}

/**
 * Maps a node id nested anywhere inside a visible layer entity back to that
 * entity, so `fn_refs` pointing at a node inside a collapsed layer resolve.
 */
export function buildFnRefNodeToEntityMap(
	entities: readonly { id: string }[],
	boardLayers?: Record<string, ILayer>,
): Map<string, string> {
	const nodeToEntity = new Map<string, string>();
	if (!boardLayers) return nodeToEntity;

	const childrenByParent = new Map<string, string[]>();
	for (const layer of Object.values(boardLayers)) {
		const parentId =
			(layer.parent_id ?? "") === "" ? undefined : layer.parent_id;
		if (!parentId) continue;
		const children = childrenByParent.get(parentId) ?? [];
		children.push(layer.id);
		childrenByParent.set(parentId, children);
	}
	for (const children of childrenByParent.values()) {
		children.sort((a, b) => a.localeCompare(b));
	}

	for (const entity of entities) {
		const stack = [entity.id];
		const seen = new Set<string>();
		while (stack.length > 0) {
			const layerId = stack.pop();
			if (!layerId || seen.has(layerId)) continue;
			seen.add(layerId);

			const layer = boardLayers[layerId];
			if (!layer) continue;
			for (const nodeId of Object.keys(layer.nodes ?? {})) {
				nodeToEntity.set(nodeId, entity.id);
			}
			for (const childId of childrenByParent.get(layerId) ?? []) {
				stack.push(childId);
			}
		}
	}

	return nodeToEntity;
}

function classifyNode(
	node: INode,
	isEntity: boolean,
	hasExecPin: boolean,
): LNodeKind {
	if (isEntity) return "entity";
	if (isRerouteNode(node)) return "reroute";
	return hasExecPin ? "exec" : "pure";
}

export function buildLayoutGraph(input: AutoLayoutInput): LGraph {
	const { layerNodes, layerEntities, boardLayers, nodeSizes, only } = input;

	const entityIds = new Set(layerEntities.map((entity) => entity.id));
	const scoped = (id: string) => !only || only.has(id);

	const entityNodes: INode[] = layerEntities.map(
		(entity) =>
			({
				id: entity.id,
				coordinates: entity.coordinates,
				pins: boardLayers?.[entity.id]?.pins ?? {},
				start: false,
				event_callback: false,
				fn_refs: null,
				category: "",
				description: "",
				friendly_name: "",
				name: boardLayers?.[entity.id]?.name ?? entity.id,
			}) as unknown as INode,
	);

	const sourceNodes = [...layerNodes, ...entityNodes].filter((node) =>
		scoped(node.id),
	);

	const nodes = new Map<string, LNode>();
	for (const node of sourceNodes) {
		const isEntity = entityIds.has(node.id);
		const isReroute = !isEntity && isRerouteNode(node);
		const pins = isEntity
			? Object.values(node.pins ?? {})
			: visiblePinsOf(node);

		const execIn: PinRef[] = [];
		const execOut: PinRef[] = [];
		const dataIn: PinRef[] = [];
		const dataOut: PinRef[] = [];
		for (const pin of pins) {
			const ref = makePinRef(pin, isReroute);
			if (isExecPin(pin)) {
				if (isInputPin(pin)) execIn.push(ref);
				else if (isOutputPin(pin)) execOut.push(ref);
			} else if (isInputPin(pin)) {
				dataIn.push(ref);
			} else if (isOutputPin(pin)) {
				dataOut.push(ref);
			}
		}
		const byIndex = (a: PinRef, b: PinRef) =>
			a.index - b.index || a.id.localeCompare(b.id);
		execIn.sort(byIndex);
		execOut.sort(byIndex);
		dataIn.sort(byIndex);
		dataOut.sort(byIndex);

		const measured = nodeSizes?.get(node.id);
		const fallback = isEntity
			? measureLayerBox(
					boardLayers?.[node.id] ?? ({ pins: node.pins } as unknown as ILayer),
				)
			: measureNodeBox(node);
		const useMeasured = measured && isPlausibleSize(measured[0], measured[1]);

		nodes.set(node.id, {
			id: node.id,
			kind: classifyNode(
				node,
				isEntity,
				execIn.length > 0 || execOut.length > 0,
			),
			isStart: node.start === true,
			width: useMeasured ? measured[0] : fallback.width,
			height: useMeasured ? measured[1] : fallback.height,
			execIn,
			execOut,
			dataIn,
			dataOut,
			out: [],
			in: [],
			fnRefTargets: [...(node.fn_refs?.fn_refs ?? [])].sort((a, b) =>
				a.localeCompare(b),
			),
			component: -1,
			column: 0,
			order: 0,
			owner: null,
			depth: 0,
			x: 0,
			y: 0,
			placed: false,
		});
	}

	const pinOwner = buildPinOwnerMap(sourceNodes, layerEntities, boardLayers);
	const pinRefById = new Map<string, PinRef>();
	for (const node of sourceNodes) {
		const isEntity = entityIds.has(node.id);
		const isReroute = !isEntity && isRerouteNode(node);
		const pins = isEntity
			? Object.values(node.pins ?? {})
			: visiblePinsOf(node);
		for (const pin of pins) {
			pinRefById.set(pin.id, makePinRef(pin, isReroute));
		}
	}

	const edges: LEdge[] = [];
	const seenEdges = new Set<string>();
	for (const node of sourceNodes) {
		const isEntity = entityIds.has(node.id);
		const pins = isEntity
			? Object.values(node.pins ?? {})
			: visiblePinsOf(node);
		const outputs = pins
			.filter((pin) => pin.pin_type === IPinType.Output)
			.sort((a, b) => a.index - b.index || a.id.localeCompare(b.id));

		for (const pin of outputs) {
			const fromPin = pinRefById.get(pin.id);
			if (!fromPin) continue;
			const targets = [...(pin.connected_to ?? [])].sort((a, b) =>
				a.localeCompare(b),
			);
			for (const targetPinId of targets) {
				const targetId = pinOwner.get(targetPinId);
				if (!targetId || targetId === node.id) continue;
				if (!nodes.has(targetId) || !nodes.has(node.id)) continue;

				const toPin = pinRefById.get(targetPinId);
				if (!toPin) continue;

				const key = `${pin.id}->${targetPinId}`;
				if (seenEdges.has(key)) continue;
				seenEdges.add(key);

				edges.push({
					from: node.id,
					to: targetId,
					fromPin,
					toPin,
					kind: isExecPin(pin) ? "exec" : "data",
					reversed: false,
				});
			}
		}
	}

	// One canonical adjacency order: source pin row, then target pin row, then id.
	const byPinOrder = (a: LEdge, b: LEdge) =>
		a.fromPin.index - b.fromPin.index ||
		a.toPin.index - b.toPin.index ||
		a.to.localeCompare(b.to) ||
		a.from.localeCompare(b.from);

	for (const edge of edges) {
		nodes.get(edge.from)?.out.push(edge);
		nodes.get(edge.to)?.in.push(edge);
	}
	for (const node of nodes.values()) {
		node.out.sort(byPinOrder);
		node.in.sort(byPinOrder);
	}

	return {
		nodes,
		order: [...nodes.keys()].sort((a, b) => a.localeCompare(b)),
		edges,
		entityIds,
	};
}

// ─── Components ──────────────────────────────────────────────────────────────

export interface ComponentInfo {
	id: number;
	nodeIds: string[];
	roots: string[];
}

/**
 * Weakly connected components over exec+data edges. `fn_refs` are deliberately
 * excluded: they are a placement relation, not connectivity.
 *
 * One component is laid out as one unit, which is what structurally prevents a
 * node from being claimed — and overwritten — by two different event groups.
 */
export function findComponents(graph: LGraph): ComponentInfo[] {
	const parent = new Map<string, string>();
	for (const id of graph.order) parent.set(id, id);

	const find = (id: string): string => {
		let root = id;
		while (parent.get(root) !== root) {
			root = parent.get(root) as string;
		}
		let cursor = id;
		while (parent.get(cursor) !== root) {
			const next = parent.get(cursor) as string;
			parent.set(cursor, root);
			cursor = next;
		}
		return root;
	};

	const union = (a: string, b: string) => {
		const rootA = find(a);
		const rootB = find(b);
		if (rootA === rootB) return;
		// Deterministic merge direction.
		if (rootA < rootB) parent.set(rootB, rootA);
		else parent.set(rootA, rootB);
	};

	for (const edge of graph.edges) union(edge.from, edge.to);

	const byRoot = new Map<string, string[]>();
	for (const id of graph.order) {
		const root = find(id);
		const members = byRoot.get(root) ?? [];
		members.push(id);
		byRoot.set(root, members);
	}

	const roots = [...byRoot.keys()].sort((a, b) => a.localeCompare(b));
	return roots.map((root, index) => {
		const nodeIds = byRoot.get(root) as string[];
		const info: ComponentInfo = {
			id: index,
			nodeIds,
			roots: pickComponentRoots(graph, nodeIds),
		};
		for (const id of nodeIds) {
			const node = graph.nodes.get(id);
			if (node) node.component = index;
		}
		return info;
	});
}

/**
 * Deterministic, coordinate-free root selection. Priority: explicit start
 * nodes, then exec nodes with no incoming exec edge, then nodes with no
 * incoming edge at all, then the lowest id.
 */
function pickComponentRoots(graph: LGraph, nodeIds: string[]): string[] {
	const starts: string[] = [];
	const execSources: string[] = [];
	const sources: string[] = [];

	for (const id of nodeIds) {
		const node = graph.nodes.get(id);
		if (!node) continue;
		if (node.isStart) {
			starts.push(id);
			continue;
		}
		const hasExecIn = node.in.some((edge) => edge.kind === "exec");
		if (node.kind === "exec" && !hasExecIn) {
			execSources.push(id);
			continue;
		}
		if (node.in.length === 0) sources.push(id);
	}

	const roots =
		starts.length > 0 ? starts : execSources.length > 0 ? execSources : sources;
	if (roots.length > 0) return [...roots].sort((a, b) => a.localeCompare(b));
	return [nodeIds.slice().sort((a, b) => a.localeCompare(b))[0]];
}

// ─── Cycle breaking ──────────────────────────────────────────────────────────

/**
 * Iterative three-colour DFS. Every edge into a grey (on-stack) node is a back
 * edge and is marked `reversed`; all later phases ignore reversed edges, which
 * makes every subsequent pass a single sweep over a proven DAG. Termination is
 * structural rather than guarded — this is what removes the pure-data-cycle
 * hang and the right-to-left fallback ordering.
 */
export function breakCycles(graph: LGraph, components: ComponentInfo[]): void {
	const WHITE = 0;
	const GREY = 1;
	const BLACK = 2;
	const colour = new Map<string, number>();
	for (const id of graph.order) colour.set(id, WHITE);

	const seeds: string[] = [];
	for (const component of components) {
		seeds.push(...component.roots);
	}
	seeds.push(...graph.order);

	for (const seed of seeds) {
		if (colour.get(seed) !== WHITE) continue;

		const stack: Array<{ id: string; next: number }> = [{ id: seed, next: 0 }];
		colour.set(seed, GREY);

		while (stack.length > 0) {
			const frame = stack[stack.length - 1];
			const node = graph.nodes.get(frame.id);
			if (!node || frame.next >= node.out.length) {
				colour.set(frame.id, BLACK);
				stack.pop();
				continue;
			}

			const edge = node.out[frame.next];
			frame.next += 1;
			if (edge.reversed) continue;

			const targetColour = colour.get(edge.to);
			if (targetColour === GREY) {
				edge.reversed = true;
				continue;
			}
			if (targetColour === BLACK) continue;

			colour.set(edge.to, GREY);
			stack.push({ id: edge.to, next: 0 });
		}
	}
}
