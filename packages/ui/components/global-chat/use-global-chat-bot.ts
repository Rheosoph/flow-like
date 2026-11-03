"use client";

import { useTranslation } from "@flow-like/locales";
import { useQuery } from "@tanstack/react-query";
import { useEffect, useMemo } from "react";
import { botKeys, useBotLabels } from "../../hooks/use-bots";
import { useBackend, useSignedIn } from "../../state/backend-state";
import {
	type BotSystemKey,
	type IBot,
	SYSTEM_BOT_APPEARANCES,
} from "../../state/backend-state/bot-state";
import {
	ASSISTANT_BOT_ID,
	botConversationId,
	conversationBotId,
	forgetMovedConversation,
} from "../../state/global-chat/global-chat-bots";
import {
	selectActiveBotId,
	useGlobalChatStore,
} from "../../state/global-chat/global-chat-store";
import type { BotAppearance } from "../bots/types";

/** Who the global chat talks to, as its header shows it. */
export interface GlobalChatBotView {
	/** Null for Flux; `general` for the Assistant without an account row. */
	botId: string | null;
	/** The bot's row, when the caller's bot list holds it. */
	bot?: IBot;
	name: string;
	description: string;
	appearance: Partial<BotAppearance>;
}

type BotLabels = ReturnType<typeof useBotLabels>;

function builtInKey(botId: string | null): BotSystemKey | null {
	if (botId === null) return "flux";
	return botId === ASSISTANT_BOT_ID ? "general" : null;
}

/**
 * Built-in bots fall back to their default look and words, and prefer the account's row to one on
 * this computer; a missing bot shows `missingName`.
 */
export function describeChatBot(
	botId: string | null,
	bots: readonly IBot[] | undefined,
	labels: BotLabels,
	missingName: string,
): GlobalChatBotView {
	const system = builtInKey(botId);
	const bot = system
		? (bots?.find((item) => item.system === system && !item.local) ??
			bots?.find((item) => item.system === system))
		: bots?.find((item) => item.id === botId);
	if (bot) return { botId, bot, ...labels(bot), appearance: bot.appearance };
	if (system) {
		return {
			botId,
			...labels({ system }),
			appearance: SYSTEM_BOT_APPEARANCES[system],
		};
	}
	return { botId, name: missingName, description: "", appearance: {} };
}

/** The caller's bots, read only where the Bots tab would read them too. */
export function useChatBotList() {
	const state = useBackend().botState;
	const signedIn = useSignedIn();
	return useQuery({
		queryKey: botKeys.list(),
		queryFn: () => {
			if (!state) throw new Error("Bots need a signed-in Flow-Like account");
			return state.listBots();
		},
		enabled: Boolean(state) && (signedIn || Boolean(state?.local)),
		staleTime: 30_000,
	});
}

/** The Assistant row that answers for `general`: the account's signed in, else this computer's. */
function assistantRowFor(
	bots: readonly IBot[],
	signedIn: boolean,
): IBot | undefined {
	return bots.find(
		(bot) => bot.system === "general" && Boolean(bot.local) !== signedIn,
	);
}

/**
 * Which row answers for the chats of Assistants that no longer show: `general` goes to the
 * Assistant's row; signed in, an Assistant on this computer the list hides (never edited here, the
 * account has its own) goes to the account's. A Flux row never holds a `bot:` chat.
 */
function chatHandOvers(
	bots: readonly IBot[],
	signedIn: boolean,
	onThisComputer: readonly IBot[],
): Map<string, string> {
	const handOvers = new Map<string, string>();
	const assistant = assistantRowFor(bots, signedIn);
	if (!assistant) return handOvers;
	handOvers.set(ASSISTANT_BOT_ID, assistant.id);
	if (!signedIn) return handOvers;
	const listed = new Set(bots.map((bot) => bot.id));
	for (const bot of onThisComputer) {
		if (bot.system === "general" && !listed.has(bot.id)) {
			handOvers.set(bot.id, assistant.id);
		}
	}
	return handOvers;
}

/** Chats handed to another row in this app session, by the bot id they had. */
const handedOver = new Map<string, string>();

/** Moves each chat once; one no longer handed over answers under its own id again. */
function applyChatHandOvers(wanted: ReadonlyMap<string, string>) {
	for (const [from, to] of handedOver) {
		if (wanted.get(from) === to) continue;
		handedOver.delete(from);
		forgetMovedConversation(botConversationId(from));
	}
	for (const [from, to] of wanted) {
		if (handedOver.get(from) === to) continue;
		handedOver.set(from, to);
		void useGlobalChatStore
			.getState()
			.rebindBotChat(from, to)
			.catch((cause: unknown) => {
				handedOver.delete(from);
				console.warn(`The chat of bot ${from} couldn't move to ${to}`, cause);
			});
	}
}

/** Signed in on a computer that keeps bots: every bot there, the hidden built-ins included. */
function useBotsOnThisComputer(signedIn: boolean) {
	const local = useBackend().botState?.local;
	return useQuery({
		queryKey: [...botKeys.all, "on-this-computer"],
		queryFn: () => {
			if (!local) throw new Error("This computer keeps no bots");
			return local.list();
		},
		enabled: signedIn && Boolean(local),
		staleTime: 30_000,
	});
}

/**
 * Chats of Assistants that no longer show move to the row that does, once: `bot:general` (from
 * before a row existed) and, signed in, a hidden Assistant's on this computer. Without a row the
 * Assistant is `general` again. A failed or loading list changes nothing. Mounted once, by the
 * overlay every shell keeps.
 */
export function useAdoptAssistantChat() {
	const signedIn = useSignedIn();
	const botState = useBackend().botState;
	const listed = Boolean(botState) && (signedIn || Boolean(botState?.local));
	const list = useChatBotList();
	const onThisComputer = useBotsOnThisComputer(signedIn);
	const localSettled =
		!(signedIn && botState?.local) ||
		(onThisComputer.isSuccess && !onThisComputer.isFetching);
	const settled = list.isSuccess && !list.isFetching && localSettled;
	const wanted = useMemo(() => {
		if (!listed) return new Map<string, string>();
		if (!settled || !list.data) return null;
		return chatHandOvers(list.data, signedIn, onThisComputer.data ?? []);
	}, [listed, settled, list.data, signedIn, onThisComputer.data]);
	useEffect(() => {
		if (wanted) applyChatHandOvers(wanted);
	}, [wanted]);
}

/** The bot `botId` names (`null` for Flux, `general` for the Assistant without a row), as its chat shows it. */
export function useChatBotView(botId: string | null): GlobalChatBotView {
	const { t } = useTranslation("bots");
	const labels = useBotLabels();
	const bots = useChatBotList().data;
	const missingName = bots
		? t("identity.deleted", "Deleted bot")
		: t("identity.bot", "Bot");
	return useMemo(
		() => describeChatBot(botId, bots, labels, missingName),
		[botId, bots, labels, missingName],
	);
}

/**
 * The bot a conversation (the active one by default) belongs to: Flux, the Assistant or one of the
 * caller's bots.
 */
export function useGlobalChatBot(conversationId?: string): GlobalChatBotView {
	const activeBotId = useGlobalChatStore(selectActiveBotId);
	return useChatBotView(
		conversationId === undefined
			? activeBotId
			: conversationBotId(conversationId),
	);
}
