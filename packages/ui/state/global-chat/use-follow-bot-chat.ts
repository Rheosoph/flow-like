import { useQueryClient } from "@tanstack/react-query";
import { useEffect } from "react";
import { plainBody } from "../../components/bots-tab/approval-entry";
import { noteRefOf } from "../../components/bots-tab/bot-note-entry";
import { invalidateBotConfiguration } from "../../hooks/use-bots";
import { watchConversation } from "../../lib/realtime/conversation-feed";
import type { IInboxEntry } from "../backend-state/inbox-state";
import { pushedTranscript } from "./bot-history";
import { useBotHistoryStore } from "./bot-history-store";
import { activeBotTarget } from "./bot-inbox";
import { conversationBotId } from "./global-chat-bots";
import { useGlobalChatStore } from "./global-chat-store";

/**
 * Adds pushed entries to the bot chat on screen. The answer still streaming counts as shown: the
 * host stores the bot's reply before the chat commits the streamed one, under another id.
 */
export function followPushedEntries(
	botId: string,
	sessionId: string,
	entries: readonly IInboxEntry[],
): void {
	const store = useGlobalChatStore.getState();
	const { messages, seqs } = pushedTranscript(entries, botId, sessionId, [
		...store.messages,
		...store.streamingMessages,
	]);
	const history = useBotHistoryStore.getState();
	for (const [messageId, seq] of Object.entries(seqs)) {
		history.note(botId, messageId, seq);
	}
	for (const entry of entries) {
		const replyTo = plainBody(entry)?.reply_to;
		const note = entry.sender_id === botId ? noteRefOf(entry) : undefined;
		if (note && typeof replyTo === "string") history.noteReply(replyTo, note);
	}
	if (messages.length > 0) {
		store.mergeConversationMessages(sessionId, messages);
	}
	const newest = entries.at(-1);
	if (newest) history.advance(botId, newest.cursor);
}

/**
 * Adds what the realtime feed pushes into the bot chat on screen (a trigger, a bot ask, the bot's
 * answer to either) while it shows its newest messages; a jump window stays as it is.
 */
export function useFollowBotChat(sessionId: string): void {
	const queryClient = useQueryClient();
	const botId = conversationBotId(sessionId);
	const view = useBotHistoryStore((state) =>
		botId ? state.views[botId] : undefined,
	);
	const following = Boolean(view && !view.windowed);
	const cursor = view?.cursor ?? undefined;

	useEffect(() => {
		if (!botId || !following) return;
		const target = activeBotTarget(sessionId);
		if (!target) return;
		return watchConversation(
			{
				conversationId: target.conversationId,
				...(cursor ? { after: cursor } : {}),
			},
			(entries) => {
				followPushedEntries(botId, sessionId, entries);
				// A headless reply can follow a settings change approved from this transcript.
				if (
					entries.some(
						(entry) =>
							entry.sender_id === botId &&
							entry.kind === "message" &&
							!plainBody(entry)?.approval,
					)
				) {
					invalidateBotConfiguration(queryClient, botId);
				}
			},
		);
	}, [botId, sessionId, following, cursor, queryClient]);
}
