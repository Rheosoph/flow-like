"use client";

import { useTranslation } from "@flow-like/locales";
import { useQueryClient } from "@tanstack/react-query";
import {
	type ReactNode,
	useCallback,
	useEffect,
	useMemo,
	useRef,
	useState,
} from "react";
import { useOrg } from "../../hooks/use-bot-teams";
import { emitGuideEvent } from "../../lib/guide/guide-events";
import { useBackend } from "../../state/backend-state";
import type { IBot } from "../../state/backend-state/bot-state";
import {
	selectActiveBotId,
	useGlobalChatStore,
} from "../../state/global-chat/global-chat-store";
import { ChatBotReactionProvider } from "../global-chat/chat-bot-reaction";
import { Button } from "../ui/button";
import type { BotPanelActions } from "./bot-details";
import type { BotPanelTab } from "./bot-panel-tabs";
import { BotVisibilityDialog } from "./bot-visibility-dialog";
import { BotsChatsHeader } from "./bots-chats-header";
import { ChatEmptyDock } from "./chat-empty-dock";
import {
	type CrewData,
	CrewDataScope,
	NO_CONVERSATIONS,
	revealPromptChat,
	useBotIds,
	useCrewPickerSource,
	useNeedsYou,
} from "./crew-data";
import { CrewDock } from "./crew-dock";
import { type CrewMember, useCrewMembers, useShownCrew } from "./crew-grid";
import { crewPrincipalIds, matchesSearch } from "./crew-model";
import { CrewSplitLayout, type DockPanelLayout } from "./crew-split-layout";
import {
	type DockSelection,
	dockSelectionForBot,
	dockSelectionForChatBot,
	dockSelectionKey,
	resolveDockSelection,
} from "./dock-selection";
import { BotEditorDialog } from "./editor";
import { LocalBotsBanner, useLocalBotsBanner } from "./local-bots-banner";
import { requestMessageJump } from "./message-jump";
import { StarredMessagesDialog } from "./message-marks";
import { MoveLocalBotsDialog } from "./move-local-bots-dialog";
import { NewMessageDialog } from "./new-message-dialog";
import { NewRoomDialog } from "./new-room-dialog";
import { OrgChartView } from "./org-chart";
import { TeamChatList } from "./team-chat-list";
import { teamRooms } from "./team-chat-list-model";
import { useBotsView } from "./use-bots-view";
import { useConversationText } from "./use-conversation-text";
import { useCrewDirectory } from "./use-crew-directory";
import { useDockSelection } from "./use-dock-selection";
import { moveCheckKeys } from "./use-local-bot-moves";
import { useMoveRequest } from "./use-move-request";

const NO_BOTS: readonly IBot[] = [];

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
	const { view: urlView, setView } = useBotsView();
	const { org, available: teamsAvailable } = useOrg();
	const showViewSwitch = teamsAvailable && signedIn;
	const view = showViewSwitch ? urlView : "chats";
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
	const showVisibility = useCallback(() => setVisibilityOpen(true), []);
	const myBotIds = useBotIds(bots);
	const localBots = useMemo(
		() => bots?.filter((bot) => bot.local) ?? NO_BOTS,
		[bots],
	);
	const move = useMoveReview(canMove);
	const localBanner = useLocalBotsBanner(canMove ? me : null, localBots);
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
		() => [
			...crewPrincipalIds(conversations, invitations),
			...org.bots.map((bot) => bot.owner_id),
			...org.invitations.flatMap((item) =>
				item.invited_by ? [item.invited_by] : [],
			),
		],
		[conversations, invitations, org],
	);
	const directory = useCrewDirectory(principalIds, { me, bots });
	const text = useConversationText(directory, me);
	const pickerSource = useCrewPickerSource(data, directory);

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
	const listContext = useMemo(
		() => ({
			me,
			text,
			directory,
			selectedConversationId:
				selectedKey && resolved.type === "conversation"
					? resolved.conversationId
					: null,
			onOpen: openConversation,
			onLeave: (conversationId: string) => {
				if (
					resolved.type === "conversation" &&
					resolved.conversationId === conversationId
				)
					closeDock();
			},
		}),
		[me, text, directory, selectedKey, resolved, openConversation, closeDock],
	);
	const rooms = useMemo(
		() => teamRooms(org, conversations),
		[org, conversations],
	);

	const openMember = useCallback(
		(member: CrewMember) => {
			select(member.selection);
			emitGuideEvent("bots:chat-opened");
		},
		[select],
	);
	const openChatOf = useCallback(
		(botId: string) => {
			const bot = bots?.find((item) => item.id === botId);
			select(bot ? dockSelectionForBot(bot, bots) : { type: "bot", botId });
		},
		[bots, select],
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

	const header = (
		<BotsChatsHeader
			view={view}
			onViewChange={setView}
			showViewSwitch={showViewSwitch}
			search={search}
			onSearchChange={setSearch}
			canManageBlocks={signedIn}
			onNewBot={hasCrew && bots ? openNewBot : undefined}
			onNewRoom={hasCrew ? () => setNewRoomOpen(true) : undefined}
			onNewMessage={hasCrew ? () => setNewMessageOpen(true) : undefined}
			onShowStarred={hasCrew ? () => setStarredOpen(true) : undefined}
			onManageVisibility={hasCrew && bots ? showVisibility : undefined}
		/>
	);
	const banners = (
		<>
			{localBanner.shown && (
				<div className="px-3 pt-3">
					<LocalBotsBanner
						count={localBots.length}
						onReview={() => showMove(null)}
						onDismiss={localBanner.hide}
					/>
				</div>
			)}
			{data.botsFailed && (
				<div className="px-3 pt-3">
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
				</div>
			)}
		</>
	);

	return (
		<>
			{view === "org" ? (
				<div className="flex h-full min-h-0 w-full flex-1 flex-col">
					{header}
					{banners}
					<div className="flex min-h-0 flex-1 flex-col">
						<OrgChartView
							me={me}
							bots={bots}
							onOpenChat={openChatOf}
							onOpenConversation={openConversation}
						/>
					</div>
				</div>
			) : (
				<CrewSplitLayout
					listFirst
					header={header}
					panelOpen={panelOpen}
					onPanelLayoutChange={setPanelLayout}
					onCloseDock={closeDock}
					emptyDock={
						<ChatEmptyDock
							rooms={rooms}
							onOpenConversation={openConversation}
						/>
					}
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
					{banners}
					<TeamChatList
						org={org}
						conversations={hasCrew ? conversations : NO_CONVERSATIONS}
						crew={crew}
						context={listContext}
						needs={visibleNeeds}
						search={search}
						selectedKey={selectedKey}
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
						loadingCrew={data.botsLoading}
						onOpenConversation={openConversation}
						onOpenMember={openMember}
						onOpenAssistant={openPromptChat}
						onSettings={openSettings}
					/>
				</CrewSplitLayout>
			)}
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
 * The Bots tab at `/bots`: chats grouped by team beside the open conversation, or the org chart. The global chat overlay closes while it is open, so no chat shows twice.
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
