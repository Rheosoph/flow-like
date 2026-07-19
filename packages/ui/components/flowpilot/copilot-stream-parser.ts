/**
 * Shared parser for the FlowPilot copilot stream protocol.
 *
 * Both the board FlowPilot (copilot_chat) and the global assistant (global_chat) stream over a Tauri
 * `Channel<string>` that interleaves raw assistant text with XML-tagged control frames
 * (`<tool_start>…</tool_start>`, `<plan_step>…</plan_step>`, `<commands>…`, etc.). Every backend
 * (Bits/rig, GitHub Copilot, Codex, Claude Code) emits this same grammar. This module turns a chunk
 * stream into typed events so every consumer shares one grammar and one partial-tag buffering rule
 * instead of re-deriving it.
 */

export type CopilotStreamEventType =
	| "text"
	| "scope_decision"
	| "flowscript_workspace"
	| "tool_start"
	| "tool_progress"
	| "tool_end"
	| "plan_step"
	| "commands"
	| "components"
	| "canvas_settings"
	| "usage_stat";

export interface CopilotStreamEvent {
	type: CopilotStreamEventType;
	/** Best-effort parsed JSON payload of a tag body (undefined if not JSON). */
	data?: unknown;
	/** Raw inner string of the tag body. */
	raw?: string;
	/** Cleaned assistant text delta, present only on `text` events. */
	text?: string;
}

const TAG_TYPES: CopilotStreamEventType[] = [
	"scope_decision",
	"flowscript_workspace",
	"tool_start",
	"tool_progress",
	"tool_end",
	"plan_step",
	"commands",
	"components",
	"canvas_settings",
	"usage_stat",
];
const OPEN_TAGS = TAG_TYPES.map((type) => ({ type, tag: `<${type}>` }));

function earliestOpeningTag(input: string, from: number) {
	let match: {
		type: CopilotStreamEventType;
		index: number;
		tag: string;
	} | null = null;
	for (const candidate of OPEN_TAGS) {
		const index = input.indexOf(candidate.tag, from);
		if (index < 0 || (match && index >= match.index)) continue;
		match = { ...candidate, index };
	}
	return match;
}

/** Return a trailing `<tool_...` prefix that may become a known opening tag next chunk. */
function partialOpeningTagIndex(input: string, from: number) {
	const index = input.lastIndexOf("<");
	if (index < from) return -1;
	const suffix = input.slice(index);
	return OPEN_TAGS.some(({ tag }) => tag.startsWith(suffix)) ? index : -1;
}

function safeParse(input: string): unknown {
	try {
		return JSON.parse(input);
	} catch {
		return undefined;
	}
}

export interface CopilotStreamParser {
	/** Feed the next raw chunk; returns the control + text events it produced. */
	push(chunk: string): CopilotStreamEvent[];
	/**
	 * Call once when the stream ends: emits any held-back partial-tag fragment as plain text so
	 * replies that legitimately end with `<...` are not silently truncated.
	 */
	flush(): CopilotStreamEvent[];
	/** Reset buffering state between conversations. */
	reset(): void;
	/** Exposed for diagnostics/tests; buffered protocol state is always bounded. */
	bufferedLength(): number;
}

export interface CopilotStreamParserOptions {
	/** Maximum incomplete control-frame size before that frame is discarded. */
	maxBufferedChars?: number;
}

const DEFAULT_MAX_BUFFERED_CHARS = 1024 * 1024;
const DEFAULT_MAX_PROGRESS_DETAIL_CHARS = 12_000;

/** Append progress text while retaining recent diagnostics within a fixed renderer-memory bound. */
export function appendBoundedStreamDetail(
	existing: string | undefined,
	message: string,
	maxChars = DEFAULT_MAX_PROGRESS_DETAIL_CHARS,
) {
	const combined = existing ? `${existing}\n\n${message}` : message;
	if (combined.length <= maxChars) return combined;
	const marker = "[earlier progress truncated]\n\n";
	return marker + combined.slice(-(maxChars - marker.length));
}

export function createCopilotStreamParser(
	options: CopilotStreamParserOptions = {},
): CopilotStreamParser {
	const maxBufferedChars = Math.max(
		256,
		options.maxBufferedChars ?? DEFAULT_MAX_BUFFERED_CHARS,
	);
	let buffer = "";
	let discardedFrameClosingTag: string | undefined;
	let discardedFrameTail = "";

	return {
		reset() {
			buffer = "";
			discardedFrameClosingTag = undefined;
			discardedFrameTail = "";
		},
		bufferedLength() {
			return buffer.length + discardedFrameTail.length;
		},
		flush(): CopilotStreamEvent[] {
			// An oversized control frame is intentionally omitted. It cannot be safely parsed and
			// retaining it would defeat the parser's memory bound.
			discardedFrameClosingTag = undefined;
			discardedFrameTail = "";
			if (!buffer) return [];
			const text = buffer;
			buffer = "";
			return [{ type: "text", text }];
		},
		push(chunk: string): CopilotStreamEvent[] {
			let token = buffer + chunk;
			buffer = "";
			const events: CopilotStreamEvent[] = [];
			if (discardedFrameClosingTag) {
				const discarded = discardedFrameTail + token;
				const closingIndex = discarded.indexOf(discardedFrameClosingTag);
				if (closingIndex < 0) {
					discardedFrameTail = discarded.slice(
						-Math.max(0, discardedFrameClosingTag.length - 1),
					);
					return events;
				}
				token = discarded.slice(closingIndex + discardedFrameClosingTag.length);
				discardedFrameClosingTag = undefined;
				discardedFrameTail = "";
			}
			let cursor = 0;
			while (cursor < token.length) {
				const opening = earliestOpeningTag(token, cursor);
				if (!opening) {
					const partialIndex = partialOpeningTagIndex(token, cursor);
					const textEnd = partialIndex >= 0 ? partialIndex : token.length;
					if (textEnd > cursor) {
						events.push({ type: "text", text: token.slice(cursor, textEnd) });
					}
					if (partialIndex >= 0) buffer = token.slice(partialIndex);
					break;
				}

				if (opening.index > cursor) {
					events.push({
						type: "text",
						text: token.slice(cursor, opening.index),
					});
				}
				const bodyStart = opening.index + opening.tag.length;
				const closingTag = `</${opening.type}>`;
				const closingIndex = token.indexOf(closingTag, bodyStart);
				if (closingIndex < 0) {
					// Keep the complete opener and body together until its matching close arrives.
					const incompleteFrame = token.slice(opening.index);
					if (incompleteFrame.length <= maxBufferedChars) {
						buffer = incompleteFrame;
					} else {
						// Drop the oversized frame without buffering the rest of its body. Subsequent chunks
						// are scanned only for the matching close, retaining at most a close-tag prefix.
						discardedFrameClosingTag = closingTag;
						discardedFrameTail = incompleteFrame.slice(
							-Math.max(0, closingTag.length - 1),
						);
					}
					break;
				}
				const raw = token.slice(bodyStart, closingIndex);
				events.push({ type: opening.type, raw, data: safeParse(raw) });
				cursor = closingIndex + closingTag.length;
			}

			return events;
		},
	};
}
