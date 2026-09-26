"use client";

import type { PlateEditor } from "platejs/react";

import { insertCallout } from "@platejs/callout";
import { insertCodeBlock, toggleCodeBlock } from "@platejs/code-block";
import { insertDate } from "@platejs/date";
import { insertColumnGroup, toggleColumnGroup } from "@platejs/layout";
import { triggerFloatingLink } from "@platejs/link/react";
import { insertEquation, insertInlineEquation } from "@platejs/math";
import {
	insertAudioPlaceholder,
	insertFilePlaceholder,
	insertMedia,
	insertVideoPlaceholder,
} from "@platejs/media";
import { SuggestionPlugin } from "@platejs/suggestion/react";
import { TablePlugin } from "@platejs/table/react";
import { insertToc } from "@platejs/toc";
import {
	KEYS,
	type NodeEntry,
	type Path,
	PathApi,
	type TElement,
	type TRange,
} from "platejs";

const ACTION_THREE_COLUMNS = "action_three_columns";

const insertList = (editor: PlateEditor, type: string) => {
	editor.tf.insertNodes(
		editor.api.create.block({
			indent: 1,
			listStyleType: type,
		}),
		{ select: true },
	);
};

const insertBlockMap: Record<
	string,
	(editor: PlateEditor, type: string) => void
> = {
	[KEYS.listTodo]: insertList,
	[KEYS.ol]: insertList,
	[KEYS.ul]: insertList,
	[ACTION_THREE_COLUMNS]: (editor) =>
		insertColumnGroup(editor, { columns: 3, select: true }),
	[KEYS.audio]: (editor) => insertAudioPlaceholder(editor, { select: true }),
	[KEYS.callout]: (editor) => insertCallout(editor, { select: true }),
	[KEYS.codeBlock]: (editor) => insertCodeBlock(editor, { select: true }),
	[KEYS.equation]: (editor) => insertEquation(editor, { select: true }),
	[KEYS.file]: (editor) => insertFilePlaceholder(editor, { select: true }),
	[KEYS.img]: (editor) =>
		insertMedia(editor, {
			select: true,
			type: KEYS.img,
		}),
	[KEYS.mediaEmbed]: (editor) =>
		insertMedia(editor, {
			select: true,
			type: KEYS.mediaEmbed,
		}),
	[KEYS.table]: (editor) =>
		editor.getTransforms(TablePlugin).insert.table({}, { select: true }),
	[KEYS.toc]: (editor) => insertToc(editor, { select: true }),
	[KEYS.video]: (editor) => insertVideoPlaceholder(editor, { select: true }),
};

const insertInlineMap: Record<
	string,
	(editor: PlateEditor, type: string) => void
> = {
	[KEYS.date]: (editor) => insertDate(editor, { select: true }),
	[KEYS.inlineEquation]: (editor) =>
		insertInlineEquation(editor, "", { select: true }),
	[KEYS.link]: (editor) => triggerFloatingLink(editor, { focused: true }),
};

export const insertBlock = (editor: PlateEditor, type: string) => {
	editor.tf.withoutNormalizing(() => {
		const outerQuote =
			type === editor.getType(KEYS.blockquote)
				? editor.api.above<TElement>({ match: { type }, mode: "highest" })
				: undefined;
		const block = outerQuote ?? editor.api.block();

		if (!block) return;
		if (type in insertBlockMap) {
			insertBlockMap[type](editor, type);
		} else {
			editor.tf.insertNodes(editor.api.create.block({ type }), {
				at: PathApi.next(block[1]),
				select: true,
			});
		}
		if (getBlockType(block[0]) !== type) {
			editor.getApi(SuggestionPlugin).suggestion.withoutSuggestions(() => {
				editor.tf.removeNodes({ previousEmptyBlock: true });
			});
		}
	});
};

export const insertInlineElement = (editor: PlateEditor, type: string) => {
	if (insertInlineMap[type]) {
		insertInlineMap[type](editor, type);
	}
};

const setList = (
	editor: PlateEditor,
	type: string,
	entry: NodeEntry<TElement>,
) => {
	editor.tf.setNodes(
		editor.api.create.block({
			indent: 1,
			listStyleType: type,
		}),
		{
			at: entry[1],
		},
	);
};

