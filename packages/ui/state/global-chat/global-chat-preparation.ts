import { createId } from "@paralleldrive/cuid2";
import { create } from "zustand";
import { currentConversationId } from "./global-chat-bots";

export interface PreparingChatReply {
	runId: string;
	conversationId: string;
	finish: () => void;
}

/** Setup survives switching between the page and dock, but never enters saved history. */
export const useChatPreparationStore = create<{
	replies: Readonly<Record<string, PreparingChatReply>>;
}>(() => ({ replies: {} }));

export function preparingChatReplies(
	conversationId: string,
): PreparingChatReply[] {
	const target = currentConversationId(conversationId);
	return Object.values(useChatPreparationStore.getState().replies).filter(
		(reply) => currentConversationId(reply.conversationId) === target,
	);
}

export function prepareChatReply(conversationId: string): PreparingChatReply {
	const runId = createId();
	const finish = () => {
		useChatPreparationStore.setState((state) => {
			const { replies } = state;
			if (!replies[runId]) return state;
			const { [runId]: _finished, ...remaining } = replies;
			return { replies: remaining };
		});
	};
	const reply: PreparingChatReply = {
		runId,
		conversationId,
		finish,
	};
	useChatPreparationStore.setState(({ replies }) => ({
		replies: { ...replies, [runId]: reply },
	}));
	return reply;
}
