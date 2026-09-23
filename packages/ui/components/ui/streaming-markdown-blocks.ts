import type { Descendant, Value } from "platejs";
import { preprocessDirectiveBlocks } from "../editor/plugins/remark-directives";
import {
	RICH_REMARK_PLUGINS,
	deserializeMarkdown,
	safeDeserialize,
	transformSpecialLinks,
} from "./text-editor";

/**
 * Incremental markdown parsing for the streaming path.
 *
 * Re-parsing the whole accumulated reply on every token is O(n) per token and
 * therefore O(n²) over a stream. Every completed top-level block, though, parses
 * to the same nodes no matter how much text arrives after it, so completed
 * blocks are cached by their source text and only the growing tail is
 * re-parsed. Reusing the cached node *objects* is what lets Plate's
 * `ElementStatic` (memoized on element identity) skip them on re-render.
 *
 * The splitter below is deliberately not `splitMarkdownPreservingCodeBlocks`
 * from text-editor.ts: that one trims blocks and treats a fence opener as an
 * unconditional boundary, both of which change what the markdown means. This
 * one was validated to produce byte-identical output to a whole-document parse
 * at *every character prefix* of a corpus of cross-block constructs.
 */

export type StreamingBlockEntry = {
	readonly text: string;
	readonly nodes: readonly Descendant[];
	/** The deserializer threw and the raw text stands in as a paragraph. */
	readonly fallback: boolean;
};

export type StreamingParseState = {
	readonly source: string;
	readonly entries: readonly StreamingBlockEntry[];
	readonly blocks: Value;
	/** Flattened index of the first block that was re-parsed this update. */
	readonly firstChangedBlock: number;
};

const EMPTY_PARAGRAPH = (): Value => [{ type: "p", children: [{ text: "" }] }];

/**
 * True when every block came from the per-block parser — the result a settled
 * editor may adopt as-is. A whole-document fallback leaves `entries` empty; a
 * block the deserializer rejected is flagged.
 */
export function isCompleteParse(state: StreamingParseState): boolean {
	return (
		state.entries.length > 0 && state.entries.every((entry) => !entry.fallback)
	);
}

/** `source` is a sentinel no real content can equal, so the first parse runs. */
export const EMPTY_STREAMING_STATE: StreamingParseState = {
	source: "\u0000",
	entries: [],
	blocks: EMPTY_PARAGRAPH(),
	firstChangedBlock: 0,
};

const DEFINITION = /^ {0,3}\[[^\]]+\]:\s/m;
const REFERENCE_USE = /\]\[[^[\]]*\]|\[\^[^[\]]+\]/;
const HTML_BLOCK = /^ {0,3}<[a-zA-Z!/?]/m;

/**
 * Link-reference definitions, footnotes and HTML blocks all resolve across
 * block boundaries, so a document containing them cannot be parsed piecewise.
 * Those fall back to a whole-document parse — exactly the previous behaviour.
 */
export function isBlockCacheable(markdown: string): boolean {
	if (DEFINITION.test(markdown) && REFERENCE_USE.test(markdown)) return false;
	return !HTML_BLOCK.test(markdown);
}

