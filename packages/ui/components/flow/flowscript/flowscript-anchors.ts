/**
 * Anchor comments tie rendered FlowScript lines back to board entities:
 * `//@n:<id>` (node), `//@v:<id>` (variable), `//@l:<id>` (function layer).
 * The renderer always emits them TRAILING on the line that owns the entity —
 * a statement's anchor at end of line, a branch node's anchor on its `if (...) {`
 * line (possibly after a pin-name comment like `// exec_out_exists`), a variable's
 * on its `const … =` line, a function's `//@l:` on its `function … {` line.
 *
 * Parsing scans the document with a tiny string/template/comment state machine so
 * anchor-shaped text inside string or template literals is never treated as an
 * anchor. (`maskLiteralsWithSpans` cannot be reused here: it blanks comment
 * contents, which is exactly the text the anchors live in.)
 */

export type FlowScriptAnchorKind = "node" | "variable" | "layer";

export interface FlowScriptAnchor {
	id: string;
	kind: FlowScriptAnchorKind;
	/** 1-based line the anchor comment sits on. */
	line: number;
	/** 1-based column where `//@` starts. */
	column: number;
	/** 1-based column just past the end of the anchor text. */
	endColumn: number;
}

export interface FlowScriptAnchorIndex {
	anchors: FlowScriptAnchor[];
	byLine: Map<number, FlowScriptAnchor>;
	/** First line an id is anchored on (canvas → editor lookups). */
	firstLineById: Map<string, number>;
}

const ANCHOR_KINDS: Record<string, FlowScriptAnchorKind> = {
	n: "node",
	v: "variable",
	l: "layer",
};

/** Matches a trailing anchor comment at the very end of a line. */
const TRAILING_ANCHOR = /\/\/@([nvl]):([A-Za-z0-9_-]+)[ \t]*$/;

const NL = 10;
const DQUOTE = 34;
const SQUOTE = 39;
const BACKTICK = 96;
const SLASH = 47;
const BACKSLASH = 92;
const OPEN_BRACE = 123;
const CLOSE_BRACE = 125;
const DOLLAR = 36;

type ScanState =
	| { kind: "code"; inTemplateExpr: boolean; depth: number }
	| { kind: "string"; quote: number }
	| { kind: "template" }
	| { kind: "comment" };

/**
 * Offset of the first line comment (`//`) opened in code context on each line,
 * keyed by 0-based line index. Lines whose end sits inside a string or template
 * literal are absent, so literal contents can never contribute anchors.
 *
 * Uses char codes throughout: this runs on every keystroke over the whole
 * document, and per-character string comparisons were the dominant cost.
 */
function lineCommentStarts(text: string): Map<number, number> {
	const starts = new Map<number, number>();
	const stack: ScanState[] = [
		{ kind: "code", inTemplateExpr: false, depth: 0 },
	];
	let line = 0;
	let lineStart = 0;
	let i = 0;
	while (i < text.length) {
		const code = text.charCodeAt(i);
		const top = stack[stack.length - 1];
		if (code === NL) {
			if (top.kind === "comment" || top.kind === "string") stack.pop();
			line++;
			lineStart = i + 1;
			i++;
			continue;
		}
		switch (top.kind) {
			case "code":
				if (code === DQUOTE || code === SQUOTE) {
					stack.push({ kind: "string", quote: code });
				} else if (code === BACKTICK) {
					stack.push({ kind: "template" });
				} else if (code === SLASH && text.charCodeAt(i + 1) === SLASH) {
					if (!starts.has(line)) starts.set(line, i - lineStart);
					stack.push({ kind: "comment" });
					i += 2;
					continue;
				} else if (top.inTemplateExpr && code === OPEN_BRACE) {
					top.depth++;
				} else if (top.inTemplateExpr && code === CLOSE_BRACE) {
					// Closes the `${ … }` template expression once its own braces balance.
					if (top.depth === 0) stack.pop();
					else top.depth--;
				}
				break;
			case "string":
				if (code === BACKSLASH) {
					i += 2;
					continue;
				}
				if (code === top.quote) stack.pop();
				break;
			case "template":
				if (code === BACKSLASH) {
					i += 2;
					continue;
				}
				if (code === BACKTICK) {
					stack.pop();
				} else if (code === DOLLAR && text.charCodeAt(i + 1) === OPEN_BRACE) {
					stack.push({ kind: "code", inTemplateExpr: true, depth: 0 });
					i += 2;
					continue;
				}
				break;
			case "comment": {
				// Skip to the end of the line in one step; the comment state pops there.
				const next = text.indexOf("\n", i);
				i = next < 0 ? text.length : next;
				continue;
			}
		}
		i++;
	}
	return starts;
}

let lastParsedText: string | undefined;
let lastParsedIndex: FlowScriptAnchorIndex | undefined;

export function parseFlowScriptAnchors(text: string): FlowScriptAnchorIndex {
	// One-slot memo: the panel and its effects parse the same text several times
	// per change (render memo, decorations, cursor sync) — share one pass.
	if (text === lastParsedText && lastParsedIndex) return lastParsedIndex;
	const parsed = parseFlowScriptAnchorsUncached(text);
	lastParsedText = text;
	lastParsedIndex = parsed;
	return parsed;
}

function parseFlowScriptAnchorsUncached(text: string): FlowScriptAnchorIndex {
	const anchors: FlowScriptAnchor[] = [];
	const byLine = new Map<number, FlowScriptAnchor>();
	const firstLineById = new Map<string, number>();
	const commentStarts = lineCommentStarts(text);
	if (commentStarts.size > 0) {
		const lines = text.split("\n");
		for (const [lineIndex, commentStart] of commentStarts) {
			const lineText = lines[lineIndex] ?? "";
			const match = TRAILING_ANCHOR.exec(lineText);
			if (!match || match.index < commentStart) continue;
			const kind = ANCHOR_KINDS[match[1]];
			if (!kind) continue;
			const anchor: FlowScriptAnchor = {
				id: match[2],
				kind,
				line: lineIndex + 1,
				column: match.index + 1,
				endColumn: match.index + 1 + match[0].trimEnd().length,
			};
			anchors.push(anchor);
			byLine.set(anchor.line, anchor);
			if (!firstLineById.has(anchor.id))
				firstLineById.set(anchor.id, anchor.line);
		}
	}
	return { anchors, byLine, firstLineById };
}

export function anchorAtLine(
	index: FlowScriptAnchorIndex,
	line: number,
): FlowScriptAnchor | undefined {
	return index.byLine.get(line);
}

/**
 * The anchor on `line`, or the nearest one above it (for cursor positions on
 * un-anchored lines like `} else {`, closing braces or blank lines). `maxUp`
 * bounds the walk so the top of a document without anchors stays cheap.
 */
export function anchorAtOrAbove(
	index: FlowScriptAnchorIndex,
	line: number,
	maxUp = 50,
): FlowScriptAnchor | undefined {
	for (
		let candidate = line;
		candidate >= 1 && line - candidate <= maxUp;
		candidate--
	) {
		const anchor = index.byLine.get(candidate);
		if (anchor) return anchor;
	}
	return undefined;
}
