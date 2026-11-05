"use client";

import { useTranslation } from "@flow-like/locales";
import {
	MoreHorizontalIcon,
	PinIcon,
	PinOffIcon,
	StarIcon,
	StarOffIcon,
	Trash2Icon,
} from "lucide-react";
import { type ReactNode, useCallback, useMemo, useState } from "react";
import { toast } from "sonner";
import {
	shownEntry,
	useConversationPins,
	useConversationStars,
	useSetMessagePinned,
	useSetMessageStarred,
	useStarredMessages,
} from "../../hooks/use-inbox-marks";
import { cn } from "../../lib/utils";
import {
	type IConversation,
	type IInboxEntry,
	type IPinnedMessage,
	type IStarredMessage,
	MAX_PINNED_MESSAGES,
} from "../../state/backend-state/inbox-state";
import { IdentityAvatar } from "../identity/identity-avatar";
import { Button } from "../ui/button";
import {
	Dialog,
	DialogContent,
	DialogDescription,
	DialogHeader,
	DialogTitle,
} from "../ui/dialog";
import {
	DropdownMenu,
	DropdownMenuContent,
	DropdownMenuItem,
	DropdownMenuSeparator,
	DropdownMenuTrigger,
} from "../ui/dropdown-menu";
import { Popover, PopoverContent, PopoverTrigger } from "../ui/popover";
import { Skeleton } from "../ui/skeleton";
import type { ThreadMarks } from "./conversation-messages";
import type { ConversationText } from "./use-conversation-text";

const NO_PINS: IPinnedMessage[] = [];

/** One conversation's pins and your stars in it, and the actions that change them. */
export function useThreadMarks(
	conversationId: string,
	onError: (error: unknown) => void,
) {
	const { t } = useTranslation("bots");
	const pins = useConversationPins(conversationId).data ?? NO_PINS;
	const { starredIds } = useConversationStars(conversationId);
	const { mutate: setStarred } = useSetMessageStarred();
	const { mutate: setPinned } = useSetMessagePinned();
	const marks = useMemo<ThreadMarks>(
		() => ({
			starred: starredIds,
			pinnedBy: new Map(pins.map((pin) => [pin.message_id, pin.pinned_by])),
		}),
		[starredIds, pins],
	);
	const star = useCallback(
		(messageId: string, starred: boolean) =>
			setStarred({ conversationId, messageId, starred }, { onError }),
		[conversationId, onError, setStarred],
	);
	const full = pins.length >= MAX_PINNED_MESSAGES;
	const pin = useCallback(
		(messageId: string, pinned: boolean) => {
			if (pinned && full) {
				toast.error(
					t(
						"crew.marks.pinLimit",
						"This conversation already has {{max}} pinned messages. Unpin one first.",
						{ max: MAX_PINNED_MESSAGES },
					),
				);
				return;
			}
			setPinned({ conversationId, messageId, pinned }, { onError });
		},
		[conversationId, full, onError, setPinned, t],
	);
	return { pins, marks, star, pin };
}

const ICON_TRIGGER_CLASS =
	"inline-flex size-7 items-center justify-center rounded-md border bg-popover text-muted-foreground shadow-sm outline-none hover:bg-muted hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring";

/** What one message's hover menu offers: star and pin for everyone, delete for your own. */
export function MessageMenu({
	starred,
	pinned,
	onStar,
	onPin,
	onDelete,
}: Readonly<{
	starred: boolean;
	pinned: boolean;
	onStar: () => void;
	onPin: () => void;
	onDelete?: () => void;
}>) {
	const { t } = useTranslation("bots");
	const label = t("crew.messages.more", "More actions");
	return (
		<DropdownMenu>
			<DropdownMenuTrigger asChild>
				<button
					type="button"
					aria-label={label}
					title={label}
					className={ICON_TRIGGER_CLASS}
				>
					<MoreHorizontalIcon className="size-4" aria-hidden />
				</button>
			</DropdownMenuTrigger>
			<DropdownMenuContent align="end">
				<DropdownMenuItem onSelect={onStar}>
					{starred ? <StarOffIcon aria-hidden /> : <StarIcon aria-hidden />}
					{starred
						? t("crew.marks.unstar", "Remove star")
						: t("crew.marks.star", "Star message")}
				</DropdownMenuItem>
				<DropdownMenuItem onSelect={onPin}>
					{pinned ? <PinOffIcon aria-hidden /> : <PinIcon aria-hidden />}
					{pinned
						? t("crew.marks.unpinMessage", "Unpin for everyone")
						: t("crew.marks.pinMessage", "Pin for everyone")}
				</DropdownMenuItem>
				{onDelete && (
					<>
						<DropdownMenuSeparator />
						<DropdownMenuItem variant="destructive" onSelect={onDelete}>
							<Trash2Icon aria-hidden />
							{t("crew.messages.delete", "Delete")}
						</DropdownMenuItem>
					</>
				)}
			</DropdownMenuContent>
		</DropdownMenu>
	);
}