const FENCE = /^(\s*)(`{3,}|~{3,})/;
const ORDERED_ITEM = /^ {0,3}\d{1,9}([.)](\s|$)|$)/;

const isBlank = (line: string) => line.trim() === "";
const isIndented = (line: string) => /^\s/.test(line) && !isBlank(line);

/**
 * Splits already-directive-preprocessed markdown into top-level blocks whose
 * parse does not depend on surrounding text.
 *
 * Blocks are never trimmed — leading whitespace carries list indentation — and
 * an unterminated fence swallows the rest of the input so a half-received code
 * block is always one tail block rather than being shredded into paragraphs.
 */
export function splitStreamingBlocks(markdown: string): string[] {
	const lines = markdown.split("\n");
	const blocks: string[] = [];
	let current: string[] = [];
	let fenceMarker: string | null = null;
	let fenceIndented = false;

	const flush = () => {
		if (current.length && current.join("\n").trim() !== "")
			blocks.push(current.join("\n"));
		current = [];
	};

	for (let i = 0; i < lines.length; i++) {
		const line = lines[i];

		if (fenceMarker) {
			current.push(line);
			const closing = FENCE.exec(line);
			if (
				closing &&
				closing[2][0] === fenceMarker[0] &&
				closing[2].length >= fenceMarker.length
			) {
				fenceMarker = null;
				// An indented fence belongs to the list item that opened it, so the
				// block continues past the closing marker.
				if (!fenceIndented) flush();
			}
			continue;
		}

		const opening = FENCE.exec(line);
		if (opening) {
			fenceIndented = opening[1].length > 0;
			// A fence opener only starts a new block when it is at the margin;
			// indented, it is a continuation of the current list item.
			if (!fenceIndented) flush();
			fenceMarker = opening[2];
			current.push(line);
			continue;
		}

		if (isBlank(line)) {
			const next = lines.slice(i + 1).find((candidate) => !isBlank(candidate));
			// A trailing blank run stays with the current block so its text is
			// byte-stable as more of the stream arrives.
			if (next === undefined || isIndented(next)) {
				current.push(line);
				continue;
			}
			flush();
			continue;
		}

		current.push(line);
	}

	flush();
	return mergeOrderedListRuns(blocks);
}

/**
 * An ordered list's numbering comes from the list, not from the literal marker,
 * so `1.` / `1.` / `1.` separated by blank lines must parse as one run or the
 * items all render as "1".
 */
function mergeOrderedListRuns(blocks: readonly string[]): string[] {
	const merged: string[] = [];

	for (const block of blocks) {
		const previous = merged[merged.length - 1];
		if (previous && startsOrderedItem(previous) && startsOrderedItem(block)) {
			merged[merged.length - 1] = `${previous}\n\n${block}`;
			continue;
		}
		merged.push(block);
	}

	return merged;
}

function startsOrderedItem(block: string): boolean {
	const first = block.split("\n").find((line) => !isBlank(line));
	return first !== undefined && ORDERED_ITEM.test(first);
}

function parseBlock(worker: unknown, text: string): StreamingBlockEntry {
	try {
		const nodes = deserializeMarkdown(worker, text, RICH_REMARK_PLUGINS);
		// A block may legitimately yield zero nodes (a definition-only block);
		// injecting a placeholder paragraph here would desync from a whole-document
		// parse, so an empty result is passed through as-is.
		return {
			text,
			nodes: transformSpecialLinks(nodes) as unknown as readonly Descendant[],
			fallback: false,
		};
	} catch {
		return {
			text,
			nodes: [
				{ type: "p", children: [{ text }] },
			] as unknown as readonly Descendant[],
			fallback: true,
		};
	}
}

/** Restores object identity for blocks that are unchanged but were re-parsed. */
function reuseIdenticalBlocks(
	next: readonly Descendant[],
	previous: readonly Descendant[],
): { blocks: Value; firstChanged: number } {
	const blocks: Descendant[] = [];
	let firstChanged = next.length;

	for (let i = 0; i < next.length; i++) {
		const before = previous[i];
		if (
			firstChanged === next.length &&
			before !== undefined &&
			JSON.stringify(before) === JSON.stringify(next[i])
		) {
			blocks.push(before);
			continue;
		}
		if (firstChanged === next.length) firstChanged = i;
		blocks.push(next[i]);
	}

	return { blocks: blocks as Value, firstChanged };
}

/**
 * Parses `content` reusing everything unchanged since `previous`.
 *
 * Idempotent: calling it again with the same `content` and a state derived from
 * that same `content` matches every entry by text and returns the identical
 * objects, so a discarded render cannot corrupt the document.
 */
export function parseStreamingMarkdown(
	worker: unknown,
	content: string,
	previous: StreamingParseState,
): StreamingParseState {
	if (content === previous.source) return previous;

	if (!content) {
		return {
			source: content,
			entries: [],
			blocks: EMPTY_PARAGRAPH(),
			firstChangedBlock: 0,
		};
	}

	const preprocessed = preprocessDirectiveBlocks(content);

	if (!isBlockCacheable(preprocessed)) {
		const parsed = safeDeserialize(
			worker,
			content,
			true,
			RICH_REMARK_PLUGINS,
		) as readonly Descendant[];
		const { blocks, firstChanged } = reuseIdenticalBlocks(
			parsed,
			previous.blocks,
		);
		return {
			source: content,
			entries: [],
			blocks: blocks.length ? blocks : EMPTY_PARAGRAPH(),
			firstChangedBlock: firstChanged,
		};
	}

	const texts = splitStreamingBlocks(preprocessed);
	const entries: StreamingBlockEntry[] = [];
	let reusedThrough = 0;

	// Reuse stops permanently at the first mismatch: keeping every cached object
	// at exactly one position is what makes the path index and Slate's
	// node-keyed maps sound.
	while (
		reusedThrough < texts.length &&
		previous.entries[reusedThrough]?.text === texts[reusedThrough]
	) {
		entries.push(previous.entries[reusedThrough]);
		reusedThrough++;
	}

	let firstChangedBlock = 0;
	for (let i = 0; i < reusedThrough; i++)
		firstChangedBlock += entries[i].nodes.length;

	for (let i = reusedThrough; i < texts.length; i++)
		entries.push(parseBlock(worker, texts[i]));

	const blocks = entries.flatMap((entry) => entry.nodes as Descendant[]);

	return {
		source: content,
		entries,
		blocks: (blocks.length ? blocks : EMPTY_PARAGRAPH()) as Value,
		firstChangedBlock: blocks.length ? firstChangedBlock : 0,
	};
}
