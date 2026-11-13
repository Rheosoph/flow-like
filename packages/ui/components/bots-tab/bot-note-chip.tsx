"use client";

import { useTranslation } from "@flow-like/locales";
import { CheckIcon, NotebookPenIcon, StickyNoteIcon } from "lucide-react";
import { useState } from "react";
import { useUserIdentity } from "../../hooks/use-user-lookup";
import { cn } from "../../lib/utils";
import { useBackend } from "../../state/backend-state";
import type { IBotNoteRef } from "../../state/backend-state/bot-state";
import { Button } from "../ui/button";
import { BotNoteDialog } from "./bot-note-dialog";
import { isSelfPrincipal } from "./crew-model";
import type { CrewDirectory } from "./use-crew-directory";

/** Title and status of the note a bot reply wrote; the owner can open the note itself. */
export function BotNoteChip({
	note,
	botId,
	conversationId,
	directory,
	me,
}: Readonly<{
	note: IBotNoteRef;
	botId: string;
	conversationId: string;
	/** A room's members; without it only the bot identity says whether you own the bot. */
	directory?: CrewDirectory;
	me?: string | null;
}>) {
	const { t } = useTranslation("bots");
	const [open, setOpen] = useState(false);
	const readable = Boolean(useBackend().botState?.getNote);
	const bot = useUserIdentity(botId).bot;
	const owner =
		bot?.isOwn ||
		(directory ? isSelfPrincipal(directory(botId).ownerId, me) : false);
	const Icon = note.closed ? CheckIcon : StickyNoteIcon;

	return (
		<div className="flex items-center gap-1">
			<span
				className={cn(
					"inline-flex h-6 max-w-full items-center gap-1.5 rounded-full border bg-muted/40 px-2.5 text-xs",
					note.closed && "text-muted-foreground",
				)}
			>
				<Icon className="size-3 shrink-0" aria-hidden />
				<span className="truncate font-medium">{note.title}</span>
				{note.status && (
					<span className="truncate text-muted-foreground">
						· {note.status}
					</span>
				)}
				{note.closed && (
					<span className="sr-only">{t("notes.closed", "Closed")}</span>
				)}
			</span>
			{owner && readable && (
				<>
					<Button
						type="button"
						variant="ghost"
						size="icon"
						className="size-6 text-muted-foreground"
						aria-label={t("notes.open", "Open note")}
						title={t("notes.open", "Open note")}
						onClick={() => setOpen(true)}
					>
						<NotebookPenIcon className="size-3.5" />
					</Button>
					{open && (
						<BotNoteDialog
							open={open}
							onOpenChange={setOpen}
							botId={botId}
							conversationId={conversationId}
							noteRef={note}
						/>
					)}
				</>
			)}
		</div>
	);
}
