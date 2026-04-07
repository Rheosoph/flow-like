import type { ILayer } from "./schema/flow/board/commands/upsert-layer";
import type { INode, IPin } from "./schema/flow/node";
import { IPinType, IVariableType } from "./schema/flow/node";

// ─── Types ───────────────────────────────────────────────────────────────────

interface LayoutNode {
	id: string;
	isStart: boolean;
	isEventCallback: boolean;
	isImpure: boolean;
	pinCount: number;
	execOutCount: number;
	execInCount: number;
	fnRefTargets: string[];
	canBeReferencedByFns: boolean;
	outgoingExec: Map<string, string[]>;
	outgoingData: Map<string, string[]>;
	incomingExec: Set<string>;
	incomingData: Set<string>;
	sortX: number;
	sortY: number;
}

interface LayoutEntity {
	id: string;
	coordinates: number[];
}

export interface AutoLayoutInput {
	layerNodes: INode[];
	layerEntities: LayoutEntity[];
	boardLayers?: Record<string, ILayer>;
	currentLayer: string | undefined;
}

export type LayoutStyle = "compact" | "expanded" | "balanced";

interface StyleConfig {
	hGap: number;
	vGap: number;
	pureHGap: number;
	pureVGap: number;
	eventGroupGap: number;
	branchSpread: number;
}

function getStyleConfig(style: LayoutStyle): StyleConfig {
	switch (style) {
		case "compact":
			return {
				hGap: 320,
				vGap: 160,
				pureHGap: 280,
				pureVGap: 130,
				eventGroupGap: 300,
				branchSpread: 1,
			};
		case "expanded":
			return {
				hGap: 450,
				vGap: 240,
				pureHGap: 380,
				pureVGap: 200,
				eventGroupGap: 500,
				branchSpread: 1.5,
			};
		default:
			return {
				hGap: 380,
				vGap: 190,
				pureHGap: 320,
				pureVGap: 160,
				eventGroupGap: 380,
				branchSpread: 1.2,
			};
	}
}

// ─── Pin Helpers ─────────────────────────────────────────────────────────────

function isExecPin(pin: IPin): boolean {
	return pin.data_type === IVariableType.Execution;
}

function isOutputPin(pin: IPin): boolean {
	return pin.pin_type === IPinType.Output;
}

function isInputPin(pin: IPin): boolean {
	return pin.pin_type === IPinType.Input;
}

const DEFAULT_ENTITY_WIDTH = 300;
const DEFAULT_NODE_HEIGHT = 88;
const DEFAULT_NODE_WIDTH = 240;

function estimateNodeHeight(node: INode): number {
	const pins = Object.values(node.pins);
	const inputCount = pins.filter(isInputPin).length;
	const outputCount = pins.filter(isOutputPin).length;
	return Math.max(
		DEFAULT_NODE_HEIGHT,
		Math.max(inputCount, outputCount) * 15 + 28,
	);
}

function estimateNodeWidth(node: INode, isEntity: boolean): number {
	const pins = Object.values(node.pins);
	const maxPins = Math.max(
		pins.filter(isInputPin).length,
		pins.filter(isOutputPin).length,
	);
	const baseWidth = isEntity ? DEFAULT_ENTITY_WIDTH : DEFAULT_NODE_WIDTH;
	return baseWidth + Math.max(0, maxPins - 4) * 8;
}

function compareNodeIdsByPreferredPosition(
	aId: string,
	bId: string,
	graph: Map<string, LayoutNode>,
): number {
	const a = graph.get(aId);
	const b = graph.get(bId);
	if (!a || !b) return aId.localeCompare(bId);
	return a.sortY - b.sortY || a.sortX - b.sortX || a.id.localeCompare(b.id);
}

function pushUnique(values: string[], value: string) {
	if (!values.includes(value)) {
		values.push(value);
	}
}

function getPureDirection(
	nodeId: string,
	anchorY: number,
	graph: Map<string, LayoutNode>,
): -1 | 1 {
	const node = graph.get(nodeId);
	if (!node) {
		return -1;
	}

	return node.sortY <= anchorY ? -1 : 1;
}

function offsetPureYFromAnchor(
	desiredY: number,
	anchorY: number,
	direction: -1 | 1,
	gap: number,
): number {
	const bandY = anchorY + direction * gap;
	if (direction < 0) {
		return Math.min(desiredY, bandY);
	}

	return Math.max(desiredY, bandY);
}

// ─── Graph Building ──────────────────────────────────────────────────────────

function buildPinOwnerMap(
	nodes: INode[],
	layers: LayoutEntity[],
	boardLayers?: Record<string, ILayer>,
): Map<string, string> {
	const pinOwner = new Map<string, string>();
	for (const node of nodes) {
		for (const pin of Object.values(node.pins)) {
			pinOwner.set(pin.id, node.id);
		}
	}
	if (boardLayers) {
		for (const entity of layers) {
			const layer = boardLayers[entity.id];
			if (!layer) continue;
			for (const pin of Object.values(layer.pins)) {
				pinOwner.set(pin.id, entity.id);
			}
		}
	}
	return pinOwner;
}

function buildLayoutGraph(
	nodes: INode[],
	entities: LayoutEntity[],
	pinOwner: Map<string, string>,
): Map<string, LayoutNode> {
	const graph = new Map<string, LayoutNode>();
	const nodeMap = new Map<string, INode>();

	for (const node of nodes) {
		nodeMap.set(node.id, node);
	}

	const allIds = new Set([
		...nodes.map((n) => n.id),
		...entities.map((e) => e.id),
	]);

	for (const id of allIds) {
		const node = nodeMap.get(id);
		const pins = node ? Object.values(node.pins) : [];
		const execOutPins = pins.filter((p) => isExecPin(p) && isOutputPin(p));
		const execInPins = pins.filter((p) => isExecPin(p) && isInputPin(p));

		graph.set(id, {
			id,
			isStart: node?.start === true,
			isEventCallback: node?.event_callback === true,
			isImpure: execInPins.length > 0 || execOutPins.length > 0,
			pinCount: Math.max(
				pins.filter(isInputPin).length,
				pins.filter(isOutputPin).length,
			),
			execOutCount: execOutPins.length,
			execInCount: execInPins.length,
			fnRefTargets: node?.fn_refs?.fn_refs ?? [],
			canBeReferencedByFns: node?.fn_refs?.can_be_referenced_by_fns === true,
			outgoingExec: new Map(),
			outgoingData: new Map(),
			incomingExec: new Set(),
			incomingData: new Set(),
			sortX: node?.coordinates?.[0] ?? 0,
			sortY: node?.coordinates?.[1] ?? 0,
		});
	}

	for (const node of nodes) {
		for (const pin of Object.values(node.pins)) {
			if (!isOutputPin(pin)) continue;
			for (const targetPinId of pin.connected_to) {
				const targetId = pinOwner.get(targetPinId);
				if (!targetId || !allIds.has(targetId) || targetId === node.id)
					continue;

				const layoutNode = graph.get(node.id);
				const targetLayout = graph.get(targetId);
				if (!layoutNode || !targetLayout) continue;

				if (isExecPin(pin)) {
					const targets = layoutNode.outgoingExec.get(pin.id) ?? [];
					pushUnique(targets, targetId);
					layoutNode.outgoingExec.set(pin.id, targets);
					targetLayout.incomingExec.add(node.id);
				} else {
					const existing = layoutNode.outgoingData.get(pin.id) ?? [];
					pushUnique(existing, targetId);
					layoutNode.outgoingData.set(pin.id, existing);
					targetLayout.incomingData.add(node.id);
				}
			}
		}
	}

	return graph;
}