/** The line above a pinned or starred message. */
export function MessageMarkLine({
	pinnedBy,
	starred,
	mine,
}: Readonly<{ pinnedBy?: string; starred: boolean; mine: boolean }>) {
	const { t } = useTranslation("bots");
	if (!pinnedBy && !starred) return null;
	return (
		<div
			className={cn(
				"flex items-center gap-3 px-4 text-[11px] text-muted-foreground",
				mine && "justify-end",
			)}
		>
			{pinnedBy && (
				<span className="inline-flex items-center gap-1">
					<PinIcon className="size-3" aria-hidden />
					{t("crew.marks.pinnedBy", "Pinned by {{name}}", { name: pinnedBy })}
				</span>
			)}
			{starred && (
				<span className="inline-flex items-center gap-1">
					<StarIcon className="size-3 fill-current" aria-hidden />
					{t("crew.marks.starred", "Starred")}
				</span>
			)}
		</div>
	);
}

/** One pinned or starred message in a list: who wrote it, when, what it says. */
function MarkedMessageRow({
	entry,
	context,
	text,
	action,
	onOpen,
}: Readonly<{
	entry: IInboxEntry | null;
	context?: string;
	text: ConversationText;
	action?: ReactNode;
	onOpen?: () => void;
}>) {
	const { t } = useTranslation("bots");
	const body = entry
		? text.entryText(entry)
		: t(
				"crew.marks.beforeYouJoined",
				"A message from before you joined this conversation",
			);
	const content = (
		<>
			{entry ? (
				<IdentityAvatar subject={entry.sender_id} size={24} />
			) : (
				<span className="size-6 shrink-0 rounded-full bg-muted" />
			)}
			<span className="flex min-w-0 flex-1 flex-col gap-0.5">
				<span className="flex min-w-0 items-baseline gap-2">
					<span className="truncate text-[13px] font-semibold">
						{entry ? text.senderLabel(entry.sender_id) : "—"}
					</span>
					{entry && (
						<time
							dateTime={entry.created_at}
							className="shrink-0 text-[11px] text-muted-foreground"
						>
							{text.activity(entry.created_at)}
						</time>
					)}
				</span>
				{context && (
					<span className="truncate text-[11px] text-muted-foreground">
						{context}
					</span>
				)}
				<span className="line-clamp-3 text-[13px] leading-normal text-muted-foreground">
					{body}
				</span>
			</span>
		</>
	);
	return (
		<li className="group/marked flex items-start gap-1 rounded-lg hover:bg-muted/60">
			{onOpen && entry ? (
				<button
					type="button"
					onClick={onOpen}
					title={t("crew.marks.jump", "Show in conversation")}
					className="flex min-w-0 flex-1 items-start gap-2.5 rounded-lg px-2.5 py-2 text-left outline-none focus-visible:ring-2 focus-visible:ring-ring"
				>
					{content}
				</button>
			) : (
				<div className="flex min-w-0 flex-1 items-start gap-2.5 px-2.5 py-2">
					{content}
				</div>
			)}
			{action && <span className="shrink-0 pt-1.5 pr-1.5">{action}</span>}
		</li>
	);
}

function useMarkFailed() {
	const { t } = useTranslation("bots");
	return (error: unknown) => {
		console.warn("Updating a pin or star failed", error);
		toast.error(
			t("crew.messages.actionFailed", "That didn't go through. Try again."),
		);
	};
}

function RemoveMarkButton({
	label,
	icon,
	onClick,
}: Readonly<{ label: string; icon: ReactNode; onClick: () => void }>) {
	return (
		<Button
			type="button"
			variant="ghost"
			size="icon"
			className="size-7 text-muted-foreground opacity-0 group-hover/marked:opacity-100 focus-visible:opacity-100"
			aria-label={label}
			title={label}
			onClick={onClick}
		>
			{icon}
		</Button>
	);
}

