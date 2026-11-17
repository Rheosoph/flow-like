"use client";

import { useTranslation } from "@flow-like/locales";
import { UsersIcon } from "lucide-react";
import { type ReactNode, useState } from "react";
import { cn } from "../../lib/utils";
import type { IConversation } from "../../state/backend-state/inbox-state";
import { IdentityAvatar } from "../identity/identity-avatar";
import { Button } from "../ui/button";
import { LocalConversationMark } from "./bot-labels";
import { ConversationMenu } from "./conversation-menu";
import {
	type TimelineBucket,
	conversationTimeline,
	directPartnerId,
	foldTimeline,
	isSelfPrincipal,
	otherMemberIds,
} from "./crew-model";
import { UnreadBadge } from "./crew-section";
import { conversationBlockTarget } from "./principal-block-action";
import { RoomAvatar } from "./room-avatar";
import type { ConversationText } from "./use-conversation-text";
import type { CrewDirectory } from "./use-crew-directory";

export interface ConversationListContext {
	me?: string | null;
	text: ConversationText;
	directory: CrewDirectory;
	selectedConversationId?: string | null;
	onOpen: (conversationId: string) => void;
	onLeave?: (conversationId: string) => void;
}

/** Folded, the timeline still shows at least this many conversations. */
const FOLDED_MINIMUM = 10;

/** Time gutter, rail and row; the gutter goes and the time moves into the row on narrow screens. */
const TIMELINE_GRID =
	"grid grid-cols-[var(--gutter)_var(--rail)_minmax(0,1fr)] @max-[520px]/crew:grid-cols-[var(--rail)_minmax(0,1fr)]";

/** The gutter cell, which leaves the grid with its column on narrow screens. */
const GUTTER_CELL = "@max-[520px]/crew:hidden";

/** Who a conversation is with and its newest line, prefixed with who wrote it where that matters. */
export function useConversationPreview(
	conversation: IConversation,
	context: ConversationListContext,
) {
	const { t } = useTranslation("bots");
	const { me, text, directory } = context;
	const room = conversation.kind === "group";
	const partner = directPartnerId(conversation, me);
	const last = conversation.last_entry;
	const preview = text.entryText(last);
	let line = preview;
	if (isSelfPrincipal(last?.sender_id, me))
		line = t("crew.rooms.youWrote", "You: {{text}}", { text: preview });
	else if (room && last)
		line = t("crew.rooms.theyWrote", "{{name}}: {{text}}", {
			name: text.senderLabel(last.sender_id),
			text: preview,
		});
	return {
		room,
		partner,
		partnerIsBot: partner ? directory(partner).isBot : false,
		line,
	};
}

/** A room's picture or members, or the person or bot of a direct chat. */
export function ConversationAvatar({
	conversation,
	context,
	size,
}: Readonly<{
	conversation: IConversation;
	context: ConversationListContext;
	size: number;
}>) {
	const partner = directPartnerId(conversation, context.me);
	if (conversation.kind === "group")
		return (
			<RoomAvatar
				roomId={conversation.id}
				title={context.text.title(conversation)}
				principalIds={otherMemberIds(conversation, context.me)}
				imageUrl={conversation.image_url}
				size={size}
			/>
		);
	if (partner) return <IdentityAvatar subject={partner} size={size} />;
	return (
		<span
			className="shrink-0 rounded-full bg-muted"
			style={{ width: size, height: size }}
		/>
	);
}

/** Options for one conversation, shown on hover or focus and always on touch screens. */
export function ConversationOptions({
	conversation,
	context,
	className,
}: Readonly<{
	conversation: IConversation;
	context: ConversationListContext;
	className?: string;
}>) {
	return (
		<ConversationMenu
			conversation={conversation}
			defaultTitle={context.text.title({ ...conversation, title: null })}
			withMute
			me={context.me}
			blockTarget={conversationBlockTarget(
				conversation,
				context.directory,
				context.me,
			)}
			onLeave={() => context.onLeave?.(conversation.id)}
			className={cn(
				"size-7 opacity-0 group-focus-within/conversation:opacity-100 group-hover/conversation:opacity-100 data-[state=open]:opacity-100 [@media(hover:none)]:opacity-100",
				className,
			)}
		/>
	);
}

