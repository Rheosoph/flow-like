"use client";

import { PlateStatic, type Value, createSlateEditor } from "platejs";
import { memo, useMemo, useRef } from "react";
import { BaseEditorKit } from "../editor/editor-base-kit";
import { indexEditorPaths } from "./lazy-plate-static";
import {
	EMPTY_STREAMING_STATE,
	isCompleteParse,
	parseStreamingMarkdown,
} from "./streaming-markdown-blocks";
import {
	PROSE_WRAPPER_CLASSNAME,
	STATIC_EDITOR_CLASSNAME,
	createStaticProseHandlers,
	getStaticParseWorker,
	publishSettledParse,
} from "./text-editor";

const EMPTY_VALUE: Value = [{ type: "p", children: [{ text: "" }] }];

function StreamingTextEditorInner({
	content,
	onFocusNode,
	onUserMention,
}: Readonly<{
	content: string;
	onFocusNode?: (nodeId: string) => void;
	onUserMention?: (sub: string) => void;
}>) {
	const editor = useMemo(
		() =>
			createSlateEditor({
				id: "streaming-rendered-editor",
				plugins: BaseEditorKit,
				value: EMPTY_VALUE,
				// Node ids are re-randomised per parse, which would defeat both the
				// block cache and the identity memo — and nothing here reads them.
				nodeId: false,
			}),
		[],
	);
	const stateRef = useRef(EMPTY_STREAMING_STATE);

	// Parsing during render rather than in an effect is deliberate: the chat
	// asserts that a content update is visible in the same commit. Assigning
	// `editor.children` here is safe because parseStreamingMarkdown is
	// idempotent — re-running it for the same content returns the same objects.
	useMemo(() => {
		const previous = stateRef.current;
		const next = parseStreamingMarkdown(
			getStaticParseWorker(BaseEditorKit),
			content,
			previous,
		);
		stateRef.current = next;
		editor.children = next.blocks;
		indexEditorPaths(editor, next.firstChangedBlock);
		// Published on every parse, not on unmount: the settled editor for this
		// content renders in the same pass that drops the streaming one, before
		// any cleanup would run.
		publishSettledParse(
			content,
			{ value: next.blocks, complete: isCompleteParse(next) },
			previous.source,
		);
	}, [content, editor]);

	return (
		<div
			{...createStaticProseHandlers({ onFocusNode, onUserMention })}
			className={PROSE_WRAPPER_CLASSNAME}
		>
			<PlateStatic editor={editor} className={STATIC_EDITOR_CLASSNAME} />
		</div>
	);
}

export const StreamingTextEditor = memo(
	StreamingTextEditorInner,
	(prev, next) =>
		prev.content === next.content &&
		prev.onFocusNode === next.onFocusNode &&
		prev.onUserMention === next.onUserMention,
);
StreamingTextEditor.displayName = "StreamingTextEditor";