/** The bar under a conversation's header: its newest pin, and every pin one click away. */
export function PinnedMessagesBar({
	conversationId,
	pins,
	text,
	onJump,
}: Readonly<{
	conversationId: string;
	pins: readonly IPinnedMessage[];
	text: ConversationText;
	onJump: (messageId: string) => void;
}>) {
	const { t } = useTranslation("bots");
	const [open, setOpen] = useState(false);
	const { mutate } = useSetMessagePinned();
	const failed = useMarkFailed();
	const newest = pins[0];
	if (!newest) return null;
	const preview = newest.entry
		? text.entryText(shownEntry(newest.entry, newest.latest_edit))
		: t(
				"crew.marks.beforeYouJoined",
				"A message from before you joined this conversation",
			);
	const unpinLabel = t("crew.marks.unpinMessage", "Unpin for everyone");
	return (
		<Popover open={open} onOpenChange={setOpen}>
			<PopoverTrigger asChild>
				<button
					type="button"
					className="flex w-full shrink-0 items-center gap-2 border-b bg-muted/40 px-4.5 py-2 text-left text-[13px] outline-none hover:bg-muted/70 focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-inset"
				>
					<PinIcon className="size-3.5 shrink-0 text-primary" aria-hidden />
					<span className="min-w-0 flex-1 truncate">{preview}</span>
					<span className="shrink-0 text-[11px] text-muted-foreground">
						{t("crew.marks.pinnedCount", "{{count}} pinned", {
							count: pins.length,
						})}
					</span>
				</button>
			</PopoverTrigger>
			<PopoverContent
				align="start"
				className="w-[min(26rem,calc(100vw-2rem))] p-1.5"
			>
				<p className="px-2.5 pt-1.5 pb-1 text-xs font-semibold text-muted-foreground">
					{t("crew.marks.pinnedTitle", "Pinned messages")}
				</p>
				<ul className="flex max-h-96 flex-col overflow-y-auto">
					{pins.map((pin) => (
						<MarkedMessageRow
							key={pin.message_id}
							entry={pin.entry && shownEntry(pin.entry, pin.latest_edit)}
							context={t("crew.marks.pinnedBy", "Pinned by {{name}}", {
								name: text.senderLabel(pin.pinned_by),
							})}
							text={text}
							onOpen={() => {
								setOpen(false);
								onJump(pin.message_id);
							}}
							action={
								<RemoveMarkButton
									label={unpinLabel}
									icon={<PinOffIcon aria-hidden />}
									onClick={() =>
										mutate(
											{
												conversationId,
												messageId: pin.message_id,
												pinned: false,
											},
											{ onError: failed },
										)
									}
								/>
							}
						/>
					))}
				</ul>
			</PopoverContent>
		</Popover>
	);
}

function StarList({
	stars,
	conversations,
	text,
	onOpen,
}: Readonly<{
	stars: readonly IStarredMessage[];
	conversations?: readonly IConversation[];
	text: ConversationText;
	onOpen: (star: IStarredMessage) => void;
}>) {
	const { t } = useTranslation("bots");
	const { mutate } = useSetMessageStarred();
	const failed = useMarkFailed();
	const byId = useMemo(
		() =>
			new Map(
				(conversations ?? []).map((conversation) => [
					conversation.id,
					conversation,
				]),
			),
		[conversations],
	);
	const unstarLabel = t("crew.marks.unstar", "Remove star");
	return (
		<ul className="flex flex-col">
			{stars.map((star) => {
				const conversation = byId.get(star.conversation_id);
				return (
					<MarkedMessageRow
						key={star.message_id}
						entry={shownEntry(star.entry, star.latest_edit)}
						context={conversation ? text.title(conversation) : undefined}
						text={text}
						onOpen={() => onOpen(star)}
						action={
							<RemoveMarkButton
								label={unstarLabel}
								icon={<StarOffIcon aria-hidden />}
								onClick={() =>
									mutate(
										{
											conversationId: star.conversation_id,
											messageId: star.message_id,
											starred: false,
										},
										{ onError: failed },
									)
								}
							/>
						}
					/>
				);
			})}
		</ul>
	);
}

