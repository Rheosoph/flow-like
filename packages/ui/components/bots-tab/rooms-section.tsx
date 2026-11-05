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
import { IdentityAvatar } from "../identity/identity-avatar";
import { Button } from "../ui/button";
import { Skeleton } from "../ui/skeleton";
import { LocalConversationMark } from "./bot-labels";
import { ConversationMenu, PinnedMark } from "./conversation-menu";
import { directPartnerId, isSelfPrincipal, otherMemberIds } from "./crew-model";
import { CrewSection, UnreadBadge, crewCardClass } from "./crew-section";
import { MemberStack } from "./member-stack";
import { conversationBlockTarget } from "./principal-block-action";
import { RoomAvatar } from "./room-avatar";
import type { ConversationText } from "./use-conversation-text";
import type { CrewDirectory } from "./use-crew-directory";

export const ROOM_GRID_CLASS =
	"grid grid-cols-[repeat(auto-fill,minmax(min(280px,100%),1fr))] gap-3 @max-[520px]/crew:grid-cols-1 @max-[520px]/crew:gap-1.5";

export interface RoomCardContext {
	me?: string | null;
	text: ConversationText;
	directory: CrewDirectory;
	selectedConversationId?: string | null;
	onOpen: (conversationId: string) => void;
	onLeave?: (conversationId: string) => void;
}

export interface RoomsStatus {
	searching: boolean;
	loading: boolean;
	failed: boolean;
	onRetry: () => void;
	/** Replaces the default hint while there are no conversations. */
	emptyText?: string;
}

/** A card with its conversation menu in the corner, shown on hover or focus and on touch screens. */
function WithConversationMenu({
	conversation,
	text,
	me,
	onLeave,
	directory,
	children,
}: Readonly<{
	conversation: IConversation;
	text: ConversationText;
	me?: string | null;
	onLeave?: (conversationId: string) => void;
	directory: CrewDirectory;
	children: ReactNode;
}>) {
	return (
		<div className="group/conversation relative min-w-0">
			{children}
			<ConversationMenu
				conversation={conversation}
				defaultTitle={text.title({ ...conversation, title: null })}
				withMute
				me={me}
				blockTarget={conversationBlockTarget(conversation, directory, me)}
				onLeave={() => onLeave?.(conversation.id)}
				className="absolute top-2 right-2 size-7 bg-popover opacity-0 shadow-sm group-focus-within/conversation:opacity-100 group-hover/conversation:opacity-100 data-[state=open]:opacity-100 [@media(hover:none)]:opacity-100"
			/>
		</div>
	);
}

export function RoomCard({
	conversation,
	context,
}: Readonly<{ conversation: IConversation; context: RoomCardContext }>) {
	const { me, text } = context;
	const selected = conversation.id === context.selectedConversationId;
	const last = conversation.last_entry;
	const image = conversation.image_url;
	return (
		<WithConversationMenu
			conversation={conversation}
			text={text}
			directory={context.directory}
			me={me}
			onLeave={context.onLeave}
		>
			<button
				type="button"
				aria-current={selected ? "true" : undefined}
				onClick={() => context.onOpen(conversation.id)}
				className={crewCardClass(
					selected,
					"flex size-full min-w-0 flex-col gap-2 p-3.5 @max-[520px]/crew:grid @max-[520px]/crew:grid-cols-[36px_minmax(0,1fr)] @max-[520px]/crew:gap-x-3 @max-[520px]/crew:gap-y-0.5 @max-[520px]/crew:rounded-xl @max-[520px]/crew:px-3 @max-[520px]/crew:py-2.5",
				)}
			>
				<span className="flex w-full items-center justify-between gap-2 @max-[520px]/crew:col-start-1 @max-[520px]/crew:row-span-2 @max-[520px]/crew:self-center">
					<RoomAvatar
						roomId={conversation.id}
						title={text.title(conversation)}
						principalIds={otherMemberIds(conversation, me)}
						imageUrl={image}
						className="hidden @max-[520px]/crew:inline-flex"
					/>
					<span className="flex min-w-0 items-center gap-2 @max-[520px]/crew:hidden">
						{image && (
							<RoomAvatar
								roomId={conversation.id}
								title={text.title(conversation)}
								principalIds={[]}
								imageUrl={image}
								size={30}
							/>
						)}
						<MemberStack
							principalIds={otherMemberIds(conversation, me)}
							max={image ? 5 : 7}
							size={30}
						/>
					</span>
					<span className="shrink-0 text-xs text-muted-foreground @max-[520px]/crew:hidden">
						{text.activity(conversation.last_activity_at)}
					</span>
				</span>
				<span className="mt-0.5 flex min-w-0 items-center gap-2 @max-[520px]/crew:col-start-2 @max-[520px]/crew:mt-0">
					<span className="truncate text-[15px] font-semibold">
						{text.title(conversation)}
					</span>
					{conversation.pinned_at && <PinnedMark />}
					{conversation.local && <LocalConversationMark />}
					<UnreadBadge count={conversation.unread_count} />
				</span>
				<span className="text-xs text-muted-foreground @max-[520px]/crew:hidden">
					{text.composition(conversation)}
				</span>
				<span className="line-clamp-2 text-[13px] leading-normal text-muted-foreground @max-[520px]/crew:col-start-2 @max-[520px]/crew:line-clamp-1">
					{last && (
						<span className="font-semibold text-foreground">
							{text.senderLabel(last.sender_id)}:{" "}
						</span>
					)}
					{text.entryText(last)}
				</span>
			</button>
		</WithConversationMenu>
	);
}

