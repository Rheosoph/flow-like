import { useCallback, useEffect, useRef, useState } from "react";

export interface ChatMessage {
	id: string;
	sub: string;
	text: string;
	timestamp: number;
}

interface UseRealtimeChatProps {
	// biome-ignore lint/suspicious/noExplicitAny: Yjs awareness is untyped
	awareness: any | undefined;
	sub?: string;
}

export function useRealtimeChat({ awareness, sub }: UseRealtimeChatProps) {
	const [messages, setMessages] = useState<ChatMessage[]>([]);
	const [unreadCount, setUnreadCount] = useState(0);
	const isOpenRef = useRef(false);

	// Use awareness to exchange chat messages (lightweight, no CRDT doc binding needed)
	// We store a rolling buffer of recent messages in each peer's awareness state,
	// and merge them client-side for display.
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
				const peerMessages = state?.chatMessages as ChatMessage[] | undefined;
				if (!peerMessages?.length) continue;

				for (const msg of peerMessages) {
					if (!messagesRef.current.has(msg.id)) {
						messagesRef.current.set(msg.id, msg);
						changed = true;

						// Count as unread if not from self and chat is closed
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
				// Keep only last 200 messages
				if (sorted.length > 200) {
					const toRemove = sorted.slice(0, sorted.length - 200);
					for (const msg of toRemove) {
						messagesRef.current.delete(msg.id);
					}
				}
				setMessages(sorted.slice(-200));
			}
		};

		awareness.on("change", handleChange);
		// Initial load
		handleChange();

		return () => {
			try {
				awareness.off("change", handleChange);
			} catch {}
		};
	}, [awareness, sub]);

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

			// Get current local messages from awareness and append
			const currentState = awareness.getLocalState();
			const existing = (currentState?.chatMessages as ChatMessage[]) ?? [];
			// Keep a rolling window of last 50 messages per peer
			const updated = [...existing, msg].slice(-50);
			awareness.setLocalStateField("chatMessages", updated);

			// Update local state immediately
			const sorted = Array.from(messagesRef.current.values()).sort(
				(a, b) => a.timestamp - b.timestamp,
			);
			setMessages(sorted.slice(-200));
		},
		[awareness, sub],
	);

	const markAsRead = useCallback(() => {
		setUnreadCount(0);
	}, []);

	const setIsOpen = useCallback((open: boolean) => {
		isOpenRef.current = open;
		if (open) {
			setUnreadCount(0);
		}
	}, []);

	return {
		messages,
		sendMessage,
		unreadCount,
		markAsRead,
		setIsOpen,
	};
}
