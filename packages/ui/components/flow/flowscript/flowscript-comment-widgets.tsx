"use client";

/**
 * The comment-thread overlay for the FlowScript editor.
 *
 * Rendered as a Monaco CONTENT WIDGET hosting a React portal — not a
 * screen-positioned Popover — because a content widget is anchored to a text
 * position and repositioned by Monaco itself on every scroll/layout/font
 * change: it tracks its line exactly while scrolling and disappears with it,
 * where a Popover pinned to a captured pixel position would float detached
 * the moment the user scrolls (or need hand-rolled onDidScrollChange syncing).
 * `allowEditorOverflow` lets it escape the editor box near the edges.
 */

import { useTranslation } from "@flow-like/locales";
import type { Monaco, OnMount } from "@monaco-editor/react";
import {
	MessageSquareTextIcon,
	PencilIcon,
	SendHorizonalIcon,
	Trash2Icon,
	XIcon,
} from "lucide-react";
import { useEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { type PeerUserInfo, colorFromSub } from "../../../hooks/use-peer-users";
import type { IComment } from "../../../lib/schema/flow/board";
import { userInitials } from "../../../lib/user-display";
import {
	Avatar,
	AvatarFallback,
	AvatarImage,
	Badge,
	Button,
	RelativeTime,
	Textarea,
} from "../../ui";
import {
	canModifyFlowScriptComment,
	commentTimestampMs,
} from "./flowscript-comments";

type FlowScriptEditor = Parameters<OnMount>[0];

const COMMENT_WIDGET_ID = "flowscript.commentThread";

/** Which thread the overlay shows and whether it opened straight into typing. */
export interface FlowScriptCommentThreadState {
	anchorId: string;
	/** Line at open time — the live anchor line wins whenever it resolves. */
	line: number;
	focusComposer: boolean;
}

export interface FlowScriptCommentOverlayProps {
	editor: FlowScriptEditor | null;
	monaco: Monaco | null;
	anchorId: string;
	line: number;
	comments: readonly IComment[];
	/** Resolved author identities (names/avatars), shared lookup cache. */
	authors: ReadonlyMap<string, PeerUserInfo>;
	sub?: string;
	/** False hides the composer and every edit/delete control (read-only view). */
	editable: boolean;
	focusComposer: boolean;
	onCreate: (anchorId: string, content: string) => Promise<void> | void;
	onUpdate: (comment: IComment, content: string) => Promise<void> | void;
	onDelete: (comment: IComment) => Promise<void> | void;
	onClose: () => void;
}

export function FlowScriptCommentOverlay({
	editor,
	monaco,
	anchorId,
	line,
	comments,
	authors,
	sub,
	editable,
	focusComposer,
	onCreate,
	onUpdate,
	onDelete,
	onClose,
}: Readonly<FlowScriptCommentOverlayProps>) {
	const { t } = useTranslation("flow");
	const hostRef = useRef<HTMLDivElement | null>(null);
	if (hostRef.current === null && typeof document !== "undefined") {
		hostRef.current = document.createElement("div");
	}
	const lineRef = useRef(line);
	lineRef.current = line;
	const onCloseRef = useRef(onClose);
	onCloseRef.current = onClose;
	const widgetRef = useRef<
		Parameters<FlowScriptEditor["addContentWidget"]>[0] | null
	>(null);

	useEffect(() => {
		const host = hostRef.current;
		if (!editor || !monaco || !host) return;
		const widget = {
			getId: () => COMMENT_WIDGET_ID,
			getDomNode: () => host,
			allowEditorOverflow: true,
			getPosition: () => ({
				position: { lineNumber: lineRef.current, column: 1 },
				preference: [
					monaco.editor.ContentWidgetPositionPreference.BELOW,
					monaco.editor.ContentWidgetPositionPreference.ABOVE,
				],
			}),
		};
		widgetRef.current = widget;
		editor.addContentWidget(widget);
		// The thread list scrolls itself. Portal events bubble the React tree,
		// not the DOM tree, so Monaco's own DOM-level wheel listener needs a
		// native stop here or wheeling over the card scrolls the code instead.
		const stopWheel = (event: WheelEvent) => event.stopPropagation();
		host.addEventListener("wheel", stopWheel, { passive: true });
		return () => {
			host.removeEventListener("wheel", stopWheel);
			editor.removeContentWidget(widget);
			widgetRef.current = null;
		};
	}, [editor, monaco]);

	// The anchor line moves as the user edits above it — reseat the widget.
	// biome-ignore lint/correctness/useExhaustiveDependencies: `line` is the reseat trigger — getPosition reads it through lineRef
	useEffect(() => {
		const widget = widgetRef.current;
		if (editor && widget) editor.layoutContentWidget(widget);
	}, [editor, line]);

	// Click-away and Escape both close; everything inside the card stays live.
	useEffect(() => {
		const host = hostRef.current;
		if (!host) return;
		const onPointerDown = (event: PointerEvent) => {
			if (event.target instanceof Node && host.contains(event.target)) return;
			onCloseRef.current();
		};
		const onKeyDown = (event: KeyboardEvent) => {
			if (event.key !== "Escape") return;
			event.stopPropagation();
			onCloseRef.current();
		};
		document.addEventListener("pointerdown", onPointerDown, true);
		host.addEventListener("keydown", onKeyDown);
		return () => {
			document.removeEventListener("pointerdown", onPointerDown, true);
			host.removeEventListener("keydown", onKeyDown);
		};
	}, []);

	const host = hostRef.current;
	if (!host) return null;

	return createPortal(
		<div className="w-80 max-w-[85vw] overflow-hidden rounded-lg border bg-popover font-sans text-popover-foreground shadow-lg">
			<div className="flex items-center justify-between gap-2 border-b px-3 py-1.5">
				<span className="flex min-w-0 items-center gap-1.5 text-xs font-medium">
					<MessageSquareTextIcon className="h-3.5 w-3.5 shrink-0 text-primary" />
					{t("flowscriptComments", "Comments")}
					{comments.length > 0 && (
						<Badge variant="secondary" className="text-[10px]">
							{comments.length}
						</Badge>
					)}
				</span>
				<Button
					variant="ghost"
					size="icon"
					className="h-6 w-6"
					onClick={onClose}
				>
					<XIcon className="h-3 w-3" />
				</Button>
			</div>
			<div className="max-h-64 overflow-y-auto">
				{comments.length === 0 ? (
					<p className="px-3 py-3 text-xs text-muted-foreground">
						{t("flowscriptNoCommentsYet", "No comments on this statement yet")}
					</p>
				) : (
					comments.map((comment) => (
						<CommentRow
							key={comment.id}
							comment={comment}
							authors={authors}
							sub={sub}
							editable={editable}
							onUpdate={onUpdate}
							onDelete={onDelete}
						/>
					))
				)}
			</div>
			{editable && (
				<CommentComposer
					autoFocus={focusComposer || comments.length === 0}
					onSubmit={(content) => onCreate(anchorId, content)}
				/>
			)}
		</div>,
		host,
	);
}

interface CommentRowProps {
	comment: IComment;
	authors: ReadonlyMap<string, PeerUserInfo>;
	sub?: string;
	editable: boolean;
	onUpdate: (comment: IComment, content: string) => Promise<void> | void;
	onDelete: (comment: IComment) => Promise<void> | void;
}

function CommentRow({
	comment,
	authors,
	sub,
	editable,
	onUpdate,
	onDelete,
}: Readonly<CommentRowProps>) {
	const { t } = useTranslation("flow");
	const [editing, setEditing] = useState(false);
	const [draft, setDraft] = useState(comment.content);
	const [busy, setBusy] = useState(false);

	const author =
		comment.author && comment.author !== "anonymous"
			? comment.author
			: undefined;
	const info = author ? authors.get(author) : undefined;
	const name = info?.name ?? t("common:user", "User");
	const canModify = editable && canModifyFlowScriptComment(comment, sub);

	const saveEdit = async () => {
		const trimmed = draft.trim();
		if (busy || trimmed.length === 0 || trimmed === comment.content) {
			setEditing(false);
			setDraft(comment.content);
			return;
		}
		setBusy(true);
		try {
			await onUpdate(comment, trimmed);
			setEditing(false);
		} finally {
			setBusy(false);
		}
	};

	const remove = async () => {
		if (busy) return;
		setBusy(true);
		try {
			await onDelete(comment);
		} finally {
			setBusy(false);
		}
	};

	return (
		<div className="group border-b px-3 py-2 last:border-b-0">
			<div className="flex items-center gap-2">
				<Avatar className="h-5 w-5 shrink-0">
					<AvatarImage src={info?.avatarUrl} alt={name} />
					<AvatarFallback
						className="text-[8px] font-semibold text-white"
						style={{ backgroundColor: colorFromSub(author) }}
					>
						{userInitials(name, "?")}
					</AvatarFallback>
				</Avatar>
				<span className="min-w-0 truncate text-xs font-medium">{name}</span>
				<RelativeTime
					value={commentTimestampMs(comment)}
					className="shrink-0 text-[10px] text-muted-foreground"
					style="short"
				/>
				{canModify && !editing && (
					<span className="ml-auto flex shrink-0 items-center gap-0.5 opacity-0 transition-opacity group-hover:opacity-100">
						<Button
							variant="ghost"
							size="icon"
							className="h-5 w-5"
							title={t("edit", "Edit")}
							disabled={busy}
							onClick={() => {
								setDraft(comment.content);
								setEditing(true);
							}}
						>
							<PencilIcon className="h-3 w-3" />
						</Button>
						<Button
							variant="ghost"
							size="icon"
							className="h-5 w-5 text-destructive"
							title={t("delete", "Delete")}
							disabled={busy}
							onClick={() => void remove()}
						>
							<Trash2Icon className="h-3 w-3" />
						</Button>
					</span>
				)}
			</div>
			{editing ? (
				<div className="mt-1.5 space-y-1.5">
					<Textarea
						value={draft}
						onChange={(event) => setDraft(event.target.value)}
						className="min-h-14 text-xs"
						disabled={busy}
					/>
					<div className="flex justify-end gap-1.5">
						<Button
							variant="ghost"
							size="sm"
							className="h-6 px-2 text-xs"
							disabled={busy}
							onClick={() => {
								setEditing(false);
								setDraft(comment.content);
							}}
						>
							{t("cancel", "Cancel")}
						</Button>
						<Button
							size="sm"
							className="h-6 px-2 text-xs"
							disabled={busy || draft.trim().length === 0}
							onClick={() => void saveEdit()}
						>
							{t("save", "Save")}
						</Button>
					</div>
				</div>
			) : (
				<p className="mt-1 whitespace-pre-wrap break-words text-xs">
					{comment.content}
				</p>
			)}
		</div>
	);
}

interface CommentComposerProps {
	autoFocus: boolean;
	onSubmit: (content: string) => Promise<void> | void;
}

function CommentComposer({
	autoFocus,
	onSubmit,
}: Readonly<CommentComposerProps>) {
	const { t } = useTranslation("flow");
	const [value, setValue] = useState("");
	const [busy, setBusy] = useState(false);
	const textareaRef = useRef<HTMLTextAreaElement | null>(null);

	// Deferred a frame: child effects run before the overlay's effect attaches
	// the widget host to the document, and focus() is a no-op until then.
	useEffect(() => {
		if (!autoFocus) return;
		const handle = requestAnimationFrame(() => textareaRef.current?.focus());
		return () => cancelAnimationFrame(handle);
	}, [autoFocus]);

	const submit = async () => {
		const trimmed = value.trim();
		if (busy || trimmed.length === 0) return;
		setBusy(true);
		try {
			await onSubmit(trimmed);
			setValue("");
			textareaRef.current?.focus();
		} finally {
			setBusy(false);
		}
	};

	return (
		<div className="flex items-end gap-1.5 border-t px-3 py-2">
			<Textarea
				ref={textareaRef}
				value={value}
				onChange={(event) => setValue(event.target.value)}
				onKeyDown={(event) => {
					if (event.key === "Enter" && !event.shiftKey) {
						event.preventDefault();
						void submit();
					}
				}}
				placeholder={t("flowscriptCommentPlaceholder", "Write a comment…")}
				className="min-h-9 flex-1 resize-none text-xs"
				rows={1}
				disabled={busy}
			/>
			<Button
				size="icon"
				className="h-7 w-7 shrink-0"
				disabled={busy || value.trim().length === 0}
				title={t("flowscriptCommentSend", "Add comment (Enter)")}
				onClick={() => void submit()}
			>
				<SendHorizonalIcon className="h-3.5 w-3.5" />
			</Button>
		</div>
	);
}
