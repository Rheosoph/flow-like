"use client";

import { useTranslation } from "@flow-like/locales";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { EyeOffIcon } from "lucide-react";
import {
	type ReactNode,
	useCallback,
	useEffect,
	useMemo,
	useRef,
	useState,
} from "react";
import { useAuth } from "react-oidc-context";
import { botPresence, useBotPresence } from "../../hooks/use-bot-presence";
import { useBackend } from "../../state/backend-state";
import type { IBot } from "../../state/backend-state/bot-state";
import {
	selectActiveBotId,
	useGlobalChatStore,
} from "../../state/global-chat/global-chat-store";
import { ChatBotReactionProvider } from "../global-chat/chat-bot-reaction";
import { useHomeScope } from "../home/home-content/shared";
import { homeGreetingName } from "../home/home-greeting";
import { Button } from "../ui/button";
import type { BotPanelActions } from "./bot-details";
import type { BotPanelTab } from "./bot-panel-tabs";
import { BotVisibilityDialog } from "./bot-visibility-dialog";
import { shownBots } from "./bot-visibility-model";
import {
	type CrewData,
	CrewDataScope,
	revealPromptChat,
	useBotIds,
	useCrewPickerSource,
	useNeedsYou,
} from "./crew-data";
import { CrewDock } from "./crew-dock";
import {
	CREW_GRID_CLASS,
	CrewBotCard,
	CrewGridSkeleton,
	type CrewMember,
	NewBotTile,
	useCrewMembers,
	useShownCrew,
} from "./crew-grid";
import { CrewHeader } from "./crew-header";
import {
	crewPrincipalIds,
	crewSummaryLine,
	hasBotsHostedElsewhere,
	matchesSearch,
	otherMemberIds,
	splitConversations,
} from "./crew-model";
import { CrewSection } from "./crew-section";
import { CrewSplitLayout, type DockPanelLayout } from "./crew-split-layout";
import {
	type DockSelection,
	dockSelectionForBot,
	dockSelectionForChatBot,
	dockSelectionKey,
	resolveDockSelection,
} from "./dock-selection";
import { BotEditorDialog } from "./editor";
import { LocalBotsBanner } from "./local-bots-banner";
import { requestMessageJump } from "./message-jump";
import { StarredMessagesDialog } from "./message-marks";
import { MoveLocalBotsDialog } from "./move-local-bots-dialog";
import { NeedsYouSection } from "./needs-you-section";
import { NewMessageDialog } from "./new-message-dialog";
import { NewRoomDialog } from "./new-room-dialog";
import { RoomsSection } from "./rooms-section";
import { useConversationText } from "./use-conversation-text";
import { useCrewDirectory } from "./use-crew-directory";
import { useDockSelection } from "./use-dock-selection";
import { moveCheckKeys } from "./use-local-bot-moves";
import { useMoveRequest } from "./use-move-request";

const NO_BOTS: readonly IBot[] = [];

function useGreetingName(): string | undefined {
	const auth = useAuth();
	const backend = useBackend();
	const scope = useHomeScope();
	const account = useQuery({
		queryKey: [backend.userState.getInfo.name, ...scope],
		queryFn: () => backend.userState.getInfo(),
		enabled: auth.isAuthenticated && Boolean(auth.user?.access_token),
		staleTime: 60_000,
		retry: false,
	});
	return homeGreetingName(undefined, account.data, auth.user?.profile);
}

/**
 * Shows the global chat each time it asks to be seen: a queued draft, an opened activity entry, or
 * a tool waiting for the user. Requests from before this view mounted are not replayed, except a
 * draft still waiting to be sent.
 */
function useRevealGlobalChat(reveal: () => void) {
	const revealRequest = useGlobalChatStore((s) => s.revealRequest);
	const [draftAtMount] = useState(
		() => useGlobalChatStore.getState().draft !== null,
	);
	const seen = useRef<number | null>(draftAtMount ? null : revealRequest);
	useEffect(() => {
		if (seen.current === revealRequest) return;
		seen.current = revealRequest;
		reveal();
	}, [revealRequest, reveal]);
}