/**
 * One conversation as a row: the person or bot of a direct chat, or a room's members, with the
 * newest message and the unread count.
 */
export function ConversationRow({
	conversation,
	context,
}: Readonly<{ conversation: IConversation; context: RoomCardContext }>) {
	const { t } = useTranslation("bots");
	const { me, text, directory } = context;
	const selected = conversation.id === context.selectedConversationId;
	const room = conversation.kind === "group";
	const partner = directPartnerId(conversation, me);
	const partnerIsBot = partner ? directory(partner).isBot : false;
	const last = conversation.last_entry;
	const preview = text.entryText(last);
	const mine = isSelfPrincipal(last?.sender_id, me);
	let previewLine = preview;
	if (mine)
		previewLine = t("crew.rooms.youWrote", "You: {{text}}", { text: preview });
	else if (room && last)
		previewLine = t("crew.rooms.theyWrote", "{{name}}: {{text}}", {
			name: text.senderLabel(last.sender_id),
			text: preview,
		});
	return (
		<WithConversationMenu
			conversation={conversation}
			text={text}
			directory={context.directory}
			me={me}
			onLeave={context.onLeave}
		>
			<button
				type="button"
				aria-current={selected ? "true" : undefined}
				onClick={() => context.onOpen(conversation.id)}
				className={crewCardClass(
					selected,
					"flex size-full min-w-0 items-center gap-3 px-3.5 py-3 @max-[520px]/crew:rounded-xl @max-[520px]/crew:px-3 @max-[520px]/crew:py-2.5",
				)}
			>
				{room ? (
					<RoomAvatar
						roomId={conversation.id}
						title={text.title(conversation)}
						principalIds={otherMemberIds(conversation, me)}
						imageUrl={conversation.image_url}
					/>
				) : partner ? (
					<IdentityAvatar subject={partner} size={38} />
				) : (
					<span className="size-9.5 shrink-0 rounded-full bg-muted" />
				)}
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
						{previewLine}
					</span>
				</span>
				<UnreadBadge count={conversation.unread_count} />
			</button>
		</WithConversationMenu>
	);
}

export function RoomsStatusLine({
	status,
	empty,
}: Readonly<{ status: RoomsStatus; empty: boolean }>) {
	const { t } = useTranslation("bots");
	if (status.loading) {
		return (
			<div className={ROOM_GRID_CLASS}>
				<Skeleton className="h-34 rounded-2xl" />
				<Skeleton className="h-34 rounded-2xl" />
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

/** Group conversations as room cards, direct conversations as people and bot cards. */
export function RoomsSection({
	title,
	rooms,
	direct,
	context,
	status,
	onNewRoom,
	onNewMessage,
	onShowStarred,
}: Readonly<{
	/** Replaces "Rooms and people", for a view that shows only some of them. */
	title?: ReactNode;
	rooms: readonly IConversation[];
	direct: readonly IConversation[];
	context: RoomCardContext;
	status: RoomsStatus;
	onNewRoom: () => void;
	onNewMessage: () => void;
	/** Opens your starred messages across every conversation. */
	onShowStarred?: () => void;
}>) {
	const { t } = useTranslation("bots");
	const empty = rooms.length === 0 && direct.length === 0;
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
			{rooms.length > 0 && (
				<div className={ROOM_GRID_CLASS}>
					{rooms.map((conversation) => (
						<RoomCard
							key={conversation.id}
							conversation={conversation}
							context={context}
						/>
					))}
				</div>
			)}
			{direct.length > 0 && (
				<div className={ROOM_GRID_CLASS}>
					{direct.map((conversation) => (
						<ConversationRow
							key={conversation.id}
							conversation={conversation}
							context={context}
						/>
					))}
				</div>
			)}
		</CrewSection>
	);
}
