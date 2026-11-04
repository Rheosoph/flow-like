"use client";

import { type Value, createSlateEditor } from "platejs";
import { PlateStatic } from "platejs/static";
import { memo, useMemo, useRef } from "react";
import { BaseEditorKit } from "../editor/editor-base-kit";
import {
	IsStreamingMarkdownContext,
	type RendererPresentation,
	RendererPresentationContext,
} from "../editor/renderer-presentation";
import { normalizeStreamingEditorialImage } from "../editor/streaming-editorial-image";
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
	presentation = "default",
	onFocusNode,
	onUserMention,
}: Readonly<{
	content: string;
	presentation?: RendererPresentation;
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
	const publishedSourceRef = useRef<string | undefined>(undefined);

	// Parsing during render rather than in an effect is deliberate: the chat
	// asserts that a content update is visible in the same commit. Assigning
	// `editor.children` here is safe because parseStreamingMarkdown is
	// idempotent — re-running it for the same content returns the same objects.
	useMemo(() => {
		const previous = stateRef.current;
		const display = normalizeStreamingEditorialImage(content);
		const next = parseStreamingMarkdown(
			getStaticParseWorker(BaseEditorKit),
			display.markdown,
			previous,
		);
		stateRef.current = next;
		editor.children = next.blocks;
		indexEditorPaths(editor, next.firstChangedBlock);
		// Published on every parse, not on unmount: the settled editor for this
		// content renders in the same pass that drops the streaming one, before
		// any cleanup would run.
		// A synthetic image tail is only a streaming preview. A stopped response
		// must still parse its original Markdown when it settles.
		if (!display.pending) {
			publishSettledParse(
				content,
				{ value: next.blocks, complete: isCompleteParse(next) },
				publishedSourceRef.current,
			);
			publishedSourceRef.current = content;
		}
	}, [content, editor]);

	return (
		<RendererPresentationContext.Provider value={presentation}>
			<IsStreamingMarkdownContext.Provider value={true}>
				<div
					{...createStaticProseHandlers({ onFocusNode, onUserMention })}
					className={PROSE_WRAPPER_CLASSNAME}
					data-fl-renderer={
						presentation === "default" ? undefined : presentation
					}
				>
					<PlateStatic editor={editor} className={STATIC_EDITOR_CLASSNAME} />
				</div>
			</IsStreamingMarkdownContext.Provider>
		</RendererPresentationContext.Provider>
	);
}

export const StreamingTextEditor = memo(
	StreamingTextEditorInner,
	(prev, next) =>
		prev.content === next.content &&
		prev.presentation === next.presentation &&
		prev.onFocusNode === next.onFocusNode &&
		prev.onUserMention === next.onUserMention,
);
StreamingTextEditor.displayName = "StreamingTextEditor";