function CrewNotice({
	children,
	action,
}: Readonly<{ children: string; action?: ReactNode }>) {
	return (
		<div className="flex flex-wrap items-center gap-3 rounded-2xl border border-dashed px-4 py-3 text-sm text-muted-foreground">
			<span className="min-w-0 flex-1">{children}</span>
			{action}
		</div>
	);
}

/**
 * How the dock shows a bot's chat: with its panel open or not, and whether that panel is a sheet
 * that hides the chat (a column leaves it on screen).
 */
function dockChatView(
	dockOpen: boolean,
	selection: DockSelection,
	panel: BotPanelTab | null,
	layout: DockPanelLayout,
) {
	const chat = dockOpen && selection.type !== "conversation";
	const covered = panel !== null && layout === "sheet";
	return {
		panelOpen: chat && panel !== null,
		covered,
		chatOnScreen: chat && !covered,
	};
}

const MOVE_CLOSED = { open: false, botId: null } as const;

/**
 * The review that moves bots from this computer to the account: opened by the banner, a bot's
 * chat (just that bot) or `?move=local` appearing in the URL, and only while signed in where
 * bots live locally. Closing drops the parameter.
 */
function useMoveReview(available: boolean) {
	const { requested, dismiss } = useMoveRequest();
	const [shown, setShown] = useState<{ open: boolean; botId: string | null }>(
		() => (requested ? { open: true, botId: null } : MOVE_CLOSED),
	);
	const [seenRequest, setSeenRequest] = useState(requested);
	if (requested !== seenRequest) {
		setSeenRequest(requested);
		if (requested) setShown({ open: true, botId: null });
	}
	const show = useCallback(
		(botId: string | null) => setShown({ open: true, botId }),
		[],
	);
	const close = useCallback(() => {
		setShown(MOVE_CLOSED);
		dismiss();
	}, [dismiss]);
	return {
		open: available && shown.open,
		botId: shown.botId,
		show,
		close,
	};
}

