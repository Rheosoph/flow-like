"use client";

import { insertEmptyCodeBlock } from "@platejs/code-block";
import {
	CodeBlockPlugin,
	CodeLinePlugin,
	CodeSyntaxPlugin,
} from "@platejs/code-block/react";
import { KEYS, createBlockStartInputRule } from "platejs";
import {
	CodeBlockElement,
	CodeLineElement,
	CodeSyntaxLeaf,
} from "../ui/code-block-node";
import { createEditorLowlight } from "./code-block-lowlight";
import { notInside } from "./input-rule-guards";

const lowlight = createEditorLowlight();

const codeFenceRule = createBlockStartInputRule({
	enabled: notInside(KEYS.codeBlock),
	match: "``",
	trigger: "`",
	apply: ({ editor }, { range }) => {
		editor.tf.delete({ at: range });
		insertEmptyCodeBlock(editor, {
			defaultType: editor.getType(KEYS.p),
			insertNodesOptions: { select: true },
		});
		return true;
	},
});

export const CodeBlockKit = [
	CodeBlockPlugin.configure({
		inputRules: [codeFenceRule],
		node: { component: CodeBlockElement },
		options: { lowlight },
		shortcuts: { toggle: { keys: "mod+alt+8" } },
	}),
	CodeLinePlugin.withComponent(CodeLineElement),
	CodeSyntaxPlugin.withComponent(CodeSyntaxLeaf),
];
