"use client";

import { HeadingRules } from "@platejs/basic-nodes";
import {
	BlockquotePlugin,
	H1Plugin,
	H2Plugin,
	H3Plugin,
	HorizontalRulePlugin,
} from "@platejs/basic-nodes/react";
import {
	type BlockStartInputRuleMatch,
	ElementApi,
	KEYS,
	type SelectionInputRuleContext,
	type SlateEditor,
	createBlockStartInputRule,
} from "platejs";
import { ParagraphPlugin } from "platejs/react";

import { BlockquoteElement } from "../ui/blockquote-node";
import { H1Element, H2Element, H3Element } from "../ui/heading-node";
import { HrElement } from "../ui/hr-node";
import { ParagraphElement } from "../ui/paragraph-node";
import { notInside } from "./input-rule-guards";

const blockquoteMarkdownRule = createBlockStartInputRule({
	enabled: notInside(KEYS.codeBlock, KEYS.blockquote),
	match: ">",
	trigger: " ",
	apply: ({ editor }, { range }) => {
		editor.tf.delete({ at: range });
		editor.tf.wrapNodes(
			{ children: [], type: editor.getType(KEYS.blockquote) },
			{ match: (node) => editor.api.isBlock(node) },
		);
		return true;
	},
});

const insertHorizontalRule = (
	{ editor }: { editor: SlateEditor },
	{ range }: BlockStartInputRuleMatch,
) => {
	editor.tf.delete({ at: range });
	editor.tf.setNodes({ type: editor.getType(KEYS.hr) });
	editor.tf.insertNodes({
		children: [{ text: "" }],
		type: editor.getType(KEYS.p),
	});
	return true;
};

const notInsideCodeBlock = notInside(KEYS.codeBlock);

// Plate 53 resets a heading to a paragraph on Backspace at its start; 49 merged it into the block above.
const headingRules = {
	break: { empty: "reset" },
	delete: { start: "default" },
} as const;

/** Only at the block end: the hr void would otherwise swallow the text after the cursor. */
const horizontalRuleEnabled = (context: SelectionInputRuleContext) =>
	notInsideCodeBlock(context) && context.getCharAfter() === undefined;

// "--" is already "—" when the third dash arrives (autoformat-kit).
const horizontalRuleMarkdownRules = [
	createBlockStartInputRule({
		apply: insertHorizontalRule,
		enabled: horizontalRuleEnabled,
		match: /^(--|—)$/,
		trigger: "-",
	}),
	createBlockStartInputRule({
		apply: insertHorizontalRule,
		enabled: horizontalRuleEnabled,
		match: "___",
		trigger: " ",
	}),
];

export const BasicBlocksKit = [
	ParagraphPlugin.withComponent(ParagraphElement),
	H1Plugin.configure({
		inputRules: [
			HeadingRules.markdown({ enabled: notInside(KEYS.codeBlock, KEYS.h1) }),
		],
		node: {
			component: H1Element,
		},
		rules: headingRules,
		shortcuts: { toggle: { keys: "mod+alt+1" } },
	}),
	H2Plugin.configure({
		inputRules: [
			HeadingRules.markdown({ enabled: notInside(KEYS.codeBlock, KEYS.h2) }),
		],
		node: {
			component: H2Element,
		},
		rules: headingRules,
		shortcuts: { toggle: { keys: "mod+alt+2" } },
	}),
	H3Plugin.configure({
		inputRules: [
			HeadingRules.markdown({ enabled: notInside(KEYS.codeBlock, KEYS.h3) }),
		],
		node: {
			component: H3Element,
		},
		rules: headingRules,
		shortcuts: { toggle: { keys: "mod+alt+3" } },
	}),
	BlockquotePlugin.configure({
		inputRules: [blockquoteMarkdownRule],
		node: { component: BlockquoteElement },
		shortcuts: { toggle: { keys: "mod+shift+period" } },
	}).overrideEditor(({ editor, tf: { normalizeNode }, type }) => ({
		transforms: {
			// The stock normalizer replaces a legacy flat quote's children, which
			// drops the selection: every keystroke after the first was lost.
			normalizeNode(entry) {
				const [node, path] = entry;
				if (
					ElementApi.isElement(node) &&
					node.type === type &&
					node.children.every(
						(child) =>
							!ElementApi.isElement(child) || editor.api.isInline(child),
					)
				) {
					editor.tf.wrapNodes(
						{ children: [], type: editor.getType(KEYS.p) },
						{
							at: path,
							match: (_child, childPath) =>
								childPath.length === path.length + 1,
						},
					);
					return;
				}
				normalizeNode(entry);
			},
		},
	})),
	HorizontalRulePlugin.configure({
		inputRules: horizontalRuleMarkdownRules,
		node: { component: HrElement },
	}),
];
