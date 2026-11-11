"use client";

import { type ReactNode, useMemo } from "react";
import { useAuth } from "react-oidc-context";
import { useBotAvatarSync, useBots } from "../../hooks/use-bots";
import { useConversations, useInvitations } from "../../hooks/use-inbox";
import { LocalPrincipalsProvider } from "../../hooks/use-user-lookup";
import { useBackend, useSignedIn } from "../../state/backend-state";
import { type IBot, LOCAL_OWNER_ID } from "../../state/backend-state/bot-state";
import type {
	IConversation,
	IInvitation,
} from "../../state/backend-state/inbox-state";
import { conversationBotId } from "../../state/global-chat/global-chat-bots";
import {
	selectActiveBotId,
	selectToolPromptConversationId,
	selectToolPromptHere,
	useGlobalChatStore,
} from "../../state/global-chat/global-chat-store";
import { restoreGlobalChatConversation } from "../../state/global-chat/global-chat-stream";
import { useGlobalChatBot } from "../global-chat/use-global-chat-bot";
import { NO_HIDDEN_BOTS } from "./bot-visibility-model";
import { type CrewMode, recentPeopleIds, selectNeedsYou } from "./crew-model";
import { useCrewLocalPrincipals } from "./local-principals";
import type { NeedsYouItem } from "./needs-you-section";
import type { PickerSource } from "./principal-picker";
import { useHiddenBots } from "./use-bot-visibility";
import type { CrewDirectory } from "./use-crew-directory";

export interface CrewData {
	me: string | null;
	mode: CrewMode;
	bots?: IBot[];
	/** Visibility keys of the bots the current profile hides; `bots` still holds them for the chats that name them. */
	hiddenBots: ReadonlySet<string>;
	botsLoading: boolean;
	botsFailed: boolean;
	retryBots: () => void;
	conversations: readonly IConversation[];
	conversationsLoading: boolean;
	conversationsFailed: boolean;
	retryConversations: () => void;
}

export const NO_CONVERSATIONS: readonly IConversation[] = [];
const NO_BOTS: readonly IBot[] = [];
const NO_INVITATIONS: readonly IInvitation[] = [];
const noop = () => undefined;

/** Signed out where bots can't live on the device: Assistant and Flux only. */
const GUEST_CREW: CrewData = {
	me: null,
	mode: "guest",
	hiddenBots: NO_HIDDEN_BOTS,
	botsLoading: false,
	botsFailed: false,
	retryBots: noop,
	conversations: NO_CONVERSATIONS,
	conversationsLoading: false,
	conversationsFailed: false,
	retryConversations: noop,
};

/** Who reads the crew: the account's sub, `local` where bots live on this device signed out, else a guest. */
export function useCrewAccount(): { me: string | null; mode: CrewMode } {
	const signedIn = useSignedIn();
	const sub = useAuth().user?.profile.sub ?? null;
	const keepsLocalBots = Boolean(useBackend().botState?.local);
	if (signedIn) return { me: sub, mode: "account" };
	return keepsLocalBots
		? { me: LOCAL_OWNER_ID, mode: "local" }
		: { me: null, mode: "guest" };
}

/** Bots and conversations from the routed states; local ones resolve without the directory. */
function ConnectedCrewData({
	me,
	mode,
	children,
}: Readonly<{
	me: string | null;
	mode: Exclude<CrewMode, "guest">;
	children: (data: CrewData) => ReactNode;
}>) {
	const bots = useBots();
	useBotAvatarSync(mode === "account" ? bots.data : undefined);
	const hiddenBots = useHiddenBots().keys;
	const conversations = useConversations();
	const list = conversations.data ?? NO_CONVERSATIONS;
	const resolveLocal = useCrewLocalPrincipals(bots.data, list);
	return (
		<LocalPrincipalsProvider value={resolveLocal}>
			{children({
				me,
				mode,
				bots: bots.data,
				hiddenBots,
				botsLoading: bots.isPending && bots.fetchStatus !== "idle",
				botsFailed: bots.isError && !bots.data,
				retryBots: () => void bots.refetch(),
				conversations: list,
				conversationsLoading:
					conversations.isPending && conversations.fetchStatus !== "idle",
				conversationsFailed: conversations.isError && !conversations.data,
				retryConversations: () => void conversations.refetch(),
			})}
		</LocalPrincipalsProvider>
	);
}

/**
 * The reader's bots and conversations, for every view of the crew. Switching account, this
 * computer or guest remounts what it renders, so no view keeps state from another reader.
 */
