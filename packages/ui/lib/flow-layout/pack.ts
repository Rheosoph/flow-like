import type { ComponentInfo } from "./build";
import { type Bounds, componentBounds, translateNodes } from "./place";
import type { LGraph, LayoutComment, StyleConfig } from "./types";

function unionBounds(a: Bounds, b: Bounds): Bounds {
	return {
		minX: Math.min(a.minX, b.minX),
		minY: Math.min(a.minY, b.minY),
		maxX: Math.max(a.maxX, b.maxX),
		maxY: Math.max(a.maxY, b.maxY),
	};
}

/**
 * Component-level `fn_ref` edges. These never merge components — a function
 * call is a placement relation, not connectivity — they only decide which
 * component is drawn below which.
 */
function buildFnRefForest(
	graph: LGraph,
	components: ComponentInfo[],
	fnRefNodeToEntity: Map<string, string>,
): {
	childrenOf: Map<number, Array<{ fromNodeId: string; child: number }>>;
	incoming: Map<number, number>;
} {
	const componentOf = new Map<string, number>();
	for (const component of components) {
		for (const id of component.nodeIds) componentOf.set(id, component.id);
	}

	const childrenOf = new Map<
		number,
		Array<{ fromNodeId: string; child: number }>
	>();
	const incoming = new Map<number, number>();
	for (const component of components) incoming.set(component.id, 0);

	const seen = new Set<string>();
	for (const id of graph.order) {
		const node = graph.nodes.get(id);
		if (!node || node.fnRefTargets.length === 0) continue;
		const parent = componentOf.get(id);
		if (parent === undefined) continue;

		for (const rawTarget of node.fnRefTargets) {
			const targetId = graph.nodes.has(rawTarget)
				? rawTarget
				: fnRefNodeToEntity.get(rawTarget);
			if (!targetId) continue;
			const child = componentOf.get(targetId);
			if (child === undefined || child === parent) continue;

			const key = `${parent}:${id}:${child}`;
			if (seen.has(key)) continue;
			seen.add(key);

			const children = childrenOf.get(parent) ?? [];
			children.push({ fromNodeId: id, child });
			childrenOf.set(parent, children);
			incoming.set(child, (incoming.get(child) ?? 0) + 1);
		}
	}

	return { childrenOf, incoming };
}

/**
 * Packs components. Each is positioned by its bounding box rather than by its
 * root node, so a component whose fan-out rises above its root can no longer
 * land on top of the component above it.
 */
export function packComponents(
	graph: LGraph,
	components: ComponentInfo[],
	cfg: StyleConfig,
	fnRefNodeToEntity: Map<string, string>,
	originalPositions: ReadonlyMap<string, readonly [number, number]>,
): void {
	if (components.length === 0) return;

	const { childrenOf, incoming } = buildFnRefForest(
		graph,
		components,
		fnRefNodeToEntity,
	);
	const byId = new Map(
		components.map((component) => [component.id, component]),
	);
	const placed = new Set<number>();
	const active = new Set<number>();

	const placeSubtree = (
		componentId: number,
		originX: number,
		originY: number,
	): Bounds => {
		const component = byId.get(componentId);
		if (!component || placed.has(componentId) || active.has(componentId)) {
			return { minX: originX, minY: originY, maxX: originX, maxY: originY };
		}
		active.add(componentId);

		const local = componentBounds(graph, component.nodeIds);
		translateNodes(
			graph,
			component.nodeIds,
			originX - local.minX,
			originY - local.minY,
		);

		let bounds = componentBounds(graph, component.nodeIds);
		const children = (childrenOf.get(componentId) ?? [])
			.slice()
			.sort((a, b) => {
				const nodeA = graph.nodes.get(a.fromNodeId);
				const nodeB = graph.nodes.get(b.fromNodeId);
				return (
					(nodeA?.y ?? 0) - (nodeB?.y ?? 0) ||
					(nodeA?.x ?? 0) - (nodeB?.x ?? 0) ||
					a.fromNodeId.localeCompare(b.fromNodeId) ||
					a.child - b.child
				);
			});

		// One thread advancing across ALL children of this component. Keeping the
		// cursor outside the loop is what stops two call sites from stacking their
		// callees at identical coordinates.
		let threadX = bounds.minX;
		const threadY = bounds.maxY + cfg.componentGap;
		for (const { fromNodeId, child } of children) {
			if (placed.has(child) || active.has(child)) continue;
			const anchor = graph.nodes.get(fromNodeId);
			const childX = Math.max(threadX, anchor?.x ?? threadX);
			const childBounds = placeSubtree(child, childX, threadY);
			bounds = unionBounds(bounds, childBounds);
			threadX = childBounds.maxX + cfg.componentGap;
		}

		active.delete(componentId);
		placed.add(componentId);
		return bounds;
	};

	const hasExec = (component: ComponentInfo) =>
		component.nodeIds.some((id) => {
			const node = graph.nodes.get(id);
			return node?.kind === "exec" || node?.kind === "entity" || node?.isStart;
		});

	// A component gets its own full-width band if it calls into another
	// component, or if it carries control flow across more than one node.
	// Everything else — lone nodes, pure islands — is shelf-packed at the
	// bottom instead of claiming a whole band each.
	const isMain = (component: ComponentInfo) =>
		childrenOf.has(component.id) ||
		(hasExec(component) && component.nodeIds.length > 1);

	// Components keep the user's existing top-to-bottom arrangement. This is the
	// one coordinate read outside the final anchor, and it is stable under
	// re-layout: the previous run already stacked them in this order, so a
	// second run computes the same sequence.
	const topLeftOf = (component: ComponentInfo): [number, number] => {
		let minY = Number.POSITIVE_INFINITY;
		let minX = Number.POSITIVE_INFINITY;
		for (const id of component.nodeIds) {
			const position = originalPositions.get(id);
			if (!position) continue;
			minY = Math.min(minY, position[1]);
			minX = Math.min(minX, position[0]);
		}
		return [Number.isFinite(minX) ? minX : 0, Number.isFinite(minY) ? minY : 0];
	};

	const ordered = [...components].sort((a, b) => {
		const [ax, ay] = topLeftOf(a);
		const [bx, by] = topLeftOf(b);
		return (
			ay - by || ax - bx || a.roots[0].localeCompare(b.roots[0]) || a.id - b.id
		);
	});

	const isFnRefRoot = (component: ComponentInfo) =>
		(incoming.get(component.id) ?? 0) === 0;

	let cursorY = 0;
	for (const component of ordered) {
		if (placed.has(component.id)) continue;
		if (!isMain(component) || !isFnRefRoot(component)) continue;
		const bounds = placeSubtree(component.id, 0, cursorY);
		cursorY = bounds.maxY + cfg.componentGap;
	}

	// Anything still unplaced that carries control flow: fn-ref cycles, or a
	// callee whose caller never got placed.
	for (const component of ordered) {
		if (placed.has(component.id) || !isMain(component)) continue;
		const bounds = placeSubtree(component.id, 0, cursorY);
		cursorY = bounds.maxY + cfg.componentGap;
	}

	// Islands are usually small and numerous; shelf-pack them into rows instead
	// of giving each a full-width band.
	const remaining = ordered.filter((component) => !placed.has(component.id));
	if (remaining.length === 0) return;

	const boxes = remaining.map((component) => ({
		component,
		bounds: componentBounds(graph, component.nodeIds),
	}));
	const stripWidth = Math.max(
		...boxes.map((box) => box.bounds.maxX - box.bounds.minX),
		Math.ceil(Math.sqrt(boxes.length)) * 400,
	);

	let shelfX = 0;
	let shelfY = cursorY;
	let shelfHeight = 0;
	for (const { component, bounds } of boxes) {
		const width = bounds.maxX - bounds.minX;
		const height = bounds.maxY - bounds.minY;
		if (shelfX > 0 && shelfX + width > stripWidth) {
			shelfX = 0;
			shelfY += shelfHeight + cfg.componentGap;
			shelfHeight = 0;
		}
		placeSubtree(component.id, shelfX, shelfY);
		shelfX += width + cfg.componentGap;
		shelfHeight = Math.max(shelfHeight, height);
	}
}