function CrewLayout(data: Readonly<CrewData>) {
	const { t } = useTranslation("bots");
	const queryClient = useQueryClient();
	const { me, mode, bots, conversations } = data;
	const signedIn = mode === "account";
	const hasCrew = mode !== "guest";
	const keepsLocalBots = Boolean(useBackend().botState?.local);
	const canMove = signedIn && keepsLocalBots;
	const {
		selection,
		panel,
		dockOpen,
		select,
		selectPanel,
		closeDock,
		openDock,
	} = useDockSelection();
	const resolved = useMemo(
		() => resolveDockSelection(selection, bots, conversations, me),
		[selection, bots, conversations, me],
	);
	const selectedKey = dockOpen ? dockSelectionKey(resolved) : null;
	const [panelLayout, setPanelLayout] = useState<DockPanelLayout>("sheet");
	const {
		panelOpen,
		covered: panelCoversChat,
		chatOnScreen: showingGlobalChat,
	} = dockChatView(dockOpen, resolved, panel, panelLayout);
	const panelControl = useMemo(
		() => ({ tab: panel, onTabChange: selectPanel }),
		[panel, selectPanel],
	);

	const [search, setSearch] = useState("");
	const [editor, setEditor] = useState<{ open: boolean; botId: string | null }>(
		{ open: false, botId: null },
	);
	const [newRoomOpen, setNewRoomOpen] = useState(false);
	const [newMessageOpen, setNewMessageOpen] = useState(false);
	const [starredOpen, setStarredOpen] = useState(false);
	const [visibilityOpen, setVisibilityOpen] = useState(false);

	const allCrew = useCrewMembers(bots);
	const crew = useShownCrew(allCrew, data.hiddenBots);
	const hiddenCount = allCrew.length - crew.length;
	const showVisibility = useCallback(() => setVisibilityOpen(true), []);
	const presence = useBotPresence(hasBotsHostedElsewhere(bots)).data;
	const myBotIds = useBotIds(bots);
	const localBots = useMemo(
		() => bots?.filter((bot) => bot.local) ?? NO_BOTS,
		[bots],
	);
	const move = useMoveReview(canMove);
	const { items: needs, invitations } = useNeedsYou(conversations, {
		me,
		myBotIds,
		openConversationId:
			dockOpen && resolved.type === "conversation"
				? resolved.conversationId
				: null,
		showingGlobalChat,
	});
	const principalIds = useMemo(
		() => crewPrincipalIds(conversations, invitations),
		[conversations, invitations],
	);
	const directory = useCrewDirectory(principalIds, { me, bots });
	const text = useConversationText(directory, me);
	const pickerSource = useCrewPickerSource(data, directory);
	const greetingName = useGreetingName();

	const openConversation = useCallback(
		(conversationId: string) => {
			if (
				selection.type === "conversation" &&
				selection.conversationId === conversationId &&
				panelCoversChat
			) {
				selectPanel(null);
				return;
			}
			select({ type: "conversation", conversationId });
		},
		[panelCoversChat, select, selectPanel, selection],
	);
	const openStarred = useCallback(
		(conversationId: string, messageId: string) => {
			requestMessageJump(conversationId, messageId);
			openConversation(conversationId);
		},
		[openConversation],
	);
	/** Opens the dock on a bot's chat, writing the URL only when that changes what it shows. */
	const showChatOf = useCallback(
		(botId: string | null) => {
			const target = resolveDockSelection(dockSelectionForChatBot(botId), bots);
			if (dockSelectionKey(target) !== dockSelectionKey(resolved)) {
				select(target);
				return;
			}
			// Activity can restore another Flux session while the dock is closed. Keep that
			// session in the URL before the reopened dock reads its bookmark.
			const state = useGlobalChatStore.getState();
			if (botId === null && selectActiveBotId(state) === null) {
				const url = new URL(window.location.href);
				if (url.searchParams.get("session") !== state.activeConversationId) {
					url.searchParams.set("session", state.activeConversationId);
					window.history.replaceState(
						null,
						"",
						`${url.pathname}${url.search}${url.hash}`,
					);
				}
			}
			if (panelCoversChat) selectPanel(null);
			else openDock();
		},
		[bots, openDock, panelCoversChat, resolved, select, selectPanel],
	);
	/** "Show chat" reopens the dock on its chat, not under the sheet it was closed on. */
	const showDockChat = useCallback(() => {
		if (panelCoversChat) selectPanel(null);
		else openDock();
	}, [openDock, panelCoversChat, selectPanel]);
	const openPromptChat = useCallback(
		() => revealPromptChat(showChatOf),
		[showChatOf],
	);
	// A draft names the bot it was written to; FlowPilot entry points leave it to Flux.
	const revealGlobalChat = useCallback(() => {
		const state = useGlobalChatStore.getState();
		showChatOf(
			state.draft ? (state.draft.botId ?? null) : selectActiveBotId(state),
		);
	}, [showChatOf]);
	useRevealGlobalChat(revealGlobalChat);

	const query = search.trim();
	const visibleCrew = crew.filter((member) =>
		matchesSearch(query, [member.name, member.description]),
	);
	const visibleNeeds = needs.filter((item) =>
		item.type === "assistant"
			? matchesSearch(query, [item.name])
			: item.type === "invitation"
				? matchesSearch(query, [
						item.invitation.title,
						item.invitation.bot_name,
						directory(item.invitation.invited_by ?? "").name,
					])
				: matchesSearch(query, [
						text.title(item.conversation),
						directory(item.conversation.last_entry?.sender_id ?? "").name,
						text.entryText(item.conversation.last_entry),
					]),
	);
	const { rooms, direct } = splitConversations(conversations);
	const visibleRooms = rooms.filter((conversation) =>
		matchesSearch(query, [
			text.title(conversation),
			...otherMemberIds(conversation, me).map((id) => directory(id).name),
		]),
	);
	const visibleDirect = direct.filter((conversation) =>
		matchesSearch(query, [text.title(conversation)]),
	);
	const nothingFound =
		Boolean(query) &&
		visibleCrew.length === 0 &&
		visibleNeeds.length === 0 &&
		visibleRooms.length === 0 &&
		visibleDirect.length === 0;

	let summary: string;
	if (mode === "account") {
		summary = crewSummaryLine((key, options) => t(key, options), {
			needs: needs.length,
			bots: bots ? shownBots(bots, data.hiddenBots).length : null,
		});
	} else if (mode === "local") {
		summary = t(
			"local.summary",
			"Your bots and chats stay on this computer. Sign in to talk with people.",
		);
	} else {
		summary = t(
			"crew.summary.signedOut",
			"Sign in to create bots and talk with people. Assistant and Flux work without an account.",
		);
	}

	const openMember = useCallback(
		(member: CrewMember) => select(member.selection),
		[select],
	);
	const editBot = useCallback(
		(botId: string) => setEditor({ open: true, botId }),
		[],
	);
	const openSettings = useCallback((bot: IBot) => editBot(bot.id), [editBot]);
	const openNewBot = useCallback(
		() => setEditor({ open: true, botId: null }),
		[],
	);
	const setEditorOpen = useCallback(
		(open: boolean) => {
			setEditor((current) => ({ ...current, open }));
			// Whatever the editor changed may have fixed, or caused, a move issue.
			if (!open)
				void queryClient.invalidateQueries({ queryKey: moveCheckKeys.all });
		},
		[queryClient],
	);
	const selectSaved = useCallback(
		(bot: IBot) => {
			if (!editor.botId) select(dockSelectionForBot(bot, bots));
		},
		[bots, editor.botId, select],
	);
	const selectMoved = useCallback(
		(bot: IBot) => select(dockSelectionForBot(bot, bots)),
		[bots, select],
	);
	const { show: showMove, close: closeMove } = move;
	const moveBot = useCallback((bot: IBot) => showMove(bot.id), [showMove]);
	const panelActions = useMemo<BotPanelActions>(
		() => ({
			onEdit: openSettings,
			onDuplicated: selectMoved,
			onMove: canMove ? moveBot : undefined,
			onOpenConversation: openConversation,
		}),
		[canMove, moveBot, openConversation, openSettings, selectMoved],
	);
	const openPicked = useCallback(
		({ bot, conversationId }: { bot?: IBot; conversationId?: string }) => {
			if (bot) select(dockSelectionForBot(bot, bots));
			else if (conversationId) openConversation(conversationId);
		},
		[bots, openConversation, select],
	);

	return (
		<>
			<CrewSplitLayout
				panelOpen={panelOpen}
				onPanelLayoutChange={setPanelLayout}
				dock={
					dockOpen ? (
						<CrewDock
							selection={resolved}
							sourceSelection={selection}
							account={{
								me,
								mode,
								bots,
								botsLoading: data.botsLoading,
								botsFailed: data.botsFailed,
								retryBots: data.retryBots,
							}}
							actions={panelActions}
							panel={panelControl}
							onClose={closeDock}
						/>
					) : undefined
				}
			>
				<div className="mx-auto flex max-w-280 flex-col gap-8.5 px-9 pt-7 pb-14 @max-[640px]/crew:px-4 @max-[640px]/crew:pt-5 @max-[640px]/crew:pb-10 @max-[520px]/crew:gap-5">
					<CrewHeader
						canManageBlocks={signedIn}
						name={greetingName}
						summary={summary}
						search={search}
						onSearchChange={setSearch}
						onNewBot={hasCrew && bots ? openNewBot : undefined}
						onShowChat={dockOpen ? undefined : showDockChat}
						onManageVisibility={hasCrew && bots ? showVisibility : undefined}
					/>
					{canMove && localBots.length > 0 && (
						<LocalBotsBanner
							count={localBots.length}
							onReview={() => showMove(null)}
						/>
					)}
					{data.botsFailed && (
						<CrewNotice
							action={
								<Button
									type="button"
									variant="outline"
									size="sm"
									onClick={data.retryBots}
								>
									{t("crew.retry", "Try again")}
								</Button>
							}
						>
							{t(
								"crew.loadFailedFlux",
								"Couldn't load your bots. Flux still works.",
							)}
						</CrewNotice>
					)}
					<NeedsYouSection
						items={visibleNeeds}
						directory={directory}
						text={text}
						onOpenConversation={openConversation}
						onOpenAssistant={openPromptChat}
					/>
					{(!query || visibleCrew.length > 0) && (
						<CrewSection
							title={t("crew.yourCrew", "Your crew")}
							actions={
								hiddenCount > 0 && (
									<Button
										type="button"
										variant="ghost"
										size="sm"
										className="text-muted-foreground"
										onClick={showVisibility}
									>
										<EyeOffIcon aria-hidden />
										{t(
											"crew.visibility.hiddenCount",
											"{{count}} hidden in this profile",
											{ count: hiddenCount },
										)}
									</Button>
								)
							}
							aside={
								bots && (
									<span className="text-[13px] font-normal text-muted-foreground">
										{t("crew.summary.bots", "{{count}} bots", {
											count: crew.length,
											defaultValue_one: "{{count}} bot",
										})}
									</span>
								)
							}
						>
							<div className={CREW_GRID_CLASS}>
								{data.botsLoading ? (
									<CrewGridSkeleton />
								) : (
									visibleCrew.map((member) => (
										<CrewBotCard
											key={member.key}
											member={member}
											presence={botPresence(member.bot, presence)}
											selected={
												selectedKey === dockSelectionKey(member.selection)
											}
											onSelect={openMember}
											onSettings={
												member.bot
													? () => openSettings(member.bot as IBot)
													: undefined
											}
										/>
									))
								)}
								{!query && hasCrew && (
									<NewBotTile disabled={!bots} onClick={openNewBot} />
								)}
							</div>
						</CrewSection>
					)}
					{hasCrew && (
						<RoomsSection
							rooms={visibleRooms}
							direct={visibleDirect}
							context={{
								me,
								text,
								directory,
								selectedConversationId:
									selectedKey && resolved.type === "conversation"
										? resolved.conversationId
										: null,
								onOpen: openConversation,
								onLeave: (conversationId) => {
									if (
										resolved.type === "conversation" &&
										resolved.conversationId === conversationId
									)
										closeDock();
								},
							}}
							status={{
								searching: Boolean(query),
								loading: data.conversationsLoading,
								failed: data.conversationsFailed,
								onRetry: data.retryConversations,
								emptyText:
									mode === "local"
										? t(
												"local.roomsEmpty",
												"No conversations yet. Start one with one of your bots.",
											)
										: undefined,
							}}
							onNewRoom={() => setNewRoomOpen(true)}
							onNewMessage={() => setNewMessageOpen(true)}
							onShowStarred={() => setStarredOpen(true)}
						/>
					)}
					{nothingFound && (
						<p className="py-8 text-center text-sm text-muted-foreground">
							{t("crew.search.empty", "Nothing matches “{{query}}”.", {
								query,
							})}
						</p>
					)}
				</div>
			</CrewSplitLayout>
			<BotEditorDialog
				open={editor.open}
				onOpenChange={setEditorOpen}
				botId={editor.botId}
				onSaved={selectSaved}
				onDuplicated={selectMoved}
			/>
			{hasCrew && visibilityOpen && (
				<BotVisibilityDialog
					members={allCrew}
					onOpenChange={setVisibilityOpen}
				/>
			)}
			{hasCrew && (
				<>
					<NewRoomDialog
						open={newRoomOpen}
						onOpenChange={setNewRoomOpen}
						source={pickerSource}
						onCreated={openConversation}
					/>
					<NewMessageDialog
						open={newMessageOpen}
						onOpenChange={setNewMessageOpen}
						source={pickerSource}
						onPicked={openPicked}
					/>
					<StarredMessagesDialog
						open={starredOpen}
						onOpenChange={setStarredOpen}
						conversations={conversations}
						text={text}
						onOpen={openStarred}
					/>
				</>
			)}
			{canMove && (
				<MoveLocalBotsDialog
					open={move.open}
					onOpenChange={(open) => {
						if (!open) closeMove();
					}}
					bots={localBots}
					botId={move.botId}
					onEditBot={editBot}
					onMoved={selectMoved}
				/>
			)}
		</>
	);
}

/**
 * The Bots tab at `/bots`: your crew, what needs you, rooms and people, and a dock that holds
 * one conversation. The global chat overlay closes while it is open, so no chat shows twice.
 */
export function BotsView() {
	const closeOverlay = useGlobalChatStore((s) => s.closeOverlay);
	useEffect(() => {
		closeOverlay();
	}, [closeOverlay]);
	return (
		<ChatBotReactionProvider>
			<CrewDataScope>{(data) => <CrewLayout {...data} />}</CrewDataScope>
		</ChatBotReactionProvider>
	);
}
