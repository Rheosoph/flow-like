import {
	breakCycles,
	buildFnRefNodeToEntityMap,
	buildLayoutGraph,
	findComponents,
} from "./build";
import { assignOrder } from "./order";
import { bindComments, packComponents, resolveCommentPositions } from "./pack";
import { componentBounds, placeComponent } from "./place";
import {
	assignColumnX,
	assignColumns,
	assignOwners,
	normaliseColumns,
	topologicalOrder,
} from "./rank";
import {
	type AutoLayoutInput,
	type LGraph,
	type LayoutResult,
	type LayoutStyle,
	type StyleConfig,
	getStyleConfig,
} from "./types";

function collectOriginalPositions(
	input: AutoLayoutInput,
): Map<string, [number, number]> {
	const positions = new Map<string, [number, number]>();
	for (const node of input.layerNodes) {
		const coordinates = node.coordinates ?? [0, 0, 0];
		positions.set(node.id, [coordinates[0] ?? 0, coordinates[1] ?? 0]);
	}
	for (const entity of input.layerEntities) {
		positions.set(entity.id, [
			entity.coordinates[0] ?? 0,
			entity.coordinates[1] ?? 0,
		]);
	}
	return positions;
}

/**
 * The only place old coordinates are read. Because it is a rigid translation of
 * the whole result it cannot change relative geometry, which is what keeps
 * `layout(layout(G)) === layout(G)` true: on a second run the translation is
 * zero.
 */
function anchorTranslation(
	graph: LGraph,
	input: AutoLayoutInput,
	original: ReadonlyMap<string, [number, number]>,
): [number, number] {
	const ids = graph.order;
	if (ids.length === 0) return [0, 0];

	const bounds = componentBounds(graph, ids);

	if (input.only && input.only.size > 0) {
		// Both sides must be the SAME functional of the positions, otherwise the
		// translation is not self-cancelling and every re-run walks the selection
		// further across the canvas. Mean of top-left corners on both sides.
		let sumOldX = 0;
		let sumOldY = 0;
		let sumNewX = 0;
		let sumNewY = 0;
		let count = 0;
		for (const id of ids) {
			const position = original.get(id);
			const node = graph.nodes.get(id);
			if (!position || !node) continue;
			sumOldX += position[0];
			sumOldY += position[1];
			sumNewX += node.x;
			sumNewY += node.y;
			count += 1;
		}
		if (count > 0) {
			return [
				Math.round((sumOldX - sumNewX) / count),
				Math.round((sumOldY - sumNewY) / count),
			];
		}
	}

	// Pin the top-most event so the thing the user was looking at stays put.
	const startIds = ids
		.filter((id) => graph.nodes.get(id)?.isStart)
		.sort((a, b) => {
			const [ax, ay] = original.get(a) ?? [0, 0];
			const [bx, by] = original.get(b) ?? [0, 0];
			return ay - by || ax - bx || a.localeCompare(b);
		});
	const anchorId = startIds[0];
	if (anchorId) {
		const node = graph.nodes.get(anchorId);
		const position = original.get(anchorId);
		if (node && position) {
			return [position[0] - node.x, position[1] - node.y];
		}
	}

	let minOldX = Number.POSITIVE_INFINITY;
	let minOldY = Number.POSITIVE_INFINITY;
	for (const id of ids) {
		const position = original.get(id);
		if (!position) continue;
		minOldX = Math.min(minOldX, position[0]);
		minOldY = Math.min(minOldY, position[1]);
	}
	if (!Number.isFinite(minOldX)) return [-bounds.minX, -bounds.minY];
	return [minOldX - bounds.minX, minOldY - bounds.minY];
}

/**
 * A scoped layout only knows about the nodes it was given, so its result can
 * land on top of everything the user did not select. Sliding the whole result
 * down until it clears is still a rigid translation, so idempotency survives:
 * on a second run the result already clears and nothing is added.
 */
