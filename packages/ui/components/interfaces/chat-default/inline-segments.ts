import type { IContent } from "../../../lib/schema/llm/history";
import type { IPlanStep } from "./chat-db";

/**
 * The text an assistant message renders, as one string. Step anchors index into THIS string, so
 * the stamping side (event-processor) and the rendering side (message.tsx) must derive it the
 * same way. Text parts are streamed token fragments: concatenate raw, any injected separator
 * splits words and remark-breaks renders every newline as a hard break.
 */
export function joinContentText(
	content: string | IContent[] | null | undefined,
): string {
	if (typeof content === "string") return content;
	if (!Array.isArray(content)) return "";
	let text = "";
	for (const part of content) {
		if (part.text) text += part.text;
	}
	return text;
}

/**
 * One entry of an assistant reply rendered in stream order: either a slice of the streamed text
 * or the group of actions (tool calls, thinking, sub-agent steps) that ran at that point.
 */
export interface InlineChatSegment {
	key: string;
	text?: string;
	steps?: IPlanStep[];
}

/**
 * Splitting the text inside an unterminated code fence would leave both halves as broken markdown
 * (an unclosed fence above, orphaned code below). Push such a split past the fence's closing line,
 * and never split inside the three-backtick marker itself.
 */
export function safeSplitOffset(text: string, offset: number): number {
	const fences: number[] = [];
	for (let i = text.indexOf("```"); i !== -1; i = text.indexOf("```", i + 3)) {
		fences.push(i);
	}
	if (fences.length === 0) return offset;
	const straddled = fences.find(
		(start) => offset > start && offset < start + 3,
	);
	const point = straddled ?? offset;
	const opened = fences.filter((start) => start < point).length;
	if (opened % 2 === 0) return point;
	const close = fences.find((start) => start >= point);
	if (close === undefined) return text.length;
	const lineEnd = text.indexOf("\n", close + 3);
	return lineEnd === -1 ? text.length : lineEnd + 1;
}

/**
 * Interleave a reply's text with its actions using the steps' `content_offset` anchors, so a
 * websearch renders between the paragraph that preceded it and the evaluation that followed.
 * Steps anchored at the same point render as one group.
 *
 * Returns null when no step carries an anchor (messages saved before anchors existed, app-chat
 * surfaces) — callers then fall back to the legacy grouped block above the text.
 */
export function buildInlineSegments(
	text: string,
	steps: IPlanStep[],
): InlineChatSegment[] | null {
	if (steps.length === 0) return null;
	if (!steps.some((step) => typeof step.content_offset === "number"))
		return null;

	const anchored = steps
		.map((step, index) => ({
			step,
			offset: safeSplitOffset(
				text,
				Math.min(Math.max(step.content_offset ?? 0, 0), text.length),
			),
			index,
		}))
		.sort((a, b) => a.offset - b.offset || a.index - b.index);

	const segments: InlineChatSegment[] = [];
	let cursor = 0;
	let group: IPlanStep[] = [];
	let groupOffset = 0;
	const flushGroup = () => {
		if (group.length === 0) return;
		if (groupOffset > cursor) {
			const slice = text.slice(cursor, groupOffset);
			if (slice.trim()) segments.push({ key: `text-${cursor}`, text: slice });
			cursor = groupOffset;
		}
		segments.push({ key: `steps-${group[0].id}`, steps: group });
		group = [];
	};
	for (const { step, offset } of anchored) {
		if (group.length > 0 && offset !== groupOffset) flushGroup();
		if (group.length === 0) groupOffset = offset;
		group.push(step);
	}
	flushGroup();
	if (cursor < text.length) {
		const slice = text.slice(cursor);
		if (slice.trim()) segments.push({ key: `text-${cursor}`, text: slice });
	}
	return segments;
}
