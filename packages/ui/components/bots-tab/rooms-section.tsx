"use client";

import { useTranslation } from "@flow-like/locales";
import {
	MessageSquarePlusIcon,
	PlusIcon,
	StarIcon,
	UsersIcon,
} from "lucide-react";
import type { ReactNode } from "react";
import type { IConversation } from "../../state/backend-state/inbox-state";
import { Button } from "../ui/button";
import { Skeleton } from "../ui/skeleton";
import { LocalConversationMark } from "./bot-labels";
import { PinnedMark } from "./conversation-menu";
import {
	ConversationAvatar,
	type ConversationListContext,
	ConversationOptions,
	ConversationTimeline,
	useConversationPreview,
} from "./conversation-timeline";
import { unreadOf } from "./crew-model";
import { CrewSection, UnreadBadge, crewCardClass } from "./crew-section";

export interface RoomsStatus {
	searching: boolean;
	loading: boolean;
	failed: boolean;
	onRetry: () => void;
	/** Replaces the default hint while there are no conversations. */
	emptyText?: string;
}

/**
 * One conversation as a card row: the person or bot of a direct chat, or a room's members, with
 * the newest message and the unread count.
 */
export function ConversationRow({
	conversation,
	context,
}: Readonly<{
	conversation: IConversation;
	context: ConversationListContext;
}>) {
	const { t } = useTranslation("bots");
	const { text } = context;
	const selected = conversation.id === context.selectedConversationId;
	const { room, partnerIsBot, line } = useConversationPreview(
		conversation,
		context,
	);
	return (
		<div className="group/conversation relative min-w-0">
			<button
				type="button"
				aria-current={selected ? "true" : undefined}
				onClick={() => context.onOpen(conversation.id)}
				className={crewCardClass(
					selected,
					"flex size-full min-w-0 items-center gap-3 active:scale-[0.98] active:bg-muted/60 px-3.5 py-3 @max-[520px]/crew:rounded-xl @max-[520px]/crew:px-3 @max-[520px]/crew:py-2.5",
				)}
			>
				<ConversationAvatar
					conversation={conversation}
					context={context}
					size={38}
				/>
				<span className="flex min-w-0 flex-1 flex-col gap-0.5">
					<span className="flex items-baseline justify-between gap-2">
						<span className="flex min-w-0 items-baseline gap-1.5">
							<span className="truncate text-sm font-semibold">
								{text.title(conversation)}
							</span>
							{room ? (
								<UsersIcon
									className="size-3 shrink-0 self-center text-muted-foreground"
									aria-label={t("crew.roomTag", "Room")}
								/>
							) : (
								partnerIsBot && (
									<span className="shrink-0 text-[11px] text-muted-foreground">
										{t("crew.botTag", "Bot")}
									</span>
								)
							)}
							{conversation.pinned_at && <PinnedMark className="self-center" />}
							{conversation.local && (
								<LocalConversationMark className="self-center" />
							)}
						</span>
						<span className="shrink-0 text-xs text-muted-foreground">
							{text.activity(conversation.last_activity_at)}
						</span>
					</span>
					<span className="truncate text-[13px] text-muted-foreground">
						{line}
					</span>
				</span>
				<UnreadBadge count={unreadOf(conversation)} />
			</button>
			<ConversationOptions
				conversation={conversation}
				context={context}
				className="absolute top-2 right-2 bg-popover shadow-sm"
			/>
		</div>
	);
}

export function RoomsStatusLine({
	status,
	empty,
}: Readonly<{ status: RoomsStatus; empty: boolean }>) {
	const { t } = useTranslation("bots");
	if (status.loading) {
		return (
			<div className="flex flex-col gap-2">
				{[0, 1, 2].map((row) => (
					<Skeleton key={row} className="h-13 rounded-xl" />
				))}
			</div>
		);
	}
	if (status.failed) {
		return (
			<div className="flex flex-wrap items-center gap-3 rounded-2xl border border-dashed px-4 py-4 text-sm text-muted-foreground">
				{t("crew.rooms.loadFailed", "Couldn't load your conversations.")}
				<Button
					type="button"
					variant="outline"
					size="sm"
					onClick={status.onRetry}
				>
					{t("crew.retry", "Try again")}
				</Button>
			</div>
		);
	}
	if (!empty) return null;
	return (
		<p className="rounded-2xl border border-dashed px-4 py-6 text-center text-sm text-muted-foreground">
			{status.emptyText ??
				t(
					"crew.rooms.empty",
					"No conversations yet. Start one with a person or one of your bots.",
				)}
		</p>
	);
}

/** Rooms and direct conversations on one timeline, pinned first, then by day, newest first. */
export function RoomsSection({
	title,
	conversations,
	context,
	status,
	onNewRoom,
	onNewMessage,
	onShowStarred,
}: Readonly<{
	/** Replaces "Rooms and people", for a view that shows only some of them. */
	title?: ReactNode;
	conversations: readonly IConversation[];
	context: ConversationListContext;
	status: RoomsStatus;
	onNewRoom: () => void;
	onNewMessage: () => void;
	/** Opens your starred messages across every conversation. */
	onShowStarred?: () => void;
}>) {
	const { t } = useTranslation("bots");
	const empty = conversations.length === 0;
	if (empty && status.searching) return null;
	const starredLabel = t("crew.marks.starredTitle", "Starred messages");
	return (
		<CrewSection
			title={title ?? t("crew.rooms.title", "Rooms and people")}
			actions={
				<>
					{onShowStarred && (
						<Button
							type="button"
							variant="outline"
							size="sm"
							className="rounded-[9px] @max-[520px]/crew:size-8 @max-[520px]/crew:px-0"
							aria-label={starredLabel}
							title={starredLabel}
							onClick={onShowStarred}
						>
							<StarIcon aria-hidden />
							<span className="@max-[520px]/crew:hidden">
								{t("crew.marks.starredShort", "Starred")}
							</span>
						</Button>
					)}
					<Button
						type="button"
						variant="outline"
						size="sm"
						className="rounded-[9px] @max-[520px]/crew:size-8 @max-[520px]/crew:px-0"
						aria-label={t("crew.newMessage", "New message")}
						title={t("crew.newMessage", "New message")}
						onClick={onNewMessage}
					>
						<MessageSquarePlusIcon aria-hidden />
						<span className="@max-[520px]/crew:hidden">
							{t("crew.newMessage", "New message")}
						</span>
					</Button>
					<Button
						type="button"
						variant="outline"
						size="sm"
						className="rounded-[9px] @max-[520px]/crew:size-8 @max-[520px]/crew:px-0"
						aria-label={t("crew.newRoom", "New room")}
						title={t("crew.newRoom", "New room")}
						onClick={onNewRoom}
					>
						<PlusIcon aria-hidden />
						<span className="@max-[520px]/crew:hidden">
							{t("crew.newRoom", "New room")}
						</span>
					</Button>
				</>
			}
		>
			<RoomsStatusLine status={status} empty={empty} />
			{!empty && (
				<ConversationTimeline
					conversations={conversations}
					context={context}
					searching={status.searching}
				/>
			)}
		</CrewSection>
	);
}
