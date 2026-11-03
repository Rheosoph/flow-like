"use client";

import { useTranslation } from "@flow-like/locales";
import { LockIcon, MoreHorizontalIcon } from "lucide-react";
import { useRef, useState } from "react";
import { useUserIdentity } from "../../hooks/use-user-lookup";
import { cn } from "../../lib/utils";
import { MessageReactionPicker } from "../interfaces/chat-default/message-reactions";
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
import { Button } from "../ui/button";
import {
	DropdownMenu,
	DropdownMenuContent,
	DropdownMenuItem,
	DropdownMenuTrigger,
} from "../ui/dropdown-menu";
import { ApprovalCard } from "./approval-card";
import { isSelfPrincipal } from "./crew-model";
import type { ConversationMessage, ConversationReaction } from "./fold-entries";
import { type CrewDirectory, firstName } from "./use-crew-directory";

export interface MessageActions {
	react: (target: string, emoji: string, op: "add" | "remove") => void;
	/** Rejects when the edit didn't go through, so the editor keeps the text. */
	edit: (target: string, text: string) => Promise<unknown>;
	/** Asks before deleting. */
	requestDelete: (target: string) => void;
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
	else if (yours) label = t("identity.yourBot", "Your bot");
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

function ReactionChips({
	reactions,
	directory,
	onToggle,
}: Readonly<{
	reactions: readonly ConversationReaction[];
	directory: CrewDirectory;
	onToggle?: (reaction: ConversationReaction) => void;
}>) {
	const { t } = useTranslation("bots");
	if (reactions.length === 0) return null;
	return (
		<div className="mt-1 flex flex-wrap gap-1">
			{reactions.map((reaction) => {
				const names = reaction.reactorIds
					.map((id) =>
						directory(id).isMe ? t("crew.you", "You") : directory(id).name,
					)
					.join(", ");
				const label = t(
					"crew.messages.reactionLabel",
					"{{emoji}} from {{names}}",
					{ emoji: reaction.emoji, names },
				);
				return (
					<button
						key={reaction.emoji}
						type="button"
						aria-pressed={reaction.mine}
						aria-label={label}
						title={label}
						disabled={!onToggle}
						onClick={() => onToggle?.(reaction)}
						className={cn(
							"inline-flex h-6 items-center gap-1 rounded-full border px-2 text-xs outline-none focus-visible:ring-2 focus-visible:ring-ring disabled:cursor-default",
							reaction.mine
								? "border-primary/40 bg-primary/10 text-foreground"
								: "bg-card text-muted-foreground hover:bg-muted",
						)}
					>
						<span aria-hidden>{reaction.emoji}</span>
						<span aria-hidden>{reaction.count}</span>
					</button>
				);
			})}
		</div>
	);
}

function EditBox({
	initial,
	onSave,
	onCancel,
}: Readonly<{
	initial: string;
	onSave: (text: string) => Promise<unknown>;
	onCancel: () => void;
}>) {
	const { t } = useTranslation("bots");
	const [value, setValue] = useState(initial);
	const [saving, setSaving] = useState(false);
	const save = async () => {
		const text = value.trim();
		if (!text || text === initial.trim()) return onCancel();
		setSaving(true);
		const saved = await onSave(text).then(
			() => true,
			() => false,
		);
		setSaving(false);
		if (saved) onCancel();
	};
	return (
		<div className="flex w-full flex-col gap-1.5">
			<textarea
				// biome-ignore lint/a11y/noAutofocus: editing starts from an explicit action on this message
				autoFocus
				value={value}
				rows={Math.min(8, Math.max(2, value.split("\n").length))}
				aria-label={t("crew.messages.editLabel", "Edit message")}
				onChange={(event) => setValue(event.target.value)}
				onKeyDown={(event) => {
					if (event.key === "Escape") onCancel();
					if (
						event.key === "Enter" &&
						!event.shiftKey &&
						!event.nativeEvent.isComposing
					) {
						event.preventDefault();
						void save();
					}
				}}
				className="w-full resize-none rounded-lg border bg-card px-2.5 py-1.5 text-sm outline-none focus-visible:border-primary"
			/>
			<div className="flex justify-end gap-1.5">
				<Button type="button" variant="ghost" size="sm" onClick={onCancel}>
					{t("crew.cancel", "Cancel")}
				</Button>
				<Button
					type="button"
					size="sm"
					disabled={saving || !value.trim()}
					onClick={() => void save()}
				>
					{t("crew.messages.save", "Save")}
				</Button>
			</div>
		</div>
	);
}

function MessageToolbar({
	message,
	actions,
	onEdit,
	onDelete,
}: Readonly<{
	message: ConversationMessage;
	actions: MessageActions;
	onEdit: () => void;
	onDelete: () => void;
}>) {
	const { t } = useTranslation("bots");
	const mine = message.reactions
		.filter((reaction) => reaction.mine)
		.map((reaction) => reaction.emoji);
	const editable = message.mine && message.content.kind === "text";
	return (
		<div className="absolute -top-3.5 right-0 z-10 hidden items-center gap-0.5 rounded-lg border bg-popover p-0.5 shadow-sm group-focus-within/message:flex group-hover/message:flex has-data-[state=open]:flex">
			<MessageReactionPicker
				reactions={mine}
				onToggle={(emoji) =>
					actions.react(
						message.id,
						emoji,
						mine.includes(emoji) ? "remove" : "add",
					)
				}
				triggerClassName="size-7"
			/>
			{message.mine && (
				<DropdownMenu>
					<DropdownMenuTrigger asChild>
						<button
							type="button"
							aria-label={t("crew.messages.more", "More actions")}
							title={t("crew.messages.more", "More actions")}
							className="inline-flex size-7 items-center justify-center rounded-md text-muted-foreground outline-none hover:bg-muted hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring"
						>
							<MoreHorizontalIcon className="size-4" aria-hidden />
						</button>
					</DropdownMenuTrigger>
					<DropdownMenuContent align="end">
						{editable && (
							<DropdownMenuItem onSelect={onEdit}>
								{t("crew.messages.edit", "Edit")}
							</DropdownMenuItem>
						)}
						<DropdownMenuItem variant="destructive" onSelect={onDelete}>
							{t("crew.messages.delete", "Delete")}
						</DropdownMenuItem>
					</DropdownMenuContent>
				</DropdownMenu>
			)}
		</div>
	);
}

function MessageContent({
	message,
}: Readonly<{ message: ConversationMessage }>) {
	const { t } = useTranslation("bots");
	if (message.content.kind === "sealed") {
		return (
			<span className="inline-flex items-center gap-1.5 italic text-muted-foreground">
				<LockIcon className="size-3.5" aria-hidden />
				{t("crew.messages.encrypted", "Encrypted message")}
			</span>
		);
	}
	if (message.content.kind === "unsupported") {
		return (
			<span className="italic text-muted-foreground">
				{t("crew.messages.unsupported", "Message this app can't show")}
			</span>
		);
	}
	return (
		<>
			<span className="wrap-break-word whitespace-pre-wrap">
				{message.content.text}
			</span>
			{message.edited && (
				<span className="ml-1.5 text-[11px] text-muted-foreground">
					{t("crew.messages.edited", "(edited)")}
				</span>
			)}
		</>
	);
}

/** One message with its reactions, plus react / edit / delete on hover, focus or tap. */
export function MessageRow({
	message,
	directory,
	actions,
	readOnly,
}: Readonly<{
	message: ConversationMessage;
	directory: CrewDirectory;
	actions: MessageActions;
	readOnly: boolean;
}>) {
	const [editing, setEditing] = useState(false);
	const rowRef = useRef<HTMLDivElement>(null);
	const text = message.content.kind === "text" ? message.content.text : "";

	return (
		<div
			ref={rowRef}
			tabIndex={readOnly ? undefined : 0}
			className={cn(
				"group/message relative flex flex-col rounded-lg outline-none focus-visible:ring-2 focus-visible:ring-ring",
				message.mine ? "items-end" : "items-start",
			)}
		>
			{editing ? (
				<EditBox
					initial={text}
					onSave={(next) => actions.edit(message.id, next)}
					onCancel={() => {
						setEditing(false);
						rowRef.current?.focus();
					}}
				/>
			) : message.approval ? (
				<ApprovalCard
					approval={message.approval}
					botId={message.senderId}
					botName={directory(message.senderId).name}
					readOnly={readOnly}
				/>
			) : (
				<div
					className={cn(
						"text-sm leading-relaxed",
						message.mine &&
							"max-w-[85%] rounded-[14px] rounded-br-sm bg-secondary px-3 py-2 text-secondary-foreground",
					)}
				>
					<MessageContent message={message} />
				</div>
			)}
			<ReactionChips
				reactions={message.reactions}
				directory={directory}
				onToggle={
					readOnly
						? undefined
						: (reaction) =>
								actions.react(
									message.id,
									reaction.emoji,
									reaction.mine ? "remove" : "add",
								)
				}
			/>
			{!readOnly && !editing && (
				<MessageToolbar
					message={message}
					actions={actions}
					onEdit={() => setEditing(true)}
					onDelete={() => actions.requestDelete(message.id)}
				/>
			)}
		</div>
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
