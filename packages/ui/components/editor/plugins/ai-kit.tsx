"use client";

import { withAIBatch } from "@platejs/ai";
import {
	AIChatPlugin,
	AIPlugin,
	streamInsertChunk,
	useChatChunk,
} from "@platejs/ai/react";
import { KEYS, PathApi } from "platejs";
import { usePluginOption } from "platejs/react";

import { AILoadingBar, AIMenu } from "../ui/ai-menu";
import { AIAnchorElement, AILeaf } from "../ui/ai-node";

import { CursorOverlayKit } from "./cursor-overlay-kit";
import { MarkdownKit } from "./markdown-kit";

export const aiChatPlugin = AIChatPlugin.extend({
	render: {
		afterContainer: AILoadingBar,
		afterEditable: AIMenu,
		node: AIAnchorElement,
	},
	shortcuts: { show: { keys: "mod+j" } },
	useHooks: ({ editor, getOption }) => {
		const mode = usePluginOption(AIChatPlugin, "mode");

		useChatChunk({
			onChunk: ({ chunk, isFirst, nodes }) => {
				if (isFirst && mode === "insert") {
					// Try again and follow-ups undo the answer but keep its anchor; reuse it.
					if (!editor.getApi(AIChatPlugin).aiChat.node({ anchor: true })) {
						editor.tf.withoutSaving(() => {
							editor.tf.insertNodes(
								{
									children: [{ text: "" }],
									type: KEYS.aiChat,
								},
								{
									at: PathApi.next(editor.selection!.focus.path.slice(0, 1)),
								},
							);
						});
					}
					editor.setOption(AIChatPlugin, "streaming", true);
				}

				if (mode === "insert" && nodes.length > 0) {
					withAIBatch(
						editor,
						() => {
							if (!getOption("streaming")) return;
							editor.tf.withScrolling(() => {
								streamInsertChunk(editor, chunk, {
									textProps: {
										ai: true,
									},
								});
							});
						},
						{ split: isFirst },
					);
				}
			},
			onFinish: () => {
				editor.setOptions(AIChatPlugin, {
					_blockChunks: "",
					_blockPath: null,
					_mdxName: null,
					streaming: false,
				});
			},
		});
	},
});

export const AIKit = [
	...CursorOverlayKit,
	...MarkdownKit,
	AIPlugin.withComponent(AILeaf),
	aiChatPlugin,
];