function getExecTargets(
	node: LayoutNode,
	graph: Map<string, LayoutNode>,
	allowedIds?: Set<string>,
): string[] {
	const targets = new Set<string>();
	for (const pinTargets of node.outgoingExec.values()) {
		for (const targetId of pinTargets) {
			if (allowedIds && !allowedIds.has(targetId)) continue;
			targets.add(targetId);
		}
	}
	return [...targets].sort((a, b) =>
		compareNodeIdsByPreferredPosition(a, b, graph),
	);
}

function getDataTargets(
	node: LayoutNode,
	graph: Map<string, LayoutNode>,
	allowedIds?: Set<string>,
): string[] {
	const targets = new Set<string>();
	for (const pinTargets of node.outgoingData.values()) {
		for (const targetId of pinTargets) {
			if (allowedIds && !allowedIds.has(targetId)) continue;
			targets.add(targetId);
		}
	}
	return [...targets].sort((a, b) =>
		compareNodeIdsByPreferredPosition(a, b, graph),
	);
}

function buildFnRefNodeToEntityMap(
	entities: LayoutEntity[],
	boardLayers?: Record<string, ILayer>,
): Map<string, string> {
	const fnRefNodeToEntity = new Map<string, string>();
	if (!boardLayers) {
		return fnRefNodeToEntity;
	}

	const childLayersByParent = new Map<string, string[]>();
	for (const layer of Object.values(boardLayers)) {
		const parentId =
			(layer.parent_id ?? "") === "" ? undefined : layer.parent_id;
		if (!parentId) continue;
		const children = childLayersByParent.get(parentId) ?? [];
		children.push(layer.id);
		childLayersByParent.set(parentId, children);
	}

	for (const entity of entities) {
		const stack = [entity.id];
		while (stack.length > 0) {
			const layerId = stack.pop();
			if (!layerId) continue;
			const layer = boardLayers[layerId];
			if (!layer) continue;

			for (const nodeId of Object.keys(layer.nodes)) {
				fnRefNodeToEntity.set(nodeId, entity.id);
			}

			for (const childId of childLayersByParent.get(layerId) ?? []) {
				stack.push(childId);
			}
		}
	}

	return fnRefNodeToEntity;
}

// ─── Event Group Discovery ───────────────────────────────────────────────────

interface EventGroup {
	startNodeId: string;
	execChain: string[];
	pureNodes: Set<string>;
	allNodes: Set<string>;
}

interface GroupBounds {
	minX: number;
	minY: number;
	maxX: number;
	maxY: number;
}

interface GroupLayout {
	positions: Map<string, [number, number]>;
	bounds: GroupBounds;
}

interface FnRefEdge {
	fromGroupId: string;
	fromNodeId: string;
	toGroupId: string;
}

function resolveFnRefTargetId(
	targetId: string,
	graph: Map<string, LayoutNode>,
	fnRefNodeToEntity: Map<string, string>,
): null | string {
	if (graph.has(targetId)) return targetId;
	return fnRefNodeToEntity.get(targetId) ?? null;
}

function collectPureDepsForGroup(
	nodeId: string,
	graph: Map<string, LayoutNode>,
	pureNodes: Set<string>,
	allNodes: Set<string>,
	blockedNodes: Set<string>,
	rootId: string,
) {
	const node = graph.get(nodeId);
	if (!node) return;

	for (const depId of node.incomingData) {
		if (allNodes.has(depId)) continue;
		if (depId !== rootId && blockedNodes.has(depId)) continue;
		const dep = graph.get(depId);
		if (!dep || dep.isImpure) continue;

		pureNodes.add(depId);
		allNodes.add(depId);
		collectPureDepsForGroup(
			depId,
			graph,
			pureNodes,
			allNodes,
			blockedNodes,
			rootId,
		);
	}

	for (const targetId of getDataTargets(node, graph)) {
		if (allNodes.has(targetId)) continue;
		if (targetId !== rootId && blockedNodes.has(targetId)) continue;
		const target = graph.get(targetId);
		if (!target || target.isImpure) continue;

		pureNodes.add(targetId);
		allNodes.add(targetId);
		collectPureDepsForGroup(
			targetId,
			graph,
			pureNodes,
			allNodes,
			blockedNodes,
			rootId,
		);
	}
}

function buildEventGroup(
	startNodeId: string,
	graph: Map<string, LayoutNode>,
	blockedNodes: Set<string>,
): EventGroup {
	const execChain: string[] = [];
	const pureNodes = new Set<string>();
	const allNodes = new Set<string>();
	const execVisited = new Set<string>();
	const execQueue = [startNodeId];

	while (execQueue.length > 0) {
		const nodeId = execQueue.shift();
		if (!nodeId) continue;
		if (execVisited.has(nodeId)) continue;
		if (nodeId !== startNodeId && blockedNodes.has(nodeId)) continue;

		execVisited.add(nodeId);
		execChain.push(nodeId);
		allNodes.add(nodeId);

		const node = graph.get(nodeId);
		if (!node) continue;
		for (const targetId of getExecTargets(node, graph)) {
			if (!execVisited.has(targetId)) {
				execQueue.push(targetId);
			}
		}
	}

	for (const execNodeId of execChain) {
		collectPureDepsForGroup(
			execNodeId,
			graph,
			pureNodes,
			allNodes,
			blockedNodes,
			startNodeId,
		);
	}

	return {
		startNodeId,
		execChain,
		pureNodes,
		allNodes,
	};
}

