import { useEffect } from "react";
import { watchConversation } from "../../lib/realtime/conversation-feed";
import { pushedTranscript } from "./bot-history";
import { useBotHistoryStore } from "./bot-history-store";
import { activeBotTarget } from "./bot-inbox";
import { conversationBotId } from "./global-chat-bots";
import { useGlobalChatStore } from "./global-chat-store";

/**
 * Adds what the realtime feed pushes into the bot chat on screen (a trigger, a bot ask, the bot's
 * answer to either) while it shows its newest messages; a jump window stays as it is.
 */
export function useFollowBotChat(sessionId: string): void {
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
				const store = useGlobalChatStore.getState();
				const { messages, seqs } = pushedTranscript(
					entries,
					botId,
					sessionId,
					store.messages,
				);
				const history = useBotHistoryStore.getState();
				for (const [messageId, seq] of Object.entries(seqs)) {
					history.note(botId, messageId, seq);
				}
				if (messages.length > 0) {
					store.mergeConversationMessages(sessionId, messages);
				}
				const newest = entries.at(-1);
				if (newest) history.advance(botId, newest.cursor);
			},
		);
	}, [botId, sessionId, following, cursor]);
}