const setBlockMap: Record<
	string,
	(editor: PlateEditor, type: string, entry: NodeEntry<TElement>) => void
> = {
	[KEYS.listTodo]: setList,
	[KEYS.ol]: setList,
	[KEYS.ul]: setList,
	[ACTION_THREE_COLUMNS]: (editor) => toggleColumnGroup(editor, { columns: 3 }),
	[KEYS.codeBlock]: (editor) => toggleCodeBlock(editor),
};

const liftOutOfBlockquotes = (editor: PlateEditor, at: Path | TRange) => {
	editor.tf.unwrapNodes({
		at,
		match: { type: editor.getType(KEYS.blockquote) },
		mode: "all",
		split: true,
	});
};

/**
 * Plate 49 quotes were flat blocks, so turning one into another type replaced
 * it. Plate 53 quotes are containers: the block leaves the quote instead, and
 * a block already inside a quote is not quoted again.
 */
export const setBlockType = (
	editor: PlateEditor,
	type: string,
	{ at }: { at?: Path } = {},
) => {
	editor.tf.withoutNormalizing(() => {
		const blockquoteType = editor.getType(KEYS.blockquote);
		const quoting = type === blockquoteType;

		const setEntry = (entry: NodeEntry<TElement>) => {
			const [node, path] = entry;

			if (quoting && editor.api.above({ at: path, match: { type } })) return;
			if (node[KEYS.listType]) {
				editor.tf.unsetNodes([KEYS.listType, "indent"], { at: path });
			}
			if (type in setBlockMap) {
				return setBlockMap[type](editor, type, entry);
			}
			if (node.type !== type) {
				editor.tf.setNodes({ type }, { at: path });
			}
		};

		if (at && editor.api.node(at)) {
			const pathRef = editor.api.pathRef(at);
			if (!quoting) liftOutOfBlockquotes(editor, at);
			const path = pathRef.unref();
			const entry = path && editor.api.node<TElement>(path);
			if (entry) setEntry(entry);

			return;
		}

		if (!quoting && editor.selection) {
			liftOutOfBlockquotes(editor, editor.selection);
		}

		for (const entry of editor.api.blocks<TElement>({ mode: "lowest" })) {
			setEntry(entry);
		}
	});
};

const TEXT_BLOCK_TYPES = new Set<string>([KEYS.p, ...KEYS.heading]);

/**
 * Block menu "Turn into". On a Plate 49 flat quote `toggleBlock` changed the
 * quote's type; a Plate 53 container quote is unwrapped instead, its text
 * blocks taking the type the flat quote would have.
 */
export const toggleBlockAt = (
	editor: PlateEditor,
	type: string,
	path: Path,
) => {
	const node = editor.api.node<TElement>(path)?.[0];
	if (!node) return;

	editor.tf.withoutNormalizing(() => {
		if (node[KEYS.listType]) {
			editor.tf.unsetNodes([KEYS.listType, "indent"], { at: path });
		}

		const blockquoteType = editor.getType(KEYS.blockquote);
		const isContainerQuote =
			node.type === blockquoteType &&
			node.children.some((child) => editor.api.isBlock(child));

		if (!isContainerQuote) {
			editor.tf.toggleBlock(type, { at: path });
			return;
		}

		const target = type === blockquoteType ? editor.getType(KEYS.p) : type;
		const children = node.children as TElement[];

		editor.tf.unwrapNodes({ at: path });
		children.forEach((child, index) => {
			if (!TEXT_BLOCK_TYPES.has(child.type) || child.type === target) return;
			editor.tf.setNodes(
				{ type: target },
				{ at: [...PathApi.parent(path), path[path.length - 1] + index] },
			);
		});
	});
};

export const getBlockType = (block: TElement) => {
	if (block[KEYS.listType]) {
		if (block[KEYS.listType] === KEYS.ol) {
			return KEYS.ol;
		}
		if (block[KEYS.listType] === KEYS.listTodo) {
			return KEYS.listTodo;
		}
		return KEYS.ul;
	}

	return block.type;
};