function discoverEventGroups(
	graph: Map<string, LayoutNode>,
	fnRefNodeToEntity: Map<string, string>,
): EventGroup[] {
	const groups: EventGroup[] = [];
	const orderedNodes = [...graph.values()].sort((a, b) => {
		if (a.isStart && !b.isStart) return -1;
		if (!a.isStart && b.isStart) return 1;
		return a.sortY - b.sortY || a.sortX - b.sortX || a.id.localeCompare(b.id);
	});

	const startRootIds = orderedNodes
		.filter((n) => n.isStart)
		.map((node) => node.id);

	const startRootSet = new Set(startRootIds);
	const fnRefRootIds: string[] = [];
	const fnRefRootSeen = new Set<string>();

	for (const node of orderedNodes) {
		for (const targetId of node.fnRefTargets) {
			const resolvedId = resolveFnRefTargetId(
				targetId,
				graph,
				fnRefNodeToEntity,
			);
			if (
				!resolvedId ||
				startRootSet.has(resolvedId) ||
				fnRefRootSeen.has(resolvedId)
			) {
				continue;
			}
			fnRefRootSeen.add(resolvedId);
			fnRefRootIds.push(resolvedId);
		}
	}

	const orderedRootIds = [...startRootIds, ...fnRefRootIds];
	const blockedEventRoots = new Set(orderedRootIds);

	for (const rootId of orderedRootIds) {
		groups.push(buildEventGroup(rootId, graph, blockedEventRoots));
	}

	const claimed = new Set<string>();
	for (const group of groups) {
		for (const nodeId of group.allNodes) {
			claimed.add(nodeId);
		}
	}

	const unclaimed = [...graph.keys()].filter((id) => !claimed.has(id));
	if (unclaimed.length > 0) {
		const impure = unclaimed.filter((id) => graph.get(id)?.isImpure);

		for (const nodeId of impure) {
			if (claimed.has(nodeId)) continue;
			const group = buildEventGroup(nodeId, graph, claimed);
			for (const claimedId of group.allNodes) {
				claimed.add(claimedId);
			}
			groups.push(group);
		}

		const stillUnclaimed = [...graph.keys()].filter((id) => !claimed.has(id));
		if (stillUnclaimed.length > 0) {
			const orphan: EventGroup = {
				startNodeId: stillUnclaimed[0],
				execChain: [],
				pureNodes: new Set(stillUnclaimed),
				allNodes: new Set(stillUnclaimed),
			};
			for (const id of stillUnclaimed) claimed.add(id);
			groups.push(orphan);
		}
	}

	return groups;
}

// ─── Pure Chain Gap Computation ──────────────────────────────────────────────

function computePureChainGap(
	sourceId: string,
	sinkId: string,
	graph: Map<string, LayoutNode>,
	pureNodes: Set<string>,
	memo: Map<string, number>,
): number {
	const cacheKey = `${sourceId}->${sinkId}`;
	const cached = memo.get(cacheKey);
	if (typeof cached === "number") {
		return cached;
	}

	const source = graph.get(sourceId);
	const sink = graph.get(sinkId);
	if (!source || !sink) {
		memo.set(cacheKey, 0);
		return 0;
	}

	const depths = new Map<string, number>();
	const queue: [string, number][] = [];

	for (const tid of getDataTargets(source, graph, pureNodes)) {
		queue.push([tid, 1]);
	}

	while (queue.length > 0) {
		const next = queue.shift();
		if (!next) continue;
		const [nodeId, depth] = next;
		const existingDepth = depths.get(nodeId);
		if (typeof existingDepth === "number" && existingDepth >= depth) continue;
		depths.set(nodeId, depth);

		const node = graph.get(nodeId);
		if (!node) continue;
		for (const tid of getDataTargets(node, graph, pureNodes)) {
			queue.push([tid, depth + 1]);
		}
	}

	let maxDepth = 0;
	for (const depId of sink.incomingData) {
		const d = depths.get(depId);
		if (d !== undefined) maxDepth = Math.max(maxDepth, d);
	}

	memo.set(cacheKey, maxDepth);
	return maxDepth;
}

// ─── Execution Chain Layout ──────────────────────────────────────────────────

interface ExecLayoutResult {
	execPositions: Map<string, [number, number]>;
	inlinePurePositions: Map<string, [number, number]>;
	placedPureIds: Set<string>;
}

function computeExecColumns(
	group: EventGroup,
	graph: Map<string, LayoutNode>,
): Map<string, number> {
	const execIds = new Set(group.execChain);
	const predecessors = new Map<string, string[]>();
	for (const nodeId of group.execChain) {
		predecessors.set(nodeId, []);
	}

	for (const nodeId of group.execChain) {
		const node = graph.get(nodeId);
		if (!node) continue;
		for (const targetId of getExecTargets(node, graph, execIds)) {
			const deps = predecessors.get(targetId) ?? [];
			deps.push(nodeId);
			predecessors.set(targetId, deps);
		}
	}

	const indegree = new Map<string, number>();
	for (const nodeId of group.execChain) {
		indegree.set(nodeId, predecessors.get(nodeId)?.length ?? 0);
	}

	const columns = new Map<string, number>();
	const gapCache = new Map<string, number>();
	const queue = group.execChain
		.filter((nodeId) => (indegree.get(nodeId) ?? 0) === 0)
		.sort((a, b) => compareNodeIdsByPreferredPosition(a, b, graph));

	if (!queue.includes(group.startNodeId)) {
		queue.unshift(group.startNodeId);
	}

	const processed = new Set<string>();
	while (queue.length > 0) {
		const nodeId = queue.shift();
		if (!nodeId) continue;
		if (processed.has(nodeId)) continue;

		const deps = predecessors.get(nodeId) ?? [];
		let currentColumn = columns.get(nodeId) ?? 0;
		if (deps.length > 0) {
			currentColumn = Math.max(
				currentColumn,
				...deps.map((depId) => {
					const depColumn = columns.get(depId) ?? 0;
					return (
						depColumn +
						computePureChainGap(
							depId,
							nodeId,
							graph,
							group.pureNodes,
							gapCache,
						) +
						1
					);
				}),
			);
		}
		if (nodeId === group.startNodeId) {
			currentColumn = 0;
		}

		columns.set(nodeId, currentColumn);
		processed.add(nodeId);

		const node = graph.get(nodeId);
		if (!node) continue;
		for (const targetId of getExecTargets(node, graph, execIds)) {
			const nextColumn =
				currentColumn +
				computePureChainGap(
					nodeId,
					targetId,
					graph,
					group.pureNodes,
					gapCache,
				) +
				1;
			if ((columns.get(targetId) ?? Number.NEGATIVE_INFINITY) < nextColumn) {
				columns.set(targetId, nextColumn);
			}

			const nextDegree = Math.max(0, (indegree.get(targetId) ?? 0) - 1);
			indegree.set(targetId, nextDegree);
			if (
				nextDegree === 0 &&
				!processed.has(targetId) &&
				!queue.includes(targetId)
			) {
				queue.push(targetId);
				queue.sort((a, b) => compareNodeIdsByPreferredPosition(a, b, graph));
			}
		}
	}

	if (processed.size < group.execChain.length) {
		const remaining = group.execChain
			.filter((nodeId) => !processed.has(nodeId))
			.sort((a, b) => compareNodeIdsByPreferredPosition(a, b, graph));

		for (const nodeId of remaining) {
			const deps = predecessors.get(nodeId) ?? [];
			const fallbackColumn = deps.reduce((maxColumn, depId) => {
				const depColumn = columns.get(depId) ?? 0;
				return Math.max(
					maxColumn,
					depColumn +
						computePureChainGap(
							depId,
							nodeId,
							graph,
							group.pureNodes,
							gapCache,
						) +
						1,
				);
			}, columns.get(nodeId) ?? 0);

			columns.set(nodeId, nodeId === group.startNodeId ? 0 : fallbackColumn);
		}
	}

	return columns;
}

