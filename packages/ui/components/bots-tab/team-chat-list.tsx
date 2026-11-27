"use client";

import { useTranslation } from "@flow-like/locales";
import {
	ChevronDownIcon,
	ChevronRightIcon,
	EyeOffIcon,
	PencilIcon,
	Settings2Icon,
	StarIcon,
	UsersIcon,
} from "lucide-react";
import { type ReactNode, useCallback, useId, useMemo, useState } from "react";
import { toast } from "sonner";
import { useBotLabels } from "../../hooks/use-bots";
import { cn } from "../../lib/utils";
import type { IBot } from "../../state/backend-state/bot-state";
import type {
	IOrgBot,
	IOrgView,
} from "../../state/backend-state/bot-team-state";
import type { IConversation } from "../../state/backend-state/inbox-state";
import { BotAvatar } from "../bots/bot-avatar";
import { IdentityAvatar } from "../identity/identity-avatar";
import { PinnedMark } from "./conversation-menu";
import {
	ConversationAvatar,
	type ConversationListContext,
	ConversationRowMenu,
	useListConversationMenu,
} from "./conversation-timeline";
import { type CrewMember, crewVisibilityKey } from "./crew-grid";
import { matchesSearch } from "./crew-model";
import { isSelfPrincipal } from "./crew-model";
import { UnreadBadge } from "./crew-section";
import { dockSelectionKey } from "./dock-selection";
import { type NeedsYouItem, NeedsYouSection } from "./needs-you-section";
import { type RoomsStatus, RoomsStatusLine } from "./rooms-section";
import { type MenuItems, type MenuKit, RowContextMenu } from "./row-menu";
import {
	type BotRow,
	type ChatFilter,
	type ChatGroup,
	type ChatRow,
	type ColleagueBotRow,
	type PersonRow,
	buildChatGroups,
} from "./team-chat-list-model";
import {
	TeamInvitationCards,
	useTeamInvitations,
} from "./team-invitation-cards";
import { useSetBotHidden } from "./use-bot-visibility";

const COLLAPSED_KEY = "flow-like:bots:collapsed-chat-groups";
const OPT_IN_GROUPS: ReadonlySet<string> = new Set(["watching"]);
const DEPTH_INDENT_PX = 12;
const FILTERS: readonly ChatFilter[] = ["all", "unread", "pinned"];

function readCollapsed(): ReadonlySet<string> {
	try {
		const parsed: unknown = JSON.parse(
			localStorage.getItem(COLLAPSED_KEY) ?? "[]",
		);
		return new Set(
			Array.isArray(parsed)
				? parsed.filter((item): item is string => typeof item === "string")
				: [],
		);
	} catch {
		return new Set();
	}
}

function useCollapsedGroups() {
	const [collapsed, setCollapsed] = useState(readCollapsed);
	const toggle = useCallback((key: string) => {
		setCollapsed((current) => {
			const next = new Set(current);
			if (!next.delete(key)) next.add(key);
			try {
				localStorage.setItem(COLLAPSED_KEY, JSON.stringify([...next]));
			} catch {
				// The groups still fold for this session without storage.
			}
			return next;
		});
	}, []);
	const isOpen = useCallback(
		(key: string) => collapsed.has(key) === OPT_IN_GROUPS.has(key),
		[collapsed],
	);
	return { isOpen, toggle };
}

function useChatSnippet(context: ConversationListContext) {
	const { t } = useTranslation("bots");
	const { me, text } = context;
	return useCallback(
		(conversation: IConversation): string => {
			const last = conversation.last_entry;
			const preview = text.entryText(last);
			if (isSelfPrincipal(last?.sender_id, me))
				return t("crew.rooms.youWrote", "You: {{text}}", { text: preview });
			if (conversation.kind === "group" && last)
				return t("crew.rooms.theyWrote", "{{name}}: {{text}}", {
					name: text.senderLabel(last.sender_id),
					text: preview,
				});
			return preview;
		},
		[me, t, text],
	);
}

function BotTile({
	appearance,
	owner,
}: Readonly<{
	appearance?: IOrgBot["appearance"] | CrewMember["appearance"];
	owner?: string;
}>) {
	return (
		<span className="relative flex size-9 shrink-0">
			<span className="flex size-9 items-end justify-center overflow-hidden rounded-[10px] bg-muted">
				<BotAvatar
					appearance={appearance}
					size={36}
					animated={false}
					decorative
				/>
			</span>
			{owner && (
				<span className="absolute -right-1 -bottom-1 rounded-full ring-2 ring-card">
					<IdentityAvatar subject={owner} size={16} />
				</span>
			)}
		</span>
	);
}

