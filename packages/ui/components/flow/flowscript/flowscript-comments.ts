/**
 * Board comments projected into the FlowScript editor.
 *
 * A text comment carrying `node_id` binds to that entity's anchor line; all
 * text comments bound to the same anchor form one thread (sorted by
 * timestamp). Dangling or absent `node_id`s are legal — those comments are
 * collected as unanchored board notes and never shown inline. Image/Video
 * comments are canvas artifacts and stay out of the editor entirely.
 *
 * Everything here is pure derivation so the panel can rebuild the model per
 * text/comment change and only touch Monaco decorations when the derived
 * indicator key actually moves (same contract as flowscript-run-trace).
 */

import {
	type IComment,
	ICommentType,
	type ISystemTime,
} from "../../../lib/schema/flow/board";
import type { FlowScriptAnchorIndex } from "./flowscript-anchors";

export interface FlowScriptCommentThread {
	anchorId: string;
	/** 1-based editor line of the anchor the thread is bound to. */
	line: number;
	/** Text comments of this thread, ascending by timestamp (id tiebreak). */
	comments: IComment[];
}

export interface FlowScriptCommentModel {
	/** Threads sorted by line. */
	threads: FlowScriptCommentThread[];
	threadsByAnchorId: Map<string, FlowScriptCommentThread>;
	/** Text comments without a resolvable anchor, ascending by timestamp. */
	unanchored: IComment[];
}

export function isFlowScriptTextComment(comment: IComment): boolean {
	return comment.comment_type === ICommentType.Text;
}

export function commentTimestampMs(comment: IComment): number {
	const timestamp = comment.timestamp;
	if (!timestamp) return 0;
	return (
		(timestamp.secs_since_epoch ?? 0) * 1000 +
		Math.floor((timestamp.nanos_since_epoch ?? 0) / 1e6)
	);
}

const byTimestampThenId = (a: IComment, b: IComment) => {
	const delta = commentTimestampMs(a) - commentTimestampMs(b);
	if (delta !== 0) return delta;
	return a.id < b.id ? -1 : a.id > b.id ? 1 : 0;
};

/**
 * Group the board's text comments into per-anchor threads resolved against
 * THIS render's anchor index. A comment whose `node_id` is missing, empty, or
 * not anchored in the current text lands in `unanchored`.
 */
export function deriveFlowScriptCommentThreads(
	comments: Readonly<Record<string, IComment>> | undefined,
	anchorIndex: Pick<FlowScriptAnchorIndex, "firstLineById">,
): FlowScriptCommentModel {
	const threadsByAnchorId = new Map<string, FlowScriptCommentThread>();
	const unanchored: IComment[] = [];
	for (const comment of Object.values(comments ?? {})) {
		if (!isFlowScriptTextComment(comment)) continue;
		const anchorId = comment.node_id ?? "";
		const line = anchorId ? anchorIndex.firstLineById.get(anchorId) : undefined;
		if (!anchorId || !line) {
			unanchored.push(comment);
			continue;
		}
		const thread = threadsByAnchorId.get(anchorId);
		if (thread) thread.comments.push(comment);
		else
			threadsByAnchorId.set(anchorId, { anchorId, line, comments: [comment] });
	}
	for (const thread of threadsByAnchorId.values()) {
		thread.comments.sort(byTimestampThenId);
	}
	unanchored.sort(byTimestampThenId);
	const threads = [...threadsByAnchorId.values()].sort(
		(a, b) => a.line - b.line,
	);
	return { threads, threadsByAnchorId, unanchored };
}

export interface FlowScriptCommentIndicator {
	line: number;
	anchorId: string;
	count: number;
	/** First comment's author (for peer coloring), when it has one. */
	firstAuthor?: string;
	/** Peer palette slot of `firstAuthor`; undefined renders the neutral style. */
	slot?: number;
}

export interface FlowScriptCommentIndicators {
	indicators: FlowScriptCommentIndicator[];
	/** Canonical identity of the indicator set — equal key ⇒ skip the decoration write. */
	key: string;
}

/** One margin indicator per thread line, sorted, with a run-trace-style key. */
export function deriveFlowScriptCommentIndicators(
	threads: readonly FlowScriptCommentThread[],
	slotFor?: (sub: string) => number | undefined,
): FlowScriptCommentIndicators {
	const indicators = threads.map((thread) => {
		const author = thread.comments[0]?.author ?? undefined;
		const firstAuthor = author && author !== "anonymous" ? author : undefined;
		return {
			line: thread.line,
			anchorId: thread.anchorId,
			count: thread.comments.length,
			firstAuthor,
			slot: firstAuthor ? slotFor?.(firstAuthor) : undefined,
		};
	});
	indicators.sort((a, b) => a.line - b.line);
	const key = indicators
		.map(
			(indicator) =>
				`${indicator.line}@${indicator.anchorId}:${indicator.count}#${indicator.slot ?? "n"}`,
		)
		.join("|");
	return { indicators, key };
}

