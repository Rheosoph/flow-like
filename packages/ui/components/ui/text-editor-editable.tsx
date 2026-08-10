"use client";

import { remarkMdx, remarkMention } from "@platejs/markdown";
import { Plate, usePlateEditor } from "platejs/react";
import { useContext, useEffect, useMemo, useRef, useState } from "react";
import remarkBreaks from "remark-breaks";
import remarkEmoji from "remark-emoji";
import remarkGfm from "remark-gfm";
import remarkMath from "remark-math";
import { AIUsageAppContext } from "../editor/ai-usage-context";
import { createEditorKit } from "../editor/editor-kit";
import { remarkFocusNodes } from "../editor/plugins/remark-focus-nodes";
import { remarkInlineSpoiler } from "../editor/plugins/remark-inline-spoiler";
import { remarkUserMention } from "../editor/plugins/remark-user-mention";
import { Editor, EditorContainer } from "../editor/ui/editor";
import { PLATE_JSON_PREFIX, safeDeserialize } from "./text-editor";

/**
 * The editable half of `TextEditor`, split out so that read-only markdown does not carry it.
 *
 * `createEditorKit` assembles three dozen plugin kits — AI, media, tables, math, code
 * highlighting, the toolbars — none of which a rendered document touches. Read-only surfaces
 * (a2ui pages, chat transcripts, database previews) far outnumber editors, so this module is
 * reached through a dynamic import and never appears in their first load.
 */
export function TextEditorEditable({
	initialContent,
	onChange,
	isMarkdown,
}: Readonly<{
	initialContent: string;
	onChange: (content: string) => void;
	isMarkdown?: boolean;
	onFocusNode?: (nodeId: string) => void;
}>) {
	const appId = useContext(AIUsageAppContext);
	const editorPlugins = useMemo(() => createEditorKit(appId), [appId]);
	const remarkPlugins = useMemo(
		() => [
			[remarkMath, { singleDollarTextMath: false }],
			remarkGfm,
			remarkBreaks,
			remarkMdx,
			remarkMention,
			remarkEmoji as unknown,
			remarkFocusNodes,
			remarkUserMention,
			remarkInlineSpoiler,
		],
		[],
	);
	const lastEmittedContentRef = useRef(initialContent);
	const [editorSeed, setEditorSeed] = useState(initialContent);

	useEffect(() => {
		if (initialContent === lastEmittedContentRef.current) {
			return;
		}

		lastEmittedContentRef.current = initialContent;
		setEditorSeed(initialContent);
	}, [initialContent]);

	const editor = usePlateEditor(
		{
			id: "rendered-editor",
			plugins: editorPlugins,
			value: (self) =>
				safeDeserialize(self, editorSeed, isMarkdown ?? false, remarkPlugins),
		},
		[editorSeed, isMarkdown, remarkPlugins, editorPlugins],
	);

	return (
		<Plate
			editor={editor}
			onChange={({ editor }) => {
				const serializedNodes = editor.children;
				const newContent = `${PLATE_JSON_PREFIX}${JSON.stringify(
					serializedNodes,
				)}`;

				if (newContent === lastEmittedContentRef.current) {
					return;
				}
				lastEmittedContentRef.current = newContent;
				onChange(newContent);
			}}
		>
			<EditorContainer>
				<Editor variant="none" className="px-4 py-2" />
			</EditorContainer>
		</Plate>
	);
}