function RowButton({
	selected,
	dim = false,
	disabled = false,
	onClick,
	avatar,
	title,
	subtitle,
	time,
	unread,
	pinned = false,
	className,
}: Readonly<{
	selected: boolean;
	dim?: boolean;
	disabled?: boolean;
	onClick?: () => void;
	avatar: React.ReactNode;
	title: React.ReactNode;
	subtitle: string;
	time?: string;
	unread: number;
	pinned?: boolean;
	className?: string;
}>) {
	return (
		<button
			type="button"
			aria-current={selected ? "true" : undefined}
			disabled={disabled}
			onClick={onClick}
			className={cn(
				"flex w-full min-w-0 items-center gap-3 rounded-xl px-2.5 py-2 text-left outline-none transition-colors hover:bg-muted/60 focus-visible:ring-2 focus-visible:ring-ring active:bg-muted disabled:cursor-default disabled:hover:bg-transparent",
				selected && "bg-muted",
				dim && "opacity-60",
				className,
			)}
		>
			{avatar}
			<span className="flex min-w-0 flex-1 flex-col gap-0.5">
				<span className="flex items-baseline justify-between gap-2">
					<span className="flex min-w-0 items-baseline gap-1.5 text-sm font-semibold">
						{title}
						{pinned && <PinnedMark className="self-center" />}
					</span>
					{time && (
						<span className="shrink-0 text-xs font-normal text-muted-foreground">
							{time}
						</span>
					)}
				</span>
				<span className="truncate text-[13px] text-muted-foreground">
					{subtitle}
				</span>
			</span>
			<UnreadBadge count={unread} />
		</button>
	);
}

function LeadStar() {
	const { t } = useTranslation("bots");
	return (
		<StarIcon
			className="size-3 shrink-0 self-center fill-current text-primary"
			aria-label={t("teams.chats.lead", "Team lead")}
		/>
	);
}

function ColleagueRow({
	row,
	onOpenConversation,
}: Readonly<{
	row: ColleagueBotRow;
	onOpenConversation: (conversationId: string) => void;
}>) {
	const { t } = useTranslation("bots");
	const roomId = row.team.conversation_id;
	return (
		<RowButton
			selected={false}
			dim={row.pending}
			disabled={row.pending || !roomId}
			onClick={roomId ? () => onOpenConversation(roomId) : undefined}
			avatar={<BotTile appearance={row.bot.appearance} owner={row.ownerId} />}
			title={
				<>
					<span className="truncate">{row.name}</span>
					{row.lead && <LeadStar />}
				</>
			}
			subtitle={
				row.pending
					? t("teams.chats.waitingFor", "Waiting for {{name}}", {
							name: row.ownerName,
						})
					: row.snippet ||
						t("teams.chats.ownerBot", "{{name}}'s bot", { name: row.ownerName })
			}
			unread={0}
		/>
	);
}

function PersonChatRow({
	row,
	onMessage,
}: Readonly<{ row: PersonRow; onMessage: (userId: string) => void }>) {
	const { t } = useTranslation("bots");
	return (
		<RowButton
			selected={false}
			onClick={() => onMessage(row.userId)}
			avatar={<IdentityAvatar subject={row.userId} size={36} />}
			title={<span className="truncate">{row.name}</span>}
			subtitle={t("teams.chats.startChat", "Start a chat")}
			unread={0}
		/>
	);
}

interface RowActions {
	context: ConversationListContext;
	selectedKey: string | null;
	onOpenConversation: (conversationId: string) => void;
	onOpenMember: (member: CrewMember) => void;
	onSettings: (bot: IBot) => void;
	onMessagePerson: (userId: string) => void;
}

/** Hides a bot in the current profile, with a way back in the toast. */
function useHideBot() {
	const { t } = useTranslation("bots");
	const { mutateAsync } = useSetBotHidden();
	return useCallback(
		(key: string, name: string) => {
			const failed = (error: unknown) => {
				console.error(`Bot ${key} kept its visibility in the profile:`, error);
				toast.error(
					t(
						"crew.visibility.failed",
						"Couldn't change which bots this profile shows.",
					),
				);
			};
			mutateAsync({ key, hidden: true })
				.then(() =>
					toast.success(
						t("teams.chats.botHidden", "{{name}} is hidden in this profile", {
							name,
						}),
						{
							action: {
								label: t("teams.chats.undo", "Undo"),
								onClick: () =>
									void mutateAsync({ key, hidden: false }).catch(failed),
							},
						},
					),
				)
				.catch(failed);
		},
		[mutateAsync, t],
	);
}

