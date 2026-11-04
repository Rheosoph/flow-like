"use client";

import { useEffect, useRef, useSyncExternalStore } from "react";
import { useMarkConversationRead } from "../../hooks/use-inbox";
import { compareInboxCursors } from "../../lib/inbox-cursor";
import { useBackend } from "../../state/backend-state";
import { useBotHistoryStore } from "../../state/global-chat/bot-history-store";
import { activeBotTarget, botInbox } from "../../state/global-chat/bot-inbox";
import { conversationBotId } from "../../state/global-chat/global-chat-bots";
import { useGlobalChatStore } from "../../state/global-chat/global-chat-store";

function subscribeVisibility(onChange: () => void) {
	document.addEventListener("visibilitychange", onChange);
	return () => document.removeEventListener("visibilitychange", onChange);
}

/** A visible bot chat acknowledges only the inbox entries its transcript has loaded. */
export function useBotChatRead(botId: string | null, onScreen: boolean) {
	const inbox = useBackend().inboxState;
	const { mutate } = useMarkConversationRead();
	const sessionId = useGlobalChatStore((state) => state.activeConversationId);
	const loading = useGlobalChatStore((state) => state.conversationLoading);
	const view = useBotHistoryStore((state) =>
		botId ? state.views[botId] : undefined,
	);
	const visible = useSyncExternalStore(
		subscribeVisibility,
		() => document.visibilityState === "visible",
		() => false,
	);
	const marked = useRef<{
		inbox: typeof inbox;
		conversationId: string;
		upTo: string;
	} | null>(null);
	const cursor = view?.cursor;
	const windowed = view?.windowed;

	useEffect(() => {
		if (
			!onScreen ||
			!visible ||
			loading ||
			!botId ||
			conversationBotId(sessionId) !== botId ||
			!cursor ||
			windowed ||
			!inbox ||
			botInbox() !== inbox
		)
			return;
		const target = activeBotTarget(sessionId);
		if (!target) return;
		const previous = marked.current;
		if (
			previous?.inbox === inbox &&
			previous.conversationId === target.conversationId &&
			compareInboxCursors(cursor, previous.upTo) <= 0
		)
			return;
		const next = { inbox, conversationId: target.conversationId, upTo: cursor };
		marked.current = next;
		mutate(next, {
			onError: () => {
				if (marked.current === next) marked.current = previous;
			},
		});
	}, [
		botId,
		cursor,
		inbox,
		loading,
		mutate,
		onScreen,
		sessionId,
		visible,
		windowed,
	]);
}
