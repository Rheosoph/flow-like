import { useCallback, useEffect, useRef, useState } from "react";
import {
	CHAT_TYPING_FIELD,
	TYPING_TTL_MS,
} from "../lib/realtime/presence-signals";

export interface ChatMessage {
	id: string;
	sub: string;
	text: string;
	timestamp: number;
}

export const CHAT_MESSAGE_MAX_LENGTH = 500;

/** Peers are untrusted: a malformed entry is dropped, an oversized text cut. */
export function sanitizeChatMessage(value: unknown): ChatMessage | undefined {
	if (typeof value !== "object" || value === null || Array.isArray(value))
		return undefined;
	const raw = value as Record<string, unknown>;
	if (
		typeof raw.id !== "string" ||
		raw.id.length === 0 ||
		raw.id.length > 64 ||
		typeof raw.sub !== "string" ||
		raw.sub.length === 0 ||
		raw.sub.length > 128 ||
		typeof raw.text !== "string" ||
		typeof raw.timestamp !== "number" ||
		!Number.isFinite(raw.timestamp) ||
		raw.timestamp < 0
	)
		return undefined;
	return {
		id: raw.id,
		sub: raw.sub,
		text: raw.text.slice(0, CHAT_MESSAGE_MAX_LENGTH),
		timestamp: Math.floor(raw.timestamp),
	};
}

interface UseRealtimeChatProps {
	// biome-ignore lint/suspicious/noExplicitAny: Yjs awareness is untyped
	awareness: any | undefined;
	sub?: string;
}

/** Rolling window of the local user's own messages carried in awareness. */
const PEER_BUFFER = 50;
/** Merged messages kept for display across all peers. */
const MERGED_BUFFER = 200;
/** The typing heartbeat is republished at most this often while keys keep coming. */
const TYPING_HEARTBEAT_MS = 1000;

export function useRealtimeChat({ awareness, sub }: UseRealtimeChatProps) {
	const [messages, setMessages] = useState<ChatMessage[]>([]);
	const [unreadCount, setUnreadCount] = useState(0);
	const isOpenRef = useRef(false);

	// Chat rides awareness (lightweight, no CRDT doc binding): each peer keeps a
	// rolling buffer of its own recent messages and clients merge them for display.
	const messagesRef = useRef<Map<string, ChatMessage>>(new Map());

	useEffect(() => {
		if (!awareness) {
			setMessages([]);
			return;
		}

		const handleChange = () => {
			const states = awareness.getStates() as Map<
				number,
				Record<string, unknown>
			>;
			let changed = false;

			for (const [_clientId, state] of states) {
				const peerMessages = state?.chatMessages;
				if (!Array.isArray(peerMessages) || peerMessages.length === 0) continue;

				for (const entry of peerMessages) {
					const msg = sanitizeChatMessage(entry);
					if (!msg) continue;
					if (!messagesRef.current.has(msg.id)) {
						messagesRef.current.set(msg.id, msg);
						changed = true;

						if (msg.sub !== sub && !isOpenRef.current) {
							setUnreadCount((c) => c + 1);
						}
					}
				}
			}

			if (changed) {
				const sorted = Array.from(messagesRef.current.values()).sort(
					(a, b) => a.timestamp - b.timestamp,
				);
				if (sorted.length > MERGED_BUFFER) {
					for (const msg of sorted.slice(0, sorted.length - MERGED_BUFFER)) {
						messagesRef.current.delete(msg.id);
					}
				}
				setMessages(sorted.slice(-MERGED_BUFFER));
			}
		};

		awareness.on("change", handleChange);
		handleChange();

		return () => {
			try {
				awareness.off("change", handleChange);
			} catch {}
		};
	}, [awareness, sub]);

	const typingPublishedAtRef = useRef(0);
	const typingClearTimerRef = useRef<ReturnType<typeof setTimeout>>(undefined);

	const clearTyping = useCallback(() => {
		if (typingClearTimerRef.current !== undefined) {
			clearTimeout(typingClearTimerRef.current);
			typingClearTimerRef.current = undefined;
		}
		typingPublishedAtRef.current = 0;
		if (!awareness) return;
		try {
			if (awareness.getLocalState()?.[CHAT_TYPING_FIELD] === undefined) return;
			awareness.setLocalStateField(CHAT_TYPING_FIELD, undefined);
		} catch {}
	}, [awareness]);

	/** Publish the typing heartbeat (throttled); it clears itself TYPING_TTL_MS after the last call. */
	const notifyTyping = useCallback(() => {
		if (!awareness) return;
		const now = Date.now();
		if (now - typingPublishedAtRef.current >= TYPING_HEARTBEAT_MS) {
			typingPublishedAtRef.current = now;
			awareness.setLocalStateField(CHAT_TYPING_FIELD, { ts: now });
		}
		if (typingClearTimerRef.current !== undefined) {
			clearTimeout(typingClearTimerRef.current);
		}
		typingClearTimerRef.current = setTimeout(clearTyping, TYPING_TTL_MS);
	}, [awareness, clearTyping]);

	useEffect(() => clearTyping, [clearTyping]);

	const sendMessage = useCallback(
		(text: string) => {
			if (!awareness || !sub || !text.trim()) return;

			const msg: ChatMessage = {
				id: `${sub}-${Date.now()}-${Math.random().toString(36).slice(2, 6)}`,
				sub,
				text: text.trim(),
				timestamp: Date.now(),
			};

			messagesRef.current.set(msg.id, msg);

			const currentState = awareness.getLocalState();
			const buffered: unknown[] = Array.isArray(currentState?.chatMessages)
				? currentState.chatMessages
				: [];
			const existing = buffered
				.map(sanitizeChatMessage)
				.filter((entry): entry is ChatMessage => Boolean(entry));
			awareness.setLocalStateField(
				"chatMessages",
				[...existing, msg].slice(-PEER_BUFFER),
			);
			clearTyping();

			const sorted = Array.from(messagesRef.current.values()).sort(
				(a, b) => a.timestamp - b.timestamp,
			);
			setMessages(sorted.slice(-MERGED_BUFFER));
		},
		[awareness, sub, clearTyping],
	);

	const markAsRead = useCallback(() => {
		setUnreadCount(0);
	}, []);

	const setIsOpen = useCallback(
		(open: boolean) => {
			isOpenRef.current = open;
			if (open) {
				setUnreadCount(0);
			} else {
				clearTyping();
			}
		},
		[clearTyping],
	);

	return {
		messages,
		sendMessage,
		unreadCount,
		markAsRead,
		setIsOpen,
		notifyTyping,
		clearTyping,
	};
}
