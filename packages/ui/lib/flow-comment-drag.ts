import type { Node, XYPosition } from "@xyflow/react";
import { coversCentre } from "./flow-layout/pack";

const FRAME_TYPE = "commentNode";

const PASSENGER_TYPES = new Set([
	"node",
	"flowNode",
	"callFunctionNode",
	"commentNode",
	"mediaNode",
	"layerNode",
	"layerInnerNode",
]);

export interface CommentDragSession {
	anchorId: string;
	anchorStart: XYPosition;
	passengers: ReadonlyMap<string, XYPosition>;
}

export interface PassengerPosition {
	id: string;
	x: number;
	y: number;
}

function sizeOf(node: Node): [number, number] {
	return [
		node.measured?.width ?? node.width ?? 0,
		node.measured?.height ?? node.height ?? 0,
	];
}

/**
 * Everything the dragged comments carry: nodes whose centre a comment covers
 * at drag start, followed through nested comments. Nodes already in the drag
 * and anything pinned in place (locked comments) stay where they are.
 */
function collectCommentPassengers(
	dragged: readonly Node[],
	all: readonly Node[],
): Map<string, XYPosition> {
	const draggedIds = new Set(dragged.map((node) => node.id));
	const candidates = all.filter(
		(node) =>
			PASSENGER_TYPES.has(node.type ?? "") &&
			!node.hidden &&
			node.draggable !== false &&
			!draggedIds.has(node.id),
	);
	const frames = dragged.filter((node) => node.type === FRAME_TYPE);
	const passengers = new Map<string, XYPosition>();

	for (let index = 0; index < frames.length; index++) {
		const frame = frames[index];
		const [width, height] = sizeOf(frame);
		const rect = { x: frame.position.x, y: frame.position.y, width, height };
		for (const node of candidates) {
			if (passengers.has(node.id)) continue;
			if (!coversCentre(rect, [node.position.x, node.position.y], sizeOf(node)))
				continue;
			passengers.set(node.id, { x: node.position.x, y: node.position.y });
			if (node.type === FRAME_TYPE) frames.push(node);
		}
	}

	return passengers;
}

export function startCommentDrag(
	anchor: Node,
	dragged: readonly Node[],
	all: readonly Node[],
): CommentDragSession | undefined {
	const passengers = collectCommentPassengers(dragged, all);
	if (passengers.size === 0) return undefined;
	return {
		anchorId: anchor.id,
		anchorStart: { x: anchor.position.x, y: anchor.position.y },
		passengers,
	};
}

/** Passenger positions that keep their offset to wherever the anchor is now. */
export function followAnchor(
	session: CommentDragSession,
	dragged: readonly Pick<Node, "id" | "position">[],
): PassengerPosition[] {
	const anchor = dragged.find((node) => node.id === session.anchorId)?.position;
	if (!anchor) return [];
	const dx = anchor.x - session.anchorStart.x;
	const dy = anchor.y - session.anchorStart.y;
	return Array.from(session.passengers, ([id, start]) => ({
		id,
		x: start.x + dx,
		y: start.y + dy,
	}));
}
