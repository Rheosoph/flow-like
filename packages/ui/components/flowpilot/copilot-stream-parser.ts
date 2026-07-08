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

// Order matters only in that every control tag is stripped before the leftover is emitted as text.
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
}

export function createCopilotStreamParser(): CopilotStreamParser {
	let buffer = "";

	return {
		reset() {
			buffer = "";
		},
		flush(): CopilotStreamEvent[] {
			if (!buffer) return [];
			const text = buffer;
			buffer = "";
			return [{ type: "text", text }];
		},
		push(chunk: string): CopilotStreamEvent[] {
			let token = buffer + chunk;
			buffer = "";

			// Hold back a trailing, not-yet-closed tag so we never parse a half-arrived frame.
			const lastOpen = token.lastIndexOf("<");
			if (lastOpen !== -1 && !token.slice(lastOpen).includes(">")) {
				buffer = token.slice(lastOpen);
				token = token.slice(0, lastOpen);
			}

			const events: CopilotStreamEvent[] = [];

			for (const type of TAG_TYPES) {
				const re = new RegExp(`<${type}>([\\s\\S]*?)</${type}>`, "g");
				token = token.replace(re, (_match, inner: string) => {
					events.push({ type, raw: inner, data: safeParse(inner) });
					return "";
				});
			}

			if (token.length > 0) events.push({ type: "text", text: token });

			return events;
		},
	};
}
