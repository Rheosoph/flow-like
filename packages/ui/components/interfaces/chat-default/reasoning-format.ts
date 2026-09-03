/**
 * Shared display heuristics for streamed model reasoning.
 *
 * Historically the backend joined reasoning deltas with a newline per token;
 * the producers are fixed, but messages persisted before the fix (and any
 * third-party producer) can still carry one-word-per-line reasoning, so the
 * repair heuristic stays as a display-time guard. It must live in exactly one
 * place — it was previously duplicated verbatim in the event processor and the
 * reasoning viewer.
 */

export function hasStructuredMarkdown(reasoning: string): boolean {
	return reasoning
		.split(/\r?\n/)
		.map((line) => line.trim())
		.filter(Boolean)
		.some(
			(line) =>
				line.startsWith("```") ||
				/^#{1,6}\s/.test(line) ||
				/^[-*+]\s/.test(line) ||
				/^\d+\.\s/.test(line) ||
				/^>\s/.test(line) ||
				/^\|.*\|$/.test(line),
		);
}

export function looksLikeTokenizedReasoning(reasoning: string): boolean {
	const lines = reasoning
		.split(/\r?\n/)
		.map((line) => line.trim())
		.filter(Boolean);

	if (lines.length < 6 || hasStructuredMarkdown(reasoning)) {
		return false;
	}

	const shortLineCount = lines.filter((line) => {
		const wordCount = line.split(/\s+/).filter(Boolean).length;
		return wordCount <= 3 && line.length <= 24;
	}).length;

	return shortLineCount / lines.length >= 0.7;
}

export function normalizeReasoningWhitespace(reasoning: string): string {
	let normalized = "";
	let pendingSpace = false;

	for (const ch of reasoning) {
		if (/\s/.test(ch)) {
			pendingSpace = normalized.length > 0;
			continue;
		}

		if (pendingSpace && !/[.,;:!?)}\]'"]/.test(ch)) {
			normalized += " ";
		}

		pendingSpace = false;
		normalized += ch;
	}

	return normalized;
}

export function sanitizeReasoningForDisplay(reasoning: string): string {
	return looksLikeTokenizedReasoning(reasoning)
		? normalizeReasoningWhitespace(reasoning)
		: reasoning;
}

export interface ExtractedThinking {
	/** The visible answer with all think blocks removed. */
	text: string;
	/** Concatenated think-block content, empty when none. */
	thinking: string;
}

/**
 * Splits `<think>…</think>` blocks out of assistant content. Some models (and
 * the agent's own think tool echo) inline reasoning into the answer body this
 * way; rendered verbatim it splices thinking mid-paragraph into the reply. An
 * unterminated trailing `<think>` (mid-stream) swallows the rest of the text
 * so thinking never flashes up as answer text while streaming.
 */
export function extractThinkBlocks(content: string): ExtractedThinking {
	if (!content.includes("<think>")) {
		return { text: content, thinking: "" };
	}

	let text = "";
	let thinking = "";
	let rest = content;

	while (true) {
		const start = rest.indexOf("<think>");
		if (start < 0) {
			text += rest;
			break;
		}
		text += rest.slice(0, start);
		const afterOpen = rest.slice(start + "<think>".length);
		const end = afterOpen.indexOf("</think>");
		if (end < 0) {
			thinking += afterOpen;
			break;
		}
		thinking += afterOpen.slice(0, end);
		rest = afterOpen.slice(end + "</think>".length);
	}

	return { text: text.trim(), thinking: thinking.trim() };
}