function findNearestAvailableRow(
	desiredRow: number,
	usedRows: Set<number>,
): number {
	const baseRow = Number.isFinite(desiredRow) ? Math.round(desiredRow) : 0;
	if (!usedRows.has(baseRow)) {
		return baseRow;
	}

	for (let offset = 1; offset < 128; offset++) {
		const upper = baseRow + offset;
		if (!usedRows.has(upper)) {
			return upper;
		}

		const lower = baseRow - offset;
		if (!usedRows.has(lower)) {
			return lower;
		}
	}

	return baseRow + usedRows.size;
}

function computeExecRows(
	group: EventGroup,
	graph: Map<string, LayoutNode>,
	execColumns: Map<string, number>,
): Map<string, number> {
	const execIds = new Set(group.execChain);
	const predecessors = new Map<string, string[]>();
	for (const nodeId of group.execChain) {
		predecessors.set(nodeId, []);
	}

	for (const nodeId of group.execChain) {
		const node = graph.get(nodeId);
		if (!node) continue;
		for (const targetId of getExecTargets(node, graph, execIds)) {
			const deps = predecessors.get(targetId) ?? [];
			deps.push(nodeId);
			predecessors.set(targetId, deps);
		}
	}

	const rows = new Map<string, number>();
	const nodesByColumn = new Map<number, string[]>();
	for (const nodeId of group.execChain) {
		const column = execColumns.get(nodeId) ?? 0;
		const list = nodesByColumn.get(column) ?? [];
		list.push(nodeId);
		nodesByColumn.set(column, list);
	}

	const sortedColumns = [...nodesByColumn.keys()].sort((a, b) => a - b);
	for (const column of sortedColumns) {
		const usedRows = new Set<number>();
		const nodeIds = (nodesByColumn.get(column) ?? []).sort((a, b) => {
			const depRowsA = (predecessors.get(a) ?? [])
				.map((depId) => rows.get(depId))
				.filter((value): value is number => typeof value === "number");
			const depRowsB = (predecessors.get(b) ?? [])
				.map((depId) => rows.get(depId))
				.filter((value): value is number => typeof value === "number");
			const desiredA =
				depRowsA.length > 0
					? depRowsA.reduce((sum, value) => sum + value, 0) / depRowsA.length
					: 0;
			const desiredB =
				depRowsB.length > 0
					? depRowsB.reduce((sum, value) => sum + value, 0) / depRowsB.length
					: 0;
			return (
				desiredA - desiredB || compareNodeIdsByPreferredPosition(a, b, graph)
			);
		});

		for (const nodeId of nodeIds) {
			if (nodeId === group.startNodeId) {
				rows.set(nodeId, 0);
				usedRows.add(0);
				continue;
			}

			const depRows = (predecessors.get(nodeId) ?? [])
				.map((depId) => rows.get(depId))
				.filter((value): value is number => typeof value === "number");
			const desiredRow =
				depRows.length > 0
					? depRows.reduce((sum, value) => sum + value, 0) / depRows.length
					: 0;
			const row = findNearestAvailableRow(desiredRow, usedRows);
			rows.set(nodeId, row);
			usedRows.add(row);
		}
	}

	return rows;
}

function layoutExecChain(
	group: EventGroup,
	graph: Map<string, LayoutNode>,
	startX: number,
	startY: number,
	cfg: StyleConfig,
): ExecLayoutResult {
	const execPositions = new Map<string, [number, number]>();
	const inlinePurePositions = new Map<string, [number, number]>();
	const placedPureIds = new Set<string>();
	const groupExecIds = new Set(group.execChain);

	if (group.execChain.length === 0) {
		return { execPositions, inlinePurePositions, placedPureIds };
	}

	const execColumns = computeExecColumns(group, graph);
	const execRows = computeExecRows(group, graph, execColumns);

	for (const nodeId of group.execChain) {
		const column = execColumns.get(nodeId) ?? 0;
		const row = execRows.get(nodeId) ?? 0;
		execPositions.set(nodeId, [
			startX + column * cfg.hGap,
			startY + row * cfg.vGap,
		]);
	}

	// Place inline pure nodes in the gap columns between exec nodes
	const sortedExecIds = [...execPositions.keys()].sort((a, b) => {
		const colDelta = (execColumns.get(a) ?? 0) - (execColumns.get(b) ?? 0);
		if (colDelta !== 0) return colDelta;
		const rowDelta = (execRows.get(a) ?? 0) - (execRows.get(b) ?? 0);
		if (rowDelta !== 0) return rowDelta;
		return compareNodeIdsByPreferredPosition(a, b, graph);
	});

	for (const execId of sortedExecIds) {
		const execNode = graph.get(execId);
		const execCol = execColumns.get(execId);
		const execPos = execPositions.get(execId);
		if (!execNode || typeof execCol !== "number" || !execPos) continue;

		const pureQueue: [string, number][] = [];
		for (const tid of getDataTargets(execNode, graph, group.pureNodes)) {
			if (!placedPureIds.has(tid)) {
				pureQueue.push([tid, 1]);
			}
		}

		const colPureNodes = new Map<number, string[]>();
		const visited = new Set<string>();

		while (pureQueue.length > 0) {
			const next = pureQueue.shift();
			if (!next) continue;
			const [nodeId, depth] = next;
			if (visited.has(nodeId) || placedPureIds.has(nodeId)) continue;
			visited.add(nodeId);
			placedPureIds.add(nodeId);

			const targetCol = execCol + depth;
			const list = colPureNodes.get(targetCol) ?? [];
			list.push(nodeId);
			colPureNodes.set(targetCol, list);

			const node = graph.get(nodeId);
			if (!node) continue;
			for (const tid of getDataTargets(node, graph, group.pureNodes)) {
				if (!visited.has(tid) && !placedPureIds.has(tid)) {
					pureQueue.push([tid, depth + 1]);
				}
			}
		}

		for (const [col, nodeIds] of [...colPureNodes.entries()].sort(
			(a, b) => a[0] - b[0],
		)) {
			const x = startX + col * cfg.hGap;
			const aboveNodes = nodeIds
				.filter((nodeId) => getPureDirection(nodeId, execPos[1], graph) < 0)
				.sort((a, b) => compareNodeIdsByPreferredPosition(a, b, graph));
			const belowNodes = nodeIds
				.filter((nodeId) => getPureDirection(nodeId, execPos[1], graph) > 0)
				.sort((a, b) => compareNodeIdsByPreferredPosition(a, b, graph));

			for (let i = 0; i < aboveNodes.length; i++) {
				inlinePurePositions.set(aboveNodes[i], [
					x,
					execPos[1] - (i + 1) * cfg.pureVGap,
				]);
			}

			for (let i = 0; i < belowNodes.length; i++) {
				inlinePurePositions.set(belowNodes[i], [
					x,
					execPos[1] + (i + 1) * cfg.pureVGap,
				]);
			}
		}
	}

	return { execPositions, inlinePurePositions, placedPureIds };
}