/** The direct chat's options when the bot has one; rendered through `children` to keep hooks unconditional. */
function WithChatMenu({
	conversation,
	context,
	children,
}: Readonly<{
	conversation: IConversation;
	context: ConversationListContext;
	children: (items: MenuItems, dialogs: ReactNode) => ReactNode;
}>) {
	const { items, dialogs } = useListConversationMenu(conversation, context);
	return children(items, dialogs);
}

/** A bot's row: right-click or long press for its settings, its chat's pin and hiding it. */
function BotRowMenu({
	row,
	actions,
	children,
}: Readonly<{ row: BotRow; actions: RowActions; children: ReactNode }>) {
	const { t } = useTranslation("bots");
	const hide = useHideBot();
	const { member } = row;
	const bot = member.bot;
	const frame = (chatItems: MenuItems | null, dialogs: ReactNode) => {
		const items = (kit: MenuKit) => (
			<>
				{bot && !bot.system && (
					<kit.Item onSelect={() => actions.onSettings(bot)}>
						<PencilIcon aria-hidden />
						{t("teams.chats.editBot", "Edit bot")}
					</kit.Item>
				)}
				<kit.Item onSelect={() => hide(crewVisibilityKey(member), row.name)}>
					<EyeOffIcon aria-hidden />
					{t("teams.chats.hideBot", "Hide in this profile")}
				</kit.Item>
				{chatItems && (
					<>
						<kit.Separator />
						{chatItems(kit)}
					</>
				)}
			</>
		);
		return (
			<>
				<RowContextMenu
					items={items}
					className="group/conversation relative min-w-0"
				>
					{children}
				</RowContextMenu>
				{dialogs}
			</>
		);
	};
	if (!row.conversation) return frame(null, null);
	return (
		<WithChatMenu conversation={row.conversation} context={actions.context}>
			{frame}
		</WithChatMenu>
	);
}

function ChatRowItem({
	row,
	actions,
}: Readonly<{ row: ChatRow; actions: RowActions }>) {
	const { t } = useTranslation("bots");
	const { context, selectedKey } = actions;
	if (row.kind === "colleague")
		return (
			<ColleagueRow row={row} onOpenConversation={actions.onOpenConversation} />
		);
	if (row.kind === "person")
		return <PersonChatRow row={row} onMessage={actions.onMessagePerson} />;
	if (row.kind === "bot") {
		const { member } = row;
		const bot = member.bot;
		return (
			<BotRowMenu row={row} actions={actions}>
				<RowButton
					selected={selectedKey === dockSelectionKey(member.selection)}
					onClick={() => actions.onOpenMember(member)}
					avatar={<BotTile appearance={member.appearance} />}
					title={
						<>
							<span className="truncate">{row.name}</span>
							{row.lead && <LeadStar />}
						</>
					}
					subtitle={row.snippet}
					time={
						row.activityAt ? context.text.activity(row.activityAt) : undefined
					}
					unread={row.unread}
					pinned={row.pinned}
				/>
				{bot && !bot.system && (
					<button
						type="button"
						aria-label={t("teams.chats.settingsFor", "Settings for {{name}}", {
							name: row.name,
						})}
						title={t("teams.chats.settingsFor", "Settings for {{name}}", {
							name: row.name,
						})}
						onClick={() => actions.onSettings(bot)}
						className="absolute top-2 right-2 flex size-7 items-center justify-center rounded-md bg-popover text-muted-foreground opacity-0 shadow-sm outline-none hover:text-foreground focus-visible:opacity-100 focus-visible:ring-2 focus-visible:ring-ring group-focus-within/conversation:opacity-100 group-hover/conversation:opacity-100 [@media(hover:none)]:opacity-100"
					>
						<Settings2Icon className="size-4" aria-hidden />
					</button>
				)}
			</BotRowMenu>
		);
	}
	const { conversation } = row;
	return (
		<ConversationRowMenu
			conversation={conversation}
			context={context}
			className="group/conversation relative min-w-0"
			buttonClassName="absolute top-2 right-2 bg-popover shadow-sm"
		>
			<RowButton
				dim={row.watching}
				selected={selectedKey === `conversation:${conversation.id}`}
				onClick={() => actions.onOpenConversation(conversation.id)}
				avatar={
					<ConversationAvatar
						conversation={conversation}
						context={context}
						size={36}
					/>
				}
				title={
					<>
						<span className="truncate">{row.name}</span>
						{row.tag && (
							<span className="shrink-0 text-[11px] font-normal text-muted-foreground">
								{row.tag}
							</span>
						)}
						{row.botChat && (
							<span className="shrink-0 text-[11px] font-normal text-muted-foreground">
								{t("teams.chats.ownerBot", "{{name}}'s bot", {
									name: row.botChat.ownerName,
								})}
							</span>
						)}
						{conversation.kind === "group" && !row.tag && !row.botChat && (
							<UsersIcon
								className="size-3 shrink-0 self-center text-muted-foreground"
								aria-label={t("crew.roomTag", "Room")}
							/>
						)}
					</>
				}
				subtitle={row.snippet}
				time={context.text.activity(conversation.last_activity_at)}
				unread={row.unread}
				pinned={row.pinned}
			/>
		</ConversationRowMenu>
	);
}

