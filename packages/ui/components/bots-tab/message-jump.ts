"use client";

import { useCallback, useEffect, useSyncExternalStore } from "react";

/** One message per conversation that a list asked the conversation's view to show. */
const requests = new Map<string, string>();
const listeners = new Set<() => void>();

function emit() {
	for (const listener of listeners) listener();
}

function subscribe(listener: () => void) {
	listeners.add(listener);
	return () => {
		listeners.delete(listener);
	};
}

/** Asks the conversation's view to scroll to the message, now or once it opens. */
export function requestMessageJump(conversationId: string, messageId: string) {
	requests.set(conversationId, messageId);
	emit();
}

export function clearMessageJump(conversationId: string, messageId: string) {
	if (requests.get(conversationId) !== messageId) return;
	requests.delete(conversationId);
	emit();
}

export function useMessageJumpRequest(
	conversationId: string,
): string | undefined {
	const read = useCallback(
		() => requests.get(conversationId),
		[conversationId],
	);
	return useSyncExternalStore(subscribe, read, () => undefined);
}

/** Older pages a jump loads at most while it looks for its message. */
const MAX_JUMP_PAGES = 20;
const FIND_ATTEMPTS = 20;
const FIND_INTERVAL_MS = 50;

function messageElement(messageId: string): HTMLElement | null {
	return document.querySelector<HTMLElement>(
		`[data-chat-message="${CSS.escape(messageId)}"]`,
	);
}

function reveal(element: HTMLElement) {
	element.scrollIntoView({ block: "center", behavior: "smooth" });
	element.animate?.(
		[
			{
				backgroundColor: "color-mix(in oklch, var(--primary) 14%, transparent)",
			},
			{ backgroundColor: "transparent" },
		],
		{ duration: 1600, easing: "ease-out" },
	);
}

/**
 * Scrolls the open conversation to the message a list asked for, loading older pages until it
 * is there; `onMissing` runs when the history holds it nowhere.
 */
export function useMessageJump({
	conversationId,
	loadedIds,
	hasOlder,
	loadingOlder,
	pageCount,
	loadOlder,
	onMissing,
}: Readonly<{
	conversationId: string;
	loadedIds: ReadonlySet<string>;
	hasOlder: boolean;
	loadingOlder: boolean;
	pageCount: number;
	loadOlder: () => void;
	onMissing: () => void;
}>) {
	const target = useMessageJumpRequest(conversationId);
	const loaded = Boolean(target && loadedIds.has(target));
	useEffect(() => {
		if (!target) return;
		if (loaded) {
			let attempts = 0;
			let timer: ReturnType<typeof setTimeout> | undefined;
			const find = () => {
				const element = messageElement(target);
				if (element || attempts >= FIND_ATTEMPTS) {
					clearMessageJump(conversationId, target);
					if (element) reveal(element);
					else onMissing();
					return;
				}
				attempts += 1;
				timer = setTimeout(find, FIND_INTERVAL_MS);
			};
			find();
			return () => clearTimeout(timer);
		}
		if (loadingOlder) return;
		if (hasOlder && pageCount < MAX_JUMP_PAGES) {
			loadOlder();
			return;
		}
		clearMessageJump(conversationId, target);
		onMissing();
	}, [
		target,
		loaded,
		loadingOlder,
		hasOlder,
		pageCount,
		loadOlder,
		onMissing,
		conversationId,
	]);
}