export function CrewDataScope({
	children,
}: Readonly<{ children: (data: CrewData) => ReactNode }>) {
	const { me, mode } = useCrewAccount();
	if (mode === "guest") return children(GUEST_CREW);
	return (
		<ConnectedCrewData key={mode} me={me} mode={mode}>
			{children}
		</ConnectedCrewData>
	);
}

/** The chat whose run waits for an answer; a prompt no run owns stays with the open chat. */
export function selectPromptChat(
	state: ReturnType<typeof useGlobalChatStore.getState>,
) {
	return state.toolPrompt
		? (selectToolPromptConversationId(state) ?? state.activeConversationId)
		: null;
}

/**
 * The card for a run waiting for an answer, named after the bot that asks. The prompt itself shows
 * only in the asking chat, so there's no card while that very chat is on screen.
 */
function usePromptNeed(showingGlobalChat: boolean): NeedsYouItem | null {
	const promptChat = useGlobalChatStore(selectPromptChat);
	const askingBot = useGlobalChatBot(promptChat ?? undefined);
	const askingChatOnScreen = useGlobalChatStore(selectToolPromptHere);
	const hidden =
		promptChat === null || (showingGlobalChat && askingChatOnScreen);
	return useMemo(
		() =>
			hidden
				? null
				: {
						type: "assistant",
						name: askingBot.name,
						appearance: askingBot.appearance,
					},
		[askingBot.appearance, askingBot.name, hidden],
	);
}

/**
 * Shows the asking chat itself, which for Flux may be another of its chats than the last one:
 * `show` gets the bot whose chat to open once that chat is the active one.
 */
export function revealPromptChat(show: (botId: string | null) => void) {
	const state = useGlobalChatStore.getState();
	const promptChat = selectPromptChat(state);
	if (promptChat === null) return;
	const botId = conversationBotId(promptChat);
	if (promptChat === state.activeConversationId) {
		show(botId);
		return;
	}
	void restoreGlobalChatConversation(promptChat)
		.catch(() => false)
		.then((shown) => {
			if (shown) show(botId);
		});
}

/** What waits on the reader: a live question or approval, invitations, and inbox approvals. */
export function useNeedsYou(
	conversations: readonly IConversation[],
	options: {
		me: string | null;
		myBotIds: ReadonlySet<string>;
		openConversationId: string | null;
		showingGlobalChat: boolean;
	},
): { items: NeedsYouItem[]; invitations: readonly IInvitation[] } {
	const { me, myBotIds, openConversationId, showingGlobalChat } = options;
	const invitations = useInvitations().data ?? NO_INVITATIONS;
	const promptNeed = usePromptNeed(showingGlobalChat);
	const openBotId = useGlobalChatStore((state) =>
		showingGlobalChat && !state.conversationLoading
			? selectActiveBotId(state)
			: null,
	);
	const items = useMemo<NeedsYouItem[]>(() => {
		const waiting = selectNeedsYou(conversations, {
			me,
			myBotIds,
			openConversationId,
			openBotId,
		}).map<NeedsYouItem>((conversation) => ({
			type: "conversation",
			conversation,
		}));
		const invited = invitations.map<NeedsYouItem>((invitation) => ({
			type: "invitation",
			invitation,
		}));
		return promptNeed
			? [promptNeed, ...invited, ...waiting]
			: [...invited, ...waiting];
	}, [
		conversations,
		invitations,
		me,
		myBotIds,
		openConversationId,
		openBotId,
		promptNeed,
	]);
	return { items, invitations };
}

/** The ids of the reader's bots, for telling their messages from other people's. */
export function useBotIds(bots: readonly IBot[] | undefined) {
	return useMemo(() => new Set((bots ?? NO_BOTS).map((bot) => bot.id)), [bots]);
}

/** What the new room and new message pickers offer: your bots, recent people and the directory. */
export function useCrewPickerSource(
	data: Pick<
		CrewData,
		"me" | "mode" | "bots" | "hiddenBots" | "conversations"
	>,
	directory: CrewDirectory,
): PickerSource {
	const { me, mode, bots, hiddenBots, conversations } = data;
	const recent = useMemo(
		() => recentPeopleIds(conversations, me),
		[conversations, me],
	);
	return useMemo<PickerSource>(
		() => ({
			me,
			bots: bots ?? NO_BOTS,
			hiddenBots,
			recentPeopleIds: recent,
			directory,
			people: mode === "account",
		}),
		[bots, directory, hiddenBots, me, mode, recent],
	);
}