function clearObstacles(
	graph: LGraph,
	input: AutoLayoutInput,
	dx: number,
	dy: number,
	cfg: StyleConfig,
): number {
	const obstacles = input.obstacles;
	if (!obstacles || obstacles.length === 0 || graph.order.length === 0)
		return 0;

	const bounds = componentBounds(graph, graph.order);
	const minX = bounds.minX + dx;
	const maxX = bounds.maxX + dx;
	const sorted = [...obstacles].sort((a, b) => a.y - b.y || a.x - b.x);

	let extra = 0;
	for (let guard = 0; guard < obstacles.length + 1; guard++) {
		let pushed = false;
		for (const box of sorted) {
			const top = bounds.minY + dy + extra;
			const bottom = bounds.maxY + dy + extra;
			if (
				minX < box.x + box.width &&
				maxX > box.x &&
				top < box.y + box.height &&
				bottom > box.y
			) {
				extra = box.y + box.height + cfg.componentGap - (bounds.minY + dy);
				pushed = true;
			}
		}
		if (!pushed) break;
	}

	return Math.round(extra);
}

export function computeFlowLayoutDetailed(
	input: AutoLayoutInput,
	style: LayoutStyle = "compact",
): LayoutResult {
	const cfg = getStyleConfig(style);
	const graph = buildLayoutGraph(input);

	const components = findComponents(graph);
	breakCycles(graph, components);

	const topo = topologicalOrder(graph);
	assignColumns(graph, topo);
	assignOwners(graph, topo);
	normaliseColumns(graph, components);
	assignOrder(graph, components);

	for (const component of components) {
		assignColumnX(graph, component, cfg);
		placeComponent(graph, component, cfg);
	}

	const original = collectOriginalPositions(input);
	packComponents(
		graph,
		components,
		cfg,
		buildFnRefNodeToEntityMap(input.layerEntities, input.boardLayers),
		original,
	);

	// Snap to integers BEFORE anchoring. Measured sizes can be fractional, and
	// an anchor computed from fractional coordinates would round differently on
	// a re-run — the translation stays rigid but stops being a fixed point.
	for (const id of graph.order) {
		const node = graph.nodes.get(id);
		if (!node) continue;
		node.x = Math.round(node.x);
		node.y = Math.round(node.y);
	}

	const [dx, anchorY] = anchorTranslation(graph, input, original);
	const dy = anchorY + clearObstacles(graph, input, dx, anchorY, cfg);

	const positions = new Map<string, [number, number]>();
	const unplaced: string[] = [];
	for (const id of graph.order) {
		const node = graph.nodes.get(id);
		if (!node) {
			unplaced.push(id);
			continue;
		}
		positions.set(id, [node.x + dx, node.y + dy]);
	}

	const sizes = new Map<string, readonly [number, number]>();
	for (const id of graph.order) {
		const node = graph.nodes.get(id);
		if (node) sizes.set(id, [node.width, node.height]);
	}

	const commentPositions = resolveCommentPositions(
		bindComments(input.comments ?? [], original, sizes),
		positions,
	);

	const columns = new Map<string, number>();
	const orders = new Map<string, number>();
	const owners = new Map<string, string>();
	for (const id of graph.order) {
		const node = graph.nodes.get(id);
		if (!node) continue;
		columns.set(id, node.column);
		orders.set(id, node.order);
		if (node.owner) owners.set(id, node.owner);
	}

	return {
		positions,
		commentPositions,
		reversedEdges: graph.edges
			.filter((edge) => edge.reversed)
			.map((edge) => ({ from: edge.from, to: edge.to })),
		diagnostics: {
			components: components.map((component) => ({
				id: component.id,
				nodeIds: component.nodeIds,
				roots: component.roots,
			})),
			columns,
			orders,
			owners,
			unplaced,
		},
	};
}

export function computeFlowLayout(
	input: AutoLayoutInput,
	style: LayoutStyle = "compact",
): Map<string, [number, number]> {
	return computeFlowLayoutDetailed(input, style).positions;
}
