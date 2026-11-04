"use client";

import { useTranslation } from "@flow-like/locales";
import { useUserIdentity } from "../../hooks/use-user-lookup";
import {
	AlertDialog,
	AlertDialogAction,
	AlertDialogCancel,
	AlertDialogContent,
	AlertDialogDescription,
	AlertDialogFooter,
	AlertDialogHeader,
	AlertDialogTitle,
} from "../ui/alert-dialog";
import { isSelfPrincipal } from "./crew-model";
import { type CrewDirectory, firstName } from "./use-crew-directory";

export interface MessageActions {
	react: (target: string, emoji: string, op: "add" | "remove") => void;
	/** Rejects when the edit didn't go through, so the editor keeps the text. */
	edit: (target: string, text: string) => Promise<unknown>;
	/** Asks before deleting. */
	requestDelete: (target: string) => void;
	/** For you only. */
	star: (target: string, starred: boolean) => void;
	/** For everyone in the conversation. */
	pin: (target: string, pinned: boolean) => void;
}

/** "Your bot" or "Anna's bot" next to a bot's name. */
export function BotSenderChip({
	senderId,
	isBotMember,
	me,
	directory,
}: Readonly<{
	senderId: string;
	isBotMember: boolean;
	me?: string | null;
	directory: CrewDirectory;
}>) {
	const { t } = useTranslation("bots");
	const bot = useUserIdentity(senderId).bot;
	const sender = directory(senderId);
	if (!bot && !sender.isBot && !isBotMember) return null;
	const yours = bot?.isOwn || isSelfPrincipal(sender.ownerId, me);
	const ownerKnown =
		bot && !bot.owner.isPending && bot.owner.label !== bot.ownerId;
	let label = t("crew.botTag", "Bot");
	if (bot?.deleted) label = t("identity.deleted", "Deleted bot");
	else if (yours) return null;
	else if (ownerKnown) {
		label = t("identity.botOf", "{{owner}}'s bot", {
			owner: firstName(bot.owner.label),
		});
	}
	return (
		<span className="inline-flex h-4.5 items-center rounded-full border px-1.5 text-[11px] text-muted-foreground">
			{label}
		</span>
	);
}

/** One confirmation for the whole conversation instead of one per message. */
export function DeleteMessageDialog({
	open,
	onOpenChange,
	onConfirm,
}: Readonly<{
	open: boolean;
	onOpenChange: (open: boolean) => void;
	onConfirm: () => void;
}>) {
	const { t } = useTranslation("bots");
	return (
		<AlertDialog open={open} onOpenChange={onOpenChange}>
			<AlertDialogContent>
				<AlertDialogHeader>
					<AlertDialogTitle>
						{t("crew.messages.deleteTitle", "Delete this message?")}
					</AlertDialogTitle>
					<AlertDialogDescription>
						{t(
							"crew.messages.deleteDescription",
							"It disappears for everyone in this conversation.",
						)}
					</AlertDialogDescription>
				</AlertDialogHeader>
				<AlertDialogFooter>
					<AlertDialogCancel>{t("crew.cancel", "Cancel")}</AlertDialogCancel>
					<AlertDialogAction onClick={onConfirm}>
						{t("crew.messages.delete", "Delete")}
					</AlertDialogAction>
				</AlertDialogFooter>
			</AlertDialogContent>
		</AlertDialog>
	);
}