// ─── Comments ────────────────────────────────────────────────────────────────

export interface CommentBinding {
	id: string;
	containedNodeIds: string[];
	offsetX: number;
	offsetY: number;
}

/**
 * Records which nodes each comment currently covers so the comment can follow
 * them. Without this, every layout strands annotations over unrelated nodes.
 */
export function bindComments(
	comments: readonly LayoutComment[],
	originalPositions: ReadonlyMap<string, readonly [number, number]>,
	sizes: ReadonlyMap<string, readonly [number, number]>,
): CommentBinding[] {
	const bindings: CommentBinding[] = [];

	for (const comment of comments) {
		if (comment.isLocked) continue;
		const contained: string[] = [];
		for (const [nodeId, position] of originalPositions) {
			const size = sizes.get(nodeId);
			const centreX = position[0] + (size?.[0] ?? 0) / 2;
			const centreY = position[1] + (size?.[1] ?? 0) / 2;
			if (
				centreX >= comment.x &&
				centreX <= comment.x + comment.width &&
				centreY >= comment.y &&
				centreY <= comment.y + comment.height
			) {
				contained.push(nodeId);
			}
		}
		if (contained.length === 0) continue;

		contained.sort((a, b) => a.localeCompare(b));
		let minX = Number.POSITIVE_INFINITY;
		let minY = Number.POSITIVE_INFINITY;
		for (const nodeId of contained) {
			const position = originalPositions.get(nodeId);
			if (!position) continue;
			minX = Math.min(minX, position[0]);
			minY = Math.min(minY, position[1]);
		}

		bindings.push({
			id: comment.id,
			containedNodeIds: contained,
			offsetX: comment.x - minX,
			offsetY: comment.y - minY,
		});
	}

	return bindings;
}

export function resolveCommentPositions(
	bindings: readonly CommentBinding[],
	positions: ReadonlyMap<string, [number, number]>,
): Map<string, [number, number]> {
	const result = new Map<string, [number, number]>();

	for (const binding of bindings) {
		let minX = Number.POSITIVE_INFINITY;
		let minY = Number.POSITIVE_INFINITY;
		for (const nodeId of binding.containedNodeIds) {
			const position = positions.get(nodeId);
			if (!position) continue;
			minX = Math.min(minX, position[0]);
			minY = Math.min(minY, position[1]);
		}
		if (!Number.isFinite(minX) || !Number.isFinite(minY)) continue;
		result.set(binding.id, [minX + binding.offsetX, minY + binding.offsetY]);
	}

	return result;
}