function useBucketLabel() {
	const { t } = useTranslation("bots");
	return (bucket: TimelineBucket): string => {
		switch (bucket) {
			case "pinned":
				return t("crew.rooms.timeline.pinned", "Pinned");
			case "today":
				return t("crew.time.today", "Today");
			case "yesterday":
				return t("crew.time.yesterday", "Yesterday");
			case "week":
				return t("crew.rooms.timeline.week", "Earlier this week");
			case "older":
				return t("crew.rooms.timeline.older", "Older");
		}
	};
}

/** The gutter's stamp: the clock for the last two days, then the weekday, then the date. */
function gutterTime(
	text: ConversationText,
	bucket: TimelineBucket,
	iso: string,
): string {
	const time = Date.parse(iso);
	if (!Number.isFinite(time)) return "";
	switch (bucket) {
		case "today":
		case "yesterday":
			return text.formats.time.format(time);
		case "week":
			return text.formats.weekday.format(time);
		case "older":
			return text.formats.date.format(time);
		case "pinned":
			return text.activity(iso);
	}
}

function TimelineDot({
	unread,
	selected,
}: Readonly<{ unread: boolean; selected: boolean }>) {
	return (
		<span className="relative flex size-4 items-center justify-center rounded-full bg-background">
			<span
				className={cn(
					"rounded-full transition-[width,height,background-color]",
					unread
						? "size-2.5 bg-primary ring-4 ring-primary/15"
						: selected
							? "size-2 bg-foreground"
							: "size-1.5 bg-muted-foreground/50",
				)}
			/>
		</span>
	);
}

function TimelineHeader({
	label,
	first,
	children,
}: Readonly<{
	label: string;
	first: boolean;
	children?: ReactNode;
}>) {
	return (
		<div
			className={cn(TIMELINE_GRID, "items-center pb-1", first ? "" : "pt-5")}
		>
			<span aria-hidden className={GUTTER_CELL} />
			<span aria-hidden className="flex justify-center">
				<span className="size-2.5 rounded-full border border-muted-foreground/45 bg-background" />
			</span>
			<div className="flex min-h-6 min-w-0 items-center justify-between gap-2 pl-2.5">
				<h3 className="truncate text-[11px] font-semibold tracking-[0.08em] text-muted-foreground uppercase">
					{label}
				</h3>
				{children}
			</div>
		</div>
	);
}

