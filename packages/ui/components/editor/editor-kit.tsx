"use client";

import { TrailingBlockPlugin, type Value } from "platejs";
import { type TPlateEditor, useEditorRef } from "platejs/react";

import { AIKit } from "./plugins/ai-kit";
import { AlignKit } from "./plugins/align-kit";
import { AutoformatKit } from "./plugins/autoformat-kit";
import { BasicBlocksKit } from "./plugins/basic-blocks-kit";
import { BasicMarksKit } from "./plugins/basic-marks-kit";
import { BlockMenuKit } from "./plugins/block-menu-kit";
import { BlockPlaceholderKit } from "./plugins/block-placeholder-kit";
import { CalloutKit } from "./plugins/callout-kit";
import { CodeBlockKit } from "./plugins/code-block-kit";
import { ColumnKit } from "./plugins/column-kit";
import { CommentKit } from "./plugins/comment-kit";
import { createCopilotKit } from "./plugins/copilot-kit";
import { CursorOverlayKit } from "./plugins/cursor-overlay-kit";
import { DateKit } from "./plugins/date-kit";
import { discussionPlugin } from "./plugins/discussion-kit";
import type { EditorUser, ReviewDiscussion } from "../../lib/plate-document";
import { DndKit } from "./plugins/dnd-kit";
import { DocxKit } from "./plugins/docx-kit";
import { EmojiKit } from "./plugins/emoji-kit";
import { ExitBreakKit } from "./plugins/exit-break-kit";
import { FixedToolbarKit } from "./plugins/fixed-toolbar-kit";
import { FloatingToolbarKit } from "./plugins/floating-toolbar-kit";
import { FootnoteKit } from "./plugins/footnote-kit";
import { FontKit } from "./plugins/font-kit";
import { LineHeightKit } from "./plugins/line-height-kit";
import { LinkKit } from "./plugins/link-kit";
import { ListKit } from "./plugins/list-kit";
import { MarkdownKit } from "./plugins/markdown-kit";
import { MathKit } from "./plugins/math-kit";
import { MediaKit } from "./plugins/media-kit";
import { MentionKit } from "./plugins/mention-kit";
import { SafeUrlKit } from "./plugins/safe-url-kit";
import { SlashKit } from "./plugins/slash-kit";
import { SuggestionKit } from "./plugins/suggestion-kit";
import { TableKit } from "./plugins/table-kit";
import { TocKit } from "./plugins/toc-kit";
import { ToggleKit } from "./plugins/toggle-kit";

export interface EditorReviewOptions {
	contentReadOnly?: boolean;
	documentId?: string;
	legacy?: boolean;
	currentUser?: EditorUser;
	enabled?: boolean;
	canModerate?: boolean;
	discussions?: ReviewDiscussion[];
	users?: Record<string, EditorUser>;
}

export const createEditorKit = (
	appId?: string,
	review: EditorReviewOptions = {},
) => [
	...AIKit,
	...createCopilotKit(appId),
	...(review.contentReadOnly ? [] : BlockMenuKit),

	// Elements
	...BasicBlocksKit,
	...CodeBlockKit,
	...TableKit,
	...ToggleKit,
	...TocKit,
	...MediaKit,
	...CalloutKit,
	...ColumnKit,
	...MathKit,
	...DateKit,
	...FootnoteKit,
	...LinkKit,
	...MentionKit,
	...SafeUrlKit,

	// Marks
	...BasicMarksKit,
	...FontKit,

	// Block Style
	...ListKit,
	...AlignKit,
	...LineHeightKit,

	// Collaboration
	discussionPlugin.configure({
		options: {
			inline: !review.contentReadOnly,
			documentId: review.documentId,
			legacy: review.legacy ?? true,
			currentUserId: review.currentUser?.id ?? "",
			enabled: review.enabled ?? Boolean(review.currentUser),
			canModerate: review.canModerate ?? false,
			discussions: review.discussions ?? [],
			users: {
				...review.users,
				...(review.currentUser
					? { [review.currentUser.id]: review.currentUser }
					: {}),
			},
		},
	}),
	...CommentKit,
	...SuggestionKit,

	// Editing
	...SlashKit,
	...AutoformatKit,
	...CursorOverlayKit,
	...(review.contentReadOnly ? [] : DndKit),
	...EmojiKit,
	...ExitBreakKit,
	TrailingBlockPlugin,

	// Parsers
	...DocxKit,
	...MarkdownKit,

	// UI
	...BlockPlaceholderKit,
	...(review.contentReadOnly ? [] : FixedToolbarKit),
	...(review.contentReadOnly ? [] : FloatingToolbarKit),
];

export const EditorKit = createEditorKit();

export type MyEditor = TPlateEditor<
	Value,
	ReturnType<typeof createEditorKit>[number]
>;

export const useEditor = () => useEditorRef<MyEditor>();
