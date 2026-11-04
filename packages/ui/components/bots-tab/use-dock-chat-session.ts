"use client";

import { useSearchParams } from "next/navigation";
import { useLayoutEffect } from "react";
import { isBotConversation } from "../../state/global-chat/global-chat-bots";
import { useGlobalChatStore } from "../../state/global-chat/global-chat-store";
import {
	restoreGlobalChatConversation,
	switchGlobalChatBot,
} from "../../state/global-chat/global-chat-stream";
import {
	BOTS_ROUTE,
	type DockSelection,
	dockSelectionKey,
	parseDockSelection,
} from "./dock-selection";

function isFluxUrl(url: URL, sourceKey: string): boolean {
	const selection = parseDockSelection(url.searchParams);
	return (
		url.pathname.replace(/\/$/, "") === BOTS_ROUTE &&
		(dockSelectionKey(selection) === sourceKey ||
			(selection.type === "system" && selection.system === "flux"))
	);
}

function fluxSession(url: URL, sourceKey: string): string | null {
	if (!isFluxUrl(url, sourceKey)) return null;
	const session = url.searchParams.get("session")?.trim();
	return session && !isBotConversation(session) ? session : null;
}

/** Bots have one chat; Flux's selected history entry needs its own bookmark address. */
export function useDockChatSession(
	botId: string | null,
	sourceSelection?: DockSelection,
) {
	const params = useSearchParams();
	const sourceKey = sourceSelection
		? dockSelectionKey(sourceSelection)
		: "system:flux";
	const urlBot = params.get("bot");
	const urlConversation = params.get("conversation");
	const urlSession = params.get("session");

	// biome-ignore lint/correctness/useExhaustiveDependencies: URL changes restart restoration; read the live URL because rendered params can lag behind history writes.
	useLayoutEffect(() => {
		const controller = new AbortController();
		const options = { signal: controller.signal };
		if (botId !== null) {
			void switchGlobalChatBot(botId, options);
			return () => controller.abort();
		}

		// Read the live URL because a history write can precede Next's rendered search params.
		let expectedSession = fluxSession(new URL(window.location.href), sourceKey);
		let pending = true;
		const syncSession = () => {
			if (controller.signal.aborted || pending) return;
			const state = useGlobalChatStore.getState();
			if (
				state.conversationLoading ||
				isBotConversation(state.activeConversationId)
			) {
				return;
			}
			const url = new URL(window.location.href);
			if (
				!isFluxUrl(url, sourceKey) ||
				fluxSession(url, sourceKey) !== expectedSession
			)
				return;
			const session = state.activeConversationId;
			if (url.searchParams.get("session") === session) return;
			expectedSession = session;
			url.searchParams.set("session", session);
			window.history.replaceState(
				null,
				"",
				`${url.pathname}${url.search}${url.hash}`,
			);
		};
		const unsubscribe = useGlobalChatStore.subscribe((state, previous) => {
			if (
				state.activeConversationId !== previous.activeConversationId ||
				state.conversationLoading !== previous.conversationLoading
			) {
				syncSession();
			}
		});

		const current = useGlobalChatStore.getState().activeConversationId;
		const restore = expectedSession
			? expectedSession === current
				? Promise.resolve()
				: restoreGlobalChatConversation(expectedSession, options)
			: switchGlobalChatBot(null, options);
		void restore.then(
			() => {
				pending = false;
				syncSession();
			},
			(error) => {
				pending = false;
				console.warn(
					"[BotChat] Opening the linked conversation failed:",
					error,
				);
			},
		);
		return () => {
			controller.abort();
			unsubscribe();
		};
	}, [botId, sourceKey, urlBot, urlConversation, urlSession]);
}