function TimelineRow({
	conversation,
	bucket,
	context,
}: Readonly<{
	conversation: IConversation;
	bucket: TimelineBucket;
	context: ConversationListContext;
}>) {
	const { t } = useTranslation("bots");
	const { text } = context;
	const selected = conversation.id === context.selectedConversationId;
	const unread = conversation.unread_count > 0;
	const { room, partnerIsBot, line } = useConversationPreview(
		conversation,
		context,
	);
	return (
		<li className={cn(TIMELINE_GRID, "group/conversation items-center")}>
			<time
				dateTime={conversation.last_activity_at}
				className={cn(
					"truncate pr-3 text-right text-xs tabular-nums",
					GUTTER_CELL,
					unread ? "font-medium text-foreground" : "text-muted-foreground",
				)}
			>
				{gutterTime(text, bucket, conversation.last_activity_at)}
			</time>
			<span aria-hidden className="flex justify-center">
				<TimelineDot unread={unread} selected={selected} />
			</span>
			<div
				className={cn(
					"flex min-w-0 items-center rounded-xl transition-colors",
					selected
						? "bg-muted ring-1 ring-foreground/10 ring-inset"
						: "hover:bg-muted/40 focus-within:bg-muted/40",
				)}
			>
				<button
					type="button"
					aria-current={selected ? "true" : undefined}
					onClick={() => context.onOpen(conversation.id)}
					className="flex min-w-0 flex-1 items-center gap-3 rounded-xl py-2 pr-1 pl-2.5 text-left outline-none focus-visible:ring-2 focus-visible:ring-ring"
				>
					<ConversationAvatar
						conversation={conversation}
						context={context}
						size={36}
					/>
					<span className="flex min-w-0 flex-1 flex-col gap-px">
						<span className="flex min-w-0 items-center gap-1.5">
							<span
								className={cn(
									"truncate text-sm",
									unread ? "font-semibold" : "font-medium",
								)}
							>
								{text.title(conversation)}
							</span>
							{room ? (
								<span className="flex shrink-0 items-center gap-1 text-xs text-muted-foreground">
									<UsersIcon
										className="size-3"
										aria-label={t("crew.roomTag", "Room")}
									/>
									<span className="@max-[520px]/crew:hidden">
										{text.composition(conversation)}
									</span>
								</span>
							) : (
								partnerIsBot && (
									<span className="shrink-0 rounded-md px-1.5 text-[11px] leading-4.5 text-muted-foreground ring-1 ring-border ring-inset">
										{t("crew.botTag", "Bot")}
									</span>
								)
							)}
							{conversation.local && <LocalConversationMark />}
							<span className="ml-auto hidden shrink-0 pl-2 text-xs text-muted-foreground @max-[520px]/crew:inline">
								{text.activity(conversation.last_activity_at)}
							</span>
						</span>
						<span
							className={cn(
								"truncate text-[13px]",
								unread ? "text-foreground/85" : "text-muted-foreground",
							)}
						>
							{line}
						</span>
					</span>
					<UnreadBadge count={conversation.unread_count} />
				</button>
				<ConversationOptions
					conversation={conversation}
					context={context}
					className="mr-1.5"
				/>
			</div>
		</li>
	);
}

/**
 * Rooms and direct chats on one rail, grouped by day: pinned first, then today, yesterday, this
 * week and older, newest first. Older ones fold away unless unread or open; a search shows all.
 */
export function ConversationTimeline({
	conversations,
	context,
	searching,
}: Readonly<{
	conversations: readonly IConversation[];
	context: ConversationListContext;
	searching: boolean;
}>) {
	const { t } = useTranslation("bots");
	const bucketLabel = useBucketLabel();
	const [showOlder, setShowOlder] = useState(false);
	const timeline = conversationTimeline(conversations, new Date());
	const folded =
		searching || showOlder
			? { groups: timeline, hidden: 0 }
			: foldTimeline(
					timeline,
					FOLDED_MINIMUM,
					(conversation) =>
						conversation.unread_count > 0 ||
						conversation.id === context.selectedConversationId,
				);
	return (
		<div className="relative [--gutter:4.75rem] [--rail:1.75rem] @max-[520px]/crew:[--gutter:0px] @max-[520px]/crew:[--rail:1.25rem]">
			<span
				aria-hidden
				className="absolute inset-y-0 left-[calc(var(--gutter)+var(--rail)/2)] w-px -translate-x-1/2 bg-border [mask-image:linear-gradient(to_bottom,transparent,black_1.25rem,black_calc(100%-1.25rem),transparent)]"
			/>
			{folded.groups.map((group, index) => (
				<div key={group.bucket} className="relative">
					<TimelineHeader label={bucketLabel(group.bucket)} first={index === 0}>
						{group.bucket === "older" && (folded.hidden > 0 || showOlder) && (
							<Button
								type="button"
								variant="ghost"
								size="sm"
								className="h-6 gap-1.5 px-2 text-xs text-muted-foreground"
								aria-expanded={showOlder}
								onClick={() => setShowOlder((shown) => !shown)}
							>
								{showOlder
									? t("crew.rooms.timeline.showFewer", "Show fewer")
									: t("crew.rooms.timeline.showAll", "Show all")}
								{!showOlder && (
									<span className="tabular-nums text-muted-foreground/70">
										{folded.hidden}
									</span>
								)}
							</Button>
						)}
					</TimelineHeader>
					<ol className="flex flex-col gap-0.5">
						{group.conversations.map((conversation) => (
							<TimelineRow
								key={conversation.id}
								conversation={conversation}
								bucket={group.bucket}
								context={context}
							/>
						))}
					</ol>
				</div>
			))}
		</div>
	);
}