function useGroupLabel() {
	const { t } = useTranslation("bots");
	return (group: ChatGroup): string => {
		switch (group.kind) {
			case "team":
				return group.team?.name ?? "";
			case "pinned":
				return t("teams.chats.group.pinned", "Pinned");
			case "bots":
				return t("teams.chats.group.bots", "Your bots");
			case "colleagues":
				return t("teams.chats.group.colleagues", "Colleagues' bots");
			case "rooms":
				return t("teams.chats.group.rooms", "Rooms");
			case "watching":
				return t("teams.chats.group.watching", "Your bots at work");
			case "people":
				return t("teams.chats.group.people", "People");
		}
	};
}

function ChatGroupSection({
	group,
	label,
	open,
	onToggle,
	actions,
}: Readonly<{
	group: ChatGroup;
	label: string;
	open: boolean;
	onToggle: () => void;
	actions: RowActions;
}>) {
	const { t } = useTranslation("bots");
	const bodyId = useId();
	const Chevron = open ? ChevronDownIcon : ChevronRightIcon;
	return (
		<section
			className="flex flex-col"
			style={{ paddingLeft: group.depth * DEPTH_INDENT_PX }}
		>
			<button
				type="button"
				aria-expanded={open}
				aria-controls={bodyId}
				aria-label={
					open
						? t("teams.chats.collapse", "Collapse {{name}}", { name: label })
						: t("teams.chats.expand", "Expand {{name}}", { name: label })
				}
				onClick={onToggle}
				className="flex items-center gap-1.5 rounded-lg px-2 py-1.5 text-left text-xs font-semibold tracking-wide text-muted-foreground uppercase outline-none hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring"
			>
				<Chevron className="size-3.5 shrink-0" aria-hidden />
				<span className="min-w-0 flex-1 truncate">{label}</span>
				{!open && <UnreadBadge count={group.unread} />}
			</button>
			<div
				id={bodyId}
				className={cn("flex flex-col gap-0.5", !open && "hidden")}
			>
				{group.rows.map((row) => (
					<ChatRowItem key={row.key} row={row} actions={actions} />
				))}
			</div>
		</section>
	);
}

function FilterBar({
	filter,
	onChange,
}: Readonly<{ filter: ChatFilter; onChange: (filter: ChatFilter) => void }>) {
	const { t } = useTranslation("bots");
	const labels: Record<ChatFilter, string> = {
		all: t("teams.chats.filter.all", "All"),
		unread: t("teams.chats.filter.unread", "Unread"),
		pinned: t("teams.chats.filter.pinned", "Pinned"),
	};
	return (
		<fieldset className="m-0 flex min-w-0 gap-1.5 border-0 p-0">
			<legend className="sr-only">
				{t("teams.chats.filter.label", "Filter chats")}
			</legend>
			{FILTERS.map((item) => (
				<button
					key={item}
					type="button"
					aria-pressed={filter === item}
					onClick={() => onChange(item)}
					className={cn(
						"rounded-full border px-3 py-1 text-xs font-medium outline-none transition-colors focus-visible:ring-2 focus-visible:ring-ring",
						filter === item
							? "border-primary bg-primary/10 text-primary"
							: "text-muted-foreground hover:text-foreground",
					)}
				>
					{labels[item]}
				</button>
			))}
		</fieldset>
	);
}

