"use client";

import type { InternalNode } from "@xyflow/react";
import { type RefObject, useMemo } from "react";
import {
	type HandleBox,
	boardComment,
	boardNode,
	commentLabel,
	nodeLabel,
	pinAt,
	pinLabel,
} from "../../lib/bot-den/board-targets";
import {
	type DenHit,
	type DenRect,
	type DenTargetResolver,
	denTargetProps,
	useDenResolver,
} from "../../lib/bot-den/targets";
import type { IAppPointedAt } from "../../lib/pointed-at";
import type { IBoard } from "../../lib/schema/flow/board";
import { useReactFlow } from "./board-runtime";

/** Canvas node types a bot can be dropped on: nodes, collapsed layers and comments. */
const DROPPABLE = new Set(["node", "layerNode", "commentNode", "mediaNode"]);
const COMMENT_TYPES = new Set(["commentNode", "mediaNode"]);
/** How close to a pin, in screen pixels, a drop names the pin rather than its node. */
const PIN_REACH = 11;

interface BoardDenOptions {
	anchor: RefObject<HTMLElement | null>;
	appId: string;
	boardId: string;
	board: IBoard | undefined;
	currentLayer: string | undefined;
	/** Old versions are read-only: nothing on them is droppable. */
	readOnly: boolean;
	selectedNodeIds: readonly string[];
	selectNodes: (ids: string[]) => void;
	highlightNodes: (ids?: string[]) => void;
}

function sizeOf(internal: InternalNode): { width: number; height: number } {
	return {
		width: internal.measured?.width ?? internal.width ?? 0,
		height: internal.measured?.height ?? internal.height ?? 0,
	};
}

function contains(
	internal: InternalNode,
	point: { x: number; y: number },
): boolean {
	const { x, y } = internal.internals.positionAbsolute;
	const { width, height } = sizeOf(internal);
	return (
		point.x >= x &&
		point.x <= x + width &&
		point.y >= y &&
		point.y <= y + height
	);
}

function handlesOf(internal: InternalNode): HandleBox[] {
	const bounds = internal.internals.handleBounds;
	return [...(bounds?.source ?? []), ...(bounds?.target ?? [])];
}

/**
 * Element sensitivity for the board. The canvas stands for the board (or the layer open in it),
 * and a resolver names what the canvas draws under the pointer: a pin, a node (with the rest of
 * the selection), a collapsed layer or a comment. Returns the props for the canvas element.
 */
