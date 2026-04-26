"use client";

import { MarkdownPlugin, remarkMdx, remarkMention } from "@platejs/markdown";
import { KEYS, type Value } from "platejs";
import { Plate, PlateContent, usePlateEditor } from "platejs/react";
import { memo, useEffect, useRef } from "react";
import remarkBreaks from "remark-breaks";
import remarkEmoji from "remark-emoji";
import remarkGfm from "remark-gfm";
import remarkMath from "remark-math";
import { BaseEditorKit } from "../editor/editor-base-kit";
import { remarkFocusNodes } from "../editor/plugins/remark-focus-nodes";
import { remarkInlineSpoiler } from "../editor/plugins/remark-inline-spoiler";
import { remarkUserMention } from "../editor/plugins/remark-user-mention";
import { safeDeserialize } from "./text-editor";

const streamingRemarkPlugins = [
	remarkMath,
	remarkGfm,
	remarkBreaks,
	remarkMdx,
	remarkMention,
	remarkEmoji as any,
	remarkFocusNodes,
	remarkUserMention,
	remarkInlineSpoiler,
];

const EMPTY_VALUE: Value = [{ type: "p", children: [{ text: "" }] }];

function buildPlugins() {
	return [
		...BaseEditorKit.filter((p) => (p as any).key !== MarkdownPlugin.key),
		MarkdownPlugin.configure({
			options: {
				disallowedNodes: [KEYS.suggestion],
				remarkPlugins: streamingRemarkPlugins,
			},
		}),
	];
}

let pluginsCache: ReturnType<typeof buildPlugins> | null = null;
function getPlugins() {
	if (!pluginsCache) pluginsCache = buildPlugins();
	return pluginsCache;
}

function StreamingTextEditorInner({
	content,
	onFocusNode,
	onUserMention,
}: Readonly<{
	content: string;
	onFocusNode?: (nodeId: string) => void;
	onUserMention?: (sub: string) => void;
}>) {
	const lastContentRef = useRef("");

	const editor = usePlateEditor({
		plugins: getPlugins(),
		value: EMPTY_VALUE,
	});

	useEffect(() => {
		if (content === lastContentRef.current) return;
		lastContentRef.current = content;

		if (!content) {
			editor.tf.setValue(EMPTY_VALUE);
			return;
		}

		const nodes = safeDeserialize(
			editor,
			content,
			true,
			streamingRemarkPlugins,
		);
		editor.tf.setValue(nodes);
	}, [content, editor]);

	return (
		<div
			onClick={(e) => {
				const target = e.target as HTMLElement;
				const focusSpan = target.closest("[data-focus-node-id]");
				if (focusSpan && onFocusNode) {
					e.preventDefault();
					const nodeId = focusSpan.getAttribute("data-focus-node-id");
					if (nodeId) onFocusNode(nodeId);
				}
				const userMentionSpan = target.closest("[data-user-mention-sub]");
				if (userMentionSpan && onUserMention) {
					e.preventDefault();
					const sub = userMentionSpan.getAttribute("data-user-mention-sub");
					if (sub) onUserMention(sub);
				}
			}}
			className="overflow-hidden [&_pre]:overflow-x-auto [&_pre]:whitespace-pre-wrap [&_code]:wrap-break-word"
		>
			<Plate editor={editor} readOnly>
				<PlateContent
					readOnly
					className="py-0 outline-none [&>*:first-child_h1]:mt-0 [&>*:first-child_h2]:mt-0 [&>*:first-child_h3]:mt-0 [&>*:first-child_h4]:mt-0 [&>*:first-child_h5]:mt-0 [&>*:first-child_h6]:mt-0"
				/>
			</Plate>
		</div>
	);
}

export const StreamingTextEditor = memo(
	StreamingTextEditorInner,
	(prev, next) => prev.content === next.content,
);
StreamingTextEditor.displayName = "StreamingTextEditor";