export interface TeamChatListProps {
	org: IOrgView;
	conversations: readonly IConversation[];
	crew: readonly CrewMember[];
	context: ConversationListContext;
	needs: readonly NeedsYouItem[];
	search: string;
	selectedKey: string | null;
	status: RoomsStatus;
	loadingCrew: boolean;
	onOpenConversation: (conversationId: string) => void;
	onOpenMember: (member: CrewMember) => void;
	onOpenAssistant: () => void;
	onSettings: (bot: IBot) => void;
	/** Opens the direct chat with a teammate, starting it when there is none. */
	onMessagePerson: (userId: string) => void;
}

/**
 * Chats grouped the way people work: what needs you, pinned, one group per team with its room
 * first, then your own bots, rooms and people.
 */
export function TeamChatList({
	org,
	conversations,
	crew,
	context,
	needs,
	search,
	selectedKey,
	status,
	loadingCrew,
	onOpenConversation,
	onOpenMember,
	onOpenAssistant,
	onSettings,
	onMessagePerson,
}: Readonly<TeamChatListProps>) {
	const { t } = useTranslation("bots");
	const labels = useBotLabels();
	const [filter, setFilter] = useState<ChatFilter>("all");
	const { isOpen, toggle } = useCollapsedGroups();
	const groupLabel = useGroupLabel();
	const snippet = useChatSnippet(context);
	const query = search.trim();
	const { items: invitations, dismiss } = useTeamInvitations(org);
	const shownInvitations = useMemo(
		() =>
			query
				? invitations.filter((invitation) =>
						matchesSearch(query, [invitation.team_name, invitation.org_name]),
					)
				: invitations,
		[invitations, query],
	);
	const groups = useMemo(
		() =>
			buildChatGroups({
				org,
				me: context.me,
				conversations,
				crew,
				filter,
				query,
				title: context.text.title,
				requestsLabel: t("teams.requests.yourRequests", "Your requests"),
				snippet,
				orgBotName: (bot) =>
					labels({ system: bot.system ?? null, name: bot.name }).name,
				ownerName: (ownerId) => context.directory(ownerId).name,
			}),
		[org, conversations, crew, filter, query, context, snippet, labels, t],
	);
	const actions: RowActions = {
		context,
		selectedKey,
		onOpenConversation,
		onOpenMember,
		onSettings,
		onMessagePerson,
	};
	const showNeeds = filter !== "pinned";
	const needsShown = showNeeds ? needs : [];
	const invitedShown = showNeeds ? shownInvitations : [];
	const nothing =
		groups.length === 0 && needsShown.length === 0 && invitedShown.length === 0;
	const loading = status.loading || loadingCrew;

	return (
		<nav
			aria-label={t("teams.chats.list.label", "Chats and teams")}
			className="flex flex-col gap-3 px-3 py-3"
		>
			<FilterBar filter={filter} onChange={setFilter} />
			{showNeeds && (
				<NeedsYouSection
					compact
					items={needsShown}
					extraCount={invitedShown.length}
					extra={
						<TeamInvitationCards
							invitations={invitedShown}
							org={org}
							directory={context.directory}
							onDismiss={dismiss}
						/>
					}
					directory={context.directory}
					text={context.text}
					onOpenConversation={onOpenConversation}
					onOpenAssistant={onOpenAssistant}
				/>
			)}
			<RoomsStatusLine
				status={{ ...status, loading }}
				empty={nothing && !query && filter === "all" && !loading}
			/>
			{groups.map((group) => (
				<ChatGroupSection
					key={group.key}
					group={group}
					label={groupLabel(group)}
					open={
						isOpen(group.key) || (group.kind === "watching" && query !== "")
					}
					onToggle={() => toggle(group.key)}
					actions={actions}
				/>
			))}
			{nothing && !loading && !status.failed && (query || filter !== "all") && (
				<p className="py-6 text-center text-sm text-muted-foreground">
					{query
						? t("crew.search.empty", "Nothing matches “{{query}}”.", { query })
						: t("teams.chats.filter.empty", "Nothing here.")}
				</p>
			)}
		</nav>
	);
}