export function useBoardDenTargets({
	anchor,
	appId,
	boardId,
	board,
	currentLayer,
	readOnly,
	selectedNodeIds,
	selectNodes,
	highlightNodes,
}: BoardDenOptions): Record<string, string> {
	const flow = useReactFlow();

	const resolver = useMemo<DenTargetResolver | null>(() => {
		if (readOnly) return null;
		const toScreen = (
			at: { x: number; y: number },
			width: number,
			height: number,
		): DenRect => {
			const zoom = flow.getViewport().zoom;
			const corner = flow.flowToScreenPosition(at);
			return {
				x: corner.x,
				y: corner.y,
				width: width * zoom,
				height: height * zoom,
			};
		};
		const nodeRect = (id: string): DenRect | null => {
			const internal = flow.getInternalNode(id);
			if (!internal) return null;
			const { width, height } = sizeOf(internal);
			if (width <= 0 || height <= 0) return null;
			return toScreen(internal.internals.positionAbsolute, width, height);
		};
		const pinRect = (nodeId: string, pinId: string): DenRect | null => {
			const internal = flow.getInternalNode(nodeId);
			const handle = internal
				? handlesOf(internal).find((candidate) => candidate.id === pinId)
				: undefined;
			if (!internal || !handle) return null;
			const origin = internal.internals.positionAbsolute;
			return toScreen(
				{ x: origin.x + handle.x, y: origin.y + handle.y },
				handle.width,
				handle.height,
			);
		};
		const scoped = (target: Omit<IAppPointedAt, "app_id" | "board_id">) =>
			({ ...target, app_id: appId, board_id: boardId }) as IAppPointedAt;
		const targetOf = (
			internal: InternalNode,
			type: string,
			point: { x: number; y: number },
		): DenHit | null => {
			const id = internal.id;
			if (COMMENT_TYPES.has(type)) {
				const comment = boardComment(board, id);
				const rect = nodeRect(id);
				return comment && rect
					? {
							target: scoped({
								kind: "comment",
								id,
								label: commentLabel(comment),
							}),
							rect,
						}
					: null;
			}
			if (type === "layerNode") {
				const layer = board?.layers[id];
				const rect = nodeRect(id);
				return layer && rect
					? {
							target: scoped({ kind: "layer", id, label: layer.name || id }),
							rect,
						}
					: null;
			}
			const node = boardNode(board, id);
			const origin = internal.internals.positionAbsolute;
			const zoom = flow.getViewport().zoom;
			const pin = pinAt(
				handlesOf(internal),
				{ x: point.x - origin.x, y: point.y - origin.y },
				PIN_REACH / Math.max(zoom, 0.05),
			);
			const pinName = pin?.id ? pinLabel(node, pin.id) : undefined;
			if (pin?.id && pinName) {
				const rect = pinRect(id, pin.id);
				if (rect)
					return {
						target: scoped({
							kind: "pin",
							id: pin.id,
							ids: [id],
							label: pinName,
						}),
						rect,
					};
			}
			const label = nodeLabel(node);
			const rect = nodeRect(id);
			if (!label || !rect) return null;
			const together =
				selectedNodeIds.length > 1 && selectedNodeIds.includes(id)
					? [id, ...selectedNodeIds.filter((other) => other !== id)]
					: [id];
			return {
				target: scoped({
					kind: "node",
					ids: together,
					label:
						together.length > 1 ? `${label} +${together.length - 1}` : label,
				}),
				rect,
			};
		};
		const ownNodeIds = (target: IAppPointedAt): string[] => {
			if (target.board_id !== boardId) return [];
			if (target.kind === "node" || target.kind === "pin")
				return target.ids ?? [];
			if (
				(target.kind === "layer" && target.id !== currentLayer) ||
				target.kind === "comment"
			)
				return target.id ? [target.id] : [];
			return [];
		};
		return {
			at: (x, y) => {
				const point = flow.screenToFlowPosition({ x, y });
				let hit: { internal: InternalNode; type: string; z: number } | null =
					null;
				for (const node of flow.getNodes()) {
					if (node.hidden || !DROPPABLE.has(node.type ?? "")) continue;
					const internal = flow.getInternalNode(node.id);
					if (!internal || !contains(internal, point)) continue;
					const z = internal.internals.z ?? 0;
					if (!hit || z >= hit.z) hit = { internal, type: node.type ?? "", z };
				}
				return hit ? targetOf(hit.internal, hit.type, point) : null;
			},
			rectOf: (target) => {
				if (target.board_id !== boardId) return null;
				if (target.kind === "pin") {
					const [nodeId] = target.ids ?? [];
					return nodeId && target.id ? pinRect(nodeId, target.id) : null;
				}
				const [first] = ownNodeIds(target);
				return first ? nodeRect(first) : null;
			},
			highlight: (target) => {
				const ids = target
					? ownNodeIds(target).filter((id) => boardNode(board, id))
					: [];
				highlightNodes(ids.length > 0 ? ids : undefined);
			},
			focus: (target) => {
				if (target.board_id !== boardId) return;
				if (target.kind === "node" || target.kind === "pin") {
					if (target.ids?.length) selectNodes(target.ids);
				}
			},
		};
	}, [
		readOnly,
		flow,
		board,
		appId,
		boardId,
		currentLayer,
		selectedNodeIds,
		selectNodes,
		highlightNodes,
	]);

	useDenResolver(anchor, resolver);

	return useMemo(() => {
		if (readOnly) return {};
		const layer = currentLayer ? board?.layers[currentLayer] : undefined;
		const label = layer?.name || board?.name;
		if (!label) return {};
		return denTargetProps(
			layer
				? {
						kind: "layer",
						app_id: appId,
						board_id: boardId,
						id: layer.id,
						label,
					}
				: { kind: "board", app_id: appId, id: boardId, label },
		);
	}, [readOnly, currentLayer, board, appId, boardId]);
}
