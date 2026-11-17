import type { IBoard, IComment } from "../schema/flow/board";
import type { INode } from "../schema/flow/node";

const COMMENT_LABEL = 60;

/** A pin's handle relative to its node, as the canvas measured it. */
export interface HandleBox {
	id?: string | null;
	x: number;
	y: number;
	width: number;
	height: number;
}

/** A node of the board, in the board itself or in one of its layers. */
export function boardNode(
	board: IBoard | undefined,
	id: string,
): INode | undefined {
	if (!board) return undefined;
	return (
		board.nodes[id] ??
		Object.values(board.layers ?? {}).find((layer) => layer.nodes?.[id])?.nodes[
			id
		]
	);
}

export function nodeLabel(node: INode | undefined): string | undefined {
	return node?.friendly_name?.trim() || node?.name || undefined;
}

export function boardComment(
	board: IBoard | undefined,
	id: string,
): IComment | undefined {
	if (!board) return undefined;
	return (
		board.comments?.[id] ??
		Object.values(board.layers ?? {}).find((layer) => layer.comments?.[id])
			?.comments[id]
	);
}

/** A comment's first line of text, without markdown marks; its kind when it holds no text. */
export function commentLabel(comment: IComment): string {
	const line = comment.content
		.split("\n")
		.map((text) =>
			text
				.replace(/!?\[([^\]]*)]\([^)]*\)/g, "$1")
				.replace(/[#>*_`~|-]+/g, " ")
				.replace(/\s+/g, " ")
				.trim(),
		)
		.find((text) => text.length > 0);
	if (!line) return comment.comment_type;
	return line.length > COMMENT_LABEL
		? `${line.slice(0, COMMENT_LABEL - 1)}…`
		: line;
}

/** The pin whose handle centre is nearest to `point` (relative to its node), within `radius`. */
export function pinAt(
	handles: readonly HandleBox[],
	point: { x: number; y: number },
	radius: number,
): HandleBox | null {
	let nearest: HandleBox | null = null;
	let best = radius;
	for (const handle of handles) {
		if (!handle.id) continue;
		const distance = Math.hypot(
			handle.x + handle.width / 2 - point.x,
			handle.y + handle.height / 2 - point.y,
		);
		if (distance <= best) {
			best = distance;
			nearest = handle;
		}
	}
	return nearest;
}

/** "HTTP Request › URL": the node and the pin, as the board names them. */
export function pinLabel(
	node: INode | undefined,
	pinId: string,
): string | undefined {
	const owner = nodeLabel(node);
	const pin = node?.pins?.[pinId];
	const name = pin?.friendly_name?.trim() || pin?.name;
	if (!owner || !name) return undefined;
	return `${owner} › ${name}`;
}
