"use client";

import {
	type Descendant,
	ElementApi,
	ElementStatic,
	LeafStatic,
	NodeApi,
	type Path,
	type SlateEditor,
	pipeDecorate,
} from "platejs";
import type * as React from "react";
import {
	Fragment,
	memo,
	useCallback,
	useEffect,
	useMemo,
	useRef,
	useState,
} from "react";
import { cn } from "../../lib/utils";

/**
 * Plate's static renderer walks the whole document in one synchronous pass and
 * costs several milliseconds per node, so a long document blocks the main
 * thread for seconds. These components render top-level blocks in
 * viewport-gated chunks instead: off-screen chunks are a single sized
 * placeholder until they scroll close, and once mounted they stay mounted so
 * scroll position never shifts under the user.
 */

const BLOCKS_PER_CHUNK = 10;

/** Chunks rendered synchronously on mount, enough to cover the first screens. */
const EAGER_CHUNKS = 4;

/** How far outside the viewport a chunk starts rendering. */
const ROOT_MARGIN = "1200px 0px";

/**
 * Documents at or below this many top-level blocks render in one pass, exactly
 * as before. Keeps chat messages, comments and short previews unchanged.
 */
export const WINDOWING_BLOCK_THRESHOLD = 40;

const CHARS_PER_LINE = 90;
const LINE_HEIGHT = 24;

/** Rough rendered height of a block, used to size placeholders. */
function estimateBlockHeight(block: Descendant): number {
	const text = NodeApi.string(block);
	const type = ElementApi.isElement(block) ? (block.type as string) : "p";

	switch (type) {
		case "code_block": {
			const lines = ElementApi.isElement(block) ? block.children.length : 1;
			return lines * 21 + 64;
		}
		case "table": {
			const rows = ElementApi.isElement(block) ? block.children.length : 1;
			return rows * 37 + 96;
		}
		case "img":
		case "video":
			return 320;
		case "h1":
			return 52;
		case "h2":
			return 44;
		case "h3":
		case "h4":
		case "h5":
		case "h6":
			return 36;
		default: {
			const lines = Math.max(1, Math.ceil(text.length / CHARS_PER_LINE));
			return lines * LINE_HEIGHT + 8;
		}
	}
}

/**
 * Plate's static renderer resolves every node's path with `editor.api.findPath`.
 * Outside the React editor the DOM-backed lookup tables are empty, so it falls
 * back to matching the node against the whole document — O(total nodes) per
 * lookup, which makes rendering a long document quadratic.
 *
 * Most of those lookups are for leaves synthesized during rendering (syntax
 * highlighting tokens, decoration splits) that are not in the document at all,
 * so they pay for a full scan and still come back empty.
 *
 * A read-only static editor never mutates its value, so every real path can be
 * indexed once up front. A miss then means the node genuinely is not in the
 * document and the answer is `undefined` — the same result the scan produces,
 * without the scan.
 *
 * Safe to call repeatedly on the same editor: the `findPath` override is
 * installed once and the index behind it is swapped, so repeated calls cannot
 * chain closures. Streaming callers that only appended blocks can pass
 * `fromIndex` to re-index just the tail — valid only when every block before it
 * kept both its object identity and its position.
 */
const PATH_INDEX = Symbol.for("flow-like.staticPathIndex");

type PathIndexState = { map: WeakMap<object, Path> };

export function indexEditorPaths(editor: SlateEditor, fromIndex = 0): void {
	const holder = editor as unknown as Record<symbol, PathIndexState | undefined>;
	let state = holder[PATH_INDEX];

	if (!state) {
		state = { map: new WeakMap<object, Path>() };
		holder[PATH_INDEX] = state;

		const installed = state;
		const fallback = editor.api.findPath;
		editor.api.findPath = (node, options) => {
			// `options` narrows the search (`at`, `match`, …); leave those to Plate.
			if (options) return fallback(node, options);
			return installed.map.get(node as object);
		};
	}

	if (fromIndex === 0) state.map = new WeakMap<object, Path>();
	const paths = state.map;

	const walk = (nodes: readonly Descendant[], base: Path) => {
		nodes.forEach((node, index) => {
			const path = [...base, index];
			paths.set(node, path);
			if (ElementApi.isElement(node)) walk(node.children, path);
		});
	};

	for (let index = fromIndex; index < editor.children.length; index++) {
		const node = editor.children[index];
		paths.set(node, [index]);
		if (ElementApi.isElement(node)) walk(node.children, [index]);
	}
}

type ChunkDescriptor = {
	readonly blocks: readonly Descendant[];
	readonly height: number;
	readonly offset: number;
};