// ─── Remaining Pure Node Layout ──────────────────────────────────────────────

function getSpatialBucketKey(
	x: number,
	y: number,
	bucketWidth: number,
	bucketHeight: number,
): string {
	return `${Math.floor(x / bucketWidth)}:${Math.floor(y / bucketHeight)}`;
}

function buildSpatialIndex(
	positions: Map<string, [number, number]>,
	bucketWidth: number,
	bucketHeight: number,
): Map<string, string[]> {
	const index = new Map<string, string[]>();
	for (const [nodeId, [x, y]] of positions) {
		const bucketKey = getSpatialBucketKey(x, y, bucketWidth, bucketHeight);
		const bucket = index.get(bucketKey) ?? [];
		bucket.push(nodeId);
		index.set(bucketKey, bucket);
	}
	return index;
}

function addToSpatialIndex(
	index: Map<string, string[]>,
	nodeId: string,
	x: number,
	y: number,
	bucketWidth: number,
	bucketHeight: number,
) {
	const bucketKey = getSpatialBucketKey(x, y, bucketWidth, bucketHeight);
	const bucket = index.get(bucketKey) ?? [];
	bucket.push(nodeId);
	index.set(bucketKey, bucket);
}

function getNearbySpatialIds(
	index: Map<string, string[]>,
	x: number,
	y: number,
	bucketWidth: number,
	bucketHeight: number,
): string[] {
	const bucketX = Math.floor(x / bucketWidth);
	const bucketY = Math.floor(y / bucketHeight);
	const nearby = new Set<string>();

	for (let dx = -1; dx <= 1; dx++) {
		for (let dy = -1; dy <= 1; dy++) {
			for (const nodeId of index.get(`${bucketX + dx}:${bucketY + dy}`) ?? []) {
				nearby.add(nodeId);
			}
		}
	}

	return [...nearby];
}

function hasNearbyPosition(
	positions: Map<string, [number, number]>,
	index: Map<string, string[]>,
	x: number,
	y: number,
	bucketWidth: number,
	bucketHeight: number,
	thresholdX: number,
	thresholdY: number,
): boolean {
	for (const nodeId of getNearbySpatialIds(
		index,
		x,
		y,
		bucketWidth,
		bucketHeight,
	)) {
		const pos = positions.get(nodeId);
		if (!pos) continue;
		if (
			Math.abs(pos[0] - x) < thresholdX &&
			Math.abs(pos[1] - y) < thresholdY
		) {
			return true;
		}
	}
	return false;
}

function findOpenPureSlot(
	positions: Map<string, [number, number]>,
	index: Map<string, string[]>,
	x: number,
	y: number,
	bucketWidth: number,
	bucketHeight: number,
	thresholdX: number,
	thresholdY: number,
	gap: number,
	preferredDirection: -1 | 1,
): [number, number] {
	if (
		!hasNearbyPosition(
			positions,
			index,
			x,
			y,
			bucketWidth,
			bucketHeight,
			thresholdX,
			thresholdY,
		)
	) {
		return [x, y];
	}

	for (let attempt = 1; attempt < 40; attempt++) {
		const preferredY = y + preferredDirection * attempt * gap;
		if (
			!hasNearbyPosition(
				positions,
				index,
				x,
				preferredY,
				bucketWidth,
				bucketHeight,
				thresholdX,
				thresholdY,
			)
		) {
			return [x, preferredY];
		}

		const alternateY = y - preferredDirection * attempt * gap;
		if (
			!hasNearbyPosition(
				positions,
				index,
				x,
				alternateY,
				bucketWidth,
				bucketHeight,
				thresholdX,
				thresholdY,
			)
		) {
			return [x, alternateY];
		}
	}

	return [x, y];
}

