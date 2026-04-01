import type { INode, IPin } from "./schema/flow/node";
import { IPinType, IVariableType } from "./schema/flow/node";
import type { ILayer } from "./schema/flow/board/commands/upsert-layer";

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
	outgoingExec: Map<string, string>;
	outgoingData: Map<string, string[]>;
	incomingExec: Set<string>;
	incomingData: Set<string>;
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
			return { hGap: 320, vGap: 160, pureHGap: 280, pureVGap: 130, eventGroupGap: 300, branchSpread: 1 };
		case "expanded":
			return { hGap: 450, vGap: 240, pureHGap: 380, pureVGap: 200, eventGroupGap: 500, branchSpread: 1.5 };
		case "balanced":
		default:
			return { hGap: 380, vGap: 190, pureHGap: 320, pureVGap: 160, eventGroupGap: 380, branchSpread: 1.2 };
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

function estimateNodeHeight(node: INode): number {
	const pins = Object.values(node.pins);
	const inputCount = pins.filter(isInputPin).length;
	const outputCount = pins.filter(isOutputPin).length;
	return Math.max(inputCount, outputCount) * 15 + 28;
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
		});
	}

	for (const node of nodes) {
		for (const pin of Object.values(node.pins)) {
			if (!isOutputPin(pin)) continue;
			for (const targetPinId of pin.connected_to) {
				const targetId = pinOwner.get(targetPinId);
				if (!targetId || !allIds.has(targetId) || targetId === node.id) continue;

				const layoutNode = graph.get(node.id)!;
				const targetLayout = graph.get(targetId)!;

				if (isExecPin(pin)) {
					layoutNode.outgoingExec.set(pin.id, targetId);
					targetLayout.incomingExec.add(node.id);
				} else {
					const existing = layoutNode.outgoingData.get(pin.id) ?? [];
					existing.push(targetId);
					layoutNode.outgoingData.set(pin.id, existing);
					targetLayout.incomingData.add(node.id);
				}
			}
		}
	}

	return graph;
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
		collectPureDepsForGroup(depId, graph, pureNodes, allNodes, blockedNodes, rootId);
	}

	for (const targets of node.outgoingData.values()) {
		for (const targetId of targets) {
			if (allNodes.has(targetId)) continue;
			if (targetId !== rootId && blockedNodes.has(targetId)) continue;
			const target = graph.get(targetId);
			if (!target || target.isImpure) continue;

			pureNodes.add(targetId);
			allNodes.add(targetId);
			collectPureDepsForGroup(targetId, graph, pureNodes, allNodes, blockedNodes, rootId);
		}
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
		const nodeId = execQueue.shift()!;
		if (execVisited.has(nodeId)) continue;
		if (nodeId !== startNodeId && blockedNodes.has(nodeId)) continue;

		execVisited.add(nodeId);
		execChain.push(nodeId);
		allNodes.add(nodeId);

		const node = graph.get(nodeId);
		if (!node) continue;
		for (const targetId of node.outgoingExec.values()) {
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

	const startRootIds = [...graph.values()]
		.filter((n) => n.isStart || n.isEventCallback)
		.sort((a, b) => {
			if (a.isStart && !b.isStart) return -1;
			if (!a.isStart && b.isStart) return 1;
			return 0;
		})
		.map((node) => node.id);

	const startRootSet = new Set(startRootIds);
	const fnRefRootIds: string[] = [];
	const fnRefRootSeen = new Set<string>();

	for (const node of graph.values()) {
		for (const targetId of node.fnRefTargets) {
			const resolvedId = resolveFnRefTargetId(targetId, graph, fnRefNodeToEntity);
			if (!resolvedId || startRootSet.has(resolvedId) || fnRefRootSeen.has(resolvedId)) {
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
		const impure = unclaimed.filter((id) => graph.get(id)!.isImpure);

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
): number {
	const source = graph.get(sourceId);
	const sink = graph.get(sinkId);
	if (!source || !sink) return 0;

	const depths = new Map<string, number>();
	const queue: [string, number][] = [];

	for (const targets of source.outgoingData.values()) {
		for (const tid of targets) {
			if (pureNodes.has(tid)) queue.push([tid, 1]);
		}
	}

	while (queue.length > 0) {
		const [nodeId, depth] = queue.shift()!;
		if (depths.has(nodeId) && depths.get(nodeId)! >= depth) continue;
		depths.set(nodeId, depth);

		const node = graph.get(nodeId)!;
		for (const targets of node.outgoingData.values()) {
			for (const tid of targets) {
				if (pureNodes.has(tid)) queue.push([tid, depth + 1]);
			}
		}
	}

	let maxDepth = 0;
	for (const depId of sink.incomingData) {
		const d = depths.get(depId);
		if (d !== undefined) maxDepth = Math.max(maxDepth, d);
	}

	return maxDepth;
}

// ─── Execution Chain Layout ──────────────────────────────────────────────────

interface ExecLayoutResult {
	execPositions: Map<string, [number, number]>;
	inlinePurePositions: Map<string, [number, number]>;
	placedPureIds: Set<string>;
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
	const execColumns = new Map<string, number>();
	const groupExecIds = new Set(group.execChain);

	if (group.execChain.length === 0) {
		return { execPositions, inlinePurePositions, placedPureIds };
	}

	const placed = new Set<string>();
	const queue: { nodeId: string; col: number; row: number }[] = [
		{ nodeId: group.startNodeId, col: 0, row: 0 },
	];

	while (queue.length > 0) {
		const { nodeId, col, row } = queue.shift()!;
		if (placed.has(nodeId)) continue;
		if (!groupExecIds.has(nodeId)) continue;
		placed.add(nodeId);

		const actualRow = row;
		execPositions.set(nodeId, [
			startX + col * cfg.hGap,
			startY + actualRow * cfg.vGap,
		]);
		execColumns.set(nodeId, col);

		const node = graph.get(nodeId)!;
		const targets = [...node.outgoingExec.values()].filter(
			(t) => groupExecIds.has(t) && !placed.has(t),
		);

		if (targets.length === 1) {
			const gap = computePureChainGap(
				nodeId,
				targets[0],
				graph,
				group.pureNodes,
			);
			queue.push({
				nodeId: targets[0],
				col: col + gap + 1,
				row: actualRow,
			});
		} else if (targets.length > 1) {
			const spread = cfg.branchSpread;
			const half = ((targets.length - 1) * spread) / 2;
			for (let i = 0; i < targets.length; i++) {
				const gap = computePureChainGap(
					nodeId,
					targets[i],
					graph,
					group.pureNodes,
				);
				const branchRow = actualRow + Math.round(i * spread - half);
				queue.push({
					nodeId: targets[i],
					col: col + gap + 1,
					row: branchRow,
				});
			}
		}
	}

	// Place inline pure nodes in the gap columns between exec nodes
	for (const [execId] of execPositions) {
		const execNode = graph.get(execId)!;
		const execCol = execColumns.get(execId)!;
		const execPos = execPositions.get(execId)!;

		const pureQueue: [string, number][] = [];
		for (const targets of execNode.outgoingData.values()) {
			for (const tid of targets) {
				if (group.pureNodes.has(tid) && !placedPureIds.has(tid)) {
					pureQueue.push([tid, 1]);
				}
			}
		}

		const colPureNodes = new Map<number, string[]>();
		const visited = new Set<string>();

		while (pureQueue.length > 0) {
			const [nodeId, depth] = pureQueue.shift()!;
			if (visited.has(nodeId) || placedPureIds.has(nodeId)) continue;
			visited.add(nodeId);
			placedPureIds.add(nodeId);

			const targetCol = execCol + depth;
			const list = colPureNodes.get(targetCol) ?? [];
			list.push(nodeId);
			colPureNodes.set(targetCol, list);

			const node = graph.get(nodeId)!;
			for (const targets of node.outgoingData.values()) {
				for (const tid of targets) {
					if (
						group.pureNodes.has(tid) &&
						!visited.has(tid) &&
						!placedPureIds.has(tid)
					) {
						pureQueue.push([tid, depth + 1]);
					}
				}
			}
		}

		for (const [col, nodeIds] of colPureNodes) {
			const x = startX + col * cfg.hGap;
			if (nodeIds.length === 1) {
				inlinePurePositions.set(nodeIds[0], [x, execPos[1]]);
			} else {
				for (let i = 0; i < nodeIds.length; i++) {
					const direction = i % 2 === 0 ? -1 : 1;
					const tier = Math.floor(i / 2) + 1;
					const y = execPos[1] + direction * tier * cfg.pureVGap;
					inlinePurePositions.set(nodeIds[i], [x, y]);
				}
			}
		}
	}

	return { execPositions, inlinePurePositions, placedPureIds };
}

// ─── Remaining Pure Node Layout ──────────────────────────────────────────────

function layoutRemainingPures(
	group: EventGroup,
	graph: Map<string, LayoutNode>,
	allPlacedPositions: Map<string, [number, number]>,
	skip: Set<string>,
	cfg: StyleConfig,
): Map<string, [number, number]> {
	const positions = new Map<string, [number, number]>();
	const remaining = [...group.pureNodes].filter((id) => !skip.has(id));
	if (remaining.length === 0) return positions;

	const pureToAnchor = new Map<string, string>();

	for (const pureId of remaining) {
		const pureNode = graph.get(pureId);
		if (!pureNode) continue;

		for (const targets of pureNode.outgoingData.values()) {
			for (const tid of targets) {
				if (allPlacedPositions.has(tid)) {
					pureToAnchor.set(pureId, tid);
					break;
				}
			}
			if (pureToAnchor.has(pureId)) break;
		}
		if (pureToAnchor.has(pureId)) continue;

		for (const depId of pureNode.incomingData) {
			if (allPlacedPositions.has(depId)) {
				pureToAnchor.set(pureId, depId);
				break;
			}
		}
	}

	// Second pass: link to already-anchored pures
	for (const pureId of remaining) {
		if (pureToAnchor.has(pureId)) continue;
		const pureNode = graph.get(pureId);
		if (!pureNode) continue;
		for (const targets of pureNode.outgoingData.values()) {
			for (const tid of targets) {
				if (pureToAnchor.has(tid)) {
					pureToAnchor.set(pureId, pureToAnchor.get(tid)!);
					break;
				}
			}
			if (pureToAnchor.has(pureId)) break;
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

	for (const [anchorId, pureIds] of byAnchor) {
		const anchorPos = allPlacedPositions.get(anchorId);
		if (!anchorPos) continue;

		let slotIdx = 0;
		for (const pureId of pureIds) {
			const direction = slotIdx % 2 === 0 ? -1 : 1;
			const tier = Math.floor(slotIdx / 2) + 1;
			const x = anchorPos[0] - cfg.pureHGap;
			let y = anchorPos[1] + direction * tier * cfg.pureVGap;

			for (const [, placed] of allPlacedPositions) {
				if (Math.abs(placed[0] - x) < 200 && Math.abs(placed[1] - y) < 80) {
					y += direction * 80;
				}
			}

			positions.set(pureId, [x, y]);
			allPlacedPositions.set(pureId, [x, y]);
			slotIdx++;
		}
	}

	if (orphans.length > 0) {
		let maxY = -Infinity;
		for (const [, pos] of allPlacedPositions) {
			if (pos[1] > maxY) maxY = pos[1];
		}
		if (maxY === -Infinity) maxY = 0;

		const orphanY = maxY + cfg.vGap;
		for (let i = 0; i < orphans.length; i++) {
			if (positions.has(orphans[i])) continue;
			positions.set(orphans[i], [i * cfg.pureHGap, orphanY]);
			allPlacedPositions.set(orphans[i], [i * cfg.pureHGap, orphanY]);
		}
	}

	return positions;
}

// ─── Overlap Resolution ─────────────────────────────────────────────────────

function resolveOverlaps(
	positions: Map<string, [number, number]>,
	nodeHeights: Map<string, number>,
	lockedNodeIds?: Set<string>,
): void {
	const entries = [...positions.entries()].sort(
		(a, b) => a[1][0] - b[1][0] || a[1][1] - b[1][1],
	);

	for (let pass = 0; pass < 3; pass++) {
		for (let i = 0; i < entries.length; i++) {
			for (let j = i + 1; j < entries.length; j++) {
				const posA = entries[i][1];
				const posB = entries[j][1];

				const dx = Math.abs(posA[0] - posB[0]);
				const dy = Math.abs(posA[1] - posB[1]);

				const heightA = nodeHeights.get(entries[i][0]) ?? 100;
				const heightB = nodeHeights.get(entries[j][0]) ?? 100;
				const minV = Math.max(heightA, heightB) + 40;

				if (dx < 220 && dy < minV) {
					const idA = entries[i][0];
					const idB = entries[j][0];
					const lockA = lockedNodeIds?.has(idA) ?? false;
					const lockB = lockedNodeIds?.has(idB) ?? false;
					if (lockA && lockB) continue;

					const shift = minV - dy;
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
): GroupBounds {
	let minX = Infinity;
	let minY = Infinity;
	let maxX = -Infinity;
	let maxY = -Infinity;

	for (const nodeId of nodeIds) {
		const pos = positions.get(nodeId);
		if (!pos) continue;
		const height = nodeHeights.get(nodeId) ?? 100;
		minX = Math.min(minX, pos[0]);
		minY = Math.min(minY, pos[1]);
		maxX = Math.max(maxX, pos[0]);
		maxY = Math.max(maxY, pos[1] + height);
	}

	if (minX === Infinity) {
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

	resolveOverlaps(positions, nodeHeights, new Set(group.execChain));

	return {
		positions,
		bounds: computeGroupBounds(positions, group.allNodes, nodeHeights),
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
				const resolvedId = resolveFnRefTargetId(targetId, graph, fnRefNodeToEntity);
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
					pins: boardLayers?.[e.id]?.pins ?? {},
					start: false,
					event_callback: false,
					fn_refs: null,
					category: "",
					description: "",
					friendly_name: "",
					name: "",
				}) as unknown as INode,
		),
	];

	const pinOwner = buildPinOwnerMap(layerNodes, layerEntities, boardLayers);
	const graph = buildLayoutGraph(allNodes, layerEntities, pinOwner);

	// Map node IDs inside child layers to their parent layer entity ID
	const fnRefNodeToEntity = new Map<string, string>();
	if (boardLayers) {
		for (const entity of layerEntities) {
			const layer = boardLayers[entity.id];
			if (!layer?.nodes) continue;
			for (const nodeId of Object.keys(layer.nodes)) {
				fnRefNodeToEntity.set(nodeId, entity.id);
			}
		}
	}

	const eventGroups = discoverEventGroups(graph, fnRefNodeToEntity);
	const fnRefEdges = buildFnRefEdges(eventGroups, graph, fnRefNodeToEntity);

	const nodeHeights = new Map<string, number>();
	for (const node of layerNodes) {
		nodeHeights.set(node.id, estimateNodeHeight(node));
	}

	const allPositions = new Map<string, [number, number]>();
	const groupById = new Map<string, EventGroup>();
	const baseLayouts = new Map<string, GroupLayout>();
	for (const group of eventGroups) {
		groupById.set(group.startNodeId, group);
		baseLayouts.set(group.startNodeId, buildGroupLayout(group, graph, nodeHeights, cfg));
	}

	const childrenByGroup = new Map<string, Map<string, FnRefEdge[]>>();
	const incomingCounts = new Map<string, number>();
	for (const group of eventGroups) {
		incomingCounts.set(group.startNodeId, 0);
	}
	for (const edge of fnRefEdges) {
		incomingCounts.set(edge.toGroupId, (incomingCounts.get(edge.toGroupId) ?? 0) + 1);
		const byNode = childrenByGroup.get(edge.fromGroupId) ?? new Map<string, FnRefEdge[]>();
		const edges = byNode.get(edge.fromNodeId) ?? [];
		edges.push(edge);
		byNode.set(edge.fromNodeId, edges);
		childrenByGroup.set(edge.fromGroupId, byNode);
	}

	const placedGroups = new Set<string>();
	const activeGroups = new Set<string>();

	function placeGroupSubtree(groupId: string, startX: number, startY: number): GroupBounds {
		const baseLayout = baseLayouts.get(groupId);
		if (!baseLayout) {
			return { minX: startX, minY: startY, maxX: startX, maxY: startY };
		}
		if (placedGroups.has(groupId)) {
			const placedBounds = computeGroupBounds(allPositions, groupById.get(groupId)?.allNodes ?? [], nodeHeights);
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
			for (const [fromNodeId, edges] of nodeGroups) {
				const anchorPos = translated.positions.get(fromNodeId);
				if (!anchorPos) continue;

				let threadX = anchorPos[0];
				const threadY = parentBottomY + cfg.eventGroupGap;
				for (const edge of edges) {
					const childBounds = placeGroupSubtree(edge.toGroupId, threadX, threadY);
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
