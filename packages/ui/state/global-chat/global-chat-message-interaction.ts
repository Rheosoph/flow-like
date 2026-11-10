import type { IMessage } from "../../components/interfaces/chat-default/chat-db";
import {
	type IChatMessageInteraction,
	applyMessageReaction,
	historyMessageForModel,
} from "../../components/interfaces/chat-default/chat-message-interaction";
import { joinContentText } from "../../components/interfaces/chat-default/inline-segments";
import { userContextAwareHistory } from "../../lib/bot-user-context-history";
import { globalChatDb } from "./global-chat-db";
import { useGlobalChatStore } from "./global-chat-store";
import { persistGlobalChatMessage } from "./global-chat-stream";

/** Keep the user's text in the transcript and send referenced context only to the model. */
export function globalChatModelInput(
	message: IMessage,
	history: readonly IMessage[],
	now = Date.now(),
) {
	return {
		prompt: joinContentText(historyMessageForModel(message).content),
		history: userContextAwareHistory(history, now),
	};
}

/** Apply an accepted reaction once; replaying a queued operation cannot toggle it back. */
export async function persistGlobalChatReaction(
	interaction: IChatMessageInteraction | undefined,
	conversationId: string,
) {
	if (interaction?.type !== "reaction") return;
	const store = useGlobalChatStore.getState();
	const target =
		store.messages.find(
			(message) => message.id === interaction.target.messageId,
		) ?? (await globalChatDb.messages.get(interaction.target.messageId));
	if (!target || target.sessionId !== conversationId) {
		throw new Error("The message is no longer available in this conversation.");
	}
	const next = {
		...target,
		reactions: applyMessageReaction(
			target.reactions,
			interaction.emoji,
			interaction.operation,
			interaction.replacesEmoji,
		),
	};
	if (!(await persistGlobalChatMessage(next))) {
		throw new Error("The reaction could not be saved.");
	}
	useGlobalChatStore.getState().commitMessage(next);
}