function layoutRemainingPures(
	group: EventGroup,
	graph: Map<string, LayoutNode>,
	allPlacedPositions: Map<string, [number, number]>,
	skip: Set<string>,
	cfg: StyleConfig,
): Map<string, [number, number]> {
	const positions = new Map<string, [number, number]>();
	const remaining = [...group.pureNodes]
		.filter((id) => !skip.has(id))
		.sort((a, b) => compareNodeIdsByPreferredPosition(a, b, graph));
	if (remaining.length === 0) return positions;
	const remainingSet = new Set(remaining);

	const pureToAnchor = new Map<string, string>();
	const resolvedAnchors = new Map<string, null | string>();

	const pickBestAnchor = (
		pureId: string,
		candidateIds: string[],
	): null | string => {
		const uniqueCandidates = [...new Set(candidateIds)].filter((candidateId) =>
			allPlacedPositions.has(candidateId),
		);
		if (uniqueCandidates.length === 0) {
			return null;
		}

		const pureNode = graph.get(pureId);
		const sortX = pureNode?.sortX ?? 0;
		const sortY = pureNode?.sortY ?? 0;

		return uniqueCandidates.sort((a, b) => {
			const posA = allPlacedPositions.get(a) ?? [0, 0];
			const posB = allPlacedPositions.get(b) ?? [0, 0];
			const scoreA = Math.abs(posA[1] - sortY) * 2 + Math.abs(posA[0] - sortX);
			const scoreB = Math.abs(posB[1] - sortY) * 2 + Math.abs(posB[0] - sortX);
			return scoreA - scoreB || compareNodeIdsByPreferredPosition(a, b, graph);
		})[0];
	};

	const resolvePureAnchor = (
		pureId: string,
		visiting = new Set<string>(),
	): null | string => {
		if (resolvedAnchors.has(pureId)) {
			return resolvedAnchors.get(pureId) ?? null;
		}
		if (visiting.has(pureId)) {
			return null;
		}

		visiting.add(pureId);
		const pureNode = graph.get(pureId);
		if (!pureNode) {
			resolvedAnchors.set(pureId, null);
			visiting.delete(pureId);
			return null;
		}

		const directCandidates = [
			...getDataTargets(pureNode, graph).filter((targetId) =>
				allPlacedPositions.has(targetId),
			),
			...[...pureNode.incomingData].filter((depId) =>
				allPlacedPositions.has(depId),
			),
		];

		const directAnchor = pickBestAnchor(pureId, directCandidates);
		if (directAnchor) {
			resolvedAnchors.set(pureId, directAnchor);
			visiting.delete(pureId);
			return directAnchor;
		}

		const recursiveCandidates: string[] = [];
		for (const targetId of getDataTargets(pureNode, graph)) {
			if (!remainingSet.has(targetId)) continue;
			const targetAnchor = resolvePureAnchor(targetId, visiting);
			if (targetAnchor) {
				recursiveCandidates.push(targetAnchor);
			}
		}

		const resolvedAnchor = pickBestAnchor(pureId, recursiveCandidates);
		resolvedAnchors.set(pureId, resolvedAnchor);
		visiting.delete(pureId);
		return resolvedAnchor;
	};

	for (const pureId of remaining) {
		const anchor = resolvePureAnchor(pureId);
		if (anchor) {
			pureToAnchor.set(pureId, anchor);
		}
	}

	const byAnchor = new Map<string, string[]>();
	const orphans: string[] = [];

	for (const pureId of remaining) {
		const anchor = pureToAnchor.get(pureId);
		if (anchor) {
			const list = byAnchor.get(anchor) ?? [];
			list.push(pureId);
			byAnchor.set(anchor, list);
		} else {
			orphans.push(pureId);
		}
	}

	const bucketWidth = Math.max(220, Math.round(cfg.pureHGap * 0.75));
	const bucketHeight = Math.max(100, cfg.pureVGap);
	const occupancy = buildSpatialIndex(
		allPlacedPositions,
		bucketWidth,
		bucketHeight,
	);
	const placedPureLevels = new Map<string, number>();

	const pureDepthMemo = new Map<string, number>();
	const computePureDepth = (
		pureId: string,
		visiting = new Set<string>(),
	): number => {
		const cachedDepth = pureDepthMemo.get(pureId);
		if (typeof cachedDepth === "number") {
			return cachedDepth;
		}
		if (visiting.has(pureId)) {
			return 1;
		}

		visiting.add(pureId);
		const pureNode = graph.get(pureId);
		const anchorId = pureToAnchor.get(pureId);
		if (!pureNode || !anchorId) {
			visiting.delete(pureId);
			pureDepthMemo.set(pureId, 1);
			return 1;
		}

		let depth = 1;
		for (const targetId of getDataTargets(pureNode, graph)) {
			if (targetId === anchorId) {
				depth = Math.max(depth, 1);
				continue;
			}
			if (!remainingSet.has(targetId)) {
				continue;
			}
			if (pureToAnchor.get(targetId) !== anchorId) {
				continue;
			}

			depth = Math.max(depth, computePureDepth(targetId, visiting) + 1);
		}

		visiting.delete(pureId);
		pureDepthMemo.set(pureId, depth);
		return depth;
	};

	const computeDesiredPureY = (pureId: string, anchorId: string): number => {
		const pureNode = graph.get(pureId);
		const pureDepth = placedPureLevels.get(pureId) ?? computePureDepth(pureId);
		const candidateYs: number[] = [];

		if (!pureNode) {
			return allPlacedPositions.get(anchorId)?.[1] ?? 0;
		}

		for (const targetId of getDataTargets(pureNode, graph)) {
			if (targetId === anchorId) {
				const anchorPos = allPlacedPositions.get(anchorId);
				if (anchorPos) {
					candidateYs.push(anchorPos[1]);
				}
				continue;
			}

			const targetLevel = placedPureLevels.get(targetId);
			const targetPos = allPlacedPositions.get(targetId);
			if (
				targetPos &&
				typeof targetLevel === "number" &&
				targetLevel < pureDepth
			) {
				candidateYs.push(targetPos[1]);
			}
		}

		for (const depId of pureNode.incomingData) {
			const depLevel = placedPureLevels.get(depId);
			const depPos = allPlacedPositions.get(depId);
			if (depPos && typeof depLevel === "number" && depLevel > pureDepth) {
				candidateYs.push(depPos[1]);
			}
		}

		if (candidateYs.length === 0) {
			const anchorPos = allPlacedPositions.get(anchorId);
			if (anchorPos) {
				candidateYs.push(anchorPos[1]);
			}
		}

		if (candidateYs.length === 0) {
			return pureNode.sortY;
		}

		return (
			candidateYs.reduce((sum, value) => sum + value, 0) / candidateYs.length
		);
	};

	for (const [anchorId, pureIds] of byAnchor) {
		const anchorPos = allPlacedPositions.get(anchorId);
		if (!anchorPos) continue;

		const byDepth = new Map<number, string[]>();
		for (const pureId of pureIds) {
			const depth = computePureDepth(pureId);
			placedPureLevels.set(pureId, depth);
			const nodesAtDepth = byDepth.get(depth) ?? [];
			nodesAtDepth.push(pureId);
			byDepth.set(depth, nodesAtDepth);
		}

		for (const depth of [...byDepth.keys()].sort((a, b) => a - b)) {
			const x = anchorPos[0] - depth * cfg.pureHGap;
			const depthNodes = byDepth.get(depth) ?? [];
			const aboveNodes = depthNodes
				.filter((pureId) => getPureDirection(pureId, anchorPos[1], graph) < 0)
				.sort((a, b) => {
					const desiredA = computeDesiredPureY(a, anchorId);
					const desiredB = computeDesiredPureY(b, anchorId);
					return (
						desiredB - desiredA ||
						compareNodeIdsByPreferredPosition(a, b, graph)
					);
				});
			const belowNodes = depthNodes
				.filter((pureId) => getPureDirection(pureId, anchorPos[1], graph) > 0)
				.sort((a, b) => {
					const desiredA = computeDesiredPureY(a, anchorId);
					const desiredB = computeDesiredPureY(b, anchorId);
					return (
						desiredA - desiredB ||
						compareNodeIdsByPreferredPosition(a, b, graph)
					);
				});

			for (const pureId of [...aboveNodes, ...belowNodes]) {
				const direction = getPureDirection(pureId, anchorPos[1], graph);
				const desiredY = offsetPureYFromAnchor(
					computeDesiredPureY(pureId, anchorId),
					anchorPos[1],
					direction,
					cfg.pureVGap,
				);
				const [slotX, slotY] = findOpenPureSlot(
					allPlacedPositions,
					occupancy,
					x,
					desiredY,
					bucketWidth,
					bucketHeight,
					180,
					90,
					cfg.pureVGap,
					direction,
				);

				positions.set(pureId, [slotX, slotY]);
				allPlacedPositions.set(pureId, [slotX, slotY]);
				addToSpatialIndex(
					occupancy,
					pureId,
					slotX,
					slotY,
					bucketWidth,
					bucketHeight,
				);
			}
		}
	}

	if (orphans.length > 0) {
		let minX = Number.POSITIVE_INFINITY;
		let maxY = Number.NEGATIVE_INFINITY;
		for (const [, pos] of allPlacedPositions) {
			if (pos[0] < minX) minX = pos[0];
			if (pos[1] > maxY) maxY = pos[1];
		}
		if (minX === Number.POSITIVE_INFINITY) minX = 0;
		if (maxY === Number.NEGATIVE_INFINITY) maxY = 0;

		const orphanY = maxY + cfg.vGap;
		const orphanColumns = Math.max(1, Math.ceil(Math.sqrt(orphans.length)));
		for (let i = 0; i < orphans.length; i++) {
			if (positions.has(orphans[i])) continue;
			const column = i % orphanColumns;
			const row = Math.floor(i / orphanColumns);
			const x = minX + column * cfg.pureHGap;
			const y = orphanY + row * cfg.pureVGap;
			const [slotX, slotY] = findOpenPureSlot(
				allPlacedPositions,
				occupancy,
				x,
				y,
				bucketWidth,
				bucketHeight,
				180,
				90,
				cfg.pureVGap,
				1,
			);
			positions.set(orphans[i], [slotX, slotY]);
			allPlacedPositions.set(orphans[i], [slotX, slotY]);
			addToSpatialIndex(
				occupancy,
				orphans[i],
				slotX,
				slotY,
				bucketWidth,
				bucketHeight,
			);
		}
	}

	return positions;
}