function StarsState({
	loading,
	failed,
	empty,
	onRetry,
}: Readonly<{
	loading: boolean;
	failed: boolean;
	empty: boolean;
	onRetry: () => void;
}>) {
	const { t } = useTranslation("bots");
	if (loading) {
		return (
			<div className="flex flex-col gap-2 p-2">
				<Skeleton className="h-12 rounded-lg" />
				<Skeleton className="h-12 rounded-lg" />
			</div>
		);
	}
	if (failed) {
		return (
			<div className="flex flex-col items-center gap-2 py-6 text-center text-sm text-muted-foreground">
				{t("crew.marks.loadFailed", "Couldn't load your starred messages.")}
				<Button type="button" variant="outline" size="sm" onClick={onRetry}>
					{t("crew.retry", "Try again")}
				</Button>
			</div>
		);
	}
	if (!empty) return null;
	return (
		<p className="py-8 text-center text-sm text-muted-foreground">
			{t(
				"crew.marks.noStars",
				"Nothing starred yet. Star a message from its menu to find it here.",
			)}
		</p>
	);
}

function ConversationStars({
	conversationId,
	text,
	onOpen,
}: Readonly<{
	conversationId: string;
	text: ConversationText;
	onOpen: (star: IStarredMessage) => void;
}>) {
	const query = useConversationStars(conversationId);
	return (
		<>
			<StarsState
				loading={query.isPending}
				failed={query.isError}
				empty={query.stars.length === 0}
				onRetry={() => void query.refetch()}
			/>
			<StarList stars={query.stars} text={text} onOpen={onOpen} />
		</>
	);
}

function AllStars({
	conversations,
	text,
	onOpen,
}: Readonly<{
	conversations?: readonly IConversation[];
	text: ConversationText;
	onOpen: (star: IStarredMessage) => void;
}>) {
	const { t } = useTranslation("bots");
	const query = useStarredMessages();
	const pages = query.data?.pages;
	const stars = useMemo(
		() => pages?.flatMap((page) => page.stars) ?? [],
		[pages],
	);
	return (
		<>
			<StarsState
				loading={query.isPending}
				failed={query.isError && !pages}
				empty={stars.length === 0 && !query.hasNextPage}
				onRetry={() => void query.refetch()}
			/>
			<StarList
				stars={stars}
				conversations={conversations}
				text={text}
				onOpen={onOpen}
			/>
			{query.hasNextPage && (
				<div className="flex justify-center py-2">
					<Button
						type="button"
						variant="ghost"
						size="sm"
						disabled={query.isFetchingNextPage}
						onClick={() => void query.fetchNextPage()}
					>
						{query.isFetchingNextPage
							? t("crew.messages.loadingOlder", "Loading…")
							: t("crew.marks.loadMore", "Show more")}
					</Button>
				</div>
			)}
		</>
	);
}

/** Your starred messages: one conversation's, or with no `conversationId` all of them. */
export function StarredMessagesDialog({
	open,
	onOpenChange,
	conversationId,
	conversations,
	text,
	onOpen,
}: Readonly<{
	open: boolean;
	onOpenChange: (open: boolean) => void;
	conversationId?: string;
	conversations?: readonly IConversation[];
	text: ConversationText;
	onOpen: (conversationId: string, messageId: string) => void;
}>) {
	const { t } = useTranslation("bots");
	const openStar = (star: IStarredMessage) => {
		onOpenChange(false);
		onOpen(star.conversation_id, star.message_id);
	};
	return (
		<Dialog open={open} onOpenChange={onOpenChange}>
			<DialogContent className="flex max-h-[min(40rem,85vh)] flex-col gap-3 sm:max-w-lg">
				<DialogHeader>
					<DialogTitle>
						{t("crew.marks.starredTitle", "Starred messages")}
					</DialogTitle>
					<DialogDescription>
						{t("crew.marks.starredDescription", "Only you see what you star.")}
					</DialogDescription>
				</DialogHeader>
				<div className="-mx-2 min-h-0 flex-1 overflow-y-auto">
					{open &&
						(conversationId ? (
							<ConversationStars
								conversationId={conversationId}
								text={text}
								onOpen={openStar}
							/>
						) : (
							<AllStars
								conversations={conversations}
								text={text}
								onOpen={openStar}
							/>
						))}
				</div>
			</DialogContent>
		</Dialog>
	);
}
