"use client";

import { useMemo, useRef } from "react";
import { useConversationReplyDrafts } from "../../hooks/use-conversation-reply-drafts";
import { useSmoothTexts } from "../../hooks/use-smooth-text";
import { IRole } from "../../lib/schema/llm/history";
import { activeBotTarget } from "../../state/global-chat/bot-inbox";
import { GLOBAL_CHAT_APP_ID } from "../../state/global-chat/global-chat-db";
import { useGlobalChatStore } from "../../state/global-chat/global-chat-store";
import type { IMessage } from "../interfaces/chat-default/chat-db";

const NO_MESSAGES: readonly IMessage[] = [];

/**
 * The reply a bot that answers on its own host is writing into this chat, as live bubbles keyed by
 * the id its stored message will have. They never enter the store, where a stored reply under that
 * id would read as already shown. A bubble leaves once that message is in the transcript, which
 * then shows in its place. `botId` is null where this chat runs the bot itself.
 */
export function useHostedReplyDrafts(
	sessionId: string,
	botId: string | null,
): readonly IMessage[] {
	const conversationId = botId
		? activeBotTarget(sessionId)?.conversationId
		: undefined;
	const drafts = useConversationReplyDrafts(conversationId);
	const stored = useGlobalChatStore((state) => state.messages);
	const live = useMemo(() => {
		if (drafts.length === 0) return drafts;
		const shown = new Set(stored.map((message) => message.id));
		return drafts.filter(
			(draft) =>
				draft.senderId === botId &&
				!shown.has(draft.messageId) &&
				draft.text.trim().length > 0,
		);
	}, [drafts, stored, botId]);
	const targets = useMemo(
		() => new Map(live.map((draft) => [draft.messageId, draft.text])),
		[live],
	);
	const texts = useSmoothTexts(targets);
	const firstSeen = useRef(new Map<string, number>());

	return useMemo(() => {
		if (live.length === 0) return NO_MESSAGES;
		return live.map((draft): IMessage => {
			let timestamp = firstSeen.current.get(draft.messageId);
			if (timestamp === undefined) {
				timestamp = Date.now();
				firstSeen.current.set(draft.messageId, timestamp);
			}
			return {
				id: draft.messageId,
				appId: GLOBAL_CHAT_APP_ID,
				sessionId,
				inner: {
					role: IRole.Assistant,
					content: texts.get(draft.messageId) ?? "",
				},
				files: [],
				tools: [],
				actions: [],
				timestamp,
			};
		});
	}, [live, texts, sessionId]);
}
