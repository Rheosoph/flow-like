import type { IContent } from "../../../lib/schema/llm/history";
import type { IChatWidget, IPlanStep } from "./chat-db";

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

/** A node the chat host places into a message at a text offset (e.g. a global chat app card). */
export interface InlineAnchoredContent {
	key: string;
	offset: number;
}

/**
 * One entry of an assistant reply rendered in stream order: a slice of the streamed text, the
 * group of actions (tool calls, thinking, sub-agent steps) that ran at that point, the widgets
 * pushed there, or one host-anchored node.
 */
export interface InlineChatSegment<
	C extends InlineAnchoredContent = InlineAnchoredContent,
> {
	key: string;
	text?: string;
	steps?: IPlanStep[];
	widgets?: IChatWidget[];
	content?: C;
}

type InlineAnchor<C> =
	| { offset: number; index: number; kind: "steps"; step: IPlanStep }
	| { offset: number; index: number; kind: "widgets"; widget: IChatWidget }
	| { offset: number; index: number; kind: "content"; content: C };

/** At one offset the actions render first, then the widgets they pushed, then host content. */
const KIND_ORDER = { steps: 0, widgets: 1, content: 2 } as const;

const isAnchored = (item: { content_offset?: number }) =>
	typeof item.content_offset === "number";

/** Widgets that render after the text when segments are shown; the same array when none is anchored. */
export function unanchoredWidgets(
	widgets: IChatWidget[] | undefined,
): IChatWidget[] | undefined {
	if (!widgets?.some(isAnchored)) return widgets;
	return widgets.filter((widget) => !isAnchored(widget));
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

const groupSegment = <C extends InlineAnchoredContent>(
	group: InlineAnchor<C>[],
): InlineChatSegment<C> => {
	const first = group[0];
	switch (first.kind) {
		case "steps":
			return {
				key: `steps-${first.step.id}`,
				steps: group.flatMap((anchor) =>
					anchor.kind === "steps" ? [anchor.step] : [],
				),
			};
		case "widgets":
			return {
				key: `widgets-${first.widget.instance_id}`,
				widgets: group.flatMap((anchor) =>
					anchor.kind === "widgets" ? [anchor.widget] : [],
				),
			};
		case "content":
			return { key: `content-${first.content.key}`, content: first.content };
	}
};

/**
 * Interleave a reply's text with its actions using the steps' `content_offset` anchors, so a
 * websearch renders between the paragraph that preceded it and the evaluation that followed.
 * Steps anchored at the same point render as one group, and so do widgets; each host `content`
 * entry is its own segment so removing one never re-keys its neighbours.
 *
 * Returns null when nothing carries an anchor (messages saved before anchors existed, app-chat
 * surfaces) — callers then fall back to the legacy grouped block above the text. Steps join the
 * segments only when at least one of them is anchored, and unanchored widgets never do: both keep
 * their legacy placement.
 */
export function buildInlineSegments<
	C extends InlineAnchoredContent = InlineAnchoredContent,
>(
	text: string,
	steps: IPlanStep[],
	widgets: readonly IChatWidget[] = [],
	content: readonly C[] = [],
): InlineChatSegment<C>[] | null {
	const stepsAnchored = steps.some(isAnchored);
	const anchoredWidgets = widgets.filter(isAnchored);
	if (!stepsAnchored && anchoredWidgets.length === 0 && content.length === 0)
		return null;

	const place = (offset: number | undefined) =>
		safeSplitOffset(text, Math.min(Math.max(offset ?? 0, 0), text.length));
	const anchors: InlineAnchor<C>[] = [
		...(stepsAnchored
			? steps.map((step, index) => ({
					offset: place(step.content_offset),
					index,
					kind: "steps" as const,
					step,
				}))
			: []),
		...anchoredWidgets.map((widget, index) => ({
			offset: place(widget.content_offset),
			index,
			kind: "widgets" as const,
			widget,
		})),
		...content.map((entry, index) => ({
			offset: place(entry.offset),
			index,
			kind: "content" as const,
			content: entry,
		})),
	].sort(
		(a, b) =>
			a.offset - b.offset ||
			KIND_ORDER[a.kind] - KIND_ORDER[b.kind] ||
			a.index - b.index,
	);

	const segments: InlineChatSegment<C>[] = [];
	let cursor = 0;
	const pushTextUntil = (end: number) => {
		if (end <= cursor) return;
		const slice = text.slice(cursor, end);
		if (slice.trim()) segments.push({ key: `text-${cursor}`, text: slice });
		cursor = end;
	};
	let group: InlineAnchor<C>[] = [];
	const flushGroup = () => {
		if (group.length === 0) return;
		pushTextUntil(group[0].offset);
		segments.push(groupSegment(group));
		group = [];
	};
	for (const anchor of anchors) {
		const head = group[0];
		if (
			head &&
			(head.offset !== anchor.offset ||
				head.kind !== anchor.kind ||
				anchor.kind === "content")
		)
			flushGroup();
		group.push(anchor);
	}
	flushGroup();
	pushTextUntil(text.length);
	return segments;
}