// ─── Overlap Resolution ─────────────────────────────────────────────────────

function resolveOverlaps(
	positions: Map<string, [number, number]>,
	nodeHeights: Map<string, number>,
	nodeWidths: Map<string, number>,
	lockedNodeIds?: Set<string>,
): void {
	for (let pass = 0; pass < 3; pass++) {
		const entries = [...positions.entries()].sort(
			(a, b) => a[1][0] - b[1][0] || a[1][1] - b[1][1],
		);
		const comparedPairs = new Set<string>();
		const spatialIndex = buildSpatialIndex(positions, 260, 140);

		for (const [idA, posA] of entries) {
			for (const idB of getNearbySpatialIds(
				spatialIndex,
				posA[0],
				posA[1],
				260,
				140,
			)) {
				if (idA === idB) continue;
				const pairKey = idA < idB ? `${idA}|${idB}` : `${idB}|${idA}`;
				if (comparedPairs.has(pairKey)) continue;
				comparedPairs.add(pairKey);

				const posB = positions.get(idB);
				if (!posB) continue;

				const dx = Math.abs(posA[0] - posB[0]);
				const dy = Math.abs(posA[1] - posB[1]);
				const widthA = nodeWidths.get(idA) ?? DEFAULT_NODE_WIDTH;
				const widthB = nodeWidths.get(idB) ?? DEFAULT_NODE_WIDTH;
				const heightA = nodeHeights.get(idA) ?? DEFAULT_NODE_HEIGHT;
				const heightB = nodeHeights.get(idB) ?? DEFAULT_NODE_HEIGHT;
				const minH = Math.max(180, Math.min(widthA, widthB) - 24);
				const minV = Math.max(heightA, heightB) + 40;

				if (dx < minH && dy < minV) {
					const lockA = lockedNodeIds?.has(idA) ?? false;
					const lockB = lockedNodeIds?.has(idB) ?? false;
					if (lockA && lockB) continue;

					const shift = Math.max(24, minV - dy);
					if (!lockB) {
						posB[1] += posA[1] <= posB[1] ? shift : -shift;
						positions.set(idB, posB);
					} else if (!lockA) {
						posA[1] += posB[1] <= posA[1] ? shift : -shift;
						positions.set(idA, posA);
					}
				}
			}
		}
	}
}

function computeGroupBounds(
	positions: Map<string, [number, number]>,
	nodeIds: Iterable<string>,
	nodeHeights: Map<string, number>,
	nodeWidths: Map<string, number>,
): GroupBounds {
	let minX = Number.POSITIVE_INFINITY;
	let minY = Number.POSITIVE_INFINITY;
	let maxX = Number.NEGATIVE_INFINITY;
	let maxY = Number.NEGATIVE_INFINITY;

	for (const nodeId of nodeIds) {
		const pos = positions.get(nodeId);
		if (!pos) continue;
		const height = nodeHeights.get(nodeId) ?? 100;
		const width = nodeWidths.get(nodeId) ?? DEFAULT_NODE_WIDTH;
		minX = Math.min(minX, pos[0]);
		minY = Math.min(minY, pos[1]);
		maxX = Math.max(maxX, pos[0] + width);
		maxY = Math.max(maxY, pos[1] + height);
	}

	if (minX === Number.POSITIVE_INFINITY) {
		return { minX: 0, minY: 0, maxX: 0, maxY: 0 };
	}

	return { minX, minY, maxX, maxY };
}

function translateGroupLayout(
	layout: GroupLayout,
	dx: number,
	dy: number,
): GroupLayout {
	const positions = new Map<string, [number, number]>();
	for (const [nodeId, pos] of layout.positions) {
		positions.set(nodeId, [pos[0] + dx, pos[1] + dy]);
	}

	return {
		positions,
		bounds: {
			minX: layout.bounds.minX + dx,
			minY: layout.bounds.minY + dy,
			maxX: layout.bounds.maxX + dx,
			maxY: layout.bounds.maxY + dy,
		},
	};
}

function buildGroupLayout(
	group: EventGroup,
	graph: Map<string, LayoutNode>,
	nodeHeights: Map<string, number>,
	nodeWidths: Map<string, number>,
	cfg: StyleConfig,
): GroupLayout {
	const { execPositions, inlinePurePositions, placedPureIds } = layoutExecChain(
		group,
		graph,
		0,
		0,
		cfg,
	);

	const positions = new Map<string, [number, number]>();
	for (const [nodeId, pos] of execPositions) {
		positions.set(nodeId, pos);
	}
	for (const [nodeId, pos] of inlinePurePositions) {
		positions.set(nodeId, pos);
	}

	const remainingPurePositions = layoutRemainingPures(
		group,
		graph,
		positions,
		placedPureIds,
		cfg,
	);
	for (const [nodeId, pos] of remainingPurePositions) {
		positions.set(nodeId, pos);
	}

	resolveOverlaps(positions, nodeHeights, nodeWidths, new Set(group.execChain));

	return {
		positions,
		bounds: computeGroupBounds(
			positions,
			group.allNodes,
			nodeHeights,
			nodeWidths,
		),
	};
}