/**
 * Lines that should show the hover "add a comment" affordance: node-anchored
 * lines whose node holds no thread yet. Branch statements anchor one node on
 * several lines — every line of a node that already has a thread is skipped,
 * so the affordance never sits next to an existing indicator of the same node.
 */
export function deriveFlowScriptCommentAddLines(
	anchorIndex: Pick<FlowScriptAnchorIndex, "anchors">,
	model: Pick<FlowScriptCommentModel, "threadsByAnchorId">,
): number[] {
	const lines = new Set<number>();
	for (const anchor of anchorIndex.anchors) {
		if (anchor.kind !== "node") continue;
		if (model.threadsByAnchorId.has(anchor.id)) continue;
		lines.add(anchor.line);
	}
	return [...lines].sort((a, b) => a - b);
}

/**
 * Edit/delete gate: own comments only. Authorless comments (and the canvas's
 * legacy "anonymous" author) belong to nobody and stay editable by everyone.
 */
export function canModifyFlowScriptComment(
	comment: IComment,
	sub?: string,
): boolean {
	const author = comment.author ?? "";
	if (author === "" || author === "anonymous") return true;
	return author === sub;
}

export function flowScriptCommentTimestamp(nowMs: number): ISystemTime {
	return {
		secs_since_epoch: Math.floor(nowMs / 1000),
		nanos_since_epoch: Math.round((nowMs % 1000) * 1e6),
	};
}

/** Spatial facts about the anchored board node a new comment is placed near. */
export interface FlowScriptNodeSpatial {
	coordinates?: readonly number[] | null;
	layer?: string | null;
}

/** Canvas offset so an editor-created comment doesn't cover its node. */
export const FLOWSCRIPT_COMMENT_OFFSET_X = 48;
export const FLOWSCRIPT_COMMENT_OFFSET_Y = -96;

export interface BuildFlowScriptCommentInput {
	id: string;
	anchorId: string;
	content: string;
	/** The local user's sub; absent leaves the comment authorless. */
	author?: string;
	/** The anchored node's canvas position/layer, when the board knows it. */
	node?: FlowScriptNodeSpatial;
	nowMs: number;
}

/**
 * Payload for a comment created from the editor: bound to the line's anchor,
 * placed near the node on the canvas (slightly offset), on the node's layer.
 * Without node coordinates it falls back to the canvas's comment-less default
 * (origin; width/height stay unset so the canvas render defaults apply).
 */
export function buildFlowScriptComment({
	id,
	anchorId,
	content,
	author,
	node,
	nowMs,
}: BuildFlowScriptCommentInput): IComment {
	const nodeCoordinates =
		node?.coordinates &&
		typeof node.coordinates[0] === "number" &&
		typeof node.coordinates[1] === "number"
			? node.coordinates
			: undefined;
	const coordinates = nodeCoordinates
		? [
				nodeCoordinates[0] + FLOWSCRIPT_COMMENT_OFFSET_X,
				nodeCoordinates[1] + FLOWSCRIPT_COMMENT_OFFSET_Y,
				typeof nodeCoordinates[2] === "number" ? nodeCoordinates[2] : 0,
			]
		: [0, 0, 0];
	return {
		id,
		author: author && author.length > 0 ? author : undefined,
		comment_type: ICommentType.Text,
		content,
		coordinates,
		layer: node?.layer ?? undefined,
		node_id: anchorId,
		timestamp: flowScriptCommentTimestamp(nowMs),
	};
}

/** Content edit that never touches identity, binding, placement or timestamp. */
export function withFlowScriptCommentContent(
	comment: IComment,
	content: string,
): IComment {
	return { ...comment, content };
}

/**
 * Plain-text hover preview for a thread's margin indicator. Formatting of
 * names and times is injected so this stays pure (and testable) while the
 * panel supplies identity lookup + localized relative time.
 */
export function formatFlowScriptCommentPreview(
	thread: FlowScriptCommentThread,
	nameFor: (author?: string) => string,
	timeFor: (ms: number) => string,
	maxComments = 3,
	maxContentLength = 140,
): string {
	const lines = thread.comments.slice(0, maxComments).map((comment) => {
		const content =
			comment.content.length > maxContentLength
				? `${comment.content.slice(0, maxContentLength - 1)}…`
				: comment.content;
		const author = comment.author ?? undefined;
		const name = nameFor(author && author !== "anonymous" ? author : undefined);
		return `${name} · ${timeFor(commentTimestampMs(comment))}: ${content.replaceAll("\n", " ")}`;
	});
	const overflow = thread.comments.length - maxComments;
	if (overflow > 0) lines.push(`+${overflow}`);
	return lines.join("\n");
}
