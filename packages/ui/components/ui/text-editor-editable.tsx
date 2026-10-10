"use client";

import { remarkMdx, remarkMention } from "@platejs/markdown";
import {
	Plate,
	usePlateEditor,
	usePluginOption,
	useEditorRef,
} from "platejs/react";
import {
	type HTMLAttributes,
	cloneElement,
	useCallback,
	useContext,
	useEffect,
	useMemo,
	useRef,
	useState,
} from "react";
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
import { Toolbar } from "../editor/ui/toolbar";
import { Comment, CommentCreateForm } from "../editor/ui/comment";
import { commentPlugin } from "../editor/plugins/comment-kit";
import { getDraftCommentKey } from "@platejs/comment";
import { CommentToolbarButton } from "../editor/ui/comment-toolbar-button";
import { safeDeserialize } from "./text-editor";
import {
	type EditorUser,
	parsePlateDocument,
	serializePlateDocument,
} from "../../lib/plate-document";
import {
	discussionPlugin,
	getPlateEditorDocument,
} from "../editor/plugins/discussion-kit";

/**
 * The editable half of `TextEditor`, split out so that read-only markdown does not carry it.
 *
 * `createEditorKit` assembles three dozen plugin kits — AI, media, tables, math, code
 * highlighting, the toolbars — none of which a rendered document touches. Read-only surfaces
 * (a2ui pages, chat transcripts, database previews) far outnumber editors, so this module is
 * reached through a dynamic import and never appears in their first load.
 */
export function TextEditorEditable({
	documentId,
	currentUser,
	reviewEnabled,
	reviewCanModerate,
	contentReadOnly,
	editorProps,
	initialContent,
	onChange,
	isMarkdown,
}: Readonly<{
	documentId?: string;
	currentUser?: EditorUser;
	reviewEnabled?: boolean;
	reviewCanModerate?: boolean;
	contentReadOnly?: boolean;
	editorProps?: HTMLAttributes<HTMLDivElement>;
	initialContent: string;
	onChange: (content: string) => void;
	isMarkdown?: boolean;
	onFocusNode?: (nodeId: string) => void;
}>) {
	const appId = useContext(AIUsageAppContext);

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

	const document = useMemo(() => parsePlateDocument(editorSeed), [editorSeed]);
	const editorPlugins = useMemo(
		() =>
			createEditorKit(appId, {
				currentUser,
				contentReadOnly,
				enabled: reviewEnabled ?? Boolean(currentUser),
				canModerate: reviewCanModerate,
				discussions: document?.discussions,
				users: document?.users,
				documentId: documentId ?? document?.documentId,
				legacy: document?.legacy ?? !documentId,
			}),
		[
			appId,
			currentUser,
			reviewEnabled,
			reviewCanModerate,
			document,
			documentId,
			contentReadOnly,
		],
	);
	const editor = usePlateEditor(
		{
			id: "rendered-editor",
			plugins: editorPlugins,
			value: (self) =>
				safeDeserialize(self, editorSeed, isMarkdown ?? false, remarkPlugins),
		},
		[editorSeed, isMarkdown, remarkPlugins, editorPlugins],
	);

	const emit = useCallback(() => {
		const content = serializePlateDocument(getPlateEditorDocument(editor));
		if (content === lastEmittedContentRef.current) return;
		lastEmittedContentRef.current = content;
		onChange(content);
	}, [editor, onChange]);

	const { id: fieldId, ...fieldProps } = editorProps ?? {};
	return (
		<Plate
			editor={editor}
			onChange={contentReadOnly ? undefined : emit}
			readOnly={contentReadOnly}
		>
			<ReviewStateBridge onChange={emit} />
			{contentReadOnly && (
				<Toolbar
					aria-label="Document review"
					className="flex items-center gap-2 border-b p-2 text-sm"
				>
					<CommentToolbarButton />
					<span>Select text to add a review comment.</span>
				</Toolbar>
			)}
			<EditorContainer>
				<Editor
					variant="none"
					className="px-4 py-2"
					{...fieldProps}
					readOnly={contentReadOnly}
					aria-readonly={contentReadOnly || fieldProps["aria-readonly"]}
					renderEditable={
						fieldId
							? (element) => cloneElement(element, { id: fieldId })
							: undefined
					}
				/>
			</EditorContainer>
			{contentReadOnly && <ReviewPanel />}
			<ResolvedDiscussions />
		</Plate>
	);
}

function ReviewStateBridge({ onChange }: { onChange: () => void }) {
	const discussions = usePluginOption(discussionPlugin, "discussions");
	const previous = useRef(discussions);
	useEffect(() => {
		if (previous.current === discussions) return;
		previous.current = discussions;
		onChange();
	}, [discussions, onChange]);
	return null;
}

function ResolvedDiscussions() {
	const discussions = usePluginOption(discussionPlugin, "discussions");
	const enabled = usePluginOption(discussionPlugin, "enabled");
	const canModerate = usePluginOption(discussionPlugin, "canModerate");
	const currentUserId = usePluginOption(discussionPlugin, "currentUserId");
	const editor = useEditorRef();
	const resolved = discussions.filter((discussion) => discussion.isResolved);
	if (resolved.length === 0) return null;
	return (
		<details className="border-t p-3 text-sm">
			<summary>Resolved discussions ({resolved.length})</summary>
			{resolved.map((discussion) => (
				<div
					key={discussion.id}
					className="mt-2 flex items-center justify-between gap-3"
				>
					<span>
						{discussion.documentContent || "Document discussion"} (
						{discussion.comments.length} comments)
					</span>
					{enabled &&
						currentUserId &&
						(canModerate || currentUserId === discussion.userId) && (
							<button
								type="button"
								className="underline"
								onClick={() =>
									editor.setOption(
										discussionPlugin,
										"discussions",
										discussions.map((item) =>
											item.id === discussion.id
												? { ...item, isResolved: false }
												: item,
										),
									)
								}
							>
								Reopen
							</button>
						)}
				</div>
			))}
		</details>
	);
}

function ReviewPanel() {
	const activeId = usePluginOption(commentPlugin, "activeId");
	const discussions = usePluginOption(discussionPlugin, "discussions");
	const [editingId, setEditingId] = useState<string | null>(null);
	const open = discussions.filter((discussion) => !discussion.isResolved);
	return (
		<section aria-label="Review discussions" className="border-t p-3">
			{activeId === getDraftCommentKey() && <CommentCreateForm />}
			{open.map((discussion) => (
				<div key={discussion.id} className="border-b py-2">
					{discussion.comments.map((comment, index) => (
						<Comment
							key={comment.id}
							comment={comment}
							discussionLength={discussion.comments.length}
							index={index}
							editingId={editingId}
							setEditingId={setEditingId}
							documentContent={discussion.documentContent}
							showDocumentContent
						/>
					))}
					<CommentCreateForm discussionId={discussion.id} />
				</div>
			))}
		</section>
	);
}
