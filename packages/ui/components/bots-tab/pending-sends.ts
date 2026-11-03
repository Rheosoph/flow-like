"use client";

import { useEffect } from "react";
import { create } from "zustand";
import type { PendingSend } from "./conversation-messages";

type PendingSendList = readonly PendingSend[];

const NONE: PendingSendList = [];

/**
 * Messages on their way, per conversation. They live outside the conversation view so a send
 * that fails after the view closed still shows as failed, with retry, when the reader returns.
 */
const usePendingSendStore = create<{
	byConversation: Readonly<Record<string, PendingSendList>>;
}>(() => ({ byConversation: {} }));

const openViews = new Map<string, number>();

function update(
	conversationId: string,
	change: (sends: PendingSendList) => PendingSendList,
) {
	usePendingSendStore.setState((state) => ({
		byConversation: {
			...state.byConversation,
			[conversationId]: change(state.byConversation[conversationId] ?? NONE),
		},
	}));
}

export function usePendingSendList(conversationId: string): PendingSendList {
	return usePendingSendStore(
		(state) => state.byConversation[conversationId] ?? NONE,
	);
}

/** While mounted, this conversation's failed sends show in place instead of as a toast. */
export function useShowsPendingSends(conversationId: string) {
	useEffect(() => {
		openViews.set(conversationId, (openViews.get(conversationId) ?? 0) + 1);
		return () => {
			const count = (openViews.get(conversationId) ?? 1) - 1;
			if (count > 0) openViews.set(conversationId, count);
			else openViews.delete(conversationId);
		};
	}, [conversationId]);
}

export async function sendPendingMessage(
	conversationId: string,
	send: Omit<PendingSend, "failed">,
	deliver: () => Promise<unknown>,
	onFailedUnseen: () => void,
): Promise<void> {
	const others = (sends: PendingSendList) =>
		sends.filter((item) => item.messageId !== send.messageId);
	update(conversationId, (sends) => [
		...others(sends),
		{ ...send, failed: false },
	]);
	try {
		await deliver();
		update(conversationId, others);
	} catch (error) {
		console.warn("Sending a message failed", error);
		update(conversationId, (sends) =>
			sends.map((item) =>
				item.messageId === send.messageId ? { ...item, failed: true } : item,
			),
		);
		if (!openViews.has(conversationId)) onFailedUnseen();
	}
}

export function discardPendingSend(conversationId: string, messageId: string) {
	update(conversationId, (sends) =>
		sends.filter((item) => item.messageId !== messageId),
	);
}