function buildFnRefEdges(
	eventGroups: EventGroup[],
	graph: Map<string, LayoutNode>,
	fnRefNodeToEntity: Map<string, string>,
): FnRefEdge[] {
	const edges: FnRefEdge[] = [];
	const groupByStartId = new Map<string, EventGroup>();
	for (const group of eventGroups) {
		groupByStartId.set(group.startNodeId, group);
	}

	const seen = new Set<string>();
	for (const group of eventGroups) {
		for (const nodeId of group.allNodes) {
			const node = graph.get(nodeId);
			if (!node) continue;

			for (const targetId of node.fnRefTargets) {
				const resolvedId = resolveFnRefTargetId(
					targetId,
					graph,
					fnRefNodeToEntity,
				);
				if (!resolvedId || resolvedId === group.startNodeId) continue;
				if (!groupByStartId.has(resolvedId)) continue;

				const key = `${group.startNodeId}:${nodeId}:${resolvedId}`;
				if (seen.has(key)) continue;
				seen.add(key);

				edges.push({
					fromGroupId: group.startNodeId,
					fromNodeId: nodeId,
					toGroupId: resolvedId,
				});
			}
		}
	}

	return edges;
}

// ─── Main Entry ──────────────────────────────────────────────────────────────

export function computeFlowLayout(
	input: AutoLayoutInput,
	style: LayoutStyle = "compact",
): Map<string, [number, number]> {
	const { layerNodes, layerEntities, boardLayers } = input;
	const cfg = getStyleConfig(style);

	const allNodes = [
		...layerNodes,
		...layerEntities.map(
			(e) =>
				({
					id: e.id,
					coordinates: e.coordinates,
					pins: boardLayers?.[e.id]?.pins ?? {},
					start: false,
					event_callback: false,
					fn_refs: null,
					category: "",
					description: "",
					friendly_name: "",
					name: boardLayers?.[e.id]?.name ?? e.id,
				}) as unknown as INode,
		),
	];

	const pinOwner = buildPinOwnerMap(layerNodes, layerEntities, boardLayers);
	const graph = buildLayoutGraph(allNodes, layerEntities, pinOwner);
	const fnRefNodeToEntity = buildFnRefNodeToEntityMap(
		layerEntities,
		boardLayers,
	);

	const eventGroups = discoverEventGroups(graph, fnRefNodeToEntity);
	const fnRefEdges = buildFnRefEdges(eventGroups, graph, fnRefNodeToEntity);

	const nodeHeights = new Map<string, number>();
	const nodeWidths = new Map<string, number>();
	const layerEntityIds = new Set(layerEntities.map((entity) => entity.id));
	for (const node of allNodes) {
		nodeHeights.set(node.id, estimateNodeHeight(node));
		nodeWidths.set(
			node.id,
			estimateNodeWidth(node, layerEntityIds.has(node.id)),
		);
	}

	const allPositions = new Map<string, [number, number]>();
	const groupById = new Map<string, EventGroup>();
	const baseLayouts = new Map<string, GroupLayout>();
	for (const group of eventGroups) {
		groupById.set(group.startNodeId, group);
		baseLayouts.set(
			group.startNodeId,
			buildGroupLayout(group, graph, nodeHeights, nodeWidths, cfg),
		);
	}

	const childrenByGroup = new Map<string, Map<string, FnRefEdge[]>>();
	const incomingCounts = new Map<string, number>();
	for (const group of eventGroups) {
		incomingCounts.set(group.startNodeId, 0);
	}
	for (const edge of fnRefEdges) {
		incomingCounts.set(
			edge.toGroupId,
			(incomingCounts.get(edge.toGroupId) ?? 0) + 1,
		);
		const byNode =
			childrenByGroup.get(edge.fromGroupId) ?? new Map<string, FnRefEdge[]>();
		const edges = byNode.get(edge.fromNodeId) ?? [];
		edges.push(edge);
		byNode.set(edge.fromNodeId, edges);
		childrenByGroup.set(edge.fromGroupId, byNode);
	}

	const placedGroups = new Set<string>();
	const activeGroups = new Set<string>();

	function placeGroupSubtree(
		groupId: string,
		startX: number,
		startY: number,
	): GroupBounds {
		const baseLayout = baseLayouts.get(groupId);
		if (!baseLayout) {
			return { minX: startX, minY: startY, maxX: startX, maxY: startY };
		}
		if (placedGroups.has(groupId)) {
			const placedBounds = computeGroupBounds(
				allPositions,
				groupById.get(groupId)?.allNodes ?? [],
				nodeHeights,
				nodeWidths,
			);
			return placedBounds;
		}
		if (activeGroups.has(groupId)) {
			return { minX: startX, minY: startY, maxX: startX, maxY: startY };
		}

		activeGroups.add(groupId);

		const rootPos =
			baseLayout.positions.get(groupId) ??
			([baseLayout.bounds.minX, baseLayout.bounds.minY] as [number, number]);

		const translated = translateGroupLayout(
			baseLayout,
			startX - rootPos[0],
			startY - rootPos[1],
		);
		for (const [nodeId, pos] of translated.positions) {
			allPositions.set(nodeId, pos);
		}

		let subtreeBounds = translated.bounds;
		const nodeGroups = childrenByGroup.get(groupId);
		const parentBottomY = translated.bounds.maxY;

		if (nodeGroups) {
			for (const [fromNodeId, edges] of [...nodeGroups.entries()].sort(
				(a, b) => {
					const posA = translated.positions.get(a[0]) ?? [0, 0];
					const posB = translated.positions.get(b[0]) ?? [0, 0];
					return (
						posA[1] - posB[1] || posA[0] - posB[0] || a[0].localeCompare(b[0])
					);
				},
			)) {
				const anchorPos = translated.positions.get(fromNodeId);
				if (!anchorPos) continue;

				let threadX = anchorPos[0];
				const threadY = parentBottomY + cfg.eventGroupGap;
				for (const edge of edges.sort((a, b) =>
					compareNodeIdsByPreferredPosition(a.toGroupId, b.toGroupId, graph),
				)) {
					const childBounds = placeGroupSubtree(
						edge.toGroupId,
						threadX,
						threadY,
					);
					subtreeBounds = {
						minX: Math.min(subtreeBounds.minX, childBounds.minX),
						minY: Math.min(subtreeBounds.minY, childBounds.minY),
						maxX: Math.max(subtreeBounds.maxX, childBounds.maxX),
						maxY: Math.max(subtreeBounds.maxY, childBounds.maxY),
					};
					threadX = childBounds.maxX + cfg.hGap;
				}
			}
		}

		activeGroups.delete(groupId);
		placedGroups.add(groupId);
		return subtreeBounds;
	}

	let currentY = 0;
	for (const group of eventGroups) {
		if ((incomingCounts.get(group.startNodeId) ?? 0) > 0) continue;
		if (placedGroups.has(group.startNodeId)) continue;

		const bounds = placeGroupSubtree(group.startNodeId, 0, currentY);
		currentY = bounds.maxY + cfg.eventGroupGap;
	}

	for (const group of eventGroups) {
		if (placedGroups.has(group.startNodeId)) continue;
		const bounds = placeGroupSubtree(group.startNodeId, 0, currentY);
		currentY = bounds.maxY + cfg.eventGroupGap;
	}

	return allPositions;
}