function buildChunks(blocks: readonly Descendant[]): ChunkDescriptor[] {
	const chunks: ChunkDescriptor[] = [];
	for (let offset = 0; offset < blocks.length; offset += BLOCKS_PER_CHUNK) {
		const slice = blocks.slice(offset, offset + BLOCKS_PER_CHUNK);
		chunks.push({
			blocks: slice,
			height: slice.reduce((sum, block) => sum + estimateBlockHeight(block), 0),
			offset,
		});
	}
	return chunks;
}

type DecorateFn = NonNullable<ReturnType<typeof pipeDecorate>>;

/** Matches Plate's own fallback when no plugin contributes decorations. */
const NO_DECORATIONS: DecorateFn = () => [];

/**
 * Renders one top-level block. Mirrors Plate's own `Children` loop, except the
 * path is the block index rather than a `findPath` lookup — at the top level
 * those are the same thing, and `findPath` falls back to a full-document scan
 * under static rendering.
 */
function renderBlock(
	editor: SlateEditor,
	decorate: DecorateFn,
	block: Descendant,
	index: number,
) {
	const decorations = decorate([block, [index]]) ?? [];

	return ElementApi.isElement(block) ? (
		<ElementStatic
			key={index}
			decorate={decorate}
			decorations={decorations}
			editor={editor}
			element={block}
		/>
	) : (
		<LeafStatic
			key={index}
			decorations={decorations}
			editor={editor}
			text={block}
		/>
	);
}

const BlockChunk = memo(function BlockChunk({
	chunk,
	decorate,
	editor,
	index,
	mounted,
	onReach,
}: Readonly<{
	chunk: ChunkDescriptor;
	decorate: DecorateFn;
	editor: SlateEditor;
	index: number;
	mounted: boolean;
	onReach: (index: number) => void;
}>) {
	const placeholderRef = useRef<HTMLDivElement>(null);

	useEffect(() => {
		if (mounted) return;

		const element = placeholderRef.current;
		if (!element || typeof IntersectionObserver === "undefined") {
			onReach(index);
			return;
		}

		const observer = new IntersectionObserver(
			(entries) => {
				if (entries.some((entry) => entry.isIntersecting)) onReach(index);
			},
			{ rootMargin: ROOT_MARGIN },
		);
		observer.observe(element);

		return () => observer.disconnect();
	}, [index, mounted, onReach]);

	if (!mounted) {
		return (
			<div
				ref={placeholderRef}
				aria-hidden="true"
				data-slate-placeholder="true"
				style={{ height: chunk.height }}
			/>
		);
	}

	return (
		<Fragment>
			{chunk.blocks.map((block, offset) =>
				renderBlock(editor, decorate, block, chunk.offset + offset),
			)}
		</Fragment>
	);
});

/**
 * Drop-in replacement for Plate's `PlateStatic` that only renders blocks near
 * the viewport. Emits the same wrapper element, and mounted blocks are direct
 * children of it just like in `PlateStatic`, so `:first-of-type` style rules
 * and the document's DOM shape are unchanged.
 */
export function LazyPlateStatic({
	className,
	editor,
	style,
}: Readonly<{
	className?: string;
	editor: SlateEditor;
	style?: React.CSSProperties;
}>) {
	const decorate = useMemo(
		() => pipeDecorate(editor) ?? NO_DECORATIONS,
		[editor],
	);
	const chunks = useMemo(() => buildChunks(editor.children), [editor.children]);

	const [mounted, setMounted] = useState<ReadonlySet<number>>(
		() => new Set(chunks.map((_, index) => index).slice(0, EAGER_CHUNKS)),
	);

	// `rootMargin` only pads the viewport, not the clip rect of a scrollable
	// ancestor, so inside a scrolling dialog a chunk would mount the moment it
	// appears. Revealing the follower too keeps a chunk of runway in any
	// container. Revealing a *neighbour* rather than a prefix matters: dragging
	// the scrollbar to the end must not mount everything above it.
	const reveal = useCallback((index: number) => {
		setMounted((previous) => {
			if (previous.has(index) && previous.has(index + 1)) return previous;
			const next = new Set(previous);
			next.add(index);
			next.add(index + 1);
			return next;
		});
	}, []);

	return (
		<div
			className={cn("slate-editor", className)}
			data-slate-editor
			data-slate-node="value"
			style={style}
		>
			{chunks.map((chunk, index) => (
				<BlockChunk
					key={chunk.offset}
					chunk={chunk}
					decorate={decorate}
					editor={editor}
					index={index}
					mounted={mounted.has(index)}
					onReach={reveal}
				/>
			))}
		</div>
	);
}
