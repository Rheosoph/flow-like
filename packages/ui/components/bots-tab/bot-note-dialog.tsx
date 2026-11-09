"use client";

import { useTranslation } from "@flow-like/locales";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useMemo, useState } from "react";
import { useUserIdentity } from "../../hooks/use-user-lookup";
import { formatAbsoluteDateTime, formatRelativeTime } from "../../lib/date";
import { useBackend } from "../../state/backend-state";
import type {
	BotNoteState,
	IBotNote,
	IBotNoteRef,
} from "../../state/backend-state/bot-state";
import { Badge } from "../ui/badge";
import { Button } from "../ui/button";
import {
	Dialog,
	DialogContent,
	DialogDescription,
	DialogFooter,
	DialogHeader,
	DialogTitle,
} from "../ui/dialog";
import { Input } from "../ui/input";
import { TextEditor } from "../ui/text-editor";
import { Textarea } from "../ui/textarea";
import { isNoteConflict, noteVersionView } from "./bot-note-entry";

function noteKey(botId: string, conversationId: string, key: string) {
	return ["bot-note", botId, conversationId, key] as const;
}

/** A bot's note as the message referenced it; its owner can edit, close and reopen it. */
export function BotNoteDialog({
	open,
	onOpenChange,
	botId,
	conversationId,
	noteRef,
}: Readonly<{
	open: boolean;
	onOpenChange: (open: boolean) => void;
	botId: string;
	conversationId: string;
	noteRef: IBotNoteRef;
}>) {
	const { t } = useTranslation("bots");
	const botState = useBackend().botState;
	const getNote = botState?.getNote?.bind(botState);
	const updateNote = botState?.updateNote?.bind(botState);
	const queryClient = useQueryClient();
	const botName = useUserIdentity(botId).label;
	const queryKey = noteKey(botId, conversationId, noteRef.key);
	const [editing, setEditing] = useState(false);
	const [body, setBody] = useState("");
	const [status, setStatus] = useState("");
	const [error, setError] = useState<"conflict" | "failed" | null>(null);

	const query = useQuery({
		queryKey,
		enabled: open && getNote !== undefined,
		queryFn: () =>
			getNote?.(botId, conversationId, noteRef.key) as Promise<IBotNote>,
	});
	const note = query.data;
	const view = useMemo(
		() => (note ? noteVersionView(note, noteRef.version) : undefined),
		[note, noteRef.version],
	);

	const save = useMutation({
		mutationFn: (write: {
			body?: string;
			status?: string;
			state?: BotNoteState;
		}) =>
			updateNote?.(botId, conversationId, noteRef.key, {
				...write,
				expected_version: note?.version,
			}) as Promise<IBotNote>,
		onSuccess: (saved) => {
			setError(null);
			setEditing(false);
			queryClient.setQueryData(queryKey, saved);
			void queryClient.invalidateQueries({ queryKey });
		},
		onError: (failure) => {
			setError(isNoteConflict(failure) ? "conflict" : "failed");
			void queryClient.invalidateQueries({ queryKey });
		},
	});

	const startEdit = () => {
		if (!note) return;
		setBody(note.body);
		setStatus(note.status ?? "");
		setError(null);
		setEditing(true);
	};

	const author = (updatedBy: string) =>
		updatedBy === botId ? botName : t("notes.you", "You");

	const shown = view?.version;
	const canWrite = updateNote !== undefined && Boolean(note);

	return (
		<Dialog open={open} onOpenChange={onOpenChange}>
			<DialogContent className="max-h-[85vh] overflow-y-auto sm:max-w-2xl">
				<DialogHeader>
					<DialogTitle className="flex items-center gap-2">
						<span className="truncate">{shown?.title ?? noteRef.title}</span>
						{note?.state === "closed" && (
							<Badge variant="secondary">{t("notes.closed", "Closed")}</Badge>
						)}
					</DialogTitle>
					<DialogDescription>
						{t("notes.ownerOnly", "Only you can see this note")}
					</DialogDescription>
				</DialogHeader>

				{query.isPending && (
					<p className="text-sm text-muted-foreground">
						{t("notes.loading", "Loading note…")}
					</p>
				)}
				{query.isError && (
					<p role="alert" className="text-sm text-destructive">
						{t("notes.loadFailed", "Couldn't load this note.")}
					</p>
				)}

				{note && shown && (
					<div className="flex flex-col gap-3">
						{view?.updatedSince && (
							<p className="rounded-md border bg-muted/40 px-3 py-2 text-xs text-muted-foreground">
								{t("notes.updatedSince", "Updated since this message")}
							</p>
						)}
						<div className="flex flex-wrap items-center gap-x-3 gap-y-1 text-xs text-muted-foreground">
							{shown.status && !editing && (
								<Badge variant="outline">{shown.status}</Badge>
							)}
							<span title={formatAbsoluteDateTime(shown.updated_at)}>
								{t("notes.updatedBy", "Updated {{when}} by {{who}}", {
									when: formatRelativeTime(shown.updated_at),
									who: author(shown.updated_by),
								})}
							</span>
						</div>

						{editing ? (
							<div className="flex flex-col gap-2">
								<Input
									value={status}
									maxLength={120}
									aria-label={t("notes.status", "Status")}
									placeholder={t("notes.statusPlaceholder", "Status")}
									onChange={(event) => setStatus(event.target.value)}
								/>
								<Textarea
									value={body}
									rows={12}
									aria-label={t("notes.body", "Note")}
									className="font-mono text-sm"
									onChange={(event) => setBody(event.target.value)}
								/>
							</div>
						) : (
							<div data-fl-chat-prose>
								<TextEditor
									key={`${shown.version}`}
									presentation="chat"
									initialContent={shown.body}
									isMarkdown={true}
									editable={false}
								/>
							</div>
						)}

						{error && (
							<p role="alert" className="text-sm text-destructive">
								{error === "conflict"
									? t(
											"notes.conflict",
											"The note changed while you were editing. The latest version is loaded; try again.",
										)
									: t("notes.saveFailed", "Couldn't save the note.")}
							</p>
						)}
					</div>
				)}

				{canWrite && (
					<DialogFooter className="gap-2 sm:justify-between">
						<Button
							type="button"
							variant="outline"
							disabled={save.isPending || editing}
							onClick={() =>
								save.mutate({
									state: note?.state === "closed" ? "open" : "closed",
								})
							}
						>
							{note?.state === "closed"
								? t("notes.reopen", "Reopen")
								: t("notes.closeNote", "Close note")}
						</Button>
						{editing ? (
							<div className="flex gap-2">
								<Button
									type="button"
									variant="ghost"
									disabled={save.isPending}
									onClick={() => setEditing(false)}
								>
									{t("notes.cancel", "Cancel")}
								</Button>
								<Button
									type="button"
									disabled={save.isPending}
									onClick={() => save.mutate({ body, status: status.trim() })}
								>
									{t("notes.save", "Save")}
								</Button>
							</div>
						) : (
							<Button type="button" onClick={startEdit}>
								{t("notes.edit", "Edit")}
							</Button>
						)}
					</DialogFooter>
				)}
			</DialogContent>
		</Dialog>
	);
}
